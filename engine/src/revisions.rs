//! Each call's history: every time Augur worked out the fight that's up again, what it said and what changed, so a
//! call that moved while picks were open (and what auto bet did about it) can be looked into afterwards. The call
//! itself stands once auto bet has bet on it (or picks close): later versions are kept here, not in the call.
//!
//! Calls from before the history was kept get what can be rebuilt from what they saved: the auto bet, the last
//! version before picks closed and the one at the bell, marked as backfilled. `changes` sums it all up for
//! `augur changes`: how often and how late calls move, why, and whether auto bet holding longer would have helped.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::autobet::AutoBetSettings;
use crate::db::{self, Result};
use crate::predict::Prediction;
use crate::stake::Stake;

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS call_revisions (  -- each version of a call while its fight was up, and why it changed
    prediction_id INTEGER NOT NULL REFERENCES predictions (id) ON DELETE CASCADE,
    n INTEGER NOT NULL,                -- 1: the first
    at INTEGER,                        -- site clock (ms)
    phase TEXT,                        -- picks: it was the call; after_bet: picks open, after auto bet bet (not the
                                       -- call); bell: after picks closed
    trigger TEXT,                      -- seen (the fight on the site changed), launch (Augur started with it up),
                                       -- restake, command (augur predict), bet (auto bet bet on it), backfill
                                       -- (rebuilt for an older call)
    cause TEXT,                        -- what changed since the last, comma separated (see `Cause`)
    detail TEXT,                       -- the same in words: "away: guessed → shown"
    pick TEXT, prob REAL,
    p_stats REAL, p_species REAL, p_h2h REAL,
    estimated INTEGER,
    home_from TEXT, away_from TEXT,    -- where each side's stats came from: shown, from their box, guessed (…)
    home_stats TEXT, away_stats TEXT,  -- the HP/Attack/Defence the model used: "9383/469/706"
    stake INTEGER, stake_cmd TEXT,
    model TEXT,                        -- a fingerprint of the model settings
    app_version TEXT,
    PRIMARY KEY (prediction_id, n)
);
"#;

/// Why the call was worked out again: what the caller knows. What changed is worked out from the versions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Trigger {
    /// the fight on the site changed (a new fight, or stats shown)
    Seen,
    /// Augur started with the fight up, so it was called again from scratch
    Launch,
    /// the staking settings or the payout changed
    Restake,
    /// `augur predict` on the command line
    Command,
}

impl Trigger {
    fn word(self) -> &'static str {
        match self {
            Trigger::Seen => "seen",
            Trigger::Launch => "launch",
            Trigger::Restake => "restake",
            Trigger::Command => "command",
        }
    }
}

/// Where a version stands: the call, a later version after auto bet bet on it, or after picks closed.
pub const PICKS: &str = "picks";
pub const AFTER_BET: &str = "after_bet";
pub const BELL: &str = "bell";
/// Versions rebuilt for calls from before the history was kept.
pub const BACKFILL: &str = "backfill";
/// The moment auto bet bet: the call as it stood, with the stake it bet.
pub const BET: &str = "bet";

/// One version of a call.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Revision {
    pub n: u32,
    /// site clock (ms)
    pub at: u64,
    pub phase: String,
    pub trigger: String,
    /// what changed since the last version, comma separated: first, stats_shown, stats_source, stats, records,
    /// pal_h2h, moves (hidden moves or hit sizes), model, stake, bell, bet, unknown (backfilled), other
    pub cause: String,
    pub detail: String,
    pub pick: String,
    pub prob: f64,
    /// the home win chance from the stats, species records and pal head-to-head
    pub factors: Option<[f64; 3]>,
    pub estimated: bool,
    /// [home, away]
    pub stats_from: [String; 2],
    pub stats: [Option<[f64; 3]>; 2],
    pub stake: Option<u64>,
    pub stake_cmd: Option<String>,
    pub model: String,
    pub version: String,
}

impl Revision {
    /// The version a prediction makes. `at`: when (site clock, ms); a prediction is stamped when it was made.
    pub fn of(p: &Prediction, stake: Option<&Stake>, trigger: Trigger, at: u64) -> Revision {
        let r3 = |v: f64| (v * 1000.0).round() / 1000.0;
        let stats = |f: &crate::model::Fighter| Some([f.hp.round(), f.atk.round(), f.def.round()]);
        Revision {
            at,
            phase: if p.picks_open { PICKS } else { BELL }.into(),
            trigger: trigger.word().into(),
            pick: p.pick.clone(),
            prob: r3(p.prob),
            factors: Some([r3(p.score.p_stats), r3(p.score.p_species), r3(p.score.p_h2h)]),
            estimated: p.estimated(),
            stats_from: [p.home.stats_from.clone(), p.away.stats_from.clone()],
            stats: [stats(&p.home.fighter.fighter), stats(&p.away.fighter.fighter)],
            stake: stake.map(|s| s.amount),
            stake_cmd: stake.map(|s| s.command.clone()),
            model: fingerprint(&serde_json::to_string(&p.settings).unwrap_or_default()),
            version: env!("CARGO_PKG_VERSION").into(),
            ..Default::default()
        }
    }

    pub fn backfilled(&self) -> bool {
        self.trigger == BACKFILL
    }

    fn same_call(&self, o: &Revision) -> bool {
        self.pick == o.pick
            && (self.prob - o.prob).abs() < 0.0005
            && self.factors == o.factors
            && self.stats_from == o.stats_from
            && self.stats == o.stats
            && self.stake == o.stake
            && self.stake_cmd == o.stake_cmd
            && self.model == o.model
            && self.phase == o.phase
    }
}

/// Saves a new version of call `id`, with what changed since the last one. Nothing is saved when nothing changed
/// (the call worked out again to the same thing). Its phase comes from the call: after auto bet's bet it's no longer
/// the call, and once picks have closed it's the bell's. Returns the version saved.
pub fn record(conn: &Connection, id: i64, mut rev: Revision) -> Result<Option<Revision>> {
    let last = of_call(conn, id)?.pop();
    let closed: bool =
        conn.query_row("SELECT COALESCE(closed, 0) = 1 FROM predictions WHERE id = ?", [id], |r| r.get(0)).optional()?.unwrap_or(false);
    if closed {
        rev.phase = BELL.into();
    } else if rev.phase == PICKS && bet_went_out(conn, id)? {
        rev.phase = AFTER_BET.into();
    }
    if last.as_ref().is_some_and(|l| l.same_call(&rev)) {
        return Ok(None);
    }
    let (causes, detail) = what_changed(last.as_ref(), &rev);
    rev.cause = causes.join(",");
    rev.detail = detail.join("; ");
    rev.n = last.map_or(1, |l| l.n + 1);
    insert(conn, id, &rev)?;
    Ok(Some(rev))
}

/// Notes the moment auto bet bet on call `id`: the call as it stood, with the stake that went in. `at`: site clock.
pub fn record_bet(conn: &Connection, id: i64, at: u64, pick: &str, prob: f64, stake: &Stake) -> Result<()> {
    let last = of_call(conn, id)?.pop();
    let mut rev = last.clone().unwrap_or_default();
    rev.n = last.map_or(1, |l| l.n + 1);
    (rev.at, rev.phase, rev.trigger, rev.cause, rev.detail) = (at, PICKS.into(), BET.into(), BET.into(), String::new());
    (rev.pick, rev.prob) = (pick.into(), (prob * 1000.0).round() / 1000.0);
    (rev.stake, rev.stake_cmd) = (Some(stake.amount), Some(stake.command.clone()));
    insert(conn, id, &rev)
}

/// Every version of a call, first first.
pub fn of_call(conn: &Connection, id: i64) -> Result<Vec<Revision>> {
    let mut st = conn.prepare_cached(
        "SELECT n, at, phase, trigger, cause, detail, pick, prob, p_stats, p_species, p_h2h, estimated, home_from, away_from,
                home_stats, away_stats, stake, stake_cmd, model, app_version
         FROM call_revisions WHERE prediction_id = ? ORDER BY n",
    )?;
    let rows = st.query_map([id], |r| {
        let text = |i: usize| r.get::<_, Option<String>>(i).map(Option::unwrap_or_default);
        let factors = [r.get::<_, Option<f64>>(8)?, r.get(9)?, r.get(10)?];
        Ok(Revision {
            n: r.get(0)?,
            at: r.get::<_, Option<i64>>(1)?.unwrap_or(0).max(0) as u64,
            phase: text(2)?,
            trigger: text(3)?,
            cause: text(4)?,
            detail: text(5)?,
            pick: text(6)?,
            prob: r.get::<_, Option<f64>>(7)?.unwrap_or(0.0),
            factors: match factors {
                [Some(a), Some(b), Some(c)] => Some([a, b, c]),
                _ => None,
            },
            estimated: r.get::<_, Option<i64>>(11)? == Some(1),
            stats_from: [text(12)?, text(13)?],
            stats: [parse_stats(&text(14)?), parse_stats(&text(15)?)],
            stake: r.get::<_, Option<i64>>(16)?.map(|v| v.max(0) as u64),
            stake_cmd: r.get(17)?,
            model: text(18)?,
            version: text(19)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// When Augur first saw the call's fight (site clock, ms): None before the history was kept.
pub fn first_at(conn: &Connection, id: i64) -> Result<Option<u64>> {
    let at: Option<i64> =
        conn.query_row("SELECT MIN(at) FROM call_revisions WHERE prediction_id = ? AND trigger != 'backfill'", [id], |r| r.get(0))?;
    Ok(at.filter(|&a| a > 0).map(|a| a as u64))
}

/// Auto bet has put real credits on call `id` (or may have: typed in, or going in when Augur stopped). From then
/// the call stands.
pub fn bet_went_out(conn: &Connection, id: i64) -> Result<bool> {
    Ok(conn.query_row(
        &format!("SELECT COUNT(*) FROM auto_bets WHERE prediction_id = ?1 AND test = 0 AND status IN ({WENT_OUT})"),
        [id],
        |r| r.get::<_, i64>(0),
    )? > 0)
}

/// Another computer's versions of a call that's here (from the calls file). They replace versions rebuilt here,
/// never ones recorded here. Whether anything was saved.
pub fn merge(conn: &Connection, id: i64, incoming: &[Revision]) -> Result<bool> {
    let here = of_call(conn, id)?;
    if incoming.is_empty() || here.iter().any(|r| !r.backfilled()) || (!here.is_empty() && incoming.iter().all(Revision::backfilled)) {
        return Ok(false);
    }
    conn.execute("DELETE FROM call_revisions WHERE prediction_id = ?", [id])?;
    for r in incoming {
        insert(conn, id, r)?;
    }
    Ok(true)
}

/// Rebuilds what can be known of an older call's history from what it saved: the version auto bet bet on (when it
/// differs from the call), the last version before picks closed, and the one at the bell. Nothing when the call
/// has a history. Returns how many versions it saved.
pub fn backfill_call(conn: &Connection, id: i64) -> Result<usize> {
    if !of_call(conn, id)?.is_empty() {
        return Ok(0);
    }
    type Row = (i64, String, f64, [Option<f64>; 3], bool, Option<String>, Option<i64>, Option<String>, Option<String>);
    type Late = (Option<String>, Option<f64>, Option<i64>, bool, Option<i64>, Option<String>);
    let found: Option<(Row, Late)> = conn
        .query_row(
            "SELECT COALESCE(seen_at, 0), pick, prob, p_stats, p_species, p_h2h, COALESCE(estimated, 0) = 1, stats_from, stake,
                    stake_cmd, model, late_pick, late_prob, closed_at, COALESCE(late_only, 0) = 1, duel_id, app_version
             FROM predictions WHERE id = ?",
            [id],
            |r| {
                Ok((
                    (
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        [r.get(3)?, r.get(4)?, r.get(5)?],
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                        r.get(9)?,
                        r.get(10)?,
                    ),
                    (r.get(11)?, r.get(12)?, r.get(13)?, r.get(14)?, r.get(15)?, r.get(16)?),
                ))
            },
        )
        .optional()?;
    let Some((
        (seen_at, pick, prob, factors, estimated, stats_from, stake, stake_cmd, model),
        (late_pick, late_prob, closed_at, late_only, duel, version),
    )) = found
    else {
        return Ok(0);
    };
    let fighters = fighter_stats(conn, id)?;
    let from = split_from(stats_from.as_deref().unwrap_or(""));
    let base = Revision {
        trigger: BACKFILL.into(),
        model: model.as_deref().map(fingerprint).unwrap_or_default(),
        version: version.unwrap_or_default(),
        ..Default::default()
    };
    let mut out = vec![];
    // the version auto bet bet on, when the call changed after it
    let bet: Option<(String, f64, i64, String, i64)> = conn
        .query_row(
            &format!(
                "SELECT side, prob, amount, command, claimed_at FROM auto_bets
                 WHERE (prediction_id = ?1 OR (prediction_id IS NULL AND duel_id = ?2)) AND test = 0 AND status IN ({WENT_OUT})
                 ORDER BY id LIMIT 1"
            ),
            params![id, duel],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let changed_after_bet = bet.as_ref().is_some_and(|(side, p, ..)| *side != pick || (p - prob).abs() >= 0.0005);
    if let Some((side, p, amount, command, at)) = bet.as_ref().filter(|_| changed_after_bet) {
        out.push(Revision {
            at: (*at).max(0) as u64,
            phase: PICKS.into(),
            cause: "first".into(),
            detail: "the call auto bet bet on".into(),
            pick: side.clone(),
            prob: *p,
            stake: Some((*amount).max(0) as u64),
            stake_cmd: Some(command.clone()),
            ..base.clone()
        });
    }
    // the last version before picks closed: its fighters are the ones it saw, unless the bell's real stats replaced them
    let known = factors.iter().any(|f| f.is_some_and(|v| v != 0.0));
    out.push(Revision {
        at: seen_at.max(0) as u64,
        phase: if late_only {
            BELL
        } else if changed_after_bet {
            AFTER_BET
        } else {
            PICKS
        }
        .into(),
        cause: if out.is_empty() { "first" } else { "unknown" }.into(),
        pick: pick.clone(),
        prob,
        factors: match factors {
            [Some(a), Some(b), Some(c)] if known => Some([a, b, c]),
            _ => None,
        },
        estimated,
        stats_from: from,
        stats: if late_pick.is_none() { fighters } else { [None, None] },
        stake: stake.map(|s| s.max(0) as u64),
        stake_cmd,
        ..base.clone()
    });
    if let (Some(lp), Some(lprob)) = (late_pick, late_prob) {
        out.push(Revision {
            at: closed_at.unwrap_or(seen_at).max(seen_at).max(0) as u64,
            phase: BELL.into(),
            cause: "bell".into(),
            detail: if lp != pick { "pick flipped".into() } else { String::new() },
            pick: lp,
            prob: lprob,
            stats_from: ["shown".into(), "shown".into()],
            stats: fighters,
            ..base
        });
    }
    for (i, r) in out.iter_mut().enumerate() {
        r.n = i as u32 + 1;
        insert(conn, id, r)?;
    }
    Ok(out.len())
}

/// One-off work on an older store: every finished call gets the history that can be rebuilt for it, and a call auto
/// bet bet on has the stake that went in as Augur's (it showed the stake Staking gave, which auto bet holds to the
/// default while stats are hidden).
pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    const BACKFILLED: &str = "revisions_backfilled";
    if db::meta_get(conn, BACKFILLED)?.is_none() {
        let ids: Vec<i64> = conn
            .prepare("SELECT id FROM predictions p WHERE status != 'open' AND NOT EXISTS (SELECT 1 FROM call_revisions r WHERE r.prediction_id = p.id)")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let tx = conn.unchecked_transaction()?;
        for id in ids {
            backfill_call(&tx, id)?;
        }
        tx.commit()?;
        db::meta_set(conn, BACKFILLED, "1")?;
    }
    const MATCHED: &str = "auto_stakes_matched";
    if db::meta_get(conn, MATCHED)?.is_none() {
        match_auto_stakes(conn)?;
        db::meta_set(conn, MATCHED, "1")?;
    }
    Ok(())
}

/// Augur's stake on each call auto bet bet on, on the same side, is the one that went in. Returns how many changed.
pub fn match_auto_stakes(conn: &Connection) -> Result<usize> {
    Ok(conn.execute(
        &format!(
            "UPDATE predictions SET stake = a.amount, stake_cmd = a.command
             FROM (SELECT prediction_id, duel_id, side, amount, command FROM auto_bets
                   WHERE test = 0 AND status IN ({WENT_OUT}) AND amount > 0) a
             WHERE (a.prediction_id = predictions.id OR (a.prediction_id IS NULL AND a.duel_id = predictions.duel_id))
               AND a.side = predictions.pick AND predictions.stake IS NOT NULL AND predictions.stake != a.amount"
        ),
        [],
    )?)
}

// --- the analysis ---

/// How calls changed while their fights were up, for `augur changes`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Changes {
    /// calls with a history recorded live, and ones with a rebuilt one
    pub live: usize,
    pub backfilled: usize,
    /// live calls that changed while picks were open (before any bet), and how many flipped their pick
    pub moved: usize,
    pub flipped: usize,
    /// how far the chance moved, in points: each change's size
    pub moves: Vec<f64>,
    /// seconds after the fight was first seen that each change came: (secs, causes)
    pub when: Vec<(u64, String)>,
    /// each cause and how often
    pub causes: Vec<(String, usize)>,
    /// auto bets that went out, and how many calls then changed before picks closed (and flipped)
    pub bets: usize,
    pub changed_after_bet: usize,
    pub flipped_after_bet: usize,
    /// points moved after the bet, toward the pick (+) or away from it (−)
    pub after_bet_moves: Vec<f64>,
    /// the bell: calls re-called once the stats showed, how many flipped, and how far calls with hidden stats moved
    pub at_bell: usize,
    pub flipped_at_bell: usize,
    pub hidden_at_bell: Vec<f64>,
    /// holding before betting for longer: one row per hold
    pub holds: Vec<Hold>,
}

/// What auto bet would have done on the live calls had it held this long before betting.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Hold {
    pub secs: u64,
    /// calls it would have bet on
    pub bets: usize,
    /// of those, ones it would have bet differently from holding `hold_secs` (another side, stake or not at all)
    pub differ: usize,
    /// of the differing ones settled, how many the longer hold would have got right, and the shorter one
    pub longer_right: usize,
    pub shorter_right: usize,
    /// picks closed first: no bet
    pub too_late: usize,
}

/// Holds to try, seconds after the fight was first seen.
const HOLDS: [u64; 6] = [10, 30, 60, 90, 120, 180];

pub fn changes(conn: &Connection, auto: &AutoBetSettings) -> Result<Changes> {
    let mut c = Changes::default();
    // each call with a history: its id, the winner, when picks closed and its duel
    type Call = (i64, Option<String>, Option<i64>, Option<i64>);
    let calls: Vec<Call> = conn
        .prepare(
            "SELECT p.id, d.winner, p.closed_at, p.duel_id FROM predictions p LEFT JOIN duels d ON d.id = p.duel_id
             WHERE EXISTS (SELECT 1 FROM call_revisions r WHERE r.prediction_id = p.id) ORDER BY p.id",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut causes: HashMap<String, usize> = HashMap::new();
    let mut holds: Vec<Hold> = HOLDS.iter().map(|&secs| Hold { secs, ..Default::default() }).collect();
    for (id, winner, closed_at, duel) in calls {
        let revs = of_call(conn, id)?;
        let live = !revs.iter().any(Revision::backfilled);
        if live {
            c.live += 1;
        } else {
            c.backfilled += 1;
        }
        let open: Vec<&Revision> = revs.iter().filter(|r| r.phase != BELL && r.trigger != BET).collect();
        // the bell: the last version once picks had closed, against the last one before
        if let (Some(before), Some(bell)) = (open.last(), revs.iter().rev().find(|r| r.phase == BELL)) {
            c.at_bell += 1;
            c.flipped_at_bell += usize::from(bell.pick != before.pick);
            if before.estimated {
                c.hidden_at_bell.push(points(before, bell));
            }
        }
        // auto bet's bet against the last version before picks closed
        let bet: Option<(String, f64)> = conn
            .query_row(
                &format!(
                    "SELECT side, prob FROM auto_bets WHERE (prediction_id = ?1 OR (prediction_id IS NULL AND duel_id = ?2))
                     AND status IN ({WENT_OUT}) ORDER BY id LIMIT 1"
                ),
                params![id, duel],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let (Some((side, prob)), Some(last)) = (bet, open.last()) {
            c.bets += 1;
            let at_bet = Revision { pick: side, prob, ..Default::default() };
            let moved = points(&at_bet, last);
            if last.pick != at_bet.pick || moved.abs() >= 0.05 {
                c.changed_after_bet += 1;
                c.flipped_after_bet += usize::from(last.pick != at_bet.pick);
                c.after_bet_moves.push(moved);
            }
        }
        if !live {
            continue;
        }
        let Some(first) = open.first() else { continue };
        // changes while picks were open, the bet's own moment aside
        let mut changed = false;
        for w in open.windows(2) {
            let (a, b) = (w[0], w[1]);
            if a.pick == b.pick && (a.prob - b.prob).abs() < 0.0005 && a.stake == b.stake {
                continue;
            }
            changed = true;
            c.flipped += usize::from(a.pick != b.pick);
            c.moves.push(points(a, b).abs());
            c.when.push((b.at.saturating_sub(first.at) / 1000, b.cause.clone()));
            for cause in b.cause.split(',').filter(|s| !s.is_empty()) {
                *causes.entry(cause.to_string()).or_default() += 1;
            }
        }
        c.moved += usize::from(changed);
        // holding longer: the version standing then, under auto bet's rules
        let standing = |secs: u64| open.iter().rev().find(|r| r.at <= first.at + secs * 1000).copied();
        let closes = closed_at.filter(|&t| t > 0).map(|t| t as u64);
        let shorter = standing(auto.hold_secs).filter(|r| would_bet(auto, r));
        let right = |r: &Revision| winner.as_deref() == Some(r.pick.as_str());
        for h in &mut holds {
            let at = first.at + h.secs * 1000;
            if closes.is_some_and(|t| t <= at) {
                h.too_late += usize::from(shorter.is_some());
                continue;
            }
            let longer = standing(h.secs).filter(|r| would_bet(auto, r));
            h.bets += usize::from(longer.is_some());
            let same = match (shorter, longer) {
                (Some(s), Some(l)) => s.pick == l.pick && s.stake == l.stake,
                (None, None) => true,
                _ => false,
            };
            if !same {
                h.differ += 1;
                if winner.is_some() {
                    h.longer_right += usize::from(longer.is_some_and(right));
                    h.shorter_right += usize::from(shorter.is_some_and(right));
                }
            }
        }
    }
    let mut causes: Vec<(String, usize)> = causes.into_iter().collect();
    causes.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    c.causes = causes;
    c.holds = holds;
    Ok(c)
}

/// `changes` for the command line.
pub fn report(c: &Changes, auto: &AutoBetSettings) -> String {
    let mut out = vec![];
    let pct = |n: usize, of: usize| if of == 0 { "–".to_string() } else { format!("{:.0}%", n as f64 * 100.0 / of as f64) };
    let avg = |v: &[f64]| if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 };
    out.push(format!("CALL CHANGES: {} calls with a live history, {} with one rebuilt from what they saved", c.live, c.backfilled));
    out.push(format!(
        "  While picks were open: {} of {} live calls changed ({}), {} flipped the pick; changes moved {:.1} points on average",
        c.moved,
        c.live,
        pct(c.moved, c.live),
        c.flipped,
        avg(&c.moves)
    ));
    if !c.when.is_empty() {
        let bands = [
            (0, 15, "0–15s"),
            (15, 30, "15–30s"),
            (30, 60, "30–60s"),
            (60, 120, "1–2 min"),
            (120, 240, "2–4 min"),
            (240, u64::MAX, "later"),
        ];
        let when: Vec<String> =
            bands.iter().map(|(lo, hi, name)| format!("{name} {}", c.when.iter().filter(|(s, _)| s >= lo && s < hi).count())).collect();
        out.push(format!("    when, after the fight was first seen: {}", when.join(" · ")));
    }
    if !c.causes.is_empty() {
        out.push(format!("    why: {}", c.causes.iter().map(|(k, n)| format!("{} {n}", cause_words(k))).collect::<Vec<_>>().join(" · ")));
    }
    out.push(format!(
        "  After auto bet's bet: {} of {} bets saw the call change before picks closed, {} flipped{}",
        c.changed_after_bet,
        c.bets,
        c.flipped_after_bet,
        if c.after_bet_moves.is_empty() {
            String::new()
        } else {
            format!(" (moved {:+.1} points toward the pick on average)", avg(&c.after_bet_moves))
        }
    ));
    let down = c.hidden_at_bell.iter().filter(|&&m| m < 0.0).count();
    let size: Vec<f64> = c.hidden_at_bell.iter().map(|m| m.abs()).collect();
    out.push(format!(
        "  At the bell: {} calls worked out again with the stats shown, {} flipped; {} with hidden stats moved {:.1} points on average \
         ({:+.1} toward the pick overall, {down} less sure)",
        c.at_bell,
        c.flipped_at_bell,
        c.hidden_at_bell.len(),
        avg(&size),
        avg(&c.hidden_at_bell)
    ));
    out.push(format!(
        "\nHOLDING LONGER (auto bet's rules: {:.0}% minimum, {} for hidden stats; against holding {}s, on the live calls)",
        auto.min_chance * 100.0,
        auto.hidden_stats_from.map_or("never".to_string(), |f| format!("{:.0}%", f * 100.0)),
        auto.hold_secs
    ));
    out.push("  hold  bets  differ  longer right  shorter right  picks closed first".into());
    for h in &c.holds {
        out.push(format!(
            "  {:>3}s  {:>4}  {:>6}  {:>12}  {:>13}  {:>18}",
            h.secs, h.bets, h.differ, h.longer_right, h.shorter_right, h.too_late
        ));
    }
    if c.live < 30 {
        out.push(format!("  Only {} live calls so far: too few to change auto bet on.", c.live));
    }
    out.join("\n")
}

/// A cause in words, for the report and the call's page.
pub fn cause_words(cause: &str) -> &'static str {
    match cause {
        "first" => "first call",
        "stats_shown" => "stats shown",
        "stats_source" => "where the stats came from",
        "stats" => "stats",
        "records" => "species records",
        "pal_h2h" => "pal head-to-head",
        "moves" => "moves",
        "model" => "model settings",
        "stake" => "stake",
        "bell" => "the bell",
        "bet" => "auto bet",
        "unknown" => "not known (rebuilt)",
        _ => "other",
    }
}

// --- helpers ---

/// The statuses of an auto bet that may have put credits on the fight (`AutoBet::went_out`).
const WENT_OUT: &str = "'sending', 'sent', 'seen', 'confirmed', 'unconfirmed'";

fn insert(conn: &Connection, id: i64, r: &Revision) -> Result<()> {
    let [ps, psp, ph] = r.factors.map_or([None; 3], |f| f.map(Some));
    conn.execute(
        "INSERT OR REPLACE INTO call_revisions (prediction_id, n, at, phase, trigger, cause, detail, pick, prob, p_stats, p_species,
             p_h2h, estimated, home_from, away_from, home_stats, away_stats, stake, stake_cmd, model, app_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
        params![
            id,
            r.n,
            r.at as i64,
            r.phase,
            r.trigger,
            r.cause,
            r.detail,
            r.pick,
            r.prob,
            ps,
            psp,
            ph,
            r.estimated as i64,
            r.stats_from[0],
            r.stats_from[1],
            r.stats[0].map(stats_text),
            r.stats[1].map(stats_text),
            r.stake.map(|s| s as i64),
            r.stake_cmd,
            r.model,
            r.version,
        ],
    )?;
    Ok(())
}

/// What changed from one version to the next: the causes, and the same in words.
fn what_changed(last: Option<&Revision>, new: &Revision) -> (Vec<&'static str>, Vec<String>) {
    let Some(last) = last else { return (vec!["first"], vec![]) };
    let (mut causes, mut detail) = (vec![], vec![]);
    if new.phase == BELL && last.phase != BELL {
        causes.push("bell");
    }
    for (i, side) in ["home", "away"].iter().enumerate() {
        let (a, b) = (&last.stats_from[i], &new.stats_from[i]);
        if a != b && b == "shown" {
            causes.push("stats_shown");
            detail.push(format!("{side}'s stats shown"));
        } else if a != b {
            causes.push("stats_source");
            detail.push(format!("{side}: {a} → {b}"));
        } else if last.stats[i] != new.stats[i] {
            causes.push("stats");
            let s = |v: Option<[f64; 3]>| v.map(stats_text).unwrap_or_else(|| "?".into());
            detail.push(format!("{side}'s stats {} → {}", s(last.stats[i]), s(new.stats[i])));
        }
    }
    let factor = |i: usize| match (last.factors, new.factors) {
        (Some(a), Some(b)) => (a[i] - b[i]).abs() >= 0.0005,
        _ => false,
    };
    if factor(1) {
        causes.push("records");
    }
    if factor(2) {
        causes.push("pal_h2h");
    }
    if last.model != new.model {
        causes.push("model");
    } else if factor(0) && !causes.iter().any(|c| c.starts_with("stats")) {
        // the stats factor moved with the same stats and model: the hidden moves or the hit sizes behind them
        causes.push("moves");
    }
    if last.stake_cmd != new.stake_cmd || last.stake != new.stake {
        causes.push("stake");
        let cmd = |r: &Revision| r.stake_cmd.clone().filter(|c| !c.is_empty()).unwrap_or_else(|| "sit out".into());
        detail.push(format!("{} → {}", cmd(last), cmd(new)));
    }
    if last.pick != new.pick {
        detail.push("pick flipped".into());
    }
    if causes.is_empty() {
        causes.push("other");
    }
    (causes, detail)
}

/// How far the chance on `from`'s pick moved going to `to`, in points (+: more sure of it).
fn points(from: &Revision, to: &Revision) -> f64 {
    let on_pick = |r: &Revision| if r.pick == from.pick { r.prob } else { 1.0 - r.prob };
    ((on_pick(to) - from.prob) * 1000.0).round() / 10.0
}

/// Whether auto bet's rules would bet on a version of a call (its timing and limits aside).
fn would_bet(s: &AutoBetSettings, r: &Revision) -> bool {
    r.stake.is_some_and(|a| a > 0) && r.prob >= s.min_chance && (!r.estimated || s.hidden_stats_from.is_some_and(|f| r.prob >= f))
}

fn fighter_stats(conn: &Connection, id: i64) -> Result<[Option<[f64; 3]>; 2]> {
    let mut out = [None, None];
    let mut st = conn.prepare_cached("SELECT side, hp, atk, def FROM fighters WHERE prediction_id = ?")?;
    let rows = st.query_map([id], |r| Ok((r.get::<_, String>(0)?, [r.get::<_, f64>(1)?, r.get::<_, f64>(2)?, r.get::<_, f64>(3)?])))?;
    for row in rows {
        let (side, s) = row?;
        out[usize::from(side == "away")] = Some(s.map(f64::round));
    }
    Ok(out)
}

/// "home: from their box; away: shown" as [home, away].
pub fn split_from(text: &str) -> [String; 2] {
    let of = |side: &str| {
        text.split(';').find_map(|part| part.trim().strip_prefix(side).and_then(|r| r.strip_prefix(':'))).unwrap_or("").trim().to_string()
    };
    [of("home"), of("away")]
}

fn stats_text(s: [f64; 3]) -> String {
    format!("{}/{}/{}", s[0], s[1], s[2])
}

fn parse_stats(text: &str) -> Option<[f64; 3]> {
    let v: Vec<f64> = text.split('/').filter_map(|x| x.parse().ok()).collect();
    <[f64; 3]>::try_from(v).ok()
}

/// A short, stable name for a model's settings (FNV-1a), so a change of settings shows between versions.
fn fingerprint(json: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in json.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{:08x}", h >> 32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autobet::{self, AutoBet};
    use crate::predict::{predict, Options};

    /// The seed's duel: the home side's stats hidden until the bell (guessed), the away side's shown.
    fn prediction() -> Prediction {
        let state = crate::seed::state("state-leaflit-wumpo.json");
        let opts = Options { lookups: false, db_path: "/nonexistent".into(), settings: Default::default() };
        predict(&state, &opts).unwrap()
    }

    fn stake(amount: u64, command: &str) -> Stake {
        Stake { amount, command: command.into(), ..Default::default() }
    }

    /// The prediction's call stored, as the live loop stores it, with its first version.
    fn called(conn: &Connection, p: &Prediction, st: &Stake) -> i64 {
        let id = db::add_call(conn, &p.to_call()).unwrap();
        db::set_call_stake(conn, id, st.amount, &st.command, None).unwrap();
        record(conn, id, Revision::of(p, Some(st), Trigger::Seen, p.seen_at)).unwrap();
        id
    }

    fn bet(conn: &Connection, id: i64, p: &Prediction, amount: u64, status: &str, test: bool) {
        let b = AutoBet {
            prediction_id: Some(id),
            home_owner: p.home.owner.clone(),
            away_owner: p.away.owner.clone(),
            home_pal: p.home.pal.clone(),
            away_pal: p.away.pal.clone(),
            side: p.pick.clone(),
            amount,
            command: format!("!{} {amount}", &p.pick[..1]),
            prob: p.prob,
            test,
            status: status.into(),
            ..Default::default()
        };
        autobet::claim(conn, &b).unwrap();
    }

    /// Each change is saved with what changed; working the call out again to the same thing saves nothing.
    #[test]
    fn each_version_says_what_changed() {
        let conn = db::open_memory().unwrap();
        let p = prediction();
        let id = called(&conn, &p, &stake(500, "!a"));
        assert_eq!(record(&conn, id, Revision::of(&p, Some(&stake(500, "!a")), Trigger::Launch, p.seen_at + 5_000)).unwrap(), None);
        // the home side's stats show: the stats and the chance move
        let mut shown = p.clone();
        (shown.home.stats_from, shown.home.estimated) = ("shown".into(), false);
        shown.home.fighter.fighter.hp += 900.0;
        shown.prob -= 0.04;
        let r = record(&conn, id, Revision::of(&shown, Some(&stake(500, "!a")), Trigger::Seen, p.seen_at + 12_000)).unwrap().unwrap();
        assert_eq!((r.n, r.cause.as_str(), r.detail.as_str()), (2, "stats_shown", "home's stats shown"));
        // a restake: only the stake
        let r =
            record(&conn, id, Revision::of(&shown, Some(&stake(5000, "!a all")), Trigger::Restake, p.seen_at + 20_000)).unwrap().unwrap();
        assert_eq!((r.cause.as_str(), r.detail.as_str(), r.trigger.as_str()), ("stake", "!a → !a all", "restake"));
        let all = of_call(&conn, id).unwrap();
        assert_eq!(all.iter().map(|r| r.cause.as_str()).collect::<Vec<_>>(), ["first", "stats_shown", "stake"]);
        assert_eq!(all[0].stats_from, ["guessed".to_string(), "shown".to_string()]);
        assert!(all[0].estimated && !all[1].estimated);
        assert_eq!(all[1].stats[0].unwrap()[0], all[0].stats[0].unwrap()[0] + 900.0);
        assert_eq!(first_at(&conn, id).unwrap(), Some(p.seen_at));
        // picks close: the next version is the bell's
        db::close_call(&conn, &p.home.owner, &p.away.owner, Some(p.seen_at + 300_000)).unwrap();
        let mut late = shown.clone();
        late.prob -= 0.1;
        let r = record(&conn, id, Revision::of(&late, None, Trigger::Seen, p.seen_at + 301_000)).unwrap().unwrap();
        assert_eq!((r.phase.as_str(), r.cause.split(',').next()), (BELL, Some("bell")));
    }

    /// Once auto bet has bet, the call stands as it was bet: later versions go only to the history.
    #[test]
    fn the_call_stands_once_auto_bet_bets() {
        let conn = db::open_memory().unwrap();
        let p = prediction();
        let id = called(&conn, &p, &stake(500, "!a"));
        bet(&conn, id, &p, 500, "confirmed", false);
        record_bet(&conn, id, p.seen_at + 18_000, &p.pick, p.prob, &stake(500, "!a")).unwrap();
        let mut moved = p.clone();
        moved.prob = 0.52;
        moved.pick = if p.pick == "home" { "away".into() } else { "home".into() };
        assert_eq!(db::add_call(&conn, &moved.to_call()).unwrap(), id);
        db::set_call_stake(&conn, id, 5000, "!h all", None).unwrap();
        let r = record(&conn, id, Revision::of(&moved, Some(&stake(5000, "!h all")), Trigger::Seen, p.seen_at + 40_000)).unwrap().unwrap();
        assert_eq!(r.phase, AFTER_BET);
        assert!(r.detail.contains("pick flipped"));
        let (pick, prob, cmd): (String, f64, String) = conn
            .query_row("SELECT pick, prob, stake_cmd FROM predictions WHERE id = ?", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap();
        assert_eq!((pick, cmd), (p.pick.clone(), "!a".to_string()), "the call and its stake stand as bet");
        assert!((prob - p.prob).abs() < 0.001);
        assert_eq!(of_call(&conn, id).unwrap().iter().map(|r| r.phase.as_str()).collect::<Vec<_>>(), [PICKS, PICKS, AFTER_BET]);
    }

    /// A test-mode bet, or one ruled out before it went in, leaves the call free to change.
    #[test]
    fn only_a_real_bet_freezes_the_call() {
        for (status, test) in [("confirmed", true), ("skipped", false), ("cancelled", false), ("failed", false)] {
            let conn = db::open_memory().unwrap();
            let p = prediction();
            let id = called(&conn, &p, &stake(500, "!a"));
            bet(&conn, id, &p, 500, status, test);
            let mut moved = p.clone();
            moved.prob = 0.61;
            db::add_call(&conn, &moved.to_call()).unwrap();
            let prob: f64 = conn.query_row("SELECT prob FROM predictions WHERE id = ?", [id], |r| r.get(0)).unwrap();
            assert!((prob - 0.61).abs() < 0.001, "{status} (test {test}) doesn't freeze it");
        }
    }

    /// Augur's stake on a call auto bet bet on is what went in, on its side only, real bets only.
    #[test]
    fn augur_s_stake_is_what_auto_bet_bet() {
        let conn = db::open_memory().unwrap();
        let p = prediction();
        let stake_of = |id: i64| -> (i64, String) {
            conn.query_row("SELECT stake, stake_cmd FROM predictions WHERE id = ?", [id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
        };
        let mut ids = vec![];
        for (i, (status, test, side)) in [
            ("confirmed", false, true),
            ("unconfirmed", false, true),
            ("confirmed", true, true),
            ("skipped", false, true),
            ("confirmed", false, false),
        ]
        .into_iter()
        .enumerate()
        {
            let mut q = p.clone();
            q.home.owner = format!("player-{i}");
            let id = called(&conn, &q, &stake(5000, "!a all"));
            if !side {
                q.pick = if q.pick == "home" { "away".into() } else { "home".into() };
            }
            bet(&conn, id, &q, 500, status, test);
            ids.push(id);
        }
        assert_eq!(match_auto_stakes(&conn).unwrap(), 2);
        let all = (5000, "!a all".to_string());
        assert_eq!(
            ids.iter().map(|&id| stake_of(id)).collect::<Vec<_>>(),
            [(500, "!a 500".into()), (500, "!a 500".into()), all.clone(), all.clone(), all]
        );
        assert_eq!(match_auto_stakes(&conn).unwrap(), 0, "once is enough");
    }

    /// Another computer's auto bet comes in through sync, linked to the call by its duel once that's known: the
    /// call's stake here becomes the one it bet.
    #[test]
    fn a_synced_auto_bet_sets_augur_s_stake() {
        let conn = db::open_memory().unwrap();
        let p = prediction();
        let id = called(&conn, &p, &stake(5000, "!a all"));
        db::settle_call(&conn, id, 77, &p.home.owner, &p.away.owner, "home").unwrap();
        let theirs =
            AutoBet { id: 3, side: p.pick.clone(), amount: 500, command: "!a".into(), status: "confirmed".into(), ..Default::default() };
        let file = std::env::temp_dir().join(format!("augur-autobets-{}.jsonl", std::process::id()));
        let stake_now = || conn.query_row("SELECT stake FROM predictions WHERE id = ?", [id], |r| r.get::<_, i64>(0)).unwrap();
        std::fs::write(&file, serde_json::to_string(&theirs).unwrap()).unwrap();
        autobet::import(&conn, &file, "pc").unwrap();
        assert_eq!(stake_now(), 5000, "not linked to a duel yet");
        std::fs::write(&file, serde_json::to_string(&AutoBet { duel_id: Some(77), ..theirs }).unwrap()).unwrap();
        autobet::import(&conn, &file, "pc").unwrap();
        let _ = std::fs::remove_file(file);
        assert_eq!(stake_now(), 500);
    }

    /// An older call gets what can be rebuilt: the version auto bet bet on, the last one before picks closed and the
    /// bell's.
    #[test]
    fn older_calls_get_a_rebuilt_history() {
        let conn = db::open_memory().unwrap();
        let p = prediction();
        let id = db::add_call(&conn, &p.to_call()).unwrap();
        db::set_call_stake(&conn, id, 500, "!a", None).unwrap();
        // bet at a lower chance than the call ended on, and called again at the bell
        let b = AutoBet {
            prediction_id: Some(id),
            side: p.pick.clone(),
            amount: 500,
            command: "!a".into(),
            prob: p.prob - 0.03,
            status: "confirmed".into(),
            claimed_at: p.seen_at - 20_000,
            ..Default::default()
        };
        autobet::claim(&conn, &b).unwrap();
        conn.execute(
            "UPDATE predictions SET closed = 1, closed_at = ?2, late_pick = 'home', late_prob = 0.58 WHERE id = ?1",
            params![id, p.seen_at as i64 + 200_000],
        )
        .unwrap();
        assert_eq!(backfill_call(&conn, id).unwrap(), 3);
        let revs = of_call(&conn, id).unwrap();
        assert!(revs.iter().all(Revision::backfilled));
        assert_eq!(
            revs.iter().map(|r| (r.phase.as_str(), r.cause.as_str())).collect::<Vec<_>>(),
            [(PICKS, "first"), (AFTER_BET, "unknown"), (BELL, "bell")]
        );
        assert!((revs[0].prob - (p.prob - 0.03)).abs() < 1e-9);
        assert_eq!(revs[1].stats_from, ["guessed".to_string(), "shown".to_string()]);
        assert_eq!(revs[1].stats, [None, None], "the bell's real stats replaced the ones it saw");
        assert_eq!(backfill_call(&conn, id).unwrap(), 0, "it has a history now");
        // another computer's live history replaces the rebuilt one, and a rebuilt one never replaces a live one
        let live = vec![Revision { n: 1, trigger: "seen".into(), cause: "first".into(), ..revs[1].clone() }];
        assert!(merge(&conn, id, &live).unwrap());
        assert!(!merge(&conn, id, &revs).unwrap());
        assert_eq!(of_call(&conn, id).unwrap(), live);
    }

    /// The analysis: changes during picks, after the bet and at the bell, and what longer holds would have done.
    #[test]
    fn changes_sum_up_the_histories() {
        let conn = db::open_memory().unwrap();
        let p = prediction();
        let id = called(&conn, &p, &stake(500, "!a"));
        // 45s in, the call flips to the other side
        let mut flipped = p.clone();
        flipped.pick = if p.pick == "home" { "away".into() } else { "home".into() };
        flipped.prob = 0.8;
        flipped.home.stats_from = "shown".into();
        record(&conn, id, Revision::of(&flipped, Some(&stake(500, "!h")), Trigger::Seen, p.seen_at + 45_000)).unwrap();
        db::close_call(&conn, &p.home.owner, &p.away.owner, Some(p.seen_at + 100_000)).unwrap();
        let auto = AutoBetSettings { min_chance: 0.5, hidden_stats_from: Some(0.5), ..Default::default() };
        let c = changes(&conn, &auto).unwrap();
        assert_eq!((c.live, c.backfilled, c.moved, c.flipped), (1, 0, 1, 1));
        assert_eq!(c.when, vec![(45, "stats_shown,stake".to_string())]);
        let hold = |secs: u64| c.holds.iter().find(|h| h.secs == secs).unwrap().clone();
        assert_eq!((hold(30).bets, hold(30).differ), (1, 0), "30s in: the first version still stood");
        assert_eq!((hold(60).bets, hold(60).differ), (1, 1), "60s in: the flipped one");
        assert_eq!(hold(120).too_late, 1, "picks closed at 100s");
        assert!(report(&c, &auto).contains("1 of 1 live calls changed"));
    }
}
