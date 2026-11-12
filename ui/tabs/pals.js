// Pals: every species with duels, and one species in detail: record, moves as they really hit, owners, matchups.
import { invoke } from '../lib/bridge.js';
import { esc, fmt, pct } from '../lib/format.js';
import { orb } from '../lib/pal.js';
import { columns, empty, ring } from '../lib/viz.js';

async function detail(app) {
  const { state } = app;
  if (!state.pal) return;
  const d = await invoke('species', { pal: state.pal });
  const sp = d.species || {};
  const el = document.getElementById('pd');
  if (!el) return;
  const top = Math.max(...d.moves.map((m) => m.avg), 1);
  const moves = d.moves.length
    ? `<div class="fillin"><table class="sticky"><thead><tr><th>Move</th><th>Element</th><th>Listed</th><th>Hits</th><th>Avg</th><th>Best</th></tr></thead><tbody>${d.moves
        .slice(0, 30)
        .map(
          (m) =>
            `<tr><td>${esc(m.name)} ${m.unlisted ? '<span class="tag u">unlisted</span>' : ''}</td><td>${esc(m.element || '—')}</td><td class="num">${m.listed_power ?? '?'}</td><td class="num">${m.hits}</td><td class="num"><div class="avg"><span>${m.avg.toFixed(1)}%</span><i style="width:${(100 * m.avg) / top}%"></i></div></td><td class="num">${fmt(m.best)}%</td></tr>`,
        )
        .join('')}</tbody></table></div>`
    : empty('chart', 'No tapes yet', 'The background backfill fills these in.');
  const rate = d.duels ? d.wins / d.duels : null;
  const stat = (label, v) => `<div class="bst"><span>${label}</span><b>${v ?? '—'}</b></div>`;
  el.innerHTML = `<div class="panel hero">${orb(state.pal, sp.slug, 'h', 92)}
      <div><div class="cond" style="font-weight:800;font-size:34px;line-height:1">${esc(state.pal)}</div>
        <div style="display:flex;gap:6px;margin:8px 0">${(sp.types || []).map((t) => `<span class="el">${esc(t)}</span>`).join('')}${sp.power != null ? `<span class="el">Power ${sp.power}</span>` : ''}</div>
        <div class="bsts">${stat('Base HP', sp.base_hp)}${stat('Attack', sp.base_atk)}${stat('Defence', sp.base_def)}</div></div>
      <div class="kpi">
        <div>${ring(rate, { size: 70, tone: rate != null && rate < 0.45 ? 'bad' : 'ok', label: `${d.wins}–${d.duels - d.wins}`, sub: 'record', title: rate != null ? `${pct(rate)} won` : '' })}</div>
        <div>${ring(d.tapes ? d.unlisted_share : null, { size: 70, sub: 'unlisted', title: 'Share of damage from unlisted moves' })}</div>
        <div class="kv"><b>${esc(d.fastest_ko || '—')}</b><small>Fastest K.O.</small></div>
        <div class="kv"><b>${d.tapes}</b><small>Tapes</small></div></div></div>
    <div class="pdg grow">
      <div class="panel col pmoves"><div class="ph"><h3>Moves as they really hit</h3><span>from ${d.tapes} fight tapes</span></div>${moves}</div>
      <div class="panel col"><div class="ph"><h3>Record by month</h3><span>share won</span></div>${
        d.months.length
          ? columns(d.months.map(([m, w, n]) => ({ v: w / n, label: m.slice(5), top: `${w}/${n}`, title: `${m}: ${w}/${n} won`, tone: w / n >= 0.5 ? 'ok' : 'bad' })), 0.5)
          : empty('chart', 'No duels yet')
      }</div>
      <div class="panel col"><div class="ph"><h3>Matchups</h3><span>the species it has met most</span></div>
        ${d.matchups.length ? `<div class="mups fillin">${d.matchups.map(([o, w, n]) => `<button class="mup" data-pal="${esc(o)}"><span>${esc(o)}</span><div class="mb"><i style="width:${(100 * w) / n}%"></i></div><b class="${w / n >= 0.6 ? 'ok' : w / n <= 0.4 ? 'bad' : ''}">${w}–${n - w}</b></button>`).join('')}</div>` : empty('swords', 'No matchups yet', 'No duels against other species yet.')}</div>
      <div class="panel col"><div class="ph"><h3>Who wins with it</h3><span>owners</span></div>${
        d.owners.length
          ? `<div class="fillin bars">${d.owners
              .map(([o, w, n]) => {
                const tone = w / n >= 0.6 ? 'ok' : w / n <= 0.4 ? 'bad' : '';
                return `<div class="hbar"><span>${esc(o)}</span><b class="${tone}">${w}–${n - w}</b><div class="b"><i class="${tone}" style="width:${(100 * w) / n}%"></i></div></div>`;
              })
              .join('')}</div>`
          : empty('eye', 'No owners yet')
      }</div>
    </div>`;
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
    `<div class="pgr grow">
    <div class="panel col"><input type="text" id="ps" placeholder="Search pals…" aria-label="Search pals" style="width:100%;margin-bottom:8px"><div id="pl" class="plist fillin"></div></div><div id="pd" class="stackc"></div></div>`;
  const list = (q) => {
    const rows = all.filter((s) => s.pal.toLowerCase().includes(q.toLowerCase())).slice(0, 200);
    document.getElementById('pl').innerHTML = rows.length ? rows
      .map((s) => `<div class="it ${s.pal === state.pal ? 'on' : ''}" data-p="${esc(s.pal)}">${orb(s.pal, s.slug, 'h av', 28)}<span>${esc(s.pal)}</span><span class="num ${s.wins / s.duels >= 0.55 ? 'ok' : ''}">${s.wins}–${s.duels - s.wins}</span></div>`)
      .join('') : empty('egg', all.length ? 'No match' : 'No pals yet', all.length ? 'Try another name.' : 'Species show here once their duels are fetched.');
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
