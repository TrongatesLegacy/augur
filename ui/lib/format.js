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
// a count short enough for a narrow label: 950, 9.5k, 14k, 1.2M
export const short = (n) =>
  n == null ? '—' : n >= 1e6 ? `${+(n / 1e6).toFixed(1)}M` : n >= 1e4 ? `${Math.round(n / 1e3)}k` : n >= 1e3 ? `${+(n / 1e3).toFixed(1)}k` : fmt(n);
export const signed = (n) => (n == null ? '—' : `${n > 0 ? '+' : n < 0 ? '−' : ''}${fmt(Math.abs(n))}`);
export const initials = (name) =>
  String(name || '?')
    .split(/\s+/)
    .map((w) => w[0])
    .join('')
    .slice(0, 2)
    .toUpperCase();

// A live-feed error in words: [title, what it means]. The site not answering is different from it answering with
// something Augur can't read (it changed how it reports the duel), which needs a fix rather than patience.
export const feedProblem = (e) =>
  /couldn't read/.test(e || '')
    ? ['Can’t read PalDuels', 'PalDuels answered with something Augur can’t read yet. It keeps trying; an update may be needed.']
    : ['Can’t reach PalDuels', 'Augur keeps trying every 30 seconds.'];
