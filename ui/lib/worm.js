// "Who's ahead": a fight as one line, the HP gap between the two sides over time (home's HP left minus away's), above
// the middle while home leads and below while away does. A stem for each hit, the biggest labelled, the K.O. marked,
// Augur's projection dashed behind it, and on hover the moment: both sides' HP and that second's hits. Live draws the
// fight that's on from the live feed; a call's breakdown draws its stored tape.
import { esc } from './format.js';

const clock = (t) => `${Math.floor(t / 60)}:${String(Math.round(t % 60)).padStart(2, '0')}`;
const W = 520;
const L = 6;
const R = 6;
const T = 12;
const B = 20;

// Augur's projection as the gap at the K.O.: each side's HP left when the first K.O. lands (the model's times aren't
// seconds, so it's put at the real K.O.), home's minus away's
export function projectedGap(score) {
  const first = score && Math.min(score.t_home, score.t_away);
  if (!(first > 0) || !Number.isFinite(first)) return null;
  return 100 * ((1 - first / score.t_away) - (1 - first / score.t_home));
}

// each hit in order, with both sides' HP after it (from 100) and what made it stand out
export function moments(hits) {
  const hp = { home: 100, away: 100 };
  const sorted = [...hits].sort((a, b) => a.t - b.t);
  const biggest = Math.max(0, ...sorted.map((h) => h.pct));
  const first = {};
  let topTagged = false;
  return sorted.map((h) => {
    const target = h.side === 'home' ? 'away' : 'home';
    const was = hp[target];
    hp[target] = Math.max(0, Math.round((hp[target] - h.pct) * 10) / 10);
    const tags = [];
    if (!first[h.side]) (first[h.side] = true), tags.push('First hit');
    if (!topTagged && h.pct === biggest && biggest > 0) (topTagged = true), tags.push('Biggest');
    if (was > 0 && hp[target] === 0) tags.push('K.O.');
    return { side: h.side, t: h.t, move: h.move || '', pct: h.pct, home: hp.home, away: hp.away, tags };
  });
}

// fight: { hits, end (seconds shown), ko (seconds, or null), winner ('home'/'away' once it's over) }
// opts: { names: [home pal, away pal], proj: Augur's projected final gap (home minus away, -100..100), projAt: when it
// lands (seconds; null draws it as a mark at the right edge), live: still being fought, h: the chart's height in its
// 520-wide drawing, adv: the advantage bar under it }
export function worm(fight, { names, proj = null, projAt = null, live = false, h: H = 150, adv: showAdv = true }) {
  const ms = moments(fight.hits || []);
  const last = ms[ms.length - 1];
  const end = Math.max(10, fight.end || 0, (last?.t || 0) + 1, fight.ko || 0);
  const gap = (m) => (m ? m.home - m.away : 0);
  const lead = gap(last);
  const top = Math.max(25, ...ms.map((m) => Math.abs(gap(m))), Math.abs(proj ?? 0)) * 1.12;
  const mid = T + (H - T - B) / 2;
  const x = (t) => +(L + ((W - L - R) * Math.min(t, end)) / end).toFixed(1);
  const y = (v) => +(mid - (v / top) * ((H - T - B) / 2)).toFixed(1);
  // the line holds its value until each hit, then moves to the new gap (damage lands at once)
  let d = `M${x(0)} ${y(0)}`;
  let prev = 0;
  for (const m of ms) {
    const g = gap(m);
    if (g === prev) continue;
    d += ` L${x(Math.max(0, m.t - 0.6))} ${y(prev)} L${x(m.t)} ${y(g)}`;
    prev = g;
  }
  d += ` L${x(fight.ko ?? end)} ${y(prev)}`;
  const area = `${d} L${x(fight.ko ?? end)} ${mid} L${x(0)} ${mid} Z`;
  const id = `w${Math.random().toString(36).slice(2, 8)}`;
  const half = top / 1.12 >= 50 ? 50 : top / 1.12 >= 25 ? 25 : 10;
  const step = end <= 40 ? 10 : end <= 90 ? 20 : 30;
  const ticks = [];
  for (let t = 0; t <= end - step / 3; t += step) ticks.push(t);
  // stems at each hit, in the hitter's colour; the hits worth reading get their damage
  const worth = new Set(ms.filter((m) => m.tags.includes('Biggest') || m.pct >= 15).map((m) => m));
  const stems = ms
    .map((m) => {
      const yv = y(gap(m));
      const label = worth.has(m) ? `<g class="wl ${m.side}"><circle cx="${x(m.t)}" cy="${yv}" r="8.5"/><text x="${x(m.t)}" y="${yv + 3.2}" text-anchor="middle">${Math.round(m.pct)}</text></g>` : '';
      return `<line class="ws ${m.side}" x1="${x(m.t)}" x2="${x(m.t)}" y1="${mid}" y2="${yv}"/>${label}`;
    })
    .join('');
  const ko = fight.ko != null ? `<line class="wko" x1="${x(fight.ko)}" x2="${x(fight.ko)}" y1="${T - 4}" y2="${H - B}"/><text class="wkot" x="${x(fight.ko) - 3}" y="${T + 6}" text-anchor="end">K.O. ${clock(fight.ko)}</text>` : '';
  const projLine =
    proj == null
      ? ''
      : projAt != null
        ? `<path class="wproj" d="M${x(0)} ${mid} L${x(projAt)} ${y(proj)}"><title>Augur's projection: ${esc(proj >= 0 ? names[0] : names[1])} ${proj === 0 ? 'level' : `ahead by ${Math.round(Math.abs(proj))}`}</title></path>`
        : `<g class="wproj-m"><line class="wproj" x1="${x(end * 0.78)}" x2="${x(end)}" y1="${y(proj)}" y2="${y(proj)}"/><text class="wprojt" x="${x(end)}" y="${y(proj) + (proj >= 0 ? -4 : 10)}" text-anchor="end">Augur</text><title>Augur's projection at the K.O.: ${esc(proj >= 0 ? names[0] : names[1])} ahead by ${Math.round(Math.abs(proj))}</title></g>`;
  const svg = `<svg class="wsvg" viewBox="0 0 ${W} ${H}" role="img" aria-label="Who's ahead over the fight: ${esc(lead === 0 ? 'level' : `${lead > 0 ? names[0] : names[1]} ahead by ${Math.round(Math.abs(lead))}`)}">
    <defs><clipPath id="${id}h"><rect x="0" y="0" width="${W}" height="${mid}"/></clipPath><clipPath id="${id}a"><rect x="0" y="${mid}" width="${W}" height="${H - mid}"/></clipPath></defs>
    <g class="wg">${[half, -half].map((v) => `<line x1="${L}" x2="${W - R}" y1="${y(v)}" y2="${y(v)}"/>`).join('')}<line class="wmid" x1="${L}" x2="${W - R}" y1="${mid}" y2="${mid}"/></g>
    <text class="wside h" x="${L + 2}" y="${T + 9}">${esc(names[0])}</text><text class="wside a" x="${L + 2}" y="${H - B - 5}">${esc(names[1])}</text>
    <g class="wt">${ticks.map((t) => `<text x="${x(t)}" y="${H - 5}" text-anchor="${t === 0 ? 'start' : 'middle'}">${clock(t)}</text>`).join('')}</g>
    ${projLine}${ko}
    <path class="wf h" d="${area}" clip-path="url(#${id}h)"/><path class="wf a" d="${area}" clip-path="url(#${id}a)"/>
    ${stems}
    <path class="wline h" d="${d}" clip-path="url(#${id}h)"/><path class="wline a" d="${d}" clip-path="url(#${id}a)"/>
    ${live && !fight.ko ? `<circle class="wnow ${lead >= 0 ? 'h' : 'a'}" cx="${x(end)}" cy="${y(lead)}" r="4"/>` : ''}
    <line class="wx" x1="0" x2="0" y1="${T - 4}" y2="${H - B}" hidden/><circle class="wdot" r="4.5" cx="0" cy="0" hidden/>
    <rect class="whit" x="0" y="0" width="${W}" height="${H}"/></svg>`;
  const leader = lead === 0 ? '' : lead > 0 ? 'h' : 'a';
  const head = `<div class="wh"><span class="wsub">${
    fight.winner
      ? `<b class="${fight.winner === 'home' ? 'h' : 'a'}">${esc(fight.winner === 'home' ? names[0] : names[1])}</b> wins${fight.ko != null ? ` by K.O. at ${clock(fight.ko)}` : ''}`
      : last
        ? `<b class="${last.side === 'home' ? 'h' : 'a'}">${esc(last.side === 'home' ? names[0] : names[1])}</b> ${esc(last.move || 'hit')} ${Math.round(last.pct)}`
        : live
          ? 'Fight on: no hits yet'
          : 'No hits'
  }</span><b class="wlead ${leader}">${lead === 0 ? 'Level' : `${esc(lead > 0 ? names[0] : names[1])} +${Math.round(Math.abs(lead))}`}</b></div>`;
  // the advantage bar: the gap as a share of a whole HP bar, from the middle
  const share = Math.min(50, Math.abs(lead) / 2);
  const adv = `<div class="wadv"><span class="h">${esc(names[0])}</span><div class="wbar"><i class="${leader}" style="left:${lead >= 0 ? 50 - share : 50}%;width:${share}%"></i></div><span class="a">${esc(names[1])}</span></div>`;
  const data = esc(JSON.stringify({ end, names, ko: fight.ko ?? null, ms, top, mid, h: H }));
  return `<div class="worm" data-w="${data}">${head}<div class="wbox">${svg}<div class="wtip" hidden></div></div>${showAdv ? adv : ''}</div>`;
}

// Hover: a line down the chart at the mouse, a dot on the gap, and a card with the time, both sides' HP and that
// moment's hits. Snaps to a hit within a second and a half.
export function wireWorm(root) {
  root.querySelectorAll('.worm').forEach((el) => {
    const d = JSON.parse(el.dataset.w);
    const svg = el.querySelector('.wsvg');
    const tip = el.querySelector('.wtip');
    const cross = svg.querySelector('.wx');
    const dot = svg.querySelector('.wdot');
    const xOf = (t) => L + ((W - L - R) * Math.min(t, d.end)) / d.end;
    const yOf = (v) => d.mid - (v / d.top) * ((d.h - T - B) / 2);
    const hide = () => ((tip.hidden = true), (cross.hidden = true), (dot.hidden = true));
    const show = (e) => {
      const box = svg.getBoundingClientRect();
      const t = Math.max(0, Math.min(d.ko ?? d.end, (((e.clientX - box.left) / box.width) * W - L) / ((W - L - R) / d.end)));
      const near = d.ms.reduce((best, m) => (Math.abs(m.t - t) <= 1.5 && (!best || Math.abs(m.t - t) < Math.abs(best.t - t)) ? m : best), null);
      const at = near ? near.t : Math.round(t);
      const state = [...d.ms].reverse().find((m) => m.t <= at) || { home: 100, away: 100 };
      const hits = d.ms.filter((m) => m.t === at);
      const gap = state.home - state.away;
      const px = xOf(at);
      cross.setAttribute('x1', px);
      cross.setAttribute('x2', px);
      dot.setAttribute('cx', px);
      dot.setAttribute('cy', yOf(gap));
      dot.setAttribute('class', `wdot ${gap >= 0 ? 'h' : 'a'}`);
      cross.hidden = dot.hidden = false;
      const name = (s) => esc(s === 'home' ? d.names[0] : d.names[1]);
      tip.innerHTML = `<b>${clock(at)}</b><div class="wtv"><span class="h">${name('home')} ${Math.round(state.home)}%</span><span class="a">${name('away')} ${Math.round(state.away)}%</span></div>${hits
        .map((m) => `<div class="wtr"><span class="${m.side === 'home' ? 'h' : 'a'}">${name(m.side)}</span> ${esc(m.move || 'unnamed hit')} <b>${Math.round(m.pct)}</b>${m.tags.map((g) => `<i>${esc(g)}</i>`).join('')}</div>`)
        .join('')}`;
      tip.hidden = false;
      // the card sits beside the line, on whichever side has room
      const left = (px / W) * box.width;
      tip.style.left = left > box.width * 0.55 ? '' : `${left + 12}px`;
      tip.style.right = left > box.width * 0.55 ? `${box.width - left + 12}px` : '';
    };
    svg.addEventListener('mousemove', show);
    svg.addEventListener('mouseleave', hide);
  });
}
