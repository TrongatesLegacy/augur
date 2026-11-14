//! Settings › Sync & data: syncing settings and calls through a folder a sync app moves (`augur_engine::sync`), and a
//! whole-store backup and import for setting up another computer.

use crate::app::{now_ms, Shared};
use crate::settings::{self, AppSettings};
use augur_engine::db;
use augur_engine::sync::{self, DeviceFile, Folder, SyncState, What};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tauri_plugin_dialog::DialogExt;

/// Settings each computer keeps for itself.
pub const LOCAL: &[&str] = &["launch_at_login", "start_in_tray", "backfill"];

const OS: &str = if cfg!(target_os = "macos") {
    "macOS"
} else if cfg!(windows) {
    "Windows"
} else {
    "Linux"
};

/// What Settings › Sync & data shows.
#[derive(Clone, Debug, Serialize)]
pub struct SyncView {
    pub on: bool,
    pub folder: Option<PathBuf>,
    pub name: String,
    pub what: What,
    pub last_sync: Option<u64>,
    pub error: Option<String>,
    pub history: Vec<sync::Change>,
    pub others: Vec<DeviceFile>,
    /// folders a sync app already moves on this computer
    pub folders: Vec<Folder>,
}

pub fn view(st: &SyncState, others: &[DeviceFile]) -> SyncView {
    let home = dirs::home_dir().unwrap_or_default();
    SyncView {
        on: st.on,
        folder: st.folder.clone(),
        name: st.name.clone(),
        what: st.what.clone(),
        last_sync: st.last_sync,
        error: st.error.clone(),
        history: st.history.iter().take(12).cloned().collect(),
        others: others.to_vec(),
        folders: sync::synced_folders(&home, &|k| std::env::var(k).ok(), cfg!(windows)),
    }
}

/// This computer's name as the others see it, until the developer changes it.
pub fn default_name() -> String {
    let name = if cfg!(target_os = "macos") {
        std::process::Command::new("scutil")
            .args(["--get", "ComputerName"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        std::env::var("COMPUTERNAME").ok()
    };
    name.filter(|n| !n.is_empty()).unwrap_or_else(|| "This computer".into())
}

fn values(s: &AppSettings) -> std::collections::BTreeMap<String, serde_json::Value> {
    sync::flatten(&serde_json::to_value(s).unwrap_or_default(), LOCAL)
}

/// One sync: write this computer's files, read the others', apply what changed.
fn run_once(app: &AppHandle, shared: &Shared) {
    let Ok(conn) = db::open(&settings::db_path(&shared.dir)) else { return };
    let current = shared.settings.lock().unwrap().clone();
    let mut st = shared.sync.lock().unwrap().clone();
    let got = st.sync(&conn, &values(&current), OS, env!("CARGO_PKG_VERSION"), now_ms());
    if !got.settings.is_empty() {
        let mut v = serde_json::to_value(&current).unwrap_or_default();
        sync::apply(&mut v, &got.settings);
        if let Ok(next) = serde_json::from_value::<AppSettings>(v) {
            let _ = next.save(&shared.dir);
            *shared.settings.lock().unwrap() = next.clone();
            let _ = app.emit("settings", &next);
            crate::app::restake(app, shared);
        }
    }
    let _ = st.save(&shared.dir);
    *shared.others.lock().unwrap() = got.others.clone();
    *shared.sync.lock().unwrap() = st.clone();
    let _ = app.emit("sync", view(&st, &got.others));
    if got.calls > 0 {
        let _ = app.emit("calls", got.calls);
    }
}

/// How often to look for the other computers' files changing (a few directory listings, no reading).
const LOOK_EVERY: Duration = Duration::from_secs(5);

/// Syncs every minute while on, a few seconds after anything changes here, and as soon as the sync app brings in a
/// change from another computer (it can take minutes, so a launch's first sync may come too early to see it).
pub fn sync_loop(app: AppHandle, shared: Arc<Shared>) {
    std::thread::sleep(Duration::from_secs(5));
    while !shared.stop.load(Ordering::Relaxed) {
        shared.sync_now.store(false, Ordering::Relaxed);
        let st = shared.sync.lock().unwrap().clone();
        let stamp = || st.dir().map(|d| sync::others_stamp(&d, &st.device));
        let seen = stamp();
        if st.on {
            run_once(&app, &shared);
        }
        let end = Instant::now() + Duration::from_secs(60);
        let mut look = Instant::now() + LOOK_EVERY;
        while Instant::now() < end && !shared.stop.load(Ordering::Relaxed) && !shared.sync_now.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(500));
            if st.on && Instant::now() >= look {
                if stamp() != seen {
                    break;
                }
                look = Instant::now() + LOOK_EVERY;
            }
        }
        // let a burst of changes settle before writing
        std::thread::sleep(Duration::from_secs(2));
    }
}

// --- commands ---

type Cmd<T> = Result<T, String>;

#[tauri::command]
pub fn sync_view(shared: tauri::State<'_, Arc<Shared>>) -> SyncView {
    let st = shared.sync.lock().unwrap().clone();
    view(&st, &shared.others.lock().unwrap())
}

#[derive(Deserialize)]
pub struct SyncChange {
    pub on: Option<bool>,
    pub folder: Option<PathBuf>,
    pub name: Option<String>,
    pub what: Option<What>,
}

#[tauri::command]
pub fn sync_set(shared: tauri::State<'_, Arc<Shared>>, change: SyncChange) -> Cmd<SyncView> {
    let mut st = shared.sync.lock().unwrap();
    if let Some(f) = change.folder {
        st.folder = Some(f);
        st.on = true;
    }
    if let Some(on) = change.on {
        st.on = on && st.folder.is_some();
    }
    if let Some(n) = change.name {
        st.name = n.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(40).collect();
    }
    if let Some(w) = change.what {
        st.what = w;
    }
    st.error = None;
    st.save(&shared.dir).map_err(|e| e.to_string())?;
    shared.sync_now.store(true, Ordering::Relaxed);
    Ok(view(&st, &shared.others.lock().unwrap()))
}

#[tauri::command]
pub async fn choose_sync_folder(app: AppHandle) -> Option<PathBuf> {
    app.dialog().file().set_title("Choose the folder to sync through").blocking_pick_folder().and_then(|p| p.into_path().ok())
}

/// Saves a complete copy of the store where the developer picks. Returns where, or None if they cancelled.
#[tauri::command]
pub async fn backup(app: AppHandle, shared: tauri::State<'_, Arc<Shared>>) -> Cmd<Option<PathBuf>> {
    let day = crate::views::civil_date(now_ms() / 1000);
    let Some(path) = app
        .dialog()
        .file()
        .set_title("Back up Augur")
        .set_file_name(format!("Augur backup {day}.db"))
        .add_filter("Augur backup", &["db"])
        .blocking_save_file()
        .and_then(|p| p.into_path().ok())
    else {
        return Ok(None);
    };
    let conn = db::open(&settings::db_path(&shared.dir)).map_err(|e| e.to_string())?;
    db::backup_to(&conn, &path).map_err(|e| e.to_string())?;
    Ok(Some(path))
}

#[derive(Serialize)]
pub struct Imported {
    pub file: String,
    pub merged: db::Merged,
}

/// Merges a backup (.db) or a calls file (.jsonl) into the store. None if the developer cancelled.
#[tauri::command]
pub async fn import_file(app: AppHandle, shared: tauri::State<'_, Arc<Shared>>) -> Cmd<Option<Imported>> {
    let Some(path) = app
        .dialog()
        .file()
        .set_title("Import a backup or calls file")
        .add_filter("Augur backup or calls file", &["db", "jsonl"])
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok())
    else {
        return Ok(None);
    };
    let conn = db::open(&settings::db_path(&shared.dir)).map_err(|e| e.to_string())?;
    let merged = if path.extension().is_some_and(|x| x == "jsonl") {
        augur_engine::calls::import_all(&conn, &path).map_err(|e| e.to_string())?
    } else {
        db::merge_from(&conn, &path).map_err(|e| e.to_string())?
    };
    *shared.backtest.lock().unwrap() = None;
    crate::app::finish_setup(&app, &shared);
    let _ = app.emit("calls", merged.calls);
    shared.sync_now.store(true, Ordering::Relaxed);
    let file = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    Ok(Some(Imported { file, merged }))
}
