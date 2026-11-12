//! The desktop app: the tray (pick in the menu bar on macOS, a confidence ring on Windows), the popover, the main
//! window, the live sync loop, the background backfill, the bets import and the commands the pages call (Settings ›
//! Sync & data's are in `sync.rs`).

use crate::secure;
use crate::settings::{self, AppSettings};
use crate::views::{self, civil_date, BetsView, HistoryView, LastResult, LiveView, ModelView, PlayerView};
use augur_engine::backfill::{self, Progress};
use augur_engine::bets;
use augur_engine::history::{self, Backtest};
use augur_engine::predict;
use augur_engine::site::{self, State};
use augur_engine::stake::{self, Stake};
use augur_engine::sync::{DeviceFile, SyncState};
use augur_engine::{db, model, rusqlite};
use base64::Engine;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_positioner::{Position, WindowExt};

pub struct Shared {
    pub dir: PathBuf,
    pub settings: Mutex<AppSettings>,
    pub live: Mutex<LiveView>,
    pub backfill: Mutex<Progress>,
    /// the picks list import
    pub bets: Mutex<Progress>,
    /// read the picks list now (a result came in, or the developer asked)
    pub bets_now: AtomicBool,
    pub backtest: Mutex<Option<(model::Settings, Backtest)>>,
    pub rosters: Mutex<HashMap<String, (Instant, Vec<site::BoxPal>)>>,
    pub busy: AtomicBool,
    pub stop: AtomicBool,
    /// Settings › Sync: this computer's state, the other computers last seen, and "sync now"
    pub sync: Mutex<SyncState>,
    pub others: Mutex<Vec<DeviceFile>>,
    pub sync_now: AtomicBool,
    /// a fresh install: the backfill waits (up to 10 minutes) for an import or "start fresh"
    pub setup_wait: AtomicBool,
    /// Settings › Updates
    pub update: Mutex<crate::updates::UpdateView>,
    pub pending: Mutex<crate::updates::Pending>,
    pub tray: Mutex<Option<TrayIcon>>,
}

impl Shared {
    fn db(&self) -> Result<rusqlite::Connection, String> {
        db::open(&settings::db_path(&self.dir)).map_err(|e| e.to_string())
    }
}

/// A random number in 0..1, for polling at uneven times (no need for a random-number crate).
fn roll() -> f64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    (h.finish() >> 11) as f64 / (1u64 << 53) as f64
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The stake for a call: the staking settings, the payout the developer's wins have had and their last balance.
fn stake_for(conn: &rusqlite::Connection, settings: &AppSettings, pick: &str, prob: f64) -> Stake {
    stake::suggest(pick, prob, &settings.staking, &bets::odds(conn).unwrap_or_default())
}

/// A 32px tray icon: a gold ring filled to the chance around a dot in the favourite's colour (cyan home, pink away).
/// Windows can't show text in the tray, so the icon carries the call.
fn ring_icon(chance: Option<f64>, home: bool) -> Image<'static> {
    let n = 32u32;
    let mut px = vec![0u8; (n * n * 4) as usize];
    let c = (n as f64 - 1.0) / 2.0;
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = (x as f64 - c, y as f64 - c);
            let r = (dx * dx + dy * dy).sqrt();
            // angle clockwise from the top, 0..1
            let a = ((dx.atan2(-dy) / std::f64::consts::TAU) + 1.0) % 1.0;
            let rgba: Option<[u8; 4]> = if (11.0..=15.0).contains(&r) {
                match chance {
                    Some(p) if a <= p => Some([255, 207, 107, 255]),
                    _ => Some([255, 255, 255, 60]),
                }
            } else if r <= 6.5 {
                Some(match chance {
                    None => [255, 207, 107, 255],
                    Some(_) if home => [79, 227, 255, 255],
                    Some(_) => [255, 92, 168, 255],
                })
            } else {
                None
            };
            if let Some(v) = rgba {
                let i = ((y * n + x) * 4) as usize;
                px[i..i + 4].copy_from_slice(&v);
            }
        }
    }
    Image::new_owned(px, n, n)
}

/// Re-works the stake for the call that's up (the staking settings or the payout changed).
pub fn restake(app: &AppHandle, shared: &Shared) {
    let Ok(conn) = shared.db() else { return };
    let settings = shared.settings.lock().unwrap().clone();
    let mut live = shared.live.lock().unwrap();
    if let Some(p) = live.prediction.clone() {
        let st = stake_for(&conn, &settings, &p.pick, p.prob);
        if let Ok(Some(id)) = db::open_call(&conn, &p.home.owner, &p.away.owner) {
            let _ = db::set_call_stake(&conn, id, st.amount, &st.command);
        }
        live.stake = Some(st);
    }
    let _ = app.emit("live", &*live);
    drop(live);
    update_tray(app, shared);
}

fn update_tray(app: &AppHandle, shared: &Shared) {
    let Some(tray) = shared.tray.lock().unwrap().clone() else { return };
    let live = shared.live.lock().unwrap().clone();
    let show = shared.settings.lock().unwrap().tray_pick;
    let bet = live.stake.as_ref().filter(|s| !s.command.is_empty()).map(|s| format!(" · {}", s.command)).unwrap_or_default();
    let (title, tip) = match &live.prediction {
        Some(p) => (
            format!("{} {:.0}%", p.favourite().pal, p.prob * 100.0),
            format!(
                "Augur · {} {:.0}%{bet} · {}",
                p.favourite().pal,
                p.prob * 100.0,
                if live.picks_open { "picks open" } else { "fight on" }
            ),
        ),
        None => (String::new(), format!("Augur · {}", if live.title.is_empty() { "no duel up" } else { &live.title })),
    };
    let _ = tray.set_tooltip(Some(&tip));
    if cfg!(target_os = "macos") {
        let _ = tray.set_title(if show && !title.is_empty() { Some(title) } else { None });
    } else {
        let p = live.prediction.as_ref().filter(|_| show);
        let _ = tray.set_icon(Some(ring_icon(p.map(|p| p.prob), p.is_none_or(|p| p.pick == "home"))));
    }
    let _ = app;
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

/// Polls the live state, settles finished calls, makes the call for each new duel and keeps the tray current.
fn sync_loop(app: AppHandle, shared: Arc<Shared>) {
    let mut last_key = String::new();
    while !shared.stop.load(Ordering::Relaxed) {
        let poll = shared.settings.lock().unwrap().poll_wait(roll());
        let mut has_duel = false;
        match State::fetch() {
            Ok(state) => {
                shared.busy.store(state.busy(), Ordering::Relaxed);
                has_duel = state.home.is_some() && state.away.is_some();
                if let Ok(conn) = shared.db() {
                    if let Ok(settled) = db::settle(&conn, &state.recent, |id| site::duel(id).ok()) {
                        if !settled.is_empty() {
                            shared.bets_now.store(true, Ordering::Relaxed);
                            if let Some(c) = db::settled(&conn).ok().and_then(|all| all.into_iter().rev().find(|c| settled.contains(&c.id)))
                            {
                                let winner_pal = if c.winner.as_deref() == Some("home") { c.home_pal.clone() } else { c.away_pal.clone() };
                                let r = LastResult {
                                    home_pal: c.home_pal.clone(),
                                    away_pal: c.away_pal.clone(),
                                    winner_pal: winner_pal.clone(),
                                    right: c.right(),
                                    prob: c.prob,
                                    ko: c.ko.clone(),
                                    hp_left: c.winner_hp_left,
                                };
                                if shared.settings.lock().unwrap().notify_result {
                                    notify(
                                        &app,
                                        &format!("{} won {}", winner_pal, if c.right() { "✓" } else { "✕" }),
                                        &format!(
                                            "{} vs {}{}",
                                            c.home_pal,
                                            c.away_pal,
                                            c.ko.as_ref().map(|k| format!(" · K.O. {k}")).unwrap_or_default()
                                        ),
                                    );
                                }
                                shared.live.lock().unwrap().last_result = Some(r);
                            }
                        }
                    }
                    let est = |side: &str| {
                        site::parse_stats(state.hud.as_deref().unwrap_or(""))
                            .get(side)
                            .and_then(|s| s.get("HP"))
                            .and_then(|h| h.lv70)
                            .is_none()
                    };
                    let key = match (&state.home, &state.away) {
                        (Some(h), Some(a)) => format!("{}|{}|{}|{}|{}|{}", h.actor, a.actor, h.pal, a.pal, est("home"), est("away")),
                        _ => String::new(),
                    };
                    if key != last_key {
                        let settings = shared.settings.lock().unwrap().clone();
                        let p = predict::predict(
                            &state,
                            &predict::Options { lookups: true, db_path: settings::db_path(&shared.dir), settings: settings.model.clone() },
                        );
                        let stake = p.as_ref().map(|p| stake_for(&conn, &settings, &p.pick, p.prob));
                        if let (Some(p), Some(st)) = (&p, &stake) {
                            if let Ok(id) = db::add_call(&conn, &p.to_call()) {
                                let _ = db::set_call_stake(&conn, id, st.amount, &st.command);
                            }
                            if settings.notify_new_pick && !last_key.starts_with(&format!("{}|{}|", p.home.owner, p.away.owner)) {
                                let bet = if st.command.is_empty() { String::new() } else { format!(" · {}", st.command) };
                                notify(&app, &format!("{} {:.0}%{bet}", p.favourite().pal, p.prob * 100.0), &p.lines[0]);
                            }
                        }
                        let mut live = shared.live.lock().unwrap();
                        live.prediction = p;
                        live.stake = stake;
                        last_key = key;
                    }
                    let today = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                    let day = civil_date(today);
                    let calls = db::settled(&conn).unwrap_or_default();
                    let t = history::tally(&calls, Some(&day));
                    let staking = shared.settings.lock().unwrap().staking.clone();
                    let off_air = if has_duel { None } else { views::off_air(&conn, &staking).ok() };
                    let mut live = shared.live.lock().unwrap();
                    live.today = (t.right, t.total);
                    live.off_air = off_air;
                }
                let mut live = shared.live.lock().unwrap();
                live.first_run = shared.setup_wait.load(Ordering::Relaxed);
                live.title = state.title.clone();
                live.live = state.live;
                live.picks_open = state.picks_open();
                live.queue = state.queue.iter().filter(|q| q.pos > 0).take(4).cloned().collect();
                live.synced_at = now_ms();
                live.error = None;
            }
            Err(e) => shared.live.lock().unwrap().error = Some(e.to_string()),
        }
        let view = shared.live.lock().unwrap().clone();
        let _ = app.emit("live", &view);
        update_tray(&app, &shared);
        let wait = if has_duel { poll } else { Duration::from_secs(30) };
        let end = Instant::now() + wait;
        while Instant::now() < end && !shared.stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

/// Fetches species and past duels in the background, pausing whenever a duel is up. Runs again every 6 hours to pick
/// up new duels.
fn backfill_loop(app: AppHandle, shared: Arc<Shared>) {
    std::thread::sleep(Duration::from_secs(20));
    // a fresh install: wait for an import (it fills most of this in) or "start fresh", at most 10 minutes
    let asked = Instant::now();
    while shared.setup_wait.load(Ordering::Relaxed) && asked.elapsed() < Duration::from_secs(600) && !shared.stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_secs(1));
    }
    finish_setup(&app, &shared);
    while !shared.stop.load(Ordering::Relaxed) {
        if shared.settings.lock().unwrap().backfill {
            if let Ok(conn) = shared.db() {
                let busy = || shared.busy.load(Ordering::Relaxed);
                let progress = |p: &Progress| {
                    *shared.backfill.lock().unwrap() = p.clone();
                    let _ = app.emit("backfill", p);
                };
                let ctl = backfill::Control { stop: &shared.stop, busy: &busy, progress: &progress };
                if let Err(e) = backfill::run(&conn, &ctl) {
                    shared.backfill.lock().unwrap().last_error = Some(e.to_string());
                }
                *shared.backtest.lock().unwrap() = None;
            }
        }
        let end = Instant::now() + Duration::from_secs(6 * 3600);
        while Instant::now() < end && !shared.stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}

/// Reads the developer's picks list (signed in) every 5 minutes and soon after each result: the first time all of it,
/// a page at a time, pausing while a duel is up; afterwards only what's new.
fn bets_loop(app: AppHandle, shared: Arc<Shared>) {
    std::thread::sleep(Duration::from_secs(30));
    let mut pacer = backfill::Pacer::new(Duration::from_millis(1000));
    while !shared.stop.load(Ordering::Relaxed) {
        shared.bets_now.store(false, Ordering::Relaxed);
        pacer.gave_up = false;
        let on = shared.settings.lock().unwrap().import_bets;
        let conn = shared.db().ok();
        let user = conn.as_ref().and_then(|c| db::meta_get(c, "account").ok().flatten()).filter(|u| !u.is_empty());
        if let (true, Some(conn), Some(user), Some(cookie)) = (on, conn, user, secure::load(&shared.dir)) {
            pacer.cookie = Some(cookie);
            let busy = || shared.busy.load(Ordering::Relaxed);
            let progress = |p: &Progress| {
                *shared.bets.lock().unwrap() = p.clone();
                let _ = app.emit("bets", p);
            };
            let ctl = backfill::Control { stop: &shared.stop, busy: &busy, progress: &progress };
            let mut page = |n: u32, p: &mut Progress| pacer.get(&format!("{}/u/{user}?bp={n}", site::BASE), |b| b.to_string(), &ctl, p);
            let outcome = bets::import(&conn, &mut page, &ctl, now_ms());
            // your pet and Hall: two more pages, at most every 30 minutes
            let last: u64 = db::meta_get(&conn, "pet_page_at").ok().flatten().and_then(|v| v.parse().ok()).unwrap_or(0);
            if outcome.is_ok() && now_ms().saturating_sub(last) > 30 * 60_000 {
                if let Some(cookie) = pacer.cookie.as_deref() {
                    if let Ok((pet, hall)) = site::pet_and_hall(cookie) {
                        let _ = db::meta_set(&conn, "pet_page", &serde_json::to_string(&pet).unwrap_or_default());
                        let _ = db::meta_set(&conn, "hall_page", &serde_json::to_string(&hall).unwrap_or_default());
                        let _ = db::meta_set(&conn, "pet_page_at", &now_ms().to_string());
                    }
                }
            }
            let mut p = shared.bets.lock().unwrap();
            p.phase = "done".into();
            p.last_error = outcome.err().map(|e| e.to_string());
            let _ = app.emit("bets", &*p);
        }
        let end = Instant::now() + Duration::from_secs(300);
        while Instant::now() < end && !shared.stop.load(Ordering::Relaxed) && !shared.bets_now.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_secs(1));
        }
        // a result shows in the picks list a little after the duel ends
        if shared.bets_now.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_secs(20));
        }
    }
}

/// The pages are laid out for 1280x720. A bigger or smaller window zooms them to fit, so they look the same at any
/// size instead of stretching (or crowding) the layout.
fn fit_zoom(w: &tauri::WebviewWindow) {
    let (Ok(size), Ok(scale)) = (w.inner_size(), w.scale_factor()) else { return };
    let size = size.to_logical::<f64>(scale);
    let _ = w.set_zoom((size.width / 1280.0).min(size.height / 720.0).clamp(0.8, 2.0));
}

/// What each staking choice would suggest at a spread of chances, for Settings › Staking.
#[tauri::command]
fn stake_preview(shared: tauri::State<'_, Arc<Shared>>, staking: stake::StakeSettings) -> Cmd<Vec<(f64, Stake)>> {
    let conn = shared.db()?;
    let odds = bets::odds(&conn).unwrap_or_default();
    Ok([0.55, 0.6, 0.65, 0.7, 0.75, 0.8, 0.85, 0.9].iter().map(|&p| (p, stake::suggest("home", p, &staking, &odds))).collect())
}

fn toggle_popover(app: &AppHandle) {
    let Some(w) = app.get_webview_window("tray") else { return };
    if w.is_visible().unwrap_or(false) {
        let _ = w.hide();
    } else {
        let _ = w.move_window(if cfg!(target_os = "macos") { Position::TrayCenter } else { Position::TrayBottomCenter });
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// macOS: in the Dock (and ⌘-Tab) while the main window is open, menu bar only while it isn't.
fn in_dock(app: &AppHandle, shown: bool) {
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(if shown { tauri::ActivationPolicy::Regular } else { tauri::ActivationPolicy::Accessory });
    let _ = (app, shown);
}

pub fn open_main(app: &AppHandle, tab: Option<&str>) {
    in_dock(app, true);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        if let Some(t) = tab {
            let _ = w.emit("navigate", t);
        }
    }
    if let Some(t) = app.get_webview_window("tray") {
        let _ = t.hide();
    }
}

// --- commands the pages call ---

type Cmd<T> = Result<T, String>;

#[tauri::command]
fn live(shared: tauri::State<'_, Arc<Shared>>) -> LiveView {
    shared.live.lock().unwrap().clone()
}

#[tauri::command]
async fn history(shared: tauri::State<'_, Arc<Shared>>, range: String) -> Cmd<HistoryView> {
    let s = shared.settings.lock().unwrap().model.clone();
    views::history_view(&shared.db()?, &s, &range, now_ms() / 1000).map_err(|e| e.to_string())
}

#[tauri::command]
async fn bets_view(shared: tauri::State<'_, Arc<Shared>>, range: String) -> Cmd<BetsView> {
    let staking = shared.settings.lock().unwrap().staking.clone();
    let import = shared.bets.lock().unwrap().clone();
    views::bets_view(&shared.db()?, &staking, &range, now_ms() / 1000, import).map_err(|e| e.to_string())
}

/// A fresh install has chosen (or waited long enough): stop asking and let the backfill start.
pub fn finish_setup(app: &AppHandle, shared: &Shared) {
    if shared.setup_wait.swap(false, Ordering::Relaxed) {
        if let Ok(conn) = shared.db() {
            let _ = db::meta_set(&conn, "setup", "done");
        }
        let mut live = shared.live.lock().unwrap();
        live.first_run = false;
        let _ = app.emit("live", &*live);
    }
}

#[tauri::command]
fn start_fresh(app: AppHandle, shared: tauri::State<'_, Arc<Shared>>) {
    finish_setup(&app, &shared);
}

#[tauri::command]
fn backfill_progress(shared: tauri::State<'_, Arc<Shared>>) -> Progress {
    shared.backfill.lock().unwrap().clone()
}

#[tauri::command]
async fn you_view(shared: tauri::State<'_, Arc<Shared>>) -> Cmd<views::YouView> {
    let staking = shared.settings.lock().unwrap().staking.clone();
    views::you_view(&shared.db()?, &staking).map_err(|e| e.to_string())
}

#[tauri::command]
fn import_bets(shared: tauri::State<'_, Arc<Shared>>) {
    shared.bets_now.store(true, Ordering::Relaxed);
}

#[tauri::command]
async fn species_list(shared: tauri::State<'_, Arc<Shared>>) -> Cmd<Vec<history::SpeciesRecord>> {
    history::species_records(&shared.db()?).map_err(|e| e.to_string())
}

#[tauri::command]
async fn species(shared: tauri::State<'_, Arc<Shared>>, pal: String) -> Cmd<history::SpeciesDetail> {
    history::species_detail(&shared.db()?, &pal).map_err(|e| e.to_string())
}

#[tauri::command]
async fn player(shared: tauri::State<'_, Arc<Shared>>, name: String) -> Cmd<PlayerView> {
    let key = name.to_lowercase();
    let cached = shared.rosters.lock().unwrap().get(&key).filter(|(t, _)| t.elapsed() < Duration::from_secs(600)).map(|(_, r)| r.clone());
    let roster = match cached {
        Some(r) => Ok(r),
        None => site::roster(&key).inspect(|r| {
            shared.rosters.lock().unwrap().insert(key, (Instant::now(), r.clone()));
        }),
    };
    let s = shared.settings.lock().unwrap().model.clone();
    views::player_view(&shared.db()?, &s, &name, roster).map_err(|e| e.to_string())
}

#[tauri::command]
async fn model_view(shared: tauri::State<'_, Arc<Shared>>) -> Cmd<ModelView> {
    let s = shared.settings.lock().unwrap().model.clone();
    let cached = shared.backtest.lock().unwrap().clone().filter(|(cs, _)| *cs == s).map(|(_, b)| b);
    let backfill = shared.backfill.lock().unwrap().clone();
    let synced_at = shared.live.lock().unwrap().synced_at;
    let v = views::model_view(&shared.db()?, &s, cached, backfill, synced_at).map_err(|e| e.to_string())?;
    *shared.backtest.lock().unwrap() = v.backtest.clone().map(|b| (s, b));
    Ok(v)
}

#[tauri::command]
fn get_settings(shared: tauri::State<'_, Arc<Shared>>) -> AppSettings {
    shared.settings.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(app: AppHandle, shared: tauri::State<'_, Arc<Shared>>, settings: AppSettings) -> Cmd<()> {
    use tauri_plugin_autostart::ManagerExt;
    settings.save(&shared.dir)?;
    let autostart = app.autolaunch();
    let _ = if settings.launch_at_login { autostart.enable() } else { autostart.disable() };
    let _ = app.emit("settings", &settings);
    *shared.settings.lock().unwrap() = settings;
    // the staking settings may have changed: re-work the stake for the call that's up
    restake(&app, &shared);
    shared.sync_now.store(true, Ordering::Relaxed);
    Ok(())
}

/// A pal's art from palduels.com, downloaded the first time it's needed and cached (personal use).
#[tauri::command]
async fn pal_image(shared: tauri::State<'_, Arc<Shared>>, slug: String) -> Cmd<Option<String>> {
    if slug.is_empty() || !slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Ok(None);
    }
    let path = shared.dir.join("img").join("pals").join(format!("{slug}.webp"));
    let fresh = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .map(|t| t.elapsed().unwrap_or_default() < Duration::from_secs(30 * 86_400))
        .unwrap_or(false);
    if !fresh {
        if let Ok(bytes) = site::fetch_bytes(&format!("{}/img/pals/{slug}.webp", site::BASE)) {
            let _ = std::fs::create_dir_all(path.parent().unwrap());
            let _ = std::fs::write(&path, bytes);
        }
    }
    Ok(std::fs::read(&path).ok().map(|b| format!("data:image/webp;base64,{}", base64::engine::general_purpose::STANDARD.encode(b))))
}

/// A player's Kick profile picture, fetched the first time it's needed and cached for a week. A player with none is
/// remembered for a day, so a page full of them doesn't ask Kick again on every visit.
#[tauri::command]
async fn player_image(shared: tauri::State<'_, Arc<Shared>>, name: String) -> Cmd<Option<String>> {
    let Some(slug) = site::kick_slug(&name) else { return Ok(None) };
    let dir = shared.dir.join("img").join("players");
    let (path, none) = (dir.join(format!("{slug}.img")), dir.join(format!("{slug}.none")));
    let age = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok().map(|t| t.elapsed().unwrap_or_default());
    let fresh =
        age(&path).is_some_and(|a| a < Duration::from_secs(7 * 86_400)) || age(&none).is_some_and(|a| a < Duration::from_secs(86_400));
    if !fresh {
        let _ = std::fs::create_dir_all(&dir);
        match tauri::async_runtime::spawn_blocking(move || site::kick_pic(&name)).await {
            Ok(Ok(Some(bytes))) if image_type(&bytes).is_some() => {
                let _ = std::fs::write(&path, bytes);
                let _ = std::fs::remove_file(&none);
            }
            Ok(Ok(_)) => {
                let _ = std::fs::remove_file(&path);
                let _ = std::fs::write(&none, b"");
            }
            _ => {} // Kick didn't answer: keep whatever we had, and try again next time
        }
    }
    let bytes = std::fs::read(&path).ok();
    Ok(bytes.and_then(|b| image_type(&b).map(|t| format!("data:{t};base64,{}", base64::engine::general_purpose::STANDARD.encode(&b)))))
}

/// The kind of image in some bytes, for the ones Kick serves.
fn image_type(b: &[u8]) -> Option<&'static str> {
    match b {
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        _ => None,
    }
}

#[tauri::command]
fn open_window(app: AppHandle, tab: Option<String>) {
    open_main(&app, tab.as_deref());
}

/// The popover's height follows what it shows (the page measures itself).
#[tauri::command]
fn fit_popover(app: AppHandle, height: f64) {
    if let Some(w) = app.get_webview_window("tray") {
        let _ = w.set_size(tauri::LogicalSize::new(340.0, height.clamp(160.0, 640.0)));
        // Windows: the popover sits above the taskbar, so keep its bottom edge there
        if cfg!(windows) && w.is_visible().unwrap_or(false) {
            let _ = w.move_window(Position::TrayBottomCenter);
        }
    }
}

#[derive(Serialize)]
struct About {
    version: String,
    data_dir: String,
    /// calls, duels, fight tapes, species, your bets
    counts: [i64; 5],
}

/// Settings › General's About panel: the version, where the data lives and how much there is.
#[tauri::command]
fn about(shared: tauri::State<'_, Arc<Shared>>) -> Cmd<About> {
    let conn = shared.db()?;
    let count = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0);
    Ok(About {
        version: env!("CARGO_PKG_VERSION").into(),
        data_dir: shared.dir.display().to_string(),
        counts: [
            count("SELECT COUNT(*) FROM predictions WHERE status = 'settled'"),
            count("SELECT COUNT(*) FROM duels"),
            count("SELECT COUNT(DISTINCT duel_id) FROM hits"),
            count("SELECT COUNT(*) FROM species"),
            count("SELECT COUNT(*) FROM bets"),
        ],
    })
}

/// The popover's power button: quit Augur.
#[tauri::command]
fn quit_app(app: AppHandle, shared: tauri::State<'_, Arc<Shared>>) {
    shared.stop.store(true, Ordering::Relaxed);
    app.exit(0);
}

#[tauri::command]
fn hide_popover(app: AppHandle) {
    if let Some(w) = app.get_webview_window("tray") {
        let _ = w.hide();
    }
}

#[derive(Serialize)]
struct Account {
    signed_in: bool,
    name: Option<String>,
}

#[tauri::command]
fn account(shared: tauri::State<'_, Arc<Shared>>) -> Account {
    let name = db::open(&settings::db_path(&shared.dir)).ok().and_then(|c| db::meta_get(&c, "account").ok().flatten());
    Account { signed_in: secure::load(&shared.dir).is_some(), name }
}

/// Opens palduels.com's sign-in in its own window. Once the site lands on a profile page the session cookie is saved,
/// encrypted, and the window closes. Augur never sees the password.
#[tauri::command]
fn sign_in(app: AppHandle) -> Cmd<()> {
    if let Some(w) = app.get_webview_window("signin") {
        let _ = w.set_focus();
        return Ok(());
    }
    let url = tauri::Url::parse("https://palduels.com/login").map_err(|e| e.to_string())?;
    let handle = app.clone();
    WebviewWindowBuilder::new(&app, "signin", WebviewUrl::External(url))
        .title("Sign in to PalDuels")
        .inner_size(520.0, 720.0)
        .on_page_load(move |w, payload| {
            if payload.event() != PageLoadEvent::Finished {
                return;
            }
            let page = payload.url().clone();
            if page.host_str() != Some("palduels.com") || !page.path().starts_with("/u/") {
                return;
            }
            let name = page.path().trim_start_matches("/u/").trim_end_matches('/').to_string();
            let handle = handle.clone();
            // reading cookies on the UI thread deadlocks on Windows (WebView2), so do it from a thread
            std::thread::spawn(move || {
                let Ok(cookies) = w.cookies_for_url(tauri::Url::parse("https://palduels.com").unwrap()) else { return };
                let header = cookies.iter().map(|c| format!("{}={}", c.name(), c.value())).collect::<Vec<_>>().join("; ");
                let shared = handle.state::<Arc<Shared>>();
                if !header.is_empty() && secure::store(&shared.dir, &header).is_ok() {
                    if let Ok(conn) = shared.db() {
                        let _ = db::meta_set(&conn, "account", &name);
                    }
                    shared.bets_now.store(true, Ordering::Relaxed);
                    let _ = handle.emit("account", &name);
                    let _ = w.close();
                }
            });
        })
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn sign_out(app: AppHandle, shared: tauri::State<'_, Arc<Shared>>) {
    secure::clear(&shared.dir);
    if let Ok(conn) = shared.db() {
        let _ = conn.execute("DELETE FROM meta WHERE key = 'account'", []);
    }
    let _ = app.emit("account", "");
}

pub fn run() {
    let dir = settings::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    // a fresh install waits for the developer to import a backup or start fresh before fetching anything
    let first_run = db::open(&settings::db_path(&dir))
        .ok()
        .is_some_and(|c| db::is_empty(&c).unwrap_or(false) && db::meta_get(&c, "setup").ok().flatten().is_none());
    let mut sync_state = SyncState::load(&dir);
    if sync_state.name.is_empty() {
        sync_state.name = crate::sync::default_name();
    }
    let shared = Arc::new(Shared {
        settings: Mutex::new(AppSettings::load(&dir)),
        dir,
        live: Mutex::new(LiveView::default()),
        backfill: Mutex::new(Progress::default()),
        bets: Mutex::new(Progress::default()),
        bets_now: AtomicBool::new(false),
        backtest: Mutex::new(None),
        rosters: Mutex::new(HashMap::new()),
        busy: AtomicBool::new(false),
        stop: AtomicBool::new(false),
        sync: Mutex::new(sync_state),
        others: Mutex::new(vec![]),
        sync_now: AtomicBool::new(false),
        setup_wait: AtomicBool::new(first_run),
        update: Mutex::new(Default::default()),
        pending: Mutex::new(Default::default()),
        tray: Mutex::new(None),
    });
    tauri::Builder::default()
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .manage(shared.clone())
        .invoke_handler(tauri::generate_handler![
            live,
            history,
            bets_view,
            import_bets,
            you_view,
            stake_preview,
            start_fresh,
            backfill_progress,
            species_list,
            species,
            player,
            model_view,
            get_settings,
            save_settings,
            pal_image,
            player_image,
            open_window,
            hide_popover,
            fit_popover,
            quit_app,
            about,
            account,
            sign_in,
            sign_out,
            crate::sync::sync_view,
            crate::sync::sync_set,
            crate::sync::choose_sync_folder,
            crate::sync::backup,
            crate::sync::import_file,
            crate::updates::update_view,
            crate::updates::update_check,
            crate::updates::update_install,
            crate::updates::update_seen
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let open = MenuItem::with_id(app, "open", "Open Augur", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Augur", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            let icon = if cfg!(target_os = "macos") {
                Image::from_bytes(include_bytes!("../icons/tray-template.png"))?
            } else {
                ring_icon(None, true)
            };
            let tray = TrayIconBuilder::with_id("augur")
                .icon(icon)
                .icon_as_template(cfg!(target_os = "macos"))
                .tooltip("Augur")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "open" => open_main(app, None),
                    "quit" => {
                        app.state::<Arc<Shared>>().stop.store(true, Ordering::Relaxed);
                        app.exit(0)
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        toggle_popover(tray.app_handle());
                    }
                })
                .build(app)?;
            *shared.tray.lock().unwrap() = Some(tray);
            if let Some(w) = app.get_webview_window("tray") {
                let w2 = w.clone();
                w.on_window_event(move |e| {
                    if let WindowEvent::Focused(false) = e {
                        let _ = w2.hide();
                    }
                });
            }
            if let Some(w) = app.get_webview_window("main") {
                let w2 = w.clone();
                w.on_window_event(move |e| match e {
                    WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        let _ = w2.hide();
                        in_dock(w2.app_handle(), false);
                    }
                    WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => fit_zoom(&w2),
                    _ => {}
                });
                fit_zoom(&w);
            }
            // first launch: show the window so it's clear the app is running (afterwards it starts in the tray)
            if !shared.dir.join("settings.json").exists() {
                let _ = shared.settings.lock().unwrap().save(&shared.dir);
                open_main(app.handle(), Some("live"));
            }
            let (h1, s1) = (app.handle().clone(), shared.clone());
            std::thread::spawn(move || sync_loop(h1, s1));
            let (h2, s2) = (app.handle().clone(), shared.clone());
            std::thread::spawn(move || backfill_loop(h2, s2));
            let (h3, s3) = (app.handle().clone(), shared.clone());
            std::thread::spawn(move || bets_loop(h3, s3));
            let (h4, s4) = (app.handle().clone(), shared.clone());
            std::thread::spawn(move || crate::sync::sync_loop(h4, s4));
            let started = crate::updates::UpdateView::at_start(&shared);
            if let Some(from) = &started.updated_from {
                notify(
                    app.handle(),
                    &format!("Augur updated to {}", started.current),
                    &format!("from {from} · Settings › Updates says what's new"),
                );
            }
            *shared.update.lock().unwrap() = started;
            // restarted by an update with the window open: show it again
            if let Ok(conn) = shared.db() {
                if db::meta_get(&conn, "reopen_main").ok().flatten().as_deref() == Some("1") {
                    let _ = db::meta_set(&conn, "reopen_main", "");
                    open_main(app.handle(), None);
                }
            }
            let (h5, s5) = (app.handle().clone(), shared.clone());
            std::thread::spawn(move || crate::updates::check_loop(h5, s5));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Augur failed to start")
        .run(|app, event| {
            // macOS: clicking Augur in the Dock brings the window back
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                open_main(app, None);
            }
            let _ = (app, event);
        });
}

#[cfg(test)]
mod tests {
    /// macOS draws the menu bar icon from its transparency alone, so an icon with no transparent pixels shows as a
    /// white square.
    #[test]
    fn the_menu_bar_icon_is_a_shape_on_transparent() {
        let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template.png")).unwrap();
        let alpha: Vec<u8> = icon.rgba().chunks(4).map(|p| p[3]).collect();
        let clear = alpha.iter().filter(|&&a| a == 0).count();
        let solid = alpha.iter().filter(|&&a| a > 128).count();
        assert!(clear > alpha.len() / 2 && solid > alpha.len() / 10, "{clear} clear and {solid} solid of {} pixels", alpha.len());
    }
}
