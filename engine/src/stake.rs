//! How much to bet on a call, and the chat command for it. Bets are typed in Kick chat: `!h` / `!a` bets the default
//! amount, `!h 2300` an amount and `!h all` the most allowed. The limits are settings (the site doesn't publish them),
//! and the payout comes from the developer's own past wins: the site shows no odds or crowd split before a fight.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    /// a share of the Kelly stake: more on bigger edges, nothing when there's none
    #[default]
    Kelly,
    /// the same amount on every call
    Fixed,
    /// the max from `max_from` up, the default bet below it
    DefaultOrMax,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StakeSettings {
    pub strategy: Strategy,
    /// share of the full Kelly stake (full Kelly swings hard when the chance is a little off)
    pub kelly_fraction: f64,
    pub fixed_amount: u64,
    /// Default or max: bet the max from this chance up
    pub max_from: f64,
    /// sit out calls below this chance (None: always suggest a bet)
    pub skip_below: Option<f64>,
    pub min_bet: u64,
    pub max_bet: u64,
    /// what a bare `!h` bets
    pub default_bet: u64,
    pub round_to: u64,
    /// type the max as `!h all`, which stays right if the stream changes the max
    pub max_as_all: bool,
    /// the balance Kelly uses when Augur can't read it (signed out)
    pub bankroll: Option<u64>,
}

impl Default for StakeSettings {
    fn default() -> Self {
        StakeSettings {
            strategy: Strategy::Kelly,
            kelly_fraction: 0.25,
            fixed_amount: 1000,
            max_from: 0.70,
            skip_below: None,
            min_bet: 100,
            max_bet: 5000,
            default_bet: 500,
            round_to: 100,
            max_as_all: true,
            bankroll: None,
        }
    }
}

/// What a winning bet pays: profit per credit staked, from the developer's own wins.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Odds {
    pub rate: f64,
    /// how many wins it's from (0: none yet, so `ASSUMED`)
    pub wins: usize,
}

impl Default for Odds {
    fn default() -> Self {
        Odds::assumed()
    }
}

impl Odds {
    /// until there are wins to go on: about what favourites paid in the first bets looked at
    pub const ASSUMED: f64 = 0.5;

    pub fn assumed() -> Self {
        Odds { rate: Self::ASSUMED, wins: 0 }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Stake {
    /// credits to bet; 0 means sit it out
    pub amount: u64,
    /// what to type in chat ("" when sitting it out)
    pub command: String,
    pub why: String,
    /// expected profit per credit at the usual payout (negative: loses on average)
    pub edge: f64,
}

/// The chat command for a bet on `pick` ("home" or "away").
pub fn command(pick: &str, amount: u64, s: &StakeSettings) -> String {
    let side = if pick == "home" { "!h" } else { "!a" };
    if amount == 0 {
        String::new()
    } else if amount >= s.max_bet && s.max_as_all {
        format!("{side} all")
    } else if amount == s.default_bet {
        side.to_string()
    } else {
        format!("{side} {amount}")
    }
}

/// The stake for a call with chance `prob` on `pick`. `balance`: the developer's credits, when known.
pub fn suggest(pick: &str, prob: f64, s: &StakeSettings, odds: &Odds, balance: Option<u64>) -> Stake {
    let edge = prob * odds.rate - (1.0 - prob);
    let pct = |p: f64| format!("{:.0}%", p * 100.0);
    let sit = |why: String| Stake { amount: 0, command: String::new(), why, edge };
    if let Some(floor) = s.skip_below.filter(|f| prob < *f) {
        return sit(format!("Sit it out: {} is under your {} floor", pct(prob), pct(floor)));
    }
    let (amount, why) = match s.strategy {
        Strategy::Fixed => (s.fixed_amount, "Fixed amount".to_string()),
        Strategy::DefaultOrMax if prob >= s.max_from => (s.max_bet, format!("Max bet: {} is {} or more", pct(prob), pct(s.max_from))),
        Strategy::DefaultOrMax => (s.default_bet, format!("Default bet: {} is under {}", pct(prob), pct(s.max_from))),
        Strategy::Kelly => {
            if edge <= 0.0 {
                return sit(format!(
                    "Sit it out: wins pay about {:.2} a credit, so it needs {}+ to pay on average",
                    odds.rate,
                    pct(1.0 / (1.0 + odds.rate))
                ));
            }
            let Some(bank) = balance.or(s.bankroll) else {
                return Stake {
                    amount: s.default_bet,
                    command: command(pick, s.default_bet, s),
                    why: "Default bet: Kelly needs your balance (sign in, or set it in Settings)".into(),
                    edge,
                };
            };
            let share = edge / odds.rate * s.kelly_fraction;
            ((bank as f64 * share) as u64, format!("{:.1}% of your {} credits", share * 100.0, short(bank)))
        }
    };
    let rounded = if s.round_to > 1 { ((amount as f64 / s.round_to as f64).round() as u64) * s.round_to } else { amount };
    let clamped = rounded.clamp(s.min_bet, s.max_bet.max(s.min_bet));
    let why = match clamped.cmp(&rounded) {
        std::cmp::Ordering::Less => format!("{why}, capped at the {} max", short(s.max_bet)),
        std::cmp::Ordering::Greater => format!("{why}, raised to the {} minimum", short(s.min_bet)),
        std::cmp::Ordering::Equal => why,
    };
    Stake { amount: clamped, command: command(pick, clamped, s), why, edge }
}

/// 248000 -> "248k", 5000 -> "5,000".
fn short(n: u64) -> String {
    if n >= 100_000 {
        format!("{}k", n / 1000)
    } else {
        let s = n.to_string();
        let mut out = String::new();
        for (i, c) in s.chars().enumerate() {
            if i > 0 && (s.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(c);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ODDS: Odds = Odds { rate: 0.45, wins: 20 };

    #[test]
    fn commands() {
        let s = StakeSettings::default();
        assert_eq!(command("home", 500, &s), "!h", "the default bet is a bare command");
        assert_eq!(command("away", 2300, &s), "!a 2300");
        assert_eq!(command("home", 5000, &s), "!h all");
        assert_eq!(command("home", 5000, &StakeSettings { max_as_all: false, ..s.clone() }), "!h 5000");
        assert_eq!(command("home", 0, &s), "");
    }

    #[test]
    fn kelly() {
        let s = StakeSettings::default();
        // edge at 75%: 0.75 * 0.45 - 0.25 = 0.0875 a credit; full Kelly 0.0875 / 0.45 = 19.4%, a quarter 4.86%
        let k = suggest("home", 0.75, &s, &ODDS, Some(40_000));
        assert_eq!((k.amount, k.command.as_str()), (1900, "!h 1900"));
        assert!((k.edge - 0.0875).abs() < 1e-9);
        let big = suggest("away", 0.75, &s, &ODDS, Some(248_000));
        assert_eq!((big.amount, big.command.as_str()), (5000, "!a all"));
        assert!(big.why.contains("capped at the 5,000 max"), "{}", big.why);
        let none = suggest("home", 0.65, &s, &ODDS, Some(248_000));
        assert_eq!(none.amount, 0, "at 65% a 0.45 payout loses on average");
        assert!(none.why.contains("needs 69%+"), "{}", none.why);
        let small = suggest("home", 0.70, &s, &ODDS, Some(1_000));
        assert_eq!(small.amount, 100, "never under the minimum");
        let unknown = suggest("home", 0.8, &s, &ODDS, None);
        assert_eq!((unknown.amount, unknown.command.as_str()), (500, "!h"), "no balance: the default bet");
        assert_eq!(suggest("home", 0.75, &StakeSettings { bankroll: Some(40_000), ..s }, &ODDS, None).amount, 1900);
    }

    #[test]
    fn fixed_and_default_or_max() {
        let fixed = StakeSettings { strategy: Strategy::Fixed, fixed_amount: 1250, ..Default::default() };
        assert_eq!(suggest("home", 0.55, &fixed, &ODDS, None).amount, 1300, "rounded to the nearest 100");
        let tiers = StakeSettings { strategy: Strategy::DefaultOrMax, ..Default::default() };
        assert_eq!(suggest("home", 0.69, &tiers, &ODDS, None).command, "!h");
        assert_eq!(suggest("away", 0.70, &tiers, &ODDS, None).command, "!a all");
        let floor = StakeSettings { skip_below: Some(0.6), ..tiers };
        let sat = suggest("home", 0.55, &floor, &ODDS, None);
        assert_eq!((sat.amount, sat.command.as_str()), (0, ""));
        assert!(sat.why.contains("under your 60% floor"));
    }

    #[test]
    fn short_numbers() {
        assert_eq!(short(248_000), "248k");
        assert_eq!(short(5000), "5,000");
        assert_eq!(short(100), "100");
    }
}
