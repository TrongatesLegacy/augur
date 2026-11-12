---
name: site-probe
description: Investigate palduels.com - the live state API and the duel, pal, rivalry and player pages - with a few read-only requests before changing a parser, or when predictions, the backfill or a page stop working.
---

# Probe palduels.com

The site has no documented API and changes without notice. Check what it returns today before changing `engine/src/site.rs`.

## Rules

- **Read-only, and few.** GET requests only, a handful at a time, through `augur_engine::site::fetch` (it backs off) or `curl` with a pause between pages. The site answers bursts with HTTP 429.
- **Signed-out by default.** Everything Augur needs is public. Pages that need a sign-in (the picks history, the pet, Hall) only with the developer's OK, in their own browser; report what's there, never the data.
- **No crowd data.** Don't read or report the crowd's picks (`call.crowd`), even while probing.
- **Nothing personal in the repo or the PR.** Report status codes, field names, class names and counts, not other players' names or balances.

## What Augur reads

| What | Where | Parsed by |
|---|---|---|
| The duel that's up, its stats, skills, passives, tale of the tape, recent results, queue | `GET /api/state.php` (JSON; `hud` and `skills` are HTML) | `site::State`, `parse_stats`, `parse_skills`, `parse_passives`, `parse_tale` |
| A finished duel: result, K.O., fight tape | `GET /duel/<id>?show=1` | `parse_duel` |
| A species: types, power, base stats, moves, owners, duels (paged) | `GET /pal/<slug>`, `?p=N` | `parse_species`, `parse_moveset`, `duel_rows` |
| Two players' rivalry | `GET /vs/<a>/<b>` | `parse_rivalry` |
| A player's pal box: IVs, rerolls, passives | `GET /u/<name>` (the "Pals" section) | `parse_roster` |
| Every species | `GET /pals` | `species_slugs` |
| Pal art (personal use, cached, never committed) | `GET /img/pals/<slug>.webp` | `fetch_bytes` |

## When a parser changes

Save the page that shows the new markup into `engine/tests/pages/` (signed out), replace other players' names with `player-NN` placeholders (and anything naming the stream with `the-stream`), and add or update the test in `site.rs` that reads it. Then the change is tested without the network, like everything else.
