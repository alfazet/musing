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
pub struct UniqueArgs(pub TagKey, pub FilterExpr, pub Vec<TagKey>);
pub struct AddArgs(pub Vec<u32>); // db ids
pub struct PlayArgs(pub u32); // queue id
pub struct VolumeChangeArgs(pub i32); // in range 0..=100
pub struct SeekArgs(pub i32); // in seconds

pub enum RequestKind {
    Update,
    Select(SelectArgs),
    Metadata(MetadataArgs),
    Unique(UniqueArgs),

    Pause,
    Resume,
    Toggle,
    Stop,

    Clear,
    Add(AddArgs),
    Play(PlayArgs),

    VolumeChange(VolumeChangeArgs),
    Seek(SeekArgs),
}

pub struct Request {
    pub kind: RequestKind,
    pub tx_response: RespondTo,
}

impl TryFrom<&[String]> for SelectArgs {
    type Error = anyhow::Error;

    fn try_from(args: &[String]) -> Result<Self> {
        let Some(arg1) = args.get(0).map(|s| s.as_str()) else {
            bail!(MyError::Syntax("Invalid arguments to `select`".into()));
        };
        let filter_expr = FilterExpr::try_from(arg1)?;
        let sort_by = args
            .get(1)
            .map(|v| {
                v.trim_end_matches(',')
                    .split(',')
                    .map(|s| Comparator::try_from(s))
                    .collect::<Result<Vec<Comparator>>>()
            })
            .unwrap_or(Ok(Vec::new()))?;

        Ok(Self(filter_expr, sort_by))
    }
}

impl TryFrom<&str> for RequestKind {
    type Error = anyhow::Error;

    fn try_from(s: &str) -> Result<Self> {
        use RequestKind as Kind;

        let tokens = request::tokenize(s)?;
        let kind = match tokens
            .get(0)
            .map(|s| s.as_str())
            .ok_or(MyError::Syntax("Empty request".into()))?
        {
            "select" => Kind::Select(tokens[1..].try_into()?),
            _ => todo!(),
        };

        Ok(kind)
    }
}

impl Request {
    pub fn new(kind: RequestKind, tx_response: RespondTo) -> Self {
        Self { kind, tx_response }
    }
}
