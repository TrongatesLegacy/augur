// Settings, in five pages: General (checking, the tray and notifications), Staking (with what it would suggest),
// Chat lines, Account, and Sync & data (sync across computers, back up, import).
import { invoke } from '../lib/bridge.js';
import { ago, esc, fmt, pct } from '../lib/format.js';
import { SHORT, VOICES } from '../lib/pal.js';

const PAGES = [
  ['general', 'General'],
  ['staking', 'Staking'],
  ['chat', 'Chat lines'],
  ['account', 'Account'],
  ['sync', 'Sync & data'],
];

const STRATEGY = {
  kelly: ['Kelly', 'More on bigger edges, nothing when there’s none'],
  fixed: ['Fixed amount', 'The same on every call'],
  default_or_max: ['Default or max', 'A bare !h, or !h all on strong calls'],
};

const FOLDER_ICON = { onedrive: '☁', icloud: '☁', gdrive: '▲', dropbox: '◆' };

const row = (label, sub, control) => `<div class="srow"><div>${label}${sub ? `<small>${sub}</small>` : ''}</div>${control}</div>`;
const toggle = (attr, on, label) => `<button class="tog ${on ? 'on' : ''}" ${attr} aria-pressed="${!!on}" aria-label="${esc(label)}"></button>`;

function general(s) {
  const tog = (k, label, sub) => row(label, sub, toggle(`data-k="${k}"`, s[k], label));
  return `<div class="sg">
    <div class="panel"><div class="ph"><h3>Stream</h3></div>
      ${row('Duels', 'the live duels on palduels.com', '<span class="muted">palduels.com</span>')}
      ${row('Check for new fights', 'while a duel is up (the site’s page checks every 2s)', `<select data-n="poll_secs">${[3, 5, 10, 15].map((n) => `<option value="${n}" ${s.poll_secs === n ? 'selected' : ''}>Every ${n} seconds</option>`).join('')}</select>`)}
      ${tog('backfill', 'Fetch past duels in the background', 'Gently, one page at a time; pauses while a duel is up')}
      ${tog('launch_at_login', 'Launch at login')}</div>
    <div class="panel"><div class="ph"><h3>Tray &amp; notifications</h3></div>
      ${tog('tray_pick', 'Show the pick in the tray', 'macOS: name and chance · Windows: confidence ring')}
      ${tog('notify_new_pick', 'Notify on each new pick', 'with the bet to type')}
      ${tog('notify_result', 'Notify with each result', '"Chillet Lux won ✓"')}</div>
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
    st.strategy === 'kelly'
      ? row('Kelly share', 'a quarter is steadier when the chance is a little off', `<select data-st="kelly_fraction">${[[0.125, 'An eighth'], [0.25, 'A quarter'], [0.5, 'Half'], [1, 'Full']].map(([v, l]) => `<option value="${v}" ${st.kelly_fraction === v ? 'selected' : ''}>${l}</option>`).join('')}</select>`) +
        row('Balance when signed out', 'Kelly sizes from your balance; signed in, Augur reads it', num('bankroll', 'placeholder="Not set" style="width:120px"'))
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
    <div class="panel"><div class="ph"><h3>What it would suggest</h3><span>for a call at each chance</span></div>
      <div class="pvs">${bars}</div>
      <div class="small" style="margin-top:10px">${esc(preview.find(([p]) => p === 0.75)?.[1].why || '')}</div></div>
  </div>`;
}

function chat(s, live) {
  const lines = live?.prediction?.lines;
  return `<div class="sg">
    <div class="panel"><div class="ph"><h3>Chat lines</h3><span>shown in the tray and Live</span></div>
      ${row('Voices', 'tap to show or hide', `<div class="vchips">${VOICES.map((v) => `<button class="${s.voices.includes(v) ? 'on' : ''}" data-voice="${esc(v)}">${esc(SHORT[v])}</button>`).join('')}</div>`)}
      ${row('Default voice', '', `<select data-n="default_voice">${VOICES.map((v) => `<option ${s.default_voice === v ? 'selected' : ''}>${esc(v)}</option>`).join('')}</select>`)}
      ${row('Include the chance', 'adds "(83%)" to the line', toggle('data-k="include_chance"', s.include_chance, 'Include the chance'))}</div>
    ${lines ? `<div class="panel list"><div class="ph"><h3>This call, in each voice</h3></div>${VOICES.map((v, i) => `<div class="row"><span><b>${esc(SHORT[v])}</b> <span class="muted">·</span> ${esc(lines[i])}</span></div>`).join('')}</div>` : ''}
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
        <div class="small">Stays on each computer: the sign-in, launch at login and background fetching. Duels, species and your bets come from the site on each.</div></div>
    </div>
    <div style="display:grid;gap:14px;align-content:start">
      <div class="panel list"><div class="ph"><h3>Other computers</h3></div>${others}</div>
      <div class="panel list"><div class="ph"><h3>Changes from them</h3></div>${history}</div>
      <div class="panel"><div class="ph"><h3>Back up &amp; import</h3></div>
        <div class="small" style="margin-bottom:10px">A backup is the whole store: calls, duels, tapes, species and bets. Import one on a new computer to skip the hour-long first fetch. Importing adds what's missing and never overwrites.</div>
        <div style="display:flex;gap:8px"><button class="btn" id="bk">Back up…</button><button class="btn ghost" id="im">Import…</button></div>
        ${flashMsg ? `<div class="small ok" style="margin-top:10px">${flashMsg}</div>` : ''}</div>
    </div>
  </div>`;
}

export async function render(app) {
  const { state, main, header, flash } = app;
  state.settings = await invoke('get_settings');
  state.settingsPage ||= 'general';
  const page = state.settingsPage;
  const s = state.settings;
  const [acct, preview, sync] = await Promise.all([
    page === 'account' ? invoke('account') : null,
    page === 'staking' ? invoke('stake_preview', { staking: s.staking }) : null,
    page === 'sync' ? invoke('sync_view') : null,
  ]);
  if (state.tab !== 'settings' || state.settingsPage !== page) return;
  const tabs = `<div class="seg ml">${PAGES.map(([k, l]) => `<button class="${page === k ? 'on' : ''}" data-page="${k}">${l}</button>`).join('')}</div>`;
  const body = { general: () => general(s), staking: () => staking(s, preview), chat: () => chat(s, state.live), account: () => account(s, acct), sync: () => syncPage(sync, state.syncFlash) }[page]();
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
      if (k === 'poll_secs') s[k] = +input.value;
      else if (k === 'reroll_cost') s[k] = input.value ? +input.value : null;
      else s[k] = input.value.trim();
      save();
    };
  });
  main.querySelectorAll('[data-st]').forEach((input) => {
    input.onchange = () => {
      const st = s.staking;
      const k = input.dataset.st;
      const v = input.value.trim();
      if (input.dataset.pct) st[k] = v ? Math.min(99, Math.max(1, +v)) / 100 : k === 'skip_below' ? null : st[k];
      else if (k === 'bankroll') st[k] = v ? Math.max(0, Math.round(+v)) : null;
      else if (k === 'kelly_fraction') st[k] = +v;
      else if (v) st[k] = Math.max(0, Math.round(+v));
      save(true);
    };
  });
  document.getElementById('si')?.addEventListener('click', () => invoke('sign_in'));
  document.getElementById('so')?.addEventListener('click', async () => {
    await invoke('sign_out');
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
