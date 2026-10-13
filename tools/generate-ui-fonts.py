#!/usr/bin/env python3
"""Rasterize the deck's Noto Sans type ramp into 4-bit glyph atlases.

    tools/generate-ui-fonts.py           writes assets/fonts/gen/*.bin and src/ui/font_data.rs
    tools/generate-ui-fonts.py --check   regenerates into a temp dir and diffs against the tree

Requires Pillow (FreeType) and fontTools. The firmware ships alpha bitmaps and
needs no font engine. RAMP is THE size table: `src/ui/theme.rs` (`RAMP`) must
agree with it, and a unit test checks the generated `px` against it.

Atlas format (`assets/fonts/gen/<font>.bin`): every glyph's bitmap in turn, rows
top to bottom, each row `(w + 1) // 2` bytes of packed 4-bit coverage (first
pixel in the high nibble, 15 = opaque). `font_data.rs` holds the sorted
codepoint table with advance / bearing / size / offset per glyph.

Glyphs Noto Sans lacks (the arrows, the check mark, the return symbols) and the
replacement box are drawn here from lines; see assets/fonts/gen/README.md.
"""
import argparse
import difflib
import filecmp
import pathlib
import sys
import tempfile

from PIL import Image, ImageDraw, ImageFont
from fontTools.ttLib import TTFont

ROOT = pathlib.Path(__file__).resolve().parents[1]
TTF = ROOT / "assets/fonts"
GEN = ROOT / "assets/fonts/gen"
RS = ROOT / "src/ui/font_data.rs"

# (Font, weight, em px) in `theme::Font` order.
RAMP = [
    ("Display", "Bold", 48),
    ("Title", "SemiBold", 32),
    ("Name", "SemiBold", 27),
    ("Macro", "SemiBold", 21),
    ("Text", "Regular", 29),
    ("Row", "Regular", 25),
    ("Body", "Regular", 22),
    ("Small", "Regular", 18),
    ("Label", "Bold", 18),
    ("Hint", "Regular", 15),
    ("Keycap", "SemiBold", 13),
    ("RowStrong", "SemiBold", 25),
    ("TextStrong", "SemiBold", 29),
    ("Reading", "Regular", 32),
    ("Code", "Regular", 25),
]

# ASCII, Latin-1 Supplement (incl. · ×), Latin Extended-A, then the deck's
# punctuation and symbols. U+FFFD is the replacement box (also drawn for every
# codepoint outside this table); it must be present in every font.
REPLACEMENT = 0xFFFD
COVERAGE = sorted(
    [*range(0x20, 0x7F), *range(0xA0, 0x180)]
    + [0x2013, 0x2014, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2026, 0x2039, 0x203A]
    + [0x2190, 0x2191, 0x2192, 0x2193, 0x21B5, 0x2713, 0x23CE, REPLACEMENT]
    + [0x2303, 0x2318, 0x2325, 0x21E7]  # ⌃ ⌘ ⌥ ⇧: the Mac modifier keys
)

# Baseline-relative line box the firmware lays text out with (em fractions);
# Noto's own 1.069 / 0.293 leaves too much air for the mockup rectangles.
ASCENT_EM = 0.80
LINE_HEIGHT_EM = 1.25

# Line width of the drawn glyphs, in em, per weight (a little under Noto's
# stems so they sit next to text without looking heavier).
STROKE_EM = {"Regular": 0.075, "SemiBold": 0.095, "Bold": 0.115}
SUPER = 8  # drawn glyphs are rendered at this scale and box-filtered down


def drawn_shape(cp, weight):
    """Lines (em units, y up from the baseline) and the advance of a drawn glyph."""
    if cp == REPLACEMENT:
        return None, 0.62
    s = STROKE_EM[weight]
    if cp == 0x2192:  # →
        return [[(0.08, 0.30), (0.78, 0.30)], [(0.50, 0.58), (0.78, 0.30), (0.50, 0.02)]], 0.86
    if cp == 0x2190:  # ←
        return [[(0.78, 0.30), (0.08, 0.30)], [(0.36, 0.58), (0.08, 0.30), (0.36, 0.02)]], 0.86
    if cp == 0x2191:  # ↑
        return [[(0.30, 0.00), (0.30, 0.70)], [(0.04, 0.44), (0.30, 0.70), (0.56, 0.44)]], 0.60
    if cp == 0x2193:  # ↓
        return [[(0.30, 0.70), (0.30, 0.00)], [(0.04, 0.26), (0.30, 0.00), (0.56, 0.26)]], 0.60
    if cp == 0x2713:  # ✓
        return [[(0.10, 0.34), (0.32, 0.08), (0.72, 0.64)]], 0.82
    if cp in (0x21B5, 0x23CE):  # ↵ ⏎ (the same return arrow)
        return [
            [(0.74, 0.66), (0.74, 0.26), (0.10, 0.26)],
            [(0.34, 0.50), (0.10, 0.26), (0.34, 0.02)],
        ], 0.86
    if cp == 0x2303:  # ⌃ (control: a chevron)
        return [[(0.10, 0.26), (0.38, 0.60), (0.66, 0.26)]], 0.78
    if cp == 0x2318:  # ⌘ (command: a square with a loop at every corner)
        return [
            [(0.24, 0.16), (0.56, 0.16), (0.56, 0.48), (0.24, 0.48), (0.24, 0.16)],
            [(0.24, 0.48), (0.24, 0.64), (0.08, 0.64), (0.08, 0.48), (0.24, 0.48)],
            [(0.56, 0.48), (0.56, 0.64), (0.72, 0.64), (0.72, 0.48), (0.56, 0.48)],
            [(0.24, 0.16), (0.24, 0.00), (0.08, 0.00), (0.08, 0.16), (0.24, 0.16)],
            [(0.56, 0.16), (0.56, 0.00), (0.72, 0.00), (0.72, 0.16), (0.56, 0.16)],
        ], 0.84
    if cp == 0x2325:  # ⌥ (option: a switch)
        return [[(0.06, 0.60), (0.28, 0.60), (0.54, 0.06), (0.76, 0.06)], [(0.52, 0.60), (0.76, 0.60)]], 0.84
    if cp == 0x21E7:  # ⇧ (shift: an outlined up arrow)
        return [[(0.20, 0.00), (0.44, 0.00), (0.44, 0.32), (0.62, 0.32), (0.32, 0.66), (0.02, 0.32), (0.20, 0.32), (0.20, 0.00)]], 0.70
    raise KeyError(hex(cp))


def render_drawn(cp, weight, px):
    """Rasterize a drawn glyph: (mask image at 1x, left, top, advance)."""
    lines, advance_em = drawn_shape(cp, weight)
    size = px * 2 * SUPER
    canvas = Image.new("L", (size, size), 0)
    draw = ImageDraw.Draw(canvas)
    ox, oy = px * SUPER // 2, px * SUPER * 3 // 2  # origin, y down
    scale = px * SUPER
    width = max(SUPER, round(STROKE_EM[weight] * scale))
    if lines is None:
        # Hollow box, ~cap height, one pixel of line at every size.
        width = max(SUPER, round(0.06 * scale))
        x0, y0 = ox + round(0.09 * scale), oy - round(0.70 * scale)
        x1, y1 = ox + round(0.53 * scale), oy
        draw.rectangle([x0, y0, x1 - 1, y1 - 1], outline=255, width=width)
    else:
        for line in lines:
            points = [(ox + x * scale, oy - y * scale) for x, y in line]
            draw.line(points, fill=255, width=width, joint="curve")
            for x, y in points:  # round the line ends
                r = width / 2 - 0.5
                draw.ellipse([x - r, y - r, x + r, y + r], fill=255)
    small = canvas.resize((px * 2, px * 2), Image.Resampling.BOX)
    box = small.getbbox()
    if box is None:
        return Image.new("L", (0, 0)), 0, 0, round(advance_em * px)
    left, top, right, bottom = box
    return small.crop(box), left - px // 2, (px * 3 // 2) - top, round(advance_em * px)


def render_truetype(font, ch, px):
    """Rasterize one FreeType glyph: (mask image, left, top, advance)."""
    size = px * 4
    canvas = Image.new("L", (size, size), 0)
    ox, oy = px, px * 2
    ImageDraw.Draw(canvas).text((ox, oy), ch, font=font, fill=255, anchor="ls")
    advance = round(font.getlength(ch))
    box = canvas.getbbox()
    if box is None:
        return Image.new("L", (0, 0)), 0, 0, advance
    left, top, right, bottom = box
    assert left >= 0 and right <= size and top >= 0 and bottom <= size, ch
    return canvas.crop(box), left - ox, oy - top, advance


def pack(mask):
    """Rows of packed nibbles, first pixel in the high nibble."""
    w, h = mask.size
    data = mask.tobytes()
    out = bytearray()
    for y in range(h):
        row = data[y * w:(y + 1) * w]
        for x in range(0, w, 2):
            hi = (row[x] * 15 + 127) // 255
            lo = (row[x + 1] * 15 + 127) // 255 if x + 1 < w else 0
            out.append(hi << 4 | lo)
    return bytes(out)


def build_font(name, weight, px):
    ttf = TTF / ("NotoSansMono.ttf" if name == "Code" else f"NotoSans-{weight}.ttf")
    source = TTFont(str(ttf))
    cmap = source.getBestCmap()
    font = ImageFont.truetype(str(ttf), px, layout_engine=ImageFont.Layout.BASIC)
    if "fvar" in source:
        font.set_variation_by_axes([400 if axis.axisTag == "wght" else axis.defaultValue for axis in source["fvar"].axes])
    fallback = ImageFont.truetype(str(TTF / f"NotoSans-{weight}.ttf"), px, layout_engine=ImageFont.Layout.BASIC)
    fallback_cmap = TTFont(str(TTF / f"NotoSans-{weight}.ttf")).getBestCmap()
    atlas = bytearray()
    glyphs = []
    drawn = []
    for cp in COVERAGE:
        if cp in cmap and cp != REPLACEMENT:
            mask, left, top, advance = render_truetype(font, chr(cp), px)
        elif cp in fallback_cmap and cp != REPLACEMENT:
            mask, left, top, advance = render_truetype(fallback, chr(cp), px)
        else:
            mask, left, top, advance = render_drawn(cp, weight, px)
            drawn.append(cp)
        w, h = mask.size
        assert 0 <= w < 128 and 0 <= h < 128 and 0 <= advance < 256, (name, cp)
        assert -128 <= left < 128 and -128 <= top < 128, (name, cp)
        glyphs.append((cp, advance, left, top, w, h, len(atlas)))
        atlas += pack(mask)
    above = max((g[3] for g in glyphs), default=0)
    below = max((g[5] - g[3] for g in glyphs), default=0)
    metrics = {
        "px": px,
        "ascent": round(ASCENT_EM * px),
        "line_height": round(LINE_HEIGHT_EM * px),
        "above": max(above, 0),
        "below": max(below, 0),
    }
    return glyphs, bytes(atlas), metrics, drawn


def rust_source(fonts):
    out = []
    out.append("// GENERATED by tools/generate-ui-fonts.py from assets/fonts/NotoSans-*.ttf; do not edit.")
    out.append("//! Glyph metrics of the deck type ramp (`theme::RAMP` order). The 4 bpp")
    out.append("//! bitmaps live in `assets/fonts/gen/<font>.bin`: one glyph after another,")
    out.append("//! rows top to bottom, `(w + 1) / 2` bytes per row, first pixel in the")
    out.append("//! high nibble, 15 = opaque. Codepoints are sorted for binary search; ASCII")
    out.append("//! 0x20..=0x7E are the first 95 entries in order.")
    out.append("")
    out.append("#[derive(Clone, Copy, PartialEq, Eq, Debug)]")
    out.append("#[repr(C)]")
    out.append("pub struct Glyph {")
    out.append("    /// Byte offset of the bitmap in the atlas.")
    out.append("    pub offset: u32,")
    out.append("    pub cp: u16,")
    out.append("    pub advance: u8,")
    out.append("    /// Bitmap left edge relative to the pen.")
    out.append("    pub left: i8,")
    out.append("    /// Rows the bitmap top lies above the baseline.")
    out.append("    pub top: i8,")
    out.append("    pub w: u8,")
    out.append("    pub h: u8,")
    out.append("}")
    out.append("")
    out.append("pub struct FontData {")
    out.append("    pub glyphs: &'static [Glyph],")
    out.append("    pub atlas: &'static [u8],")
    out.append("    /// Em size.")
    out.append("    pub px: u8,")
    out.append("    /// Baseline down from the top of a line box.")
    out.append("    pub ascent: u8,")
    out.append("    /// Baseline to baseline.")
    out.append("    pub line_height: u8,")
    out.append("    /// Furthest any bitmap reaches above / below the baseline.")
    out.append("    pub above: u8,")
    out.append("    pub below: u8,")
    out.append("}")
    out.append("")
    out.append(f"pub const REPLACEMENT: u16 = 0x{REPLACEMENT:04X};")
    out.append(f"pub const FONT_COUNT: usize = {len(fonts)};")
    out.append("")
    out.append("pub static FONTS: [FontData; FONT_COUNT] = [")
    for name, _weight, _px, glyphs, atlas, metrics in fonts:
        out.append("    FontData {")
        out.append(f"        glyphs: &{name.upper()},")
        out.append(f'        atlas: include_bytes!("../../assets/fonts/gen/{name.lower()}.bin"),')
        for key in ("px", "ascent", "line_height", "above", "below"):
            out.append(f"        {key}: {metrics[key]},")
        out.append("    },")
    out.append("];")
    for name, weight, px, glyphs, atlas, metrics in fonts:
        out.append("")
        out.append(f"// {name}: Noto Sans {weight} {px} px, {len(glyphs)} glyphs, {len(atlas)} atlas bytes")
        out.append(f"static {name.upper()}: [Glyph; {len(glyphs)}] = [")
        for cp, advance, left, top, w, h, offset in glyphs:
            out.append(
                f"    Glyph {{ offset: {offset}, cp: 0x{cp:04X}, advance: {advance}, "
                f"left: {left}, top: {top}, w: {w}, h: {h} }},"
            )
        out.append("];")
    return "\n".join(out) + "\n"


def generate(gen_dir, rs_path):
    fonts = []
    total = 0
    drawn = None
    for name, weight, px in RAMP:
        glyphs, atlas, metrics, drawn = build_font(name, weight, px)
        fonts.append((name, weight, px, glyphs, atlas, metrics))
        (gen_dir / f"{name.lower()}.bin").write_bytes(atlas)
        total += len(atlas)
    rs_path.write_text(rust_source(fonts))
    table = len(RAMP) * len(COVERAGE) * 12
    print(
        f"Generated {len(RAMP)} fonts x {len(COVERAGE)} glyphs: {total} atlas bytes"
        f" + ~{table} bytes of metrics; drawn: {' '.join(chr(cp) for cp in drawn)}"
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--check", action="store_true", help="regenerate to a temp dir and diff")
    args = parser.parse_args()
    if not args.check:
        GEN.mkdir(parents=True, exist_ok=True)
        generate(GEN, RS)
        return
    with tempfile.TemporaryDirectory() as tmp:
        tmp = pathlib.Path(tmp)
        (tmp / "gen").mkdir()
        generate(tmp / "gen", tmp / "font_data.rs")
        stale = []
        for name, _, _ in RAMP:
            new = tmp / "gen" / f"{name.lower()}.bin"
            if not filecmp.cmp(new, GEN / new.name, shallow=False):
                stale.append(new.name)
        if RS.read_text() != (tmp / "font_data.rs").read_text():
            stale.append(RS.name)
            sys.stdout.writelines(difflib.unified_diff(
                RS.read_text().splitlines(True),
                (tmp / "font_data.rs").read_text().splitlines(True),
                str(RS), "regenerated", n=1,
            ))
    if stale:
        print("stale: " + " ".join(stale) + " (run tools/generate-ui-fonts.py)")
        sys.exit(1)
    print("font atlases and metrics are up to date")


if __name__ == "__main__":
    main()
