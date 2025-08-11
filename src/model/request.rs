use anyhow::{Result, anyhow, bail};
use tokio::sync::{mpsc::Sender, oneshot};

use crate::{
    model::{comparator::Comparator, filter::FilterExpr, response::Response, tag_key::TagKey},
    parsers::request,
};

type RespondTo = oneshot::Sender<Response>;

pub enum Volume {
    Change(i8),
    Set(u8),
}

pub struct MetadataArgs(pub Vec<u32>, pub Vec<TagKey>);
pub struct SelectArgs(pub FilterExpr, pub Vec<Comparator>);
pub struct UniqueArgs(pub TagKey, pub Vec<TagKey>, pub FilterExpr);
pub enum DbRequestKind {
    Metadata(MetadataArgs),
    Reset,
    Select(SelectArgs),
    Unique(UniqueArgs),
    Update,
}

pub struct SeekArgs(pub i32); // in seconds
pub struct VolumeArgs(pub Volume);
pub enum PlaybackRequestKind {
    Pause,
    Resume,
    Seek(SeekArgs),
    Stop,
    Toggle,
    Volume(VolumeArgs),
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
    Remove(RemoveArgs),
}

pub enum StatusRequestKind {
    Current,
    Elapsed,
    Queue,
    Volume,
}

pub enum RequestKind {
    Db(DbRequestKind),
    Playback(PlaybackRequestKind),
    Queue(QueueRequestKind),
    Status(StatusRequestKind),
}

pub struct Request {
    pub kind: RequestKind,
    pub tx_response: RespondTo,
}

impl TryFrom<&[String]> for MetadataArgs {
    type Error = anyhow::Error;

    fn try_from(args: &[String]) -> Result<Self> {
        if args.len() != 2 {
            bail!("invalid arguments to `metadata`");
        }
        let ids = args[0]
            .trim_end_matches(',')
            .split(',')
            .map(|s| s.parse::<u32>().map_err(|e| e.into()))
            .collect::<Result<Vec<u32>>>()?;
        let tags = args[1]
            .trim_end_matches(',')
            .split(',')
            .map(TagKey::try_from)
            .collect::<Result<Vec<TagKey>>>()?;

        Ok(Self(ids, tags))
    }
}

impl TryFrom<&[String]> for SelectArgs {
    type Error = anyhow::Error;

    fn try_from(args: &[String]) -> Result<Self> {
        let filter_expr = args.first().map_or_else(
            || Ok(FilterExpr::default()),
            |s| FilterExpr::try_from(s.as_str()),
        )?;
        let sort_by = args
            .get(1)
            .map(|v| {
                v.trim_end_matches(',')
                    .split(',')
                    .map(Comparator::try_from)
                    .collect::<Result<Vec<Comparator>>>()
            })
            .unwrap_or(Ok(Vec::new()))?;

        Ok(Self(filter_expr, sort_by))
    }
}

impl TryFrom<&[String]> for UniqueArgs {
    type Error = anyhow::Error;

    fn try_from(args: &[String]) -> Result<Self> {
        if args.is_empty() {
            bail!("invalid arguments to `unique`");
        }
        let tag = TagKey::try_from(args[0].as_str())?;
        let group_by = match args.get(1).map(|s| s.as_str()) {
            Some("groupby") => args
                .get(2)
                .ok_or(anyhow!("no tags provided to `groupby`"))?
                .trim_end_matches(',')
                .split(',')
                .map(TagKey::try_from)
                .collect::<Result<Vec<TagKey>>>()?,
            _ => Vec::new(),
        };
        let filter_expr = args
            .get(1 + if group_by.is_empty() { 0 } else { 2 })
            .map_or_else(
                || Ok(FilterExpr::default()),
                |s| FilterExpr::try_from(s.as_str()),
            )?;

        Ok(Self(tag, group_by, filter_expr))
    }
}

impl TryFrom<&[String]> for SeekArgs {
    type Error = anyhow::Error;

    fn try_from(args: &[String]) -> Result<Self> {
        if args.is_empty() {
            bail!("invalid arguments to `seek`");
        }
        let secs = args[0].parse::<i32>()?;

        Ok(Self(secs))
    }
}

impl TryFrom<&[String]> for VolumeArgs {
    type Error = anyhow::Error;

    fn try_from(args: &[String]) -> Result<Self> {
        // args are non-empty here
        let chars: Vec<_> = args[0].chars().collect();
        let volume = match chars.first().unwrap() {
            '+' => {
                let x = args[0].trim_start_matches('+').parse::<i8>()?;
                Volume::Change(x)
            }
            '-' => {
                let x = args[0].parse::<i8>()?;
                Volume::Change(x)
            }
            _ => {
                let x = args[0].parse::<u8>()?;
                Volume::Set(x)
            }
        };

        Ok(Self(volume))
    }
}

impl TryFrom<&[String]> for AddArgs {
    type Error = anyhow::Error;

    fn try_from(args: &[String]) -> Result<Self> {
        if args.is_empty() {
            bail!("invalid arguments to `add`");
        }
        let ids = args[0]
            .trim_end_matches(',')
            .split(',')
            .map(|s| s.parse::<u32>().map_err(|e| e.into()))
            .collect::<Result<Vec<u32>>>()?;
        let pos = args.get(1).and_then(|x| x.parse::<usize>().ok());

        Ok(Self(ids, pos))
    }
}

impl TryFrom<&[String]> for PlayArgs {
    type Error = anyhow::Error;

    fn try_from(args: &[String]) -> Result<Self> {
        if args.is_empty() {
            bail!("invalid arguments to `play`");
        }
        let id = args[0].parse::<u32>()?;

        Ok(Self(id))
    }
}

impl TryFrom<&[String]> for RemoveArgs {
    type Error = anyhow::Error;

    fn try_from(args: &[String]) -> Result<Self> {
        if args.is_empty() {
            bail!("invalid arguments to `remove`");
        }
        let ids = args[0]
            .trim_end_matches(',')
            .split(',')
            .map(|s| s.parse::<u32>().map_err(|e| e.into()))
            .collect::<Result<Vec<u32>>>()?;

        Ok(Self(ids))
    }
}

impl TryFrom<&str> for RequestKind {
    type Error = anyhow::Error;

    fn try_from(s: &str) -> Result<Self> {
        use DbRequestKind as Db;
        use PlaybackRequestKind as Playback;
        use QueueRequestKind as Queue;
        use RequestKind as Request;
        use StatusRequestKind as Status;

        let tokens = request::tokenize(s)?;
        let kind = match tokens.first().map(|s| s.as_str()) {
            Some(request) => match request {
                "metadata" => Request::Db(Db::Metadata(tokens[1..].try_into()?)),
                "reset" => Request::Db(Db::Reset),
                "select" => Request::Db(Db::Select(tokens[1..].try_into()?)),
                "unique" => Request::Db(Db::Unique(tokens[1..].try_into()?)),
                "update" => Request::Db(Db::Update),

                "pause" => Request::Playback(Playback::Pause),
                "resume" => Request::Playback(Playback::Resume),
                "seek" => Request::Playback(Playback::Seek(tokens[1..].try_into()?)),
                "stop" => Request::Playback(Playback::Stop),
                "toggle" => Request::Playback(Playback::Toggle),

                "add" => Request::Queue(Queue::Add(tokens[1..].try_into()?)),
                "clear" => Request::Queue(Queue::Clear),
                "next" => Request::Queue(Queue::Next),
                "play" => Request::Queue(Queue::Play(tokens[1..].try_into()?)),
                "previous" => Request::Queue(Queue::Previous),
                "remove" => Request::Queue(Queue::Remove(tokens[1..].try_into()?)),

                "current" => Request::Status(Status::Current),
                "elapsed" => Request::Status(Status::Elapsed),
                "queue" => Request::Status(Status::Queue),
                "volume" => match tokens.len() {
                    1 => Request::Status(Status::Volume),
                    _ => Request::Playback(Playback::Volume(tokens[1..].try_into()?)),
                },

                _ => bail!("invalid request"),
            },
            None => bail!("empty request"),
        };

        Ok(kind)
    }
}

impl Request {
    pub fn new(kind: RequestKind, tx_response: RespondTo) -> Self {
        Self { kind, tx_response }
    }
}
