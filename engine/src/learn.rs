//! The model tunes itself. Each setting is re-fitted on the evidence that can see it (past duels for types, move
//! power and records; real calls for the stat settings, which past duels can't see) and a change is kept only when it
//! predicts duels it wasn't fitted on better: the newest quarter of each set is held back to check.

use crate::db::{self, CallRecord};
use crate::error::Result as R;
use crate::history::{self, PastDuel};
use crate::model::Settings;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// A held-back check must improve the log loss by at least this much to keep a change (smaller is chance).
const MARGIN_PAST: f64 = 0.001;
const MARGIN_LIVE: f64 = 0.005;
/// Real calls needed before the stat settings learn from them (the first ones are too few to beat chance).
pub const MIN_CALLS: usize = 60;
/// Calls with a cooldown passive on one side needed before cooldowns learn.
pub const MIN_COOLDOWN_CALLS: usize = 15;
/// Changes kept in the log.
const LOG_LEN: usize = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    PastDuels,
    LiveCalls,
}

struct Knob {
    key: &'static str,
    source: Source,
    grid: &'static [f64],
}

const KNOBS: [Knob; 7] = [
    Knob { key: "TYPE_BONUS", source: Source::PastDuels, grid: &[1.0, 1.1, 1.25, 1.4, 1.5, 1.65, 1.8, 2.0, 2.25, 2.5] },
    Knob { key: "TYPE_RESIST", source: Source::PastDuels, grid: &[1.0, 1.1, 1.25, 1.4, 1.5, 1.65, 1.8, 2.0] },
    Knob { key: "POWER_EXP", source: Source::PastDuels, grid: &[0.0, 0.1, 0.2, 0.3, 0.35, 0.4, 0.5, 0.65, 0.8, 1.0] },
    Knob { key: "SPECIES_WEIGHT", source: Source::PastDuels, grid: &[0.0, 0.25, 0.5, 0.75, 1.0, 1.2, 1.4, 1.6, 2.0, 2.5] },
    Knob { key: "PAL_H2H_WEIGHT", source: Source::PastDuels, grid: &[0.0, 0.25, 0.5, 0.75, 1.0, 1.5] },
    Knob { key: "STATS_STEEPNESS", source: Source::LiveCalls, grid: &[1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 5.0, 6.0] },
    Knob { key: "COOLDOWN_EXP", source: Source::LiveCalls, grid: &[0.0, 0.25, 0.5, 0.75, 1.0] },
];

/// One setting after a learning run.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct KnobReport {
    pub key: String,
    pub value: f64,
    /// the value before this run
    pub was: f64,
    pub source: Source,
    /// "changed", "held" (re-tested, still best) or "waiting" (not enough evidence yet)
    pub status: String,
    /// for "waiting": how much evidence there is and how much it needs
    pub have: usize,
    pub need: usize,
}

/// Log loss on the held-back duels and calls, before and after the run.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Check {
    pub past_n: usize,
    pub past_before: f64,
    pub past_after: f64,
    pub past_right_before: f64,
    pub past_right_after: f64,
    pub live_n: usize,
    pub live_before: f64,
    pub live_after: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Learned {
    pub at_ms: u64,
    pub settings: Settings,
    /// evidence the run had: past duels and settled real calls
    pub past_duels: usize,
    pub live_calls: usize,
    pub check: Check,
    pub knobs: Vec<KnobReport>,
}

/// A change the learner kept.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Change {
    pub at_ms: u64,
    pub key: String,
    pub from: f64,
    pub to: f64,
    /// held-back log loss before and after, and on how many duels or calls
    pub loss: (f64, f64),
    pub n: usize,
}

fn has_cooldown(c: &CallRecord) -> bool {
    c.fighters.as_ref().is_some_and(|(h, a)| (h.fighter.speed - 1.0).abs() > 1e-9 || (a.fighter.speed - 1.0).abs() > 1e-9)
}

/// The best grid value for one setting, all else fixed. Near-ties (within a tenth of the margin) go to the value
/// closest to the current one, so the settings don't wander on noise.
fn best(knob: &Knob, s: &Settings, loss: &dyn Fn(&Settings) -> f64, margin: f64) -> f64 {
    let now = s.get(knob.key).unwrap_or_default();
    let scored: Vec<(f64, f64)> = knob.grid.iter().map(|v| (*v, loss(&s.with(knob.key, *v)))).collect();
    let low = scored.iter().map(|x| x.1).fold(f64::INFINITY, f64::min);
    let now_loss = loss(s);
    if now_loss <= low + margin / 10.0 {
        return now;
    }
    scored.iter().filter(|x| x.1 <= low + margin / 10.0).min_by(|a, b| (a.0 - now).abs().total_cmp(&(b.0 - now).abs())).map_or(now, |x| x.0)
}

/// Re-fit the settings, starting from `current`. Changes nothing in the database; `save` keeps the result.
pub fn learn(conn: &Connection, current: &Settings, now_ms: u64) -> R<Learned> {
    let past = history::past_duels(conn)?;
    let calls: Vec<CallRecord> = db::settled(conn)?.into_iter().filter(|c| c.fighters.is_some() && c.species_rec.is_some()).collect();
    Ok(learn_on(&past, &calls, current, now_ms))
}

pub fn learn_on(past: &[PastDuel], calls: &[CallRecord], current: &Settings, now_ms: u64) -> Learned {
    let (past_fit, past_check) = past.split_at(past.len() * 3 / 4);
    let (calls_fit, calls_check) = calls.split_at(calls.len() * 3 / 4);
    let past_loss = |set: &[PastDuel], s: &Settings| history::score_past(set, s).log_loss;
    let live_loss = |set: &[CallRecord], s: &Settings| history::evaluate(set, s).0;
    let cooldown_calls = calls.iter().filter(|c| has_cooldown(c)).count();
    let ready = |k: &Knob| match (k.source, k.key) {
        (Source::PastDuels, _) => past.len() >= 200,
        (Source::LiveCalls, "COOLDOWN_EXP") => calls.len() >= MIN_CALLS && cooldown_calls >= MIN_COOLDOWN_CALLS,
        (Source::LiveCalls, _) => calls.len() >= MIN_CALLS,
    };

    // coordinate descent: each setting in turn, a few rounds, on the fitting part only
    let mut cand = current.clone();
    for _ in 0..3 {
        for k in KNOBS.iter().filter(|k| ready(k)) {
            let v = match k.source {
                Source::PastDuels => best(k, &cand, &|s| past_loss(past_fit, s), MARGIN_PAST),
                Source::LiveCalls => best(k, &cand, &|s| live_loss(calls_fit, s), MARGIN_LIVE),
            };
            cand = cand.with(k.key, v);
        }
    }

    // keep each group's changes only if they do better on what was held back
    let moved = |src: Source| KNOBS.iter().any(|k| k.source == src && cand.get(k.key) != current.get(k.key));
    let mut out = current.clone();
    if moved(Source::PastDuels) {
        let trial =
            KNOBS.iter().filter(|k| k.source == Source::PastDuels).fold(out.clone(), |s, k| s.with(k.key, cand.get(k.key).unwrap()));
        let live_ok = calls_check.len() < 20 || live_loss(calls_check, &trial) <= live_loss(calls_check, &out) + MARGIN_LIVE;
        if past_loss(past_check, &trial) < past_loss(past_check, &out) - MARGIN_PAST && live_ok {
            out = trial;
        }
    }
    if moved(Source::LiveCalls) && calls_check.len() >= 15 {
        let trial =
            KNOBS.iter().filter(|k| k.source == Source::LiveCalls).fold(out.clone(), |s, k| s.with(k.key, cand.get(k.key).unwrap()));
        if live_loss(calls_check, &trial) < live_loss(calls_check, &out) - MARGIN_LIVE {
            out = trial;
        }
    }

    let before = history::score_past(past_check, current);
    let after = history::score_past(past_check, &out);
    let check = Check {
        past_n: past_check.len(),
        past_before: before.log_loss,
        past_after: after.log_loss,
        past_right_before: before.accuracy,
        past_right_after: after.accuracy,
        live_n: calls_check.len(),
        live_before: live_loss(calls_check, current),
        live_after: live_loss(calls_check, &out),
    };
    let knobs = KNOBS
        .iter()
        .map(|k| {
            let (value, was) = (out.get(k.key).unwrap(), current.get(k.key).unwrap());
            let (have, need) = match (k.source, k.key) {
                (Source::PastDuels, _) => (past.len(), 200),
                (Source::LiveCalls, "COOLDOWN_EXP") if calls.len() >= MIN_CALLS => (cooldown_calls, MIN_COOLDOWN_CALLS),
                (Source::LiveCalls, _) => (calls.len(), MIN_CALLS),
            };
            let status = if !ready(k) {
                "waiting"
            } else if value != was {
                "changed"
            } else {
                "held"
            };
            KnobReport { key: k.key.into(), value, was, source: k.source, status: status.into(), have, need }
        })
        .collect();
    Learned { at_ms: now_ms, settings: out, past_duels: past.len(), live_calls: calls.len(), check, knobs }
}

/// The last learning run, if there has been one.
pub fn load(conn: &Connection) -> Option<Learned> {
    db::meta_get(conn, "learned_model").ok().flatten().and_then(|v| serde_json::from_str(&v).ok())
}

/// The changes the learner has kept, newest first.
pub fn changes(conn: &Connection) -> Vec<Change> {
    db::meta_get(conn, "learned_changes").ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
}

/// Keep a run, and log what it changed.
pub fn save(conn: &Connection, l: &Learned) -> R<()> {
    let mut log = changes(conn);
    for k in l.knobs.iter().filter(|k| k.status == "changed") {
        let (loss, n) = match k.source {
            Source::PastDuels => ((l.check.past_before, l.check.past_after), l.check.past_n),
            Source::LiveCalls => ((l.check.live_before, l.check.live_after), l.check.live_n),
        };
        log.insert(0, Change { at_ms: l.at_ms, key: k.key.clone(), from: k.was, to: k.value, loss, n });
    }
    log.truncate(LOG_LEN);
    db::meta_set(conn, "learned_changes", &serde_json::to_string(&log).unwrap_or_default())?;
    db::meta_set(conn, "learned_model", &serde_json::to_string(l).unwrap_or_default())?;
    Ok(())
}

/// Whether there's new evidence since the last run: a newly settled call, or 25 more past duels (or no run yet).
pub fn due(conn: &Connection, last: Option<&Learned>) -> bool {
    let Some(l) = last else { return true };
    let count = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0) as usize;
    let duels = count("SELECT COUNT(*) FROM duels WHERE winner IS NOT NULL");
    let calls = count("SELECT COUNT(*) FROM predictions WHERE status = 'settled'");
    duels >= l.past_duels + 25 || calls > l.live_calls
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Fighter, Move};

    fn fighter(types: &[&str], el: &str) -> Fighter {
        Fighter {
            pal: "x".into(),
            types: types.iter().map(|t| t.to_string()).collect(),
            hp: 4000.0,
            atk: 500.0,
            def: 600.0,
            skills: vec![Move { name: "m".into(), el: el.into(), power: 100.0, lv: 1 }],
            passives: vec![],
            hidden_boost: 1.0,
            speed: 1.0,
        }
    }

    /// Made-up history where the side with the type edge wins 9 in 10: the learner should raise the type edge,
    /// and keep it because the held-back duels agree.
    #[test]
    fn learns_a_type_edge_the_duels_show() {
        let past: Vec<PastDuel> = (0..800u64)
            .map(|i| {
                let fire_home = i % 2 == 0;
                let (fire, grass) = (fighter(&["fire"], "fire"), fighter(&["grass"], "grass"));
                let (home, away) = if fire_home { (fire, grass) } else { (grass, fire) };
                let edge_won = i % 10 != 0;
                PastDuel { id: i, home, away, species_rec: [[0.0; 2]; 2], pal_h2h: [0.0; 2], home_won: fire_home == edge_won }
            })
            .collect();
        let start = Settings { type_bonus: 1.0, ..Default::default() };
        let l = learn_on(&past, &[], &start, 1);
        assert!(l.settings.type_bonus > 1.2, "{:?}", l.settings);
        assert!(l.check.past_after < l.check.past_before);
        let k = l.knobs.iter().find(|k| k.key == "TYPE_BONUS").unwrap();
        assert_eq!((k.status.as_str(), k.was), ("changed", 1.0));
        let stats = l.knobs.iter().find(|k| k.key == "STATS_STEEPNESS").unwrap();
        assert_eq!((stats.status.as_str(), stats.have, stats.need), ("waiting", 0, MIN_CALLS));
    }

    /// Coin-flip history: nothing to learn, so nothing moves.
    #[test]
    fn holds_when_the_evidence_is_noise() {
        let past: Vec<PastDuel> = (0..800u64)
            .map(|i| PastDuel {
                id: i,
                home: fighter(&["fire"], "fire"),
                away: fighter(&["grass"], "grass"),
                species_rec: [[0.0; 2]; 2],
                pal_h2h: [0.0; 2],
                home_won: (i * 7919) % 2 == 0,
            })
            .collect();
        let l = learn_on(&past, &[], &Settings::default(), 1);
        assert!(l.knobs.iter().all(|k| k.status != "changed") || l.check.past_after < l.check.past_before - MARGIN_PAST, "{:?}", l.knobs);
    }
}
