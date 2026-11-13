// A pal box card, as Players (anyone's box) and You › Pals (yours, with what a reroll would do) both show it: the
// pal, its tier and power (the site's rating for its species), its three talent rolls and its passives.
import { esc, fmt } from './format.js';
import { art, passivePills } from './pal.js';

export const STATS = ['HP', 'ATK', 'DEF'];
// a talent roll's colour: green from 80, gold from 50, red below
export const ivTone = (x) => (x >= 80 ? 'var(--ok)' : x >= 50 ? 'var(--gold)' : 'var(--bad)');

// the three rolls side by side: label, value, bar
export const ivRow = (ivs) =>
  `<div class="bxi">${STATS.map((k, i) => {
    const v = Math.round(ivs[i] ?? 0);
    return `<div style="--c:${ivTone(v)};--v:${v}%"><span>${k}</span><b>${fmt(v)}</b><i></i></div>`;
  }).join('')}</div>`;

export const ivsOf = (p) => STATS.map((k) => p.stats?.[k]?.[1] ?? 0);

// columns for n cards: the box fills its panel, about square cards, the last row centred
export const boxCols = (n) => Math.max(1, Math.min(5, n <= 3 ? n : Math.round(Math.sqrt(n * 1.4))));

// the box around the cards, `cols` across. Each card is two of the grid's columns wide, so an empty cell half a card
// wide per missing card centres a short last row
export function boxGrid(cards, cols, cls = '') {
  const short = cards.length % cols ? cols - (cards.length % cols) : 0;
  const gap = short ? `<i class="bxgap" style="grid-column: span ${short}"></i>` : '';
  const last = cards.length - (cards.length % cols || cols);
  const id = `bx-${Math.random().toString(36).slice(2)}`;
  queueMicrotask(() => stackWhenRoomy(id));
  return `<div class="fillin bxbox ${cls}" id="${id}" style="grid-template-columns: repeat(${cols * 2}, minmax(0, 1fr))">${cards.slice(0, last).join('')}${gap}${cards.slice(last).join('')}</div>`;
}

// one card: its art faded in behind (once it's cached), a header, its rolls and a foot (the box lines these up across a row); `extra` goes in the foot, `attrs` on the card itself (a click target, a selection), `passives: false`
// leaves them off (they don't change with a reroll, so You › Pals shows them with the reroll instead)
export function boxCard(p, { extra = '', attrs = '', cls = '', title = '', passives = true } = {}) {
  const tier = (p.tier || '').toLowerCase();
  const pills = passives ? `${passivePills(p.passives || [])}${p.rerolls ? `<span class="pill">rerolled ×${fmt(p.rerolls)}</span>` : ''}` : '';
  return `<div class="bxp ${tier ? `tier-${esc(tier)}` : ''} ${cls}" ${attrs} title="${esc(title)}">${art(p.pal, p.slug, 'bxart')}
    <div class="bxh">
      <div class="bxn"><b>${esc(p.pal)}</b><small>${tier ? `<span class="tierb">${esc(p.tier)}</span>` : ''}${p.alpha ? '<span class="alpha">alpha</span>' : ''}</small></div>
      <div class="bxpw" title="The site's power rating for ${esc(p.pal)}"><b>${p.power != null ? fmt(p.power) : '—'}</b><small>power</small></div></div>
    ${ivRow(ivsOf(p))}
    ${pills || extra ? `<div class="bxft">${pills ? `<div class="bxf pills">${pills}</div>` : ''}${extra}</div>` : ''}</div>`;
}

// a box with height to spare (a few pals in a tall panel) stacks each card's rolls, a bar a line, rather than leave
// the cards half empty; side by side again whenever stacked wouldn't fit. The box's own size is set by its panel, so
// the switch never resizes it
function stackWhenRoomy(id) {
  const el = document.getElementById(id);
  if (!el) return;
  const fit = () => {
    el.classList.add('roomy');
    if (el.scrollHeight > el.clientHeight + 1) el.classList.remove('roomy');
  };
  // straight away, so the first paint is the right one, then whenever the window resizes it
  fit();
  const watch = new ResizeObserver(() => (el.isConnected ? fit() : watch.disconnect()));
  watch.observe(el);
}
