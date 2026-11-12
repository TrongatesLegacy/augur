// You › Titles: every title on PalDuels by category, as Augur last read your profile. What you've earned (the rare
// ones picked out), what's closest, and each category's titles: ladders as steps, crowns with who holds them, the
// element ring as its nine elements.
import { esc, fmt, pct } from '../lib/format.js';
import { face } from '../lib/pal.js';
import { empty, ring } from '../lib/viz.js';

const ELEMENT = { neutral: '#d8cfb0', fire: '#ff7a45', water: '#4fa8ff', grass: '#6fdc5a', electric: '#ffd84f', ice: '#8fe8ff', ground: '#d09a5e', dark: '#a77bff', dragon: '#e46cff' };

const share = (p) => (p && p[1] ? Math.min(1, p[0] / p[1]) : 0);
const count = (p) => `${fmt(p[0])}<small>/${fmt(p[1])}</small>`;
const bar = (p) => `<div class="tbar"><i style="width:${(100 * share(p)).toFixed(1)}%"></i></div>`;
// how many chatters hold an earned title, from its badge's tooltip ("…, held by 13 chatters")
const heldBy = (b) => +(b?.about.match(/held by ([\d,]+) chatter/)?.[1]?.replace(/,/g, '') || 0) || null;
const key = (s) => String(s || '').trim().toUpperCase();
// a ladder or the element ring counts each of its titles
const size = (t) => Math.max(1, t.steps.length);

function card(t, ctx) {
  const badge = ctx.badges.get(key(t.name));
  const rare = badge?.rare ? '<span class="trare">Rare</span>' : '';
  if (t.kind === 'ladder') return ladder(t, ctx);
  if (t.kind === 'ring') return elements(t);
  if (t.kind === 'crown') {
    const mine = t.holders.some((h) => key(h) === key(ctx.me));
    return `<div class="tc tcrown ${mine ? 'got' : ''}" title="${esc(t.about)}"><span class="tci">♛</span>
      <div class="tcb"><b>${esc(t.name)}</b><small>${esc(t.about)}</small>
        <span class="thold">${t.holders.length ? (mine ? 'You hold it' : `${face(t.holders[0], 'av', 18)}<span>Held by <b>${esc(t.holders.join(', '))}</b></span>`) : '<span>Nobody holds it yet</span>'}</span></div>
      <span class="tcs">Crown</span></div>`;
  }
  const state = t.earned ? 'got' : t.progress ? 'prog' : 'dim';
  const held = heldBy(badge);
  return `<div class="tc ${state} ${badge?.rare ? 'rare' : ''}" title="${esc(t.about)}"><span class="tci">${esc(t.icon || '★')}</span>
    <div class="tcb"><b>${esc(t.name)}${badge?.times ? ` <em>×${fmt(badge.times)}</em>` : ''}</b><small>${esc(t.about)}</small>${held ? `<small class="tfew">held by ${fmt(held)}</small>` : ''}</div>
    ${t.earned ? `<span class="tcs ok">${rare || '✓'}</span>` : t.progress ? `<span class="tcs num">${count(t.progress)}</span>` : '<span class="tcs">Locked</span>'}
    ${!t.earned && t.progress ? bar(t.progress) : ''}</div>`;
}

function ladder(t, ctx) {
  const got = t.steps.filter((s) => s.earned).length;
  return `<div class="tc lad ${t.earned ? 'got' : got ? 'part' : 'dim'} ${t.steps.length > 2 ? 'long' : ''}">
    <div class="tlh"><b>${esc(t.name)}</b><span class="tcs ${t.earned ? 'ok' : ''}">${t.earned ? '✓ Complete' : `${got} of ${t.steps.length}`}</span></div>
    <div class="rungs" style="--n:${t.steps.length};--at:${Math.max(0, got - 1)}">${t.steps
      .map((s, i) => {
        const rare = ctx.badges.get(key(s.name))?.rare;
        const next = !s.earned && (i === 0 || t.steps[i - 1].earned);
        return `<div class="rung ${s.earned ? 'got' : next ? 'next' : ''} ${rare ? 'rare' : ''}"><i>${esc(s.label)}</i><b>${esc(s.name)}</b>
          <small>${s.earned ? (rare ? 'Rare' : 'Earned') : s.progress ? count(s.progress) : 'Locked'}</small>${next && s.progress ? bar(s.progress) : ''}</div>`;
      })
      .join('')}</div></div>`;
}

// the element ring: one title per element, its progress as a ring round the element's colour
function elements(t) {
  const got = t.steps.filter((s) => s.earned).length;
  const r = 15;
  const c = 2 * Math.PI * r;
  return `<div class="tc ering">
    <div class="tlh"><b>${esc(t.name)}</b><span class="tcs ${got ? 'ok' : ''}">${got} of ${t.steps.length}</span></div>
    <div class="els9">${t.steps
      .map((s) => {
        const f = s.earned ? 1 : share(s.progress);
        return `<div class="el9 ${s.earned ? 'got' : f ? 'part' : ''}" style="--e:${ELEMENT[s.label.toLowerCase()] || 'var(--gold)'}" title="${esc(s.name)}${s.progress ? ` · ${fmt(s.progress[0])} of ${fmt(s.progress[1])}` : s.earned ? ' · earned' : ''}">
          <svg viewBox="0 0 36 36" aria-hidden="true"><circle cx="18" cy="18" r="${r}" class="t"/>${f ? `<circle cx="18" cy="18" r="${r}" class="f" stroke-dasharray="${(c * f).toFixed(1)} ${c.toFixed(1)}"/>` : ''}</svg>
          <span>${s.earned ? '✓' : s.progress ? count(s.progress) : ''}</span><b>${esc(s.label)}</b></div>`;
      })
      .join('')}</div>
    ${t.about ? `<small class="tabout">${esc(t.about)}</small>` : ''}</div>`;
}

function group(g, i, ctx) {
  const [got, of] = g.earned || [g.titles.filter((t) => t.earned).reduce((n, t) => n + size(t), 0), g.titles.reduce((n, t) => n + size(t), 0)];
  // earned first, then the nearest, then the rest; ladders and the ring keep their place at the top
  const rank = (t) => (t.kind === 'ring' ? -3 : t.kind === 'ladder' ? -2 : t.earned ? -1 : t.kind === 'crown' ? 2 : t.progress ? 1 - share(t.progress) : 1.5);
  const titles = [...g.titles].sort((a, b) => rank(a) - rank(b));
  return `<section class="tgrp" id="tg${i}">
    <div class="tgh"><div><h4>${esc(g.name)}</h4>${g.about ? `<small>${esc(g.about)}</small>` : ''}</div>
      <span class="tgc ${of && got === of ? 'ok' : ''}"><b>${fmt(got)}</b>/${fmt(of)}</span><div class="tbar wide"><i style="width:${of ? ((100 * got) / of).toFixed(1) : 0}%"></i></div></div>
    <div class="tcards">${titles.map((t) => card(t, ctx)).join('')}</div></section>`;
}

export function titlesView(y) {
  const pg = y.page;
  const groups = pg.groups || [];
  const [got, of] = pg.titles_earned || [pg.titles.length, groups.reduce((n, g) => n + (g.earned?.[1] || 0), 0)];
  const rare = pg.titles.filter((b) => b.rare);
  const ctx = { badges: new Map(pg.titles.map((b) => [key(b.name), b])), me: y.name };
  // the rarest first: fewest holders
  const earned = [...pg.titles].sort((a, b) => b.rare - a.rare || (heldBy(a) ?? 1e9) - (heldBy(b) ?? 1e9));
  const hero = `<div class="panel thero">
    ${ring(of ? got / of : null, { size: 92, label: fmt(got), sub: `of ${fmt(of)}`, title: `${fmt(got)} of ${fmt(of)} titles earned` })}
    <div class="thw"><b>${fmt(got)} titles earned</b><small>${fmt(Math.max(0, of - got))} still to unlock${of ? ` · ${pct(got / of)} of them all` : ''}</small>
      ${rare.length ? `<div class="trares">${rare.map((b) => `<span class="badge rare" title="${esc(b.about)}"><i>${esc(b.icon)}</i>${esc(b.name)}</span>`).join('')}</div>` : ''}</div>
    <div class="tcats" role="tablist" aria-label="Title categories"><button class="tcat tall-tab" data-tg="all" role="tab"><span>All</span><b>${fmt(got)}<small>/${fmt(of)}</small></b><div class="tbar"><i style="width:${of ? ((100 * got) / of).toFixed(1) : 0}%"></i></div></button>${groups
      .map((g, i) => {
        const [n, m] = g.earned || [0, 0];
        return `<button class="tcat ${m && n === m ? 'done' : ''}" data-tg="${i}" role="tab" title="${esc(g.about)}"><span>${esc(g.name)}</span><b>${fmt(n)}<small>/${fmt(m)}</small></b><div class="tbar"><i style="width:${m ? ((100 * n) / m).toFixed(1) : 0}%"></i></div></button>`;
      })
      .join('')}</div></div>`;
  const closest = (pg.closest || []).length
    ? `<div class="panel tnear"><div class="ph"><h3>Closest to unlock</h3></div><div class="bars">${pg.closest
        .map((q) => `<div class="hbar" title="${esc(q.about)}"><span><i class="tqi">${esc(q.icon)}</i>${esc(q.name)}</span><b>${count([q.have, q.need])}</b><div class="b"><i style="width:${(100 * share([q.have, q.need])).toFixed(1)}%"></i></div></div>`)
        .join('')}</div></div>`
    : '';
  const mine = `<div class="panel col grow"><div class="ph"><h3>Earned</h3><span>rarest first</span></div>
    <div class="tearned fillin">${
      earned.length
        ? earned
            .map((b) => {
              const held = heldBy(b);
              return `<div class="ter ${b.rare ? 'rare' : ''}" title="${esc(b.about)}"><span class="tci">${esc(b.icon)}</span><div><b>${esc(b.name)}${b.times ? ` <em>×${fmt(b.times)}</em>` : ''}</b><small>${held ? `held by ${fmt(held)}` : esc(b.about)}</small></div>${b.rare ? '<span class="trare">Rare</span>' : ''}</div>`;
            })
            .join('')
        : empty('star', 'None yet', 'Win duels and picks to earn them.')
    }</div></div>`;
  const all = `<div class="panel col"><div class="ph"><h3 class="tall-h">Every title</h3><span class="tall-n" data-all="${groups.length ? `${fmt(groups.length)} categories` : ''}">${groups.length ? `${fmt(groups.length)} categories` : ''}</span></div>
    <div class="tall fillin">${groups.length ? groups.map((g, i) => group(g, i, ctx)).join('') : empty('star', 'Not read yet', 'Augur reads every title with your profile, next time it reads your picks.')}</div></div>`;
  return `${hero}<div class="tgrid grow"><div class="stackc">${closest}${mine}</div>${all}</div>`;
}

// the category buttons are tabs: one category at a time, or all of them (remembered while the app is open)
let chosen = 'all';

export function wire(main) {
  const tabs = [...main.querySelectorAll('.tcat[data-tg]')];
  const show = (tg) => {
    chosen = main.querySelector(`.tcat[data-tg="${tg}"]`) ? tg : 'all';
    tabs.forEach((b) => {
      b.classList.toggle('on', b.dataset.tg === chosen);
      b.setAttribute('aria-selected', b.dataset.tg === chosen);
    });
    main.querySelectorAll('.tgrp').forEach((g) => (g.hidden = chosen !== 'all' && g.id !== `tg${chosen}`));
    const list = main.querySelector('.tall');
    if (list) list.scrollTop = 0;
    const head = main.querySelector('.tall-h');
    const g = chosen === 'all' ? null : main.querySelector(`#tg${chosen}`);
    if (head) head.textContent = g ? g.querySelector('h4').textContent : 'Every title';
    // one category: its name is the panel's title, so its own heading row keeps just the blurb and the count
    list?.classList.toggle('one', !!g);
    const note = main.querySelector('.tall-n');
    if (note) note.textContent = g ? `${g.querySelector('.tgc').textContent} earned` : note.dataset.all;
  };
  tabs.forEach((b) => (b.onclick = () => show(b.dataset.tg)));
  show(chosen);
}
