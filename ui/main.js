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
await app.show('live');
// for the headless UI tests
window.augur = app;
