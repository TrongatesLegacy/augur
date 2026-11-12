//! The calls file: Augur's calls as JSON Lines, one call per line with its fighters as seen before the bell.
//! `fixtures/calls.jsonl` (made-up test calls) uses this format, plus the model's "expected" numbers, which import ignores.

use crate::db::{self, CallFighter, NewCall};
use crate::error::Error;
use crate::model::Settings;
use crate::site::{Duel, Recent};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
pub struct Line {
    pub call: i64,
    pub seen_at: i64,
    pub duel: Option<u64>,
    pub home_owner: String,
    pub away_owner: String,
    pub pick: String,
    pub prob: f64,
    pub winner: Option<String>,
    pub species_rec: [[f64; 2]; 2],
    pub pal_h2h: [f64; 2],
    pub fighters: HashMap<String, CallFighter>,
    pub model: Settings,
    /// the stake Augur suggested and its chat command (calls made before stakes have none)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stake: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stake_cmd: Option<String>,
}

/// Writes every settled call with full fighter data. Returns how many.
pub fn export(conn: &Connection, path: &Path) -> crate::Result<usize> {
    let mut out = String::new();
    let mut n = 0;
    for c in db::settled(conn)? {
        let (Some((home, away)), Some(species_rec)) = (c.fighters, c.species_rec) else { continue };
        let (model, stake_cmd): (Option<String>, Option<String>) = conn
            .query_row("SELECT model, stake_cmd FROM predictions WHERE id = ?", [c.id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap_or_default();
        let model: Settings = model.and_then(|m| serde_json::from_str(&m).ok()).unwrap_or_default();
        let line = Line {
            call: c.id,
            seen_at: c.seen_at,
            duel: c.duel_id,
            home_owner: c.home_owner,
            away_owner: c.away_owner,
            pick: c.pick,
            prob: c.prob,
            winner: c.winner,
            species_rec,
            pal_h2h: c.pal_h2h,
            fighters: HashMap::from([("home".to_string(), home), ("away".to_string(), away)]),
            model,
            stake: c.stake,
            stake_cmd,
        };
        out.push_str(&serde_json::to_string(&line)?);
        out.push('\n');
        n += 1;
    }
    std::fs::write(path, out)?;
    Ok(n)
}

/// Loads calls into the store, skipping ones already there (same owners and seen_at). Each call is settled from its
/// recorded winner. Returns how many were added.
pub fn import(conn: &Connection, path: &Path) -> crate::Result<usize> {
    let text = std::fs::read_to_string(path)?;
    let mut added = 0;
    for (i, raw) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
        let l: Line = serde_json::from_str(raw).map_err(|e| Error::Parse(format!("line {} of the calls file: {e}", i + 1)))?;
        let exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM predictions WHERE home_owner = ?1 AND away_owner = ?2 AND seen_at = ?3",
            rusqlite::params![l.home_owner, l.away_owner, l.seen_at],
            |r| r.get(0),
        )?;
        if exists > 0 {
            continue;
        }
        let (Some(home), Some(away)) = (l.fighters.get("home").cloned(), l.fighters.get("away").cloned()) else { continue };
        let call = NewCall {
            seen_at: l.seen_at.max(0) as u64,
            home_owner: l.home_owner.clone(),
            away_owner: l.away_owner.clone(),
            home_pal: home.fighter.pal.clone(),
            away_pal: away.fighter.pal.clone(),
            pick: l.pick,
            prob: l.prob,
            p_stats: 0.0,
            p_species: 0.0,
            p_h2h: 0.0,
            estimated: false,
            species_rec: l.species_rec,
            pal_h2h: l.pal_h2h,
            model: l.model,
            home,
            away,
            picks_open: true,
        };
        let id = db::add_call(conn, &call)?;
        if let Some(amount) = l.stake {
            db::set_call_stake(conn, id, amount, l.stake_cmd.as_deref().unwrap_or(""))?;
        }
        if let (Some(duel), Some(winner)) = (l.duel, l.winner) {
            let recent = [Recent { id: duel, home: l.home_owner, away: l.away_owner, win: winner.clone(), at: l.seen_at.max(0) as u64 }];
            db::settle(conn, &recent, |_| Some(Duel { winner: Some(winner.clone()), ..Default::default() }))?;
            let _ = id;
        }
        added += 1;
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_imports_and_exports_round_trip() {
        let conn = db::open_memory().unwrap();
        let fixture = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/calls.jsonl"));
        let n = import(&conn, fixture).unwrap();
        assert!(n >= 19);
        assert_eq!(import(&conn, fixture).unwrap(), 0, "importing twice adds nothing");
        let done = db::settled(&conn).unwrap();
        assert_eq!(done.len(), n);
        assert_eq!(done.iter().filter(|c| c.right()).count(), 14, "14 of the seed's 19 calls were right");
        let out = std::env::temp_dir().join(format!("augur-calls-{}.jsonl", std::process::id()));
        assert_eq!(export(&conn, &out).unwrap(), n);
        let _ = std::fs::remove_file(out);
    }
}
