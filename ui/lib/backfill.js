// What the background fetch is doing, in a few words, for the sidebar and the Model tab.
import { fmt } from './format.js';

const WHAT = { species: 'species', duels: 'past duels', bets: 'your bets' };

const clock = (ms) => new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });

// null when there's nothing to show (not started, waiting for setup, or done)
export function backfillState(b) {
  if (!b || !b.phase || b.phase === 'done' || b.phase === 'setup') return null;
  const share = b.total ? b.done / b.total : 0;
  const what = WHAT[b.phase] || b.phase;
  if (b.gave_up) return { share, short: 'Paused', long: 'The site kept rate-limiting, so this run stopped. The next one carries on.', warn: true };
  if (b.resume_at) return { share, short: 'Waiting', long: `The site is rate-limiting: trying again at ${clock(b.resume_at)}.`, warn: true };
  if (b.paused) return { share, short: 'Paused', long: `Fetching ${what} paused while a duel is up.` };
  return { share, short: `${Math.floor(share * 100)}%`, long: `Fetching ${what}: ${fmt(b.done)} of ${fmt(b.total)}.` };
}
