// You (only when signed in): your PalDuels profile as Augur last read it (your balance, predictions, duels, spending
// and titles), your pet and your Hall, and your bets next to Augur's calls.
import { invoke } from '../lib/bridge.js';
import { ago, esc, fmt, pct, signed } from '../lib/format.js';
import { orb } from '../lib/pal.js';

const shortDay = (d) => new Date(d + 'T12:00:00Z').toLocaleDateString(undefined, { day: 'numeric', month: 'short' });

function petPanel(p) {
  if (!p) return '';
  const now = p.path.find((s) => s[2]);
  return `<div class="panel ypet"><div class="ph"><h3>Your pet</h3><span>${now ? esc(now[0]) : ''}</span></div>
    <div class="yp">${orb(p.name, p.name.toLowerCase().replace(/\s+/g, '-'), 'h', 72, true)}<div><b>${esc(p.name)}</b>
      <div class="path">${p.path.map(([s, done, isNow]) => `<span class="${isNow ? 'now' : done ? 'done' : ''}" title="${esc(s)}">${esc(s)}</span>`).join('')}</div>
      ${p.passives.length ? `<div class="chips">${p.passives.map((x) => `<span class="chip">${esc(x)}</span>`).join('')}</div>` : ''}</div></div>
    ${p.log.length ? `<div class="plog">${p.log.slice(0, 4).map(([g, w]) => `<div><b class="${g ? 'ok' : 'muted'}">${g ? '+' + g : '·'}</b><span>${esc(w)}</span></div>`).join('')}</div>` : ''}</div>`;
}

function hallPanel(h) {
  if (!h) return '';
  return `<div class="panel"><div class="ph"><h3>Your Hall</h3><span>${h.found} of ${h.total} pals found</span></div>
    <div class="htiers">${h.tiers.map(([t, stage, n, of]) => `<div><span class="tier tier-${esc(t.toLowerCase())}">${esc(t)}</span><div class="mb"><i style="width:${of ? (100 * n) / of : 0}%"></i></div><small>${n}/${of}</small></div>`).join('')}</div>
    <div class="hpals">${h.pals.map(([pal, slug, tier, d]) => `<span class="chip" title="Tier ${esc(tier)} · found ${esc(d)}">${orb(pal, slug, 'h av', 22)}${esc(pal)} <small>${esc(shortDay(d))}</small></span>`).join('')}</div></div>`;
}

// the last 10, oldest first, as dots
const form = (f) => (f?.length ? `<span class="form">${f.map((w) => `<i class="${w ? 'w' : 'l'}" title="${w ? 'Win' : 'Loss'}"></i>`).join('')}</span>` : '');

function panel(p, wide) {
  if (!p) return '';
  return `<div class="panel ${wide ? 'span2' : ''}"><div class="ph"><h3>${esc(p.title)}</h3>${form(p.form)}</div>
    <div class="ystats">${p.stats.map(([k, v, tone]) => `<div><b class="${tone === 'good' ? 'ok' : tone === 'bad' ? 'bad' : ''}">${esc(v)}</b><small>${esc(k)}</small></div>`).join('')}</div></div>`;
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
      `<div class="panel empty"><b>Reading your profile…</b>Augur reads it with your picks list, every few minutes while you're signed in.</div>`;
  } else {
    const c = y.comparison;
    const t = pg.titles_earned;
    main.innerHTML =
      header(`You · ${esc(y.name || 'PalDuels')}`, refresh) +
      `<div class="ytop">
        <div class="panel ybal"><span class="label">Your balance</span><b>${pg.balance != null ? fmt(pg.balance) : '—'}</b><small>credits · only you can see this</small></div>
        ${panel(find('prediction'), true)}
      </div>
      <div class="g3">
        ${panel(find('duel record'))}
        <div class="panel"><div class="ph"><h3>You and Augur</h3><span>on Augur's calls</span></div>
          <div class="ystats"><div><b class="${c.yours >= 0 ? 'ok' : 'bad'}">${signed(c.yours)}</b><small>you made</small></div><div><b class="${c.augur >= 0 ? 'ok' : 'bad'}">${signed(c.augur)}</b><small>following Augur</small></div><div><b>${c.followed}·${c.deviated}</b><small>followed · deviated</small></div><div><b>${c.missed}</b><small>missed</small></div></div>
          <div class="small" style="margin-top:8px">Augur's calls: ${y.augur_calls[0]}/${y.augur_calls[1]}${y.augur_calls[1] ? ` · ${pct(y.augur_calls[0] / y.augur_calls[1])}` : ''} · <a href="#" id="yb">every bet in History ›</a></div></div>
        ${panel(find('chaos'))}
      </div>
      ${y.pet || y.hall ? `<div class="g2 even">${petPanel(y.pet)}${hallPanel(y.hall)}</div>` : ''}
      ${
        t || pg.titles.length
          ? `<div class="panel"><div class="ph"><h3>Titles</h3>${t ? `<span>${t[0]} of ${t[1]} earned · ${t[1] - t[0]} to unlock</span>` : ''}</div>
        ${t ? `<div class="progress"><i style="width:${(100 * t[0]) / t[1]}%"></i></div>` : ''}
        <div class="badges">${pg.titles.map((b) => `<span class="badge ${b.rare ? 'rare' : ''}" title="${esc(b.about)}"><i>${esc(b.icon)}</i>${esc(b.name)}${b.times ? ` <b>×${b.times}</b>` : ''}</span>`).join('')}</div></div>`
          : ''
      }`;
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
