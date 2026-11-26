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
    /// each version of the call while its fight was up (`revisions`); rebuilt ones stay on the computer that
    /// rebuilt them, since every computer can rebuild them
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub revisions: Vec<crate::revisions::Revision>,
    /// the stream read for a hidden side: what it found and the frame's name (the frames go apart: see `sync`)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stream_reads: Vec<crate::stream::StreamRead>,
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
        // a call from a computer that didn't keep its history: what can be rebuilt of it
        crate::revisions::backfill_call(conn, id)?;
        added += 1;
    }
    Ok(db::Merged { calls: added, filled, ..Default::default() })
}

pub(crate) fn details(conn: &Connection, id: i64) -> crate::Result<Details> {
    let revisions: Vec<_> = crate::revisions::of_call(conn, id)?;
    let revisions = if revisions.iter().any(|r| r.backfilled()) { vec![] } else { revisions };
    let mut d = conn.query_row(
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
                revisions: vec![],
                stream_reads: vec![],
            })
        },
    )?;
    d.revisions = revisions;
    // (a backup from before the stream was read hasn't the table)
    d.stream_reads = crate::stream::of_call(conn, id).unwrap_or_default();
    Ok(d)
}

/// Fills in what a call here doesn't know yet from another computer's line, keeping what it has. Whether it gained
/// anything.
pub(crate) fn fill_details(conn: &Connection, id: i64, d: &Details) -> crate::Result<bool> {
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
    let history = crate::revisions::merge(conn, id, &d.revisions)?;
    let reads = crate::stream::merge(conn, id, &d.stream_reads)?;
    Ok(n > 0 || history || reads)
}

/// Brings in what a backup (`other`, opened on its own) knows about its calls beyond what `db::merge_from` copies:
/// for each of its calls that's here (the same fight seen at the same moment), its details, history and stream
/// reads, its marks (overwritten after the fight, closed, first called after picks closed) and staking. Then its auto
/// bets, tied to the calls here. A backup from an older Augur lacks some of these, and they're skipped. Returns how
/// many calls that were here already (ids up to `before`) gained something, and how many auto bets came in.
pub fn merge_backup(conn: &Connection, other: &Connection, before: i64) -> crate::Result<(usize, usize)> {
    let theirs: Vec<(i64, String, String, i64)> = other
        .prepare("SELECT id, home_owner, away_owner, seen_at FROM predictions WHERE seen_at > 0")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut ids = HashMap::new();
    let mut gained = 0;
    for (theirs_id, home, away, seen_at) in theirs {
        let Some(here) = db::call_at(conn, &home, &away, seen_at)? else { continue };
        ids.insert(theirs_id, here);
        let mut more = details(other, theirs_id).ok().map(|d| fill_details(conn, here, &d)).transpose()?.unwrap_or(false);
        let marks: Option<(i64, i64, i64, Option<String>)> = other
            .query_row(
                "SELECT COALESCE(damaged, 0), COALESCE(closed, 0), COALESCE(late_only, 0), staking FROM predictions WHERE id = ?",
                [theirs_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .ok();
        if let Some((damaged, closed, late_only, staking)) = marks {
            more |= conn.execute(
                "UPDATE predictions SET damaged = MAX(COALESCE(damaged, 0), ?2), closed = MAX(COALESCE(closed, 0), ?3),
                     late_only = MAX(COALESCE(late_only, 0), ?4), staking = COALESCE(staking, ?5)
                 WHERE id = ?1 AND (COALESCE(damaged, 0) < ?2 OR COALESCE(closed, 0) < ?3 OR COALESCE(late_only, 0) < ?4
                     OR (staking IS NULL AND ?5 IS NOT NULL))",
                rusqlite::params![here, damaged, closed, late_only, staking],
            )? > 0;
        }
        gained += usize::from(more && here <= before);
    }
    let mut bets = 0;
    for b in crate::autobet::all(other).unwrap_or_default() {
        let call = b.prediction_id.and_then(|id| ids.get(&id).copied());
        // one tied to a call here that already has its auto bet is that bet
        if call.is_some_and(|id| crate::autobet::for_call(conn, id).ok().flatten().is_some()) {
            continue;
        }
        bets += usize::from(crate::autobet::insert_copy(conn, &b, call, "backup")?);
    }
    Ok((gained, bets))
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

    /// A call's history travels with it to another computer: a live one replaces the history rebuilt there, and a
    /// rebuilt one stays where it was rebuilt.
    #[test]
    fn the_history_travels_with_the_calls() {
        use crate::revisions::{self, Revision};
        let fixture = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/calls.jsonl"));
        let pc = db::open_memory().unwrap();
        import(&pc, fixture).unwrap();
        let calls = db::settled(&pc).unwrap();
        let (live, rebuilt) = (calls[0].id, calls[1].id);
        assert!(revisions::of_call(&pc, rebuilt).unwrap().iter().all(Revision::backfilled), "an imported call without one gets it rebuilt");
        let history = vec![
            Revision {
                n: 1,
                at: 1_000,
                phase: "picks".into(),
                trigger: "seen".into(),
                cause: "first".into(),
                pick: "home".into(),
                prob: 0.7,
                ..Default::default()
            },
            Revision {
                n: 2,
                at: 9_000,
                phase: "picks".into(),
                trigger: "bet".into(),
                cause: "bet".into(),
                pick: "home".into(),
                prob: 0.7,
                ..Default::default()
            },
        ];
        assert!(revisions::merge(&pc, live, &history).unwrap());
        let out = std::env::temp_dir().join(format!("augur-history-{}.jsonl", std::process::id()));
        export(&pc, &out).unwrap();
        let text = std::fs::read_to_string(&out).unwrap();
        assert_eq!(text.matches("\"revisions\"").count(), 1, "only the live history goes in the file");
        // a computer with the same calls, their histories rebuilt there
        let mac = db::open_memory().unwrap();
        import(&mac, fixture).unwrap();
        let m = import_all(&mac, &out).unwrap();
        let _ = std::fs::remove_file(out);
        assert_eq!((m.calls, m.filled), (0, 1));
        let here = |seen_at: i64| db::call_at(&mac, &calls[0].home_owner, &calls[0].away_owner, seen_at).unwrap().unwrap();
        assert_eq!(revisions::of_call(&mac, here(calls[0].seen_at)).unwrap(), history);
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

    /// A backup brings in more than its calls: their history, stream reads and frames, marks, staking and auto bets.
    #[test]
    fn a_backup_brings_everything_its_calls_know() {
        use crate::stream::{self, StreamRead};
        let theirs = crate::seed::store();
        let c = db::settled(&theirs).unwrap().pop().unwrap();
        // a history kept live (the seed's are rebuilt ones, which every computer can rebuild for itself)
        theirs.execute("DELETE FROM call_revisions WHERE prediction_id = ?", [c.id]).unwrap();
        crate::revisions::record(&theirs, c.id, crate::revisions::Revision { pick: c.pick.clone(), prob: c.prob, ..Default::default() })
            .unwrap();
        let name = stream::frame_name(9, stream::READ);
        let read = StreamRead { at: 9, outcome: stream::READ.into(), side: "home".into(), shot: Some(name.clone()), ..Default::default() };
        stream::record(&theirs, c.id, &read).unwrap();
        stream::save_frame(&theirs, &name, b"jpeg").unwrap();
        theirs
            .execute(
                "UPDATE predictions SET damaged = 1, stats_from = 'home: read from the stream; away: shown', closed_at = 77 WHERE id = ?",
                [c.id],
            )
            .unwrap();
        let bet = crate::autobet::AutoBet {
            prediction_id: Some(c.id),
            home_owner: c.home_owner.clone(),
            away_owner: c.away_owner.clone(),
            side: c.pick.clone(),
            amount: 500,
            command: "!h".into(),
            status: "confirmed".into(),
            ..Default::default()
        };
        crate::autobet::claim(&theirs, &bet).unwrap();
        let file = std::env::temp_dir().join(format!("augur-backup-test-{}.db", std::process::id()));
        db::backup_to(&theirs, &file).unwrap();

        let here = db::open_memory().unwrap();
        let m = db::merge_from(&here, &file).unwrap();
        let id = db::call_at(&here, &c.home_owner, &c.away_owner, c.seen_at).unwrap().unwrap();
        assert_eq!((m.frames, m.auto_bets), (1, 1));
        assert_eq!(stream::of_call(&here, id).unwrap(), [read]);
        assert_eq!(stream::frame(&here, &name).unwrap().as_deref(), Some(&b"jpeg"[..]));
        assert_eq!(crate::revisions::of_call(&here, id).unwrap().len(), 1, "its history");
        let (damaged, from, closed): (i64, String, i64) = here
            .query_row("SELECT damaged, stats_from, closed_at FROM predictions WHERE id = ?", [id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!((damaged, from.as_str(), closed), (1, "home: read from the stream; away: shown", 77));
        let b = crate::autobet::for_call(&here, id).unwrap().unwrap();
        assert_eq!((b.status.as_str(), b.device.as_deref()), ("confirmed", Some("backup")), "never sent out as this computer's own");
        // merging it again adds nothing
        let again = db::merge_from(&here, &file).unwrap();
        assert_eq!((again.calls, again.frames, again.auto_bets), (0, 0, 0));
        assert_eq!(stream::of_call(&here, id).unwrap().len(), 1);
        let _ = std::fs::remove_file(file);
    }
}
