// Live: the duel that's up, built from the same pieces as a call's breakdown in History (the headline, how Augur is
// calling it and the tale of the tape), plus what only Live has: the bet to copy, the projected HP race (the "before"
// of the breakdown's How it went), the head to head, what's next and the chat line.
import { headline, tape, waterfall } from '../lib/callparts.js';
import { copyText } from '../lib/clipboard.js';
import { esc, feedProblem, fmt, pct, short } from '../lib/format.js';
import { SHORT, face, orb, pickVoice, voiceLine, whyLine } from '../lib/pal.js';
import { betCommand } from '../lib/stake.js';
import { columns, empty, icon, ring, signedBar } from '../lib/viz.js';

// the queue. Off air it's a panel of its own; with a duel up it sits compact under the chat line, two to a row
function upNext(live, compact) {
  const q = live?.queue || [];
  if (compact) {
    const body = q.length
      ? `<div class="unx">${q.map((x) => `<div class="unr" title="${esc(x.home)} vs ${esc(x.away)}"><span class="unp">#${x.pos}</span><span class="unn">${esc(x.home)} <i>vs</i> ${esc(x.away)}</span></div>`).join('')}</div>`
      : '<div class="small">Nothing queued: fights show here as chat lines them up.</div>';
    return `<div class="panel un"><div class="ph"><h3>Up next</h3><span>pals show at the bell</span></div>${body}</div>`;
  }
  const body = q.length
    ? `<div class="fillin">${q.map((x) => `<div class="row"><span>${esc(x.home)} <span class="small">vs</span> ${esc(x.away)}</span><span class="small">#${x.pos}</span></div>`).join('')}</div>`
    : empty('queue', 'Nothing queued', 'Fights show here as chat lines them up.');
  return `<div class="panel col list"><div class="ph"><h3>Up next</h3><span>pals show at the bell</span></div>${body}</div>`;
}

// what Augur says to bet, under its pick in the headline: the command with a copy button, the credits, and why
function bet(p, stake, command) {
  const copy = command ? `<button class="btn sm" id="cmd" title="Copy ${esc(command)} for chat">Copy</button>` : '';
  if (!stake) return { verdict: `<div class="lbet"><b>${esc(command)}</b>${copy}</div>`, line: esc(whyLine(p)) };
  if (!stake.amount) return { verdict: `<div class="lbet off"><b>No bet</b>${copy}</div>`, line: esc(stake.why) };
  return { verdict: `<div class="lbet"><b>${esc(stake.command)}</b><span>${fmt(stake.amount)} credits</span>${copy}</div>`, line: esc(stake.why) };
}

// a name as one of the owners: a meeting's sides and the rivalry's records name them as the site shows them
const same = (name, s) => [s.owner, s.owner_key].some((n) => n && n.toLowerCase() === String(name || '').toLowerCase());

// the two owners face to face, their record, their latest meetings, then the two species' own record
function headToHead(p) {
  const r = p.owners;
  const h = p.home;
  const a = p.away;
  const ms = r?.meetings || [];
  // each meeting from this duel's sides: the home owner's pal and the away owner's (the site's order is that duel's)
  const rows = ms.map((m) => {
    const [mh, ma] = same(m.sides[1].owner, h) || same(m.sides[0].owner, a) ? [m.sides[1], m.sides[0]] : m.sides;
    return { when: m.when, h: mh, a: ma };
  });
  const wins = (s, mine) => r?.owners?.find(([n]) => same(n, s))?.[1] ?? rows.filter((x) => x[mine].won).length;
  const [hw, aw] = [wins(h, 'h'), wins(a, 'a')];
  const n = hw + aw || ms.length;
  const since = r?.since ? new Date(r.since + 'T12:00:00Z').toLocaleDateString(undefined, { day: 'numeric', month: 'short' }) : '';
  const sub = !r ? 'owners · pals' : n ? `${fmt(n)} meeting${n === 1 ? '' : 's'}${since ? ` since ${esc(since)}` : ''}` : 'first meeting';
  const owner = (s, cls) => `<div class="hho ${cls}">${face(s.owner, `${cls} hf`, 22)}<span title="${esc(s.owner)}">${esc(s.owner)}</span></div>`;
  const lead = hw > aw ? 'h' : aw > hw ? 'a' : '';
  const score = n
    ? `<b class="hhs${lead ? ` l${lead}` : ''}" aria-label="${esc(h.owner)} ${fmt(hw)}, ${esc(a.owner)} ${fmt(aw)}"><i class="h">${fmt(hw)}</i>–<i class="a">${fmt(aw)}</i></b>`
    : '<span class="hhx">vs</span>';
  const bar = n ? `<div class="sb hhb">${hw ? `<i class="home" style="flex:${hw}"></i>` : ''}${aw ? `<i class="away" style="flex:${aw}"></i>` : ''}</div>` : '';
  const record = r ? `<div class="hhv${n ? '' : ' new'}">${owner(h, 'h')}${score}${owner(a, 'a')}</div>${bar}` : '';
  const pal = (x, cls) => `<span class="hp ${cls}${x.won ? ' w' : ''}" title="${esc(x.pal)}${x.won ? ' won' : ''}">${x.won ? '<i class="cr" aria-label="won">♛</i>' : ''}${esc(x.pal)}</span>`;
  const list = !r
    ? `<div class="hhe">Owner head to head not loaded</div>`
    : rows.length
      ? `<div class="hhm">${rows.map((x) => `<div class="hhr"><span class="wn">${esc(x.when)}</span>${pal(x.h, 'h')}<span class="vs">vs</span>${pal(x.a, 'a')}</div>`).join('')}</div>`
      : `<div class="hhe">They haven’t met before: this is the first</div>`;
  const ph = p.pal_h2h;
  const met = ph.home_wins + ph.away_wins;
  const palLead = ph.home_wins > ph.away_wins ? 'h' : ph.away_wins > ph.home_wins ? 'a' : '';
  const mid =
    h.pal === a.pal
      ? '<span class="hpn">mirror match</span>'
      : met
        ? `<b class="hhs sm${palLead ? ` l${palLead}` : ''}"><i class="h">${fmt(ph.home_wins)}</i>–<i class="a">${fmt(ph.away_wins)}</i></b>`
        : '<span class="hpn">never met</span>';
  const species = `<div class="hhp"><span class="label">Pals</span>
    <div class="hpo h">${orb(h.pal, h.slug, 'h', 24)}<span title="${esc(h.pal)}">${esc(h.pal)}</span></div>${mid}<div class="hpo a"><span title="${esc(a.pal)}">${esc(a.pal)}</span>${orb(a.pal, a.slug, 'a', 24)}</div></div>`;
  // the body scrolls as one only in a window too short for it; otherwise just the meetings scroll
  return `<div class="panel col hh"><div class="ph"><h3>Head to head</h3><span>${sub}</span></div><div class="fillin hhw">${record}${list}${species}</div></div>`;
}

// The model's race: each side's HP falling at its projected rate. The favourite lands the K.O. first (at t_fav) and
// keeps 1 - t_fav / t_dog of its HP; its line carries on faintly to where it would have fallen. The end values sit in
// a column beside the chart, so they never overlap however close the lines run.
function hpRace(p) {
  const fav = p.pick === 'home' ? p.home : p.away;
  const dog = p.pick === 'home' ? p.away : p.home;
  const [fc, dc] = p.pick === 'home' ? ['h', 'a'] : ['a', 'h'];
  const tFav = p.pick === 'home' ? p.score.t_home : p.score.t_away;
  const tDog = p.pick === 'home' ? p.score.t_away : p.score.t_home;
  // a side that does no damage on paper never lands its K.O. (t is infinite)
  const ok = tFav > 0 && Number.isFinite(tFav) && tDog > 0;
  const left = ok ? Math.max(0, 1 - tFav / tDog) : 0;
  const W = 230;
  const H = 92;
  const L = 28;
  const T = 8;
  const B = 18;
  // a little past the K.O., or to where the favourite would fall when that's sooner
  const end = ok ? Math.min(tDog, tFav * 1.3) : 1;
  const x = (t) => (L + ((W - L - 4) * t) / end).toFixed(1);
  const y = (v) => (T + (H - T - B) * (1 - v)).toFixed(1);
  const ko = ok ? tFav : end;
  const koX = +x(ko);
  const ghost = ok && end > tFav ? `<path class="gh ${fc}" d="M${x(tFav)} ${y(left)} L${x(end)} ${y(1 - end / tDog)}"/>` : '';
  // the underdog's dashed line goes on top: where the two all but meet, both colours still show
  const svg = `<svg class="race" viewBox="0 0 ${W} ${H}" aria-hidden="true">
    <g class="grid">${[1, 0.5, 0].map((v) => `<line x1="${L}" x2="${W - 4}" y1="${y(v)}" y2="${y(v)}"/><text x="${L - 5}" y="${+y(v) + 3.5}" text-anchor="end">${v * 100}%</text>`).join('')}</g>
    <line class="ko" x1="${koX}" x2="${koX}" y1="${T - 4}" y2="${H - B}"/><text class="kot" x="${koX}" y="${H - 5}" text-anchor="${koX > W - 24 ? 'end' : 'middle'}">K.O.</text>
    ${ghost}
    <path class="ln ${fc}" d="M${x(0)} ${y(1)} L${x(ko)} ${y(left)}"/>
    <path class="ln ${dc} dog" d="M${x(0)} ${y(1)} L${x(ko)} ${y(0)}"/>
    <circle class="${fc}" cx="${koX}" cy="${y(left)}" r="3.2"/><circle class="${dc}" cx="${koX}" cy="${y(0)}" r="3.2"/></svg>`;
  const longer = ok ? tDog / tFav - 1 : 0;
  const small = (f) => (f > 0 && f < 0.01 ? '&lt;1%' : pct(f));
  const endCard = (s, cls, value, sub, dashed) =>
    `<div class="re ${cls}"><div class="rn"><i class="${dashed ? 'd' : ''}"></i><span title="${esc(s.pal)}">${esc(s.pal)}</span></div><div class="rv"><b>${value}</b><small>${sub}</small></div></div>`;
  const note = !ok
    ? 'Not enough to project the race'
    : !Number.isFinite(longer)
      ? `<b class="${dc}">${esc(dog.pal)}</b> does no damage on paper`
      : longer < 0.05
        ? `Neck and neck: <b class="${dc}">${esc(dog.pal)}</b> would need only ${small(longer)} longer`
        : `<b class="${dc}">${esc(dog.pal)}</b> would need ${pct(longer)} longer to land its K.O.`;
  return `<div class="panel hr"><div class="ph"><h3>Projected HP race</h3><span>model estimate</span></div>
    <div class="hrw" role="img" aria-label="${esc(fav.pal)} projected to win with about ${pct(left)} HP left">${svg}
      <div class="rends">${endCard(fav, fc, small(left), 'HP left', false)}${endCard(dog, dc, 'K.O.', 'out first', true)}</div></div>
    <div class="small rnote">${note}</div></div>`;
}

export function render(app) {
  const { state, main, header } = app;
  const live = state.live;
  const p = live?.prediction;
  const status = live?.error
    ? `<div class="pill ml" title="${esc(live.error)}"><span class="dot idle"></span>${feedProblem(live.error)[0]}</div>`
    : p
      ? `<div class="pill ml"><span class="dot"></span>${live.live ? 'Fight on' : 'Picks open'}</div>`
      : `<div class="pill ml"><span class="dot idle"></span>${esc(live?.title || 'Waiting')}</div>`;
  if (!p) {
    const r = live?.last_result;
    const last = r
      ? `Last fight: <span class="${r.right ? 'ok' : 'bad'}">${esc(r.winner_pal)} won ${r.right ? '✓' : '✕'}</span> (${esc(r.home_pal)} vs ${esc(r.away_pal)}${r.ko ? `, K.O. ${esc(r.ko)}` : ''}${r.hp_left != null ? `, ${r.hp_left}% HP left` : ''})`
      : 'Augur checks every few seconds and calls the next fight as soon as it’s up.';
    const o = live?.off_air;
    const between = (live?.queue || []).length > 0;
    const ls = o?.last_stream;
    const day = (d) => new Date(d + 'T12:00:00Z').toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' });
    const all = o?.all_time?.[1] ? o.all_time[0] / o.all_time[1] : null;
    const stream = ls
      ? `<div class="panel col"><div class="ph"><h3>${between ? 'Today' : 'Last stream'}</h3><span>${esc(day(ls[0]))}</span></div>
          <div class="yrow">${ring(ls[1] / ls[2], { size: 84, sub: 'right' })}
            <div class="ycol"><div class="qn big"><b style="font-size:${Math.min(46, 330 / (fmt(ls[1]).length + 0.6 * (fmt(ls[2]).length + 1))).toFixed(1)}px">${fmt(ls[1])}<i>/${fmt(ls[2])}</i></b></div><span class="small">best run ${fmt(ls[3])}${o.all_time[1] ? ` · all time ${fmt(o.all_time[0])}/${fmt(o.all_time[1])}` : ''}</span></div></div>
          ${o.last_bets ? `<div class="lsb">${signedBar('You', o.last_bets[0], Math.max(Math.abs(o.last_bets[0]), Math.abs(o.last_bets[1]), 1))}${signedBar('Following Augur', o.last_bets[1], Math.max(Math.abs(o.last_bets[0]), Math.abs(o.last_bets[1]), 1))}</div>` : ''}</div>`
      : `<div class="panel col"><div class="ph"><h3>Last stream</h3></div>${empty('chart', 'No streams yet', 'Augur keeps score from the first call.')}</div>`;
    const recent = `<div class="panel col list lcalls"><div class="ph"><h3>Latest calls</h3>${o?.recent?.length ? `<span>${o.recent.filter((r) => r[2]).length} of ${o.recent.length} right</span>` : ''}</div>${
      o?.recent?.length
        ? `<div class="fillin">${o.recent.map(([pick, other, right, prob]) => `<div class="row"><span><span class="res ${right ? 'o' : 'x'}">${right ? '✓' : '✕'}</span> <b>${esc(pick)}</b> <span class="muted">over</span> ${esc(other)}</span><span class="small"><span class="cond lp">${prob ? pct(prob) : ''}</span> <span class="${right ? 'ok' : 'bad'}">${right ? 'Right' : 'Missed'}</span></span></div>`).join('')}</div>`
        : empty('swords', 'No calls yet', 'The first fight Augur calls shows here.')
    }</div>`;
    const known = o
      ? `<div class="known"><span class="label">What Augur knows</span><div>${[
          [o.store[0], 'duels', 'swords'],
          [o.store[1], 'fight tapes', 'chart'],
          [o.store[2], 'species', 'egg'],
          [o.store[3], 'your bets', 'star'],
        ]
          .map(([n, l, k]) => `<div title="from palduels.com">${icon(k, 18)}<b>${fmt(n)}</b><small>${l}</small></div>`)
          .join('')}</div></div>`
      : '';
    const hero = `<div class="panel offair"><div><b>${live?.error ? feedProblem(live.error)[0] : between ? 'Between duels' : 'Off air'}</b><span>${live?.error ? feedProblem(live.error)[1] : last}</span>
      <small>Every call stands on stats, types, moves and this history, never on chat.</small></div>${known}</div>`;
    const days = o?.days?.length
      ? `<div class="panel col lstreams"><div class="ph"><h3>Recent streams</h3><span>calls right, one bar per stream${all != null ? ` · dashed: all time ${pct(all)}` : ''}</span></div>${columns(
          o.days.map(([d, r, n]) => ({ v: r / n, label: d.slice(5).replace('-', '/'), top: `${short(r)}/${short(n)}`, title: `${d}: ${fmt(r)}/${fmt(n)} right`, tone: r / n >= 0.5 ? '' : 'bad' })),
          all,
        )}</div>`
      : `<div class="panel col lstreams"><div class="ph"><h3>Recent streams</h3></div>${empty('chart', 'No streams yet', 'One bar per stream, once Augur has called some fights.')}</div>`;
    main.innerHTML = header('Live · PalDuels', status) + hero + `<div class="loff grow">${stream}${upNext(live, false)}${recent}${days}</div>`;
    return;
  }
  const { voices, voice } = pickVoice(state.voice, state.settings);
  state.voice = voice;
  const command = betCommand(p, live.stake);
  const d = live.detail;
  const chat =
    state.settings?.chat_lines === false
      ? ''
      : `<div class="panel lchat"><div class="ph"><h3>Say it in chat</h3><span>${voices.length} voices</span></div>
        <div class="vo" id="vb">${voices.map((v) => `<button class="${v === voice ? 'on' : ''}" data-v="${esc(v)}">${esc(SHORT[v] || v)}</button>`).join('')}</div>
        <div class="lq"><div class="q" id="vq">${esc(voiceLine(p, voice, state.settings?.include_chance !== false))}</div><button class="btn sm" id="cp">Copy</button></div></div>`;
  main.innerHTML =
    header(`Live · ${esc(p.title || 'PalDuels')}`, status) +
    (d
      ? headline(d, { ...bet(p, live.stake, command), chance: p.prob }) +
        `<div class="cgrid lgrid grow">
          <div class="stackc">${hpRace(p)}${headToHead(p)}</div>
          <div class="stackc">${waterfall(d)}</div>
          <div class="stackc">${tape(d)}${chat}${upNext(live, true)}</div>
        </div>`
      : `<div class="panel col grow">${empty('eye', 'Working out the call', 'The breakdown shows as soon as Augur has made it.')}</div>`);
  // a panel with more below fades out at its foot, until it's scrolled to the end
  main.querySelectorAll('.lgrid .fillin').forEach((el) => {
    const more = () => el.classList.toggle('more', el.scrollTop + el.clientHeight < el.scrollHeight - 2);
    el.addEventListener('scroll', more, { passive: true });
    more();
  });
  main.querySelectorAll('#vb button').forEach((b) => {
    b.onclick = () => {
      state.voice = b.dataset.v;
      render(app);
    };
  });
  document.getElementById('cp')?.addEventListener('click', (e) => copyText(document.getElementById('vq').textContent, e.target));
  document.getElementById('cmd')?.addEventListener('click', (e) => copyText(command, e.target));
}
