// Auto betting on the pages: Live's column (the stream's Kick chat with the auto bet's status, its Arm switch and the
// countdown's Cancel), placing that chat over its spot, and how History says which bets Augur placed.
import { invoke } from './bridge.js';
import { ago, esc, fmt } from './format.js';

// an auto bet's status (autobet::AutoBet), as History and Live say it
export const AUTO_STATUS = {
  sending: ['Sending', 'gold'],
  sent: ['Typed in', 'gold'],
  seen: ['In chat', 'gold'],
  confirmed: ['Confirmed', 'ok'],
  unconfirmed: ['Unconfirmed', 'bad'],
  failed: ['Not sent', 'bad'],
  cancelled: ['Cancelled', 'muted'],
  skipped: ['Skipped', 'muted'],
};

export const statusChip = (s) => {
  const [label, tone] = AUTO_STATUS[s] || [s, 'muted'];
  return `<span class="ast ${tone}">${esc(label)}</span>`;
};

// "Auto" or "Manual" on one of your bets, with ×2 when you and Augur both bet on the fight
export function viaChip(v) {
  if (!v) return '';
  const auto = v.via === 'auto';
  const title = auto ? `Augur placed this bet (auto betting)${v.status ? `: ${AUTO_STATUS[v.status]?.[0] || v.status}` : ''}` : 'You placed this bet';
  return `<span class="via ${auto ? 'auto' : 'man'}${v.double ? ' dbl' : ''}" title="${esc(title)}${v.double ? '. You and Augur both bet on this fight' : ''}">${auto ? 'Auto' : 'Manual'}${v.double ? ' ×2' : ''}</span>`;
}

// one line of auto betting's record: what it did on a fight, and why
export const autoRow = (b) =>
  `<div class="row arow"><span>${statusChip(b.status)} <b>${esc(b.command || '—')}</b> <span class="muted">${esc(b.home_pal)} vs ${esc(b.away_pal)}</span>${b.test ? ' <span class="ast muted">test</span>' : ''}${
    b.reason ? `<small>${esc(b.reason)}</small>` : b.reply ? `<small>${esc(b.reply.slice(0, 90))}</small>` : ''
  }</span><span class="small">${ago(b.claimed_at)}</span></div>`;

// Live's column: signed out, a way to Settings; signed in, the auto bet's status above the chat
export function column(a) {
  if (!a.signed_in)
    return `<div class="kcol"><div class="panel kpanel"><div class="ph"><h3>Auto bet</h3></div>
      <p class="small">Sign in to Kick to see the stream's chat here and bet by itself.</p>
      <button class="btn sm" id="kgo">Sign in in Settings</button></div></div>`;
  const where = a.test ? `Test mode: into your own chat (<b>${esc(a.channel || 'set your Kick name')}</b>)` : a.channel ? `Into <b>${esc(a.channel)}</b>’s chat` : 'No channel set';
  // a line each, so neither breaks in the middle: where the bets go, then today's tally
  const today = a.today?.[0] ? `Today <b>${fmt(a.today[0])}</b> bet${a.today[0] === 1 ? '' : 's'} · <b>${fmt(a.today[1])}</b> credits` : '';
  const cd = a.phase === 'countdown' && a.countdown_until;
  const status = !a.armed
    ? `<span class="dot idle"></span>Off · arm it for this session`
    : a.problem
      ? `<span class="dot bad"></span><span class="bad">Can't bet: ${esc(a.problem)}</span>`
      : cd
        ? `<span class="dot"></span><b>${esc(a.command)}</b> in <b id="kcd">${Math.max(1, Math.ceil((a.countdown_until - Date.now()) / 1000))}</b>s <button class="btn sm ghost" id="kcx">Cancel</button>`
        : `<span class="dot ${a.phase === 'done' && a.bet && ['failed', 'unconfirmed'].includes(a.bet.status) ? 'bad' : a.phase === 'waiting' || a.phase === 'idle' ? 'idle' : ''}"></span>${a.bet ? `${statusChip(a.bet.status)} <b>${esc(a.bet.command)}</b> ` : ''}${esc(a.message || 'Armed')}`;
  return `<div class="kcol"><div class="panel kpanel"><div class="ph"><h3>Auto bet</h3><button class="tog ${a.armed ? 'on' : ''}" id="karm" aria-pressed="${!!a.armed}" aria-label="Arm auto bet"></button></div>
    <div class="kst">${status}</div>
    <div class="ksub"><span>${where}</span>${today ? `<span>${today}</span>` : ''}</div>${streamLine(a.stream)}</div>
    ${a.stream && a.stream.status !== 'off' ? '<div class="panel kstream" id="kstream"><span class="small">The stream</span></div>' : ''}
    <div class="panel kchat" id="kchat"><span class="small">Kick chat</span></div></div>`;
}

// What the stream reader is doing, under auto bet's status: a chip, then the hidden side and its stats (or what
// it's waiting on, on hover), then how far behind live. With nothing hidden, its check on the fight instead. A
// warning instead when it keeps missing. Nothing while idle.
function streamLine(v) {
  if (!v || v.status === 'off') return '';
  if (v.warning) return `<div class="kstr"><span class="ast bad">Stream</span><span class="bad">${esc(v.warning)}</span></div>`;
  if (!['watching', 'read', 'checking', 'checked', 'misread'].includes(v.status)) return '';
  const how = [v.height ? `${v.height}p` : '', v.latency != null ? `${v.latency.toFixed(1)}s behind live` : ''].filter(Boolean).join(' · ');
  const behind = how ? `<small>${how}</small>` : '';
  if (v.status === 'checking') return `<div class="kstr" title="${esc(v.message)}"><span class="ast gold">Checking</span><span>the reader on this fight</span>${behind}</div>`;
  if (v.status === 'checked' || v.status === 'misread') {
    const ok = v.status === 'checked';
    return `<div class="kstr" title="${esc(v.message)}"><span class="ast ${ok ? 'ok' : 'bad'}">${ok ? 'Checked' : 'Misread'}</span><span>${ok ? 'both sides read right' : 'a side read wrong'}</span>${behind}</div>`;
  }
  const side = v.side ? `${esc(v.side)} side` : 'the hidden side';
  if (v.status === 'read' && v.stats) {
    const nums = v.stats.map((n) => `<b>${fmt(n)}</b>`).join('<i>/</i>');
    return `<div class="kstr" title="${esc(v.message)}"><span class="ast ok">Read</span><span>${side} ${nums}</span>${behind}</div>`;
  }
  return `<div class="kstr" title="${esc(v.message)}"><span class="ast gold">Watching</span><span>for the ${side}’s stats</span>${behind}</div>`;
}

// The webviews' spot on Live: the page's CSS pixels, which the app scales by the window's zoom. None when it's not
// on the page.
const spotOf = (id) => {
  const r = document.getElementById(id)?.getBoundingClientRect();
  return r && r.width > 40 ? { x: r.left, y: r.top, w: r.width, h: r.height, visible: true } : null;
};

// Puts the chat webview over its spot on Live (or hides it, with no spot): the page's CSS pixels, which the app
// scales by the window's zoom.
// (and the stream's player over its spot above it)
export function placeChat() {
  const spot = spotOf('kchat') || { x: 0, y: 0, w: 0, h: 0, visible: false };
  return invoke('chat_place', { spot, stream: spotOf('kstream') }).catch(() => {});
}

// Live's and the popover's Copy beside an armed auto bet: its status, and whether Copy is dimmed (an auto bet on this
// fight has gone out, so copying would bet twice). Copy stays usable either way: after a Cancel, a skip or a failed
// send the bet is yours to type. Test mode bets into your own chat, so it changes nothing here.
export const WENT_OUT = ['sending', 'sent', 'seen', 'confirmed', 'unconfirmed'];
export function besideCopy(a) {
  if (!a?.enabled || !a.armed || a.test) return { note: '', dim: false };
  const out = !!a.bet && WENT_OUT.includes(a.bet.status);
  const chip = a.bet ? statusChip(a.bet.status) : a.phase === 'countdown' ? '<span class="ast gold">Soon</span>' : '<span class="ast">Armed</span>';
  return { note: `<span class="anote" title="Auto bet: ${esc(a.message || 'armed')}">${chip}</span>`, dim: out };
}

// Hides the chat webview (another tab is showing)
export const hideChat = () => invoke('chat_place', { spot: { x: 0, y: 0, w: 0, h: 0, visible: false }, stream: null }).catch(() => {});

let tick = null;

// Live's column, wired: Arm, Cancel, the countdown ticking, and the chat placed over its spot
export function wireColumn(app, rerender) {
  clearInterval(tick);
  const { state, flash } = app;
  document.getElementById('kgo')?.addEventListener('click', () => ((state.settingsPage = 'autobet'), app.show('settings')));
  document.getElementById('karm')?.addEventListener('click', async () => {
    try {
      state.auto = await invoke('auto_arm', { on: !state.auto.armed });
      rerender();
    } catch (e) {
      flash(e);
    }
  });
  document.getElementById('kcx')?.addEventListener('click', () => invoke('auto_cancel').catch(flash));
  const cd = document.getElementById('kcd');
  if (cd) tick = setInterval(() => (cd.textContent = Math.max(1, Math.ceil((state.auto.countdown_until - Date.now()) / 1000))), 250);
  placeChat();
}
