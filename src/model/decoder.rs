use anyhow::{Result, anyhow, bail};
use crossbeam_channel::{self as cbeam_chan, RecvTimeoutError, TryRecvError};
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
use tokio::{
    sync::mpsc::{self as tokio_chan},
    task,
};

use crate::model::song::{Song, SongEvent};

pub type BaseSample = f64;

const CHUNK_SIZE: usize = 512;

#[derive(Debug)]
pub enum SeekDirection {
    Forward,
    Backward,
}

#[derive(Debug)]
pub enum DecoderRequest {
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
        tx_sample_chunk: cbeam_chan::Sender<Vec<BaseSample>>,
        tx_event: tokio_chan::UnboundedSender<SongEvent>,
        rx_request: cbeam_chan::Receiver<DecoderRequest>,
    ) -> Result<()> {
        let mut chunk = Vec::new();
        loop {
            match rx_request.try_recv() {
                Ok(request) => log::warn!("{:?}", request),
                Err(TryRecvError::Disconnected) => break Ok(()),
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
                            if chunk.len() >= CHUNK_SIZE {
                                // TODO: send the chunk of samples together
                                // with its timestamp
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
