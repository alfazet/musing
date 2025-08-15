use anyhow::{Result, anyhow, bail};
use std::mem;
use symphonia::core::{
    audio::SampleBuffer,
    codecs::{Decoder as SymphoniaDecoder, DecoderOptions as SymphoniaDecoderOptions},
    conv::ConvertibleSample,
    errors::Error as SymphoniaError,
    formats::{FormatOptions, FormatReader, Track},
    io::MediaSourceStream,
    meta::{self, Metadata, MetadataOptions, MetadataRevision},
    probe::{Hint, ProbeResult, ProbedMetadata},
};
use tokio::{sync::mpsc, task};

use crate::model::song::*;

pub type BaseSample = f64;
type ReceiverDecoderRequest = crossbeam_channel::Receiver<DecoderRequest>;
type SenderSampleChunk = crossbeam_channel::Sender<Vec<BaseSample>>;

const CHUNK_SIZE: usize = 512;

#[derive(Debug)]
pub enum SeekDirection {
    Forward,
    Backward,
}

#[derive(Debug)]
pub enum DecoderRequest {
    Elapsed,
    Seek(u64, SeekDirection),
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
        tx_sample_chunk: SenderSampleChunk,
        rx_request: ReceiverDecoderRequest,
    ) -> Result<()> {
        let mut chunk = Vec::new();
        loop {
            if let Ok(request) = rx_request.try_recv() {
                // handle seeking and elapsed (get TimeBase from decoder.codec_params())
                eprintln!("{:?}", request);
            }
            match self.demuxer.next_packet() {
                Ok(packet) if packet.track_id() == self.track_id => {
                    match self.decoder.decode(&packet) {
                        Ok(decoded) => {
                            let mut buf =
                                SampleBuffer::new(decoded.frames() as u64, *decoded.spec());
                            buf.copy_interleaved_ref(decoded);
                            chunk.extend_from_slice(buf.samples());
                            if chunk.len() >= CHUNK_SIZE {
                                if tx_sample_chunk.send(mem::take(&mut chunk)).is_err() {
                                    // receiver of sample chunks went out of scope =>
                                    // playback of this song ended
                                    return Ok(());
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
                    _ => bail!(e),
                },
                _ => (),
            }
        }
    }

    pub fn sample_rate(&self) -> Option<u32> {
        self.decoder.codec_params().sample_rate
    }
}
