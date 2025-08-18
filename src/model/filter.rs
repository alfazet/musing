use anyhow::{Result, anyhow, bail};
use regex::Regex;
use std::fmt::{self, Display, Formatter};
use unidecode::unidecode;

use crate::{
    model::{
        song::{Metadata, Song},
        tag_key::TagKey,
    },
    parsers::filter as parser,
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

struct NullFilter; // matches everything

// TODO: FuzzyFilter based on edit distance
// TODO: ignore caps by default

#[derive(Debug)]
pub enum FilterExprOperator {
    OpAnd,
    OpOr,
}

pub enum FilterExprSymbol {
    Filter(Box<dyn Filter + Send + Sync>),
    Operator(FilterExprOperator),
}

pub struct FilterExpr {
    symbols: Vec<FilterExprSymbol>, // in rpn
}

impl Filter for RegexFilter {
    fn matches(&self, song: &Song) -> bool {
        match song.metadata.get(&self.tag) {
            Some(value) => {
                let v = self.regex.is_match(&unidecode(value));
                if self.inverted { !v } else { v }
            }
            None => false,
        }
    }
}

impl RegexFilter {
    pub fn new(tag: TagKey, regex: Regex, inverted: bool) -> Self {
        Self {
            tag,
            regex,
            inverted,
        }
    }
}

impl Filter for NullFilter {
    fn matches(&self, _: &Song) -> bool {
        true
    }
}

impl TryFrom<FilterArgs> for FilterExprSymbol {
    type Error = anyhow::Error;

    fn try_from((tag, comparator, pattern): FilterArgs) -> Result<Self> {
        let tag_key = tag.as_str().try_into()?;
        let regex = Regex::new(&pattern)?;
        let boxed_filter = match comparator.as_str() {
            "==" => Box::new(RegexFilter::new(tag_key, regex, false)),
            "!=" => Box::new(RegexFilter::new(tag_key, regex, true)),
            _ => bail!("invalid comparator"),
        };

        Ok(Self::Filter(boxed_filter))
    }
}

impl Default for FilterExpr {
    fn default() -> Self {
        Self {
            symbols: vec![FilterExprSymbol::Filter(Box::new(NullFilter))],
        }
    }
}

impl TryFrom<Vec<FilterExprSymbol>> for FilterExpr {
    type Error = anyhow::Error;

    fn try_from(symbols: Vec<FilterExprSymbol>) -> Result<Self> {
        use FilterExprSymbol as FESymbol;

        let mut stack_size = 0;
        for symbol in symbols.iter() {
            match symbol {
                FESymbol::Operator(_) => {
                    if stack_size < 2 {
                        bail!("invalid filter expression");
                    }
                    stack_size -= 1;
                }
                FESymbol::Filter(_) => stack_size += 1,
            }
        }
        if stack_size == 1 {
            Ok(Self { symbols })
        } else {
            bail!("invalid filter expression");
        }
    }
}

impl TryFrom<&str> for FilterExpr {
    type Error = anyhow::Error;

    fn try_from(s: &str) -> Result<Self> {
        let infix = parser::tokenize(s)?;
        let rpn = parser::into_rpn(infix)?;
        let filter_expr_symbols = rpn
            .into_iter()
            .map(parser::token_to_symbol)
            .collect::<Result<Vec<FilterExprSymbol>>>()?;

        FilterExpr::try_from(filter_expr_symbols)
    }
}

impl FilterExpr {
    // unwraps here will never panic because all
    // filter expressions pass a validity check on creation
    pub fn evaluate(&self, song: &Song) -> bool {
        use FilterExprOperator as FEOperator;
        use FilterExprSymbol as FESymbol;

        let mut stack = Vec::new();
        for symbol in self.symbols.iter() {
            match symbol {
                FESymbol::Operator(op) => {
                    let f1 = stack.pop().unwrap();
                    let f2 = stack.pop().unwrap();
                    let res = match op {
                        FEOperator::OpAnd => f1 & f2,
                        FEOperator::OpOr => f1 | f2,
                    };
                    stack.push(res);
                }
                FESymbol::Filter(filter) => stack.push(filter.matches(song)),
            }
        }

        stack.pop().unwrap()
    }
}
