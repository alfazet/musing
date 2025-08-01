use std::fmt::Display;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MyError {
    #[error("FileError: {0}")]
    File(String),
    #[error("SyntaxError: {0}")]
    Syntax(String),
}
