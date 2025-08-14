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
    formats::{FormatOptions, FormatReader, Track},
    io::MediaSourceStream,
    meta::{self, MetadataOptions, MetadataRevision},
    probe::{Hint, ProbeResult, ProbedMetadata},
};
use tokio::sync::mpsc;

use crate::model::tag_key::TagKey;

// TODO: see if a HashMap<TagKey, Vec<u8>> is possible
// if it is, then we can keep the cover_art in the
// same hashmap for convenience
#[derive(Default)]
pub struct Metadata {
    data: HashMap<TagKey, String>,
}

pub struct Song {
    pub path: PathBuf, // absolute path
    pub metadata: Metadata,
}

impl From<&MetadataRevision> for Metadata {
    fn from(revision: &MetadataRevision) -> Self {
        let mut data = HashMap::new();
        for tag in revision.tags() {
            if let Some(tag_key) = tag.std_key.and_then(|key| TagKey::try_from(key).ok()) {
                data.insert(tag_key, tag.value.to_string());
            }
        }

        Self { data }
    }
}

impl Metadata {
    pub fn get(&self, tag: &TagKey) -> Option<&str> {
        self.data.get(tag).map(|s| s.as_str())
    }
}

impl TryFrom<&Path> for Song {
    type Error = anyhow::Error;

    fn try_from(path: &Path) -> Result<Self> {
        let mut probe_res = song_utils::get_probe_result(path, false)?;
        let metadata = probe_res
            .metadata
            .get()
            .map(|mut metadata| {
                metadata
                    .skip_to_latest()
                    .map(Metadata::from)
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        let song = Self {
            path: path.to_path_buf(),
            metadata,
        };

        Ok(song)
    }
}

impl Song {
    pub fn demuxer(&self, gapless: bool) -> Result<Box<dyn FormatReader>> {
        let probe_res = song_utils::get_probe_result(&self.path, gapless)?;
        Ok(probe_res.format)
    }
}

mod song_utils {
    use super::*;

    pub fn get_probe_result(path: &Path, enable_gapless: bool) -> Result<ProbeResult> {
        let source = Box::new(File::open(path)?);
        let mut hint = Hint::new();
        if let Some(ext) = path.extension()
            && let Some(ext) = ext.to_str()
        {
            hint.with_extension(ext);
        }
        let mss = MediaSourceStream::new(source, Default::default());
        let format_opts = FormatOptions {
            enable_gapless,
            ..Default::default()
        };
        let metadata_opts: MetadataOptions = Default::default();
        let probe_res =
            symphonia::default::get_probe().format(&hint, mss, &format_opts, &metadata_opts)?;

        Ok(probe_res)
    }
}
