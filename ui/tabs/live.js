// Live: the duel that's up, the call and why, the head-to-heads, what's next and the chat line.
import { invoke } from '../lib/bridge.js';
import { copyText } from '../lib/clipboard.js';
import { esc, pct } from '../lib/format.js';
import { SHORT, dial, orb, pickVoice, tapeRows, voiceLine, whyLine } from '../lib/pal.js';
import { betCommand, stakeLine } from '../lib/stake.js';

// a fresh install: bring a backup across from another computer, or start fresh (the background fetch waits)
function welcome(live) {
  if (!live?.first_run) return '';
  return `<div class="panel welcome"><div class="wt"><b>Setting up on a new computer?</b><small>Import a backup from your other computer to bring your calls, bets and every past duel across, and skip the hour-long first fetch. Live calls work either way.</small></div>
    <div class="wb"><button class="btn" id="wi">Import a backup…</button><button class="btn ghost" id="wf">Start fresh</button></div></div>`;
}

function wireWelcome(app) {
  document.getElementById('wf')?.addEventListener('click', async () => {
    await invoke('start_fresh');
    app.state.live = await invoke('live');
    render(app);
  });
  document.getElementById('wi')?.addEventListener('click', async (e) => {
    e.target.disabled = true;
    try {
      const r = await invoke('import_file');
      if (r) {
        const m = r.merged;
        app.state.live = await invoke('live');
        render(app);
        app.flash(`Imported ${r.file}: ${m.calls} calls, ${m.duels} duels, ${m.bets} bets. Only newer duels are left to fetch.`, true);
        return;
      }
    } catch (err) {
      app.flash(err);
    }
    e.target.disabled = false;
  });
}

function upNext(live, inline) {
  const q = live?.queue || [];
  const body = q.length
    ? q.map((x) => `<div class="row"><span>${esc(x.home)} <span class="small">vs</span> ${esc(x.away)}</span><span class="small">#${x.pos}</span></div>`).join('')
    : '<div class="small">Nothing queued.</div>';
  return `<div class="panel list" ${inline ? '' : 'style="max-width:520px"'}><div class="ph"><h3>Up next</h3><span>pals show at the bell</span></div>${body}</div>`;
}

function side(s, cls, isFav) {
  return `<div class="fx ${cls === 'a' ? 'r' : ''}">${orb(s.pal, s.slug, cls + (isFav ? '' : ' dim'), 86, isFav)}<div><div class="nm" style="${isFav ? '' : 'color:var(--t2)'}">${esc(s.pal)}</div><div class="ow">${cls === 'h' ? 'HOME' : 'AWAY'} · ${esc(s.owner)}</div><div class="els">${s.fighter.types.map((t) => `<span class="el">${esc(t)}</span>`).join('')}</div></div></div>`;
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
    main.innerHTML = header('Live · PalDuels', status) + welcome(live) + `<div class="panel empty"><b>No duel up right now</b>${last}</div>` + upNext(live);
    wireWelcome(app);
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
    welcome(live) +
    `<div class="stage">${side(p.home, 'h', p.pick === 'home')}
      <div class="center">${dial(p.prob, 112)}<div class="cond" style="font-weight:800;font-size:24px;letter-spacing:.06em;margin-top:4px;text-transform:uppercase">${esc(fav.pal)}</div><div class="small" style="color:var(--t2)">${esc(whyLine(p))}</div>${p.home.estimated || p.away.estimated ? '<div class="small gold">Some stats hidden until the bell: estimated</div>' : ''}${stakeLine(live.stake)}</div>
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
    <div class="g3">
      <div class="panel list"><div class="ph"><h3>Head to head</h3><span>owners · pals</span></div>
        <div class="row"><span>${esc(owners?.summary || 'Owners: not loaded')}</span></div>
        ${(owners?.meetings || [])
          .slice(0, 2)
          .map((m) => `<div class="row"><span class="small">${esc(m.when)} · ${esc(m.sides[0].pal)} vs ${esc(m.sides[1].pal)}</span><span class="small">${esc(m.sides[0].won ? m.sides[0].owner : m.sides[1].owner)}</span></div>`)
          .join('')}
        <div class="row"><span>${esc(p.home.pal)} vs ${esc(p.away.pal)}</span><b class="gold">${p.pal_h2h.home_wins}–${p.pal_h2h.away_wins}</b></div></div>
      ${upNext(live, true)}
      <div class="panel"><div class="ph"><h3>Say it in chat</h3><span>${voices.length} voices</span></div>
        <div class="vo" id="vb">${voices.map((v) => `<button class="${v === voice ? 'on' : ''}" data-v="${esc(v)}">${esc(SHORT[v] || v)}</button>`).join('')}</div>
        <div class="q" id="vq">${esc(voiceLine(p, voice, state.settings?.include_chance !== false))}</div><button class="btn" id="cp" style="width:100%;margin-top:4px">Copy for chat</button></div>
    </div>`;
  main.querySelectorAll('#vb button').forEach((b) => {
    b.onclick = () => {
      state.voice = b.dataset.v;
      render(app);
    };
  });
  document.getElementById('cp').onclick = (e) => copyText(document.getElementById('vq').textContent, e.target);
  document.getElementById('cmd')?.addEventListener('click', (e) => copyText(command, e.target));
  wireWelcome(app);
}
