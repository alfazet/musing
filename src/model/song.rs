use anyhow::{Result, bail};
use std::{
    collections::HashMap,
    fs::File,
    path::{Path, PathBuf},
};
use symphonia::core::{
    formats::{FormatOptions, FormatReader},
    io::MediaSourceStream,
    meta::{MetadataOptions, MetadataRevision, Visual},
    probe::{Hint, ProbeResult},
};

use crate::model::tag_key::TagKey;

#[derive(Default)]
pub struct Metadata {
    data: HashMap<TagKey, String>,
}

pub struct Song {
    pub path: PathBuf, // absolute path
    pub metadata: Metadata,
}

pub struct SongProxy {
    pub path: PathBuf, // absolute path
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

impl TryFrom<&Path> for Song {
    type Error = anyhow::Error;

    fn try_from(path: &Path) -> Result<Self> {
        let mut probe_res = song_utils::get_probe_result(path, false)?;
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
            path: path.to_path_buf(),
            metadata: metadata_container.merge(metadata_probe),
        };

        Ok(song)
    }
}

impl Song {
    // TODO: base64 encode
    pub fn get_cover_art(&self) -> Option<Vec<u8>> {
        None
    }
}

impl From<&Song> for SongProxy {
    fn from(song: &Song) -> SongProxy {
        Self {
            path: song.path.clone(),
        }
    }
}

impl SongProxy {
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
