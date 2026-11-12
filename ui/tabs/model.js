// Model: the settings and what they were fitted on, a suggested change, the species with secret moves, data health.
import { backfillState } from '../lib/backfill.js';
import { invoke } from '../lib/bridge.js';
import { ago, esc, fmt, pct } from '../lib/format.js';
import { empty, icon, ring } from '../lib/viz.js';

// a setting as a position on its range, with the range's ends under it
const slider = (label, v, lo, hi, shown) =>
  `<div class="set"><span>${label}</span><div class="slw"><div class="sl"><b style="width:${((v - lo) / (hi - lo)) * 100}%"></b><i style="left:${((v - lo) / (hi - lo)) * 100}%"></i></div><div class="se"><span>${lo}</span><span>${hi}</span></div></div><span class="v">${shown}</span></div>`;

// a headline number with its picture on the left
const kpi = (pic, label, value, sub) => `<div class="panel kp">${pic}<div><span class="label">${label}</span><b>${value}</b><small>${sub}</small></div></div>`;

function backfillLine(b) {
  if (b.phase === 'setup') return 'waiting for setup';
  if (!b.phase) return 'waiting';
  if (b.phase === 'done') return 'up to date';
  return esc(backfillState(b)?.long || `${b.phase} ${fmt(b.done)}/${fmt(b.total)}`);
}

export async function render(app) {
  const { state, main, header } = app;
  main.innerHTML = header('Model') + `<div class="panel col grow">${empty('chart', 'Re-testing…', 'on every logged duel')}</div>`;
  const m = await invoke('model_view');
  if (state.tab !== 'model') return;
  const s = m.settings;
  const b = m.backfill || {};
  const sug = m.suggestion;
  const fit = m.calibration ? m.calibration[3].toFixed(2) : null;
  const suggestion = sug
    ? `<div class="sug"><div class="label gold">Suggested by the re-test</div><div class="cond" style="font-size:20px;font-weight:700;margin:8px 0 4px">Type edge ${s.TYPE_BONUS}× → ${sug[2].TYPE_BONUS}×</div><div class="small" style="color:var(--t2)">Live calls ${m.live_right} → ${sug[1]} right of ${m.live_total}, log loss ${m.live_loss.toFixed(3)} → ${sug[0].toFixed(3)}. Scored on the same calls it was fitted to, so small gains may be chance.</div><div style="display:flex;gap:8px;margin-top:10px"><button class="btn ghost" id="keep">Keep ${s.TYPE_BONUS}×</button><button class="btn" id="try">Try ${sug[2].TYPE_BONUS}×</button></div></div>`
    : `<div class="panel"><div class="ph"><h3>Re-test</h3><span class="ok">✓ settings hold</span></div><div class="small" style="color:var(--t2)">No setting change does clearly better on the ${m.live_total} real calls. Augur keeps the current settings.</div></div>`;
  const bt = m.backtest;
  // the per-hit fit and the model's own value on one scale (1× to 2.5×)
  const at = (x) => `${Math.max(0, Math.min(100, ((x - 1) / 1.5) * 100))}%`;
  const fitScale = `<div class="fitg" role="img" aria-label="Per-hit fit ${fit ?? 'not known'}, model ${s.TYPE_BONUS}"><div class="ft"></div>${fit ? `<i class="fit" style="left:${at(+fit)}" title="Per-hit fit ${fit}×"></i>` : ''}<i class="mdl" style="left:${at(s.TYPE_BONUS)}" title="Model ${s.TYPE_BONUS}×"></i><span>1×</span><span>2.5×</span></div>`;
  const health = (label, value, ok) => `<div class="hb"><span class="hdot ${ok === 'bad' ? 'bad' : ok ? 'ok' : ''}" aria-hidden="true"></span><span class="small">${label}</span><span>${value}</span></div>`;
  const maxShare = Math.max(...m.unlisted.map((u) => u.share), 0.01);
  main.innerHTML =
    header('Model · settings, learning and data', `<button class="btn ghost ml" id="rt">Re-test now</button>`) +
    `<div class="g4">
      ${kpi(ring(m.live_total ? m.calls_right / m.live_total : null, { size: 64 }), 'Live calls', `${m.calls_right} / ${m.live_total}`, `today's logic would get ${m.live_right}`)}
      ${kpi(ring(bt?.duels ? bt.accuracy : null, { size: 64, tone: 'home', label: bt?.duels ? `${(bt.accuracy * 100).toFixed(0)}%` : '—' }), 'History test', bt?.duels ? (bt.accuracy * 100).toFixed(1) + '%' : '—', `${bt ? fmt(bt.duels) : 0} past duels, base stats only`)}
      <div class="panel kp kfit"><div><span class="label">Per-hit fit</span><b>${fit ? fit + '×' : '—'}</b><small>real value of a type edge (model ${s.TYPE_BONUS}×)</small></div>${fitScale}</div>
      ${kpi(`<span class="kic">${icon('chart', 26)}</span>`, 'Data', fmt(m.hits), `hits from ${fmt(m.tapes)} fight tapes`)}</div>
    <div class="g3 grow">
      <div class="panel col"><div class="ph"><h3>Settings</h3><span>fitted on logged fights</span></div><div class="sets">
        ${slider('Type edge bonus', s.TYPE_BONUS, 1, 2.5, s.TYPE_BONUS + '×')}${slider('Resisted moves', s.TYPE_RESIST, 1, 2, '÷' + s.TYPE_RESIST)}${slider('Move power weight', s.POWER_EXP, 0, 1, '^' + s.POWER_EXP)}
        ${slider('Stats → chance', s.STATS_STEEPNESS, 1, 5, s.STATS_STEEPNESS)}${slider('Species records', s.SPECIES_WEIGHT, 0, 2, s.SPECIES_WEIGHT)}${slider('Pal head to head', s.PAL_H2H_WEIGHT, 0, 2, s.PAL_H2H_WEIGHT)}</div></div>
      <div class="stackc">${suggestion}
        <div class="panel col grow"><div class="ph"><h3>Pals with secret moves</h3><span>share of damage</span></div>${
          m.unlisted.length
            ? `<div class="fillin bars">${m.unlisted.map((u) => `<div class="hbar" title="${esc(u.moves.map((x) => x[0]).join(', '))}"><span>${esc(u.pal)} <span class="muted">· ${u.moves.map((x) => esc(x[0])).join(', ')}</span></span><b class="gold">${pct(u.share)}</b><div class="b"><i style="width:${(100 * u.share) / maxShare}%"></i></div></div>`).join('')}</div>`
            : empty('eye', 'None found yet', 'Pals whose damage comes from moves the site doesn’t list, once enough fight tapes are in.')
        }</div></div>
      <div class="stackc">
        <div class="panel"><div class="ph"><h3>Data health</h3></div>
          ${health('Live feed', m.synced_at ? `synced ${Date.now() - m.synced_at < 90000 ? `${Math.max(0, Math.round((Date.now() - m.synced_at) / 1000))}s ago` : ago(m.synced_at)}` : 'not yet', !!m.synced_at)}
          ${health('Past duels', fmt(m.duels), !!m.duels)}
          ${health('Species', fmt(m.species), !!m.species)}
          ${health('Backfill', backfillLine(b), b.last_error ? 'bad' : true)}
          ${b.phase && b.phase !== 'done' && b.total ? `<div class="progress"><i style="width:${(b.done / b.total) * 100}%"></i></div>` : ''}
          ${b.last_error ? `<div class="small bad">${esc(b.last_error)}</div>` : ''}</div>
        <div class="panel col grow"><div class="ph"><h3>Open questions</h3></div><div class="oq fillin">
          <div><span>${icon('star', 16)}</span><p>Is a type edge worth anything? The per-hit fit says ${fit ?? '?'}×.</p></div>
          <div><span>${icon('chart', 16)}</span><p>Do cooldown passives mean more attacks? Counted at half strength.</p></div>
          <div><span>${icon('eye', 16)}</span><p>Owners matter but aren't in the model yet.</p></div></div></div></div>
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
