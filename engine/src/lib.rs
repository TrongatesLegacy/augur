//! Augur's engine: everything that isn't the desktop app. Fetching and parsing palduels.com (`site`), the
//! prediction model (`model`), the SQLite store (`db`), a whole prediction from the live state (`predict`), the
//! gentle background backfill (`backfill`), the chat voices (`voices`), the calls file (`calls`), and the developer's
//! bets (`bets`) with the stake Augur suggests (`stake`), whether a pal's IVs are worth rerolling (`reroll`), and sync
//! across computers (`sync`).

pub mod error;
pub mod model;

pub mod backfill;
pub mod bets;
pub mod calls;
pub mod db;
pub mod history;
pub mod learn;
pub mod predict;
pub mod reroll;
pub mod site;
pub mod stake;
pub mod sync;
pub mod voices;

/// The store's SQLite binding, for callers that hold a connection.
pub use rusqlite;
pub mod seed;

pub use error::{Error, Result};
