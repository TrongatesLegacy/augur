//! Whether a pal's IVs are worth rerolling, from the duels its species has actually had. Duels are matched by power
//! (over 1,346 duels on 13 Nov the two sides' power correlated 0.85, a median of 5 points apart), so a pal meets the
//! species its own has met: a reroll changes how it does against them, not who they are. You keep the better of the
//! new roll and the old, so a reroll never makes a pal worse; the question is whether what it's likely to add is
//! worth the price, which doubles with each reroll.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::db;
use crate::error::Result as R;
use crate::model::{self, Fighter, Settings, StatLine};
use crate::site::BoxPal;

/// Win-chance points a reroll should add per 1,000 credits to be worth it, and to be worth a look.
const WORTH: f64 = 2.0;
const MAYBE: f64 = 0.75;
/// Below this win chance, with no reroll worth it, it's the species holding the pal back: replace it.
const WEAK: f64 = 0.42;
/// The species' past duels the opponents come from, and the fewest that are enough.
const RECENT: usize = 80;
const ENOUGH: usize = 10;
/// Species within this much power stand in when a species has had too few duels.
const NEAR: i64 = 10;
/// The price assumed when it isn't known yet (a first reroll is 250 to 1,000).
const PRICE_UNKNOWN: u64 = 1000;
/// Steps in the spread of new rolls' win chances.
const BINS: usize = 16;
/// Each IV of a new roll: an even spread over 0 to 100.
const STEPS: [f64; 10] = [5.0, 15.0, 25.0, 35.0, 45.0, 55.0, 65.0, 75.0, 85.0, 95.0];

/// What a reroll would do for one pal.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Advice {
    /// its win chance against the pals it meets, now
    pub now: f64,
    /// on average after one reroll, keeping the better of the two rolls
    pub after: f64,
    /// the share of new rolls that would beat the rolls it has, and its win chance with the best roll there is
    pub better: f64,
    pub best: f64,
    /// how new rolls' win chances spread: the share of rolls in each of `BINS` equal steps from `low` to `high`
    pub spread: Vec<f64>,
    pub low: f64,
    pub high: f64,
    /// the past duels its opponents come from, and whether they're its own species' (or species of its power)
    pub duels: usize,
    pub own_duels: bool,
    pub price: Option<u64>,
    /// "Keep", "Reroll", "Maybe" or "Replace"
    pub tag: String,
}

/// A pal and the pals it meets, ready to score any roll of its IVs.
pub struct Matchups {
    me: Fighter,
    base: [f64; 3],
    ivs: [f64; 3],
    record: [f64; 2],
    /// each opponent species at an average roll, its record, and how often it was met
    opponents: Vec<(Fighter, [f64; 2], f64)>,
    duels: usize,
    own_duels: bool,
}

/// The pal's matchups, or None when the store doesn't know its species well enough (no base stats or moves yet).
pub fn matchups(conn: &Connection, pal: &BoxPal) -> R<Option<Matchups>> {
    let Some((types, base, power)) = species(conn, &pal.pal)? else { return Ok(None) };
    let skills = db::moves_of(conn, &pal.pal)?;
    let stat = |k: &str| pal.stats.get(k).copied().unwrap_or((0.0, 0.0));
    let (now, ivs) = (["HP", "ATK", "DEF"].map(|k| stat(k).0), ["HP", "ATK", "DEF"].map(|k| stat(k).1));
    if skills.is_empty() || now.contains(&0.0) {
        return Ok(None);
    }
    let me = Fighter {
        pal: pal.pal.clone(),
        types: if types.is_empty() { pal.types.clone() } else { types },
        hp: now[0],
        atk: now[1],
        def: now[2],
        skills,
        passives: pal.passives.clone(),
        hidden_boost: 1.0,
        speed: model::speed(&pal.passives),
        pal_rank: None,
    };
    // who its species has met lately; species of its power when that's too few to go on
    let met: Vec<(String, f64)> = conn
        .prepare(
            "SELECT o, COUNT(*) FROM (SELECT CASE WHEN home_pal = ?1 THEN away_pal ELSE home_pal END o FROM duels
             WHERE (home_pal = ?1 OR away_pal = ?1) AND winner IS NOT NULL ORDER BY id DESC LIMIT ?2) GROUP BY o",
        )?
        .query_map(rusqlite::params![pal.pal, RECENT as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as f64)))?
        .collect::<rusqlite::Result<_>>()?;
    let duels = met.iter().map(|(_, n)| *n as usize).sum::<usize>();
    let own_duels = duels >= ENOUGH;
    let pool: Vec<(String, f64)> = if own_duels {
        met
    } else {
        let mine = power.or(pal.power.map(i64::from)).unwrap_or(50);
        conn.prepare("SELECT pal FROM species WHERE pal != ?1 AND power BETWEEN ?2 AND ?3 AND base_hp IS NOT NULL")?
            .query_map(rusqlite::params![pal.pal, mine - NEAR, mine + NEAR], |r| r.get::<_, String>(0))?
            .map(|p| p.map(|p| (p, 1.0)))
            .collect::<rusqlite::Result<_>>()?
    };
    let mut opponents = vec![];
    for (name, weight) in pool {
        if let Some(f) = average_roll(conn, &name)? {
            opponents.push((f, record(conn, &name)?, weight));
        }
    }
    if opponents.is_empty() {
        return Ok(None);
    }
    Ok(Some(Matchups { me, base, ivs, record: record(conn, &pal.pal)?, opponents, duels: if own_duels { duels } else { 0 }, own_duels }))
}

impl Matchups {
    /// Its win chance against the pals it meets with these IVs (its stats now, moved by what the new IVs add).
    pub fn win(&self, s: &Settings, ivs: [f64; 3]) -> f64 {
        let (then, was) = (rolled(self.base, ivs, &self.me.passives), rolled(self.base, self.ivs, &self.me.passives));
        let at = |i: usize, now: f64| if was[i] > 0.0 { now * then[i] / was[i] } else { now };
        let me = Fighter { hp: at(0, self.me.hp), atk: at(1, self.me.atk), def: at(2, self.me.def), ..self.me.clone() };
        let s = s.for_calls();
        let (mut sum, mut weight) = (0.0, 0.0);
        for (opp, rec, w) in &self.opponents {
            sum += w * model::score(&s, &me, opp, [self.record, *rec], [0.0, 0.0]).p_home;
            weight += w;
        }
        sum / weight
    }

    /// Its win chance now, on average after one reroll (keeping the better roll), and whether that's worth `price`.
    pub fn advise(&self, s: &Settings, price: Option<u64>) -> Advice {
        let now = self.win(s, self.ivs);
        let (mut total, mut wins, mut all) = (0.0, 0usize, vec![]);
        for hp in STEPS {
            for atk in STEPS {
                for def in STEPS {
                    let w = self.win(s, [hp, atk, def]);
                    total += w.max(now);
                    wins += usize::from(w > now);
                    all.push(w);
                }
            }
        }
        let rolls = all.len() as f64;
        let (low, high) = all.iter().fold((f64::MAX, f64::MIN), |(lo, hi), w| (lo.min(*w), hi.max(*w)));
        let mut spread = vec![0.0; BINS];
        for w in &all {
            let i = if high > low { (((w - low) / (high - low)) * BINS as f64) as usize } else { 0 };
            spread[i.min(BINS - 1)] += 1.0 / rolls;
        }
        let after = total / rolls;
        let best = self.win(s, [100.0; 3]).max(now);
        // win-chance points per 1,000 credits
        let worth = (after - now) * 100.0 / (price.unwrap_or(PRICE_UNKNOWN) as f64 / 1000.0);
        let tag = if worth >= WORTH {
            "Reroll"
        } else if worth >= MAYBE {
            "Maybe"
        } else if now < WEAK {
            "Replace"
        } else {
            "Keep"
        };
        Advice {
            now,
            after,
            better: wins as f64 / rolls,
            best,
            spread,
            low,
            high,
            duels: self.duels,
            own_duels: self.own_duels,
            price,
            tag: tag.into(),
        }
    }
}

// --- helpers ---

/// A species' types, base HP/ATK/DEF and power.
type Species = (Vec<String>, [f64; 3], Option<i64>);

/// The species as the store has it, if it does.
fn species(conn: &Connection, pal: &str) -> R<Option<Species>> {
    Ok(conn
        .query_row("SELECT types, base_hp, base_atk, base_def, power FROM species WHERE pal = ? AND base_hp IS NOT NULL", [pal], |r| {
            Ok((
                serde_json::from_str(&r.get::<_, Option<String>>(0)?.unwrap_or_default()).unwrap_or_default(),
                [r.get(1)?, r.get(2)?, r.get(3)?],
                r.get(4)?,
            ))
        })
        .optional()?)
}

/// A species at an average roll, as the past-duel test sees it.
fn average_roll(conn: &Connection, pal: &str) -> R<Option<Fighter>> {
    let Some((types, base, _)) = species(conn, pal)? else { return Ok(None) };
    let skills = db::moves_of(conn, pal)?;
    if skills.is_empty() {
        return Ok(None);
    }
    let v = rolled(base, [model::ASSUMED_IV; 3], &[]);
    Ok(Some(Fighter {
        pal: pal.into(),
        types,
        hp: v[0],
        atk: v[1],
        def: v[2],
        skills,
        passives: vec![],
        hidden_boost: 1.0,
        speed: 1.0,
        pal_rank: None,
    }))
}

/// Lv 70 HP, ATK and DEF from base stats and IVs (with stat passives), by the same fit as hidden stats.
fn rolled(base: [f64; 3], ivs: [f64; 3], passives: &[String]) -> [f64; 3] {
    let stats: HashMap<String, StatLine> = ["HP", "ATK", "DEF"]
        .iter()
        .enumerate()
        .map(|(i, k)| (k.to_string(), StatLine { base: Some(base[i]), iv: Some(ivs[i]), lv70: None }))
        .collect();
    model::resolve(&stats, passives).0
}

/// A species' wins and losses in the store.
fn record(conn: &Connection, pal: &str) -> R<[f64; 2]> {
    let (w, n): (i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM((home_pal = ?1 AND winner = 'home') OR (away_pal = ?1 AND winner = 'away')), 0), COUNT(*) FROM duels
         WHERE (home_pal = ?1 OR away_pal = ?1) AND winner IS NOT NULL",
        [pal],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok([w as f64, (n - w) as f64])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seed's store (19 calls' duels, one species with base stats and moves) with a few made-up duels for a
    /// pal of that species, and that pal in a box.
    fn seeded(ivs: [f64; 3]) -> (Connection, BoxPal) {
        let conn = crate::seed::store();
        let pal: String = conn.query_row("SELECT pal FROM species WHERE base_hp IS NOT NULL LIMIT 1", [], |r| r.get(0)).unwrap();
        // it has met itself (the only species the seed knows in full) a dozen times
        for i in 0..12 {
            conn.execute(
                "INSERT INTO duels (id, home_pal, away_pal, winner) VALUES (?1, ?2, ?2, ?3)",
                rusqlite::params![900_000 + i, pal, if i % 2 == 0 { "home" } else { "away" }],
            )
            .unwrap();
        }
        let base = species(&conn, &pal).unwrap().unwrap().1;
        let now = rolled(base, ivs, &[]);
        let me = BoxPal {
            pal: pal.clone(),
            stats: ["HP", "ATK", "DEF"].iter().enumerate().map(|(i, k)| (k.to_string(), (now[i], ivs[i]))).collect(),
            ..Default::default()
        };
        (conn, me)
    }

    #[test]
    fn a_reroll_helps_a_poor_roll_more_than_a_good_one() {
        let s = Settings::default();
        let (conn, poor) = seeded([10.0, 10.0, 10.0]);
        let (_, good) = seeded([90.0, 90.0, 90.0]);
        let (poor, good) = (matchups(&conn, &poor).unwrap().unwrap(), matchups(&conn, &good).unwrap().unwrap());
        let (p, g) = (poor.advise(&s, Some(250)), good.advise(&s, Some(250)));
        assert!(p.own_duels && p.duels >= 12, "its own species' duels: {p:?}");
        assert!(p.after > p.now && g.after >= g.now, "keeping the better roll never makes it worse");
        assert!(p.after - p.now > g.after - g.now, "{p:?} vs {g:?}");
        assert!(g.now > p.now, "better IVs win more");
        assert_eq!(p.tag, "Reroll", "{p:?}");
        assert!(p.better > 0.8 && g.better < 0.1, "most new rolls beat a poor one, few a good one: {p:?} vs {g:?}");
        assert!(p.best >= p.after && g.best >= g.after);
        assert!((p.spread.iter().sum::<f64>() - 1.0).abs() < 1e-9 && p.spread.len() == BINS, "every roll is in the spread");
        assert!(p.low < p.now && p.now < p.high);
    }

    #[test]
    fn the_price_decides_if_its_worth_it() {
        let s = Settings::default();
        let (conn, pal) = seeded([40.0, 40.0, 40.0]);
        let m = matchups(&conn, &pal).unwrap().unwrap();
        let (cheap, dear) = (m.advise(&s, Some(250)), m.advise(&s, Some(64_000)));
        assert_eq!((cheap.now, cheap.after), (dear.now, dear.after), "the price doesn't change the odds");
        assert_ne!(cheap.tag, dear.tag, "only whether they're worth it: {cheap:?} vs {dear:?}");
        assert!(["Keep", "Replace"].contains(&dear.tag.as_str()));
    }

    #[test]
    fn an_unknown_species_has_no_advice() {
        let conn = crate::seed::store();
        let pal = BoxPal { pal: "Nobody".into(), ..Default::default() };
        assert!(matchups(&conn, &pal).unwrap().is_none());
    }
}
