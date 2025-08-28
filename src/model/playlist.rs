use anyhow::Result;
use std::{
    fs::File,
    io::{BufReader, prelude::*},
    path::{Path, PathBuf},
    rc::Rc,
};

pub struct Playlist(Rc<[PathBuf]>);

impl Playlist {
    pub fn try_new(path: impl AsRef<Path>) -> Result<Self> {
        let file = File::open(&path)?;
        let stream = BufReader::new(file);
        // lines starting with `#` are comments in m3u
        let songs: Vec<_> = stream
            .lines()
            .map_while(Result::ok)
            .filter(|l| !l.starts_with("#"))
            .map(|l| l.into())
            .collect();

        Ok(Self(Rc::from(songs)))
    }

    pub fn inner(&self) -> Rc<[PathBuf]> {
        Rc::clone(&self.0)
    }
}
