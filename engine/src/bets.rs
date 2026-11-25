//! The developer's bets: importing their picks list from palduels.com (signed in), what their wins have paid, and how
//! following Augur's calls and stakes would have done next to what they actually did. Fights they missed (busy, away)
//! are kept apart: they aren't deviations.

use crate::backfill::{self, Control, Progress};
use crate::db::{self, CallRecord};
use crate::error::{Error, Result};
use crate::site::{self, Pick};
use crate::stake::{self, Odds, StakeSettings};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// The picks list shows 5 bets a page.
pub const PER_PAGE: u64 = 5;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Imported {
    /// bets new or changed (a result coming in) this time
    pub new: usize,
    /// bets in the whole picks history
    pub total: u64,
    pub balance: Option<u64>,
}

/// Reads the picks list: newer pages while whole pages are new, then the pages older than anything stored (the first
/// import reads them all, one page at a time, pausing while a duel is up; a stopped one resumes where it left off).
/// `page(n, progress)` returns page n's HTML, None when stopped.
pub fn import(
    conn: &Connection,
    page: &mut dyn FnMut(u32, &mut Progress) -> Option<String>,
    ctl: &Control,
    now_ms: u64,
) -> Result<Imported> {
    let mut p = Progress { phase: "bets".into(), ..Default::default() };
    let Some((first, balance, account)) = read(page, 1, &mut p)? else { return Ok(Imported::default()) };
    if let Some(b) = balance {
        db::meta_set(conn, "balance", &b.to_string())?;
    }
    // the same page has the rest of your profile: kept for the You tab
    db::meta_set(conn, "account_page", &serde_json::to_string(&account)?)?;
    db::meta_set(conn, "account_page_at", &now_ms.to_string())?;
    let mut out = Imported { new: db::save_picks(conn, &first.picks, now_ms)?, total: first.total, balance };
    let (mut at, mut pages, mut total) = (1u32, first.pages, first.total);
    let mut fresh = !first.picks.is_empty() && out.new == first.picks.len();
    loop {
        p.done = conn.query_row("SELECT COUNT(*) FROM bets", [], |r| r.get::<_, i64>(0))? as usize;
        p.total = total as usize;
        (ctl.progress)(&p);
        let oldest = db::oldest_bet(conn)?.unwrap_or(1);
        let next = if fresh {
            at + 1
        } else if oldest > 1 {
            (total.saturating_sub(oldest - 1) / PER_PAGE + 1) as u32
        } else {
            break;
        };
        if next > pages || (next == at && !fresh) || !backfill::wait_while_busy(ctl, &mut p) {
            break;
        }
        let Some((pg, ..)) = read(page, next, &mut p)? else { break };
        let saved = db::save_picks(conn, &pg.picks, now_ms)?;
        out.new += saved;
        fresh = fresh && !pg.picks.is_empty() && saved == pg.picks.len();
        (at, pages, total) = (next, pg.pages, pg.total);
        if !fresh && db::oldest_bet(conn)?.unwrap_or(1) == oldest && saved == 0 {
            break; // nothing older came back: stop rather than loop
        }
    }
    link_unclipped(conn)?;
    db::meta_set(conn, "bets_synced_at", &now_ms.to_string())?;
    Ok(out)
}

/// How far a fight can be from when its bet was first read and still be that bet's (ms). The list is read around
/// every fight, so a few hours is generous; it keeps a first import's old bets from landing on a recent rematch.
const LINK_WINDOW_MS: i64 = 6 * 3_600_000;

/// Gives settled bets with no duel link one from the stored duels. The picks list names a bet's duel only in its
/// clip link, and a fight without a clip (they can come late, or not at all) leaves the bet unlinked, so views by
/// date lose it. A duel is the bet's when it has the same owners, pals and winner, was fought within
/// `LINK_WINDOW_MS` of the bet being read, isn't another bet's, and sits between the duels of the bets before and
/// after it (bets and duels are both numbered in order). More than one such duel: left alone, not guessed.
/// Returns how many were linked.
pub fn link_unclipped(conn: &Connection) -> Result<usize> {
    let mut st = conn.prepare(
        "SELECT b.seq, d.id,
                (SELECT MAX(o.duel_id) FROM bets o WHERE o.seq < b.seq),
                (SELECT MIN(o.duel_id) FROM bets o WHERE o.seq > b.seq)
         FROM bets b JOIN duels d
           ON d.home_owner = b.home_owner AND d.away_owner = b.away_owner AND d.home_pal = b.home_pal
          AND d.away_pal = b.away_pal AND d.winner = b.winner AND d.fought_at LIKE '%T%'
          AND ABS(CAST(strftime('%s', d.fought_at) AS INTEGER) * 1000 - b.seen_at) <= ?1
         WHERE b.duel_id IS NULL AND b.result IS NOT NULL AND b.seen_at > 0
           AND d.id NOT IN (SELECT duel_id FROM bets WHERE duel_id IS NOT NULL)
         ORDER BY b.seq, d.id",
    )?;
    let found: Vec<(i64, i64, Option<i64>, Option<i64>)> =
        st.query_map([LINK_WINDOW_MS], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut taken = std::collections::HashSet::new();
    for bet in found.chunk_by(|a, b| a.0 == b.0) {
        let in_order: Vec<i64> =
            bet.iter().filter(|(_, d, before, after)| before.is_none_or(|b| b < *d) && after.is_none_or(|a| *d < a)).map(|f| f.1).collect();
        let [duel] = in_order[..] else { continue };
        if taken.insert(duel) {
            conn.execute("UPDATE bets SET duel_id = ?1 WHERE seq = ?2 AND duel_id IS NULL", [duel, bet[0].0])?;
        }
    }
    Ok(taken.len())
}

fn read(
    page: &mut dyn FnMut(u32, &mut Progress) -> Option<String>,
    n: u32,
    p: &mut Progress,
) -> Result<Option<(site::PicksPage, Option<u64>, site::AccountPage)>> {
    let Some(body) = page(n, p) else { return Ok(None) };
    match site::parse_picks(&body) {
        Some(pg) => Ok(Some((pg, site::parse_balance(&body), site::parse_account(&body)))),
        None => Err(Error::Parse("your picks: they only show when signed in, so sign in again in Settings".into())),
    }
}

/// What the developer's wins have paid per credit staked: the median of their latest 200.
pub fn odds(conn: &Connection) -> Result<Odds> {
    let mut st = conn.prepare("SELECT stake, result FROM bets WHERE result > 0 AND stake > 0 ORDER BY seq DESC LIMIT 200")?;
    let mut rates: Vec<f64> =
        st.query_map([], |r| Ok(r.get::<_, i64>(1)? as f64 / r.get::<_, i64>(0)? as f64))?.collect::<rusqlite::Result<_>>()?;
    if rates.is_empty() {
        return Ok(Odds::assumed());
    }
    rates.sort_by(f64::total_cmp);
    let mid = rates.len() / 2;
    let rate = if rates.len().is_multiple_of(2) { (rates[mid - 1] + rates[mid]) / 2.0 } else { rates[mid] };
    Ok(Odds { rate, wins: rates.len() })
}

/// Your profile as the last import read it (balance, predictions, duels, spending, titles), and when (ms).
pub fn account_page(conn: &Connection) -> Option<(site::AccountPage, u64)> {
    let page = serde_json::from_str(&db::meta_get(conn, "account_page").ok().flatten()?).ok()?;
    let at = db::meta_get(conn, "account_page_at").ok().flatten()?.parse().ok()?;
    Some((page, at))
}

/// The balance from the last import, if it was shown.
pub fn balance(conn: &Connection) -> Option<u64> {
    db::meta_get(conn, "balance").ok().flatten().and_then(|b| b.parse().ok())
}

// --- following Augur vs what happened ---

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Label {
    /// bet on Augur's pick (any stake), or sat out a call Augur said to sit out
    Followed,
    /// bet on the other side, or bet when Augur said to sit it out
    Deviated,
    /// no bet on a call Augur would have bet on: not counted as a deviation
    Missed,
    /// a bet on a fight Augur didn't call
    NoCall,
    /// the picks list hasn't been read since the fight, or the fight isn't over
    Pending,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fight {
    /// Augur's call on it (None for no-call fights), to open its breakdown
    #[serde(default)]
    pub call_id: Option<i64>,
    pub duel_id: Option<u64>,
    pub home_pal: String,
    pub away_pal: String,
    pub winner: Option<String>,
    pub label: Label,
    /// Augur's pick and chance (None for no-call fights)
    pub pick: Option<String>,
    pub prob: Option<f64>,
    pub backed: Option<String>,
    pub stake: Option<u64>,
    pub result: Option<i64>,
    /// what Augur suggested and what that would have made
    pub augur_stake: u64,
    pub augur_command: String,
    pub augur_result: Option<i64>,
    /// Augur's result is an estimate: a win on a side the developer didn't back, at their usual payout
    pub estimated: bool,
    /// the bet's place in the picks list
    #[serde(default)]
    pub seq: Option<u64>,
    /// whether Augur placed the bet (auto betting) or you did
    #[serde(default)]
    pub via: Option<crate::autobet::Via>,
    /// a missed call auto betting was armed for but sat out: why
    #[serde(default)]
    pub auto_skipped: Option<String>,
    /// the staking settings Augur's stake came from, in a line (None: not saved, or no call)
    #[serde(default)]
    pub staking: Option<String>,
    /// when the call was made (site clock, ms; None for no-call fights)
    #[serde(default)]
    pub at: Option<u64>,
}

/// A stretch of calls staked with the same settings, oldest first, and what each kind of bet made in it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StakingPeriod {
    /// the settings in a line (`stake::summary`); None: calls from before they were saved
    pub staking: Option<String>,
    pub from: u64,
    pub to: u64,
    /// settled calls in it
    pub calls: usize,
    /// Augur's way on every one of them, missed calls included
    pub augur: i64,
    /// your bets on them, and the ones auto betting placed
    pub yours: i64,
    pub auto: i64,
    pub auto_bets: usize,
}

/// Splits the calls into stretches with the same staking settings (`fights` as `compare` gives them, with `via` set),
/// so a change of strategy, or turning auto betting on, can be judged by what came after it.
pub fn staking_periods(fights: &[Fight]) -> Vec<StakingPeriod> {
    let mut calls: Vec<&Fight> = fights.iter().filter(|f| f.call_id.is_some() && f.label != Label::Pending).collect();
    calls.sort_by_key(|f| (f.at.unwrap_or(0), f.duel_id.unwrap_or(0)));
    let mut out: Vec<StakingPeriod> = Vec::new();
    for f in calls {
        let at = f.at.unwrap_or(0);
        if out.last().is_none_or(|p| p.staking != f.staking) {
            out.push(StakingPeriod { staking: f.staking.clone(), from: at, ..Default::default() });
        }
        let p = out.last_mut().expect("just pushed");
        p.to = at;
        p.calls += 1;
        p.augur += f.augur_result.unwrap_or(0);
        if matches!(f.label, Label::Followed | Label::Deviated) {
            p.yours += f.result.unwrap_or(0);
        }
        if f.via.as_ref().is_some_and(|v| v.via == "auto") {
            p.auto += f.result.unwrap_or(0);
            p.auto_bets += 1;
        }
    }
    out
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Comparison {
    /// newest first
    pub fights: Vec<Fight>,
    pub followed: usize,
    pub deviated: usize,
    pub missed: usize,
    pub no_call: usize,
    pub pending: usize,
    /// calls the developer bet on (followed or deviated with a bet; sitting out with Augur counts as followed)
    pub bet_on: usize,
    /// on the calls the developer bet on: what they made, and what Augur's pick and stake would have made
    pub yours: i64,
    pub augur: i64,
    /// what Augur's stakes would have made on the calls they missed
    pub missed_augur: i64,
    /// their bets on fights Augur didn't call
    pub no_call_yours: i64,
    pub odds: Odds,
    /// oldest first (`staking_periods`, once the fights know which bets auto betting placed)
    #[serde(default)]
    pub staking: Vec<StakingPeriod>,
}

/// Lines up Augur's calls (oldest first, as `db::settled` gives them) with the developer's bets. `synced_ms`: when the
/// picks list was last read (calls after it are pending). Calls without a saved stake get one from today's settings.
pub fn compare(calls: &[CallRecord], bets: &[Pick], s: &StakeSettings, odds: &Odds, synced_ms: u64) -> Comparison {
    let mut c = Comparison { odds: odds.clone(), ..Default::default() };
    let mut used = std::collections::HashSet::new();
    for call in calls {
        let bet = bets.iter().find(|b| match (b.duel_id, call.duel_id) {
            (Some(b), Some(d)) => b == d,
            _ => {
                b.home_owner == call.home_owner
                    && b.away_owner == call.away_owner
                    && b.home_pal == call.home_pal
                    && b.away_pal == call.away_pal
            }
        });
        if let Some(b) = bet {
            used.insert(b.seq);
        }
        // its call was overwritten after the fight, so its stake isn't one you were shown: left out of the money
        if call.damaged {
            continue;
        }
        let suggested = stake::suggest(&call.pick, call.prob, s, odds);
        let amount = call.stake.unwrap_or(suggested.amount);
        let won = call.winner.as_deref() == Some(call.pick.as_str());
        // the developer's bet on Augur's side shows exactly what that side paid, scaled to Augur's stake
        let (augur_result, estimated) = match bet {
            _ if amount == 0 => (Some(0), false),
            Some(b) if b.backed == call.pick && b.stake > 0 => {
                (b.result.map(|r| (r as f64 * amount as f64 / b.stake as f64).round() as i64), false)
            }
            _ if won => (Some((amount as f64 * odds.rate).round() as i64), true),
            _ => (Some(-(amount as i64)), false),
        };
        let label = match bet {
            Some(b) if b.result.is_none() => Label::Pending,
            Some(b) if amount == 0 || b.backed != call.pick => Label::Deviated,
            Some(_) => Label::Followed,
            None if synced_ms <= call.seen_at.max(0) as u64 || call.winner.is_none() => Label::Pending,
            None if amount == 0 => Label::Followed,
            None => Label::Missed,
        };
        let result = bet.and_then(|b| b.result);
        if bet.is_some() && matches!(label, Label::Followed | Label::Deviated) {
            c.bet_on += 1;
        }
        match label {
            Label::Followed if bet.is_some() => {
                (c.followed, c.yours, c.augur) = (c.followed + 1, c.yours + result.unwrap_or(0), c.augur + augur_result.unwrap_or(0))
            }
            Label::Followed => c.followed += 1,
            Label::Deviated => {
                (c.deviated, c.yours, c.augur) = (c.deviated + 1, c.yours + result.unwrap_or(0), c.augur + augur_result.unwrap_or(0))
            }
            Label::Missed => (c.missed, c.missed_augur) = (c.missed + 1, c.missed_augur + augur_result.unwrap_or(0)),
            Label::Pending => c.pending += 1,
            Label::NoCall => {}
        }
        c.fights.push(Fight {
            call_id: Some(call.id),
            duel_id: call.duel_id,
            home_pal: call.home_pal.clone(),
            away_pal: call.away_pal.clone(),
            winner: call.winner.clone(),
            label,
            pick: Some(call.pick.clone()),
            prob: Some(call.prob),
            backed: bet.map(|b| b.backed.clone()),
            stake: bet.map(|b| b.stake),
            result,
            augur_stake: amount,
            augur_command: stake::command(&call.pick, amount, call.staking.as_ref().unwrap_or(s)),
            augur_result,
            estimated: estimated && label != Label::Pending,
            seq: bet.map(|b| b.seq),
            via: None,
            auto_skipped: None,
            staking: call.staking.as_ref().map(stake::summary),
            at: Some(call.seen_at.max(0) as u64),
        });
    }
    for b in bets.iter().filter(|b| !used.contains(&b.seq) && b.result.is_some()) {
        c.no_call += 1;
        c.no_call_yours += b.result.unwrap_or(0);
        c.fights.push(Fight {
            call_id: None,
            duel_id: b.duel_id,
            home_pal: b.home_pal.clone(),
            away_pal: b.away_pal.clone(),
            winner: b.winner.clone(),
            label: Label::NoCall,
            pick: None,
            prob: None,
            backed: Some(b.backed.clone()),
            stake: Some(b.stake),
            result: b.result,
            augur_stake: 0,
            augur_command: String::new(),
            augur_result: None,
            estimated: false,
            seq: Some(b.seq),
            via: None,
            auto_skipped: None,
            staking: None,
            at: None,
        });
    }
    // newest first: by duel number where both have one, calls (made live) ahead of old unlinked bets
    c.fights.sort_by_key(|f| std::cmp::Reverse(f.duel_id.unwrap_or(0)));
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn seeded() -> Connection {
        let conn = crate::seed::store();
        let stop = AtomicBool::new(false);
        let ctl = Control { stop: &stop, busy: &|| false, progress: &|_| {} };
        let mut pages = |n: u32, _: &mut Progress| Some(crate::seed::page(&format!("picks-page-{n}.html")));
        let got = import(&conn, &mut pages, &ctl, crate::seed::BETS_READ_AT).unwrap();
        assert_eq!((got.new, got.total, got.balance), (13, 13, Some(248_000)));
        conn
    }

    #[test]
    fn the_whole_picks_list_imports_once_then_only_whats_new() {
        let conn = seeded();
        let bets = db::bets(&conn).unwrap();
        assert_eq!(bets.iter().map(|b| b.seq).collect::<Vec<_>>(), (1..=13).rev().collect::<Vec<_>>());
        let stop = AtomicBool::new(false);
        let ctl = Control { stop: &stop, busy: &|| false, progress: &|_| {} };
        let read = std::cell::RefCell::new(vec![]);
        let mut pages = |n: u32, _: &mut Progress| {
            read.borrow_mut().push(n);
            Some(crate::seed::page(&format!("picks-page-{n}.html")))
        };
        let again = import(&conn, &mut pages, &ctl, 1_794_400_100_000).unwrap();
        assert_eq!(again.new, 0);
        assert_eq!(*read.borrow(), [1], "nothing new on page 1 and nothing older missing: one page read");
        assert_eq!(balance(&conn), Some(248_000));
        let (account, at) = account_page(&conn).unwrap();
        assert_eq!((account.balance, account.titles_earned, at), (Some(248_120), Some((16, 52)), 1_794_400_100_000));
    }

    /// A duel between two made-up owners' pals, fought `mins` after the seed's picks list was read.
    fn duel(conn: &Connection, id: i64, mins: i64, winner: &str) {
        let at: String = conn
            .query_row(
                "SELECT strftime('%Y-%m-%dT%H:%M:%SZ', ?1, 'unixepoch')",
                [crate::seed::BETS_READ_AT as i64 / 1000 + mins * 60],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO duels (id, fought_at, home_owner, away_owner, home_pal, away_pal, winner)
             VALUES (?1, ?2, 'player-01', 'player-02', 'Lamball', 'Cattiva', ?3)",
            rusqlite::params![id, at, winner],
        )
        .unwrap();
    }

    /// A settled bet on that pairing as the picks list shows it, with or without a clip.
    fn pick(seq: u64, winner: &str, duel_id: Option<u64>) -> Pick {
        Pick {
            seq,
            duel_id,
            home_owner: "player-01".into(),
            away_owner: "player-02".into(),
            home_pal: "Lamball".into(),
            away_pal: "Cattiva".into(),
            backed: "home".into(),
            winner: Some(winner.into()),
            stake: 500,
            result: Some(if winner == "home" { 150 } else { -500 }),
            ..Default::default()
        }
    }

    fn link_of(conn: &Connection, seq: u64) -> Option<u64> {
        db::bets(conn).unwrap().iter().find(|b| b.seq == seq).unwrap().duel_id
    }

    #[test]
    fn a_row_with_no_clip_names_no_duel() {
        let page = crate::seed::page("picks-page-1.html");
        let no_clips = regex::Regex::new(r#"(?s)<a class="clip-i".*?</a>"#).unwrap().replace_all(&page, "");
        let (with, without) = (site::parse_picks(&page).unwrap().picks, site::parse_picks(&no_clips).unwrap().picks);
        assert!(with.iter().all(|p| p.duel_id.is_some()) && without.iter().all(|p| p.duel_id.is_none()));
        let unlinked: Vec<Pick> = with.iter().cloned().map(|p| Pick { duel_id: None, ..p }).collect();
        assert_eq!(without, unlinked, "the rest of the row reads the same");
    }

    #[test]
    fn a_bet_with_no_clip_takes_its_duels_id_and_keeps_it() {
        let conn = seeded();
        let now = crate::seed::BETS_READ_AT + 60_000;
        duel(&conn, 9101, -2, "home");
        assert_eq!(db::save_picks(&conn, &[pick(14, "home", None)], now).unwrap(), 1);
        assert_eq!((link_unclipped(&conn).unwrap(), link_of(&conn, 14)), (1, Some(9101)));
        // read again, still with no clip: nothing new, and the link stays
        assert_eq!(db::save_picks(&conn, &[pick(14, "home", None)], now + 1).unwrap(), 0);
        assert_eq!((link_unclipped(&conn).unwrap(), link_of(&conn, 14)), (0, Some(9101)));
        // the site's own link, once the clip is there, is the one kept
        assert_eq!(db::save_picks(&conn, &[pick(14, "home", Some(9102))], now + 2).unwrap(), 1);
        assert_eq!(link_of(&conn, 14), Some(9102));
    }

    #[test]
    fn a_rematch_is_never_guessed() {
        let conn = seeded();
        let now = crate::seed::BETS_READ_AT + 60_000;
        for (id, mins, winner) in [(9101, -90, "home"), (9102, -50, "away"), (9103, -2, "home")] {
            duel(&conn, id, mins, winner);
        }
        db::save_picks(&conn, &[pick(14, "home", None), pick(15, "away", None)], now).unwrap();
        // the lost one has one fight it can be; the won one has two
        assert_eq!(link_unclipped(&conn).unwrap(), 1);
        assert_eq!((link_of(&conn, 14), link_of(&conn, 15)), (None, Some(9102)));
        // which leaves the earlier win as the only one before it
        assert_eq!((link_unclipped(&conn).unwrap(), link_of(&conn, 14)), (1, Some(9101)));
        // and a later bet on the third fight can't take a duel that's another bet's
        db::save_picks(&conn, &[pick(16, "home", None)], now).unwrap();
        assert_eq!((link_unclipped(&conn).unwrap(), link_of(&conn, 16)), (1, Some(9103)));
    }

    #[test]
    fn old_bets_and_far_off_fights_stay_unlinked() {
        let conn = seeded();
        // seq 3 is one of the seed's old bets with no link; the bets after it are on duels 8001 and up
        let old = db::bets(&conn).unwrap().into_iter().find(|b| b.seq == 3).unwrap();
        assert_eq!(old.duel_id, None);
        conn.execute(
            "INSERT INTO duels (id, fought_at, home_owner, away_owner, home_pal, away_pal, winner)
             SELECT 9500, strftime('%Y-%m-%dT%H:%M:%SZ', seen_at / 1000 - 60, 'unixepoch'), home_owner, away_owner, home_pal, away_pal, winner
             FROM bets WHERE seq = 3",
            [],
        )
        .unwrap();
        assert_eq!((link_unclipped(&conn).unwrap(), link_of(&conn, 3)), (0, None), "a newer duel than the next bet's");
        duel(&conn, 9101, -7 * 60, "home");
        db::save_picks(&conn, &[pick(14, "home", None)], crate::seed::BETS_READ_AT).unwrap();
        assert_eq!((link_unclipped(&conn).unwrap(), link_of(&conn, 14)), (0, None), "fought 7 hours before the read");
        // a bet whose fight is still on has no winner to check yet
        let conn = seeded();
        duel(&conn, 9102, -1, "home");
        db::save_picks(&conn, &[Pick { result: None, winner: None, ..pick(15, "home", None) }], crate::seed::BETS_READ_AT).unwrap();
        assert_eq!((link_unclipped(&conn).unwrap(), link_of(&conn, 15)), (0, None));
    }

    #[test]
    fn a_signed_out_page_is_an_error() {
        let conn = db::open_memory().unwrap();
        let stop = AtomicBool::new(false);
        let ctl = Control { stop: &stop, busy: &|| false, progress: &|_| {} };
        let mut pages = |_: u32, _: &mut Progress| Some(crate::seed::page("user-trongateslegacy.html"));
        assert!(import(&conn, &mut pages, &ctl, 1).unwrap_err().to_string().contains("sign in"));
    }

    #[test]
    fn payout_rate_is_the_median_win() {
        let conn = seeded();
        // the seed's 8 wins pay 0.42 to 2.83 a credit; the middle two are 512/1000 and 277/500
        let o = odds(&conn).unwrap();
        assert_eq!(o.wins, 8);
        assert!((o.rate - (0.512 + 0.554) / 2.0).abs() < 1e-9, "{}", o.rate);
        assert_eq!(odds(&db::open_memory().unwrap()).unwrap(), Odds::assumed());
    }

    /// A call marked damaged (overwritten after the fight) is left out of the money comparison, and its bet isn't
    /// counted as one on a fight Augur didn't call.
    #[test]
    fn damaged_calls_stay_out_of_the_money() {
        let conn = seeded();
        let mut calls = db::settled(&conn).unwrap();
        let bets = db::bets(&conn).unwrap();
        let (s, o) = (StakeSettings::default(), Odds { rate: 0.5, wins: 8 });
        let all = compare(&calls, &bets, &s, &o, crate::seed::BETS_READ_AT);
        let hit = calls.iter_mut().find(|c| c.duel_id == Some(9019)).unwrap();
        hit.damaged = true;
        let some = compare(&calls, &bets, &s, &o, crate::seed::BETS_READ_AT);
        assert!(some.fights.iter().all(|f| f.duel_id != Some(9019)));
        assert_eq!(some.fights.len(), all.fights.len() - 1, "its bet doesn't reappear as a no-call fight");
    }

    #[test]
    fn following_augur_vs_what_happened() {
        let conn = seeded();
        let calls = db::settled(&conn).unwrap();
        let bets = db::bets(&conn).unwrap();
        let s = StakeSettings { strategy: stake::Strategy::DefaultOrMax, ..Default::default() };
        let o = Odds { rate: 0.5, wins: 8 };
        let c = compare(&calls, &bets, &s, &o, crate::seed::BETS_READ_AT);
        let fight = |d: u64| c.fights.iter().find(|f| f.duel_id == Some(d)).unwrap();
        // 9019: Augur away at 68% (default bet, !a); you backed away with 5,000 and lost
        assert_eq!((fight(9019).label, fight(9019).augur_command.as_str(), fight(9019).augur_result), (Label::Followed, "!a", Some(-500)));
        // 9018: Augur home at 75% (max); you backed home with 5,000 and won 2,105: Augur's max makes the same
        assert_eq!((fight(9018).label, fight(9018).augur_result, fight(9018).estimated), (Label::Followed, Some(2105), false));
        // 9016: you backed home against Augur's away pick, and away won: Augur's 500 at the usual payout
        assert_eq!((fight(9016).label, fight(9016).result, fight(9016).augur_result), (Label::Deviated, Some(-500), Some(250)));
        assert!(fight(9016).estimated);
        // 9015: you went against Augur's 85% home pick on away, and it paid
        assert_eq!((fight(9015).label, fight(9015).result, fight(9015).augur_result), (Label::Deviated, Some(1290), Some(-5000)));
        // 9017: no bet on a call Augur would have bet: missed, not a deviation
        assert_eq!(fight(9017).label, Label::Missed);
        // 9014: followed for 500, scaled exactly
        assert_eq!((fight(9014).label, fight(9014).augur_result), (Label::Followed, Some(310)));
        assert_eq!(fight(9020).label, Label::NoCall);
        assert_eq!((c.followed, c.deviated, c.no_call, c.bet_on), (3, 2, 8, 5));
        assert_eq!(c.missed, calls.len() - 5, "every other call had no bet");
        assert_eq!((c.yours, c.augur), (-5000 + 2105 - 500 + 1290 + 310, -500 + 2105 + 250 - 5000 + 310));
        assert_eq!(c.no_call_yours, 277 + 512 - 500 + 230 + 480 + 50 - 100 - 200);
        let before = compare(&calls, &bets, &s, &o, 0);
        assert_eq!(before.missed, 0, "not judged before the picks list was read");
    }

    /// Calls split into stretches where the staking settings change, each with what Augur's way, your bets and the
    /// auto bets made; pending calls and fights Augur didn't call are left out.
    #[test]
    fn staking_periods_split_where_the_settings_change() {
        let fight = |at: u64, staking: Option<&str>, label: Label, augur: i64, yours: Option<i64>, auto: bool| Fight {
            call_id: Some(at as i64),
            duel_id: Some(at),
            home_pal: "A".into(),
            away_pal: "B".into(),
            winner: Some("home".into()),
            label,
            pick: Some("home".into()),
            prob: Some(0.7),
            backed: yours.map(|_| "home".into()),
            stake: yours.map(|_| 500),
            result: yours,
            augur_stake: 500,
            augur_command: "!h".into(),
            augur_result: Some(augur),
            estimated: false,
            seq: None,
            via: auto.then(|| crate::autobet::Via { via: "auto".into(), status: None, double: false }),
            auto_skipped: None,
            staking: staking.map(String::from),
            at: Some(at),
        };
        let mut no_call = fight(5, None, Label::NoCall, 0, Some(70), false);
        no_call.call_id = None;
        // newest first, as compare gives them
        let fights = vec![
            fight(9, Some("max"), Label::Pending, 0, None, false),
            fight(8, Some("max"), Label::Followed, 300, Some(300), true),
            fight(7, Some("max"), Label::Missed, -500, None, false),
            no_call,
            fight(4, Some("scaled"), Label::Deviated, 200, Some(-100), false),
            fight(3, Some("scaled"), Label::Followed, -400, Some(-400), false),
            fight(1, None, Label::Followed, 100, Some(100), false),
        ];
        let p = staking_periods(&fights);
        let got: Vec<_> = p.iter().map(|p| (p.staking.as_deref(), p.from, p.to, p.calls, p.augur, p.yours, p.auto, p.auto_bets)).collect();
        assert_eq!(
            got,
            vec![(None, 1, 1, 1, 100, 100, 0, 0), (Some("scaled"), 3, 4, 2, -200, -500, 0, 0), (Some("max"), 7, 8, 2, -200, 300, 300, 1),]
        );
    }
}
