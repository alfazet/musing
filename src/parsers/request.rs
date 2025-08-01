use anyhow::{Result, anyhow, bail};
use std::mem;

use crate::error::MyError;

// based on https://github.com/tmiasko/shell-words/
// modified to include square brackets as a delimiter for filters
#[derive(Debug)]
enum State {
    Whitespace,
    Backslash,
    Unquoted,
    UnquotedBackslash,
    SingleQuoted,
    DoubleQuoted,
    DoubleQuotedBackslash,
    SquareBracket,
    SquareBracketBackslash,
}

/// Rules:
///     - splits the string on whitespace ...
///     - ... except strings in quotes or square brackets, which are treated as one contiguous string
///     - inside strings in quotes the characters " (double quote) and \ (slash) need to be escaped
///     as \" and \\ respectively
///     - inside strings in square brackets the characters ] (closing bracket) and \ (slash) need to be escaped
///     as \] and \\ respectively
pub fn tokenize(s: &str) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut s = s.chars();
    let mut cur = String::new();
    let mut state = State::Whitespace;
    loop {
        let c = s.next();
        state = match state {
            State::Whitespace => match c {
                None => break,
                Some('\'') => State::SingleQuoted,
                Some('\"') => State::DoubleQuoted,
                Some('\\') => State::Backslash,
                Some('[') => State::SquareBracket,
                Some(c) if c.is_whitespace() => State::Whitespace,
                Some(c) => {
                    cur.push(c);
                    State::Unquoted
                }
            },
            State::Backslash => match c {
                None => {
                    cur.push('\\');
                    tokens.push(mem::take(&mut cur));
                    break;
                }
                Some('\n') => State::Whitespace,
                Some(c) => {
                    cur.push(c);
                    State::Unquoted
                }
            },
            State::Unquoted => match c {
                None => {
                    tokens.push(mem::take(&mut cur));
                    break;
                }
                Some('\'') => State::SingleQuoted,
                Some('\"') => State::DoubleQuoted,
                Some('\\') => State::UnquotedBackslash,
                Some(c) if c.is_whitespace() => {
                    tokens.push(mem::take(&mut cur));
                    State::Whitespace
                }
                Some(c) => {
                    cur.push(c);
                    State::Unquoted
                }
            },
            State::UnquotedBackslash => match c {
                None => {
                    cur.push('\\');
                    tokens.push(mem::take(&mut cur));
                    break;
                }
                Some('\n') => State::Unquoted,
                Some(c) => {
                    cur.push(c);
                    State::Unquoted
                }
            },
            State::SingleQuoted => match c {
                None => bail!(MyError::Syntax("Unclosed single quote".into())),
                Some('\'') => State::Unquoted,
                Some(c) => {
                    cur.push(c);
                    State::SingleQuoted
                }
            },
            State::DoubleQuoted => match c {
                None => bail!(MyError::Syntax("Unclosed double quote".into())),
                Some('\"') => State::Unquoted,
                Some('\\') => State::DoubleQuotedBackslash,
                Some(c) => {
                    cur.push(c);
                    State::DoubleQuoted
                }
            },
            State::DoubleQuotedBackslash => match c {
                None => bail!(MyError::Syntax("Unclosed double quote".into())),
                Some('\n') => State::DoubleQuoted,
                Some(c @ '$') | Some(c @ '`') | Some(c @ '"') | Some(c @ '\\') => {
                    cur.push(c);
                    State::DoubleQuoted
                }
                Some(c) => {
                    cur.push('\\');
                    cur.push(c);
                    State::DoubleQuoted
                }
            },
            State::SquareBracket => match c {
                None => bail!(MyError::Syntax("Unclosed square bracket".into())),
                Some(']') => State::Unquoted,
                Some('\\') => State::SquareBracketBackslash,
                Some(c) => {
                    cur.push(c);
                    State::SquareBracket
                }
            },
            State::SquareBracketBackslash => match c {
                None => bail!(MyError::Syntax("Unclosed square bracket".into())),
                Some(c @ ']') | Some(c @ '\\') => {
                    cur.push(c);
                    State::SquareBracket
                }
                Some(c) => {
                    cur.push('\\');
                    cur.push(c);
                    State::SquareBracket
                }
            },
        }
    }

    Ok(tokens)
}
