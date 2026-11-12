// You (only when signed in): your PalDuels profile as Augur last read it (your balance, predictions, duels, spending
// and titles), your pet and your Hall, and your bets next to Augur's calls. Its Titles view is in titles.js.
import { invoke } from '../lib/bridge.js';
import { ago, esc, fmt, pct, short } from '../lib/format.js';
import { orb, passivePills } from '../lib/pal.js';
import { empty, formDots, icon, ring, signedBar, split, stack } from '../lib/viz.js';
import { titlesView, wire as wireTitles } from './titles.js';

const YOU_VIEWS = [
  ['profile', 'Profile'],
  ['titles', 'Titles'],
];

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
      ${ring(rate?.n != null ? rate.n / 100 : null, { size: 72, tone: 'gold', label: rate ? esc(rate.text) : '—', sub: 'hit rate' })}
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
    <div class="yrow">${ring(rate?.n != null ? rate.n / 100 : null, { size: 62, tone: 'ok', label: rate ? esc(rate.text) : '—', sub: 'win rate' })}
      <div class="ycol">${total ? `<div class="yhead"><b class="bn">${esc(total.text)}</b><span>ranked duels</span></div>` : ''}
        ${wins && losses ? split(wins.n, losses.n, 'wins', 'losses') : ''}</div></div>
    ${r.rest().length ? r.facts(r.rest()) : ''}</div>`;
}

function versus(y) {
  const c = y.comparison;
  const max = Math.max(Math.abs(c.yours), Math.abs(c.augur), 1);
  const [right, total] = y.augur_calls;
  return `<div class="panel"><div class="ph"><h3>You and Augur</h3><a href="#" id="yb" title="Every bet next to Augur's calls, in History">Augur's calls ${fmt(right)}/${fmt(total)}${total ? ` · ${pct(right / total)}` : ''} ›</a></div>
    ${signedBar('You made', c.yours, max)}${signedBar('Following Augur', c.augur, max)}
    ${stack([
      [c.followed, 'followed', 'ok'],
      [c.deviated, 'deviated', 'gold'],
      [c.missed, 'missed', 'dim'],
    ])}</div>`;
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

// the pet's latest growth lines (the newest first)
const PET_LOG = 3;

function petPanel(p) {
  if (!p) return `<div class="panel col"><div class="ph"><h3>Your pet</h3></div>${empty('egg', 'No pet yet', 'Augur reads your pet with your picks list.')}</div>`;
  const now = p.path.findIndex((s) => s[2]);
  const reached = now >= 0 ? now : p.path.filter((s) => s[1]).length - 1;
  return `<div class="panel col ypet"><div class="ph"><h3>Your pet</h3><span>${now >= 0 ? esc(p.path[now][0]) : ''}</span></div>
    <div class="yp">${orb(p.name, p.name.toLowerCase().replace(/\s+/g, '-'), 'h', 54, true)}<div><b>${esc(p.name)}</b>
      ${p.passives.length ? `<div class="pills">${passivePills(p.passives)}</div>` : ''}</div></div>
    <div class="steps" style="--n:${p.path.length};--at:${Math.max(0, reached)}">${p.path.map(([s, done, isNow]) => `<span class="${isNow ? 'now' : done ? 'done' : ''}" title="${esc(s)}"><i></i>${esc(s)}</span>`).join('')}</div>
    <div class="plog">${p.log.length ? p.log.slice(0, PET_LOG).map(([g, w]) => `<div><b class="${g ? 'ok' : 'muted'}">${g ? '+' + fmt(g) : '·'}</b><span>${esc(w)}</span></div>`).join('') : '<div class="small">No growth logged yet.</div>'}</div></div>`;
}

// the Hall's pals as faces, newest first: two rows of them at most
const HALL_FACES = 16;

function hallPanel(h) {
  if (!h) return `<div class="panel col"><div class="ph"><h3>Your Hall</h3></div>${empty('box', 'Hall not read yet', 'Every pal your pets become, by tier.')}</div>`;
  return `<div class="panel col"><div class="ph"><h3>Your Hall</h3><span>${fmt(h.found)} of ${fmt(h.total)} pals found</span></div>
    <div class="yrow">${ring(h.total ? h.found / h.total : null, { size: 62, label: fmt(h.found), sub: `of ${fmt(h.total)}` })}
      <div class="htiers">${h.tiers.map(([t, stage, n, of]) => `<div title="${esc(stage)}"><span class="tier">${esc(t)}</span><div class="mb"><i style="width:${of ? Math.max(n ? 2 : 0, (100 * n) / of) : 0}%"></i></div><small>${fmt(n)}/${fmt(of)}</small></div>`).join('')}</div></div>
    <div class="hpals">${[...h.pals]
      .sort((x, y) => String(y[3]).localeCompare(String(x[3])))
      .slice(0, HALL_FACES)
      .map(([pal, slug, tier, d]) => `<span title="${esc(pal)} · tier ${esc(tier)} · found ${esc(shortDay(d))}">${orb(pal, slug, 'h av', 28)}</span>`)
      .join('')}${h.pals.length > HALL_FACES ? `<span class="more">+${fmt(h.pals.length - HALL_FACES)}</span>` : ''}</div></div>`;
}

// the panel keeps room for the badges; You › Titles has the rest
const CLOSEST_HERE = 2;

// what fits under them: the rare ones by name, then the rest as tokens; the Titles view has them all
const RARE_HERE = 3;
const TOKENS_HERE = 12;

function titlesPanel(pg) {
  const t = pg.titles_earned;
  const rare = pg.titles.filter((b) => b.rare).slice(0, RARE_HERE);
  const rest = pg.titles.filter((b) => !rare.includes(b)).slice(0, TOKENS_HERE);
  const more = pg.titles.length - rare.length - rest.length;
  if (!t && !pg.titles.length) return `<div class="panel col"><div class="ph"><h3>Titles</h3></div>${empty('star', 'No titles yet', 'Win duels and picks to earn them.')}</div>`;
  return `<div class="panel col"><div class="ph"><h3>Titles</h3><span>${t ? `${fmt(t[0])} of ${fmt(t[1])} earned · ` : ''}<a href="#" id="yt">All titles ›</a></span></div>
    ${(pg.closest || []).length ? `<div class="yq"><span class="label">Closest to unlock</span>${pg.closest.slice(0, CLOSEST_HERE).map((q) => `<div class="hbar" title="${esc(q.about)}"><span>${esc(q.icon)} ${esc(q.name)}</span><b>${fmt(q.have)}<small>/${fmt(q.need)}</small></b><div class="b"><i style="width:${Math.min(100, (100 * q.have) / q.need)}%"></i></div></div>`).join('')}</div>` : ''}
    <div class="badges">${rare
      .map((b) => `<span class="badge rare" title="${esc(b.about)}"><i>${esc(b.icon)}</i>${esc(b.name)}${b.times ? ` <b>×${b.times}</b>` : ''}</span>`)
      .join('')}${rest
      .map((b) => `<span class="tok" title="${esc(b.name)}${b.times ? ` ×${b.times}` : ''}: ${esc(b.about)}"><i>${esc(b.icon)}</i>${b.times ? `<b>${short(b.times)}</b>` : ''}</span>`)
      .join('')}${more ? `<a href="#" class="tok more" data-all title="Every title you've earned">+${fmt(more)}</a>` : ''}</div></div>`;
}

export async function render(app) {
  const { state, main, header } = app;
  const y = await invoke('you_view');
  if (state.tab !== 'you') return;
  const pg = y.page;
  const find = (t) => pg?.panels.find((p) => p.title.toLowerCase().startsWith(t));
  const titles = state.youView === 'titles' && pg;
  const views = pg ? `<div class="seg ml">${YOU_VIEWS.map(([k, l]) => `<button class="${(titles ? 'titles' : 'profile') === k ? 'on' : ''}" data-yv="${k}">${l}</button>`).join('')}</div>` : '';
  const refresh = `${views}<span class="sub ${pg ? '' : 'ml'}">${y.read_at ? `read ${ago(y.read_at)}` : 'not read yet'}</span><button class="btn ghost" id="yr">Read now</button>`;
  if (titles) {
    main.innerHTML = header(`You · ${esc(y.name || 'PalDuels')} · Titles`, refresh) + titlesView(y);
    wireTitles(main);
  } else if (!pg) {
    main.innerHTML =
      header(`You${y.name ? ' · ' + esc(y.name) : ''}`, refresh) +
      `<div class="panel col grow">${empty('eye', 'Reading your profile…', "Augur reads it with your picks list, every few minutes while you're signed in.")}</div>`;
  } else {
    main.innerHTML =
      header(`You · ${esc(y.name || 'PalDuels')}`, refresh) +
      `<div class="ytop">
        <div class="panel ybal"><span class="label">Your balance</span><b style="font-size:${Math.min(50, 400 / fmt(pg.balance).length).toFixed(1)}px">${pg.balance != null ? fmt(pg.balance) : '—'}</b><small>credits · only you can see this</small><span class="coin" aria-hidden="true"></span></div>
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
  main.querySelectorAll('.seg button[data-yv]').forEach((b) => {
    b.onclick = () => {
      state.youView = b.dataset.yv;
      render(app);
    };
  });
  main.querySelectorAll('#yt, [data-all]').forEach((el) => el.addEventListener('click', (e) => {
    e.preventDefault();
    state.youView = 'titles';
    render(app);
  }));
  document.getElementById('yb')?.addEventListener('click', (e) => {
    e.preventDefault();
    state.view = 'bets';
    state.range = 'all';
    app.show('history');
  });
}
