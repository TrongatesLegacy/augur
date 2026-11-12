// Live: the duel that's up, the call and why, the head-to-heads, what's next and the chat line.
import { copyText } from '../lib/clipboard.js';
import { esc, fmt, pct, signed } from '../lib/format.js';
import { SHORT, dial, face, orb, pickVoice, tapeRows, voiceLine, whyLine } from '../lib/pal.js';
import { betCommand, stakeLine } from '../lib/stake.js';
import { columns, empty, icon, ring, signedBar } from '../lib/viz.js';

function upNext(live) {
  const q = live?.queue || [];
  const body = q.length
    ? `<div class="fillin">${q.map((x) => `<div class="row"><span>${esc(x.home)} <span class="small">vs</span> ${esc(x.away)}</span><span class="small">#${x.pos}</span></div>`).join('')}</div>`
    : empty('queue', 'Nothing queued', 'Fights show here as chat lines them up.');
  return `<div class="panel col list"><div class="ph"><h3>Up next</h3><span>pals show at the bell</span></div>${body}</div>`;
}

function side(s, cls, isFav) {
  return `<div class="fx ${cls === 'a' ? 'r' : ''}">${orb(s.pal, s.slug, cls + (isFav ? '' : ' dim'), 86, isFav)}<div><div class="nm" style="${isFav ? '' : 'color:var(--t2)'}">${esc(s.pal)}</div><div class="ow">${face(s.owner, 'a av', 18)}${cls === 'h' ? 'HOME' : 'AWAY'} · ${esc(s.owner)}</div><div class="els">${s.fighter.types.map((t) => `<span class="el">${esc(t)}</span>`).join('')}</div></div></div>`;
}

// one factor's home win chance as a bar from the middle: left for home, right for away
function factor(label, ph) {
  const l = Math.min(ph, 0.5) * 100;
  const r = Math.max(ph, 0.5) * 100;
  const color = ph >= 0.5 ? 'var(--home)' : 'var(--away)';
  return `<div class="tr"><span>${label}</span><div class="track"><span class="fill" style="left:${100 - r}%;right:${l}%;background:${color}"></span></div><span class="pc">${pct(ph)}</span></div>`;
}

export function render(app) {
  const { state, main, header } = app;
  const live = state.live;
  const p = live?.prediction;
  const status = live?.error
    ? `<div class="pill ml"><span class="dot idle"></span>Can't reach PalDuels</div>`
    : p
      ? `<div class="pill ml"><span class="dot"></span>${live.live ? 'Fight on' : 'Picks open'}</div>`
      : `<div class="pill ml"><span class="dot idle"></span>${esc(live?.title || 'Waiting')}</div>`;
  if (!p) {
    const r = live?.last_result;
    const last = r
      ? `Last fight: <span class="${r.right ? 'ok' : 'bad'}">${esc(r.winner_pal)} won ${r.right ? '✓' : '✕'}</span> (${esc(r.home_pal)} vs ${esc(r.away_pal)}${r.ko ? `, K.O. ${esc(r.ko)}` : ''}${r.hp_left != null ? `, ${r.hp_left}% HP left` : ''})`
      : 'Augur checks every few seconds and calls the next fight as soon as it’s up.';
    const o = live?.off_air;
    const between = (live?.queue || []).length > 0;
    const ls = o?.last_stream;
    const day = (d) => new Date(d + 'T12:00:00Z').toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' });
    const all = o?.all_time?.[1] ? o.all_time[0] / o.all_time[1] : null;
    const stream = ls
      ? `<div class="panel col"><div class="ph"><h3>${between ? 'Today' : 'Last stream'}</h3><span>${esc(day(ls[0]))}</span></div>
          <div class="yrow">${ring(ls[1] / ls[2], { size: 84, sub: 'right' })}
            <div class="ycol"><div class="qn big"><b>${ls[1]}<i>/${ls[2]}</i></b></div><span class="small">best run ${ls[3]}${o.all_time[1] ? ` · all time ${o.all_time[0]}/${o.all_time[1]}` : ''}</span></div></div>
          ${o.last_bets ? `<div class="lsb">${signedBar('You', o.last_bets[0], Math.max(Math.abs(o.last_bets[0]), Math.abs(o.last_bets[1]), 1))}${signedBar('Following Augur', o.last_bets[1], Math.max(Math.abs(o.last_bets[0]), Math.abs(o.last_bets[1]), 1))}</div>` : ''}</div>`
      : `<div class="panel col"><div class="ph"><h3>Last stream</h3></div>${empty('chart', 'No streams yet', 'Augur keeps score from the first call.')}</div>`;
    const recent = `<div class="panel col list lcalls"><div class="ph"><h3>Latest calls</h3>${o?.recent?.length ? `<span>${o.recent.filter((r) => r[2]).length} of ${o.recent.length} right</span>` : ''}</div>${
      o?.recent?.length
        ? `<div class="fillin">${o.recent.map(([pick, other, right, prob]) => `<div class="row"><span><span class="res ${right ? 'o' : 'x'}">${right ? '✓' : '✕'}</span> <b>${esc(pick)}</b> <span class="muted">over</span> ${esc(other)}</span><span class="small"><span class="cond lp">${prob ? pct(prob) : ''}</span> <span class="${right ? 'ok' : 'bad'}">${right ? 'Right' : 'Missed'}</span></span></div>`).join('')}</div>`
        : empty('swords', 'No calls yet', 'The first fight Augur calls shows here.')
    }</div>`;
    const known = o
      ? `<div class="known"><span class="label">What Augur knows</span><div>${[
          [o.store[0], 'duels', 'swords'],
          [o.store[1], 'fight tapes', 'chart'],
          [o.store[2], 'species', 'egg'],
          [o.store[3], 'your bets', 'star'],
        ]
          .map(([n, l, k]) => `<div title="from palduels.com">${icon(k, 18)}<b>${fmt(n)}</b><small>${l}</small></div>`)
          .join('')}</div></div>`
      : '';
    const hero = `<div class="panel offair"><div><b>${live?.error ? 'Can’t reach PalDuels' : between ? 'Between duels' : 'Off air'}</b><span>${last}</span>
      <small>Every call stands on stats, types, moves and this history, never on chat.</small></div>${known}</div>`;
    const days = o?.days?.length
      ? `<div class="panel col lstreams"><div class="ph"><h3>Recent streams</h3><span>calls right, one bar per stream${all != null ? ` · dashed: all time ${pct(all)}` : ''}</span></div>${columns(
          o.days.map(([d, r, n]) => ({ v: r / n, label: d.slice(5).replace('-', '/'), top: `${r}/${n}`, title: `${d}: ${r}/${n} right`, tone: r / n >= 0.5 ? '' : 'bad' })),
          all,
        )}</div>`
      : `<div class="panel col lstreams"><div class="ph"><h3>Recent streams</h3></div>${empty('chart', 'No streams yet', 'One bar per stream, once Augur has called some fights.')}</div>`;
    main.innerHTML = header('Live · PalDuels', status) + hero + `<div class="loff grow">${stream}${upNext(live)}${recent}${days}</div>`;
    return;
  }
  const fav = p.pick === 'home' ? p.home : p.away;
  // the projected race: the favourite knocks the other out first; by then it has lost t_fav / t_dog of its HP
  const tFav = p.pick === 'home' ? p.score.t_home : p.score.t_away;
  const tDog = p.pick === 'home' ? p.score.t_away : p.score.t_home;
  const left = Math.max(0, 1 - tFav / tDog);
  const hidden = [p.home, p.away]
    .filter((s) => s.hidden.moves.length)
    .map((s) => `<span class="chip">${esc(s.pal)}: unlisted <b>${s.hidden.moves.slice(0, 2).map((m) => esc(m[0] || 'unnamed')).join(', ')}</b></span>`)
    .join('');
  const owners = p.owners;
  const { voices, voice } = pickVoice(state.voice, state.settings);
  state.voice = voice;
  const command = betCommand(p, live.stake);
  main.innerHTML =
    header(`Live · ${esc(p.title || 'PalDuels')}`, status + (command ? `<button class="btn ghost" id="cmd">Copy ${esc(command)}</button>` : '')) +
    `<div class="stage">${side(p.home, 'h', p.pick === 'home')}
      <div class="center">${dial(p.prob, 100)}<div class="cond" style="font-weight:800;font-size:24px;letter-spacing:.06em;margin-top:4px;text-transform:uppercase">${esc(fav.pal)}</div><div class="small" style="color:var(--t2)">${esc(whyLine(p))}</div>${p.home.estimated || p.away.estimated ? '<div class="small gold">Some stats hidden until the bell: estimated</div>' : ''}${stakeLine(live.stake)}</div>
      ${side(p.away, 'a', p.pick === 'away')}</div>
    <div class="g3">
      <div class="panel"><div class="ph"><h3>Tale of the tape</h3><span>Lv 70</span></div>${tapeRows(p)}
        <div class="small">Passives: ${esc(p.home.fighter.passives.join(', ') || 'none')} vs ${esc(p.away.fighter.passives.join(', ') || 'none')} · ${p.home.usable_moves} vs ${p.away.usable_moves} moves</div></div>
      <div class="panel"><div class="ph"><h3>Projected HP race</h3><span>model estimate</span></div>
        <svg width="100%" height="100" viewBox="0 0 340 100" role="img" aria-label="${esc(fav.pal)} projected to win with about ${pct(left)} HP left">
          <g stroke="rgba(255,255,255,.06)"><line x1="0" y1="6" x2="340" y2="6"/><line x1="0" y1="48" x2="340" y2="48"/><line x1="0" y1="90" x2="340" y2="90"/></g>
          <path d="M0 6 L 220 90" stroke="${p.pick === 'home' ? '#ff5ca8' : '#4fe3ff'}" stroke-width="2.4" fill="none" stroke-dasharray="6 4"/>
          <path d="M0 6 L 220 ${90 - 84 * left}" stroke="${p.pick === 'home' ? '#4fe3ff' : '#ff5ca8'}" stroke-width="2.4" fill="none"/>
          <text x="228" y="${94 - 84 * left}" fill="#b3aec9" font-size="12">${esc(fav.pal)} ~${pct(left)}</text><text x="228" y="94" fill="#b3aec9" font-size="12">K.O.</text></svg>
        <div class="small">Solid: ${esc(fav.pal)} · dashed: the other side</div></div>
      <div class="panel"><div class="ph"><h3>What decides it</h3><span>home ◂ ▸ away</span></div>
        ${factor('Stats', p.score.p_stats)}${factor('Records', p.score.p_species)}${factor('Pal H2H', p.score.p_h2h)}
        <div class="chips">${fav.type_edge_moves.length ? `<span class="chip"><b>${fav.type_edge_moves.length}</b> type-edge moves</span>` : ''}${hidden || '<span class="chip">No unlisted moves</span>'}</div></div>
    </div>
    <div class="g3 grow">
      <div class="panel list"><div class="ph"><h3>Head to head</h3><span>owners · pals</span></div>
        <div class="row"><span>${esc(owners?.summary || 'Owners: not loaded')}</span></div>
        ${(owners?.meetings || [])
          .slice(0, 2)
          .map((m) => `<div class="row"><span class="small">${esc(m.when)} · ${esc(m.sides[0].pal)} vs ${esc(m.sides[1].pal)}</span><span class="small">${esc(m.sides[0].won ? m.sides[0].owner : m.sides[1].owner)}</span></div>`)
          .join('')}
        <div class="row"><span>${esc(p.home.pal)} vs ${esc(p.away.pal)}</span><b class="gold">${p.pal_h2h.home_wins}–${p.pal_h2h.away_wins}</b></div></div>
      ${upNext(live)}
      ${
        state.settings?.chat_lines === false
          ? ''
          : `<div class="panel"><div class="ph"><h3>Say it in chat</h3><span>${voices.length} voices</span></div>
        <div class="vo" id="vb">${voices.map((v) => `<button class="${v === voice ? 'on' : ''}" data-v="${esc(v)}">${esc(SHORT[v] || v)}</button>`).join('')}</div>
        <div class="q" id="vq">${esc(voiceLine(p, voice, state.settings?.include_chance !== false))}</div><button class="btn" id="cp" style="width:100%;margin-top:4px">Copy for chat</button></div>`
      }
    </div>`;
  main.querySelectorAll('#vb button').forEach((b) => {
    b.onclick = () => {
      state.voice = b.dataset.v;
      render(app);
    };
  });
  document.getElementById('cp')?.addEventListener('click', (e) => copyText(document.getElementById('vq').textContent, e.target));
  document.getElementById('cmd')?.addEventListener('click', (e) => copyText(command, e.target));
}
