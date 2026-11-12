// The main window: the sidebar, the shared state the tabs read, and the events from the Rust side.
import { backfillState } from './lib/backfill.js';
import { invoke, listen } from './lib/bridge.js';
import { esc } from './lib/format.js';
import * as history from './tabs/history.js';
import * as live from './tabs/live.js';
import * as model from './tabs/model.js';
import * as pals from './tabs/pals.js';
import * as players from './tabs/players.js';
import * as settings from './tabs/settings.js';

const TABS = { live, history, pals, players, model, settings };

const app = {
  main: document.getElementById('main'),
  state: { tab: 'live', range: 'today', view: 'calls', pal: null, player: null, voice: null, live: null, settings: null, speciesList: null },
  header: (sub, right = '') => `<div class="top"><div class="bm">AUGUR</div><span class="sub">${sub}</span>${right}</div>`,
  show(tab) {
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

// the sidebar's small ring while the background fetch runs (click: the Model tab's details)
const bf = document.getElementById('bf');
function showBackfill(p) {
  const st = backfillState(p);
  bf.hidden = !st;
  if (!st) return;
  bf.title = st.long;
  bf.classList.toggle('warn', !!st.warn);
  bf.innerHTML = `<svg viewBox="0 0 36 36" aria-hidden="true"><circle cx="18" cy="18" r="15" class="t"/><circle cx="18" cy="18" r="15" class="f" style="stroke-dasharray:${(st.share * 94.2).toFixed(1)} 94.2"/></svg><span>${esc(st.short)}</span><small>${st.warn ? 'Site busy' : 'Fetching'}</small>`;
}
bf.onclick = () => app.show('model');
listen('backfill', showBackfill);
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
listen('sync', () => {
  if (app.state.tab === 'settings' && app.state.settingsPage === 'sync') settings.render(app);
});
listen('calls', () => {
  if (app.state.tab === 'history') history.render(app);
});
listen('bets', (p) => {
  // redraw Your bets when an import finishes or moves on a page
  if (app.state.tab === 'history' && app.state.view === 'bets' && (p.phase === 'done' || p.done % 25 === 0)) history.render(app);
});
listen('account', () => {
  if (app.state.tab === 'settings') settings.render(app);
});

app.state.settings = await invoke('get_settings');
app.state.live = await invoke('live');
showBackfill(await invoke('backfill_progress'));
showWelcome(app.state.live);
await app.show('live');
// for the headless UI tests
window.augur = Object.assign(app, { showWelcome });
