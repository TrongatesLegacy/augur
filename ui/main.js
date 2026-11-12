// The main window: the sidebar, the shared state the tabs read, and the events from the Rust side.
import { invoke, listen } from './lib/bridge.js';
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
  // a short message at the top of the page, e.g. when saving settings fails
  flash(msg) {
    const d = Object.assign(document.createElement('div'), { className: 'panel bad', textContent: String(msg) });
    app.main.prepend(d);
    setTimeout(() => d.remove(), 5000);
  },
};

document.querySelectorAll('.nav').forEach((n) => (n.onclick = () => app.show(n.dataset.tab)));
listen('live', (v) => {
  app.state.live = v;
  if (app.state.tab === 'live') live.render(app);
});
listen('navigate', (tab) => app.show(tab));
listen('bets', (p) => {
  // redraw Your bets when an import finishes or moves on a page
  if (app.state.tab === 'history' && app.state.view === 'bets' && (p.phase === 'done' || p.done % 25 === 0)) history.render(app);
});
listen('account', () => {
  if (app.state.tab === 'settings') settings.render(app);
});

app.state.settings = await invoke('get_settings');
app.state.live = await invoke('live');
await app.show('live');
// for the headless UI tests
window.augur = app;
