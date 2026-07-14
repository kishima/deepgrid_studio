#!/usr/bin/env python3
"""Regenerate the sample project's party portraits (CLAUDE.md 素材の記録ルール).

Downloads five public-domain engravings from Wikimedia Commons, crops each to a
square bust and writes 256x256 grayscale PNGs into
assets/projects/sample/portraits/. Sources and licenses are recorded in
CREDITS.md. Requires: python3 + Pillow, network access.

Usage: python3 scripts/fetch_portraits.py
"""

import json
import pathlib
import time
import urllib.parse
import urllib.request

from PIL import Image, ImageOps

UA = {"User-Agent": "DeepGridStudio-asset-fetch/1.0 (personal game project)"}
API = (
    "https://commons.wikimedia.org/w/api.php?action=query&prop=imageinfo"
    "&iiprop=url&iiurlwidth=1400&format=json&titles="
)
OUT = pathlib.Path(__file__).resolve().parent.parent / "assets/projects/sample/portraits"

# (output name, Commons file title, crop: center-x/-y as W/H fractions, half-size as W fraction)
SOURCES = [
    ("knight", "File:Knight, Death and the Devil MET DP159049.jpg", 0.53, 0.36, 0.26),
    ("mage", "File:A Scholar in His Study ('Faust') MET DP814791.jpg", 0.37, 0.46, 0.21),
    ("priest", "File:125.Baruch.jpg", 0.53, 0.44, 0.27),
    ("rogue", "File:Gustave Doré - The Holy Bible - Plate CXLI, The Judas Kiss.jpg", 0.71, 0.46, 0.17),
    ("barbarian", "File:060.Samson Slays a Lion.jpg", 0.68, 0.66, 0.25),
]


def fetch(url: str) -> bytes:
    for attempt in range(5):
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers=UA)) as r:
                return r.read()
        except Exception as e:  # 429s happen; back off and retry
            print(f"  retry {attempt}: {e}")
            time.sleep(8)
    raise RuntimeError(f"failed to fetch {url}")


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, title, fx, fy, fh in SOURCES:
        info = json.loads(fetch(API + urllib.parse.quote(title)))
        page = next(iter(info["query"]["pages"].values()))
        thumb = page["imageinfo"][0]["thumburl"]
        print(name, "<-", title)
        raw = fetch(thumb)
        tmp = OUT / f".{name}_src.jpg"
        tmp.write_bytes(raw)
        im = Image.open(tmp).convert("L")
        w, h = im.size
        cx, cy, half = fx * w, fy * h, fh * w
        box = (
            max(int(cx - half), 0),
            max(int(cy - half), 0),
            min(int(cx + half), w),
            min(int(cy + half), h),
        )
        crop = ImageOps.autocontrast(im.crop(box), cutoff=1)
        crop = crop.resize((256, 256), Image.LANCZOS)
        crop.convert("RGB").save(OUT / f"{name}.png")
        tmp.unlink()
        time.sleep(4)  # be polite to Commons
    print("done ->", OUT)


if __name__ == "__main__":
    main()
