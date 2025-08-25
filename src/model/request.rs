use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value};
use tokio::sync::oneshot;

use crate::model::{
    comparator::Comparator,
    filter::{Filter, FilterExpr},
    response::Response,
    tag_key::TagKey,
};

pub struct MetadataArgs(pub Vec<u32>, pub Vec<TagKey>);
pub struct SelectArgs(pub FilterExpr, pub Vec<Comparator>);
pub struct UniqueArgs(pub TagKey, pub FilterExpr, pub Vec<TagKey>);
pub enum DbRequestKind {
    Metadata(MetadataArgs),
    Reset,
    Select(SelectArgs),
    Unique(UniqueArgs),
    Update,
}

pub struct DisableArgs(pub String);
pub struct EnableArgs(pub String);
pub enum DeviceRequestKind {
    Disable(DisableArgs),
    Enable(EnableArgs),
    ListDevices,
}

pub struct SeekArgs(pub i64); // in seconds
pub struct SetVolumeArgs(pub u8);
pub struct ChangeVolumeArgs(pub i8);
pub enum PlaybackRequestKind {
    ChangeVolume(ChangeVolumeArgs),
    Gapless,
    Pause,
    Resume,
    Seek(SeekArgs),
    SetVolume(SetVolumeArgs),
    Stop,
    Toggle,
}

pub struct AddArgs(pub Vec<u32>, pub Option<usize>); // db ids
pub struct PlayArgs(pub u32); // queue id
pub struct RemoveArgs(pub Vec<u32>); // queue ids
pub enum QueueRequestKind {
    Add(AddArgs),
    Clear,
    Next,
    Play(PlayArgs),
    Previous,
    Random,
    Remove(RemoveArgs),
    Sequential,
    Single,
}

pub enum StatusRequestKind {
    Current,
    Elapsed,
    Queue,
    State,
    Volume,
}

pub enum RequestKind {
    Db(DbRequestKind),
    Device(DeviceRequestKind),
    Playback(PlaybackRequestKind),
    Queue(QueueRequestKind),
    Status(StatusRequestKind),
}

pub struct Request {
    pub kind: RequestKind,
    pub tx_response: oneshot::Sender<Response>,
}

impl TryFrom<&mut Map<String, Value>> for MetadataArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut Map<String, Value>) -> Result<Self> {
        let ids: Vec<u32> =
            serde_json::from_value(args.remove("ids").ok_or(anyhow!("key `ids` not found"))?)?;
        let tags: Vec<TagKey> = serde_json::from_value::<Vec<String>>(
            args.remove("tags").ok_or(anyhow!("key `tags` not found"))?,
        )?
        .into_iter()
        .map(|s| TagKey::try_from(s.as_str()))
        .collect::<Result<_>>()?;

        Ok(Self(ids, tags))
    }
}

impl TryFrom<&mut Map<String, Value>> for SelectArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut Map<String, Value>) -> Result<Self> {
        let filters: Vec<Box<dyn Filter>> =
            serde_json::from_value::<Vec<Value>>(args.remove("filters").unwrap_or_default())?
                .into_iter()
                .map(|mut v| match v.as_object_mut() {
                    Some(v) => v.try_into(),
                    None => Err(anyhow!("`filters` must be an array of JSON maps")),
                })
                .collect::<Result<_>>()?;

        let comparators: Vec<Comparator> =
            serde_json::from_value::<Vec<Value>>(args.remove("comparators").unwrap_or_default())?
                .into_iter()
                .map(|mut v| match v.as_object_mut() {
                    Some(v) => v.try_into(),
                    None => Err(anyhow!("`comparators` must be an array of JSON maps")),
                })
                .collect::<Result<_>>()?;

        Ok(Self(FilterExpr(filters), comparators))
    }
}

impl TryFrom<&mut Map<String, Value>> for UniqueArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut Map<String, Value>) -> Result<Self> {
        let tag: TagKey = serde_json::from_value::<String>(
            args.remove("tag").ok_or(anyhow!("key `tag` not found"))?,
        )?
        .as_str()
        .try_into()?;

        let filters: Vec<Box<dyn Filter>> =
            serde_json::from_value::<Vec<Value>>(args.remove("filters").unwrap_or_default())?
                .into_iter()
                .map(|mut v| match v.as_object_mut() {
                    Some(v) => v.try_into(),
                    // TODO: get rid of these error messages (return the errors from the functions
                    // that can error out)
                    None => Err(anyhow!("`filters` must be an array of JSON maps")),
                })
                .collect::<Result<_>>()?;

        let group_by: Vec<TagKey> =
            serde_json::from_value::<Vec<String>>(args.remove("group_by").unwrap_or_default())?
                .into_iter()
                .map(|s| TagKey::try_from(s.as_str()))
                .collect::<Result<_>>()?;

        Ok(Self(tag, FilterExpr(filters), group_by))
    }
}

impl TryFrom<&mut Map<String, Value>> for DisableArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut Map<String, Value>) -> Result<Self> {
        let device: String = serde_json::from_value(
            args.remove("device")
                .ok_or(anyhow!("key `device` not found"))?,
        )?;

        Ok(Self(device))
    }
}

impl TryFrom<&mut Map<String, Value>> for EnableArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut Map<String, Value>) -> Result<Self> {
        let device: String = serde_json::from_value(
            args.remove("device")
                .ok_or(anyhow!("key `device` not found"))?,
        )?;

        Ok(Self(device))
    }
}

impl TryFrom<&mut Map<String, Value>> for SeekArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut Map<String, Value>) -> Result<Self> {
        let seconds: i64 = serde_json::from_value(
            args.remove("seconds")
                .ok_or(anyhow!("key `seconds` not found"))?,
        )?;

        Ok(Self(seconds))
    }
}

impl TryFrom<&mut Map<String, Value>> for ChangeVolumeArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut Map<String, Value>) -> Result<Self> {
        let delta: i8 = serde_json::from_value(
            args.remove("delta")
                .ok_or(anyhow!("key `delta` not found"))?,
        )?;

        Ok(Self(delta))
    }
}

impl TryFrom<&mut Map<String, Value>> for SetVolumeArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut Map<String, Value>) -> Result<Self> {
        let volume: u8 = serde_json::from_value(
            args.remove("volume")
                .ok_or(anyhow!("key `volume` not found"))?,
        )?;

        Ok(Self(volume))
    }
}

impl TryFrom<&mut Map<String, Value>> for AddArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut Map<String, Value>) -> Result<Self> {
        let ids: Vec<u32> =
            serde_json::from_value(args.remove("ids").ok_or(anyhow!("key `ids` not found"))?)?;
        let pos = args.remove("pos").map(serde_json::from_value).transpose()?;

        Ok(Self(ids, pos))
    }
}

impl TryFrom<&mut Map<String, Value>> for PlayArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut Map<String, Value>) -> Result<Self> {
        let id: u32 =
            serde_json::from_value(args.remove("id").ok_or(anyhow!("key `ids` not found"))?)?;

        Ok(Self(id))
    }
}

impl TryFrom<&mut Map<String, Value>> for RemoveArgs {
    type Error = anyhow::Error;

    fn try_from(args: &mut Map<String, Value>) -> Result<Self> {
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
        use QueueRequestKind as Queue;
        use StatusRequestKind as Status;

        let mut temp = serde_json::from_str::<Value>(s)?;
        let map = temp
            .as_object_mut()
            .ok_or(anyhow!("the request must be a JSON map"))?;
        let kind: String =
            serde_json::from_value(map.remove("kind").ok_or(anyhow!("key `kind` not found"))?)?;
        let kind = match kind.as_str() {
            "metadata" => RequestKind::Db(Db::Metadata(map.try_into()?)),
            "reset" => RequestKind::Db(Db::Reset),
            "select" => RequestKind::Db(Db::Select(map.try_into()?)),
            "unique" => RequestKind::Db(Db::Unique(map.try_into()?)),
            "update" => RequestKind::Db(Db::Update),

            "disable" => RequestKind::Device(Device::Disable(map.try_into()?)),
            "enable" => RequestKind::Device(Device::Enable(map.try_into()?)),
            "listdev" => RequestKind::Device(Device::ListDevices),
            "gapless" => RequestKind::Playback(Playback::Gapless),
            "pause" => RequestKind::Playback(Playback::Pause),
            "resume" => RequestKind::Playback(Playback::Resume),
            "seek" => RequestKind::Playback(Playback::Seek(map.try_into()?)),
            "stop" => RequestKind::Playback(Playback::Stop),
            "toggle" => RequestKind::Playback(Playback::Toggle),
            "setvol" => RequestKind::Playback(Playback::SetVolume(map.try_into()?)),
            "changevol" => RequestKind::Playback(Playback::ChangeVolume(map.try_into()?)),
            "add" => RequestKind::Queue(Queue::Add(map.try_into()?)),
            "clear" => RequestKind::Queue(Queue::Clear),
            "next" => RequestKind::Queue(Queue::Next),
            "play" => RequestKind::Queue(Queue::Play(map.try_into()?)),
            "previous" => RequestKind::Queue(Queue::Previous),
            "random" => RequestKind::Queue(Queue::Random),
            "remove" => RequestKind::Queue(Queue::Remove(map.try_into()?)),
            "sequential" => RequestKind::Queue(Queue::Sequential),
            "single" => RequestKind::Queue(Queue::Single),

            "current" => RequestKind::Status(Status::Current),
            "elapsed" => RequestKind::Status(Status::Elapsed),
            "queue" => RequestKind::Status(Status::Queue),
            "state" => RequestKind::Status(Status::State),
            "volume" => RequestKind::Status(Status::Volume),

            other => bail!("invalid value of key `kind`: `{}`", other),
        };

        Ok(kind)
    }
}
