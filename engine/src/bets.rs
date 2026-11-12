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
    db::meta_set(conn, "bets_synced_at", &now_ms.to_string())?;
    Ok(out)
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
            augur_command: stake::command(&call.pick, amount, s),
            augur_result,
            estimated: estimated && label != Label::Pending,
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
}
