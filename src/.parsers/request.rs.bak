use anyhow::{Result, bail};
use std::mem;

// based on https://github.com/tmiasko/shell-words/
// modified to include square brackets as a "verbatim" delimiter
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
                None => bail!("unclosed single quote"),
                Some('\'') => State::Unquoted,
                Some(c) => {
                    cur.push(c);
                    State::SingleQuoted
                }
            },
            State::DoubleQuoted => match c {
                None => bail!("unclosed double quote"),
                Some('\"') => State::Unquoted,
                Some('\\') => State::DoubleQuotedBackslash,
                Some(c) => {
                    cur.push(c);
                    State::DoubleQuoted
                }
            },
            State::DoubleQuotedBackslash => match c {
                None => bail!("unclosed double quote"),
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
                None => bail!("unclosed square bracket"),
                Some(']') => State::Unquoted,
                Some('\\') => State::SquareBracketBackslash,
                Some(c) => {
                    cur.push(c);
                    State::SquareBracket
                }
            },
            State::SquareBracketBackslash => match c {
                None => bail!("unclosed square bracket"),
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn filter_expr() {
        let res = tokenize(
            r#"select [(albumartist=="ILLENIUM" & album!="a") | tracktitle=="e"] date,tracktitle"#,
        )
        .unwrap();
        assert_eq!(
            res,
            vec![
                "select".to_string(),
                r#"(albumartist=="ILLENIUM" & album!="a") | tracktitle=="e""#.to_string(),
                "date,tracktitle".to_string()
            ]
        );
    }
}
