// The tray popover: only what's useful at a glance, and the copy buttons (the bet and the chat line). Between duels
// and off air it says how the last stream went. It sizes itself to what it shows.
import { invoke, listen } from './lib/bridge.js';
import { copyText } from './lib/clipboard.js';
import { esc, fmt, pct, signed } from './lib/format.js';
import { SHORT, dial, face, pickVoice, tapeRows, voiceLine, whyLine } from './lib/pal.js';
import { betCommand, stakeLine } from './lib/stake.js';

const el = document.getElementById('pop');
const state = { view: null, settings: null, voice: null, update: null };

const shortDay = (d) => new Date(d + 'T12:00:00Z').toLocaleDateString(undefined, { day: 'numeric', month: 'short' });
// today's date as the calls are dated (YYYY-MM-DD, local)
const todayIso = () => new Date(Date.now() - new Date().getTimezoneOffset() * 60000).toISOString().slice(0, 10);

// the next duel: each owner's face and name on their side, a long name trimmed with "…" (the full one on hover)
function nextUp(queue) {
  const [q, ...rest] = queue;
  const side = (name, cls) => `<span class="nu-s ${cls}" title="${esc(name)}">${face(name, `${cls === 'h' ? 'h' : 'a'} av`, 24)}<span>${esc(name)}</span></span>`;
  return `<div class="nu"><div class="nu-t"><b>Next up</b>${rest.length ? `<small>+${rest.length} queued</small>` : ''}</div>
    <div class="nu-m">${side(q.home, 'h')}<span class="nu-v">vs</span>${side(q.away, 'a')}</div></div>`;
}

// no duel up: between duels (something's queued) or off air, and how the last stream went
function quiet(view) {
  const o = view?.off_air;
  const between = (view?.queue || []).length > 0;
  const head = view?.error ? 'OFFLINE' : between ? 'BETWEEN DUELS' : 'OFF AIR';
  const hero = view?.error
    ? '<b>Can’t reach PalDuels</b><small>Augur keeps trying every 30 seconds.</small>'
    : between
      ? nextUp(view.queue)
      : '<b>See you next stream</b><small>Augur calls the next fight the moment it’s up.</small>';
  const ls = o?.last_stream;
  const stream = ls
    ? `<div class="qs"><div class="qh"><span>${ls[0] === todayIso() ? 'Today' : 'Last stream'} · ${esc(shortDay(ls[0]))}</span><span>best run ${fmt(ls[3])}</span></div>
        <div class="qn"><b>${fmt(ls[1])}<i>/${fmt(ls[2])}</i></b><span>${pct(ls[1] / ls[2])} right</span></div>
        ${o.last_bets ? `<div class="qb"><span>You <b class="${o.last_bets[0] >= 0 ? 'ok' : 'bad'}">${signed(o.last_bets[0])}</b></span><span>Following Augur <b class="${o.last_bets[1] >= 0 ? 'ok' : 'bad'}">${signed(o.last_bets[1])}</b></span></div>` : ''}</div>`
    : '';
  const recent = o?.recent?.length
    ? `<div class="qr">${o.recent.slice(0, 3).map(([pick, other, right]) => `<div><span class="res ${right ? 'o' : 'x'}">${right ? '✓' : '✕'}</span><span><b>${esc(pick)}</b> over ${esc(other)}</span></div>`).join('')}</div>`
    : '';
  const all = o?.all_time?.[1] ? `<div class="qa">All time <b>${fmt(o.all_time[0])}/${fmt(o.all_time[1])}</b> · ${pct(o.all_time[0] / o.all_time[1])}</div>` : '';
  return `<div class="hd"><span>${head}</span><span class="c">AUGUR</span></div><div class="quiet">${hero}</div>${stream}${recent}${all}`;
}

// the popover's window fits what it shows
function fit() {
  // the card, plus the clear margin around it
  const pad = parseFloat(getComputedStyle(document.body).paddingTop) + parseFloat(getComputedStyle(document.body).paddingBottom);
  requestAnimationFrame(() => invoke('fit_popover', { height: Math.ceil(el.getBoundingClientRect().height + pad) }).catch(() => {}));
}

function render() {
  const { view, settings } = state;
  const p = view?.prediction;
  const today = view?.today || [0, 0];
  const u = state.update;
  const upd = u?.latest && u.ready ? `<div class="tup"><span>✦ Augur <b>${esc(u.latest.version)}</b> is ready</span><button id="tu">${u.installing ? 'Restarting…' : 'Restart to update'}</button></div>` : '';
  const foot = upd + `<div class="ft"><span>Today <b>${fmt(today[0])}/${fmt(today[1])}</b>${today[1] ? ` · ${pct(today[0] / today[1])}` : ''}</span><span class="fb"><button id="open">Open Augur ›</button><button id="quit" class="qt" title="Quit Augur" aria-label="Quit Augur"><svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"><path d="M12 3v8"/><path d="M6.3 6.8a8 8 0 1 0 11.4 0"/></svg></button></span></div>`;
  if (!p) {
    el.innerHTML = quiet(view) + foot;
  } else {
    const fav = p.pick === 'home' ? p.home : p.away;
    const dog = p.pick === 'home' ? p.away : p.home;
    const { voices, voice } = pickVoice(state.voice, settings);
    state.voice = voice;
    const command = betCommand(p, view.stake);
    const bet = view.stake ? stakeLine(view.stake) : `<div class="bet"><div><b>${esc(command)}</b></div></div>`;
    el.innerHTML = `<div class="hd"><span><i></i>${view.live ? 'FIGHT ON' : 'PICKS OPEN'}</span><span class="c">${p.home.estimated || p.away.estimated ? 'STATS AT THE BELL' : esc(p.tale.verdict)}</span></div>
      <div class="call"><div><div class="who">${esc(fav.pal)}</div><div class="vs">over <b>${esc(dog.pal)}</b></div><div class="small">${esc(p.home.owner)} vs ${esc(p.away.owner)}</div><div class="why">${esc(whyLine(p))}</div></div>${dial(p.prob, 84)}</div>
      <div class="stats">${tapeRows(p)}</div>
      <div class="betrow">${bet}${command ? '<button class="btn ghost" id="cb">Copy bet</button>' : ''}</div>
      ${settings?.chat_lines === false ? '' : `<div class="say"><div class="vo" id="vb">${voices.map((v) => `<button class="${v === voice ? 'on' : ''}" data-v="${esc(v)}">${esc(SHORT[v] || v)}</button>`).join('')}</div>
      <div class="q" id="vq">${esc(voiceLine(p, voice, settings?.include_chance !== false))}</div><button class="btn copy" id="cp">Copy for chat</button></div>`}${foot}`;
    el.querySelectorAll('#vb button').forEach((b) => {
      b.onclick = () => {
        state.voice = b.dataset.v;
        render();
      };
    });
    document.getElementById('cp')?.addEventListener('click', (e) => copyText(document.getElementById('vq').textContent, e.target));
    document.getElementById('cb')?.addEventListener('click', (e) => copyText(command, e.target));
  }
  document.getElementById('open').onclick = () => invoke('open_window', { tab: 'live' });
  document.getElementById('tu')?.addEventListener('click', (e) => {
    e.target.disabled = true;
    invoke('update_install');
  });
  document.getElementById('quit').onclick = () => invoke('quit_app');
  fit();
}

addEventListener('keydown', (e) => {
  if (e.key === 'Escape') invoke('hide_popover');
});
listen('update', (u) => {
  state.update = u;
  render();
});
listen('settings', (v) => {
  state.settings = v;
  render();
});
listen('live', (v) => {
  state.view = v;
  render();
});
state.settings = await invoke('get_settings');
state.view = await invoke('live');
state.update = await invoke('update_view');
render();
// for the headless UI tests
window.augur = { rendered: true };
