//! A small log in the data folder (`augur.log`): what Augur's live loop was doing when something went wrong or took
//! too long, so a stall can be told apart afterwards from a quiet site or a closed app. Kept to about 1 MB: the older
//! half moves to `augur.log.1`.

use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::views::civil_date;

/// The log moves to `augur.log.1` once it's this big.
const MAX_BYTES: u64 = 512 * 1024;

/// Adds a line, stamped with the time (UTC).
pub fn line(dir: &Path, what: &str) {
    let path = dir.join("augur.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(&path, dir.join("augur.log.1"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{} {what}", stamp(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)));
    }
}

/// "2026-11-27 17:11:05Z"
fn stamp(secs: u64) -> String {
    let t = secs % 86_400;
    format!("{} {:02}:{:02}:{:02}Z", civil_date(secs), t / 3600, t % 3600 / 60, t % 60)
}

#[cfg(test)]
mod tests {
    #[test]
    fn lines_are_stamped_and_the_log_rolls_over() {
        assert_eq!(super::stamp(1_795_799_465), "2026-11-27 17:11:05Z");
        let dir = std::env::temp_dir().join(format!("augur-journal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        super::line(&dir, "first");
        super::line(&dir, "second");
        let log = std::fs::read_to_string(dir.join("augur.log")).unwrap();
        assert!(log.lines().count() == 2 && log.ends_with("second\n"), "{log}");
        std::fs::write(dir.join("augur.log"), vec![b'x'; super::MAX_BYTES as usize + 1]).unwrap();
        super::line(&dir, "after");
        assert_eq!(std::fs::read_to_string(dir.join("augur.log")).unwrap().lines().count(), 1, "a new log once it's full");
        assert!(dir.join("augur.log.1").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
