use std::collections::BTreeMap;

use crate::model::song::Song;

struct Entry {
    pub song: Song,
    pub id: u32,
}

pub struct Playlist {
    list: BTreeMap<u32, Entry>,
}
