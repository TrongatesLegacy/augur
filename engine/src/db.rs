//! The SQLite store: calls (picks made live, with the fighters as seen before the bell), finished duels with their
//! fight tapes, every species' types, base stats and listed moves, and the developer's own bets.

use crate::model::{Fighter, Move};
use crate::site::{Duel, Hit, Pick, Recent, Species};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub use crate::error::Result;
/// rusqlite's own result, for row closures and collecting rows.
type Rq<T> = rusqlite::Result<T>;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS duels (
    id INTEGER PRIMARY KEY,            -- palduels.com duel number
    fought_at TEXT,                    -- ISO time from the duel page
    home_owner TEXT, away_owner TEXT,
    home_pal TEXT, away_pal TEXT,
    winner TEXT CHECK (winner IN ('home', 'away')),
    ko TEXT,                           -- K.O. time, e.g. 0:27
    winner_hp_left INTEGER,            -- % of HP the winner had left
    home_casts INTEGER, away_casts INTEGER
);
CREATE TABLE IF NOT EXISTS hits (      -- the fight tape, one row per hit
    duel_id INTEGER NOT NULL REFERENCES duels (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    side TEXT NOT NULL CHECK (side IN ('home', 'away')),
    t INTEGER,                         -- seconds into the fight
    move TEXT,                         -- '' for unnamed hits (likely damage over time)
    pct INTEGER,                       -- % of the target's HP
    PRIMARY KEY (duel_id, seq)
);
CREATE TABLE IF NOT EXISTS species (   -- each species' page: types, the ladder's power rating, base stats
    pal TEXT PRIMARY KEY,
    slug TEXT,
    types TEXT,                        -- JSON list
    power INTEGER,
    base_hp INTEGER, base_atk INTEGER, base_def INTEGER
);
CREATE TABLE IF NOT EXISTS species_moves ( -- each species' moves as the site lists them
    pal TEXT NOT NULL,
    name TEXT NOT NULL,
    element TEXT,
    power INTEGER,
    unlock_lv INTEGER,
    PRIMARY KEY (pal, name)
);
CREATE TABLE IF NOT EXISTS predictions (   -- Augur's calls
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    seen_at INTEGER,                   -- site clock (ms) when the pick was made
    home_owner TEXT, away_owner TEXT,
    home_pal TEXT, away_pal TEXT,
    pick TEXT CHECK (pick IN ('home', 'away')),
    prob REAL,                         -- chance given to the pick
    p_stats REAL, p_species REAL, p_h2h REAL,  -- each factor's home win chance
    estimated INTEGER DEFAULT 0,       -- 1 if some stats were hidden until the bell
    home_species_w INTEGER, home_species_l INTEGER,
    away_species_w INTEGER, away_species_l INTEGER,
    pal_h2h_home INTEGER, pal_h2h_away INTEGER,
    model TEXT,                        -- JSON of the model settings used
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'settled', 'skipped')),
    duel_id INTEGER REFERENCES duels (id),
    stake INTEGER,                     -- credits Augur suggested betting on the pick (0: sit it out)
    stake_cmd TEXT                     -- the chat command it suggested, e.g. !h all
);
CREATE TABLE IF NOT EXISTS fighters (  -- each side of a call as seen before the bell
    prediction_id INTEGER NOT NULL REFERENCES predictions (id) ON DELETE CASCADE,
    side TEXT NOT NULL CHECK (side IN ('home', 'away')),
    pal TEXT,
    types TEXT,                        -- JSON list
    hp INTEGER, atk INTEGER, def INTEGER,
    hp_iv INTEGER, atk_iv INTEGER, def_iv INTEGER,
    passives TEXT,                     -- JSON list
    hidden_boost REAL DEFAULT 1,
    speed REAL DEFAULT 1,
    PRIMARY KEY (prediction_id, side)
);
CREATE TABLE IF NOT EXISTS bets (     -- the developer's own bets, from their picks list on palduels.com (signed in)
    seq INTEGER PRIMARY KEY,           -- place in their picks history, 1 = their first bet (stable as bets are added)
    duel_id INTEGER,                   -- recent rows link their duel; older ones don't
    home_owner TEXT, away_owner TEXT,
    home_pal TEXT, away_pal TEXT,
    backed TEXT CHECK (backed IN ('home', 'away')),
    winner TEXT,
    stake INTEGER,                     -- credits bet
    result INTEGER,                    -- credits won (+) or lost (-); NULL while the fight is on
    seen_at INTEGER                    -- when Augur first read it (ms)
);
CREATE TABLE IF NOT EXISTS meta (      -- small key/value state: backfill progress and the like
    key TEXT PRIMARY KEY,
    value TEXT
);
CREATE INDEX IF NOT EXISTS predictions_status ON predictions (status);
CREATE INDEX IF NOT EXISTS duels_home_pal ON duels (home_pal);
CREATE INDEX IF NOT EXISTS duels_away_pal ON duels (away_pal);
CREATE INDEX IF NOT EXISTS bets_duel ON bets (duel_id);
"#;

/// Columns added since the first release, for stores made before them.
const ADDED: &[(&str, &str, &str)] = &[
    ("predictions", "stake", "INTEGER"),
    ("predictions", "stake_cmd", "TEXT"),
    // picks closed: the call is what was showing then; a later re-call (stats shown at the bell) is kept apart
    ("predictions", "closed", "INTEGER DEFAULT 0"),
    ("predictions", "late_pick", "TEXT"),
    ("predictions", "late_prob", "REAL"),
    // first called after picks closed (Augur started mid-fight): logged and learned from, but not in the tally
    ("predictions", "late_only", "INTEGER DEFAULT 0"),
    // where each side's stats came from when called: "home: from their box; away: shown"
    ("predictions", "stats_from", "TEXT"),
    // the Augur version that made the call, to compare results across algorithm changes
    ("predictions", "app_version", "TEXT"),
];

fn migrate(conn: &Connection) -> Result<()> {
    for (table, column, kind) in ADDED {
        let has: i64 =
            conn.query_row(&format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?"), [column], |r| r.get(0))?;
        if has == 0 {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {kind}"))?;
        }
    }
    Ok(())
}

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(15))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.execute_batch(SCHEMA)?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn open_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(SCHEMA)?;
    migrate(&conn)?;
    Ok(conn)
}

// --- meta ---

pub fn meta_get(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn.query_row("SELECT value FROM meta WHERE key = ?", [key], |r| r.get(0)).optional()?)
}

pub fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute("INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value", [key, value])?;
    Ok(())
}

// --- species ---

pub fn save_moves(conn: &Connection, pal: &str, moves: &[Move]) -> Result<()> {
    let mut st = conn.prepare_cached(
        "INSERT INTO species_moves (pal, name, element, power, unlock_lv) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (pal, name) DO UPDATE SET element = excluded.element, power = excluded.power, unlock_lv = excluded.unlock_lv",
    )?;
    for m in moves {
        st.execute(params![pal, m.name, m.el, m.power as i64, m.lv])?;
    }
    Ok(())
}

pub fn moves_of(conn: &Connection, pal: &str) -> Result<Vec<Move>> {
    let mut st = conn.prepare_cached("SELECT name, element, power, unlock_lv FROM species_moves WHERE pal = ? ORDER BY unlock_lv, name")?;
    let rows = st.query_map([pal], |r| {
        Ok(Move {
            name: r.get(0)?,
            el: r.get::<_, Option<String>>(1)?.unwrap_or_else(|| "neutral".into()),
            power: r.get::<_, i64>(2)? as f64,
            lv: r.get(3)?,
            hit: 1.0,
        })
    })?;
    Ok(rows.collect::<Rq<_>>()?)
}

/// How hard each move really hits in the stored fight tapes, against the average move: its mean % of the target's
/// HP per hit, pulled toward the average by 10 hits' worth so rare moves stay near 1. Only tapes of duels before
/// `before` when given (so a test on newer duels doesn't see their own tapes).
pub fn hit_sizes(conn: &Connection, before: Option<u64>) -> Result<std::collections::HashMap<String, f64>> {
    let mut st = conn.prepare("SELECT move, AVG(pct), COUNT(*) FROM hits WHERE move != '' AND duel_id < ? GROUP BY move")?;
    let rows: Vec<(String, f64, f64)> =
        st.query_map([before.map_or(i64::MAX, |b| b as i64)], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Rq<_>>()?;
    let hits: f64 = rows.iter().map(|r| r.2).sum();
    if hits == 0.0 {
        return Ok(Default::default());
    }
    let mean = rows.iter().map(|r| r.1 * r.2).sum::<f64>() / hits;
    Ok(rows.into_iter().map(|(m, avg, n)| (m, ((avg * n + mean * 10.0) / (n + 10.0)) / mean)).collect())
}

/// Sets each move's real hit size from `hit_sizes`.
pub fn with_hit_sizes(moves: &mut [Move], sizes: &std::collections::HashMap<String, f64>) {
    for m in moves {
        m.hit = (sizes.get(&m.name).copied().unwrap_or(1.0) * 1000.0).round() / 1000.0;
    }
}

pub fn save_species(conn: &Connection, sp: &Species) -> Result<()> {
    conn.execute(
        "INSERT INTO species (pal, slug, types, power, base_hp, base_atk, base_def) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (pal) DO UPDATE SET slug = excluded.slug, types = excluded.types, power = excluded.power,
         base_hp = excluded.base_hp, base_atk = excluded.base_atk, base_def = excluded.base_def",
        params![sp.pal, sp.slug, serde_json::to_string(&sp.types).unwrap(), sp.power, sp.base_hp, sp.base_atk, sp.base_def],
    )?;
    save_moves(conn, &sp.pal, &sp.moves)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SpeciesRow {
    pub pal: String,
    pub slug: String,
    pub types: Vec<String>,
    pub power: Option<u32>,
    pub base_hp: Option<u32>,
    pub base_atk: Option<u32>,
    pub base_def: Option<u32>,
}

pub fn species_row(conn: &Connection, pal: &str) -> Result<Option<SpeciesRow>> {
    Ok(conn
        .query_row("SELECT pal, slug, types, power, base_hp, base_atk, base_def FROM species WHERE pal = ?", [pal], |r| {
            Ok(SpeciesRow {
                pal: r.get(0)?,
                slug: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                types: serde_json::from_str(&r.get::<_, Option<String>>(2)?.unwrap_or_default()).unwrap_or_default(),
                power: r.get(3)?,
                base_hp: r.get(4)?,
                base_atk: r.get(5)?,
                base_def: r.get(6)?,
            })
        })
        .optional()?)
}

pub fn species_count(conn: &Connection) -> Result<u32> {
    Ok(conn.query_row("SELECT COUNT(*) FROM species", [], |r| r.get(0))?)
}

// --- duels and tapes ---

pub fn save_duel(conn: &Connection, id: u64, d: &Duel) -> Result<()> {
    conn.execute(
        "INSERT INTO duels (id, fought_at, home_owner, away_owner, home_pal, away_pal, winner, ko, winner_hp_left, home_casts, away_casts)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) ON CONFLICT (id) DO UPDATE SET
         fought_at = excluded.fought_at, home_owner = excluded.home_owner, away_owner = excluded.away_owner,
         home_pal = excluded.home_pal, away_pal = excluded.away_pal, winner = excluded.winner, ko = excluded.ko,
         winner_hp_left = excluded.winner_hp_left, home_casts = excluded.home_casts, away_casts = excluded.away_casts",
        params![
            id as i64,
            d.fought_at,
            d.home_owner,
            d.away_owner,
            d.home_pal,
            d.away_pal,
            d.winner,
            d.ko,
            d.winner_hp_left,
            d.home_casts,
            d.away_casts
        ],
    )?;
    conn.execute("DELETE FROM hits WHERE duel_id = ?", [id as i64])?;
    let mut st = conn.prepare_cached("INSERT INTO hits (duel_id, seq, side, t, move, pct) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
    for (i, h) in d.hits.iter().enumerate() {
        st.execute(params![id as i64, i as i64, h.side, h.t, h.mv, h.pct as i64])?;
    }
    Ok(())
}

pub fn get_duel(conn: &Connection, id: u64) -> Result<Option<Duel>> {
    let d = conn
        .query_row(
            "SELECT fought_at, home_owner, away_owner, home_pal, away_pal, winner, ko, winner_hp_left, home_casts, away_casts FROM duels WHERE id = ?",
            [id as i64],
            |r| {
                Ok(Duel {
                    fought_at: r.get(0)?,
                    home_owner: r.get(1)?,
                    away_owner: r.get(2)?,
                    home_pal: r.get(3)?,
                    away_pal: r.get(4)?,
                    winner: r.get(5)?,
                    ko: r.get(6)?,
                    winner_hp_left: r.get(7)?,
                    home_casts: r.get::<_, Option<u32>>(8)?.unwrap_or(0),
                    away_casts: r.get::<_, Option<u32>>(9)?.unwrap_or(0),
                    ..Default::default()
                })
            },
        )
        .optional()?;
    let Some(mut d) = d else { return Ok(None) };
    let mut st = conn.prepare_cached("SELECT side, t, move, pct FROM hits WHERE duel_id = ? ORDER BY seq")?;
    d.hits = st
        .query_map([id as i64], |r| {
            Ok(Hit {
                side: r.get(0)?,
                t: r.get::<_, Option<u32>>(1)?.unwrap_or(0),
                mv: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                pct: r.get::<_, i64>(3)? as f64,
            })
        })?
        .collect::<Rq<_>>()?;
    Ok(Some(d))
}

/// Whether a duel is stored (an import may have filled it in since a backfill made its list).
pub fn has_duel(conn: &Connection, id: u64) -> Result<bool> {
    Ok(conn.query_row("SELECT COUNT(*) FROM duels WHERE id = ?", [id as i64], |r| r.get::<_, i64>(0))? > 0)
}

pub fn has_species(conn: &Connection, slug: &str) -> Result<bool> {
    Ok(conn.query_row("SELECT COUNT(*) FROM species WHERE slug = ?", [slug], |r| r.get::<_, i64>(0))? > 0)
}

/// Nothing in the store yet: no calls and no duels (a fresh install).
pub fn is_empty(conn: &Connection) -> Result<bool> {
    Ok(conn.query_row("SELECT (SELECT COUNT(*) FROM duels) + (SELECT COUNT(*) FROM predictions)", [], |r| r.get::<_, i64>(0))? == 0)
}

pub fn duel_ids(conn: &Connection) -> Result<std::collections::HashSet<u64>> {
    let mut st = conn.prepare("SELECT id FROM duels")?;
    let ids = st.query_map([], |r| r.get::<_, i64>(0))?;
    Ok(ids.map(|r| r.map(|i| i as u64)).collect::<Rq<_>>()?)
}

// --- calls ---

/// A side of a call: the model's view plus the talent rolls the site showed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallFighter {
    #[serde(flatten)]
    pub fighter: Fighter,
    #[serde(default)]
    pub ivs: std::collections::HashMap<String, Option<f64>>,
}

/// A call as made, before the result is known.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewCall {
    pub seen_at: u64,
    pub home_owner: String,
    pub away_owner: String,
    pub home_pal: String,
    pub away_pal: String,
    pub pick: String,
    pub prob: f64,
    pub p_stats: f64,
    pub p_species: f64,
    pub p_h2h: f64,
    pub estimated: bool,
    pub species_rec: [[f64; 2]; 2],
    pub pal_h2h: [f64; 2],
    pub model: crate::model::Settings,
    pub home: CallFighter,
    pub away: CallFighter,
    /// picks were open when it was made (old calls files don't say: they were)
    #[serde(default = "yes")]
    pub picks_open: bool,
    /// where each side's stats came from ("home: from their box; away: shown")
    #[serde(default)]
    pub stats_from: String,
    /// the Augur version that made it
    #[serde(default)]
    pub version: String,
}

fn yes() -> bool {
    true
}

/// Record a call. Re-running the same open fight while picks are open updates it in place. Once picks have closed
/// the call stands as it was: a re-call (stats shown at the bell) only updates the fighters, so the model learns from
/// the real stats, and is kept as the "after the bell" pick. A fight first seen after picks closed is logged as late.
pub fn add_call(conn: &Connection, c: &NewCall) -> Result<i64> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM predictions WHERE status = 'open' AND home_owner = ?1 AND away_owner = ?2 AND home_pal = ?3 AND away_pal = ?4",
            params![c.home_owner, c.away_owner, c.home_pal, c.away_pal],
            |r| r.get(0),
        )
        .optional()?;
    let closed: bool = match existing {
        Some(id) => conn.query_row("SELECT COALESCE(closed, 0) FROM predictions WHERE id = ?", [id], |r| r.get::<_, i64>(0))? == 1,
        None => false,
    };
    if let (Some(id), true) = (existing, closed || !c.picks_open) {
        conn.execute("UPDATE predictions SET closed = 1, late_pick = ?1, late_prob = ?2 WHERE id = ?3", params![c.pick, c.prob, id])?;
        conn.execute("DELETE FROM fighters WHERE prediction_id = ?", [id])?;
        save_fighters(conn, id, c)?;
        return Ok(id);
    }
    let model = serde_json::to_string(&c.model).unwrap();
    let vals = params![
        c.seen_at as i64,
        c.home_owner,
        c.away_owner,
        c.home_pal,
        c.away_pal,
        c.pick,
        c.prob,
        c.p_stats,
        c.p_species,
        c.p_h2h,
        c.estimated as i64,
        c.species_rec[0][0] as i64,
        c.species_rec[0][1] as i64,
        c.species_rec[1][0] as i64,
        c.species_rec[1][1] as i64,
        c.pal_h2h[0] as i64,
        c.pal_h2h[1] as i64,
        model
    ];
    let cols = "seen_at, home_owner, away_owner, home_pal, away_pal, pick, prob, p_stats, p_species, p_h2h, estimated, \
                home_species_w, home_species_l, away_species_w, away_species_l, pal_h2h_home, pal_h2h_away, model";
    let id = match existing {
        Some(id) => {
            let sets = cols.split(", ").skip(1).enumerate().map(|(i, c)| format!("{c} = ?{}", i + 2)).collect::<Vec<_>>().join(", ");
            conn.execute(&format!("UPDATE predictions SET {sets} WHERE id = ?19"), {
                let mut v: Vec<&dyn rusqlite::ToSql> = vals.to_vec();
                v.push(&id);
                rusqlite::params_from_iter(v)
            })?;
            conn.execute("DELETE FROM fighters WHERE prediction_id = ?", [id])?;
            id
        }
        None => {
            conn.execute(&format!("INSERT INTO predictions ({cols}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)"), vals)?;
            let id = conn.last_insert_rowid();
            if !c.picks_open {
                conn.execute("UPDATE predictions SET closed = 1, late_only = 1 WHERE id = ?", [id])?;
            }
            id
        }
    };
    conn.execute("UPDATE predictions SET stats_from = ?1, app_version = ?2 WHERE id = ?3", params![c.stats_from, c.version, id])?;
    save_fighters(conn, id, c)?;
    Ok(id)
}

/// Picks have closed on the fight up: its open call stands as it is now.
pub fn close_call(conn: &Connection, home_owner: &str, away_owner: &str) -> Result<()> {
    conn.execute(
        "UPDATE predictions SET closed = 1 WHERE status = 'open' AND COALESCE(closed, 0) = 0 AND home_owner = ?1 AND away_owner = ?2",
        params![home_owner, away_owner],
    )?;
    Ok(())
}

fn save_fighters(conn: &Connection, id: i64, c: &NewCall) -> Result<()> {
    for (side, f) in [("home", &c.home), ("away", &c.away)] {
        let iv = |k: &str| f.ivs.get(k).copied().flatten().map(|v| v as i64);
        conn.execute(
            "INSERT INTO fighters (prediction_id, side, pal, types, hp, atk, def, hp_iv, atk_iv, def_iv, passives, hidden_boost, speed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                id,
                side,
                f.fighter.pal,
                serde_json::to_string(&f.fighter.types).unwrap(),
                f.fighter.hp as i64,
                f.fighter.atk as i64,
                f.fighter.def as i64,
                iv("HP"),
                iv("ATK"),
                iv("DEF"),
                serde_json::to_string(&f.fighter.passives).unwrap(),
                f.fighter.hidden_boost,
                f.fighter.speed
            ],
        )?;
        save_moves(conn, &f.fighter.pal, &f.fighter.skills)?;
    }
    Ok(())
}

/// Match open calls to finished duels in the state's recent list and store their tapes. A call is only "skipped"
/// once two other fights have finished after it was made, because the next duel is queued (and predicted) before
/// the current one is recorded.
pub fn settle(conn: &Connection, recent: &[Recent], fetch_duel: impl Fn(u64) -> Option<Duel>) -> Result<Vec<i64>> {
    let open: Vec<(i64, String, String, i64)> = conn
        .prepare("SELECT id, home_owner, away_owner, COALESCE(seen_at, 0) FROM predictions WHERE status = 'open'")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<Rq<_>>()?;
    let mut settled = vec![];
    for (id, home, away, seen) in open {
        let hit = recent.iter().find(|r| r.home == home && r.away == away && r.at as i64 >= seen - 3_600_000);
        if let Some(r) = hit {
            if get_duel(conn, r.id)?.is_none_or(|d| d.winner.is_none()) {
                let d = fetch_duel(r.id).unwrap_or_else(|| Duel {
                    winner: Some(r.win.clone()),
                    home_owner: Some(r.home.clone()),
                    away_owner: Some(r.away.clone()),
                    ..Default::default()
                });
                save_duel(conn, r.id, &d)?;
            }
            conn.execute("UPDATE predictions SET status = 'settled', duel_id = ?1 WHERE id = ?2", params![r.id as i64, id])?;
            settled.push(id);
        } else if recent.iter().filter(|r| r.at as i64 > seen).count() >= 2 {
            conn.execute("UPDATE predictions SET status = 'skipped' WHERE id = ?", [id])?;
        }
    }
    Ok(settled)
}

/// A finished call with its result.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallRecord {
    pub id: i64,
    pub seen_at: i64,
    pub home_owner: String,
    pub away_owner: String,
    pub home_pal: String,
    pub away_pal: String,
    pub pick: String,
    pub prob: f64,
    pub p_stats: Option<f64>,
    pub p_species: Option<f64>,
    pub p_h2h: Option<f64>,
    pub species_rec: Option<[[f64; 2]; 2]>,
    pub pal_h2h: [f64; 2],
    pub duel_id: Option<u64>,
    pub winner: Option<String>,
    pub fought_at: Option<String>,
    pub ko: Option<String>,
    pub winner_hp_left: Option<u32>,
    /// what Augur suggested betting when it made the call (None for calls made before it suggested stakes)
    pub stake: Option<u64>,
    pub fighters: Option<(CallFighter, CallFighter)>,
    /// re-called after picks closed (stats shown at the bell): that pick and its chance
    #[serde(default)]
    pub late: Option<(String, f64)>,
    /// first called after picks closed: not a call you could bet on
    #[serde(default)]
    pub late_only: bool,
    /// the Augur version and model settings (JSON) that made it: an "era" of the algorithm
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub model: String,
}

impl CallRecord {
    pub fn right(&self) -> bool {
        self.winner.as_deref() == Some(self.pick.as_str())
    }
}

fn fighters_of(conn: &Connection, id: i64, sizes: &std::collections::HashMap<String, f64>) -> Result<Option<(CallFighter, CallFighter)>> {
    let mut st = conn.prepare_cached(
        "SELECT side, pal, types, hp, atk, def, hp_iv, atk_iv, def_iv, passives, hidden_boost, speed FROM fighters WHERE prediction_id = ?",
    )?;
    let rows: Vec<(String, CallFighter)> = st
        .query_map([id], |r| {
            let pal: String = r.get(1)?;
            let mut ivs = std::collections::HashMap::new();
            for (k, i) in [("HP", 6), ("ATK", 7), ("DEF", 8)] {
                ivs.insert(k.to_string(), r.get::<_, Option<i64>>(i)?.map(|v| v as f64));
            }
            Ok((
                r.get(0)?,
                CallFighter {
                    fighter: Fighter {
                        pal: pal.clone(),
                        types: serde_json::from_str(&r.get::<_, Option<String>>(2)?.unwrap_or_default()).unwrap_or_default(),
                        hp: r.get::<_, i64>(3)? as f64,
                        atk: r.get::<_, i64>(4)? as f64,
                        def: r.get::<_, i64>(5)? as f64,
                        skills: vec![],
                        passives: serde_json::from_str(&r.get::<_, Option<String>>(9)?.unwrap_or_default()).unwrap_or_default(),
                        hidden_boost: r.get::<_, Option<f64>>(10)?.unwrap_or(1.0),
                        speed: r.get::<_, Option<f64>>(11)?.unwrap_or(1.0),
                    },
                    ivs,
                },
            ))
        })?
        .collect::<Rq<_>>()?;
    let mut home = None;
    let mut away = None;
    for (side, mut f) in rows {
        f.fighter.skills = moves_of(conn, &f.fighter.pal)?;
        with_hit_sizes(&mut f.fighter.skills, sizes);
        if side == "home" {
            home = Some(f)
        } else {
            away = Some(f)
        }
    }
    Ok(home.zip(away))
}

/// Finished calls you could bet on (made while picks were open), oldest first: the tally, your bets and History.
pub fn settled(conn: &Connection) -> Result<Vec<CallRecord>> {
    Ok(settled_all(conn)?.into_iter().filter(|c| !c.late_only).collect())
}

/// Every finished call, including ones first made after picks closed: what the model learns from.
pub fn settled_all(conn: &Connection) -> Result<Vec<CallRecord>> {
    settled_where(conn, "", [])
}

/// One finished call, by its id (late-only calls too), for its breakdown.
pub fn settled_call(conn: &Connection, id: i64) -> Result<Option<CallRecord>> {
    Ok(settled_where(conn, "AND p.id = ?1", [id])?.pop())
}

fn settled_where(conn: &Connection, and: &str, args: impl rusqlite::Params) -> Result<Vec<CallRecord>> {
    let mut st = conn.prepare(&format!(
        "SELECT p.id, COALESCE(p.seen_at, 0), p.home_owner, p.away_owner, p.home_pal, p.away_pal, p.pick, p.prob, p.p_stats, p.p_species,
                p.p_h2h, p.home_species_w, p.home_species_l, p.away_species_w, p.away_species_l, p.pal_h2h_home, p.pal_h2h_away,
                p.duel_id, d.winner, d.fought_at, d.ko, d.winner_hp_left, p.stake, p.late_pick, p.late_prob, COALESCE(p.late_only, 0),
                COALESCE(p.app_version, ''), COALESCE(p.model, '')
         FROM predictions p LEFT JOIN duels d ON d.id = p.duel_id WHERE p.status = 'settled' AND d.winner IS NOT NULL {and} ORDER BY p.id"
    ))?;
    let calls: Vec<CallRecord> = st
        .query_map(args, |r| {
            let rec: [Option<i64>; 4] = [r.get(11)?, r.get(12)?, r.get(13)?, r.get(14)?];
            Ok(CallRecord {
                id: r.get(0)?,
                seen_at: r.get(1)?,
                home_owner: r.get(2)?,
                away_owner: r.get(3)?,
                home_pal: r.get(4)?,
                away_pal: r.get(5)?,
                pick: r.get(6)?,
                prob: r.get(7)?,
                p_stats: r.get(8)?,
                p_species: r.get(9)?,
                p_h2h: r.get(10)?,
                species_rec: match rec {
                    [Some(a), Some(b), Some(c), Some(d)] => Some([[a as f64, b as f64], [c as f64, d as f64]]),
                    _ => None,
                },
                pal_h2h: [r.get::<_, Option<i64>>(15)?.unwrap_or(0) as f64, r.get::<_, Option<i64>>(16)?.unwrap_or(0) as f64],
                duel_id: r.get::<_, Option<i64>>(17)?.map(|v| v as u64),
                winner: r.get(18)?,
                fought_at: r.get(19)?,
                ko: r.get(20)?,
                winner_hp_left: r.get(21)?,
                stake: r.get::<_, Option<i64>>(22)?.map(|v| v.max(0) as u64),
                fighters: None,
                late: r.get::<_, Option<String>>(23)?.zip(r.get::<_, Option<f64>>(24)?),
                late_only: r.get::<_, i64>(25)? == 1,
                version: r.get(26)?,
                model: r.get(27)?,
            })
        })?
        .collect::<Rq<_>>()?;
    let sizes = hit_sizes(conn, None)?;
    calls
        .into_iter()
        .map(|mut c| {
            c.fighters = fighters_of(conn, c.id, &sizes)?;
            Ok(c)
        })
        .collect()
}

/// Saves the stake Augur suggested with a call.
pub fn set_call_stake(conn: &Connection, id: i64, amount: u64, command: &str) -> Result<()> {
    // once picks have closed the stake stands too
    conn.execute(
        "UPDATE predictions SET stake = ?1, stake_cmd = ?2 WHERE id = ?3 AND COALESCE(closed, 0) = 0",
        params![amount as i64, command, id],
    )?;
    Ok(())
}

// --- the developer's bets ---

/// Stores picks from the picks list. Returns how many were new or changed (a result coming in).
pub fn save_picks(conn: &Connection, picks: &[Pick], now_ms: u64) -> Result<usize> {
    let mut changed = 0;
    for p in picks {
        let before: Option<(Option<i64>, Option<i64>)> =
            conn.query_row("SELECT result, duel_id FROM bets WHERE seq = ?", [p.seq as i64], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        if before.is_some_and(|(result, duel)| result == p.result && duel.map(|d| d as u64) == p.duel_id) {
            continue;
        }
        conn.execute(
            "INSERT INTO bets (seq, duel_id, home_owner, away_owner, home_pal, away_pal, backed, winner, stake, result, seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) ON CONFLICT (seq) DO UPDATE SET
             duel_id = excluded.duel_id, winner = excluded.winner, result = excluded.result",
            params![
                p.seq as i64,
                p.duel_id.map(|d| d as i64),
                p.home_owner,
                p.away_owner,
                p.home_pal,
                p.away_pal,
                p.backed,
                p.winner,
                p.stake as i64,
                p.result,
                now_ms as i64
            ],
        )?;
        changed += 1;
    }
    Ok(changed)
}

/// Every stored bet, newest first.
pub fn bets(conn: &Connection) -> Result<Vec<Pick>> {
    let mut st = conn.prepare(
        "SELECT seq, duel_id, home_owner, away_owner, home_pal, away_pal, backed, winner, stake, result FROM bets ORDER BY seq DESC",
    )?;
    let rows = st.query_map([], |r| {
        Ok(Pick {
            seq: r.get::<_, i64>(0)? as u64,
            duel_id: r.get::<_, Option<i64>>(1)?.map(|d| d as u64),
            home_owner: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            away_owner: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            home_pal: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            away_pal: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
            backed: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
            winner: r.get(7)?,
            stake: r.get::<_, Option<i64>>(8)?.unwrap_or(0).max(0) as u64,
            result: r.get(9)?,
            ago: String::new(),
        })
    })?;
    Ok(rows.collect::<Rq<_>>()?)
}

/// The oldest stored bet's place in the history (1 once everything is in), None before any import.
pub fn oldest_bet(conn: &Connection) -> Result<Option<u64>> {
    Ok(conn.query_row("SELECT MIN(seq) FROM bets", [], |r| r.get::<_, Option<i64>>(0))?.map(|v| v as u64))
}

// --- backing up and merging whole stores ---

/// A complete, consistent copy of the store, safe while the app is writing to it.
pub fn backup_to(conn: &Connection, path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    conn.execute("VACUUM INTO ?1", [path.to_string_lossy()])?;
    Ok(())
}

/// What merging another store added.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Merged {
    pub calls: usize,
    pub duels: usize,
    pub tapes: usize,
    pub species: usize,
    pub bets: usize,
}

/// Adds what another store (a backup from another computer) has and this one doesn't: calls with their fighters,
/// duels and tapes, species and moves, bets. Nothing here is changed or removed, so it's safe to run again.
pub fn merge_from(conn: &Connection, other: &Path) -> Result<Merged> {
    if !other.is_file() {
        return Err(crate::Error::Parse(format!("{}: not a file", other.display())));
    }
    conn.execute("ATTACH DATABASE ?1 AS other", [other.to_string_lossy()])?;
    let out = merge_attached(conn);
    let _ = conn.execute_batch("DETACH DATABASE other");
    out
}

fn merge_attached(conn: &Connection) -> Result<Merged> {
    let has = |table: &str| -> Result<bool> {
        Ok(conn.query_row("SELECT COUNT(*) FROM other.sqlite_master WHERE type = 'table' AND name = ?", [table], |r| r.get::<_, i64>(0))?
            > 0)
    };
    if !has("predictions")? || !has("duels")? {
        return Err(crate::Error::Parse("that file: it isn't an Augur backup".into()));
    }
    let count = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0)).map(|n| n as usize);
    let mut m = Merged::default();
    let tx = conn.unchecked_transaction()?;
    let before = (count("SELECT COUNT(*) FROM duels")?, count("SELECT COUNT(DISTINCT duel_id) FROM hits")?);
    let duel_cols = "id, fought_at, home_owner, away_owner, home_pal, away_pal, winner, ko, winner_hp_left, home_casts, away_casts";
    tx.execute(&format!("INSERT OR IGNORE INTO duels ({duel_cols}) SELECT {duel_cols} FROM other.duels"), [])?;
    // tapes for duels stored here without one
    tx.execute(
        "INSERT OR IGNORE INTO hits (duel_id, seq, side, t, move, pct) SELECT duel_id, seq, side, t, move, pct FROM other.hits
         WHERE duel_id NOT IN (SELECT DISTINCT duel_id FROM hits)",
        [],
    )?;
    (m.duels, m.tapes) = (count("SELECT COUNT(*) FROM duels")? - before.0, count("SELECT COUNT(DISTINCT duel_id) FROM hits")? - before.1);
    m.species = tx.execute(
        "INSERT OR IGNORE INTO species (pal, slug, types, power, base_hp, base_atk, base_def)
         SELECT pal, slug, types, power, base_hp, base_atk, base_def FROM other.species",
        [],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO species_moves (pal, name, element, power, unlock_lv) SELECT pal, name, element, power, unlock_lv FROM other.species_moves",
        [],
    )?;
    if has("bets")? {
        m.bets = tx.execute(
            "INSERT OR IGNORE INTO bets (seq, duel_id, home_owner, away_owner, home_pal, away_pal, backed, winner, stake, result, seen_at)
             SELECT seq, duel_id, home_owner, away_owner, home_pal, away_pal, backed, winner, stake, result, seen_at FROM other.bets",
            [],
        )?;
    }
    // calls: the same fight seen at the same moment is the same call
    let stake_cols = conn
        .query_row("SELECT COUNT(*) FROM pragma_table_info('predictions', 'other') WHERE name = 'stake'", [], |r| r.get::<_, i64>(0))?
        > 0;
    let pcols = "seen_at, home_owner, away_owner, home_pal, away_pal, pick, prob, p_stats, p_species, p_h2h, estimated, home_species_w,                  home_species_l, away_species_w, away_species_l, pal_h2h_home, pal_h2h_away, model, status, duel_id";
    let extra = if stake_cols { ", stake, stake_cmd" } else { "" };
    let new: Vec<i64> = tx
        .prepare(
            "SELECT o.id FROM other.predictions o WHERE NOT EXISTS (SELECT 1 FROM predictions p
             WHERE p.home_owner = o.home_owner AND p.away_owner = o.away_owner AND p.seen_at = o.seen_at) ORDER BY o.id",
        )?
        .query_map([], |r| r.get(0))?
        .collect::<Rq<_>>()?;
    for old in new {
        tx.execute(&format!("INSERT INTO predictions ({pcols}{extra}) SELECT {pcols}{extra} FROM other.predictions WHERE id = ?"), [old])?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO fighters (prediction_id, side, pal, types, hp, atk, def, hp_iv, atk_iv, def_iv, passives, hidden_boost, speed)
             SELECT ?1, side, pal, types, hp, atk, def, hp_iv, atk_iv, def_iv, passives, hidden_boost, speed FROM other.fighters WHERE prediction_id = ?2",
            [id, old],
        )?;
        m.calls += 1;
    }
    tx.commit()?;
    Ok(m)
}

/// The open call for this fight, if Augur has made one.
pub fn open_call(conn: &Connection, home_owner: &str, away_owner: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT id FROM predictions WHERE status = 'open' AND home_owner = ?1 AND away_owner = ?2 ORDER BY id DESC LIMIT 1",
            [home_owner, away_owner],
            |r| r.get(0),
        )
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Settings;

    fn fighter(pal: &str) -> CallFighter {
        CallFighter {
            fighter: Fighter {
                pal: pal.into(),
                types: vec!["fire".into()],
                hp: 5000.0,
                atk: 600.0,
                def: 700.0,
                skills: vec![],
                passives: vec![],
                hidden_boost: 1.0,
                speed: 1.0,
            },
            ivs: Default::default(),
        }
    }

    fn call(home: &str, away: &str) -> NewCall {
        NewCall {
            seen_at: 1_000,
            home_owner: home.into(),
            away_owner: away.into(),
            home_pal: "A".into(),
            away_pal: "B".into(),
            pick: "home".into(),
            prob: 0.7,
            p_stats: 0.7,
            p_species: 0.5,
            p_h2h: 0.5,
            estimated: false,
            species_rec: [[1.0, 1.0], [1.0, 1.0]],
            pal_h2h: [0.0, 0.0],
            model: Settings::default(),
            home: fighter("A"),
            away: fighter("B"),
            picks_open: true,
            stats_from: String::new(),
            version: String::new(),
        }
    }

    #[test]
    fn calls_settle_from_recent_results() {
        let conn = open_memory().unwrap();
        let id = add_call(&conn, &call("x", "y")).unwrap();
        assert_eq!(add_call(&conn, &call("x", "y")).unwrap(), id, "re-running the same open fight updates it");
        let recent = vec![Recent { id: 7, home: "x".into(), away: "y".into(), win: "home".into(), at: 2_000 }];
        settle(&conn, &recent, |_| None).unwrap();
        let done = settled(&conn).unwrap();
        assert_eq!(done.len(), 1);
        assert!(done[0].right());
        assert!(done[0].fighters.is_some());
    }

    #[test]
    fn a_backup_merges_into_another_store_once() {
        let dir = std::env::temp_dir().join(format!("augur-backup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("backup.db");
        let mac = crate::seed::store();
        crate::seed::import_bets(&mac);
        backup_to(&mac, &file).unwrap();
        let pc = open_memory().unwrap();
        let m = merge_from(&pc, &file).unwrap();
        assert_eq!((m.calls, m.duels, m.tapes, m.species, m.bets), (19, 20, 1, 1, 13), "the 19 calls' duels and #4702 with its tape");
        assert_eq!(settled(&pc).unwrap().len(), 19, "the calls arrive settled, with their fighters");
        assert!(settled(&pc).unwrap().iter().all(|c| c.fighters.is_some()));
        let again = merge_from(&pc, &file).unwrap();
        assert_eq!((again.calls, again.duels, again.bets), (0, 0, 0), "merging twice adds nothing");
        assert!(merge_from(&pc, &dir.join("missing.db")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn what_the_store_has() {
        let conn = open_memory().unwrap();
        assert!(is_empty(&conn).unwrap());
        let seeded = crate::seed::store();
        assert!(!is_empty(&seeded).unwrap());
        assert!(has_duel(&seeded, 4702).unwrap() && !has_duel(&seeded, 1).unwrap());
        assert!(has_species(&seeded, "bushi-noct").unwrap() && !has_species(&seeded, "lamball").unwrap());
    }

    #[test]
    fn unfought_calls_are_skipped_after_two_later_fights() {
        let conn = open_memory().unwrap();
        add_call(&conn, &call("x", "y")).unwrap();
        let later = |id, at| Recent { id, home: "p".into(), away: "q".into(), win: "home".into(), at };
        settle(&conn, &[later(1, 2_000)], |_| None).unwrap();
        let status: String = conn.query_row("SELECT status FROM predictions", [], |r| r.get(0)).unwrap();
        assert_eq!(status, "open", "one later fight isn't enough: the result may not be recorded yet");
        settle(&conn, &[later(1, 2_000), later(2, 3_000)], |_| None).unwrap();
        let status: String = conn.query_row("SELECT status FROM predictions", [], |r| r.get(0)).unwrap();
        assert_eq!(status, "skipped");
    }

    /// The call that counts is the one showing when picks closed. A re-call at the bell (stats shown) is kept apart:
    /// its real stats go to the fighters, and the stake stands.
    #[test]
    fn a_call_stands_once_picks_close() {
        let conn = open_memory().unwrap();
        let mut first = call("x", "y");
        (first.pick, first.prob) = ("home".into(), 0.557);
        let id = add_call(&conn, &first).unwrap();
        set_call_stake(&conn, id, 600, "!h 600").unwrap();
        // still open: an update replaces it
        first.prob = 0.56;
        assert_eq!(add_call(&conn, &first).unwrap(), id);
        close_call(&conn, "x", "y").unwrap();
        let mut late = call("x", "y");
        (late.pick, late.prob, late.picks_open) = ("away".into(), 0.575, false);
        late.away.fighter.def = 999.0;
        assert_eq!(add_call(&conn, &late).unwrap(), id);
        set_call_stake(&conn, id, 900, "!a 900").unwrap();
        let row: (String, f64, String, String, f64) = conn
            .query_row("SELECT pick, prob, stake_cmd, late_pick, late_prob FROM predictions WHERE id = ?", [id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap();
        assert_eq!(row, ("home".into(), 0.56, "!h 600".into(), "away".into(), 0.575));
        let def: f64 = conn.query_row("SELECT def FROM fighters WHERE prediction_id = ? AND side = 'away'", [id], |r| r.get(0)).unwrap();
        assert_eq!(def, 999.0, "the real stats are kept for learning");

        // a fight first seen after picks closed: logged, learned from, not counted
        let mut seen_late = call("p", "q");
        seen_late.picks_open = false;
        let late_id = add_call(&conn, &seen_late).unwrap();
        let r = |id, home: &str, away: &str| Recent { id, home: home.into(), away: away.into(), win: "home".into(), at: 2_000 };
        settle(&conn, &[r(10, "x", "y"), r(11, "p", "q")], |_| None).unwrap();
        let counted: Vec<i64> = settled(&conn).unwrap().iter().map(|c| c.id).collect();
        let learned: Vec<i64> = settled_all(&conn).unwrap().iter().map(|c| c.id).collect();
        assert_eq!((counted, learned), (vec![id], vec![id, late_id]));
        assert_eq!(settled(&conn).unwrap()[0].late, Some(("away".into(), 0.575)));
    }

    /// A move's real hit size is its mean % per hit against the average move, pulled toward 1 when there are few
    /// hits, and only from tapes before a duel when asked.
    #[test]
    fn hit_sizes_from_tapes() {
        let conn = open_memory().unwrap();
        for (duel, mv, pct) in [(1, "Big", 40), (1, "Small", 10), (2, "Big", 40), (2, "Small", 10), (3, "Big", 90)] {
            conn.execute("INSERT OR IGNORE INTO duels (id, winner) VALUES (?, 'home')", [duel]).unwrap();
            let seq: i64 = conn.query_row("SELECT COUNT(*) FROM hits", [], |r| r.get(0)).unwrap();
            conn.execute(
                "INSERT INTO hits (duel_id, seq, side, t, move, pct) VALUES (?1, ?2, 'home', 1, ?3, ?4)",
                rusqlite::params![duel, seq, mv, pct],
            )
            .unwrap();
        }
        let before = hit_sizes(&conn, Some(3)).unwrap();
        // mean 25% per hit; Big 40% over 2 hits pulled toward 25 by 10 hits' worth = 27.5 / 25
        assert!((before["Big"] - 1.1).abs() < 1e-9 && (before["Small"] - 0.9).abs() < 1e-9, "{before:?}");
        assert!(hit_sizes(&conn, None).unwrap()["Big"] > before["Big"], "duel 3's tape counts once it's allowed");
        let mut moves = vec![Move { name: "Big".into(), el: "neutral".into(), power: 100.0, lv: 1, hit: 1.0 }];
        with_hit_sizes(&mut moves, &before);
        assert_eq!(moves[0].hit, 1.1);
    }
}
