//! The engine's one error type. It turns into text only at the edges: the app's commands and the command line.

use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// palduels.com answered with this HTTP status.
    Http(u16),
    /// No answer from palduels.com: network down, timeout, TLS.
    Net(String),
    /// The store (SQLite).
    Db(rusqlite::Error),
    /// A file, e.g. a calls file being imported or exported.
    Io(std::io::Error),
    /// Something that should have been JSON or a known format wasn't.
    Parse(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Http(code) => write!(f, "palduels.com answered HTTP {code}"),
            Error::Net(e) => write!(f, "couldn't reach palduels.com: {e}"),
            Error::Db(e) => write!(f, "store: {e}"),
            Error::Io(e) => write!(f, "file: {e}"),
            Error::Parse(e) => write!(f, "couldn't read {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Db(e) => Some(e),
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Db(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Parse(e.to_string())
    }
}
