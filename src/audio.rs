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

use crate::{
    constants,
    model::{
        decoder::{Decoder, DecoderRequest, Seek, Volume},
        device::{BaseSample, Device, DeviceProxy},
        song::{Song, SongEvent},
    },
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

pub struct Audio {
    playback: Playback,
    devices: HashMap<String, Device>,
    tx_request: Option<cbeam_chan::Sender<DecoderRequest>>,
    tx_event: tokio_chan::UnboundedSender<SongEvent>,
}

impl Audio {
    pub fn new(tx_event: tokio_chan::UnboundedSender<SongEvent>) -> Self {
        Self {
            playback: Playback::default(),
            devices: HashMap::new(),
            tx_request: None,
            tx_event,
        }
    }

    pub fn with_default(mut self, default_devices_names: &[String]) -> Self {
        for name in default_devices_names.iter() {
            if let Err(e) = self.add_device(name).and_then(|_| self.enable_device(name)) {
                log::error!("could not add device {} ({})", name, e);
            }
        }

        self
    }

    pub fn play(&mut self, mut decoder: Decoder) -> Result<()> {
        let volume = Arc::clone(&self.playback.volume);
        let elapsed = Arc::clone(&self.playback.elapsed);
        let (tx_request, rx_request) = crossbeam_channel::unbounded();
        let sample_rate = decoder.sample_rate();
        for device in self.devices.values_mut().filter(|d| d.is_enabled()) {
            device.play(self.tx_event.clone())?;
        }
        let device_proxies: Vec<_> = self
            .devices
            .values()
            .filter_map(DeviceProxy::try_new)
            .collect();
        if device_proxies.is_empty() {
            bail!("playback error (all audio devices are disabled)");
        }
        if let Some(tx_request) = &self.tx_request {
            tx_request.send(DecoderRequest::Stop);
        }
        tokio::task::spawn_blocking(move || {
            if let Err(e) = decoder.run(device_proxies, rx_request, volume, elapsed) {
                log::error!("decoder error ({})", e);
            }
        });
        self.tx_request = Some(tx_request);
        self.playback.state = PlaybackState::Playing;

        Ok(())
    }

    pub fn add_device(&mut self, device_name: &str) -> Result<()> {
        let cpal_device = audio_utils::get_device_by_name(device_name)?;
        let device = Device::try_from(cpal_device)?;
        self.devices.insert(String::from(device_name), device);

        Ok(())
    }

    pub fn disable_device(&mut self, device_name: String) -> Result<()> {
        let res = self
            .devices
            .get_mut(&device_name)
            .ok_or(anyhow!(format!("device {} not found", &device_name)))
            .map(|d| d.disable());
        if res.is_ok()
            && let Some(tx_request) = &self.tx_request
        {
            let _ = tx_request.send(DecoderRequest::Disable(device_name));
        }

        res
    }

    pub fn enable_device(&mut self, device_name: &str) -> Result<()> {
        match self.devices.get_mut(device_name) {
            Some(device) => match self.playback.state {
                PlaybackState::Stopped => device.enable(None),
                _ => {
                    let res = device.enable(Some(self.tx_event.clone()));
                    if res.is_ok()
                        && let Some(tx_request) = &self.tx_request
                    {
                        let proxy = DeviceProxy::try_new(&device).unwrap();
                        let _ = tx_request.send(DecoderRequest::Enable(proxy));
                    }

                    res
                }
            },
            None => bail!(format!("device {} not found", device_name)),
        }
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
        if let Some(tx_request) = &self.tx_request {
            tx_request.send(DecoderRequest::Stop);
        }
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

    pub fn seek(&mut self, secs: i64) {
        if let Some(tx) = &self.tx_request {
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
                    .map(|d| d.name().unwrap_or(constants::UNKNOWN_DEVICE.into()))
                {
                    err_msg += &name;
                    err_msg.push(',');
                }
                bail!(err_msg)
            }
        }
    }
}
