//! Headless UI tests: the real pages in ui/, rendered by headless Chrome with a stand-in for the Tauri bridge that
//! serves what the app's commands would return (built here from the repo's fixtures, no network). Nothing appears on
//! screen and the user's own Chrome profile isn't touched. Skipped when Chrome isn't installed (set CHROME to point
//! at it). AUGUR_UI_SHOTS=<folder> also saves a screenshot of every tab there (AUGUR_UI_SIZE=1024x576 for another size).

use crate::settings::AppSettings;
use crate::views;
use augur_engine::seed::{page, root};
use augur_engine::stake::{self, StakeSettings};
use augur_engine::{bets, learn, model, predict, seed, site};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Chrome is needed; locally the tests skip without it, in CI (CI is set) they fail rather than pass unrun.
fn have_chrome() -> bool {
    if chrome().is_some() {
        return true;
    }
    assert!(std::env::var_os("CI").is_none(), "no Chrome on this CI runner: the UI tests can't run (set CHROME)");
    eprintln!("skipped: no Chrome found (set CHROME)");
    false
}

pub fn chrome() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CHROME") {
        return Some(PathBuf::from(p));
    }
    [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.exists())
}

/// What each command returns, from the seed (augur_engine::seed): 19 made-up calls, duel #4702 with its tape, a
/// saved live state (Leaflit Slimey vs Wumpo, home stats hidden until the bell), TrongatesLegacy's pal box and a
/// made-up picks list.
pub fn fixture_data() -> Value {
    let conn = seed::store();
    seed::import_bets(&conn);
    seed::store_pet_and_hall(&conn);
    let s = model::Settings::default();
    let state = seed::state("state-leaflit-wumpo.json");
    // sync times two minutes ago, so they read like real ones
    let recent = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64 - 120_000;
    let p = predict::predict(&state, &predict::Options { lookups: false, db_path: PathBuf::from("/nonexistent"), settings: s.clone() })
        .unwrap();
    let stake = stake::suggest(&p.pick, p.prob, &StakeSettings::default(), &bets::odds(&conn).unwrap());
    let live = views::LiveView {
        title: state.title.clone(),
        picks_open: true,
        prediction: Some(p),
        stake: Some(stake),
        queue: state.queue.iter().filter(|q| q.pos > 0).take(3).cloned().collect(),
        synced_at: recent,
        today: (14, 19),
        first_run: false,
        ..Default::default()
    };
    let roster = Ok(site::parse_roster(&page("user-trongateslegacy.html")));
    let preview: Vec<_> = [0.55, 0.6, 0.65, 0.7, 0.75, 0.8, 0.85, 0.9]
        .map(|p| (p, stake::suggest("home", p, &StakeSettings::default(), &bets::odds(&conn).unwrap())))
        .to_vec();
    let model_view = {
        // a made-up self-tuning run: one change kept, one held, the stat settings still waiting for calls
        let mut l = learn::learn(&conn, &s, recent - 3_600_000).unwrap();
        l.past_duels = 4_702;
        l.check = learn::Check {
            past_n: 1_176,
            past_before: 0.6611,
            past_after: 0.6585,
            past_right_before: 0.62,
            past_right_after: 0.627,
            live_n: 5,
            live_before: 0.83,
            live_after: 0.83,
        };
        for k in &mut l.knobs {
            if k.source == learn::Source::PastDuels {
                (k.status, k.have) = ("held".into(), 4_702);
            }
        }
        l.knobs[0] = learn::KnobReport {
            key: "TYPE_BONUS".into(),
            value: 1.5,
            was: 1.4,
            source: learn::Source::PastDuels,
            status: "changed".into(),
            have: 4_702,
            need: 200,
        };
        learn::save(&conn, &l).unwrap();
        views::model_view(&conn, &s, true, None, Default::default(), recent).unwrap()
    };
    json!({
        "live": live,
        "history": views::history_view(&conn, &s, "all", 0).unwrap(),
        "bets_view": views::bets_view(&conn, &StakeSettings::default(), "all", 0, Default::default()).unwrap(),
        "import_bets": null,
        "species_list": augur_engine::history::species_records(&conn).unwrap(),
        "species": augur_engine::history::species_detail(&conn, "Chillet Lux").unwrap(),
        "player": views::player_view(&conn, &s, "TrongatesLegacy", roster).unwrap(),
        "model_view": model_view,
        // checking at random times, so Settings shows the range
        "get_settings": AppSettings { poll_secs: 4, poll_max_secs: Some(12), ..Default::default() },
        "stake_preview": preview,
        "sync_view": crate::sync::SyncView {
            on: true,
            folder: Some(PathBuf::from("/Users/you/Library/CloudStorage/OneDrive-Personal")),
            name: "Studio Mac".into(),
            what: Default::default(),
            last_sync: Some(recent),
            error: None,
            history: vec![augur_engine::sync::Change { at: recent, from: "Gaming PC".into(), summary: "strategy, max from".into() }],
            others: vec![augur_engine::sync::DeviceFile {
                device: "pc".into(),
                name: "Gaming PC".into(),
                os: "Windows".into(),
                app: "0.1.0".into(),
                at: recent,
                settings: Default::default(),
                calls: 19,
            }],
            folders: vec![augur_engine::sync::Folder {
                id: "onedrive".into(),
                name: "OneDrive".into(),
                path: PathBuf::from("/Users/you/Library/CloudStorage/OneDrive-Personal"),
            }],
        },
        "account": { "signed_in": true, "name": "trongateslegacy" },
        "you_view": views::you_view(&conn, &StakeSettings::default()).unwrap(),
        "start_fresh": null,
        // up to date, and (swapped in by the "updates" page and the tray test) a new version downloaded and ready
        "update_view": crate::updates::UpdateView { current: "0.1.1".into(), checked_at: Some(recent), ..Default::default() },
        "update_ready": crate::updates::UpdateView {
            current: "0.1.1".into(),
            checked_at: Some(recent),
            ready: true,
            latest: Some(crate::updates::Release {
                version: "0.2.0".into(),
                notes: "## What's Changed\n* Check for updates in the app by @player-01 in https://github.com/TrongatesLegacy/augur/pull/8\n* Faster History by @player-01 in https://github.com/TrongatesLegacy/augur/pull/9\n\n**Full Changelog**: https://github.com/TrongatesLegacy/augur/compare/0.1.1...0.2.0".into(),
                published: Some(recent as i64),
            }),
            ..Default::default()
        },
        "update_check": null,
        "update_install": null,
        "update_seen": null,
        "fit_popover": null,
        "quit_app": null,
        "about": { "version": "0.1.1", "data_dir": "/Users/you/Library/Application Support/Augur", "counts": [19, 20, 1, 1, 13] },
        // what Live and the popover show between duels (swapped in by the "offair" page and the off-air tray test)
        "off_air": views::off_air(&conn, &StakeSettings::default()).unwrap(),
        "backfill_progress": augur_engine::backfill::Progress { phase: "duels".into(), done: 1200, total: 4702, ..Default::default() },
        "pal_image": null,
        // a stand-in Kick picture, so the screenshots show faces swapped in
        "learned_settings": model::Settings::default(),
        "learn_now": null,
        "player_image": "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAxMCAxMCI+PHJlY3Qgd2lkdGg9IjEwIiBoZWlnaHQ9IjEwIiBmaWxsPSIjNTNmYzE4Ii8+PGNpcmNsZSBjeD0iNSIgY3k9IjQiIHI9IjIiIGZpbGw9IiMwYjEwMjAiLz48cmVjdCB4PSIyIiB5PSI3IiB3aWR0aD0iNiIgaGVpZ2h0PSI0IiByeD0iMyIgZmlsbD0iIzBiMTAyMCIvPjwvc3ZnPg==",
    })
}

/// Serves ui/ over plain HTTP on 127.0.0.1 (Chrome won't load ES modules from file:// pages), with the entry page
/// rewritten as /test.html: the stand-in bridge goes in first, the report and driver at the end.
fn serve(entry: &str, data: &Value, driver: &str) -> u16 {
    let ui = root().join("ui");
    let page = std::fs::read_to_string(ui.join(entry)).unwrap();
    let stub = format!(
        r#"<script>
window.__errors = [];
window.onerror = (m, s, l) => window.__errors.push(m + ' @' + s + ':' + l);
window.addEventListener('unhandledrejection', (e) => window.__errors.push('rejection: ' + (e.reason && e.reason.message || e.reason)));
const DATA = {data};
window.__TAURI__ = {{
  core: {{ invoke: (cmd) => cmd in DATA ? Promise.resolve(JSON.parse(JSON.stringify(DATA[cmd]))) : Promise.reject(new Error('no stand-in for ' + cmd)) }},
  event: {{ listen: () => Promise.resolve(() => {{}}) }},
}};
</script>"#
    );
    let page = page.replacen("<head>", &format!("<head>{stub}"), 1).replacen(
        "</body>",
        &format!(r#"<pre id="report" style="display:none"></pre><script>{driver}</script></body>"#),
        1,
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let path = line.split_whitespace().nth(1).unwrap_or("/").split(['?', '#']).next().unwrap_or("/").to_string();
            let (status, kind, body) = if path == "/test.html" {
                ("200 OK", "text/html", page.clone().into_bytes())
            } else if path.contains("..") {
                ("404 Not Found", "text/plain", vec![])
            } else {
                match std::fs::read(ui.join(path.trim_start_matches('/'))) {
                    Ok(b) => {
                        let kind = match path.rsplit('.').next() {
                            Some("js") => "text/javascript",
                            Some("css") => "text/css",
                            Some("html") => "text/html",
                            _ => "application/octet-stream",
                        };
                        ("200 OK", kind, b)
                    }
                    Err(_) => ("404 Not Found", "text/plain", vec![]),
                }
            };
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {kind}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(&body));
        }
    });
    port
}

// Waits for the page's module to start (it sets window.augur), visits every tab and reports what each showed.
const MAIN_DRIVER: &str = r#"
(async () => {
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  for (let i = 0; i < 60 && !window.augur; i++) await sleep(100);
  const out = {};
  const only = (location.hash.match(/tab=(\w+)/) || [])[1];
  // text that spills out of its box: content wider than the element, unless it scrolls or trims with "…"
  out.spills = {};
  const spills = () =>
    [...document.querySelectorAll('#main *')]
      .filter((el) => {
        const cs = getComputedStyle(el);
        if (/auto|scroll/.test(cs.overflowX) || cs.textOverflow === 'ellipsis' || el.closest('svg') || !el.clientWidth || !el.textContent.trim()) return false;
        // a number (or a record like 1,000–950) broken over two lines
        const t = el.textContent.trim();
        if (/^[+−-]?[\d,.]+(%|×|k|M)?([–/ ]+[\d,.]+)?$/.test(t) && !el.children.length) {
          const range = document.createRange();
          range.selectNodeContents(el);
          if (new Set([...range.getClientRects()].map((r) => Math.round(r.top))).size > 1) return true;
        }
        if (el.scrollWidth > el.clientWidth + 2) {
          // only what's laid out counts: a decoration placed past the edge (and clipped) is meant to be
          const box = el.getBoundingClientRect();
          const kids = [...el.children].filter((c) => !/absolute|fixed/.test(getComputedStyle(c).position));
          return !el.children.length || kids.some((c) => c.getBoundingClientRect().right > box.right + 2 || c.scrollWidth > c.clientWidth + 2) || [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim());
        }
        // centred text spilling out both sides doesn't scroll: compare with the parent's edges instead
        const [r, p] = [el.getBoundingClientRect(), el.parentElement.getBoundingClientRect()];
        const pcs = getComputedStyle(el.parentElement);
        return cs.position !== 'absolute' && !/auto|scroll/.test(pcs.overflowX) && (r.left < p.left - 2 || r.right > p.right + 2);
      })
      .map((el) => `${el.tagName.toLowerCase()}.${[...el.classList].join('.')} (${el.clientWidth}px, needs ${el.scrollWidth}px): ${el.textContent.trim().replace(/\s+/g, ' ').slice(0, 50)}`);
  if (!window.augur) window.__errors.push('the page never started (a module failed to load)');
  else {
    for (const tab of only ? [only] : ['live', 'history', 'bets', 'pals', 'players', 'you', 'model', 'settings', 'staking', 'ownmodel', 'sync', 'updates', 'offair']) {
      // "ownmodel" is Settings › Model with self-tuning off and your own values
      const own = DATA.get_settings;
      if (tab === 'ownmodel') DATA.get_settings = { ...own, model_auto: false };
      // "updates" is Settings › Updates with a new version downloaded and ready (and its bar over the window)
      if (tab === 'updates') {
        DATA.update_view = DATA.update_ready;
        window.augur.showUpdate(DATA.update_ready);
      }
      // "bets" is History's Your bets view; "staking", "sync" and "updates" are Settings pages
      window.augur.state.view = tab === 'bets' ? 'bets' : 'calls';
      // "offair" is Live with no duel up, after a stream
      if (tab === 'offair') window.augur.state.live = { ...DATA.live, prediction: null, queue: [], off_air: DATA.off_air, title: 'Last duel' };
      else window.augur.state.live = DATA.live;
      window.augur.state.settingsPage = ['staking', 'sync', 'updates'].includes(tab) ? tab : tab === 'ownmodel' ? 'model' : 'general';
      // "welcome" is the fresh install's overlay, over Live
      await window.augur.show(tab === 'bets' ? 'history' : ['staking', 'sync', 'updates', 'ownmodel'].includes(tab) ? 'settings' : ['welcome', 'offair'].includes(tab) ? 'live' : tab);
      await sleep(300);
      out[tab] = document.getElementById('main').innerText;
      out.spills[tab] = spills();
      DATA.get_settings = own;
    }
  }
  out.sidebar = document.querySelector('.side')?.innerText || '';
  out.upbar = document.getElementById('upbar')?.innerText || '';
  // a fresh install's welcome, over everything
  window.augur?.showWelcome({ first_run: true });
  out.welcome = document.getElementById('welcome')?.innerText || '';
  if (only !== 'welcome') document.getElementById('welcome')?.remove();
  out.errors = window.__errors;
  document.getElementById('report').textContent = JSON.stringify(out);
})();
"#;

const TRAY_DRIVER: &str = r#"
// a new version downloaded and ready: the popover offers Restart to update
DATA.update_view = DATA.update_ready;
(async () => {
  document.body.style.width = '340px'; // the popover's width (headless Chrome's window can't be that narrow)
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  for (let i = 0; i < 60 && !window.augur; i++) await sleep(100);
  if (!window.augur) window.__errors.push('the popover never started (a module failed to load)');
  document.getElementById('report').textContent = JSON.stringify({ tray: document.getElementById('pop').innerText, errors: window.__errors });
})();
"#;

/// Renders the page in headless Chrome and returns its report.
fn render(port: u16, name: &str, shot: Option<(&Path, &str)>) -> Value {
    render_at(port, name, shot, shot_size())
}

/// The main window's size for screenshots: AUGUR_UI_SIZE=1024x576 (the smallest it goes), default 1280x720.
fn shot_size() -> (u32, u32) {
    std::env::var("AUGUR_UI_SIZE")
        .ok()
        .and_then(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))))
        .unwrap_or((1280, 720))
}

fn render_at(port: u16, name: &str, shot: Option<(&Path, &str)>, (w, h): (u32, u32)) -> Value {
    let chrome = chrome().expect("Chrome");
    // its own Chrome profile: tests run at the same time
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let run = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("augur-ui-{}-{name}-{run}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let url = format!("http://127.0.0.1:{port}/test.html");
    let base = |c: &mut Command| {
        c.arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--hide-scrollbars")
            .arg("--default-background-color=00000000") // the popover's window is transparent around its card
            .arg("--lang=en-GB") // dates read the same on every machine ("11 Nov")
            .arg(format!("--user-data-dir={}", dir.join("profile").display()))
            .arg("--virtual-time-budget=10000");
    };
    let mut c = Command::new(&chrome);
    base(&mut c);
    // the report is read at the same window size as the screenshots, so layout checks see what they show
    let out = c.arg(format!("--window-size={w},{h}")).arg("--dump-dom").arg(&url).output().expect("run Chrome");
    let dom = String::from_utf8_lossy(&out.stdout);
    let report = dom.split("<pre id=\"report\" style=\"display:none\">").nth(1).and_then(|s| s.split("</pre>").next()).unwrap_or("");
    let report = html_unescape(report);
    if let Some((folder, tab)) = shot {
        let mut c = Command::new(&chrome);
        base(&mut c);
        let _ = c
            .arg(format!("--window-size={w},{h}"))
            .arg(format!("--screenshot={}", folder.join(format!("{tab}.png")).display()))
            .arg(format!("{url}#tab={tab}"))
            .output();
    }
    let _ = std::fs::remove_dir_all(&dir);
    serde_json::from_str(&report)
        .unwrap_or_else(|e| panic!("no report from the {name} page ({e}); Chrome said: {}", String::from_utf8_lossy(&out.stderr)))
}

/// Credits as the pages show them: +2,105 / −500.
fn signed(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    format!(
        "{}{out}",
        if n > 0 {
            "+"
        } else if n < 0 {
            "\u{2212}"
        } else {
            ""
        }
    )
}

fn html_unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&#39;", "'").replace("&amp;", "&")
}

/// Every count 1,000 times bigger (a long-running install, a big balance), for the layout checks below. Ids,
/// times, rolls, ranks and positions keep their values.
fn bigger(v: &mut Value, key: &str) {
    let k = key.to_ascii_lowercase();
    let keep = ["id", "seq", "pos", "at", "ms", "n", "z"].contains(&k.as_str())
        || ["_id", "_at", "_ms"].iter().any(|e| k.ends_with(e))
        || ["iv", "tier", "secs", "hp_left", "zoom", "width", "height", "version"].iter().any(|e| k.contains(e))
        || k.starts_with("lv")
        || k.starts_with("page");
    match v {
        Value::Number(n) if !keep => {
            if let Some(i) = n.as_i64().filter(|i| *i != 0) {
                *v = json!(i * 1000);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|x| bigger(x, key)),
        Value::Object(map) => map.iter_mut().for_each(|(k, x)| bigger(x, k)),
        _ => {}
    }
}

/// Big numbers fit: with every count 1,000 times bigger, no text spills out of its box on any tab.
#[test]
fn big_numbers_fit() {
    if !have_chrome() {
        return;
    }
    let mut data = fixture_data();
    bigger(&mut data, "");
    let port = serve("index.html", &data, MAIN_DRIVER);
    // the size the window is laid out at (smaller windows zoom it down)
    let r = render_at(port, "main", None, (1280, 720));
    assert_eq!(r["errors"], json!([]), "script errors: {}", r["errors"]);
    let spills: Vec<String> = r["spills"]
        .as_object()
        .unwrap()
        .iter()
        .flat_map(|(tab, list)| list.as_array().unwrap().iter().map(move |x| format!("{tab}: {}", x.as_str().unwrap_or(""))))
        .collect();
    if let Some(folder) = std::env::var_os("AUGUR_UI_SHOTS").map(PathBuf::from) {
        let folder = folder.join("big");
        std::fs::create_dir_all(&folder).unwrap();
        for tab in
            ["live", "history", "bets", "pals", "players", "you", "model", "settings", "staking", "ownmodel", "sync", "updates", "offair"]
        {
            render(port, tab, Some((&folder, tab)));
        }
    }
    assert!(spills.is_empty(), "text spills out of its box with big numbers:\n{}", spills.join("\n"));
}

#[test]
fn every_tab_renders_headless() {
    if !have_chrome() {
        return;
    }
    let data = fixture_data();
    let r = render(serve("index.html", &data, MAIN_DRIVER), "main", None);
    assert_eq!(r["errors"], json!([]), "script errors: {}", r["errors"]);
    // compared lowercased: labels are styled uppercase and innerText reflects that
    let text = |tab: &str| r[tab].as_str().unwrap_or("").to_lowercase();
    assert!(text("live").contains("wumpo"), "Live: {}", text("live"));
    let p = &data["live"]["prediction"];
    let chance = format!("{}%", (p["prob"].as_f64().unwrap() * 100.0).round());
    assert!(text("live").contains(&chance), "Live shows the chance {chance}: {}", text("live"));
    assert!(text("live").contains(&p["lines"][0].as_str().unwrap().to_lowercase()), "Live shows the Yoda line");
    assert!(text("live").contains("some stats hidden until the bell"), "Live says the home stats are estimated");
    assert!(text("history").contains("14 / 19"), "History: {}", text("history"));
    assert!(
        text("you").contains("248,120")
            && text("you").contains("predictions")
            && text("you").contains("tournament champion")
            && text("you").contains("16 of 52")
            && text("you").contains("lamball cryst")
            && text("you").contains("4 of 356"),
        "You: {}",
        text("you")
    );
    assert!(text("sidebar").contains("you"), "signed in, the sidebar has You: {}", text("sidebar"));
    assert!(
        text("offair").contains("off air") && text("offair").contains("last stream") && text("offair").contains("latest calls"),
        "Live off air: {}",
        text("offair")
    );
    assert!(
        text("welcome").contains("welcome to augur")
            && text("welcome").contains("import a backup")
            && text("welcome").contains("start fresh"),
        "a fresh install's welcome: {}",
        text("welcome")
    );
    assert!(text("sidebar").contains("25%") && text("sidebar").contains("fetching"), "the sidebar shows the fetch: {}", text("sidebar"));
    let stake = data["live"]["stake"]["command"].as_str().unwrap();
    assert!(!stake.is_empty() && text("live").contains(&format!("copy {stake}")), "Live offers the bet {stake}: {}", text("live"));
    let b = &data["bets_view"]["comparison"];
    assert!(text("bets").contains("following augur") && text("bets").contains("deviated"), "Your bets: {}", text("bets"));
    assert!(
        text("bets").contains(&format!("{} left on the table", signed(b["missed_augur"].as_i64().unwrap()))),
        "Your bets totals missed fights apart: {}",
        text("bets")
    );
    assert!(text("bets").contains("no call"), "Your bets lists bets on fights Augur didn't call");
    assert!(
        text("pals").contains("chillet lux") && text("pals").contains("rocket slam"),
        "Pals shows the species and its real hits: {}",
        text("pals")
    );
    assert!(text("players").contains("nyafia primo"), "Players: {}", text("players"));
    assert!(
        text("model").contains("history test")
            && text("model").contains("learned from your duels")
            && text("model").contains("type edge bonus 1.4× → 1.5×"),
        "Model: {}",
        text("model")
    );
    assert!(
        text("model").contains("waits for real calls: 19 of 60") || text("model").contains("waits for real calls"),
        "Model: {}",
        text("model")
    );
    assert!(
        text("ownmodel").contains("tune itself") && text("ownmodel").contains("use the learned values"),
        "Settings › Model: {}",
        text("ownmodel")
    );
    assert!(
        text("settings").contains("chat lines")
            && text("settings").contains("notify with each result")
            && text("settings").contains("a new random wait"),
        "Settings: {}",
        text("settings")
    );
    assert!(text("staking").contains("what it would suggest") && text("staking").contains("sit out"), "Staking: {}", text("staking"));
    assert!(text("sync").contains("gaming pc") && text("sync").contains("back up"), "Sync & data: {}", text("sync"));
    assert!(
        text("updates").contains("augur 0.2.0 is ready")
            && text("updates").contains("restart to update")
            && text("updates").contains("faster history"),
        "Updates: {}",
        text("updates")
    );
    assert!(
        !text("updates").contains("@player-01") && !text("updates").contains("full changelog"),
        "the notes are tidied: {}",
        text("updates")
    );
    assert!(text("upbar").contains("restart to update"), "a ready update floats a bar over the window: {}", text("upbar"));

    if let Some(folder) = std::env::var_os("AUGUR_UI_SHOTS").map(PathBuf::from) {
        std::fs::create_dir_all(&folder).unwrap();
        let port = serve("index.html", &data, MAIN_DRIVER);
        for tab in [
            "live", "history", "bets", "pals", "players", "you", "model", "settings", "staking", "ownmodel", "sync", "welcome", "updates",
            "offair",
        ] {
            render(port, tab, Some((&folder, tab)));
        }
    }
}

#[test]
fn tray_popover_renders_headless() {
    if !have_chrome() {
        return;
    }
    let r = render(serve("tray.html", &fixture_data(), TRAY_DRIVER), "tray", None);
    assert_eq!(r["errors"], json!([]), "script errors: {}", r["errors"]);
    let tray = r["tray"].as_str().unwrap();
    assert!(tray.contains("WUMPO") || tray.contains("Wumpo"), "tray: {tray}");
    assert!(tray.contains("Copy for chat"));
    assert!(tray.contains("Copy bet"), "the tray copies the bet command: {tray}");
    assert!(tray.contains("Restart to update"), "the tray offers a ready update: {tray}");
    if let Some(folder) = std::env::var_os("AUGUR_UI_SHOTS").map(PathBuf::from) {
        std::fs::create_dir_all(&folder).unwrap();
        render_at(serve("tray.html", &fixture_data(), TRAY_DRIVER), "tray", Some((&folder, "tray")), (340, 600));
    }
    assert!(tray.contains("Today 14/19"));
}

// No duel up, after a stream: the popover says how it went.
const TRAY_OFF_AIR_DRIVER: &str = r#"
DATA.live = { ...DATA.live, prediction: null, queue: [], off_air: DATA.off_air, title: 'Last duel' };
(async () => {
  document.body.style.width = '340px'; // the popover's width
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  for (let i = 0; i < 60 && !window.augur; i++) await sleep(100);
  await sleep(200);
  document.getElementById('report').textContent = JSON.stringify({ tray: document.getElementById('pop').innerText, errors: window.__errors });
})();
"#;

#[test]
fn tray_popover_off_air_renders_headless() {
    if !have_chrome() {
        return;
    }
    let r = render(serve("tray.html", &fixture_data(), TRAY_OFF_AIR_DRIVER), "tray-off-air", None);
    assert_eq!(r["errors"], json!([]), "script errors: {}", r["errors"]);
    let tray = r["tray"].as_str().unwrap();
    let t = tray.to_lowercase();
    assert!(
        t.contains("off air") && t.contains("14/19") && t.contains("best run 8") && (t.contains("11 nov") || t.contains("nov 11")),
        "the last stream: {tray}"
    );
    assert!(tray.contains("over"), "the latest calls: {tray}");
    if let Some(folder) = std::env::var_os("AUGUR_UI_SHOTS").map(PathBuf::from) {
        std::fs::create_dir_all(&folder).unwrap();
        render_at(serve("tray.html", &fixture_data(), TRAY_OFF_AIR_DRIVER), "tray-off-air", Some((&folder, "tray-off-air")), (340, 540));
    }
}

// Between duels: the next one queued, with a long name, and another behind it.
const TRAY_BETWEEN_DRIVER: &str = r#"
DATA.live = { ...DATA.live, prediction: null, off_air: DATA.off_air, title: 'Last duel',
  queue: [{ pos: 1, home: 'player-28', away: 'a-very-long-kick-name-indeed-2026' }, { pos: 2, home: 'player-37', away: 'player-20' }] };
(async () => {
  document.body.style.width = '340px'; // the popover's width
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  for (let i = 0; i < 60 && !window.augur; i++) await sleep(100);
  await sleep(200);
  // the long name stays on one line, trimmed inside its half
  const names = [...document.querySelectorAll('.nu-s > span:last-child')];
  const oneLine = names.every((n) => n.getClientRects().length === 1 && n.scrollWidth >= n.clientWidth);
  const inside = names.every((n) => n.getBoundingClientRect().right <= document.getElementById('pop').getBoundingClientRect().right);
  document.getElementById('report').textContent = JSON.stringify({ tray: document.getElementById('pop').innerText, oneLine, inside, errors: window.__errors });
})();
"#;

#[test]
fn tray_popover_between_duels_shows_the_next_one() {
    if !have_chrome() {
        return;
    }
    let r = render(serve("tray.html", &fixture_data(), TRAY_BETWEEN_DRIVER), "tray-between", None);
    assert_eq!(r["errors"], json!([]), "script errors: {}", r["errors"]);
    let tray = r["tray"].as_str().unwrap();
    let t = tray.to_lowercase();
    assert!(t.contains("between duels") && t.contains("next up") && t.contains("player-28"), "the next duel: {tray}");
    assert!(t.contains("+1 queued") && !t.contains("player-37"), "only the next duel, and how many more: {tray}");
    assert!(t.contains("last stream"), "the stream card isn't called today when it wasn't: {tray}");
    assert_eq!((r["oneLine"].as_bool(), r["inside"].as_bool()), (Some(true), Some(true)), "long names stay on one line inside the card");
    if let Some(folder) = std::env::var_os("AUGUR_UI_SHOTS").map(PathBuf::from) {
        std::fs::create_dir_all(&folder).unwrap();
        render_at(serve("tray.html", &fixture_data(), TRAY_BETWEEN_DRIVER), "tray-between", Some((&folder, "tray-between")), (340, 600));
    }
}

// The live feed answered with something Augur can't read: the popover says so, rather than "can't reach".
const TRAY_UNREADABLE_DRIVER: &str = r#"
DATA.live = { ...DATA.live, prediction: null, queue: [], off_air: DATA.off_air, error: "couldn't read the live state: invalid type: null, expected a string" };
(async () => {
  document.body.style.width = '340px';
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  for (let i = 0; i < 60 && !window.augur; i++) await sleep(100);
  await sleep(200);
  document.getElementById('report').textContent = JSON.stringify({ tray: document.getElementById('pop').innerText, errors: window.__errors });
})();
"#;

#[test]
fn tray_popover_says_when_the_feed_cant_be_read() {
    if !have_chrome() {
        return;
    }
    let r = render(serve("tray.html", &fixture_data(), TRAY_UNREADABLE_DRIVER), "tray-unreadable", None);
    assert_eq!(r["errors"], json!([]), "script errors: {}", r["errors"]);
    let t = r["tray"].as_str().unwrap().to_lowercase();
    assert!(t.contains("can’t read palduels") && !t.contains("can’t reach"), "{t}");
}
