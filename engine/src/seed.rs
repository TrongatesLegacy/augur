//! The seeded store every test uses: 19 made-up calls (fixtures/calls.jsonl) plus saved palduels.com pages
//! (engine/tests/pages): Bushi Noct's species page and duel #4702 (Chillet Lux vs Chikipi Ryu) with its fight tape.
//! No network, no local data, the same on every machine and in CI.

use crate::{calls, db, site};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// The repo root (this crate is <root>/engine).
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// A saved page from engine/tests/pages.
pub fn page(name: &str) -> String {
    std::fs::read_to_string(root().join("engine/tests/pages").join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// A saved live state.
pub fn state(name: &str) -> site::State {
    serde_json::from_str(&page(name)).unwrap()
}

/// An in-memory store with the seed loaded.
pub fn store() -> Connection {
    let conn = db::open_memory().unwrap();
    calls::import(&conn, &root().join("fixtures/calls.jsonl")).unwrap();
    db::save_species(&conn, &site::parse_species(&page("pal-bushi-noct.html"), "bushi-noct")).unwrap();
    db::save_duel(&conn, 4702, &site::parse_duel(&page("duel-4702.html"))).unwrap();
    conn
}
