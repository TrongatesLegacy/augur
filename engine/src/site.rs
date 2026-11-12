//! Fetching and parsing palduels.com: the live state API and the HTML pages for duels, pals, rivalries and
//! players. The site rate-limits bursts (HTTP 429), so `fetch` backs off and callers keep parallel fetches low.

pub use crate::error::{Error, Result};
use crate::model::{Move, StatLine};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

pub const BASE: &str = "https://palduels.com";
pub const STATE_URL: &str = "https://palduels.com/api/state.php";
const UA: &str = concat!("Augur/", env!("CARGO_PKG_VERSION"), " (personal PalDuels companion)");

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(15))).build().into())
}

/// One GET, no retries. `cookie` is a signed-in session's Cookie header, for the few pages that need it.
pub fn fetch_once(url: &str, cookie: Option<&str>) -> Result<String> {
    let mut req = agent().get(url).header("User-Agent", UA).header("Cache-Control", "no-store");
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }
    match req.call() {
        Ok(mut resp) => resp.body_mut().read_to_string().map_err(|e| Error::Net(e.to_string())),
        Err(ureq::Error::StatusCode(code)) => Err(Error::Http(code)),
        Err(e) => Err(Error::Net(e.to_string())),
    }
}

/// GET with backoff on 429/5xx.
pub fn fetch(url: &str) -> Result<String> {
    let mut last = Error::Net("not tried".into());
    for i in 0..4u64 {
        match fetch_once(url, None) {
            Ok(body) => return Ok(body),
            Err(Error::Http(code)) if code == 429 || code >= 500 => {
                last = Error::Http(code);
                std::thread::sleep(Duration::from_millis(1500 * (i + 1)));
            }
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

pub fn fetch_bytes(url: &str) -> Result<Vec<u8>> {
    match agent().get(url).header("User-Agent", UA).call() {
        Ok(mut resp) => resp.body_mut().read_to_vec().map_err(|e| Error::Net(e.to_string())),
        Err(ureq::Error::StatusCode(code)) => Err(Error::Http(code)),
        Err(e) => Err(Error::Net(e.to_string())),
    }
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid pattern"))
}

macro_rules! regex {
    ($p:expr) => {{
        static CELL: OnceLock<Regex> = OnceLock::new();
        re(&CELL, $p)
    }};
}

/// Visible text of an HTML fragment.
pub fn text(s: &str) -> String {
    html_escape::decode_html_entities(&regex!(r"<[^>]+>").replace_all(s, "")).trim().to_string()
}

/// Text with runs of whitespace collapsed.
fn squash(s: &str) -> String {
    regex!(r"\s+").replace_all(&text(&regex!(r"<[^>]+>").replace_all(s, " ")), " ").trim().to_string()
}

fn num(s: &str) -> Option<f64> {
    let t = text(s).replace([',', '#'], "");
    regex!(r"^-?\d+$").is_match(&t).then(|| t.parse().ok()).flatten()
}

/// Pal page slug from its image path: /img/pals/bushi-noct.webp?v=1 -> bushi-noct.
pub fn slug_of(img: &str) -> Option<String> {
    regex!(r"/img/pals/([^./?]+)").captures(img).map(|c| c[1].to_string())
}

/// The slug a pal's name most likely has (used when no image path is at hand).
pub fn slug_for(pal: &str) -> String {
    pal.to_lowercase().split_whitespace().collect::<Vec<_>>().join("-")
}

fn between<'a>(s: &'a str, start: &str, ends: &[&str]) -> Option<&'a str> {
    let i = s.find(start)?;
    let rest = &s[i..];
    let j = ends.iter().filter_map(|e| rest[start.len()..].find(e).map(|k| k + start.len())).min().unwrap_or(rest.len());
    Some(&rest[..j])
}

// --- the live state (/api/state.php) ---

/// One side of the duel in the live state.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSide {
    pub actor: String,
    #[serde(default)]
    pub actor_key: String,
    pub pal: String,
    #[serde(default)]
    pub img: String,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub hp: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Recent {
    pub id: u64,
    pub home: String,
    pub away: String,
    pub win: String,
    #[serde(default)]
    pub at: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QueueItem {
    #[serde(default)]
    pub pos: u32,
    pub home: String,
    pub away: String,
}

/// The parts of /api/state.php Augur uses. The crowd split (`call.crowd`) is deliberately not read: picks stand
/// on stats, types, moves and history alone.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub live: bool,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub badge: String,
    pub home: Option<StateSide>,
    pub away: Option<StateSide>,
    #[serde(default)]
    pub hud: Option<String>,
    #[serde(default)]
    pub skills: Option<String>,
    #[serde(default)]
    pub bets: Option<serde_json::Value>,
    #[serde(default)]
    pub recent: Vec<Recent>,
    #[serde(default)]
    pub queue: Vec<QueueItem>,
    #[serde(default)]
    pub now: Option<u64>,
}

impl State {
    pub fn fetch() -> Result<State> {
        let body = fetch(STATE_URL)?;
        serde_json::from_str(&body).map_err(|e| Error::Parse(format!("the live state: {e}")))
    }

    /// Picks are open for the duel on screen.
    pub fn picks_open(&self) -> bool {
        self.bets.as_ref().and_then(|b| b.get("state")).and_then(|s| s.as_str()) == Some("open")
    }

    /// Something is happening on stream: a duel is up or being fought.
    pub fn busy(&self) -> bool {
        self.live || (self.home.is_some() && self.away.is_some() && self.picks_open())
    }
}

pub type Sides<T> = HashMap<String, T>;

/// {side: {stat: StatLine}} from the HUD stats grid.
pub fn parse_stats(hud: &str) -> Sides<HashMap<String, StatLine>> {
    let mut out: Sides<HashMap<String, StatLine>> = HashMap::new();
    out.insert("home".into(), HashMap::new());
    out.insert("away".into(), HashMap::new());
    let Some(start) = regex!(r#"<div class="bc-st[^"]*""#).find(hud) else { return out };
    let grid = between(&hud[start.start()..], "<div", &[r#"<p class="bc-st-f""#, "</div></div>"]).unwrap_or("");
    let cells: Vec<(String, String)> = regex!(r#"(?s)<(?:b|span) class="(bc-st-(?:iv|b|t|n))[^"]*">(.*?)</(?:b|span)>"#)
        .captures_iter(grid)
        .map(|c| (c[1].to_string(), c[2].to_string()))
        .collect();
    // each stat row is: iv(home) base(home) total(home) name total(away) base(away) iv(away)
    for i in 0..cells.len() {
        if cells[i].0 == "bc-st-n" && i >= 3 && i + 3 < cells.len() {
            let name = text(&cells[i].1);
            let line = |iv: usize, base: usize, total: usize| StatLine {
                iv: num(&cells[iv].1),
                base: num(&cells[base].1),
                lv70: num(&cells[total].1),
            };
            out.get_mut("home").unwrap().insert(name.clone(), line(i - 3, i - 2, i - 1));
            out.get_mut("away").unwrap().insert(name, line(i + 3, i + 2, i + 1));
        }
    }
    out
}

/// {side: [moves]} from the state's skills panel.
pub fn parse_skills(skills_html: &str) -> Sides<Vec<Move>> {
    let mut out = HashMap::new();
    for side in ["home", "away"] {
        let sect = between(skills_html, &format!(r#"<div class="skl skl-{side}""#), &[r#"<div class="skl skl-"#]).unwrap_or("");
        let moves = regex!(r#"(?s)<span class="skl-lv">(\d+)</span>.*?alt="([^"]*)".*?<span class="skl-nm[^"]*">(.*?)</span>.*?<span class="skl-p">(\d+)</span>"#)
            .captures_iter(sect)
            .map(|c| Move { lv: c[1].parse().unwrap_or(1), el: c[2].to_lowercase(), name: text(&c[3]), power: c[4].parse().unwrap_or(0.0) })
            .collect();
        out.insert(side.to_string(), moves);
    }
    out
}

/// {side: [passives]}; "unknown until the bell" when the site hides them.
pub fn parse_passives(hud: &str) -> Sides<Vec<String>> {
    let mut out = HashMap::new();
    for side in ["home", "away"] {
        let pat = format!(r#"(?s)<div class="bc-c is-{side}".*?<span class="bc-pass-v">(.*?)</span></div>"#);
        let raw = Regex::new(&pat).unwrap().captures(hud).map(|c| c[1].to_string()).unwrap_or_default();
        let items: Vec<String> = regex!(r"(?s)<i[^>]*>(.*?)</i>").captures_iter(&raw).map(|c| text(&c[1])).collect();
        let v = text(&raw);
        let list = if v.to_uppercase().contains("NAMED") {
            vec!["unknown until the bell".to_string()]
        } else if !items.is_empty() {
            items
        } else if v.is_empty() || v.eq_ignore_ascii_case("none") {
            vec![]
        } else {
            vec![v]
        };
        out.insert(side.to_string(), list);
    }
    out
}

/// The tale of the tape: rows of (key, home, away), plus the type edge and the site's verdict.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Tale {
    pub rows: Vec<(String, String, String)>,
    pub type_edge: String,
    pub verdict: String,
}

impl Tale {
    pub fn get(&self, key: &str) -> Option<(&str, &str)> {
        self.rows.iter().find(|r| r.0 == key).map(|r| (r.1.as_str(), r.2.as_str()))
    }

    /// Species records as [[home w, l], [away w, l]].
    pub fn species_records(&self) -> [[f64; 2]; 2] {
        let parse = |s: &str| {
            let n: Vec<f64> = regex!(r"\d+").find_iter(s).filter_map(|m| m.as_str().parse().ok()).collect();
            [n.first().copied().unwrap_or(0.0), n.get(1).copied().unwrap_or(0.0)]
        };
        match self.get("SPECIES RECORD") {
            Some((h, a)) => [parse(h), parse(a)],
            None => [[0.0; 2]; 2],
        }
    }
}

pub fn parse_tale(hud: &str) -> Tale {
    let rows = regex!(r#"(?s)<div class="bc-tt-r[^"]*"><span class="bc-tt-v is-home[^"]*">(?P<h>.*?)</span><span class="bc-tt-k">(?P<k>[^<]*)</span><span class="bc-tt-v is-away[^"]*">(?P<a>.*?)</span></div>"#)
        .captures_iter(hud)
        .map(|c| {
            let clean = |s: &str| regex!(r"\s+").replace_all(&text(s), "").to_string();
            (c["k"].to_string(), clean(&c["h"]), clean(&c["a"]))
        })
        .collect();
    let type_edge = regex!(r#"<p class="bc-tt-f(?: is-(home|away))?">TYPE EDGE (?:<span class="bc-side">\w+</span> )?<b>(.*?)</b>"#)
        .captures(hud)
        .map(|c| format!("{}{}", c.get(1).map(|s| format!("{}: ", s.as_str().to_uppercase())).unwrap_or_default(), text(&c[2])))
        .unwrap_or_else(|| "?".into());
    let verdict =
        regex!(r#"(?s)<span class="bc-w-lead[^"]*">(.*?)</span>"#).captures(hud).map(|c| text(&c[1])).unwrap_or_else(|| "?".into());
    Tale { rows, type_edge, verdict }
}

// --- duel lists (pal pages, rivalry pages) ---

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RowSide {
    pub side: String,
    pub won: bool,
    pub owner: String,
    pub pal: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DuelRow {
    pub id: Option<u64>,
    pub when: String,
    pub sides: [RowSide; 2],
}

impl DuelRow {
    pub fn winner(&self) -> &RowSide {
        if self.sides[0].won {
            &self.sides[0]
        } else {
            &self.sides[1]
        }
    }
}

pub fn duel_rows(page: &str) -> Vec<DuelRow> {
    regex!(r#"(?s)<li class="dl-row">(.*?)</li>"#)
        .captures_iter(page)
        .filter_map(|row| {
            let row = &row[1];
            let sides: Vec<RowSide> = regex!(r#"(?s)<div class="dl-f dl-(home|away) is-(won|lost)".*?<a class="dl-n[^"]*"[^>]*>(.*?)</a>.*?<span class="dl-paln" title="([^"]*)""#)
                .captures_iter(row)
                .map(|c| RowSide { side: c[1].into(), won: &c[2] == "won", owner: text(&c[3]), pal: text(&c[4]) })
                .collect();
            let when = regex!(r#"(?s)<span class="dl-when"[^>]*>(.*?)</span>"#)
                .captures(row)
                .map(|c| text(&regex!(r"(?s)<small.*?</small>").replace_all(&c[1], "")))
                .unwrap_or_default();
            let id = regex!(r"/duel/(\d+)").captures(row).and_then(|c| c[1].parse().ok());
            let [a, b]: [RowSide; 2] = sides.try_into().ok()?;
            Some(DuelRow { id, when, sides: [a, b] })
        })
        .collect()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Rivalry {
    pub summary: String,
    /// (owner, wins here, overall record)
    pub owners: Vec<(String, u32, String)>,
    pub meetings: Vec<DuelRow>,
}

pub fn parse_rivalry(page: &str) -> Rivalry {
    Rivalry {
        summary: regex!(r#"(?s)<p class="sub">(.*?)</p>"#).captures(page).map(|c| squash(&c[1])).unwrap_or_else(|| "first meeting".into()),
        owners: regex!(r#"(?s)<p class="vs-name">(.*?)</p><p class="vs-wins"><b>(\d+)</b>.*?<p class="vs-all">(.*?)</p>"#)
            .captures_iter(page)
            .map(|c| (text(&c[1]), c[2].parse().unwrap_or(0), text(&c[3])))
            .collect(),
        meetings: duel_rows(page),
    }
}

pub fn owner_rivalry(home_key: &str, away_key: &str) -> Result<Rivalry> {
    Ok(parse_rivalry(&fetch(&format!("{BASE}/vs/{home_key}/{away_key}"))?))
}

/// Every duel list page for a species (/pal/<slug>?p=N), up to max_pages.
pub fn pal_pages(slug: &str, max_pages: usize) -> Result<Vec<String>> {
    let first = fetch(&format!("{BASE}/pal/{slug}?p=1"))?;
    let pages = regex!(r"Page \d+ of (\d+)").captures(&first).and_then(|c| c[1].parse::<usize>().ok()).unwrap_or(1).min(max_pages);
    let mut out = vec![first];
    for n in 2..=pages {
        out.push(fetch(&format!("{BASE}/pal/{slug}?p={n}"))?);
    }
    Ok(out)
}

// --- species pages (/pal/<slug>) ---

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Species {
    pub pal: String,
    pub slug: String,
    pub types: Vec<String>,
    pub power: Option<u32>,
    pub base_hp: Option<u32>,
    pub base_atk: Option<u32>,
    pub base_def: Option<u32>,
    pub moves: Vec<Move>,
    /// "Who owns one": owners listed on the page
    pub owners: Vec<String>,
}

pub fn parse_moveset(page: &str) -> Vec<Move> {
    regex!(r#"(?s)<li class="mv-r[^"]*">(.*?)</li>"#) // the strongest move is "mv-r is-fin"
        .captures_iter(page)
        .filter_map(|c| {
            let li = &c[1];
            let name = regex!(r#"(?s)<span class="mv-n[^"]*">(.*?)</span>"#).captures(li)?;
            let power = regex!(r#"(?s)<span class="mv-p">.*?(\d+)</span>"#).captures(li)?;
            Some(Move {
                name: text(&name[1]),
                power: power[1].parse().unwrap_or(0.0),
                el: regex!(r#"class="mv-el"[^>]*alt="([^"]*)""#)
                    .captures(li)
                    .map(|e| e[1].to_lowercase())
                    .unwrap_or_else(|| "neutral".into()),
                lv: regex!(r#"(?s)<span class="mv-lv">.*?(\d+)</span>"#).captures(li).and_then(|l| l[1].parse().ok()).unwrap_or(1),
            })
        })
        .collect()
}

pub fn parse_species(page: &str, slug: &str) -> Species {
    let hero = regex!(r#"(?s)<div class="pal-hero-tags">(.*?)</div>"#).captures(page).map(|c| c[1].to_string()).unwrap_or_default();
    let stat = |k: &str| {
        Regex::new(&format!(r#"<div class="stat-v">(\d+)</div><div class="stat-k">Base {k}</div>"#))
            .unwrap()
            .captures(page)
            .and_then(|c| c[1].parse().ok())
    };
    let owners_sect = between(page, "<h2>Who owns one", &["<h2>"]).unwrap_or("");
    Species {
        pal: regex!(r"(?s)<h1>(.*?)</h1>").captures(page).map(|c| text(&c[1])).unwrap_or_else(|| slug.into()),
        slug: slug.into(),
        types: regex!(r"elements/([a-z]+)\.png").captures_iter(&hero).map(|c| c[1].to_string()).collect(),
        power: regex!(r"Power (\d+) out of 100").captures(page).and_then(|c| c[1].parse().ok()),
        base_hp: stat("HP"),
        base_atk: stat("attack"),
        base_def: stat("defence"),
        moves: parse_moveset(page),
        owners: regex!(r#"href="/u/[^"]+"[^>]*>([^<]+)</a>"#).captures_iter(owners_sect).map(|c| text(&c[1])).collect(),
    }
}

pub fn species(slug: &str) -> Result<Species> {
    Ok(parse_species(&fetch(&format!("{BASE}/pal/{slug}"))?, slug))
}

/// Every species' page slug, from /pals.
pub fn species_slugs() -> Result<Vec<String>> {
    let page = fetch(&format!("{BASE}/pals"))?;
    let mut slugs: Vec<String> = regex!(r#"href="/pal/([a-z0-9-]+)""#).captures_iter(&page).map(|c| c[1].to_string()).collect();
    slugs.sort();
    slugs.dedup();
    Ok(slugs)
}

// --- a finished duel (/duel/<id>?show=1) ---

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Hit {
    pub side: String,
    pub t: u32,
    /// "" for unnamed hits (likely damage over time)
    #[serde(rename = "move")]
    pub mv: String,
    /// % of the target's HP
    pub pct: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Duel {
    pub hits: Vec<Hit>,
    pub home_casts: u32,
    pub away_casts: u32,
    pub home_owner: Option<String>,
    pub away_owner: Option<String>,
    pub home_pal: Option<String>,
    pub away_pal: Option<String>,
    pub home_slug: Option<String>,
    pub away_slug: Option<String>,
    pub winner: Option<String>,
    pub fought_at: Option<String>,
    pub ko: Option<String>,
    pub winner_hp_left: Option<u32>,
}

pub fn parse_duel(page: &str) -> Duel {
    let mut d = Duel {
        hits: regex!(r#"(?s)<span class="lp-tip-e is-(home|away) is-h" data-t="(\d+)"[^>]*><i>\w+ HIT</i>\s*(.*?)\s*<b>(\d+)</b>"#)
            .captures_iter(page)
            .map(|c| Hit { side: c[1].into(), t: c[2].parse().unwrap_or(0), mv: text(&c[3]), pct: c[4].parse().unwrap_or(0.0) })
            .collect(),
        home_casts: page.matches(r#"<span class="lp-tip-e is-home is-c""#).count() as u32,
        away_casts: page.matches(r#"<span class="lp-tip-e is-away is-c""#).count() as u32,
        ..Default::default()
    };
    if let Some(res) = regex!(r#"(?s)<h2>Result</h2>.*?<div class="clip-head">(.*?)</div>"#).captures(page) {
        for c in regex!(r#"(?s)<span class="clip-side clip-(home|away) is-(won|lost)">.*?<a class="clip-who"[^>]*>(.*?)</a>.*?<a class="clip-pal" href="/pal/([^"]+)">(.*?)</a>"#)
            .captures_iter(&res[1])
        {
            let (owner, pal, slug) = (Some(text(&c[3])), Some(text(&c[5])), Some(c[4].to_string()));
            if &c[1] == "home" {
                (d.home_owner, d.home_pal, d.home_slug) = (owner, pal, slug);
            } else {
                (d.away_owner, d.away_pal, d.away_slug) = (owner, pal, slug);
            }
            if &c[2] == "won" {
                d.winner = Some(c[1].to_string());
            }
        }
    }
    d.fought_at = regex!(r#"<time datetime="([^"]+)""#).captures(page).map(|c| c[1].to_string());
    d.ko = regex!(r#"<span class="stat-v">([\d:]+)</span><span class="stat-k">K.O. at</span>"#).captures(page).map(|c| c[1].to_string());
    d.winner_hp_left = regex!(r"HP (\d+)%").captures(&text(page)).and_then(|c| c[1].parse().ok());
    d
}

pub fn duel(id: u64) -> Result<Duel> {
    Ok(parse_duel(&fetch(&format!("{BASE}/duel/{id}?show=1"))?))
}

// --- players (/u/<name>) ---

/// A pal in a player's box: species, talent rolls (IVs) and rerolls.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BoxPal {
    pub pal: String,
    pub slug: String,
    pub tier: String,
    pub power: Option<u32>,
    pub types: Vec<String>,
    /// stat -> (Lv 70 value, IV)
    pub stats: HashMap<String, (f64, f64)>,
    pub passives: Vec<String>,
    pub rerolls: u32,
    /// the IVs it hatched with, shown once rerolled
    pub born: HashMap<String, f64>,
    pub alpha: bool,
}

pub fn parse_roster(page: &str) -> Vec<BoxPal> {
    let Some(sect) = between(page, r#"<section class="sect" id="pals">"#, &["</section>"]) else { return vec![] };
    regex!(r#"(?s)<article class="pal[^"]*">(.*?)</article>"#)
        .captures_iter(sect)
        .filter_map(|c| {
            let a = &c[1];
            let name = regex!(r#"<h3 class="pal-n"><a href="/pal/([^"]+)">(.*?)</a>"#).captures(a)?;
            let foot = between(a, r#"<div class="pal-foot">"#, &["</div>"]).unwrap_or("");
            Some(BoxPal {
                slug: name[1].to_string(),
                pal: text(&name[2]),
                tier: regex!(r#"title="Tier (\w)"#).captures(a).map(|t| t[1].to_string()).unwrap_or_default(),
                power: regex!(r#"pwr-v">(\d+)"#).captures(a).and_then(|p| p[1].parse().ok()),
                types: regex!(r"el el-(\w+)").captures_iter(a).map(|e| e[1].to_string()).collect(),
                stats: regex!(r#"talent roll (\d+) out of 100"><span class="st-k">(\w+)</span><span class="st-v">([\d,]+)"#)
                    .captures_iter(a)
                    .map(|s| (s[2].to_string(), (s[3].replace(',', "").parse().unwrap_or(0.0), s[1].parse().unwrap_or(0.0))))
                    .collect(),
                passives: regex!(r#"<span class="pass[^"]*">(.*?)</span>"#)
                    .captures_iter(foot)
                    .map(|p| text(&p[1]))
                    .filter(|p| !p.eq_ignore_ascii_case("no combat passives"))
                    .collect(),
                rerolls: regex!(r"RE-ROLLED &times;(\d+)").captures(a).and_then(|r| r[1].parse().ok()).unwrap_or(0),
                born: regex!(r"<span>(HP|ATK|DEF) (\d+)</span>")
                    .captures_iter(a)
                    .map(|b| (b[1].to_string(), b[2].parse().unwrap_or(0.0)))
                    .collect(),
                alpha: a.contains("alpha-tag"),
            })
        })
        .collect()
}

pub fn roster(user_key: &str) -> Result<Vec<BoxPal>> {
    Ok(parse_roster(&fetch(&format!("{BASE}/u/{user_key}"))?))
}

// --- your picks (/u/<name>?bp=<page>#bets, only shown when signed in) ---

/// One of your bets as the picks list shows it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Pick {
    /// its place in your whole picks history, 1 = your first bet ever (doesn't move as new bets are added)
    pub seq: u64,
    /// recent rows link their duel; older ones don't
    pub duel_id: Option<u64>,
    pub home_owner: String,
    pub away_owner: String,
    pub home_pal: String,
    pub away_pal: String,
    /// "home" or "away"
    pub backed: String,
    pub winner: Option<String>,
    pub stake: u64,
    /// credits won (+) or lost (-), None while the fight is on
    pub result: Option<i64>,
    /// "5h ago": the list has no dates
    pub ago: String,
}

/// A page of the picks list. The site shows 5 a page, newest first, and has no other way to list them.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PicksPage {
    pub picks: Vec<Pick>,
    pub page: u32,
    pub pages: u32,
    pub total: u64,
}

/// None when the page has no picks list: it's only there when signed in.
pub fn parse_picks(page: &str) -> Option<PicksPage> {
    let sect = between(page, r#"<section class="panel" id="bets">"#, &["</section>"])?;
    let counts = regex!(r#"<span class="muted">([\d,]+)-([\d,]+) of ([\d,]+)</span>"#).captures(sect);
    let n = |c: &regex::Captures, i: usize| c[i].replace(',', "").parse::<u64>().unwrap_or(0);
    let (first, total) = counts.as_ref().map(|c| (n(c, 1), n(c, 3))).unwrap_or((1, 0));
    let at = regex!(r"Page (\d+) of (\d+)").captures(sect);
    let picks = sect
        .split(r#"<li class="dl-row">"#)
        .skip(1)
        .enumerate()
        .filter_map(|(i, row)| {
            // each side from its opening tag's remaining classes ("is-won is-mark") to the end of its block
            let half = |side: &str, until: &str| {
                let start = format!(r#"<div class="dl-f dl-{side} "#);
                between(row, &start, &[until]).map(|h| &h[start.len()..]).unwrap_or("")
            };
            let (home, away) = (half("home", r#"<span class="dl-vs">"#), half("away", r#"<div class="dl-out">"#));
            let who = |h: &str| regex!(r#"<a class="dl-n[^"]*" href="/u/[^"]*"[^>]*>(.*?)</a>"#).captures(h).map(|c| text(&c[1]));
            let pal = |h: &str| regex!(r#"<span class="dl-paln"[^>]*><a href="/pal/[^"]*">(.*?)</a>"#).captures(h).map(|c| text(&c[1]));
            let classes = |h: &str| regex!(r#"^([^"]*)""#).captures(h).map(|c| c[1].to_string()).unwrap_or_default();
            let (hc, ac) = (classes(home), classes(away));
            let fig = |k: &str| {
                Regex::new(&format!(r#"<span class="dl-k">{k}</span><span class="dl-v[^"]*">([^<]*)</span>"#))
                    .unwrap()
                    .captures(row)
                    .map(|c| text(&c[1]).replace([',', '+'], "").replace('\u{2212}', "-"))
            };
            Some(Pick {
                seq: (total + 1).saturating_sub(first + i as u64),
                duel_id: regex!(r#"data-clip="(\d+)""#).captures(row).and_then(|c| c[1].parse().ok()),
                home_owner: who(home)?,
                away_owner: who(away)?,
                home_pal: pal(home)?,
                away_pal: pal(away)?,
                backed: if hc.contains("is-mark") {
                    "home"
                } else if ac.contains("is-mark") {
                    "away"
                } else {
                    return None;
                }
                .into(),
                winner: if hc.contains("is-won") {
                    Some("home".into())
                } else if ac.contains("is-won") {
                    Some("away".into())
                } else {
                    None
                },
                stake: fig("Credits")?.parse().ok()?,
                result: fig("Result").and_then(|r| r.parse().ok()),
                ago: regex!(r#"<span class="dl-when">(.*?)</span>"#).captures(row).map(|c| squash(&c[1])).unwrap_or_default(),
            })
        })
        .collect();
    Some(PicksPage {
        picks,
        page: at.as_ref().and_then(|c| c[1].parse().ok()).unwrap_or(1),
        pages: at.as_ref().and_then(|c| c[2].parse().ok()).unwrap_or(1),
        total,
    })
}

/// Your balance from the signed-in page header ("248k credits"), to the nearest thousand when that's all it shows.
pub fn parse_balance(page: &str) -> Option<u64> {
    let link = regex!(r##"(?s)href="/u/[^"#]+#balance"[^>]*>(.*?)</a>"##).captures(page)?;
    let shown = squash(&link[1]);
    let c = regex!(r"([\d.,]+)\s*([kKmM]?)").captures(&shown)?;
    let v: f64 = c[1].replace(',', "").parse().ok()?;
    Some(
        (v * match &c[2] {
            "k" | "K" => 1e3,
            "m" | "M" => 1e6,
            _ => 1.0,
        }) as u64,
    )
}

/// A page of your picks, with the session cookie. Ok(None) when the site didn't show them (signed out).
pub fn picks(user_key: &str, page: u32, cookie: &str) -> Result<Option<(PicksPage, Option<u64>)>> {
    let body = fetch_once(&format!("{BASE}/u/{user_key}?bp={page}"), Some(cookie))?;
    Ok(parse_picks(&body).map(|p| (p, parse_balance(&body))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(name: &str) -> String {
        std::fs::read_to_string(format!("{}/tests/pages/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    #[test]
    fn live_state() {
        let s: State = serde_json::from_str(&page("state-leaflit-wumpo.json")).unwrap();
        let (h, a) = (s.home.clone().unwrap(), s.away.clone().unwrap());
        assert_eq!((h.pal.as_str(), a.pal.as_str()), ("Leaflit Slimey", "Wumpo"));
        assert!(s.picks_open());
        let hud = s.hud.unwrap();
        let stats = parse_stats(&hud);
        assert_eq!(stats["away"]["HP"], StatLine { iv: Some(46.0), base: Some(140.0), lv70: Some(8705.0) });
        assert_eq!(stats["home"]["HP"].lv70, None, "home stats are hidden until the bell");
        let tale = parse_tale(&hud);
        assert_eq!(tale.species_records(), [[57.0, 99.0], [7.0, 8.0]]);
        assert_eq!(tale.type_edge, "no type edge either way");
        let skills = parse_skills(&s.skills.unwrap());
        assert_eq!(skills["home"].len(), 8);
        assert!(skills["away"].iter().any(|m| m.name == "Diamond Rain" && m.power == 600.0 && m.el == "ice"));
    }

    #[test]
    fn finished_duel() {
        let d = parse_duel(&page("duel-4702.html"));
        assert_eq!(d.winner.as_deref(), Some("home"));
        assert_eq!((d.home_pal.as_deref(), d.away_pal.as_deref()), (Some("Chillet Lux"), Some("Chikipi Ryu")));
        assert_eq!(d.ko.as_deref(), Some("0:31"));
        assert_eq!(d.hits.len(), 24);
        assert_eq!(d.winner_hp_left, Some(11));
    }

    #[test]
    fn species_page() {
        let sp = parse_species(&page("pal-bushi-noct.html"), "bushi-noct");
        assert_eq!(sp.pal, "Bushi Noct");
        assert_eq!(sp.types, ["fire", "dark"]);
        assert_eq!((sp.base_hp, sp.base_atk, sp.base_def), (Some(80), Some(130), Some(80)));
        assert_eq!(sp.moves.len(), 8);
        assert!(sp.moves.iter().any(|m| m.name == "Dark Whisp" && m.power == 600.0), "the strongest move is included");
        assert!(!duel_rows(&page("pal-bushi-noct.html")).is_empty());
    }

    #[test]
    fn rivalry_page() {
        let r = parse_rivalry(&page("vs-player-09-player-49.html"));
        assert!(r.summary.contains("player-09"));
        assert_eq!(r.owners.len(), 2);
        assert!(r.meetings.len() >= 10);
    }

    #[test]
    fn player_box() {
        let pals = parse_roster(&page("user-trongateslegacy.html"));
        assert_eq!(pals.len(), 12);
        let nyafia = pals.iter().find(|p| p.pal == "Nyafia Primo").unwrap();
        assert_eq!(nyafia.stats["HP"], (7518.0, 89.0));
        assert_eq!(nyafia.rerolls, 0);
    }

    #[test]
    fn picks_pages() {
        let p = parse_picks(&page("picks-page-1.html")).unwrap();
        assert_eq!((p.page, p.pages, p.total, p.picks.len()), (1, 3, 13, 5));
        let newest = &p.picks[0];
        assert_eq!((newest.seq, newest.duel_id, newest.backed.as_str()), (13, Some(9020), "home"));
        assert_eq!((newest.stake, newest.result, newest.winner.as_deref()), (500, Some(277), Some("home")));
        assert_eq!((newest.home_pal.as_str(), newest.away_owner.as_str(), newest.ago.as_str()), ("Lamball", "player-66", "5h ago"));
        assert_eq!((p.picks[1].backed.as_str(), p.picks[1].result), ("away", Some(-5000)));
        assert_eq!(p.picks[4].seq, 9);
        assert_eq!(parse_balance(&page("picks-page-1.html")), Some(248_000));
        let oldest = parse_picks(&page("picks-page-3.html")).unwrap();
        assert_eq!(oldest.picks.iter().map(|p| p.seq).collect::<Vec<_>>(), [3, 2, 1]);
        assert!(oldest.picks.iter().all(|p| p.duel_id.is_none()), "older rows have no duel link");
        assert!(parse_picks(&page("user-trongateslegacy.html")).is_none(), "signed out, the profile has no picks list");
    }
}
