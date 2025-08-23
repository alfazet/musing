use anyhow::{Result, bail};
use cpal::{
    Device as CpalDevice, FromSample, OutputCallbackInfo, SampleFormat, SizedSample,
    SupportedStreamConfig,
    platform::Stream as CpalStream,
    traits::{DeviceTrait, StreamTrait},
};
use crossbeam_channel::{self as cbeam_chan};
use tokio::sync::mpsc::{self as tokio_chan};

use crate::{constants, model::song::SongEvent};

pub type BaseSample = f64;
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

struct Stream {
    cpal_stream: CpalStream,
    tx_sample: cbeam_chan::Sender<BaseSample>,
}

#[derive(Default)]
enum DeviceState {
    #[default]
    Disabled,
    Idle,
    Active(Stream),
}

pub struct Device {
    cpal_device: CpalDevice,
    config: SupportedStreamConfig,
    state: DeviceState,
}

pub struct DeviceProxy {
    pub name: String,
    pub sample_rate: u32,
    pub tx_sample: cbeam_chan::Sender<BaseSample>,
}

impl TryFrom<CpalDevice> for Device {
    type Error = anyhow::Error;

    fn try_from(cpal_device: CpalDevice) -> Result<Self> {
        let config = cpal_device.default_output_config()?;
        Ok(Self {
            cpal_device,
            config,
            state: DeviceState::default(),
        })
    }
}

impl Device {
    fn create_data_callback<T>(
        &self,
        rx_sample: cbeam_chan::Receiver<BaseSample>,
        tx_event: tokio_chan::UnboundedSender<SongEvent>,
    ) -> Result<impl FnMut(&mut [T], &OutputCallbackInfo) + Send + 'static>
    where
        T: Sample,
    {
        let callback = move |data: &mut [T], _: &OutputCallbackInfo| {
            let mut i = 0;
            while let Ok(s) = rx_sample.try_recv() {
                // NAN == the end of this song
                if s.is_nan() {
                    let _ = tx_event.send(SongEvent::Over);
                    break;
                }
                data[i] = T::from_sample(s);
                i += 1;
                if i >= data.len() {
                    break;
                }
            }
            data[i..].fill(T::EQUILIBRIUM);
        };

        Ok(callback)
    }

    fn build_stream(&self, tx_event: tokio_chan::UnboundedSender<SongEvent>) -> Result<Stream> {
        // buffer 100 ms of audio
        // too little buffering forces the decoder to pause frequently,
        // and too much causes considerable delays on volume changes and seeks
        // 100 ms seems to be a decent middle ground
        let (tx_sample, rx_sample) = cbeam_chan::bounded(
            self.config.channels() as usize * self.config.sample_rate().0 as usize / 10,
        );

        macro_rules! build_output_stream {
            ($type:ty) => {
                Ok(Stream {
                    cpal_stream: self.cpal_device.build_output_stream(
                        &self.config.clone().into(),
                        self.create_data_callback::<$type>(rx_sample, tx_event)?,
                        |e| log::error!("playback error ({})", e),
                        None,
                    )?,
                    tx_sample,
                })
            };
        }

        use SampleFormat::*;
        match self.config.sample_format() {
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

    pub fn is_enabled(&self) -> bool {
        !matches!(self.state, DeviceState::Disabled)
    }

    pub fn name(&self) -> Result<String> {
        self.cpal_device.name().map_err(|e| e.into())
    }

    pub fn disable(&mut self) {
        // this drops the stream (and stops it)
        self.state = DeviceState::Disabled;
    }

    // returns Some(true) if the device had been disabled before
    pub fn enable(
        &mut self,
        tx_event: Option<tokio_chan::UnboundedSender<SongEvent>>,
    ) -> Result<bool> {
        if let DeviceState::Disabled = self.state {
            self.state = DeviceState::Idle;
            if let Some(tx_event) = tx_event {
                self.play(tx_event)?;
            }
            return Ok(true);
        }

        Ok(false)
    }

    pub fn play(&mut self, tx_event: tokio_chan::UnboundedSender<SongEvent>) -> Result<()> {
        let stream = self.build_stream(tx_event)?;
        match stream.cpal_stream.play() {
            Ok(_) => {
                self.state = DeviceState::Active(stream);
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn pause(&mut self) -> Result<()> {
        match &self.state {
            DeviceState::Active(stream) => stream.cpal_stream.pause().map_err(|e| e.into()),
            _ => Ok(()),
        }
    }

    pub fn resume(&mut self) -> Result<()> {
        match &self.state {
            DeviceState::Active(stream) => stream.cpal_stream.play().map_err(|e| e.into()),
            _ => Ok(()),
        }
    }

    pub fn stop(&mut self) {
        // this drops the stream
        self.state = DeviceState::Idle;
    }
}

impl DeviceProxy {
    pub fn try_new(device: &Device) -> Option<Self> {
        match &device.state {
            DeviceState::Active(stream) => Some(Self {
                name: device
                    .cpal_device
                    .name()
                    .unwrap_or(constants::UNKNOWN_DEVICE.into()),
                sample_rate: device.config.sample_rate().0,
                tx_sample: stream.tx_sample.clone(),
            }),
            _ => None,
        }
    }
}
