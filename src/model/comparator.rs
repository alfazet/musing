use anyhow::{Result, anyhow, bail};
use serde_json::Value;
use std::cmp::Ordering;

use crate::model::{
    song::Metadata,
    tag_key::{TagKey, TagKeyKind},
};

#[derive(Debug, Default)]
enum ComparisonOrder {
    #[default]
    Ascending,
    Descending,
}

#[derive(Debug)]
pub struct Comparator {
    tag: TagKey,
    order: ComparisonOrder,
}

impl TryFrom<&str> for ComparisonOrder {
    type Error = anyhow::Error;

    fn try_from(s: &str) -> Result<Self> {
        match s {
            "ascending" => Ok(ComparisonOrder::Ascending),
            "descending" => Ok(ComparisonOrder::Descending),
            _ => bail!("`order` must be 'ascending' or 'descending'"),
        }
    }
}

impl TryFrom<Value> for Comparator {
    type Error = anyhow::Error;

    fn try_from(mut v: Value) -> Result<Self> {
        let map = v
            .as_object_mut()
            .ok_or(anyhow!("a comparator must be a JSON map"))?;
        let tag: TagKey = map
            .remove("tag")
            .ok_or(anyhow!("key `tag` not found"))?
            .as_str()
            .ok_or(anyhow!("`tag` must be a string"))?
            .try_into()?;
        let order: ComparisonOrder = match map.remove("order") {
            Some(v) => v
                .as_str()
                .ok_or(anyhow!("`order` must be a string"))?
                .try_into()?,
            None => ComparisonOrder::Ascending,
        };

        Ok(Comparator { tag, order })
    }
}

impl Comparator {
    fn cmp_values<S, T>(&self, lhs: S, rhs: T) -> Ordering
    where
        S: AsRef<str>,
        T: AsRef<str>,
    {
        let (lhs, rhs) = (lhs.as_ref(), rhs.as_ref());
        match self.tag.kind {
            TagKeyKind::String => lhs.cmp(rhs),
            TagKeyKind::Integer => {
                let lhs = lhs.parse::<i32>();
                let rhs = rhs.parse::<i32>();
                match (lhs, rhs) {
                    (Ok(lhs), Ok(rhs)) => lhs.cmp(&rhs),
                    _ => Ordering::Equal,
                }
            }
            // we can't just compare strings of type X/Y lexicographically
            // because (e.g.) "10/12" < "2/12"
            TagKeyKind::OutOf => {
                let lhs = lhs.split('/').next().and_then(|n| n.parse::<i32>().ok());
                let rhs = rhs.split('/').next().and_then(|n| n.parse::<i32>().ok());
                match (lhs, rhs) {
                    (Some(lhs), Some(rhs)) => lhs.cmp(&rhs),
                    _ => Ordering::Equal,
                }
            }
        }
    }

    pub fn cmp(&self, lhs: &Metadata, rhs: &Metadata) -> Ordering {
        let lhs = lhs.get(&self.tag);
        let rhs = rhs.get(&self.tag);
        let ordering = match (lhs, rhs) {
            (Some(lhs), Some(rhs)) => self.cmp_values(lhs, rhs),
            (Some(_), None) => Ordering::Greater,
            (None, Some(_)) => Ordering::Less,
            (None, None) => Ordering::Equal,
        };

        match self.order {
            ComparisonOrder::Ascending => ordering,
            ComparisonOrder::Descending => ordering.reverse(),
        }
    }
}
