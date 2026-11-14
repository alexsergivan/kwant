#!/usr/bin/env python3
"""Slims the site's web fonts in place: keeps only the weights site.css uses and Latin-1 plus the symbols the pages use.

    python3 scripts/slim_fonts.py        # needs fonttools and brotli (pip install fonttools brotli)

Layout features are fontTools' defaults plus tnum (Chrome draws list numbers with tabular figures).
Inter keeps wght 400-700, JetBrains Mono 400-600 and Pixelify Sans its full 400-700 range; the @font-face rules in
site.css say the same. Running it again on slimmed files changes nothing. On Nov 14 2026 it took the three files from
98 KB to 64 KB, which is most of what the phone home page loads before its first paint.
"""
import pathlib
import subprocess
import sys
import tempfile

from fontTools.ttLib import TTFont

FONTS = pathlib.Path(__file__).resolve().parent.parent / "site" / "assets" / "fonts"
WEIGHTS = {"inter-var.woff2": "400:700", "jetbrains-mono.woff2": "400:600", "pixelify-sans.woff2": None}
UNICODES = ("U+0020-007E,U+00A0-00FF,U+010C-010D,U+0160-0161,U+017D-017E,U+2013-2014,U+2018-201E,U+2022,U+2026,"
            "U+2039-203A,U+20AC,U+2122,U+2153,U+2190-2193,U+2212,U+2318,U+2605,U+2713,U+2717,U+2726")


def run(*args):
    subprocess.run([sys.executable, "-m", *args], check=True)


for name, wght in WEIGHTS.items():
    path = FONTS / name
    before = path.stat().st_size
    with tempfile.TemporaryDirectory() as tmp:
        src = path
        axis = next(a for a in TTFont(path)["fvar"].axes if a.axisTag == "wght")
        if wght and wght != f"{axis.minValue:g}:{axis.maxValue:g}":
            src = pathlib.Path(tmp) / "inst.ttf"
            run("fontTools.varLib.instancer", str(path), f"wght={wght}", "-q", "-o", str(src))
        run("fontTools.subset", str(src), f"--unicodes={UNICODES}", "--layout-features+=tnum", "--flavor=woff2", f"--output-file={path}")
    print(f"{name}: {before} -> {path.stat().st_size} bytes")
