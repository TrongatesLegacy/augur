//! Reading the stats panel off a frame of the stream. palduels.com hides a side's stats until the bell, but the
//! stream's overlay shows both sides' (HP, Attack and Defence at level 70) while picks are still open. The app grabs
//! the panel's part of a frame from Kick's player (`PANEL`, scaled to 1080p), and `read` turns it into numbers.
//!
//! The overlay's font stays put but its layout has changed before, so this finds the layout from the middle column's
//! lavender labels (which rows, where the middle is) and reads outward from them to the first white or grey number,
//! ignoring anything coloured. Each number's digits are split apart and matched against stored digit shapes
//! (`FONTS`, averaged from the stream's own frames). A read is only trusted when the side the site shows reads
//! exactly as the site has it (`check`), which also proves the frame is of the duel being called.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::db::Result;

/// The panel's part of a 1920×1080 frame: x, y, width, height. Frames of other sizes are scaled to 1080p first.
/// It's roomier than the panel itself: the rows and the middle column are found from the labels, so a panel that sits
/// a little higher, lower or to one side still reads (it covers the overlay's redesigns of 26 and 27 November 2026).
pub const PANEL: (u32, u32, u32, u32) = (400, 170, 780, 380);

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
    let (rows, half, mid) = label_rows(c)?;
    // the numbers are white or grey: coloured pixels (the markers, IV badges, bars and labels) count as dark
    let lum: Vec<f64> = c
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| {
            let (hi, lo) = (p[0].max(p[1]).max(p[2]) as f64, p[0].min(p[1]).min(p[2]) as f64);
            if hi - lo > MAX_TINT * hi {
                0.0
            } else {
                (p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.0
            }
        })
        .collect();
    let num = |x0: usize, x1: usize, y: usize, from_right: bool| number(&lum, c.w, x0, x1, (y - half, y + half), from_right);
    let (home_end, away_start) = (mid.checked_sub(NUMBERS_FROM_MID)?, mid + NUMBERS_FROM_MID);
    (away_start < c.w).then(|| Panel { home: rows.map(|y| num(0, home_end, y, true)), away: rows.map(|y| num(away_start, c.w, y, false)) })
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

/// What a read says when the site shows both sides: the reader checked against the site.
#[derive(Clone, Debug, PartialEq)]
pub enum BothCheck {
    /// the panel wasn't on screen
    NoPanel,
    /// neither side read as the site has it: another duel's panel (the player runs behind), or not up yet
    Other,
    /// both sides read exactly as the site shows them
    Agreed,
    /// one side agreed (so it's this duel's panel) and the other read differently: a misread
    Misread { home: [Option<u32>; 3], away: [Option<u32>; 3] },
}

/// Checks a read against both sides' stats as the site shows them (when it hides neither), so every fight tests
/// the reader, not only the ones with a hidden side.
pub fn check_both(panel: Option<&Panel>, home: [u32; 3], away: [u32; 3]) -> BothCheck {
    let Some(p) = panel else { return BothCheck::NoPanel };
    match (p.home == home.map(Some), p.away == away.map(Some)) {
        (true, true) => BothCheck::Agreed,
        (false, false) => BothCheck::Other,
        _ => BothCheck::Misread { home: p.home, away: p.away },
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
/// A fight with nothing hidden, read to check the reader: both sides agreed with the site, one didn't, or picks
/// closed without the panel reading (with the last frame either way). None of these count as misses.
pub const CHECKED: &str = "checked";
pub const MISREAD: &str = "misread";
pub const UNCHECKED: &str = "unchecked";

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

/// The middle column's labels (POWER, HP, ATTACK, DEFENCE), which say the panel is up and where its rows and middle
/// are, as each version of the overlay has drawn them: where to look, and their colour. Tried newest first.
const LABEL_STYLES: [LabelStyle; 2] = [
    // from 27 November 2026: white or pale lavender-white, between numbers that come close to the middle (wide enough
    // on the right for DEFENCE with its marker in front of it)
    LabelStyle { x: (315, 462), colour: pale },
    // before: lavender, deep or pale, in a wider strip (the numbers sat further out)
    LabelStyle { x: (280, 494), colour: lavender },
];

struct LabelStyle {
    x: (usize, usize),
    colour: fn(i32, i32, i32) -> bool,
}

/// White or a pale lavender-white: light, with blue at least level with red and green (the text tinted mint, pink or
/// salmon under the labels, "+23%", isn't).
fn pale(r: i32, g: i32, b: i32) -> bool {
    b > 190 && r > 150 && b >= r - 10 && b >= g - 10
}

fn lavender(r: i32, g: i32, b: i32) -> bool {
    b > 190 && r > 150 && b - g > 20 && b >= r
}
/// The home side's numbers end (right-aligned) and the away side's start (left-aligned) at least this far from the
/// middle. The redesigned labels are pale enough to pass for grey and DEFENCE reaches about 60 either side; the older,
/// wider ones are coloured, so reading skips them anyway.
const NUMBERS_FROM_MID: usize = 66;
/// A pixel this far from grey (its brightest channel less its dimmest, as a share of the brightest) is coloured, not
/// part of a number; dim pixels count too, so a coloured label's soft edges don't pass for a digit.
const MAX_TINT: f64 = 0.3;
/// A pixel row is part of a label from this many lavender pixels.
const LABEL_HITS: usize = 4;
/// No label reaches this close to the ends of its strip.
const LABEL_EDGE: usize = 4;
/// A label's height: anything shorter or taller is something else.
const LABEL_H: (usize, usize) = (14, 30);
/// How far apart the stat rows are, give or take, and how much the two gaps may differ.
const ROW_STEP: (usize, usize) = (50, 80);
const ROW_EVEN: usize = 4;
/// A row's numbers are looked for this share of the row spacing above and below the middle of its label (they've sat
/// from 9px above it to 3px below).
const ROW_REACH: f64 = 0.45;

/// The middle of each stat row's label (HP, Attack, Defence), found from the last three labels (POWER above them is
/// optional), how far above and below it to look for its numbers, and the middle of the labels' column, in whichever
/// of the overlay's styles finds them; None when none does.
fn label_rows(c: &Crop) -> Option<([usize; 3], usize, usize)> {
    LABEL_STYLES.iter().find_map(|style| label_rows_in(c, style))
}

fn label_rows_in(c: &Crop, style: &LabelStyle) -> Option<([usize; 3], usize, usize)> {
    let label_x = style.x;
    let rgb = |x: usize, y: usize| {
        let i = (y * c.w + x) * 4;
        (c.rgba[i] as i32, c.rgba[i + 1] as i32, c.rgba[i + 2] as i32)
    };
    let label = |x: usize, y: usize| {
        let (r, g, b) = rgb(x, y);
        (style.colour)(r, g, b)
    };
    // the green or red a badge (the gap between the sides, "+23%") is filled with: its white text isn't a label
    let fill = |x: usize, y: usize| {
        let (r, g, b) = rgb(x, y);
        r.max(g) > 110 && r.max(g) > b + 40
    };
    let lit: Vec<bool> = (0..c.h).map(|y| (label_x.0..label_x.1).filter(|&x| label(x, y)).count() >= LABEL_HITS).collect();
    // runs of lit rows, bridging the odd unlit row inside a word
    let mut bands = vec![];
    let mut at_edge = vec![];
    let mut y = 0;
    while y < c.h {
        if !lit[y] {
            y += 1;
            continue;
        }
        let (start, mut end) = (y, y);
        while y < c.h && (lit[y] || (y + 1 < c.h && lit[y + 1]) || (y + 2 < c.h && lit[y + 2])) {
            if lit[y] {
                end = y;
            }
            y += 1;
        }
        let h = end - start + 1;
        // a label sits in the middle of the strip: text across it, to both edges, is the caption under the panel, and
        // a label at one edge means the panel is off to one side, where its numbers would read as labels too
        let edges = |x: usize| (start..=end).any(|y| label(x, y));
        let (left, right) = ((label_x.0..label_x.0 + LABEL_EDGE).any(edges), (label_x.1 - LABEL_EDGE..label_x.1).any(edges));
        if left != right && (LABEL_H.0..=LABEL_H.1).contains(&h) {
            at_edge.push((start, end));
        }
        let inside = !left && !right;
        let count = |f: &dyn Fn(usize, usize) -> bool| {
            (start..=end).flat_map(|y| (label_x.0..label_x.1).map(move |x| (x, y))).filter(|&(x, y)| f(x, y)).count()
        };
        let on_fill = count(&fill) > count(&label);
        if (LABEL_H.0..=LABEL_H.1).contains(&h) && inside && !on_fill {
            bands.push((start, end));
        }
    }
    // the last three evenly spaced: lines under the panel (a caption, the skills' heading) can pass for labels too
    let middle = |(a, b): (usize, usize)| (a + b) / 2;
    let steps_of = |w: &[(usize, usize)]| [middle(w[1]) - middle(w[0]), middle(w[2]) - middle(w[1])];
    let even = |w: &&[(usize, usize)]| {
        let s = steps_of(w);
        s.iter().all(|d| (ROW_STEP.0..=ROW_STEP.1).contains(d)) && s[0].abs_diff(s[1]) <= ROW_EVEN
    };
    let &[hp, atk, def] = bands.windows(3).rev().find(even)? else { return None };
    let steps = steps_of(&[hp, atk, def]);
    // a label at one edge a row below these (DEFENCE, off to one side) would make them the wrong three
    let next = middle(def) + steps[1];
    if at_edge.iter().any(|&b| middle(b).abs_diff(next) <= ROW_EVEN) {
        return None;
    }
    let rows = [hp, atk, def].map(middle);
    let half = ((steps[0] + steps[1]) as f64 / 2.0 * ROW_REACH) as usize;
    // DEFENCE is the widest label, so its ends give the column's middle
    let xs: Vec<usize> = (def.0..=def.1).flat_map(|y| (label_x.0..label_x.1).filter(move |&x| label(x, y))).collect();
    let mid = (xs.iter().min()? + xs.iter().max()?) / 2;
    (rows[0] >= half && rows[2] + half <= c.h).then_some((rows, half, mid))
}

// --- digits ---

/// Digits are at least this tall (at 1080p); the first run this tall from the aligned edge sets the number's height,
/// and the rest of it is judged against that.
const MIN_DIGIT_H: usize = 15;
/// A wider gap than this ends the number (the arrow and the species base come next).
const GAP: usize = 8;
/// How far a digit's top and bottom may sit from the first digit's (more for bigger digits).
const DIGIT_ALIGN: usize = 2;
/// A digit's shape is compared at this size.
const GW: usize = 10;
const GH: usize = 16;

/// The number at a stat row, read from its aligned edge.
fn number(lum: &[f64], w: usize, x0: usize, x1: usize, (top, bottom): (usize, usize), from_right: bool) -> Option<u32> {
    let rows = bottom - top;
    let cols = x1 - x0;
    let px = |r: usize, c: usize| lum[(top + r) * w + x0 + c];
    let mut vals: Vec<f64> = Vec::with_capacity(rows * cols);
    for r in 0..rows {
        for c in 0..cols {
            vals.push(px(r, c));
        }
    }
    // coloured pixels were set to 0: leave them out of the split, or they drag it down into the digits' soft edges
    vals.retain(|v| *v > 0.0);
    let th = otsu(&vals).max(80.0);
    let lit = |r: usize, c: usize| px(r, c) > th;
    // the number's own rows: the band of lit rows at least a digit tall nearest the window's middle, so a bar under
    // it or the row above don't join onto its digits
    let lit_rows: Vec<bool> = (0..rows).map(|r| (0..cols).filter(|&c| lit(r, c)).count() >= 2).collect();
    let mut bands = vec![];
    let mut r = 0;
    while r < rows {
        if !lit_rows[r] {
            r += 1;
            continue;
        }
        let start = r;
        while r < rows && lit_rows[r] {
            r += 1;
        }
        if r - start >= MIN_DIGIT_H {
            bands.push((start, r - 1));
        }
    }
    let (b0, b1) = bands.into_iter().min_by_key(|&(a, b)| (a + b).abs_diff(rows))?;
    let on = |r: usize, c: usize| (b0..=b1).contains(&r) && lit(r, c);
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
    // digits (full height) and commas (low and narrow) from the aligned edge, until something else or a wide gap;
    // the first digit sets the height the rest are judged by
    let mut kept: Vec<(bool, usize, usize, usize, usize)> = vec![];
    let mut prev: Option<(usize, usize)> = None;
    let mut first: Option<(usize, usize, usize)> = None;
    for (a, b, t, btm) in runs {
        let h = btm - t + 1;
        let Some((dh, dt, db)) = first else {
            if h >= MIN_DIGIT_H {
                first = Some((h, t, btm));
                kept.push((true, a, b, t, btm));
                prev = Some((a, b));
            }
            continue;
        };
        if let Some((pa, pb)) = prev {
            let gap = if from_right { pa.saturating_sub(b) } else { a.saturating_sub(pb) };
            if gap > GAP {
                break;
            }
        }
        prev = Some((a, b));
        // a digit lines up with the first one, top and bottom (a box beside the number, "BASE 82", doesn't)
        let align = DIGIT_ALIGN.max(dh / 12);
        if t.abs_diff(dt) <= align && btm.abs_diff(db) <= align {
            kept.push((true, a, b, t, btm));
        } else if h * 20 <= dh * 9 && t * 5 >= dt * 5 + dh * 2 && (b - a) * 20 <= dh * 9 {
            kept.push((false, a, b, t, btm));
        } else {
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
    // the closest shape in any of the fonts the overlay has used
    let best =
        FONTS.iter().flat_map(|font| font.iter().enumerate()).min_by(|a, b| score(a.1).total_cmp(&score(b.1))).map(|(i, _)| i).unwrap_or(0);
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

/// The overlay's fonts, each as its digits' shapes, 0 to 9.
const FONTS: [&[(f64, [u8; GW * GH]); 10]; 2] = [&DIGITS, &DIGITS_27_NOV];

/// Each digit's width over height and shape (GW × GH, 0-255), averaged from 647 digits in 32 of the stream's frames
/// at 1080p, all read right whichever frame was left out. 0 to 9. The overlay's font until 27 November 2026.
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

/// The font from 27 November 2026 (condensed; its 1 has a flag the first font's 7 is close to): averaged the same
/// way, from the stream's frames that day, each labelled with the site's numbers.
#[rustfmt::skip]
const DIGITS_27_NOV: [(f64, [u8; GW * GH]); 10] = [
    // 0 (42 samples)
    (0.513, [0, 41, 153, 249, 255, 255, 255, 178, 70, 0, 52, 234, 255, 255, 255, 255, 255, 255, 248, 70, 186, 255, 255, 255, 232, 232, 255, 255, 255, 218, 255, 255, 255, 210, 21, 25, 215, 255, 255, 255, 255, 255, 255, 201, 0, 0, 201, 255, 255, 255, 255, 255, 255, 201, 0, 0, 201, 255, 255, 255, 255, 255, 255, 201, 0, 0, 201, 255, 255, 255, 255, 255, 255, 201, 0, 0, 201, 255, 255, 255, 255, 255, 255, 201, 0, 0, 201, 255, 255, 255, 255, 255, 255, 201, 0, 0, 201, 255, 255, 255, 255, 255, 255, 201, 0, 0, 201, 255, 255, 255, 255, 255, 255, 201, 0, 0, 201, 255, 255, 255, 255, 255, 255, 215, 25, 23, 215, 255, 255, 255, 186, 255, 255, 255, 232, 222, 255, 255, 255, 218, 52, 234, 255, 255, 255, 255, 255, 255, 248, 70, 0, 48, 154, 249, 255, 255, 255, 179, 77, 0]),
    // 1 (66 samples)
    (0.514, [4, 79, 130, 241, 255, 255, 215, 0, 0, 0, 236, 252, 255, 255, 255, 255, 215, 0, 0, 0, 255, 255, 255, 255, 255, 255, 215, 0, 0, 0, 255, 239, 158, 241, 255, 255, 215, 0, 0, 0, 32, 24, 9, 211, 255, 255, 215, 0, 0, 0, 0, 0, 9, 214, 255, 255, 215, 0, 0, 0, 0, 0, 9, 216, 255, 255, 215, 0, 0, 0, 0, 0, 9, 216, 255, 255, 215, 0, 0, 0, 0, 0, 9, 216, 255, 255, 215, 0, 0, 0, 0, 0, 9, 216, 255, 255, 215, 0, 0, 0, 0, 0, 9, 216, 255, 255, 215, 0, 0, 0, 0, 0, 9, 219, 255, 255, 215, 0, 0, 0, 0, 0, 10, 208, 255, 255, 217, 9, 1, 0, 151, 151, 159, 245, 255, 255, 249, 189, 157, 151, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255]),
    // 2 (26 samples)
    (0.551, [0, 23, 135, 255, 255, 255, 255, 135, 29, 0, 40, 220, 255, 255, 255, 255, 255, 255, 237, 40, 159, 255, 255, 255, 227, 227, 255, 255, 255, 154, 252, 255, 255, 167, 18, 18, 158, 255, 255, 252, 188, 188, 188, 97, 0, 0, 138, 255, 255, 255, 0, 0, 0, 0, 0, 15, 243, 255, 255, 145, 0, 0, 0, 0, 0, 83, 255, 255, 247, 79, 0, 0, 0, 0, 58, 239, 255, 255, 182, 0, 0, 0, 0, 22, 195, 255, 255, 237, 50, 0, 0, 0, 7, 193, 255, 255, 239, 72, 0, 0, 0, 6, 135, 255, 255, 252, 122, 0, 0, 0, 0, 118, 255, 255, 255, 155, 5, 0, 0, 0, 54, 244, 255, 255, 221, 40, 0, 0, 0, 0, 239, 255, 255, 255, 219, 157, 157, 157, 157, 157, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255]),
    // 3 (20 samples)
    (0.556, [0, 14, 113, 227, 255, 255, 255, 170, 50, 0, 14, 184, 255, 255, 255, 255, 255, 255, 234, 25, 85, 255, 255, 255, 234, 212, 255, 255, 255, 135, 184, 255, 255, 227, 50, 14, 210, 255, 255, 191, 85, 113, 113, 85, 0, 0, 191, 255, 255, 191, 0, 0, 0, 0, 0, 20, 217, 255, 255, 163, 0, 0, 0, 50, 198, 218, 255, 255, 234, 43, 0, 0, 0, 64, 255, 255, 255, 255, 50, 0, 0, 0, 0, 57, 227, 234, 255, 255, 255, 78, 0, 0, 0, 0, 0, 21, 213, 255, 255, 197, 0, 0, 0, 0, 0, 0, 191, 255, 255, 234, 113, 113, 113, 85, 0, 0, 191, 255, 255, 251, 191, 255, 255, 198, 10, 14, 198, 255, 255, 203, 163, 255, 255, 255, 200, 212, 255, 255, 255, 177, 43, 234, 255, 255, 255, 255, 255, 255, 234, 57, 0, 50, 170, 255, 255, 255, 255, 170, 50, 0]),
    // 4 (36 samples)
    (0.581, [0, 0, 0, 28, 255, 255, 255, 57, 0, 0, 0, 0, 0, 165, 255, 255, 200, 20, 0, 0, 0, 0, 24, 206, 255, 255, 144, 0, 0, 0, 0, 0, 57, 255, 255, 255, 36, 0, 0, 0, 0, 0, 180, 255, 255, 156, 4, 0, 0, 0, 0, 57, 236, 255, 255, 96, 0, 0, 0, 0, 0, 126, 255, 255, 255, 0, 0, 0, 0, 0, 0, 227, 255, 255, 145, 29, 66, 66, 58, 0, 84, 248, 255, 249, 88, 113, 255, 255, 227, 0, 141, 255, 255, 227, 0, 113, 255, 255, 227, 0, 255, 255, 255, 211, 41, 136, 255, 255, 231, 41, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 107, 107, 107, 107, 107, 173, 255, 255, 239, 107, 0, 0, 0, 0, 0, 113, 255, 255, 227, 0, 0, 0, 0, 0, 0, 113, 255, 255, 227, 0]),
    // 5 (74 samples)
    (0.506, [116, 255, 255, 255, 255, 255, 255, 255, 255, 194, 107, 255, 255, 255, 255, 255, 255, 255, 255, 194, 107, 255, 255, 251, 190, 162, 162, 162, 162, 135, 107, 255, 255, 237, 38, 0, 0, 0, 0, 0, 107, 255, 255, 237, 34, 0, 0, 0, 0, 0, 107, 255, 255, 241, 122, 185, 185, 169, 47, 0, 107, 255, 255, 255, 255, 255, 255, 255, 228, 69, 107, 255, 255, 255, 255, 255, 255, 255, 255, 210, 96, 227, 228, 214, 87, 56, 226, 255, 255, 255, 14, 33, 34, 18, 0, 0, 201, 255, 255, 255, 0, 0, 0, 0, 0, 0, 214, 255, 255, 255, 131, 131, 131, 81, 0, 0, 214, 255, 255, 255, 250, 255, 255, 221, 21, 15, 218, 255, 255, 255, 156, 255, 255, 255, 224, 215, 255, 255, 255, 231, 64, 235, 255, 255, 255, 255, 255, 255, 248, 82, 0, 46, 152, 249, 255, 255, 255, 175, 67, 0]),
    // 6 (84 samples)
    (0.547, [0, 0, 0, 2, 188, 255, 255, 237, 30, 0, 0, 0, 0, 70, 252, 255, 255, 140, 0, 0, 0, 0, 3, 176, 255, 255, 235, 28, 0, 0, 0, 0, 59, 254, 255, 251, 122, 0, 0, 0, 0, 9, 187, 255, 255, 233, 10, 0, 0, 0, 0, 79, 252, 255, 255, 130, 0, 0, 0, 0, 7, 196, 255, 255, 248, 116, 80, 76, 7, 0, 73, 252, 255, 255, 255, 255, 255, 253, 188, 24, 172, 255, 255, 255, 255, 255, 255, 255, 255, 152, 236, 255, 255, 227, 91, 92, 231, 255, 255, 250, 255, 255, 252, 92, 0, 0, 120, 255, 255, 255, 255, 255, 250, 73, 0, 0, 115, 255, 255, 255, 245, 255, 253, 129, 15, 15, 145, 255, 255, 253, 192, 255, 255, 255, 217, 217, 255, 255, 255, 172, 48, 226, 255, 255, 255, 255, 255, 255, 226, 48, 0, 33, 144, 255, 255, 255, 255, 144, 33, 0]),
    // 7 (94 samples)
    (0.563, [245, 255, 255, 255, 255, 255, 255, 255, 255, 255, 250, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 254, 196, 160, 160, 222, 255, 255, 235, 255, 255, 253, 99, 0, 3, 221, 255, 255, 145, 237, 239, 238, 92, 0, 21, 249, 255, 252, 104, 18, 18, 18, 5, 0, 114, 255, 255, 241, 19, 0, 0, 0, 0, 0, 154, 255, 255, 217, 0, 0, 0, 0, 0, 6, 249, 255, 255, 113, 0, 0, 0, 0, 0, 63, 255, 255, 251, 79, 0, 0, 0, 0, 0, 138, 255, 255, 229, 2, 0, 0, 0, 0, 7, 205, 255, 255, 193, 0, 0, 0, 0, 0, 18, 254, 255, 255, 96, 0, 0, 0, 0, 0, 101, 255, 255, 252, 42, 0, 0, 0, 0, 1, 157, 255, 255, 215, 0, 0, 0, 0, 0, 7, 226, 255, 255, 158, 0, 0, 0, 0, 0, 29, 255, 255, 252, 80, 0, 0, 0]),
    // 8 (66 samples)
    (0.520, [0, 36, 146, 245, 255, 255, 248, 162, 30, 0, 46, 219, 255, 255, 255, 255, 255, 255, 217, 33, 153, 254, 255, 255, 218, 202, 255, 255, 254, 174, 166, 255, 255, 211, 30, 25, 215, 255, 255, 202, 166, 255, 255, 187, 0, 13, 193, 255, 255, 202, 136, 255, 255, 212, 36, 44, 220, 255, 254, 177, 27, 203, 255, 253, 227, 210, 251, 253, 199, 42, 0, 55, 207, 255, 255, 255, 255, 218, 46, 0, 54, 222, 255, 255, 240, 244, 255, 255, 221, 42, 193, 255, 255, 214, 53, 55, 204, 255, 255, 202, 255, 255, 255, 141, 0, 0, 118, 255, 255, 255, 255, 255, 255, 133, 0, 0, 118, 255, 255, 255, 255, 255, 255, 185, 15, 20, 168, 255, 255, 251, 195, 255, 255, 255, 216, 209, 255, 255, 255, 197, 54, 230, 255, 255, 255, 255, 255, 255, 237, 62, 0, 38, 146, 245, 255, 255, 248, 162, 56, 0]),
    // 9 (32 samples)
    (0.552, [0, 22, 136, 255, 255, 255, 255, 136, 22, 0, 38, 219, 255, 255, 255, 255, 255, 255, 219, 38, 208, 255, 255, 255, 218, 218, 255, 255, 255, 169, 255, 255, 253, 128, 16, 16, 162, 255, 255, 255, 255, 255, 248, 67, 0, 0, 131, 255, 255, 255, 255, 255, 248, 72, 0, 0, 131, 255, 255, 255, 255, 255, 254, 190, 47, 47, 198, 255, 255, 233, 190, 255, 255, 255, 239, 239, 255, 255, 255, 127, 69, 243, 255, 255, 255, 255, 255, 255, 250, 69, 0, 52, 150, 202, 209, 255, 255, 255, 175, 0, 0, 0, 0, 0, 99, 255, 255, 248, 57, 0, 0, 0, 0, 8, 235, 255, 255, 172, 0, 0, 0, 0, 1, 137, 255, 255, 248, 35, 0, 0, 0, 0, 12, 255, 255, 255, 145, 0, 0, 0, 0, 1, 147, 255, 255, 247, 41, 0, 0, 0, 0, 30, 252, 255, 255, 173, 0, 0, 0, 0]),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// A saved panel (a PNG crop of `PANEL` from a frame of the stream: numbers and labels only, no names or art).
    /// One file per overlay version: `nov25-*` the first (cut lower and shorter at the time, padded out with black),
    /// `nov26-*` its redesign of 26 November 2026 (`-2` and `-3` padded too), `nov27-*` the one from 27 November, and
    /// `nov27b-*` its change later that day (no POWER row, pale labels, the gaps as plain text, a see-through panel).
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
            ("nov25-451.png", [4770, 612, 767], [4986, 582, 696]),
            ("nov25-452.png", [5890, 655, 695], [4807, 513, 776]),
            ("nov25-465.png", [6270, 464, 797], [5072, 661, 677]),
            ("nov25-466.png", [4986, 576, 706], [5778, 778, 691]),
            ("nov25-481.png", [7688, 1044, 1145], [8826, 919, 1318]),
            ("nov26-1.png", [7484, 885, 1034], [6902, 941, 1033]),
            ("nov26-2.png", [5246, 563, 673], [4986, 582, 696]),
            ("nov26-3.png", [5086, 510, 650], [5532, 550, 624]),
            ("nov27-1.png", [5890, 655, 695], [4770, 612, 767]),
            ("nov27-2.png", [8336, 718, 1144], [6851, 778, 1280]),
            ("nov27b-1.png", [5013, 604, 699], [5192, 608, 877]),
            // the markers before the labels (◀ HP), not after: the labels sit further right
            ("nov27b-2.png", [7749, 669, 1003], [7484, 764, 940]),
        ] {
            let p = read(&crop(name)).unwrap_or_else(|| panic!("{name}: the panel is up"));
            assert_eq!((p.home, p.away), (some(home), some(away)), "{name}");
        }
    }

    #[test]
    fn reads_a_720p_frame_scaled_up() {
        let p = read(&crop("nov25-465-720p.png")).unwrap();
        assert_eq!((p.home, p.away), (some([6270, 464, 797]), some([5072, 661, 677])));
        let p = read(&crop("nov26-1-720p.png")).unwrap();
        assert_eq!((p.home, p.away), (some([7484, 885, 1034]), some([6902, 941, 1033])));
    }

    #[test]
    fn reads_a_panel_that_has_moved() {
        // the rows and middle are found from the labels, so a panel that sits a little off still reads
        let moved = |c: &Crop, dx: isize, dy: isize| {
            let mut rgba = vec![0; c.rgba.len()];
            for y in 0..c.h {
                for x in 0..c.w {
                    let (sx, sy) = (x as isize - dx, y as isize - dy);
                    if (0..c.w as isize).contains(&sx) && (0..c.h as isize).contains(&sy) {
                        let (to, from) = ((y * c.w + x) * 4, (sy as usize * c.w + sx as usize) * 4);
                        rgba[to..to + 4].copy_from_slice(&c.rgba[from..from + 4]);
                    }
                }
            }
            Crop { w: c.w, h: c.h, rgba }
        };
        for (name, home, away, shifts) in [
            ("nov26-1.png", [7484, 885, 1034], [6902, 941, 1033], [(0, -20), (0, 25), (-12, 0), (10, 0), (8, -15), (-8, 15)]),
            ("nov27-1.png", [5890, 655, 695], [4770, 612, 767], [(0, -25), (0, 25), (-4, 0), (14, 0), (12, -15), (-4, 20)]),
            ("nov27b-1.png", [5013, 604, 699], [5192, 608, 877], [(0, -25), (0, 25), (-12, 0), (4, 0), (-8, -15), (4, 20)]),
            ("nov27b-2.png", [7749, 669, 1003], [7484, 764, 940], [(0, -25), (0, 25), (-16, 0), (4, 0), (-8, -15), (4, 20)]),
        ] {
            let c = crop(name);
            for (dx, dy) in shifts {
                let p = read(&moved(&c, dx, dy)).unwrap_or_else(|| panic!("{name} moved {dx}, {dy}: the panel is up"));
                assert_eq!((p.home, p.away), (some(home), some(away)), "{name} moved {dx}, {dy}");
            }
        }
        // further off to one side, today's numbers reach the labels' strip: no read, rather than the wrong rows
        assert_eq!(read(&moved(&crop("nov27-1.png"), -14, 0)), None);
        assert_eq!(read(&moved(&crop("nov27b-1.png"), 12, 0)), None);
    }

    #[test]
    fn no_panel_when_the_stream_shows_the_game() {
        assert_eq!(read(&crop("game-nov25.png")), None);
        assert_eq!(read(&crop("game.png")), None);
        assert_eq!(read(&Crop { w: 10, h: 10, rgba: vec![0; 400] }), None, "the wrong size");
    }

    #[test]
    fn a_read_counts_only_when_the_shown_side_agrees() {
        let p = read(&crop("nov25-465.png"));
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
    fn a_fight_with_nothing_hidden_checks_both_sides() {
        let p = read(&crop("nov26-1.png"));
        let (home, away) = ([7484, 885, 1034], [6902, 941, 1033]);
        assert_eq!(check_both(p.as_ref(), home, away), BothCheck::Agreed);
        assert_eq!(
            check_both(p.as_ref(), home, [6902, 941, 1034]),
            BothCheck::Misread { home: some(home), away: some(away) },
            "one side agrees, so it's this duel's panel and the other side misread"
        );
        assert_eq!(check_both(p.as_ref(), [1, 2, 3], [4, 5, 6]), BothCheck::Other, "another duel's panel");
        assert_eq!(check_both(None, home, away), BothCheck::NoPanel);
    }

    #[test]
    fn the_record_counts_misses_in_a_row() {
        let conn = crate::db::open_memory().unwrap();
        for id in 1..=6 {
            conn.execute("INSERT INTO predictions (id, status) VALUES (?, 'settled')", [id]).unwrap();
        }
        let line = |outcome: &str| StreamRead { outcome: outcome.into(), side: "away".into(), ..Default::default() };
        record(&conn, 1, &line(MISSED)).unwrap();
        record(&conn, 2, &line(READ)).unwrap();
        record(&conn, 2, &line(AGREED)).unwrap();
        record(&conn, 3, &line(MISSED)).unwrap();
        record(&conn, 4, &line(MISSED)).unwrap();
        assert_eq!(misses_in_a_row(&conn).unwrap(), 2, "back to the last one it read");
        record(&conn, 5, &line(UNCHECKED)).unwrap();
        assert_eq!(misses_in_a_row(&conn).unwrap(), 2, "a fight with nothing hidden is neither a read nor a miss");
        record(&conn, 6, &line(READ)).unwrap();
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
