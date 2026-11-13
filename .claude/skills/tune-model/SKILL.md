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
- **Real hit size** (`HIT_WEIGHT`, starts at 0): each move's mean hit in the tapes against the average move. On 12 Nov a weight of 0.25 improved the held-back loss 0.6582 → 0.6555 in a one-off test, but the learner kept 0 on its own fit; it switches on if the evidence grows. Rockburst hits 1.76× the average move.
- **Records on real calls** (`SPECIES_LIVE`, follows `SPECIES_WEIGHT` until learned): the species weight is fitted on past duels, which have base stats only. On the first 50 real calls (13 Nov), stats alone got 37/49 right and the records alone 24/42; where the two disagreed, stats were right 13 of 17. Re-scored, 0.3 got 40/50 (log loss 0.591) against 36/50 (0.610) at 1.2, but the two halves disagreed and a leave-one-out fit wasn't better (0.611), so it learns on real calls rather than being set by hand: after every result it moves calls / (calls + 60) of the way from `SPECIES_WEIGHT` to the weight that fits every call best, in steps of 0.05. On the 54 calls of 13 Nov that's 0.9.
- **Move power** matters a little and unevenly (best hits by power band: 40-80: 9%, 81-250: 13%, 251-450: 19%, 451+: 13%).
- **Pal rank** (`PAL_RANK_WEIGHT`, starts at 0, per 10 points of the site's power score; learns on real calls that read the rank, once there are 60): the site's "Tier C - ranked 154 of 357 species, power 55/100", which moves as a species wins or loses against its rank. On 13 Nov, over 1,346 recent past duels (from the 357 pal pages; fit on the oldest 1,009, checked on the newest 337), power alone got 57.6% right against Augur's 65.9%, and added to Augur its best weight was slightly negative (held-back loss 0.6651 → 0.6660), the same for species with under 20 past duels. On the 36 real calls in that set it hinted at about 0.4 per 10 points (69% → 75% right, scored on the calls it was fitted to; 9 held back, no change), too few to tell.
- **Owners** were tested and left out: over 4,702 duels (fit on the oldest 3,500, checked on the rest) an owner's record alone got 50.4% right, and added to the species records it changed nothing (58.4% against 58.7%).
