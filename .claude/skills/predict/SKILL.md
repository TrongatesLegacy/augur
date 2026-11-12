---
name: predict
description: Predict the current or next PalDuels fight from palduels.com/live and reply with the pick, the five chat lines and a short case. Use whenever the developer says "next", "next fight", "check" or sends a PalDuels screenshot.
---

# Predict the next fight

```sh
cargo run -q --release -p augur -- predict
```

(Or the installed app's own binary: `/Applications/Augur.app/Contents/MacOS/augur predict`.) It settles finished calls, prints the pick first, then the five chat lines, the case and the tally, and saves the call in the app's data folder, the same store the installed app uses (`augur status` shows where). `--quick` skips the head-to-head lookups; `--json` prints everything.

## The reply

Lead with the pick, then:

1. **Paste for chat:** all five voice lines, each under its own bold label (Yoda, Gollum, Dr. Rockso, Scooby-Doo, Daffy Duck) as a blockquote. Always all five.
2. **Pick:** `## Pick: <pal> (<side>, <owner>), <chance>`, then the `STAKE:` line as **Bet:** the command to type and why (e.g. `!a all`, "capped at the 5,000 max", or "no bet").
3. **A stats table:** types, HP, Attack, Defence, passives if any, species record. Bold the better value.
4. **Three to five bullets** on what decides it: stats, type edge (count the super-effective moves), records, pal and owner head-to-head, unlisted moves if they matter.
5. **The risk to the pick**, when there is a real one.
6. **Last fight:** right or wrong, with the K.O. time and HP left; for a miss, what the tape shows went wrong (`sqlite3 "<data folder>/augur.db" "SELECT side, t, move, pct FROM hits WHERE duel_id = N"`).
7. **Tally:** live, and with today's logic.

If you think the model's number is off (one factor carrying it, a coin flip called at 80%+), say what you'd put it at and why.

## Rules

- **No crowd data.** Never use or fetch chat or crowd picks (`call.crowd`), and don't move a pick because chat disagrees.
- **Hidden stats.** When the owner has several of the pal, its stats stay hidden until the bell and are estimated (marked in the report). Say so, and run it again once the fight starts or the developer sends a screenshot.
- **A miss** is a reason to look at the tape and, if there's a pattern, run the `tune-model` skill. Don't retune on one fight.
- **"No duel up right now"** means nothing is queued. If the developer says a fight happened, the site may not have recorded it yet: wait a minute and run again.
