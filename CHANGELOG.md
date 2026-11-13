# What's new in Augur

Augur is in beta, so versions start with 0. Every update raises the last number (0.1.0 → 0.1.1 → 0.1.2 …). The middle number only goes up for a big milestone. Version 1.0 will be the first finished release.

## 0.1.10 (current)
- The main window fills large screens on Windows too

## 0.1.9
- Records on real calls learns from every call, a little at a time
- Installer builds reuse their compiled crates again

## 0.1.8
- Species records get their own weight on real calls, learned once there are 60

## 0.1.7
- Update notes come from the changelog built into the app
- The window reopens at its last size, and nothing bounces when scrolled
- A damaged mark syncs straight away, not at the next result
- The window opens at launch, with a Start in the tray setting
- Installers build about 3x faster (thin LTO)
- Every view fits the window: nothing scrolls but History's lists

## 0.1.6
- The damaged mark syncs, so every computer adds up the money the same

## 0.1.5
- Score the call that was showing when picks closed
- Hidden stats come from the owner's pal box instead of a guess
- Titles: just the closest to unlock, with room for your badges
- Results by algorithm version, and no calls once a fight is over
- A Titles view on You: every title by category
- Moves can count by how hard they really hit (a learnable setting, off to start)
- See how each called duel went and how Augur called it
- Titles: category tabs, one category at a time
- Next up is the fight about to start, even before its pals load
- Calls overwritten after the fight stay out of the money comparison
- Live: built from the duel breakdown's pieces, with Augur's projected race in the breakdown
- Duel breakdown: what swung the call, in one sentence
- Live: a clearer head to head and an HP race whose labels never overlap
- Fix main's build (the live call detail sets damaged)
- CI: Quality on Linux, Clippy in the Windows tests

## 0.1.4
- Windows: sign-in loads and closes, and the tray popover opens above the taskbar
- Tray: one next duel, with faces and long names trimmed cleanly
- Tray: the next duel framed, centred and evenly spaced
- Calls keep coming when the queue has a slot waiting for an opponent
- Log a call the app missed: augur predict --state FILE --save

## 0.1.3
- Kick profile pictures, your real pet and titles, and tidier pal cards
- The model tunes itself from every result, with a switch to set it by hand
- Big numbers fit everywhere
- Learn straight after each result

## 0.1.2
- Update Augur from GitHub releases with Restart to update
- Fix the menu bar icon, show Augur in the Dock, and a rounded popover with Quit
- A You tab while signed in: your balance, predictions, duels, spending and titles
- Your pet and Hall, staking that scales with the chance, and a visual overhaul

## 0.1.1
- Fix the Mac installers missing from releases
- Suggest a stake for each call and track your bets against Augur's
- Sync settings and calls across computers, with backup and import
- Check for new fights at random times within a range
- Don't credit Claude in commits or pull requests
- Offer to import a backup on first launch, and back off harder on rate limits

## 0.1.0
- The first version: calls each PalDuels fight in the tray, with the reasons, five chat voices and the copy button
- History, Pals, Players (pal box and reroll advice), Model and Settings in the main window
- Fetches past duels and fight tapes in the background, and never while a duel is up
- Saves a PalDuels sign-in encrypted: the Keychain on macOS, DPAPI on Windows
