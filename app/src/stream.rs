//! The stream on Live, and reading it for the stats palduels.com hides until the bell. Kick's own embed player
//! (player.kick.com, as Stream Multiview uses) sits above the chat in a private webview: its own browser storage,
//! kept apart from the chat's and the sign-in's, so it watches logged out and never counts as your Kick account (Pear
//! Drops watches as you). While a fight with a hidden side is up, the panel's part of a frame is read every few
//! seconds (`augur_engine::stream`); a read the shown side agrees with goes into the call, and each fight's reads
//! (and misses, with the frame) go into its history.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use serde::Serialize;
use tauri::webview::WebviewBuilder;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl};

use crate::app::{notify, now_ms, Shared};
use crate::autobet::{eval, Spot};
use augur_engine::predict::{Prediction, Side};
use augur_engine::stream::{self, Check, Crop, StreamRead};
use augur_engine::{db, rusqlite};

/// The player webview inside the main window.
const STREAM: &str = "kickstream";
/// How often a frame is read while a hidden side's stats are wanted.
const READ_EVERY: Duration = Duration::from_secs(5);
/// Further behind live than this (seconds), twice running, and the player is loaded again, which starts it at live.
const LAG_MAX: f64 = 8.0;

/// What Live shows about the stream reader.
#[derive(Clone, Debug, Default, Serialize)]
pub struct StreamView {
    /// what it's doing: off, idle, watching, read
    pub status: String,
    pub message: String,
    /// the hidden side it's reading, and what it read
    pub side: Option<String>,
    pub stats: Option<[u32; 3]>,
    /// how far behind live the player is (seconds), at the last frame
    pub latency: Option<f64>,
    /// the fights with hidden stats in a row it couldn't read, and the warning once that's `MISSES_TO_WARN`
    pub misses: u32,
    pub warning: Option<String>,
}

/// The reader's state, in `Shared`.
#[derive(Default)]
pub struct StreamState {
    pub view: StreamView,
    /// the stats read for the fight up: its key (see `duel_key`), the hidden side, its HP/Attack/Defence, the call
    pub reading: Option<Reading>,
    /// where Live wants the player, when it's showing
    pub spot: Option<(LogicalPosition<f64>, LogicalSize<f64>)>,
    attempt: Option<Attempt>,
    lagging: u32,
}

#[derive(Clone, Debug)]
pub struct Reading {
    pub key: String,
    pub side: String,
    pub stats: [u32; 3],
    pub call_id: i64,
    /// compared with the site's stats at the bell
    pub compared: bool,
}

/// The fight a hidden side is being read for, and how the last try went.
#[derive(Clone, Debug)]
struct Attempt {
    key: String,
    call_id: i64,
    side: String,
    why: String,
    shot: Option<String>,
    latency: Option<f64>,
    last: Option<Instant>,
}

/// A fight's key: the owners and pals, as the live loop tells fights apart.
pub fn duel_key(p: &Prediction) -> String {
    format!("{}|{}|{}|{}", p.home.owner, p.away.owner, p.home.pal, p.away.pal)
}

/// The read for this fight, for the call: (side, stats).
pub fn reading_for(shared: &Shared, key: &str) -> Option<(String, [u32; 3])> {
    shared.stream.lock().unwrap().reading.as_ref().filter(|r| r.key == key).map(|r| (r.side.clone(), r.stats))
}

/// Live's spot for the player (above the chat), or none when Live isn't showing it.
pub fn place(app: &AppHandle, shared: &Shared, spot: Option<Spot>) {
    let zoom = *shared.zoom.lock().unwrap();
    let at = spot
        .filter(|s| s.visible && s.w > 40.0 && s.h > 40.0)
        .map(|s| (LogicalPosition::new(s.x * zoom, s.y * zoom), LogicalSize::new(s.w * zoom, s.h * zoom)));
    shared.stream.lock().unwrap().spot = at;
    if let Some(w) = app.get_webview(STREAM) {
        put(&w, at);
    }
}

/// Reads the stream while it's wanted, and keeps the player there while Live shows it. Runs for the app's life.
pub fn stream_loop(app: AppHandle, shared: Arc<Shared>) {
    while !shared.stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_secs(1));
        let s = shared.settings.lock().unwrap().auto_bet.clone();
        let live = shared.live.lock().unwrap().clone();
        let p = live.prediction.as_ref();
        let key = p.map(duel_key).unwrap_or_default();
        let hidden = p.filter(|_| live.picks_open).and_then(hidden_side);
        let shows = shared.stream.lock().unwrap().spot.is_some();
        let on = s.read_stream && !s.channel.is_empty();
        let Ok(conn) = shared.db() else { continue };

        // a fight the stream was being read for that's over, or whose picks closed, without a read
        let ended = {
            let st = shared.stream.lock().unwrap();
            st.attempt.clone().filter(|a| a.key != key || hidden.is_none())
        };
        if let Some(a) = ended {
            shared.stream.lock().unwrap().attempt = None;
            let read = shared.stream.lock().unwrap().reading.as_ref().is_some_and(|r| r.key == a.key);
            if !read {
                missed(&app, &shared, &conn, &a);
            }
        }
        // at the bell the site shows the side it hid: did the stream show the same?
        if let Some(p) = p {
            compare_at_bell(&shared, &conn, p, &key);
        }

        if !on || (hidden.is_none() && !shows) {
            if let Some(w) = app.get_webview(STREAM) {
                let _ = w.close();
            }
            let status = if on { "idle" } else { "off" };
            set_view(&app, &shared, |v| (v.status, v.message, v.side, v.stats) = (status.into(), String::new(), None, None));
            continue;
        }
        let w = match player(&app, &shared, &s.channel) {
            Ok(w) => w,
            Err(e) => {
                set_view(&app, &shared, |v| (v.status, v.message) = ("idle".into(), format!("The stream player couldn't open: {e}")));
                continue;
            }
        };
        let (Some(p), Some(side)) = (p, hidden) else {
            // the read stays up for the fight it was for
            let read = reading_for(&shared, &key).is_some();
            if !read {
                set_view(&app, &shared, |v| (v.status, v.message, v.side, v.stats) = ("idle".into(), String::new(), None, None));
            }
            continue;
        };
        if reading_for(&shared, &key).is_some() {
            continue;
        }
        // a new fight to read: note it, then a frame every few seconds
        let due = {
            let mut st = shared.stream.lock().unwrap();
            if st.attempt.as_ref().is_none_or(|a| a.key != key) {
                let Ok(Some(call_id)) = db::open_call(&conn, &p.home.owner, &p.away.owner) else { continue };
                st.attempt = Some(Attempt {
                    key: key.clone(),
                    call_id,
                    side: side.into(),
                    why: "the stream hadn't been read yet".into(),
                    shot: None,
                    latency: None,
                    last: None,
                });
            }
            let a = st.attempt.as_ref().unwrap();
            a.last.is_none_or(|l| l.elapsed() >= READ_EVERY)
        };
        if !due {
            continue;
        }
        set_view(&app, &shared, |v| {
            if v.status != "watching" {
                (v.status, v.message, v.stats) = ("watching".into(), "Watching the stream for the hidden stats".into(), None);
            }
            v.side = Some(side.into());
        });
        let frame = capture(&w);
        read_frame(&app, &shared, &conn, p, side, &key, frame);
    }
}

// --- reading a frame ---

/// What a frame gave: the panel's crop, the frame to keep, how far behind live it was.
struct Frame {
    crop: Crop,
    shot: Option<Vec<u8>>,
    latency: Option<f64>,
}

/// Grabs a frame from the player: the panel's part scaled to 1080p, and the whole frame (smaller) to keep.
fn capture(w: &tauri::Webview) -> Result<Frame, String> {
    let v = eval(w, capture_js(), Duration::from_secs(4)).ok_or("the player didn't answer")?;
    if v["ok"] != true {
        return Err(v["err"].as_str().unwrap_or("no picture yet").to_string());
    }
    let b64 = base64::engine::general_purpose::STANDARD;
    let rgba = b64.decode(v["panel"].as_str().unwrap_or("")).map_err(|e| e.to_string())?;
    let (_, _, pw, ph) = stream::PANEL;
    let shot = v["shot"].as_str().and_then(|s| s.split_once(',')).and_then(|(_, data)| b64.decode(data).ok());
    Ok(Frame { crop: Crop { w: pw as usize, h: ph as usize, rgba }, shot, latency: v["latency"].as_f64() })
}

fn read_frame(
    app: &AppHandle,
    shared: &Shared,
    conn: &rusqlite::Connection,
    p: &Prediction,
    side: &str,
    key: &str,
    frame: Result<Frame, String>,
) {
    let shown_key = if side == "home" { "away" } else { "home" };
    let shown = stats_of(if side == "home" { &p.away } else { &p.home });
    let (why, shot, latency) = match frame {
        Err(e) => (format!("no picture from the stream ({e})"), None, None),
        Ok(f) => {
            let check = stream::check(stream::read(&f.crop).as_ref(), (shown_key, shown));
            let why = match &check {
                Check::NoPanel => "the stats panel wasn't on the stream".to_string(),
                Check::ShownDiffers { read, .. } => {
                    format!("the {shown_key} side read differently from the site ({}), so not this fight's panel", words(read))
                }
                Check::Unreadable { read } => format!("the {side} side's numbers didn't all read ({})", words(read)),
                Check::Read { stats } => {
                    let at = p.seen_at.max(now_ms());
                    let call_id = shared.stream.lock().unwrap().attempt.as_ref().map(|a| a.call_id).unwrap_or(0);
                    let shot = f.shot.as_deref().and_then(|b| save_shot(shared, call_id, at, b));
                    let _ = stream::record(
                        conn,
                        call_id,
                        &StreamRead {
                            at,
                            outcome: stream::READ.into(),
                            side: side.into(),
                            stats: stream::stats_text(*stats),
                            detail: String::new(),
                            latency: f.latency,
                            shot,
                        },
                    );
                    let mut st = shared.stream.lock().unwrap();
                    st.reading = Some(Reading { key: key.into(), side: side.into(), stats: *stats, call_id, compared: false });
                    st.attempt = None;
                    st.lagging = 0;
                    drop(st);
                    set_view(app, shared, |v| {
                        v.status = "read".into();
                        v.message = format!("Read the {side} side's stats off the stream: {}", stream::stats_text(*stats));
                        (v.side, v.stats) = (Some(side.into()), Some(*stats));
                        v.latency = f.latency;
                        (v.misses, v.warning) = (0, None);
                    });
                    return;
                }
            };
            (why, f.shot, f.latency)
        }
    };
    // still trying: keep how it went (and the frame) for the record if picks close without a read
    let behind = latency.is_some_and(|l| l > LAG_MAX);
    let reload = {
        let mut st = shared.stream.lock().unwrap();
        st.lagging = if behind { st.lagging + 1 } else { 0 };
        if let Some(a) = st.attempt.as_mut() {
            a.why = why.clone();
            a.latency = latency;
            a.last = Some(Instant::now());
            if let Some(b) = shot {
                a.shot = Some(base64::engine::general_purpose::STANDARD.encode(b));
            }
        }
        st.lagging >= 2
    };
    if reload {
        // the player fell behind live: loading it again starts it at live
        if let Some(w) = app.get_webview(STREAM) {
            let channel = shared.settings.lock().unwrap().auto_bet.channel.clone();
            if let Ok(url) = tauri::Url::parse(&player_url(&channel)) {
                let _ = w.navigate(url);
            }
        }
        shared.stream.lock().unwrap().lagging = 0;
    }
    set_view(app, shared, |v| {
        v.message = format!("Watching the stream for the hidden stats: {why}");
        v.latency = latency;
    });
}

/// Picks closed (or the fight went) without a read: into the call's history with why and the last frame, and Live
/// warns once it keeps happening.
fn missed(app: &AppHandle, shared: &Shared, conn: &rusqlite::Connection, a: &Attempt) {
    let at = now_ms();
    let shot = a
        .shot
        .as_deref()
        .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
        .and_then(|b| save_shot(shared, a.call_id, at, &b));
    let _ = stream::record(
        conn,
        a.call_id,
        &StreamRead {
            at,
            outcome: stream::MISSED.into(),
            side: a.side.clone(),
            stats: String::new(),
            detail: a.why.clone(),
            latency: a.latency,
            shot,
        },
    );
    let n = stream::misses_in_a_row(conn).unwrap_or(0);
    let warn = n >= stream::MISSES_TO_WARN;
    let was = shared.stream.lock().unwrap().view.warning.is_some();
    set_view(app, shared, |v| {
        v.misses = n;
        v.warning = warn.then(|| format!("Couldn't read the stream for the last {n} fights with hidden stats: {}", a.why));
    });
    if warn && !was {
        notify(app, "Can't read the stream", &format!("{n} fights with hidden stats in a row: {}", a.why));
    }
}

/// The site's stats for the side it hid, once it shows them, against what the stream showed. Once a fight.
fn compare_at_bell(shared: &Shared, conn: &rusqlite::Connection, p: &Prediction, key: &str) {
    let r = {
        let st = shared.stream.lock().unwrap();
        st.reading.clone().filter(|r| r.key == key && !r.compared)
    };
    let Some(r) = r else { return };
    let side = if r.side == "home" { &p.home } else { &p.away };
    if side.stats_from != "shown" {
        return;
    }
    let site = stats_of(side);
    let agreed = site == r.stats;
    let _ = stream::record(
        conn,
        r.call_id,
        &StreamRead {
            at: now_ms(),
            outcome: if agreed { stream::AGREED } else { stream::DIFFERED }.into(),
            side: r.side.clone(),
            stats: stream::stats_text(site),
            detail: if agreed {
                String::new()
            } else {
                format!("the stream showed {}, the site {}", stream::stats_text(r.stats), stream::stats_text(site))
            },
            latency: None,
            shot: None,
        },
    );
    if let Some(x) = shared.stream.lock().unwrap().reading.as_mut() {
        x.compared = true;
    }
}

// --- the player ---

fn player_url(channel: &str) -> String {
    format!("https://player.kick.com/{}?autoplay=true&muted=true", channel.trim().to_lowercase())
}

/// The player, made if it isn't there: private (logged out), muted, at Live's spot or a speck in the corner while
/// Live isn't showing it (a hidden page's video can stop, and a frame is read at full size whatever it's shown at).
fn player(app: &AppHandle, shared: &Shared, channel: &str) -> Result<tauri::Webview, String> {
    let spot = shared.stream.lock().unwrap().spot;
    if let Some(w) = app.get_webview(STREAM) {
        put(&w, spot);
        return Ok(w);
    }
    let window = crate::app::main_window(app).ok_or("no main window")?;
    let url = tauri::Url::parse(&player_url(channel)).map_err(|e| e.to_string())?;
    let builder = WebviewBuilder::new(STREAM, WebviewUrl::External(url))
        // its own browser storage, gone when it closes: never signed in to Kick
        .incognito(true)
        .initialization_script(KEEP_PLAYING);
    let (pos, size) = spot.unwrap_or(SPECK);
    let w = window.add_child(builder, pos, size).map_err(|e| e.to_string())?;
    put(&w, spot);
    Ok(w)
}

/// Where the player waits while Live isn't showing it: two pixels in the window's corner.
const SPECK: (LogicalPosition<f64>, LogicalSize<f64>) = (LogicalPosition { x: 0.0, y: 0.0 }, LogicalSize { width: 2.0, height: 2.0 });

fn put(w: &tauri::Webview, spot: Option<(LogicalPosition<f64>, LogicalSize<f64>)>) {
    let (pos, size) = spot.unwrap_or(SPECK);
    let _ = w.set_position(pos);
    let _ = w.set_size(size);
    let _ = w.show();
}

/// Run in the player's page: keeps it playing, at its best quality (Kick's embed caps itself at 720p; its Amazon IVS
/// player, found from the <video> as Stream Multiview finds it, takes a higher cap), and keeps the player to hand.
const KEEP_PLAYING: &str = r#"(function () {
  var isPlayer = function (o) { return !!o && typeof o === 'object' && typeof o.setAutoMaxQuality === 'function' && typeof o.getQualities === 'function'; };
  var find = function () {
    for (var el = document.querySelector('video'), i = 0; el && i < 12; el = el.parentElement, i++) {
      var key = Object.keys(el).find(function (k) { return k.indexOf('__reactFiber$') === 0; });
      for (var f = key ? el[key] : null, d = 0; f && d < 60; f = f.return, d++) {
        for (var h = f.memoizedState; h && typeof h === 'object' && 'memoizedState' in h; h = h.next) {
          var m = h.memoizedState;
          if (isPlayer(m)) return m;
          if (m && isPlayer(m.current)) return m.current;
        }
        var props = f.memoizedProps || {};
        for (var k in props) if (isPlayer(props[k])) return props[k];
      }
    }
    return null;
  };
  setInterval(function () {
    try {
      var v = document.querySelector('video');
      if (v && v.paused) v.play().catch(function () {});
      var p = window.__augurPlayer && window.__augurPlayer.getQualities ? window.__augurPlayer : find();
      if (!p) return;
      window.__augurPlayer = p;
      var all = (p.getQualities() || []).slice().sort(function (a, b) { return b.height - a.height; });
      if (all.length && p.__augurCap !== all[0].name) {
        if (p.setAutoQualityMode && p.isAutoQualityMode && !p.isAutoQualityMode()) p.setAutoQualityMode(true);
        p.setAutoMaxQuality(all[0]);
        p.__augurCap = all[0].name;
      }
    } catch (e) {}
  }, 3000);
})();"#;

/// Run in the player's page: the panel's part of the frame showing (scaled to 1080p) as base64 RGBA, the whole frame
/// as a JPEG, and how far behind live the player is.
const CAPTURE: &str = r#"(function () {
  try {
    var v = document.querySelector('video');
    if (!v || !v.videoWidth) return { ok: false, err: location.pathname.indexOf('/offline') >= 0 ? 'the stream is offline' : 'the player has no picture yet' };
    if (v.paused) v.play().catch(function () {});
    var c = window.__augurCanvas || (window.__augurCanvas = document.createElement('canvas'));
    c.width = 1920; c.height = 1080;
    var g = c.getContext('2d', { willReadFrequently: true });
    g.drawImage(v, 0, 0, 1920, 1080);
    var d = g.getImageData(PANEL_X, PANEL_Y, PANEL_W, PANEL_H).data, s = '';
    for (var i = 0; i < d.length; i += 32768) s += String.fromCharCode.apply(null, d.subarray(i, i + 32768));
    var shot = document.createElement('canvas');
    shot.width = 960; shot.height = 540;
    shot.getContext('2d').drawImage(v, 0, 0, 960, 540);
    var p = window.__augurPlayer, latency = null;
    try { latency = p && p.getLiveLatency ? p.getLiveLatency() : null; } catch (e) {}
    return { ok: true, latency: latency, panel: btoa(s), shot: shot.toDataURL('image/jpeg', 0.8) };
  } catch (e) {
    return { ok: false, err: String(e) };
  }
})()"#;

fn capture_js() -> String {
    let (x, y, w, h) = stream::PANEL;
    CAPTURE
        .replace("PANEL_X", &x.to_string())
        .replace("PANEL_Y", &y.to_string())
        .replace("PANEL_W", &w.to_string())
        .replace("PANEL_H", &h.to_string())
}

// --- helpers ---

/// The side whose stats the site is hiding and the call is still guessing, if one is.
fn hidden_side(p: &Prediction) -> Option<&'static str> {
    let guessing = |s: &Side| s.estimated && s.stats_from != "shown";
    if guessing(&p.home) {
        Some("home")
    } else if guessing(&p.away) {
        Some("away")
    } else {
        None
    }
}

fn stats_of(s: &Side) -> [u32; 3] {
    let f = &s.fighter.fighter;
    [f.hp, f.atk, f.def].map(|v| v.round().max(0.0) as u32)
}

fn words(read: &[Option<u32>; 3]) -> String {
    read.map(|v| v.map_or("?".to_string(), |n| n.to_string())).join("/")
}

/// Keeps a frame in the data folder's stream folder; returns its file name.
fn save_shot(shared: &Shared, call_id: i64, at: u64, jpeg: &[u8]) -> Option<String> {
    let dir = shared.dir.join("stream");
    std::fs::create_dir_all(&dir).ok()?;
    let name = format!("{call_id}-{at}.jpg");
    std::fs::write(dir.join(&name), jpeg).ok()?;
    Some(name)
}

fn set_view(app: &AppHandle, shared: &Shared, f: impl FnOnce(&mut StreamView)) {
    let view = {
        let mut st = shared.stream.lock().unwrap();
        let before = st.view.clone();
        f(&mut st.view);
        if format!("{before:?}") == format!("{:?}", st.view) {
            return;
        }
        st.view.clone()
    };
    shared.auto.lock().unwrap().stream = view;
    let _ = app.emit("autobet", &*shared.auto.lock().unwrap());
}

/// A kept frame, for the call's history: as a data URL. Only names in the stream folder.
#[tauri::command]
pub fn stream_shot(shared: tauri::State<'_, Arc<Shared>>, name: String) -> Result<String, String> {
    if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
        return Err("not a frame".into());
    }
    let bytes = std::fs::read(shared.dir.join("stream").join(&name)).map_err(|e| e.to_string())?;
    Ok(format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}
