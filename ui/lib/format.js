// Turning values into text for the pages. Anything from palduels.com goes through esc() before it's shown.

const ENTITIES = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };

export const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ENTITIES[c]);
export const pct = (p) => `${Math.round(p * 100)}%`;
export const fmt = (n) => (n == null ? '—' : Number(n).toLocaleString('en-GB', { maximumFractionDigits: 0 }));
// "just now", "5 min ago", "3h ago", "2d ago" for a time in ms ("never" without one)
export function ago(ms) {
  if (!ms) return 'never';
  const m = Math.max(0, Math.round((Date.now() - ms) / 60000));
  return m < 1 ? 'just now' : m < 60 ? `${m} min ago` : m < 1440 ? `${Math.round(m / 60)}h ago` : `${Math.round(m / 1440)}d ago`;
}
// credits won or lost: +2,105 / −500
export const signed = (n) => (n == null ? '—' : `${n > 0 ? '+' : n < 0 ? '−' : ''}${fmt(Math.abs(n))}`);
export const initials = (name) =>
  String(name || '?')
    .split(/\s+/)
    .map((w) => w[0])
    .join('')
    .slice(0, 2)
    .toUpperCase();
