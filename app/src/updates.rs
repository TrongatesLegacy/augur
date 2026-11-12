//! Updates: checking the latest GitHub release for a newer Augur (Tauri's updater, reading the release's latest.json),
//! downloading it in the background, and installing it when the developer clicks Restart to update. Updates are signed
//! with the project's update key, and one that isn't is refused. Nothing installs or restarts without that click, so a
//! duel is never cut off. After a restart, Settings › Updates says what changed, from the CHANGELOG.md built into the app.

use crate::app::{now_ms, Shared};
use crate::settings;
use augur_engine::db;
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::{Update, UpdaterExt};

/// Checks this often while the app runs (and shortly after it starts), when automatic checks are on.
const EVERY: Duration = Duration::from_secs(6 * 3600);

#[derive(Clone, Debug, Serialize)]
pub struct Release {
    pub version: String,
    pub notes: String,
    /// when it was published (ms)
    pub published: Option<i64>,
}

/// What the pages show about updates.
#[derive(Clone, Debug, Default, Serialize)]
pub struct UpdateView {
    pub current: String,
    /// a newer release, when there is one
    pub latest: Option<Release>,
    pub checked_at: Option<u64>,
    pub checking: bool,
    pub error: Option<String>,
    /// while downloading: (bytes so far, size when known)
    pub progress: Option<(u64, Option<u64>)>,
    /// downloaded and waiting for Restart to update
    pub ready: bool,
    /// installing now: Augur is about to restart
    pub installing: bool,
    /// set after an update, until the developer has seen it: the version it came from
    pub updated_from: Option<String>,
    /// this version's CHANGELOG.md, built in: what changed in it and every version before
    pub changelog: &'static str,
}

/// The newer release and, once downloaded, its files.
#[derive(Default)]
pub struct Pending {
    pub update: Option<Update>,
    pub bytes: Option<Vec<u8>>,
}

impl UpdateView {
    /// At start: the running version, and whether it's new since the last run (an update just installed).
    pub fn at_start(shared: &Shared) -> Self {
        let current = env!("CARGO_PKG_VERSION").to_string();
        let mut v = UpdateView { current: current.clone(), changelog: include_str!("../../CHANGELOG.md"), ..Default::default() };
        if let Ok(conn) = db::open(&settings::db_path(&shared.dir)) {
            let last = db::meta_get(&conn, "last_version").ok().flatten();
            if last.as_deref().is_some_and(|l| l != current) {
                v.updated_from = last;
            }
            let _ = db::meta_set(&conn, "last_version", &current);
        }
        v
    }
}

fn emit(app: &AppHandle, shared: &Shared) {
    let v = shared.update.lock().unwrap().clone();
    let _ = app.emit("update", &v);
}

/// Asks GitHub for the latest release, and (when `download`) fetches it in the background so the restart is instant.
pub async fn check(app: &AppHandle, shared: &Arc<Shared>, download: bool) {
    shared.update.lock().unwrap().checking = true;
    emit(app, shared);
    let found = match app.updater() {
        Ok(u) => u.check().await.map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    {
        let mut v = shared.update.lock().unwrap();
        v.checking = false;
        v.checked_at = Some(now_ms());
        match &found {
            Ok(found) => {
                v.error = None;
                let same = v.latest.as_ref().map(|r| r.version.as_str()) == found.as_ref().map(|u| u.version.as_str());
                v.latest = found.as_ref().map(|u| Release {
                    version: u.version.clone(),
                    notes: u.body.clone().unwrap_or_default(),
                    published: u.date.map(|d| d.unix_timestamp() * 1000),
                });
                if !same {
                    v.ready = false;
                    shared.pending.lock().unwrap().bytes = None;
                }
            }
            // offline or GitHub unreachable: say so, try again next time
            Err(e) => v.error = Some(format!("Couldn't check for updates: {e}")),
        }
    }
    let update = found.ok().flatten();
    shared.pending.lock().unwrap().update = update.clone();
    emit(app, shared);
    if let Some(u) = update.filter(|_| download && !shared.update.lock().unwrap().ready) {
        if fetch(app, shared, &u).await.is_ok() {
            let _ = app
                .notification()
                .builder()
                .title(format!("Augur {} is ready", u.version))
                .body("Restart to update: open Augur, or Settings › Updates")
                .show();
        }
    }
}

/// Downloads the update into memory (a few MB), reporting progress.
async fn fetch(app: &AppHandle, shared: &Arc<Shared>, u: &Update) -> Result<(), String> {
    let mut got = 0u64;
    let (a, s) = (app.clone(), shared.clone());
    shared.update.lock().unwrap().progress = Some((0, None));
    emit(app, shared);
    let bytes = u
        .download(
            move |chunk, total| {
                got += chunk as u64;
                s.update.lock().unwrap().progress = Some((got, total));
                emit(&a, &s);
            },
            || {},
        )
        .await;
    let mut v = shared.update.lock().unwrap();
    v.progress = None;
    let out = match bytes {
        Ok(b) => {
            shared.pending.lock().unwrap().bytes = Some(b);
            v.ready = true;
            Ok(())
        }
        Err(e) => {
            v.error = Some(format!("The update didn't download: {e}"));
            Err(e.to_string())
        }
    };
    drop(v);
    emit(app, shared);
    out
}

/// Checks shortly after the app starts and every 6 hours, while automatic checks are on.
pub fn check_loop(app: AppHandle, shared: Arc<Shared>) {
    std::thread::sleep(Duration::from_secs(15));
    while !shared.stop.load(Ordering::Relaxed) {
        let (on, download) = {
            let s = shared.settings.lock().unwrap();
            (s.check_updates, s.auto_update)
        };
        if on {
            tauri::async_runtime::block_on(check(&app, &shared, download));
        }
        let end = Instant::now() + EVERY;
        while Instant::now() < end && !shared.stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_secs(5));
        }
    }
}

// --- commands ---

#[tauri::command]
pub fn update_view(shared: tauri::State<'_, Arc<Shared>>) -> UpdateView {
    shared.update.lock().unwrap().clone()
}

#[tauri::command]
pub async fn update_check(app: AppHandle, shared: tauri::State<'_, Arc<Shared>>) -> Result<UpdateView, String> {
    let download = shared.settings.lock().unwrap().auto_update;
    check(&app, shared.inner(), download).await;
    Ok(shared.update.lock().unwrap().clone())
}

/// Restart to update: installs the downloaded update (downloading it first if it isn't yet) and restarts into it. The
/// main window comes back if it was open.
#[tauri::command]
pub async fn update_install(app: AppHandle, shared: tauri::State<'_, Arc<Shared>>) -> Result<(), String> {
    let shared = shared.inner().clone();
    let pending = shared.pending.lock().unwrap().update.clone();
    let update = match pending {
        Some(u) => u,
        None => app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?.ok_or("Augur is already up to date")?,
    };
    if shared.pending.lock().unwrap().bytes.is_none() {
        fetch(&app, &shared, &update).await?;
    }
    let bytes = shared.pending.lock().unwrap().bytes.clone().ok_or("The update didn't download")?;
    // kept so the next start can show the window again
    if let Ok(conn) = db::open(&settings::db_path(&shared.dir)) {
        let open = app.get_webview_window("main").is_some_and(|w| w.is_visible().unwrap_or(false));
        let _ = db::meta_set(&conn, "reopen_main", if open { "1" } else { "" });
    }
    shared.update.lock().unwrap().installing = true;
    emit(&app, &shared);
    if let Err(e) = update.install(bytes) {
        let mut v = shared.update.lock().unwrap();
        v.installing = false;
        v.error = Some(format!("The update didn't install: {e}"));
        drop(v);
        emit(&app, &shared);
        return Err(e.to_string());
    }
    shared.stop.store(true, Ordering::Relaxed);
    app.restart();
}

/// The developer has seen what changed in this version.
#[tauri::command]
pub fn update_seen(shared: tauri::State<'_, Arc<Shared>>) {
    let mut v = shared.update.lock().unwrap();
    v.updated_from = None;
}
