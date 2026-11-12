// Model: the settings it has learned and what it changed, the species with secret moves, data health.
import { backfillState } from '../lib/backfill.js';
import { invoke } from '../lib/bridge.js';
import { ago, esc, fmt, pct } from '../lib/format.js';
import { empty, icon, ring } from '../lib/viz.js';

// a setting as a position on its range, with what the learner last did with it under the name
const slider = (label, v, lo, hi, shown, note = '') =>
  `<div class="set"><span>${label}${note}</span><div class="slw"><div class="sl"><b style="width:${((v - lo) / (hi - lo)) * 100}%"></b><i style="left:${((v - lo) / (hi - lo)) * 100}%"></i></div><div class="se"><span>${lo}</span><span>${hi}</span></div></div><span class="v">${shown}</span></div>`;

// every setting: label, range, how its value reads
export const KNOBS = {
  TYPE_BONUS: ['Type edge bonus', 1, 2.5, (v) => `${v}×`],
  TYPE_RESIST: ['Resisted moves', 1, 2.5, (v) => `÷${v}`],
  POWER_EXP: ['Move power weight', 0, 1, (v) => `^${v}`],
  STATS_STEEPNESS: ['Stats → chance', 1, 6, (v) => `${v}`],
  SPECIES_WEIGHT: ['Species records', 0, 2.5, (v) => `${v}`],
  PAL_H2H_WEIGHT: ['Pal head to head', 0, 1.5, (v) => `${v}`],
  COOLDOWN_EXP: ['Cooldown passives', 0, 1, (v) => `^${v}`],
  HIT_WEIGHT: ['Real hit size', 0, 1, (v) => `^${v}`],
};

// what the learner last did with a setting, as a line under its name
function knobNote(k, auto) {
  if (!k) return '';
  const from = k.source === 'past_duels' ? 'past duels' : 'real calls';
  const [label, text] =
    k.status === 'changed'
      ? ['ok', `learned: was ${KNOBS[k.key][3](k.was)}`]
      : k.status === 'waiting'
        ? ['muted', `waits for ${from}: ${fmt(k.have)} of ${fmt(k.need)}`]
        : ['', `holds on ${from}`];
  return `<small class="kn ${auto ? label : 'muted'}">${text}</small>`;
}

// a headline number with its picture on the left
const kpi = (pic, label, value, sub) => `<div class="panel kp">${pic}<div><span class="label">${label}</span><b>${value}</b><small>${sub}</small></div></div>`;

// how the calls did since the algorithm last changed (a new version or new settings), next to before
function erasBlock(eras) {
  if (!eras?.length) return '';
  const row = (e, label) =>
    `<div class="era"><span>${label}<small>${e.changes.length ? esc(e.changes.map((c) => c.replace(/^[A-Z_]+/, (k) => KNOBS[k]?.[0] || k)).join(' · ')) : e.version ? `version ${esc(e.version)}` : 'before versions were recorded'}</small></span><b class="${e.total && e.right / e.total >= 0.6 ? 'ok' : ''}">${fmt(e.right)}/${fmt(e.total)}</b><span class="muted">${e.total ? pct(e.right / e.total) : '—'}</span></div>`;
  return `<div class="eras"><span class="label">By algorithm</span>${eras.map((e, i) => row(e, i === 0 ? `Since ${ago(e.since)}` : `From ${ago(e.since)}`)).join('')}</div>`;
}

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
  const fit = m.calibration ? m.calibration[3].toFixed(2) : null;
  const l = m.learned;
  const c = l?.check;
  const gain = c && c.past_n ? c.past_right_after - c.past_right_before : 0;
  const learning = !m.auto
    ? `<div class="sug"><div class="label gold">Set by you</div><div class="small" style="color:var(--t2);margin-top:6px">Augur is using your own settings (Settings › Model) and keeps learning in the background, so you can switch back any time.</div><div style="display:flex;gap:8px;margin-top:10px"><button class="btn" id="auto">Let it tune itself</button></div></div>`
    : `<div class="panel"><div class="ph"><h3>Learning</h3><span>${l ? `checked ${ago(l.at_ms)}` : 'first check soon'}</span></div>${
        l
          ? `<div class="lrn"><div><b class="cond">${fmt(l.past_duels)}</b><small>past duels</small></div><div><b class="cond">${fmt(l.live_calls)}</b><small>real calls</small></div><div><b class="cond ${gain > 0 ? 'ok' : ''}">${(c.past_right_after * 100).toFixed(1)}%</b><small>right on the ${fmt(c.past_n)} newest${gain > 0 ? `, up ${(gain * 100).toFixed(1)}` : ''}</small></div></div>
             ${erasBlock(m.eras)}
             <div class="small lrnhow">Re-fits after each result; keeps a change only if it predicts the newest duels better.</div>`
          : `<div class="small" style="color:var(--t2)">Augur re-fits its settings on your duels and calls a minute after it starts, then whenever new results come in.</div>`
      }</div>`;
  const bt = m.backtest;
  // the per-hit fit and the model's own value on one scale (1× to 2.5×)
  const at = (x) => `${Math.max(0, Math.min(100, ((x - 1) / 1.5) * 100))}%`;
  const fitScale = `<div class="fitg" role="img" aria-label="Per-hit fit ${fit ?? 'not known'}, model ${s.TYPE_BONUS}"><div class="ft"></div>${fit ? `<i class="fit" style="left:${at(+fit)}" title="Per-hit fit ${fit}×"></i>` : ''}<i class="mdl" style="left:${at(s.TYPE_BONUS)}" title="Model ${s.TYPE_BONUS}×"></i><span>1×</span><span>2.5×</span></div>`;
  const health = (label, value, ok) => `<div class="hb"><span class="hdot ${ok === 'bad' ? 'bad' : ok ? 'ok' : ''}" aria-hidden="true"></span><span class="small">${label}</span><span>${value}</span></div>`;
  const maxShare = Math.max(...m.unlisted.map((u) => u.share), 0.01);
  main.innerHTML =
    header('Model · learns from every result', `<button class="btn ghost ml" id="rt">Learn now</button>`) +
    `<div class="g4">
      ${kpi(ring(m.live_total ? m.calls_right / m.live_total : null, { size: 64 }), 'Live calls', `${fmt(m.calls_right)} / ${fmt(m.live_total)}`, `today's logic would get ${m.live_right}`)}
      ${kpi(ring(bt?.duels ? bt.accuracy : null, { size: 64, tone: 'home', label: bt?.duels ? `${(bt.accuracy * 100).toFixed(0)}%` : '—' }), 'History test', bt?.duels ? (bt.accuracy * 100).toFixed(1) + '%' : '—', `${bt ? fmt(bt.duels) : 0} past duels, base stats only`)}
      <div class="panel kp kfit"><div><span class="label">Per-hit fit</span><b>${fit ? fit + '×' : '—'}</b><small>real value of a type edge (model ${s.TYPE_BONUS}×)</small></div>${fitScale}</div>
      ${kpi(`<span class="kic">${icon('chart', 26)}</span>`, 'Data', fmt(m.hits), `hits from ${fmt(m.tapes)} fight tapes`)}</div>
    <div class="g3 grow">
      <div class="panel col"><div class="ph"><h3>Settings</h3><span>${m.auto ? 'learned from your duels' : 'set by you'}</span></div><div class="sets">
        ${Object.entries(KNOBS)
          .map(([key, [label, lo, hi, show]]) => slider(label, s[key], lo, hi, show(s[key]), knobNote(l?.knobs.find((k) => k.key === key), m.auto)))
          .join('')}</div></div>
      <div class="stackc">${learning}
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
        <div class="panel col grow"><div class="ph"><h3>What it changed</h3><span>newest first</span></div>${
          m.changes.length
            ? `<div class="oq fillin">${m.changes
                .map((x) => {
                  const [label, , , show] = KNOBS[x.key] || [x.key, 0, 1, String];
                  return `<div><span>${icon(x.to > x.from ? 'chart' : 'star', 16)}</span><p><b>${esc(label)}</b> ${show(x.from)} → ${show(x.to)} <span class="muted">· ${ago(x.at_ms)}</span><br><small>log loss ${x.loss[0].toFixed(3)} → ${x.loss[1].toFixed(3)} on the ${fmt(x.n)} newest</small></p></div>`;
                })
                .join('')}</div>`
            : empty('star', 'Nothing yet', 'Every check so far says the settings hold. Changes show here when the results say so.')
        }</div></div>
    </div>`;
  document.getElementById('rt').onclick = async (e) => {
    e.target.disabled = true;
    await invoke('learn_now').catch(() => {});
    render(app);
  };
  document.getElementById('auto')?.addEventListener('click', async () => {
    state.settings.model_auto = true;
    await invoke('save_settings', { settings: state.settings });
    render(app);
  });
}
