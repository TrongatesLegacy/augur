// History: Augur's own calls (today, this week or all time), calibration and the record by stream.
import { invoke } from '../lib/bridge.js';
import { esc, pct } from '../lib/format.js';

const RANGES = [
  ['today', 'Today'],
  ['week', 'This week'],
  ['all', 'All time'],
];

function callRow(c) {
  const right = c.winner === c.pick;
  return `<tr><td><span class="res ${right ? 'o' : 'x'}">${right ? '✓' : '✕'}</span></td><td>${esc(c.home_pal)} <span class="muted">vs</span> ${esc(c.away_pal)}</td><td>${esc(c.pick === 'home' ? c.home_pal : c.away_pal)}</td><td class="num">${pct(c.prob)}</td><td class="${right ? 'ok' : 'bad'}">${right ? 'Right' : 'Missed'}</td><td class="num">${esc(c.ko || '')}</td></tr>`;
}

export async function render(app) {
  const { state, main, header } = app;
  const h = await invoke('history', { range: state.range });
  if (state.tab !== 'history') return;
  const t = h.tally;
  const seg = `<div class="seg ml">${RANGES.map(([k, l]) => `<button class="${state.range === k ? 'on' : ''}" data-r="${k}">${l}</button>`).join('')}</div>`;
  const miss = t.biggest_miss;
  const label = RANGES.find(([k]) => k === state.range)?.[1] || 'All calls';
  main.innerHTML =
    header("History · Augur's own calls", seg) +
    `<div class="tiles">
      <div class="tile hero"><span class="label">${state.range === 'all' ? 'All calls' : label}</span><b>${t.right} / ${t.total}</b><small>${t.total ? pct(t.right / t.total) + ' right' : 'no calls yet'}</small></div>
      <div class="tile"><span class="label">Best run</span><b>${t.best_run}</b><small>right in a row</small></div>
      <div class="tile"><span class="label">Current run</span><b>${t.current_run}</b><small>right in a row now</small></div>
      <div class="tile"><span class="label">Avg confidence</span><b>${t.total ? pct(t.avg_confidence) : '—'}</b><small>vs ${t.total ? pct(t.right / t.total) : '—'} hit rate</small></div>
      <div class="tile"><span class="label">Today's logic</span><b>${h.rescored_right} / ${h.rescored_total}</b><small>re-scored with current settings</small></div>
    </div>
    <div class="g2">
      <div class="panel"><table><thead><tr><th></th><th>Fight</th><th>Called</th><th>Chance</th><th>Result</th><th>K.O.</th></tr></thead><tbody>
        ${h.calls.map(callRow).join('') || '<tr><td colspan="6" class="muted">No calls in this range yet.</td></tr>'}
      </tbody></table></div>
      <div style="display:grid;gap:14px;align-content:start">
        <div class="panel"><div class="ph"><h3>Calibration</h3><span>said vs won</span></div><div class="cal">
          ${t.calibration.map(([band, won, n, promised]) => `<span>${band}</span><div class="cb"><i style="width:${(won / n) * 100}%"></i><s style="left:${promised * 100}%"></s></div><span>${won}/${n}</span>`).join('') || '<span class="muted">—</span>'}
        </div><div class="small" style="margin-top:10px">Bar: how often it won · tick: what it promised. Trustworthy after a few hundred calls.</div></div>
        <div class="panel"><div class="ph"><h3>By stream</h3><span>one bar per day</span></div><div class="days">
          ${t.by_day.slice(-12).map(([d, r, n]) => `<div title="${esc(d)}: ${r}/${n}"><i style="height:${Math.max(4, (r / n) * 80)}px"></i>${esc(d.slice(5))}</div>`).join('') || '<span class="muted">—</span>'}
        </div></div>
        ${miss ? `<div class="panel"><div class="ph"><h3>Biggest miss</h3></div>${esc(miss.pick === 'home' ? miss.home_pal : miss.away_pal)} at ${pct(miss.prob)}, beaten by ${esc(miss.pick === 'home' ? miss.away_pal : miss.home_pal)}</div>` : ''}
      </div>
    </div>`;
  main.querySelectorAll('.seg button').forEach((b) => {
    b.onclick = () => {
      state.range = b.dataset.r;
      render(app);
    };
  });
}
