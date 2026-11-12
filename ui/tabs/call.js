// History › one call's breakdown: the two pals and the result, how the duel went (each side's HP over the fight,
// from its tape, and the damage by move), and how the model came to its call (the three factors added up on the
// log-odds scale, the tale of the tape and the settings it used). Opened from a row in Augur's calls or Your bets.
import { invoke } from '../lib/bridge.js';
import { esc, fmt } from '../lib/format.js';
import { SIDES, headline as callHeadline, modelFooter, other, pal, tape, waterfall } from '../lib/callparts.js';
import { empty } from '../lib/viz.js';

const clock = (t) => `${Math.floor(t / 60)}:${String(Math.round(t % 60)).padStart(2, '0')}`;

// the shared headline, with how it ended: right or missed, the winner, the K.O. and the HP it had left
function headline(d) {
  const c = d.call;
  const f = d.fight;
  const right = c.winner === c.pick;
  const result = [`${esc(pal(c, c.winner))} won`, f?.ko || c.ko ? `K.O. ${esc(f?.ko || c.ko)}` : '', (f?.winner_hp_left ?? c.winner_hp_left) != null ? `${f?.winner_hp_left ?? c.winner_hp_left}% HP left` : '']
    .filter(Boolean)
    .join(' · ');
  const verdict = `<div class="cres ${right ? 'ok' : 'bad'}"><span class="res ${right ? 'o' : 'x'}">${right ? '✓' : '✕'}</span>${right ? 'Right' : 'Missed'}</div>`;
  return callHeadline(d, { verdict, line: result });
}

// each side's HP over the fight as steps, from the tape: home cyan, away pink; the hardest hits ringed, the K.O. marked
function hpChart(f, c, m) {
  const W = 520;
  const H = 150;
  const L = 38;
  const B = 18;
  const end = Math.max(f.ko_secs || 0, ...f.hits.map((h) => h.t), 1) + 1;
  const x = (t) => (L + ((W - L - 6) * t) / end).toFixed(1);
  const y = (v) => (6 + ((H - B - 6) * (100 - Math.max(0, Math.min(100, v)))) / 100).toFixed(1);
  const path = (pts) => pts.map(([t, v], i) => (i ? `H${x(t)} V${y(v)}` : `M${x(t)} ${y(v)}`)).join(' ');
  const big = new Set(f.biggest.slice(0, 3).map((h) => `${h.side}${h.t}${h.move}${h.pct}`));
  // each hit lands on the other side's line: walk the tape again to place it
  const left = { home: 100, away: 100 };
  const dots = f.hits
    .map((h) => {
      const target = other(h.side);
      left[target] = Math.max(0, left[target] - h.pct);
      const ring = big.has(`${h.side}${h.t}${h.move}${h.pct}`);
      const name = h.move || 'unnamed hit';
      return `<circle class="${h.side}${ring ? ' big' : ''}" cx="${x(h.t)}" cy="${y(left[target])}" r="${ring ? 4.5 : 2.5}"><title>${clock(h.t)} · ${esc(pal(c, h.side))}'s ${esc(name)}: ${h.pct}% of ${esc(pal(c, target))}'s HP</title></circle>`;
    })
    .join('');
  const ticks = [...new Set([0, Math.round(end / 3), Math.round((2 * end) / 3)])].filter((t) => f.ko_secs == null || x(f.ko_secs) - x(t) > 46 || t === 0);
  // Augur's projected race, faint behind the real one: each side's HP falling at the rate the model gave it. The
  // model's times aren't seconds, so its first K.O. is put at the real one; a miss shows where the fight left it
  const sc = m?.score;
  const first = sc && Math.min(sc.t_home, sc.t_away);
  const proj =
    first > 0 && Number.isFinite(first) && f.ko_secs
      ? SIDES.map((s) => {
          // a side falls as fast as the other knocks it out (t: each side's time to land its K.O.)
          const t = s === 'home' ? sc.t_away : sc.t_home;
          return `<path class="proj ${s}" d="M${x(0)} ${y(100)} L${x(f.ko_secs)} ${y(100 * (1 - first / t))}"><title>Augur's projection: ${esc(pal(c, s))} ${t === first ? 'knocked out first' : `left with ${Math.round(100 * (1 - first / t))}% HP`}</title></path>`;
        }).join('')
      : '';
  const ko = f.ko_secs != null ? `<line class="ko" x1="${x(f.ko_secs)}" x2="${x(f.ko_secs)}" y1="4" y2="${H - B}"/><text class="kot" x="${x(f.ko_secs)}" y="${H - 4}" text-anchor="end">K.O. ${esc(f.ko)}</text>` : '';
  return `<svg class="hpc" viewBox="0 0 ${W} ${H}" role="img" aria-label="Each side's HP over the fight: ${esc(pal(c, c.winner))} won${f.ko ? ` by K.O. at ${esc(f.ko)}` : ''}">
    <g class="grid">${[100, 50, 0].map((v) => `<line x1="${L}" x2="${W - 6}" y1="${y(v)}" y2="${y(v)}"/><text x="${L - 6}" y="${+y(v) + 4}" text-anchor="end">${v}%</text>`).join('')}</g>
    <g class="grid">${ticks.map((t) => `<text x="${x(t)}" y="${H - 4}" text-anchor="middle">${clock(t)}</text>`).join('')}</g>
    ${ko}${proj}
    ${SIDES.map((s, i) => `<path class="area ${s}" d="${path(f.hp[i])} V${y(0)} H${x(0)} Z"/><path class="line ${s}" d="${path(f.hp[i])}"/>`).join('')}
    ${dots}</svg>`;
}

function howItWent(d) {
  const f = d.fight;
  const c = d.call;
  const casts = f ? `${fmt(f.casts[0])} vs ${fmt(f.casts[1])} casts` : '';
  if (!f || !f.hits.length) {
    return `<div class="panel col chart"><div class="ph"><h3>How it went</h3><span>${casts}</span></div>${empty('chart', f ? 'No fight tape for this duel' : 'The duel isn’t stored', f ? 'Only the result was kept: older duels came without their tape.' : 'Its result came from the live feed.')}</div>`;
  }
  const projected = d.model?.score ? '<span class="lgp" title="Augur’s projected race, its first K.O. put at the real one"><i></i>Projected</span>' : '';
  const lg = SIDES.map((s) => `<span><i class="${s}"></i>${esc(pal(c, s))}</span>`).join('') + projected;
  const top = f.biggest
    .slice(0, 3)
    .map((h) => `<span class="chip ${h.side}" title="${clock(h.t)} · ${esc(pal(c, h.side))}"><b>${h.pct}%</b> ${esc(h.move)}</span>`)
    .join('');
  return `<div class="panel chart"><div class="ph"><h3>How it went</h3><div class="lg">${lg}</div></div>
    ${hpChart(f, c, d.model)}
    <div class="crow chips"><span class="label">Top hits</span>${top}</div></div>`;
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
        return `<div class="mv"><div class="mvt"><span>${name}${edge}</span><b>${Math.round(m.total)}%</b></div>
          <div class="b"><i class="${s}" style="width:${Math.min(100, m.total)}%"></i></div>
          <small>${fmt(m.hits)} hit${m.hits === 1 ? '' : 's'} · best ${Math.round(m.max)}%${m.power != null ? ` · ${esc(m.el)} ${fmt(m.power)}` : ''}</small></div>`;
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
      header(`History · ${esc(c.home_pal)} vs ${esc(c.away_pal)}${d.fight ? ` · duel #${d.fight.id}` : ''}`, bar) +
      headline(d) +
      `<div class="cgrid grow">
        <div class="stackc">${howItWent(d)}${damage(d)}</div>
        <div class="stackc">${waterfall(d, modelFooter(d))}</div>
        ${tape(d)}
      </div>`;
  }
  const b = document.getElementById('back');
  b.onclick = back;
  b.focus({ preventScroll: true });
}
