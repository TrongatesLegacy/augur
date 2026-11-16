//! Auto betting: when to bet on a call by itself, the one-bet-per-fight record that makes sure it never bets twice,
//! how it knows the bet landed (Kick's chat), and which of your bets it placed.
//!
//! The rules that keep it safe:
//! - a fight is claimed (a row in `auto_bets`, one per call) before anything is sent, so a crash, a restart or a
//!   second look at the same fight can never send again;
//! - nothing is ever retried: a send that can't be confirmed is marked unconfirmed, not sent again;
//! - it stands down when you've bet on the fight yourself (your own bet command in chat, or in your picks list).

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::kick::{self, Line};
use crate::site::Pick;

/// Settings › Auto bet. Kept on each computer (not synced): only one computer should bet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoBetSettings {
    /// shows the stream's chat on Live and lets auto betting be armed (arming lasts until Augur quits)
    pub enabled: bool,
    /// the stream's Kick channel
    pub channel: String,
    /// type the bets into your own channel's chat instead: everything runs but no bet is placed
    pub test_mode: bool,
    /// sit out calls under this chance
    pub min_chance: f64,
    pub max_bets_per_day: u32,
    pub max_credits_per_day: u64,
    /// sit out a call made while some of the stats were still hidden
    pub skip_hidden_stats: bool,
    /// the call must stay the same this long before betting (it can change while picks are open)
    pub hold_secs: u64,
    /// the countdown on Live, with Cancel, before the bet goes in
    pub countdown_secs: u64,
    /// never bet later than this after picks opened, so a slow call doesn't miss the lock
    pub latest_secs: u64,
    /// arm it each time Augur starts (once it can: signed in, a channel set, no other computer betting), instead of
    /// on Live or from the tray each session
    pub arm_at_start: bool,
}

impl Default for AutoBetSettings {
    fn default() -> Self {
        AutoBetSettings {
            enabled: false,
            channel: String::new(),
            test_mode: true,
            min_chance: 0.6,
            max_bets_per_day: 40,
            max_credits_per_day: 100_000,
            skip_hidden_stats: true,
            hold_secs: 10,
            countdown_secs: 5,
            // picks stay open about five minutes; three leaves room for a slow call and still beats the lock
            latest_secs: 180,
            arm_at_start: false,
        }
    }
}

/// What auto betting does about the call that's up, right now.
#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    /// not yet, or not this call as it stands (it may still change while picks are open): why
    Wait(String),
    /// never on this fight: why (recorded, so it isn't looked at again)
    Skip(String),
    /// start the countdown
    Go,
}

/// The call that's up, as auto betting sees it.
#[derive(Clone, Debug, Default)]
pub struct Moment {
    pub picks_open: bool,
    pub prob: f64,
    /// the stake Augur suggests (0: sit it out)
    pub amount: u64,
    /// some stats were hidden when the call was made
    pub estimated: bool,
    /// how long the call has stayed the same, and how long picks have been open (ms)
    pub held_ms: u64,
    pub open_ms: u64,
    /// today's auto bets so far: how many, and how many credits
    pub today: (u32, u64),
    pub signed_in: bool,
    /// you've bet on this fight yourself
    pub already_bet: bool,
}

pub fn decide(s: &AutoBetSettings, m: &Moment) -> Decision {
    if m.already_bet {
        return Decision::Skip("you bet on it yourself".into());
    }
    if !m.signed_in {
        return Decision::Skip("signed out of Kick".into());
    }
    if m.open_ms > s.latest_secs * 1000 {
        return Decision::Skip(format!("too late: picks had been open over {}s", s.latest_secs));
    }
    if m.today.0 >= s.max_bets_per_day || m.today.1 + m.amount > s.max_credits_per_day {
        return Decision::Skip("today's auto bet limit".into());
    }
    // these can change while picks are open, so they wait rather than skip
    if m.amount == 0 {
        return Decision::Wait("Augur says sit it out".into());
    }
    if m.prob < s.min_chance {
        return Decision::Wait(format!("{:.0}% is under your {:.0}% minimum", m.prob * 100.0, s.min_chance * 100.0));
    }
    if m.estimated && s.skip_hidden_stats {
        return Decision::Wait("some stats are still hidden".into());
    }
    if m.held_ms < s.hold_secs * 1000 {
        return Decision::Wait("letting the call settle".into());
    }
    Decision::Go
}

// --- the record ---

/// An auto bet: claimed, then sent, then (with luck) confirmed. Or cancelled or skipped, never sent.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AutoBet {
    pub id: i64,
    pub prediction_id: Option<i64>,
    /// a row from another computer: which, and its id there
    pub device: Option<String>,
    pub remote_id: Option<i64>,
    pub home_owner: String,
    pub away_owner: String,
    pub home_pal: String,
    pub away_pal: String,
    pub side: String,
    pub amount: u64,
    pub command: String,
    pub prob: f64,
    pub channel: String,
    pub test: bool,
    /// "sending" (claimed, going in now), "sent" (typed in), "seen" (in the chat), "confirmed" (the bot took it),
    /// "unconfirmed" (typed in but never seen), "cancelled", "skipped" or "failed"
    pub status: String,
    pub reason: Option<String>,
    pub claimed_at: u64,
    pub sent_at: Option<u64>,
    pub done_at: Option<u64>,
    /// the chat line it went in as, and the bot's answer
    pub line_id: Option<String>,
    pub reply: Option<String>,
    pub duel_id: Option<u64>,
}

impl AutoBet {
    /// It may have gone into chat: typed in, or claimed and going in when Augur stopped.
    pub fn went_out(&self) -> bool {
        matches!(self.status.as_str(), "sending" | "sent" | "seen" | "confirmed" | "unconfirmed")
    }
}

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS auto_bets (   -- Augur's own bets, one per fight at most
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    prediction_id INTEGER UNIQUE,        -- the call it's for: the claim that makes a second bet impossible
    device TEXT, remote_id INTEGER,      -- synced in from another computer: which, and its id there
    home_owner TEXT, away_owner TEXT, home_pal TEXT, away_pal TEXT,
    side TEXT, amount INTEGER, command TEXT, prob REAL,
    channel TEXT, test INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL, reason TEXT,
    claimed_at INTEGER, sent_at INTEGER, done_at INTEGER,
    line_id TEXT, reply TEXT,
    duel_id INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS auto_bets_remote ON auto_bets (device, remote_id);
"#;

/// Claims the fight for an auto bet, with `status` ("sending", or "cancelled"/"skipped" to rule it out). None when the
/// fight was already claimed: then nothing may be sent.
pub fn claim(conn: &Connection, b: &AutoBet) -> Result<Option<i64>> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO auto_bets (prediction_id, home_owner, away_owner, home_pal, away_pal, side, amount, command, prob,
                                         channel, test, status, reason, claimed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            b.prediction_id,
            b.home_owner,
            b.away_owner,
            b.home_pal,
            b.away_pal,
            b.side,
            b.amount as i64,
            b.command,
            b.prob,
            b.channel,
            b.test,
            b.status,
            b.reason,
            b.claimed_at as i64
        ],
    )?;
    Ok((n == 1).then(|| conn.last_insert_rowid()))
}

/// Moves an auto bet on. `sent_at` is kept once set.
pub fn update(conn: &Connection, id: i64, status: &str, reason: Option<&str>, now: u64) -> Result<()> {
    let sent = matches!(status, "sent").then_some(now as i64);
    let done = matches!(status, "confirmed" | "unconfirmed" | "failed" | "cancelled" | "skipped").then_some(now as i64);
    conn.execute(
        "UPDATE auto_bets SET status = ?2, reason = COALESCE(?3, reason), sent_at = COALESCE(sent_at, ?4), done_at = COALESCE(?5, done_at)
         WHERE id = ?1",
        params![id, status, reason, sent, done],
    )?;
    Ok(())
}

/// The chat line the bet went in as, and the bot's answer when it comes.
pub fn set_line(conn: &Connection, id: i64, line_id: &str, reply: Option<&str>) -> Result<()> {
    conn.execute("UPDATE auto_bets SET line_id = ?2, reply = COALESCE(?3, reply) WHERE id = ?1", params![id, line_id, reply])?;
    Ok(())
}

const COLS: &str = "a.id, a.prediction_id, a.device, a.remote_id, a.home_owner, a.away_owner, a.home_pal, a.away_pal, a.side,
                    a.amount, a.command, a.prob, a.channel, a.test, a.status, a.reason, a.claimed_at, a.sent_at, a.done_at,
                    a.line_id, a.reply, COALESCE(a.duel_id, p.duel_id)";

fn row(r: &rusqlite::Row) -> rusqlite::Result<AutoBet> {
    let u = |v: Option<i64>| v.map(|v| v.max(0) as u64);
    Ok(AutoBet {
        id: r.get(0)?,
        prediction_id: r.get(1)?,
        device: r.get(2)?,
        remote_id: r.get(3)?,
        home_owner: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
        away_owner: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
        home_pal: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
        away_pal: r.get::<_, Option<String>>(7)?.unwrap_or_default(),
        side: r.get::<_, Option<String>>(8)?.unwrap_or_default(),
        amount: u(r.get(9)?).unwrap_or(0),
        command: r.get::<_, Option<String>>(10)?.unwrap_or_default(),
        prob: r.get::<_, Option<f64>>(11)?.unwrap_or(0.0),
        channel: r.get::<_, Option<String>>(12)?.unwrap_or_default(),
        test: r.get(13)?,
        status: r.get(14)?,
        reason: r.get(15)?,
        claimed_at: u(r.get(16)?).unwrap_or(0),
        sent_at: u(r.get(17)?),
        done_at: u(r.get(18)?),
        line_id: r.get(19)?,
        reply: r.get(20)?,
        duel_id: u(r.get(21)?),
    })
}

/// Every auto bet, newest first, with its duel once its call has settled.
pub fn all(conn: &Connection) -> Result<Vec<AutoBet>> {
    let mut st = conn.prepare(&format!(
        "SELECT {COLS} FROM auto_bets a LEFT JOIN predictions p ON p.id = a.prediction_id ORDER BY a.claimed_at DESC, a.id DESC"
    ))?;
    let rows = st.query_map([], row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// The auto bet on a call, if there is one.
pub fn for_call(conn: &Connection, prediction_id: i64) -> Result<Option<AutoBet>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM auto_bets a LEFT JOIN predictions p ON p.id = a.prediction_id WHERE a.prediction_id = ?1"),
            [prediction_id],
            row,
        )
        .optional()?)
}

/// Real auto bets that went out since `since` (ms): how many, and their credits. Test bets don't count.
pub fn spent_since(conn: &Connection, since: u64) -> Result<(u32, u64)> {
    let all = all(conn)?;
    let mine = all.iter().filter(|b| b.device.is_none() && !b.test && b.went_out() && b.claimed_at >= since);
    Ok(mine.fold((0, 0), |(n, c), b| (n + 1, c + b.amount)))
}

// --- what the chat says ---

/// Your own bet command in the chat since `since` (ms), other than the auto bet's own line.
pub fn own_bet<'a>(lines: &'a [Line], user: &str, since: u64, not: Option<&str>) -> Option<&'a Line> {
    let user = user.trim().to_lowercase();
    lines.iter().find(|l| l.user == user && l.at + SLACK_MS >= since && kick::bet_side(&l.content).is_some() && Some(l.id.as_str()) != not)
}

/// The line the auto bet went in as: yours, with its text, from just before it was sent (Kick's times are to the
/// second, and the two clocks differ a little).
pub fn sent_line<'a>(lines: &'a [Line], user: &str, command: &str, sent_at: u64) -> Option<&'a Line> {
    let user = user.trim().to_lowercase();
    lines.iter().find(|l| l.user == user && l.at + SLACK_MS >= sent_at && l.content.trim().eq_ignore_ascii_case(command.trim()))
}

/// The betting bot's answer to a line: a reply to it from someone else that takes the bet ("2300 on HOME ...").
pub fn bot_reply<'a>(lines: &'a [Line], line: &Line) -> Option<&'a Line> {
    lines.iter().find(|l| {
        l.reply_to.as_deref() == Some(line.id.as_str()) && l.user != line.user && {
            let c = l.content.to_uppercase();
            c.contains(" ON HOME") || c.contains(" ON AWAY")
        }
    })
}

/// The betting bot taking the bet, found by its reply alone: a reply to your line with the command's text, after
/// it was sent. The reply keeps what it answers, so this works even when your own line has gone (the stream's AI
/// moderator deletes some bet commands after the bot has taken them, or the chat moved on).
pub fn bot_took<'a>(lines: &'a [Line], user: &str, command: &str, sent_at: u64) -> Option<&'a Line> {
    let user = user.trim().to_lowercase();
    lines.iter().find(|l| {
        l.at + SLACK_MS >= sent_at
            && l.reply_to_user.as_deref() == Some(user.as_str())
            && l.reply_to_content.as_deref().is_some_and(|c| c.trim().eq_ignore_ascii_case(command.trim()))
            && {
                let c = l.content.to_uppercase();
                c.contains(" ON HOME") || c.contains(" ON AWAY")
            }
    })
}

/// Auto bets that went out unconfirmed but are in your picks list now (a bet of yours matched to them, as History's
/// Auto label does): confirmed. Returns how many.
pub fn confirm_from_picks(conn: &Connection, bets: &[Pick], bet_seen_at: &HashMap<u64, u64>, now: u64) -> Result<usize> {
    let autos = all(conn)?;
    let via = via(bets, &autos, bet_seen_at);
    let mut n = 0;
    for a in autos.iter().filter(|a| a.device.is_none() && !a.test && matches!(a.status.as_str(), "unconfirmed" | "sent" | "seen")) {
        let theirs = |b: &&Pick| b.home_owner == a.home_owner && b.away_owner == a.away_owner && b.backed == a.side;
        if bets.iter().filter(theirs).any(|b| via.get(&b.seq).is_some_and(|v| v.via == "auto")) {
            update(conn, a.id, "confirmed", Some("in your picks list"), now)?;
            n += 1;
        }
    }
    Ok(n)
}

/// Kick's chat times are to the second, and its clock and this computer's differ a little.
const SLACK_MS: u64 = 5_000;

// --- sync: each computer's auto bets, so every computer can tell its bets apart ---

/// Writes this computer's own auto bets (not those synced in) to `path`, one JSON line each. Returns a mark of what was
/// written: the file only needs writing again when it changes.
pub fn export(conn: &Connection, path: &std::path::Path) -> Result<String> {
    let mine: Vec<AutoBet> = all(conn)?.into_iter().filter(|b| b.device.is_none()).collect();
    let mut out = String::new();
    for b in &mine {
        out.push_str(&serde_json::to_string(b)?);
        out.push('\n');
    }
    std::fs::write(path, out)?;
    Ok(mark(&mine))
}

/// What this computer's auto bets are now, to tell whether its file needs writing again.
pub fn mark_now(conn: &Connection) -> Result<String> {
    Ok(mark(&all(conn)?.into_iter().filter(|b| b.device.is_none()).collect::<Vec<_>>()))
}

fn mark(bets: &[AutoBet]) -> String {
    bets.iter().map(|b| format!("{}:{}:{}", b.id, b.status, b.duel_id.unwrap_or(0))).collect::<Vec<_>>().join(",")
}

/// Brings in another computer's auto bets from its file, updating the ones already here. Returns how many changed.
pub fn import(conn: &Connection, path: &std::path::Path, device: &str) -> Result<usize> {
    let text = std::fs::read_to_string(path)?;
    let mut changed = 0;
    for raw in text.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(b) = serde_json::from_str::<AutoBet>(raw) else { continue };
        changed += conn.execute(
            "INSERT INTO auto_bets (device, remote_id, home_owner, away_owner, home_pal, away_pal, side, amount, command, prob, channel,
                                    test, status, reason, claimed_at, sent_at, done_at, line_id, reply, duel_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
             ON CONFLICT (device, remote_id) DO UPDATE SET status = excluded.status, reason = excluded.reason,
                 sent_at = excluded.sent_at, done_at = excluded.done_at, line_id = excluded.line_id, reply = excluded.reply,
                 duel_id = excluded.duel_id
             WHERE status IS NOT excluded.status OR duel_id IS NOT excluded.duel_id OR reply IS NOT excluded.reply",
            params![
                device,
                b.id,
                b.home_owner,
                b.away_owner,
                b.home_pal,
                b.away_pal,
                b.side,
                b.amount as i64,
                b.command,
                b.prob,
                b.channel,
                b.test,
                b.status,
                b.reason,
                b.claimed_at as i64,
                b.sent_at.map(|v| v as i64),
                b.done_at.map(|v| v as i64),
                b.line_id,
                b.reply,
                b.duel_id.map(|v| v as i64)
            ],
        )?;
    }
    Ok(changed)
}

// --- which of your bets Augur placed ---

/// How one of your bets was placed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Via {
    /// "auto" or "manual"
    pub via: String,
    /// the auto bet's status, for an auto bet
    pub status: Option<String>,
    /// you and Augur both bet on this fight
    pub double: bool,
}

/// Your bets (from the picks list) that auto bets placed, by their place in the picks list. A bet counts as Augur's
/// when it's on the same fight and side as an auto bet that went out (same duel, or the same owners and pals read
/// after it was claimed), preferring one with the same stake. Every other bet on a fight Augur bet on is yours, and
/// both are marked as a double bet.
pub fn via(bets: &[Pick], autos: &[AutoBet], bet_seen_at: &HashMap<u64, u64>) -> HashMap<u64, Via> {
    let mut out: HashMap<u64, Via> = bets.iter().map(|b| (b.seq, Via { via: "manual".into(), status: None, double: false })).collect();
    for a in autos.iter().filter(|a| a.went_out() && !a.test) {
        let same_fight = |b: &&Pick| match (b.duel_id, a.duel_id) {
            (Some(x), Some(y)) => x == y,
            _ => {
                b.home_owner == a.home_owner
                    && b.away_owner == a.away_owner
                    && b.home_pal == a.home_pal
                    && b.away_pal == a.away_pal
                    && bet_seen_at.get(&b.seq).is_some_and(|&t| t + 60_000 >= a.claimed_at && t < a.claimed_at + 12 * 3_600_000)
            }
        };
        let fight: Vec<&Pick> = bets.iter().filter(same_fight).collect();
        let free = |b: &&&Pick| b.backed == a.side && out.get(&b.seq).is_some_and(|v| v.via == "manual");
        let mine = fight.iter().filter(free).find(|b| b.stake == a.amount).or_else(|| fight.iter().find(free));
        let Some(mine) = mine else { continue };
        let double = fight.len() > 1;
        out.insert(mine.seq, Via { via: "auto".into(), status: Some(a.status.clone()), double });
        for b in fight.iter().filter(|b| b.seq != mine.seq) {
            out.insert(b.seq, Via { via: "manual".into(), status: None, double: true });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moment() -> Moment {
        Moment { picks_open: true, prob: 0.72, amount: 2300, held_ms: 12_000, open_ms: 20_000, signed_in: true, ..Default::default() }
    }

    #[test]
    fn decides_when_to_bet() {
        let s = AutoBetSettings::default();
        assert_eq!(decide(&s, &moment()), Decision::Go);
        let wait = |m: Moment| matches!(decide(&s, &m), Decision::Wait(_));
        let skip = |m: Moment| matches!(decide(&s, &m), Decision::Skip(_));
        // the call can still change while picks are open: these wait
        assert!(wait(Moment { held_ms: 3_000, ..moment() }), "the call hasn't settled");
        assert!(wait(Moment { amount: 0, ..moment() }), "Augur says sit it out");
        assert!(wait(Moment { prob: 0.55, ..moment() }), "under the minimum");
        assert!(wait(Moment { estimated: true, ..moment() }), "stats still hidden");
        // these rule the fight out
        assert!(skip(Moment { already_bet: true, ..moment() }), "you bet yourself");
        assert!(skip(Moment { signed_in: false, ..moment() }));
        assert!(skip(Moment { open_ms: 181_000, ..moment() }), "too late");
        assert!(skip(Moment { today: (40, 0), ..moment() }), "bets a day");
        assert!(skip(Moment { today: (3, 98_000), ..moment() }), "credits a day");
        assert_eq!(
            decide(&AutoBetSettings { skip_hidden_stats: false, ..s.clone() }, &Moment { estimated: true, ..moment() }),
            Decision::Go
        );
    }

    fn bet(prediction_id: i64) -> AutoBet {
        AutoBet {
            prediction_id: Some(prediction_id),
            home_owner: "player-01".into(),
            away_owner: "player-02".into(),
            home_pal: "Gumoss".into(),
            away_pal: "Mozzarina".into(),
            side: "home".into(),
            amount: 2300,
            command: "!h 2300".into(),
            prob: 0.72,
            channel: "the-stream".into(),
            status: "sending".into(),
            claimed_at: 1_000,
            ..Default::default()
        }
    }

    /// A fight is claimed once: the second claim, however it comes (a restart, a second look, a cancel after the
    /// bet), gets nothing, so nothing more can be sent.
    #[test]
    fn a_fight_is_claimed_once() {
        let conn = crate::db::open_memory().unwrap();
        let id = claim(&conn, &bet(7)).unwrap().expect("the first claim");
        assert_eq!(claim(&conn, &bet(7)).unwrap(), None);
        assert_eq!(claim(&conn, &AutoBet { status: "cancelled".into(), ..bet(7) }).unwrap(), None);
        update(&conn, id, "sent", None, 2_000).unwrap();
        update(&conn, id, "confirmed", Some("the bot took it"), 3_000).unwrap();
        let b = for_call(&conn, 7).unwrap().unwrap();
        assert_eq!(
            (b.status.as_str(), b.sent_at, b.done_at, b.reason.as_deref()),
            ("confirmed", Some(2_000), Some(3_000), Some("the bot took it"))
        );
        // a fight ruled out is claimed too
        assert!(claim(&conn, &AutoBet { status: "skipped".into(), ..bet(8) }).unwrap().is_some());
        assert_eq!(claim(&conn, &bet(8)).unwrap(), None);
    }

    #[test]
    fn counts_todays_real_bets() {
        let conn = crate::db::open_memory().unwrap();
        claim(&conn, &bet(1)).unwrap();
        claim(&conn, &AutoBet { claimed_at: 5_000, ..bet(2) }).unwrap();
        claim(&conn, &AutoBet { test: true, ..bet(3) }).unwrap();
        claim(&conn, &AutoBet { status: "cancelled".into(), ..bet(4) }).unwrap();
        assert_eq!(spent_since(&conn, 0).unwrap(), (2, 4600), "a claimed bet counts even before it's confirmed");
        assert_eq!(spent_since(&conn, 2_000).unwrap(), (1, 2300));
    }

    #[test]
    fn finds_the_bet_in_chat_and_the_bots_answer() {
        let lines =
            kick::parse_history(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/pages/kick-history.json")).unwrap())
                .unwrap();
        let at = lines.iter().find(|l| l.content == "!h 2300").unwrap().at;
        let line = sent_line(&lines, "Player-01", "!H 2300", at + 2_000).expect("sent a moment before Augur thought");
        assert_eq!(bot_reply(&lines, line).map(|l| &l.content[..12]), Some("2300 on HOME"));
        assert!(sent_line(&lines, "player-01", "!h 2300", at + 10_000).is_none(), "an older line isn't this bet");
        assert!(sent_line(&lines, "player-01", "!a 2300", at).is_none());
        // your own bet, before Augur's
        assert_eq!(own_bet(&lines, "player-02", at - 30_000, None).map(|l| l.content.as_str()), Some("!a"));
        assert!(own_bet(&lines, "player-01", at - 30_000, Some(&line.id)).is_none(), "Augur's own line isn't yours");
        assert!(own_bet(&lines, "player-03", 0, None).is_none(), "gg isn't a bet");
    }

    /// The other computer gets this one's auto bets, and their later changes, without doubling them.
    #[test]
    fn auto_bets_travel_to_the_other_computer() {
        let (mac, pc) = (crate::db::open_memory().unwrap(), crate::db::open_memory().unwrap());
        let id = claim(&mac, &bet(1)).unwrap().unwrap();
        let file = std::env::temp_dir().join(format!("augur-autobets-{}.jsonl", std::process::id()));
        let first = export(&mac, &file).unwrap();
        assert_eq!(import(&pc, &file, "mac").unwrap(), 1);
        assert_eq!(import(&pc, &file, "mac").unwrap(), 0, "nothing new");
        update(&mac, id, "confirmed", None, 5_000).unwrap();
        assert_ne!(mark_now(&mac).unwrap(), first, "a change needs the file written again");
        export(&mac, &file).unwrap();
        assert_eq!(import(&pc, &file, "mac").unwrap(), 1);
        let theirs = all(&pc).unwrap();
        assert_eq!(theirs.len(), 1);
        assert_eq!((theirs[0].status.as_str(), theirs[0].device.as_deref(), theirs[0].prediction_id), ("confirmed", Some("mac"), None));
        // the PC writes only its own: none
        export(&pc, &file).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
        assert_eq!(spent_since(&pc, 0).unwrap(), (0, 0), "another computer's bets don't count against this one's limits");
        let _ = std::fs::remove_file(file);
    }

    /// The bot's reply alone confirms the bet: your line can be gone (the AI moderator deletes some bet commands
    /// after the bot has taken them).
    #[test]
    fn the_bots_reply_confirms_it_even_without_your_line() {
        let mut lines =
            kick::parse_history(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/pages/kick-history.json")).unwrap())
                .unwrap();
        let at = lines.iter().find(|l| l.content == "!h 2300").unwrap().at;
        lines.retain(|l| l.content != "!h 2300");
        assert!(sent_line(&lines, "player-01", "!h 2300", at).is_none(), "the line itself is gone");
        assert!(bot_took(&lines, "Player-01", "!H 2300", at - 1_000).is_some_and(|r| r.content.starts_with("2300 on HOME")));
        assert!(bot_took(&lines, "player-02", "!h 2300", at).is_none(), "someone else's bet isn't yours");
        assert!(bot_took(&lines, "player-01", "!h 2300", at + 60_000).is_none(), "an older reply isn't this bet");
    }

    /// An auto bet the chat couldn't confirm is confirmed once your picks list shows it.
    #[test]
    fn the_picks_list_confirms_an_unconfirmed_bet() {
        let conn = crate::db::open_memory().unwrap();
        let id = claim(&conn, &AutoBet { claimed_at: 1_000_000, ..bet(1) }).unwrap().unwrap();
        update(&conn, id, "unconfirmed", Some("not seen in the chat"), 1_030_000).unwrap();
        let seen = HashMap::from([(9, 1_200_000)]);
        assert_eq!(confirm_from_picks(&conn, &[pick(9, None, "away", 2300)], &seen, 2_000_000).unwrap(), 0, "the other side isn't it");
        assert_eq!(confirm_from_picks(&conn, &[pick(9, None, "home", 2300)], &seen, 2_000_000).unwrap(), 1);
        let b = for_call(&conn, 1).unwrap().unwrap();
        assert_eq!((b.status.as_str(), b.reason.as_deref()), ("confirmed", Some("in your picks list")));
    }

    fn pick(seq: u64, duel: Option<u64>, side: &str, stake: u64) -> Pick {
        Pick {
            seq,
            duel_id: duel,
            home_owner: "player-01".into(),
            away_owner: "player-02".into(),
            home_pal: "Gumoss".into(),
            away_pal: "Mozzarina".into(),
            backed: side.into(),
            stake,
            ..Default::default()
        }
    }

    #[test]
    fn tells_augurs_bets_from_yours() {
        let auto = AutoBet { id: 1, status: "confirmed".into(), duel_id: Some(50), claimed_at: 1_000_000, ..bet(1) };
        let seen: HashMap<u64, u64> = HashMap::new();
        // one bet on the fight, Augur's
        let v = via(&[pick(1, Some(50), "home", 2300), pick(2, Some(49), "home", 500)], std::slice::from_ref(&auto), &seen);
        assert_eq!((v[&1].via.as_str(), v[&1].double, v[&2].via.as_str()), ("auto", false, "manual"));
        assert_eq!(v[&1].status.as_deref(), Some("confirmed"));
        // you bet on the same fight too: the one with Augur's stake is Augur's, and both are a double bet
        let v = via(&[pick(3, Some(50), "home", 500), pick(4, Some(50), "home", 2300)], std::slice::from_ref(&auto), &seen);
        assert_eq!((v[&4].via.as_str(), v[&4].double, v[&3].via.as_str(), v[&3].double), ("auto", true, "manual", true));
        // an older bet with no duel link: matched by the fight, read after the claim
        let seen = HashMap::from([(5, 1_010_000), (6, 100_000)]);
        let unlinked = AutoBet { duel_id: None, ..auto.clone() };
        let v = via(&[pick(5, None, "home", 2300)], std::slice::from_ref(&unlinked), &seen);
        assert_eq!(v[&5].via, "auto");
        assert_eq!(via(&[pick(6, None, "home", 2300)], &[unlinked], &seen)[&6].via, "manual", "read long before Augur bet");
        // a test bet, or one that never went out, places nothing
        for a in [AutoBet { test: true, ..auto.clone() }, AutoBet { status: "cancelled".into(), ..auto }] {
            assert_eq!(via(&[pick(7, Some(50), "home", 2300)], &[a], &HashMap::new())[&7].via, "manual");
        }
    }
}
