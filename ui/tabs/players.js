// Players: anyone's record, rivals and pals used, and their pal box (rerolling your own lives in You › Pals).
import { invoke } from '../lib/bridge.js';
import { esc, fmt, pct } from '../lib/format.js';
import { boxCard, boxCols } from '../lib/box.js';
import { face, orb } from '../lib/pal.js';
import { empty, icon, ring, stack } from '../lib/viz.js';

const slugOf = (pal) => pal.toLowerCase().replace(/\s+/g, '-');

// strongest first: the site's power, then how often it beats a typical pal on stats
const byStrength = (a, b) => (b.power ?? -1) - (a.power ?? -1) || b.rating - a.rating;

// a record per rival or pal: name, W–L and the share won as a bar
const records = (rows, pals) =>
  rows
    .map(([name, w, n]) => {
      const r = w / n;
      const tone = r < 0.45 ? 'bad' : r > 0.55 ? 'ok' : '';
      return `<div class="rec">${pals ? orb(name, slugOf(name), 'h av', 24) : face(name, 'a av', 24)}<div class="hbar"><span>${esc(name)}</span><b class="${tone}">${fmt(w)}–${fmt(n - w)}</b><div class="b"><i class="${tone}" style="width:${100 * r}%"></i></div></div></div>`;
    })
    .join('');

export async function render(app) {
  const { state, main, header } = app;
  state.player ??= state.settings?.kick_name || 'TrongatesLegacy';
  main.innerHTML =
    header(
      `Players · <b style="color:var(--t1)">${esc(state.player)}</b>`,
      `<form id="pf" class="ml" style="display:flex;gap:6px"><input type="text" id="pn" placeholder="Find a player…" value="" aria-label="Find a player"><button class="btn ghost">Go</button></form>`,
    ) + `<div class="panel col grow">${empty('eye', 'Loading…')}</div>`;
  document.getElementById('pf').onsubmit = (e) => {
    e.preventDefault();
    const name = document.getElementById('pn').value.trim();
    if (name) {
      state.player = name;
      render(app);
    }
  };
  let v;
  try {
    v = await invoke('player', { name: state.player });
  } catch (e) {
    main.lastElementChild.innerHTML = empty('eye', 'Couldn’t load that player', esc(e));
    return;
  }
  if (state.tab !== 'players') return;
  const d = v.detail;
  const best = d.pals_used.filter(([, , n]) => n >= 3).sort((a, b) => b[1] / b[2] - a[1] / a[2])[0];
  const most = d.pals_used[0];
  const tiers = (t) => v.roster.filter((p) => (p.tier || '').toUpperCase() === t).length;
  const palStat = (label, x, tone, none) =>
    `<div class="pst">${x ? orb(x[0], slugOf(x[0]), 'h av', 40) : `<span class="pst-o">${icon('egg', 18)}</span>`}<div><span class="label">${label}</span><b class="${x ? tone : 'muted'}">${x ? esc(x[0]) : '—'}</b><small>${x ? `${fmt(x[1])}–${fmt(x[2] - x[1])} · ${pct(x[1] / x[2])}` : none}</small></div></div>`;
  main.lastElementChild.outerHTML = `<div class="panel phero">
      ${face(state.player, 'a', 64)}
      <div class="pwho"><b>${esc(d.owner || state.player)}</b><small>${d.duels ? `${fmt(d.duels)} ranked duels` : 'no ranked duels logged yet'}</small></div>
      ${ring(d.duels ? d.wins / d.duels : null, { size: 96, tone: d.duels && d.wins / d.duels < 0.45 ? 'bad' : 'ok', label: d.duels ? `${fmt(d.wins)}–${fmt(d.duels - d.wins)}` : '—', sub: d.duels ? `${pct(d.wins / d.duels)} won` : 'duel record', title: 'Duel record' })}
      ${palStat('Best pal for them', best, 'ok', 'needs 3+ duels with a pal')}
      ${palStat('Most used', most, '', 'no duels yet')}
      <div class="pboxs"><div class="yhead"><b class="bn">${v.roster.length}</b><span>in the pal box${v.rerolled_pals ? ` · ${v.rerolled_pals} rerolled` : ''}${v.roster.length ? ' · by tier' : ''}</span></div>
        ${v.roster.length ? `${stack([[tiers('A'), 'A', 'gold'], [tiers('B'), 'B', 'ok'], [tiers('C'), 'C', 'home'], [tiers('D'), 'D', 'dim']])}` : ''}</div></div>
    <div class="pgrid grow"><div class="panel col"><div class="ph"><h3>Pal box</h3><span>strongest first · power is the site's rating for the species</span></div>
      ${v.roster_error ? `<div class="small bad">Couldn't load the box: ${esc(v.roster_error)}</div>` : ''}
      ${v.roster.length ? `<div class="fillin bxbox" style="--cols:${boxCols(v.roster.length)}">${v.roster.sort(byStrength).map((p) => boxCard(p, { title: `Beats a typical pal ${pct(p.rating)} of the time on stats` })).join('')}</div>` : empty('box', 'No pals in the box', 'Their pal box shows here once it can be read.')}</div>
      <div class="stackc">
        <div class="panel col grow"><div class="ph"><h3>Rivals</h3><span>most met</span></div>${d.rivals.length ? `<div class="fillin bars">${records(d.rivals)}</div>` : empty('swords', 'No duels yet', 'Who they meet most, and how it goes.')}</div>
        <div class="panel col grow"><div class="ph"><h3>Pals used</h3></div>${d.pals_used.length ? `<div class="fillin bars">${records(d.pals_used, true)}</div>` : empty('egg', 'No pals used yet', 'The pals they duel with, and their record on each.')}</div>
      </div></div>`;
}
