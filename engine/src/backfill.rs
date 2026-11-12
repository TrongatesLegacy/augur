//! The gentle background backfill: every species' page, then past duels newest first (the ones with fight tapes),
//! then the older results-only ones. One page at a time; every HTTP 429 raises the delay for the rest of the run;
//! it pauses while a duel is up on stream, and resumes where it left off.

use crate::db;
use crate::error::Error;
use crate::site;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Tape-era duels start around #2985; older duels have a result but no fight tape.
pub const TAPES_FROM: u64 = 2980;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Progress {
    /// "species", "duels", "bets" or "done"
    pub phase: String,
    pub done: usize,
    pub total: usize,
    pub delay_ms: u64,
    pub paused: bool,
    pub last_error: Option<String>,
}

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
}

impl Pacer {
    pub fn new(delay: Duration) -> Self {
        Pacer { delay, cookie: None }
    }

    /// parse(page) for the page at url; None on 404 or when stopped.
    pub fn get<T>(&mut self, url: &str, parse: impl Fn(&str) -> T, ctl: &Control, p: &mut Progress) -> Option<T> {
        loop {
            if ctl.stop.load(Ordering::Relaxed) {
                return None;
            }
            match site::fetch_once(url, self.cookie.as_deref()) {
                Ok(page) => {
                    std::thread::sleep(self.delay);
                    return Some(parse(&page));
                }
                Err(Error::Http(429)) => {
                    self.delay = (self.delay.mul_f64(1.5)).min(Duration::from_secs(30));
                    p.delay_ms = self.delay.as_millis() as u64;
                    p.last_error = Some(format!("429 at {url}"));
                    (ctl.progress)(p);
                    sleep_unless_stopped(self.delay * 4, ctl.stop);
                }
                Err(Error::Http(404)) => return None,
                Err(e) => {
                    p.last_error = Some(format!("{url}: {e}"));
                    (ctl.progress)(p);
                    sleep_unless_stopped(Duration::from_secs(10), ctl.stop);
                }
            }
        }
    }
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
        if let Some(sp) = pacer.get(&format!("{}/pal/{slug}", site::BASE), |page| site::parse_species(page, slug), ctl, &mut p) {
            db::save_species(conn, &sp)?;
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
    let order: Vec<u64> = (TAPES_FROM..=latest).rev().chain((1..TAPES_FROM.min(latest + 1)).rev()).filter(|i| !have.contains(i)).collect();
    let mut p = Progress { phase: "duels".into(), total: order.len(), delay_ms: pacer.delay.as_millis() as u64, ..Default::default() };
    (ctl.progress)(&p);
    for id in order {
        if !wait_while_busy(ctl, &mut p) {
            break;
        }
        if let Some(d) = pacer.get(&format!("{}/duel/{id}?show=1", site::BASE), site::parse_duel, ctl, &mut p) {
            if d.home_pal.is_some() || !d.hits.is_empty() {
                db::save_duel(conn, id, &d)?;
            }
        }
        p.done += 1;
        if p.done.is_multiple_of(25) || p.done == p.total {
            (ctl.progress)(&p);
        }
    }
    Ok(p.done)
}

/// The whole backfill: species first (the Pals tab and the model need them), then duels.
pub fn run(conn: &Connection, ctl: &Control) -> crate::Result<()> {
    let mut pacer = Pacer::new(Duration::from_millis(500));
    species(conn, &mut pacer, ctl, false)?;
    if ctl.stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    let latest = site::State::fetch()?.recent.iter().map(|r| r.id).max().unwrap_or(0);
    if latest > 0 {
        duels(conn, latest, &mut pacer, ctl)?;
    }
    if !ctl.stop.load(Ordering::Relaxed) {
        let _ = db::meta_set(conn, "backfill_done_at", &latest.to_string());
        (ctl.progress)(&Progress { phase: "done".into(), ..Default::default() });
    }
    Ok(())
}
