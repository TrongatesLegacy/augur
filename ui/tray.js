// The tray popover: only what's useful at a glance, and the copy button.
import { invoke, listen } from './lib/bridge.js';
import { copyText } from './lib/clipboard.js';
import { esc, pct } from './lib/format.js';
import { SHORT, dial, pickVoice, tapeRows, voiceLine, whyLine } from './lib/pal.js';

const el = document.getElementById('pop');
const state = { view: null, settings: null, voice: null };

function render() {
  const { view, settings } = state;
  const p = view?.prediction;
  const today = view?.today || [0, 0];
  const foot = `<div class="ft"><span>Today <b>${today[0]}/${today[1]}</b>${today[1] ? ` · ${pct(today[0] / today[1])}` : ''}</span><button id="open">Open Augur ›</button></div>`;
  if (!p) {
    const r = view?.last_result;
    el.innerHTML = `<div class="hd"><span>${view?.error ? 'OFFLINE' : 'NO DUEL UP'}</span><span class="c">AUGUR</span></div>
      <div class="empty" style="padding:30px 14px"><b>${view?.error ? 'Can’t reach PalDuels' : 'Waiting for the next fight'}</b>
      ${r ? `Last: <span class="${r.right ? 'ok' : 'bad'}">${esc(r.winner_pal)} won ${r.right ? '✓' : '✕'}</span>${r.ko ? ` · K.O. ${esc(r.ko)}` : ''}` : esc(view?.title || '')}</div>${foot}`;
  } else {
    const fav = p.pick === 'home' ? p.home : p.away;
    const dog = p.pick === 'home' ? p.away : p.home;
    const { voices, voice } = pickVoice(state.voice, settings);
    state.voice = voice;
    el.innerHTML = `<div class="hd"><span><i></i>${view.live ? 'FIGHT ON' : 'PICKS OPEN'}</span><span class="c">${p.home.estimated || p.away.estimated ? 'STATS AT THE BELL' : esc(p.tale.verdict)}</span></div>
      <div class="call"><div><div class="who">${esc(fav.pal)}</div><div class="vs">over <b>${esc(dog.pal)}</b></div><div class="small">${esc(p.home.owner)} vs ${esc(p.away.owner)}</div><div class="why">${esc(whyLine(p))}</div></div>${dial(p.prob, 84)}</div>
      <div class="stats">${tapeRows(p)}</div>
      <div class="say"><div class="vo" id="vb">${voices.map((v) => `<button class="${v === voice ? 'on' : ''}" data-v="${esc(v)}">${esc(SHORT[v] || v)}</button>`).join('')}</div>
      <div class="q" id="vq">${esc(voiceLine(p, voice, settings?.include_chance !== false))}</div><button class="btn copy" id="cp">Copy for chat</button></div>${foot}`;
    el.querySelectorAll('#vb button').forEach((b) => {
      b.onclick = () => {
        state.voice = b.dataset.v;
        render();
      };
    });
    document.getElementById('cp').onclick = (e) => copyText(document.getElementById('vq').textContent, e.target);
  }
  document.getElementById('open').onclick = () => invoke('open_window', { tab: 'live' });
}

addEventListener('keydown', (e) => {
  if (e.key === 'Escape') invoke('hide_popover');
});
listen('live', (v) => {
  state.view = v;
  render();
});
state.settings = await invoke('get_settings');
state.view = await invoke('live');
render();
// for the headless UI tests
window.augur = { rendered: true };
