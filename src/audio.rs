use anyhow::{Result, anyhow, bail};
use cpal::{
    BufferSize, Data as CpalData, Device as CpalDevice, OutputCallbackInfo, SampleFormat,
    SampleRate, StreamConfig, SupportedStreamConfig,
    platform::Stream as CpalStream,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use crossbeam_channel::{RecvTimeoutError, TryRecvError};
use std::{
    collections::HashMap,
    fs::File,
    mem,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
use symphonia::core::{
    conv::{FromSample, IntoSample},
    units::TimeBase,
};
use tokio::{
    sync::{mpsc, oneshot::Sender as OneShotSender},
    task::{self, JoinHandle},
};

use crate::{error::MyError, model::song::*};

type SeekReceiver = mpsc::UnboundedReceiver<i32>;

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
    samples: Arc<(Mutex<Vec<f32>>, Condvar)>,
    sample_i: Arc<Mutex<(usize, usize)>>,
    volume: Arc<Mutex<f32>>,
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
    elapsed: Arc<Mutex<u64>>,
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
    pub fn build_cpal_stream(&self, stream_data: StreamData) -> Result<CpalStream> {
        let StreamData {
            audio_meta,
            samples,
            sample_i,
            volume,
        } = stream_data;

        let default_n_channels = self.stream_config.channels();
        let default_sample_rate = self.stream_config.sample_rate().0;
        let sample_format = self.stream_config.sample_format();
        let stream_config = StreamConfig {
            channels: audio_meta.n_channels.unwrap_or(default_n_channels),
            sample_rate: SampleRate(audio_meta.sample_rate.unwrap_or(default_sample_rate)),
            buffer_size: BufferSize::Default,
        };

        // the entire macro
        // let callback = ...

        bail!("abc");
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
            elapsed: Arc::new(Mutex::new(0)),
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

    pub fn start(
        &mut self,
        song: &Song,
        mut rx_seek: SeekReceiver,
        tx_over: OneShotSender<()>,
    ) -> Result<()> {
        // TODO: take a look at the typical number of samples in one batch
        // and choose this number so that the producer is one second of audio ahead
        let (tx_samples, mut rx_samples) = mpsc::channel(1);
        song.spawn_sample_producer(tx_samples)?;
        let samples = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        let sample_i = Arc::new(Mutex::new((0, 0))); // (last processed sample, target sample)

        let volume = match &self.stream_data {
            Some(stream_data) => Arc::clone(&stream_data.volume),
            None => Arc::new(Mutex::new(0.5)),
        };
        let stream_data = StreamData {
            audio_meta: song.audio_meta,
            samples: Arc::clone(&samples),
            sample_i: Arc::clone(&sample_i),
            volume,
        };
        for device in self.devices.values_mut().filter(|d| d.enabled) {
            let stream = device.build_cpal_stream(stream_data.clone())?;
            device.start(stream)?;
        }
        self.playback_state = PlaybackState::Playing;
        self.stream_data = Some(stream_data);

        tokio::spawn(async move {
            // this task will end when tx_seek is dropped in the Player
            let (samples, samples_cvar) = &*samples;
            loop {
                tokio::select! {
                    res = rx_samples.recv() => if let Some(new_samples) = res {
                        let mut samples = samples.lock().unwrap();
                        samples.extend(new_samples);
                        samples_cvar.notify_one();

                        // send on tx_over if over
                    },
                    // TODO: rx_seek could as well receive a message when a new device becomes
                    // enabled/disabled
                    res = rx_seek.recv() => match res {
                        Some(seek) => (),
                        None => break,
                    },
                    else => break,
                }
            }
        });

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
        *self.elapsed.lock().unwrap() = 0;
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
