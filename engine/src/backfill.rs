//! The gentle background backfill: every species' page, then past duels newest first (the ones with fight tapes),
//! then the older results-only ones. One page at a time; every HTTP 429 raises the delay for the rest of the run, and
//! 429s in a row wait longer and longer, then end the run (the next one tries again); it pauses while a duel is up on
//! stream, and resumes where it left off. Each page is checked against the store just
//! before it's fetched, so an import part-way through (a backup from another computer) stops the work it covers.
//! At launch a short catch-up fetches the duels fought while Augur was closed first, slowly but without pausing.

use crate::db;
use crate::error::Error;
use crate::site;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Tape-era duels start around #2985; older duels have a result but no fight tape.
pub const TAPES_FROM: u64 = 2980;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Progress {
    /// "catchup" (the duels missed while closed), "species", "duels", "bets" or "done"
    pub phase: String,
    pub done: usize,
    pub total: usize,
    pub delay_ms: u64,
    pub paused: bool,
    pub last_error: Option<String>,
    /// waiting out the site's rate limit until then (ms)
    pub resume_at: Option<u64>,
    /// the site kept rate-limiting, so the run stopped early
    pub gave_up: bool,
}

/// 429s in a row on one page before a run gives up until the next one.
const MAX_STRIKES: u32 = 6;

/// Hooks the caller provides: whether to pause (a duel is up on stream), whether to stop, and progress reports.
pub struct Control<'a> {
    pub stop: &'a AtomicBool,
    pub busy: &'a dyn Fn() -> bool,
    pub progress: &'a dyn Fn(&Progress),
}

/// Fetches one page at a time, with a delay that grows on every 429 and never shrinks during a run.
pub struct Pacer {
    pub delay: Duration,
    /// a signed-in session's Cookie header, for pages that need it (your picks)
    pub cookie: Option<String>,
    /// the site kept answering 429: stop until the next run
    pub gave_up: bool,
    /// the last page asked for wasn't there (404)
    pub missing: bool,
}

impl Pacer {
    pub fn new(delay: Duration) -> Self {
        Pacer { delay, cookie: None, gave_up: false, missing: false }
    }

    /// How long to wait after the `strikes`th 429 in a row: four times the pace, doubling with each one, at most 30
    /// minutes (at the slowest pace: 2, 4, 8, 16, 30 minutes).
    pub fn cool_off(&self, strikes: u32) -> Duration {
        (self.delay * 4 * 2u32.pow(strikes.saturating_sub(1).min(10))).min(Duration::from_secs(30 * 60))
    }

    /// parse(page) for the page at url; None on 404 (`missing`), when stopped, or when the site keeps rate-limiting
    /// (`gave_up`).
    pub fn get<T>(&mut self, url: &str, parse: impl Fn(&str) -> T, ctl: &Control, p: &mut Progress) -> Option<T> {
        self.missing = false;
        let mut strikes = 0;
        loop {
            if ctl.stop.load(Ordering::Relaxed) {
                return None;
            }
            match site::fetch_once(url, self.cookie.as_deref()) {
                Ok(page) => {
                    if p.resume_at.take().is_some() {
                        p.last_error = None;
                        (ctl.progress)(p);
                    }
                    std::thread::sleep(self.delay);
                    return Some(parse(&page));
                }
                Err(Error::Http(429)) => {
                    strikes += 1;
                    self.delay = (self.delay.mul_f64(1.5)).min(Duration::from_secs(30));
                    p.delay_ms = self.delay.as_millis() as u64;
                    if strikes >= MAX_STRIKES {
                        self.gave_up = true;
                        p.gave_up = true;
                        p.resume_at = None;
                        p.last_error = Some("The site kept rate-limiting, so this run stopped; the next one carries on".into());
                        (ctl.progress)(p);
                        return None;
                    }
                    let wait = self.cool_off(strikes);
                    p.resume_at = Some(now_ms() + wait.as_millis() as u64);
                    p.last_error = Some(format!("The site is rate-limiting (429 {strikes} in a row)"));
                    (ctl.progress)(p);
                    sleep_unless_stopped(wait, ctl.stop);
                }
                Err(Error::Http(404)) => {
                    self.missing = true;
                    return None;
                }
                Err(e) => {
                    p.last_error = Some(format!("{url}: {e}"));
                    (ctl.progress)(p);
                    sleep_unless_stopped(Duration::from_secs(10), ctl.stop);
                }
            }
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn sleep_unless_stopped(d: Duration, stop: &AtomicBool) {
    let end = Instant::now() + d;
    while Instant::now() < end && !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Waits while a duel is up on stream, checking every 15 seconds. Returns false if stopped.
pub fn wait_while_busy(ctl: &Control, p: &mut Progress) -> bool {
    while (ctl.busy)() {
        if !p.paused {
            p.paused = true;
            (ctl.progress)(p);
        }
        sleep_unless_stopped(Duration::from_secs(15), ctl.stop);
        if ctl.stop.load(Ordering::Relaxed) {
            return false;
        }
    }
    if p.paused {
        p.paused = false;
        (ctl.progress)(p);
    }
    !ctl.stop.load(Ordering::Relaxed)
}

/// Every species' types, base stats and moves. Skips species already stored unless `refresh`.
pub fn species(conn: &Connection, pacer: &mut Pacer, ctl: &Control, refresh: bool) -> crate::Result<usize> {
    let slugs = site::species_slugs()?;
    let have: std::collections::HashSet<String> = if refresh {
        Default::default()
    } else {
        conn.prepare("SELECT slug FROM species")
            .and_then(|mut st| Ok(st.query_map([], |r| r.get::<_, Option<String>>(0))?.flatten().flatten().collect()))
            .unwrap_or_default()
    };
    let todo: Vec<&String> = slugs.iter().filter(|s| !have.contains(*s)).collect();
    let mut p = Progress { phase: "species".into(), total: todo.len(), delay_ms: pacer.delay.as_millis() as u64, ..Default::default() };
    (ctl.progress)(&p);
    for slug in todo {
        if !wait_while_busy(ctl, &mut p) {
            break;
        }
        if !refresh && db::has_species(conn, slug)? {
            p.done += 1;
            continue;
        }
        if let Some(sp) = pacer.get(&format!("{}/pal/{slug}", site::BASE), |page| site::parse_species(page, slug), ctl, &mut p) {
            db::save_species(conn, &sp)?;
        }
        if pacer.gave_up {
            break;
        }
        p.done += 1;
        if p.done.is_multiple_of(10) || p.done == p.total {
            (ctl.progress)(&p);
        }
    }
    Ok(p.done)
}

/// Past duels up to `latest` (the newest in the live state), tape-era ones first, newest first. Skips stored ones.
pub fn duels(conn: &Connection, latest: u64, pacer: &mut Pacer, ctl: &Control) -> crate::Result<usize> {
    let have = db::duel_ids(conn)?;
    let missing = missing_duels(conn)?;
    let order: Vec<u64> = (TAPES_FROM..=latest)
        .rev()
        .chain((1..TAPES_FROM.min(latest + 1)).rev())
        .filter(|i| !have.contains(i) && !missing.contains(i))
        .collect();
    fetch_duels(conn, order, "duels", true, pacer, ctl)
}

/// The most duels a catch-up fetches: about a day of streaming. More than that waits for the full run.
pub const CATCH_UP_MAX: usize = 100;

/// The duels since the newest one stored, newest first, at most `CATCH_UP_MAX`. None on a store with no duels yet
/// (that's the full run's job).
pub fn catch_up_ids(have: &HashSet<u64>, missing: &BTreeSet<u64>, latest: u64) -> Vec<u64> {
    let Some(newest) = have.iter().max() else { return vec![] };
    (newest + 1..=latest).rev().filter(|i| !missing.contains(i)).take(CATCH_UP_MAX).collect()
}

/// At launch: the duels fought while Augur was closed, before anything else. Unlike the full run it carries on while
/// a duel is up, so the caller gives it a slow pace.
pub fn catch_up(conn: &Connection, latest: u64, pacer: &mut Pacer, ctl: &Control) -> crate::Result<usize> {
    let order = catch_up_ids(&db::duel_ids(conn)?, &missing_duels(conn)?, latest);
    fetch_duels(conn, order, "catchup", false, pacer, ctl)
}

/// The whole backfill: species first (the Pals tab and the model need them; every species is read again once a
/// week, for changes on the site), then duels the store doesn't have.
pub fn run(conn: &Connection, ctl: &Control) -> crate::Result<()> {
    let mut pacer = Pacer::new(Duration::from_millis(500));
    let refreshed: u64 = db::meta_get(conn, "species_refreshed_at")?.and_then(|v| v.parse().ok()).unwrap_or(0);
    let refresh = refreshed > 0 && now_ms().saturating_sub(refreshed) > 7 * 86_400_000;
    species(conn, &mut pacer, ctl, refresh)?;
    if ctl.stop.load(Ordering::Relaxed) || pacer.gave_up {
        return Ok(());
    }
    if refresh || refreshed == 0 {
        db::meta_set(conn, "species_refreshed_at", &now_ms().to_string())?;
    }
    let latest = site::State::fetch()?.recent.iter().map(|r| r.id).max().unwrap_or(0);
    if latest > 0 {
        duels(conn, latest, &mut pacer, ctl)?;
    }
    if !ctl.stop.load(Ordering::Relaxed) && !pacer.gave_up {
        let _ = db::meta_set(conn, "backfill_done_at", &latest.to_string());
        (ctl.progress)(&Progress { phase: "done".into(), ..Default::default() });
    }
    Ok(())
}

// --- helpers ---

/// Duel numbers the site has said aren't there: not asked for again.
fn missing_duels(conn: &Connection) -> crate::Result<BTreeSet<u64>> {
    Ok(db::meta_get(conn, "missing_duels")?.and_then(|m| serde_json::from_str(&m).ok()).unwrap_or_default())
}

/// Fetches and stores each duel in `order`, reporting progress under `phase`; `pause` waits while a duel is up.
fn fetch_duels(conn: &Connection, order: Vec<u64>, phase: &str, pause: bool, pacer: &mut Pacer, ctl: &Control) -> crate::Result<usize> {
    let mut missing = missing_duels(conn)?;
    let mut p = Progress { phase: phase.into(), total: order.len(), delay_ms: pacer.delay.as_millis() as u64, ..Default::default() };
    (ctl.progress)(&p);
    for id in order {
        if (pause && !wait_while_busy(ctl, &mut p)) || ctl.stop.load(Ordering::Relaxed) {
            break;
        }
        if db::has_duel(conn, id)? {
            p.done += 1;
            continue;
        }
        if let Some(d) = pacer.get(&format!("{}/duel/{id}?show=1", site::BASE), site::parse_duel, ctl, &mut p) {
            if d.home_pal.is_some() || !d.hits.is_empty() {
                db::save_duel(conn, id, &d)?;
            }
        } else if pacer.missing {
            missing.insert(id);
            db::meta_set(conn, "missing_duels", &serde_json::to_string(&missing)?)?;
        }
        if pacer.gave_up {
            break;
        }
        p.done += 1;
        if p.done.is_multiple_of(25) || p.done == p.total {
            (ctl.progress)(&p);
        }
    }
    Ok(p.done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limits_wait_longer_each_time() {
        let slow = Pacer::new(Duration::from_secs(30));
        let mins: Vec<u64> = (1..=6).map(|n| slow.cool_off(n).as_secs() / 60).collect();
        assert_eq!(mins, [2, 4, 8, 16, 30, 30], "at the slowest pace: 2, 4, 8, 16, then 30 minutes");
        assert_eq!(Pacer::new(Duration::from_millis(750)).cool_off(1).as_secs(), 3);
    }

    #[test]
    fn a_catch_up_fetches_only_whats_new_newest_first() {
        let have: HashSet<u64> = [4700, 4702, 4703].into();
        let missing: BTreeSet<u64> = [4706].into();
        assert_eq!(catch_up_ids(&have, &missing, 4708), [4708, 4707, 4705, 4704], "older gaps wait for the full run");
        assert!(catch_up_ids(&have, &missing, 4703).is_empty(), "nothing new");
        assert!(catch_up_ids(&HashSet::new(), &missing, 4708).is_empty(), "a new store waits for the full run");
        assert_eq!(catch_up_ids(&[10].into(), &missing, 5000).len(), CATCH_UP_MAX);
    }
}
