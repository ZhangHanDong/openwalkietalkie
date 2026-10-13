#!/usr/bin/env python3
"""Rasterize the vendored Lucide font glyphs; no hand-traced icon paths.

Requires Pillow. The firmware includes alpha masks, so it needs no font engine.
The TTF subset, glyph names/codepoints, provenance and license live together.
"""
import json
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "assets/lucide"
SIZE, SCALE = 40, 4

def main():
    font = ImageFont.truetype(str(ASSETS / "lucide-ui.ttf"), 36 * SCALE)
    glyphs = json.loads((ASSETS / "glyphs.json").read_text())["glyphs"]
    for glyph in glyphs:
        if glyph["codepoint"] is None:
            (ASSETS / (glyph["name"] + ".alpha")).write_bytes(bytes(SIZE * SIZE))
            continue
        char = chr(glyph["codepoint"])
        left, top, right, bottom = font.getbbox(char)
        image = Image.new("L", (SIZE * SCALE, SIZE * SCALE))
        origin = ((SIZE * SCALE - right - left) / 2, (SIZE * SCALE - bottom - top) / 2)
        ImageDraw.Draw(image).text(origin, char, font=font, fill=255)
        if glyph["rotation"]:
            image = image.rotate(glyph["rotation"], resample=Image.Resampling.BICUBIC)
        image = image.resize((SIZE, SIZE), Image.Resampling.LANCZOS)
        (ASSETS / (glyph["name"] + ".alpha")).write_bytes(image.tobytes())
    print(f"Generated {len(glyphs)} Lucide glyphs ({SIZE}×{SIZE} alpha masks)")

if __name__ == "__main__":
    main()
