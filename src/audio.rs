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

use crate::model::song::*;

// after how many ms should we give up waiting for samples and write silence
const UNDERRUN_THRESHOLD: u64 = 50;

trait Sample: FromSample<f32> + SizedSample + Send + 'static {}

impl Sample for i8 {}
impl Sample for i16 {}
impl Sample for i32 {}
impl Sample for u8 {}
impl Sample for u16 {}
impl Sample for u32 {}
impl Sample for f32 {}
impl Sample for f64 {}

type SenderSeek = crossbeam_channel::Sender<i32>;
type ReceiverSeek = crossbeam_channel::Receiver<i32>;
type SenderSongOver = mpsc::Sender<()>;

#[derive(Clone, Copy, Debug, Default)]
enum PlaybackState {
    #[default]
    Stopped,
    Playing,
    Paused,
}

#[derive(Clone)]
struct StreamData {
    song: PlayerSong,
    rx_seek: ReceiverSeek,
    tx_over: SenderSongOver,
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
    volume: Arc<RwLock<u8>>,
    tx_seek: SenderSeek,
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
            song,
            rx_seek,
            tx_over,
            elapsed,
        } = stream_data;

        let mut samples = Vec::new();
        let mut target_sample = 0;

        let callback = move |data: &mut [T], _: &OutputCallbackInfo| {
            let seek = rx_seek.try_recv().unwrap_or(0);
            if seek > 0 {
                target_sample += (time_base.denom as usize) * (seek as usize);
            } else if seek < 0 {
                target_sample = target_sample
                    .saturating_sub((time_base.denom as usize) * (seek.unsigned_abs() as usize));
            }
            target_sample += data.len();

            loop {
                match song
                    .rx_samples
                    .recv_timeout(Duration::from_millis(UNDERRUN_THRESHOLD))
                {
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
            let cur_slice: Vec<T> = samples[(target_sample - data.len())..target_sample]
                .iter()
                .map(|s| T::from_sample(*s))
                .collect();
            data.copy_from_slice(&cur_slice);
            let elapsed_here = time_base.calc_time(target_sample as u64).seconds;
            let elapsed_global = { *elapsed.read().unwrap() };
            if elapsed_here != elapsed_global {
                *elapsed.write().unwrap() = elapsed_here;
            }
        };

        Ok(callback)
    }

    fn build_cpal_stream(&self, stream_data: StreamData) -> Result<CpalStream> {
        let audio_meta = stream_data.song.audio_meta;
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
                    |e| log::error!("playback error ({})", e),
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
            x => bail!(format!("sample format {:?} is not supported", x)),
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
        let (tx_seek, _) = crossbeam_channel::unbounded();
        Self {
            playback_state: PlaybackState::default(),
            stream_data: None,
            devices: HashMap::new(),
            volume: Arc::new(RwLock::new(50)),
            tx_seek,
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

    pub fn start(&mut self, mut song: PlayerSong, tx_over: SenderSongOver) -> Result<()> {
        // a "middleman" task to transform samples before they're written to devices
        let (tx_middle, rx_middle) = crossbeam_channel::bounded(1);
        let rx_samples = mem::replace(&mut song.rx_samples, rx_middle);
        let volume = Arc::clone(&self.volume);
        tokio::task::spawn_blocking(move || {
            loop {
                match rx_samples.recv() {
                    Ok(mut samples) => {
                        let v = { *volume.read().unwrap() };
                        let mult = audio_utils::volume_to_mult(v);
                        samples = samples
                            .into_iter()
                            .map(|s| (s * mult).clamp(-1.0, 1.0))
                            .collect();
                        let _ = tx_middle.send(samples);
                    }
                    Err(_) => break,
                }
            }
        });

        let (tx_seek, rx_seek) = crossbeam_channel::unbounded();
        let elapsed = Arc::new(RwLock::new(0));
        let stream_data = StreamData {
            song,
            rx_seek,
            tx_over,
            elapsed,
        };
        for device in self.devices.values_mut().filter(|d| d.enabled) {
            let stream = device.build_cpal_stream(stream_data.clone())?;
            device.start(stream)?;
        }
        self.playback_state = PlaybackState::Playing;
        self.stream_data = Some(stream_data);
        self.tx_seek = tx_seek;

        Ok(())
    }

    pub fn pause(&mut self) -> Result<()> {
        if let PlaybackState::Stopped = self.playback_state {
            return Ok(());
        }
        for device in self.devices.values_mut().filter(|d| d.enabled) {
            device.pause()?;
        }
        self.playback_state = PlaybackState::Paused;

        Ok(())
    }

    pub fn resume(&mut self) -> Result<()> {
        if let PlaybackState::Stopped = self.playback_state {
            return Ok(());
        }
        for device in self.devices.values_mut().filter(|d| d.enabled) {
            device.resume()?;
        }
        self.playback_state = PlaybackState::Playing;

        Ok(())
    }

    pub fn seek(&mut self, secs: i32) {
        let _ = self.tx_seek.send(secs);
        if let (Some(stream_data), PlaybackState::Paused) =
            (self.stream_data.as_ref(), self.playback_state)
        {
            // TODO: this can go beyond the duration of the song
            // PlayerSong should also contain an Option<Duration>
            // to prevent this (and to show song durations to the client)
            if secs > 0 {
                *stream_data.elapsed.write().unwrap() += secs as u64;
            } else if secs < 0 {
                let cur_elapsed = { *stream_data.elapsed.read().unwrap() };
                let new_elapsed = cur_elapsed.saturating_sub(secs.unsigned_abs() as u64);
                *stream_data.elapsed.write().unwrap() = new_elapsed;
            }
        }
    }

    pub fn stop(&mut self) -> Result<()> {
        for device in self.devices.values_mut().filter(|d| d.enabled) {
            device.stop();
        }
        self.playback_state = PlaybackState::Stopped;
        let _ = self.stream_data.take();

        Ok(())
    }

    pub fn toggle(&mut self) -> Result<()> {
        match self.playback_state {
            PlaybackState::Playing => self.pause(),
            PlaybackState::Paused => self.resume(),
            _ => Ok(()),
        }
    }

    pub fn change_volume(&mut self, x: i8) {
        let cur_vol = { *self.volume.read().unwrap() };
        let new_vol = if x < 0 {
            cur_vol.saturating_sub(x.unsigned_abs())
        } else {
            (cur_vol + (x as u8)).clamp(0, 100)
        };
        *self.volume.write().unwrap() = new_vol;
    }

    pub fn set_volume(&mut self, x: u8) {
        *self.volume.write().unwrap() = x.clamp(0, 100);
    }

    pub fn elapsed(&self) -> u64 {
        self.stream_data
            .as_ref()
            .map(|data| *data.elapsed.read().unwrap())
            .unwrap_or(0)
    }

    pub fn state(&self) -> u8 {
        match self.playback_state {
            PlaybackState::Stopped => 0,
            PlaybackState::Playing => 1,
            PlaybackState::Paused => 2,
        }
    }

    pub fn volume(&self) -> u8 {
        *self.volume.read().unwrap()
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
    pub fn volume_to_mult(v: u8) -> f32 {
        (0.07 * (v as f32)).exp() / 1000.0
    }
}
