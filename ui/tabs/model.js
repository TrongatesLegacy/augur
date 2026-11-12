// Model: the settings and what they were fitted on, a suggested change, the species with secret moves, data health.
import { invoke } from '../lib/bridge.js';
import { esc, fmt, pct } from '../lib/format.js';

const slider = (label, v, lo, hi, shown) =>
  `<div class="set"><span>${label}</span><div class="sl"><i style="left:${((v - lo) / (hi - lo)) * 100}%"></i></div><span class="v">${shown}</span></div>`;

function backfillLine(b) {
  if (!b.phase) return 'waiting';
  if (b.phase === 'done') return 'up to date';
  return `${esc(b.phase)} ${fmt(b.done)}/${fmt(b.total)}${b.paused ? ' (paused: duel up)' : ''}`;
}

export async function render(app) {
  const { state, main, header } = app;
  main.innerHTML = header('Model') + '<div class="panel empty">Re-testing on every logged duel…</div>';
  const m = await invoke('model_view');
  if (state.tab !== 'model') return;
  const s = m.settings;
  const b = m.backfill || {};
  const sug = m.suggestion;
  const fit = m.calibration ? m.calibration[3].toFixed(2) : null;
  const suggestion = sug
    ? `<div class="sug"><div class="label gold">Suggested by the re-test</div><div class="cond" style="font-size:20px;font-weight:700;margin:8px 0 4px">Type edge ${s.TYPE_BONUS}× → ${sug[2].TYPE_BONUS}×</div><div class="small" style="color:var(--t2)">Live calls ${m.live_right} → ${sug[1]} right of ${m.live_total}, log loss ${m.live_loss.toFixed(3)} → ${sug[0].toFixed(3)}. Scored on the same calls it was fitted to, so small gains may be chance.</div><div style="display:flex;gap:8px;margin-top:10px"><button class="btn ghost" id="keep">Keep ${s.TYPE_BONUS}×</button><button class="btn" id="try">Try ${sug[2].TYPE_BONUS}×</button></div></div>`
    : `<div class="panel"><div class="ph"><h3>Re-test</h3></div><div class="small" style="color:var(--t2)">No setting change does clearly better on the ${m.live_total} real calls. Augur keeps the current settings.</div></div>`;
  main.innerHTML =
    header('Model · settings, learning and data', `<button class="btn ghost ml" id="rt">Re-test now</button>`) +
    `<div class="tiles">
      <div class="tile"><span class="label">Live calls</span><b>${m.calls_right} / ${m.live_total}</b><small>today's logic would get ${m.live_right}</small></div>
      <div class="tile"><span class="label">History test</span><b>${m.backtest ? (m.backtest.accuracy * 100).toFixed(1) + '%' : '—'}</b><small>${m.backtest ? fmt(m.backtest.duels) : 0} past duels, base stats only</small></div>
      <div class="tile"><span class="label">Per-hit fit</span><b>${fit ? fit + '×' : '—'}</b><small>real value of a type edge (model ${s.TYPE_BONUS}×)</small></div>
      <div class="tile"><span class="label">Data</span><b>${fmt(m.hits)}</b><small>hits from ${fmt(m.tapes)} fight tapes</small></div></div>
    <div class="g3">
      <div class="panel"><div class="ph"><h3>Settings</h3><span>fitted on logged fights</span></div>
        ${slider('Type edge bonus', s.TYPE_BONUS, 1, 2.5, s.TYPE_BONUS + '×')}${slider('Resisted moves', s.TYPE_RESIST, 1, 2, '÷' + s.TYPE_RESIST)}${slider('Move power weight', s.POWER_EXP, 0, 1, '^' + s.POWER_EXP)}
        ${slider('Stats → chance', s.STATS_STEEPNESS, 1, 5, s.STATS_STEEPNESS)}${slider('Species records', s.SPECIES_WEIGHT, 0, 2, s.SPECIES_WEIGHT)}${slider('Pal head to head', s.PAL_H2H_WEIGHT, 0, 2, s.PAL_H2H_WEIGHT)}</div>
      <div style="display:grid;gap:14px;align-content:start">${suggestion}
        <div class="panel"><div class="ph"><h3>Pals with secret moves</h3><span>share of damage</span></div><table><tbody>${m.unlisted.map((u) => `<tr><td>${esc(u.pal)}<div class="small">${u.moves.map((x) => esc(x[0])).join(', ')}</div></td><td class="num gold">${pct(u.share)}</td></tr>`).join('')}</tbody></table></div></div>
      <div style="display:grid;gap:14px;align-content:start">
        <div class="panel"><div class="ph"><h3>Data health</h3></div>
          <div class="hb"><span class="small">Live feed</span><span>${m.synced_at ? `synced ${Math.max(0, Math.round((Date.now() - m.synced_at) / 1000))}s ago` : 'not yet'}</span><span class="${m.synced_at ? 'ok' : 'muted'}">●</span></div>
          <div class="hb"><span class="small">Past duels</span><span>${fmt(m.duels)}</span><span class="${m.duels ? 'ok' : 'muted'}">●</span></div>
          <div class="hb"><span class="small">Species</span><span>${fmt(m.species)}</span><span class="${m.species ? 'ok' : 'muted'}">●</span></div>
          <div class="hb"><span class="small">Backfill</span><span>${backfillLine(b)}</span><span class="${b.last_error ? 'bad' : 'ok'}">●</span></div>
          ${b.phase && b.phase !== 'done' && b.total ? `<div class="progress"><i style="width:${(b.done / b.total) * 100}%"></i></div>` : ''}
          ${b.last_error ? `<div class="small bad">${esc(b.last_error)}</div>` : ''}</div>
        <div class="panel"><div class="ph"><h3>Open questions</h3></div><div class="small" style="color:var(--t2);font-size:12.5px;line-height:1.55">• Is a type edge worth anything? The per-hit fit says ${fit ?? '?'}×.<br>• Do cooldown passives mean more attacks? Counted at half strength.<br>• Owners matter but aren't in the model yet.</div></div></div>
    </div>`;
  document.getElementById('rt').onclick = () => render(app);
  if (sug) {
    document.getElementById('try').onclick = async () => {
      state.settings.model = sug[2];
      await invoke('save_settings', { settings: state.settings });
      render(app);
    };
    document.getElementById('keep').onclick = (e) => (e.target.closest('.sug').style.display = 'none');
  }
}
