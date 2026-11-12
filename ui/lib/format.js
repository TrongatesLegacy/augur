// Turning values into text for the pages. Anything from palduels.com goes through esc() before it's shown.

const ENTITIES = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };

export const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ENTITIES[c]);
export const pct = (p) => `${Math.round(p * 100)}%`;
export const fmt = (n) => (n == null ? '—' : Number(n).toLocaleString('en-GB', { maximumFractionDigits: 0 }));
// credits won or lost: +2,105 / −500
export const signed = (n) => (n == null ? '—' : `${n > 0 ? '+' : n < 0 ? '−' : ''}${fmt(Math.abs(n))}`);
export const initials = (name) =>
  String(name || '?')
    .split(/\s+/)
    .map((w) => w[0])
    .join('')
    .slice(0, 2)
    .toUpperCase();
