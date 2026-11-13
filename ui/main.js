// The main window: the sidebar, the shared state the tabs read, and the events from the Rust side.
import { activity, activityHtml, FLASH_MS } from './lib/activity.js';
import { invoke, listen } from './lib/bridge.js';
import { esc } from './lib/format.js';
import * as history from './tabs/history.js';
import * as live from './tabs/live.js';
import * as model from './tabs/model.js';
import * as pals from './tabs/pals.js';
import * as players from './tabs/players.js';
import * as settings from './tabs/settings.js';
import * as you from './tabs/you.js';

const TABS = { live, history, pals, players, you, model, settings };

// the You tab is there only while signed in to PalDuels
const youNav = document.querySelector('.nav[data-tab="you"]');
const showYou = (on) => {
  youNav.hidden = !on;
  if (!on && app.state.tab === 'you') app.show('live');
};

const app = {
  main: document.getElementById('main'),
  state: { tab: 'live', range: 'today', view: 'calls', call: null, youView: 'profile', pal: null, player: null, voice: null, live: null, settings: null, speciesList: null },
  header: (sub, right = '') => `<div class="top"><div class="bm">AUGUR</div><span class="sub">${sub}</span>${right}</div>`,
  show(tab) {
    // another tab closes a call's breakdown in History
    if (tab !== app.state.tab) app.state.call = null;
    app.state.tab = tab;
    document.querySelectorAll('.nav').forEach((n) => n.classList.toggle('on', n.dataset.tab === tab));
    return TABS[tab].render(app);
  },
  // a short message at the top of the page: an error (saving settings failed), or `good` news (an import's result)
  flash(msg, good) {
    const d = Object.assign(document.createElement('div'), { className: `panel ${good ? 'note' : 'bad'}`, textContent: String(msg) });
    app.main.prepend(d);
    setTimeout(() => d.remove(), 5000);
  },
};

document.querySelectorAll('.nav').forEach((n) => (n.onclick = () => app.show(n.dataset.tab)));

// A fresh install: a welcome over the whole window that stays until a choice is made, since the background fetch
// waits for it. Import a backup (from another computer) or start fresh.
const LOGO = document.querySelector('.logo').outerHTML;
function showWelcome(live) {
  let el = document.getElementById('welcome');
  if (!live?.first_run) return el?.remove();
  if (el) return;
  el = Object.assign(document.createElement('div'), { id: 'welcome', className: 'overlay' });
  el.setAttribute('role', 'dialog');
  el.setAttribute('aria-modal', 'true');
  el.setAttribute('aria-labelledby', 'wh');
  el.innerHTML = `<div class="sheet">${LOGO}<h1 id="wh">Welcome to Augur</h1>
    <p>It calls each PalDuels fight before the bell. First, is this a new computer for an Augur you already use?</p>
    <div class="choices">
      <button class="choice main" id="wi"><b>Import a backup</b><small>From your other computer (Settings › Sync &amp; data › Back up…). Brings your calls, bets and every past duel across, so there's no hour-long first fetch.</small></button>
      <button class="choice" id="wf"><b>Start fresh</b><small>Augur fetches the duel history in the background, gently, pausing while a duel is up.</small></button>
    </div>
    <p class="small" id="wm">Live calls work straight away either way.</p></div>`;
  document.body.append(el);
  const done = async (msg) => {
    el.remove();
    app.state.live = await invoke('live');
    if (app.state.tab === 'live') live.render(app);
    if (msg) app.flash(msg, true);
  };
  el.querySelector('#wf').onclick = async () => {
    await invoke('start_fresh');
    done();
  };
  el.querySelector('#wi').onclick = async (e) => {
    const b = e.currentTarget;
    b.disabled = true;
    try {
      const r = await invoke('import_file');
      if (r) return done(`Imported ${r.file}: ${r.merged.calls} calls, ${r.merged.duels} duels, ${r.merged.bets} bets. Only newer duels are left to fetch.`);
    } catch (err) {
      el.querySelector('#wm').innerHTML = `<span class="bad">${esc(err)}</span>`;
    }
    b.disabled = false;
  };
  el.querySelector('#wi').focus();
}

// the sidebar's activity spot: what's being brought in (click: where it's shown in full)
const bf = document.getElementById('bf');
const busy = { fetch: null, bets: null, sync: null, flash: null };
let flashTimer;
let tickTimer;
function showActivity() {
  const a = activity(busy);
  bf.hidden = !a;
  clearTimeout(tickTimer);
  if (!a) return;
  // a countdown in the ring: no event comes while the fetch waits out the site, so count down here
  if (a.tick) tickTimer = setTimeout(showActivity, 15_000);
  bf.title = a.long;
  bf.dataset.go = a.go;
  bf.className = `bf ${a.kind}${a.warn ? ' warn' : ''}`;
  bf.innerHTML = activityHtml(a);
}
bf.onclick = () => {
  const go = bf.dataset.go;
  if (go === 'sync') (app.state.settingsPage = 'sync'), app.show('settings');
  else if (go === 'bets') (app.state.view = 'bets'), app.show('history');
  else app.show('model');
};
listen('backfill', (p) => ((busy.fetch = p), showActivity()));
listen('bets', (p) => ((busy.bets = p), showActivity()));
listen('live', (v) => {
  app.state.live = v;
  showWelcome(v);
  if (app.state.tab === 'live') live.render(app);
});
listen('navigate', (tab) => app.show(tab));
listen('settings', (s) => {
  // another computer's change arrived through sync
  app.state.settings = s;
  if (app.state.tab === 'settings') settings.render(app);
});
listen('sync', (v) => {
  // a change from another computer that's newer than the last one shown: "Synced" for a few seconds
  const c = v?.history?.[0];
  if (c && busy.sync && c.at > (busy.sync.history?.[0]?.at || 0)) {
    busy.flash = c;
    clearTimeout(flashTimer);
    flashTimer = setTimeout(() => ((busy.flash = null), showActivity()), FLASH_MS);
  }
  busy.sync = v;
  showActivity();
  if (app.state.tab === 'settings' && app.state.settingsPage === 'sync') settings.render(app);
});
listen('calls', () => {
  if (app.state.tab === 'history') history.render(app);
});
// Updates: a dot on the Settings icon while there's news, and a bar floating over the window when an update is ready
// (Restart to update) or was just installed (what's new). "Later" hides the ready bar until the next start.
const gear = document.querySelector('.nav.gear');
let later = false;
function showUpdate(u) {
  gear.classList.toggle('dot-on', !!(u && (u.latest || u.updated_from)));
  let bar = document.getElementById('upbar');
  const ready = u?.latest && u.ready && !later;
  const news = u?.updated_from;
  if (!ready && !news && !u?.installing) return bar?.remove();
  if (!bar) {
    bar = Object.assign(document.createElement('div'), { id: 'upbar', className: 'upbar' });
    bar.setAttribute('role', 'status');
    document.body.append(bar);
  }
  bar.classList.toggle('news', !ready && !!news);
  bar.innerHTML = u.installing
    ? `<span class="ui-g">↻</span><div><b>Restarting into Augur ${esc(u.latest?.version || '')}…</b><small>back in a few seconds</small></div>`
    : ready
      ? `<span class="ui-g">✦</span><div><b>Augur ${esc(u.latest.version)} is ready</b><small>${app.state.live?.live ? 'a duel is on: maybe after the result' : 'restarting takes a few seconds'}</small></div>
         <button class="btn ghost sm" data-u="new">What’s new</button><button class="btn ghost sm" data-u="later">Later</button><button class="btn sm" data-u="go">Restart to update</button>`
      : `<span class="ui-g">✓</span><div><b>Updated to ${esc(u.current)}</b><small>from ${esc(news)}</small></div>
         <button class="btn ghost sm" data-u="new">What’s new</button><button class="btn ghost sm" data-u="ok" aria-label="Dismiss">✕</button>`;
  const open = () => ((app.state.settingsPage = 'updates'), app.show('settings'));
  bar.querySelectorAll('[data-u]').forEach((b) => {
    b.onclick = async () => {
      const k = b.dataset.u;
      if (k === 'new') open();
      else if (k === 'later') (later = true), showUpdate(u);
      else if (k === 'ok') await invoke('update_seen'), showUpdate(await invoke('update_view'));
      else (b.disabled = true), invoke('update_install').catch((e) => app.flash(e));
    };
  });
}
app.badge = showUpdate;
listen('update', (u) => {
  showUpdate(u);
  if (app.state.tab === 'settings' && app.state.settingsPage === 'updates') settings.render(app);
});
listen('bets', (p) => {
  // redraw Your bets (and You) when an import finishes or moves on a page
  if (app.state.tab === 'history' && app.state.view === 'bets' && (p.phase === 'done' || p.done % 25 === 0)) history.render(app);
  if (app.state.tab === 'you' && p.phase === 'done') you.render(app);
});
listen('account', (name) => {
  showYou(!!name);
  if (app.state.tab === 'settings') settings.render(app);
});

app.state.settings = await invoke('get_settings');
app.state.live = await invoke('live');
busy.fetch = await invoke('backfill_progress');
busy.sync = await invoke('sync_view').catch(() => null);
showActivity();
showWelcome(app.state.live);
showYou((await invoke('account')).signed_in);
showUpdate(await invoke('update_view'));
await app.show('live');
// for the headless UI tests
window.augur = Object.assign(app, { showWelcome, showUpdate });
