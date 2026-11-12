//! The seeded store every test uses: 19 made-up calls (fixtures/calls.jsonl) plus saved palduels.com pages
//! (engine/tests/pages): Bushi Noct's species page and duel #4702 (Chillet Lux vs Chikipi Ryu) with its fight tape.
//! `tape` gives a seeded duel a made-up fight tape (the calls file has results only), for the duel breakdown.
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

/// Stores the seed's pet and Hall pages (pet.html, hall.html) the way the app keeps them.
pub fn store_pet_and_hall(conn: &Connection) {
    db::meta_set(conn, "pet_page", &serde_json::to_string(&site::parse_pet(&page("pet.html"))).unwrap()).unwrap();
    db::meta_set(conn, "hall_page", &serde_json::to_string(&site::parse_hall(&page("hall.html"))).unwrap()).unwrap();
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

/// Gives a seeded call's duel a made-up fight tape: both sides casting their listed moves every few seconds, hits
/// sized by move power and type edge, until the winner (the call's recorded result) knocks the other out. The same
/// on every run. Returns false when the duel has no call with fighters.
pub fn tape(conn: &Connection, duel_id: u64) -> bool {
    let Some(c) = db::settled(conn).unwrap().into_iter().find(|c| c.duel_id == Some(duel_id)) else { return false };
    let (Some((home, away)), Some(winner)) = (c.fighters, c.winner) else { return false };
    let s = crate::model::Settings::default();
    // a small fixed random sequence per duel
    let mut seed = duel_id.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
    let mut next = move || {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as f64 / (1u64 << 31) as f64
    };
    let ko = 24 + (duel_id % 7) as u32 * 5;
    let left = 12 + (duel_id % 9) as u32 * 7;
    let mut hits = vec![];
    let mut casts = [0u32; 2];
    for (i, (side, me, them)) in [("home", &home, &away), ("away", &away, &home)].into_iter().enumerate() {
        let moves: Vec<_> = me.fighter.skills.iter().filter(|m| m.lv <= 70).collect();
        let mut raw = vec![];
        let mut t = 1 + (next() * 3.0) as u32;
        while t < ko && !moves.is_empty() {
            let m = moves[(next() * moves.len() as f64) as usize % moves.len()];
            let size = m.power.powf(s.power_exp) * crate::model::type_mult(&s, &m.el, &them.fighter.types) * (0.75 + next() * 0.5);
            raw.push((t, m.name.clone(), size));
            casts[i] += 1;
            t += 3 + (next() * 4.0) as u32;
        }
        // a little damage over time from an unnamed source
        raw.push((ko / 2, String::new(), 1.5));
        // the winner's hits take all of the other side's HP (the last one at the K.O.), the loser's leave some
        let total = if side == winner { 100.0 } else { 100.0 - left as f64 };
        if side == winner {
            if let Some(last) = raw.iter_mut().filter(|h| !h.1.is_empty()).last() {
                last.0 = ko;
            }
        }
        let sum: f64 = raw.iter().map(|h| h.2).sum();
        let mut given = 0.0;
        let n = raw.len();
        for (j, (t, mv, size)) in raw.into_iter().enumerate() {
            let pct = if j + 1 == n { total - given } else { (size / sum * total).round() };
            given += pct;
            hits.push(site::Hit { side: side.into(), t, mv, pct: pct.max(0.0) });
        }
    }
    hits.sort_by_key(|h| h.t);
    let before = db::get_duel(conn, duel_id).unwrap().unwrap_or_default();
    let duel = site::Duel {
        hits,
        home_casts: casts[0],
        away_casts: casts[1],
        home_owner: Some(c.home_owner),
        away_owner: Some(c.away_owner),
        home_pal: Some(c.home_pal),
        away_pal: Some(c.away_pal),
        winner: Some(winner),
        ko: Some(format!("{}:{:02}", ko / 60, ko % 60)),
        winner_hp_left: Some(left),
        ..before
    };
    db::save_duel(conn, duel_id, &duel).unwrap();
    true
}
