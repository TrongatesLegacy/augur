// A pal box card, as Players (anyone's box) and You › Pals (yours, with what a reroll would do) both show it: the
// pal, its tier and power (the site's rating for its species), its three talent rolls and its passives.
import { esc, fmt } from './format.js';
import { orb, passivePills } from './pal.js';

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

// one card; `extra` goes at its foot, `attrs` on the card itself (a click target, a selection), `passives: false`
// leaves them off (they don't change with a reroll, so You › Pals shows them with the reroll instead)
export function boxCard(p, { extra = '', attrs = '', cls = '', title = '', passives = true } = {}) {
  const tier = (p.tier || '').toLowerCase();
  const pills = passives ? `${passivePills(p.passives || [])}${p.rerolls ? `<span class="pill">rerolled ×${fmt(p.rerolls)}</span>` : ''}` : '';
  return `<div class="bxp ${tier ? `tier-${esc(tier)}` : ''} ${cls}" ${attrs} title="${esc(title)}">
    <div class="bxh">${orb(p.pal, p.slug, 'h av', 36)}
      <div class="bxn"><b>${esc(p.pal)}</b><small>${tier ? `<span class="tierb">${esc(p.tier)}</span>` : ''}${p.alpha ? '<span class="alpha">alpha</span>' : ''}</small></div>
      <div class="bxpw" title="The site's power rating for ${esc(p.pal)}"><b>${p.power != null ? fmt(p.power) : '—'}</b><small>power</small></div></div>
    ${ivRow(ivsOf(p))}
    ${pills ? `<div class="bxf pills">${pills}</div>` : ''}${extra}</div>`;
}
