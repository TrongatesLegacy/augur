//! Auto betting in the app: signing in to Kick, the stream's chat on Live (Kick's popout chat in a webview inside the
//! main window, shown once you're signed in), and the loop that bets on a call by itself by typing the command into
//! that chat, the way you would. The rules that make it safe are in `augur_engine::autobet`: a fight is claimed before
//! anything is typed, nothing is ever retried, and it stands down when you've bet yourself.

use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::webview::{PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::app::{notify, now_ms, Shared};
use augur_engine::autobet::{self, AutoBet, AutoBetSettings, Decision, Moment};
use augur_engine::kick::{self, Line};
use augur_engine::{db, rusqlite};

/// The chat webview inside the main window, and the sign-in window.
const CHAT: &str = "kickchat";
const SIGNIN: &str = "kicksignin";
const KICK: &str = "https://kick.com";
/// How often the chat is read while a fight is up and auto betting is armed (one small GET).
const READ_EVERY: Duration = Duration::from_secs(4);
/// How long to look for the bet in chat, and the bot's answer, after typing it.
const CONFIRM_FOR: Duration = Duration::from_secs(24);
/// Another computer heard from this recently with auto betting on counts as betting.
const OTHER_FRESH_MS: u64 = 10 * 60_000;

/// What Live, Settings and the tray show about auto betting.
#[derive(Clone, Debug, Default, Serialize)]
pub struct AutoView {
    pub enabled: bool,
    pub armed: bool,
    pub signed_in: bool,
    /// your Kick name, as signed in (or Settings' Kick name until it's known)
    pub user: Option<String>,
    /// the chat bets go into: the stream's, or yours in test mode
    pub channel: String,
    pub test: bool,
    /// another computer with auto betting on, if one is
    pub other: Option<String>,
    /// "off", "idle", "waiting", "countdown", "sending", "done"
    pub phase: String,
    pub message: String,
    pub command: String,
    /// the countdown ends at (ms), with Cancel until then
    pub countdown_until: Option<u64>,
    /// the auto bet on the call that's up, once there is one
    pub bet: Option<AutoBet>,
    /// today's real auto bets: how many, and their credits
    pub today: (u32, u64),
    /// how long picks usually stay open (seconds) and over how many calls
    pub window: Option<(u64, usize)>,
    /// armed but something's wrong (signed out, the chat isn't ready, the chat can't be read...): what, and whether
    /// it stops auto betting until it's fixed. The tray, the popover and Live all say so.
    pub problem: Option<String>,
    pub blocking: bool,
    /// the channel is streaming, as Kick said at the last check (None before the first)
    pub stream_live: Option<bool>,
}

/// Kick, as this computer is signed in to it.
#[derive(Clone, Debug, Default)]
pub struct KickLogin {
    pub signed_in: bool,
    pub user: Option<String>,
    pub checked: Option<Instant>,
}

/// Where Live wants the chat: its spot on the page (CSS pixels) and whether it shows.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Spot {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub visible: bool,
}

type Cmd<T> = Result<T, String>;

/// The channel bets go into: yours in test mode, the stream's otherwise.
pub fn channel(s: &AutoBetSettings, user: Option<&str>, kick_name: &str) -> String {
    if s.test_mode {
        user.filter(|u| !u.is_empty()).unwrap_or(kick_name).trim().to_lowercase()
    } else {
        s.channel.trim().to_lowercase()
    }
}

// --- signing in to Kick ---

fn has_session(cookies: &[tauri::webview::Cookie<'static>]) -> bool {
    cookies.iter().any(|c| c.name() == "session_token" && !c.value().is_empty())
}

/// Reads Kick's cookies from any of Augur's webviews (they share one store). Never on the UI thread: that deadlocks
/// WebView2.
fn kick_cookies(app: &AppHandle) -> Vec<tauri::webview::Cookie<'static>> {
    let url = tauri::Url::parse(KICK).unwrap();
    app.webviews().values().next().and_then(|w| w.cookies_for_url(url).ok()).unwrap_or_default()
}

/// Whether this computer is signed in to Kick, checked at most every 30 seconds.
pub fn signed_in(app: &AppHandle, shared: &Shared) -> bool {
    let fresh = shared.kick.lock().unwrap().checked.is_some_and(|t| t.elapsed() < Duration::from_secs(30));
    if !fresh {
        let on = has_session(&kick_cookies(app));
        let mut k = shared.kick.lock().unwrap();
        if k.signed_in && !on {
            notify(app, "Signed out of Kick", "Sign in again in Settings › Auto bet to keep auto betting");
        }
        k.signed_in = on;
        k.checked = Some(Instant::now());
    }
    shared.kick.lock().unwrap().signed_in
}

/// Runs a script in a webview and waits for what it returns (as JSON), up to `wait`.
fn eval(w: &tauri::Webview, js: String, wait: Duration) -> Option<serde_json::Value> {
    let (tx, rx) = mpsc::channel();
    w.eval_with_callback(js, move |out| {
        let _ = tx.send(out);
    })
    .ok()?;
    rx.recv_timeout(wait).ok().and_then(|s| serde_json::from_str(&s).ok())
}

/// Your Kick name, asked from inside a kick.com page with its own sign-in (so Kick sees its own site asking).
fn who_am_i(w: &tauri::Webview, cookies: &[tauri::webview::Cookie<'static>]) -> Option<String> {
    let get = |n: &str| cookies.iter().find(|c| c.name() == n).map(|c| c.value().to_string()).unwrap_or_default();
    let (token, xsrf) = (serde_json::to_string(&get("session_token")).ok()?, serde_json::to_string(&get("XSRF-TOKEN")).ok()?);
    let start = format!(
        r#"(function(t, x) {{
  if (window.__augurMe === undefined) {{
    window.__augurMe = null;
    var h = {{ Accept: 'application/json' }};
    if (t) h.Authorization = 'Bearer ' + decodeURIComponent(t);
    if (x) h['X-XSRF-TOKEN'] = decodeURIComponent(x);
    fetch('/api/v1/user', {{ credentials: 'include', headers: h }})
      .then(function (r) {{ return r.ok ? r.json() : null; }})
      .then(function (j) {{ window.__augurMe = (j && (j.username || j.slug)) || ''; }})
      .catch(function () {{ window.__augurMe = ''; }});
  }}
  return window.__augurMe;
}})({token}, {xsrf})"#
    );
    for _ in 0..12 {
        match eval(w, start.clone(), Duration::from_secs(2)) {
            Some(serde_json::Value::String(name)) if !name.is_empty() => return Some(name),
            Some(serde_json::Value::String(_)) => return None,
            _ => std::thread::sleep(Duration::from_millis(500)),
        }
    }
    None
}

/// Settings › Auto bet › Sign in to Kick: kick.com in its own window. Once Kick's sign-in cookie is there, Augur
/// notes your name and closes the window. Augur never sees your password and doesn't keep the cookie: the chat on Live
/// uses the same signed-in browser storage.
// async: on Windows, building a window inside a synchronous command deadlocks WebView2
#[tauri::command]
pub async fn kick_sign_in(app: AppHandle) -> Cmd<()> {
    if let Some(w) = app.get_webview_window(SIGNIN) {
        let _ = w.set_focus();
        return Ok(());
    }
    let url = tauri::Url::parse(KICK).map_err(|e| e.to_string())?;
    let w = WebviewWindowBuilder::new(&app, SIGNIN, WebviewUrl::External(url))
        .title("Sign in to Kick")
        .inner_size(560.0, 760.0)
        .build()
        .map_err(|e| e.to_string())?;
    let handle = app.clone();
    // Kick signs in without loading a new page, so watch for its cookie rather than a page load
    std::thread::spawn(move || {
        let end = Instant::now() + Duration::from_secs(20 * 60);
        while Instant::now() < end {
            std::thread::sleep(Duration::from_millis(1500));
            let Some(win) = handle.get_webview_window(SIGNIN) else { return };
            let cookies = win.cookies_for_url(tauri::Url::parse(KICK).unwrap()).unwrap_or_default();
            if !has_session(&cookies) {
                continue;
            }
            let name = who_am_i(win.as_ref(), &cookies);
            let shared = handle.state::<Arc<Shared>>();
            *shared.kick.lock().unwrap() = KickLogin { signed_in: true, user: name.clone(), checked: Some(Instant::now()) };
            if let (Ok(conn), Some(n)) = (shared.db(), &name) {
                let _ = db::meta_set(&conn, "kick_user", n);
            }
            let _ = handle.emit("kick", name.unwrap_or_default());
            refresh(&handle, &shared);
            let _ = win.close();
            return;
        }
    });
    let _ = w;
    Ok(())
}

/// Signs out of Kick: its cookies go, the chat closes and auto betting is disarmed.
#[tauri::command]
pub async fn kick_sign_out(app: AppHandle, shared: tauri::State<'_, Arc<Shared>>) -> Cmd<()> {
    let handle = app.clone();
    std::thread::spawn(move || {
        if let Some(w) = handle.webviews().values().next() {
            for c in kick_cookies(&handle) {
                let _ = w.delete_cookie(c);
            }
        }
        if let Some(chat) = handle.get_webview(CHAT) {
            let _ = chat.close();
        }
        handle.state::<Arc<Shared>>().chat_channel.lock().unwrap().clear();
    })
    .join()
    .map_err(|_| "Couldn't sign out of Kick".to_string())?;
    shared.armed.store(false, Ordering::Relaxed);
    *shared.kick.lock().unwrap() = KickLogin { signed_in: false, user: None, checked: Some(Instant::now()) };
    if let Ok(conn) = shared.db() {
        let _ = conn.execute("DELETE FROM meta WHERE key = 'kick_user'", []);
    }
    let _ = app.emit("kick", "");
    refresh(&app, &shared);
    Ok(())
}

// --- the chat on Live ---

/// Live's chat: Kick's popout chat for the channel bets go into, a webview inside the main window laid over Live's
/// chat panel (so it moves, hides and comes back with the window). Shown only with auto betting on and Kick signed
/// in; hidden on the other tabs.
#[tauri::command]
pub async fn chat_place(app: AppHandle, shared: tauri::State<'_, Arc<Shared>>, spot: Spot) -> Cmd<()> {
    let view = shared.auto.lock().unwrap().clone();
    let show = spot.visible && view.enabled && view.signed_in && !view.channel.is_empty() && spot.w > 40.0 && spot.h > 40.0;
    if !show {
        if let Some(w) = app.get_webview(CHAT) {
            let _ = w.hide();
        }
        shared.chat_shown.store(false, Ordering::Relaxed);
        return Ok(());
    }
    let zoom = *shared.zoom.lock().unwrap();
    let (pos, size) = (LogicalPosition::new(spot.x * zoom, spot.y * zoom), LogicalSize::new(spot.w * zoom, spot.h * zoom));
    let w = point_chat(&app, &shared, &view.channel, pos, size)?;
    let _ = w.set_position(pos);
    let _ = w.set_size(size);
    let _ = w.show();
    shared.chat_shown.store(true, Ordering::Relaxed);
    Ok(())
}

/// The chat, made if it isn't there yet and sent to `channel` if it was last pointed somewhere else. Goes by the
/// channel it was last pointed at, never by its address: that differs while a page loads, and going by it reloaded
/// the chat over and over.
fn point_chat(
    app: &AppHandle,
    shared: &Shared,
    channel: &str,
    pos: LogicalPosition<f64>,
    size: LogicalSize<f64>,
) -> Result<tauri::Webview, String> {
    let url = tauri::Url::parse(&kick::popout_url(channel)).map_err(|e| e.to_string())?;
    let mut pointed = shared.chat_channel.lock().unwrap();
    if let Some(w) = app.get_webview(CHAT) {
        if *pointed != channel {
            let _ = w.navigate(url);
            *pointed = channel.to_string();
        }
        return Ok(w);
    }
    let window = crate::app::main_window(app).ok_or("no main window")?;
    let builder = WebviewBuilder::new(CHAT, WebviewUrl::External(url)).on_page_load(|w, p| {
        // a page that isn't Kick's chat (a link clicked in it) goes back to the chat
        if p.event() == PageLoadEvent::Finished && p.url().host_str().is_some_and(|h| h != "kick.com" && !h.ends_with(".kick.com")) {
            let _ = w.eval("history.back()");
        }
    });
    let w = window.add_child(builder, pos, size).map_err(|e| e.to_string())?;
    *pointed = channel.to_string();
    Ok(w)
}

/// The main window closed to the tray: the chat is hidden with it, and bets go in with Kick's own request.
pub fn hide_chat(app: &AppHandle) {
    let shared = app.state::<Arc<Shared>>();
    shared.chat_shown.store(false, Ordering::Relaxed);
    if let Some(w) = app.get_webview(CHAT) {
        let _ = w.hide();
    }
}

/// While armed, the chat is kept ready even when Live isn't showing (Augur in the tray): made hidden, on the right
/// channel. Returns whether it was just made or sent somewhere new (it needs a moment to load).
fn ensure_chat(app: &AppHandle, shared: &Shared, channel: &str) -> bool {
    let fresh = app.get_webview(CHAT).is_none() || *shared.chat_channel.lock().unwrap() != channel;
    if fresh {
        if let Ok(w) = point_chat(app, shared, channel, LogicalPosition::new(0.0, 0.0), LogicalSize::new(10.0, 10.0)) {
            if !shared.chat_shown.load(Ordering::Relaxed) {
                let _ = w.hide();
            }
        }
    }
    fresh
}

/// What typing a bet in did.
#[derive(Debug, PartialEq)]
enum Typed {
    /// it went in (typed, or Kick's own send answered), with Kick's id for the line when it said
    Sent(Option<String>),
    /// nothing went in, so nothing was sent: why
    NotSent(String),
}

const FILL: &str = r#"(function (text) {
  try {
    var box = document.querySelector('[data-testid="chat-input"]');
    if (!box) return 'no chat box (signed out, or the chat is still loading)';
    if (box.getAttribute('contenteditable') === 'false' || box.getAttribute('aria-disabled') === 'true' || box.closest('[aria-disabled="true"]'))
      return 'off';
    if ((box.innerText || '').trim()) return 'you were typing in the chat';
    var ed = box.__lexicalEditor;
    if (ed) {
      var t = { detail: 0, format: 0, mode: 'normal', style: '', text: text, type: 'text', version: 1 };
      ed.setEditorState(ed.parseEditorState(JSON.stringify({ root: { children: [{ children: [t], direction: 'ltr', format: '', indent: 0, type: 'paragraph', version: 1 }], direction: 'ltr', format: '', indent: 0, type: 'root', version: 1 } })));
    } else {
      box.focus();
      document.execCommand('selectAll');
      document.execCommand('insertText', false, text);
    }
    return 'filled';
  } catch (e) {
    return 'couldn’t fill the chat box: ' + e.message;
  }
})"#;

const PRESS: &str = r#"(function (text) {
  var box = document.querySelector('[data-testid="chat-input"]');
  if (!box) return 'the chat box went';
  if ((box.innerText || '').trim() !== text) return 'the chat box changed before Enter';
  box.focus();
  box.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true, cancelable: true }));
  return 'pressed';
})"#;

const AFTER: &str = r#"(function (text) {
  var box = document.querySelector('[data-testid="chat-input"]');
  if (!box || !(box.innerText || '').trim()) return 'cleared';
  if ((box.innerText || '').trim() !== text) return 'cleared';
  var ed = box.__lexicalEditor;
  if (ed) ed.setEditorState(ed.parseEditorState(JSON.stringify({ root: { children: [{ children: [], direction: null, format: '', indent: 0, type: 'paragraph', version: 1 }], direction: null, format: '', indent: 0, type: 'root', version: 1 } })));
  return 'stuck';
})"#;

/// How long typing waits for a greyed-out chat box (slow mode's few seconds after a message of yours).
const SLOW_WAIT_SECS: u32 = 6;

/// Types the command into the chat and presses Enter, once. Never presses again: if Kick doesn't take it, the
/// command is cleared from the box (so it can't go in later by accident) and the bet is marked not sent.
fn type_bet(app: &AppHandle, channel: &str, command: &str) -> Typed {
    let Some(w) = app.get_webview(CHAT) else { return Typed::NotSent("the chat isn't ready".into()) };
    let want = kick::popout_url(channel);
    if w.url().map(|u| u.as_str().trim_end_matches('/').to_lowercase()).ok() != Some(want.to_lowercase()) {
        return Typed::NotSent(format!("the chat on Live isn't {channel}'s"));
    }
    let text = serde_json::to_string(command).unwrap_or_default();
    // slow mode greys the box out for a few seconds after any message of yours: wait for it (only to type: Enter is
    // still pressed once)
    let mut tries = 0;
    loop {
        match eval(&w, format!("{FILL}({text})"), Duration::from_secs(3)) {
            Some(serde_json::Value::String(s)) if s == "filled" => break,
            Some(serde_json::Value::String(s)) if s == "off" && tries < SLOW_WAIT_SECS => {
                tries += 1;
                std::thread::sleep(Duration::from_secs(1));
            }
            Some(serde_json::Value::String(s)) if s == "off" => {
                return Typed::NotSent("the chat box stayed greyed out (followers or subscribers only?)".into())
            }
            Some(serde_json::Value::String(s)) => return Typed::NotSent(s),
            _ => return Typed::NotSent("the chat didn't answer".into()),
        }
    }
    // let the editor draw the text before Enter, as you would see it
    std::thread::sleep(Duration::from_millis(250));
    match eval(&w, format!("{PRESS}({text})"), Duration::from_secs(3)) {
        Some(serde_json::Value::String(s)) if s == "pressed" => {}
        Some(serde_json::Value::String(s)) => {
            let _ = eval(&w, format!("{AFTER}({text})"), Duration::from_secs(2));
            return Typed::NotSent(s);
        }
        // Enter may have gone in: say so, and let the chat tell
        _ => return Typed::Sent(None),
    }
    std::thread::sleep(Duration::from_millis(1200));
    match eval(&w, format!("{AFTER}({text})"), Duration::from_secs(2)) {
        Some(serde_json::Value::String(s)) if s == "stuck" => Typed::NotSent("Kick didn't take the Enter (cleared it from the box)".into()),
        _ => Typed::Sent(None),
    }
}

/// Starts the one send, from inside the hidden Kick chat page with its own sign-in: one POST to Kick's own
/// message endpoint, the request Kick's chat makes when you press Enter. Keyed by the auto bet, and only ever started
/// by this script, so polling it (below) can't send again.
const SEND_START: &str = r#"(function (room, text, key, t, x) {
  var s = (window.__augurSend = window.__augurSend || {});
  if (s[key]) return 'already';
  s[key] = { state: 'sending' };
  var h = { Accept: 'application/json', 'Content-Type': 'application/json' };
  if (t) h.Authorization = 'Bearer ' + decodeURIComponent(t);
  if (x) h['X-XSRF-TOKEN'] = decodeURIComponent(x);
  fetch('https://kick.com/api/v2/messages/send/' + room, { method: 'POST', credentials: 'include', headers: h, body: JSON.stringify({ content: text, type: 'message' }) })
    .then(function (r) {
      return r.text().then(function (b) {
        var j = null;
        try { j = JSON.parse(b); } catch (e) {}
        s[key] = { state: r.ok ? 'sent' : 'refused', code: r.status, id: (j && j.data && j.data.id) || '', msg: (j && (j.message || (j.status && j.status.message))) || '' };
      });
    })
    .catch(function (e) { s[key] = { state: 'unknown', msg: String((e && e.message) || e) }; });
  return 'started';
})"#;

const SEND_POLL: &str = r#"(function (key) { var s = window.__augurSend || {}; return s[key] || { state: 'gone' }; })"#;

/// The bet with the window hidden (or Live not showing): pages that aren't on screen may not take typing, so the
/// hidden chat page sends it with Kick's own request instead. Once, never again: an answer that doesn't come counts
/// as maybe sent, and the chat decides.
fn api_send(app: &AppHandle, room: u64, command: &str, key: i64) -> Typed {
    let Some(w) = app.get_webview(CHAT) else { return Typed::NotSent("the chat isn't ready".into()) };
    let cookies = kick_cookies(app);
    let get = |n: &str| {
        serde_json::to_string(&cookies.iter().find(|c| c.name() == n).map(|c| c.value().to_string()).unwrap_or_default())
            .unwrap_or_default()
    };
    let text = serde_json::to_string(command).unwrap_or_default();
    let key = format!("\"bet-{key}\"");
    match eval(&w, format!("{SEND_START}({room}, {text}, {key}, {}, {})", get("session_token"), get("XSRF-TOKEN")), Duration::from_secs(3))
    {
        Some(serde_json::Value::String(s)) if s == "started" => {}
        Some(serde_json::Value::String(s)) if s == "already" => return Typed::Sent(None),
        // the script may have run: maybe sent
        _ => return Typed::Sent(None),
    }
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(500));
        let Some(r) = eval(&w, format!("{SEND_POLL}({key})"), Duration::from_secs(2)) else { continue };
        match r["state"].as_str() {
            Some("sent") => return Typed::Sent(r["id"].as_str().filter(|i| !i.is_empty()).map(str::to_string)),
            Some("refused") => {
                let why = r["msg"].as_str().filter(|m| !m.is_empty()).unwrap_or("no reason given");
                return Typed::NotSent(format!("Kick refused it (HTTP {}: {why})", r["code"]));
            }
            Some("sending") => continue,
            _ => return Typed::Sent(None),
        }
    }
    Typed::Sent(None)
}

/// The bet goes in: typed into the chat when you can see it on Live, sent by the hidden chat page otherwise.
fn send_bet(app: &AppHandle, shared: &Shared, v: &AutoView, room: Option<u64>, command: &str, id: i64) -> Typed {
    let main = crate::app::main_window(app);
    let on_screen = shared.chat_shown.load(Ordering::Relaxed)
        && main.as_ref().is_some_and(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(true));
    if on_screen {
        return type_bet(app, &v.channel, command);
    }
    match room {
        Some(room) => api_send(app, room, command, id),
        None => Typed::NotSent(format!("couldn't look up {}'s chat on Kick", v.channel)),
    }
}

// --- the health check: armed, can it actually bet? ---

const PROBE: &str = r#"(function () {
  var box = document.querySelector('[data-testid="chat-input"]');
  if (!box) return document.readyState === 'complete' ? 'no-box' : 'loading';
  if (box.getAttribute('contenteditable') === 'false' || box.getAttribute('aria-disabled') === 'true' || box.closest('[aria-disabled="true"]')) return 'off';
  return 'ok';
})()"#;

/// The chat box greyed out. Once is slow mode's few seconds; twice in a row (30 seconds apart) it's followers or
/// subscribers only, and stops betting.
const BOX_OFF: &str = "The chat box is greyed out (followers or subscribers only?)";

/// What's stopping auto betting, if anything, and whether it blocks it. `chat_since`: when the chat was made or sent
/// to a new channel (it gets a moment to load).
fn health(app: &AppHandle, shared: &Shared, v: &AutoView, signed: bool, chat_since: Instant) -> Option<(String, bool)> {
    if !signed {
        return Some(("Signed out of Kick".into(), true));
    }
    if let Some(o) = &v.other {
        return Some((format!("{o} has auto betting on"), true));
    }
    if v.channel.is_empty() {
        return Some((if v.test { "Set your Kick name for test mode" } else { "No channel set" }.into(), true));
    }
    let chan = match kick::channel(&v.channel) {
        Ok(c) => c,
        Err(augur_engine::error::Error::Http(404)) => return Some((format!("{} isn't a Kick channel", v.channel), true)),
        Err(_) => return Some(("Can't reach Kick".into(), true)),
    };
    // off air, Kick tightens the chat (it greys out) and there's nothing to bet on: only the problems above, which want
    // fixing before the stream, count until it's back
    shared.auto.lock().unwrap().stream_live = Some(chan.live);
    if !chan.live {
        return None;
    }
    if kick::history(chan.id).is_err() {
        return Some(("Can't read the Kick chat, so it can't check you haven't bet".into(), true));
    }
    let loading = chat_since.elapsed() < Duration::from_secs(30);
    match app.get_webview(CHAT).and_then(|w| eval(&w, PROBE.to_string(), Duration::from_secs(3))) {
        Some(serde_json::Value::String(s)) if s == "ok" => {}
        // slow mode greys it out for a few seconds after any message: only a box still greyed out at the next check
        // stops betting (the loop decides)
        Some(serde_json::Value::String(s)) if s == "off" => return Some((BOX_OFF.into(), false)),
        _ if loading => {}
        _ => return Some(("The Kick chat isn't ready (sign in to Kick again?)".into(), true)),
    }
    // the last real bet today didn't go in: worth knowing, but the next one may
    let today = shared.db().ok().and_then(|c| autobet::all(&c).ok()).unwrap_or_default();
    if let Some(b) =
        today.iter().find(|b| b.device.is_none() && b.claimed_at >= midnight(now_ms()) && b.status != "skipped" && b.status != "cancelled")
    {
        if b.status == "failed" {
            return Some((format!("Last auto bet not sent: {}", b.reason.clone().unwrap_or_default()), false));
        }
    }
    None
}

// --- arming, cancelling, and what Live shows ---

/// Arms auto betting for this session (until Augur quits), or disarms it.
#[tauri::command]
pub fn auto_arm(app: AppHandle, shared: tauri::State<'_, Arc<Shared>>, on: bool) -> Cmd<AutoView> {
    if on {
        can_arm(&shared.auto.lock().unwrap())?;
    }
    shared.armed.store(on, Ordering::Relaxed);
    shared.auto_check.store(true, Ordering::Relaxed);
    refresh(&app, &shared);
    Ok(shared.auto.lock().unwrap().clone())
}

/// Why auto betting can't be armed now, if it can't.
fn can_arm(v: &AutoView) -> Result<(), String> {
    if !v.enabled {
        return Err("Turn on auto betting in Settings › Auto bet first.".into());
    }
    if !v.signed_in {
        return Err("Sign in to Kick in Settings › Auto bet first.".into());
    }
    if v.channel.is_empty() {
        return Err(if v.test {
            "Set your Kick name (Settings › Account) for test mode."
        } else {
            "Set the stream's Kick channel in Settings › Auto bet."
        }
        .into());
    }
    if let Some(other) = &v.other {
        return Err(format!("{other} has auto betting on: only one computer should bet."));
    }
    Ok(())
}

/// Arm when Augur starts: the first wait lets sync hear from the other computers (only one should bet), and it keeps
/// trying while the sign-in and the chat come up. Given up on, with a notification saying why, after this long.
const ARM_AT_START_AFTER: Duration = Duration::from_secs(20);
const ARM_AT_START_UNTIL: Duration = Duration::from_secs(10 * 60);

/// Cancel during the countdown: this fight is ruled out (and can't be bet on later).
#[tauri::command]
pub fn auto_cancel(app: AppHandle, shared: tauri::State<'_, Arc<Shared>>) -> Cmd<()> {
    *shared.cancel.lock().unwrap() = true;
    refresh(&app, &shared);
    Ok(())
}

#[tauri::command]
pub fn auto_view(shared: tauri::State<'_, Arc<Shared>>) -> AutoView {
    shared.auto.lock().unwrap().clone()
}

/// Works out the view's settled parts (settings, sign-in, the other computers, today's bets) and sends it.
pub fn refresh(app: &AppHandle, shared: &Shared) {
    let settings = shared.settings.lock().unwrap().clone();
    let s = &settings.auto_bet;
    let login = shared.kick.lock().unwrap().clone();
    let now = now_ms();
    let other =
        shared.others.lock().unwrap().iter().find(|o| o.auto_bet && now.saturating_sub(o.at) < OTHER_FRESH_MS).map(|o| o.name.clone());
    if !s.enabled {
        shared.armed.store(false, Ordering::Relaxed);
    }
    let (today, window) = shared
        .db()
        .ok()
        .map(|c| (autobet::spent_since(&c, midnight(now)).unwrap_or_default(), db::picks_window(&c).ok().flatten()))
        .unwrap_or_default();
    let mut v = shared.auto.lock().unwrap();
    v.enabled = s.enabled;
    v.armed = shared.armed.load(Ordering::Relaxed);
    v.signed_in = login.signed_in;
    v.user = login.user.clone().or_else(|| (!settings.kick_name.is_empty()).then(|| settings.kick_name.clone()));
    v.channel = channel(s, login.user.as_deref(), &settings.kick_name);
    v.test = s.test_mode;
    v.other = other;
    v.today = today;
    v.window = window;
    if !v.armed {
        v.phase = "off".into();
        v.countdown_until = None;
        v.problem = None;
        v.blocking = false;
    }
    let _ = app.emit("autobet", &*v);
    let armed = v.armed;
    drop(v);
    if let Some(item) = shared.arm_item.lock().unwrap().as_ref() {
        let _ = item.set_text(if armed { "Disarm auto bet" } else { "Arm auto bet" });
        let _ = item.set_enabled(s.enabled);
    }
    crate::app::update_tray(app, shared);
}

/// The start of today, local time near enough (UTC: the limits are a safety cap, not a calendar).
fn midnight(now: u64) -> u64 {
    now - now % 86_400_000
}

// --- the loop ---

/// The fight auto betting is watching.
struct Track {
    call_id: i64,
    /// what the call is now (pick and command): a change restarts the hold
    key: String,
    held_since: u64,
    countdown_until: Option<u64>,
    /// chat lines read since the fight came up
    lines: Vec<Line>,
    read_at: Option<Instant>,
    /// why it's waiting, kept for the record if picks close first
    waiting: String,
}

/// Every second while auto betting is on: watches the call that's up, and when it's time, bets on it once.
pub fn auto_loop(app: AppHandle, shared: Arc<Shared>) {
    let mut track: Option<Track> = None;
    let mut channel_id: Option<(String, u64, Instant)> = None;
    let mut last_refresh = Instant::now() - Duration::from_secs(60);
    let mut next_check = Instant::now();
    let mut chat_since = Instant::now();
    // checks in a row that found the chat box greyed out
    let mut box_off = 0u32;
    // Arm when Augur starts: tried once this session (disarming later keeps it off)
    let started = Instant::now();
    let mut arm_at_start = shared.settings.lock().unwrap().auto_bet.arm_at_start;
    // the name you signed in as, from last time
    if let Ok(conn) = shared.db() {
        shared.kick.lock().unwrap().user = db::meta_get(&conn, "kick_user").ok().flatten();
    }
    while !shared.stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_secs(1));
        // Kick's sign-in is only looked at while auto betting is on
        let enabled = shared.settings.lock().unwrap().auto_bet.enabled;
        let signed = enabled && signed_in(&app, &shared);
        if last_refresh.elapsed() > Duration::from_secs(15) {
            refresh(&app, &shared);
            last_refresh = Instant::now();
        }
        let view = shared.auto.lock().unwrap().clone();
        if arm_at_start && !view.armed && started.elapsed() >= ARM_AT_START_AFTER {
            match can_arm(&view) {
                Ok(()) => {
                    arm_at_start = false;
                    shared.armed.store(true, Ordering::Relaxed);
                    shared.auto_check.store(true, Ordering::Relaxed);
                    refresh(&app, &shared);
                    notify(
                        &app,
                        "Auto bet armed",
                        &format!("as Augur started, into {}'s chat{}", view.channel, if view.test { " (test mode)" } else { "" }),
                    );
                    continue;
                }
                Err(why) if !view.enabled || started.elapsed() >= ARM_AT_START_UNTIL => {
                    arm_at_start = false;
                    if view.enabled {
                        notify(&app, "Auto bet not armed at start", &why);
                    }
                }
                Err(_) => {}
            }
        } else if view.armed {
            arm_at_start = false;
        }
        if !(view.enabled && view.armed) {
            track = None;
            *shared.cancel.lock().unwrap() = false;
            continue;
        }
        // armed: keep the chat ready, and check every 30 seconds (and straight after arming) that it can bet
        if signed && !view.channel.is_empty() && ensure_chat(&app, &shared, &view.channel) {
            chat_since = Instant::now();
        }
        if shared.auto_check.swap(false, Ordering::Relaxed) || Instant::now() >= next_check {
            next_check = Instant::now() + Duration::from_secs(30);
            let mut found = health(&app, &shared, &view, signed, chat_since);
            box_off = if found.as_ref().is_some_and(|(p, _)| p == BOX_OFF) { box_off + 1 } else { 0 };
            if box_off == 1 {
                found = None; // slow mode's moment, most likely: look again next time
            } else if let Some(f) = found.as_mut().filter(|(p, _)| p == BOX_OFF) {
                f.1 = true;
            }
            let before = view.problem.clone();
            {
                let mut v = shared.auto.lock().unwrap();
                (v.problem, v.blocking) = found.clone().map_or((None, false), |(p, b)| (Some(p), b));
                let _ = app.emit("autobet", &*v);
            }
            // a problem cleared because the stream ended isn't "working again"
            let on_air = shared.auto.lock().unwrap().stream_live != Some(false);
            match (&before, &found) {
                (None, Some((p, _))) => notify(&app, "Auto bet can't bet", p),
                (Some(_), None) if on_air => notify(&app, "Auto bet is working again", &format!("into {}'s chat", view.channel)),
                _ => {}
            }
            crate::app::update_tray(&app, &shared);
        }
        let view = shared.auto.lock().unwrap().clone();
        if let (Some(p), true) = (&view.problem, view.blocking) {
            set_phase(&app, &shared, "waiting", p, "", None, None);
            continue;
        }
        // another computer turned it on after this one was armed: neither should bet twice, so this one stands down
        if let Some(other) = &view.other {
            set_phase(&app, &shared, "waiting", &format!("{other} has auto betting on"), "", None, None);
            continue;
        }
        let Ok(conn) = shared.db() else { continue };
        let settings = shared.settings.lock().unwrap().clone();
        let s = settings.auto_bet.clone();
        let live = shared.live.lock().unwrap().clone();
        let (Some(p), Some(stake)) = (live.prediction.clone(), live.stake.clone()) else {
            if let Some(t) = track.take() {
                close_out(&conn, &t, "the fight went before picks closed");
            }
            set_phase(&app, &shared, "idle", "Waiting for the next fight", "", None, None);
            continue;
        };
        let Ok(Some(call_id)) = db::open_call(&conn, &p.home.owner, &p.away.owner) else { continue };
        if let Some(b) = autobet::for_call(&conn, call_id).ok().flatten() {
            let msg = match b.status.as_str() {
                "confirmed" => "Bet in and confirmed".to_string(),
                "cancelled" => "Cancelled for this fight".to_string(),
                s => b.reason.clone().unwrap_or_else(|| s.to_string()),
            };
            set_phase(&app, &shared, "done", &msg, &b.command.clone(), None, Some(b));
            continue;
        }
        let key = format!("{}|{}", p.pick, stake.command);
        let now = now_ms();
        if track.as_ref().is_none_or(|t| t.call_id != call_id) {
            if let Some(old) = track.take() {
                close_out(&conn, &old, "picks closed before it was time to bet");
            }
            *shared.cancel.lock().unwrap() = false;
            track = Some(Track {
                call_id,
                key: key.clone(),
                held_since: now,
                countdown_until: None,
                lines: vec![],
                read_at: None,
                waiting: String::new(),
            });
        }
        let Some(t) = track.as_mut() else { continue };
        if t.key != key {
            (t.key, t.held_since, t.countdown_until) = (key, now, None);
        }
        // the chat, every few seconds: your own bets, and later the auto bet's confirmation
        if !view.channel.is_empty() && t.read_at.is_none_or(|r| r.elapsed() >= READ_EVERY) {
            t.read_at = Some(Instant::now());
            if channel_id.as_ref().is_none_or(|(c, _, at)| *c != view.channel || at.elapsed() > Duration::from_secs(600)) {
                channel_id = kick::channel(&view.channel).ok().map(|c| (view.channel.clone(), c.id, Instant::now()));
            }
            if let Some((_, id, _)) = &channel_id {
                if let Ok(lines) = kick::history(*id) {
                    merge(&mut t.lines, lines);
                }
            }
        }
        if !live.picks_open {
            let reason = if t.waiting.is_empty() { "picks closed".to_string() } else { format!("picks closed: {}", t.waiting) };
            if let Some(t) = track.take() {
                close_out(&conn, &t, &reason);
            }
            continue;
        }
        if std::mem::take(&mut *shared.cancel.lock().unwrap()) {
            let _ = autobet::claim(&conn, &record(&p, &stake, call_id, &view, "cancelled", Some("you cancelled it")));
            notify(&app, "Auto bet cancelled", &format!("{} on {} vs {}", stake.command, p.home.pal, p.away.pal));
            continue;
        }
        let user = view.user.clone().unwrap_or_default();
        let site_now = live.site_now.map(|n| n + now.saturating_sub(live.synced_at)).unwrap_or(now);
        let since = p.seen_at.saturating_sub(2_000);
        let m = Moment {
            picks_open: true,
            prob: p.prob,
            amount: stake.amount,
            estimated: p.estimated(),
            held_ms: now.saturating_sub(t.held_since),
            open_ms: site_now.saturating_sub(p.seen_at),
            today: autobet::spent_since(&conn, midnight(now)).unwrap_or_default(),
            signed_in: signed,
            already_bet: (!user.is_empty() && autobet::own_bet(&t.lines, &user, since, None).is_some())
                || bet_in_picks(&conn, &p.home.owner, &p.away.owner, now.saturating_sub(site_now.saturating_sub(p.seen_at))),
        };
        match autobet::decide(&s, &m) {
            Decision::Wait(why) => {
                t.countdown_until = None;
                t.waiting = why.clone();
                set_phase(&app, &shared, "waiting", &why, &stake.command, None, None);
            }
            Decision::Skip(why) => {
                let _ = autobet::claim(&conn, &record(&p, &stake, call_id, &view, "skipped", Some(&why)));
                set_phase(&app, &shared, "done", &format!("Not betting: {why}"), &stake.command, None, None);
            }
            Decision::Go => {
                let until = *t.countdown_until.get_or_insert(now + s.countdown_secs * 1000);
                if now < until {
                    set_phase(&app, &shared, "countdown", "Betting in a moment", &stake.command, Some(until), None);
                    continue;
                }
                // the claim: from here this fight can never be bet on again, whatever happens next
                let Ok(Some(id)) = autobet::claim(&conn, &record(&p, &stake, call_id, &view, "sending", None)) else { continue };
                set_phase(&app, &shared, "sending", "Typing it into the chat", &stake.command, None, None);
                let lines = std::mem::take(&mut t.lines);
                bet_now(&app, &shared, &conn, id, &view, &p, &stake.command, lines);
                track = None;
            }
        }
    }
}

fn record(
    p: &augur_engine::predict::Prediction,
    stake: &augur_engine::stake::Stake,
    call_id: i64,
    v: &AutoView,
    status: &str,
    reason: Option<&str>,
) -> AutoBet {
    AutoBet {
        prediction_id: Some(call_id),
        home_owner: p.home.owner.clone(),
        away_owner: p.away.owner.clone(),
        home_pal: p.home.pal.clone(),
        away_pal: p.away.pal.clone(),
        side: p.pick.clone(),
        amount: stake.amount,
        command: stake.command.clone(),
        prob: p.prob,
        channel: v.channel.clone(),
        test: v.test,
        status: status.into(),
        reason: reason.map(str::to_string),
        claimed_at: now_ms(),
        ..Default::default()
    }
}

/// A fight that went without a bet is recorded as skipped, with why, so History can say.
fn close_out(conn: &rusqlite::Connection, t: &Track, why: &str) {
    if t.waiting.is_empty() {
        return; // never got as far as thinking about it
    }
    let _ = conn.execute(
        "INSERT OR IGNORE INTO auto_bets (prediction_id, status, reason, claimed_at, home_owner, away_owner, home_pal, away_pal, side, prob)
         SELECT id, 'skipped', ?2, ?3, home_owner, away_owner, home_pal, away_pal, pick, prob FROM predictions WHERE id = ?1",
        rusqlite::params![t.call_id, why, now_ms() as i64],
    );
}

/// You bet on this fight yourself, as your picks list last showed it (read since the fight came up, still open).
fn bet_in_picks(conn: &rusqlite::Connection, home: &str, away: &str, since_local: u64) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM bets WHERE home_owner = ?1 AND away_owner = ?2 AND result IS NULL AND seen_at >= ?3",
        rusqlite::params![home, away, since_local as i64],
        |r| r.get::<_, i64>(0),
    )
    .is_ok_and(|n| n > 0)
}

fn merge(into: &mut Vec<Line>, more: Vec<Line>) {
    for l in more {
        if !into.iter().any(|x| x.id == l.id) {
            into.push(l);
        }
    }
    into.sort_by_key(|l| l.at);
    let keep = into.len().saturating_sub(400);
    into.drain(..keep);
}

fn set_phase(app: &AppHandle, shared: &Shared, phase: &str, msg: &str, command: &str, until: Option<u64>, bet: Option<AutoBet>) {
    let mut v = shared.auto.lock().unwrap();
    if v.phase == phase && v.message == msg && v.command == command && v.countdown_until == until && v.bet == bet {
        return;
    }
    (v.phase, v.message, v.command, v.countdown_until, v.bet) = (phase.into(), msg.into(), command.into(), until, bet);
    let _ = app.emit("autobet", &*v);
}

/// The bet, once: type it in, then watch the chat for it and the bot's answer. Never sent again, whatever happens.
#[allow(clippy::too_many_arguments)]
fn bet_now(
    app: &AppHandle,
    shared: &Shared,
    conn: &rusqlite::Connection,
    id: i64,
    v: &AutoView,
    p: &augur_engine::predict::Prediction,
    command: &str,
    mut lines: Vec<Line>,
) {
    let what = format!("{command} on {} vs {}", p.home.pal, p.away.pal);
    let chan = kick::channel(&v.channel).ok();
    let sent_at = now_ms();
    let line_id = match send_bet(app, shared, v, chan.as_ref().map(|c| c.chatroom_id), command, id) {
        Typed::NotSent(why) => {
            let _ = autobet::update(conn, id, "failed", Some(&why), now_ms());
            notify(app, "Auto bet not sent", &format!("{what}: {why}"));
            finish(app, shared, conn, id);
            return;
        }
        Typed::Sent(line) => {
            let _ = autobet::update(conn, id, "sent", None, sent_at);
            if let Some(l) = &line {
                let _ = autobet::set_line(conn, id, l, None);
            }
            line
        }
    };
    finish(app, shared, conn, id);
    let user = v.user.clone().unwrap_or_default();
    let end = Instant::now() + CONFIRM_FOR;
    let mut seen: Option<Line> = None;
    while Instant::now() < end {
        std::thread::sleep(Duration::from_secs(3));
        let Some(c) = &chan else { break };
        if let Ok(more) = kick::history(c.id) {
            merge(&mut lines, more);
        }
        if seen.is_none() {
            // Kick's id for the line when its send said, else yours with the command's text
            let found = match &line_id {
                Some(lid) => lines.iter().find(|l| &l.id == lid),
                None => autobet::sent_line(&lines, &user, command, sent_at),
            };
            if let Some(l) = found {
                let _ = autobet::set_line(conn, id, &l.id, None);
                let _ = autobet::update(conn, id, "seen", None, now_ms());
                seen = Some(l.clone());
                finish(app, shared, conn, id);
                if v.test {
                    break;
                }
            }
        }
        // the bot took it, found by its reply (even if your line has gone: the AI moderator deletes some commands)
        if let Some(r) = autobet::bot_took(&lines, &user, command, sent_at).filter(|_| !v.test) {
            if let Some(lid) = &r.reply_to {
                let _ = autobet::set_line(conn, id, lid, Some(&r.content));
            }
            let _ = autobet::update(conn, id, "confirmed", None, now_ms());
            notify(app, "Auto bet in ✓", &format!("{what} · {}", r.content.chars().take(80).collect::<String>()));
            finish(app, shared, conn, id);
            shared.bets_now.store(true, Ordering::Relaxed);
            return;
        }
        if let Some(l) = &seen {
            if let Some(r) = autobet::bot_reply(&lines, l) {
                let _ = autobet::set_line(conn, id, &l.id, Some(&r.content));
                let _ = autobet::update(conn, id, "confirmed", None, now_ms());
                notify(app, "Auto bet in ✓", &format!("{what} · {}", r.content.chars().take(80).collect::<String>()));
                finish(app, shared, conn, id);
                shared.bets_now.store(true, Ordering::Relaxed);
                return;
            }
        }
    }
    let (status, why) = match (&seen, v.test) {
        (Some(_), true) => ("confirmed", "test: in your chat, no bet placed"),
        (Some(_), false) => ("unconfirmed", "in the chat, but the bot didn't answer"),
        (None, _) => ("unconfirmed", "not seen in the chat"),
    };
    let _ = autobet::update(conn, id, status, Some(why), now_ms());
    notify(
        app,
        if status == "confirmed" { "Auto bet test ✓" } else { "Auto bet unconfirmed" },
        &format!("{what}: {why}. It won't be sent again."),
    );
    finish(app, shared, conn, id);
    shared.bets_now.store(true, Ordering::Relaxed);
}

fn finish(app: &AppHandle, shared: &Shared, conn: &rusqlite::Connection, id: i64) {
    let b = autobet::all(conn).ok().and_then(|all| all.into_iter().find(|b| b.id == id));
    if let Some(b) = b {
        let msg = match b.status.as_str() {
            "sent" => "Typed in: looking for it in the chat".to_string(),
            "seen" => "In the chat: waiting for the bot".to_string(),
            "confirmed" => "Bet in and confirmed".to_string(),
            _ => b.reason.clone().unwrap_or_else(|| b.status.clone()),
        };
        set_phase(
            app,
            shared,
            if matches!(b.status.as_str(), "sent" | "seen") { "sending" } else { "done" },
            &msg,
            &b.command.clone(),
            None,
            Some(b),
        );
    }
    refresh(app, shared);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bets_go_into_the_right_chat() {
        let s = AutoBetSettings { channel: " The-Stream ".into(), test_mode: false, ..Default::default() };
        assert_eq!(channel(&s, Some("Me"), "fallback"), "the-stream");
        let test = AutoBetSettings { test_mode: true, ..s };
        assert_eq!(channel(&test, Some("TrongatesLegacy"), "x"), "trongateslegacy", "test mode: your own chat");
        assert_eq!(channel(&test, None, "MyName"), "myname", "your Kick name until the sign-in says");
    }

    #[test]
    fn midnight_is_the_start_of_the_day() {
        assert_eq!(midnight(1_794_355_200_000 + 5_000), 1_794_355_200_000);
    }
}
