// Settings: how often to check, the tray and notifications, staking, the chat voices, the optional sign-in.
import { invoke } from '../lib/bridge.js';
import { esc } from '../lib/format.js';
import { SHORT, VOICES } from '../lib/pal.js';

export async function render(app) {
  const { state, main, header, flash } = app;
  state.settings = await invoke('get_settings');
  const acct = await invoke('account');
  const s = state.settings;
  const tog = (key, label, sub) =>
    `<div class="srow"><div>${label}${sub ? `<small>${sub}</small>` : ''}</div><button class="tog ${s[key] ? 'on' : ''}" data-k="${key}" aria-pressed="${!!s[key]}" aria-label="${esc(label)}"></button></div>`;
  const st = s.staking;
  const num = (key, opts = '') => `<input type="number" data-st="${key}" value="${st[key] ?? ''}" ${opts}>`;
  const percent = (key, placeholder) =>
    `<input type="number" data-st="${key}" data-pct="1" min="1" max="99" value="${st[key] == null ? '' : Math.round(st[key] * 100)}" placeholder="${placeholder}" style="width:80px">`;
  const strategy = { kelly: 'Kelly: more on bigger edges', fixed: 'Fixed amount', default_or_max: 'Default or max, by chance' };
  const staking = `<div class="panel"><div class="ph"><h3>Staking</h3><span>suggestions only</span></div>
        <div class="srow"><div>Strategy<small>Augur suggests the bet; you type it in chat</small></div><select data-st="strategy">${Object.entries(strategy).map(([k, l]) => `<option value="${k}" ${st.strategy === k ? 'selected' : ''}>${l}</option>`).join('')}</select></div>
        ${
          st.strategy === 'kelly'
            ? `<div class="srow"><div>Kelly share<small>a quarter is steadier when the chance is a little off</small></div><select data-st="kelly_fraction">${[
                [0.125, 'An eighth'],
                [0.25, 'A quarter'],
                [0.5, 'Half'],
                [1, 'Full'],
              ].map(([v, l]) => `<option value="${v}" ${st.kelly_fraction === v ? 'selected' : ''}>${l}</option>`).join('')}</select></div>
        <div class="srow"><div>Balance when signed out<small>Kelly sizes from your balance; signed in, Augur reads it</small></div>${num('bankroll', 'placeholder="Not set" style="width:110px"')}</div>`
            : st.strategy === 'fixed'
              ? `<div class="srow"><div>Amount</div>${num('fixed_amount', 'style="width:110px"')}</div>`
              : `<div class="srow"><div>Max bet from (%)<small>below it, the default bet</small></div>${percent('max_from', '70')}</div>`
        }
        <div class="srow"><div>Sit out below (%)<small>empty: always suggest a bet</small></div>${percent('skip_below', 'Never')}</div>
        <div class="srow"><div>Bet limits<small>min · max · default (a bare !h); the site doesn't publish them</small></div><div class="st3">${num('min_bet')}${num('max_bet')}${num('default_bet')}</div></div>
        <div class="srow"><div>Type the max as "all"<small>!h all stays right if the stream changes the max</small></div><button class="tog ${st.max_as_all ? 'on' : ''}" data-stk="max_as_all" aria-pressed="${!!st.max_as_all}" aria-label="Type the max as all"></button></div>
        ${tog('import_bets', 'Track your bets', 'signed in, Augur reads your picks list to compare with its calls')}</div>`;
  main.innerHTML =
    header('Settings') +
    `<div class="sg">
    <div style="display:grid;gap:14px">
      <div class="panel"><div class="ph"><h3>Stream</h3></div>
        <div class="srow"><div>Duels<small>the live duels on palduels.com</small></div><span class="muted">palduels.com</span></div>
        <div class="srow"><div>Check for new fights<small>while a duel is up (the site's page checks every 2s)</small></div><select data-n="poll_secs">${[3, 5, 10, 15].map((n) => `<option value="${n}" ${s.poll_secs === n ? 'selected' : ''}>Every ${n} seconds</option>`).join('')}</select></div>
        ${tog('backfill', 'Fetch past duels in the background', 'Gently, one page at a time; pauses while a duel is up')}</div>
      <div class="panel"><div class="ph"><h3>Tray &amp; notifications</h3></div>
        ${tog('tray_pick', 'Show the pick in the tray', 'macOS: name and chance · Windows: confidence ring')}
        ${tog('notify_new_pick', 'Notify on each new pick')}
        ${tog('notify_result', 'Notify with each result', '"Chillet Lux won ✓"')}
        ${tog('launch_at_login', 'Launch at login')}</div>

      <div class="panel"><div class="ph"><h3>Account</h3><span>optional</span></div>
        <div class="srow"><div>PalDuels sign-in<small>${acct.signed_in ? `Signed in${acct.name ? ' as ' + esc(acct.name) : ''}. ` : ''}Only reads your picks and balance. Augur never picks, chats or spends for you. Saved encrypted (Keychain on macOS, DPAPI on Windows).</small></div>${acct.signed_in ? '<button class="btn ghost" id="so">Sign out</button>' : '<button class="btn ghost" id="si">Sign in</button>'}</div>
        <div class="srow"><div>Your Kick name<small>for your pal box and rivals; public, no sign-in needed</small></div><input type="text" data-n="kick_name" value="${esc(s.kick_name)}" placeholder="TrongatesLegacy"></div>
        <div class="srow"><div>Reroll cost (credits)<small>not on the site yet; enables exact reroll advice</small></div><input type="number" data-n="reroll_cost" value="${s.reroll_cost ?? ''}" placeholder="Not set" style="width:110px"></div></div>
    </div>
    <div style="display:grid;gap:14px">
      ${staking}
      <div class="panel"><div class="ph"><h3>Chat lines</h3></div>
        <div class="srow"><div>Voices<small>shown in the tray and Live tab</small></div><div class="vchips">${VOICES.map((v) => `<button class="${s.voices.includes(v) ? 'on' : ''}" data-voice="${esc(v)}">${esc(SHORT[v])}</button>`).join('')}</div></div>
        <div class="srow"><div>Default voice</div><select data-n="default_voice">${VOICES.map((v) => `<option ${s.default_voice === v ? 'selected' : ''}>${esc(v)}</option>`).join('')}</select></div>
        ${tog('include_chance', 'Include the chance', 'adds "(83%)" to the line')}</div>

    </div></div>`;
  const save = () => invoke('save_settings', { settings: s }).catch((e) => flash(e));
  main.querySelectorAll('.tog').forEach((b) => {
    b.onclick = () => {
      s[b.dataset.k] = !s[b.dataset.k];
      b.classList.toggle('on', s[b.dataset.k]);
      b.setAttribute('aria-pressed', s[b.dataset.k]);
      save();
    };
  });
  main.querySelectorAll('[data-stk]').forEach((b) => {
    b.onclick = () => {
      st[b.dataset.stk] = !st[b.dataset.stk];
      b.classList.toggle('on', st[b.dataset.stk]);
      b.setAttribute('aria-pressed', st[b.dataset.stk]);
      save();
    };
  });
  main.querySelectorAll('[data-st]').forEach((input) => {
    input.onchange = () => {
      const k = input.dataset.st;
      const v = input.value.trim();
      if (k === 'strategy') st[k] = v;
      else if (input.dataset.pct) st[k] = v ? Math.min(99, Math.max(1, +v)) / 100 : k === 'skip_below' ? null : st[k];
      else if (k === 'bankroll') st[k] = v ? Math.max(0, Math.round(+v)) : null;
      else if (k === 'kelly_fraction') st[k] = +v;
      else if (v) st[k] = Math.max(0, Math.round(+v));
      save();
      if (k === 'strategy') render(app);
    };
  });
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
  document.getElementById('si')?.addEventListener('click', () => invoke('sign_in'));
  document.getElementById('so')?.addEventListener('click', async () => {
    await invoke('sign_out');
    render(app);
  });
}
