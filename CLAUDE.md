# Augur

Calls the fights on [PalDuels](https://palduels.com/live) (chat-run Palworld duels on a Kick stream) and keeps score. A Tauri 2 tray app: Rust engine in `engine/`, the app and command line in `app/`, pages in `ui/`. `README.md` covers the layout, the data, the model, the tests and releases.

## Working here

- **Predictions:** use the `predict` skill whenever the developer says "next", "check" or sends a screenshot. Its reply format (all five chat voices first, then a stats table and a short case) is what the developer wants every time.
- **Changing the model:** use the `tune-model` skill. The developer is happy for the logic to adapt as results come in, but re-test first, and say when a setting was scored on the same calls it was fitted to.
- **End of stream:** use the `stream-recap` skill.
- **No crowd data.** Never use or fetch chat or crowd picks (the state's `call.crowd`), and don't move a pick because chat disagrees.
- **Be gentle with the site.** It rate-limits bursts (HTTP 429). Fetch through `augur_engine::site` (it backs off), keep parallel fetches at 4 or fewer, and pace backfills (`backfill::Pacer`).
- **Pal art** comes from palduels.com on demand and is cached locally for personal use. Never commit or bundle it.
- **The repo is public.** Never commit the developer's own calls, bets, balance or database, or a real player's name (saved pages use `player-NN`; the stream is "the-stream"). Test data is made up or scrubbed. Their real calls live in their data folder, with a backup at `data/calls-real-19.jsonl` (gitignored).

## Code conventions

CI enforces the mechanical ones (`cargo fmt --check`, `cargo clippy -- -D warnings`, the workspace lints in `Cargo.toml`, a syntax check of `ui/`). The rest is by hand:

- **Every Rust file** starts with a `//!` comment saying what it's for. Then imports in three groups, blank line between: `std`, other crates, then `crate::` / `augur_engine::`. Public items first, private helpers after, `#[cfg(test)] mod tests` last. Long files are split with `// --- section ---` lines.
- **Errors:** the engine returns `augur_engine::Result` (one `Error` enum: Http, Net, Db, Io, Parse). Errors become text only at the edges: the Tauri commands (`Result<T, String>`) and the command line. rusqlite's own `Result` is only for row closures and collecting rows (`Rq` in `db.rs` and `history.rs`).
- **Unsafe code** is denied; the two Windows system calls that need it have an `#[allow(unsafe_code)]` with a comment saying why.
- **JSON field names** are `snake_case`, except the model's `Settings`, which keeps `TYPE_BONUS`-style names (the fixtures and the app's settings files use them).
- **Comments** say why, not what; a named value or a short doc comment beats a long explanation. Plain words, UK spelling.
- **The pages** (`ui/`) are ES modules with no build step and no globals: `ui/lib/` for what both windows share (the bridge, formatting, pal pieces, the clipboard), one file per tab in `ui/tabs/` exporting `render(app)`, `ui/main.js` and `ui/tray.js` as the entry points. Anything from palduels.com goes through `esc()` before it's shown. No `alert`/`confirm`/`prompt`: use `app.flash()`.
- **Names in the repo:** no real player names or anything naming the stream in fixtures, tests or docs. Saved pages use `player-NN` and `the-stream` placeholders (see the `site-probe` skill).

## Tests and checking your work

- `cargo test --workspace` runs everything from the seed (`augur_engine::seed`: `fixtures/calls.jsonl` and `engine/tests/pages/`). Tests never use the network or the local data folder. A new parser gets a saved page in `engine/tests/pages/`.
- **Never put anything on the developer's screen without asking.** They may be playing or streaming. Check pages with the headless UI tests (`app/src/ui_tests.rs`, headless Chrome) and look at screenshots from `AUGUR_UI_SHOTS=<folder> cargo test -p augur every_tab`. Don't run the app itself (`cargo run -p augur` without a command), `cargo tauri dev`, or a `.dmg` build (it opens Finder) unless the developer says it's a good time. The command line (`augur predict`, `analyse`, ...) is fine: it opens no windows.
- Changing a model setting changes the parity fixture's expected numbers: see the top of `engine/tests/parity.rs`.

## Git and GitHub

- The repo is `TrongatesLegacy/augur` (public). Commits use TrongatesLegacy with the noreply email (set in this repo's config and by `~/.gitconfig-trongates`); never another identity.
- `gh` here needs `GH_TOKEN="$(gh auth token --user TrongatesLegacy)"` (a hook enforces it); never `gh auth switch`.
- Changes go on a branch and end as a pull request; don't change the version or `CHANGELOG.md` in one. Releases are published from GitHub (see README > Contributing and releases).
