//! What the store says: the tally of real calls, calibration, re-scoring with other settings (tuning), the history
//! backtest on every past duel, and the per-species and per-player views the screens show.

use crate::db::{self, CallRecord};
use crate::model::{self, Fighter, Settings, StatLine};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::Result as R;
/// rusqlite's own result, for row closures and collecting rows.
type Rq<T> = rusqlite::Result<T>;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Tally {
    pub right: usize,
    pub total: usize,
    pub best_run: usize,
    pub current_run: usize,
    pub avg_confidence: f64,
    /// (label, won, total, promised) per confidence band
    pub calibration: Vec<(String, usize, usize, f64)>,
    /// (date, right, total) per stream day
    pub by_day: Vec<(String, usize, usize)>,
    pub biggest_miss: Option<CallRecord>,
}

/// The record of real calls, optionally only those on or after `since` (an ISO date).
pub fn tally(calls: &[CallRecord], since: Option<&str>) -> Tally {
    let calls: Vec<&CallRecord> = calls.iter().filter(|c| since.is_none_or(|s| c.fought_at.as_deref().unwrap_or("") >= s)).collect();
    let mut t = Tally { total: calls.len(), ..Default::default() };
    let mut run = 0;
    for c in &calls {
        if c.right() {
            t.right += 1;
            run += 1;
            t.best_run = t.best_run.max(run);
        } else {
            run = 0;
        }
    }
    t.current_run = run;
    if !calls.is_empty() {
        t.avg_confidence = calls.iter().map(|c| c.prob).sum::<f64>() / calls.len() as f64;
    }
    for (label, lo, hi) in [("80%+", 0.8, 1.01), ("70–79%", 0.7, 0.8), ("60–69%", 0.6, 0.7), ("50–59%", 0.0, 0.6)] {
        let band: Vec<&&CallRecord> = calls.iter().filter(|c| c.prob >= lo && c.prob < hi).collect();
        if !band.is_empty() {
            let promised = band.iter().map(|c| c.prob).sum::<f64>() / band.len() as f64;
            t.calibration.push((label.into(), band.iter().filter(|c| c.right()).count(), band.len(), promised));
        }
    }
    let mut days: Vec<(String, usize, usize)> = vec![];
    for c in &calls {
        let day = c.fought_at.as_deref().unwrap_or("").get(..10).unwrap_or("").to_string();
        match days.last_mut() {
            Some(d) if d.0 == day => {
                d.1 += c.right() as usize;
                d.2 += 1;
            }
            _ => days.push((day, c.right() as usize, 1)),
        }
    }
    t.by_day = days;
    t.biggest_miss = calls.iter().filter(|c| !c.right()).max_by(|a, b| a.prob.total_cmp(&b.prob)).map(|c| (*c).clone());
    t
}

/// A past call re-run with other settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rescored {
    pub call: CallRecord,
    pub pick_now: String,
    pub prob_now: f64,
}

pub fn rescore(calls: &[CallRecord], s: &Settings) -> Vec<Rescored> {
    calls
        .iter()
        .filter_map(|c| {
            let (h, a) = c.fighters.as_ref()?;
            let p = model::score(s, &h.fighter, &a.fighter, c.species_rec?, c.pal_h2h).p_home;
            Some(Rescored { call: c.clone(), pick_now: if p >= 0.5 { "home" } else { "away" }.into(), prob_now: p.max(1.0 - p) })
        })
        .collect()
}

/// Log loss (lower is better; it punishes confident misses) and picks right, for settings on the calls with fighters.
pub fn evaluate(calls: &[CallRecord], s: &Settings) -> (f64, usize, usize) {
    let r = rescore(calls, s);
    let mut loss = 0.0;
    let mut right = 0;
    for x in &r {
        let p_winner = if Some(x.pick_now.as_str()) == x.call.winner.as_deref() { x.prob_now } else { 1.0 - x.prob_now };
        loss -= p_winner.clamp(1e-6, 1.0 - 1e-6).ln();
        right += (p_winner > 0.5) as usize;
    }
    (if r.is_empty() { 0.0 } else { loss / r.len() as f64 }, right, r.len())
}

/// Every combination of `grid` values, best (lowest log loss) first. Settings not in the grid stay as in `base`.
pub fn tune(calls: &[CallRecord], base: &Settings, grid: &[(&str, Vec<f64>)]) -> Vec<(f64, usize, Settings)> {
    let mut combos = vec![base.clone()];
    for (key, values) in grid {
        combos = combos
            .into_iter()
            .flat_map(|c| {
                values.iter().map(move |v| {
                    let mut c = c.clone();
                    match *key {
                        "TYPE_BONUS" => c.type_bonus = *v,
                        "TYPE_RESIST" => c.type_resist = *v,
                        "POWER_EXP" => c.power_exp = *v,
                        "STATS_STEEPNESS" => c.stats_steepness = *v,
                        "SPECIES_WEIGHT" => c.species_weight = *v,
                        "PAL_H2H_WEIGHT" => c.pal_h2h_weight = *v,
                        _ => {}
                    }
                    c
                })
            })
            .collect();
    }
    let mut out: Vec<(f64, usize, Settings)> = combos
        .into_iter()
        .map(|s| {
            let (loss, right, _) = evaluate(calls, &s);
            (loss, right, s)
        })
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

/// The per-hit calibration fit on the real calls' fight tapes: (moves sampled, fights, power exponent, type edge).
pub fn calibration(conn: &Connection, calls: &[CallRecord], s: &Settings) -> R<Option<(usize, usize, f64, f64)>> {
    let mut samples = vec![];
    let mut fights = 0;
    for c in calls {
        let (Some((h, a)), Some(id)) = (&c.fighters, c.duel_id) else { continue };
        let Some(duel) = db::get_duel(conn, id)? else { continue };
        fights += 1;
        let mut peak: HashMap<(String, String), f64> = HashMap::new();
        for hit in &duel.hits {
            let e = peak.entry((hit.side.clone(), hit.mv.clone())).or_insert(0.0);
            *e = e.max(hit.pct);
        }
        for ((side, mv), pct) in peak {
            let (att, dfn) = if side == "home" { (&h.fighter, &a.fighter) } else { (&a.fighter, &h.fighter) };
            if let Some(m) = att.skills.iter().find(|m| m.name == mv) {
                samples.push((m.power, model::type_mult(s, &m.el, &dfn.types), pct, att.atk, dfn.def));
            }
        }
    }
    Ok(model::calibrate(s, &samples).map(|(pa, edge)| (samples.len(), fights, pa, edge)))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Backtest {
    pub duels: usize,
    pub log_loss: f64,
    pub accuracy: f64,
}

/// The model on every past duel, with fighters rebuilt from species base stats at an average roll and species
/// records and pal head-to-heads as they stood before each duel. No IVs, passives or hidden moves, so it tests
/// types, move strength and records, not the stat settings.
pub fn backtest(conn: &Connection, s: &Settings) -> R<Backtest> {
    let mut species: HashMap<String, (Vec<String>, [f64; 3])> = HashMap::new();
    for row in
        conn.prepare("SELECT pal, types, base_hp, base_atk, base_def FROM species WHERE base_hp IS NOT NULL")?.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, [r.get::<_, f64>(2)?, r.get::<_, f64>(3)?, r.get::<_, f64>(4)?]))
        })?
    {
        let (pal, types, base) = row?;
        species.insert(pal, (serde_json::from_str(&types.unwrap_or_default()).unwrap_or_default(), base));
    }
    let mut moves: HashMap<String, Vec<model::Move>> = HashMap::new();
    for pal in species.keys() {
        moves.insert(pal.clone(), db::moves_of(conn, pal)?);
    }
    let duels: Vec<(String, String, String)> = conn
        .prepare("SELECT home_pal, away_pal, winner FROM duels WHERE winner IS NOT NULL ORDER BY id")?
        .query_map([], |r| {
            Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get::<_, Option<String>>(1)?.unwrap_or_default(), r.get(2)?))
        })?
        .collect::<Rq<_>>()?;
    let fighter = |pal: &str| -> Option<Fighter> {
        let (types, base) = species.get(pal)?;
        let skills = moves.get(pal).filter(|m| !m.is_empty())?.clone();
        let stats: HashMap<String, StatLine> = ["HP", "ATK", "DEF"]
            .iter()
            .enumerate()
            .map(|(i, k)| (k.to_string(), StatLine { base: Some(base[i]), ..Default::default() }))
            .collect();
        let (v, _) = model::resolve(&stats, &[]);
        Some(Fighter {
            pal: pal.into(),
            types: types.clone(),
            hp: v[0],
            atk: v[1],
            def: v[2],
            skills,
            passives: vec![],
            hidden_boost: 1.0,
            speed: 1.0,
        })
    };
    let mut rec: HashMap<String, [f64; 2]> = HashMap::new();
    let mut h2h: HashMap<(String, String), [f64; 2]> = HashMap::new();
    let (mut loss, mut right, mut n) = (0.0, 0usize, 0usize);
    for (hp, ap, win) in &duels {
        if let (Some(h), Some(a)) = (fighter(hp), fighter(ap)) {
            let key = if hp <= ap { (hp.clone(), ap.clone()) } else { (ap.clone(), hp.clone()) };
            let pair = h2h.get(&key).copied().unwrap_or([0.0; 2]);
            let pair = if &key.0 == hp { pair } else { [pair[1], pair[0]] };
            let p =
                model::score(s, &h, &a, [rec.get(hp).copied().unwrap_or([0.0; 2]), rec.get(ap).copied().unwrap_or([0.0; 2])], pair).p_home;
            let q = if win == "home" { p } else { 1.0 - p };
            loss -= q.clamp(1e-6, 1.0 - 1e-6).ln();
            right += (q > 0.5) as usize;
            n += 1;
        }
        let (w, l) = if win == "home" { (hp, ap) } else { (ap, hp) };
        rec.entry(w.clone()).or_insert([0.0; 2])[0] += 1.0;
        rec.entry(l.clone()).or_insert([0.0; 2])[1] += 1.0;
        let key = if w <= l { (w.clone(), l.clone()) } else { (l.clone(), w.clone()) };
        h2h.entry(key.clone()).or_insert([0.0; 2])[if &key.0 == w { 0 } else { 1 }] += 1.0;
    }
    Ok(Backtest {
        duels: n,
        log_loss: if n > 0 { loss / n as f64 } else { 0.0 },
        accuracy: if n > 0 { right as f64 / n as f64 } else { 0.0 },
    })
}

// --- species ---

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SpeciesRecord {
    pub pal: String,
    pub slug: String,
    pub wins: u32,
    pub duels: u32,
}

/// Every species with its record, most duels first.
pub fn species_records(conn: &Connection) -> R<Vec<SpeciesRecord>> {
    Ok(conn.prepare(
        "WITH s AS (SELECT home_pal p, winner = 'home' w FROM duels WHERE winner IS NOT NULL
                    UNION ALL SELECT away_pal, winner = 'away' FROM duels WHERE winner IS NOT NULL)
         SELECT p, COALESCE((SELECT slug FROM species WHERE pal = p), ''), SUM(w), COUNT(*) FROM s WHERE p IS NOT NULL GROUP BY p ORDER BY COUNT(*) DESC",
    )?
    .query_map([], |r| Ok(SpeciesRecord { pal: r.get(0)?, slug: r.get(1)?, wins: r.get(2)?, duels: r.get(3)? }))?
    .collect::<Rq<_>>()?)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MoveStats {
    pub name: String,
    pub element: Option<String>,
    pub listed_power: Option<f64>,
    pub hits: u32,
    pub avg: f64,
    pub best: f64,
    pub unlisted: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SpeciesDetail {
    pub species: Option<db::SpeciesRow>,
    pub wins: u32,
    pub duels: u32,
    pub fastest_ko: Option<String>,
    pub tapes: u32,
    pub unlisted_share: f64,
    pub moves: Vec<MoveStats>,
    /// (YYYY-MM, wins, duels)
    pub months: Vec<(String, u32, u32)>,
    /// (owner, wins, duels)
    pub owners: Vec<(String, u32, u32)>,
}

pub fn species_detail(conn: &Connection, pal: &str) -> R<SpeciesDetail> {
    let mine = "((home_pal = ?1 AND winner = 'home') OR (away_pal = ?1 AND winner = 'away'))";
    let (wins, duels): (u32, u32) = conn.query_row(
        &format!("SELECT COALESCE(SUM({mine}), 0), COUNT(*) FROM duels WHERE (home_pal = ?1 OR away_pal = ?1) AND winner IS NOT NULL"),
        [pal],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let fastest_ko: Option<String> = conn.query_row(
        &format!("SELECT ko FROM duels WHERE {mine} AND ko IS NOT NULL ORDER BY CAST(substr(ko, 1, instr(ko, ':') - 1) AS INTEGER) * 60 + CAST(substr(ko, instr(ko, ':') + 1) AS INTEGER) LIMIT 1"),
        [pal], |r| r.get(0)).ok();
    let listed = db::moves_of(conn, pal)?;
    let mut st = conn.prepare(
        "SELECT h.move, COUNT(*), AVG(h.pct), MAX(h.pct), SUM(h.pct) FROM hits h JOIN duels d ON d.id = h.duel_id
         WHERE (h.side = 'home' AND d.home_pal = ?1) OR (h.side = 'away' AND d.away_pal = ?1) GROUP BY h.move ORDER BY SUM(h.pct) DESC",
    )?;
    let rows: Vec<(String, u32, f64, f64, f64)> = st
        .query_map([pal], |r| Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
        .collect::<Rq<_>>()?;
    let total: f64 = rows.iter().map(|r| r.4).sum();
    let hidden: f64 = rows.iter().filter(|r| !listed.iter().any(|m| m.name == r.0)).map(|r| r.4).sum();
    let moves = rows
        .iter()
        .map(|(name, n, avg, best, _)| {
            let m = listed.iter().find(|m| &m.name == name);
            MoveStats {
                name: if name.is_empty() { "(unnamed hits)".into() } else { name.clone() },
                element: m.map(|m| m.el.clone()),
                listed_power: m.map(|m| m.power),
                hits: *n,
                avg: *avg,
                best: *best,
                unlisted: m.is_none(),
            }
        })
        .collect();
    let tapes: u32 = conn.query_row(
        "SELECT COUNT(DISTINCT d.id) FROM duels d JOIN hits h ON h.duel_id = d.id WHERE d.home_pal = ?1 OR d.away_pal = ?1",
        [pal],
        |r| r.get(0),
    )?;
    let months = conn
        .prepare(&format!("SELECT substr(fought_at, 1, 7) m, SUM({mine}), COUNT(*) FROM duels WHERE (home_pal = ?1 OR away_pal = ?1) AND winner IS NOT NULL GROUP BY m ORDER BY m"))?
        .query_map([pal], |r| Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get(1)?, r.get(2)?)))?
        .collect::<Rq<_>>()?;
    let owners = conn
        .prepare(&format!("SELECT CASE WHEN home_pal = ?1 THEN home_owner ELSE away_owner END o, SUM({mine}), COUNT(*) FROM duels WHERE (home_pal = ?1 OR away_pal = ?1) AND winner IS NOT NULL GROUP BY o ORDER BY COUNT(*) DESC LIMIT 8"))?
        .query_map([pal], |r| Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get(1)?, r.get(2)?)))?
        .collect::<Rq<_>>()?;
    Ok(SpeciesDetail {
        species: db::species_row(conn, pal)?,
        wins,
        duels,
        fastest_ko,
        tapes,
        unlisted_share: if total > 0.0 { hidden / total } else { 0.0 },
        moves,
        months,
        owners,
    })
}

/// A species that does much of its damage with moves its listed moveset doesn't have.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UnlistedLeader {
    pub pal: String,
    /// share of its tape damage from unlisted moves
    pub share: f64,
    /// its top unlisted moves: (move, total damage %)
    pub moves: Vec<(String, f64)>,
}

/// Damage totals per species: (all damage, unlisted damage, unlisted moves).
type DamageSplit = (f64, f64, Vec<(String, f64)>);

/// Species doing the most damage with moves their listed moveset doesn't have.
pub fn unlisted_leaders(conn: &Connection, min_damage: f64, top: usize) -> R<Vec<UnlistedLeader>> {
    let mut st = conn.prepare(
        "SELECT CASE WHEN h.side = 'home' THEN d.home_pal ELSE d.away_pal END pal, h.move, SUM(h.pct)
         FROM hits h JOIN duels d ON d.id = h.duel_id GROUP BY pal, h.move",
    )?;
    let rows: Vec<(String, String, f64)> = st
        .query_map([], |r| {
            Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get::<_, Option<String>>(1)?.unwrap_or_default(), r.get(2)?))
        })?
        .collect::<Rq<_>>()?;
    let mut listed: HashMap<String, Vec<String>> = HashMap::new();
    for row in
        conn.prepare("SELECT pal, name FROM species_moves")?.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
    {
        let (p, n) = row?;
        listed.entry(p).or_default().push(n);
    }
    let mut by_pal: HashMap<String, DamageSplit> = HashMap::new();
    for (pal, mv, dmg) in rows {
        let Some(l) = listed.get(&pal) else { continue };
        let e = by_pal.entry(pal).or_default();
        e.0 += dmg;
        if !l.contains(&mv) {
            e.1 += dmg;
            e.2.push((if mv.is_empty() { "(unnamed)".into() } else { mv }, dmg));
        }
    }
    let mut out: Vec<UnlistedLeader> = by_pal
        .into_iter()
        .filter(|(_, v)| v.0 >= min_damage && v.1 > 0.0)
        .map(|(p, (total, hidden, mut mv))| {
            mv.sort_by(|a, b| b.1.total_cmp(&a.1));
            mv.truncate(3);
            UnlistedLeader { pal: p, share: hidden / total, moves: mv }
        })
        .collect();
    out.sort_by(|a, b| b.share.total_cmp(&a.share));
    out.truncate(top);
    Ok(out)
}

// --- players ---

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlayerDetail {
    pub owner: String,
    pub wins: u32,
    pub duels: u32,
    /// (rival, wins, meetings)
    pub rivals: Vec<(String, u32, u32)>,
    /// (pal, wins, duels)
    pub pals_used: Vec<(String, u32, u32)>,
}

pub fn player(conn: &Connection, owner: &str) -> R<PlayerDetail> {
    let mine = "((home_owner = ?1 COLLATE NOCASE AND winner = 'home') OR (away_owner = ?1 COLLATE NOCASE AND winner = 'away'))";
    let side = "(home_owner = ?1 COLLATE NOCASE OR away_owner = ?1 COLLATE NOCASE) AND winner IS NOT NULL";
    let (wins, duels): (u32, u32) =
        conn.query_row(&format!("SELECT COALESCE(SUM({mine}), 0), COUNT(*) FROM duels WHERE {side}"), params![owner], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    let rivals = conn
        .prepare(&format!("SELECT CASE WHEN home_owner = ?1 COLLATE NOCASE THEN away_owner ELSE home_owner END r, SUM({mine}), COUNT(*) FROM duels WHERE {side} GROUP BY r ORDER BY COUNT(*) DESC LIMIT 6"))?
        .query_map(params![owner], |r| Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get(1)?, r.get(2)?)))?
        .collect::<Rq<_>>()?;
    let pals_used = conn
        .prepare(&format!("SELECT CASE WHEN home_owner = ?1 COLLATE NOCASE THEN home_pal ELSE away_pal END p, SUM({mine}), COUNT(*) FROM duels WHERE {side} GROUP BY p ORDER BY COUNT(*) DESC LIMIT 8"))?
        .query_map(params![owner], |r| Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get(1)?, r.get(2)?)))?
        .collect::<Rq<_>>()?;
    Ok(PlayerDetail { owner: owner.into(), wins, duels, rivals, pals_used })
}

/// A pal in a box, rated against a typical opponent on stats alone (no types, no records): the chance it beats
/// a fighter with the median stats Augur has seen and the same moves.
pub fn box_rating(conn: &Connection, s: &Settings, pal: &str, hp: f64, atk: f64, def: f64, passives: &[String]) -> R<f64> {
    let median = |col: &str| -> R<f64> {
        let mut v: Vec<f64> =
            conn.prepare(&format!("SELECT {col} FROM fighters"))?.query_map([], |r| r.get::<_, f64>(0))?.collect::<Rq<_>>()?;
        v.sort_by(f64::total_cmp);
        Ok(if v.is_empty() { 0.0 } else { v[v.len() / 2] })
    };
    let skills: Vec<model::Move> = db::moves_of(conn, pal)?
        .into_iter()
        .map(|mut m| {
            m.el = "none".into();
            m
        })
        .collect();
    let me = Fighter {
        pal: pal.into(),
        types: vec![],
        hp,
        atk,
        def,
        skills: skills.clone(),
        passives: passives.to_vec(),
        hidden_boost: 1.0,
        speed: model::speed(passives),
    };
    let (mh, ma, md) = (median("hp")?, median("atk")?, median("def")?);
    if mh == 0.0 {
        return Ok(0.5);
    }
    let typical =
        Fighter { pal: "typical".into(), types: vec![], hp: mh, atk: ma, def: md, skills, passives: vec![], hidden_boost: 1.0, speed: 1.0 };
    Ok(model::score(s, &me, &typical, [[0.0; 2]; 2], [0.0; 2]).p_stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tally_and_tuning_on_the_fixture() {
        let conn = crate::seed::store();
        let calls = db::settled(&conn).unwrap();
        let t = tally(&calls, None);
        assert_eq!((t.right, t.total, t.best_run), (14, 19, 8));
        let (_, right, n) = evaluate(&calls, &Settings::default());
        assert_eq!(n, 19);
        assert_eq!(right, 14, "the seed's calls were made with today's settings, so re-scoring them changes nothing");
        let best = tune(&calls, &Settings::default(), &[("TYPE_BONUS", vec![1.0, 1.5, 2.0])]);
        assert_eq!(best.len(), 3);
    }
}
