//! The model tunes itself. Each setting is re-fitted on the evidence that can see it (past duels for types, move
//! power and records; real calls for the stat settings, which past duels can't see; every call, a little at a time,
//! for the two the sources disagree on) and a change is kept only when it
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
/// The settings that learn from every real call, gradually: each moves from its starting point toward what the calls
/// fit best by calls / (calls + this). A few calls barely move it; 60 take it halfway
pub const GRADUAL_PRIOR: usize = 60;
/// Records on real calls, and how much moves' real hit sizes count: past duels, at base stats, say one thing about
/// each and real calls another, so neither is trusted outright. Hit size goes first, as the records sit on top of it.
const GRADUAL: [&str; 2] = ["HIT_WEIGHT", "SPECIES_LIVE"];

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

const KNOBS: [Knob; 10] = [
    Knob { key: "TYPE_BONUS", source: Source::PastDuels, grid: &[1.0, 1.1, 1.25, 1.4, 1.5, 1.65, 1.8, 2.0, 2.25, 2.5] },
    Knob { key: "TYPE_RESIST", source: Source::PastDuels, grid: &[1.0, 1.1, 1.25, 1.4, 1.5, 1.65, 1.8, 2.0] },
    Knob { key: "POWER_EXP", source: Source::PastDuels, grid: &[0.0, 0.1, 0.2, 0.3, 0.35, 0.4, 0.5, 0.65, 0.8, 1.0] },
    Knob { key: "SPECIES_WEIGHT", source: Source::PastDuels, grid: &[0.0, 0.25, 0.5, 0.75, 1.0, 1.2, 1.4, 1.6, 2.0, 2.5] },
    Knob { key: "PAL_H2H_WEIGHT", source: Source::PastDuels, grid: &[0.0, 0.25, 0.5, 0.75, 1.0, 1.5] },
    Knob { key: "STATS_STEEPNESS", source: Source::LiveCalls, grid: &[1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 5.0, 6.0] },
    Knob { key: "COOLDOWN_EXP", source: Source::LiveCalls, grid: &[0.0, 0.25, 0.5, 0.75, 1.0] },
    Knob { key: "HIT_WEIGHT", source: Source::LiveCalls, grid: &[0.0, 0.25, 0.5, 0.75, 1.0] },
    Knob { key: "SPECIES_LIVE", source: Source::LiveCalls, grid: &[0.0, 0.25, 0.5, 0.75, 1.0, 1.2, 1.4, 1.6, 2.0] },
    // per 10 points of the site's power score; past duels showed nothing, so only real calls that have it decide
    Knob { key: "PAL_RANK_WEIGHT", source: Source::LiveCalls, grid: &[0.0, 0.05, 0.1, 0.15, 0.2, 0.25, 0.3, 0.4, 0.5] },
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

/// Both sides' pal ranks were read when it was called (calls from before Augur read them haven't).
fn has_pal_ranks(c: &CallRecord) -> bool {
    c.fighters.as_ref().is_some_and(|(h, a)| h.fighter.pal_rank.is_some() && a.fighter.pal_rank.is_some())
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

fn is_gradual(key: &str) -> bool {
    GRADUAL.contains(&key)
}

/// A gradual setting: the value that fits every call best, taken calls / (calls + GRADUAL_PRIOR) of the way from its
/// starting point, in steps of 0.05 (so it doesn't wander on each result). Records on real calls start at the past
/// duels' species weight; hit size starts at 0, since past duels saw no gain from it (on 14 Nov 0.5 took their log
/// loss 0.661 -> 0.675, while the 87 real calls went 0.548 -> 0.536 and both halves of them agreed).
fn gradual(key: &str, calls: &[CallRecord], s: &Settings) -> f64 {
    let (prior, top): (f64, f64) = match key {
        "SPECIES_LIVE" => (s.species_weight, 2.0),
        _ => (0.0, 1.0),
    };
    let loss = |v: f64| history::evaluate(calls, &s.with(key, v)).0;
    let steps = (top * 20.0).round() as u32;
    let best = (0..=steps).map(|i| f64::from(i) * 0.05).min_by(|a, b| loss(*a).total_cmp(&loss(*b))).unwrap_or(prior);
    let n = calls.len() as f64;
    let v = prior + (best - prior) * n / (n + GRADUAL_PRIOR as f64);
    (v * 20.0).round() / 20.0
}

/// Re-fit the settings, starting from `current`. Changes nothing in the database; `save` keeps the result.
pub fn learn(conn: &Connection, current: &Settings, now_ms: u64) -> R<Learned> {
    let past = history::past_duels(conn)?;
    let calls: Vec<CallRecord> = db::settled_all(conn)?.into_iter().filter(|c| c.fighters.is_some() && c.species_rec.is_some()).collect();
    Ok(learn_on(&past, &calls, current, now_ms))
}

pub fn learn_on(past: &[PastDuel], calls: &[CallRecord], current: &Settings, now_ms: u64) -> Learned {
    let (past_fit, past_check) = past.split_at(past.len() * 3 / 4);
    let (calls_fit, calls_check) = calls.split_at(calls.len() * 3 / 4);
    let past_loss = |set: &[PastDuel], s: &Settings| history::score_past(set, s).log_loss;
    let live_loss = |set: &[CallRecord], s: &Settings| history::evaluate(set, s).0;
    let cooldown_calls = calls.iter().filter(|c| has_cooldown(c)).count();
    let rank_calls = calls.iter().filter(|c| has_pal_ranks(c)).count();
    let ready = |k: &Knob| match (k.source, k.key) {
        (Source::PastDuels, _) => past.len() >= 200,
        (Source::LiveCalls, key) if is_gradual(key) => !calls.is_empty(),
        (Source::LiveCalls, "COOLDOWN_EXP") => calls.len() >= MIN_CALLS && cooldown_calls >= MIN_COOLDOWN_CALLS,
        (Source::LiveCalls, "PAL_RANK_WEIGHT") => rank_calls >= MIN_CALLS,
        (Source::LiveCalls, _) => calls.len() >= MIN_CALLS,
    };

    // coordinate descent: each setting in turn, a few rounds, on the fitting part only
    let mut cand = current.clone();
    for _ in 0..3 {
        for k in KNOBS.iter().filter(|k| ready(k) && !is_gradual(k.key)) {
            let v = match k.source {
                Source::PastDuels => best(k, &cand, &|s| past_loss(past_fit, s), MARGIN_PAST),
                Source::LiveCalls => best(k, &cand, &|s| live_loss(calls_fit, s), MARGIN_LIVE),
            };
            cand = cand.with(k.key, v);
        }
    }

    // keep each group's changes only if they do better on what was held back
    let held = |k: &&Knob| !is_gradual(k.key);
    let moved = |src: Source| KNOBS.iter().filter(held).any(|k| k.source == src && cand.get(k.key) != current.get(k.key));
    let mut out = current.clone();
    if moved(Source::PastDuels) {
        let trial = KNOBS
            .iter()
            .filter(held)
            .filter(|k| k.source == Source::PastDuels)
            .fold(out.clone(), |s, k| s.with(k.key, cand.get(k.key).unwrap()));
        let live_ok = calls_check.len() < 20 || live_loss(calls_check, &trial) <= live_loss(calls_check, &out) + MARGIN_LIVE;
        if past_loss(past_check, &trial) < past_loss(past_check, &out) - MARGIN_PAST && live_ok {
            out = trial;
        }
    }
    if moved(Source::LiveCalls) && calls_check.len() >= 15 {
        let trial = KNOBS
            .iter()
            .filter(held)
            .filter(|k| k.source == Source::LiveCalls)
            .fold(out.clone(), |s, k| s.with(k.key, cand.get(k.key).unwrap()));
        if live_loss(calls_check, &trial) < live_loss(calls_check, &out) - MARGIN_LIVE {
            out = trial;
        }
    }
    if !calls.is_empty() {
        for key in GRADUAL {
            out = out.with(key, gradual(key, calls, &out));
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
                (Source::LiveCalls, key) if is_gradual(key) => (calls.len(), 0),
                (Source::LiveCalls, "COOLDOWN_EXP") if calls.len() >= MIN_CALLS => (cooldown_calls, MIN_COOLDOWN_CALLS),
                (Source::LiveCalls, "PAL_RANK_WEIGHT") => (rank_calls, MIN_CALLS),
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
            skills: vec![Move { name: "m".into(), el: el.into(), power: 100.0, lv: 1, hit: 1.0 }],
            passives: vec![],
            hidden_boost: 1.0,
            speed: 1.0,
            pal_rank: None,
        }
    }

    /// A settled real call between `home` and `away`, the home side winning or not.
    fn call(i: i64, home: Fighter, away: Fighter, species_rec: [[f64; 2]; 2], home_won: bool) -> CallRecord {
        let side = |fighter| crate::db::CallFighter { fighter, ivs: Default::default() };
        CallRecord {
            id: i,
            seen_at: i,
            home_owner: "a".into(),
            away_owner: "b".into(),
            home_pal: "x".into(),
            away_pal: "y".into(),
            pick: "home".into(),
            prob: 0.5,
            p_stats: None,
            p_species: None,
            p_h2h: None,
            species_rec: Some(species_rec),
            pal_h2h: [0.0; 2],
            duel_id: None,
            winner: Some(if home_won { "home" } else { "away" }.into()),
            fought_at: None,
            ko: None,
            winner_hp_left: None,
            stake: None,
            fighters: Some((side(home), side(away))),
            late: None,
            late_only: false,
            version: String::new(),
            model: String::new(),
            damaged: false,
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

    /// Real calls where the stronger pal wins 4 in 5 though the species records favour the other: the learner
    /// lowers the records' weight on real calls, more as the calls add up, and leaves the past duels' species weight
    /// alone.
    #[test]
    fn learns_how_much_records_count_on_real_calls() {
        let pal = |hp: f64| Fighter { hp, ..fighter(&["neutral"], "neutral") };
        let calls: Vec<CallRecord> =
            (0..120i64).map(|i| call(i, pal(6000.0), pal(4000.0), [[10.0, 40.0], [40.0, 10.0]], i % 5 != 0)).collect();
        let start = Settings::default();
        assert_eq!(start.species_on_calls(), start.species_weight, "unset, it follows the species weight");
        let l = learn_on(&[], &calls, &start, 1);
        assert!(l.settings.species_on_calls() < start.species_weight, "{:?}", l.settings);
        assert_eq!(l.settings.species_weight, start.species_weight, "the past duels' weight is theirs");
        assert!(l.check.live_after < l.check.live_before);
        let k = l.knobs.iter().find(|k| k.key == "SPECIES_LIVE").unwrap();
        assert_eq!((k.status.as_str(), k.source), ("changed", Source::LiveCalls));
        // it learns from every call, a little at a time: 20 calls move it less than 120
        let few = learn_on(&[], &calls[..20], &start, 1);
        let v = |l: &Learned| l.settings.species_on_calls();
        assert!(v(&few) < start.species_weight && v(&few) > v(&l), "20 calls: {}, 120 calls: {}", v(&few), v(&l));
    }

    /// Real calls between equal pals where the one whose move really hits harder in the tapes wins 4 in 5: hit size
    /// starts at 0 (past duels saw nothing) and moves up a little at a time, more as the calls add up.
    #[test]
    fn learns_how_much_real_hit_size_counts_on_real_calls() {
        let pal = |hit: f64| Fighter {
            skills: vec![Move { name: "m".into(), el: "neutral".into(), power: 100.0, lv: 1, hit }],
            ..fighter(&["neutral"], "neutral")
        };
        let calls: Vec<CallRecord> = (0..120i64).map(|i| call(i, pal(1.5), pal(0.7), [[0.0; 2]; 2], i % 5 != 0)).collect();
        let start = Settings::default();
        assert_eq!(start.hit_weight, 0.0);
        let l = learn_on(&[], &calls, &start, 1);
        let k = l.knobs.iter().find(|k| k.key == "HIT_WEIGHT").unwrap();
        assert_eq!((k.status.as_str(), k.source, k.have, k.need), ("changed", Source::LiveCalls, 120, 0));
        assert!(l.check.live_after < l.check.live_before);
        let few = learn_on(&[], &calls[..20], &start, 1);
        assert!(
            few.settings.hit_weight > 0.0 && few.settings.hit_weight < l.settings.hit_weight,
            "20 calls: {}, 120 calls: {}",
            few.settings.hit_weight,
            l.settings.hit_weight
        );
        // however hard the calls lean, 120 of them take it only 120 / 180 of the way
        assert!(l.settings.hit_weight < 1.0, "{:?}", l.settings);
    }

    /// Real calls between equal pals where the higher-ranked one wins 4 in 5: the pal rank waits for 60 calls that
    /// read it, then switches on.
    #[test]
    fn the_pal_rank_learns_once_enough_calls_have_it() {
        use crate::model::PalRank;
        let pal = |power: Option<u32>| Fighter {
            pal_rank: power.map(|power| PalRank { tier: "B".into(), rank: 400 - power * 3, of: 357, power }),
            ..fighter(&["neutral"], "neutral")
        };
        let calls = |n: i64, ranked: bool| -> Vec<CallRecord> {
            let (home, away) = if ranked { (Some(80), Some(40)) } else { (None, None) };
            (0..n).map(|i| call(i, pal(home), pal(away), [[0.0; 2]; 2], i % 5 != 0)).collect()
        };
        let start = Settings::default();
        let rank = |l: &Learned| l.knobs.iter().find(|k| k.key == "PAL_RANK_WEIGHT").unwrap().clone();
        // 120 calls, but from before Augur read the ranks
        let k = rank(&learn_on(&[], &calls(120, false), &start, 1));
        assert_eq!((k.status.as_str(), k.value, k.have, k.need), ("waiting", 0.0, 0, MIN_CALLS));
        let mut some = calls(80, false);
        some.extend(calls(40, true));
        assert_eq!(rank(&learn_on(&[], &some, &start, 1)).have, 40, "only the calls that read the ranks count");
        let l = learn_on(&[], &calls(120, true), &start, 1);
        assert!(l.settings.pal_rank_weight > 0.0, "{:?}", l.settings);
        assert_eq!(rank(&l).status, "changed");
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
