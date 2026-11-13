// What the sidebar's activity spot shows: whatever Augur is bringing in right now (the duels missed while closed, the
// background fetch, your bets) and, for a few seconds, what just arrived from another computer.
import { backfillState } from './backfill.js';
import { esc } from './format.js';

// how long "Synced" stays up after something arrives from another computer
export const FLASH_MS = 12_000;

const clock = (ms) => new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
const running = (p, phase) => p && p.phase === phase && p.total > 0;

// { share, short, label, long, warn, kind, go } for the spot, or null when nothing is going on.
// `fetch`: the backfill's progress, `bets`: the bets import's, `sync`: Settings › Sync's view, `flash`: the change
// that just arrived (or null).
export function activity({ fetch, bets, sync, flash }, now = Date.now()) {
  const items = [];
  if (flash && now - flash.at < FLASH_MS) {
    const n = /^(\d+) call/.exec(flash.summary)?.[1];
    items.push({ kind: 'sync', go: 'sync', share: 1, short: n ? `+${n}` : '✓', label: 'Synced', long: `${flash.summary} from ${flash.from}` });
  }
  if (running(fetch, 'catchup')) {
    const st = backfillState(fetch);
    items.push({ ...st, kind: 'fetch', go: 'model', label: st.warn ? 'Site busy' : 'Catch-up' });
  } else if (fetch && fetch.phase !== 'catchup') {
    const st = backfillState(fetch);
    if (st) items.push({ ...st, kind: 'fetch', go: 'model', label: st.warn ? 'Site busy' : 'Fetching' });
  }
  if (running(bets, 'bets')) {
    const st = backfillState(bets);
    items.push({ ...st, kind: 'bets', go: 'bets', label: st.warn ? 'Site busy' : 'Bets' });
  }
  if (!items.length) return null;
  const lines = items.map((i) => i.long);
  if (sync?.on && sync.last_sync) lines.push(`Synced with your other computers at ${clock(sync.last_sync)}.`);
  return { ...items[0], long: lines.join('\n') };
}

// The spot's insides: a ring filled to the share, the short text in it, the label under it.
export function activityHtml(a) {
  return `<svg viewBox="0 0 36 36" aria-hidden="true"><circle cx="18" cy="18" r="15" class="t"/><circle cx="18" cy="18" r="15" class="f" style="stroke-dasharray:${(a.share * 94.2).toFixed(1)} 94.2"/></svg><span>${esc(a.short)}</span><small>${esc(a.label)}</small>`;
}
