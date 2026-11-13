//! What each page is sent, built from the store without the desktop app, so the app's commands, `augur ui-fixtures`
//! (the data the headless UI tests render) and anything else share one version.

use augur_engine::backfill::Progress;
use augur_engine::bets::{self, Comparison};
use augur_engine::history::{self, Backtest};
use augur_engine::predict::{self, Prediction};
use augur_engine::site::{self, BoxPal, QueueItem};
use augur_engine::stake::{Stake, StakeSettings};
use augur_engine::{db, learn, model, rusqlite};
use rusqlite::{Connection, OptionalExtension};
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
    /// the call on the duel that's up, broken down like a settled one in History (no fight yet)
    pub detail: Option<CallDetail>,
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
    /// the latest calls, newest first: (pal called, the other pal, right, the chance it gave)
    pub recent: Vec<(String, String, bool, f64)>,
    /// the latest stream days, oldest first: (date, right, total)
    pub days: Vec<(String, usize, usize)>,
    /// what's stored: duels, fight tapes, species, your bets
    pub store: (i64, i64, i64, i64),
}

/// Enough latest calls to fill Live's list when off air (the popover shows the first few).
const RECENT_CALLS: usize = 10;

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
        .take(RECENT_CALLS)
        .map(|c| {
            let (pick, other) = if c.pick == "home" { (&c.home_pal, &c.away_pal) } else { (&c.away_pal, &c.home_pal) };
            (pick.clone(), other.clone(), c.right(), c.prob)
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
    /// the algorithm's eras (version or settings changes), newest first, and how each did
    pub eras: Vec<history::Era>,
    /// the model's last self-tuning run, and the changes it has kept (newest first)
    pub learned: Option<learn::Learned>,
    pub changes: Vec<learn::Change>,
    /// false while you've set the model yourself (Settings › Model)
    pub auto: bool,
    pub duels: i64,
    pub tapes: i64,
    pub hits: i64,
    pub species: i64,
    pub backfill: Progress,
    pub synced_at: u64,
    pub unlisted: Vec<history::UnlistedLeader>,
}

/// backtest: a cached run for these settings, if there is one (otherwise it's run, about a second).
pub fn model_view(
    conn: &Connection,
    s: &model::Settings,
    auto: bool,
    backtest: Option<Backtest>,
    backfill: Progress,
    synced_at: u64,
) -> R<ModelView> {
    let calls = db::settled(conn)?;
    let (live_loss, live_right, live_total) = history::evaluate(&calls, s);
    let backtest = match backtest {
        Some(b) => b,
        None => history::backtest(conn, s)?,
    };
    let count = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0);
    Ok(ModelView {
        calibration: history::calibration(conn, &calls, s)?,
        calls_right: calls.iter().filter(|c| c.right()).count(),
        settings: s.clone(),
        live_loss,
        live_right,
        live_total,
        backtest: Some(backtest),
        eras: history::eras(&calls).into_iter().take(4).collect(),
        learned: learn::load(conn),
        changes: learn::changes(conn),
        auto,
        duels: count("SELECT COUNT(*) FROM duels"),
        tapes: count("SELECT COUNT(DISTINCT duel_id) FROM hits"),
        hits: count("SELECT COUNT(*) FROM hits"),
        species: count("SELECT COUNT(*) FROM species"),
        backfill,
        synced_at,
        unlisted: history::unlisted_leaders(conn, 300.0, 6)?,
    })
}

// --- one call's breakdown ---

/// A call opened from History: the call and its duel, how the fight went (from its tape), and how the model came to
/// its pick (the three factors on the log-odds scale, re-run with the settings the call was made with).
#[derive(Clone, Debug, Serialize)]
pub struct CallDetail {
    /// with its fighters as seen before the bell (None for calls logged before fighters were kept)
    pub call: db::CallRecord,
    /// each pal's art: [home, away]
    pub slugs: [String; 2],
    /// where each side's stats came from ("shown", "from their box", "guessed (…)"); empty when not recorded
    pub stats_from: [String; 2],
    /// some stats were hidden until the bell and guessed
    pub estimated: bool,
    /// the chat command Augur suggested, e.g. !h 2300
    pub stake_cmd: Option<String>,
    /// the model settings the call was made with (None: not saved with it; today's defaults stand in)
    pub settings: Option<model::Settings>,
    pub fight: Option<FightView>,
    pub model: Option<ModelCall>,
    /// each side's moves as the model saw them against the other: [home, away] (empty without fighters)
    pub sides: Vec<SideDetail>,
}

/// The duel as it was fought.
#[derive(Clone, Debug, Serialize)]
pub struct FightView {
    pub id: u64,
    pub fought_at: Option<String>,
    pub ko: Option<String>,
    /// the K.O. in seconds, from ko
    pub ko_secs: Option<u32>,
    pub winner_hp_left: Option<u32>,
    pub casts: [u32; 2],
    /// the tape, one row per hit; empty for duels stored without one
    pub hits: Vec<site::Hit>,
    /// each side's HP over the fight, from 100%: [(seconds, % left)] for [home, away]
    pub hp: [Vec<(u32, f64)>; 2],
    /// the damage each side dealt, by move, biggest first: [home, away]
    pub moves: [Vec<MoveDamage>; 2],
    /// the hardest hits of the fight, biggest first
    pub biggest: Vec<site::Hit>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MoveDamage {
    /// "" for unnamed hits (likely damage over time)
    pub name: String,
    pub hits: u32,
    /// % of the other side's HP, all hits together, and the biggest one
    pub total: f64,
    pub max: f64,
    /// from the side's listed moves, when it's one of them
    pub el: Option<String>,
    pub power: Option<f64>,
    /// the type multiplier against the other side (above 1: a type edge)
    pub mult: Option<f64>,
}

/// The model's sum: each factor's home win chance and what it adds on the log-odds scale (positive leans home).
#[derive(Clone, Debug, Serialize)]
pub struct ModelCall {
    pub factors: Vec<Factor>,
    /// the factors added up (log-odds toward home), and as a chance
    pub total: f64,
    pub p_home: f64,
    /// the model re-run on the call's fighters with its settings (None without fighters or records)
    pub score: Option<model::Score>,
    /// the factors came from that re-run, not the call as stored (older calls didn't keep them)
    pub rescored: bool,
    /// what went into the stats factor, each on the same log-odds scale (positive leans home); they add up to it
    pub parts: Vec<Part>,
    /// one sentence on what swung the call most, and what pulled against it
    pub summary: String,
}

/// One input to the stats factor: HP, Attack, Defence, move power, type edges, unlisted moves or cooldowns.
#[derive(Clone, Debug, Serialize)]
pub struct Part {
    pub key: String,
    pub label: String,
    /// log-odds toward home
    pub logit: f64,
    /// in words: "9,383 HP against 7,618"
    pub detail: String,
    /// each side's figure, for the summary: [home, away]
    #[serde(skip)]
    pub values: [String; 2],
}

/// The stats factor taken apart. Its log-odds are steepness × ln(home's time to be knocked out ÷ away's), and with
/// time = HP × Defence ÷ (Attack × move strength) that is a sum of one log ratio per input.
fn stats_parts(s: &model::Settings, h: &model::Fighter, a: &model::Fighter) -> Vec<Part> {
    let k = s.stats_steepness;
    let lr = |x: f64, y: f64| if x > 0.0 && y > 0.0 { k * (x / y).ln() } else { 0.0 };
    let neutral: Vec<String> = vec![];
    let power = |f: &model::Fighter| model::strength(s, &f.skills, &neutral) / model::type_mult(s, "neutral", &neutral);
    let edge =
        |f: &model::Fighter, them: &model::Fighter| model::strength(s, &f.skills, &them.types) / model::strength(s, &f.skills, &neutral);
    let pace = |f: &model::Fighter| f.speed.powf(s.cooldown_exp / 0.5);
    let n = |x: f64| fmt_n(x);
    let mut parts = vec![
        Part {
            key: "hp".into(),
            label: "HP".into(),
            logit: lr(h.hp, a.hp),
            detail: format!("{} HP against {}", n(h.hp), n(a.hp)),
            values: [n(h.hp), n(a.hp)],
        },
        Part {
            key: "atk".into(),
            label: "Attack".into(),
            logit: lr(h.atk, a.atk),
            detail: format!("Attack {} against {}", n(h.atk), n(a.atk)),
            values: [n(h.atk), n(a.atk)],
        },
        Part {
            key: "def".into(),
            label: "Defence".into(),
            logit: lr(h.def, a.def),
            detail: format!("Defence {} against {}", n(h.def), n(a.def)),
            values: [n(h.def), n(a.def)],
        },
        Part {
            key: "power".into(),
            label: "Move power".into(),
            logit: lr(power(h), power(a)),
            detail: format!("move power {:.1} against {:.1}", power(h), power(a)),
            values: [format!("{:.1}", power(h)), format!("{:.1}", power(a))],
        },
        Part {
            key: "type".into(),
            label: "Type edges".into(),
            logit: lr(edge(h, a), edge(a, h)),
            detail: format!("moves hit ×{:.2} for {} against ×{:.2} for {}", edge(h, a), h.pal, edge(a, h), a.pal),
            values: [format!("×{:.2}", edge(h, a)), format!("×{:.2}", edge(a, h))],
        },
        Part {
            key: "unlisted".into(),
            label: "Unlisted moves".into(),
            logit: lr(h.hidden_boost, a.hidden_boost),
            detail: format!("unlisted moves ×{:.2} for {} against ×{:.2} for {}", h.hidden_boost, h.pal, a.hidden_boost, a.pal),
            values: [format!("×{:.2}", h.hidden_boost), format!("×{:.2}", a.hidden_boost)],
        },
        Part {
            key: "cooldown".into(),
            label: "Cooldowns".into(),
            logit: lr(pace(h), pace(a)),
            detail: format!("cooldown passives ×{:.2} for {} against ×{:.2} for {}", pace(h), h.pal, pace(a), a.pal),
            values: [format!("×{:.2}", pace(h)), format!("×{:.2}", pace(a))],
        },
    ];
    parts.retain(|p| p.logit.abs() > 0.005);
    parts.sort_by(|x, y| y.logit.abs().total_cmp(&x.logit.abs()));
    parts
}

fn fmt_n(x: f64) -> String {
    let v = x.round() as i64;
    let s = v.abs().to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if v < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// A sentence or two: the input that pushed hardest toward the pick, the strongest one against it, and how it ended.
/// Each figure leads with the side it helped: "Species records counted most for Grizzbolt: 58–20 against
/// Mammorest's 48–65."
fn summary(c: &db::CallRecord, parts: &[Part], factors: &[Factor]) -> String {
    let pal = |home: bool| if home { c.home_pal.as_str() } else { c.away_pal.as_str() };
    let toward_home = c.pick == "home";
    // a fight still to come is told in the present, and has no result to set against the call
    let settled = c.winner.is_some();
    // (label, log-odds toward home, [home figure, away figure])
    let mut drivers: Vec<(String, f64, [String; 2])> = parts.iter().map(|p| (p.label.clone(), p.logit, p.values.clone())).collect();
    for f in factors.iter().filter(|f| f.key != "stats") {
        let values = match (f.key.as_str(), c.species_rec) {
            // word joiners keep a record like 58–20 on one line
            ("species", Some([[hw, hl], [aw, al]])) => [format!("{hw}\u{2060}–\u{2060}{hl}"), format!("{aw}\u{2060}–\u{2060}{al}")],
            ("h2h", _) => [format!("{} wins", c.pal_h2h[0]), format!("{}", c.pal_h2h[1])],
            ("rank", _) => match ranks(c) {
                Some((h, a)) => [format!("#{}", h.rank), format!("#{}", a.rank)],
                None => continue,
            },
            _ => continue,
        };
        let label = match f.key.as_str() {
            "species" => "Species records",
            "rank" => "Pal rank",
            _ => "Their head-to-head",
        };
        drivers.push((label.into(), f.logit, values));
    }
    if parts.is_empty() {
        if let Some(f) = factors.iter().find(|f| f.key == "stats") {
            drivers.push(("Stats".into(), f.logit, [pct_text(f.p_home, true), pct_text(f.p_home, false)]));
        }
    }
    let lean = |d: &(String, f64, [String; 2])| if toward_home { d.1 } else { -d.1 };
    let say = |d: &(String, f64, [String; 2]), home: bool| {
        let (mine, theirs) = if home { (&d.2[0], &d.2[1]) } else { (&d.2[1], &d.2[0]) };
        format!("{mine} against {}'s {theirs}", pal(!home))
    };
    let top = drivers.iter().filter(|d| lean(d) > 0.0).max_by(|x, y| lean(x).total_cmp(&lean(y)));
    let against = drivers.iter().filter(|d| lean(d) < 0.0).min_by(|x, y| lean(x).total_cmp(&lean(y)));
    let mut out = match top {
        Some(d) => {
            format!("{} {} for {}: {}.", d.0, if settled { "counted most" } else { "counts most" }, pal(toward_home), say(d, toward_home))
        }
        None => format!("Nothing stood out for {}.", pal(toward_home)),
    };
    if let Some(d) = against {
        let pulls = if settled { "pulled" } else { "pulls" };
        out.push_str(&format!(" {} {pulls} toward {}: {}.", d.0, pal(!toward_home), say(d, !toward_home)));
    }
    if let Some(w) = c.winner.as_deref().filter(|w| *w != c.pick) {
        out.push_str(&format!(" {} won anyway.", pal(w == "home")));
    }
    out
}

fn pct_text(p_home: f64, toward_home: bool) -> String {
    format!("{:.0}%", 100.0 * if toward_home { p_home } else { 1.0 - p_home })
}

#[derive(Clone, Debug, Serialize)]
pub struct Factor {
    /// "stats", "species" or "h2h"
    pub key: String,
    pub p_home: f64,
    pub weight: f64,
    /// weight × log-odds of p_home
    pub logit: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SideDetail {
    /// average hitting power over its usable moves against the other side (the model's move strength)
    pub strength: f64,
    pub moves: Vec<MoveLine>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MoveLine {
    pub name: String,
    pub el: String,
    pub power: f64,
    pub lv: u32,
    /// the type multiplier against the other side
    pub mult: f64,
    /// unlocked by Lv 70 (the fights are at Lv 70)
    pub usable: bool,
    /// how hard it really hits in the stored tapes, against the average move (1: average, or too few hits)
    pub hit: f64,
}

/// One settled call's breakdown, or None if there's no such settled call.
pub fn call_detail(conn: &Connection, id: i64) -> R<Option<CallDetail>> {
    let Some(call) = db::settled_call(conn, id)? else { return Ok(None) };
    let (stats_from, estimated, stake_cmd): (Option<String>, bool, Option<String>) =
        conn.query_row("SELECT stats_from, COALESCE(estimated, 0) = 1, stake_cmd FROM predictions WHERE id = ?", [id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
    let settings: Option<model::Settings> = serde_json::from_str(&call.model).ok();
    let s = settings.clone().unwrap_or_default();
    let slug = |pal: &str| -> R<String> {
        let stored: Option<String> =
            conn.query_row("SELECT slug FROM species WHERE pal = ? AND slug != ''", [pal], |r| r.get(0)).optional()?;
        Ok(stored.unwrap_or_else(|| site::slug_for(pal)))
    };
    let fight = match call.duel_id {
        Some(d) => db::get_duel(conn, d)?.map(|duel| fight_view(d, duel, call.fighters.as_ref(), &s)),
        None => None,
    };
    let sides = match &call.fighters {
        Some((h, a)) => vec![side_detail(&s, &h.fighter, &a.fighter), side_detail(&s, &a.fighter, &h.fighter)],
        None => vec![],
    };
    Ok(Some(CallDetail {
        slugs: [slug(&call.home_pal)?, slug(&call.away_pal)?],
        stats_from: split_stats_from(stats_from.as_deref().unwrap_or("")),
        estimated,
        stake_cmd: stake_cmd.filter(|c| !c.is_empty()),
        model: model_call(&call, &s),
        settings,
        fight,
        sides,
        call,
    }))
}

/// The open call on the duel that's up, broken down by the same code as a settled one: no fight, no winner, its
/// factors straight from the prediction. stake: what Augur suggests betting on it.
pub fn live_detail(p: &Prediction, stake: Option<&Stake>) -> CallDetail {
    let s = &p.settings;
    let n = p.to_call();
    let call = db::CallRecord {
        id: 0,
        seen_at: p.seen_at as i64,
        home_owner: n.home_owner,
        away_owner: n.away_owner,
        home_pal: n.home_pal,
        away_pal: n.away_pal,
        pick: n.pick,
        // unrounded, so the sum lands on the chance the dial shows
        prob: p.prob,
        p_stats: Some(p.score.p_stats),
        p_species: Some(p.score.p_species),
        p_h2h: Some(p.score.p_h2h),
        species_rec: Some(p.species_rec),
        pal_h2h: n.pal_h2h,
        duel_id: None,
        winner: None,
        fought_at: None,
        ko: None,
        winner_hp_left: None,
        stake: stake.map(|st| st.amount),
        fighters: Some((n.home, n.away)),
        late: None,
        late_only: !p.picks_open,
        version: n.version,
        model: serde_json::to_string(s).unwrap_or_default(),
        // a call that's up hasn't been overwritten
        damaged: false,
    };
    let sides = call.fighters.as_ref().map(|(h, a)| vec![side_detail(s, &h.fighter, &a.fighter), side_detail(s, &a.fighter, &h.fighter)]);
    let slug = |side: &predict::Side| if side.slug.is_empty() { site::slug_for(&side.pal) } else { side.slug.clone() };
    CallDetail {
        slugs: [slug(&p.home), slug(&p.away)],
        stats_from: [p.home.stats_from.clone(), p.away.stats_from.clone()],
        estimated: p.estimated(),
        stake_cmd: stake.map(|st| st.command.clone()).filter(|c| !c.is_empty()),
        settings: Some(s.clone()),
        fight: None,
        model: model_call(&call, s),
        sides: sides.unwrap_or_default(),
        call,
    }
}

/// "home: from their box; away: shown" as [home, away].
fn split_stats_from(text: &str) -> [String; 2] {
    let of = |side: &str| {
        text.split(';').find_map(|part| part.trim().strip_prefix(side).and_then(|r| r.strip_prefix(':'))).unwrap_or("").trim().to_string()
    };
    [of("home"), of("away")]
}

/// "0:27" (or "1:05") in seconds.
fn ko_secs(ko: &str) -> Option<u32> {
    let (m, s) = ko.split_once(':')?;
    Some(m.trim().parse::<u32>().ok()? * 60 + s.trim().parse::<u32>().ok()?)
}

/// How many of the fight's hardest hits the breakdown lists.
const BIGGEST_HITS: usize = 5;

fn fight_view(id: u64, d: site::Duel, fighters: Option<&(db::CallFighter, db::CallFighter)>, s: &model::Settings) -> FightView {
    let ko = d.ko.as_deref().and_then(ko_secs);
    // each side's HP falls by the other side's hits
    let mut hp = [vec![(0, 100.0)], vec![(0, 100.0)]];
    let mut left = [100.0f64, 100.0];
    for h in &d.hits {
        let target = usize::from(h.side == "home");
        left[target] = (left[target] - h.pct).max(0.0);
        hp[target].push((h.t, left[target]));
    }
    // the loser goes down at the K.O. even when the tape's rounding leaves it a sliver
    if let (Some(w), Some(t), false) = (d.winner.as_deref(), ko, d.hits.is_empty()) {
        let loser = usize::from(w == "home");
        if left[loser] > 0.0 {
            hp[loser].push((t.max(hp[loser].last().map_or(0, |p| p.0)), 0.0));
        }
    }
    let moves = [0, 1].map(|i| {
        let side = if i == 0 { "home" } else { "away" };
        let listed = fighters.map(|(h, a)| if i == 0 { (&h.fighter, &a.fighter) } else { (&a.fighter, &h.fighter) });
        let mut out: Vec<MoveDamage> = vec![];
        for h in d.hits.iter().filter(|h| h.side == side) {
            match out.iter_mut().find(|m| m.name == h.mv) {
                Some(m) => {
                    m.hits += 1;
                    m.total += h.pct;
                    m.max = m.max.max(h.pct);
                }
                None => {
                    let known = listed.and_then(|(me, them)| me.skills.iter().find(|m| m.name == h.mv).map(|m| (m, them)));
                    out.push(MoveDamage {
                        name: h.mv.clone(),
                        hits: 1,
                        total: h.pct,
                        max: h.pct,
                        el: known.map(|(m, _)| m.el.clone()),
                        power: known.map(|(m, _)| m.power),
                        mult: known.map(|(m, them)| model::type_mult(s, &m.el, &them.types)),
                    })
                }
            }
        }
        out.sort_by(|a, b| b.total.total_cmp(&a.total));
        out
    });
    let mut biggest: Vec<site::Hit> = d.hits.iter().filter(|h| !h.mv.is_empty()).cloned().collect();
    biggest.sort_by(|a, b| b.pct.total_cmp(&a.pct).then(a.t.cmp(&b.t)));
    biggest.truncate(BIGGEST_HITS);
    FightView {
        id,
        fought_at: d.fought_at,
        ko: d.ko,
        ko_secs: ko,
        winner_hp_left: d.winner_hp_left,
        casts: [d.home_casts, d.away_casts],
        hits: d.hits,
        hp,
        moves,
        biggest,
    }
}

/// Both sides' pal ranks, when the call read them.
fn ranks(c: &db::CallRecord) -> Option<(&model::PalRank, &model::PalRank)> {
    let (h, a) = c.fighters.as_ref()?;
    h.fighter.pal_rank.as_ref().zip(a.fighter.pal_rank.as_ref())
}

fn model_call(c: &db::CallRecord, s: &model::Settings) -> Option<ModelCall> {
    let score = match (&c.fighters, c.species_rec) {
        (Some((h, a)), Some(rec)) => Some(model::score(&s.for_calls(), &h.fighter, &a.fighter, rec, c.pal_h2h)),
        _ => None,
    };
    // calls imported from a calls file don't keep the factors: the re-run stands in
    let ok = |p: Option<f64>| p.filter(|p| *p > 0.0 && *p < 1.0);
    let stored = ok(c.p_stats).zip(ok(c.p_species)).zip(ok(c.p_h2h)).map(|((a, b), c)| [a, b, c]);
    let (ps, rescored) = match (stored, &score) {
        (Some(p), _) => (p, false),
        (None, Some(sc)) => ([sc.p_stats, sc.p_species, sc.p_h2h], true),
        (None, None) => return None,
    };
    let mut factors: Vec<Factor> = [("stats", 1.0), ("species", s.species_on_calls()), ("h2h", s.pal_h2h_weight)]
        .iter()
        .zip(ps)
        .map(|((key, weight), p)| Factor { key: key.to_string(), p_home: p, weight: *weight, logit: weight * model::logit(p) })
        .collect();
    // the pal ranks count only once the learner has given them a weight, and only on calls that read them
    if let Some(sc) = score.as_ref().filter(|_| s.pal_rank_weight > 0.0 && ranks(c).is_some()) {
        let w = s.pal_rank_weight;
        factors.push(Factor { key: "rank".into(), p_home: sc.p_rank, weight: w, logit: w * model::logit(sc.p_rank) });
    }
    let total = factors.iter().map(|f| f.logit).sum();
    let parts = c.fighters.as_ref().map(|(h, a)| stats_parts(s, &h.fighter, &a.fighter)).unwrap_or_default();
    let summary = summary(c, &parts, &factors);
    Some(ModelCall { factors, total, p_home: model::sigmoid(total), score, rescored, parts, summary })
}

fn side_detail(s: &model::Settings, me: &model::Fighter, them: &model::Fighter) -> SideDetail {
    SideDetail {
        strength: model::strength(s, &me.skills, &them.types),
        moves: me
            .skills
            .iter()
            .map(|m| MoveLine {
                name: m.name.clone(),
                el: m.el.clone(),
                power: m.power,
                lv: m.lv,
                mult: model::type_mult(s, &m.el, &them.types),
                usable: m.lv <= 70,
                hit: m.hit,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn off_air_lists_the_latest_calls_newest_first() {
        let conn = augur_engine::seed::store();
        let o = super::off_air(&conn, &Default::default()).unwrap();
        assert_eq!(o.recent.len(), super::RECENT_CALLS);
        let newest = augur_engine::db::settled(&conn).unwrap().pop().unwrap();
        assert_eq!(o.recent[0].3, newest.prob);
        assert!(o.recent.iter().all(|r| r.3 > 0.0 && r.3 <= 1.0));
    }

    #[test]
    fn civil_dates() {
        assert_eq!(super::civil_date(0), "1970-01-01");
        assert_eq!(super::civil_date(1_794_355_200), "2026-11-11");
    }

    #[test]
    fn a_call_breaks_down_into_its_fight_and_the_models_sum() {
        let conn = augur_engine::seed::store();
        let calls = augur_engine::db::settled(&conn).unwrap();
        let c = calls.last().unwrap();
        assert!(augur_engine::seed::tape(&conn, c.duel_id.unwrap()));
        let d = super::call_detail(&conn, c.id).unwrap().expect("the newest call");
        assert_eq!(d.call.id, c.id);
        assert_eq!(d.slugs[0], augur_engine::site::slug_for(&c.home_pal));
        assert!(d.settings.is_some(), "the call's own settings");
        let f = d.fight.as_ref().expect("its duel");
        let winner = usize::from(c.winner.as_deref() == Some("away"));
        let loser = 1 - winner;
        assert!(!f.hits.is_empty());
        assert_eq!(f.hp[loser].last().unwrap(), &(f.ko_secs.unwrap(), 0.0), "the loser goes down at the K.O.");
        assert_eq!(f.hp[winner].last().unwrap().1, f.winner_hp_left.unwrap() as f64, "the winner ends with the HP the duel says");
        assert!(f.hp.iter().all(|h| h[0] == (0, 100.0)), "both start full");
        for side in &f.moves {
            assert!(side.windows(2).all(|w| w[0].total >= w[1].total), "biggest damage first");
        }
        let dealt: f64 = f.moves[winner].iter().map(|m| m.total).sum();
        assert_eq!(dealt, 100.0, "the winner took all of the other side's HP");
        assert!(f.moves[winner].iter().any(|m| m.name.is_empty() && m.el.is_none()), "unnamed hits are kept apart");
        assert!(f.moves[winner].iter().filter(|m| !m.name.is_empty()).all(|m| m.power.is_some()), "listed moves have their power");
        assert!(f.biggest.windows(2).all(|w| w[0].pct >= w[1].pct) && f.biggest.len() <= super::BIGGEST_HITS);
        // the factors add up to the call's chance (the seed's calls are re-run: their factors weren't kept)
        let m = d.model.as_ref().unwrap();
        assert!(m.rescored && m.score.is_some());
        // the stats factor taken apart: its parts add up to its log-odds (re-run from the same fighters)
        let stats = m.factors.iter().find(|f| f.key == "stats").unwrap();
        let sum: f64 = m.parts.iter().map(|p| p.logit).sum();
        assert!((sum - stats.logit).abs() < 0.02, "parts {sum} vs stats {}", stats.logit);
        assert!(m.parts.windows(2).all(|w| w[0].logit.abs() >= w[1].logit.abs()), "biggest first");
        let pick = if c.pick == "home" { &c.home_pal } else { &c.away_pal };
        assert!(m.summary.contains(&format!("counted most for {pick}")), "{}", m.summary);
        assert_eq!(m.factors.len(), 3);
        let sum: f64 = m.factors.iter().map(|f| f.logit).sum();
        assert!((sum - m.total).abs() < 1e-12);
        let p_pick = if c.pick == "home" { m.p_home } else { 1.0 - m.p_home };
        assert!((p_pick - c.prob).abs() < 0.01, "{p_pick} vs the call's {}", c.prob);
        assert_eq!(d.sides.len(), 2);
        assert!(d.sides[0].moves.iter().any(|m| m.usable));
    }

    #[test]
    fn the_pal_rank_joins_the_sum_once_it_has_a_weight() {
        let conn = augur_engine::seed::store();
        let mut c = augur_engine::db::settled_all(&conn).unwrap().pop().unwrap();
        let (h, a) = c.fighters.as_mut().unwrap();
        h.fighter.pal_rank = Some(augur_engine::model::PalRank { tier: "B".into(), rank: 88, of: 357, power: 66 });
        a.fighter.pal_rank = Some(augur_engine::model::PalRank { tier: "C".into(), rank: 171, of: 357, power: 51 });
        let keys =
            |s: &augur_engine::model::Settings| super::model_call(&c, s).unwrap().factors.iter().map(|f| f.key.clone()).collect::<Vec<_>>();
        let off = augur_engine::model::Settings::default();
        assert_eq!(keys(&off), ["stats", "species", "h2h"], "at 0 it isn't part of the call");
        let on = off.with("PAL_RANK_WEIGHT", 0.3);
        assert_eq!(keys(&on), ["stats", "species", "h2h", "rank"]);
        let m = super::model_call(&c, &on).unwrap();
        let f = m.factors.last().unwrap();
        assert!(f.p_home > 0.5 && (f.logit - 0.3 * 1.5).abs() < 1e-9, "15 points toward home at 0.3: {}", f.logit);
    }

    #[test]
    fn a_call_without_a_tape_or_fighters_still_breaks_down() {
        let conn = augur_engine::seed::store();
        let c = augur_engine::db::settled(&conn).unwrap().remove(0);
        let d = super::call_detail(&conn, c.id).unwrap().unwrap();
        let f = d.fight.unwrap();
        assert!(f.hits.is_empty() && f.hp.iter().all(|h| h.len() == 1) && f.biggest.is_empty(), "no tape: nothing made up");
        // an old call: no fighters kept, factors stored
        conn.execute("DELETE FROM fighters WHERE prediction_id = ?", [c.id]).unwrap();
        conn.execute("UPDATE predictions SET p_stats = 0.3, p_species = 0.6, p_h2h = 0.5, stats_from = 'home: from their box; away: shown' WHERE id = ?", [c.id]).unwrap();
        let d = super::call_detail(&conn, c.id).unwrap().unwrap();
        assert!(d.call.fighters.is_none() && d.sides.is_empty());
        let m = d.model.unwrap();
        assert!(!m.rescored && m.score.is_none());
        assert_eq!(m.factors[0].p_home, 0.3);
        assert_eq!(d.stats_from, ["from their box".to_string(), "shown".to_string()]);
        assert!(super::call_detail(&conn, 999_999).unwrap().is_none());
    }

    #[test]
    fn the_call_up_now_breaks_down_like_a_settled_one() {
        let state = augur_engine::seed::state("state-leaflit-wumpo.json");
        let opts = augur_engine::predict::Options {
            lookups: false,
            db_path: std::path::PathBuf::from("/nonexistent"),
            settings: Default::default(),
        };
        let p = augur_engine::predict::predict(&state, &opts).unwrap();
        let stake = augur_engine::stake::suggest(&p.pick, p.prob, &Default::default(), &Default::default());
        let d = super::live_detail(&p, Some(&stake));
        assert!(d.fight.is_none() && d.call.winner.is_none(), "no fight yet");
        assert_eq!((d.call.home_pal.as_str(), d.call.pick.as_str()), (p.home.pal.as_str(), p.pick.as_str()));
        assert_eq!(d.stats_from[0], p.home.stats_from);
        assert!(d.estimated, "the home side's stats are hidden until the bell");
        assert_eq!(d.stake_cmd.as_deref(), Some(stake.command.as_str()).filter(|c| !c.is_empty()));
        let m = d.model.as_ref().expect("the model's sum");
        assert!(!m.rescored, "the factors come straight from the prediction");
        assert_eq!(m.factors.len(), 3);
        assert_eq!(m.factors[0].p_home, p.score.p_stats);
        assert!((m.p_home - p.score.p_home).abs() < 1e-9, "{} vs {}", m.p_home, p.score.p_home);
        let stats = m.factors.iter().find(|f| f.key == "stats").unwrap();
        assert!(!m.parts.is_empty());
        let sum: f64 = m.parts.iter().map(|x| x.logit).sum();
        assert!((sum - stats.logit).abs() < 0.02, "parts {sum} vs stats {}", stats.logit);
        // told in the present, with no result to set against it
        assert!(m.summary.contains(&format!("counts most for {}", p.favourite().pal)), "{}", m.summary);
        assert!(!m.summary.contains("won anyway") && !m.summary.contains("counted"), "{}", m.summary);
        assert_eq!(d.sides.len(), 2);
        assert!(d.sides.iter().all(|s| s.moves.iter().any(|m| m.usable)));
    }

    #[test]
    fn ko_times() {
        assert_eq!(super::ko_secs("0:27"), Some(27));
        assert_eq!(super::ko_secs("1:05"), Some(65));
        assert_eq!(super::ko_secs("soon"), None);
    }
}
