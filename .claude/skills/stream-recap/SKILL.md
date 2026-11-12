---
name: stream-recap
description: Summarise a PalDuels stream's calls - the tally, streaks, the misses and what they taught, and what's still uncertain. Use when the developer says it was the last fight, the stream is over, or asks how it went.
---

# Stream recap

1. Run `cargo run -q --release -p augur -- predict --quick` to settle the last call. If the last fight isn't recorded yet, say so; it settles on the next run (or in the app).
2. `cargo run -q --release -p augur -- analyse` for the tally, calibration and the per-hit fit. For the day's calls, from the data folder `augur status` prints:

```sh
sqlite3 -header -column "<data folder>/augur.db" "
  SELECT p.home_pal, p.away_pal, p.pick, round(p.prob * 100) AS pct, d.winner, d.ko, d.winner_hp_left
  FROM predictions p JOIN duels d ON d.id = p.duel_id
  WHERE p.status = 'settled' AND date(d.fought_at) = date('now') ORDER BY p.id"
```

3. Reply with:
   - **The tally** for the day (and the percentage), and the re-score with today's logic.
   - **The best run** of right calls.
   - **Each miss in one line:** what the model believed and what the tape showed.
   - **What the day confirmed** and **what's still uncertain** (the open questions in the `tune-model` skill).
4. If the day changed what the model believes, update those open questions.
