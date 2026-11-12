//! The model must give the expected numbers on every call in the seed (fixtures/calls.jsonl: made-up fighters, real
//! species). It guards the model: after deliberately changing a setting in `Settings::default()`, rewrite them with
//!     AUGUR_UPDATE_FIXTURE=1 cargo test -p augur-engine --test parity
//! and say in the pull request what moved.

use augur_engine::model::{score, Fighter, Settings};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/calls.jsonl");

#[derive(Deserialize)]
struct Call {
    call: u32,
    fighters: HashMap<String, Fighter>,
    species_rec: [[f64; 2]; 2],
    pal_h2h: [f64; 2],
    model: Settings,
    expected: HashMap<String, f64>,
}

fn scored(c: &Call, s: &Settings) -> [(&'static str, f64); 6] {
    let sc = score(s, &c.fighters["home"], &c.fighters["away"], c.species_rec, c.pal_h2h);
    [
        ("t_home", sc.t_home),
        ("t_away", sc.t_away),
        ("p_stats", sc.p_stats),
        ("p_species", sc.p_species),
        ("p_h2h", sc.p_h2h),
        ("p_home", sc.p_home),
    ]
}

#[test]
fn the_model_gives_the_expected_numbers_on_every_seeded_call() {
    let text = std::fs::read_to_string(FIXTURE).unwrap();
    if std::env::var_os("AUGUR_UPDATE_FIXTURE").is_some() {
        let s = Settings::default();
        let mut out = String::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let c: Call = serde_json::from_str(line).unwrap();
            let mut v: Value = serde_json::from_str(line).unwrap();
            v["model"] = serde_json::to_value(&s).unwrap();
            v["expected"] = scored(&c, &s)
                .iter()
                .map(|(k, x)| (k.to_string(), Value::from((x * 1e9).round() / 1e9)))
                .collect::<serde_json::Map<_, _>>()
                .into();
            out.push_str(&serde_json::to_string(&v).unwrap());
            out.push('\n');
        }
        std::fs::write(FIXTURE, out).unwrap();
        return;
    }
    let mut n = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let c: Call = serde_json::from_str(line).unwrap();
        for (k, v) in scored(&c, &c.model) {
            let want = c.expected[k];
            assert!((v - want).abs() <= 1e-6 * want.abs().max(1.0), "call {}: {k} is {v}, expected {want}", c.call);
        }
        n += 1;
    }
    assert!(n >= 19, "only {n} calls in the fixture");
}

#[test]
fn the_shipped_settings_are_the_ones_the_fixture_was_scored_with() {
    let text = std::fs::read_to_string(FIXTURE).unwrap();
    let first: Call = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(first.model, Settings::default(), "the model's settings changed: rewrite the fixture (see the top of this file)");
}
