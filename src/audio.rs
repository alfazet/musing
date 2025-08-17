use anyhow::{Result, anyhow, bail};
use cpal::{
    BufferSize, Data as CpalData, Device as CpalDevice, FromSample, OutputCallbackInfo,
    SampleFormat, SampleRate, SizedSample, StreamConfig, SupportedStreamConfig,
    platform::Stream as CpalStream,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use crossbeam_channel::{self as cbeam_chan, RecvTimeoutError, TryRecvError};
use std::{
    collections::{HashMap, VecDeque},
    fs::File,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};
use symphonia::core::units::TimeBase;
use tokio::{
    sync::mpsc::{self as tokio_chan},
    task::{self, JoinHandle},
};

use crate::model::{
    decoder::{BaseSample, Decoder, DecoderRequest},
    device::{Device, StreamData},
    song::{Song, SongEvent},
};

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

struct Decoding {
    sample_rate: Option<u32>,
    tx_request: cbeam_chan::Sender<DecoderRequest>,
}

#[derive(Default)]
pub struct Audio {
    playback: Playback,
    devices: HashMap<String, Device>,
    decoding: Option<Decoding>,
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

impl Audio {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_default(mut self, default_device_name: &str) -> Result<Self> {
        self.add_device(default_device_name)?;
        self.enable_device(default_device_name)?;

        Ok(self)
    }

    pub fn play(
        &mut self,
        mut decoder: Decoder,
        tx_event: tokio_chan::UnboundedSender<SongEvent>,
    ) -> Result<()> {
        let volume = Arc::clone(&self.playback.volume);
        let (tx_request, rx_request) = crossbeam_channel::unbounded();
        let sample_rate = decoder.sample_rate();
        self.decoding = Some(Decoding {
            sample_rate,
            tx_request,
        });
        let stream_data = StreamData::new(sample_rate);
        let (tx_chunk, rx_chunk) = crossbeam_channel::bounded(1);
        for device in self.devices.values_mut().filter(|d| d.is_enabled()) {
            device.play(stream_data)?;
        }
        let mut txs: Vec<cbeam_chan::Sender<Vec<BaseSample>>> =
            self.devices.values().filter_map(|d| d.tx_clone()).collect();

        tokio::task::spawn_blocking(move || {
            if let Err(e) = decoder.run(tx_chunk, tx_event, rx_request) {
                log::error!("decoder error ({})", e);
            }
        });

        tokio::task::spawn_blocking(move || {
            while let Ok(mut chunk) = rx_chunk.recv() {
                // get the timestamp of this chunk and change elapsed
                let v = { *volume.lock().unwrap() };
                let mult = audio_utils::volume_to_mult(v);
                chunk = chunk
                    .into_iter()
                    .map(|s| (s * mult).clamp(-1.0, 1.0))
                    .collect();
                for tx in txs.iter() {
                    let _ = tx.send(chunk.clone());
                }
            }
        });
        self.playback.state = PlaybackState::Playing;

        Ok(())
    }

    pub fn add_device(&mut self, device_name: &str) -> Result<()> {
        let cpal_device = audio_utils::get_device_by_name(device_name)?;
        let device = Device::from(cpal_device);
        self.devices.insert(String::from(device_name), device);

        Ok(())
    }

    pub fn enable_device(&mut self, device_name: &str) -> Result<()> {
        if let Some(device) = self.devices.get_mut(device_name) {
            device.enable(
                self.decoding
                    .as_ref()
                    .map(|d| StreamData::new(d.sample_rate)),
            )?;
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
            if device.is_enabled() {
                device.disable();
            } else {
                device.enable(
                    self.decoding
                        .as_ref()
                        .map(|d| StreamData::new(d.sample_rate)),
                )?;
            }
        }

        Ok(())
    }

    pub fn pause(&mut self) -> Result<()> {
        if let PlaybackState::Stopped = self.playback.state {
            return Ok(());
        }
        for device in self.devices.values_mut().filter(|d| d.is_enabled()) {
            device.pause()?;
        }
        self.playback.state = PlaybackState::Paused;

        Ok(())
    }

    pub fn resume(&mut self) -> Result<()> {
        if let PlaybackState::Stopped = self.playback.state {
            return Ok(());
        }
        for device in self.devices.values_mut().filter(|d| d.is_enabled()) {
            device.resume()?;
        }
        self.playback.state = PlaybackState::Playing;

        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        for device in self.devices.values_mut().filter(|d| d.is_enabled()) {
            device.stop();
        }
        self.playback.state = PlaybackState::Stopped;
        let _ = self.decoding.take();

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
