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
    decoder::{BaseSample, Decoder, DecoderRequest, Volume},
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

#[derive(Default)]
struct Playback {
    state: PlaybackState,
    volume: Arc<RwLock<Volume>>,
    elapsed: Arc<RwLock<u64>>,
}

struct DecoderData {
    sample_rate: Option<u32>,
    tx_request: cbeam_chan::Sender<DecoderRequest>,
}

#[derive(Default)]
pub struct Audio {
    playback: Playback,
    devices: HashMap<String, Device>,
    decoder_data: Option<DecoderData>,
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
        let elapsed = Arc::clone(&self.playback.elapsed);
        let (tx_request, rx_request) = crossbeam_channel::unbounded();
        let sample_rate = decoder.sample_rate();
        self.decoder_data = Some(DecoderData {
            sample_rate,
            tx_request,
        });
        let stream_data = StreamData::new(sample_rate);
        for device in self.devices.values_mut().filter(|d| d.is_enabled()) {
            device.play(stream_data)?;
        }
        let devices_txs: Vec<cbeam_chan::Sender<Vec<BaseSample>>> =
            self.devices.values().filter_map(|d| d.tx_clone()).collect();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = decoder.run(devices_txs, tx_event, rx_request, volume, elapsed) {
                log::error!("decoder error ({})", e);
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
                self.decoder_data
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
                    self.decoder_data
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
        let _ = self.decoder_data.take();

        Ok(())
    }

    pub fn toggle(&mut self) -> Result<()> {
        match self.playback.state {
            PlaybackState::Playing => self.pause(),
            PlaybackState::Paused => self.resume(),
            _ => Ok(()),
        }
    }

    pub fn change_volume(&mut self, delta: i8) {
        let mut v_lock = self.playback.volume.write().unwrap();
        let v: u8 = (*v_lock).into();
        // TODO: clean up when
        // https://doc.rust-lang.org/std/primitive.u8.html#method.saturating_sub_signed
        // stablizies
        *v_lock = {
            if delta < 0 {
                v.saturating_sub(delta.unsigned_abs())
            } else {
                v.saturating_add(delta.unsigned_abs())
            }
        }
        .into()
    }

    pub fn set_volume(&mut self, new_v: u8) {
        *self.playback.volume.write().unwrap() = new_v.into();
    }

    pub fn volume(&self) -> u8 {
        (*self.playback.volume.read().unwrap()).into()
    }

    pub fn state(&self) -> u8 {
        self.playback.state as u8
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
}
