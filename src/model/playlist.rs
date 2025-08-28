use anyhow::Result;
use std::{
    fs::File,
    io::{BufReader, prelude::*},
    path::{Path, PathBuf},
};

pub struct Playlist(Vec<PathBuf>);

impl Playlist {
    pub fn try_new(path: impl AsRef<Path>) -> Result<Self> {
        let file = File::open(&path)?;
        let stream = BufReader::new(file);
        // lines starting with `#` are comments in m3u files
        let songs: Vec<_> = stream
            .lines()
            .map_while(Result::ok)
            .filter(|l| !l.starts_with("#"))
            .map(|l| l.into())
            .collect();

        Ok(Self(songs))
    }

    pub fn inner(&self) -> &[PathBuf] {
        &self.0
    }

    pub fn append(&mut self, path: impl Into<PathBuf>) {
        self.0.push(path.into());
    }
}
