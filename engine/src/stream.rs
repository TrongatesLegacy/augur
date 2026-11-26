//! Reading the stats panel off a frame of the stream. palduels.com hides a side's stats until the bell, but the
//! stream's overlay shows both sides' (HP, Attack and Defence at level 70) while picks are still open. The app grabs
//! the panel's part of a frame from Kick's player (`PANEL`, scaled to 1080p), and `read` turns it into numbers.
//!
//! The overlay has a fixed layout and font, so this reads it the simple way: each number sits in a known spot,
//! its digits are split apart and matched against stored digit shapes (`DIGITS`, averaged from the stream's own
//! frames). A read is only trusted when the side the site shows reads exactly as the site has it (`check`), which
//! also proves the frame is of the duel being called.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::db::Result;

/// The panel's part of a 1920×1080 frame: x, y, width, height. Frames of other sizes are scaled to 1080p first.
pub const PANEL: (u32, u32, u32, u32) = (430, 305, 720, 155);

/// The part of a frame at `PANEL`, as RGBA (what a canvas gives).
#[derive(Clone, Debug, Default)]
pub struct Crop {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

/// What the panel showed: each side's HP, Attack and Defence (None where a number didn't read).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Panel {
    pub home: [Option<u32>; 3],
    pub away: [Option<u32>; 3],
}

impl Panel {
    pub fn side(&self, key: &str) -> [Option<u32>; 3] {
        if key == "home" {
            self.home
        } else {
            self.away
        }
    }
}

/// The panel in a frame, or None when it isn't on screen (the stream shows the game in between).
pub fn read(c: &Crop) -> Option<Panel> {
    if c.w != PANEL.2 as usize || c.h != PANEL.3 as usize || c.rgba.len() != c.w * c.h * 4 {
        return None;
    }
    if !ROWS.iter().all(|&y| label_shows(c, y)) {
        return None;
    }
    let lum: Vec<f64> = c.rgba.as_chunks::<4>().0.iter().map(|p| (p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.0).collect();
    let num = |x0: usize, x1: usize, y: usize, from_right: bool| number(&lum, c.w, x0, x1, y, from_right);
    Some(Panel { home: ROWS.map(|y| num(HOME_X.0, HOME_X.1, y, true)), away: ROWS.map(|y| num(AWAY_X.0, AWAY_X.1, y, false)) })
}

/// What a read says about the hidden side.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "is")]
pub enum Check {
    /// the panel wasn't on screen
    NoPanel,
    /// the side the site shows read differently from the site: another duel's panel, or a misread
    ShownDiffers { read: [Option<u32>; 3], site: [u32; 3] },
    /// the shown side agreed, but some of the hidden side's numbers didn't read
    Unreadable { read: [Option<u32>; 3] },
    /// the hidden side's HP, Attack and Defence
    Read { stats: [u32; 3] },
}

/// Checks a read against the side the site shows (`shown`: its key and its HP, Attack and Defence) and returns the
/// hidden side's numbers when it agrees.
pub fn check(panel: Option<&Panel>, shown: (&str, [u32; 3])) -> Check {
    let Some(p) = panel else { return Check::NoPanel };
    let (key, site) = shown;
    let read = p.side(key);
    if read != site.map(Some) {
        return Check::ShownDiffers { read, site };
    }
    let hidden = p.side(if key == "home" { "away" } else { "home" });
    match hidden {
        [Some(hp), Some(atk), Some(def)] => Check::Read { stats: [hp, atk, def] },
        _ => Check::Unreadable { read: hidden },
    }
}

/// What a read means for the hidden side's stats, in the call's words ("6270/464/797").
pub fn stats_text(s: [u32; 3]) -> String {
    format!("{}/{}/{}", s[0], s[1], s[2])
}

// --- the record ---

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS stream_reads (  -- the stream read for a call with hidden stats: what it found, and the frame
    prediction_id INTEGER NOT NULL REFERENCES predictions (id) ON DELETE CASCADE,
    at INTEGER,                        -- site clock (ms)
    outcome TEXT,                      -- read: the hidden side's stats came from the stream; missed: picks closed
                                       -- without a read; agreed / differed: the read against the site's at the bell
    side TEXT,                         -- the hidden side: home or away
    stats TEXT,                        -- what the stream showed for it: "6270/464/797"
    detail TEXT,                       -- in words: why it missed, or what differed
    latency REAL,                      -- how far the player was behind live (seconds)
    height INTEGER,                    -- the frame's height (1080 for 1080p), as the player sent it
    shot TEXT                          -- the frame, a file in the data folder's stream folder
);
CREATE INDEX IF NOT EXISTS stream_reads_call ON stream_reads (prediction_id);
CREATE TABLE IF NOT EXISTS stream_frames (  -- the frames the stream reads kept, in the store so a backup has them
    name TEXT PRIMARY KEY,             -- the read's shot: "<at>-<outcome>.jpg", the same on every computer
    jpeg BLOB NOT NULL
);
"#;

pub const READ: &str = "read";
pub const MISSED: &str = "missed";
pub const AGREED: &str = "agreed";
pub const DIFFERED: &str = "differed";

/// One line of a call's stream record.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamRead {
    pub at: u64,
    pub outcome: String,
    pub side: String,
    pub stats: String,
    pub detail: String,
    pub latency: Option<f64>,
    pub height: Option<u32>,
    pub shot: Option<String>,
}

pub fn record(conn: &Connection, call_id: i64, r: &StreamRead) -> Result<()> {
    conn.execute(
        "INSERT INTO stream_reads (prediction_id, at, outcome, side, stats, detail, latency, height, shot) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![call_id, r.at as i64, r.outcome, r.side, r.stats, r.detail, r.latency, r.height, r.shot],
    )?;
    Ok(())
}

pub fn of_call(conn: &Connection, call_id: i64) -> Result<Vec<StreamRead>> {
    let mut st = conn.prepare_cached(
        "SELECT at, outcome, side, stats, detail, latency, height, shot FROM stream_reads WHERE prediction_id = ? ORDER BY at, rowid",
    )?;
    let rows = st.query_map([call_id], |r| {
        let text = |i: usize| r.get::<_, Option<String>>(i).map(Option::unwrap_or_default);
        Ok(StreamRead {
            at: r.get::<_, Option<i64>>(0)?.unwrap_or(0).max(0) as u64,
            outcome: text(1)?,
            side: text(2)?,
            stats: text(3)?,
            detail: text(4)?,
            latency: r.get(5)?,
            height: r.get(6)?,
            shot: r.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// How many of the latest calls that needed the stream in a row it couldn't read (newest first, stopping at one it
/// read). Live warns from `MISSES_TO_WARN`.
pub fn misses_in_a_row(conn: &Connection) -> Result<u32> {
    let mut st = conn.prepare_cached(
        "SELECT prediction_id, MAX(outcome = 'read'), MAX(outcome = 'missed') FROM stream_reads
         GROUP BY prediction_id ORDER BY prediction_id DESC LIMIT 20",
    )?;
    let rows: Vec<(bool, bool)> =
        st.query_map([], |r| Ok((r.get::<_, i64>(1)? == 1, r.get::<_, i64>(2)? == 1)))?.collect::<rusqlite::Result<_>>()?;
    Ok(rows.iter().filter(|(read, missed)| *read || *missed).take_while(|(read, _)| !read).count() as u32)
}

/// The fights in a row the stream couldn't be read for before Live says so.
pub const MISSES_TO_WARN: u32 = 3;

/// Adds another computer's (or a backup's) reads for a call here: the ones it hasn't got (the same moment and
/// outcome). Whether it gained any.
pub fn merge(conn: &Connection, call_id: i64, incoming: &[StreamRead]) -> Result<bool> {
    let here = of_call(conn, call_id)?;
    let mut gained = false;
    for r in incoming.iter().filter(|r| !here.iter().any(|h| h.at == r.at && h.outcome == r.outcome)) {
        record(conn, call_id, r)?;
        gained = true;
    }
    Ok(gained)
}

/// A frame's name: the moment it was kept and what it was kept for, so it's the same on every computer.
pub fn frame_name(at: u64, outcome: &str) -> String {
    format!("{at}-{outcome}.jpg")
}

/// Keeps a frame (a JPEG) under its name; one already kept stays as it is.
pub fn save_frame(conn: &Connection, name: &str, jpeg: &[u8]) -> Result<()> {
    conn.execute("INSERT OR IGNORE INTO stream_frames (name, jpeg) VALUES (?, ?)", params![name, jpeg])?;
    Ok(())
}

pub fn frame(conn: &Connection, name: &str) -> Result<Option<Vec<u8>>> {
    Ok(conn.query_row("SELECT jpeg FROM stream_frames WHERE name = ?", [name], |r| r.get(0)).optional()?)
}

/// Every kept frame's name.
pub fn frame_names(conn: &Connection) -> Result<Vec<String>> {
    let mut st = conn.prepare_cached("SELECT name FROM stream_frames ORDER BY name")?;
    let rows = st.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The frames reads here name that aren't kept here yet (their reads came from another computer).
pub fn frames_missing(conn: &Connection) -> Result<Vec<String>> {
    let mut st = conn.prepare_cached(
        "SELECT DISTINCT shot FROM stream_reads WHERE COALESCE(shot, '') != '' AND shot NOT IN (SELECT name FROM stream_frames)",
    )?;
    let rows = st.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// A frame name that's safe to use as a file name in the sync folder.
pub fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 100
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        && !name.contains("..")
}

// --- the layout, in the panel's own pixels (1080p) ---

/// The middle of each stat row: HP, Attack, Defence.
const ROWS: [usize; 3] = [23, 78, 133];
/// Where the home side's numbers end (right-aligned) and the away side's start (left-aligned).
const HOME_X: (usize, usize) = (0, 162);
const AWAY_X: (usize, usize) = (560, 720);
/// The middle column's lavender labels (HP, ATTACK, DEFENCE), which say the panel is up.
const LABEL_X: (usize, usize) = (270, 450);

fn label_shows(c: &Crop, y: usize) -> bool {
    let (mut hits, mut all) = (0usize, 0usize);
    for yy in y.saturating_sub(14)..(y + 14).min(c.h) {
        for x in LABEL_X.0..LABEL_X.1 {
            let i = (yy * c.w + x) * 4;
            let (r, g, b) = (c.rgba[i] as i32, c.rgba[i + 1] as i32, c.rgba[i + 2] as i32);
            all += 1;
            if b > 170 && r > 130 && g < 170 && b - g > 40 {
                hits += 1;
            }
        }
    }
    hits as f64 / all.max(1) as f64 > 0.08
}

// --- digits ---

/// Rows above and below a stat row's middle that its numbers take.
const ABOVE: usize = 15;
const BELOW: usize = 12;
/// A wider gap than this ends the number (the arrow and the species base come next).
const GAP: usize = 8;
/// A digit's shape is compared at this size.
const GW: usize = 10;
const GH: usize = 16;

/// The number at a stat row, read from its aligned edge.
fn number(lum: &[f64], w: usize, x0: usize, x1: usize, y: usize, from_right: bool) -> Option<u32> {
    let (top, rows) = (y - ABOVE, ABOVE + BELOW);
    let cols = x1 - x0;
    let px = |r: usize, c: usize| lum[(top + r) * w + x0 + c];
    let mut vals: Vec<f64> = Vec::with_capacity(rows * cols);
    for r in 0..rows {
        for c in 0..cols {
            vals.push(px(r, c));
        }
    }
    let th = otsu(&vals).max(80.0);
    let on = |r: usize, c: usize| px(r, c) > th;
    let count: Vec<usize> = (0..cols).map(|c| (0..rows).filter(|&r| on(r, c)).count()).collect();
    // runs of lit columns, each with the rows it spans
    let mut runs = vec![];
    let mut start = None;
    for c in 0..=cols {
        let lit = count.get(c).is_some_and(|n| *n > 0);
        match (lit, start) {
            (true, None) => start = Some(c),
            (false, Some(s)) => {
                let lit_rows: Vec<usize> = (0..rows).filter(|&r| (s..c).any(|cc| on(r, cc))).collect();
                runs.push((s, c, lit_rows[0], *lit_rows.last().unwrap()));
                start = None;
            }
            _ => {}
        }
    }
    if from_right {
        runs.reverse();
    }
    // digits (full height) and commas (low and narrow) from the aligned edge, until something else or a wide gap
    let mut kept: Vec<(bool, usize, usize, usize, usize)> = vec![];
    let mut prev: Option<(usize, usize)> = None;
    for (a, b, t, btm) in runs {
        let started = kept.iter().any(|k| k.0);
        if started {
            if let Some((pa, pb)) = prev {
                let gap = if from_right { pa.saturating_sub(b) } else { a.saturating_sub(pb) };
                if gap > GAP {
                    break;
                }
            }
        }
        prev = Some((a, b));
        let h = btm - t + 1;
        if h >= 15 {
            kept.push((true, a, b, t, btm));
        } else if started && h <= 10 && t >= 12 && b - a <= 8 {
            kept.push((false, a, b, t, btm));
        } else if started {
            break;
        }
    }
    while kept.last().is_some_and(|k| !k.0) {
        kept.pop();
    }
    if from_right {
        kept.reverse();
    }
    let mut digits = String::new();
    for &(digit, a, b, t, btm) in &kept {
        if !digit {
            continue;
        }
        // bold digits can touch: split a run as wide as several digits where it's thinnest
        let h = btm - t + 1;
        let n = (((b - a + 3) as f64) / (h as f64 * 0.83)).round().max(1.0) as usize;
        let mut cuts = vec![a];
        for j in 1..n {
            let c = a + (b - a) * j / n;
            let (lo, hi) = ((a + 4).max(c.saturating_sub(4)), (b.saturating_sub(4)).min(c + 5));
            cuts.push(if hi > lo { (lo..hi).min_by_key(|&x| count[x]).unwrap_or(c) } else { c });
        }
        cuts.push(b);
        for pair in cuts.windows(2) {
            let (p, q) = (pair[0], pair[1]);
            let lit: Vec<usize> = (p..q).filter(|&x| (t..=btm).any(|r| on(r, x))).collect();
            let (Some(&l), Some(&rr)) = (lit.first(), lit.last()) else { continue };
            let glyph: Vec<Vec<bool>> = (t..=btm).map(|r| (l..=rr).map(|x| on(r, x)).collect()).collect();
            digits.push(classify(&glyph));
        }
    }
    if digits.is_empty() || digits.len() > 5 {
        return None;
    }
    digits.parse().ok()
}

/// Otsu's threshold over 64 bins: the split between the panel's dark background and its lit digits.
fn otsu(vals: &[f64]) -> f64 {
    let mut hist = [0f64; 64];
    for v in vals {
        hist[((*v / 4.0) as usize).min(63)] += 1.0;
    }
    let total: f64 = hist.iter().sum();
    let p: Vec<f64> = hist.iter().map(|h| h / total.max(1.0)).collect();
    let (mut best, mut at) = (0.0, 32);
    for t in 1..64 {
        let w0: f64 = p[..t].iter().sum();
        let w1 = 1.0 - w0;
        if w0 <= 0.0 || w1 <= 0.0 {
            continue;
        }
        let m0 = p[..t].iter().enumerate().map(|(i, x)| i as f64 * x).sum::<f64>() / w0;
        let m1 = p[t..].iter().enumerate().map(|(i, x)| (i + t) as f64 * x).sum::<f64>() / w1;
        let s = w0 * w1 * (m0 - m1).powi(2);
        if s > best {
            (best, at) = (s, t);
        }
    }
    at as f64 * 4.0
}

/// The digit a glyph looks most like.
fn classify(glyph: &[Vec<bool>]) -> char {
    let (h, w) = (glyph.len(), glyph[0].len());
    let aspect = w as f64 / h as f64;
    let shape = shrink(glyph);
    let score = |(t_aspect, t): &(f64, [u8; GW * GH])| {
        shape.iter().zip(t.iter()).map(|(a, b)| (a - *b as f64 / 255.0).abs()).sum::<f64>() + 3.0 * (aspect - t_aspect).abs()
    };
    let best = DIGITS.iter().enumerate().min_by(|a, b| score(a.1).total_cmp(&score(b.1))).map(|(i, _)| i).unwrap_or(0);
    char::from(b'0' + best as u8)
}

/// A glyph averaged down to GW × GH, each cell the share of it that's lit.
fn shrink(glyph: &[Vec<bool>]) -> [f64; GW * GH] {
    let (h, w) = (glyph.len() as f64, glyph[0].len() as f64);
    let mut out = [0.0; GW * GH];
    for r in 0..GH {
        for c in 0..GW {
            let (y0, y1) = (r as f64 * h / GH as f64, (r + 1) as f64 * h / GH as f64);
            let (x0, x1) = (c as f64 * w / GW as f64, (c + 1) as f64 * w / GW as f64);
            let (mut lit, mut area) = (0.0, 0.0);
            for (y, row) in glyph.iter().enumerate().take(y1.ceil() as usize).skip(y0 as usize) {
                let wy = (y as f64 + 1.0).min(y1) - (y as f64).max(y0);
                for (x, lit_px) in row.iter().enumerate().take(x1.ceil() as usize).skip(x0 as usize) {
                    let wx = (x as f64 + 1.0).min(x1) - (x as f64).max(x0);
                    if *lit_px {
                        lit += wy * wx;
                    }
                    area += wy * wx;
                }
            }
            out[r * GW + c] = lit / area;
        }
    }
    out
}

/// Each digit's width over height and shape (GW × GH, 0-255), averaged from 647 digits in 32 of the stream's frames
/// at 1080p, all read right whichever frame was left out. 0 to 9.
#[rustfmt::skip]
const DIGITS: [(f64, [u8; GW * GH]); 10] = [
    // 0
    (0.757, [0, 5, 60, 186, 246, 253, 214, 117, 15, 0, 0, 71, 220, 254, 255, 255, 255, 241, 118, 2, 29, 204, 255, 255, 255, 255, 255, 255, 232, 59, 97, 244, 255, 255, 232, 224, 255, 255, 254, 133, 176, 254, 255, 254, 145, 126, 253, 255, 255, 206, 211, 255, 255, 249, 67, 65, 244, 255, 255, 239, 235, 255, 255, 247, 57, 37, 242, 255, 255, 255, 246, 255, 255, 247, 57, 35, 241, 255, 255, 255, 234, 255, 255, 247, 56, 35, 240, 255, 255, 255, 236, 255, 255, 247, 57, 49, 245, 255, 255, 249, 220, 255, 255, 252, 72, 84, 250, 255, 255, 220, 195, 255, 255, 255, 178, 168, 255, 255, 255, 160, 116, 248, 255, 255, 255, 255, 255, 255, 248, 92, 35, 212, 255, 255, 255, 255, 255, 255, 193, 19, 0, 80, 216, 255, 255, 255, 255, 222, 57, 0, 0, 6, 57, 200, 254, 253, 185, 56, 4, 0]),
    // 1
    (0.523, [0, 0, 2, 23, 46, 139, 227, 244, 246, 186, 40, 80, 106, 177, 219, 253, 255, 255, 255, 241, 203, 246, 249, 254, 255, 255, 255, 255, 255, 238, 249, 255, 255, 255, 255, 255, 255, 255, 255, 242, 236, 255, 254, 229, 235, 255, 255, 255, 255, 238, 73, 101, 83, 106, 213, 255, 255, 255, 255, 238, 0, 0, 0, 62, 209, 255, 255, 255, 255, 239, 0, 0, 0, 67, 210, 255, 255, 255, 255, 237, 0, 0, 0, 67, 210, 255, 255, 255, 255, 235, 0, 0, 0, 67, 210, 255, 255, 255, 255, 236, 0, 0, 0, 65, 209, 255, 255, 255, 255, 235, 0, 0, 0, 65, 209, 255, 255, 255, 255, 237, 0, 0, 0, 65, 209, 255, 255, 255, 255, 235, 0, 0, 0, 65, 209, 255, 255, 255, 255, 236, 0, 0, 0, 67, 210, 255, 255, 255, 255, 238, 0, 0, 0, 20, 159, 245, 255, 255, 251, 167]),
    // 2
    (0.706, [10, 45, 161, 229, 250, 250, 220, 136, 31, 1, 55, 206, 249, 255, 255, 255, 255, 244, 170, 17, 68, 238, 255, 255, 255, 255, 255, 255, 245, 93, 69, 239, 224, 202, 205, 248, 255, 255, 255, 154, 40, 151, 98, 26, 39, 198, 255, 255, 255, 192, 10, 34, 1, 0, 5, 184, 255, 255, 255, 176, 0, 0, 0, 0, 85, 238, 255, 255, 255, 139, 0, 0, 0, 69, 226, 255, 255, 255, 233, 70, 0, 0, 79, 230, 255, 255, 255, 253, 140, 6, 1, 62, 230, 255, 255, 255, 241, 146, 10, 0, 45, 197, 254, 255, 255, 244, 107, 9, 0, 0, 133, 252, 255, 255, 255, 152, 66, 64, 60, 29, 200, 254, 255, 255, 255, 247, 242, 244, 240, 190, 243, 255, 255, 255, 255, 255, 255, 255, 255, 250, 248, 255, 255, 255, 255, 255, 255, 255, 255, 253, 171, 253, 255, 255, 255, 255, 255, 255, 254, 168]),
    // 3
    (0.667, [37, 166, 229, 251, 251, 247, 197, 110, 29, 0, 93, 250, 255, 255, 255, 255, 255, 247, 175, 29, 94, 253, 255, 255, 255, 255, 255, 255, 250, 83, 48, 174, 193, 193, 213, 254, 255, 255, 255, 142, 4, 14, 15, 15, 67, 234, 255, 255, 255, 147, 0, 5, 31, 31, 126, 251, 255, 255, 247, 86, 0, 67, 232, 233, 250, 255, 255, 254, 175, 30, 0, 92, 255, 255, 255, 255, 255, 249, 65, 0, 0, 92, 255, 255, 255, 255, 255, 255, 200, 37, 0, 55, 158, 160, 206, 255, 255, 255, 255, 153, 0, 0, 6, 7, 49, 208, 255, 255, 255, 247, 27, 61, 62, 62, 91, 217, 255, 255, 255, 251, 189, 240, 241, 241, 254, 255, 255, 255, 255, 197, 251, 255, 255, 255, 255, 255, 255, 255, 243, 92, 216, 255, 255, 255, 255, 255, 255, 237, 114, 9, 77, 208, 254, 255, 255, 251, 167, 69, 6, 0]),
    // 4
    (0.821, [0, 0, 0, 0, 66, 222, 240, 218, 50, 0, 0, 0, 0, 3, 169, 255, 255, 250, 84, 0, 0, 0, 0, 59, 239, 255, 255, 250, 84, 0, 0, 0, 2, 162, 255, 255, 255, 250, 84, 0, 0, 0, 46, 229, 255, 255, 255, 250, 84, 0, 0, 3, 149, 255, 255, 255, 255, 250, 84, 0, 0, 54, 236, 255, 238, 250, 255, 249, 82, 0, 5, 174, 255, 254, 177, 237, 255, 249, 82, 0, 94, 249, 255, 235, 74, 227, 255, 250, 88, 0, 209, 255, 255, 221, 121, 249, 255, 254, 166, 92, 246, 255, 255, 255, 255, 255, 255, 255, 255, 252, 222, 255, 255, 255, 255, 255, 255, 255, 255, 244, 77, 115, 115, 115, 126, 239, 255, 252, 160, 99, 0, 0, 0, 0, 22, 224, 255, 250, 84, 0, 0, 0, 0, 0, 23, 229, 255, 250, 84, 0, 0, 0, 0, 0, 9, 175, 255, 236, 44, 0]),
    // 5
    (0.658, [24, 150, 241, 239, 238, 239, 237, 235, 194, 63, 52, 228, 255, 255, 255, 255, 255, 255, 246, 94, 57, 233, 255, 255, 255, 255, 255, 255, 245, 91, 66, 238, 255, 255, 238, 213, 213, 213, 199, 54, 82, 247, 255, 255, 170, 26, 24, 24, 22, 5, 85, 249, 255, 255, 230, 167, 165, 130, 16, 2, 91, 255, 255, 255, 255, 255, 255, 248, 175, 32, 100, 255, 255, 255, 255, 255, 255, 255, 252, 138, 124, 255, 255, 255, 255, 255, 255, 255, 255, 222, 86, 161, 162, 165, 195, 253, 255, 255, 255, 250, 0, 2, 2, 3, 38, 200, 255, 255, 255, 248, 29, 55, 55, 57, 74, 220, 255, 255, 255, 221, 194, 244, 243, 246, 254, 255, 255, 255, 255, 152, 251, 255, 255, 255, 255, 255, 255, 255, 211, 46, 228, 255, 255, 255, 255, 255, 255, 200, 60, 0, 83, 198, 254, 255, 255, 238, 121, 36, 0, 0]),
    // 6
    (0.737, [0, 0, 14, 74, 181, 238, 246, 235, 155, 10, 0, 9, 129, 229, 255, 255, 255, 255, 210, 16, 1, 98, 243, 255, 255, 255, 255, 255, 211, 17, 22, 206, 255, 255, 255, 224, 202, 202, 164, 10, 86, 243, 255, 255, 220, 80, 19, 19, 15, 0, 161, 255, 255, 253, 180, 178, 180, 180, 57, 1, 214, 255, 255, 255, 252, 255, 255, 255, 209, 40, 240, 255, 255, 255, 255, 255, 255, 255, 255, 146, 250, 255, 255, 255, 255, 255, 255, 255, 255, 221, 242, 255, 255, 255, 223, 211, 254, 255, 255, 253, 213, 255, 255, 249, 112, 82, 237, 255, 255, 250, 173, 255, 255, 253, 150, 120, 243, 255, 255, 218, 102, 248, 255, 255, 255, 253, 255, 255, 255, 145, 19, 190, 255, 255, 255, 255, 255, 255, 217, 47, 0, 58, 214, 255, 255, 255, 255, 226, 81, 0, 0, 3, 50, 182, 253, 254, 198, 62, 5, 0]),
    // 7
    (0.737, [183, 239, 245, 243, 241, 241, 243, 243, 239, 179, 249, 255, 255, 255, 255, 255, 255, 255, 255, 248, 244, 255, 255, 255, 255, 255, 255, 255, 255, 233, 169, 206, 206, 206, 206, 236, 255, 255, 254, 170, 16, 21, 21, 21, 39, 203, 255, 255, 234, 68, 0, 0, 0, 0, 84, 249, 255, 255, 174, 12, 0, 0, 0, 12, 185, 255, 255, 254, 98, 0, 0, 0, 0, 79, 244, 255, 255, 219, 38, 0, 0, 0, 4, 171, 255, 255, 254, 172, 11, 0, 2, 0, 30, 237, 255, 255, 251, 88, 0, 0, 3, 0, 102, 248, 255, 255, 245, 46, 0, 0, 1, 6, 155, 254, 255, 255, 213, 10, 0, 0, 0, 21, 196, 255, 255, 255, 163, 4, 0, 0, 0, 38, 226, 255, 255, 255, 124, 2, 0, 0, 0, 60, 243, 255, 255, 247, 78, 0, 0, 0, 0, 47, 227, 255, 255, 209, 26, 0, 0, 0]),
    // 8
    (0.747, [0, 20, 103, 203, 246, 250, 220, 139, 36, 0, 14, 145, 238, 255, 255, 255, 255, 247, 190, 35, 76, 239, 255, 255, 248, 245, 254, 255, 253, 130, 151, 254, 255, 253, 151, 104, 243, 255, 255, 179, 134, 253, 255, 249, 83, 38, 238, 255, 255, 166, 56, 224, 255, 254, 145, 100, 249, 255, 238, 87, 5, 126, 254, 255, 248, 242, 255, 255, 156, 7, 0, 127, 255, 255, 255, 255, 255, 255, 151, 4, 80, 240, 255, 255, 255, 255, 255, 255, 244, 87, 191, 255, 255, 255, 225, 204, 251, 255, 255, 194, 238, 255, 255, 244, 90, 49, 221, 255, 255, 251, 243, 255, 255, 239, 38, 14, 204, 255, 255, 250, 202, 255, 255, 251, 143, 106, 240, 255, 255, 196, 91, 241, 255, 255, 250, 248, 255, 255, 247, 97, 5, 129, 242, 255, 255, 255, 255, 242, 142, 8, 0, 13, 89, 215, 254, 255, 220, 85, 14, 0]),
    // 9
    (0.746, [0, 21, 113, 206, 248, 248, 202, 80, 9, 0, 13, 132, 242, 255, 255, 255, 255, 235, 91, 1, 72, 235, 255, 255, 255, 255, 255, 255, 211, 31, 172, 255, 255, 253, 215, 223, 255, 255, 251, 108, 231, 255, 255, 245, 92, 116, 250, 255, 255, 184, 247, 255, 255, 244, 50, 73, 248, 255, 255, 237, 246, 255, 255, 252, 146, 169, 254, 255, 255, 249, 206, 255, 255, 255, 255, 255, 255, 255, 255, 252, 102, 248, 255, 255, 255, 255, 255, 255, 255, 246, 20, 157, 252, 255, 255, 255, 255, 255, 255, 226, 0, 21, 114, 194, 190, 187, 253, 255, 255, 164, 1, 39, 55, 67, 90, 206, 255, 255, 246, 90, 44, 205, 234, 237, 249, 255, 255, 255, 212, 27, 75, 242, 255, 255, 255, 255, 255, 243, 88, 1, 60, 234, 255, 255, 255, 255, 247, 120, 9, 0, 12, 167, 252, 255, 254, 211, 88, 9, 0, 0]),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// A saved panel (a PNG crop of `PANEL` from a frame of the stream: numbers and labels only, no names or art).
    fn crop(name: &str) -> Crop {
        let path = format!("{}/tests/frames/{name}", env!("CARGO_MANIFEST_DIR"));
        let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        let (w, h) = (info.width as usize, info.height as usize);
        let rgba = match info.color_type {
            png::ColorType::Rgba => buf[..w * h * 4].to_vec(),
            png::ColorType::Rgb => buf[..w * h * 3].as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
            other => panic!("{other:?}"),
        };
        Crop { w, h, rgba }
    }

    fn some(v: [u32; 3]) -> [Option<u32>; 3] {
        v.map(Some)
    }

    #[test]
    fn reads_both_sides() {
        // each against the stats the site showed at the bell
        for (name, home, away) in [
            ("panel-451.png", [4770, 612, 767], [4986, 582, 696]),
            ("panel-452.png", [5890, 655, 695], [4807, 513, 776]),
            ("panel-465.png", [6270, 464, 797], [5072, 661, 677]),
            ("panel-466.png", [4986, 576, 706], [5778, 778, 691]),
            ("panel-481.png", [7688, 1044, 1145], [8826, 919, 1318]),
        ] {
            let p = read(&crop(name)).unwrap_or_else(|| panic!("{name}: the panel is up"));
            assert_eq!((p.home, p.away), (some(home), some(away)), "{name}");
        }
    }

    #[test]
    fn reads_a_720p_frame_scaled_up() {
        let p = read(&crop("panel-465-720p.png")).unwrap();
        assert_eq!((p.home, p.away), (some([6270, 464, 797]), some([5072, 661, 677])));
    }

    #[test]
    fn no_panel_when_the_stream_shows_the_game() {
        assert_eq!(read(&crop("panel-472.png")), None);
        assert_eq!(read(&Crop { w: 10, h: 10, rgba: vec![0; 400] }), None, "the wrong size");
    }

    #[test]
    fn a_read_counts_only_when_the_shown_side_agrees() {
        let p = read(&crop("panel-465.png"));
        // the away side was hidden: home is what the site showed
        assert_eq!(check(p.as_ref(), ("home", [6270, 464, 797])), Check::Read { stats: [5072, 661, 677] });
        assert_eq!(check(p.as_ref(), ("away", [5072, 661, 677])), Check::Read { stats: [6270, 464, 797] });
        // another duel's panel (or a misread) says nothing about this one
        assert_eq!(
            check(p.as_ref(), ("home", [6270, 464, 798])),
            Check::ShownDiffers { read: some([6270, 464, 797]), site: [6270, 464, 798] }
        );
        assert_eq!(check(None, ("home", [1, 2, 3])), Check::NoPanel);
        let half = Panel { home: some([6270, 464, 797]), away: [Some(5072), None, Some(677)] };
        assert_eq!(check(Some(&half), ("home", [6270, 464, 797])), Check::Unreadable { read: half.away });
    }

    #[test]
    fn the_record_counts_misses_in_a_row() {
        let conn = crate::db::open_memory().unwrap();
        for id in 1..=5 {
            conn.execute("INSERT INTO predictions (id, status) VALUES (?, 'settled')", [id]).unwrap();
        }
        let line = |outcome: &str| StreamRead { outcome: outcome.into(), side: "away".into(), ..Default::default() };
        record(&conn, 1, &line(MISSED)).unwrap();
        record(&conn, 2, &line(READ)).unwrap();
        record(&conn, 2, &line(AGREED)).unwrap();
        record(&conn, 3, &line(MISSED)).unwrap();
        record(&conn, 4, &line(MISSED)).unwrap();
        assert_eq!(misses_in_a_row(&conn).unwrap(), 2, "back to the last one it read");
        record(&conn, 5, &line(READ)).unwrap();
        assert_eq!(misses_in_a_row(&conn).unwrap(), 0);
        assert_eq!(of_call(&conn, 2).unwrap().iter().map(|r| r.outcome.as_str()).collect::<Vec<_>>(), [READ, AGREED]);
    }

    #[test]
    fn reads_and_frames_merge_without_repeats() {
        let conn = crate::db::open_memory().unwrap();
        conn.execute("INSERT INTO predictions (id, status) VALUES (1, 'settled')", []).unwrap();
        let name = frame_name(1_000, READ);
        let read = StreamRead { at: 1_000, outcome: READ.into(), side: "away".into(), shot: Some(name.clone()), ..Default::default() };
        let agreed = StreamRead { at: 2_000, outcome: AGREED.into(), side: "away".into(), ..Default::default() };
        assert!(merge(&conn, 1, std::slice::from_ref(&read)).unwrap());
        assert!(merge(&conn, 1, &[read.clone(), agreed]).unwrap(), "the bell's check is new");
        assert!(!merge(&conn, 1, &[read]).unwrap(), "nothing new the second time");
        assert_eq!(of_call(&conn, 1).unwrap().len(), 2);
        assert_eq!(frames_missing(&conn).unwrap(), std::slice::from_ref(&name), "the read names a frame not kept here yet");
        save_frame(&conn, &name, b"jpeg").unwrap();
        save_frame(&conn, &name, b"other").unwrap();
        assert_eq!(frame(&conn, &name).unwrap().as_deref(), Some(&b"jpeg"[..]), "kept once");
        assert!(frames_missing(&conn).unwrap().is_empty());
        assert_eq!(frame_names(&conn).unwrap(), std::slice::from_ref(&name));
        assert!(safe_name(&name) && !safe_name("../x.jpg") && !safe_name("a/b.jpg") && !safe_name(""));
    }
}
