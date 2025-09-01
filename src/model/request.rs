use anyhow::{Result, anyhow, bail};
use serde_json::Value;
use std::path::PathBuf;
use tokio::sync::oneshot;

use crate::model::{
    comparator::Comparator,
    filter::{Filter, FilterExpr},
    response::{JsonObject, Response},
    tag_key::TagKey,
};

pub struct LsArgs(pub PathBuf);
pub struct MetadataArgs(pub Vec<PathBuf>, pub Vec<TagKey>);
pub struct SelectArgs(pub FilterExpr, pub Vec<Comparator>);
pub struct UniqueArgs(pub TagKey, pub FilterExpr, pub Vec<TagKey>);
pub enum DbRequestKind {
    Ls(LsArgs),
    Metadata(MetadataArgs),
    Select(SelectArgs),
    Unique(UniqueArgs),
    Update,
}

pub struct DisableArgs(pub String);
pub struct EnableArgs(pub String);
pub enum DeviceRequestKind {
    Disable(DisableArgs),
    Enable(EnableArgs),
}

pub struct ChangeVolumeArgs(pub i8);
pub struct SeekArgs(pub i64); // in seconds
pub struct SetVolumeArgs(pub u8);
pub struct SpeedArgs(pub u16);
pub enum PlaybackRequestKind {
    ChangeVolume(ChangeVolumeArgs),
    Gapless,
    Pause,
    Resume,
    Seek(SeekArgs),
    SetVolume(SetVolumeArgs),
    Speed(SpeedArgs),
    Stop,
    Toggle,
}

pub struct AddToPlaylistArgs(pub PathBuf, pub PathBuf); // playlist, song
pub struct ListSongsArgs(pub PathBuf);
// playlist, range (inclusive), position
pub struct LoadArgs(pub PathBuf, pub Option<(usize, usize)>, pub Option<usize>);
pub struct RemoveFromPlaylistArgs(pub PathBuf, pub usize); // playlist, position
pub struct SaveArgs(pub PathBuf);
pub enum PlaylistRequestKind {
    AddToPlaylist(AddToPlaylistArgs),
    ListSongs(ListSongsArgs),
    Load(LoadArgs),
    RemoveFromPlaylist(RemoveFromPlaylistArgs),
    Save(SaveArgs),
}

pub struct AddToQueueArgs(pub Vec<PathBuf>, pub Option<usize>); // relative or absolute paths
pub struct PlayArgs(pub u32); // queue id
pub struct RemoveFromQueueArgs(pub Vec<u32>); // queue ids
pub enum QueueRequestKind {
    AddToQueue(AddToQueueArgs),
    Clear,
    Next,
    Play(PlayArgs),
    Previous,
    Random,
    RemoveFromQueue(RemoveFromQueueArgs),
    Sequential,
    Single,
}

pub enum RequestKind {
    Db(DbRequestKind),
    Device(DeviceRequestKind),
    Playback(PlaybackRequestKind),
    Playlist(PlaylistRequestKind),
    Queue(QueueRequestKind),
    State,
}

pub struct Request {
    pub kind: RequestKind,
    pub tx_response: oneshot::Sender<Response>,
}

impl TryFrom<&mut JsonObject> for LsArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut JsonObject) -> Result<Self> {
        let dir: PathBuf =
            serde_json::from_value(args.remove("dir").ok_or(anyhow!("key `dir` not found"))?)?;
        Ok(Self(dir))
    }
}

impl TryFrom<&mut JsonObject> for MetadataArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut JsonObject) -> Result<Self> {
        let paths: Vec<PathBuf> = serde_json::from_value(
            args.remove("paths")
                .ok_or(anyhow!("key `paths` not found"))?,
        )?;
        let tags: Vec<TagKey> = serde_json::from_value::<Vec<String>>(
            args.remove("tags").ok_or(anyhow!("key `tags` not found"))?,
        )?
        .into_iter()
        .map(|s| TagKey::try_from(s.as_str()))
        .collect::<Result<_>>()?;

        Ok(Self(paths, tags))
    }
}

impl TryFrom<&mut JsonObject> for SelectArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut JsonObject) -> Result<Self> {
        let filters: Vec<Box<dyn Filter>> =
            serde_json::from_value::<Vec<Value>>(args.remove("filters").unwrap_or_default())?
                .into_iter()
                .map(|v| v.try_into())
                .collect::<Result<_>>()?;

        let comparators: Vec<Comparator> =
            serde_json::from_value::<Vec<Value>>(args.remove("comparators").unwrap_or_default())?
                .into_iter()
                .map(|v| v.try_into())
                .collect::<Result<_>>()?;

        Ok(Self(FilterExpr(filters), comparators))
    }
}

impl TryFrom<&mut JsonObject> for UniqueArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut JsonObject) -> Result<Self> {
        let tag: TagKey = serde_json::from_value::<String>(
            args.remove("tag").ok_or(anyhow!("key `tag` not found"))?,
        )?
        .as_str()
        .try_into()?;

        let filters: Vec<Box<dyn Filter>> =
            serde_json::from_value::<Vec<Value>>(args.remove("filters").unwrap_or_default())?
                .into_iter()
                .map(|v| v.try_into())
                .collect::<Result<_>>()?;

        let group_by: Vec<TagKey> =
            serde_json::from_value::<Vec<String>>(args.remove("group_by").unwrap_or_default())?
                .into_iter()
                .map(|s| TagKey::try_from(s.as_str()))
                .collect::<Result<_>>()?;

        Ok(Self(tag, FilterExpr(filters), group_by))
    }
}

impl TryFrom<&mut JsonObject> for DisableArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut JsonObject) -> Result<Self> {
        let device: String = serde_json::from_value(
            args.remove("device")
                .ok_or(anyhow!("key `device` not found"))?,
        )?;

        Ok(Self(device))
    }
}

impl TryFrom<&mut JsonObject> for EnableArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut JsonObject) -> Result<Self> {
        let device: String = serde_json::from_value(
            args.remove("device")
                .ok_or(anyhow!("key `device` not found"))?,
        )?;

        Ok(Self(device))
    }
}

impl TryFrom<&mut JsonObject> for SeekArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut JsonObject) -> Result<Self> {
        let seconds: i64 = serde_json::from_value(
            args.remove("seconds")
                .ok_or(anyhow!("key `seconds` not found"))?,
        )?;

        Ok(Self(seconds))
    }
}

impl TryFrom<&mut JsonObject> for ChangeVolumeArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut JsonObject) -> Result<Self> {
        let delta: i8 = serde_json::from_value(
            args.remove("delta")
                .ok_or(anyhow!("key `delta` not found"))?,
        )?;

        Ok(Self(delta))
    }
}

impl TryFrom<&mut JsonObject> for SetVolumeArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut JsonObject) -> Result<Self> {
        let volume: u8 = serde_json::from_value(
            args.remove("volume")
                .ok_or(anyhow!("key `volume` not found"))?,
        )?;

        Ok(Self(volume))
    }
}

impl TryFrom<&mut JsonObject> for SpeedArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut JsonObject) -> Result<Self> {
        let speed: u16 = serde_json::from_value(
            args.remove("speed")
                .ok_or(anyhow!("key `speed` not found"))?,
        )?;

        Ok(Self(speed))
    }
}

impl TryFrom<&mut JsonObject> for AddToPlaylistArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut JsonObject) -> Result<Self> {
        let playlist: PathBuf = serde_json::from_value(
            args.remove("playlist")
                .ok_or(anyhow!("key `playlist` not found"))?,
        )?;
        let song: PathBuf =
            serde_json::from_value(args.remove("song").ok_or(anyhow!("key `song` not found"))?)?;

        Ok(Self(playlist, song))
    }
}

impl TryFrom<&mut JsonObject> for ListSongsArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut JsonObject) -> Result<Self> {
        let playlist: PathBuf = serde_json::from_value(
            args.remove("playlist")
                .ok_or(anyhow!("key `playlist` not found"))?,
        )?;

        Ok(Self(playlist))
    }
}

impl TryFrom<&mut JsonObject> for LoadArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut JsonObject) -> Result<Self> {
        let playlist: PathBuf = serde_json::from_value(
            args.remove("playlist")
                .ok_or(anyhow!("key `playlist` not found"))?,
        )?;
        let range = args
            .remove("range")
            .map(serde_json::from_value)
            .transpose()?;
        let pos = args.remove("pos").map(serde_json::from_value).transpose()?;

        Ok(Self(playlist, range, pos))
    }
}

impl TryFrom<&mut JsonObject> for SaveArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut JsonObject) -> Result<Self> {
        let path: PathBuf =
            serde_json::from_value(args.remove("path").ok_or(anyhow!("key `path` not found"))?)?;

        Ok(Self(path))
    }
}

impl TryFrom<&mut JsonObject> for RemoveFromPlaylistArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut JsonObject) -> Result<Self> {
        let playlist: PathBuf = serde_json::from_value(
            args.remove("playlist")
                .ok_or(anyhow!("key `playlist` not found"))?,
        )?;
        let pos: usize =
            serde_json::from_value(args.remove("pos").ok_or(anyhow!("key `pos` not found"))?)?;

        Ok(Self(playlist, pos))
    }
}

impl TryFrom<&mut JsonObject> for AddToQueueArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut JsonObject) -> Result<Self> {
        let paths: Vec<PathBuf> = serde_json::from_value(
            args.remove("paths")
                .ok_or(anyhow!("key `paths` not found"))?,
        )?;
        let pos = args.remove("pos").map(serde_json::from_value).transpose()?;

        Ok(Self(paths, pos))
    }
}

impl TryFrom<&mut JsonObject> for PlayArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut JsonObject) -> Result<Self> {
        let id: u32 =
            serde_json::from_value(args.remove("id").ok_or(anyhow!("key `id` not found"))?)?;

        Ok(Self(id))
    }
}

impl TryFrom<&mut JsonObject> for RemoveFromQueueArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut JsonObject) -> Result<Self> {
        let ids: Vec<u32> =
            serde_json::from_value(args.remove("ids").ok_or(anyhow!("key `ids` not found"))?)?;

        Ok(Self(ids))
    }
}

impl TryFrom<&str> for RequestKind {
    type Error = anyhow::Error;

    fn try_from(s: &str) -> Result<Self> {
        use DbRequestKind as Db;
        use DeviceRequestKind as Device;
        use PlaybackRequestKind as Playback;
        use PlaylistRequestKind as Playlist;
        use QueueRequestKind as Queue;

        let mut temp = serde_json::from_str::<Value>(s)?;
        let map = temp
            .as_object_mut()
            .ok_or(anyhow!("a request must be a JSON object"))?;
        let kind: String =
            serde_json::from_value(map.remove("kind").ok_or(anyhow!("key `kind` not found"))?)?;
        let kind = match kind.as_str() {
            "ls" => RequestKind::Db(Db::Ls(map.try_into()?)),
            "metadata" => RequestKind::Db(Db::Metadata(map.try_into()?)),
            "select" => RequestKind::Db(Db::Select(map.try_into()?)),
            "unique" => RequestKind::Db(Db::Unique(map.try_into()?)),
            "update" => RequestKind::Db(Db::Update),

            "disable" => RequestKind::Device(Device::Disable(map.try_into()?)),
            "enable" => RequestKind::Device(Device::Enable(map.try_into()?)),
            "changevol" => RequestKind::Playback(Playback::ChangeVolume(map.try_into()?)),
            "gapless" => RequestKind::Playback(Playback::Gapless),
            "pause" => RequestKind::Playback(Playback::Pause),
            "resume" => RequestKind::Playback(Playback::Resume),
            "seek" => RequestKind::Playback(Playback::Seek(map.try_into()?)),
            "setvol" => RequestKind::Playback(Playback::SetVolume(map.try_into()?)),
            "speed" => RequestKind::Playback(Playback::Speed(map.try_into()?)),
            "stop" => RequestKind::Playback(Playback::Stop),
            "toggle" => RequestKind::Playback(Playback::Toggle),

            "addplaylist" => RequestKind::Playlist(Playlist::AddToPlaylist(map.try_into()?)),
            "listsongs" => RequestKind::Playlist(Playlist::ListSongs(map.try_into()?)),
            "load" => RequestKind::Playlist(Playlist::Load(map.try_into()?)),
            "removeplaylist" => {
                RequestKind::Playlist(Playlist::RemoveFromPlaylist(map.try_into()?))
            }
            "save" => RequestKind::Playlist(Playlist::Save(map.try_into()?)),

            "addqueue" => RequestKind::Queue(Queue::AddToQueue(map.try_into()?)),
            "clear" => RequestKind::Queue(Queue::Clear),
            "next" => RequestKind::Queue(Queue::Next),
            "play" => RequestKind::Queue(Queue::Play(map.try_into()?)),
            "previous" => RequestKind::Queue(Queue::Previous),
            "random" => RequestKind::Queue(Queue::Random),
            "removequeue" => RequestKind::Queue(Queue::RemoveFromQueue(map.try_into()?)),
            "sequential" => RequestKind::Queue(Queue::Sequential),
            "single" => RequestKind::Queue(Queue::Single),

            "state" => RequestKind::State,

            other => bail!("invalid value of key `kind`: `{}`", other),
        };

        Ok(kind)
    }
}
