// History › one call's breakdown: the two pals and the result, how the duel went (each side's HP over the fight,
// from its tape, and the damage by move), and how the model came to its call (the three factors added up on the
// log-odds scale, the tale of the tape and the settings it used). Opened from a row in Augur's calls or Your bets.
import { AUTO_STATUS, viaChip } from '../lib/auto.js';
import { invoke } from '../lib/bridge.js';
import { esc, fmt, pct, signed } from '../lib/format.js';
import { SIDES, headline as callHeadline, modelFooter, pal, tape, waterfall } from '../lib/callparts.js';
import { BET_LABELS } from '../lib/stake.js';
import { projectedGap, wireWorm, worm } from '../lib/worm.js';
import { empty } from '../lib/viz.js';

const clock = (t) => `${Math.floor(t / 60)}:${String(Math.round(t % 60)).padStart(2, '0')}`;

// the shared headline, with how it ended: right or missed, the winner, the K.O. and the HP it had left
function headline(d) {
  const c = d.call;
  const right = c.winner === c.pick;
  const verdict = `<div class="cres ${right ? 'ok' : 'bad'}"><span class="res ${right ? 'o' : 'x'}">${right ? '✓' : '✕'}</span>${right ? 'Right' : 'Missed'}</div>`;
  return callHeadline(d, { verdict, extra: ledger(d), lead: timeline(d), stream: streamReads(d) });
}

// The stream read for a hidden side, in a word on the call's line: ✓ read (and the site agreed at the bell), ✕ the
// site differed, or couldn't read it. A fight with nothing hidden shows the reader's check on it instead: ✓ both
// sides read as the site showed them, ✕ a misread, or – no panel to check. What it read, or why it couldn't, is on
// hover; its frame beside it (click to enlarge, loaded once the page is up)
function streamReads(d) {
  const rs = d.stream_reads || [];
  const first = rs.find((r) => r.outcome === 'read' || r.outcome === 'missed');
  if (!first) return streamCheck(rs);
  const bell = rs.find((r) => r.outcome === 'agreed' || r.outcome === 'differed');
  const how = [first.height ? `${first.height}p` : '', first.latency != null ? `${first.latency.toFixed(1)}s behind live` : ''].filter(Boolean).join(', ');
  const behind = how ? ` (${how})` : '';
  const read = first.outcome === 'read';
  const said = read ? `Read the ${first.side} side off the stream: ${first.stats}${behind}` : `Couldn't read the ${first.side} side off the stream: ${first.detail}${behind}`;
  const atBell = !bell ? '' : bell.outcome === 'agreed' ? '. The site agreed at the bell' : `. At the bell ${bell.detail}`;
  const ok = read && bell?.outcome !== 'differed';
  const shot = first.shot ? `<img data-shot="${esc(first.shot)}" alt="The stream's frame">` : '';
  return `<span class="ts sr ${ok ? 'ok' : 'bad'}" title="${esc(said + atBell)}">stream <b>${ok ? '✓' : '✕'}</b>${shot}</span>`;
}

function streamCheck(rs) {
  const c = rs.find((r) => ['checked', 'misread', 'unchecked'].includes(r.outcome));
  if (!c) return '';
  const how = [c.height ? `${c.height}p` : '', c.latency != null ? `${c.latency.toFixed(1)}s behind live` : ''].filter(Boolean).join(', ');
  const behind = how ? ` (${how})` : '';
  const [tone, mark, said] = {
    checked: ['ok', '✓', `Nothing was hidden, so the reader was checked: both sides read as the site showed them, ${c.stats}`],
    misread: ['bad', '✕', `Nothing was hidden, so the reader was checked: ${c.detail}`],
    unchecked: ['', '–', `Nothing was hidden, and the reader couldn't be checked: ${c.detail}`],
  }[c.outcome];
  const shot = c.shot ? `<img data-shot="${esc(c.shot)}" alt="The stream's frame">` : '';
  return `<span class="ts sr ${tone}" title="${esc(said + behind)}">stream <b>${mark}</b>${shot}</span>`;
}

// what changed a version of the call, in words (revisions::cause_words)
const CAUSES = {
  first: 'first call',
  stats_shown: 'stats shown',
  stats_source: 'where the stats came from',
  stats: 'stats',
  records: 'species records',
  pal_h2h: 'pal head-to-head',
  moves: 'moves',
  model: 'model settings',
  stake: 'stake',
  bell: 'the bell',
  bet: 'auto bet',
  unknown: 'not known',
};
const WENT_OUT = ['sending', 'sent', 'seen', 'confirmed', 'unconfirmed'];
// the most steps the line shows: the first, then the latest
const STEPS = 5;

// How the call changed while its fight was up, in a line: each version's chance (and its pal, when it called the
// other side), auto bet's bet, and the call at the bell. What changed each one is on hover. Nothing for a call that
// never changed.
function timeline(d) {
  const c = d.call;
  const revs = d.revisions || [];
  if (!revs.length) return '';
  const rebuilt = revs.some((r) => r.trigger === 'backfill');
  const t0 = revs[0].at;
  const when = (r) => (rebuilt || r.phase === 'bell' ? '' : `${clock(Math.max(0, r.at - t0) / 1000)} `);
  const chance = (r) => `${r.pick === c.pick ? '' : `${esc(pal(c, r.pick))} `}<b>${pct(r.prob)}</b>`;
  const why = (r) => [r.cause.split(',').filter(Boolean).map((k) => CAUSES[k] || 'other').join(', '), r.detail].filter(Boolean).join(': ');
  const a = d.auto_bet;
  const bet = (r) => `<span class="ts tbet" title="${esc(`Auto bet bet ${r.stake_cmd || ''} at ${pct(r.prob)}`)}">bet <b>${esc(r.stake_cmd || '')}</b></span>`;
  const steps = [];
  let betShown = false;
  revs.forEach((r) => {
    if (r.trigger === 'bet') {
      steps.push(bet(r));
      betShown = true;
      return;
    }
    // another computer's bet, or one from before the history was kept: it went in on the call as it stood
    if (!betShown && r.phase !== 'picks' && a && !a.test && WENT_OUT.includes(a.status)) {
      steps.push(bet({ stake_cmd: a.command, prob: a.prob }));
      betShown = true;
    }
    const label = r.phase === 'bell' ? 'bell ' : r.n === 1 ? 'called ' : when(r);
    const tone = r.phase === 'picks' ? '' : ' after';
    steps.push(`<span class="ts${tone}" title="${esc(why(r))}${r.phase === 'after_bet' ? ' (after the bet: not the call)' : r.phase === 'bell' ? ' (after picks closed: not the call)' : ''}">${label}${chance(r)}</span>`);
  });
  if (!betShown && a && !a.test && WENT_OUT.includes(a.status)) steps.push(bet({ stake_cmd: a.command, prob: a.prob }));
  if (steps.length < 2) return '';
  const shown = steps.length > STEPS ? [steps[0], `<span class="ts more">+${steps.length - STEPS + 1}</span>`, ...steps.slice(-(STEPS - 2))] : steps;
  const title = rebuilt ? 'Rebuilt from what the call saved: changes before the last one weren’t kept then' : 'Each version of the call while its fight was up';
  return `<div class="ctl" title="${esc(title)}">${shown.join('<i>›</i>')}</div>`;
}

// Under the dial, a column each: how it ended, then the money on it as Your bets lines it up (Augur's bet and what it
// made, then yours once the picks list has been read). The winner is on its side already, crowned.
function ledger(d) {
  const c = d.call;
  const f = d.fight;
  const b = d.bet;
  const ko = f?.ko || c.ko;
  const hp = f?.winner_hp_left ?? c.winner_hp_left;
  const col = (label, value) => `<div class="lc"><span class="ll">${label}</span><span class="lv">${value}</span></div>`;
  const made = (n, est) =>
    n == null ? '' : `<em class="${n > 0 ? 'ok' : n < 0 ? 'bad' : 'muted'}">${signed(n)}${est ? '<small title="Estimated at your usual payout"> est.</small>' : ''}</em>`;
  // yours as the chat command, like Augur's (the amount and the pal are on hover)
  const cmd = (side, n) => `${side === 'home' ? '!h' : '!a'} ${n}`;
  const cols = [ko ? col('K.O.', `<b>${esc(ko)}</b>${hp != null ? `<small>${hp}% HP left</small>` : ''}`) : col('Won', `<b>${esc(pal(c, c.winner))}</b>`)];
  // an auto bet that went through is your bet, so it's said once there; one that was skipped or cancelled isn't news
  // (Your bets lists why); one that went wrong tags your bet instead, in a word (what happened, and the bot's reply, on
  // hover), or has a column of its own while your bets are still to be read
  const a = d.auto_bet;
  const placed = b?.via?.via === 'auto';
  let trouble = '';
  if (a && !placed && !['skipped', 'cancelled'].includes(a.status)) {
    const [word, tone] = AUTO_STATUS[a.status] || [a.status, 'muted'];
    const why = [a.command, a.reason || a.reply].filter(Boolean).join(': ');
    trouble = `<span class="ast ${tone}" title="Auto bet ${esc(word.toLowerCase())}${why ? `: ${esc(why)}` : ''}">${esc(word)}</span>`;
  }
  if (b) {
    const how = b.staking ? ` title="${esc(b.staking)}"` : '';
    cols.push(col('Augur’s bet', b.augur_stake ? `<b${how}>${esc(b.augur_command)}</b>${made(b.augur_result, b.estimated)}` : `<small${how}>sat out</small>`));
    if (d.bets_read) {
      const you = b.backed ? `<b title="${fmt(b.stake)} on ${esc(pal(c, b.backed))}">${cmd(b.backed, b.stake)}</b>${made(b.result)}` : '<small>no bet</small>';
      const tag = placed ? viaChip(b.via) : trouble || `<span class="lab ${b.label}">${BET_LABELS[b.label]}</span>${b.via?.double ? viaChip(b.via) : ''}`;
      cols.push(col(`Your bet ${tag}`, you));
      trouble = '';
    }
  } else if (c.damaged) {
    cols.push(col('Money', '<small title="Its call was overwritten after the fight, so it’s left out of the money">left out</small>'));
  }
  if (trouble) cols.push(col('Auto bet', trouble));
  return `<div class="ledger">${cols.join('')}</div>`;
}

function howItWent(d) {
  const f = d.fight;
  const c = d.call;
  const casts = f ? `${fmt(f.casts[0])} vs ${fmt(f.casts[1])} casts` : '';
  if (!f || !f.hits.length) {
    return `<div class="panel col chart"><div class="ph"><h3>How it went</h3><span>${casts}</span></div>${empty('chart', f ? 'No fight tape for this duel' : 'The duel isn’t stored', f ? 'Only the result was kept: older duels came without their tape.' : 'Its result came from the live feed.')}</div>`;
  }
  const proj = projectedGap(d.model?.score);
  const lg = proj != null ? '<span class="lgp" title="Augur’s projected gap at the K.O., its first K.O. put at the real one"><i></i>Projected</span>' : '';
  return `<div class="panel chart"><div class="ph"><h3>How it went</h3><div class="lg">${lg}</div></div>
    ${worm({ hits: f.hits, end: f.ko_secs ?? Math.max(0, ...f.hits.map((h) => h.t)) + 1, ko: f.ko_secs ?? null, winner: c.winner }, { names: [c.home_pal, c.away_pal], proj, projAt: f.ko_secs ?? null, h: 128, adv: false })}
</div>`;
}

// the damage each side dealt, by move: one bar per move, scaled to the other side's whole HP
function damage(d) {
  const f = d.fight;
  const c = d.call;
  if (!f || !f.hits.length) return `<div class="panel col grow"><div class="ph"><h3>Damage by move</h3></div>${empty('swords', 'Nothing to break down', 'The hits show here when the duel has its tape.')}</div>`;
  const col = (s, i) => {
    const moves = f.moves[i];
    const dealt = moves.reduce((a, m) => a + m.total, 0);
    const rows = moves
      .map((m) => {
        const edge = m.mult > 1 ? '<span class="pill ok" title="A type edge against the other side">edge</span>' : m.mult != null && m.mult < 1 ? '<span class="pill bad" title="Resisted by the other side">resisted</span>' : '';
        const name = m.name ? esc(m.name) : '<span class="muted" title="Likely damage over time">Unnamed hits</span>';
        // the hits, the best one and the move's element and power are on hover
        const more = `${fmt(m.hits)} hit${m.hits === 1 ? '' : 's'} · best ${Math.round(m.max)}%${m.power != null ? ` · ${m.el} ${fmt(m.power)}` : ''}`;
        return `<div class="mv" title="${esc(more)}"><div class="mvt"><span>${name}${edge}</span><small>${fmt(m.hits)}×</small><b>${Math.round(m.total)}%</b></div>
          <div class="b"><i class="${s}" style="width:${Math.min(100, m.total)}%"></i></div></div>`;
      })
      .join('');
    return `<div class="mvc"><div class="mvh ${s}"><b>${esc(pal(c, s))}</b><span>${Math.round(dealt)}% dealt</span></div>${rows || '<span class="small">no hits landed</span>'}</div>`;
  };
  return `<div class="panel col grow"><div class="ph"><h3>Damage by move</h3><span title="Each move's hits as a share of the other side's HP">${fmt(f.hits.length)} hits · ${fmt(f.casts[0])} vs ${fmt(f.casts[1])} casts</span></div>
    <div class="fillin mvs">${SIDES.map(col).join('')}</div></div>`;
}

export async function render(app, back) {
  const { state, main, header } = app;
  const id = state.call;
  const d = await invoke('call_detail', { id });
  if (state.tab !== 'history' || state.call !== id) return;
  const bar = `<button class="btn ghost ml" id="back">‹ Back to ${state.view === 'bets' ? 'your bets' : 'the calls'}</button>`;
  if (!d) {
    main.innerHTML = header('History · a call', bar) + `<div class="panel col grow">${empty('swords', 'This call isn’t settled', 'It shows here once its duel has a result.')}</div>`;
  } else {
    const c = d.call;
    main.innerHTML =
      header(`History · ${esc(c.home_pal)} vs ${esc(c.away_pal)}${d.fight ? ` · duel #${d.fight.id}` : ''}${c.version ? ` · called by Augur ${esc(c.version)}` : ''}`, bar) +
      headline(d) +
      `<div class="cgrid grow">
        <div class="stackc">${howItWent(d)}${damage(d)}</div>
        <div class="stackc">${waterfall(d, modelFooter(d), true)}</div>
        ${tape(d)}
      </div>`;
  }
  // How Augur called it: the sum, or the model it used
  main.querySelectorAll('[data-wv]').forEach((t) => {
    t.onclick = () => {
      main.querySelectorAll('[data-wv]').forEach((x) => x.classList.toggle('on', x === t));
      main.querySelectorAll('[data-wvp]').forEach((x) => (x.hidden = x.dataset.wvp !== t.dataset.wv));
    };
  });
  wireWorm(main);
  main.querySelectorAll('img[data-shot]').forEach((img) => {
    invoke('stream_shot', { name: img.dataset.shot })
      .then((src) => (img.src = src))
      .catch(() => img.remove());
    // the frame at a size to read, over the page: a click closes it
    img.onclick = () => {
      const big = document.createElement('div');
      big.className = 'shotbig';
      big.innerHTML = `<img src="${img.src}" alt="The stream's frame">`;
      big.onclick = () => big.remove();
      document.body.appendChild(big);
    };
  });
  const b = document.getElementById('back');
  b.onclick = back;
  b.focus({ preventScroll: true });
}
