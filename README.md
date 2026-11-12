<p align="center"><img src=".github/assets/hero.svg" alt="Augur: calls each PalDuels fight before the bell. The menu bar shows the favourite and its chance; the popover sweeps the dial, shows the stats and gives the chat line in five voices" width="100%"></p>

# Augur

![Version](https://img.shields.io/github/v/release/TrongatesLegacy/augur?include_prereleases&label=version&color=ffcf6b) ![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey) ![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-24c8db) ![Tests](https://img.shields.io/github/actions/workflow/status/TrongatesLegacy/augur/test.yml?branch=main&label=tests)

Calls the [PalDuels](https://palduels.com/live) fights (chat-run Palworld duels on a Kick stream) before the bell, and keeps score.

Augur lives in the menu bar (macOS) or the tray (Windows). The menu bar shows the favourite and its chance; on Windows the tray icon is a gold ring filled to the chance around a dot in the favourite's colour. Click it for the call: the pick, why, the stats, and the chat line in five voices with a copy button. The main window has the rest: Live, History, Pals, Players (pal boxes and reroll advice), Model and Settings.

Built with [Tauri 2](https://tauri.app): a Rust engine and a small web page for the screens. About 9 MB.

## Layout

| Path | What it is |
|---|---|
| `engine/` | Everything that isn't the desktop app (`augur-engine`): fetching and parsing palduels.com (`site`), the model (`model`), the SQLite store (`db`), a whole prediction (`predict`), the background backfill (`backfill`), the chat voices (`voices`), the calls file (`calls`), what the history says (`history`) and the test seed (`seed`). |
| `app/` | The desktop app and command line (`augur`): tray, windows, sync loop, encrypted sign-in (`secure.rs`), settings, what each page is sent (`views.rs`), headless UI tests (`ui_tests.rs`). |
| `ui/` | The pages, as ES modules with no build step: `index.html` + `main.js` (main window), `tray.html` + `tray.js` (popover), `lib/` (shared pieces), `tabs/` (one file per tab), `style.css`. |
| `fixtures/calls.jsonl` | 19 made-up calls (real species, invented fighters, owners and results) in the calls-file format, with the model's expected numbers. Test data only: your own calls stay in your data folder. |
| `engine/tests/pages/` | Saved palduels.com pages the tests parse. |
| `assets/` | The logo and tray icon sources. `app/icons/` is generated from `assets/icon.png` (`cargo tauri icon`). |
| `scripts/` | The release scripts the build workflow runs. |

## Running it

```sh
cargo run -p augur                 # the app
cargo run -q -p augur -- predict   # the duel that's up now, from the command line
cargo run -q -p augur -- --help    # predict, backfill, import, export, analyse, tune, status
```

The app and the command line share one data folder: `~/Library/Application Support/Augur` on macOS, `%APPDATA%\Augur` on Windows (`AUGUR_DATA_DIR` overrides it). It holds `augur.db`, `settings.json`, the pal art cache and the encrypted sign-in.

On first launch the store is empty: live calls work straight away, and in the background Augur fetches every species (a few minutes), then past duels with fight tapes, newest first, then the older results-only ones (about an hour in all). One page at a time, slower after any HTTP 429, paused whenever a duel is up on stream. To bring your own calls across: `augur import calls.jsonl` (and `augur export` to back them up).

## The data

| Table | One row per |
|---|---|
| `predictions` | Call: chance, each factor's share, species records and pal head-to-head as they stood, model settings used, status (`open`, `settled`, `skipped`). |
| `fighters` | Side of a call: final Lv 70 stats (with passives), IVs, types, passives, hidden-move boost, cooldown speed. |
| `duels` | Finished duel: owners, pals, winner, K.O. time, winner's HP left, cast counts. |
| `hits` | Hit on a duel's tape: side, time, move, damage as % of the target's HP. |
| `species` | Species: types, the ladder's power rating, base HP, Attack and Defence. |
| `species_moves` | Species' listed move: element, power, unlock level. |
| `meta` | Small state: backfill progress, the signed-in account's name. |

## How the model works

1. **Stats:** time to knock the other side out = its HP ÷ (my Attack ÷ its Defence × my move strength). Move strength averages every usable move, with power counting only a little (`power ^ 0.35`), times the type multiplier (1.5× into a type it beats, ÷1.5 into one that resists it; dual types multiply). The ratio of the two times becomes a win chance.
2. **Adjustments to move strength:** damage from moves the site doesn't list (species-only attacks like Bushi Noct's Iai, found in the pal's last 3 fight tapes), and cooldown passives. Both are damped until proven.
3. **Records:** the species' win/loss records and past fights between the two species shift that chance on the log-odds scale, each pulled toward 50% so small samples barely move it.

Hidden stats (an owner with several of the pal) are estimated from base stats at an average IV until the bell. The crowd's picks are never used.

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
