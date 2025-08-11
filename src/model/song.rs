use anyhow::{Result, anyhow};
use crossbeam_channel::{RecvTimeoutError, TryRecvError};
use std::{
    collections::HashMap,
    fs::File,
    mem,
    path::{Path, PathBuf},
};
use symphonia::core::{
    audio::SampleBuffer,
    codecs::DecoderOptions,
    conv::ConvertibleSample,
    formats::{FormatOptions, Track},
    io::MediaSourceStream,
    meta::{self, Metadata, MetadataOptions, MetadataRevision},
    probe::{Hint, ProbeResult, ProbedMetadata},
};
use tokio::{sync::mpsc, task};

use crate::model::tag_key::TagKey;

pub type SenderSamples = crossbeam_channel::Sender<Vec<f32>>;
pub type ReceiverSamples = crossbeam_channel::Receiver<Vec<f32>>;

const BUF_SIZE: usize = 1024;

#[derive(Default)]
pub struct SongMeta {
    data: HashMap<TagKey, String>,
    // TODO: cover_art: (),
}

#[derive(Clone, Copy)]
pub struct AudioMeta {
    pub n_channels: Option<u16>,
    pub bit_depth: Option<u32>,
    pub sample_rate: Option<u32>,
    pub duration: Option<u64>, // in seconds
}

pub struct Song {
    pub path: PathBuf, // absolute
    pub song_meta: SongMeta,
    pub audio_meta: AudioMeta,
}

#[derive(Clone)]
pub struct PlayerSong {
    pub audio_meta: AudioMeta,
    pub rx_samples: ReceiverSamples,
}

impl SongMeta {
    pub fn from_revision(revision: &MetadataRevision) -> Self {
        let mut data = HashMap::new();
        for tag in revision.tags() {
            if let Some(tag_key) = tag.std_key.and_then(|key| TagKey::try_from(key).ok()) {
                data.insert(tag_key, tag.value.to_string());
            }
        }

        Self { data }
    }

    pub fn contains(&self, tag: &TagKey) -> bool {
        self.data.contains_key(tag)
    }

    pub fn get(&self, tag: &TagKey) -> Option<&str> {
        self.data.get(tag).map(|s| s.as_str())
    }
}

impl AudioMeta {
    pub fn from_track(track: &Track) -> Self {
        let codec_params = &track.codec_params;
        let n_channels = codec_params
            .channels
            .map(|channels| channels.count() as u16);
        let bit_depth = codec_params.bits_per_sample;
        let sample_rate = codec_params.sample_rate;
        let time_base = codec_params.time_base.unwrap_or_default();
        let duration = codec_params.n_frames.map(|n| {
            let duration = time_base.calc_time(n);
            duration.seconds + if duration.frac > 0.5 { 1 } else { 0 }
        });

        Self {
            n_channels,
            bit_depth,
            sample_rate,
            duration,
        }
    }
}

impl TryFrom<&Path> for Song {
    type Error = anyhow::Error;

    fn try_from(path: &Path) -> Result<Self> {
        let mut probe_res = song_utils::get_probe_result(path)?;
        let song_meta = probe_res
            .metadata
            .get()
            .map(|mut metadata| {
                metadata
                    .skip_to_latest()
                    .map(SongMeta::from_revision)
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        let format_reader = probe_res.format;
        let track = format_reader.default_track().ok_or(anyhow!(
            "no audio track found in `{}`",
            path.to_string_lossy()
        ))?;
        let audio_meta = AudioMeta::from_track(track);
        let song = Self {
            path: path.to_path_buf(),
            song_meta,
            audio_meta,
        };

        Ok(song)
    }
}

impl TryFrom<&Song> for PlayerSong {
    type Error = anyhow::Error;

    fn try_from(song: &Song) -> Result<Self> {
        let audio_meta = song.audio_meta;
        let rx_samples = song.spawn_sample_producer()?;

        Ok(Self {
            audio_meta,
            rx_samples,
        })
    }
}

impl Song {
    pub fn spawn_sample_producer(&self) -> Result<ReceiverSamples> {
        let mut format_reader = song_utils::get_probe_result(&self.path)?.format;
        let track = format_reader.default_track().ok_or(anyhow!(
            "no audio track found in `{}`",
            self.path.to_string_lossy()
        ))?;
        let decoder_opts: DecoderOptions = Default::default();
        let mut decoder =
            symphonia::default::get_codecs().make(&track.codec_params, &decoder_opts)?;
        let track_id = track.id;

        let (tx, rx) = crossbeam_channel::bounded(1);
        tokio::task::spawn_blocking(move || {
            let mut batch = Vec::new();
            while let Ok(packet) = format_reader.next_packet() {
                if packet.track_id() != track_id {
                    continue;
                }
                match decoder.decode(&packet) {
                    Ok(decoded) => {
                        if decoded.frames() > 0 {
                            let spec = *decoded.spec();
                            let mut buf = SampleBuffer::new(decoded.frames() as u64, spec);
                            buf.copy_interleaved_ref(decoded);
                            batch.extend_from_slice(buf.samples());
                            if batch.len() >= BUF_SIZE {
                                let to_send: Vec<_> = mem::take(&mut batch);
                                if tx.send(to_send).is_err() {
                                    // receiver went out of scope because playback had been stopped
                                    return;
                                }
                            }
                        }
                    }
                    Err(e) => log::warn!("decoding error ({})", e),
                }
            }
            if !batch.is_empty() {
                let _ = tx.send(batch);
            }
        });

        Ok(rx)
    }
}

mod song_utils {
    use super::*;

    pub fn get_probe_result(path: &Path) -> Result<ProbeResult> {
        let file = File::open(path)?;
        let source = Box::new(file);
        let mss = MediaSourceStream::new(source, Default::default());
        let format_opts: FormatOptions = Default::default();
        let metadata_opts: MetadataOptions = Default::default();
        let mut hint = Hint::new();
        if let Some(ext) = path.extension() {
            if let Some(ext) = ext.to_str() {
                hint.with_extension(ext);
            }
        }
        let probe_res =
            symphonia::default::get_probe().format(&hint, mss, &format_opts, &metadata_opts)?;

        Ok(probe_res)
    }
}
