// You (only when signed in): your PalDuels profile as Augur last read it (your balance, predictions, duels, spending
// and titles), your pet and your Hall, and your bets next to Augur's calls.
import { invoke } from '../lib/bridge.js';
import { ago, esc, fmt, pct } from '../lib/format.js';
import { orb } from '../lib/pal.js';
import { empty, formDots, icon, ring, signedBar, split, stack } from '../lib/viz.js';

const shortDay = (d) => new Date(d + 'T12:00:00Z').toLocaleDateString(undefined, { day: 'numeric', month: 'short' });

// the site shows numbers as text ("+4,310", "58%", "−1,795")
const num = (s) => {
  const n = parseFloat(String(s ?? '').replace(/−/g, '-').replace(/[^\d.-]/g, ''));
  return Number.isNaN(n) ? null : n;
};
const toneOf = (t) => (t === 'good' ? 'ok' : t === 'bad' ? 'bad' : '');

// One profile panel's numbers, looked up by label; whatever a layout doesn't draw goes in the facts list, so a new
// stat on the site still shows.
function reader(p) {
  const used = new Set();
  const get = (k) => {
    const s = p?.stats.find(([l]) => l.toLowerCase() === k);
    if (s) used.add(s);
    return s ? { text: s[1], n: num(s[1]), tone: toneOf(s[2]) } : null;
  };
  const rest = () => (p?.stats || []).filter((s) => !used.has(s));
  const facts = (rows) =>
    `<div class="facts">${rows.map(([k, v, tone]) => `<div><span>${esc(k)}</span><b class="${toneOf(tone)}">${esc(v)}</b></div>`).join('')}</div>`;
  return { get, rest, facts };
}

function predictions(p) {
  if (!p) return '';
  const r = reader(p);
  const rate = r.get('hit rate');
  const won = r.get('won');
  const lost = r.get('lost');
  const picks = r.get('picks');
  const cin = r.get('credits in');
  const back = r.get('returned');
  const max = Math.max(cin?.n || 0, back?.n || 0, 1);
  const flow = (label, x, cls) =>
    x ? `<div class="hbar"><span>${label}</span><b>${esc(x.text)}</b><div class="b"><i class="${cls}" style="width:${(100 * x.n) / max}%"></i></div></div>` : '';
  return `<div class="panel ypred"><div class="ph"><h3>${esc(p.title)}</h3>${formDots(p.form)}</div>
    <div class="ypg">
      ${ring(rate?.n != null ? rate.n / 100 : null, { size: 90, tone: 'gold', label: rate ? esc(rate.text) : '—', sub: 'hit rate' })}
      <div class="ycol">
        ${picks ? `<div class="yhead"><b class="bn">${esc(picks.text)}</b><span>picks made</span></div>` : ''}
        ${won && lost ? split(won.n, lost.n, 'won', 'lost') : ''}
      </div>
      <div class="ycol bars">${flow('Credits in', cin, '')}${flow('Returned', back, back && cin && back.n >= cin.n ? 'ok' : 'bad')}</div>
      ${r.facts(r.rest())}
    </div></div>`;
}

function duels(p) {
  if (!p) return '';
  const r = reader(p);
  const rate = r.get('win rate');
  const wins = r.get('wins');
  const losses = r.get('losses');
  const total = r.get('duels');
  return `<div class="panel"><div class="ph"><h3>${esc(p.title)}</h3>${formDots(p.form)}</div>
    <div class="yrow">${ring(rate?.n != null ? rate.n / 100 : null, { size: 76, tone: 'ok', label: rate ? esc(rate.text) : '—', sub: 'win rate' })}
      <div class="ycol">${total ? `<div class="yhead"><b class="bn">${esc(total.text)}</b><span>ranked duels</span></div>` : ''}
        ${wins && losses ? split(wins.n, losses.n, 'wins', 'losses') : ''}</div></div>
    ${r.rest().length ? r.facts(r.rest()) : ''}</div>`;
}

function versus(y) {
  const c = y.comparison;
  const max = Math.max(Math.abs(c.yours), Math.abs(c.augur), 1);
  const [right, total] = y.augur_calls;
  return `<div class="panel"><div class="ph"><h3>You and Augur</h3><span>on Augur's calls</span></div>
    ${signedBar('You made', c.yours, max)}${signedBar('Following Augur', c.augur, max)}
    ${stack([
      [c.followed, 'followed', 'ok'],
      [c.deviated, 'deviated', 'gold'],
      [c.missed, 'missed', 'dim'],
    ])}
    <div class="small yfoot">Augur's calls: <b>${right}/${total}</b>${total ? ` · ${pct(right / total)}` : ''} · <a href="#" id="yb">every bet in History ›</a></div></div>`;
}

const CHAOS_ICON = { eggs: 'egg', zergs: 'swords', gifts: 'star' };

function chaos(p) {
  if (!p) return '';
  const r = reader(p);
  const burned = r.get('credits burned');
  const counts = r.rest();
  return `<div class="panel"><div class="ph"><h3>${esc(p.title)}</h3></div>
    <div class="ychaos">${counts
      .map(([k, v, t]) => `<div><span>${icon(CHAOS_ICON[k.toLowerCase()] || 'star', 18)}</span><b class="${toneOf(t)}">${esc(v)}</b><small>${esc(k)}</small></div>`)
      .join('')}</div>
    ${burned ? `<div class="yburn"><span>${icon('chart', 16)} Credits burned</span><b class="${burned.tone}">${esc(burned.text)}</b></div>` : ''}</div>`;
}

function petPanel(p) {
  if (!p) return `<div class="panel col"><div class="ph"><h3>Your pet</h3></div>${empty('egg', 'No pet yet', 'Augur reads your pet with your picks list.')}</div>`;
  const now = p.path.findIndex((s) => s[2]);
  const reached = now >= 0 ? now : p.path.filter((s) => s[1]).length - 1;
  return `<div class="panel col ypet"><div class="ph"><h3>Your pet</h3><span>${now >= 0 ? esc(p.path[now][0]) : ''}</span></div>
    <div class="yp">${orb(p.name, p.name.toLowerCase().replace(/\s+/g, '-'), 'h', 54, true)}<div><b>${esc(p.name)}</b>
      ${p.passives.length ? `<div class="chips">${p.passives.map((x) => `<span class="chip">${esc(x)}</span>`).join('')}</div>` : ''}</div></div>
    <div class="steps" style="--n:${p.path.length};--at:${Math.max(0, reached)}">${p.path.map(([s, done, isNow]) => `<span class="${isNow ? 'now' : done ? 'done' : ''}" title="${esc(s)}"><i></i>${esc(s)}</span>`).join('')}</div>
    <div class="plog fillin">${p.log.length ? p.log.map(([g, w]) => `<div><b class="${g ? 'ok' : 'muted'}">${g ? '+' + g : '·'}</b><span>${esc(w)}</span></div>`).join('') : '<div class="small">No growth logged yet.</div>'}</div></div>`;
}

function hallPanel(h) {
  if (!h) return `<div class="panel col"><div class="ph"><h3>Your Hall</h3></div>${empty('box', 'Hall not read yet', 'Every pal your pets become, by tier.')}</div>`;
  return `<div class="panel col"><div class="ph"><h3>Your Hall</h3><span>${h.found} of ${h.total} pals found</span></div>
    <div class="yrow">${ring(h.total ? h.found / h.total : null, { size: 62, label: fmt(h.found), sub: `of ${fmt(h.total)}` })}
      <div class="htiers">${h.tiers.map(([t, stage, n, of]) => `<div title="${esc(stage)}"><span class="tier">${esc(t)}</span><div class="mb"><i style="width:${of ? Math.max(n ? 2 : 0, (100 * n) / of) : 0}%"></i></div><small>${n}/${of}</small></div>`).join('')}</div></div>
    <div class="hpals fillin">${h.pals.map(([pal, slug, tier, d]) => `<span class="chip" title="Tier ${esc(tier)} · found ${esc(d)}">${orb(pal, slug, 'h av', 22)}${esc(pal)} <small>${esc(shortDay(d))}</small></span>`).join('')}</div></div>`;
}

function titlesPanel(pg) {
  const t = pg.titles_earned;
  if (!t && !pg.titles.length) return `<div class="panel col"><div class="ph"><h3>Titles</h3></div>${empty('star', 'No titles yet', 'Win duels and picks to earn them.')}</div>`;
  return `<div class="panel col"><div class="ph"><h3>Titles</h3>${t ? `<span>${t[0]} of ${t[1]} earned · ${t[1] - t[0]} to unlock</span>` : ''}</div>
    ${t ? `<div class="progress thick"><i style="width:${(100 * t[0]) / t[1]}%"></i></div>` : ''}
    <div class="badges fillin">${pg.titles.map((b) => `<span class="badge ${b.rare ? 'rare' : ''}" title="${esc(b.about)}"><i>${esc(b.icon)}</i>${esc(b.name)}${b.times ? ` <b>×${b.times}</b>` : ''}</span>`).join('')}</div></div>`;
}

export async function render(app) {
  const { state, main, header } = app;
  const y = await invoke('you_view');
  if (state.tab !== 'you') return;
  const pg = y.page;
  const find = (t) => pg?.panels.find((p) => p.title.toLowerCase().startsWith(t));
  const refresh = `<span class="sub ml">${y.read_at ? `read ${ago(y.read_at)}` : 'not read yet'}</span><button class="btn ghost" id="yr">Read now</button>`;
  if (!pg) {
    main.innerHTML =
      header(`You${y.name ? ' · ' + esc(y.name) : ''}`, refresh) +
      `<div class="panel col grow">${empty('eye', 'Reading your profile…', "Augur reads it with your picks list, every few minutes while you're signed in.")}</div>`;
  } else {
    main.innerHTML =
      header(`You · ${esc(y.name || 'PalDuels')}`, refresh) +
      `<div class="ytop">
        <div class="panel ybal"><span class="label">Your balance</span><b>${pg.balance != null ? fmt(pg.balance) : '—'}</b><small>credits · only you can see this</small><span class="coin" aria-hidden="true"></span></div>
        ${predictions(find('prediction'))}
      </div>
      <div class="g3 ymid">${duels(find('duel record'))}${versus(y)}${chaos(find('chaos'))}</div>
      <div class="g3 grow">${petPanel(y.pet)}${hallPanel(y.hall)}${titlesPanel(pg)}</div>`;
  }
  document.getElementById('yr')?.addEventListener('click', async (e) => {
    e.target.disabled = true;
    e.target.textContent = 'Reading…';
    await invoke('import_bets');
  });
  document.getElementById('yb')?.addEventListener('click', (e) => {
    e.preventDefault();
    state.view = 'bets';
    state.range = 'all';
    app.show('history');
  });
}
