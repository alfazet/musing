use anyhow::{Result, anyhow, bail};
use cpal::{
    BufferSize, Device as CpalDevice, OutputCallbackInfo, SampleFormat, SampleRate, StreamConfig,
    SupportedStreamConfig,
    platform::Stream,
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
use symphonia::core::{
    conv::{FromSample, IntoSample},
    units::TimeBase,
};
use tokio::{
    sync::{mpsc, oneshot},
    task::{self, JoinHandle},
};

use crate::{error::MyError, model::song::*};

type SeekReceiver = mpsc::UnboundedReceiver<i32>;

#[derive(Debug)]
enum PlaybackState {
    Playing,
    Paused,
    Stopped,
}

struct Playback {
    state: PlaybackState,
    stream: Option<Stream>,
}

struct AudioState {
    playback: Playback,
    volume: Arc<RwLock<f32>>,
    elapsed: Arc<RwLock<u64>>,
}

struct AudioDevice {
    cpal_device: CpalDevice,
    stream_config: SupportedStreamConfig,
    enabled: bool,
}

pub struct AudioBackend {
    devices: HashMap<String, AudioDevice>,
    state: AudioState,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            state: PlaybackState::Stopped,
            stream: None,
        }
    }
}

impl Playback {
    pub fn resume(&mut self) -> Result<()> {
        if let PlaybackState::Paused = self.state {
            match self.stream.as_ref().unwrap().play() {
                Ok(_) => {
                    self.state = PlaybackState::Playing;
                    Ok(())
                }
                Err(e) => Err(e.into()),
            }
        } else {
            Ok(())
        }
    }

    pub fn pause(&mut self) -> Result<()> {
        if let PlaybackState::Playing = self.state {
            match self.stream.as_ref().unwrap().pause() {
                Ok(_) => {
                    self.state = PlaybackState::Paused;
                    Ok(())
                }
                Err(e) => Err(e.into()),
            }
        } else {
            Ok(())
        }
    }

    pub fn toggle(&mut self) -> Result<()> {
        match self.state {
            PlaybackState::Playing => self.pause()?,
            PlaybackState::Paused => self.resume()?,
            _ => (),
        }

        Ok(())
    }

    pub fn start(&mut self, stream: Stream) -> Result<()> {
        let _ = self.stream.take();
        match stream.play() {
            Ok(_) => {
                self.stream = Some(stream);
                self.state = PlaybackState::Playing;
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn stop(&mut self) {
        let _ = self.stream.take();
        self.state = PlaybackState::Stopped;
    }
}

impl Default for AudioState {
    fn default() -> Self {
        Self {
            playback: Playback::default(),
            volume: Arc::new(RwLock::new(1.0)),
            elapsed: Arc::new(RwLock::new(0)),
        }
    }
}

impl TryFrom<CpalDevice> for AudioDevice {
    type Error = anyhow::Error;

    fn try_from(cpal_device: CpalDevice) -> Result<Self> {
        let stream_config = cpal_device.default_output_config()?;
        let audio_device = Self {
            cpal_device,
            stream_config,
            enabled: false,
        };

        Ok(audio_device)
    }
}

impl AudioBackend {
    pub fn new() -> Self {
        let state = AudioState::default();

        Self {
            devices: HashMap::new(),
            state,
        }
    }

    pub fn with_default(mut self, default_device_name: &str) -> Result<Self> {
        self.add_device(default_device_name)?;
        self.enable_device(default_device_name);

        Ok(self)
    }

    pub fn add_device(&mut self, device_name: &str) -> Result<()> {
        let cpal_device = audio_utils::get_device_by_name(device_name)?;
        let audio_device = AudioDevice::try_from(cpal_device)?;
        self.devices.insert(String::from(device_name), audio_device);

        Ok(())
    }

    pub fn enable_device(&mut self, device_name: &str) {
        if let Some(device) = self.devices.get_mut(device_name) {
            device.enabled = true;
        }
    }

    pub fn disable_device(&mut self, device_name: &str) {
        if let Some(device) = self.devices.get_mut(device_name) {
            device.enabled = false;
        }
    }

    pub fn toggle_device(&mut self, device_name: &str) {
        if let Some(device) = self.devices.get_mut(device_name) {
            device.enabled ^= true;
        }
    }

    pub async fn start_playback(
        &mut self,
        song: &Song,
        rx_seek: SeekReceiver,
        tx_over: oneshot::Sender<()>,
    ) -> Result<()> {
        // volume and elapsed are Arc<RwLocks> kept in AudioState
        // let (tx_samples, rx_samples) = mpsc::bounded(1);
        // make a blocking task to produce samples
        // tokio::spawn the task that will tokio::select one of two receivers
        // - samples receiver (on recv adds them to the RwLocked vector)
        //      if the song ends notify the Player by tx_over
        // - rx_seek (its tx half will be kept in the Player as an Option)
        // (we should be able to cancel this task in case we start a new playback)

        // create the stream (with _raw, don't use that weird macro anymore)
        // the cpal callback should *only* consist of waiting until the length of the samples Vec
        // is >= target sample, if yes write samples to output, if time runs out write silence

        Ok(())
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
