#!/usr/bin/env python3
"""Run by the release workflow when a release is published.

Sets the app's version from the release tag (Cargo.toml's workspace version and app/tauri.conf.json) and adds the
release notes to CHANGELOG.md, so the installers match the release. Pull requests never touch the version.

    python3 scripts/prepare_release.py v0.2.0 notes.md
"""
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent


def bullets(notes):
    """GitHub's "Generate release notes" lines ("* Add X by @me in https://...") become "- Add X"."""
    out, skip = [], False
    for raw in notes.replace("\r", "").split("\n"):
        line = raw.strip()
        if re.match(r"^#+\s", line):
            skip = bool(re.search(r"new contributors", line, re.I))
            continue
        if skip or not line or re.match(r"^\*\*full changelog\*\*", line, re.I):
            continue
        m = re.match(r"^[-*]\s+(.*)$", line)
        if not m:
            continue
        item = re.sub(r"\s+by\s+@[\w-]+(\[bot\])?\s+in\s+https?://\S+$", "", m.group(1), flags=re.I)
        item = re.sub(r"\s+in\s+https?://\S+$", "", item, flags=re.I).strip()
        if item:
            out.append("- " + item)
    if not out:
        plain = [s.strip() for s in notes.replace("\r", "").split("\n") if s.strip() and not s.strip().startswith("#")]
        out = ["- " + re.sub(r"^[-*]\s+", "", s) for s in plain] or ["- Small fixes and improvements."]
    return out


def main():
    tag = sys.argv[1] if len(sys.argv) > 1 else ""
    version = re.sub(r"^v", "", tag, flags=re.I)
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        sys.exit(f'The release tag "{tag}" isn\'t a version like v0.2.0. Use a tag of that form.')
    notes_file = pathlib.Path(sys.argv[2]) if len(sys.argv) > 2 else None
    notes = notes_file.read_text() if notes_file and notes_file.exists() else ""

    cargo = ROOT / "Cargo.toml"
    text = cargo.read_text()
    text, n = re.subn(r'(\[workspace\.package\][^\[]*?\nversion\s*=\s*)"[^"]*"', rf'\g<1>"{version}"', text, count=1, flags=re.S)
    if not n:
        sys.exit("Couldn't find [workspace.package] version in Cargo.toml")
    cargo.write_text(text)

    conf = ROOT / "app" / "tauri.conf.json"
    data = json.loads(conf.read_text())
    data["version"] = version
    conf.write_text(json.dumps(data, indent=2) + "\n")

    cl = ROOT / "CHANGELOG.md"
    lines = [re.sub(r"\s*\(current\)\s*$", "", l, flags=re.I) if l.startswith("## ") else l for l in cl.read_text().split("\n")]
    head = re.compile(rf"^##\s+v?{re.escape(version)}(\s|$)")
    at = next((i for i, l in enumerate(lines) if head.match(l)), -1)
    new = bullets(notes)
    if at >= 0:
        lines[at] = lines[at].rstrip() + " (current)"  # already written by hand: keep it
    else:
        first = next((i for i, l in enumerate(lines) if l.startswith("## ")), len(lines))
        lines[first:first] = [f"## {version} (current)", *new, ""]
    cl.write_text("\n".join(lines))
    print(f"Version {version}; What's new:\n" + "\n".join(["(kept the existing entry)"] if at >= 0 else new))


if __name__ == "__main__":
    main()
