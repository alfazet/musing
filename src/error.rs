use std::fmt::{self, Display, Formatter};

#[derive(Debug)]
pub enum MyError {
    Audio(String),
    File(String),
    Syntax(String),
}

impl Display for MyError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Audio(e) => write!(f, "AudioError: {}", e),
            Self::File(e) => write!(f, "FileError: {}", e),
            Self::Syntax(e) => write!(f, "SyntaxError: {}", e),
        }
    }
}

impl std::error::Error for MyError {}
