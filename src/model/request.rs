use anyhow::{Result, bail};
use tokio::sync::{mpsc::Sender, oneshot};

use crate::{
    error::MyError,
    model::{comparator::Comparator, filter::FilterExpr, response::Response, tag_key::TagKey},
    parsers::request,
};

type RespondTo = oneshot::Sender<Response>;

pub struct SelectArgs(pub FilterExpr, pub Vec<Comparator>);
pub struct MetadataArgs(pub Vec<u32>, pub Vec<TagKey>);
pub struct UniqueArgs(pub TagKey, pub Vec<TagKey>, pub FilterExpr);
pub enum DbRequestKind {
    Update,
    Select(SelectArgs),
    Metadata(MetadataArgs),
    Unique(UniqueArgs),
}

pub struct SeekArgs(pub i32); // in seconds
pub struct VolumeArgs(pub i32); // in range 0..=100
pub enum PlaybackRequestKind {
    Pause,
    Resume,
    Toggle,
    Stop,
    Seek(SeekArgs),
    Volume(VolumeArgs),
}

pub struct AddArgs(pub Vec<u32>, pub Option<usize>); // db ids
pub struct PlayArgs(pub u32); // queue id
pub enum QueueRequestKind {
    Clear,
    Add(AddArgs),
    Play(PlayArgs),
    Next,
    Previous,
}

pub enum StatusRequestKind {
    Elapsed,
    Playlist,
    Song,
    Volume,
}

pub enum RequestKind {
    Db(DbRequestKind),
    Playback(PlaybackRequestKind),
    Queue(QueueRequestKind),
}

pub struct Request {
    pub kind: RequestKind,
    pub tx_response: RespondTo,
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

impl TryFrom<&[String]> for MetadataArgs {
    type Error = anyhow::Error;

    fn try_from(args: &[String]) -> Result<Self> {
        if args.len() != 2 {
            bail!(MyError::Syntax("Invalid arguments to `metadata`".into()));
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

impl TryFrom<&[String]> for UniqueArgs {
    type Error = anyhow::Error;

    fn try_from(args: &[String]) -> Result<Self> {
        if args.is_empty() {
            bail!(MyError::Syntax("Invalid arguments to `unique`".into()));
        }
        let tag = TagKey::try_from(args[0].as_str())?;
        let group_by = match args.get(1).map(|s| s.as_str()) {
            Some("groupby") => args
                .get(2)
                .ok_or(MyError::Syntax("No tags provided to `groupby`".into()))?
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

impl TryFrom<&[String]> for AddArgs {
    type Error = anyhow::Error;

    fn try_from(args: &[String]) -> Result<Self> {
        if args.is_empty() {
            bail!(MyError::Syntax("Invalid arguments to `add`".into()));
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
            bail!(MyError::Syntax("Invalid arguments to `play`".into()));
        }
        let id = args[0].parse::<u32>()?;

        Ok(Self(id))
    }
}

impl TryFrom<&str> for RequestKind {
    type Error = anyhow::Error;

    fn try_from(s: &str) -> Result<Self> {
        use DbRequestKind as Db;
        use PlaybackRequestKind as Playback;
        use QueueRequestKind as Queue;
        use RequestKind as Request;

        let tokens = request::tokenize(s)?;
        let kind = match tokens.first().map(|s| s.as_str()) {
            Some(request) => match request {
                "update" => Request::Db(Db::Update),
                "select" => Request::Db(Db::Select(tokens[1..].try_into()?)),
                "metadata" => Request::Db(Db::Metadata(tokens[1..].try_into()?)),
                "unique" => Request::Db(Db::Unique(tokens[1..].try_into()?)),

                "add" => Request::Queue(Queue::Add(tokens[1..].try_into()?)),
                "play" => Request::Queue(Queue::Play(tokens[1..].try_into()?)),
                "next" => Request::Queue(Queue::Next),
                "previous" => Request::Queue(Queue::Previous),

                _ => bail!(MyError::Syntax("Invalid request".into())),
            },
            None => bail!(MyError::Syntax("Empty request".into())),
        };

        Ok(kind)
    }
}

impl Request {
    pub fn new(kind: RequestKind, tx_response: RespondTo) -> Self {
        Self { kind, tx_response }
    }
}
