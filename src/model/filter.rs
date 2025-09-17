use anyhow::{Result, anyhow, bail};
use regex::Regex;
use serde_json::Value;
use unidecode::unidecode;

use crate::model::{song::Song, tag_key::TagKey};

pub trait Filter: Send + Sync {
    fn matches(&self, song: &Song) -> bool;
}

// filters inside of one expression are joined by a logical "and"
pub struct FilterExpr(pub Vec<Box<dyn Filter>>);

// matches iff the tag value matches the regex
#[derive(Debug)]
struct RegexFilter {
    tag: TagKey,
    regex: Regex,
}

impl FilterExpr {
    pub fn evaluate(&self, song: &Song) -> bool {
        self.0.iter().all(|filter| filter.matches(song))
    }
}

impl Filter for RegexFilter {
    fn matches(&self, song: &Song) -> bool {
        match song.metadata.get(self.tag) {
            Some(value) => self.regex.is_match(&unidecode(value)),
            None => false,
        }
    }
}

impl TryFrom<Value> for Box<dyn Filter> {
    type Error = anyhow::Error;

    fn try_from(mut v: Value) -> Result<Self> {
        let map = v
            .as_object_mut()
            .ok_or(anyhow!("a filter must be a JSON object"))?;
        let kind = map.remove("kind").ok_or(anyhow!("key `kind` not found"))?;
        let tag: TagKey = map
            .remove("tag")
            .ok_or(anyhow!("key `tag` not found"))?
            .as_str()
            .ok_or(anyhow!("`tag` must be a string"))?
            .try_into()?;
        let filter = match kind.as_str().ok_or(anyhow!("`kind` must be a string"))? {
            "regex" => {
                let regex = Regex::new(
                    map.remove("regex")
                        .ok_or(anyhow!("key `regex` not found"))?
                        .as_str()
                        .ok_or(anyhow!("`regex` must be a string"))?,
                )?;

                Box::new(RegexFilter { tag, regex })
            }
            other => bail!("invalid value of key `kind`: `{}`", other),
        };

        Ok(filter)
    }
}
