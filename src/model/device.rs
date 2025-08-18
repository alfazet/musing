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

// after how many ms should we give up waiting for samples and write silence
const UNDERRUN_THRESHOLD: u64 = 250;

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
    tx_sample_chunk: cbeam_chan::Sender<Vec<BaseSample>>,
}

#[derive(Default)]
enum DeviceState {
    #[default]
    Disabled,
    Idle,
    Active(Stream),
}

// data that's required to setup a new device to
// play the current stream
#[derive(Clone, Copy)]
pub struct StreamData {
    sample_rate: Option<u32>,
}

pub struct Device {
    cpal_device: CpalDevice,
    config: SupportedStreamConfig,
    state: DeviceState,
}

// a lightweight struct allowing the decoder to "access" the device
pub struct ActiveDeviceProxy {
    pub tx_sample_chunk: cbeam_chan::Sender<Vec<BaseSample>>,
    pub sample_rate: u32,
}

impl StreamData {
    pub fn new(sample_rate: Option<u32>) -> Self {
        Self { sample_rate }
    }
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
        rx_sample_chunk: cbeam_chan::Receiver<Vec<BaseSample>>,
    ) -> Result<impl FnMut(&mut [T], &OutputCallbackInfo) + Send + 'static>
    where
        T: Sample,
    {
        let mut samples = Vec::new();
        let callback = move |data: &mut [T], _: &OutputCallbackInfo| {
            let deadline = Instant::now() + Duration::from_millis(UNDERRUN_THRESHOLD);
            loop {
                match rx_sample_chunk.recv_deadline(deadline) {
                    Ok(chunk) => {
                        samples.extend(chunk.into_iter().map(|s| T::from_sample(s)));
                        if samples.len() > data.len() {
                            break;
                        }
                    }
                    Err(_) => {
                        data.fill(T::EQUILIBRIUM);
                        return;
                    }
                }
            }
            data.copy_from_slice(&samples[..data.len()]);
            samples = samples[data.len()..].to_vec();
        };

        Ok(callback)
    }

    fn build_cpal_stream(&self, stream_data: StreamData) -> Result<Stream> {
        // let StreamData { sample_rate } = stream_data;
        let default_config = self.cpal_device.default_output_config()?;
        let (tx_sample_chunk, rx_sample_chunk) = cbeam_chan::bounded(1);

        macro_rules! build_output_stream {
            ($type:ty) => {
                Ok(Stream {
                    cpal_stream: self.cpal_device.build_output_stream(
                        &default_config.into(),
                        self.create_data_callback::<$type>(rx_sample_chunk)?,
                        |e| log::error!("playback error ({})", e),
                        None,
                    )?,
                    tx_sample_chunk,
                })
            };
        }

        use SampleFormat::*;
        match default_config.sample_format() {
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

    pub fn tx_clone(&self) -> Option<cbeam_chan::Sender<Vec<BaseSample>>> {
        match &self.state {
            DeviceState::Active(stream) => Some(stream.tx_sample_chunk.clone()),
            _ => None,
        }
    }

    pub fn is_enabled(&self) -> bool {
        !matches!(self.state, DeviceState::Disabled)
    }

    pub fn enable(&mut self, stream_data: Option<StreamData>) -> Result<()> {
        if matches!(self.state, DeviceState::Disabled) {
            // give the current stream to the new device so it can join in
            self.state = DeviceState::Idle;
            if let Some(stream_data) = stream_data {
                self.play(stream_data)?;
            }
        }

        Ok(())
    }

    pub fn disable(&mut self) {
        // this drops the stream (and stops it)
        self.state = DeviceState::Disabled;
    }

    pub fn play(&mut self, stream_data: StreamData) -> Result<()> {
        let stream = self.build_cpal_stream(stream_data)?;
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
        // this drops the stream (and stops it)
        self.state = DeviceState::Idle;
    }
}

impl ActiveDeviceProxy {
    pub fn try_new(device: &Device) -> Option<Self> {
        match &device.state {
            DeviceState::Active(stream) => Some(Self {
                tx_sample_chunk: stream.tx_sample_chunk.clone(),
                sample_rate: device.config.sample_rate().0,
            }),
            _ => None,
        }
    }
}
