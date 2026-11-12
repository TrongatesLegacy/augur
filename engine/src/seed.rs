//! The seeded store every test uses: 19 made-up calls (fixtures/calls.jsonl) plus saved palduels.com pages
//! (engine/tests/pages): Bushi Noct's species page and duel #4702 (Chillet Lux vs Chikipi Ryu) with its fight tape.
//! The picks pages (picks-page-1..3.html) are made-up bets on the seed's calls in the signed-in picks list's markup;
//! `bets` tests import them.
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

/// When the seed's picks list was read (ms): after every seeded call.
pub const BETS_READ_AT: u64 = 1_794_400_000_000;

/// Imports the seed's picks list (picks-page-1..3.html) into a store.
pub fn import_bets(conn: &Connection) {
    let stop = std::sync::atomic::AtomicBool::new(false);
    let ctl = crate::backfill::Control { stop: &stop, busy: &|| false, progress: &|_| {} };
    let mut pages = |n: u32, _: &mut crate::backfill::Progress| Some(page(&format!("picks-page-{n}.html")));
    crate::bets::import(conn, &mut pages, &ctl, BETS_READ_AT).unwrap();
}

/// An in-memory store with the seed loaded.
pub fn store() -> Connection {
    let conn = db::open_memory().unwrap();
    calls::import(&conn, &root().join("fixtures/calls.jsonl")).unwrap();
    db::save_species(&conn, &site::parse_species(&page("pal-bushi-noct.html"), "bushi-noct")).unwrap();
    db::save_duel(&conn, 4702, &site::parse_duel(&page("duel-4702.html"))).unwrap();
    // the calls file has no dates: the seed's stream was the evening of 11 Nov 2026, a fight every seven minutes
    conn.execute(
        "UPDATE duels SET fought_at = strftime('%Y-%m-%dT%H:%M:%SZ', '2026-11-11 20:00:00', '+' || ((id - 9001) * 7) || ' minutes')
         WHERE fought_at IS NULL AND id BETWEEN 9001 AND 9019",
        [],
    )
    .unwrap();
    conn
}
