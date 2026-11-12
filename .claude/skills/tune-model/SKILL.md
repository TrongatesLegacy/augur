---
name: tune-model
description: Re-test Augur's model settings against the real calls and the backfilled history, and change them only when the evidence holds. Use after a miss with a pattern, when the per-hit fit drifts from the model's settings, or when the developer asks to adapt or improve the logic.
---

# Tune the model

The app tunes itself (`engine/src/learn.rs`): it re-fits each setting on new results and keeps a change only when the held-back newest quarter of the duels (or calls) agrees. `Settings::default()` in `engine/src/model.rs` is only where it starts, so change a default only to give new installs a better start; to change how the learning works, edit `learn.rs` (each setting's evidence and grid, the margins, the minimum calls). `cargo run -q --release -p augur -- learn --dry-run` runs it now and prints what it would change.

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
- **Cooldown passives** start at half strength (`COOLDOWN_EXP` 0.5) and learn once 60 calls are in, 15 of them with a cooldown passive.
- **Move power** matters a little and unevenly (best hits by power band: 40-80: 9%, 81-250: 13%, 251-450: 19%, 451+: 13%).
- **Owners** were tested and left out: over 4,702 duels (fit on the oldest 3,500, checked on the rest) an owner's record alone got 50.4% right, and added to the species records it changed nothing (58.4% against 58.7%).
