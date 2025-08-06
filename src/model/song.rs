use anyhow::{Result, anyhow};
use std::{
    collections::HashMap,
    fs::File,
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

use crate::{error::MyError, model::tag_key::TagKey, utils};

#[derive(Default)]
pub struct SongMeta {
    data: HashMap<TagKey, String>,
    // TODO: cover_art: (),
}

#[derive(Debug)]
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
        let mut probe_res = get_probe_result(path)?;
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
        let track = format_reader.default_track().ok_or(MyError::File(format!(
            "No audio track found in `{}`",
            path.to_string_lossy()
        )))?;
        let audio_meta = AudioMeta::from_track(track);
        let song = Self {
            path: path.to_path_buf(),
            song_meta,
            audio_meta,
        };

        Ok(song)
    }
}

fn get_probe_result(path: &Path) -> Result<ProbeResult> {
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
