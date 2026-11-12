// Players: anyone's record, rivals and pals used, and their pal box with reroll advice.
import { invoke } from '../lib/bridge.js';
import { esc, fmt, pct } from '../lib/format.js';

const TAG = { Keep: 'keep', Reroll: 'rr', Maybe: 'rr', Replace: 'no' };
const ivColour = (x) => (x >= 80 ? '#3ddc97' : x >= 50 ? '#ffcf6b' : '#ff5f6d');

function boxPal(p) {
  const ivs = ['HP', 'ATK', 'DEF']
    .map((k) => {
      const iv = p.stats[k]?.[1] ?? 0;
      return `<div class="iv"><span>${k}</span><div class="b"><i style="width:${iv}%;background:${ivColour(iv)}"></i></div><span>${fmt(iv)}</span></div>`;
    })
    .join('');
  return `<div class="pcd" title="Beats a typical opponent ${pct(p.rating)} of the time on stats"><div class="t"><b>${esc(p.pal)}</b><span class="tag ${TAG[p.advice] || 'no'}">${esc(p.advice)}</span></div>${ivs}
    <div class="small">${pct(p.rating)} vs typical${p.rerolls ? ` · rerolled ×${p.rerolls}` : ''}${p.passives.length ? ` · ${esc(p.passives.join(', '))}` : ''}</div></div>`;
}

export async function render(app) {
  const { state, main, header } = app;
  state.player ??= state.settings?.kick_name || 'TrongatesLegacy';
  main.innerHTML =
    header(
      `Players · <b style="color:var(--t1)">${esc(state.player)}</b>`,
      `<form id="pf" class="ml" style="display:flex;gap:6px"><input type="text" id="pn" placeholder="Find a player…" value=""><button class="btn ghost">Go</button></form>`,
    ) + '<div class="panel empty">Loading…</div>';
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
    main.lastElementChild.innerHTML = esc(e);
    return;
  }
  if (state.tab !== 'players') return;
  const d = v.detail;
  const best = d.pals_used.filter(([, , n]) => n >= 3).sort((a, b) => b[1] / b[2] - a[1] / a[2])[0];
  const most = d.pals_used[0];
  main.lastElementChild.outerHTML = `<div class="tiles">
      <div class="tile"><span class="label">Duel record</span><b>${d.wins}–${d.duels - d.wins}</b><small>${d.duels ? pct(d.wins / d.duels) : '—'} · ${d.duels} ranked duels</small></div>
      <div class="tile"><span class="label">Best pal for them</span><b class="ok">${best ? esc(best[0]) : '—'}</b><small>${best ? `${best[1]}–${best[2] - best[1]}` : 'needs 3+ duels with a pal'}</small></div>
      <div class="tile"><span class="label">Most used</span><b>${most ? esc(most[0]) : '—'}</b><small>${most ? `${most[1]}–${most[2] - most[1]}` : ''}</small></div>
      <div class="tile"><span class="label">Pal box</span><b>${v.roster.length}</b><small>${v.rerolled_pals} rerolled</small></div></div>
    <div class="g2"><div class="panel"><div class="ph"><h3>Pal box</h3><span>talents are fixed at hatch; a reroll redraws all three</span></div>
      ${v.roster_error ? `<div class="small bad">Couldn't load the box: ${esc(v.roster_error)}</div>` : ''}
      <div class="box">${v.roster.sort((a, b) => b.rating - a.rating).map(boxPal).join('')}</div>
      <div class="small" style="margin-top:8px">Advice is rough until the reroll cost is set in Settings: Keep strong pals, Reroll good species let down by one roll, Replace weak species no roll can fix.</div></div>
      <div style="display:grid;gap:14px;align-content:start">
        <div class="panel"><div class="ph"><h3>Rivals</h3><span>most met</span></div><table><tbody>${d.rivals.map(([r, w, n]) => `<tr><td>${esc(r)}</td><td class="num ${w / n < 0.45 ? 'bad' : w / n > 0.55 ? 'ok' : ''}">${w}–${n - w}</td></tr>`).join('') || '<tr><td class="muted">No duels yet.</td></tr>'}</tbody></table></div>
        <div class="panel"><div class="ph"><h3>Pals used</h3></div><table><tbody>${d.pals_used.map(([p, w, n]) => `<tr><td>${esc(p)}</td><td class="num">${w}–${n - w}</td></tr>`).join('')}</tbody></table></div>
      </div></div>`;
}
