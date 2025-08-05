use anyhow::{Result, bail};
use tokio::sync::{mpsc::Sender, oneshot};

use crate::{
    error::MyError,
    model::{comparator::Comparator, filter::FilterExpr, response::Response, tag_key::TagKey},
};

type RespondTo = oneshot::Sender<Response>;

type UpdateArgs = ();
type SelectArgs = (FilterExpr, Vec<Comparator>);
type MetadataArgs = (Vec<u32>, Vec<TagKey>);
type UniqueArgs = (TagKey, FilterExpr, Vec<TagKey>);

// Pause, Toggle, Resume, Stop, Next, Prev, Clear = ()
// VolumeChange, Seek = (i32)
// Add = (db_id: u32)
// Play = (queue_id: u32)

pub struct Request<T> {
    pub kind: T,
    pub tx_response: RespondTo,
}

// copy over the parsing from -model
