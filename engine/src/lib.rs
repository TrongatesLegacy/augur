//! Augur's engine: everything that isn't the desktop app. Fetching and parsing palduels.com (`site`), the
//! prediction model (`model`), the SQLite store (`db`), a whole prediction from the live state (`predict`), the
//! gentle background backfill (`backfill`), the chat voices (`voices`) and the calls file (`calls`).

pub mod error;
pub mod model;

pub mod backfill;
pub mod calls;
pub mod db;
pub mod history;
pub mod predict;
pub mod site;
pub mod voices;

/// The store's SQLite binding, for callers that hold a connection.
pub use rusqlite;
pub mod seed;

pub use error::{Error, Result};
