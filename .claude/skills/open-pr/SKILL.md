---
name: open-pr
description: Open a pull request for an Augur change - run the checks, write the description from the PR template with before/after screenshots, show the draft for approval, then push and create it with gh. Use when a change is ready for review or the developer asks to open a PR.
---

# Open a pull request

`gh` here needs `GH_TOKEN="$(gh auth token --user TrongatesLegacy)"` in front of it.

## 1. Check the branch

- It must not be `main`. If the work is on `main`, move it to a new branch first.
- The commits must not change the version (`Cargo.toml`, `app/tauri.conf.json`) or `CHANGELOG.md`: releases do that.

## 2. Run the checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The UI tests run in headless Chrome: nothing appears on screen. Report failures honestly in the PR, with what was and wasn't run.

## 3. Write the draft

Follow `.github/pull_request_template.md` and the style of recent PRs (`gh pr list --state merged --limit 5 --json title,body`): plain words, a bold label per section, short bullets.

- **Title:** one line a user would understand. Titles become the release's "What's new".
- **Why**, **What changed**, **Tests**, **Before / after** (a table of screenshots, or N/A), **Notes** (drop if empty).
- A model change says which settings moved, on how many calls, and that they were scored on the same calls they were fitted to.

Save the body to a file in the temp folder (not the repo).

## 4. Show the draft and wait

Show the title, the body and the attachments. Point out anything private in the screenshots (real player names, balances). **Don't push until the developer approves.**

## 5. Push and create

```sh
git push -u origin <branch>
gh pr create --base main --title "<title>" --body-file <body.md> --attach <file> [--attach <file> ...]
```

Check every attachment became a GitHub URL (`gh pr view --json body`); fix with `gh pr edit --body-file ... --attach ...` if not.

## 6. Updating an open PR

When new commits change something visible, capture again with the same steps, update the body, show it, wait for approval, then `gh pr edit <n> --body-file <body.md> --attach <file>`.

## 7. Merging

When the developer wants it merged once the checks pass, turn on auto-merge: `gh pr merge <n> --auto --merge`. GitHub merges it when the required checks (Protect main) pass, and does nothing if one fails: then fix it on the branch and push.

## 8. After merging

Once `gh pr view <n> --json state` says `MERGED`, from the main checkout:

```sh
git worktree remove .worktrees/<branch> && git branch -d <branch>
git push origin --delete <branch>
```

Never delete the branch on GitHub before the merge: that closes the PR.
