<p align="center"><img src=".github/assets/hero.svg" alt="Augur: calls each PalDuels fight before the bell. The menu bar shows the favourite and its chance; the popover sweeps the dial, shows the stats and gives the chat line in five voices" width="100%"></p>

# Augur

![Version](https://img.shields.io/github/v/release/TrongatesLegacy/augur?include_prereleases&label=version&color=ffcf6b) ![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey) ![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-24c8db) ![Tests](https://img.shields.io/github/actions/workflow/status/TrongatesLegacy/augur/test.yml?branch=main&label=tests)

Calls the [PalDuels](https://palduels.com/live) fights (chat-run Palworld duels on a Kick stream) before the bell, and keeps score.

Augur lives in the menu bar (macOS) or the tray (Windows). The menu bar shows the favourite and its chance; on Windows the tray icon is a gold ring filled to the chance around a dot in the favourite's colour. Click it for the call: the pick, why, the stats, the bet to type (`!h`, `!h 2300` or `!h all`) and the chat line in five voices, each with a copy button. The main window has the rest: Live, History (Augur's calls, and your bets next to them), Pals, Players (pal boxes and reroll advice), Model and Settings.

Built with [Tauri 2](https://tauri.app): a Rust engine and a small web page for the screens. About 9 MB.

## Layout

| Path | What it is |
|---|---|
| `engine/` | Everything that isn't the desktop app (`augur-engine`): fetching and parsing palduels.com (`site`), the model (`model`), the SQLite store (`db`), a whole prediction (`predict`), the background backfill (`backfill`), the chat voices (`voices`), the calls file (`calls`), what the history says (`history`), the stake to suggest (`stake`), your bets and how following Augur compares (`bets`), sync across computers (`sync`) and the test seed (`seed`). |
| `app/` | The desktop app and command line (`augur`): tray, windows, sync loop, encrypted sign-in (`secure.rs`), settings, what each page is sent (`views.rs`), headless UI tests (`ui_tests.rs`). |
| `ui/` | The pages, as ES modules with no build step: `index.html` + `main.js` (main window), `tray.html` + `tray.js` (popover), `lib/` (shared pieces), `tabs/` (one file per tab), `style.css`. |
| `fixtures/calls.jsonl` | 19 made-up calls (real species, invented fighters, owners and results) in the calls-file format, with the model's expected numbers. Test data only: your own calls stay in your data folder. |
| `engine/tests/pages/` | Saved palduels.com pages the tests parse, with placeholder players. The picks pages are made-up bets in the signed-in picks list's markup. |
| `assets/` | The logo and tray icon sources. `app/icons/` is generated from `assets/icon.png` (`cargo tauri icon`). |
| `scripts/` | The release scripts the build workflow runs. |

## Running it

```sh
cargo run -p augur                 # the app
cargo run -q -p augur -- predict   # the duel that's up now, from the command line
cargo run -q -p augur -- --help    # predict, backfill, import, export, analyse, tune, bets, status
```

The app and the command line share one data folder: `~/Library/Application Support/Augur` on macOS, `%APPDATA%\Augur` on Windows (`AUGUR_DATA_DIR` overrides it). It holds `augur.db`, `settings.json`, `sync.json`, the pal art cache and the encrypted sign-in.

On first launch the store is empty: live calls work straight away, and in the background Augur fetches every species (a few minutes), then past duels with fight tapes, newest first, then the older results-only ones (about an hour in all). One page at a time, slower after any HTTP 429, paused whenever a duel is up on stream. To bring your own calls across: `augur import calls.jsonl` (and `augur export` to back them up).

## More than one computer

**Settings › Sync & data › Sync** works the way Stream Multiview's does:
- **The folder:** you pick a folder a sync app already moves. Augur finds OneDrive, iCloud Drive, Dropbox and Google Drive, or you can choose another folder. It creates `Augur Sync/` inside it.
- **No clashes:** each computer writes only its own files there (`devices/<id>.json` and `calls/<id>.jsonl`) and reads the others' every minute. Nothing needs a lock, and the sync app never makes "conflicted copies".
- **What syncs:** your settings (the latest change to each one wins) and Augur's calls (merged, never duplicated).
- **What stays on each computer:** the sign-in (it's encrypted for that computer), launch at login and background fetching. Duels, species and your bets come from the site on each computer.

**Back up… and Import…** cover setting up a new computer:
- **Back up…** saves a complete copy of the store (calls, duels, tapes, species, bets) wherever you choose, such as OneDrive.
- **Import…** merges one in, or a calls file. It adds what's missing and never overwrites, so a new PC skips the hour-long first fetch.

Don't move the data folder itself into OneDrive: the app writes to its database every few seconds, and syncing a live SQLite file can corrupt it.

The main window lays out for 1280×720 and zooms to fit any other size, so it looks the same on a laptop and on a big monitor.

## The data

| Table | One row per |
|---|---|
| `predictions` | Call: chance, each factor's share, species records and pal head-to-head as they stood, model settings used, status (`open`, `settled`, `skipped`). |
| `fighters` | Side of a call: final Lv 70 stats (with passives), IVs, types, passives, hidden-move boost, cooldown speed. |
| `duels` | Finished duel: owners, pals, winner, K.O. time, winner's HP left, cast counts. |
| `hits` | Hit on a duel's tape: side, time, move, damage as % of the target's HP. |
| `species` | Species: types, the ladder's power rating, base HP, Attack and Defence. |
| `species_moves` | Species' listed move: element, power, unlock level. |
| `bets` | Your bet, from your picks list (signed in): its place in your history, the duel (recent ones), the side backed, stake and result. |
| `meta` | Small state: backfill progress, the signed-in account's name, your last balance, when the picks list was read. |

## How the model works

1. **Stats:** time to knock the other side out = its HP ÷ (my Attack ÷ its Defence × my move strength). Move strength averages every usable move, with power counting only a little (`power ^ 0.35`), times the type multiplier (1.5× into a type it beats, ÷1.5 into one that resists it; dual types multiply). The ratio of the two times becomes a win chance.
2. **Adjustments to move strength:** damage from moves the site doesn't list (species-only attacks like Bushi Noct's Iai, found in the pal's last 3 fight tapes), and cooldown passives. Both are damped until proven.
3. **Records:** the species' win/loss records and past fights between the two species shift that chance on the log-odds scale, each pulled toward 50% so small samples barely move it.

Hidden stats (an owner with several of the pal) are estimated from base stats at an average IV until the bell. The crowd's picks are never used.

## Bets

Augur never bets for you: it suggests a stake and the chat command, and you type it. Settings > Staking picks how:

- **Kelly** (the default, a quarter of it): bet a share of your balance that grows with the edge, and sit out calls with none. The payout is the median of your own recent wins (the site shows no odds or crowd split before a fight), so at a typical 0.45 a credit a call needs about 69% to be worth a bet.
- **Fixed amount** on every call.
- **Default or max:** the max (`!h all`) from a chance you choose (70% to start), the default bet (a bare `!h`) below it.

The limits (100 to 5,000, 500 for a bare command) are settings because the site doesn't publish them. "Type the max as all" keeps `!h all` right if the stream changes the max.

Signed in, Augur reads your picks list (only shown to you, 5 bets a page): all of it once, a page at a time and paused while a duel is up, then just what's new every few minutes. History > Your bets lines your bets up with Augur's calls: **Followed** (you backed Augur's pick, at any stake), **Deviated** (the other side, or a bet when Augur said sit out), **Missed** (no bet on a call Augur would have bet on: kept out of the comparison and totalled on their own) and **No call**. "Augur's way" uses the stake Augur suggested at the time: on the side you backed it's your real payout scaled to that stake; a win on the other side is an estimate at your usual payout.

## Tests

```sh
cargo test --workspace
```

Every test runs from the seed in the repo (`fixtures/calls.jsonl` and `engine/tests/pages/`, loaded by `augur_engine::seed`): no network, no local data. The saved pages use placeholders (`player-NN`) instead of real player names. The UI tests serve the real pages from a small local web server and render them in **headless Chrome**, with a stand-in for the Tauri bridge serving what the app's commands return. Nothing appears on screen and your Chrome profile isn't touched. They skip without Chrome locally (set `CHROME` to point at it) and fail without it in CI. `AUGUR_UI_SHOTS=<folder> cargo test -p augur every_tab` also saves a 1280×720 screenshot of every tab.

## Contributing and releases

**Making a change:** open a pull request against `main`. Don't change the version or `CHANGELOG.md` in it. Write the title as one line a user would understand, because titles become the release's "What's new". Every pull request runs the tests (macOS, Windows, and the engine on Linux) and builds both installers as a test (under the run's **Artifacts**).

**Making a release:** publish a GitHub release with a new tag like `v0.1.1` from the latest `main`, and click **Generate release notes**. The build workflow sets the version from the tag, adds the notes to `CHANGELOG.md`, commits that to `main`, moves the tag onto it, builds the installers and attaches them:

- **Mac:** `Augur_<version>_universal.dmg` (Apple silicon and Intel) and the app as a `.zip`.
- **Windows:** `Augur_<version>_x64-setup.exe` (installs for the current user, no admin needed).

**Secrets the release uses:**

| Secret | What it's for |
|---|---|
| `MAC_CERT_P12`, `MAC_CERT_PASSWORD` | Augur's own self-signed certificate ("Augur Code Signing"), base64 of the .p12 and its password. Every version is signed with it, so macOS recognises updates as the same app and keeps letting Augur read its Keychain item (the saved sign-in). Without it the app is signed ad hoc and the saved sign-in won't survive updates. |
| `RELEASE_DEPLOY_KEY` | Optional: a deploy key with write access, for pushing the version commit if `main` is protected by a ruleset that lists the key under Bypass. Without it the release pushes with the workflow's own token. |

The certificate isn't Apple's, so the first time someone opens a new install, macOS asks them to allow it in **System Settings → Privacy & Security → Open Anyway**. Windows SmartScreen may say "Windows protected your PC": **More info → Run anyway**.
