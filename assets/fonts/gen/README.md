# Generated glyph atlases

`<font>.bin` are the 4 bpp glyph atlases of the deck type ramp, one per
`theme::Font`, with their metrics in `src/ui/font_data.rs`. Both are written by
`tools/generate-ui-fonts.py` from the vendored Noto Sans static TTFs in the
parent directory (`NotoSans-{Regular,SemiBold,Bold}.ttf`, SIL Open Font License
1.1, see `../OFL.txt`; Noto Sans 2.x from fonts.google.com). The generator's
`RAMP` table is the one size table; `theme::RAMP` must agree with it.

Regenerate after changing the ramp, the coverage or the TTFs:

    tools/generate-ui-fonts.py            # rewrites gen/*.bin and src/ui/font_data.rs
    tools/generate-ui-fonts.py --check    # regenerates to a temp dir and diffs

Needs Pillow (FreeType rendering, hinted, at the em size) and fontTools (to
read the cmap). Output depends on the installed FreeType version; commit the
regenerated files together with the change that needed them.

The rich message faces add RowStrong (25 px), TextStrong (29 px), Reading
(32 px regular), and Code (25 px Noto Sans Mono). The variable
`../NotoSansMono.ttf` is rasterized at regular weight and normal width; its
license is `../NotoSansMono-OFL.txt`. Missing glyphs fall back to Noto Sans
before the deck’s drawn symbols.

Coverage: ASCII, Latin-1 Supplement, Latin Extended-A, – — ‘ ’ “ ” • … ‹ › and
the symbols ← ↑ → ↓ ↵ ⏎ ✓ and the Mac modifiers ⌃ ⌥ ⇧ ⌘, plus U+FFFD. Noto
Sans has no arrows, check mark, return or modifier symbols, so those eleven are
drawn by the generator from lines in the weight's stroke width (`drawn_shape`). U+FFFD is drawn as a hollow box and is
what every codepoint outside the table renders as.
