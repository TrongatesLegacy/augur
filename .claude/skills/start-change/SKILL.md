---
name: start-change
description: Start a change to Augur (bug fix, feature, issue) - plan it, create its branch in a separate git worktree, capture "before" screenshots headlessly, build and check it, then hand over to open-pr. Use when asked to fix, add or change something in this repo, or to work on an issue.
---

# Start a change

Every change goes on its own branch in its own worktree and ends as a pull request. Never commit to `main`.

## 1. What's the change?

- If the request already has a description or an issue (`#12`, an issue URL), use it. Don't ask again.
- For an issue: `GH_TOKEN="$(gh auth token --user TrongatesLegacy)" gh issue view <n> --comments`.
- **Mockups and screenshots in the issue are the design.** Download every image (`https://github.com/user-attachments/assets/<id>` links need the GitHub login: `curl -sL -H "Authorization: token $GH_TOKEN" -o <file> <url>`, into the temp folder, never the repo) and look at each one before planning.
- If nothing was given, ask once: "What's the change? A short description or an issue number/URL."

## 2. Plan (read-only, before any code changes)

Look at the relevant code (the layout table in `README.md`), then show a short plan and wait for approval:

- **Why:** the problem in one or two lines (plus `Fixes #<n>` if there's an issue).
- **Changes:** which files and how.
- **Testing:** what goes in `cargo test --workspace` (a parser change gets a saved page in `engine/tests/pages/`; a model change moves the parity fixture, see `engine/tests/parity.rs`; a page change gets a check in `app/src/ui_tests.rs`), and whether anything needs the real app (the tray, window placement, notifications, sign-in).
- **Before / after:** what to capture, or N/A (rules below).
- **Branch:** e.g. `fix/12-hidden-stats` or `feat/owner-ratings`. Pick it yourself; don't ask.

For large or risky changes, use plan mode.

**Before / after rules:**
- **Pages** (`ui/`): screenshots from the headless UI tests: `AUGUR_UI_SHOTS=<folder> cargo test -p augur every_tab` saves every tab at 1280×720, rendered from the seed. Nothing appears on screen.
- **The tray, windows, notifications:** these need the real app on screen. Only with the developer's OK (the `run-app` skill); otherwise say they weren't captured.
- **N/A** when nothing visible changes (engine, model, CI, docs).

## 3. Create the worktree

```sh
git fetch origin
git worktree add .worktrees/<branch> -b <branch> origin/main
```

`.worktrees/` is ignored by git. Work only in that folder from now on. Its `target/` starts empty, so the first build takes a few minutes.

## 4. Capture "before"

The new worktree is still identical to `main`, so capture "before" now, before editing anything, into the temp folder (never the repo).

## 5. Build and check

- Make the change, following *Code conventions* in `CLAUDE.md`.
- Add or update tests next to similar ones: for every new code path, from the seed (never the network or the local data folder).
- Run `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`.
- Capture "after" with exactly the same steps as "before". If the issue has mockups, compare them side by side and fix what doesn't match.
- Commit in small, clear commits. Don't change the version or `CHANGELOG.md`.

## 6. Hand over

When the developer is happy, follow the `open-pr` skill with the plan's Why, Testing, the issue number and the captures.
