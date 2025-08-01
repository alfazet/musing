use anyhow::{Result, anyhow, bail};
use regex::Regex;

use crate::{error::MyError, model::song::*};

pub type FilterArgs = (String, String, String);

pub trait Filter {
    fn matches(&self, song: &Song) -> bool;
}

struct RegexFilter {
    tag: String,
    regex: Regex,
    positive: bool,
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
                if self.positive { v } else { !v }
            }
            None => false,
        }
    }
}

impl RegexFilter {
    pub fn new(tag: String, regex: String, positive: bool) -> Result<Self> {
        let regex = Regex::new(&regex).map_err(|e| MyError::Syntax(e.to_string()))?;
        Ok(Self {
            tag,
            regex,
            positive,
        })
    }
}

impl FilterExprSymbol {
    pub fn try_into_filter((tag, comparator, pattern): FilterArgs) -> Result<Self> {
        let boxed_filter = match comparator.as_str() {
            "==" => Box::new(RegexFilter::new(tag, pattern, true)?),
            "!=" => Box::new(RegexFilter::new(tag, pattern, false)?),
            _ => bail!(MyError::Syntax("Invalid comparator".into())),
        };

        Ok(Self::Filter(boxed_filter))
    }
}

impl FilterExpr {
    pub fn new() -> Self {
        Self {
            symbols: Vec::new(),
        }
    }

    pub fn push(&mut self, symbol: FilterExprSymbol) {
        self.symbols.push(symbol);
    }

    pub fn evaluate(&self, song: &Song) -> Result<bool> {
        use FilterExprOperator::*;
        use FilterExprSymbol::*;

        let mut stack = Vec::new();
        for symbol in self.symbols.iter() {
            match symbol {
                Operator(op) => {
                    let f1 = stack
                        .pop()
                        .ok_or(MyError::Syntax("Invalid filter expression".into()))?;
                    let f2 = stack
                        .pop()
                        .ok_or(MyError::Syntax("Invalid filter expression".into()))?;
                    let res = match op {
                        OpAnd => f1 & f2,
                        OpOr => f1 | f2,
                    };
                    stack.push(res);
                }
                Filter(filter) => stack.push(filter.matches(song)),
            }
        }

        Ok(stack
            .pop()
            .ok_or(MyError::Syntax("Invalid filter expression".into()))?)
    }
}
