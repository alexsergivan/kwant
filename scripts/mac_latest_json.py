#!/usr/bin/env python3
"""Writes latest.json, the feed the desktop app's updater reads, for one release.

The app asks https://github.com/alexsergivan/kwant/releases/latest/download/latest.json, so only
the newest non-prerelease release's file counts. The release notes are that version's CHANGELOG
section ("## [Mac 0.1.2] - 2026-10-27"); a release without one fails here, before anything is
published. Both Mac architectures point at the same universal app.

The Linux app (AppImage and .deb) ships in the same release, so the one latest.json serves both.
Its entries are optional and only ever added after the Mac ones: the updater looks up
`{os}-{arch}-{installer}` and then `{os}-{arch}`, so a Mac reads `darwin-*` and never sees them,
and there is no bare `linux-x86_64` entry that an AppImage and a .deb would both match.

    python3 scripts/mac_latest_json.py --version 0.1.2 --signature kwant.app.tar.gz.sig \\
        --url https://github.com/alexsergivan/kwant/releases/download/mac-v0.1.2/kwant_universal.app.tar.gz \\
        > latest.json

    # the same, plus the Linux packages
    python3 scripts/mac_latest_json.py ... \\
        --linux linux-x86_64-appimage kwant_x86_64.AppImage.sig https://.../kwant_x86_64.AppImage \\
        --linux linux-x86_64-deb kwant_amd64.deb.sig https://.../kwant_amd64.deb > latest.json
"""

import argparse
import datetime
import json
import re
import sys

PLATFORMS = ("darwin-aarch64", "darwin-x86_64")
# What the Linux app looks up first (tauri-plugin-updater: `{os}-{arch}-{installer}`)
LINUX_PLATFORMS = ("linux-x86_64-appimage", "linux-x86_64-deb")
VERSION = re.compile(r"^\d+\.\d+\.\d+$")


def changelog_section(changelog: str, version: str) -> str:
    """The body of `## [Mac <version>]`, up to the next `## ` heading, trimmed."""
    heading = re.compile(r"^## \[Mac " + re.escape(version) + r"\](?:\s|$)")
    lines = changelog.splitlines()
    for i, line in enumerate(lines):
        if heading.match(line):
            body = []
            for rest in lines[i + 1:]:
                if rest.startswith("## "):
                    break
                body.append(rest)
            return "\n".join(body).strip()
    raise ValueError(f"CHANGELOG.md has no '## [Mac {version}]' section")


def release_notes(changelog: str, version: str) -> str:
    """The update notes for a full release: its CHANGELOG section, which must say something
    besides `### ` headings."""
    notes = changelog_section(changelog, version)
    if not any(line.strip() and not line.startswith("#") for line in notes.splitlines()):
        raise ValueError(f"the CHANGELOG section for {version} is empty")
    return notes


def _entry(signature: str, url: str) -> dict:
    signature = signature.strip()
    if not signature:
        raise ValueError("the signature is empty")
    if not url.startswith("https://"):
        raise ValueError("the download URL must be https")
    return {"signature": signature, "url": url}


def latest_json(version: str, signature: str, url: str, notes: str, pub_date: str,
                linux=None) -> dict:
    """`linux` maps a LINUX_PLATFORMS key to its (signature, url); its entries follow the Mac ones."""
    if not VERSION.match(version):
        raise ValueError(f"'{version}' is not a release version (x.y.z); prereleases get no latest.json")
    signature = signature.strip()
    if not signature:
        raise ValueError("the signature is empty")
    if not url.startswith("https://"):
        raise ValueError("the download URL must be https")
    if not notes:
        raise ValueError(f"the CHANGELOG section for {version} is empty")
    entry = {"signature": signature, "url": url}
    platforms = {p: dict(entry) for p in PLATFORMS}
    for key, (sig, link) in (linux or {}).items():
        if key not in LINUX_PLATFORMS:
            raise ValueError(f"'{key}' is not a Linux platform ({', '.join(LINUX_PLATFORMS)})")
        platforms[key] = _entry(sig, link)
    return {
        "version": version,
        "notes": notes,
        "pub_date": pub_date,
        "platforms": platforms,
    }


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--version", required=True)
    ap.add_argument("--check-notes", action="store_true",
                    help="only check that the CHANGELOG section exists and isn't empty")
    ap.add_argument("--signature", help="the .sig file the Tauri build wrote")
    ap.add_argument("--url", help="where the release serves the .app.tar.gz")
    ap.add_argument("--linux", nargs=3, action="append", default=[], metavar=("PLATFORM", "SIG", "URL"),
                    help=f"add a Linux package: one of {', '.join(LINUX_PLATFORMS)}, its .sig file and URL")
    ap.add_argument("--changelog", default="CHANGELOG.md")
    ap.add_argument("--pub-date", help="RFC 3339; default now (UTC)")
    args = ap.parse_args(argv)
    if not args.check_notes and not (args.signature and args.url):
        ap.error("--signature and --url are required unless --check-notes")
    pub_date = args.pub_date or datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    try:
        with open(args.changelog, encoding="utf-8") as f:
            notes = release_notes(f.read(), args.version)
        if args.check_notes:
            return 0
        with open(args.signature, encoding="utf-8") as f:
            signature = f.read()
        linux = {}
        for key, sig_file, link in args.linux:
            if key in linux:
                raise ValueError(f"{key} is given twice")
            with open(sig_file, encoding="utf-8") as f:
                linux[key] = (f.read(), link)
        doc = latest_json(args.version, signature, args.url, notes, pub_date, linux)
    except (OSError, ValueError) as e:
        print(f"mac_latest_json: {e}", file=sys.stderr)
        return 1
    json.dump(doc, sys.stdout, indent=2, ensure_ascii=False)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
