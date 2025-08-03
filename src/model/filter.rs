use anyhow::{Result, anyhow, bail};
use regex::Regex;

use crate::{
    error::MyError,
    model::{song::*, tag_key::TagKey},
};

pub type FilterArgs = (String, String, String);

pub trait Filter {
    fn matches(&self, song: &Song) -> bool;
}

struct RegexFilter {
    tag: TagKey,
    regex: Regex,
    inverted: bool,
}

// TODO: FuzzyFilter based on edit distance

pub enum FilterExprOperator {
    OpAnd,
    OpOr,
}

pub enum FilterExprSymbol {
    Filter(Box<dyn Filter>),
    Operator(FilterExprOperator),
}

pub struct FilterExpr {
    symbols: Vec<FilterExprSymbol>, // in rpn
}

impl Filter for RegexFilter {
    fn matches(&self, song: &Song) -> bool {
        match song.song_meta.get(&self.tag) {
            Some(value) => {
                let v = self.regex.is_match(value);
                if self.inverted { !v } else { v }
            }
            None => false,
        }
    }
}

impl RegexFilter {
    pub fn new(tag: TagKey, regex: String, inverted: bool) -> Result<Self> {
        let regex = Regex::new(&regex).map_err(|e| MyError::Syntax(e.to_string()))?;
        Ok(Self {
            tag,
            regex,
            inverted,
        })
    }
}

impl FilterExprSymbol {
    pub fn try_into_filter((tag, comparator, pattern): FilterArgs) -> Result<Self> {
        let tag_key = tag.as_str().try_into()?;
        let boxed_filter = match comparator.as_str() {
            "==" => Box::new(RegexFilter::new(tag_key, pattern, false)?),
            "!=" => Box::new(RegexFilter::new(tag_key, pattern, true)?),
            _ => bail!(MyError::Syntax("Invalid comparator".into())),
        };

        Ok(Self::Filter(boxed_filter))
    }
}

impl FilterExpr {
    pub fn try_new(symbols: Vec<FilterExprSymbol>) -> Result<Self> {
        use FilterExprOperator::*;
        use FilterExprSymbol::*;

        let mut stack_size = 0;
        for symbol in symbols.iter() {
            match symbol {
                Operator(op) => {
                    if stack_size < 2 {
                        bail!(MyError::Syntax("Invalid filter expression".into()));
                    }
                    stack_size -= 1;
                }
                Filter(filter) => stack_size += 1,
            }
        }
        if stack_size == 1 {
            Ok(Self { symbols })
        } else {
            bail!(MyError::Syntax("Invalid filter expression".into()));
        }
    }

    // unwraps here will never panic because all
    // filter expressions pass a validity check on creation
    pub fn evaluate(&self, song: &Song) -> bool {
        use FilterExprOperator::*;
        use FilterExprSymbol::*;

        let mut stack = Vec::new();
        for symbol in self.symbols.iter() {
            match symbol {
                Operator(op) => {
                    let f1 = stack.pop().unwrap();
                    let f2 = stack.pop().unwrap();
                    let res = match op {
                        OpAnd => f1 & f2,
                        OpOr => f1 | f2,
                    };
                    stack.push(res);
                }
                Filter(filter) => stack.push(filter.matches(song)),
            }
        }

        stack.pop().unwrap()
    }
}
