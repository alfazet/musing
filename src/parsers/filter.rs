use anyhow::{Result, anyhow, bail};
use std::{iter::Peekable, str};

use crate::{error::MyError, model::filter::*};

#[derive(Clone, Debug, PartialEq)]
struct Operator(u8);
const OP_AND: Operator = Operator(1);
const OP_OR: Operator = Operator(2);

#[derive(Clone, Debug)]
enum Token {
    Operator(Operator),
    Filter(FilterArgs),
    OpeningParen,
    ClosingParen,
}

fn tokenize_filter(s: &mut Peekable<str::Chars>) -> Result<FilterArgs> {
    #[derive(Debug)]
    enum State {
        Start,
        Tag,
        Comparator,
        Pattern,
        PatternUnquoted,
        PatternQuoted,
        PatternQuotedBackslash,
    }

    let mut tag = String::new();
    let mut comparator = String::new();
    let mut pattern = String::new();
    let mut state = State::Start;
    loop {
        let c = s.next();
        state = match state {
            State::Start => match c {
                Some(' ') | Some('\t') | Some('\n') => State::Start,
                Some(c) if c.is_alphanumeric() => {
                    tag.push(c);
                    State::Tag
                }
                None => bail!(MyError::Syntax("Incomplete filter".into())),
                _ => bail!(MyError::Syntax("Tag must be alphanumeric".into())),
            },
            State::Tag => match c {
                Some(c) if c.is_alphanumeric() => {
                    tag.push(c);
                    State::Tag
                }
                Some(c @ '!') | Some(c @ '=') => {
                    comparator.push(c);
                    State::Comparator
                }
                None => bail!(MyError::Syntax("Incomplete filter".into())),
                _ => bail!(MyError::Syntax("Tag must be alphanumeric".into())),
            },
            State::Comparator => match c {
                Some(c @ '=') => {
                    comparator.push(c);
                    State::Pattern
                }
                None => bail!(MyError::Syntax("Incomplete filter".into())),
                _ => bail!(MyError::Syntax("Invalid comparator".into())),
            },
            State::Pattern => match c {
                Some('\"') => State::PatternQuoted,
                Some(c) => {
                    pattern.push(c);
                    State::PatternUnquoted
                }
                None => break,
            },
            State::PatternUnquoted => match c {
                None => break,
                Some(c) if c.is_whitespace() => break,
                Some(c) => {
                    pattern.push(c);
                    State::PatternUnquoted
                }
            },
            State::PatternQuoted => match c {
                None => bail!(MyError::Syntax("Unclosed double quote".into())),
                Some('\\') => State::PatternQuotedBackslash,
                Some('\"') => break,
                Some(c) => {
                    pattern.push(c);
                    State::PatternQuoted
                }
            },
            State::PatternQuotedBackslash => match c {
                None => bail!(MyError::Syntax("Unclosed double quote".into())),
                Some('\n') => State::PatternQuoted,
                Some(c @ '\"') | Some(c @ '\\') => {
                    pattern.push(c);
                    State::PatternQuoted
                }
                Some(c) => {
                    pattern.push('\\');
                    pattern.push(c);
                    State::PatternQuoted
                }
            },
        };
    }

    Ok((tag, comparator, pattern))
}

fn tokenize(s: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut s = s.chars().peekable();
    loop {
        let mut already = false;
        match s.peek() {
            Some(c) if c.is_whitespace() => (),
            Some('(') => tokens.push(Token::OpeningParen),
            Some(')') => tokens.push(Token::ClosingParen),
            Some('&') => tokens.push(Token::Operator(OP_AND)),
            Some('|') => tokens.push(Token::Operator(OP_OR)),
            Some(c) => match tokenize_filter(&mut s) {
                Ok(filter) => {
                    tokens.push(Token::Filter(filter));
                    already = true;
                }
                Err(e) => bail!(MyError::Syntax(e.to_string())),
            },
            None => break,
        }
        if !already {
            s.next();
        }
    }

    Ok(tokens)
}

fn into_rpn(infix: Vec<Token>) -> Result<Vec<Token>> {
    let mut op_stack = Vec::new();
    let mut rpn = Vec::new();
    for token in infix.into_iter() {
        match token {
            ref token @ Token::Operator(ref op1) => {
                while let Some(Token::Operator(op2)) = op_stack.last() {
                    if op1.0 <= op2.0 {
                        rpn.push(op_stack.pop().unwrap());
                    } else {
                        break;
                    }
                }
                op_stack.push(token.clone());
            }
            Token::Filter(_) => rpn.push(token),
            Token::OpeningParen => op_stack.push(token),
            Token::ClosingParen => loop {
                match op_stack.last() {
                    Some(&Token::OpeningParen) => {
                        let _ = op_stack.pop();
                        break;
                    }
                    Some(_) => rpn.push(op_stack.pop().unwrap()),
                    None => bail!(MyError::Syntax("Mismatched parentheses".into())),
                }
            },
        }
    }
    while let Some(operator) = op_stack.pop() {
        match operator {
            Token::OpeningParen | Token::ClosingParen => {
                bail!(MyError::Syntax("Mismatched parentheses".into()))
            }
            _ => rpn.push(operator),
        }
    }

    Ok(rpn)
}

impl TryFrom<String> for FilterExpr {
    type Error = anyhow::Error;

    fn try_from(s: String) -> Result<Self> {
        let infix = tokenize(&s)?;
        let rpn = into_rpn(infix)?;
        let mut filter_expr_symbols = Vec::new();
        for token in rpn.into_iter() {
            let symbol = match token {
                Token::Operator(op) => match op {
                    OP_AND => FilterExprSymbol::Operator(FilterExprOperator::OpAnd),
                    OP_OR => FilterExprSymbol::Operator(FilterExprOperator::OpOr),
                    _ => unreachable!(),
                },
                Token::Filter(args) => FilterExprSymbol::try_into_filter(args)?,
                // there are no parentheses in rpn
                _ => unreachable!(),
            };
            filter_expr_symbols.push(symbol);
        }

        FilterExpr::try_new(filter_expr_symbols)
    }
}
