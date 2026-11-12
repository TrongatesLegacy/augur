//! What each page is sent, built from the store without the desktop app, so the app's commands, `augur ui-fixtures`
//! (the data the headless UI tests render) and anything else share one version.

use augur_engine::backfill::Progress;
use augur_engine::bets::{self, Comparison};
use augur_engine::history::{self, Backtest};
use augur_engine::predict::Prediction;
use augur_engine::site::{BoxPal, QueueItem};
use augur_engine::stake::{Stake, StakeSettings};
use augur_engine::{db, model, rusqlite};
use rusqlite::Connection;
use serde::Serialize;

use augur_engine::Result as R;

#[derive(Clone, Debug, Default, Serialize)]
pub struct LastResult {
    pub home_pal: String,
    pub away_pal: String,
    pub winner_pal: String,
    pub right: bool,
    pub prob: f64,
    pub ko: Option<String>,
    pub hp_left: Option<u32>,
}

/// What the Live tab and the popover show.
#[derive(Clone, Debug, Default, Serialize)]
pub struct LiveView {
    pub title: String,
    pub live: bool,
    pub picks_open: bool,
    pub prediction: Option<Prediction>,
    /// what to bet on the pick, and the chat command for it
    pub stake: Option<Stake>,
    pub queue: Vec<QueueItem>,
    pub last_result: Option<LastResult>,
    pub synced_at: u64,
    pub error: Option<String>,
    /// today's calls: (right, total)
    pub today: (usize, usize),
    /// a fresh install that hasn't chosen yet: import a backup, or start fresh
    pub first_run: bool,
    /// between duels and off air: how the last stream went
    pub off_air: Option<OffAir>,
}

/// What the popover and Live show when no duel is up.
#[derive(Clone, Debug, Default, Serialize)]
pub struct OffAir {
    /// the last stream day with calls: date, right, total, best run
    pub last_stream: Option<(String, usize, usize, usize)>,
    /// on that day's calls you bet: what you made, and what following Augur would have
    pub last_bets: Option<(i64, i64)>,
    /// every call: right, total
    pub all_time: (usize, usize),
    /// the latest calls, newest first: (pal called, beaten pal, right)
    pub recent: Vec<(String, String, bool)>,
    /// the latest stream days, oldest first: (date, right, total)
    pub days: Vec<(String, usize, usize)>,
    /// what's stored: duels, fight tapes, species, your bets
    pub store: (i64, i64, i64, i64),
}

/// How the last stream went, for the quiet times.
pub fn off_air(conn: &Connection, staking: &StakeSettings) -> R<OffAir> {
    let calls = db::settled(conn)?;
    let all = history::tally(&calls, None);
    let mut out = OffAir { all_time: (all.right, all.total), ..Default::default() };
    let dated: Vec<_> = all.by_day.iter().filter(|(d, ..)| !d.is_empty()).cloned().collect();
    out.days = dated[dated.len().saturating_sub(10)..].to_vec();
    let count = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0);
    out.store = (
        count("SELECT COUNT(*) FROM duels"),
        count("SELECT COUNT(DISTINCT duel_id) FROM hits"),
        count("SELECT COUNT(*) FROM species"),
        count("SELECT COUNT(*) FROM bets"),
    );
    if let Some((day, right, total)) = all.by_day.iter().rev().find(|(d, ..)| !d.is_empty()).cloned() {
        let on_day: Vec<db::CallRecord> =
            calls.iter().filter(|c| c.fought_at.as_deref().is_some_and(|f| f.starts_with(&day))).cloned().collect();
        out.last_stream = Some((day, right, total, history::tally(&on_day, None).best_run));
        let synced = db::meta_get(conn, "bets_synced_at")?.and_then(|v| v.parse().ok()).unwrap_or(0);
        let c = bets::compare(&on_day, &db::bets(conn)?, staking, &bets::odds(conn)?, synced);
        if c.bet_on > 0 {
            out.last_bets = Some((c.yours, c.augur));
        }
    }
    out.recent = calls
        .iter()
        .rev()
        .take(3)
        .map(|c| {
            let (pick, other) = if c.pick == "home" { (&c.home_pal, &c.away_pal) } else { (&c.away_pal, &c.home_pal) };
            (pick.clone(), other.clone(), c.right())
        })
        .collect();
    Ok(out)
}

/// YYYY-MM-DD (UTC) for a Unix time, without a date library.
pub fn civil_date(secs: u64) -> String {
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", yoe + era * 400 + (m <= 2) as i64, m, d)
}

#[derive(Serialize)]
pub struct HistoryView {
    pub tally: history::Tally,
    pub calls: Vec<db::CallRecord>,
    pub rescored_right: usize,
    pub rescored_total: usize,
}

/// The first day in a range ("today", "week" or "all": None) as YYYY-MM-DD.
fn since(range: &str, now_secs: u64) -> Option<String> {
    match range {
        "today" => Some(civil_date(now_secs)),
        "week" => Some(civil_date(now_secs.saturating_sub(7 * 86_400))),
        _ => None,
    }
}

/// Settled calls fought in the range.
fn calls_in(conn: &Connection, range: &str, now_secs: u64) -> R<Vec<db::CallRecord>> {
    let since = since(range, now_secs);
    Ok(db::settled(conn)?.into_iter().filter(|c| since.as_deref().is_none_or(|s| c.fought_at.as_deref().unwrap_or("") >= s)).collect())
}

/// range: "today", "week" or "all"; now_secs: the current Unix time.
pub fn history_view(conn: &Connection, s: &model::Settings, range: &str, now_secs: u64) -> R<HistoryView> {
    let calls = calls_in(conn, range, now_secs)?;
    let (_, rescored_right, rescored_total) = history::evaluate(&calls, s);
    let tally = history::tally(&calls, None);
    let mut calls = calls;
    calls.reverse();
    for c in calls.iter_mut() {
        c.fighters = None; // the list doesn't need them
    }
    Ok(HistoryView { tally, calls, rescored_right, rescored_total })
}

#[derive(Serialize)]
pub struct BetsView {
    pub comparison: Comparison,
    pub balance: Option<u64>,
    /// when the picks list was last read (ms), None before the first import
    pub synced_at: Option<u64>,
    pub stored: usize,
    pub import: Progress,
}

/// The developer's bets next to Augur's calls in the range. Bets on fights Augur didn't call count in the range when
/// their duel's date is known (all of them for "all").
pub fn bets_view(conn: &Connection, s: &StakeSettings, range: &str, now_secs: u64, import: Progress) -> R<BetsView> {
    let calls = calls_in(conn, range, now_secs)?;
    let since = since(range, now_secs);
    let dates: std::collections::HashMap<u64, Option<String>> = conn
        .prepare("SELECT b.seq, d.fought_at FROM bets b LEFT JOIN duels d ON d.id = b.duel_id")?
        .query_map([], |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let bets: Vec<_> = db::bets(conn)?
        .into_iter()
        .filter(|b| since.as_deref().is_none_or(|s| dates.get(&b.seq).cloned().flatten().is_some_and(|d| d.as_str() >= s)))
        .collect();
    let synced_at = db::meta_get(conn, "bets_synced_at")?.and_then(|v| v.parse().ok());
    let balance = bets::balance(conn);
    let mut comparison = bets::compare(&calls, &bets, s, &bets::odds(conn)?, synced_at.unwrap_or(0));
    comparison.fights.truncate(200);
    let stored = conn.query_row("SELECT COUNT(*) FROM bets", [], |r| r.get::<_, i64>(0))? as usize;
    Ok(BetsView { comparison, balance, synced_at, stored, import })
}

/// The You tab: your signed-in profile as the picks list import last read it, and your bets next to Augur's.
#[derive(Serialize)]
pub struct YouView {
    pub name: Option<String>,
    pub page: Option<augur_engine::site::AccountPage>,
    /// when the profile was read (ms)
    pub read_at: Option<u64>,
    pub comparison: Comparison,
    /// Augur's calls: right, total
    pub augur_calls: (usize, usize),
    /// your pet and your Hall, as last read
    pub pet: Option<augur_engine::site::Pet>,
    pub hall: Option<augur_engine::site::Hall>,
}

pub fn you_view(conn: &Connection, s: &StakeSettings) -> R<YouView> {
    let page = bets::account_page(conn);
    let synced = db::meta_get(conn, "bets_synced_at")?.and_then(|v| v.parse().ok()).unwrap_or(0);
    let calls = db::settled(conn)?;
    let t = history::tally(&calls, None);
    let mut comparison = bets::compare(&calls, &db::bets(conn)?, s, &bets::odds(conn)?, synced);
    comparison.fights.clear();
    Ok(YouView {
        name: db::meta_get(conn, "account")?,
        read_at: page.as_ref().map(|(_, at)| *at),
        page: page.map(|(p, _)| p),
        comparison,
        augur_calls: (t.right, t.total),
        pet: db::meta_get(conn, "pet_page")?.and_then(|p| serde_json::from_str(&p).ok()).flatten(),
        hall: db::meta_get(conn, "hall_page")?.and_then(|h| serde_json::from_str(&h).ok()).flatten(),
    })
}

#[derive(Serialize)]
pub struct BoxPalView {
    #[serde(flatten)]
    pub pal: BoxPal,
    pub rating: f64,
    pub advice: String,
}

#[derive(Serialize)]
pub struct PlayerView {
    pub detail: history::PlayerDetail,
    pub roster: Vec<BoxPalView>,
    pub roster_error: Option<String>,
    pub rerolled_pals: u32,
}

/// roster: the player's pal box from their profile page, or why it couldn't be read.
pub fn player_view(conn: &Connection, s: &model::Settings, name: &str, roster: R<Vec<BoxPal>>) -> R<PlayerView> {
    let detail = history::player(conn, name)?;
    let (roster, roster_error) = match roster {
        Ok(r) => (r, None),
        Err(err) => (vec![], Some(err.to_string())),
    };
    let rerolled_pals = roster.iter().filter(|p| p.rerolls > 0).count() as u32;
    let roster = roster
        .into_iter()
        .map(|p| {
            let st = |k: &str| p.stats.get(k).copied().unwrap_or((0.0, 0.0));
            let rating = history::box_rating(conn, s, &p.pal, st("HP").0, st("ATK").0, st("DEF").0, &p.passives).unwrap_or(0.5);
            let worst_iv = ["HP", "ATK", "DEF"].iter().map(|k| st(k).1).fold(100.0, f64::min);
            // rough until the reroll cost is known: keep strong pals, reroll good species let down by one roll,
            // replace weak species (no roll can fix them)
            let advice = if rating >= 0.75 {
                "Keep"
            } else if p.power.unwrap_or(0) >= 30 && worst_iv < 40.0 {
                "Reroll"
            } else if p.power.unwrap_or(0) < 30 && rating < 0.4 {
                "Replace"
            } else {
                "Maybe"
            };
            BoxPalView { pal: p, rating, advice: advice.into() }
        })
        .collect();
    Ok(PlayerView { detail, roster, roster_error, rerolled_pals })
}

#[derive(Serialize)]
pub struct ModelView {
    pub settings: model::Settings,
    pub live_loss: f64,
    pub live_right: usize,
    pub live_total: usize,
    pub calls_right: usize,
    pub calibration: Option<(usize, usize, f64, f64)>,
    pub backtest: Option<Backtest>,
    pub suggestion: Option<(f64, usize, model::Settings)>,
    pub duels: i64,
    pub tapes: i64,
    pub hits: i64,
    pub species: i64,
    pub backfill: Progress,
    pub synced_at: u64,
    pub unlisted: Vec<history::UnlistedLeader>,
}

/// backtest: a cached run for these settings, if there is one (otherwise it's run, about a second).
pub fn model_view(conn: &Connection, s: &model::Settings, backtest: Option<Backtest>, backfill: Progress, synced_at: u64) -> R<ModelView> {
    let calls = db::settled(conn)?;
    let (live_loss, live_right, live_total) = history::evaluate(&calls, s);
    let backtest = match backtest {
        Some(b) => b,
        None => history::backtest(conn, s)?,
    };
    let suggestion = history::tune(&calls, s, &[("TYPE_BONUS", vec![1.0, 1.25, 1.5, 2.0])])
        .into_iter()
        .next()
        .filter(|(loss, _, cand)| cand != s && *loss < live_loss - 0.005);
    let count = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0);
    Ok(ModelView {
        calibration: history::calibration(conn, &calls, s)?,
        calls_right: calls.iter().filter(|c| c.right()).count(),
        settings: s.clone(),
        live_loss,
        live_right,
        live_total,
        backtest: Some(backtest),
        suggestion,
        duels: count("SELECT COUNT(*) FROM duels"),
        tapes: count("SELECT COUNT(DISTINCT duel_id) FROM hits"),
        hits: count("SELECT COUNT(*) FROM hits"),
        species: count("SELECT COUNT(*) FROM species"),
        backfill,
        synced_at,
        unlisted: history::unlisted_leaders(conn, 300.0, 6)?,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn civil_dates() {
        assert_eq!(super::civil_date(0), "1970-01-01");
        assert_eq!(super::civil_date(1_794_355_200), "2026-11-11");
    }
}
