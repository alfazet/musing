use anyhow::{Result, anyhow, bail};
use regex::Regex;
use serde_json::{Map, Value};
use unidecode::unidecode;

use crate::model::{song::Song, tag_key::TagKey};

pub trait Filter: Send + Sync {
    fn matches(&self, song: &Song) -> bool;
}

// filters inside of one expression are joined by a logical "and"
pub struct FilterExpr(pub Vec<Box<dyn Filter>>);

// matches iff the tag value matches the regex
struct RegexFilter {
    tag: TagKey,
    regex: Regex,
}

// matches iff edit distance between
// the tag value and the pattern is <= dist
struct FuzzyFilter {
    tag: TagKey,
    pattern: String,
    dist: u32,
}

impl FilterExpr {
    pub fn evaluate(&self, song: &Song) -> bool {
        self.0.iter().all(|filter| filter.matches(song))
    }
}

impl Filter for RegexFilter {
    fn matches(&self, song: &Song) -> bool {
        match song.metadata.get(&self.tag) {
            Some(value) => self.regex.is_match(&unidecode(value)),
            None => false,
        }
    }
}

impl Filter for FuzzyFilter {
    fn matches(&self, _: &Song) -> bool {
        true
    }
}

impl TryFrom<&mut Map<String, Value>> for Box<dyn Filter> {
    type Error = anyhow::Error;

    fn try_from(map: &mut Map<String, Value>) -> Result<Self> {
        let kind = map.remove("kind").ok_or(anyhow!("key `kind` not found"))?;
        let tag: TagKey = map
            .remove("tag")
            .ok_or(anyhow!("key `tag` not found"))?
            .as_str()
            .ok_or(anyhow!("key `tag` must be a string"))?
            .try_into()?;
        let filter = match kind
            .as_str()
            .ok_or(anyhow!("key `kind` must be a string"))?
        {
            "regex" => {
                let regex = Regex::new(
                    map.remove("regex")
                        .ok_or(anyhow!("key `regex` not found"))?
                        .as_str()
                        .ok_or(anyhow!("key `regex` must be a string"))?,
                )?;

                Box::new(RegexFilter { tag, regex })
            }
            other => bail!("invalid value of key `kind`: `{}`", other),
        };

        Ok(filter)
    }
}
