use anyhow::{Result, anyhow, bail};
use cpal::{
    BufferSize, Data as CpalData, Device as CpalDevice, FromSample, OutputCallbackInfo,
    SampleFormat, SampleRate, SizedSample, StreamConfig, SupportedStreamConfig,
    platform::Stream as CpalStream,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use crossbeam_channel::{RecvTimeoutError, TryRecvError};
use std::{
    collections::{HashMap, VecDeque},
    fs::File,
    mem,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};
use symphonia::core::units::TimeBase;
use tokio::{
    sync::mpsc,
    task::{self, JoinHandle},
};

use crate::model::{
    decoder::{BaseSample, Decoder, DecoderRequest},
    song::{Song, SongEvent},
};

// after how many ms should we give up waiting for samples and write silence
const UNDERRUN_THRESHOLD: u64 = 50;

type SenderSongEvent = mpsc::UnboundedSender<SongEvent>;
type SenderDecoderRequest = crossbeam_channel::Sender<DecoderRequest>;
type ReceiverSampleChunk = crossbeam_channel::Receiver<Vec<BaseSample>>;

trait Sample: FromSample<BaseSample> + SizedSample + Send + 'static {}

impl Sample for i8 {}
impl Sample for i16 {}
impl Sample for i32 {}
impl Sample for i64 {}
impl Sample for u8 {}
impl Sample for u16 {}
impl Sample for u32 {}
impl Sample for u64 {}
impl Sample for f32 {}
impl Sample for f64 {}

#[derive(Clone, Copy, Debug, Default)]
enum PlaybackState {
    #[default]
    Stopped,
    Playing,
    Paused,
}

#[derive(Clone, Copy)]
struct Volume(u8);

#[derive(Default)]
struct Playback {
    state: PlaybackState,
    volume: Arc<Mutex<Volume>>,
}

#[derive(Clone)]
struct StreamData {
    sample_rate: Option<u32>,
    rx_sample_chunk: ReceiverSampleChunk,
}

// TODO: change this to stream: AudioStream,
// where AudioStream in an enum { Off, Empty, Streaming(CpalStream) }
struct AudioDevice {
    cpal_device: CpalDevice,
    stream: Option<CpalStream>,
    enabled: bool,
}

#[derive(Default)]
pub struct Audio {
    playback: Playback,
    devices: HashMap<String, AudioDevice>,
    stream_data: Option<StreamData>,
    tx_request: Option<SenderDecoderRequest>,
}

impl From<u8> for Volume {
    fn from(x: u8) -> Self {
        Self { 0: x.clamp(0, 100) }
    }
}

impl From<Volume> for u8 {
    fn from(v: Volume) -> Self {
        v.0
    }
}

impl Default for Volume {
    fn default() -> Self {
        Self { 0: 50 }
    }
}

impl From<CpalDevice> for AudioDevice {
    fn from(cpal_device: CpalDevice) -> Self {
        Self {
            cpal_device,
            stream: None,
            enabled: false,
        }
    }
}

impl AudioDevice {
    fn create_data_callback<T>(
        &self,
        rx_sample_chunk: ReceiverSampleChunk,
    ) -> Result<impl FnMut(&mut [T], &OutputCallbackInfo) + Send + 'static>
    where
        T: Sample,
    {
        let mut samples = Vec::new();
        let callback = move |data: &mut [T], _: &OutputCallbackInfo| {
            let deadline = Instant::now() + Duration::from_millis(UNDERRUN_THRESHOLD);
            loop {
                match rx_sample_chunk.recv_deadline(deadline) {
                    Ok(chunk) => {
                        samples.extend(chunk.into_iter().map(|s| T::from_sample(s)));
                        if samples.len() > data.len() {
                            break;
                        }
                    }
                    Err(_) => {
                        data.fill(T::EQUILIBRIUM);
                        return;
                    }
                }
            }
            data.copy_from_slice(&samples[..data.len()]);
            samples = samples[data.len()..].to_vec();
        };

        Ok(callback)
    }

    fn build_cpal_stream(&self, stream_data: StreamData) -> Result<CpalStream> {
        let StreamData {
            sample_rate,
            rx_sample_chunk,
        } = stream_data;
        let sample_rate = SampleRate(sample_rate.unwrap_or(0));
        let supported_configs = self.cpal_device.supported_output_configs()?;
        let first_ok = supported_configs
            .skip_while(|c| sample_rate < c.min_sample_rate() || sample_rate > c.max_sample_rate())
            .next();
        let config = first_ok
            .map(|c| c.with_sample_rate(sample_rate))
            .unwrap_or(self.cpal_device.default_output_config()?);

        macro_rules! build_output_stream {
            ($type:ty) => {
                Ok(self.cpal_device.build_output_stream(
                    &config.into(),
                    self.create_data_callback::<$type>(rx_sample_chunk)?,
                    |e| log::error!("playback error ({})", e),
                    None,
                )?)
            };
        }

        use SampleFormat::*;
        match config.sample_format() {
            I8 => build_output_stream!(i8),
            I16 => build_output_stream!(i16),
            I32 => build_output_stream!(i32),
            I64 => build_output_stream!(i64),
            U8 => build_output_stream!(u8),
            U16 => build_output_stream!(u16),
            U32 => build_output_stream!(u32),
            U64 => build_output_stream!(u64),
            F32 => build_output_stream!(f32),
            F64 => build_output_stream!(f64),
            x => bail!(format!("unsupported sample format `{:?}`", x)),
        }
    }

    // give the current stream to the new device so it can "join in"
    pub fn enable(&mut self, stream_data: Option<StreamData>) -> Result<()> {
        if let Some(stream_data) = stream_data {
            let stream = self.build_cpal_stream(stream_data)?;
            self.start(stream)?;
        }
        self.enabled = true;

        Ok(())
    }

    pub fn disable(&mut self) {
        let _ = self.stream.take();
        self.enabled = false;
    }

    pub fn start(&mut self, stream: CpalStream) -> Result<()> {
        let _ = self.stream.take();
        match stream.play() {
            Ok(_) => {
                self.stream = Some(stream);
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn pause(&mut self) -> Result<()> {
        if let Some(stream) = self.stream.as_ref() {
            stream.pause().map_err(|e| e.into())
        } else {
            Ok(())
        }
    }

    pub fn resume(&mut self) -> Result<()> {
        if let Some(stream) = self.stream.as_ref() {
            stream.play().map_err(|e| e.into())
        } else {
            Ok(())
        }
    }

    pub fn stop(&mut self) {
        let _ = self.stream.take();
    }
}

impl Audio {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_default(mut self, default_device_name: &str) -> Result<Self> {
        self.add_device(default_device_name)?;
        self.enable_device(default_device_name)?;

        Ok(self)
    }

    pub fn play(&mut self, decoder: Decoder, tx_event: SenderSongEvent) -> Result<()> {
        let volume = Arc::clone(&self.playback.volume);
        let (tx_request, rx_request) = crossbeam_channel::unbounded();
        self.tx_request = Some(tx_request);
        let (tx_sample_chunk, rx_sample_chunk) = crossbeam_channel::bounded(1);

        // somehow make it so that each enabled device has its own channel
        // this needs to support adding new devices in the middle of playback
        let stream_data = StreamData {
            sample_rate: decoder.sample_rate(),
            rx_sample_chunk,
        };
        for device in self.devices.values_mut().filter(|d| d.enabled) {
            let stream = device.build_cpal_stream(stream_data.clone())?;
            device.start(stream)?;
        }
        tokio::task::spawn_blocking(move || {
            if let Err(e) = decoder.run(tx_sample_chunk, rx_request) {
                log::error!("decoder error ({})", e);
            }
        });
        tokio::task::spawn_blocking(move || {
            while let Ok(mut chunk) = rx_sample_chunk.recv() {
                let v = { *volume.lock().unwrap() };
                let mult = audio_utils::volume_to_mult(v);
                chunk = chunk
                    .into_iter()
                    .map(|s| (s * mult).clamp(-1.0, 1.0))
                    .collect();
            }
        });

        self.playback.state = PlaybackState::Playing;
        self.stream_data = Some(stream_data);

        Ok(())
    }

    pub fn add_device(&mut self, device_name: &str) -> Result<()> {
        let cpal_device = audio_utils::get_device_by_name(device_name)?;
        let audio_device = AudioDevice::from(cpal_device);
        self.devices.insert(String::from(device_name), audio_device);

        Ok(())
    }

    pub fn enable_device(&mut self, device_name: &str) -> Result<()> {
        if let Some(device) = self.devices.get_mut(device_name) {
            device.enable(self.stream_data.clone())?;
        }

        Ok(())
    }

    pub fn disable_device(&mut self, device_name: &str) {
        if let Some(device) = self.devices.get_mut(device_name) {
            device.disable();
        }
    }

    pub fn toggle_device(&mut self, device_name: &str) -> Result<()> {
        if let Some(device) = self.devices.get_mut(device_name) {
            if device.enabled {
                device.disable();
            } else {
                device.enable(self.stream_data.clone())?;
            }
        }

        Ok(())
    }

    pub fn pause(&mut self) -> Result<()> {
        if let PlaybackState::Stopped = self.playback.state {
            return Ok(());
        }
        for device in self.devices.values_mut().filter(|d| d.enabled) {
            device.pause()?;
        }
        self.playback.state = PlaybackState::Paused;

        Ok(())
    }

    pub fn resume(&mut self) -> Result<()> {
        if let PlaybackState::Stopped = self.playback.state {
            return Ok(());
        }
        for device in self.devices.values_mut().filter(|d| d.enabled) {
            device.resume()?;
        }
        self.playback.state = PlaybackState::Playing;

        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        for device in self.devices.values_mut().filter(|d| d.enabled) {
            device.stop();
        }
        self.playback.state = PlaybackState::Stopped;
        let _ = self.stream_data.take();
        let _ = self.tx_request.take();

        Ok(())
    }

    pub fn toggle(&mut self) -> Result<()> {
        match self.playback.state {
            PlaybackState::Playing => self.pause(),
            PlaybackState::Paused => self.resume(),
            _ => Ok(()),
        }
    }
}

mod audio_utils {
    use super::*;

    pub fn get_device_by_name(device_name: &str) -> Result<CpalDevice> {
        let host = cpal::default_host();
        host.output_devices()?
            .find(|x| x.name().map(|s| s == device_name).unwrap_or(false))
            .ok_or(anyhow!("audio device `{}` unavailable", device_name))
    }

    // non-linear volume slider
    // source: https://www.dr-lex.be/info-stuff/volumecontrols.html
    pub fn volume_to_mult(v: Volume) -> f64 {
        let v: u8 = v.into();
        (0.07 * (v as f64)).exp() / 1000.0
    }
}
