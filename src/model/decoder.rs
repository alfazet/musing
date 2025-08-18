use anyhow::{Result, anyhow, bail};
use crossbeam_channel::{self as cbeam_chan, RecvTimeoutError, TryRecvError};
use std::{
    io, mem,
    sync::{Arc, RwLock},
};
use symphonia::core::{
    audio::SampleBuffer,
    codecs::{Decoder as SymphoniaDecoder, DecoderOptions as SymphoniaDecoderOptions},
    conv::ConvertibleSample,
    errors::Error as SymphoniaError,
    formats::{FormatOptions, FormatReader, SeekMode, SeekTo, Track},
    io::MediaSourceStream,
    meta::{self, Metadata, MetadataOptions, MetadataRevision},
    probe::{Hint, ProbeResult, ProbedMetadata},
    units::Time,
};
use tokio::{
    sync::mpsc::{self as tokio_chan},
    task,
};

use crate::model::song::{Song, SongEvent};

pub type BaseSample = f64;

const BASE_SAMPLE_MIN: BaseSample = -1.0;
const BASE_SAMPLE_MAX: BaseSample = 1.0;
const MAX_VOLUME: u8 = 100;
const CHUNK_SIZE: usize = 512;

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

#[derive(Debug)]
pub enum DecoderRequest {
    Seek(Seek),
}

pub struct Decoder {
    demuxer: Box<dyn FormatReader>,
    decoder: Box<dyn SymphoniaDecoder>,
    track_id: u32,
}

impl Decoder {
    pub fn try_new(song: &Song, gapless: bool) -> Result<Self> {
        let demuxer = song.demuxer(gapless)?;
        let track = demuxer.default_track().ok_or(anyhow!(
            "no audio track found in `{}`",
            &song.path.to_string_lossy()
        ))?;
        let track_id = track.id;
        let decoder_opts: SymphoniaDecoderOptions = Default::default();
        let decoder = symphonia::default::get_codecs().make(&track.codec_params, &decoder_opts)?;

        Ok(Self {
            demuxer,
            decoder,
            track_id,
        })
    }

    pub fn run(
        &mut self,
        txs_sample_chunk: Vec<cbeam_chan::Sender<Vec<BaseSample>>>,
        tx_event: tokio_chan::UnboundedSender<SongEvent>,
        rx_request: cbeam_chan::Receiver<DecoderRequest>,
        volume: Arc<RwLock<Volume>>,
        elapsed: Arc<RwLock<u64>>,
    ) -> Result<()> {
        let send_chunk = |chunk: Vec<BaseSample>| -> bool {
            let v = { *volume.read().unwrap() };
            let mult = decoder_utils::volume_to_mult(v);
            let to_send: Vec<_> = chunk
                .iter()
                .map(|s| (s * mult).clamp(BASE_SAMPLE_MIN, BASE_SAMPLE_MAX))
                .collect();
            for tx in txs_sample_chunk.iter() {
                if tx.send(to_send.clone()).is_err() {
                    // receiver(s) went out of scope => playback of this song had been stopped
                    return false;
                }
            }

            true
        };

        let time_base = self.decoder.codec_params().time_base;
        let mut prev_elapsed: u64 = 0;
        let mut chunk = Vec::new();
        loop {
            match rx_request.try_recv() {
                Ok(request) => match request {
                    DecoderRequest::Seek(seek) => {
                        let target_elapsed = match seek {
                            Seek::Forwards(secs) => prev_elapsed.saturating_add(secs),
                            Seek::Backwards(secs) => prev_elapsed.saturating_sub(secs),
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
                },
                // the player went out of scope (due to an error or a Ctrl+C)
                Err(TryRecvError::Disconnected) => return Ok(()),
                _ => (),
            }
            match self.demuxer.next_packet() {
                Ok(packet) if packet.track_id() == self.track_id => {
                    match self.decoder.decode(&packet) {
                        Ok(decoded) => {
                            let mut buf =
                                SampleBuffer::new(decoded.frames() as u64, *decoded.spec());
                            buf.copy_interleaved_ref(decoded);
                            chunk.extend_from_slice(buf.samples());
                            if chunk.len() >= CHUNK_SIZE && !send_chunk(mem::take(&mut chunk)) {
                                return Ok(());
                            }
                            if let Some(time_base) = time_base {
                                let new_elapsed = time_base.calc_time(packet.ts).seconds;
                                if new_elapsed != prev_elapsed {
                                    *elapsed.write().unwrap() = new_elapsed;
                                    prev_elapsed = new_elapsed;
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
                        // send any leftovers
                        if !chunk.is_empty() {
                            let _ = send_chunk(chunk);
                        }
                        // the entire song has been processed
                        break;
                    }
                    _ => bail!(e),
                },
                _ => (),
            }
        }
        let _ = tx_event.send(SongEvent::Over);

        Ok(())
    }

    pub fn sample_rate(&self) -> Option<u32> {
        self.decoder.codec_params().sample_rate
    }
}

mod decoder_utils {
    use super::*;

    // non-linear volume slider
    // source: https://www.dr-lex.be/info-stuff/volumecontrols.html
    pub fn volume_to_mult(v: Volume) -> f64 {
        let v: u8 = v.into();
        (0.07 * (v as f64)).exp() / 1000.0
    }
}
