use anyhow::{Result, anyhow, bail};
use std::{cmp::Ordering, str::FromStr};

use crate::model::{
    song::*,
    tag_key::{TagKey, TagKeyKind},
};

pub struct Comparator {
    tag_key: TagKey,
    inverted: bool,
}

impl TryFrom<&str> for Comparator {
    type Error = anyhow::Error;

    fn try_from(s: &str) -> Result<Self> {
        let (tag_key, inverted) = if let Some(s) = s.strip_prefix('!') {
            (s.try_into()?, true)
        } else {
            (s.try_into()?, false)
        };

        Ok(Self { tag_key, inverted })
    }
}

impl Comparator {
    fn cmp_values(&self, lhs: &str, rhs: &str) -> Ordering {
        match self.tag_key.kind {
            TagKeyKind::String => lhs.cmp(rhs),
            TagKeyKind::Integer => {
                let lhs = lhs.parse::<i32>();
                let rhs = rhs.parse::<i32>();
                match (lhs, rhs) {
                    (Ok(lhs), Ok(rhs)) => lhs.cmp(&rhs),
                    _ => Ordering::Equal,
                }
            }
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

    pub fn cmp(&self, lhs: &SongMeta, rhs: &SongMeta) -> Ordering {
        let lhs = lhs.get(&self.tag_key);
        let rhs = rhs.get(&self.tag_key);
        let ordering = match (lhs, rhs) {
            (Some(lhs), Some(rhs)) => self.cmp_values(lhs, rhs),
            (Some(lhs), None) => Ordering::Greater,
            (None, Some(rhs)) => Ordering::Less,
            (None, None) => Ordering::Equal,
        };

        let v = Vec::<i32>::new();
        if self.inverted {
            ordering.reverse()
        } else {
            ordering
        }
    }
}
