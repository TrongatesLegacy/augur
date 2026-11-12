// Small pictures the tabs share, inline SVG and CSS only: a ring for a rate, a split bar, a signed bar, bar columns,
// a sparkline, form dots and the empty state.
import { esc, fmt, pct, signed } from './format.js';

const clamp = (x) => Math.max(0, Math.min(1, x || 0));

// A rate as a ring with its number in the middle. frac 0..1 (null: an empty ring); tone: gold, ok, bad, home, away.
export function ring(frac, { size = 72, tone = 'gold', label, sub = '', title = '' } = {}) {
  const r = 26;
  const c = 2 * Math.PI * r;
  const text = label ?? (frac == null ? '—' : pct(frac));
  // long labels (a 147–172 record) shrink to stay on one line inside the ring
  const fit = Math.min(0.27, 1.1 / String(text).length).toFixed(3);
  return `<div class="ring ${tone}" style="--s:${size}px;--f:${fit}" role="img" aria-label="${esc(title || `${text} ${sub}`)}"${title ? ` title="${esc(title)}"` : ''}>
    <svg viewBox="0 0 60 60" aria-hidden="true"><circle cx="30" cy="30" r="${r}" class="t"/>${frac == null ? '' : `<circle cx="30" cy="30" r="${r}" class="f" stroke-dasharray="${(c * clamp(frac)).toFixed(1)} ${c.toFixed(1)}"/>`}</svg>
    <div><b>${text}</b>${sub ? `<small>${sub}</small>` : ''}</div></div>`;
}

// Two counts as one bar (won and lost, followed and deviated), with the counts under each end.
export function split(a, b, la, lb, ta = 'ok', tb = 'bad') {
  return `<div class="split"><div class="sb" role="img" aria-label="${fmt(a)} ${esc(la)}, ${fmt(b)} ${esc(lb)}">${a ? `<i class="${ta}" style="flex:${a}" title="${fmt(a)} ${esc(la)}"></i>` : ''}${b ? `<i class="${tb}" style="flex:${b}" title="${fmt(b)} ${esc(lb)}"></i>` : ''}</div>
    <div class="sk"><span><b>${fmt(a)}</b> ${la}</span><span>${lb} <b>${fmt(b)}</b></span></div></div>`;
}

// Several counts as one stacked bar with a legend: [[n, label, tone], ...]
export function stack(parts) {
  const shown = parts.filter(([n]) => n > 0);
  return `<div class="stack"><div class="sb">${shown.map(([n, l, t]) => `<i class="${t}" style="flex:${n}" title="${fmt(n)} ${esc(l)}"></i>`).join('')}</div>
    <div class="lg">${parts.map(([n, l, t]) => `<span><i class="${t}"></i>${esc(l)} <b>${fmt(n)}</b></span>`).join('')}</div></div>`;
}

// Credits won or lost as a bar from a middle line (right for a gain, left for a loss), scaled to `max`.
export function signedBar(label, n, max) {
  const w = max ? (50 * Math.min(Math.abs(n), max)) / max : 0;
  const tone = n > 0 ? 'ok' : n < 0 ? 'bad' : 'muted';
  return `<div class="sgb"><span>${label}</span><div class="sgt"><i class="${tone}" style="${n < 0 ? 'right' : 'left'}:50%;width:${w}%"></i></div><b class="${tone}">${signed(n)}</b></div>`;
}

// Bar columns that fill their panel's height: [{ v: 0..1, label, sub, title, tone }]. ref: a dashed line at that
// height (a rate to beat). Up to ~30 bars; each stays slim when there are only a few.
export function columns(items, ref) {
  if (!items.length) return '';
  const many = items.length > 14;
  return `<div class="cols ${many ? 'many' : ''}"><div class="plot">${items
    .map((x) => `<span title="${esc(x.title || '')}" style="--h:${Math.max(3, clamp(x.v) * 100).toFixed(1)}%"><i class="${x.tone || ''}"></i>${x.top != null && !many ? `<em>${x.top}</em>` : ''}</span>`)
    .join('')}${ref != null ? `<s style="bottom:${clamp(ref) * 100}%" title="${pct(ref)}"></s>` : ''}</div>
    <div class="xl">${items.map((x, i) => `<span>${many && i % Math.ceil(items.length / 10) ? '' : esc(x.label)}</span>`).join('')}</div></div>`;
}

// A trend as a line with a soft fill under it, values 0..1, oldest first.
export function spark(values, { w = 240, h = 44, tone = 'gold', label = '' } = {}) {
  if (values.length < 2) return '';
  const x = (i) => ((i / (values.length - 1)) * w).toFixed(1);
  const y = (v) => (h - 2 - clamp(v) * (h - 4)).toFixed(1);
  const line = values.map((v, i) => `${i ? 'L' : 'M'}${x(i)} ${y(v)}`).join(' ');
  return `<svg class="spark ${tone}" viewBox="0 0 ${w} ${h}" preserveAspectRatio="none" role="img" aria-label="${esc(label)}"><path class="a" d="${line} L${w} ${h} L0 ${h} Z"/><path class="l" d="${line}" vector-effect="non-scaling-stroke"/></svg>`;
}

// the last few results, oldest first, as dots
export const formDots = (f) => (f?.length ? `<span class="form" role="img" aria-label="Last ${f.length}: ${f.filter(Boolean).length} won">${f.map((w) => `<i class="${w ? 'w' : 'l'}" title="${w ? 'Win' : 'Loss'}"></i>`).join('')}</span>` : '');

const ICONS = {
  queue: '<path d="M4 6h16M4 12h10M4 18h6"/><circle cx="18" cy="16" r="3"/>',
  swords: '<path d="M4 4l9 9M4 4v4M4 4h4M20 4l-9 9M20 4v4M20 4h-4M8 16l-3 3M16 16l3 3"/>',
  chart: '<path d="M4 20V10M10 20V4M16 20v-7M22 20H2"/>',
  box: '<path d="M3 8l9-5 9 5v8l-9 5-9-5z"/><path d="M3 8l9 5 9-5M12 13v8"/>',
  egg: '<path d="M12 3c4 0 7 6 7 11a7 7 0 0 1-14 0c0-5 3-11 7-11z"/>',
  star: '<path d="M12 3l2.6 5.6 6 .7-4.5 4.1 1.2 6L12 16.5 6.7 19.4l1.2-6L3.4 9.3l6-.7z"/>',
  eye: '<path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/>',
};

export const icon = (k, size = 22) =>
  `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[k] || ICONS.star}</svg>`;

// a friendly empty panel body: a soft icon, a line and a hint
export const empty = (k, title, hint = '') => `<div class="void"><span>${icon(k, 26)}</span><b>${title}</b>${hint ? `<small>${hint}</small>` : ''}</div>`;
