#!/usr/bin/env python3
"""Prints the release notes for one version, taken from CHANGELOG.md.

    python3 scripts/release_notes.py v0.2.0
    python3 scripts/release_notes.py v0.2.0 --changes   (only its list of changes: the notes in latest.json)
"""
import pathlib
import re
import sys

want = re.sub(r"^v", "", sys.argv[1] if len(sys.argv) > 1 else "", flags=re.I).strip()
lines = (pathlib.Path(__file__).resolve().parent.parent / "CHANGELOG.md").read_text().split("\n")
body, on = [], False
for line in lines:
    m = re.match(r"^##\s+v?([0-9][0-9.]*)", line)
    if m:
        if on:
            break
        on = m.group(1) == want
        continue
    if on:
        body.append(line)
changes = "\n".join(body).strip() or "- See CHANGELOG.md for details."
if "--changes" in sys.argv:
    print(changes)
    sys.exit()
print(f"""### What's new in {want}
{changes}

### Download
- **Mac:** `Augur_{want}_universal.dmg`
- **Windows:** `Augur_{want}_x64-setup.exe`

First time installing? On a Mac, if it says the app can't be checked, open **System Settings → Privacy & Security** and click **Open Anyway**. Windows may say "Windows protected your PC": click **More info → Run anyway**. Augur is signed with its own certificate, not Apple's or Microsoft's.

Full history: `CHANGELOG.md`""")
