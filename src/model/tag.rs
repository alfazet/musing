use anyhow::{Result, bail};
use lazy_static::lazy_static;
use std::{
    collections::HashMap,
    fmt::{self, Display, Formatter},
    str::FromStr,
};
use symphonia::core::meta::StandardTagKey;

use crate::{error::MyError, utils};

static TAG_NAMES: [&'static str; 30] = [
    "album",
    "albumartist",
    "arranger",
    "artist",
    "bpm",
    "composer",
    "conductor",
    "date",
    "discnumber",
    "disctotal",
    "ensemble",
    "genre",
    "label",
    "language",
    "lyricist",
    "mood",
    "movementname",
    "movementnumber",
    "part",
    "parttotal",
    "performer",
    "producer",
    "script",
    "sortalbum",
    "sortalbumartist",
    "sortartist",
    "sortcomposer",
    "sorttracktitle",
    "tracknumber",
    "tracktitle",
];
static TAG_KEYS: [StandardTagKey; 30] = [
    StandardTagKey::Album,
    StandardTagKey::AlbumArtist,
    StandardTagKey::Arranger,
    StandardTagKey::Artist,
    StandardTagKey::Bpm,
    StandardTagKey::Composer,
    StandardTagKey::Conductor,
    StandardTagKey::Date,
    StandardTagKey::DiscNumber,
    StandardTagKey::DiscTotal,
    StandardTagKey::Ensemble,
    StandardTagKey::Genre,
    StandardTagKey::Label,
    StandardTagKey::Language,
    StandardTagKey::Lyricist,
    StandardTagKey::Mood,
    StandardTagKey::MovementName,
    StandardTagKey::MovementNumber,
    StandardTagKey::Part,
    StandardTagKey::PartTotal,
    StandardTagKey::Performer,
    StandardTagKey::Producer,
    StandardTagKey::Script,
    StandardTagKey::SortAlbum,
    StandardTagKey::SortAlbumArtist,
    StandardTagKey::SortArtist,
    StandardTagKey::SortComposer,
    StandardTagKey::SortTrackTitle,
    StandardTagKey::TrackNumber,
    StandardTagKey::TrackTitle,
];

pub enum TagKind {
    String,
    Integer,
    OutOf, // e.g. track 3 out of 12, written in metadata as "3/12"
}

pub struct Tag {
    key: StandardTagKey,
    kind: TagKind,
}

lazy_static! {
    static ref TAG_MAP: HashMap<&'static str, StandardTagKey> = {
        TAG_NAMES
            .iter()
            .cloned()
            .zip(TAG_KEYS.iter().cloned())
            .collect()
    };
}

impl FromStr for Tag {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        use StandardTagKey::*;

        let Some(key) = TAG_MAP.get(s).cloned() else {
            bail!(MyError::Syntax("Invalid tag name".into()));
        };
        let kind = match key {
            Bpm => TagKind::Integer,
            DiscNumber | MovementNumber | TrackNumber => TagKind::OutOf,
            _ => TagKind::String,
        };

        Ok(Self { key, kind })
    }
}

impl TryFrom<StandardTagKey> for Tag {
    type Error = anyhow::Error;

    fn try_from(s_key: StandardTagKey) -> Result<Self> {
        use StandardTagKey::*;

        let key = match TAG_KEYS.iter().find(|key| &&s_key == key) {
            Some(key) => key.clone(),
            None => bail!(MyError::Syntax("Invalid tag key".into())),
        };
        let kind = match key {
            Bpm => TagKind::Integer,
            DiscNumber | MovementNumber | TrackNumber => TagKind::OutOf,
            _ => TagKind::String,
        };

        Ok(Self { key, kind })
    }
}
