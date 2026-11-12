//! A whole prediction from the live state: the model's pick plus everything the screens and the chat lines show.

use crate::db::{self, CallFighter, NewCall};
use crate::model::{self, Fighter, Score, Settings};
use crate::site::{self, DuelRow, Rivalry, State, StateSide, Tale};
use crate::voices;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Moves a species hit with in its latest fight tapes that its listed moveset doesn't have.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HiddenMoves {
    /// share of its damage from unlisted moves
    pub share: f64,
    /// (move, best hit %), "" names an unnamed hit (likely damage over time)
    pub moves: Vec<(String, f64)>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PalHeadToHead {
    pub home_wins: u32,
    pub away_wins: u32,
    pub meetings: Vec<DuelRow>,
    pub pages_scanned: usize,
}

/// One side of the fight, as shown.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Side {
    pub owner: String,
    pub owner_key: String,
    pub pal: String,
    pub slug: String,
    pub fighter: CallFighter,
    /// some stats were hidden until the bell and are estimated
    pub estimated: bool,
    pub base: [Option<f64>; 3],
    pub usable_moves: usize,
    pub strength: f64,
    pub type_edge_moves: Vec<String>,
    pub hidden: HiddenMoves,
    pub species_rate: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Prediction {
    pub seen_at: u64,
    pub title: String,
    pub live: bool,
    pub picks_open: bool,
    pub home: Side,
    pub away: Side,
    pub tale: Tale,
    pub score: Score,
    pub species_rec: [[f64; 2]; 2],
    pub pal_h2h: PalHeadToHead,
    pub owners: Option<Rivalry>,
    /// "home" or "away"
    pub pick: String,
    /// the pick's chance
    pub prob: f64,
    pub reason: String,
    pub lines: [String; 5],
    pub settings: Settings,
}

impl Prediction {
    pub fn favourite(&self) -> &Side {
        if self.pick == "home" {
            &self.home
        } else {
            &self.away
        }
    }

    pub fn underdog(&self) -> &Side {
        if self.pick == "home" {
            &self.away
        } else {
            &self.home
        }
    }

    pub fn estimated(&self) -> bool {
        self.home.estimated || self.away.estimated
    }

    /// The call to save.
    pub fn to_call(&self) -> NewCall {
        NewCall {
            seen_at: self.seen_at,
            home_owner: self.home.owner.clone(),
            away_owner: self.away.owner.clone(),
            home_pal: self.home.pal.clone(),
            away_pal: self.away.pal.clone(),
            pick: self.pick.clone(),
            prob: (self.prob * 1000.0).round() / 1000.0,
            p_stats: (self.score.p_stats * 1000.0).round() / 1000.0,
            p_species: (self.score.p_species * 1000.0).round() / 1000.0,
            p_h2h: (self.score.p_h2h * 1000.0).round() / 1000.0,
            estimated: self.estimated(),
            species_rec: self.species_rec,
            pal_h2h: [self.pal_h2h.home_wins as f64, self.pal_h2h.away_wins as f64],
            model: self.settings.clone(),
            home: self.home.fighter.clone(),
            away: self.away.fighter.clone(),
            picks_open: self.picks_open,
        }
    }
}

/// From a species' latest `n` fight tapes: the share of its damage from moves outside its listed moveset (species-only
/// attacks like Bushi Noct's Iai or Tarantriss's Spider Attack), and those moves' best hits. Tapes come from the
/// store when it has them, and are stored once fetched.
pub fn hidden_moves(pal: &str, slug: &str, listed: &[String], db_path: &Path, n: usize) -> HiddenMoves {
    let Ok(page) = site::fetch(&format!("{}/pal/{slug}?p=1", site::BASE)) else { return HiddenMoves::default() };
    let recent: Vec<(u64, String)> = site::duel_rows(&page)
        .into_iter()
        .filter_map(|r| Some((r.id?, r.sides.iter().find(|s| s.pal == pal)?.side.clone())))
        .take(n)
        .collect();
    let conn = db::open(db_path).ok();
    let (mut total, mut hidden) = (0.0, 0.0);
    let mut moves: HashMap<String, f64> = HashMap::new();
    for (id, me) in recent {
        let cached = conn.as_ref().and_then(|c| db::get_duel(c, id).ok().flatten()).filter(|d| d.winner.is_some());
        let duel = match cached {
            Some(d) => d,
            None => match site::duel(id) {
                Ok(d) => {
                    if let (Some(c), true) = (conn.as_ref(), d.winner.is_some()) {
                        let _ = db::save_duel(c, id, &d);
                    }
                    d
                }
                Err(_) => continue,
            },
        };
        for h in duel.hits.iter().filter(|h| h.side == me) {
            total += h.pct;
            if !listed.contains(&h.mv) {
                hidden += h.pct;
                let best = moves.entry(h.mv.clone()).or_insert(0.0);
                *best = best.max(h.pct);
            }
        }
    }
    let mut moves: Vec<(String, f64)> = moves.into_iter().collect();
    moves.sort_by(|a, b| b.1.total_cmp(&a.1));
    HiddenMoves { share: if total > 0.0 { hidden / total } else { 0.0 }, moves }
}

/// Every past duel between these two species, from the smaller species' pages (by duels fought).
pub fn pal_head_to_head(home: &StateSide, away: &StateSide, species_rec: [[f64; 2]; 2], max_pages: usize) -> PalHeadToHead {
    if home.pal == away.pal {
        return PalHeadToHead::default();
    }
    let (scan, other) = if species_rec[0].iter().sum::<f64>() <= species_rec[1].iter().sum::<f64>() { (home, away) } else { (away, home) };
    let slug = site::slug_of(&scan.img).unwrap_or_else(|| site::slug_for(&scan.pal));
    let Ok(pages) = site::pal_pages(&slug, max_pages) else { return PalHeadToHead::default() };
    let meetings: Vec<DuelRow> = pages
        .iter()
        .flat_map(|p| site::duel_rows(p))
        .filter(|r| {
            let pals = [r.sides[0].pal.as_str(), r.sides[1].pal.as_str()];
            pals.contains(&scan.pal.as_str()) && pals.contains(&other.pal.as_str())
        })
        .collect();
    let home_wins = meetings.iter().filter(|r| r.winner().pal == home.pal).count() as u32;
    PalHeadToHead { home_wins, away_wins: meetings.len() as u32 - home_wins, meetings, pages_scanned: pages.len() }
}

/// Options for a prediction.
pub struct Options {
    /// look up the owners' rivalry, the pals' past meetings and hidden moves (network; about a second)
    pub lookups: bool,
    pub db_path: PathBuf,
    pub settings: Settings,
}

/// The prediction for the duel in the state, or None when no duel is up.
pub fn predict(state: &State, opts: &Options) -> Option<Prediction> {
    let (h, a) = (state.home.as_ref()?, state.away.as_ref()?);
    let hud = state.hud.as_deref().unwrap_or("");
    let stats = site::parse_stats(hud);
    let skills = site::parse_skills(state.skills.as_deref().unwrap_or(""));
    let passives = site::parse_passives(hud);
    let tale = site::parse_tale(hud);
    let species_rec = tale.species_records();
    let empty = Vec::new();

    let hid = |side: &StateSide, key: &str| {
        let listed: Vec<String> = skills.get(key).unwrap_or(&empty).iter().map(|m| m.name.clone()).collect();
        let slug = site::slug_of(&side.img).unwrap_or_else(|| site::slug_for(&side.pal));
        hidden_moves(&side.pal, &slug, &listed, &opts.db_path, 3)
    };
    let (owners, pal_h2h, hidden) = if opts.lookups {
        std::thread::scope(|s| {
            let owners = s.spawn(|| site::owner_rivalry(&h.actor_key, &a.actor_key).ok());
            let pals = s.spawn(|| pal_head_to_head(h, a, species_rec, 40));
            let hh = s.spawn(|| hid(h, "home"));
            let ha = s.spawn(|| hid(a, "away"));
            (owners.join().ok().flatten(), pals.join().unwrap_or_default(), [hh.join().unwrap_or_default(), ha.join().unwrap_or_default()])
        })
    } else {
        (None, PalHeadToHead::default(), [HiddenMoves::default(), HiddenMoves::default()])
    };

    let build = |key: &str, me: &StateSide, op: &StateSide, hidden: HiddenMoves| {
        let st = stats.get(key).cloned().unwrap_or_default();
        let pas = passives.get(key).cloned().unwrap_or_default();
        let (vals, est) = model::resolve(&st, &pas);
        let mv = skills.get(key).cloned().unwrap_or_default();
        let fighter = Fighter {
            pal: me.pal.clone(),
            types: me.types.clone(),
            hp: vals[0],
            atk: vals[1],
            def: vals[2],
            skills: mv.clone(),
            hidden_boost: (model::hidden_boost(hidden.share) * 1000.0).round() / 1000.0,
            speed: (model::speed(&pas) * 1000.0).round() / 1000.0,
            passives: pas,
        };
        let ivs = ["HP", "ATK", "DEF"].iter().map(|k| (k.to_string(), st.get(*k).and_then(|s| s.iv))).collect();
        let base = ["HP", "ATK", "DEF"].map(|k| st.get(k).and_then(|s| s.base));
        Side {
            owner: me.actor.clone(),
            owner_key: me.actor_key.clone(),
            pal: me.pal.clone(),
            slug: site::slug_of(&me.img).unwrap_or_else(|| site::slug_for(&me.pal)),
            estimated: est,
            base,
            usable_moves: mv.iter().filter(|m| m.lv <= 70).count(),
            strength: model::strength(&opts.settings, &mv, &op.types),
            type_edge_moves: mv
                .iter()
                .filter(|m| m.lv <= 70 && model::type_mult(&opts.settings, &m.el, &op.types) > 1.0)
                .map(|m| m.name.clone())
                .collect(),
            hidden,
            species_rate: 0.0,
            fighter: CallFighter { fighter, ivs },
        }
    };
    let [hidden_home, hidden_away] = hidden;
    let mut home = build("home", h, a, hidden_home);
    let mut away = build("away", a, h, hidden_away);
    let score = model::score(
        &opts.settings,
        &home.fighter.fighter,
        &away.fighter.fighter,
        species_rec,
        [pal_h2h.home_wins as f64, pal_h2h.away_wins as f64],
    );
    home.species_rate = score.rates[0];
    away.species_rate = score.rates[1];
    let pick = if score.p_home >= 0.5 { "home" } else { "away" };
    let prob = score.p_home.max(1.0 - score.p_home);
    let (fav, dog) = if pick == "home" { (&home, &away) } else { (&away, &home) };
    let reason = voices::reason(
        &fav.fighter.fighter,
        &dog.fighter.fighter,
        fav.type_edge_moves.len(),
        fav.species_rate,
        dog.species_rate,
        if pick == "home" { score.p_h2h } else { 1.0 - score.p_h2h },
    );
    let lines = voices::lines(&fav.pal, &dog.pal, prob, reason);
    Some(Prediction {
        seen_at: state.now.unwrap_or(0),
        title: state.title.clone(),
        live: state.live,
        picks_open: state.picks_open(),
        home,
        away,
        tale,
        score,
        species_rec,
        pal_h2h,
        owners,
        pick: pick.into(),
        prob,
        reason: reason.into(),
        lines,
        settings: opts.settings.clone(),
    })
}

/// The plain-text report the command-line `augur predict` prints: the pick first, then the five chat lines.
pub fn report(p: &Prediction) -> String {
    let mut out = vec![format!(
        "PREDICTION: {} {} ({:.0}%)   {} ({}) vs {} ({}){}",
        p.pick.to_uppercase(),
        p.favourite().pal,
        p.prob * 100.0,
        p.home.pal,
        p.home.owner,
        p.away.pal,
        p.away.owner,
        if p.estimated() { "   [some stats estimated until the bell]" } else { "" }
    )];
    out.extend(p.lines.iter().cloned());
    out.push(String::new());
    out.push(format!(
        "HOME {}: {} [{}]   vs   AWAY {}: {} [{}]",
        p.home.owner,
        p.home.pal,
        p.home.fighter.fighter.types.join(", "),
        p.away.owner,
        p.away.pal,
        p.away.fighter.fighter.types.join(", ")
    ));
    for (k, hv, av) in &p.tale.rows {
        out.push(format!("  {k:15} {hv}  vs  {av}"));
    }
    out.push(format!("  {:15} {}", "TYPE EDGE", p.tale.type_edge));
    out.push(format!("  {:15} {}", "VERDICT", p.tale.verdict));
    for s in [&p.home, &p.away] {
        let f = &s.fighter.fighter;
        out.push(format!(
            "  {:5} HP {:>6}  ATK {:>4}  DEF {:>4}{}   moves {} (strength {:.2}){}   passives {}",
            if std::ptr::eq(s, &p.home) { "HOME" } else { "AWAY" },
            f.hp,
            f.atk,
            f.def,
            if s.estimated { "  (ESTIMATED)" } else { "" },
            s.usable_moves,
            s.strength,
            if s.type_edge_moves.is_empty() { String::new() } else { format!("   type-edge moves: {}", s.type_edge_moves.join(", ")) },
            if f.passives.is_empty() { "none".into() } else { f.passives.join(", ") }
        ));
    }
    out.push(format!(
        "  Time-to-KO: home needs {:.1}, away needs {:.1}  ->  stats say home {:.0}%",
        p.score.t_home,
        p.score.t_away,
        p.score.p_stats * 100.0
    ));
    out.push(format!(
        "  Species records: home {:.0}% vs away {:.0}%  ->  records say home {:.0}%",
        p.score.rates[0] * 100.0,
        p.score.rates[1] * 100.0,
        p.score.p_species * 100.0
    ));
    out.push(format!("  Pal head to head: {}-{}  ->  says home {:.0}%", p.pal_h2h.home_wins, p.pal_h2h.away_wins, p.score.p_h2h * 100.0));
    for (label, s) in [("HOME", &p.home), ("AWAY", &p.away)] {
        if !s.hidden.moves.is_empty() {
            let list: Vec<String> =
                s.hidden.moves.iter().map(|(m, b)| format!("{} up to {b:.0}%", if m.is_empty() { "(unnamed hit)" } else { m })).collect();
            out.push(format!(
                "  {label} {} also hits with unlisted moves ({:.0}% of its damage in its last 3 fights): {}",
                s.pal,
                s.hidden.share * 100.0,
                list.join(", ")
            ));
        }
    }
    if let Some(r) = &p.owners {
        out.push(format!("\n  HEAD TO HEAD: {}", r.summary));
        for m in r.meetings.iter().take(8) {
            out.push(format!(
                "    {:>8}  {} ({}) vs {} ({})  -> {}",
                m.when,
                m.sides[0].owner,
                m.sides[0].pal,
                m.sides[1].owner,
                m.sides[1].pal,
                m.winner().owner
            ));
        }
    }
    out.push(format!(
        "\n  PAL HEAD TO HEAD: {} {} - {} {}  ({} meetings)",
        p.home.pal,
        p.pal_h2h.home_wins,
        p.pal_h2h.away_wins,
        p.away.pal,
        p.pal_h2h.meetings.len()
    ));
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predicts_the_saved_state_like_the_python_version() {
        let state = crate::seed::state("state-leaflit-wumpo.json");
        let p =
            predict(&state, &Options { lookups: false, db_path: PathBuf::from("/nonexistent"), settings: Settings::default() }).unwrap();
        assert_eq!(p.favourite().pal, "Wumpo");
        assert!(p.home.estimated, "the home side's stats were hidden until the bell");
        assert_eq!(p.away.fighter.fighter.hp, 8705.0);
        assert!(p.lines[0].contains("Wumpo"));
    }
}
