//! Sync across computers through a folder a sync app already moves (OneDrive, iCloud Drive, Dropbox, Google Drive,
//! Syncthing…), the way Stream Multiview does it. Each computer writes only its own files and reads everyone's, so
//! nothing needs a lock and sync apps never make "conflicted copies":
//!     Augur Sync/devices/<device>.json    its name, app version and settings, each with when it last changed
//!     Augur Sync/calls/<device>.jsonl     its calls (the calls file format), merged into the others' stores
//! Settings merge one at a time: the latest change wins. Calls merge without duplicates. What stays on each computer
//! (its sign-in, launch at login, the public data it fetches itself) is the caller's to leave out.

use crate::{calls, db};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The app's folder inside the chosen one.
pub const SUB: &str = "Augur Sync";
/// Changes kept in the history.
const HISTORY_MAX: usize = 50;
/// A computer not heard from in this long is left out of merges (it may come back with old values).
const STALE_MS: u64 = 120 * 86_400_000;

/// A setting's value and when it last changed (ms).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub value: Value,
    pub at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct What {
    pub settings: bool,
    pub calls: bool,
}

impl Default for What {
    fn default() -> Self {
        What { settings: true, calls: true }
    }
}

/// One sync that changed something here.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub at: u64,
    /// the computer it came from
    pub from: String,
    pub summary: String,
}

/// This computer's sync state, kept in the data folder (sync.json) whether sync is on or not, so a first sync knows
/// which settings changed when.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncState {
    pub on: bool,
    /// the chosen folder (Augur Sync is made inside it)
    pub folder: Option<PathBuf>,
    pub device: String,
    pub name: String,
    pub what: What,
    pub record: BTreeMap<String, Entry>,
    pub history: Vec<Change>,
    pub last_sync: Option<u64>,
    pub error: Option<String>,
    /// how many calls this computer last wrote to its calls file
    pub calls_written: usize,
    /// what the calls file last held: settled calls and how many are marked damaged. A new result or mark (or an
    /// update that adds something to the file, which starts this empty) writes the file again
    pub calls_mark: String,
}

/// A computer's file in the folder.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceFile {
    pub device: String,
    pub name: String,
    pub os: String,
    pub app: String,
    pub at: u64,
    #[serde(default)]
    pub settings: BTreeMap<String, Entry>,
    #[serde(default)]
    pub calls: usize,
}

/// A folder a sync app already moves on this computer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    /// "onedrive", "icloud", "dropbox", "gdrive"
    pub id: String,
    pub name: String,
    pub path: PathBuf,
}

/// What a sync did.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Synced {
    /// settings that changed here (flattened keys), with their new values
    pub settings: BTreeMap<String, Value>,
    pub calls: usize,
    /// the other computers in the folder
    pub others: Vec<DeviceFile>,
}

impl SyncState {
    pub fn load(dir: &Path) -> Self {
        let mut st: SyncState =
            std::fs::read_to_string(dir.join("sync.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        if st.device.is_empty() {
            st.device = new_id();
        }
        st
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        write_atomic(&dir.join("sync.json"), &serde_json::to_string_pretty(self)?)
    }

    pub fn dir(&self) -> Option<PathBuf> {
        self.folder.as_ref().map(|f| if f.file_name().is_some_and(|n| n == SUB) { f.clone() } else { f.join(SUB) })
    }

    /// Notes this computer's current settings: any that differ from the record changed now. Returns whether any did.
    pub fn note_local(&mut self, values: &BTreeMap<String, Value>, now: u64) -> bool {
        let mut changed = false;
        for (k, v) in values {
            if self.record.get(k).is_none_or(|e| &e.value != v) {
                // first sight of a setting at its first value counts as old, so a new computer doesn't win with defaults
                let at = if self.record.is_empty() { 0 } else { now };
                self.record.insert(k.clone(), Entry { value: v.clone(), at });
                changed = true;
            }
        }
        changed
    }

    /// Writes this computer's file and calls, reads the others' and merges them. `values`: this computer's synced
    /// settings, flattened; `os`, `app`: shown to the other computers.
    pub fn sync(&mut self, conn: &Connection, values: &BTreeMap<String, Value>, os: &str, app: &str, now: u64) -> Synced {
        let mut out = Synced::default();
        let Some(dir) = self.dir().filter(|_| self.on) else { return out };
        if !self.folder.as_ref().is_some_and(|f| f.is_dir()) {
            self.error = Some("The sync folder isn't there: is the drive connected and its sync app running?".into());
            return out;
        }
        self.note_local(values, now);
        let others = read_others(&dir, &self.device, now);
        if self.what.settings {
            let (applied, from) = merge(&mut self.record, &others);
            if !applied.is_empty() {
                let names: Vec<String> = applied.keys().map(|k| label(k)).collect();
                self.remember(Change { at: now, from: from.unwrap_or_default(), summary: names.join(", ") });
            }
            out.settings = applied;
        }
        if self.what.calls {
            for o in &others {
                let file = dir.join("calls").join(format!("{}.jsonl", o.device));
                if file.exists() {
                    out.calls += calls::import(conn, &file).unwrap_or(0);
                }
            }
            if out.calls > 0 {
                let from = if others.len() == 1 { others[0].name.clone() } else { format!("{} computers", others.len()) };
                self.remember(Change { at: now, from, summary: format!("{} call{}", out.calls, if out.calls == 1 { "" } else { "s" }) });
            }
        }
        let written = self.write_own(conn, &dir, os, app, now);
        self.error = written.err().map(|e| format!("Couldn't write to the sync folder: {e}"));
        self.last_sync = Some(now);
        out.others = others;
        out
    }

    fn write_own(&mut self, conn: &Connection, dir: &Path, os: &str, app: &str, now: u64) -> crate::Result<()> {
        let calls_file = dir.join("calls").join(format!("{}.jsonl", self.device));
        let settled = db::settled(conn)?;
        // written again when a call is added, marked damaged or gains its pal ranks (the others fill them in)
        let ranked = settled.iter().filter(|c| c.fighters.as_ref().is_some_and(|(h, _)| h.fighter.pal_rank.is_some())).count();
        let mark = format!("{}:{}:{ranked}", settled.len(), settled.iter().filter(|c| c.damaged).count());
        if self.what.calls && (mark != self.calls_mark || !calls_file.exists()) {
            std::fs::create_dir_all(calls_file.parent().unwrap())?;
            self.calls_written = calls::export(conn, &calls_file)?;
            self.calls_mark = mark;
        }
        let file = DeviceFile {
            device: self.device.clone(),
            name: self.name.clone(),
            os: os.into(),
            app: app.into(),
            at: now,
            settings: if self.what.settings { self.record.clone() } else { BTreeMap::new() },
            calls: self.calls_written,
        };
        let path = dir.join("devices").join(format!("{}.json", self.device));
        std::fs::create_dir_all(path.parent().unwrap())?;
        // written in place: some sync apps mark a file renamed over another as uploaded without uploading it
        std::fs::write(path, serde_json::to_string(&file)?)?;
        Ok(())
    }

    fn remember(&mut self, c: Change) {
        self.history.insert(0, c);
        self.history.truncate(HISTORY_MAX);
    }
}

/// The other computers' files, skipping half-synced and long-silent ones. One per computer: a sync app's leftover
/// copy of a file ("<device>-<name>.json") is older than the real one, so the newest wins.
pub fn read_others(dir: &Path, own: &str, now: u64) -> Vec<DeviceFile> {
    let Ok(names) = std::fs::read_dir(dir.join("devices")) else { return vec![] };
    let mut newest: BTreeMap<String, DeviceFile> = BTreeMap::new();
    for f in names
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_str::<DeviceFile>(&std::fs::read_to_string(e.path()).ok()?).ok())
        .filter(|f| f.device != own && now.saturating_sub(f.at) < STALE_MS)
    {
        if newest.get(&f.device).is_none_or(|n| f.at > n.at) {
            newest.insert(f.device.clone(), f);
        }
    }
    let mut out: Vec<DeviceFile> = newest.into_values().collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// A fingerprint of the other computers' files (names, sizes, times). Cheap enough to check every few seconds, so a
/// sync can follow soon after the sync app brings something in.
pub fn others_stamp(dir: &Path, own: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for sub in ["devices", "calls"] {
        let Ok(names) = std::fs::read_dir(dir.join(sub)) else { continue };
        let mut files: Vec<(String, u64, Option<std::time::SystemTime>)> = names
            .flatten()
            .filter(|e| !e.file_name().to_string_lossy().starts_with(own))
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                Some((e.file_name().to_string_lossy().into_owned(), m.len(), m.modified().ok()))
            })
            .collect();
        files.sort();
        files.hash(&mut h);
    }
    h.finish()
}

/// Takes each setting another computer changed more recently. Returns the values that changed here and the computer
/// most of them came from.
pub fn merge(record: &mut BTreeMap<String, Entry>, others: &[DeviceFile]) -> (BTreeMap<String, Value>, Option<String>) {
    let mut applied = BTreeMap::new();
    let mut from: BTreeMap<&str, usize> = BTreeMap::new();
    for o in others {
        for (k, e) in &o.settings {
            let mine = record.get(k);
            if mine.is_none_or(|m| e.at > m.at) {
                if mine.is_none_or(|m| m.value != e.value) {
                    applied.insert(k.clone(), e.value.clone());
                    *from.entry(o.name.as_str()).or_default() += 1;
                }
                record.insert(k.clone(), e.clone());
            }
        }
    }
    let top = from.into_iter().max_by_key(|(_, n)| *n).map(|(n, _)| n.to_string());
    (applied, top)
}

/// Settings as flat keys, one level into objects: {"staking": {"min_bet": 100}} -> {"staking.min_bet": 100}.
pub fn flatten(v: &Value, leave_out: &[&str]) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    if let Value::Object(map) = v {
        for (k, v) in map.iter().filter(|(k, _)| !leave_out.contains(&k.as_str())) {
            match v {
                Value::Object(inner) => {
                    for (ik, iv) in inner {
                        out.insert(format!("{k}.{ik}"), iv.clone());
                    }
                }
                _ => {
                    out.insert(k.clone(), v.clone());
                }
            }
        }
    }
    out
}

/// Puts flat keys back into a settings object.
pub fn apply(target: &mut Value, values: &BTreeMap<String, Value>) {
    for (k, v) in values {
        match k.split_once('.') {
            Some((outer, inner)) => {
                if let Some(obj) = target.get_mut(outer).and_then(|o| o.as_object_mut()) {
                    obj.insert(inner.to_string(), v.clone());
                }
            }
            None => {
                if let Some(obj) = target.as_object_mut() {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
    }
}

/// "staking.max_bet" -> "max bet".
fn label(key: &str) -> String {
    key.rsplit('.').next().unwrap_or(key).replace('_', " ")
}

/// The folders already synced on this computer.
pub fn synced_folders(home: &Path, env: &dyn Fn(&str) -> Option<String>, windows: bool) -> Vec<Folder> {
    let mut out: Vec<Folder> = vec![];
    let mut add = |id: &str, name: &str, p: PathBuf| {
        if p.is_dir() && !out.iter().any(|f| f.path == p) {
            out.push(Folder { id: id.into(), name: name.into(), path: p });
        }
    };
    let under = |dir: PathBuf, prefix: &str| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|r| r.flatten().filter(|e| e.file_name().to_string_lossy().starts_with(prefix)).map(|e| e.path()).collect())
            .unwrap_or_default()
    };
    let cloud = home.join("Library/CloudStorage");
    if windows {
        for k in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
            if let Some(p) = env(k) {
                add("onedrive", "OneDrive", PathBuf::from(p));
            }
        }
        add("icloud", "iCloud Drive", home.join("iCloudDrive"));
        add("gdrive", "Google Drive", PathBuf::from(r"G:\My Drive"));
    } else {
        add("icloud", "iCloud Drive", home.join("Library/Mobile Documents/com~apple~CloudDocs"));
        for p in under(cloud.clone(), "OneDrive") {
            add("onedrive", "OneDrive", p);
        }
        for p in under(cloud.clone(), "GoogleDrive") {
            add("gdrive", "Google Drive", p.join("My Drive"));
        }
        for p in under(cloud, "Dropbox") {
            add("dropbox", "Dropbox", p);
        }
    }
    add("onedrive", "OneDrive", home.join("OneDrive"));
    add("dropbox", "Dropbox", home.join("Dropbox"));
    out
}

fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    format!("{:016x}", h.finish())
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("augur-sync-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn computer(name: &str, folder: &Path) -> SyncState {
        SyncState { on: true, folder: Some(folder.into()), device: format!("dev-{name}"), name: name.into(), ..Default::default() }
    }

    #[test]
    fn settings_flatten_and_apply() {
        let v = json!({"poll_secs": 5, "staking": {"min_bet": 100, "strategy": "kelly"}, "launch_at_login": true});
        let flat = flatten(&v, &["launch_at_login"]);
        assert_eq!(flat.keys().collect::<Vec<_>>(), ["poll_secs", "staking.min_bet", "staking.strategy"]);
        let mut target = v.clone();
        apply(&mut target, &BTreeMap::from([("staking.min_bet".into(), json!(200)), ("poll_secs".into(), json!(10))]));
        assert_eq!(target["staking"]["min_bet"], 200);
        assert_eq!(target["poll_secs"], 10);
        assert_eq!(target["staking"]["strategy"], "kelly");
    }

    #[test]
    fn two_computers_share_settings_and_calls() {
        let folder = temp("two");
        let (mac_db, pc_db) = (crate::seed::store(), db::open_memory().unwrap());
        let (mut mac, mut pc) = (computer("Mac", &folder), computer("PC", &folder));
        let mac_vals = flatten(&json!({"staking": {"strategy": "kelly", "max_from": 0.7}, "default_voice": "Yoda"}), &[]);
        let pc_vals = flatten(&json!({"staking": {"strategy": "kelly", "max_from": 0.7}, "default_voice": "Yoda"}), &[]);
        mac.sync(&mac_db, &mac_vals, "macOS", "0.1.0", 1_000);
        let got = pc.sync(&pc_db, &pc_vals, "Windows", "0.1.0", 2_000);
        assert_eq!(got.calls, 19, "the PC picks up the Mac's calls");
        assert!(got.settings.is_empty(), "nothing differs yet");
        assert_eq!(got.others.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(), ["Mac"]);
        // the Mac changes the strategy; the PC changes the voice later: each gets the other's change
        let mac_vals = flatten(&json!({"staking": {"strategy": "default_or_max", "max_from": 0.7}, "default_voice": "Yoda"}), &[]);
        mac.sync(&mac_db, &mac_vals, "macOS", "0.1.0", 3_000);
        let pc_vals = flatten(&json!({"staking": {"strategy": "kelly", "max_from": 0.7}, "default_voice": "Gollum"}), &[]);
        let got = pc.sync(&pc_db, &pc_vals, "Windows", "0.1.0", 4_000);
        assert_eq!(got.settings, BTreeMap::from([("staking.strategy".into(), json!("default_or_max"))]));
        assert_eq!(pc.history[0].from, "Mac");
        let got = mac.sync(&mac_db, &mac_vals, "macOS", "0.1.0", 5_000);
        assert_eq!(got.settings, BTreeMap::from([("default_voice".into(), json!("Gollum"))]));
        assert_eq!(got.calls, 0, "calls already there aren't added twice");
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn a_damaged_mark_reaches_the_other_computer_without_a_new_result() {
        let folder = temp("mark");
        let (mac_db, pc_db) = (crate::seed::store(), db::open_memory().unwrap());
        let (mut mac, mut pc) = (computer("Mac", &folder), computer("PC", &folder));
        let none = BTreeMap::new();
        mac.sync(&mac_db, &none, "macOS", "0.1.0", 1_000);
        pc.sync(&pc_db, &none, "Windows", "0.1.0", 2_000);
        // the Mac marks a call it already shared: no new result, but its calls file is written again
        let id = db::settled(&mac_db).unwrap()[0].id;
        mac_db.execute("UPDATE predictions SET damaged = 1 WHERE id = ?1", [id]).unwrap();
        mac.sync(&mac_db, &none, "macOS", "0.1.0", 3_000);
        pc.sync(&pc_db, &none, "Windows", "0.1.0", 4_000);
        let marked: i64 = pc_db.query_row("SELECT COUNT(*) FROM predictions WHERE damaged = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(marked, 1, "the PC has the Mac's mark");
        let _ = std::fs::remove_dir_all(folder);
    }

    /// Pal ranks filled in on one computer (for calls made before Augur read them) reach the other, which already has
    /// those calls, without a new result; what the other has already set stays.
    #[test]
    fn filled_in_pal_ranks_reach_the_other_computer() {
        let folder = temp("ranks");
        let (mac_db, pc_db) = (crate::seed::store(), db::open_memory().unwrap());
        let (mut mac, mut pc) = (computer("Mac", &folder), computer("PC", &folder));
        let none = BTreeMap::new();
        mac.sync(&mac_db, &none, "macOS", "0.1.0", 1_000);
        pc.sync(&pc_db, &none, "Windows", "0.1.0", 2_000);
        let ranked = |conn: &Connection| -> i64 {
            conn.query_row("SELECT COUNT(*) FROM fighters WHERE power IS NOT NULL", [], |r| r.get(0)).unwrap()
        };
        assert_eq!(ranked(&pc_db), 0);
        pc_db.execute("UPDATE fighters SET power = 10 WHERE rowid = (SELECT MIN(rowid) FROM fighters)", []).unwrap();
        mac_db.execute("UPDATE fighters SET tier = 'B', pal_rank = 98, rank_of = 357, power = 65", []).unwrap();
        mac.sync(&mac_db, &none, "macOS", "0.1.0", 3_000);
        pc.sync(&pc_db, &none, "Windows", "0.1.0", 4_000);
        assert_eq!(ranked(&pc_db), ranked(&mac_db), "every side of every call has its rank");
        let kept: i64 =
            pc_db.query_row("SELECT power FROM fighters WHERE rowid = (SELECT MIN(rowid) FROM fighters)", [], |r| r.get(0)).unwrap();
        assert_eq!(kept, 10, "a value the PC already had stays");
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn a_new_computer_doesnt_overwrite_with_its_defaults() {
        let folder = temp("new");
        let conn = db::open_memory().unwrap();
        let mut mac = computer("Mac", &folder);
        mac.note_local(&flatten(&json!({"default_voice": "Yoda"}), &[]), 1_000);
        mac.sync(&conn, &flatten(&json!({"default_voice": "Gollum"}), &[]), "macOS", "0.1.0", 2_000);
        let mut pc = computer("PC", &folder);
        let got = pc.sync(&conn, &flatten(&json!({"default_voice": "Yoda"}), &[]), "Windows", "0.1.0", 3_000);
        assert_eq!(got.settings["default_voice"], "Gollum", "the Mac's change beats the PC's untouched default");
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn a_leftover_copy_of_a_computers_file_isnt_another_computer() {
        let folder = temp("copy");
        let (mac_db, pc_db) = (crate::seed::store(), db::open_memory().unwrap());
        let (mut mac, mut pc) = (computer("Mac", &folder), computer("PC", &folder));
        let none = BTreeMap::new();
        mac.sync(&mac_db, &none, "macOS", "0.1.0", 1_000);
        let dir = mac.dir().unwrap();
        let copy = dir.join("devices/dev-Mac-Old Name.json");
        std::fs::copy(dir.join("devices/dev-Mac.json"), &copy).unwrap();
        mac.sync(&mac_db, &none, "macOS", "0.1.0", 2_000);
        let got = pc.sync(&pc_db, &none, "Windows", "0.1.0", 3_000);
        assert_eq!(got.others.len(), 1);
        assert_eq!(got.others[0].at, 2_000, "the newest of the two");
        assert_eq!(pc.history[0].from, "Mac", "not \"2 computers\"");
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn the_stamp_changes_with_the_other_computers_files_only() {
        let folder = temp("stamp");
        let conn = crate::seed::store();
        let (mut mac, mut pc) = (computer("Mac", &folder), computer("PC", &folder));
        let none = BTreeMap::new();
        mac.sync(&conn, &none, "macOS", "0.1.0", 1_000);
        let dir = mac.dir().unwrap();
        let before = others_stamp(&dir, &pc.device);
        pc.sync(&db::open_memory().unwrap(), &none, "Windows", "0.1.0", 2_000);
        assert_eq!(others_stamp(&dir, &pc.device), before, "its own files don't count");
        std::fs::write(dir.join("calls/dev-Mac.jsonl"), "").unwrap();
        assert_ne!(others_stamp(&dir, &pc.device), before, "the Mac's calls file changed");
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn a_missing_folder_says_so() {
        let conn = db::open_memory().unwrap();
        let mut st = computer("Mac", Path::new("/nonexistent/augur"));
        st.sync(&conn, &BTreeMap::new(), "macOS", "0.1.0", 1);
        assert!(st.error.unwrap().contains("isn't there"));
    }

    #[test]
    fn finds_synced_folders() {
        let home = temp("home");
        std::fs::create_dir_all(home.join("Library/CloudStorage/OneDrive-Personal")).unwrap();
        std::fs::create_dir_all(home.join("Dropbox")).unwrap();
        let found = synced_folders(&home, &|_| None, false);
        assert_eq!(found.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["onedrive", "dropbox"]);
        let _ = std::fs::remove_dir_all(home);
    }
}
