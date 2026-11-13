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
    let mut p = predict::predict(&state, &predict::Options { lookups: false, db_path: PathBuf::from("/nonexistent"), settings: s.clone() })
        .unwrap();
    // the head-to-heads the lookups would bring: the owners have met five times (a long pal name among them), the
    // pals three times
    let row = |when: &str, home: (&str, &str), away: (&str, &str), home_won: bool| site::DuelRow {
        id: None,
        when: when.into(),
        sides: [
            site::RowSide { side: "home".into(), won: home_won, owner: home.0.into(), pal: home.1.into() },
            site::RowSide { side: "away".into(), won: !home_won, owner: away.0.into(), pal: away.1.into() },
        ],
    };
    p.owners = Some(site::Rivalry {
        summary: "player-51 leads player-12 3–2. 5 meetings, the first on Mon, Aug 17.".into(),
        since: Some("2026-08-17".into()),
        owners: vec![("player-12".into(), 2, "41–37 in every duel".into()), ("player-51".into(), 3, "58–30 in every duel".into())],
        meetings: vec![
            row("16d ago", ("player-51", "Splatterina"), ("player-12", "Woolipop Ryu"), true),
            row("23d ago", ("player-12", "Finsider Ignis"), ("player-51", "Chillet Noct"), true),
            row("Sep 30", ("player-12", "Jormuntide Ignis Terra"), ("player-51", "Wumpo Botan"), false),
            row("Sep 2", ("player-51", "Lamball"), ("player-12", "Depresso"), true),
            row("Aug 17", ("player-12", "Chikipi"), ("player-51", "Cattiva"), true),
        ],
    });
    let pal_row = |home_won| row("", ("player-30", "Leaflit Slimey"), ("player-44", "Wumpo"), home_won);
    p.pal_h2h = predict::PalHeadToHead {
        home_wins: 1,
        away_wins: 2,
        meetings: vec![pal_row(true), pal_row(false), pal_row(false)],
        pages_scanned: 1,
    };
    let stake = stake::suggest(&p.pick, p.prob, &StakeSettings::default(), &bets::odds(&conn).unwrap());
    let live = views::LiveView {
        title: state.title.clone(),
        picks_open: true,
        detail: Some(views::live_detail(&p, Some(&stake))),
        prediction: Some(p),
        stake: Some(stake),
        queue: state.upcoming().into_iter().take(3).collect(),
        synced_at: recent,
        today: (14, 19),
        first_run: false,
        ..Default::default()
    };
    let roster = Ok(site::parse_roster(&page("user-trongateslegacy.html")));
    let preview: Vec<_> = [0.55, 0.6, 0.65, 0.7, 0.75, 0.8, 0.85, 0.9]
        .map(|p| (p, stake::suggest("home", p, &StakeSettings::default(), &bets::odds(&conn).unwrap())))
        .to_vec();
    // the newest call was re-called after picks closed and came out the other way
    let mut history = views::history_view(&conn, &s, "all", 0).unwrap();
    if let Some(c) = history.calls.first_mut() {
        c.late = Some((if c.pick == "home" { "away" } else { "home" }.into(), 0.575));
    }
    // a call's breakdown: the newest call (the first row) with a made-up fight tape, re-called after the bell like
    // its row, its home side's stats taken from the owner's box, and your bet on it (a store of its own, so the counts
    // above hold)
    let call_detail = {
        let conn = seed::store();
        seed::import_bets(&conn);
        let newest = augur_engine::db::settled(&conn).unwrap().pop().unwrap();
        seed::tape(&conn, newest.duel_id.unwrap());
        conn.execute(
            "UPDATE predictions SET stats_from = 'home: from their box; away: shown', app_version = '0.1.5' WHERE id = ?",
            [newest.id],
        )
        .unwrap();
        let mut d = views::call_detail(&conn, newest.id, &StakeSettings::default()).unwrap().unwrap();
        d.call.late = history.calls.first().and_then(|c| c.late.clone());
        d
    };
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
        let mut mv = views::model_view(&conn, &s, true, None, Default::default(), recent).unwrap();
        // made-up eras: a new version, then the self-tuning keeping a change
        let era = |version: &str, changes: &[&str], since: i64, right, total| augur_engine::history::Era {
            version: version.into(),
            changes: changes.iter().map(|c| c.to_string()).collect(),
            since,
            right,
            total,
        };
        mv.eras = vec![
            era("0.1.5", &["TYPE_BONUS 1.4 → 1.5"], recent as i64 - 3_600_000, 4, 5),
            era("0.1.5", &["version 0.1.4 → 0.1.5"], recent as i64 - 20_000_000, 9, 13),
            era("0.1.4", &[], recent as i64 - 90_000_000, 13, 18),
        ];
        mv
    };
    json!({
        "live": live,
        "history": history,
        "call_detail": call_detail,
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
            // updated from two versions back: the card lists both versions from the changelog built in
            updated_from: Some("0.0.9".into()),
            changelog: "# What's new in Augur\n\nAn intro line.\n\n## 0.1.1 (current)\n- Calls show their K.O. time\n\n## 0.1.0\n- A tray that says who's up next\n- A line that wraps\n  onto the next one\n\n## 0.0.9\n- The first version\n",
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
  // the report's window loses some height to Chrome's toolbar; the app is laid out at the screenshots' full height
  // (the app zooms so it always has at least 720 to work with), so the checks see what the window shows
  const vh = new URLSearchParams(location.search).get('vh');
  if (vh) document.querySelector('.shell')?.style.setProperty('height', `${vh}px`);
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
    for (const tab of only ? [only] : ['live', 'liveedge', 'history', 'call', 'bets', 'pals', 'players', 'you', 'titles', 'titlesone', 'model', 'settings', 'staking', 'ownmodel', 'sync', 'changelog', 'updates', 'offair']) {
      // "ownmodel" is Settings › Model with self-tuning off and your own values
      const own = DATA.get_settings;
      if (tab === 'ownmodel') DATA.get_settings = { ...own, model_auto: false };
      // "changelog" is Settings › Updates when up to date: the changelog built in
      if (tab === 'changelog') DATA.update_view = { ...DATA.update_ready, latest: null, ready: false, updated_from: null };
      // "updates" is Settings › Updates with a new version downloaded and ready (and its bar over the window)
      if (tab === 'updates') {
        DATA.update_view = DATA.update_ready;
        window.augur.showUpdate(DATA.update_ready);
      }
      // "bets" is History's Your bets view; "staking", "sync" and "updates" are Settings pages
      window.augur.state.view = tab === 'bets' ? 'bets' : 'calls';
      // "titles" is You › Titles, every title by category
      // "titlesone" is the same with one category's tab picked
      window.augur.state.youView = ['titles', 'titlesone'].includes(tab) ? 'titles' : 'profile';
      // "call" is History with the first call's breakdown opened from its row
      window.augur.state.call = null;
      // "offair" is Live with no duel up, after a stream
      if (tab === 'offair') window.augur.state.live = { ...DATA.live, prediction: null, queue: [], off_air: DATA.off_air, title: 'Last duel' };
      // "liveedge" is Live at its edges: a race too close to call (the favourite left with about 1.5% HP), owners
      // meeting for the first time and pals that never have
      else if (tab === 'liveedge') {
        const p = DATA.live.prediction;
        const t = p.pick === 'home' ? { t_home: p.score.t_away * 0.985 } : { t_away: p.score.t_home * 0.985 };
        window.augur.state.live = { ...DATA.live, prediction: { ...p, score: { ...p.score, ...t }, owners: { summary: 'first meeting', since: null, owners: [], meetings: [] }, pal_h2h: { home_wins: 0, away_wins: 0, meetings: [], pages_scanned: 1 } } };
      } else window.augur.state.live = DATA.live;
      window.augur.state.settingsPage = ['staking', 'sync', 'updates'].includes(tab) ? tab : tab === 'ownmodel' ? 'model' : tab === 'changelog' ? 'updates' : 'general';
      // "welcome" is the fresh install's overlay, over Live
      await window.augur.show(['bets', 'call'].includes(tab) ? 'history' : ['staking', 'sync', 'changelog', 'updates', 'ownmodel'].includes(tab) ? 'settings' : ['welcome', 'offair', 'liveedge'].includes(tab) ? 'live' : ['titles', 'titlesone'].includes(tab) ? 'you' : tab);
      await sleep(300);
      // the page and every box in it that scrolls (its panel's title, and how much it holds against what shows)
      const scrolling = () =>
        [...document.querySelectorAll('#main, #main *')]
          .filter((el) => /auto|scroll/.test(getComputedStyle(el).overflowY) && el.scrollHeight > el.clientHeight + 2)
          .map((el) => `${el.closest('.panel')?.querySelector('h3')?.textContent || el.className}: ${el.clientHeight} of ${el.scrollHeight}px`);
      out.scrollers ||= {};
      out.scrollers[tab] = scrolling();
      if (tab === 'bets') out.betRows = document.querySelectorAll('#main tr[data-call]').length;
      // Updates' panels don't run into each other, and their text stays inside them (the changelog scrolls in its own)
      if (tab === 'updates' || tab === 'changelog') {
        const ps = [...document.querySelectorAll('#main .stackc > .panel')].map((p) => [p, p.getBoundingClientRect()]);
        out[tab + 'Apart'] = ps.every(([p, b], i) =>
          (i === 0 || b.top >= ps[i - 1][1].bottom) &&
          [...p.querySelectorAll('*')].every((el) => {
            if (el.parentElement.closest('.fillin')) return true;
            const r = el.getBoundingClientRect();
            return !r.height || r.bottom <= b.bottom + 1;
          }));
      }
      if (tab === 'live' || tab === 'liveedge') {
        // the head to head and the HP race keep everything inside their panels (a list scrolls inside its own box),
        // and the race's end labels apart
        const inside = (panel) => !!panel && [...panel.querySelectorAll('*')].every((el) => {
          if (el.parentElement.closest('.fillin')) return true;
          const [r, b] = [el.getBoundingClientRect(), panel.getBoundingClientRect()];
          return !r.height || (r.bottom <= b.bottom + 1 && r.right <= b.right + 1);
        });
        const ends = [...document.querySelectorAll('.hr .re')].map((e) => e.getBoundingClientRect());
        out[tab + 'Panels'] = {
          h2h: inside(document.querySelector('.hh')),
          race: inside(document.querySelector('.hr')),
          ends: ends.length,
          apart: ends.length === 2 && (ends[0].bottom <= ends[1].top || ends[1].bottom <= ends[0].top),
          meetings: document.querySelectorAll('.hh .hhr').length,
          won: document.querySelectorAll('.hh .hhr .hp.w').length,
        };
      }
      if (tab === 'call') {
        document.querySelector('#main tr[data-call]')?.click();
        await sleep(400);
        out.chart = { lines: document.querySelectorAll('.hpc path.line').length, proj: document.querySelectorAll('.hpc path.proj').length, hits: document.querySelectorAll('.hpc circle').length, ko: !!document.querySelector('.hpc .ko') };
        out.fits = document.getElementById('main').scrollHeight <= document.getElementById('main').clientHeight + 1;
        out.scrollers[tab] = scrolling();
      }
      const visible = () => [...document.querySelectorAll('.tgrp')].filter((g) => !g.hidden).map((g) => g.querySelector('h4').textContent);
      if (tab === 'titlesone') {
        document.querySelector('.tcat[data-tg="1"]')?.click();
        await sleep(100);
        out.titlesShown = visible();
      }
      if (tab === 'titles') {
        out.titlesAll = document.querySelectorAll('.tmap:not([hidden]) .mrow').length;
        // every category's cards, a tab at a time (All is the map), then back to All for the screenshot
        const cats = [];
        for (const b of document.querySelectorAll('.tcat[data-tg]:not([data-tg="all"])')) {
          b.click();
          await sleep(30);
          cats.push(document.querySelector('.tall').closest('.panel').innerText);
        }
        out.titlesCats = cats.join('\n');
        document.querySelector('.tcat[data-tg="all"]').click();
        await sleep(30);
      }
      out[tab] = document.getElementById('main').innerText;
      out.spills[tab] = spills();
      if (tab === 'live' || tab === 'liveedge') {
        // the page fits the window: its panels scroll inside, the page doesn't
        out[tab + 'Fits'] = document.getElementById('main').scrollHeight <= document.getElementById('main').clientHeight + 1;
      }
      if ((tab === 'live' || tab === 'liveedge') && !only) {
        // Windows draws text a few pixels wider than a Mac: Live keeps fitting with every letter 0.4px wider (not
        // in the screenshots)
        const els = [...document.querySelectorAll('#main *')];
        const was = els.map((el) => getComputedStyle(el).letterSpacing);
        els.forEach((el, i) => (el.style.letterSpacing = `calc(${was[i] === 'normal' ? '0px' : was[i]} + 0.4px)`));
        await sleep(50);
        out.spills[tab + 'Wide'] = spills();
      }
      if (tab === 'call' && !only) {
        // an old call: no tape, no fighters, no factors kept
        const full = DATA.call_detail;
        DATA.call_detail = { ...full, call: { ...full.call, fighters: null }, sides: [], model: null, settings: null, fight: { ...full.fight, hits: [], hp: [[[0, 100]], [[0, 100]]], moves: [[], []], biggest: [] } };
        window.augur.state.call = null;
        await window.augur.show('history');
        await sleep(200);
        document.querySelector('#main tr[data-call]')?.click();
        await sleep(300);
        out.callBare = document.getElementById('main').innerText;
        out.spills.callBare = spills();
        DATA.call_detail = full;
        // Back: the list again, in the same range and view
        document.getElementById('back')?.click();
        await sleep(300);
        out.callBack = document.getElementById('main').innerText;
        out.callBackRange = document.querySelector('.seg button[data-r].on')?.dataset.r;
      }
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
    let out = c.arg(format!("--window-size={w},{h}")).arg("--dump-dom").arg(format!("{url}?vh={h}")).output().expect("run Chrome");
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
        for tab in [
            "live",
            "liveedge",
            "history",
            "call",
            "bets",
            "pals",
            "players",
            "you",
            "titles",
            "titlesone",
            "model",
            "settings",
            "staking",
            "ownmodel",
            "sync",
            "changelog",
            "updates",
            "offair",
        ] {
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
    assert!(text("history").contains("after the bell:"), "History shows a call that changed after picks closed: {}", text("history"));
    let p = &data["live"]["prediction"];
    let chance = format!("{}%", (p["prob"].as_f64().unwrap() * 100.0).round());
    assert!(text("live").contains(&chance), "Live shows the chance {chance}: {}", text("live"));
    assert!(text("live").contains(&p["lines"][0].as_str().unwrap().to_lowercase()), "Live shows the Yoda line");
    assert!(
        text("live").contains("stats are hidden until the bell: guessed"),
        "Live says the home stats were hidden and guessed: {}",
        text("live")
    );
    // the breakdown's pieces before the bell: the pick and its bet, how Augur is calling it (told in the present, no
    // result yet), the tale of the tape, and Live's own panels
    assert!(text("live").contains("augur calls\nwumpo") && text("live").contains("augur's pick"), "Live's headline: {}", text("live"));
    assert!(
        text("live").contains("how augur called it")
            && text("live").contains("counts most for wumpo")
            && text("live").contains("together")
            && text("live").contains("move power"),
        "how Augur is calling it: {}",
        text("live")
    );
    assert!(!text("live").contains("won anyway") && !text("live").contains("counted most"), "nothing's been won yet: {}", text("live"));
    // the model's settings are in a duel's breakdown and on Model; Live keeps its room for the fight
    assert!(
        !text("live").contains("the model it’s using") && text("call").contains("model used"),
        "Live leaves the settings to the breakdown"
    );
    assert!(
        ["tale of the tape", "rolls", "projected hp race", "head to head", "chat line", "up next"].iter().all(|x| text("live").contains(x)),
        "Live's panels: {}",
        text("live")
    );
    assert!(!text("live").contains("what decides it"), "the breakdown's sum stands in for What decides it: {}", text("live"));
    // the head to head: the owners' record and meetings (home owner's pal first, the winner marked), the pals' own
    assert!(
        (text("live").contains("5 meetings since 17 aug") || text("live").contains("5 meetings since aug 17"))
            && text("live").contains("player-12")
            && text("live").contains("2–3"),
        "Live shows the owners' record: {}",
        text("live")
    );
    assert!(text("live").contains("woolipop ryu\nvs\n♛splatterina"), "each meeting from the home owner's side: {}", text("live"));
    assert!(text("live").contains("pals") && text("live").contains("1–2"), "and the pals' record: {}", text("live"));
    assert!(!text("live").contains(" ."), "no stray space before a full stop: {}", text("live"));
    assert_eq!(r["livePanels"]["meetings"], json!(5), "every meeting is listed (the list scrolls)");
    assert_eq!(r["livePanels"]["won"], json!(5), "with its winner marked");
    // the race: both pals named with what's left, the K.O. marked, and a close race still apart
    assert!(text("live").contains("21%\nhp left") && text("live").contains("k.o.\nout first"), "the race's ends: {}", text("live"));
    assert!(text("live").contains("would need 27% longer"), "and the margin: {}", text("live"));
    assert!(
        text("liveedge").contains("neck and neck") && text("liveedge").contains("2%\nhp left"),
        "a race too close to call: {}",
        text("liveedge")
    );
    assert!(
        text("liveedge").contains("first meeting") && text("liveedge").contains("haven’t met") && text("liveedge").contains("never met"),
        "owners and pals who've never met: {}",
        text("liveedge")
    );
    assert_eq!((&r["liveFits"], &r["liveedgeFits"]), (&json!(true), &json!(true)), "Live fits the window without scrolling");
    for tab in ["livePanels", "liveedgePanels"] {
        let f = &r[tab];
        assert_eq!((&f["h2h"], &f["race"]), (&json!(true), &json!(true)), "{tab}: nothing spills out of the panels: {f}");
        assert_eq!((&f["ends"], &f["apart"]), (&json!(2), &json!(true)), "{tab}: the race's end labels never overlap: {f}");
    }
    assert!(text("history").contains("14 / 19"), "History: {}", text("history"));
    // a call's breakdown, opened from its row
    let d = &data["call_detail"];
    let call = text("call");
    let (home, away) = (d["call"]["home_pal"].as_str().unwrap().to_lowercase(), d["call"]["away_pal"].as_str().unwrap().to_lowercase());
    let ko = d["fight"]["ko"].as_str().unwrap();
    assert!(call.contains(&home) && call.contains(&away) && call.contains("augur called"), "the breakdown names both pals: {call}");
    assert!(call.contains(&format!("k.o. {ko}")) && call.contains("% hp left"), "the result, K.O. and HP left: {call}");
    assert!(call.contains("how it went") && call.contains("damage by move") && call.contains("top hits"), "how the duel went: {call}");
    assert!(call.contains("counted most for"), "the breakdown says what swung the call most: {call}");
    assert!(
        call.contains("how augur called it") && call.contains("records") && call.contains("pal h2h") && call.contains("together"),
        "the model's sum: {call}"
    );
    assert!(
        call.contains("tale of the tape") && call.contains("augur 0.1.5") && call.contains("type edge"),
        "the tape and the settings: {call}"
    );
    assert!(call.contains("after the bell") && call.contains("taken from its owner’s box"), "how the call was made: {call}");
    let bet = &d["bet"];
    let (cmd, label) = (bet["augur_command"].as_str().unwrap().to_lowercase(), bet["label"].as_str().unwrap());
    assert!(bet["backed"].is_string(), "the seed has a bet on its newest call: {bet}");
    let side = if bet["backed"] == "home" { "!h" } else { "!a" };
    let yours = format!("you\n{side} {}", bet["stake"].as_u64().unwrap());
    assert!(
        call.contains(&format!("augur\n{cmd}")) && call.contains(&yours) && call.contains(label),
        "the money on it, Augur's and yours: {call}"
    );
    assert_eq!(r["chart"]["lines"], json!(2), "an HP line per side");
    assert_eq!(r["chart"]["hits"].as_u64().unwrap() as usize, d["fight"]["hits"].as_array().unwrap().len(), "a dot per hit");
    assert_eq!(r["chart"]["ko"], json!(true), "the K.O. is marked");
    assert_eq!(r["chart"]["proj"], json!(2), "with Augur's projected race behind it");
    assert_eq!(r["fits"], json!(true), "the breakdown fits the window without scrolling");
    let bare = r["callBare"].as_str().unwrap().to_lowercase();
    assert!(
        bare.contains("no fight tape")
            && bare.contains("no fighters saved")
            && bare.contains("no numbers kept")
            && bare.contains("not saved with this call"),
        "an old call without a tape or fighters still opens: {bare}"
    );
    assert!(r["callBack"].as_str().unwrap().to_lowercase().contains("calibration"), "Back returns to the calls");
    assert_eq!(r["callBackRange"], json!("today"), "in the range it was in");
    // everything fits at 1280x720 without a box scrolling inside it, even in the report's shorter window: only the
    // lists that grow without end (History's calls and your bets) scroll
    for (tab, boxes) in r["scrollers"].as_object().unwrap() {
        let allowed = ["history", "bets"].contains(&tab.as_str());
        assert!(allowed || boxes.as_array().unwrap().is_empty(), "{tab}: boxes scroll inside it: {boxes}");
    }
    assert!(r["betRows"].as_u64().unwrap() > 0, "Your bets rows with a call open its breakdown too");
    for tab in ["updates", "changelog"] {
        assert_eq!(r[format!("{tab}Apart")], true, "{tab}: the Updates panels overlap or spill");
    }
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
    assert!(text("you").contains("all titles ›"), "You links to its Titles view: {}", text("you"));
    assert_eq!(r["titlesAll"], json!(7), "All shows every category");
    assert_eq!(r["titlesShown"], json!(["Predictions"]), "a category tab shows just that category");
    assert!(
        text("titles").contains("16 titles earned")
            && text("titles").contains("36 still to unlock")
            && text("titles").contains("closest to unlock")
            && text("titles").contains("every title")
            && text("titles").contains("in the pit")
            && text("titles").contains("tournaments"),
        "Titles: {}",
        text("titles")
    );
    // each category's tab: its blurb, a ladder's steps, a crown's holder, the element ring and a locked title's progress
    let cats = r["titlesCats"].as_str().unwrap_or("").to_lowercase();
    assert!(
        cats.contains("send your pal in and win fights.")
            && cats.contains("pit regular")
            && cats.contains("held by player-44")
            && cats.contains("nobody holds it yet")
            && cats.contains("dragon")
            && cats.contains("312/400")
            && cats.contains("sparring bag"),
        "Titles, category by category: {cats}"
    );
    assert!(!text("titles").contains("your balance"), "Titles is its own view: {}", text("titles"));
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
    assert!(
        !stake.is_empty() && text("live").contains(&format!("{stake}\n")) && text("live").contains(" credits\ncopy\n"),
        "Live offers the bet {stake} with a copy button: {}",
        text("live")
    );
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
        text("updates").contains("updated to 0.1.1")
            && text("updates").contains("calls show their k.o. time")
            && text("updates").contains("a line that wraps onto the next one")
            && text("updates").contains("this version"),
        "after an update, what changed comes from the changelog built in: {}",
        text("updates")
    );
    assert!(
        text("changelog").contains("changelog")
            && text("changelog").contains("the first version")
            && text("changelog").contains("this version"),
        "up to date, Updates shows the changelog built in: {}",
        text("changelog")
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
            "live",
            "liveedge",
            "history",
            "call",
            "bets",
            "pals",
            "players",
            "you",
            "titles",
            "titlesone",
            "model",
            "settings",
            "staking",
            "ownmodel",
            "sync",
            "welcome",
            "changelog",
            "updates",
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
