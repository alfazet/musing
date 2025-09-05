use anyhow::{Result, anyhow, bail};
use bincode::{Decode, Encode};
use crossbeam_channel::{self as cbeam_chan, TryRecvError};
use std::{
    io,
    path::Path,
    sync::{Arc, RwLock},
};
use symphonia::core::{
    audio::{AudioBuffer, SampleBuffer, Signal},
    codecs::{Decoder as SymphoniaDecoder, DecoderOptions as SymphoniaDecoderOptions},
    errors::Error as SymphoniaError,
    formats::{FormatReader, SeekMode, SeekTo},
    units::{Time, TimeBase},
};
use tokio::sync::oneshot;

use crate::model::{
    device::{BaseSample, DeviceProxy},
    resampler::Resampler,
    song,
};

const BASE_SAMPLE_MIN: BaseSample = -1.0;
const BASE_SAMPLE_MAX: BaseSample = 1.0;
const MAX_VOLUME: u8 = 100;
const MIN_SPEED: u16 = 25; // x0.25
const MAX_SPEED: u16 = 400; // x4

#[derive(Clone, Copy, Debug, Default)]
pub struct PlaybackTimer {
    pub elapsed: u64,
    pub duration: u64,
    time_base: TimeBase,
}

#[derive(Clone, Copy, Debug, Decode, Encode)]
pub struct Volume(u8);

impl From<u8> for Volume {
    fn from(x: u8) -> Self {
        Self(x.min(MAX_VOLUME))
    }
}

impl From<Volume> for u8 {
    fn from(v: Volume) -> Self {
        v.0
    }
}

impl Default for Volume {
    fn default() -> Self {
        Self(MAX_VOLUME / 2)
    }
}

#[derive(Clone, Copy, Debug, Decode, Encode, PartialEq)]
pub struct Speed(u16);

impl From<u16> for Speed {
    fn from(x: u16) -> Self {
        Self(x.clamp(MIN_SPEED, MAX_SPEED))
    }
}

impl From<Speed> for u16 {
    fn from(s: Speed) -> Self {
        s.0
    }
}

impl Default for Speed {
    fn default() -> Self {
        Self(100)
    }
}

#[derive(Debug)]
pub enum Seek {
    Forwards(u64),
    Backwards(u64),
}

#[derive(Debug)]
pub enum DecoderRequest {
    Disable(String),
    Enable(DeviceProxy),
    Pause(oneshot::Sender<()>),
    Resume,
    Seek(Seek),
    Stop,
    Timer(oneshot::Sender<PlaybackTimer>),
}

#[derive(Debug, Default)]
enum DecoderState {
    #[default]
    Idle,
    Active,
}

pub struct Decoder {
    demuxer: Box<dyn FormatReader>,
    decoder: Box<dyn SymphoniaDecoder>,
    device_proxies: Vec<(DeviceProxy, Option<Resampler>)>,
    track_id: u32,
    timer: PlaybackTimer,
    state: DecoderState,
}

impl Decoder {
    pub fn try_new(
        path: impl AsRef<Path>,
        device_proxies: Vec<DeviceProxy>,
        gapless: bool,
    ) -> Result<Self> {
        let demuxer = song::demuxer(&path, gapless)?;
        let track = demuxer.default_track().ok_or(anyhow!(
            "no audio track found in `{}`",
            path.as_ref().to_string_lossy()
        ))?;
        let track_id = track.id;
        let decoder_opts: SymphoniaDecoderOptions = Default::default();
        let decoder = symphonia::default::get_codecs().make(&track.codec_params, &decoder_opts)?;
        let time_base = decoder
            .codec_params()
            .time_base
            .unwrap_or(TimeBase::default());
        let device_proxies = device_proxies.into_iter().map(|d| (d, None)).collect();
        let timer = PlaybackTimer {
            time_base,
            ..Default::default()
        };
        let state = DecoderState::default();

        Ok(Self {
            demuxer,
            decoder,
            device_proxies,
            track_id,
            timer,
            state,
        })
    }

    fn seek(&mut self, seek: Seek) {
        let target_elapsed = match seek {
            Seek::Forwards(secs) => self
                .timer
                .elapsed
                .saturating_add(secs)
                .min(self.duration().unwrap_or(u64::MAX)),
            Seek::Backwards(secs) => self.timer.elapsed.saturating_sub(secs),
        };
        let target_time = Time {
            seconds: target_elapsed,
            frac: 0.0,
        };
        let seek_to = SeekTo::Time {
            time: target_time,
            track_id: Some(self.track_id),
        };
        if let Ok(seeked_to) = self.demuxer.seek(SeekMode::Coarse, seek_to) {
            self.timer.elapsed = self.timer.time_base.calc_time(seeked_to.actual_ts).seconds;
        }
        self.decoder.reset();
    }

    fn stop(&mut self) {
        for proxy in self.device_proxies.iter() {
            let _ = proxy.0.tx_sample.send(BaseSample::NAN);
        }
        self.timer.elapsed = 0;
        self.timer.duration = 0;
    }

    // true -> stop the decoder
    fn handle_request(&mut self, req: DecoderRequest) -> bool {
        match req {
            DecoderRequest::Disable(device_name) => {
                self.device_proxies.retain(|p| p.0.name != device_name)
            }
            DecoderRequest::Enable(proxy) => {
                self.device_proxies.push((proxy, None));
            }
            DecoderRequest::Pause(tx) => {
                self.state = DecoderState::Idle;
                let _ = tx.send(());
            }
            DecoderRequest::Resume => self.state = DecoderState::Active,
            DecoderRequest::Seek(seek) => self.seek(seek),
            DecoderRequest::Stop => {
                self.stop();
                return true;
            }
            DecoderRequest::Timer(tx) => {
                let _ = tx.send(self.timer);
            }
        }

        false
    }

    pub fn run(
        &mut self,
        rx_request: cbeam_chan::Receiver<DecoderRequest>,
        volume: Arc<RwLock<Volume>>,
        speed: Arc<RwLock<Speed>>,
    ) -> Result<()> {
        fn send_decoded_packet(
            proxies: &mut [(DeviceProxy, Option<Resampler>)],
            data: AudioBuffer<BaseSample>,
            volume: Volume,
        ) {
            if data.frames() == 0 {
                return;
            }
            let mult = decoder_utils::volume_to_mult(volume);
            let spec = data.spec();
            let duration = data.capacity() as u64;
            let mut buf = SampleBuffer::new(duration, *spec);
            buf.copy_interleaved_typed(&data);
            let unchanged_samples = buf.samples();

            for (proxy, resampler) in proxies.iter_mut() {
                let samples = match resampler {
                    Some(resampler) => match resampler.resample(&data) {
                        Some(resampled_samples) => resampled_samples,
                        None => return,
                    },
                    None => unchanged_samples,
                };
                for s in samples
                    .iter()
                    .map(|&s| (s * mult).clamp(BASE_SAMPLE_MIN, BASE_SAMPLE_MAX))
                {
                    let _ = proxy.tx_sample.send(s);
                }
            }
        }

        self.timer.elapsed = 0;
        self.timer.duration = self.duration().unwrap_or_default();
        self.state = DecoderState::Active;
        let mut prev_speed = { *speed.read().unwrap() };
        loop {
            // block if idle to avoid busy waiting
            let request = match self.state {
                DecoderState::Idle => rx_request.recv().map_err(|_| TryRecvError::Disconnected),
                DecoderState::Active => rx_request.try_recv(),
            };
            match request {
                Ok(request) => {
                    if self.handle_request(request) {
                        break;
                    }
                }
                // the player went out of scope (due to an error or a Ctrl+C)
                Err(TryRecvError::Disconnected) => return Ok(()),
                _ => (),
            }
            if let DecoderState::Active = self.state {
                match self.demuxer.next_packet() {
                    Ok(packet) if packet.track_id() == self.track_id => {
                        match self.decoder.decode(&packet) {
                            Ok(data) => {
                                let speed = { *speed.read().unwrap() };
                                let spec = data.spec();
                                let duration = data.capacity() as u64;
                                for (proxy, resampler) in self.device_proxies.iter_mut() {
                                    if (resampler.is_none() && proxy.sample_rate != spec.rate)
                                        || prev_speed != speed
                                    {
                                        *resampler = Some(Resampler::new(
                                            *spec,
                                            proxy.sample_rate,
                                            duration,
                                            speed.into(),
                                        ));
                                    }
                                }
                                prev_speed = speed;

                                let mut typed_data = data.make_equivalent::<BaseSample>();
                                data.convert(&mut typed_data);
                                send_decoded_packet(
                                    &mut self.device_proxies,
                                    typed_data,
                                    *volume.read().unwrap(),
                                );
                                let new_elapsed = self.timer.time_base.calc_time(packet.ts).seconds;
                                if new_elapsed != self.timer.elapsed {
                                    self.timer.elapsed = new_elapsed;
                                }
                            }
                            Err(e) => match e {
                                SymphoniaError::ResetRequired
                                | SymphoniaError::DecodeError(_)
                                | SymphoniaError::IoError(_) => (),
                                _ => bail!(e),
                            },
                        }
                    }
                    Err(e) => match e {
                        SymphoniaError::ResetRequired => {
                            self.decoder.reset();
                        }
                        SymphoniaError::IoError(e)
                            if matches!(e.kind(), io::ErrorKind::UnexpectedEof) =>
                        {
                            // the entire song has been processed
                            self.stop();
                            break;
                        }
                        _ => bail!(e),
                    },
                    _ => (),
                }
            }
        }

        Ok(())
    }

    pub fn duration(&self) -> Option<u64> {
        match (
            self.decoder.codec_params().time_base,
            self.decoder.codec_params().n_frames,
        ) {
            (Some(tb), Some(n)) => Some(tb.calc_time(n).seconds),
            _ => None,
        }
    }
}

mod decoder_utils {
    use super::*;

    // non-linear volume slider
    // source: https://www.dr-lex.be/info-stuff/volumecontrols.html
    pub fn volume_to_mult(v: Volume) -> BaseSample {
        let v: u8 = v.into();
        (((0.07 * (v as BaseSample)).exp() - 1.0) / 1000.0).max(0.0)
    }
}
