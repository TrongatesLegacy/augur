---
name: run-app
description: Run the real Augur desktop app to check the parts the headless tests can't - the tray (menu bar title, Windows ring), the popover's placement, the windows, notifications and sign-in - against a throwaway copy of the data. Only when the developer says it's a good time.
---

# Run the app

**Ask first, every time.** The app puts an icon in the menu bar, can open windows and send notifications, and the developer may be playing or streaming. Pages are checked headlessly instead (`AUGUR_UI_SHOTS=<folder> cargo test -p augur every_tab`); this is only for the native parts.

```sh
D=$(mktemp -d)                                                              # a throwaway data folder
cp "$HOME/Library/Application Support/Augur/augur.db" "$D/augur.db" 2>/dev/null  # optional: real history, read-only copy
echo '{}' > "$D/settings.json"                                              # skips the first-launch window
AUGUR_DATA_DIR="$D" cargo run -p augur
```

- Run it in the background and keep the output; it logs nothing on success.
- Look with a screenshot of just the part you need (`screencapture -x -R x,y,w,h <file>`), not the whole screen: other windows may be private.
- **Stopping:** Quit Augur from its tray menu, or stop that one process (`kill <pid>` of the `target/debug/augur` you started). Never stop processes by name: an installed Augur has the same name.
- Delete the throwaway folder afterwards.
- Never build a `.dmg` locally: it opens Finder. `cargo tauri build --bundles app` builds just the app.
