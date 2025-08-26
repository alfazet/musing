use anyhow::{Result, anyhow, bail};
use cpal::{
    Device as CpalDevice,
    traits::{DeviceTrait, HostTrait},
};
use crossbeam_channel::{self as cbeam_chan};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use tokio::sync::{
    mpsc::{self as tokio_chan},
    oneshot,
};

use crate::{
    constants,
    model::{
        decoder::{Decoder, DecoderRequest, PlaybackTimer, Seek, Speed, Volume},
        device::{Device, DeviceProxy},
        song::{SongEvent, SongProxy},
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
    speed: Arc<RwLock<Speed>>,
    gapless: bool,
}

pub struct Audio {
    playback: Playback,
    devices: HashMap<String, Device>,
    n_enabled_devices: u8,
    tx_request: Option<cbeam_chan::Sender<DecoderRequest>>,
    tx_event: tokio_chan::UnboundedSender<SongEvent>,
}

impl Audio {
    pub fn new(tx_event: tokio_chan::UnboundedSender<SongEvent>) -> Self {
        Self {
            playback: Playback::default(),
            devices: HashMap::new(),
            n_enabled_devices: 0,
            tx_request: None,
            tx_event,
        }
    }

    pub fn play(&mut self, song_proxy: SongProxy) -> Result<()> {
        let volume = Arc::clone(&self.playback.volume);
        let speed = Arc::clone(&self.playback.speed);
        let (tx_request, rx_request) = crossbeam_channel::unbounded();
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
            let _ = tx_request.send(DecoderRequest::Stop);
        }
        let mut decoder = Decoder::try_new(song_proxy, device_proxies, self.playback.gapless)?;
        tokio::task::spawn_blocking(move || {
            if let Err(e) = decoder.run(rx_request, volume, speed) {
                log::error!("decoder error ({})", e);
            }
        });
        self.tx_request = Some(tx_request);
        self.playback.state = PlaybackState::Playing;

        Ok(())
    }

    // use either the system's default audio output device or the provided one
    pub fn with_default(mut self, default_device_name: Option<&String>) -> Result<Self> {
        if let Some(name) = default_device_name {
            let device = audio_utils::device_by_name(name)?;
            self.add_device(device, name)?;
            self.enable_device(name)?;
        } else {
            match audio_utils::default_output_device() {
                Some(device) => {
                    let name = device.name().unwrap_or(constants::UNKNOWN_DEVICE.into());
                    self.add_device(device, &name)?;
                    self.enable_device(&name)?;
                }
                None => bail!("no audio output devices found"),
            }
        }

        Ok(self)
    }

    fn add_device(&mut self, cpal_device: CpalDevice, name: &str) -> Result<()> {
        let device = Device::try_from(cpal_device)?;
        self.devices.insert(name.into(), device);

        Ok(())
    }

    pub fn disable_device(&mut self, device_name: String) -> Result<()> {
        if self.n_enabled_devices == 1 {
            bail!("at least one device must be enabled");
        }
        let res = self
            .devices
            .get_mut(&device_name)
            .ok_or(anyhow!(format!("device {} not found", &device_name)))
            .map(|d| d.disable());
        if res.is_ok() {
            self.n_enabled_devices -= 1;
            if let Some(tx_request) = &self.tx_request {
                let _ = tx_request.send(DecoderRequest::Disable(device_name));
            }
        }

        res
    }

    pub fn enable_device(&mut self, device_name: &str) -> Result<()> {
        let res = match self.devices.get_mut(device_name) {
            Some(device) => match self.playback.state {
                PlaybackState::Stopped => device.enable(None),
                _ => {
                    let res = device.enable(Some(self.tx_event.clone()));
                    if res.is_ok()
                        && let Some(tx_request) = &self.tx_request
                    {
                        let proxy = DeviceProxy::try_new(device).unwrap();
                        let _ = tx_request.send(DecoderRequest::Enable(proxy));
                    }

                    res
                }
            },
            None => bail!(format!("device {} not found", device_name)),
        };
        if let Ok(new_enabled) = res
            && new_enabled
        {
            self.n_enabled_devices += 1;
        }

        res.map(|_| ())
    }

    pub fn list_devices(&self) -> Vec<(String, bool)> {
        self.devices
            .values()
            .map(|d| {
                (
                    d.name().unwrap_or(constants::UNKNOWN_DEVICE.into()),
                    d.is_enabled(),
                )
            })
            .collect()
    }

    pub fn toggle_gapless(&mut self) {
        self.playback.gapless ^= true;
    }

    pub async fn pause(&mut self) -> Result<()> {
        if let PlaybackState::Stopped = self.playback.state {
            return Ok(());
        }
        if let Some(tx_request) = &self.tx_request {
            let (tx, rx) = oneshot::channel();
            // wait for confirmation before pausing devices so that the decoder
            // can finish sending a packet if it's in-progress
            let _ = tx_request.send(DecoderRequest::Pause(tx));
            let _ = rx.await;
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
        if let Some(tx_request) = &self.tx_request {
            let _ = tx_request.send(DecoderRequest::Resume);
        }
        for device in self.devices.values_mut().filter(|d| d.is_enabled()) {
            device.resume()?;
        }
        self.playback.state = PlaybackState::Playing;

        Ok(())
    }

    pub fn stop(&mut self) {
        for device in self.devices.values_mut().filter(|d| d.is_enabled()) {
            device.stop();
        }
        self.playback.state = PlaybackState::Stopped;
        if let Some(tx_request) = &self.tx_request {
            let _ = tx_request.send(DecoderRequest::Stop);
        }
        let _ = self.tx_request.take();
    }

    pub async fn toggle(&mut self) -> Result<()> {
        match self.playback.state {
            PlaybackState::Playing => self.pause().await,
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

    pub fn set_speed(&mut self, new_speed: u16) {
        *self.playback.speed.write().unwrap() = new_speed.into();
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
        .into();
    }

    pub fn set_volume(&mut self, new_v: u8) {
        *self.playback.volume.write().unwrap() = new_v.into();
    }

    pub fn volume(&self) -> u8 {
        (*self.playback.volume.read().unwrap()).into()
    }

    pub async fn playback_timer(&self) -> Option<PlaybackTimer> {
        if let Some(tx_request) = &self.tx_request {
            let (tx, rx) = oneshot::channel();
            let _ = tx_request.send(DecoderRequest::Timer(tx));

            rx.await.ok()
        } else {
            None
        }
    }

    pub fn gapless(&self) -> bool {
        self.playback.gapless
    }

    pub fn state(&self) -> String {
        match self.playback.state {
            PlaybackState::Stopped => "stopped",
            PlaybackState::Playing => "playing",
            PlaybackState::Paused => "paused",
        }
        .into()
    }

    pub fn speed(&self) -> u16 {
        (*self.playback.speed.read().unwrap()).into()
    }
}

mod audio_utils {
    use super::*;

    pub fn default_output_device() -> Option<CpalDevice> {
        let host = cpal::default_host();
        host.default_output_device()
    }

    pub fn device_by_name(device_name: &str) -> Result<CpalDevice> {
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
