// The bet Augur suggests for a call: the chat command to copy, and a short line on why.
import { esc, fmt } from './format.js';

// the command to copy: the suggested one, or the bare side command when no stake came with the call
export const betCommand = (p, stake) => (stake ? stake.command : p.pick === 'home' ? '!h' : '!a');

// "!h all · 5,000 credits" with the reason, or "No bet" when Augur says to sit it out
export function stakeLine(stake) {
  if (!stake) return '';
  if (!stake.amount) return `<div class="bet off"><div><b>No bet</b><small>${esc(stake.why)}</small></div></div>`;
  return `<div class="bet"><div><b>${esc(stake.command)}</b> <span class="amt">${fmt(stake.amount)} credits</span><small>${esc(stake.why)}</small></div></div>`;
}
