use anyhow::Result;
use std::{
    collections::HashMap,
    fs::File,
    path::{Path, PathBuf},
};
use symphonia::core::{
    formats::{FormatOptions, FormatReader},
    io::MediaSourceStream,
    meta::{MetadataOptions, MetadataRevision},
    probe::{Hint, ProbeResult},
};

use crate::model::tag_key::TagKey;

#[derive(Clone, Default)]
pub struct Metadata {
    data: HashMap<TagKey, String>,
}

#[derive(Clone)]
pub struct Song {
    pub path: PathBuf, // absolute path
    pub metadata: Metadata,
}

#[derive(Debug)]
pub enum SongEvent {
    Over,
}

impl From<&MetadataRevision> for Metadata {
    fn from(revision: &MetadataRevision) -> Self {
        let mut data = HashMap::new();
        for tag in revision.tags() {
            if let Some(tag_key) = tag.std_key.and_then(|key| TagKey::try_from(key).ok()) {
                data.entry(tag_key).or_insert_with(|| tag.value.to_string());
            }
        }

        Self { data }
    }
}

impl Metadata {
    pub fn get(&self, tag: &TagKey) -> Option<&str> {
        self.data.get(tag).map(|s| s.as_str())
    }

    pub fn merge(self, other: Metadata) -> Self {
        Self {
            data: self.data.into_iter().chain(other.data).collect(),
        }
    }
}

impl Song {
    pub fn try_new(path: impl AsRef<Path> + Into<PathBuf>) -> Result<Self> {
        let mut probe_res = song_utils::get_probe_result(&path, false)?;
        let metadata_container = probe_res
            .format
            .metadata()
            .current()
            .map(Metadata::from)
            .unwrap_or_default();
        let metadata_probe = probe_res
            .metadata
            .get()
            .map(|m| m.current().map(Metadata::from).unwrap_or_default())
            .unwrap_or_default();
        let song = Self {
            path: path.into(),
            metadata: metadata_container.merge(metadata_probe),
        };

        Ok(song)
    }
}

pub fn demuxer(path: impl AsRef<Path>, gapless: bool) -> Result<Box<dyn FormatReader>> {
    let probe_res = song_utils::get_probe_result(path, gapless)?;
    Ok(probe_res.format)
}

mod song_utils {
    use super::*;

    pub fn get_probe_result(path: impl AsRef<Path>, enable_gapless: bool) -> Result<ProbeResult> {
        let source = Box::new(File::open(path.as_ref())?);
        let mut hint = Hint::new();
        if let Some(ext) = path.as_ref().extension()
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
