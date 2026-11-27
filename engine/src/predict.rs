//! A whole prediction from the live state: the model's pick plus everything the screens and the chat lines show.

use crate::db::{self, CallFighter, NewCall};
use crate::model::{self, Fighter, Score, Settings, StatLine};
use crate::site::{self, DuelRow, Rivalry, State, StateSide, Tale};
use crate::voices;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
    /// where the stats came from: "shown", "from their box" (hidden, but the owner has one of that pal) or
    /// "guessed (…)" with why
    #[serde(default)]
    pub stats_from: String,
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
            stats_from: format!("home: {}; away: {}", self.home.stats_from, self.away.stats_from),
            version: env!("CARGO_PKG_VERSION").into(),
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

/// Every past duel between these two species, from the smaller species' pages (by duels fought), as many pages as
/// are read by `deadline`.
pub fn pal_head_to_head(
    home: &StateSide,
    away: &StateSide,
    species_rec: [[f64; 2]; 2],
    max_pages: usize,
    deadline: Option<Instant>,
) -> PalHeadToHead {
    if home.pal == away.pal {
        return PalHeadToHead::default();
    }
    let (scan, other) = if species_rec[0].iter().sum::<f64>() <= species_rec[1].iter().sum::<f64>() { (home, away) } else { (away, home) };
    let slug = site::slug_of(&scan.img).unwrap_or_else(|| site::slug_for(&scan.pal));
    let Ok(pages) = site::pal_pages_by(&slug, max_pages, deadline) else { return PalHeadToHead::default() };
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

/// How long a call's lookups (the owners' rivalry, the pals' past meetings, hidden moves, the owners' boxes) are given:
/// a slow or rate-limiting site makes for a call without some of them, as when one fails, not a call that never comes.
/// On 27 November 2026 Augur made no calls for over an hour while the site was being read.
pub const LOOKUP_LIMIT: Duration = Duration::from_secs(45);

/// Runs `f` on its own thread; the closure it returns waits for the answer until `deadline`, and gives None after
/// (the thread is left to finish on its own).
fn by<T: Send + 'static>(deadline: Instant, f: impl FnOnce() -> T + Send + 'static) -> impl FnOnce() -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    move || rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).ok()
}

/// Options for a prediction.
pub struct Options {
    /// look up the owners' rivalry, the pals' past meetings and hidden moves (network; about a second)
    pub lookups: bool,
    pub db_path: PathBuf,
    pub settings: Settings,
    /// the hidden side's HP, Attack and Defence as the stream showed them (`stream::check`): (side, stats)
    pub stream: Option<(String, [u32; 3])>,
}

/// A pal's stats and passives as its box card shows them.
pub type BoxCard = (HashMap<String, StatLine>, Vec<String>);

/// A hidden fighter's stats from its owner's pal box: the card shows what it fights with (stats, talent rolls and
/// passives, the same numbers the duel shows at the bell). Owners often have several of a pal (eggs hatch at random),
/// each with its own stats, and the duel doesn't say which is fighting. It does show the passives while it hides the
/// stats, so only the cards with those passives can be the one. Returns the cards it could be (one: its stats are
/// known; more: the call is worked out for each, see `over_the_box`) and where the stats came from.
pub fn box_stats(owned: Option<&[site::BoxPal]>, pal: &str, passives: &[String]) -> (Vec<BoxCard>, String) {
    let Some(owned) = owned else { return (vec![], "guessed (their box couldn't be read)".into()) };
    let same: Vec<&site::BoxPal> = owned.iter().filter(|p| p.pal == pal && p.stats.len() == 3).collect();
    let n = same.len();
    let known = !passives.iter().any(|p| p.contains("unknown"));
    let matching: Vec<&site::BoxPal> = same.iter().copied().filter(|p| known && same_passives(&p.passives, passives)).collect();
    // none matching means the passives read differently from the box's: better all of them than none
    let narrowed = !matching.is_empty() && matching.len() < n;
    let cards: Vec<BoxCard> = if matching.is_empty() { &same } else { &matching }.iter().map(|p| card(p)).collect();
    let from = match cards.len() {
        0 => "guessed (not in their box)".into(),
        1 if n == 1 => "from their box".into(),
        1 => format!("from their box (the only one of {n} with its passives)"),
        m if narrowed => format!("guessed ({n} in their box, the chance over the {m} with its passives)"),
        _ => format!("guessed ({n} in their box, the chance over all {n})"),
    };
    (cards, from)
}

/// The home win chance and the stats' share of it when a hidden side could be any of several box pals: the average
/// over each pairing, so the call is only sure when it would be sure whichever one is fighting. Averaging the stats
/// instead would make up a pal the owner doesn't have. None when each side is known (or guessed) as one fighter.
pub fn over_the_box(s: &Settings, home: &[Fighter], away: &[Fighter], species_rec: [[f64; 2]; 2], pal_h2h: [f64; 2]) -> Option<(f64, f64)> {
    if home.len() < 2 && away.len() < 2 {
        return None;
    }
    let all: Vec<Score> = home.iter().flat_map(|h| away.iter().map(move |a| model::score(s, h, a, species_rec, pal_h2h))).collect();
    let n = all.len() as f64;
    Some((all.iter().map(|x| x.p_home).sum::<f64>() / n, all.iter().map(|x| x.p_stats).sum::<f64>() / n))
}

/// The prediction for the duel in the state, or None when no duel is up.
pub fn predict(state: &State, opts: &Options) -> Option<Prediction> {
    let (h, a) = (state.home.as_ref()?, state.away.as_ref()?);
    let hud = state.hud.as_deref().unwrap_or("");
    let stats = site::parse_stats(hud);
    let mut skills = site::parse_skills(state.skills.as_deref().unwrap_or(""));
    // each move's real hit size from the stored fight tapes
    if let Some(sizes) = db::open(&opts.db_path).ok().and_then(|c| db::hit_sizes(&c, None).ok()) {
        skills.values_mut().for_each(|m| db::with_hit_sizes(m, &sizes));
    }
    let passives = site::parse_passives(hud);
    let ranks = site::parse_pal_ranks(state.arena.as_deref().unwrap_or(""));
    let tale = site::parse_tale(hud);
    let species_rec = tale.species_records();
    let empty = Vec::new();

    let deadline = Instant::now() + LOOKUP_LIMIT;
    let hid = |side: &StateSide, key: &str| {
        let listed: Vec<String> = skills.get(key).unwrap_or(&empty).iter().map(|m| m.name.clone()).collect();
        let slug = site::slug_of(&side.img).unwrap_or_else(|| site::slug_for(&side.pal));
        let (pal, db) = (side.pal.clone(), opts.db_path.clone());
        by(deadline, move || hidden_moves(&pal, &slug, &listed, &db, 3))
    };
    // the site hides a side's stats until the bell ("STATS KNOWN AT THE BELL"): its owner's box may show them
    let shown = |key: &str| stats.get(key).and_then(|s| s.get("HP")).and_then(|l| l.lv70).is_some();
    let boxed = |side: &StateSide, key: &str| {
        let (want, owner) = (opts.lookups && !shown(key), side.actor_key.clone());
        by(deadline, move || want.then(|| site::roster(&owner).ok()).flatten())
    };
    // all at once, each given until the deadline: one the site is slow with is left out, as one that failed would be
    let (owners, pal_h2h, hidden, boxes) = if opts.lookups {
        let (hk, ak) = (h.actor_key.clone(), a.actor_key.clone());
        let owners = by(deadline, move || site::owner_rivalry(&hk, &ak).ok());
        let (hs, as_) = (h.clone(), a.clone());
        let pals = by(deadline, move || pal_head_to_head(&hs, &as_, species_rec, 40, Some(deadline)));
        let (hh, ha, bh, ba) = (hid(h, "home"), hid(a, "away"), boxed(h, "home"), boxed(a, "away"));
        (
            owners().flatten(),
            pals().unwrap_or_default(),
            [hh().unwrap_or_default(), ha().unwrap_or_default()],
            [bh().flatten(), ba().flatten()],
        )
    } else {
        (None, PalHeadToHead::default(), [HiddenMoves::default(), HiddenMoves::default()], [None, None])
    };

    // a side, and the box pals a hidden side could be when there's more than one
    let build = |key: &str, me: &StateSide, op: &StateSide, hidden: HiddenMoves, owned: Option<Vec<site::BoxPal>>| {
        let (mut st, mut pas) = (stats.get(key).cloned().unwrap_or_default(), passives.get(key).cloned().unwrap_or_default());
        let mut could_be = vec![];
        let read = opts.stream.as_ref().filter(|(side, _)| side == key && !shown(key)).map(|(_, stats)| *stats);
        let stats_from = if shown(key) {
            "shown".to_string()
        } else if let Some(read) = read {
            // the stream showed the stats the site hides: the box card with those numbers (its talent rolls and
            // passives too), or the numbers alone
            let (cards, _) = box_stats(owned.as_deref(), &me.pal, &[]);
            let n = cards.len();
            let lv70 = |card: &HashMap<String, StatLine>, k: &str| card.get(k).and_then(|l| l.lv70).map(|v| v.round() as u32);
            let found = cards.into_iter().find(|(card, _)| ["HP", "ATK", "DEF"].iter().zip(read).all(|(k, v)| lv70(card, k) == Some(v)));
            match found {
                Some((card, card_passives)) => {
                    for (k, line) in card {
                        let base = st.get(&k).and_then(|l| l.base);
                        st.insert(k, StatLine { base, ..line });
                    }
                    pas = card_passives;
                    if n > 1 {
                        format!("read from the stream (one of {n} in their box)")
                    } else {
                        "read from the stream".into()
                    }
                }
                None => {
                    for (k, v) in ["HP", "ATK", "DEF"].iter().zip(read) {
                        let line = st.entry(k.to_string()).or_default();
                        line.lv70 = Some(v as f64);
                    }
                    "read from the stream (not matched to their box)".into()
                }
            }
        } else if !opts.lookups {
            "guessed".to_string()
        } else {
            let (cards, from) = box_stats(owned.as_deref(), &me.pal, &pas);
            if let [(card, card_passives)] = cards.as_slice() {
                // the box has no base stats: keep the duel's for the report
                for (k, line) in card {
                    let base = st.get(k).and_then(|l| l.base);
                    st.insert(k.clone(), StatLine { base, ..line.clone() });
                }
                pas = card_passives.clone();
            } else {
                could_be = cards;
            }
            from
        };
        let mv = skills.get(key).cloned().unwrap_or_default();
        let fighter_from = |st: &HashMap<String, StatLine>, pas: &[String]| {
            let (vals, est) = model::resolve(st, pas);
            let f = Fighter {
                pal: me.pal.clone(),
                types: me.types.clone(),
                hp: vals[0],
                atk: vals[1],
                def: vals[2],
                skills: mv.clone(),
                hidden_boost: (model::hidden_boost(hidden.share) * 1000.0).round() / 1000.0,
                speed: (model::speed(pas) * 1000.0).round() / 1000.0,
                passives: pas.to_vec(),
                pal_rank: ranks.get(key).cloned(),
            };
            (f, est)
        };
        let (fighter, est) = fighter_from(&st, &pas);
        let could_be: Vec<Fighter> = could_be.iter().map(|(card, card_passives)| fighter_from(card, card_passives).0).collect();
        let ivs = ["HP", "ATK", "DEF"].iter().map(|k| (k.to_string(), st.get(*k).and_then(|s| s.iv))).collect();
        let base = ["HP", "ATK", "DEF"].map(|k| st.get(k).and_then(|s| s.base));
        let side = Side {
            owner: me.actor.clone(),
            owner_key: me.actor_key.clone(),
            pal: me.pal.clone(),
            slug: site::slug_of(&me.img).unwrap_or_else(|| site::slug_for(&me.pal)),
            estimated: est,
            stats_from,
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
        };
        (side, could_be)
    };
    let [hidden_home, hidden_away] = hidden;
    let [box_home, box_away] = boxes;
    let (mut home, home_could_be) = build("home", h, a, hidden_home, box_home);
    let (mut away, away_could_be) = build("away", a, h, hidden_away, box_away);
    // a real call: SPECIES_LIVE weighs the records
    let (calls, h2h) = (opts.settings.for_calls(), [pal_h2h.home_wins as f64, pal_h2h.away_wins as f64]);
    let mut score = model::score(&calls, &home.fighter.fighter, &away.fighter.fighter, species_rec, h2h);
    let or_shown = |could_be: Vec<Fighter>, side: &Side| if could_be.is_empty() { vec![side.fighter.fighter.clone()] } else { could_be };
    let (home_all, away_all) = (or_shown(home_could_be, &home), or_shown(away_could_be, &away));
    if let Some((p_home, p_stats)) = over_the_box(&calls, &home_all, &away_all, species_rec, h2h) {
        (score.p_home, score.p_stats) = (p_home, p_stats);
    }
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
    // the site hid a side's stats until the bell: say where Augur got them
    for s in [&p.home, &p.away] {
        if !s.stats_from.is_empty() && s.stats_from != "shown" {
            out.push(format!("  {} {} stats hidden until the bell: {}", s.owner, s.pal, s.stats_from));
        }
    }
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

// --- the box ---

fn card(p: &site::BoxPal) -> BoxCard {
    let stats = p.stats.iter().map(|(k, (lv70, iv))| (k.clone(), StatLine { iv: Some(*iv), base: None, lv70: Some(*lv70) })).collect();
    (stats, p.passives.clone())
}

/// The same passives, in any order, as the duel and the box word them ("DEF −10%" on one, "Defense -10%" on another).
fn same_passives(a: &[String], b: &[String]) -> bool {
    let words = |v: &[String]| {
        let mut v: Vec<String> = v
            .iter()
            .map(|p| p.to_uppercase().replace('−', "-").replace("DEFENSE", "DEF").replace("DEFENCE", "DEF").replace("ATTACK", "ATK"))
            .collect();
        v.sort();
        v
    };
    words(a) == words(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predicts_the_saved_state_like_the_python_version() {
        let state = crate::seed::state("state-leaflit-wumpo.json");
        let p = predict(
            &state,
            &Options { lookups: false, db_path: PathBuf::from("/nonexistent"), settings: Settings::default(), stream: None },
        )
        .unwrap();
        assert_eq!(p.favourite().pal, "Wumpo");
        assert!(p.home.estimated, "the home side's stats were hidden until the bell");
        assert_eq!(p.away.fighter.fighter.hp, 8705.0);
        assert!(p.lines[0].contains("Wumpo"));
    }

    fn boxed(pal: &str, hp: f64) -> site::BoxPal {
        with_passives(pal, hp, &["COOLDOWN −15%"])
    }

    fn with_passives(pal: &str, hp: f64, passives: &[&str]) -> site::BoxPal {
        site::BoxPal {
            pal: pal.into(),
            stats: [("HP", (hp, 43.0)), ("ATK", (592.0, 34.0)), ("DEF", (1015.0, 2.0))].map(|(k, v)| (k.to_string(), v)).into(),
            passives: passives.iter().map(|p| p.to_string()).collect(),
            ..Default::default()
        }
    }

    /// A hidden fighter takes its stats from its owner's box when the owner has exactly one of that pal.
    #[test]
    fn hidden_stats_come_from_the_box() {
        let owned = vec![boxed("Turtacle Terra", 5592.0), boxed("Lamball", 3000.0)];
        let (cards, from) = box_stats(Some(&owned), "Turtacle Terra", &["COOLDOWN −15%".into()]);
        let [(stats, passives)] = cards.as_slice() else { panic!("one card") };
        assert_eq!((stats["HP"].lv70, stats["HP"].iv, stats["DEF"].lv70), (Some(5592.0), Some(43.0), Some(1015.0)));
        assert_eq!((passives.clone(), from.as_str()), (vec!["COOLDOWN −15%".to_string()], "from their box"));
        let two = vec![boxed("Lamball", 3000.0), boxed("Lamball", 3100.0)];
        let (cards, from) = box_stats(Some(&two), "Lamball", &["COOLDOWN −15%".into()]);
        assert_eq!((cards.len(), from.as_str()), (2, "guessed (2 in their box, the chance over all 2)"));
        assert_eq!(box_stats(Some(&owned), "Nyafia Terra", &[]).1, "guessed (not in their box)");
        assert_eq!(box_stats(None, "Lamball", &[]).1, "guessed (their box couldn't be read)");
    }

    /// Several of the same pal: the duel's passives say which it can be.
    #[test]
    fn passives_narrow_the_box() {
        let owned = vec![
            with_passives("Teafant", 4600.0, &[]),
            with_passives("Teafant", 4700.0, &["DEF +10%"]),
            with_passives("Teafant", 4800.0, &[]),
            with_passives("Lamball", 3000.0, &["DEF +10%"]),
        ];
        let (cards, from) = box_stats(Some(&owned), "Teafant", &["DEF +10%".into()]);
        assert_eq!((cards.len(), from.as_str()), (1, "from their box (the only one of 3 with its passives)"));
        assert_eq!(cards[0].0["HP"].lv70, Some(4700.0));
        let (cards, from) = box_stats(Some(&owned), "Teafant", &[]);
        assert_eq!((cards.len(), from.as_str()), (2, "guessed (3 in their box, the chance over the 2 with its passives)"));
        // the box words some passives differently
        assert_eq!(
            box_stats(
                Some(&[with_passives("Teafant", 4700.0, &["Defense +10%", "ATK +10%"])]),
                "Teafant",
                &["ATK +10%".into(), "DEF +10%".into()]
            )
            .0
            .len(),
            1
        );
        // passives the duel doesn't show yet, or that match none of them: it could be any
        let (cards, from) = box_stats(Some(&owned), "Teafant", &["unknown until the bell".into()]);
        assert_eq!((cards.len(), from.as_str()), (3, "guessed (3 in their box, the chance over all 3)"));
        assert_eq!(box_stats(Some(&owned), "Teafant", &["ATK +30%".into()]).0.len(), 3);
    }

    /// A strong and a weak copy: the call averages the two chances, so it's less sure than the strong one alone.
    #[test]
    fn the_call_is_over_every_pal_it_could_be() {
        let state = crate::seed::state("state-leaflit-wumpo.json");
        let p = predict(
            &state,
            &Options { lookups: false, db_path: PathBuf::from("/nonexistent"), settings: Settings::default(), stream: None },
        )
        .unwrap();
        let (h, a) = (p.home.fighter.fighter.clone(), p.away.fighter.fighter.clone());
        let strong = Fighter { hp: h.hp * 1.5, atk: h.atk * 1.5, ..h.clone() };
        let weak = Fighter { hp: h.hp * 0.6, atk: h.atk * 0.6, ..h.clone() };
        let s = Settings::default().for_calls();
        let one = |f: &Fighter| model::score(&s, f, &a, p.species_rec, [0.0, 0.0]).p_home;
        let (p_home, _) = over_the_box(&s, &[strong.clone(), weak.clone()], std::slice::from_ref(&a), p.species_rec, [0.0, 0.0]).unwrap();
        assert!((p_home - (one(&strong) + one(&weak)) / 2.0).abs() < 1e-12);
        assert!(one(&weak) < p_home && p_home < one(&strong));
        assert_eq!(over_the_box(&s, &[strong], &[a], p.species_rec, [0.0, 0.0]), None, "one fighter a side: the usual call");
    }

    /// Without lookups a hidden side is still guessed, and says so.
    #[test]
    fn a_hidden_side_says_it_was_guessed() {
        let state = crate::seed::state("state-leaflit-wumpo.json");
        let p = predict(
            &state,
            &Options { lookups: false, db_path: PathBuf::from("/nonexistent"), settings: Settings::default(), stream: None },
        )
        .unwrap();
        assert_eq!((p.home.stats_from.as_str(), p.away.stats_from.as_str()), ("guessed", "shown"));
        assert!(report(&p).contains("stats hidden until the bell: guessed"));
    }

    /// The stream showed the hidden side: the call uses those numbers and isn't a guess any more.
    #[test]
    fn a_hidden_side_read_from_the_stream() {
        let state = crate::seed::state("state-leaflit-wumpo.json");
        let opts = |stream| Options { lookups: false, db_path: PathBuf::from("/nonexistent"), settings: Settings::default(), stream };
        let p = predict(&state, &opts(Some(("home".into(), [6100, 650, 720])))).unwrap();
        let f = &p.home.fighter.fighter;
        assert_eq!((f.hp, f.atk, f.def), (6100.0, 650.0, 720.0));
        assert_eq!(p.home.stats_from, "read from the stream (not matched to their box)");
        assert!(!p.estimated(), "known now, so auto bet treats it like any other call");
        // a read of the side the site shows changes nothing
        let shown = predict(&state, &opts(Some(("away".into(), [1, 2, 3])))).unwrap();
        assert_eq!((shown.away.stats_from.as_str(), shown.away.fighter.fighter.hp), ("shown", 8705.0));
        assert_eq!(shown.home.stats_from, "guessed");
    }
}
