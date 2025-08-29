use anyhow::{Result, bail};
use lazy_static::lazy_static;
use std::{
    collections::HashMap,
    fmt::{self, Display, Formatter},
};
use symphonia::core::meta::StandardTagKey;

macro_rules! enum_stringify {
    ($variant:expr) => {{
        let s = format!("{:?}", $variant);
        s.split("::").last().unwrap().to_string()
    }};
}

static TAG_NAMES: [&str; 30] = [
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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TagKeyKind {
    String,
    Integer,
    OutOf, // e.g. track 3 out of 12, written in metadata as "3/12"
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TagKey {
    pub key: StandardTagKey,
    pub kind: TagKeyKind,
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

impl Display for TagKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", enum_stringify!(self.key).to_lowercase())
    }
}

impl TryFrom<&str> for TagKey {
    type Error = anyhow::Error;

    fn try_from(s: &str) -> Result<Self> {
        use StandardTagKey as STKey;

        let Some(key) = TAG_MAP.get(&s).cloned() else {
            bail!("invalid tag `{}`", s);
        };
        let kind = match key {
            STKey::Bpm => TagKeyKind::Integer,
            STKey::DiscNumber | STKey::MovementNumber | STKey::TrackNumber => TagKeyKind::OutOf,
            _ => TagKeyKind::String,
        };

        Ok(Self { key, kind })
    }
}

impl TryFrom<StandardTagKey> for TagKey {
    type Error = anyhow::Error;

    fn try_from(s_key: StandardTagKey) -> Result<Self> {
        Self::try_from(enum_stringify!(s_key).to_lowercase().as_str())
    }
}
