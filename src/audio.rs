use anyhow::{Result, anyhow, bail};
use cpal::{
    BufferSize, Data as CpalData, Device as CpalDevice, FromSample, OutputCallbackInfo,
    SampleFormat, SampleRate, SizedSample, StreamConfig, SupportedStreamConfig,
    platform::Stream as CpalStream,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use crossbeam_channel::{RecvTimeoutError, TryRecvError};
use std::{
    collections::HashMap,
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

use crate::{error::MyError, model::song::*};

const UNDERRUN_THRESHOLD: u64 = 250;

trait Sample: FromSample<f32> + SizedSample + Send + 'static {}

impl Sample for i8 {}
impl Sample for i16 {}
impl Sample for i32 {}
impl Sample for u8 {}
impl Sample for u16 {}
impl Sample for u32 {}
impl Sample for f32 {}
impl Sample for f64 {}

// type SeekReceiver = mpsc::UnboundedReceiver<i32>;
type SenderSongOver = mpsc::Sender<()>;
type ReceiverSamples = crossbeam_channel::Receiver<Vec<f32>>;

#[derive(Debug, Default)]
enum PlaybackState {
    Playing,
    Paused,
    #[default]
    Stopped,
}

#[derive(Clone)]
struct StreamData {
    audio_meta: AudioMeta,
    rx_samples: ReceiverSamples,
    tx_over: SenderSongOver,
    volume: Arc<RwLock<f32>>,
    elapsed: Arc<RwLock<u64>>,
}

struct AudioDevice {
    cpal_device: CpalDevice,
    stream_config: SupportedStreamConfig,
    stream: Option<CpalStream>,
    enabled: bool,
}

pub struct Audio {
    playback_state: PlaybackState,
    stream_data: Option<StreamData>,
    devices: HashMap<String, AudioDevice>,
}

impl TryFrom<CpalDevice> for AudioDevice {
    type Error = anyhow::Error;

    fn try_from(cpal_device: CpalDevice) -> Result<Self> {
        let stream_config = cpal_device.default_output_config()?;
        let audio_device = Self {
            cpal_device,
            stream_config,
            stream: None,
            enabled: false,
        };

        Ok(audio_device)
    }
}

impl AudioDevice {
    fn create_data_callback<T>(
        &self,
        stream_data: StreamData,
        time_base: TimeBase,
    ) -> Result<impl FnMut(&mut [T], &OutputCallbackInfo) + Send + 'static>
    where
        T: Sample,
    {
        let StreamData {
            audio_meta,
            rx_samples,
            tx_over,
            volume,
            elapsed,
        } = stream_data;

        let transform_samples = move |samples: &[f32]| -> Vec<T> {
            let volume = { *volume.read().unwrap() };
            samples.into_iter().map(|s| T::from_sample(*s)).collect()
        };

        let mut samples = Vec::new();
        let mut target_sample = 0;
        let callback = move |data: &mut [T], _: &OutputCallbackInfo| {
            target_sample += data.len();
            // TODO: seek
            loop {
                match rx_samples.recv_timeout(Duration::from_millis(UNDERRUN_THRESHOLD)) {
                    Ok(new_samples) => {
                        samples.extend(new_samples);
                        if samples.len() >= target_sample {
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        if target_sample >= samples.len() {
                            let _ = tx_over.blocking_send(());
                            return;
                        }
                        break;
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        data.fill(T::EQUILIBRIUM);
                        return;
                    }
                }
            }
            let cur_slice = &samples[(target_sample - data.len())..target_sample];
            data.copy_from_slice(&transform_samples(cur_slice));
            let elapsed_here = time_base.calc_time(target_sample as u64).seconds;
            let elapsed_global = { *elapsed.read().unwrap() };
            if elapsed_here > elapsed_global {
                *elapsed.write().unwrap() = elapsed_here;
            }
        };

        Ok(callback)
    }

    fn build_cpal_stream(&self, stream_data: StreamData) -> Result<CpalStream> {
        let audio_meta = stream_data.audio_meta;
        let default_n_channels = self.stream_config.channels();
        let default_sample_rate = self.stream_config.sample_rate().0;
        let sample_format = self.stream_config.sample_format();
        let stream_config = StreamConfig {
            channels: audio_meta.n_channels.unwrap_or(default_n_channels),
            sample_rate: SampleRate(audio_meta.sample_rate.unwrap_or(default_sample_rate)),
            buffer_size: BufferSize::Default,
        };
        let time_base = TimeBase {
            numer: 1,
            denom: stream_config.sample_rate.0 * (stream_config.channels as u32),
        }; // the fraction of a second that correponds to one sample

        macro_rules! build_output_stream {
            ($type:ty) => {
                Ok(self.cpal_device.build_output_stream(
                    &stream_config,
                    self.create_data_callback::<$type>(stream_data, time_base)?,
                    |e| log::error!("{}", e),
                    None,
                )?)
            };
        }

        use SampleFormat::*;
        match sample_format {
            I8 => build_output_stream!(i8),
            I16 => build_output_stream!(i16),
            I32 => build_output_stream!(i32),
            U8 => build_output_stream!(u8),
            U16 => build_output_stream!(u16),
            U32 => build_output_stream!(u32),
            F32 => build_output_stream!(f32),
            F64 => build_output_stream!(f64),
            x => bail!(MyError::Audio(format!(
                "Sample format {:?} is not supported",
                x
            ))),
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

impl Default for Audio {
    fn default() -> Self {
        Self {
            playback_state: PlaybackState::default(),
            devices: HashMap::new(),
            stream_data: None,
        }
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

    pub fn add_device(&mut self, device_name: &str) -> Result<()> {
        let cpal_device = audio_utils::get_device_by_name(device_name)?;
        let audio_device = AudioDevice::try_from(cpal_device)?;
        self.devices.insert(String::from(device_name), audio_device);

        Ok(())
    }

    pub fn enable_device(&mut self, device_name: &str) -> Result<()> {
        if let Some(mut device) = self.devices.get_mut(device_name) {
            device.enable(self.stream_data.clone())?;
        }

        Ok(())
    }

    pub fn disable_device(&mut self, device_name: &str) {
        if let Some(mut device) = self.devices.get_mut(device_name) {
            device.disable();
        }
    }

    pub fn toggle_device(&mut self, device_name: &str) -> Result<()> {
        if let Some(mut device) = self.devices.get_mut(device_name) {
            if device.enabled {
                device.disable();
            } else {
                device.enable(self.stream_data.clone())?;
            }
        }

        Ok(())
    }

    // TODO: seek
    pub fn start(&mut self, song: &Song, tx_over: SenderSongOver) -> Result<()> {
        let audio_meta = song.audio_meta;
        let (tx_samples, rx_samples) = crossbeam_channel::bounded(1);
        song.spawn_sample_producer(tx_samples)?;

        let volume = match &self.stream_data {
            Some(stream_data) => Arc::clone(&stream_data.volume),
            None => Arc::new(RwLock::new(1.0)),
        };
        let elapsed = Arc::new(RwLock::new(0));
        let stream_data = StreamData {
            audio_meta,
            rx_samples,
            tx_over,
            volume,
            elapsed,
        };
        for device in self.devices.values_mut().filter(|d| d.enabled) {
            let stream = device.build_cpal_stream(stream_data.clone())?;
            device.start(stream)?;
        }
        self.playback_state = PlaybackState::Playing;
        self.stream_data = Some(stream_data);

        Ok(())
    }

    pub fn pause(&mut self) -> Result<()> {
        for device in self.devices.values_mut().filter(|d| d.enabled) {
            device.pause()?;
        }
        self.playback_state = PlaybackState::Paused;

        Ok(())
    }

    pub fn resume(&mut self) -> Result<()> {
        for device in self.devices.values_mut().filter(|d| d.enabled) {
            device.resume()?;
        }
        self.playback_state = PlaybackState::Playing;

        Ok(())
    }

    pub fn stop(&mut self) {
        for device in self.devices.values_mut().filter(|d| d.enabled) {
            device.stop();
        }
        self.playback_state = PlaybackState::Stopped;
        let _ = self.stream_data.take();
    }

    pub fn toggle(&mut self) -> Result<()> {
        match self.playback_state {
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
            .ok_or(anyhow!(MyError::Audio(format!(
                "Audio device `{}` unavailable",
                device_name
            ))))
    }
}
