//! The calls file: Augur's calls as JSON Lines, one call per line with its fighters as seen before the bell.
//! `fixtures/calls.jsonl` (made-up test calls) uses this format, plus the model's "expected" numbers, which import ignores.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};

use crate::db::{self, CallFighter, NewCall};
use crate::error::Error;
use crate::model::Settings;
use crate::stake::StakeSettings;

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
    /// the staking settings the stake came from
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staking: Option<StakeSettings>,
    /// overwritten after the fight, so left out of the money comparison; carried so every computer agrees
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub damaged: bool,
    /// how the call came about, so a call made on another computer can be looked into here
    #[serde(flatten)]
    pub details: Details,
}

/// What a call's line says beyond the call itself: each left out when it isn't known (older calls, older files).
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Details {
    /// some stats were hidden and filled in (from the owner's box, or guessed)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub estimated: bool,
    /// each factor's home win chance: stats, species records, the pals' head to head
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factors: Option<[f64; 3]>,
    /// when picks were seen to close (site clock, ms)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<i64>,
    /// the call made again after the bell, once the stats were shown
    #[serde(skip_serializing_if = "Option::is_none")]
    pub late_pick: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub late_prob: Option<f64>,
    /// where each side's stats came from ("home: shown; away: guessed")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
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
        let details = details(conn, c.id)?;
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
            staking: c.staking,
            damaged: c.damaged,
            details,
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
    import_all(conn, path).map(|m| m.calls)
}

/// `import`, also counting the calls already here that gained something they were missing (`filled`): a call
/// already here keeps what it has and only fills its blanks, such as pal ranks read on another computer.
pub fn import_all(conn: &Connection, path: &Path) -> crate::Result<db::Merged> {
    let text = std::fs::read_to_string(path)?;
    // one write at a time: two copies of Augur importing the same file at once could each find a call missing and
    // both store it
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let m = import_lines(&tx, &text)?;
    tx.commit()?;
    Ok(m)
}

fn import_lines(conn: &Connection, text: &str) -> crate::Result<db::Merged> {
    let (mut added, mut filled) = (0, 0);
    for (i, raw) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
        let l: Line = serde_json::from_str(raw).map_err(|e| Error::Parse(format!("line {} of the calls file: {e}", i + 1)))?;
        if db::call_at(conn, &l.home_owner, &l.away_owner, l.seen_at)?.is_some() {
            // a call already here: the damaged mark carries over (it's only ever set, never cleared), and its
            // fighters' blanks are filled
            if l.damaged {
                conn.execute(
                    "UPDATE predictions SET damaged = 1 WHERE home_owner = ?1 AND away_owner = ?2 AND seen_at = ?3",
                    rusqlite::params![l.home_owner, l.away_owner, l.seen_at],
                )?;
            }
            let id = db::call_at(conn, &l.home_owner, &l.away_owner, l.seen_at)?.unwrap_or_default();
            let mut gained = fill_details(conn, id, &l.details)?;
            for (side, f) in &l.fighters {
                gained |= db::fill_fighter_blanks(conn, (&l.home_owner, &l.away_owner, l.seen_at), side, f)?;
            }
            filled += usize::from(gained);
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
            stats_from: String::new(),
            version: String::new(),
        };
        let id = db::insert_call(conn, &call)?;
        fill_details(conn, id, &l.details)?;
        if l.damaged {
            conn.execute("UPDATE predictions SET damaged = 1 WHERE id = ?", [id])?;
        }
        if let Some(amount) = l.stake {
            db::set_call_stake(conn, id, amount, l.stake_cmd.as_deref().unwrap_or(""), l.staking.as_ref())?;
        }
        if let (Some(duel), Some(winner)) = (l.duel, l.winner) {
            db::settle_call(conn, id, duel, &l.home_owner, &l.away_owner, &winner)?;
        }
        added += 1;
    }
    Ok(db::Merged { calls: added, filled, ..Default::default() })
}

fn details(conn: &Connection, id: i64) -> crate::Result<Details> {
    Ok(conn.query_row(
        "SELECT COALESCE(estimated, 0), p_stats, p_species, p_h2h, closed_at, late_pick, late_prob, stats_from, app_version
         FROM predictions WHERE id = ?",
        [id],
        |r| {
            let factors = [r.get::<_, Option<f64>>(1)?, r.get(2)?, r.get(3)?];
            let text = |i: usize| r.get::<_, Option<String>>(i).map(|s| s.filter(|s| !s.is_empty()));
            Ok(Details {
                estimated: r.get::<_, i64>(0)? == 1,
                // all three 0: not known (a call imported from a file without them)
                factors: match factors {
                    [Some(a), Some(b), Some(c)] if [a, b, c].iter().any(|v| *v != 0.0) => Some([a, b, c]),
                    _ => None,
                },
                closed_at: r.get(4)?,
                late_pick: text(5)?,
                late_prob: r.get(6)?,
                stats_from: text(7)?,
                app_version: text(8)?,
            })
        },
    )?)
}

/// Fills in what a call here doesn't know yet from another computer's line, keeping what it has. Whether it gained
/// anything.
fn fill_details(conn: &Connection, id: i64, d: &Details) -> crate::Result<bool> {
    let [ps, psp, ph] = d.factors.map_or([None; 3], |f| f.map(Some));
    let n = conn.execute(
        "UPDATE predictions SET
             estimated = MAX(COALESCE(estimated, 0), ?2),
             p_stats = CASE WHEN ?3 IS NOT NULL AND COALESCE(p_stats, 0) = 0 AND COALESCE(p_species, 0) = 0 AND COALESCE(p_h2h, 0) = 0
                            THEN ?3 ELSE p_stats END,
             p_species = CASE WHEN ?3 IS NOT NULL AND COALESCE(p_stats, 0) = 0 AND COALESCE(p_species, 0) = 0 AND COALESCE(p_h2h, 0) = 0
                              THEN ?4 ELSE p_species END,
             p_h2h = CASE WHEN ?3 IS NOT NULL AND COALESCE(p_stats, 0) = 0 AND COALESCE(p_species, 0) = 0 AND COALESCE(p_h2h, 0) = 0
                          THEN ?5 ELSE p_h2h END,
             closed_at = COALESCE(closed_at, ?6),
             late_pick = COALESCE(late_pick, ?7),
             late_prob = CASE WHEN late_pick IS NULL THEN ?8 ELSE late_prob END,
             stats_from = COALESCE(NULLIF(stats_from, ''), ?9),
             app_version = COALESCE(NULLIF(app_version, ''), ?10)
         WHERE id = ?1 AND (
             (?2 = 1 AND COALESCE(estimated, 0) = 0)
             OR (?3 IS NOT NULL AND COALESCE(p_stats, 0) = 0 AND COALESCE(p_species, 0) = 0 AND COALESCE(p_h2h, 0) = 0)
             OR (?6 IS NOT NULL AND closed_at IS NULL)
             OR (?7 IS NOT NULL AND late_pick IS NULL)
             OR (?9 IS NOT NULL AND COALESCE(stats_from, '') = '')
             OR (?10 IS NOT NULL AND COALESCE(app_version, '') = ''))",
        rusqlite::params![id, d.estimated as i64, ps, psp, ph, d.closed_at, d.late_pick, d.late_prob, d.stats_from, d.app_version],
    )?;
    Ok(n > 0)
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

    /// The staking settings a call's stake came from travel with it to another computer.
    #[test]
    fn staking_travels_with_the_calls() {
        let fixture = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/calls.jsonl"));
        let mac = db::open_memory().unwrap();
        import(&mac, fixture).unwrap();
        let max = StakeSettings { strategy: crate::stake::Strategy::DefaultOrMax, skip_below: Some(0.65), ..Default::default() };
        mac.execute("UPDATE predictions SET stake = 5000, stake_cmd = '!h all', staking = ?", [serde_json::to_string(&max).unwrap()])
            .unwrap();
        let out = std::env::temp_dir().join(format!("augur-staking-{}.jsonl", std::process::id()));
        export(&mac, &out).unwrap();
        let pc = db::open_memory().unwrap();
        import(&pc, &out).unwrap();
        let _ = std::fs::remove_file(out);
        let staked: Vec<_> = db::settled(&pc).unwrap().into_iter().filter(|c| c.stake.is_some()).collect();
        assert!(!staked.is_empty() && staked.iter().all(|c| c.staking.as_ref() == Some(&max)), "every staked call keeps its settings");
    }

    /// The pal ranks a call read travel with it: to the calls file and into another computer's store.
    #[test]
    fn pal_ranks_travel_with_the_calls() {
        let fixture = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/calls.jsonl"));
        let mac = db::open_memory().unwrap();
        import(&mac, fixture).unwrap();
        let id = db::settled(&mac).unwrap()[0].id;
        mac.execute("UPDATE fighters SET tier = 'C', pal_rank = 154, rank_of = 357, power = 55 WHERE prediction_id = ?", [id]).unwrap();
        let rank = |conn: &Connection| {
            let c = db::settled(conn).unwrap().into_iter().find(|c| c.fighters.as_ref().unwrap().0.fighter.pal_rank.is_some());
            c.map(|c| c.fighters.unwrap().1.fighter.pal_rank.unwrap())
        };
        assert_eq!(rank(&mac), Some(crate::model::PalRank { tier: "C".into(), rank: 154, of: 357, power: 55 }), "read back from the store");
        let out = std::env::temp_dir().join(format!("augur-ranks-{}.jsonl", std::process::id()));
        export(&mac, &out).unwrap();
        let pc = db::open_memory().unwrap();
        import(&pc, &out).unwrap();
        assert_eq!(rank(&pc), rank(&mac), "through the calls file");
        // a computer that already had the calls, without ranks, fills them in from the same file
        let older = db::open_memory().unwrap();
        import(&older, fixture).unwrap();
        let m = import_all(&older, &out).unwrap();
        assert_eq!((m.calls, m.filled), (0, 1), "nothing added, the ranked call filled in");
        assert_eq!(rank(&older), rank(&mac));
        assert_eq!(import_all(&older, &out).unwrap().filled, 0, "nothing left to fill");
        let _ = std::fs::remove_file(out);
    }

    /// How a call came about (hidden stats, its factors, when picks closed, the pick after the bell, where the stats
    /// came from, the app version) travels with it, and fills in a computer that already had the call without it.
    #[test]
    fn how_a_call_came_about_travels_with_it() {
        let fixture = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/calls.jsonl"));
        let mac = db::open_memory().unwrap();
        import(&mac, fixture).unwrap();
        let id = db::settled(&mac).unwrap()[0].id;
        mac.execute(
            "UPDATE predictions SET estimated = 1, p_stats = 0.7, p_species = 0.4, p_h2h = 0.5, closed_at = seen_at + 300000,
                 late_pick = 'away', late_prob = 0.61, stats_from = 'home: shown; away: guessed', app_version = '0.1.21' WHERE id = ?",
            [id],
        )
        .unwrap();
        let want = details(&mac, id).unwrap();
        assert!(want.estimated && want.factors == Some([0.7, 0.4, 0.5]) && want.late_pick.as_deref() == Some("away"));
        let out = std::env::temp_dir().join(format!("augur-details-{}.jsonl", std::process::id()));
        export(&mac, &out).unwrap();
        let key = |conn: &Connection| {
            conn.query_row("SELECT home_owner, away_owner, seen_at FROM predictions WHERE id = ?", [id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
            })
            .unwrap()
        };
        let (h, a, at) = key(&mac);
        let found = |conn: &Connection| details(conn, db::call_at(conn, &h, &a, at).unwrap().unwrap()).unwrap();
        // a computer that didn't have the call
        let pc = db::open_memory().unwrap();
        import(&pc, &out).unwrap();
        assert_eq!(found(&pc), want);
        // one that had it from an older file, without any of it
        let older = db::open_memory().unwrap();
        import(&older, fixture).unwrap();
        assert_eq!(found(&older), Details::default());
        assert_eq!(import_all(&older, &out).unwrap().filled, 1);
        assert_eq!(found(&older), want);
        assert_eq!(import_all(&older, &out).unwrap().filled, 0, "nothing left to fill");
        // what a call already has stays: the other computer's line only fills blanks
        let mine = db::open_memory().unwrap();
        import(&mine, fixture).unwrap();
        mine.execute("UPDATE predictions SET app_version = '0.1.20'", []).unwrap();
        import_all(&mine, &out).unwrap();
        assert_eq!(found(&mine).app_version.as_deref(), Some("0.1.20"));
        // a line from before any of this leaves it all out
        let plain = std::fs::read_to_string(fixture).unwrap();
        let first: serde_json::Value = serde_json::from_str(plain.lines().next().unwrap()).unwrap();
        assert!(first.get("estimated").is_none() && first.get("factors").is_none());
        let _ = std::fs::remove_file(out);
    }

    /// A damaged mark set on one computer reaches the others: on a call they don't have yet, and on one they do.
    #[test]
    fn the_damaged_mark_travels_with_the_calls() {
        let fixture = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/calls.jsonl"));
        let (mac, windows) = (db::open_memory().unwrap(), db::open_memory().unwrap());
        import(&mac, fixture).unwrap();
        import(&windows, fixture).unwrap();
        let id = db::settled(&mac).unwrap()[3].id;
        mac.execute("UPDATE predictions SET damaged = 1 WHERE id = ?", [id]).unwrap();
        let out = std::env::temp_dir().join(format!("augur-damaged-{}.jsonl", std::process::id()));
        export(&mac, &out).unwrap();
        assert_eq!(std::fs::read_to_string(&out).unwrap().matches("\"damaged\":true").count(), 1, "only the marked call says so");
        let damaged = |conn: &Connection| db::settled(conn).unwrap().iter().filter(|c| c.damaged).count();
        assert_eq!((import(&windows, &out).unwrap(), damaged(&windows)), (0, 1), "a call it already had gets the mark, nothing is added");
        let fresh = db::open_memory().unwrap();
        import(&fresh, &out).unwrap();
        assert_eq!(damaged(&fresh), 1, "a call it didn't have arrives marked");
        let _ = std::fs::remove_file(out);
    }

    /// Another computer's call on the fight that's up here arrives while this computer's own call is still open:
    /// it's stored as its own call, and the open one stays as it is.
    #[test]
    fn a_synced_call_leaves_the_open_call_alone() {
        let fixture = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/calls.jsonl"));
        let mac = db::open_memory().unwrap();
        import(&mac, fixture).unwrap();
        let theirs = db::settled(&mac).unwrap().pop().unwrap();
        let out = std::env::temp_dir().join(format!("augur-open-{}.jsonl", std::process::id()));
        export(&mac, &out).unwrap();
        // the PC has the same fight up, read a little later, still open, picked the other way
        let pc = db::open_memory().unwrap();
        let (home, away) = db::settled_call(&mac, theirs.id).unwrap().unwrap().fighters.unwrap();
        let ours = NewCall {
            seen_at: theirs.seen_at as u64 + 5_000,
            home_owner: theirs.home_owner.clone(),
            away_owner: theirs.away_owner.clone(),
            home_pal: theirs.home_pal.clone(),
            away_pal: theirs.away_pal.clone(),
            pick: if theirs.pick == "home" { "away".into() } else { "home".into() },
            prob: 0.55,
            p_stats: 0.5,
            p_species: 0.5,
            p_h2h: 0.5,
            estimated: false,
            species_rec: [[1.0, 1.0], [1.0, 1.0]],
            pal_h2h: [0.0, 0.0],
            model: Settings::default(),
            home,
            away,
            picks_open: true,
            stats_from: String::new(),
            version: String::new(),
        };
        let open = db::add_call(&pc, &ours).unwrap();
        let n = import(&pc, &out).unwrap();
        assert_eq!(import(&pc, &out).unwrap(), 0, "importing twice adds nothing");
        let (status, pick): (String, String) =
            pc.query_row("SELECT status, pick FROM predictions WHERE id = ?", [open], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((status.as_str(), pick), ("open", ours.pick.clone()), "the open call is untouched");
        let done = db::settled(&pc).unwrap();
        assert_eq!(done.len(), n, "every synced call arrived, the one on the open fight too");
        assert!(done.iter().any(|c| c.seen_at == theirs.seen_at && c.pick == theirs.pick));
        let _ = std::fs::remove_file(out);
    }
}
