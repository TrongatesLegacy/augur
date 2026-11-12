//! The chat lines under the pick: one call in five voices, worded by confidence, with the favourite's biggest
//! advantage as the reason.

use crate::model::Fighter;
use regex::Regex;
use std::sync::OnceLock;

pub const VOICES: [&str; 5] = ["Yoda", "Gollum", "Dr. Rockso", "Scooby-Doo", "Daffy Duck"];

/// The reason in each voice, in VOICES order.
fn reason_lines(key: &str) -> [&'static str; 5] {
    match key {
        "atk" => [
            "Hits harder, it does.",
            "Hits harder, it does, yesss.",
            "Hits harder than a drum solo, baby!",
            "Rit hits ruge!",
            "It hitth harder, thucker!",
        ],
        "hp" => [
            "More life in it, there is.",
            "So much juicy life in it, precious.",
            "More stamina than a three-hour encore!",
            "Rit's got rots of HP!",
            "More HP than you've got brainth!",
        ],
        "def" => [
            "Tough to break, it is.",
            "Tough skin, too tough for nasty biteses.",
            "Tough as a stack of Marshall amps, baby!",
            "Rit's too rough to rite!",
            "Tougher than my thecond-day thoup!",
        ],
        "types" => [
            "The types, on its side they are.",
            "The types loves it, they does.",
            "The types are rockin' its way!",
            "Rype advantage, ruh-huh!",
            "Even the typeth are on itth thide!",
        ],
        "speed" => [
            "Swift, its attacks are.",
            "Quick, quick it strikes, gollum.",
            "Shreds faster than a guitar solo!",
            "Rit's ruper rast!",
            "Fathter than a thtampeding duck!",
        ],
        "hidden" => [
            "Hidden moves, it has.",
            "Secret tricksies it has, hidden ones.",
            "Got secret riffs nobody's heard!",
            "Rit's got recret roves!",
            "Thecret movth, how dethpicable!",
        ],
        "species" => [
            "Won much before, its kind has.",
            "Its kind wins and wins, they do.",
            "Its kind's got gold records!",
            "Rit rins a rot!",
            "Itth kind winth and winth!",
        ],
        "h2h" => [
            "Beaten this foe before, it has.",
            "Beat this one before, we remembers.",
            "Already played this crowd and killed it!",
            "Reat rit refore!",
            "Already beat thith thucker before!",
        ],
        _ => [
            "Even, the stats are.",
            "Same and same they looks, precious.",
            "Dead-even battle of the bands!",
            "Ruh-roh, rit's rose!",
            "Thtatth are dead even, thufferin' thuccotash!",
        ],
    }
}

/// The favourite's single biggest advantage, as a reason key.
pub fn reason(me: &Fighter, op: &Fighter, edge_moves: usize, rate_me: f64, rate_op: f64, p_h2h_me: f64) -> &'static str {
    let edges = [
        (me.atk / op.atk, "atk"),
        (me.hp / op.hp, "hp"),
        (me.def / op.def, "def"),
        (1.0 + 0.15 * edge_moves as f64, "types"),
        (me.speed, "speed"),
        (me.hidden_boost, "hidden"),
        (rate_me / rate_op.max(0.01), "species"),
        (0.5 + p_h2h_me, "h2h"),
    ];
    let best = edges.iter().fold(edges[0], |a, b| if b.0 > a.0 { *b } else { a });
    if best.0 > 1.05 {
        best.1
    } else {
        "even"
    }
}

/// Scooby-Doo speech: swap each word's leading consonants for R (Llamatodd -> Ramatodd, Arsox -> Rarsox).
pub fn scoobify(name: &str) -> String {
    name.split_whitespace()
        .map(|w| {
            let rest: String = w.chars().skip_while(|c| !"aeiouAEIOU".contains(*c)).collect();
            if rest.len() < w.len() {
                format!("R{}", rest.to_lowercase())
            } else {
                format!("R{}", w.to_lowercase())
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Daffy Duck's lisp: s/z become th (proper names too, for comic effect).
pub fn lisp(s: &str) -> String {
    static LOWER: OnceLock<Regex> = OnceLock::new();
    static UPPER: OnceLock<Regex> = OnceLock::new();
    let s = LOWER.get_or_init(|| Regex::new("[sz]").unwrap()).replace_all(s, "th");
    UPPER.get_or_init(|| Regex::new("[SZ]").unwrap()).replace_all(&s, "Th").into_owned()
}

/// All five chat lines, in VOICES order. `p` is the favourite's chance.
pub fn lines(pal: &str, foe: &str, p: f64, reason_key: &str) -> [String; 5] {
    let pct = format!("{}%", (p * 100.0).round());
    let [yoda, gollum, rockso, scooby, daffy] = reason_lines(reason_key);
    let band = if p >= 0.8 {
        3
    } else if p >= 0.65 {
        2
    } else if p >= 0.55 {
        1
    } else {
        0
    };
    [
        match band {
            3 => format!("Win, {pal} will. {yoda} ({pct})"),
            2 => format!("Favoured, {pal} is. {yoda} ({pct})"),
            1 => format!("Toward {pal}, lean I do. {yoda} Close, it is. ({pct})"),
            _ => format!("Clouded, this fight is. {pal}, perhaps. ({pct})"),
        },
        match band {
            3 => format!("{pal} wins it, precious, yesss! {gollum} ({pct})"),
            2 => format!("We bets on {pal}, we does. {gollum} ({pct})"),
            1 => format!("Maybe {pal}, precious... maybe. {gollum} ({pct})"),
            _ => format!("Tricksy fight, nasty and close. {pal}, perhapses? ({pct})"),
        },
        match band {
            3 => format!("Dr. Rockso, the rock and roll clown, says {}! {rockso} Rock and roll! ({pct})", pal.to_uppercase()),
            2 => format!("Dr. Rockso's backing {pal}! {rockso} ({pct})"),
            1 => format!("Dr. Rockso's leaning {pal}... like a guitarist mid-solo! {rockso} ({pct})"),
            _ => format!("Dr. Rockso can't call this one, baby. {pal}, maybe? It's a coin toss in a mosh pit! ({pct})"),
        },
        match band {
            3 => format!("Ruh-roh, {}! {} rins! {scooby} Scooby-Dooby-Doo! ({pct})", scoobify(foe), scoobify(pal)),
            2 => format!("Rooby rets on {}! {scooby} ({pct})", scoobify(pal)),
            1 => format!("Rmm... {}, raybe? {scooby} ({pct})", scoobify(pal)),
            _ => format!("Ruh-roh, rit's a rystery! {}, raybe? ({pct})", scoobify(pal)),
        },
        match band {
            3 => format!("{}dethpicable! {daffy} ({pct})", lisp(&format!("{pal} winth, and {foe} is "))),
            2 => format!("{}{daffy} ({pct})", lisp(&format!("My money'th on {pal}! "))),
            1 => format!("{}{daffy} Don't hold me to it, thucker! ({pct})", lisp(&format!("I gueth {pal}... "))),
            _ => format!("{}I'm not betting the farm! ({pct})", lisp(&format!("Who knowth? {pal}, maybe. "))),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voices_match_the_python_ones() {
        let l = lines("Chillet Lux", "Chikipi Ryu", 0.83, "atk");
        assert_eq!(l[0], "Win, Chillet Lux will. Hits harder, it does. (83%)");
        assert_eq!(l[3], "Ruh-roh, Rikipi Ru! Rillet Rux rins! Rit hits ruge! Scooby-Dooby-Doo! (83%)");
        assert_eq!(l[4], "Chillet Lux winth, and Chikipi Ryu ith dethpicable! It hitth harder, thucker! (83%)");
        assert_eq!(scoobify("Arsox"), "Rarsox");
        assert_eq!(scoobify("Llamatodd"), "Ramatodd");
    }
}
