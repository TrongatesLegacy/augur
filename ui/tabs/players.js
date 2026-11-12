// Players: anyone's record, rivals and pals used, and their pal box with reroll advice.
import { invoke } from '../lib/bridge.js';
import { esc, fmt, pct } from '../lib/format.js';
import { face, orb, passivePills } from '../lib/pal.js';
import { empty, icon, ring, stack } from '../lib/viz.js';

const TAG = { Keep: 'keep', Reroll: 'rr', Maybe: 'rr', Replace: 'no' };
const slugOf = (pal) => pal.toLowerCase().replace(/\s+/g, '-');
const ivColour = (x) => (x >= 80 ? '#3ddc97' : x >= 50 ? '#ffcf6b' : '#ff5f6d');

function boxPal(p) {
  const ivs = ['HP', 'ATK', 'DEF']
    .map((k) => {
      const iv = p.stats[k]?.[1] ?? 0;
      return `<div class="iv"><span>${k}<b>${fmt(iv)}</b></span><div class="b"><i style="width:${iv}%;background:${ivColour(iv)}"></i></div></div>`;
    })
    .join('');
  // name and rating, the three rolls side by side, then the advice and talents
  return `<div class="pcd" title="Beats a typical opponent ${pct(p.rating)} of the time on stats"><div class="t">${orb(p.pal, p.slug, 'h av', 22)}<b>${esc(p.pal)}</b><b class="cond rt">${pct(p.rating)}</b></div>
    <div class="ivs">${ivs}</div>
    <div class="pft"><span class="tag ${TAG[p.advice] || 'no'}">${esc(p.advice)}</span>${passivePills(p.passives)}${p.rerolls ? `<span class="pill">rerolled ×${p.rerolls}</span>` : ''}</div></div>`;
}

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
  const advice = (k) => v.roster.filter((p) => p.advice === k).length;
  const palStat = (label, x, tone, none) =>
    `<div class="pst">${x ? orb(x[0], slugOf(x[0]), 'h av', 40) : `<span class="pst-o">${icon('egg', 18)}</span>`}<div><span class="label">${label}</span><b class="${x ? tone : 'muted'}">${x ? esc(x[0]) : '—'}</b><small>${x ? `${fmt(x[1])}–${fmt(x[2] - x[1])} · ${pct(x[1] / x[2])}` : none}</small></div></div>`;
  main.lastElementChild.outerHTML = `<div class="panel phero">
      ${face(state.player, 'a', 64)}
      <div class="pwho"><b>${esc(d.owner || state.player)}</b><small>${d.duels ? `${fmt(d.duels)} ranked duels` : 'no ranked duels logged yet'}</small></div>
      ${ring(d.duels ? d.wins / d.duels : null, { size: 96, tone: d.duels && d.wins / d.duels < 0.45 ? 'bad' : 'ok', label: d.duels ? `${fmt(d.wins)}–${fmt(d.duels - d.wins)}` : '—', sub: d.duels ? `${pct(d.wins / d.duels)} won` : 'duel record', title: 'Duel record' })}
      ${palStat('Best pal for them', best, 'ok', 'needs 3+ duels with a pal')}
      ${palStat('Most used', most, '', 'no duels yet')}
      <div class="pboxs"><div class="yhead"><b class="bn">${v.roster.length}</b><span>in the pal box · ${v.rerolled_pals} rerolled</span></div>
        ${v.roster.length ? stack([[advice('Keep'), 'keep', 'ok'], [advice('Reroll') + advice('Maybe'), 'reroll / maybe', 'gold'], [advice('Replace'), 'replace', 'dim']]) : ''}</div></div>
    <div class="pgrid grow"><div class="panel col"><div class="ph"><h3>Pal box</h3><span>talents are fixed at hatch; a reroll redraws all three</span></div>
      ${v.roster_error ? `<div class="small bad">Couldn't load the box: ${esc(v.roster_error)}</div>` : ''}
      ${v.roster.length ? `<div class="pboxw"><div class="box">${v.roster.sort((a, b) => b.rating - a.rating).map(boxPal).join('')}</div>
      <div class="pkey" title="Rough until the reroll cost is set in Settings"><span><i class="tag keep">Keep</i>strong pals</span><span><i class="tag rr">Reroll</i>a good species let down by one roll</span><span><i class="tag no">Replace</i>a weak species no roll can fix</span></div></div>` : empty('box', 'No pals in the box', 'Their pal box shows here once it can be read.')}</div>
      <div class="stackc">
        <div class="panel col grow"><div class="ph"><h3>Rivals</h3><span>most met</span></div>${d.rivals.length ? `<div class="fillin bars">${records(d.rivals)}</div>` : empty('swords', 'No duels yet', 'Who they meet most, and how it goes.')}</div>
        <div class="panel col grow"><div class="ph"><h3>Pals used</h3></div>${d.pals_used.length ? `<div class="fillin bars">${records(d.pals_used, true)}</div>` : empty('egg', 'No pals used yet', 'The pals they duel with, and their record on each.')}</div>
      </div></div>`;
}
