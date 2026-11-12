// Settings, in seven pages: General (checking, the tray and notifications), Staking (with what it would suggest),
// Chat lines, Model (self-tuning, or your own settings), Account, Sync & data (sync across computers, back up,
// import) and Updates.
import { invoke } from '../lib/bridge.js';
import { copyText } from '../lib/clipboard.js';
import { ago, esc, fmt, pct } from '../lib/format.js';
import { notesList } from '../lib/notes.js';
import { SHORT, VOICES } from '../lib/pal.js';
import { empty, icon } from '../lib/viz.js';
import { KNOBS } from './model.js';

const PAGES = [
  ['general', 'General'],
  ['staking', 'Staking'],
  ['chat', 'Chat lines'],
  ['model', 'Model'],
  ['account', 'Account'],
  ['sync', 'Sync & data'],
  ['updates', 'Updates'],
];

const STRATEGY = {
  scaled: ['Scale with chance', 'The more sure Augur is, the bigger the bet'],
  fixed: ['Fixed amount', 'The same on every call'],
  default_or_max: ['Default or max', 'A bare !h, or !h all on strong calls'],
};

const FOLDER_ICON = { onedrive: '☁', icloud: '☁', gdrive: '▲', dropbox: '◆' };

const row = (label, sub, control) => `<div class="srow"><div>${label}${sub ? `<small>${sub}</small>` : ''}</div>${control}</div>`;
const toggle = (attr, on, label) => `<button class="tog ${on ? 'on' : ''}" ${attr} aria-pressed="${!!on}" aria-label="${esc(label)}"></button>`;

// what the tray and a notification look like with these settings, on the call that's up (or a made-up one)
function trayPreview(s, live) {
  const p = live?.prediction;
  const fav = p ? (p.pick === 'home' ? p.home.pal : p.away.pal) : 'Chillet Lux';
  const chance = p ? pct(p.prob) : '72%';
  const logo = '<svg viewBox="0 0 120 120" width="14" height="14" aria-hidden="true"><path d="M10 64 C 34 22, 86 22, 110 64 C 86 40, 34 40, 10 64 Z" fill="#4fe3ff"/><path d="M10 56 C 34 98, 86 98, 110 56 C 86 80, 34 80, 10 56 Z" fill="#ff5ca8"/><circle cx="60" cy="60" r="18" fill="#ffcf6b"/></svg>';
  return `<div class="tprev" aria-label="Preview"><span class="label">Preview</span>
    <div class="mbar"><span>${logo}${s.tray_pick ? `<b>${esc(fav)} ${chance}</b>` : ''}</span><span class="muted">12:04</span></div>
    ${s.notify_new_pick || s.notify_result ? `<div class="toast">${logo}<div><b>Augur</b><small>${s.notify_new_pick ? `${esc(fav)} to win, ${chance}` : `${esc(fav)} won ✓`}</small></div></div>` : '<div class="small">No notifications.</div>'}</div>`;
}

function general(s, about, live) {
  const ranged = s.poll_max_secs != null;
  const tog = (k, label, sub) => row(label, sub, toggle(`data-k="${k}"`, s[k], label));
  return `<div class="sg">
    <div class="panel"><div class="ph"><h3>Stream</h3></div>
      ${row('Duels', 'the live duels on palduels.com', '<span class="muted">palduels.com</span>')}
      ${row('Check for new fights', 'while a duel is up (the site’s page checks every 2s)', `<select id="pollmode">${[3, 5, 10, 15].map((n) => `<option value="${n}" ${!ranged && s.poll_secs === n ? 'selected' : ''}>Every ${n} seconds</option>`).join('')}<option value="range" ${ranged ? 'selected' : ''}>At random, in a range</option></select>`)}
      ${ranged ? row('Between', 'a new random wait before each check', `<div class="st3"><input type="number" data-n="poll_secs" min="2" max="60" value="${s.poll_secs}" aria-label="Fewest seconds"><span class="muted">and</span><input type="number" data-n="poll_max_secs" min="2" max="120" value="${s.poll_max_secs}" aria-label="Most seconds"><span class="muted">seconds</span></div>`) : ''}
      ${tog('backfill', 'Fetch past duels in the background', 'Gently, one page at a time; pauses while a duel is up')}
      ${tog('launch_at_login', 'Launch at login')}</div>
    <div class="panel"><div class="ph"><h3>Tray &amp; notifications</h3></div>
      ${tog('tray_pick', 'Show the pick in the tray', 'macOS: name and chance · Windows: confidence ring')}
      ${tog('notify_new_pick', 'Notify on each new pick', 'with the bet to type')}
      ${tog('notify_result', 'Notify with each result', '"Chillet Lux won ✓"')}
      ${trayPreview(s, live)}</div>
    ${
      about
        ? `<div class="panel about"><div class="ph"><h3>About Augur</h3><span>version ${esc(about.version)}</span></div>
      <div class="kpi five">${[
        ['calls', about.counts[0], 'eye'],
        ['duels', about.counts[1], 'swords'],
        ['fight tapes', about.counts[2], 'chart'],
        ['species', about.counts[3], 'egg'],
        ['your bets', about.counts[4], 'star'],
      ]
        .map(([l, n, k]) => `<div>${icon(k, 18)}<b>${fmt(n)}</b><small>${l}</small></div>`)
        .join('')}</div>
      <div class="srow"><div>Data folder<small>${esc(about.data_dir)}</small></div><button class="btn ghost" id="cpdir">Copy path</button></div>
      <div class="small">Calls, duels and settings live here; the sign-in is encrypted for this computer. Settings › Sync &amp; data backs it all up.</div></div>`
        : ''
    }
  </div>`;
}

function staking(s, preview) {
  const st = s.staking;
  const num = (k, opts = '') => `<input type="number" data-st="${k}" value="${st[k] ?? ''}" ${opts}>`;
  const percent = (k, placeholder) =>
    `<input type="number" data-st="${k}" data-pct="1" min="1" max="99" value="${st[k] == null ? '' : Math.round(st[k] * 100)}" placeholder="${placeholder}" style="width:84px">`;
  const options = Object.entries(STRATEGY)
    .map(([k, [name, sub]]) => `<button class="opt ${st.strategy === k ? 'on' : ''}" data-strategy="${k}"><b>${name}</b><small>${sub}</small></button>`)
    .join('');
  const specific =
    st.strategy === 'scaled'
      ? row('Bets start rising from', 'below it, the default bet (a bare !h)', percent('scale_from', '55')) +
        row('Up to the max at', 'from here up, !h all', percent('scale_to', '85'))
      : st.strategy === 'fixed'
        ? row('Amount', 'credits on every call', num('fixed_amount', 'style="width:120px"'))
        : row('Max bet from', 'the chance where !h all starts; below it a bare !h', percent('max_from', '70'));
  const max = Math.max(...preview.map(([, k]) => k.amount), 1);
  const bars = preview
    .map(([p, k]) => `<div class="pv"><span>${pct(p)}</span><div class="pvb"><i style="width:${(100 * k.amount) / max}%"></i></div><b>${k.amount ? esc(k.command) : '<span class="muted">sit out</span>'}</b><small>${k.amount ? fmt(k.amount) : ''}</small></div>`)
    .join('');
  return `<div class="sg">
    <div class="panel"><div class="ph"><h3>How to stake</h3><span>suggestions only: you type the bet</span></div>
      <div class="opts">${options}</div>
      ${specific}
      ${row('Sit out below', 'empty: always suggest a bet', percent('skip_below', 'Never'))}
      ${row('Bet limits', 'min · max · a bare !h; the site doesn’t publish them', `<div class="st3">${num('min_bet', 'aria-label="Minimum bet"')}${num('max_bet', 'aria-label="Maximum bet"')}${num('default_bet', 'aria-label="Default bet"')}</div>`)}
      ${row('Type the max as "all"', '!h all stays right if the stream changes the max', toggle('data-stk="max_as_all"', st.max_as_all, 'Type the max as all'))}
      ${row('Track your bets', 'signed in, Augur reads your picks list to compare with its calls', toggle('data-k="import_bets"', s.import_bets, 'Track your bets'))}</div>
    <div class="panel col"><div class="ph"><h3>What it would suggest</h3><span>for a call at each chance</span></div>
      <div class="pvs">${bars}</div>
      <div class="small" style="margin-top:10px">${esc(preview.find(([p]) => p === 0.75)?.[1].why || '')}</div></div>
  </div>`;
}

function chat(s, live) {
  const lines = live?.prediction?.lines;
  return `<div class="sg">
    <div class="panel"><div class="ph"><h3>Chat lines</h3><span>shown in the tray and Live</span></div>
      ${row('Show the chat lines', 'the lines to paste in chat and their copy button, in the tray and on Live', toggle('data-k="chat_lines"', s.chat_lines, 'Show the chat lines'))}
      ${row('Voices', 'tap to show or hide', `<div class="vchips">${VOICES.map((v) => `<button class="${s.voices.includes(v) ? 'on' : ''}" data-voice="${esc(v)}">${esc(SHORT[v])}</button>`).join('')}</div>`)}
      ${row('Default voice', '', `<select data-n="default_voice">${VOICES.map((v) => `<option ${s.default_voice === v ? 'selected' : ''}>${esc(v)}</option>`).join('')}</select>`)}
      ${row('Include the chance', 'adds "(83%)" to the line', toggle('data-k="include_chance"', s.include_chance, 'Include the chance'))}</div>
    ${lines ? `<div class="panel list"><div class="ph"><h3>This call, in each voice</h3></div>${VOICES.map((v, i) => `<div class="row"><span><b>${esc(SHORT[v])}</b> <span class="muted">·</span> ${esc(lines[i])}</span></div>`).join('')}</div>` : ''}
  </div>`;
}

function modelPage(s) {
  const own = !s.model_auto;
  return `<div class="sg">
    <div class="panel"><div class="ph"><h3>Model</h3><span>${own ? 'your own settings' : 'tunes itself'}</span></div>
      ${row('Tune itself', 'Augur re-fits its settings as results come in and keeps a change only when it predicts the newest duels better (Model tab). Off: it uses the values below.', toggle('id="mauto"', s.model_auto, 'Tune itself'))}
      ${own ? Object.entries(KNOBS).map(([k, [label, lo, hi]]) => row(label, `${lo} to ${hi}`, `<input type="number" step="0.05" min="${lo}" max="${hi}" data-m="${k}" value="${s.model[k]}" style="width:90px">`)).join('') : ''}
      ${own ? row('Start again', 'copy what the model has learned into the values above', '<button class="btn ghost" id="mcopy">Use the learned values</button>') : ''}</div>
    <div class="panel"><div class="ph"><h3>How it learns</h3></div><div class="small" style="line-height:1.7">
      Types, move power and records learn from every past duel; the stat settings and cooldown passives wait for real calls, which see each pal's actual stats. The newest quarter of each is held back to check a change, so it can't just memorise the past. Learning carries on while you use your own settings.</div></div>
  </div>`;
}

function account(s, acct) {
  return `<div class="sg">
    <div class="panel"><div class="ph"><h3>PalDuels</h3><span>optional</span></div>
      ${row('Sign-in', `${acct.signed_in ? `Signed in${acct.name ? ' as <b>' + esc(acct.name) + '</b>' : ''}. ` : ''}Only reads your picks and balance. Augur never picks, chats or spends for you.`, acct.signed_in ? '<button class="btn ghost" id="so">Sign out</button>' : '<button class="btn" id="si">Sign in</button>')}
      ${row('Your Kick name', 'for your pal box and rivals; public, no sign-in needed', `<input type="text" data-n="kick_name" value="${esc(s.kick_name)}" placeholder="TrongatesLegacy">`)}
      ${row('Reroll cost (credits)', 'not on the site yet; enables exact reroll advice', `<input type="number" data-n="reroll_cost" value="${s.reroll_cost ?? ''}" placeholder="Not set" style="width:120px">`)}</div>
    <div class="panel"><div class="ph"><h3>Kept safe</h3></div><div class="small" style="line-height:1.7">
      The sign-in saves only the site's session cookie, never your password, encrypted with ${navigator.userAgent.includes('Windows') ? 'Windows (DPAPI)' : 'your Mac’s Keychain'}. It stays on this computer: sync doesn’t carry it, so sign in once on each.</div></div>
  </div>`;
}

function syncPage(v, flashMsg) {
  const where = v.folder ? esc(v.folder) : 'No folder chosen';
  const status = !v.on
    ? '<span class="dot idle"></span>Off'
    : v.error
      ? `<span class="dot bad"></span>${esc(v.error)}`
      : `<span class="dot"></span>Synced ${ago(v.last_sync)}${v.others.length ? ` with ${v.others.map((o) => esc(o.name)).join(', ')}` : ' · no other computers yet'}`;
  const folders = v.folders
    .map((f) => `<button class="opt ${v.folder === f.path ? 'on' : ''}" data-folder="${esc(f.path)}"><b><span class="fi">${FOLDER_ICON[f.id] || '▣'}</span>${esc(f.name)}</b><small>${esc(f.path)}</small></button>`)
    .join('');
  const others = v.others.length
    ? v.others.map((o) => `<div class="row"><span><b>${esc(o.name)}</b> <span class="muted">${esc(o.os)} · Augur ${esc(o.app)} · ${fmt(o.calls)} calls</span></span><span class="small">${ago(o.at)}</span></div>`).join('')
    : '<div class="small">None yet: turn on sync on your other computer and pick the same folder.</div>';
  const history = v.history.length
    ? v.history.map((h) => `<div class="row"><span>${esc(h.summary)} <span class="muted">from ${esc(h.from)}</span></span><span class="small">${ago(h.at)}</span></div>`).join('')
    : '<div class="small">Nothing has changed here from another computer yet.</div>';
  return `<div class="sg">
    <div style="display:grid;gap:14px">
      <div class="panel"><div class="ph"><h3>Sync</h3>${toggle('data-sync="on"', v.on, 'Sync')}</div>
        <div class="syst">${status}</div>
        <div class="small" style="margin:8px 0 10px">Through a folder a sync app already moves. Each computer writes only its own file there, so nothing clashes; the latest change to a setting wins.</div>
        <div class="opts col">${folders}<button class="opt ${v.folder && !v.folders.some((f) => f.path === v.folder) ? 'on' : ''}" id="choose"><b><span class="fi">▣</span>Another folder…</b><small>${v.folder && !v.folders.some((f) => f.path === v.folder) ? where : 'Syncthing, a network drive…'}</small></button></div>
        ${row('This computer', 'how the others see it', `<input type="text" id="sname" value="${esc(v.name)}" maxlength="40">`)}
        ${row('Settings', 'staking, voices, notifications, the model', toggle('data-what="settings"', v.what.settings, 'Sync settings'))}
        ${row('Augur’s calls', 'so every computer has the same record', toggle('data-what="calls"', v.what.calls, 'Sync calls'))}
        <div class="small foot">Stays on each computer: the sign-in, launch at login and background fetching. Duels, species and your bets come from the site on each.</div></div>
    </div>
    <div class="stackc">
      <div class="panel list"><div class="ph"><h3>Other computers</h3></div>${others}</div>
      <div class="panel col list grow"><div class="ph"><h3>Changes from them</h3></div>${v.history.length ? `<div class="fillin">${history}</div>` : empty('queue', 'No changes yet', 'Nothing has changed here from another computer yet.')}</div>
      <div class="panel"><div class="ph"><h3>Back up &amp; import</h3></div>
        <div class="small" style="margin-bottom:10px">A backup is the whole store: calls, duels, tapes, species and bets. Import one on a new computer to skip the hour-long first fetch. Importing adds what's missing and never overwrites.</div>
        <div style="display:flex;gap:8px"><button class="btn" id="bk">Back up…</button><button class="btn ghost" id="im">Import…</button></div>
        ${flashMsg ? `<div class="small ok" style="margin-top:10px">${flashMsg}</div>` : ''}</div>
    </div>
  </div>`;
}

const day = (ms) => new Date(ms).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' });
const mb = (n) => (n / 1e6).toFixed(n >= 1e7 ? 0 : 1);

// the Updates page's hero: where things stand and the one thing to do next
function updateHero(u) {
  const r = u.latest;
  const p = u.progress;
  const share = p && p[1] ? p[0] / p[1] : null;
  const ring = (cls, glyph, frac) =>
    `<div class="uring ${cls}"><svg viewBox="0 0 44 44" aria-hidden="true"><circle cx="22" cy="22" r="19" class="t"/>${frac != null ? `<circle cx="22" cy="22" r="19" class="f" style="stroke-dasharray:${(frac * 119.4).toFixed(1)} 119.4"/>` : ''}</svg><span>${glyph}</span></div>`;
  if (u.installing) return { ring: ring('go', '↻', 1), title: 'Restarting…', sub: `Installing Augur ${esc(r?.version || '')}. It’s back in a few seconds.`, act: '' };
  if (p) return { ring: ring('go', share != null ? `${Math.floor(share * 100)}` : '…', share ?? 0.25), title: `Downloading ${esc(r?.version || '')}`, sub: share != null ? `${mb(p[0])} of ${mb(p[1])} MB` : `${mb(p[0])} MB`, act: '' };
  if (r && u.ready)
    return {
      ring: ring('ready', '✦', 1),
      title: `Augur ${esc(r.version)} is ready`,
      sub: `Downloaded${r.published ? ` · published ${day(r.published)}` : ''}. Restarting takes a few seconds; your calls, settings and sign-in stay.${shown.live?.live ? ' <span class="gold">A duel is on: maybe wait for the result.</span>' : ''}`,
      act: '<button class="btn big" id="ui">Restart to update</button>',
    };
  if (r)
    return { ring: ring('ready', '↓', null), title: `Augur ${esc(r.version)} is out`, sub: r.published ? `published ${day(r.published)}` : '', act: '<button class="btn big" id="ui">Download and restart</button>' };
  if (u.checking) return { ring: ring('spin', '', 0.25), title: 'Checking…', sub: 'Asking GitHub for the latest release', act: '' };
  if (u.error) return { ring: ring('bad', '!', null), title: 'Couldn’t check', sub: esc(u.error), act: '' };
  return { ring: ring('ok', '✓', 1), title: 'Up to date', sub: u.checked_at ? `Checked ${ago(u.checked_at)}` : 'Not checked yet', act: '' };
}

function updates(s, u) {
  const h = updateHero(u);
  const r = u.latest;
  return `<div class="sg">
    <div style="display:grid;gap:14px;align-content:start">
      <div class="panel uhero">${h.ring}<div class="uh"><span class="label">Augur ${esc(u.current)}</span><b>${h.title}</b><small>${h.sub}</small></div>
        <div class="ua">${h.act}<button class="btn ghost" id="uc" ${u.checking || u.progress || u.installing ? 'disabled' : ''}>Check now</button></div></div>
      <div class="panel col grow">
        ${row('Check automatically', 'shortly after Augur starts, then every 6 hours', toggle('data-k="check_updates"', s.check_updates, 'Check for updates automatically'))}
        ${row('Download automatically', 'so Restart to update is instant; installing always waits for your click', toggle('data-k="auto_update"', s.auto_update, 'Download updates automatically'))}
        <div class="small foot">Updates come from the project’s GitHub releases and are signed with its own key. Augur refuses one that isn’t.</div></div>
    </div>
    <div class="stackc">
      ${u.updated_from ? `<div class="panel note-card"><div class="ph"><h3>Updated to ${esc(u.current)}</h3><button class="btn ghost" id="us">Got it</button></div><div class="small" style="margin-bottom:8px">from ${esc(u.updated_from)} · here’s what changed</div>${notesList(u.updated_notes)}</div>` : ''}
      ${r ? `<div class="panel col grow"><div class="ph"><h3>What’s new in ${esc(r.version)}</h3>${r.published ? `<span>${day(r.published)}</span>` : ''}</div>${notesList(r.notes)}</div>` : u.updated_from ? '' : `<div class="panel col grow"><div class="ph"><h3>What’s new</h3></div>${empty('star', 'Nothing new', 'When a new version is out, its notes show here.')}</div>`}
    </div>
  </div>`;
}

// the app's state, for the Updates page (is a duel on?)
let shown = {};

export async function render(app) {
  shown = app.state;
  const { state, main, header, flash } = app;
  state.settings = await invoke('get_settings');
  state.settingsPage ||= 'general';
  const page = state.settingsPage;
  const s = state.settings;
  const [acct, preview, sync, upd, about] = await Promise.all([
    page === 'account' ? invoke('account') : null,
    page === 'staking' ? invoke('stake_preview', { staking: s.staking }) : null,
    page === 'sync' ? invoke('sync_view') : null,
    page === 'updates' ? invoke('update_view') : null,
    page === 'general' ? invoke('about') : null,
  ]);
  if (state.tab !== 'settings' || state.settingsPage !== page) return;
  const tabs = `<div class="seg ml">${PAGES.map(([k, l]) => `<button class="${page === k ? 'on' : ''}" data-page="${k}">${l}</button>`).join('')}</div>`;
  const body = { general: () => general(s, about, state.live), staking: () => staking(s, preview), chat: () => chat(s, state.live), model: () => modelPage(s), account: () => account(s, acct), sync: () => syncPage(sync, state.syncFlash), updates: () => updates(s, upd) }[page]();
  main.innerHTML = header('Settings', tabs) + body;
  state.syncFlash = null;

  const save = async (redraw) => {
    try {
      await invoke('save_settings', { settings: s });
      if (redraw) render(app);
    } catch (e) {
      flash(e);
    }
  };
  const flip = (b, obj, k) => {
    obj[k] = !obj[k];
    b.classList.toggle('on', obj[k]);
    b.setAttribute('aria-pressed', obj[k]);
  };
  main.querySelectorAll('[data-page]').forEach((b) => (b.onclick = () => ((state.settingsPage = b.dataset.page), render(app))));
  main.querySelectorAll('[data-k]').forEach((b) => (b.onclick = () => (flip(b, s, b.dataset.k), save())));
  main.querySelectorAll('[data-stk]').forEach((b) => (b.onclick = () => (flip(b, s.staking, b.dataset.stk), save(true))));
  main.querySelectorAll('[data-strategy]').forEach((b) => (b.onclick = () => ((s.staking.strategy = b.dataset.strategy), save(true))));
  main.querySelectorAll('[data-voice]').forEach((b) => {
    b.onclick = () => {
      const v = b.dataset.voice;
      s.voices = s.voices.includes(v) ? s.voices.filter((x) => x !== v) : VOICES.filter((x) => x === v || s.voices.includes(x));
      if (!s.voices.length) s.voices = [v];
      b.classList.toggle('on', s.voices.includes(v));
      save();
    };
  });
  main.querySelectorAll('[data-n]').forEach((input) => {
    input.onchange = () => {
      const k = input.dataset.n;
      if (k === 'poll_secs' || k === 'poll_max_secs') s[k] = Math.min(120, Math.max(2, Math.round(+input.value) || 2));
      else if (k === 'reroll_cost') s[k] = input.value ? +input.value : null;
      else s[k] = input.value.trim();
      save();
    };
  });
  document.getElementById('pollmode')?.addEventListener('change', (e) => {
    const v = e.target.value;
    if (v === 'range') {
      s.poll_max_secs = Math.max(s.poll_secs + 5, 10);
    } else {
      s.poll_secs = +v;
      s.poll_max_secs = null;
    }
    save(true);
  });
  main.querySelectorAll('[data-st]').forEach((input) => {
    input.onchange = () => {
      const st = s.staking;
      const k = input.dataset.st;
      const v = input.value.trim();
      if (input.dataset.pct) st[k] = v ? Math.min(99, Math.max(1, +v)) / 100 : k === 'skip_below' ? null : st[k];
      else if (v) st[k] = Math.max(0, Math.round(+v));
      save(true);
    };
  });
  // Model: switching to your own settings starts from what it learned
  const learned = async () => (s.model = await invoke('learned_settings'));
  document.getElementById('mauto')?.addEventListener('click', async () => {
    s.model_auto = !s.model_auto;
    if (!s.model_auto) await learned();
    save(true);
  });
  document.getElementById('mcopy')?.addEventListener('click', async () => (await learned(), save(true)));
  main.querySelectorAll('[data-m]').forEach((input) => {
    input.onchange = () => {
      const [, lo, hi] = KNOBS[input.dataset.m];
      if (input.value !== '') s.model[input.dataset.m] = Math.min(hi, Math.max(lo, +input.value));
      save(true);
    };
  });
  document.getElementById('si')?.addEventListener('click', () => invoke('sign_in'));
  document.getElementById('cpdir')?.addEventListener('click', (e) => copyText(about.data_dir, e.target));
  document.getElementById('so')?.addEventListener('click', async () => {
    await invoke('sign_out');
    render(app);
  });

  // Updates
  document.getElementById('uc')?.addEventListener('click', async (e) => {
    e.target.disabled = true;
    await invoke('update_check').catch(flash);
    render(app);
  });
  document.getElementById('ui')?.addEventListener('click', (e) => {
    e.target.disabled = true;
    invoke('update_install').catch((err) => (flash(err), render(app)));
  });
  document.getElementById('us')?.addEventListener('click', async () => {
    await invoke('update_seen');
    app.badge?.(await invoke('update_view'));
    render(app);
  });

  // Sync & data
  const setSync = async (change) => {
    try {
      await invoke('sync_set', { change });
      render(app);
    } catch (e) {
      flash(e);
    }
  };
  main.querySelector('[data-sync]')?.addEventListener('click', () => (sync.folder ? setSync({ on: !sync.on }) : flash('Pick a folder to sync through first.')));
  main.querySelectorAll('[data-folder]').forEach((b) => (b.onclick = () => setSync({ folder: b.dataset.folder })));
  main.querySelectorAll('[data-what]').forEach((b) => (b.onclick = () => setSync({ what: { ...sync.what, [b.dataset.what]: !sync.what[b.dataset.what] } })));
  document.getElementById('sname')?.addEventListener('change', (e) => setSync({ name: e.target.value }));
  document.getElementById('choose')?.addEventListener('click', async () => {
    const folder = await invoke('choose_sync_folder');
    if (folder) setSync({ folder });
  });
  document.getElementById('bk')?.addEventListener('click', async () => {
    try {
      const path = await invoke('backup');
      if (path) (state.syncFlash = `Backed up to ${esc(path)}`), render(app);
    } catch (e) {
      flash(e);
    }
  });
  document.getElementById('im')?.addEventListener('click', async () => {
    try {
      const r = await invoke('import_file');
      if (!r) return;
      const m = r.merged;
      const parts = [[m.calls, 'call'], [m.duels, 'duel'], [m.tapes, 'tape'], [m.species, 'species'], [m.bets, 'bet']]
        .filter(([n]) => n)
        .map(([n, w]) => `${fmt(n)} ${w}${n === 1 || w === 'species' ? '' : 's'}`);
      state.syncFlash = `From ${esc(r.file)}: ${parts.length ? 'added ' + parts.join(', ') : 'nothing new'}.`;
      render(app);
    } catch (e) {
      flash(e);
    }
  });
}
