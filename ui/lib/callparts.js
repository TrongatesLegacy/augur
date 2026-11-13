// The pieces of a call's breakdown that Live and History share: the headline (both pals, owners, Augur's pick and
// chance), how Augur called it (the three factors added up on the log-odds scale, the stats factor taken apart, the
// model it used) and the tale of the tape. Both answer "how is Augur calling this fight": Live before the bell,
// History after it. They take a call's detail (views.rs CallDetail): a pending call has no winner and no fight.
import { esc, fmt, pct } from './format.js';
import { dial, face, orb, passivePills, tapeRows } from './pal.js';
import { empty } from './viz.js';

export const SIDES = ['home', 'away'];
export const pal = (c, s) => (s === 'home' ? c.home_pal : c.away_pal);
export const other = (s) => (s === 'home' ? 'away' : 'home');
const signedLogit = (x) => `${x > 0 ? '+' : x < 0 ? '−' : ''}${Math.abs(x).toFixed(2)}`;
const SETTINGS = [
  ['TYPE_BONUS', 'Type edge', (v) => `${v}×`],
  ['TYPE_RESIST', 'Resisted', (v) => `1/${v}`],
  ['POWER_EXP', 'Move power', (v) => `^${v}`],
  ['STATS_STEEPNESS', 'Steepness', (v) => `${v}`],
  ['SPECIES_WEIGHT', 'Records', (v) => `×${v}`],
  ['SPECIES_LIVE', 'Records on calls', (v) => `×${v}`],
  ['PAL_H2H_WEIGHT', 'Pal H2H', (v) => `×${v}`],
  ['COOLDOWN_EXP', 'Cooldowns', (v) => `^${v}`],
  ['HIT_WEIGHT', 'Real hit size', (v) => `^${v}`],
  ['PAL_RANK_WEIGHT', 'Pal rank', (v) => `×${v}`],
];

// a fight still to come: no winner yet
const pending = (c) => c.winner == null;

// the site's pal rank for a side, when the call read it
const rankOf = (c, s) => c.fighters?.[s === 'home' ? 0 : 1]?.pal_rank;
const rankTitle = (r) => `Tier ${r.tier} · ranked ${r.rank} of ${r.of} species · power ${r.power}/100`;
const rankGap = (c) => {
  const [h, a] = SIDES.map((s) => rankOf(c, s));
  return h && a ? `#${fmt(h.rank)} vs #${fmt(a.rank)} of ${fmt(h.of)}` : 'pal ranks';
};

// a side of the headline: orb (crowned if it won), name, owner, types, and whether Augur picked it. Before the bell
// the side Augur didn't pick is the one dimmed
function side(d, s) {
  const c = d.call;
  const f = c.fighters?.[s === 'home' ? 0 : 1];
  const lit = pending(c) ? c.pick === s : c.winner === s;
  const cls = s === 'home' ? 'h' : 'a';
  const owner = s === 'home' ? c.home_owner : c.away_owner;
  const result = pending(c) ? '' : c.winner === s ? '<span class="ctag won">Won</span>' : '<span class="ctag lost">Lost</span>';
  const r = rankOf(c, s);
  const tier = r ? `<span class="ctag tier tier-${esc(r.tier).toLowerCase()}" title="${esc(rankTitle(r))}">${esc(r.tier)}</span>` : '';
  const tags = `${tier}${c.pick === s ? `<span class="ctag pick">Augur's pick</span>` : ''}${result}`;
  return `<div class="fx ${s === 'away' ? 'r' : ''}">${orb(pal(c, s), d.slugs[s === 'home' ? 0 : 1], cls + (lit ? '' : ' dim'), 68, !pending(c) && lit)}
    <div><div class="ctags">${tags || '<span class="ctag none"></span>'}</div><div class="nm" style="${lit ? '' : 'color:var(--t2)'}">${esc(pal(c, s))}</div>
      <div class="ow">${face(owner, 'a av', 18)}${s.toUpperCase()} · ${esc(owner)}</div>
      ${f ? `<div class="els">${f.types.map((t) => `<span class="el">${esc(t)}</span>`).join('')}</div>` : ''}</div></div>`;
}

// what's worth knowing about how the call was made: a re-call after the bell, hidden stats, a late call
function notes(d) {
  const c = d.call;
  const out = [];
  if (c.late && c.late[0] !== c.pick) out.push(`<span class="gold">After the bell</span> (real stats, not counted): ${esc(pal(c, c.late[0]))} ${pct(c.late[1])}`);
  if (c.late_only) out.push(pending(c) ? 'Called after picks closed: it won’t count in the tally' : 'First called after picks closed: not in the tally');
  SIDES.forEach((s, i) => {
    const from = d.stats_from[i];
    if (!from || from === 'shown') return;
    const text = `${esc(pal(c, s))}'s stats are hidden until the bell: ${esc(from.replace(/^from their box$/, 'taken from its owner’s box'))}`;
    out.push(pending(c) ? (/^guessed/.test(from) ? `<span class="gold">${text}</span>` : text) : `${esc(pal(c, s))}'s hidden stats: ${esc(from.replace(/^from their box$/, 'taken from its owner’s box'))}`);
  });
  if (d.estimated && !d.stats_from.some((x) => x && x !== 'shown')) out.push('Some stats were hidden until the bell and guessed');
  return out.length ? `<div class="cnotes">${out.map((n) => `<span title="${esc(n.replace(/<[^>]+>/g, ''))}">${n}</span>`).join('')}</div>` : '';
}

// Both pals face to face, Augur's pick and its chance in the middle. verdict goes under the pick's name (History:
// right or missed; Live: the bet), line under the dial (History: how it ended; Live: why the bet), then the notes.
// chance: the dial's, when it isn't the call's own (Live shows the chance as it stands)
export function headline(d, { verdict = '', line = '', extra = '', chance } = {}) {
  const c = d.call;
  return `<div class="stage cstage${pending(c) ? ' pending' : ''}">${side(d, 'home')}
    <div class="cmid"><div class="cverdict">${dial(chance ?? c.prob, 80)}<div>
      <div class="cpick"><span class="label">${pending(c) ? 'Augur calls' : 'Augur called'}</span><b>${esc(pal(c, c.pick))}</b></div>${verdict}</div></div>
      ${line ? `<div class="cwon">${line}</div>` : ''}${extra}${notes(d)}</div>
    ${side(d, 'away')}</div>`;
}

// what went into the stats factor, biggest first: each a small bar leaning to the side it helped
function parts(m, c) {
  if (!m.parts?.length) return '';
  const most = Math.max(...m.parts.map((p) => Math.abs(p.logit)), 0.01);
  return `<div class="cparts">${m.parts
    .slice(0, 5)
    .map((p) => {
      const side = p.logit >= 0 ? 'home' : 'away';
      const w = (50 * Math.abs(p.logit)) / most;
      return `<div class="cpart" title="${esc(p.detail)}"><span>${esc(p.label)}</span><div class="cpb"><i class="${side}" style="${side === 'home' ? `right:50%` : `left:50%`};width:${w.toFixed(1)}%"></i></div><small>${esc(pal(c, side))}</small></div>`;
    })
    .join('')}</div>`;
}

// The model's sum on the log-odds scale: each factor moves the call toward home (left) or away (right), then the
// total. footer goes at its foot (the model it used), or on a tab of its own (tabbed)
export function waterfall(d, footer = modelFooter(d), tabbed = false) {
  const c = d.call;
  const m = d.model;
  if (!m) return `<div class="panel col grow"><div class="ph"><h3>How Augur called it</h3></div>${empty('eye', 'No numbers kept for this call', 'It was logged before Augur saved its factors and fighters.')}${footer}</div>`;
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
    rank: rankGap(c),
  };
  const names = { stats: 'Stats', species: 'Records', h2h: 'Pal H2H', rank: 'Pal rank' };
  const rows = steps
    .map(([a, b, f]) => {
      const [l, r] = [Math.min(at(a), at(b)), Math.max(at(a), at(b))];
      const lean = f.logit >= 0 ? 'home' : 'away';
      return `<div class="wf"><div class="wl"><b>${names[f.key]}${f.weight !== 1 ? ` <em>×${f.weight}</em>` : ''}</b><small>${detail[f.key]}</small></div>
        <div class="wt"><i class="${lean}" style="left:${l}%;width:${Math.max(0.6, r - l)}%"></i><s style="left:${at(b)}%"></s></div>
        <div class="wv"><b>${pct(f.p_home > 0.5 ? f.p_home : 1 - f.p_home)}</b><small>${esc(pal(c, f.p_home >= 0.5 ? 'home' : 'away'))}</small></div></div>${f.key === 'stats' ? parts(m, c) : ''}`;
    })
    .join('');
  const fav = m.p_home >= 0.5 ? 'home' : 'away';
  const total = `<div class="wf total"><div class="wl"><b>Together</b><small>log-odds ${signedLogit(m.total)}</small></div>
    <div class="wt"><i class="gold" style="left:${Math.min(50, at(m.total))}%;width:${Math.abs(at(m.total) - 50)}%"></i></div>
    <div class="wv"><b class="gold">${pct(fav === 'home' ? m.p_home : 1 - m.p_home)}</b><small>${esc(pal(c, fav))}</small></div></div>`;
  const sum = `${m.summary ? `<p class="csum">${esc(m.summary)}</p>` : ''}${rows}${total}
    ${m.rescored ? '<div class="small">Re-run on the fighters it saw with its settings: the call didn’t keep its factors.</div>' : ''}`;
  if (tabbed && footer)
    return `<div class="panel col grow"><div class="ph"><h3>How Augur called it</h3><div class="seg sm"><button class="on" data-wv="sum">Sum</button><button data-wv="model">Model used</button></div></div>
    <div class="fillin" data-wvp="sum">${sum}</div><div class="fillin" data-wvp="model" hidden>${footer}</div></div>`;
  return `<div class="panel col grow"><div class="ph"><h3>How Augur called it</h3><span>home ◂ ▸ away</span></div>
    <div class="fillin">${sum}${footer}</div></div>`;
}

// the settings the call was made with, shown at the foot of How Augur called it: one panel fewer, so the page fits
// small windows
export function modelFooter(d) {
  const c = d.call;
  const s = d.settings;
  const chips = s ? SETTINGS.filter(([k]) => s[k] != null).map(([k, l, f]) => `<span class="chip">${l} <b>${f(s[k])}</b></span>`).join('') : '<span class="small">Not saved with this call.</span>';
  const now = pending(c);
  const stake = d.stake_cmd ? `${now ? 'suggests' : 'suggested'} <b class="gold">${esc(d.stake_cmd)}</b>` : c.stake === 0 ? (now ? 'says sit it out' : 'said sit it out') : '';
  return `<div class="cmodel"><div class="cmh"><span class="label">${now ? 'The model it’s using' : 'The model it used'}</span><span class="small">${c.version ? `Augur ${esc(c.version)}` : 'version not recorded'}</span></div>
    <div class="chips">${chips}</div>
    <div class="small cmeta">${[stake, c.fought_at ? `fought ${esc(c.fought_at.slice(0, 16).replace('T', ' '))} UTC` : '', d.fight ? `duel #${d.fight.id}` : ''].filter(Boolean).join(' · ')}</div></div>`;
}

// the tale of the tape: stats, talent rolls, move strength, passives and the type-edge moves
// the site's pal rank for each side: the rank in figures, the bars by its power score (a lower rank is stronger)
function rankRow(c) {
  const [h, a] = SIDES.map((s) => rankOf(c, s));
  if (!h && !a) return '';
  const mx = Math.max(h?.power || 0, a?.power || 0) || 1;
  const lo = (me, them) => (me && them && me.rank > them.rank ? 'lo' : '');
  const val = (r) => (r ? `<span title="${esc(rankTitle(r))}">#${fmt(r.rank)}</span>` : '—');
  return `<div class="r5 rank"><span class="n l ${lo(h, a)}">${val(h)}</span><div class="bar bl"><i style="width:${((h?.power || 0) / mx) * 100}%"></i></div><span class="k">PAL RANK</span><div class="bar br"><i style="width:${((a?.power || 0) / mx) * 100}%"></i></div><span class="n ${lo(a, h)}">${val(a)}</span></div>`;
}

export function tape(d) {
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
    ${rankRow(c)}
    ${h.passives.length || a.passives.length ? `<div class="cpass">${[h, a].map((f) => `<div class="pills">${f.passives.length ? passivePills(f.passives) : '<span class="small">no passives</span>'}</div>`).join('')}</div>` : '<div class="small cnopass">Neither has passives</div>'}
    ${edgeLine}${moves}</div></div>`;
}
