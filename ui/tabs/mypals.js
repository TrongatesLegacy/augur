// You › Pals: your own pal box, what one reroll would do for each pal against the pals it meets, and the reroll
// itself. A roll spends credits, so it takes two clicks, and the price it's sent with is the one you saw; after it,
// the old and new rolls sit side by side with Augur's pick, and you choose with the site's own buttons.
import { invoke } from '../lib/bridge.js';
import { esc, fmt, pct } from '../lib/format.js';
import { boxCard, boxCols, boxGrid, ivRow, STATS, ivTone } from '../lib/box.js';
import { orb, passivePills } from '../lib/pal.js';
import { empty } from '../lib/viz.js';

const TAG = { Reroll: 'rr', Maybe: 'maybe', Keep: 'keep', Replace: 'no' };
const ORDER = { Reroll: 0, Maybe: 1, Keep: 2, Replace: 3 };
// how often to ask again while the site is still working on a roll, as its own page does
const POLL = { busy: 1500, sync: 2500, wait: 10000 };

const gain = (a) => (a ? a.after - a.now : 0);
// win-chance points per 1,000 credits: what the order and the tags go by
const worth = (p) => (p.advice ? (gain(p.advice) * 100) / ((p.price || 1000) / 1000) : -1);
const byWorth = (a, b) => (ORDER[a.advice?.tag] ?? 4) - (ORDER[b.advice?.tag] ?? 4) || worth(b) - worth(a);
const pts = (x) => `${(x * 100).toFixed(1)}%`;

let view = null; // the box as last shown
let sheet = null; // the selected pal's reroll panel
let selected = null;
let confirming = false;
let poll = null;

// a card's foot, on one line: its win chance now → after a reroll, then the tag with the price
function cardFoot(p) {
  const a = p.advice;
  if (!a) return `<div class="bxa"><small class="muted">Too few duels known for its species</small></div>`;
  const price = p.price != null ? ` · ${fmt(p.price)}` : p.id ? ' · …' : '';
  return `<div class="bxa" title="Win chance against the pals it meets: ${pts(a.now)} now, ${pts(a.after)} on average after one reroll${p.price != null ? `, for ${fmt(p.price)} credits` : ''}">
    <div class="bxw"><b>${Math.round(a.now * 100)}%</b><span aria-hidden="true">→</span><b class="${gain(a) >= 0.01 ? 'ok' : ''}">${Math.round(a.after * 100)}%</b></div>
    <span class="tag ${TAG[a.tag]}">${esc(a.tag)}${price}</span></div>`;
}

function card(p) {
  const on = p.id && p.id === selected;
  return boxCard(p, {
    extra: cardFoot(p),
    cls: `own ${on ? 'on' : ''}`,
    passives: false,
    attrs: p.id ? `data-pal="${esc(p.id)}" tabindex="0" role="button" aria-pressed="${on}"` : '',
    title: p.id ? `Reroll ${p.pal}` : '',
  });
}

// the right-hand panel with nothing selected: what's worth doing, and how the advice works
function overview(v) {
  const worthIt = v.pals.filter((p) => p.advice?.tag === 'Reroll').sort(byWorth);
  const cost = worthIt.reduce((n, p) => n + (p.price || 0), 0);
  return `<div class="panel col mrr"><div class="ph"><h3>Rerolls</h3><span>pick a pal</span></div>
    <div class="mrr-sum">
      <div><span class="label">Your balance</span><b>${v.balance != null ? fmt(v.balance) : '—'}</b><small>credits</small></div>
      <div><span class="label">Worth it</span><b>${fmt(worthIt.length)}</b><small>${worthIt.length ? `${fmt(cost)} credits to reroll each once` : 'no rerolls right now'}</small></div>
    </div>
    ${worthIt.length ? `<div class="mrr-best"><div class="mrr-bh"><span class="label">Best value first</span><small>win chance · price</small></div>${worthIt.slice(0, 6).map((p) => `<button class="mrr-row" data-pal="${esc(p.id)}">${orb(p.pal, p.slug, 'h av', 26)}<span>${esc(p.pal)}</span><b class="ok">+${(gain(p.advice) * 100).toFixed(1)}%</b><small>${p.price != null ? `${fmt(p.price)} cr` : '…'}</small></button>`).join('')}</div>` : ''}
    <div class="mrr-how small">
      <p>Duels are matched by power, so a pal meets the same pals whatever its rolls: Augur takes the ones its species has met, and works out its win chance against them with its rolls now and with every possible new roll.</p>
      <p>A reroll keeps the better of the new three and the old, so it can only help. <b>Reroll</b> means it buys at least 2 points of win chance per 1,000 credits; <b>Maybe</b> at least 0.75. The price doubles each time.</p>
    </div></div>`;
}

// a set of rolls as three big dice
const dice = (ivs) =>
  `<div class="mrr-dice">${STATS.map((k, i) => `<div style="--c:${ivTone(ivs[i])};--v:${ivs[i]}%"><span>${k}</span><b>${fmt(ivs[i])}</b><i></i></div>`).join('')}</div>`;

// How new rolls' win chances spread (every possible roll, the share of them at each win chance), with a line at
// each mark ({at, label, cls}): the bars right of "now" are rolls that would help, the rest you'd keep yours over.
function spreadChart(a, marks, foot = '') {
  if (!a?.spread?.length) return '';
  const top = Math.max(...a.spread, 1e-9);
  const span = a.high - a.low || 1;
  const x = (w) => Math.max(0, Math.min(100, ((w - a.low) / span) * 100));
  const bars = a.spread
    .map((s, i) => {
      const mid = a.low + ((i + 0.5) / a.spread.length) * span;
      return `<i class="${mid > a.now ? 'up' : ''}" style="height:${Math.max(2, (100 * s) / top).toFixed(1)}%" title="${pct(s)} of rolls give ${pts(mid)}"></i>`;
    })
    .join('');
  const lines = marks
    .map((m) => `<s class="${m.cls || ''} ${x(m.at) > 62 ? 'flip' : ''}" style="left:${x(m.at).toFixed(1)}%"><em>${esc(m.label)} ${pts(m.at)}</em></s>`)
    .join('');
  return `<div class="mrr-chart"><div class="mrr-ch"><span class="label">New rolls</span><small>${pct(a.better)} beat ${marks.length > 1 ? 'the old' : 'these'}</small></div>
    <div class="mrr-bars">${bars}${lines}</div>
    <div class="mrr-axis"><span>${pts(a.low)}</span><span>win chance</span><span>${pts(a.high)}</span></div>${foot ? `<small class="mrr-foot">${foot}</small>` : ''}</div>`;
}

// the selected pal: its rolls, what a reroll would do, and the panel's forms as buttons
function rerollPanel(s) {
  const p = s.pal;
  const a = p.advice;
  const panel = s.panel;
  // busy (rolling, saving a choice) or waiting: the site's own status, a spinner, no buttons; syncing (a choice
  // saved, the card catching up) shows as done
  const working = panel.busy || panel.wait;
  const head = `<div class="ph"><h3>Reroll</h3><button class="btn ghost sm" id="mrr-x">Back</button></div>
    <div class="mrr-pal">${orb(p.pal, p.slug, 'h', 52)}<div><b>${esc(p.pal)}</b><small>${p.tier ? `<span class="tierb tier-${esc(p.tier.toLowerCase())}">${esc(p.tier)}</span>` : ''}power ${p.power != null ? fmt(p.power) : '—'}${a ? `<span class="tag ${TAG[a.tag]}">${esc(a.tag)}</span>` : ''}${p.rerolls ? ` · rerolled ×${fmt(p.rerolls)}` : ''}</small>${p.passives?.length ? `<div class="pills">${passivePills(p.passives)}</div>` : ''}</div></div>`;
  let body;
  if (working && panel.rolls.length < 2) {
    body = `${panel.rolls[0] ? dice(panel.rolls[0][1]) : ''}<div class="mrr-wait"><span class="spin" aria-hidden="true"></span><b>${esc(panel.state || 'The site is working on it…')}</b><small>${esc(panel.note || '')}</small></div>`;
  } else if (panel.rolls.length >= 2) {
    // after a roll: the two side by side, Augur's pick marked, and the site's choices
    const keep = s.recommend === 'keep';
    const old = panel.rolls.findIndex(([l]) => /now|old|current|before/i.test(l));
    const o = old >= 0 ? old : 0;
    const n = o === panel.rolls.length - 1 ? 0 : panel.rolls.length - 1;
    const side = (i, pick) =>
      `<div class="mrr-roll ${pick ? 'pick' : ''}"><span class="label">${esc(panel.rolls[i][0] || (i === o ? 'Your IVs now' : 'The new roll'))}</span>${dice(panel.rolls[i][1])}
        <div class="mrr-win"><b>${s.wins[i] != null ? pts(s.wins[i]) : '—'}</b><small>win chance</small></div>${pick ? '<span class="tag keep">Augur’s pick</span>' : ''}</div>`;
    const marks = [{ at: s.wins[o] ?? a?.now, label: 'old' }];
    if (s.wins[n] != null) marks.push({ at: s.wins[n], label: 'new', cls: 'new' });
    body = `<div class="mrr-vs">${side(o, s.recommend && !keep)}${side(n, s.recommend && keep)}</div>
      ${panel.verdict ? `<div class="mrr-verdict ${keep ? 'ok' : ''}">${esc(panel.verdict)}</div>` : ''}
      ${a && marks[0].at != null ? spreadChart(a, marks) : ''}
      <div class="mrr-acts">${panel.forms.map((f, i) => `<button class="btn ${(f.fields.find(([k]) => k === 'op')?.[1] === s.recommend) ? '' : 'ghost'}" data-form="${i}">${esc(f.label || 'Choose')}</button>`).join('')}</div>
      <p class="small mrr-note ${/minute/i.test(panel.note) ? 'gold' : ''}">${esc(panel.note || 'Credits are spent either way.')}</p>`;
  } else {
    const roll = panel.forms.findIndex((f) => f.fields.some(([k, v]) => k === 'op' && v === 'roll'));
    // the price is the roll form's own limit: what's sent, and the most the site may charge
    const max = roll >= 0 ? panel.forms[roll].fields.find(([k]) => k === 'max')?.[1] : null;
    const price = max != null ? Number(String(max).replace(/,/g, '')) : null;
    body = `${panel.rolls[0] ? dice(panel.rolls[0][1]) : ''}
      ${a ? `<div class="mrr-odds"><div><b>${pts(a.now)}</b><small>win chance now</small></div><span aria-hidden="true">→</span><div><b class="${gain(a) >= 0.01 ? 'ok' : ''}">${pts(a.after)}</b><small>on average after one reroll</small></div></div>
        ${spreadChart(a, [{ at: a.now, label: 'now' }], `Against the pals it meets: ${a.own_duels ? `its species' last ${fmt(a.duels)} duels` : 'species of its power'}.`)}` : '<p class="small">Augur doesn’t know enough duels for this species to say what a reroll would do.</p>'}
      ${roll >= 0 && price != null
        ? `<div class="mrr-acts">${confirming
            ? `<button class="btn big" data-form="${roll}" data-price="${price}">Spend ${fmt(price)} credits</button><button class="btn ghost no" id="mrr-no">Cancel</button>`
            : `<button class="btn big" id="mrr-roll">Roll for ${fmt(price)} credits</button>`}</div>
          <p class="small mrr-note">${esc(panel.note || '')}${panel.note ? ` Next: ${fmt(price * 2)}.` : ''}</p>`
        : `<p class="small">${esc(panel.said || 'No roll offered right now.')}</p>`}`;
  }
  // what the site last said: its flash, or its status once a step is done ("Kept! …")
  const said = panel.said || (!working && panel.rolls.length < 2 ? panel.state : '');
  return `<div class="panel col mrr sel">${head}${said ? `<div class="mrr-said">${esc(said)}</div>` : ''}<div class="mrr-body">${body}</div></div>`;
}

function draw(app) {
  const v = view;
  const pals = [...v.pals].sort(byWorth);
  const left = `<div class="panel col"><div class="ph"><h3>Your pals</h3><span>win chance now → after a reroll · best value first${v.priced ? '' : ' · reading prices…'}</span></div>
    ${v.error ? `<div class="small bad">Couldn't read your box: ${esc(v.error)}</div>` : ''}
    ${pals.length ? boxGrid(pals.map(card), Math.min(3, boxCols(pals.length)), 'own') : empty('box', 'No pals in your box', 'Hatch one on the stream and it shows here.')}</div>`;
  const right = sheet && sheet.id === selected ? rerollPanel(sheet) : overview(v);
  app.main.querySelector('.mpg').innerHTML = left + right;
  wire(app);
}

async function open(app, id) {
  selected = id;
  confirming = false;
  sheet = null;
  draw(app);
  const right = app.main.querySelector('.mrr');
  if (right) right.innerHTML = `<div class="ph"><h3>Reroll</h3></div>${empty('egg', 'Reading its reroll panel…')}`;
  try {
    sheet = await invoke('reroll_open', { id });
  } catch (e) {
    app.flash(e);
    selected = null;
  }
  if (app.state.tab === 'you' && app.state.youView === 'pals') draw(app), keepPolling(app);
}

// while the site is still working on a roll, read the panel again as often as its own page would
function keepPolling(app) {
  clearTimeout(poll);
  const p = sheet?.panel;
  const wait = p && (p.busy ? POLL.busy : p.sync ? POLL.sync : p.wait ? POLL.wait : 0);
  if (!wait) return;
  const id = sheet.id;
  poll = setTimeout(async () => {
    if (app.state.tab !== 'you' || app.state.youView !== 'pals' || selected !== id) return;
    try {
      const was = sheet.panel;
      sheet = await invoke('reroll_open', { id });
      const now = sheet.panel;
      // a step just finished (a choice saved, the card caught up): read the box again, so the pal's rolls, advice and
      // next price are the new ones
      const settled = (((was.busy || was.wait) && !(now.busy || now.wait)) || (was.sync && !now.sync)) && now.rolls.length < 2;
      if (settled) {
        view = await invoke('own_box', { prices: false });
        sheet = await invoke('reroll_open', { id });
      }
      draw(app);
      keepPolling(app);
    } catch (e) {
      app.flash(e);
    }
  }, wait);
}

async function send(app, form, price) {
  const id = sheet.id;
  app.main.querySelectorAll('.mrr button').forEach((b) => (b.disabled = true));
  try {
    sheet = await invoke('reroll_send', { id, form, price: price ?? null });
    confirming = false;
    // a choice made (or a roll finished): the box has changed, so read it again
    const p = sheet.panel;
    if (!(p.busy || p.sync || p.wait) && (p.done || p.rolls.length < 2)) view = await invoke('own_box', { prices: false });
  } catch (e) {
    app.flash(e);
    confirming = false;
    try {
      sheet = await invoke('reroll_open', { id });
    } catch {
      /* the flash already says what went wrong */
    }
  }
  draw(app);
  keepPolling(app);
}

function wire(app) {
  const m = app.main;
  m.querySelectorAll('[data-pal]').forEach((el) => {
    el.onclick = () => open(app, el.dataset.pal);
    el.onkeydown = (e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), open(app, el.dataset.pal));
  });
  const on = (sel, f) => m.querySelector(sel)?.addEventListener('click', f);
  on('#mrr-x', () => ((selected = null), (sheet = null), (confirming = false), clearTimeout(poll), draw(app)));
  on('#mrr-roll', () => ((confirming = true), draw(app)));
  on('#mrr-no', () => ((confirming = false), draw(app)));
  m.querySelectorAll('.mrr [data-form]').forEach((b) => (b.onclick = () => send(app, +b.dataset.form, b.dataset.price ? +b.dataset.price : null)));
}

// You › Pals inside the You tab's frame (`top` is its header, `wireTop` wires it as soon as it's there)
export async function render(app, top, wireTop = () => {}) {
  const { main } = app;
  main.innerHTML = top + `<div class="mpg grow"><div class="panel col grow">${empty('box', 'Reading your box…')}</div></div>`;
  wireTop();
  try {
    view = await invoke('own_box', { prices: false });
  } catch (e) {
    main.querySelector('.mpg').innerHTML = `<div class="panel col">${empty('box', 'Couldn’t read your box', esc(e))}</div>`;
    return;
  }
  if (app.state.tab !== 'you' || app.state.youView !== 'pals') return;
  if (!view.signed_in) {
    main.querySelector('.mpg').innerHTML = `<div class="panel col">${empty('box', 'Sign in to see your pals', 'Settings › Account. Augur reads your box, and rerolls only when you click Roll and confirm.')}</div>`;
    return;
  }
  draw(app);
  if (!view.priced) {
    // each pal's real price, read one at a time; the advice then goes by it
    const priced = await invoke('own_box', { prices: true }).catch(() => null);
    if (priced && app.state.tab === 'you' && app.state.youView === 'pals') (view = priced), draw(app);
  }
}
