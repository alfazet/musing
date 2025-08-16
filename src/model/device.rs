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

use crate::model::decoder::BaseSample;

// after how many ms should we give up waiting for samples and write silence
const UNDERRUN_THRESHOLD: u64 = 50;

type ReceiverSampleChunk = crossbeam_channel::Receiver<Vec<BaseSample>>;

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
    state: DeviceState,
}

impl StreamData {
    pub fn new(sample_rate: Option<u32>) -> Self {
        Self { sample_rate }
    }
}

impl From<CpalDevice> for Device {
    fn from(cpal_device: CpalDevice) -> Self {
        Self {
            cpal_device,
            state: DeviceState::default(),
        }
    }
}

impl Device {
    fn create_data_callback<T>(
        &self,
        rx_sample_chunk: ReceiverSampleChunk,
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
        let StreamData { sample_rate } = stream_data;
        let sample_rate = SampleRate(sample_rate.unwrap_or(0));
        let supported_configs = self.cpal_device.supported_output_configs()?;
        let first_ok = supported_configs
            .skip_while(|c| sample_rate < c.min_sample_rate() || sample_rate > c.max_sample_rate())
            .next();
        let config = first_ok
            .map(|c| c.with_sample_rate(sample_rate))
            .unwrap_or(self.cpal_device.default_output_config()?);
        let (tx_sample_chunk, rx_sample_chunk) = cbeam_chan::bounded(1);

        macro_rules! build_output_stream {
            ($type:ty) => {
                Ok(Stream {
                    cpal_stream: self.cpal_device.build_output_stream(
                        &config.into(),
                        self.create_data_callback::<$type>(rx_sample_chunk)?,
                        |e| log::error!("playback error ({})", e),
                        None,
                    )?,
                    tx_sample_chunk,
                })
            };
        }

        use SampleFormat::*;
        match config.sample_format() {
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

    pub fn send_chunk(&self, chunk: Vec<BaseSample>) -> Result<()> {
        if let DeviceState::Active(stream) = &self.state {
            stream.tx_sample_chunk.send(chunk)?;
        }

        Ok(())
    }

    pub fn is_enabled(&self) -> bool {
        match self.state {
            DeviceState::Disabled => false,
            _ => true,
        }
    }

    // give the current stream to the new device so it can join in
    pub fn enable(&mut self, stream_data: Option<StreamData>) -> Result<()> {
        self.state = DeviceState::Idle;
        if let Some(stream_data) = stream_data {
            self.play(stream_data)?;
        }

        Ok(())
    }

    pub fn disable(&mut self) {
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
        self.state = DeviceState::Idle;
    }
}
