// History › one call's breakdown: the two pals and the result, how the duel went (each side's HP over the fight,
// from its tape, and the damage by move), and how the model came to its call (the three factors added up on the
// log-odds scale, the tale of the tape and the settings it used). Opened from a row in Augur's calls or Your bets.
import { invoke } from '../lib/bridge.js';
import { esc, fmt, pct } from '../lib/format.js';
import { dial, face, orb, passivePills, tapeRows } from '../lib/pal.js';
import { empty } from '../lib/viz.js';

const SIDES = ['home', 'away'];
const clock = (t) => `${Math.floor(t / 60)}:${String(Math.round(t % 60)).padStart(2, '0')}`;
const pal = (c, s) => (s === 'home' ? c.home_pal : c.away_pal);
const other = (s) => (s === 'home' ? 'away' : 'home');
const signedLogit = (x) => `${x > 0 ? '+' : x < 0 ? '−' : ''}${Math.abs(x).toFixed(2)}`;
const SETTINGS = [
  ['TYPE_BONUS', 'Type edge', (v) => `${v}×`],
  ['TYPE_RESIST', 'Resisted', (v) => `1/${v}`],
  ['POWER_EXP', 'Move power', (v) => `^${v}`],
  ['STATS_STEEPNESS', 'Steepness', (v) => `${v}`],
  ['SPECIES_WEIGHT', 'Records', (v) => `×${v}`],
  ['PAL_H2H_WEIGHT', 'Pal H2H', (v) => `×${v}`],
  ['COOLDOWN_EXP', 'Cooldowns', (v) => `^${v}`],
  ['HIT_WEIGHT', 'Real hit size', (v) => `^${v}`],
];

// a side of the headline: orb (crowned if it won), name, owner, types, and whether Augur picked it
function side(d, s) {
  const c = d.call;
  const f = c.fighters?.[s === 'home' ? 0 : 1];
  const won = c.winner === s;
  const cls = s === 'home' ? 'h' : 'a';
  const owner = s === 'home' ? c.home_owner : c.away_owner;
  const tags = `${c.pick === s ? `<span class="ctag pick">Augur's pick</span>` : ''}${won ? '<span class="ctag won">Won</span>' : '<span class="ctag lost">Lost</span>'}`;
  return `<div class="fx ${s === 'away' ? 'r' : ''}">${orb(pal(c, s), d.slugs[s === 'home' ? 0 : 1], cls + (won ? '' : ' dim'), 68, won)}
    <div><div class="ctags">${tags}</div><div class="nm" style="${won ? '' : 'color:var(--t2)'}">${esc(pal(c, s))}</div>
      <div class="ow">${face(owner, 'a av', 18)}${s.toUpperCase()} · ${esc(owner)}</div>
      ${f ? `<div class="els">${f.types.map((t) => `<span class="el">${esc(t)}</span>`).join('')}</div>` : ''}</div></div>`;
}

// what's worth knowing about how the call was made: a re-call after the bell, hidden stats, a late call
function notes(d) {
  const c = d.call;
  const out = [];
  if (c.late && c.late[0] !== c.pick) out.push(`<span class="gold">After the bell</span> (real stats, not counted): ${esc(pal(c, c.late[0]))} ${pct(c.late[1])}`);
  if (c.late_only) out.push('First called after picks closed: not in the tally');
  SIDES.forEach((s, i) => {
    const from = d.stats_from[i];
    if (from && from !== 'shown') out.push(`${esc(pal(c, s))}'s hidden stats: ${esc(from.replace(/^from their box$/, 'taken from its owner’s box'))}`);
  });
  if (d.estimated && !d.stats_from.some((x) => x && x !== 'shown')) out.push('Some stats were hidden until the bell and guessed');
  return out.length ? `<div class="cnotes">${out.map((n) => `<span title="${esc(n.replace(/<[^>]+>/g, ''))}">${n}</span>`).join('')}</div>` : '';
}

function headline(d) {
  const c = d.call;
  const f = d.fight;
  const right = c.winner === c.pick;
  const result = [`${esc(pal(c, c.winner))} won`, f?.ko || c.ko ? `K.O. ${esc(f?.ko || c.ko)}` : '', (f?.winner_hp_left ?? c.winner_hp_left) != null ? `${f?.winner_hp_left ?? c.winner_hp_left}% HP left` : '']
    .filter(Boolean)
    .join(' · ');
  return `<div class="stage cstage">${side(d, 'home')}
    <div class="cmid"><div class="cverdict">${dial(c.prob, 80)}<div>
      <div class="cpick"><span class="label">Augur called</span><b>${esc(pal(c, c.pick))}</b></div>
      <div class="cres ${right ? 'ok' : 'bad'}"><span class="res ${right ? 'o' : 'x'}">${right ? '✓' : '✕'}</span>${right ? 'Right' : 'Missed'}</div></div></div>
      <div class="cwon">${result}</div>${notes(d)}</div>
    ${side(d, 'away')}</div>`;
}

// each side's HP over the fight as steps, from the tape: home cyan, away pink; the hardest hits ringed, the K.O. marked
function hpChart(f, c) {
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
  const ko = f.ko_secs != null ? `<line class="ko" x1="${x(f.ko_secs)}" x2="${x(f.ko_secs)}" y1="4" y2="${H - B}"/><text class="kot" x="${x(f.ko_secs)}" y="${H - 4}" text-anchor="end">K.O. ${esc(f.ko)}</text>` : '';
  return `<svg class="hpc" viewBox="0 0 ${W} ${H}" role="img" aria-label="Each side's HP over the fight: ${esc(pal(c, c.winner))} won${f.ko ? ` by K.O. at ${esc(f.ko)}` : ''}">
    <g class="grid">${[100, 50, 0].map((v) => `<line x1="${L}" x2="${W - 6}" y1="${y(v)}" y2="${y(v)}"/><text x="${L - 6}" y="${+y(v) + 4}" text-anchor="end">${v}%</text>`).join('')}</g>
    <g class="grid">${ticks.map((t) => `<text x="${x(t)}" y="${H - 4}" text-anchor="middle">${clock(t)}</text>`).join('')}</g>
    ${ko}
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
  const lg = SIDES.map((s) => `<span><i class="${s}"></i>${esc(pal(c, s))}</span>`).join('');
  const top = f.biggest
    .slice(0, 3)
    .map((h) => `<span class="chip ${h.side}" title="${clock(h.t)} · ${esc(pal(c, h.side))}"><b>${h.pct}%</b> ${esc(h.move)}</span>`)
    .join('');
  return `<div class="panel chart"><div class="ph"><h3>How it went</h3><div class="lg">${lg}</div></div>
    ${hpChart(f, c)}
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

// the model's sum on the log-odds scale: each factor moves the call toward home (left) or away (right), then the total
function waterfall(d, model) {
  const c = d.call;
  const m = d.model;
  if (!m) return `<div class="panel col grow"><div class="ph"><h3>How Augur called it</h3></div>${empty('eye', 'No numbers kept for this call', 'It was logged before Augur saved its factors and fighters.')}${model}</div>`;
  const steps = [];
  let run = 0;
  for (const f of m.factors) {
    steps.push([run, run + f.logit, f]);
    run += f.logit;
  }
  const span = Math.max(1, ...steps.flatMap(([a, b]) => [Math.abs(a), Math.abs(b)])) * 1.08;
  const at = (v) => 50 - (50 * v) / span;
  const rec = c.species_rec;
  const sc = m.score;
  const detail = {
    stats: sc ? `<span title="The model's turns to knock the other out: ${esc(c.home_pal)} ${fmt(sc.t_home)}, ${esc(c.away_pal)} ${fmt(sc.t_away)}">K.O. in ${fmt(sc.t_home)} vs ${fmt(sc.t_away)}</span>` : 'stats and moves',
    species: rec ? `${fmt(rec[0][0])}–${fmt(rec[0][1])} vs ${fmt(rec[1][0])}–${fmt(rec[1][1])}` : 'species records',
    h2h: `${fmt(c.pal_h2h[0])}–${fmt(c.pal_h2h[1])} when they met`,
  };
  const names = { stats: 'Stats', species: 'Records', h2h: 'Pal H2H' };
  const rows = steps
    .map(([a, b, f]) => {
      const [l, r] = [Math.min(at(a), at(b)), Math.max(at(a), at(b))];
      const lean = f.logit >= 0 ? 'home' : 'away';
      return `<div class="wf"><div class="wl"><b>${names[f.key]}${f.weight !== 1 ? ` <em>×${f.weight}</em>` : ''}</b><small>${detail[f.key]}</small></div>
        <div class="wt"><i class="${lean}" style="left:${l}%;width:${Math.max(0.6, r - l)}%"></i><s style="left:${at(b)}%"></s></div>
        <div class="wv"><b>${pct(f.p_home > 0.5 ? f.p_home : 1 - f.p_home)}</b><small>${esc(pal(c, f.p_home >= 0.5 ? 'home' : 'away'))}</small></div></div>`;
    })
    .join('');
  const fav = m.p_home >= 0.5 ? 'home' : 'away';
  const total = `<div class="wf total"><div class="wl"><b>Together</b><small>log-odds ${signedLogit(m.total)}</small></div>
    <div class="wt"><i class="gold" style="left:${Math.min(50, at(m.total))}%;width:${Math.abs(at(m.total) - 50)}%"></i></div>
    <div class="wv"><b class="gold">${pct(fav === 'home' ? m.p_home : 1 - m.p_home)}</b><small>${esc(pal(c, fav))}</small></div></div>`;
  return `<div class="panel col grow"><div class="ph"><h3>How Augur called it</h3><span>home ◂ ▸ away</span></div>
    <div class="fillin">${rows}${total}
    ${m.rescored ? '<div class="small">Re-run on the fighters it saw with its settings: the call didn’t keep its factors.</div>' : ''}${model}</div></div>`;
}

function settings(d) {
  const c = d.call;
  const s = d.settings;
  const chips = s ? SETTINGS.filter(([k]) => s[k] != null).map(([k, l, f]) => `<span class="chip">${l} <b>${f(s[k])}</b></span>`).join('') : '<span class="small">Not saved with this call.</span>';
  const stake = d.stake_cmd ? `suggested <b class="gold">${esc(d.stake_cmd)}</b>` : c.stake === 0 ? 'said sit it out' : '';
  // shown at the foot of How Augur called it: one panel fewer, so the page fits small windows
  return `<div class="cmodel"><div class="cmh"><span class="label">The model it used</span><span class="small">${c.version ? `Augur ${esc(c.version)}` : 'version not recorded'}</span></div>
    <div class="chips">${chips}</div>
    <div class="small cmeta">${[stake, c.fought_at ? `fought ${esc(c.fought_at.slice(0, 16).replace('T', ' '))} UTC` : '', d.fight ? `duel #${d.fight.id}` : ''].filter(Boolean).join(' · ')}</div></div>`;
}

// the tale of the tape: stats, talent rolls, move strength, passives and the type-edge moves
function tape(d) {
  const c = d.call;
  if (!c.fighters) return `<div class="panel col"><div class="ph"><h3>Tale of the tape</h3></div>${empty('box', 'No fighters saved', 'Calls from before Augur kept the fighters only have the result.')}</div>`;
  const [h, a] = c.fighters;
  const row = (label, hv, av, show = fmt) => {
    const mx = Math.max(hv || 0, av || 0) || 1;
    return `<div class="r5"><span class="n l ${hv != null && av != null && hv < av ? 'lo' : ''}">${hv == null ? '—' : show(hv)}</span><div class="bar bl"><i style="width:${((hv || 0) / mx) * 100}%"></i></div><span class="k">${label}</span><div class="bar br"><i style="width:${((av || 0) / mx) * 100}%"></i></div><span class="n ${hv != null && av != null && av < hv ? 'lo' : ''}">${av == null ? '—' : show(av)}</span></div>`;
  };
  const roll = (f) => {
    const v = ['HP', 'ATK', 'DEF'].map((k) => f.ivs?.[k]).filter((x) => x != null);
    return v.length ? v.reduce((x, y) => x + y, 0) / v.length : null;
  };
  const [sh, sa] = d.sides.map((s) => s.strength);
  const edges = (i) => d.sides[i].moves.filter((m) => m.usable && m.mult > 1).map((m) => m.name);
  const usable = (i) => d.sides[i].moves.filter((m) => m.usable).length;
  const edgeLine = SIDES.map((s, i) => {
    const e = edges(i);
    return `<div class="cedge ${s}"><b>${esc(pal(c, s))}</b> <span>${fmt(usable(i))} moves${e.length ? ` · type edge: ${e.map(esc).join(', ')}` : ' · no type edge'}</span></div>`;
  }).join('');
  // each side's moves at Lv 70, hardest first: what the model's move strength averages
  const list = (i) =>
    d.sides[i].moves
      .filter((m) => m.usable)
      .sort((x, y) => y.power * y.mult - x.power * x.mult)
      .map((m) => `<div class="cml" title="${esc(m.el)} · power ${fmt(m.power)}${m.hit !== 1 ? ` · hits ${m.hit.toFixed(2)}× the average move in tapes` : ''}"><span>${esc(m.name)}${Math.abs(m.hit - 1) >= 0.2 ? ` <em class="${m.hit > 1 ? 'ok' : 'bad'}">${m.hit.toFixed(1)}× hit</em>` : ''}</span><b class="${m.mult > 1 ? 'ok' : m.mult < 1 ? 'bad' : ''}" title="power ${fmt(m.power)}${m.mult !== 1 ? `, ${m.mult > 1 ? 'type edge' : 'resisted'} ×${+m.mult.toFixed(2)}` : ''}">${fmt(m.power)}${m.mult > 1 ? '▲' : m.mult < 1 ? '▼' : ''}</b></div>`)
      .join('');
  const moves = `<div class="cmoves">${SIDES.map((s, i) => `<div class="${s}">${list(i)}</div>`).join('')}</div>`;
  return `<div class="panel col"><div class="ph"><h3>Tale of the tape</h3><span>Lv 70</span></div>
    <div class="fillin">${tapeRows({ home: { fighter: h }, away: { fighter: a } })}
    ${row('ROLLS', roll(h), roll(a), (v) => `${Math.round(v)}`)}
    ${row('MOVES', sh, sa, (v) => v.toFixed(1))}
    <div class="cpass">${[h, a].map((f) => `<div class="pills">${f.passives.length ? passivePills(f.passives) : '<span class="small">no passives</span>'}</div>`).join('')}</div>
    ${edgeLine}${moves}</div></div>`;
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
        <div class="stackc">${waterfall(d, settings(d))}</div>
        ${tape(d)}
      </div>`;
  }
  const b = document.getElementById('back');
  b.onclick = back;
  b.focus({ preventScroll: true });
}
