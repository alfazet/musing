use anyhow::{Result, anyhow, bail};
use crossbeam_channel::{self as cbeam_chan, TryRecvError};
use std::{
    io,
    sync::{Arc, RwLock},
};
use symphonia::core::{
    audio::{AudioBufferRef, SampleBuffer},
    codecs::{Decoder as SymphoniaDecoder, DecoderOptions as SymphoniaDecoderOptions},
    errors::Error as SymphoniaError,
    formats::{FormatReader, SeekMode, SeekTo},
    units::Time,
};

use crate::model::{
    device::{BaseSample, DeviceProxy},
    resampler::Resampler,
    song::SongProxy,
};

const BASE_SAMPLE_MIN: BaseSample = -1.0;
const BASE_SAMPLE_MAX: BaseSample = 1.0;
const MAX_VOLUME: u8 = 100;

#[derive(Clone, Copy)]
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

// TODO: seek forwards, backwards and percentage (e.g. 50% = to the middle of the song)
#[derive(Debug)]
pub enum Seek {
    Forwards(u64),
    Backwards(u64),
}

pub enum DecoderRequest {
    Disable(String),
    Enable(DeviceProxy),
    Seek(Seek),
    Stop,
}

pub struct Decoder {
    demuxer: Box<dyn FormatReader>,
    decoder: Box<dyn SymphoniaDecoder>,
    device_proxies: Vec<(DeviceProxy, Option<Resampler<BaseSample>>)>,
    track_id: u32,
    local_elapsed: u64,
}

impl Decoder {
    pub fn try_new(
        song_proxy: SongProxy,
        device_proxies: Vec<DeviceProxy>,
        gapless: bool,
    ) -> Result<Self> {
        let demuxer = song_proxy.demuxer(gapless)?;
        let track = demuxer.default_track().ok_or(anyhow!(
            "no audio track found in `{}`",
            song_proxy.path.to_string_lossy()
        ))?;
        let track_id = track.id;
        let decoder_opts: SymphoniaDecoderOptions = Default::default();
        let decoder = symphonia::default::get_codecs().make(&track.codec_params, &decoder_opts)?;
        let device_proxies = device_proxies.into_iter().map(|d| (d, None)).collect();

        Ok(Self {
            demuxer,
            decoder,
            device_proxies,
            track_id,
            local_elapsed: 0,
        })
    }

    fn seek(&mut self, seek: Seek) {
        let target_elapsed = match seek {
            Seek::Forwards(secs) => self
                .local_elapsed
                .saturating_add(secs)
                .min(self.duration().unwrap_or(u64::MAX)),
            Seek::Backwards(secs) => self.local_elapsed.saturating_sub(secs),
        };
        let target_time = Time {
            seconds: target_elapsed,
            frac: 0.0,
        };
        let seek_to = SeekTo::Time {
            time: target_time,
            track_id: Some(self.track_id),
        };
        let _ = self.demuxer.seek(SeekMode::Coarse, seek_to);
        self.decoder.reset();
    }

    fn stop(&self) {
        for proxy in self.device_proxies.iter() {
            let _ = proxy.0.tx_sample.send(BaseSample::NAN);
        }
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
            DecoderRequest::Seek(seek) => self.seek(seek),
            DecoderRequest::Stop => {
                self.stop();
                return true;
            }
        }

        false
    }

    pub fn run(
        &mut self,
        rx_request: cbeam_chan::Receiver<DecoderRequest>,
        volume: Arc<RwLock<Volume>>,
        elapsed: Arc<RwLock<u64>>,
    ) -> Result<()> {
        fn send_decoded_packet(
            proxies: &mut [(DeviceProxy, Option<Resampler<BaseSample>>)],
            decoded: AudioBufferRef,
            volume: Volume,
        ) {
            if decoded.frames() == 0 {
                return;
            }
            let mult = decoder_utils::volume_to_mult(volume);
            let spec = *decoded.spec();
            let duration = decoded.capacity() as u64;
            let mut buf = SampleBuffer::new(duration, spec);
            buf.copy_interleaved_ref(decoded.clone());
            let unchanged_samples = buf.samples();

            for (proxy, resampler) in proxies.iter_mut() {
                let samples = match resampler {
                    Some(resampler) => match resampler.resample(&decoded) {
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

        self.local_elapsed = 0;
        let time_base = self.decoder.codec_params().time_base;
        loop {
            match rx_request.try_recv() {
                Ok(request) => {
                    if self.handle_request(request) {
                        *elapsed.write().unwrap() = 0;
                        break;
                    }
                }
                // the player went out of scope (due to an error or a Ctrl+C)
                Err(TryRecvError::Disconnected) => return Ok(()),
                _ => (),
            }
            match self.demuxer.next_packet() {
                Ok(packet) if packet.track_id() == self.track_id => {
                    match self.decoder.decode(&packet) {
                        Ok(decoded) => {
                            let spec = decoded.spec();
                            let duration = decoded.capacity() as u64;
                            for (proxy, resampler) in self.device_proxies.iter_mut() {
                                if resampler.is_none() && proxy.sample_rate != spec.rate {
                                    // TODO: option to change playback speed
                                    *resampler =
                                        Some(Resampler::new(*spec, proxy.sample_rate, duration));
                                }
                            }
                            send_decoded_packet(&mut self.device_proxies, decoded, {
                                *volume.read().unwrap()
                            });
                            if let Some(time_base) = time_base {
                                let new_elapsed = time_base.calc_time(packet.ts).seconds;
                                if new_elapsed != self.local_elapsed {
                                    *elapsed.write().unwrap() = new_elapsed;
                                    self.local_elapsed = new_elapsed;
                                }
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
                    SymphoniaError::ResetRequired => self.decoder.reset(),
                    SymphoniaError::IoError(e)
                        if matches!(e.kind(), io::ErrorKind::UnexpectedEof) =>
                    {
                        // the entire song has been processed
                        self.stop();
                        *elapsed.write().unwrap() = 0;
                        break;
                    }
                    _ => bail!(e),
                },
                _ => (),
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
        (0.07 * (v as BaseSample)).exp() / 1000.0
    }
}
