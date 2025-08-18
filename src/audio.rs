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
    decoder::{Decoder, DecoderRequest, Seek, Volume},
    device::{ActiveDeviceProxy, BaseSample, Device, StreamData},
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
        {
            *elapsed.write().unwrap() = 0;
        }

        let (tx_request, rx_request) = crossbeam_channel::unbounded();
        let sample_rate = decoder.sample_rate();
        let stream_data = StreamData::new(sample_rate);
        for device in self.devices.values_mut().filter(|d| d.is_enabled()) {
            device.play(stream_data)?;
        }
        let device_proxies: Vec<_> = self
            .devices
            .values()
            .filter_map(|d| ActiveDeviceProxy::try_new(d))
            .collect();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = decoder.run(device_proxies, tx_event, rx_request, volume, elapsed) {
                log::error!("decoder error ({})", e);
            }
        });
        self.decoder_data = Some(DecoderData {
            sample_rate,
            tx_request,
        });
        self.playback.state = PlaybackState::Playing;

        Ok(())
    }

    pub fn add_device(&mut self, device_name: &str) -> Result<()> {
        let cpal_device = audio_utils::get_device_by_name(device_name)?;
        let device = Device::try_from(cpal_device)?;
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

    pub fn seek(&mut self, secs: i64) {
        if let Some(tx) = self.decoder_data.as_ref().map(|d| &d.tx_request) {
            let seek = if secs > 0 {
                Seek::Forwards(secs.unsigned_abs())
            } else {
                Seek::Backwards(secs.unsigned_abs())
            };
            let _ = tx.send(DecoderRequest::Seek(seek));
        }
    }

    pub fn change_volume(&mut self, delta: i8) {
        let mut v_lock = self.playback.volume.write().unwrap();
        let v: u8 = (*v_lock).into();
        // TODO: clean up when
        // https://doc.rust-lang.org/std/primitive.u8.html#method.saturating_sub_signed
        // stabilizes
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

    pub fn elapsed(&self) -> u64 {
        *self.playback.elapsed.read().unwrap()
    }

    pub fn state(&self) -> u8 {
        self.playback.state as u8
    }
}

mod audio_utils {
    use super::*;

    pub fn get_device_by_name(device_name: &str) -> Result<CpalDevice> {
        let host = cpal::default_host();
        match host
            .output_devices()?
            .find(|x| x.name().map(|s| s == device_name).unwrap_or(false))
        {
            Some(device) => Ok(device),
            None => {
                let mut err_msg = format!(
                    "audio device `{}` unavailable, available devices: ",
                    device_name
                );
                for name in host
                    .output_devices()?
                    .map(|d| d.name().unwrap_or("[unnamed]".into()))
                {
                    err_msg += &name;
                    err_msg.push(',');
                }
                bail!(err_msg)
            }
        }
    }
}
