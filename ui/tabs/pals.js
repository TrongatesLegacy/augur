// Pals: every species with duels, and one species in detail: record, moves as they really hit, owners, matchups.
import { invoke } from '../lib/bridge.js';
import { esc, fmt, pct } from '../lib/format.js';
import { orb } from '../lib/pal.js';

async function detail(app) {
  const { state } = app;
  if (!state.pal) return;
  const d = await invoke('species', { pal: state.pal });
  const sp = d.species || {};
  const el = document.getElementById('pd');
  if (!el) return;
  const moves =
    d.moves
      .slice(0, 10)
      .map((m) => `<tr><td>${esc(m.name)} ${m.unlisted ? '<span class="tag u">unlisted</span>' : ''}</td><td>${esc(m.element || '—')}</td><td class="num">${m.listed_power ?? '?'}</td><td class="num">${m.hits}</td><td class="num">${m.avg.toFixed(1)}%</td><td class="num">${fmt(m.best)}%</td></tr>`)
      .join('') || '<tr><td colspan="6" class="muted">No tapes yet: the background backfill fills these in.</td></tr>';
  el.innerHTML = `<div style="display:grid;gap:14px">
    <div class="panel hero">${orb(state.pal, sp.slug, 'h', 104)}
      <div><div class="cond" style="font-weight:800;font-size:34px;line-height:1">${esc(state.pal)}</div>
        <div style="display:flex;gap:6px;margin:8px 0">${(sp.types || []).map((t) => `<span class="el">${esc(t)}</span>`).join('')}${sp.power != null ? `<span class="el">Power ${sp.power}</span>` : ''}</div>
        <div class="small">Base HP ${sp.base_hp ?? '—'} · Attack ${sp.base_atk ?? '—'} · Defence ${sp.base_def ?? '—'}</div></div>
      <div class="kpi"><div><b>${d.wins}–${d.duels - d.wins}</b><small>Record</small></div><div><b class="gold">${pct(d.unlisted_share)}</b><small>Unlisted dmg</small></div><div><b>${esc(d.fastest_ko || '—')}</b><small>Fastest K.O.</small></div><div><b>${d.tapes}</b><small>Tapes</small></div></div></div>
    <div class="g2"><div class="panel"><div class="ph"><h3>Moves as they really hit</h3><span>from ${d.tapes} fight tapes</span></div>
      <table><thead><tr><th>Move</th><th>Element</th><th>Listed</th><th>Hits</th><th>Avg</th><th>Best</th></tr></thead><tbody>${moves}</tbody></table></div>
      <div style="display:grid;gap:14px;align-content:start">
        <div class="panel"><div class="ph"><h3>Record by month</h3></div><div class="days">${d.months.map(([m, w, n]) => `<div title="${w}/${n}"><i style="height:${Math.max(4, (w / n) * 80)}px"></i>${esc(m.slice(5))} ${w}/${n}</div>`).join('')}</div></div>
        <div class="panel"><div class="ph"><h3>Who wins with it</h3></div><table><tbody>${d.owners.map(([o, w, n]) => `<tr><td>${esc(o)}</td><td class="num ${w / n >= 0.6 ? 'ok' : ''}">${w}–${n - w}</td></tr>`).join('')}</tbody></table></div>
      </div></div>
    <div class="panel"><div class="ph"><h3>Matchups</h3><span>the species it has met most</span></div>
      ${d.matchups.length ? `<div class="mups">${d.matchups.map(([o, w, n]) => `<button class="mup" data-pal="${esc(o)}"><span>${esc(o)}</span><div class="mb"><i style="width:${(100 * w) / n}%"></i></div><b class="${w / n >= 0.6 ? 'ok' : w / n <= 0.4 ? 'bad' : ''}">${w}–${n - w}</b></button>`).join('')}</div>` : '<div class="small">No duels against other species yet.</div>'}</div></div>`;
  // a matchup opens that species
  el.querySelectorAll('.mup').forEach((b) => {
    b.onclick = () => {
      state.pal = b.dataset.pal;
      document.querySelectorAll('#pl .it').forEach((it) => it.classList.toggle('on', it.dataset.p === state.pal));
      detail(app);
    };
  });
}

export async function render(app) {
  const { state, main, header } = app;
  state.speciesList ??= await invoke('species_list');
  if (state.tab !== 'pals') return;
  const all = state.speciesList;
  state.pal ??= all[0]?.pal;
  main.innerHTML =
    header(`Pals · ${all.length} species with duels`) +
    `<div style="display:grid;grid-template-columns:270px 1fr;gap:14px;align-items:start">
    <div class="panel plist"><input type="text" id="ps" placeholder="Search pals…" style="width:100%;margin-bottom:8px"><div id="pl"></div></div><div id="pd"></div></div>`;
  const list = (q) => {
    const rows = all.filter((s) => s.pal.toLowerCase().includes(q.toLowerCase())).slice(0, 200);
    document.getElementById('pl').innerHTML = rows
      .map((s) => `<div class="it ${s.pal === state.pal ? 'on' : ''}" data-p="${esc(s.pal)}">${orb(s.pal, s.slug, 'h av', 28)}<span>${esc(s.pal)}</span><span class="num ${s.wins / s.duels >= 0.55 ? 'ok' : ''}">${s.wins}–${s.duels - s.wins}</span></div>`)
      .join('');
    document.querySelectorAll('#pl .it').forEach((it) => {
      it.onclick = () => {
        state.pal = it.dataset.p;
        list(document.getElementById('ps').value);
        detail(app);
      };
    });
  };
  document.getElementById('ps').oninput = (e) => list(e.target.value);
  list('');
  detail(app);
}
