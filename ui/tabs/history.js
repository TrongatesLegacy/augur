// History: Augur's own calls (today, this week or all time), calibration and the record by stream; and Your bets:
// the developer's bets next to Augur's calls and stakes, with missed fights kept apart from deviations.
import { invoke } from '../lib/bridge.js';
import { ago, esc, fmt, pct, short, signed } from '../lib/format.js';
import { columns, empty, ring, signedBar, spark, stack } from '../lib/viz.js';

const RANGES = [
  ['today', 'Today'],
  ['week', 'This week'],
  ['all', 'All time'],
];

// a call re-made after picks closed (stats shown at the bell) that came out differently: shown, not counted
const lateNote = (c) =>
  c.late && c.late[0] !== c.pick
    ? `<small class="late" title="Re-called with the real stats after picks closed; the call above is the one that counts">after the bell: ${esc(c.late[0] === 'home' ? c.home_pal : c.away_pal)} ${pct(c.late[1])}</small>`
    : '';

function callRow(c) {
  const right = c.winner === c.pick;
  return `<tr><td><span class="res ${right ? 'o' : 'x'}">${right ? '✓' : '✕'}</span></td><td>${esc(c.home_pal)} <span class="muted">vs</span> ${esc(c.away_pal)}</td><td>${esc(c.pick === 'home' ? c.home_pal : c.away_pal)}${lateNote(c)}</td><td class="num">${pct(c.prob)}</td><td class="${right ? 'ok' : 'bad'}">${right ? 'Right' : 'Missed'}</td><td class="num">${esc(c.ko || '')}</td></tr>`;
}

const VIEWS = [
  ['calls', "Augur's calls"],
  ['bets', 'Your bets'],
];

const LABELS = { followed: 'Followed', deviated: 'Deviated', missed: 'Missed', no_call: 'No call', pending: 'Pending' };

const side = (f, s) => (s == null ? '—' : esc(s === 'home' ? f.home_pal : f.away_pal));
const money = (n, est) => (n == null ? '<span class="muted">—</span>' : `<span class="${n > 0 ? 'ok' : n < 0 ? 'bad' : 'muted'}">${signed(n)}</span>${est ? ' <span class="est" title="Estimated at your usual payout">est.</span>' : ''}`);

// a cell with the pal on top and the detail under it
const cell = (top, under) => `<div class="two"><span>${top}</span><small>${under}</small></div>`;

function betRow(f) {
  const augur = f.pick ? cell(side(f, f.pick), `${pct(f.prob)} · ${f.augur_command ? `<b>${esc(f.augur_command)}</b>` : 'no bet'}`) : '<span class="muted">no call</span>';
  const yours = f.backed ? cell(side(f, f.backed), `${fmt(f.stake)} credits`) : '<span class="muted">no bet</span>';
  return `<tr><td>${esc(f.home_pal)} <span class="muted">vs</span> ${esc(f.away_pal)}</td><td>${augur}</td><td>${yours}</td><td><span class="lab ${f.label}">${LABELS[f.label]}</span></td><td class="num">${money(f.result)}</td><td class="num">${f.pick ? money(f.augur_result, f.estimated) : money(null)}</td></tr>`;
}

async function renderBets(app, controls) {
  const { state, main, header } = app;
  const [b, acct] = await Promise.all([invoke('bets_view', { range: state.range }), invoke('account')]);
  if (state.tab !== 'history' || state.view !== 'bets') return;
  const c = b.comparison;
  const imp = b.import || {};
  const reading = imp.phase === 'bets' && imp.total;
  const status = !acct.signed_in && !b.stored
    ? 'Sign in (Settings › Account) and Augur reads your picks list: it’s only shown to you when signed in.'
    : `${fmt(b.stored)} bets read${reading ? ` of ${fmt(imp.total)}${imp.paused ? ' · paused while a duel is up' : ''}` : ''} · last read ${ago(b.synced_at)}${imp.last_error ? `<br><span class="bad">${esc(imp.last_error)}</span>` : ''}`;
  const lead = c.augur - c.yours;
  const max = Math.max(Math.abs(c.augur), Math.abs(c.yours), 1);
  const bet = c.followed + c.deviated;
  main.innerHTML =
    header('History · your bets', controls) +
    `<div class="hrow bets">
      <div class="panel bvs"><div class="ph"><h3>Augur vs you</h3><span>${lead === 0 ? 'same as you' : `${signed(lead)} vs what you did`}</span></div>
        ${signedBar('Following Augur', c.augur, max)}${signedBar('What you did', c.yours, max)}
        <div class="small">on the ${fmt(c.bet_on)} calls you bet</div></div>
      <div class="panel"><div class="ph"><h3>Followed · deviated</h3><span>${bet ? pct(c.followed / bet) + ' followed' : 'no calls bet yet'}</span></div>
        <div class="yrow">${ring(bet ? c.followed / bet : null, { size: 70, tone: 'ok', sub: 'followed' })}
          ${stack([
            [c.followed, 'followed', 'ok'],
            [c.deviated, 'deviated', 'gold'],
          ])}</div></div>
      <div class="panel bleft"><div><span class="label">Missed</span><b class="bn">${fmt(c.missed)}</b><small>${c.missed ? `${signed(c.missed_augur)} left on the table · not deviations` : 'none'}</small></div>
        <div><span class="label">No call</span><b class="bn">${fmt(c.no_call)}</b><small>${c.no_call ? `${signed(c.no_call_yours)} on fights Augur didn't call` : 'none'}</small></div></div>
    </div>
    <div class="g2 grow">
      <div class="panel col">${c.fights.length ? `<div class="fillin"><table class="sticky"><thead><tr><th>Fight</th><th>Augur</th><th>You</th><th></th><th>You made</th><th>Augur's way</th></tr></thead><tbody>
        ${c.fights.slice(0, 200).map(betRow).join('')}
      </tbody></table></div>` : empty('star', 'No bets or calls in this range yet', 'Your bets show here next to Augur’s calls.')}</div>
      <div class="stackc">
        <div class="panel"><div class="ph"><h3>Your picks list</h3><button class="btn ghost" id="rb" ${acct.signed_in ? '' : 'disabled'}>Read now</button></div>
          <div class="small">${status}</div>${reading ? `<div class="progress"><i style="width:${Math.min(100, (100 * b.stored) / imp.total)}%"></i></div>` : ''}</div>
        <div class="panel col grow"><div class="ph"><h3>How it's worked out</h3></div><div class="fillin hwo">
          <div><span class="lab followed">Followed</span><p>you bet Augur's pick (any amount)</p></div>
          <div><span class="lab deviated">Deviated</span><p>the other side, or a bet when Augur said sit out</p></div>
          <div><span class="lab missed">Missed</span><p>no bet on a call Augur would have bet on, kept out of the comparison</p></div>
          <p class="small">Augur's way uses the stake it suggested. On the side you backed it's your real payout scaled to that stake; a win on the other side is estimated at your usual <b>${c.odds.rate.toFixed(2)}</b> a credit (${c.odds.wins ? `median of your last ${fmt(c.odds.wins)} wins` : 'assumed until you have wins'}).${b.balance ? ` Balance: <b>${fmt(b.balance)}</b>.` : ''}</p></div></div>
      </div>
    </div>`;
  document.getElementById('rb')?.addEventListener('click', async (e) => {
    e.target.disabled = true;
    e.target.textContent = 'Reading…';
    await invoke('import_bets');
  });
  wire(app);
}

function wire(app) {
  const { state, main } = app;
  main.querySelectorAll('.seg button[data-r]').forEach((b) => {
    b.onclick = () => {
      state.range = b.dataset.r;
      render(app);
    };
  });
  main.querySelectorAll('.seg button[data-v]').forEach((b) => {
    b.onclick = () => {
      state.view = b.dataset.v;
      render(app);
    };
  });
}

export async function render(app) {
  const { state, main, header } = app;
  const views = `<div class="seg ml">${VIEWS.map(([k, l]) => `<button class="${state.view === k ? 'on' : ''}" data-v="${k}">${l}</button>`).join('')}</div>`;
  const seg = views + `<div class="seg">${RANGES.map(([k, l]) => `<button class="${state.range === k ? 'on' : ''}" data-r="${k}">${l}</button>`).join('')}</div>`;
  if (state.view === 'bets') return renderBets(app, seg);
  const h = await invoke('history', { range: state.range });
  if (state.tab !== 'history' || state.view !== 'calls') return;
  const t = h.tally;
  const miss = t.biggest_miss;
  const label = RANGES.find(([k]) => k === state.range)?.[1] || 'All calls';
  // oldest first: each call right or not, and the hit rate as it ran
  const results = h.calls.map((c) => c.winner === c.pick).reverse();
  let hits = 0;
  const running = results.map((r, i) => (hits += r) / (i + 1));
  const rate = t.total ? t.right / t.total : null;
  const strip = results.slice(-40);
  const vs = (name, x, cls) => `<div class="hbar"><span>${name}</span><b>${x == null ? '—' : pct(x)}</b><div class="b"><i class="${cls}" style="width:${(x || 0) * 100}%"></i></div></div>`;
  main.innerHTML =
    header("History · Augur's own calls", seg) +
    `<div class="hrow">
      <div class="panel hhero">${ring(rate, { size: 92, sub: 'right' })}
        <div class="ycol"><span class="label">${state.range === 'all' ? 'All calls' : label}</span><b class="bn gold">${fmt(t.right)} / ${fmt(t.total)}</b><small class="small">${t.total ? `${fmt(t.total - t.right)} missed · ${pct(t.right / t.total)} right` : 'no calls yet'}</small></div>
        <div class="htrend">${running.length > 1 ? `${spark(running, { label: 'Hit rate as the calls went' })}<span class="small">hit rate as the calls went</span>` : ''}
          <div class="strip" aria-label="The last ${strip.length} calls">${strip.map((r) => `<i class="${r ? 'w' : 'l'}" title="${r ? 'Right' : 'Missed'}"></i>`).join('')}</div></div></div>
      <div class="panel hruns"><div><span class="label">Best run</span><b class="bn">${fmt(t.best_run)}</b><small>right in a row</small></div><div><span class="label">Current run</span><b class="bn ${t.current_run ? 'ok' : ''}">${fmt(t.current_run)}</b><small>right in a row now</small></div></div>
      <div class="panel hconf"><div class="bars">${vs('Avg confidence', t.total ? t.avg_confidence : null, '')}${vs('Hit rate', rate, 'ok')}</div>
        <div class="hlog"><span class="label">Today's logic</span><b>${fmt(h.rescored_right)} / ${fmt(h.rescored_total)}</b><small>re-scored with current settings</small></div></div>
    </div>
    <div class="g2 grow">
      <div class="panel col">${h.calls.length ? `<div class="fillin"><table class="sticky"><thead><tr><th></th><th>Fight</th><th>Called</th><th>Chance</th><th>Result</th><th>K.O.</th></tr></thead><tbody>
        ${h.calls.map(callRow).join('')}
      </tbody></table></div>` : empty('swords', 'No calls in this range yet', 'Augur logs every fight it calls; try This week or All time.')}</div>
      <div class="stackc">
        <div class="panel"><div class="ph"><h3>Calibration</h3><span>said vs won</span></div><div class="cal">
          ${t.calibration.map(([band, won, n, promised]) => `<span>${band}</span><div class="cb"><i style="width:${(won / n) * 100}%"></i><s style="left:${promised * 100}%"></s></div><span>${fmt(won)}/${fmt(n)}</span>`).join('') || '<span class="muted">—</span>'}
        </div><div class="small" style="margin-top:10px">Bar: how often it won · tick: what it promised. Trustworthy after a few hundred calls.</div></div>
        <div class="panel col grow"><div class="ph"><h3>By stream</h3><span>one bar per day${rate != null ? ` · dashed: ${pct(rate)}` : ''}</span></div>${
          t.by_day.length
            ? columns(t.by_day.slice(-30).map(([d, r, n]) => ({ v: r / n, label: d ? d.slice(5).replace('-', '/') : '—', top: `${short(r)}/${short(n)}`, title: `${d || 'undated'}: ${fmt(r)}/${fmt(n)}`, tone: r / n >= 0.5 ? '' : 'bad' })), rate)
            : empty('chart', 'No streams yet')
        }</div>
        ${miss ? `<div class="panel hmiss"><span class="res x">✕</span><div><span class="label">Biggest miss</span><div>${esc(miss.pick === 'home' ? miss.home_pal : miss.away_pal)} at <b class="gold">${pct(miss.prob)}</b>, beaten by ${esc(miss.pick === 'home' ? miss.away_pal : miss.home_pal)}</div></div></div>` : ''}
      </div>
    </div>`;
  wire(app);
}
