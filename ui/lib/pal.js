// Pieces both windows draw: a pal's orb, the chance dial, the tale of the tape, the reason line and the voices.
import { invoke } from './bridge.js';
import { esc, fmt, initials, pct } from './format.js';

export const VOICES = ['Yoda', 'Gollum', 'Dr. Rockso', 'Scooby-Doo', 'Daffy Duck'];
export const SHORT = { Yoda: 'Yoda', Gollum: 'Gollum', 'Dr. Rockso': 'Rockso', 'Scooby-Doo': 'Scooby', 'Daffy Duck': 'Daffy' };

// A pal's orb: coloured initials straight away, swapped for its art once cached (palduels.com, personal use).
const artCache = new Map();

export function orb(pal, slug, cls, size, crown) {
  const id = `orb-${Math.random().toString(36).slice(2)}`;
  queueMicrotask(() => loadArt(slug, id));
  return `<div class="orb ${cls}" id="${id}" style="width:${size}px;height:${size}px;font-size:${Math.round(size * 0.36)}px">${crown ? '<span class="crown">♛</span>' : ''}<span>${esc(initials(pal))}</span></div>`;
}

async function loadArt(slug, id) {
  if (!slug) return;
  let url = artCache.get(slug);
  if (url === undefined) {
    url = await invoke('pal_image', { slug }).catch(() => null);
    artCache.set(slug, url);
  }
  const el = document.getElementById(id);
  if (url && el) el.querySelector('span:not(.crown)')?.replaceWith(Object.assign(document.createElement('img'), { src: url, alt: '' }));
}

export function dial(p, size) {
  const r = 50;
  const c = 2 * Math.PI * r;
  return `<div class="dialw" style="width:${size}px;height:${size}px"><svg width="${size}" height="${size}" viewBox="0 0 120 120" aria-label="${pct(p)} chance"><circle cx="60" cy="60" r="${r}" fill="none" stroke="rgba(255,92,168,.22)" stroke-width="8"/><circle cx="60" cy="60" r="${r}" fill="none" stroke="#ffcf6b" stroke-width="8" stroke-linecap="round" stroke-dasharray="${(c * p).toFixed(1)} ${c.toFixed(1)}" transform="rotate(-90 60 60)"/></svg><b>${pct(p)}</b></div>`;
}

// The tale of the tape rows: home value, bar, label, bar, away value.
export function tapeRows(p) {
  const h = p.home.fighter;
  const a = p.away.fighter;
  return [
    ['HP', 'HP'],
    ['ATK', 'ATTACK'],
    ['DEF', 'DEFENCE'],
  ]
    .map(([k, label]) => {
      const hv = h[k];
      const av = a[k];
      const m = Math.max(hv, av) || 1;
      return `<div class="r5"><span class="n l ${hv < av ? 'lo' : ''}">${fmt(hv)}</span><div class="bar bl"><i style="width:${(hv / m) * 100}%"></i></div><span class="k">${label}</span><div class="bar br"><i style="width:${(av / m) * 100}%"></i></div><span class="n ${av < hv ? 'lo' : ''}">${fmt(av)}</span></div>`;
    })
    .join('');
}

// One line saying why the favourite is favoured.
export function whyLine(p) {
  const fav = p.pick === 'home' ? p.home : p.away;
  const dog = p.pick === 'home' ? p.away : p.home;
  const leads = ['HP', 'ATK', 'DEF'].filter((k) => fav.fighter[k] > dog.fighter[k]);
  const n = fav.type_edge_moves.length;
  const edge = n ? `${n} type-edge move${n > 1 ? 's' : ''}` : '';
  const names = { HP: 'HP', ATK: 'attack', DEF: 'defence' };
  const stats =
    leads.length === 3 ? 'Leads on every stat' : leads.length ? `More ${leads.map((k) => names[k]).join(' and ')}` : 'Weaker on paper, carried by records';
  const parts = [stats, edge || (p.tale.type_edge.includes('no type edge') ? 'no type edge either way' : '')].filter(Boolean);
  return parts.map((s) => s[0].toUpperCase() + s.slice(1)).join('. ') + '.';
}

// The chat line for a voice, honouring "include the chance".
export function voiceLine(p, voice, withChance) {
  const line = p.lines[VOICES.indexOf(voice)] ?? p.lines[0];
  return withChance ? line : line.replace(/\s*\(\d+%\)\s*$/, '');
}

// The voice to show: the one picked, else the default, else the first enabled.
export function pickVoice(current, settings) {
  const voices = settings?.voices?.length ? settings.voices : VOICES;
  if (voices.includes(current)) return { voices, voice: current };
  return { voices, voice: voices.includes(settings?.default_voice) ? settings.default_voice : voices[0] };
}
