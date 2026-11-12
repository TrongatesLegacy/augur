//! The app's settings, kept as JSON next to the database, and where everything lives.

use augur_engine::model::Settings as ModelSettings;
use augur_engine::stake::StakeSettings;
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
    /// when set, each check waits a random time from `poll_secs` up to this many seconds instead
    pub poll_max_secs: Option<u64>,
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
    /// read your picks list (signed in) to track your bets against Augur's calls
    pub import_bets: bool,
    pub staking: StakeSettings,
    pub model: ModelSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            poll_secs: 5,
            poll_max_secs: None,
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
            import_bets: true,
            staking: StakeSettings::default(),
            model: ModelSettings::default(),
        }
    }
}

impl AppSettings {
    /// How long to wait before the next check while a duel is up: `poll_secs`, or a random time in the range.
    /// `roll` is a random number in 0..1.
    pub fn poll_wait(&self, roll: f64) -> std::time::Duration {
        let min = self.poll_secs.max(2) as f64;
        let max = self.poll_max_secs.map_or(min, |m| (m as f64).max(min));
        std::time::Duration::from_millis(((min + (max - min) * roll.clamp(0.0, 1.0)) * 1000.0) as u64)
    }

    pub fn load(dir: &Path) -> Self {
        std::fs::read_to_string(dir.join("settings.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("settings.json"), serde_json::to_string_pretty(self).unwrap()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polling_waits_in_the_range() {
        let fixed = AppSettings { poll_secs: 5, ..Default::default() };
        assert_eq!(fixed.poll_wait(0.9).as_secs(), 5);
        let range = AppSettings { poll_secs: 4, poll_max_secs: Some(10), ..Default::default() };
        assert_eq!(range.poll_wait(0.0).as_secs(), 4);
        assert_eq!(range.poll_wait(0.5).as_secs(), 7);
        assert_eq!(range.poll_wait(1.0).as_secs(), 10);
        let upside_down = AppSettings { poll_secs: 8, poll_max_secs: Some(3), ..Default::default() };
        assert_eq!(upside_down.poll_wait(0.7).as_secs(), 8, "a max under the min is the min");
        assert_eq!(AppSettings { poll_secs: 0, ..Default::default() }.poll_wait(0.0).as_secs(), 2, "never faster than 2s");
    }
}
