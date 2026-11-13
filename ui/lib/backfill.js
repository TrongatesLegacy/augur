// What the background fetch is doing, in a few words, for the sidebar and the Model tab.
import { fmt } from './format.js';

const WHAT = { catchup: 'the duels missed while Augur was closed', species: 'species', duels: 'past duels', bets: 'your bets' };

const clock = (ms) => new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });

// null when there's nothing to show (not started, waiting for setup, or done). `short` goes inside the sidebar's
// ring, so it's a number (the share, or minutes until the site is asked again); `label` is the word under it.
export function backfillState(b, now = Date.now()) {
  if (!b || !b.phase || b.phase === 'done' || b.phase === 'setup') return null;
  const share = b.total ? b.done / b.total : 0;
  const pct = `${Math.floor(share * 100)}%`;
  const what = WHAT[b.phase] || b.phase;
  if (b.gave_up) return { share, short: pct, label: 'Stopped', long: 'The site kept rate-limiting, so this run stopped. The next one carries on.', warn: true };
  if (b.resume_at) {
    const mins = Math.ceil(Math.max(0, b.resume_at - now) / 60_000);
    return { share, short: mins > 1 ? `${mins}m` : '<1m', label: 'Site busy', long: `The site is rate-limiting: trying again at ${clock(b.resume_at)}.`, warn: true, tick: true };
  }
  if (b.paused) return { share, short: pct, label: 'Paused', long: `Fetching ${what} paused while a duel is up.` };
  return { share, short: pct, long: `Fetching ${what}: ${fmt(b.done)} of ${fmt(b.total)}.` };
}
