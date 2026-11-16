//! Kick, read only: a channel's chat settings and its latest chat lines, for auto betting. Sending isn't here: the app
//! types into the channel's chat on screen, signed in to Kick, so nothing here ever needs a login.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::site;

/// A Kick channel as auto betting needs it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Channel {
    pub slug: String,
    /// the channel's id, which its chat history is under
    pub id: u64,
    pub chatroom_id: u64,
    /// seconds between messages in slow mode (None: slow mode off)
    pub slow_secs: Option<u64>,
    pub followers_only: bool,
    pub subscribers_only: bool,
    pub live: bool,
}

/// One chat line.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub id: String,
    /// when it was sent (ms)
    pub at: u64,
    /// the sender's name, lowercase
    pub user: String,
    pub content: String,
    /// for a reply: the id of the line it answers, and who sent that line (lowercase)
    pub reply_to: Option<String>,
    pub reply_to_user: Option<String>,
}

pub fn channel_url(slug: &str) -> String {
    format!("https://kick.com/api/v2/channels/{}", slug.trim().to_lowercase())
}

pub fn history_url(channel_id: u64) -> String {
    format!("https://web.kick.com/api/v1/chat/{channel_id}/history")
}

/// The chat on its own, as the stream's popout shows it.
pub fn popout_url(slug: &str) -> String {
    format!("https://kick.com/popout/{}/chat", slug.trim().to_lowercase())
}

pub fn channel(slug: &str) -> Result<Channel> {
    parse_channel(&site::fetch_once(&channel_url(slug), None)?)
}

/// The latest chat lines (Kick keeps about the last 25), oldest first. One GET, no retries.
pub fn history(channel_id: u64) -> Result<Vec<Line>> {
    parse_history(&site::fetch_once(&history_url(channel_id), None)?)
}

pub fn parse_channel(body: &str) -> Result<Channel> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| Error::Parse(format!("the Kick channel: {e}")))?;
    let room = &v["chatroom"];
    let (Some(id), Some(chatroom_id)) = (v["id"].as_u64(), room["id"].as_u64()) else {
        return Err(Error::Parse("the Kick channel: no channel or chatroom id (is the name right?)".into()));
    };
    Ok(Channel {
        slug: v["slug"].as_str().unwrap_or_default().to_string(),
        id,
        chatroom_id,
        slow_secs: room["slow_mode"].as_bool().unwrap_or(false).then(|| room["message_interval"].as_u64().unwrap_or(0)),
        followers_only: room["followers_mode"].as_bool().unwrap_or(false),
        subscribers_only: room["subscribers_mode"].as_bool().unwrap_or(false),
        live: v["livestream"].is_object(),
    })
}

pub fn parse_history(body: &str) -> Result<Vec<Line>> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| Error::Parse(format!("the Kick chat: {e}")))?;
    let Some(msgs) = v["data"]["messages"].as_array() else { return Err(Error::Parse("the Kick chat: no messages".into())) };
    let mut out: Vec<Line> = msgs
        .iter()
        .filter_map(|m| {
            // a reply's metadata is a JSON string holding the line it answers
            let meta: serde_json::Value = m["metadata"].as_str().and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default();
            let original = &meta["original_message"];
            Some(Line {
                id: m["id"].as_str()?.to_string(),
                at: iso_ms(m["created_at"].as_str()?)?,
                user: m["sender"]["username"].as_str()?.to_lowercase(),
                content: m["content"].as_str().unwrap_or_default().to_string(),
                reply_to: original["id"].as_str().map(str::to_string),
                reply_to_user: original["sender"]["username"].as_str().map(str::to_lowercase),
            })
        })
        .collect();
    out.sort_by_key(|l| l.at);
    Ok(out)
}

/// A bet command and its side ("home" or "away"): `!h`, `!a`, `!home 100`, `!a all`. Anything after the amount, or a
/// different command, isn't a bet.
pub fn bet_side(content: &str) -> Option<&'static str> {
    let mut words = content.split_whitespace();
    let side = match words.next()?.to_lowercase().trim_end_matches(['\'', '!', '.']) {
        "!h" | "!home" => "home",
        "!a" | "!away" => "away",
        _ => return None,
    };
    match (words.next(), words.next()) {
        (None, _) => Some(side),
        (Some(w), None) if w.eq_ignore_ascii_case("all") || w.replace(',', "").parse::<u64>().is_ok() => Some(side),
        _ => None,
    }
}

/// "2026-11-16T15:27:37Z" or "...+00:00" as ms since 1970, without a date library.
fn iso_ms(s: &str) -> Option<u64> {
    let n = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
    // days from 1970-01-01 (Howard Hinnant's days_from_civil)
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(((days * 24 + h) * 60 + mi) * 60 + se).ok().map(|secs| secs * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(name: &str) -> String {
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/pages/").to_string() + name).unwrap()
    }

    #[test]
    fn reads_a_channel() {
        let c = parse_channel(&page("kick-channel.json")).unwrap();
        assert_eq!(
            c,
            Channel {
                slug: "the-stream".into(),
                id: 1001,
                chatroom_id: 2002,
                slow_secs: Some(5),
                followers_only: true,
                subscribers_only: true,
                live: true
            }
        );
        assert!(parse_channel(r#"{"message":"Not found"}"#).is_err());
    }

    #[test]
    fn reads_the_chat_oldest_first_with_replies() {
        let lines = parse_history(&page("kick-history.json")).unwrap();
        assert_eq!(lines.len(), 6);
        assert!(lines.windows(2).all(|w| w[0].at <= w[1].at), "oldest first");
        let bet = lines.iter().find(|l| l.content == "!h 2300").unwrap();
        assert_eq!((bet.user.as_str(), bet.at), ("player-01", iso_ms("2026-11-16T15:27:37Z").unwrap()));
        let reply = lines.iter().find(|l| l.reply_to.as_deref() == Some(bet.id.as_str())).unwrap();
        assert_eq!((reply.user.as_str(), reply.reply_to_user.as_deref()), ("the-bot", Some("player-01")));
        assert!(reply.content.starts_with("2300 on HOME"));
    }

    #[test]
    fn knows_a_bet_command() {
        for (c, side) in
            [("!h", "home"), ("!A", "away"), ("!home 100", "home"), ("!a all", "away"), ("!away'", "away"), ("!h 2,300", "home")]
        {
            assert_eq!(bet_side(c), Some(side), "{c}");
        }
        for c in ["!hello", "!h now please", "!time over", "h", "!card", ""] {
            assert_eq!(bet_side(c), None, "{c}");
        }
    }

    #[test]
    fn dates() {
        assert_eq!(iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso_ms("2026-11-11T00:00:00+00:00"), Some(1_794_355_200_000));
    }
}
