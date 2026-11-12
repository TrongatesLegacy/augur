//! The prediction model. The settings tune themselves (`learn`); `Settings::default()` is where a new install
//! starts, each with a note of what it was first fitted on (see the tune-model skill). The parity test
//! (`tests/parity.rs`) pins its numbers on every call in `fixtures/calls.jsonl`.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

/// Rough Lv 70 stat fits from observed duels: stat = m + k * base * (1 + 0.003 * IV). Used only when the site
/// hides a side's stats until the bell and its owner's box doesn't show them either.
const FIT: [(&str, f64, f64); 3] = [("HP", 979.0, 48.5), ("ATK", 98.0, 5.27), ("DEF", 136.0, 6.65)];
pub const ASSUMED_IV: f64 = 50.0;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct Settings {
    /// 4,702 past duels prefer 1.25-1.5 (base-stat backtest); per-hit tapes ~1.15x; 19 live fights 2.0 by a hair
    pub type_bonus: f64,
    /// resisted moves do 1/1.5 damage; Silvegis's dragon moves did <=7% into Llamatodd's ice (14-fight fit)
    pub type_resist: f64,
    /// damage ~ power ** POWER_EXP (tapes: 40-power moves can hit as hard as 450-power ones)
    pub power_exp: f64,
    /// time-to-KO ratio -> win chance; 4 gave 93% on Eidrolon, who lost (15-fight fit)
    pub stats_steepness: f64,
    /// how much the species win/loss records shift the stat model, on the log-odds scale
    pub species_weight: f64,
    /// same, for past fights between these two species (shrunk hard: (w+2)/(n+4))
    pub pal_h2h_weight: f64,
    /// how much a cooldown passive's extra casts count: 1 = in full, 0.5 = square root (the guess until live calls
    /// with cooldown passives say otherwise)
    #[serde(default = "half")]
    pub cooldown_exp: f64,
}

fn half() -> f64 {
    0.5
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            type_bonus: 1.5,
            type_resist: 1.5,
            power_exp: 0.35,
            stats_steepness: 3.0,
            species_weight: 1.2,
            pal_h2h_weight: 0.5,
            cooldown_exp: 0.5,
        }
    }
}

impl Settings {
    /// Every setting's key, as settings.json and `augur tune` name them.
    pub const KEYS: [&'static str; 7] =
        ["TYPE_BONUS", "TYPE_RESIST", "POWER_EXP", "STATS_STEEPNESS", "SPECIES_WEIGHT", "PAL_H2H_WEIGHT", "COOLDOWN_EXP"];

    pub fn get(&self, key: &str) -> Option<f64> {
        Some(match key {
            "TYPE_BONUS" => self.type_bonus,
            "TYPE_RESIST" => self.type_resist,
            "POWER_EXP" => self.power_exp,
            "STATS_STEEPNESS" => self.stats_steepness,
            "SPECIES_WEIGHT" => self.species_weight,
            "PAL_H2H_WEIGHT" => self.pal_h2h_weight,
            "COOLDOWN_EXP" => self.cooldown_exp,
            _ => return None,
        })
    }

    /// A copy with one setting changed (an unknown key changes nothing).
    pub fn with(&self, key: &str, v: f64) -> Settings {
        let mut c = self.clone();
        match key {
            "TYPE_BONUS" => c.type_bonus = v,
            "TYPE_RESIST" => c.type_resist = v,
            "POWER_EXP" => c.power_exp = v,
            "STATS_STEEPNESS" => c.stats_steepness = v,
            "SPECIES_WEIGHT" => c.species_weight = v,
            "PAL_H2H_WEIGHT" => c.pal_h2h_weight = v,
            "COOLDOWN_EXP" => c.cooldown_exp = v,
            _ => {}
        }
        c
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Move {
    pub name: String,
    pub el: String,
    pub power: f64,
    #[serde(default = "one")]
    pub lv: u32,
}

fn one() -> u32 {
    1
}

/// One side of a fight as the model sees it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Fighter {
    pub pal: String,
    pub types: Vec<String>,
    #[serde(rename = "HP")]
    pub hp: f64,
    #[serde(rename = "ATK")]
    pub atk: f64,
    #[serde(rename = "DEF")]
    pub def: f64,
    pub skills: Vec<Move>,
    #[serde(default)]
    pub passives: Vec<String>,
    #[serde(default = "unit")]
    pub hidden_boost: f64,
    #[serde(default = "unit")]
    pub speed: f64,
}

fn unit() -> f64 {
    1.0
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Score {
    pub t_home: f64,
    pub t_away: f64,
    pub p_stats: f64,
    pub p_species: f64,
    pub p_h2h: f64,
    pub p_home: f64,
    /// each species' record, shrunk toward 50%: [home, away]
    pub rates: [f64; 2],
}

/// Palworld type chart: attacker element -> the element it beats.
fn beats(el: &str) -> Option<&'static str> {
    Some(match el {
        "fire" => "grass",
        "water" => "fire",
        "electric" => "water",
        "grass" => "ground",
        "ground" => "electric",
        "ice" => "dragon",
        "dragon" => "dark",
        "dark" => "neutral",
        _ => return None,
    })
}

/// Fire also beats ice.
fn beats_ice(el: &str) -> Option<&'static str> {
    (el == "fire").then_some("ice")
}

/// Damage multiplier for a move against a defender's types. The beaten type takes TYPE_BONUS, a move into a type
/// that beats it does 1/TYPE_RESIST. Dual types multiply.
pub fn type_mult(s: &Settings, move_el: &str, defender_types: &[String]) -> f64 {
    let mut m = 1.0;
    for t in defender_types {
        let t = t.as_str();
        if beats(move_el) == Some(t) || beats_ice(move_el) == Some(t) {
            m *= s.type_bonus;
        } else if beats(t) == Some(move_el) || beats_ice(t) == Some(move_el) {
            m /= s.type_resist;
        }
    }
    m
}

/// Cast-rate multiplier from cooldown passives: "COOLDOWN −30%" means ~1/(1-0.3) as many casts. Damped (square
/// root) here; `score` re-weights it with the learned `cooldown_exp`.
pub fn speed(passives: &[String]) -> f64 {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?i)cooldown\s*([+\-−])(\d+)%").unwrap());
    let mut m = 1.0;
    for p in passives {
        if let Some(c) = re.captures(p) {
            let pct: f64 = c[2].parse().unwrap_or(0.0);
            let cut = pct / 100.0 * if &c[1] == "+" { -1.0 } else { 1.0 };
            m /= (1.0 - cut).max(0.3);
        }
    }
    m.powf(0.5)
}

/// Damage the listed moveset can't explain (share of recent damage from unlisted moves), scaled back in.
/// Damped: a full boost made Bellanoir Libero 83% in a fight it lost, and every pal gets ~5% from
/// Provocation/damage over time anyway.
pub fn hidden_boost(share: f64) -> f64 {
    (1.0 / (1.0 - (share - 0.05).clamp(0.0, 0.5))).powf(0.5)
}

/// A stat as the site shows it: the talent roll (IV), the species base and the final Lv 70 value.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct StatLine {
    pub iv: Option<f64>,
    pub base: Option<f64>,
    pub lv70: Option<f64>,
}

/// Fill in Lv 70 stats, estimating hidden ones. Returns ([HP, ATK, DEF], estimated?).
pub fn resolve(stats: &HashMap<String, StatLine>, passives: &[String]) -> ([f64; 3], bool) {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?i)(attack|atk|defen[cs]e|def|hp)\s*([+\-−]\d+)%").unwrap());
    let mut vals = [0.0; 3];
    let mut est = false;
    for (i, (k, m0, kk)) in FIT.iter().enumerate() {
        let s = stats.get(*k).cloned().unwrap_or_default();
        if let Some(v) = s.lv70.filter(|v| *v != 0.0) {
            vals[i] = v;
            continue;
        }
        est = true;
        let iv = s.iv.unwrap_or(ASSUMED_IV);
        let mut v = m0 + kk * s.base.filter(|b| *b != 0.0).unwrap_or(80.0) * (1.0 + 0.003 * iv);
        for p in passives {
            // shown stats already include passives; estimates don't
            if let Some(c) = re.captures(p) {
                let which = match c[1].to_lowercase().as_str() {
                    "attack" | "atk" => "ATK",
                    "hp" => "HP",
                    _ => "DEF",
                };
                if which == *k {
                    v *= 1.0 + c[2].replace('−', "-").parse::<f64>().unwrap_or(0.0) / 100.0;
                }
            }
        }
        vals[i] = v.round();
    }
    (vals, est)
}

/// Average hitting power over every usable move: power ** POWER_EXP times the type multiplier.
pub fn strength(s: &Settings, skills: &[Move], opp_types: &[String]) -> f64 {
    let usable: Vec<&Move> = skills.iter().filter(|m| m.lv <= 70).collect();
    if usable.is_empty() {
        return 100f64.powf(s.power_exp) * type_mult(s, "neutral", opp_types);
    }
    usable.iter().map(|m| m.power.powf(s.power_exp) * type_mult(s, &m.el, opp_types)).sum::<f64>() / usable.len() as f64
}

fn logit(p: f64) -> f64 {
    (p / (1.0 - p)).ln()
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

/// The model. species_rec: [[home w, l], [away w, l]]; pal_h2h: [home wins, away wins].
pub fn score(s: &Settings, home: &Fighter, away: &Fighter, species_rec: [[f64; 2]; 2], pal_h2h: [f64; 2]) -> Score {
    // a fighter's speed was worked out at the default half strength (a square root): re-weight it to cooldown_exp
    let pace = |f: &Fighter| f.speed.powf(s.cooldown_exp / 0.5);
    let st_home = strength(s, &home.skills, &away.types) * home.hidden_boost * pace(home);
    let st_away = strength(s, &away.skills, &home.types) * away.hidden_boost * pace(away);
    // "turns" to knock out the defender
    let t_home = away.hp / (home.atk / away.def * st_home);
    let t_away = home.hp / (away.atk / home.def * st_away);
    let p_stats = 1.0 / (1.0 + (t_home / t_away).powf(s.stats_steepness));
    // records shrunk toward 50% so small samples barely move them
    let rates = species_rec.map(|[w, l]| (w + 5.0) / (w + l + 10.0));
    let p_species = sigmoid(logit(rates[0]) - logit(rates[1]));
    let p_h2h = (pal_h2h[0] + 2.0) / (pal_h2h[0] + pal_h2h[1] + 4.0);
    let p_home = sigmoid(logit(p_stats) + s.species_weight * logit(p_species) + s.pal_h2h_weight * logit(p_h2h));
    Score { t_home, t_away, p_stats, p_species, p_h2h, p_home, rates }
}

/// Fit log(hit %) = c + a*log(power) + b*log(type mult) + log(ATK/DEF) by least squares over one peak hit per
/// move: samples are (power, type_mult, pct, atk, def). Returns (a, effective type edge): a says how much move
/// power matters (1 = proportional); TYPE_BONUS ** b is the real value of a type edge.
pub fn calibrate(s: &Settings, samples: &[(f64, f64, f64, f64, f64)]) -> Option<(f64, f64)> {
    let rows: Vec<[f64; 3]> = samples
        .iter()
        .filter(|(pw, _, pct, _, _)| *pct > 0.0 && *pw > 0.0)
        .map(|(pw, tm, pct, atk, def)| [pw.ln(), tm.ln(), pct.ln() - (atk / def).ln()])
        .collect();
    if rows.len() < 6 {
        return None;
    }
    let x: Vec<[f64; 3]> = rows.iter().map(|r| [1.0, r[0], r[1]]).collect();
    let mut m = [[0.0; 4]; 3];
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = x.iter().map(|r| r[i] * r[j]).sum();
        }
        m[i][3] = x.iter().zip(&rows).map(|(r, y)| r[i] * y[2]).sum();
    }
    for i in 0..3 {
        let piv = (i..3).max_by(|a, b| m[*a][i].abs().total_cmp(&m[*b][i].abs()))?;
        m.swap(i, piv);
        if m[i][i] == 0.0 {
            return None;
        }
        for r in 0..3 {
            if r != i {
                let f = m[r][i] / m[i][i];
                let pivot = m[i];
                for (x, p) in m[r].iter_mut().zip(pivot) {
                    *x -= f * p;
                }
            }
        }
    }
    let pa = m[1][3] / m[1][1];
    let pb = m[2][3] / m[2][2];
    Some((pa, s.type_bonus.powf(pb)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(t: &[&str]) -> Vec<String> {
        t.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn type_chart() {
        let s = Settings::default();
        assert_eq!(type_mult(&s, "grass", &types(&["ground"])), 1.5);
        assert_eq!(type_mult(&s, "ground", &types(&["grass"])), 1.0 / 1.5);
        assert_eq!(type_mult(&s, "fire", &types(&["ice"])), 1.5);
        assert_eq!(type_mult(&s, "ice", &types(&["fire"])), 1.0 / 1.5);
        // dark beats neutral, dragon resists dark: they cancel
        assert_eq!(type_mult(&s, "dark", &types(&["dragon", "neutral"])), 1.0);
    }

    #[test]
    fn cooldown_passive_is_damped() {
        assert!((speed(&["COOLDOWN −30%".into()]) - (1.0f64 / 0.7).sqrt()).abs() < 1e-12);
        assert_eq!(speed(&["ATK +10%".into()]), 1.0);
    }

    #[test]
    fn hidden_stats_are_estimated() {
        let mut stats = HashMap::new();
        stats.insert("HP".into(), StatLine { iv: None, base: Some(97.0), lv70: None });
        let (vals, est) = resolve(&stats, &[]);
        assert!(est);
        assert_eq!(vals[0], (979.0 + 48.5 * 97.0 * 1.15f64).round());
    }
}
