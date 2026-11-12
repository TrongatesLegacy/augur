//! Headless UI tests: the real pages in ui/, rendered by headless Chrome with a stand-in for the Tauri bridge that
//! serves what the app's commands would return (built here from the repo's fixtures, no network). Nothing appears on
//! screen and the user's own Chrome profile isn't touched. Skipped when Chrome isn't installed (set CHROME to point
//! at it). AUGUR_UI_SHOTS=<folder> also saves a screenshot of every tab there (AUGUR_UI_SIZE=1024x576 for another size).

use crate::settings::AppSettings;
use crate::views;
use augur_engine::seed::{page, root};
use augur_engine::stake::{self, StakeSettings};
use augur_engine::{bets, model, predict, seed, site};
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
    let s = model::Settings::default();
    let state = seed::state("state-leaflit-wumpo.json");
    let p = predict::predict(&state, &predict::Options { lookups: false, db_path: PathBuf::from("/nonexistent"), settings: s.clone() })
        .unwrap();
    let stake = stake::suggest(&p.pick, p.prob, &StakeSettings::default(), &bets::odds(&conn).unwrap(), bets::balance(&conn));
    let live = views::LiveView {
        title: state.title.clone(),
        picks_open: true,
        prediction: Some(p),
        stake: Some(stake),
        queue: state.queue.iter().filter(|q| q.pos > 0).take(3).cloned().collect(),
        synced_at: 1,
        today: (14, 19),
        first_run: false,
        ..Default::default()
    };
    let roster = Ok(site::parse_roster(&page("user-trongateslegacy.html")));
    // the sync times: two minutes ago, so they read like a real sync
    let recent = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64 - 120_000;
    let preview: Vec<_> = [0.55, 0.6, 0.65, 0.7, 0.75, 0.8, 0.85, 0.9]
        .map(|p| (p, stake::suggest("home", p, &StakeSettings::default(), &bets::odds(&conn).unwrap(), bets::balance(&conn))))
        .to_vec();
    json!({
        "live": live,
        "history": views::history_view(&conn, &s, "all", 0).unwrap(),
        "bets_view": views::bets_view(&conn, &StakeSettings::default(), "all", 0, Default::default()).unwrap(),
        "import_bets": null,
        "species_list": augur_engine::history::species_records(&conn).unwrap(),
        "species": augur_engine::history::species_detail(&conn, "Chillet Lux").unwrap(),
        "player": views::player_view(&conn, &s, "TrongatesLegacy", roster).unwrap(),
        "model_view": views::model_view(&conn, &s, None, Default::default(), 1).unwrap(),
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
        "account": { "signed_in": false, "name": null },
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
        "backfill_progress": augur_engine::backfill::Progress { phase: "duels".into(), done: 1200, total: 4702, ..Default::default() },
        "pal_image": null,
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
  if (!window.augur) window.__errors.push('the page never started (a module failed to load)');
  else {
    for (const tab of only ? [only] : ['live', 'history', 'bets', 'pals', 'players', 'model', 'settings', 'staking', 'sync', 'updates']) {
      // "updates" is Settings › Updates with a new version downloaded and ready (and its bar over the window)
      if (tab === 'updates') {
        DATA.update_view = DATA.update_ready;
        window.augur.showUpdate(DATA.update_ready);
      }
      // "bets" is History's Your bets view; "staking" and "sync" are Settings pages
      window.augur.state.view = tab === 'bets' ? 'bets' : 'calls';
      window.augur.state.settingsPage = ['staking', 'sync', 'updates'].includes(tab) ? tab : 'general';
      // "welcome" is the fresh install's overlay, over Live
      await window.augur.show(tab === 'bets' ? 'history' : ['staking', 'sync', 'updates'].includes(tab) ? 'settings' : tab === 'welcome' ? 'live' : tab);
      await sleep(300);
      out[tab] = document.getElementById('main').innerText;
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
    let dir = std::env::temp_dir().join(format!("augur-ui-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let url = format!("http://127.0.0.1:{port}/test.html");
    let base = |c: &mut Command| {
        c.arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--hide-scrollbars")
            .arg(format!("--user-data-dir={}", dir.join("profile").display()))
            .arg("--virtual-time-budget=10000");
    };
    let mut c = Command::new(&chrome);
    base(&mut c);
    let out = c.arg("--dump-dom").arg(&url).output().expect("run Chrome");
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
    assert!(text("model").contains("history test"), "Model: {}", text("model"));
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
        for tab in ["live", "history", "bets", "pals", "players", "model", "settings", "staking", "sync", "welcome", "updates"] {
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
