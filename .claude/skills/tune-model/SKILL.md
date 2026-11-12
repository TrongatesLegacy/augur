---
name: tune-model
description: Re-test Augur's model settings against the real calls and the backfilled history, and change them only when the evidence holds. Use after a miss with a pattern, when the per-hit fit drifts from the model's settings, or when the developer asks to adapt or improve the logic.
---

# Tune the model

The settings are `Settings::default()` in `engine/src/model.rs`, each with a comment saying what it was fitted on. The app's Model tab can also try a suggested value for one install (saved in its settings.json).

## Re-test

```sh
cargo run -q --release -p augur -- tune                                   # the default grid
cargo run -q --release -p augur -- tune TYPE_BONUS=1.25,1.5,2 STATS_STEEPNESS=3,4
cargo run -q --release -p augur -- analyse
```

- **tune** scores each combination on every real call with fighter data: **log loss** (lower is better; it punishes confident misses) and calls right. It changes nothing.
- **analyse** prints the tally and calibration, the **per-hit fit** (how much move power and a type edge really add, from the fight tapes; noisy, so trust results over it), the **history test** (every past duel with fighters rebuilt from species base stats at an average roll, records as they stood before each duel: good for type and record settings, not `stats_steepness`, which it always wants flatter) and the species leaning on unlisted moves.

Refresh the history first if it's stale: `cargo run -q --release -p augur -- backfill` (new duels only; the app does this in the background anyway).

## Change a setting only when

- the gain shows across several fights, not one (a single miss is not a pattern);
- it doesn't swap one right call for another wrong one with no net gain (say so when it does);
- you pick a value in the middle of the good range, not the single best grid point.

Then edit `engine/src/model.rs` (value and comment: why, on how many fights), re-run `cargo test --workspace` and update the fixture's expected numbers (the parity test says how), and tell the developer the before and after. Always say the settings were scored on the same calls they were fitted to.

## Adding a factor

Something the model can't see (a passive, an unlisted move, an owner's rating) goes in `model::score()` or a helper next to it, with its raw value saved on the call (`fighters` or `predictions` in `engine/src/db.rs`) so later re-scores are fair. New effects start damped (for example `.powf(0.5)`) until tapes or results confirm them.

## Known open questions

- **Type edge:** 1.5x. The 4,702-duel history test prefers 1.25-1.5x; per-hit tapes show about 1.1x; when only one side's *species* types beat the other's, that side won 48% of 1,292 duels. The first 19 live calls prefer 2x by a hair.
- **Resisted moves** don't hit softer per hit, yet `type_resist` 1.5 still scores best in both tests.
- **Cooldown passives** are counted at half strength. Check `duels.home_casts` / `away_casts` for pals with them.
- **Move power** matters a little and unevenly (best hits by power band: 40-80: 9%, 81-250: 13%, 251-450: 19%, 451+: 13%).
- **Owners** matter (7-3 rivalries) but aren't in the model yet.
