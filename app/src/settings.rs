//! The app's settings, kept as JSON next to the database, and where everything lives.

use augur_engine::model::Settings as ModelSettings;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// ~/Library/Application Support/Augur on macOS, %APPDATA%\Augur on Windows. The app and the command line share it.
pub fn data_dir() -> PathBuf {
    std::env::var_os("AUGUR_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("Augur"))
}

pub fn db_path(dir: &Path) -> PathBuf {
    dir.join("augur.db")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// seconds between checks while a duel is up (the site's own page checks every 2s)
    pub poll_secs: u64,
    /// show the pick next to the menu bar icon (macOS) / as the tray ring (Windows)
    pub tray_pick: bool,
    pub notify_new_pick: bool,
    pub notify_result: bool,
    pub launch_at_login: bool,
    /// voices shown, in order: Yoda, Gollum, Dr. Rockso, Scooby-Doo, Daffy Duck
    pub voices: Vec<String>,
    pub default_voice: String,
    pub include_chance: bool,
    /// your Kick name, for your pal box and rivals (public, no sign-in needed)
    pub kick_name: String,
    /// the stream's IV reroll command and its cost, once known
    pub reroll_command: String,
    pub reroll_cost: Option<u64>,
    /// fetch past duels and tapes in the background (never while a duel is up)
    pub backfill: bool,
    pub model: ModelSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            poll_secs: 5,
            tray_pick: true,
            notify_new_pick: false,
            notify_result: true,
            launch_at_login: false,
            voices: augur_engine::voices::VOICES.iter().map(|v| v.to_string()).collect(),
            default_voice: "Yoda".into(),
            include_chance: true,
            kick_name: String::new(),
            reroll_command: String::new(),
            reroll_cost: None,
            backfill: true,
            model: ModelSettings::default(),
        }
    }
}

impl AppSettings {
    pub fn load(dir: &Path) -> Self {
        std::fs::read_to_string(dir.join("settings.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("settings.json"), serde_json::to_string_pretty(self).unwrap()).map_err(|e| e.to_string())
    }
}
