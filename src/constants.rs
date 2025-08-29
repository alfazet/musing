use lazy_static::lazy_static;
use std::collections::HashSet;

pub const DEFAULT_PORT: u16 = 2137;
pub const DEFAULT_MUSIC_DIR: &str = ".";
pub const DEFAULT_PLAYLIST_DIR: &str = "playlists";
pub const DEFAULT_LOG_FILE: &str = "musing.log";
pub const DEFAULT_STATE_FILE: &str = "musing.state";
pub const DEFAULT_CONFIG_FILE: &str = "musing.toml";
pub const DEFAULT_CONFIG_DIR: &str = "musing";
pub const DEFAULT_IGNORE_FILE: &str = ".musingignore";
pub const UNKNOWN_DEVICE: &str = "[unknown]";

lazy_static! {
    pub static ref DEFAULT_ALLOWED_EXTS: HashSet<String> = HashSet::from([
        "aac".into(),
        "aif".into(),
        "aifc".into(),
        "aiff".into(),
        "flac".into(),
        "m4a".into(),
        "mp3".into(),
        "oga".into(),
        "ogg".into(),
        "wav".into(),
    ]);
    pub static ref DEFAULT_PLAYLIST_EXTS: HashSet<String> =
        HashSet::from(["m3u".into(), "m3u8".into()]);
}
