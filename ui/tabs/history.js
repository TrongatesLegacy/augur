// History: Augur's own calls (today, this week or all time), calibration and the record by stream; and Your bets:
// the developer's bets next to Augur's calls and stakes, with missed fights kept apart from deviations.
import { invoke } from '../lib/bridge.js';
import { esc, fmt, pct, signed } from '../lib/format.js';

const RANGES = [
  ['today', 'Today'],
  ['week', 'This week'],
  ['all', 'All time'],
];

function callRow(c) {
  const right = c.winner === c.pick;
  return `<tr><td><span class="res ${right ? 'o' : 'x'}">${right ? '✓' : '✕'}</span></td><td>${esc(c.home_pal)} <span class="muted">vs</span> ${esc(c.away_pal)}</td><td>${esc(c.pick === 'home' ? c.home_pal : c.away_pal)}</td><td class="num">${pct(c.prob)}</td><td class="${right ? 'ok' : 'bad'}">${right ? 'Right' : 'Missed'}</td><td class="num">${esc(c.ko || '')}</td></tr>`;
}

const VIEWS = [
  ['calls', "Augur's calls"],
  ['bets', 'Your bets'],
];

const LABELS = { followed: 'Followed', deviated: 'Deviated', missed: 'Missed', no_call: 'No call', pending: 'Pending' };

const side = (f, s) => (s == null ? '—' : esc(s === 'home' ? f.home_pal : f.away_pal));
const money = (n, est) => (n == null ? '<span class="muted">—</span>' : `<span class="${n > 0 ? 'ok' : n < 0 ? 'bad' : 'muted'}">${signed(n)}</span>${est ? ' <span class="est" title="Estimated at your usual payout">est.</span>' : ''}`);

function betRow(f) {
  const augur = f.pick ? `${side(f, f.pick)} <span class="muted">${pct(f.prob)}</span>${f.augur_command ? ` · <b>${esc(f.augur_command)}</b>` : ' · no bet'}` : '<span class="muted">no call</span>';
  const yours = f.backed ? `${side(f, f.backed)} <span class="muted">${fmt(f.stake)}</span>` : '<span class="muted">no bet</span>';
  return `<tr><td>${esc(f.home_pal)} <span class="muted">vs</span> ${esc(f.away_pal)}</td><td>${augur}</td><td>${yours}</td><td><span class="lab ${f.label}">${LABELS[f.label]}</span></td><td class="num">${money(f.result)}</td><td class="num">${f.pick ? money(f.augur_result, f.estimated) : money(null)}</td></tr>`;
}

function ago(ms) {
  if (!ms) return 'never';
  const m = Math.round((Date.now() - ms) / 60000);
  return m < 1 ? 'just now' : m < 60 ? `${m} min ago` : m < 1440 ? `${Math.round(m / 60)}h ago` : `${Math.round(m / 1440)}d ago`;
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
  main.innerHTML =
    header('History · your bets', controls) +
    `<div class="tiles">
      <div class="tile hero"><span class="label">Following Augur</span><b>${signed(c.augur)}</b><small>${lead === 0 ? 'same as you' : `${signed(lead)} vs what you did`}</small></div>
      <div class="tile"><span class="label">What you did</span><b>${signed(c.yours)}</b><small>on the ${c.bet_on} calls you bet</small></div>
      <div class="tile"><span class="label">Followed · deviated</span><b>${c.followed} · ${c.deviated}</b><small>${c.followed + c.deviated ? pct(c.followed / (c.followed + c.deviated)) + ' followed' : 'no calls bet yet'}</small></div>
      <div class="tile"><span class="label">Missed</span><b>${c.missed}</b><small>${c.missed ? `${signed(c.missed_augur)} left on the table · not deviations` : 'none'}</small></div>
      <div class="tile"><span class="label">No call</span><b>${c.no_call}</b><small>${c.no_call ? `${signed(c.no_call_yours)} on fights Augur didn't call` : 'none'}</small></div>
    </div>
    <div class="g2">
      <div class="panel"><table><thead><tr><th>Fight</th><th>Augur</th><th>You</th><th></th><th>You made</th><th>Augur's way</th></tr></thead><tbody>
        ${c.fights.slice(0, 60).map(betRow).join('') || '<tr><td colspan="6" class="muted">No bets or calls in this range yet.</td></tr>'}
      </tbody></table></div>
      <div style="display:grid;gap:14px;align-content:start">
        <div class="panel"><div class="ph"><h3>Your picks list</h3><button class="btn ghost" id="rb" ${acct.signed_in ? '' : 'disabled'}>Read now</button></div>
          <div class="small">${status}</div>${reading ? `<div class="progress"><i style="width:${Math.min(100, (100 * b.stored) / imp.total)}%"></i></div>` : ''}</div>
        <div class="panel"><div class="ph"><h3>How it's worked out</h3></div><div class="small">
          <b>Followed</b>: you bet Augur's pick (any amount) · <b>Deviated</b>: the other side, or a bet when Augur said sit out · <b>Missed</b>: no bet on a call Augur would have bet on, kept out of the comparison.<br>
          Augur's way uses the stake it suggested. On the side you backed it's your real payout scaled to that stake; a win on the other side is estimated at your usual <b>${c.odds.rate.toFixed(2)}</b> a credit (${c.odds.wins ? `median of your last ${c.odds.wins} wins` : 'assumed until you have wins'}).${b.balance ? ` Balance: <b>${fmt(b.balance)}</b>.` : ''}</div></div>
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
  wire(app);
}
