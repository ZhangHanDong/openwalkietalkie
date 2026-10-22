#!/usr/bin/env python3
"""Export a shareable pack of the deck's real firmware UI with demo data.

    python3 tools/marketing-screens.py [--out DIR] [--source-dir DIR]

Without --source-dir, builds and runs the existing Rev A UI preview renderer.
Requires Pillow. No service connection, live account data, or device changes.
"""
import argparse
import datetime
import hashlib
import html
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import zipfile

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent.parent
# The first twelve are the recommended starting set for product marketing.
SCENES = [
    ("roster", "Agent overview", "Follow multiple agents and their current work."),
    ("ptt-roster", "Push to talk", "Dictate an instruction to the selected agent."),
    ("card-question", "Answer an agent", "Review a question and select a response."),
    ("card-ready", "Work completed", "See a completed task and choose a next step."),
    ("channel-rich-chat", "Conversation", "Read formatted replies and code on the deck."),
    ("workflow", "Workflow progress", "Follow plan steps and recent activity."),
    ("usage-accounts", "Usage across accounts", "View session and weekly subscription usage."),
    ("macropad", "Custom shortcuts", "Run configured shortcuts from a touch grid."),
    ("omarchy", "Omarchy workspaces", "Browse workspaces and windows; requires an Omarchy host."),
    ("manager", "Manage faces", "Switch and arrange the deck's faces."),
    ("screen-locked", "Screen lock", "Unlock with the wheel before interacting."),
    ("arcade-raiden", "Raiden", "A pocket arcade game between tasks."),
    ("channel-tools", "Tool activity", "Follow an agent's tool activity in the conversation."),
    ("channel-tool-open", "Tool details", "Expand tool output, including formatted code."),
    ("workflow-complete", "Workflow completed", "Review a completed plan and recent activity."),
    ("macropad-edit", "Edit a shortcut", "Set a shortcut's label, modifiers, and key."),
    ("gallery", "Add a face", "Choose from the available face templates."),
    ("drawer", "Notifications", "Browse questions and completion notices."),
    ("settings-volume", "Audio settings", "Adjust the speaker volume and microphone gain."),
    ("settings-display", "Display settings", "Adjust brightness, idle time, and text size."),
    ("arcade", "Arcade library", "Choose between Flappy Bird, Snake, and Raiden."),
    ("arcade-snake", "Snake", "Control the classic game with the deck's inputs."),
    ("arcade-flappy", "Flappy Bird", "Use push to talk to fly."),
    ("splash", "Startup", "The OpenWalkieTalkie startup screen."),
    ("usage-reset-expiring", "Reset expiry reminder", "See when unused redeemable resets are approaching expiry."),
]
PREVIEWS = []


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, check=True,
                          capture_output=True, text=True).stdout.strip()


def contact_sheet(entries, out, name, heading, subheading):
    # Native-resolution screenshots: the sheet does not soften UI text.
    width, margin, gap, tile = 1600, 48, 32, 480
    rows = (len(entries) + 2) // 3
    sheet = Image.new("RGB", (width, 164 + rows * 560 + 24), "#eef0f3")
    draw = ImageDraw.Draw(sheet)
    regular = ROOT / "assets/fonts/NotoSans-Regular.ttf"
    bold = ROOT / "assets/fonts/NotoSans-SemiBold.ttf"
    draw.text((margin, 32), heading, font=ImageFont.truetype(str(bold), 34), fill="#171b24")
    draw.text((margin, 90), subheading, font=ImageFont.truetype(str(regular), 19), fill="#505b6b")
    for i, entry in enumerate(entries):
        x, y = margin + (i % 3) * (tile + gap), 152 + (i // 3) * 560
        with Image.open(out / entry["native"]) as im:
            sheet.paste(im, (x, y))
        draw.text((x, y + 492), f'{entry["number"]:02d}  {entry["title"]}',
                  font=ImageFont.truetype(str(bold), 22), fill="#171b24")
        draw.text((x, y + 526), entry["scene"],
                  font=ImageFont.truetype(str(regular), 17), fill="#505b6b")
    sheet.save(out / name)


def package(source, out):
    entries = []
    for number, (scene, title, caption) in enumerate(SCENES + PREVIEWS, 1):
        preview = number > len(SCENES)
        prefix = "feature-preview/" if preview else ""
        basename = f"{number:02d}-{scene}.png"
        native = f"{prefix}native-480/{basename}"
        large = f"{prefix}4x-1920/{basename}"
        for name in (native, large):
            (out / name).parent.mkdir(parents=True, exist_ok=True)
        with Image.open(source / f"{scene}.png") as image:
            if image.size != (480, 480):
                raise ValueError(f"{scene}: expected the native 480 x 480 framebuffer")
            image = image.convert("RGB")
            image.save(out / native)
            image.resize((1920, 1920), Image.Resampling.NEAREST).save(out / large)
        entries.append(dict(number=number, scene=scene, title=title, caption=caption,
                            preview=preview, recommended=number <= 12,
                            native=native, large=large,
                            sha256=hashlib.sha256((out / native).read_bytes()).hexdigest()))

    contact_sheet(entries[:12], out, "contact-sheet-01.png", "OpenWalkieTalkie / Selected screens",
                  "Actual firmware UI / Illustrative demo content / Native 480 x 480 screenshots")
    contact_sheet(entries[12:len(SCENES)], out, "contact-sheet-02.png", "OpenWalkieTalkie / More screens",
                  "Actual firmware UI / Illustrative demo content / Native 480 x 480 screenshots")
    if PREVIEWS:
        contact_sheet(entries[len(SCENES):], out, "feature-preview/contact-sheet.png", "Feature previews",
                      "Unreleased working-tree features / Illustrative demo content")
    version = re.search(r'^version = "([^"]+)"', (ROOT / "Cargo.toml").read_text(), re.MULTILINE)[1]
    metadata = dict(product="OpenWalkieTalkie", generated=datetime.datetime.now().astimezone().isoformat(),
                    source_commit=git("rev-parse", "HEAD"), source_tag=git("describe", "--tags", "--always"),
                    source_is_dirty=bool(git("status", "--porcelain")), board="hw-a",
                    renderer="native/core/examples/ui-preview.rs", content="synthetic demo fixtures",
                    native_size=[480, 480], large_size=[1920, 1920], scaling="4x nearest-neighbor",
                    screenshots=entries)
    (out / "manifest.json").write_text(json.dumps(metadata, indent=2) + "\n")
    (out / "READ-ME.txt").write_text(
        "OpenWalkieTalkie deck screenshot pack\n\n"
        "START HERE\nOpen index.html for the gallery, captions, and individual downloads.\n"
        "contact-sheet-01.png is the recommended starting set; contact-sheet-02.png has more screens.\n\n"
        f"FILES\nnative-480/: {len(SCENES)} native 480 x 480 PNGs from the actual firmware UI renderer.\n"
        f"4x-1920/: the same {len(SCENES)} screens enlarged to 1920 x 1920 with nearest-neighbor scaling.\n"
        "The large copies preserve pixels; they do not add detail.\n"
        f"Unreleased feature previews: {len(PREVIEWS)} (stored separately when present).\n"
        "manifest.json: captions, source revision, scene names, and native PNG checksums.\n\n"
        "CONTENT AND SOURCE\nAll text, account addresses, usage figures, timestamps, and agent activity are demo data.\n"
        "These are firmware-rendered UI screenshots, not photographs or captures of a live session.\n"
        f"Firmware source version: {version}; renders were made from the current working tree.\n"
        "See source_is_dirty in manifest.json before describing these as pristine release captures.\n"
        "Omarchy workspace controls require the corresponding host integration.\n"
        "All renders are upright, full frame, opaque RGB PNGs with no hardware frame or watermark.\n\n"
        "REGENERATE\nFrom the repository root: python3 tools/marketing-screens.py\n"
    )
    cards = []
    for entry in entries:
        e = {key: html.escape(str(value), quote=True) for key, value in entry.items()}
        badge = "Unreleased preview" if entry["preview"] else "Recommended" if entry["recommended"] else "Additional screen"
        cards.append(f'<article data-category="{"preview" if entry["preview"] else "recommended" if entry["recommended"] else "more"}">'
                     f'<a href="{e["native"]}"><img src="{e["native"]}" width="480" height="480" alt="{e["title"]}"></a>'
                     f'<span>{badge}</span><h2>{entry["number"]:02d} / {e["title"]}</h2><p>{e["caption"]}</p>'
                     f'<nav><a download href="{e["native"]}">PNG · 480 px</a><a download href="{e["large"]}">PNG · 1920 px</a></nav></article>')
    gallery = '''<!doctype html><html lang="en"><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>OpenWalkieTalkie screenshots</title>
<style>body{margin:0;background:#eef0f3;color:#171b24;font:16px/1.5 system-ui,sans-serif}main{max-width:1568px;margin:auto;padding:40px 28px}h1{font-size:38px;margin:0}header{max-width:880px;margin-bottom:32px}.filters{display:flex;gap:8px;flex-wrap:wrap;margin:24px 0}button{font:inherit;padding:8px 14px;border:1px solid #a7b0be;border-radius:20px;background:white;cursor:pointer}button[aria-pressed=true]{background:#171b24;color:white}section{display:grid;grid-template-columns:repeat(auto-fit,minmax(280px,1fr));gap:28px}article{min-width:0;padding:16px;background:white;border-radius:16px}article[hidden]{display:none}img{width:100%;height:auto;display:block;background:black}h2{font-size:19px;margin:8px 0}article p{min-height:48px;color:#4f5968;margin:8px 0 18px}article span{display:block;font-size:12px;text-transform:uppercase;letter-spacing:.06em;color:#596270;margin-top:16px}nav{display:flex;gap:18px;flex-wrap:wrap}a{color:#1b4b98}footer{margin-top:40px;color:#596270}</style>
<main><header><h1>OpenWalkieTalkie screenshots</h1><p>SCREEN_COUNT screens from the actual deck firmware UI, using illustrative demo content. Each screen includes its native 480 × 480 PNG and a pixel-preserving 1920 × 1920 copy.</p><nav><a href="contact-sheet-01.png">Selected contact sheet</a><a href="contact-sheet-02.png">More screens</a><a href="READ-ME.txt">Usage notes</a></nav></header>
<div class="filters" role="group" aria-label="Filter screenshots"><button data-filter="all" aria-pressed="true">All TOTAL_COUNT</button><button data-filter="recommended" aria-pressed="false">Recommended 12</button><button data-filter="more" aria-pressed="false">More screens MORE_COUNT</button>PREVIEW_FILTER</div><section>
''' + "\n".join(cards) + '''</section><footer>Rendered with the Rev A UI preview from the current source tree. Demo content; no live accounts or private conversations. See manifest.json for provenance.</footer></main>
<script>document.querySelectorAll('[data-filter]').forEach(button=>button.addEventListener('click',()=>{document.querySelectorAll('[data-filter]').forEach(b=>b.setAttribute('aria-pressed',String(b===button)));document.querySelectorAll('[data-category]').forEach(card=>card.hidden=button.dataset.filter!=='all'&&card.dataset.category!==button.dataset.filter)}));</script></html>'''
    preview_filter = f'<button data-filter="preview" aria-pressed="false">Feature previews {len(PREVIEWS)}</button>' if PREVIEWS else ""
    (out / "index.html").write_text(gallery.replace("SCREEN_COUNT", str(len(SCENES)))
                                  .replace("TOTAL_COUNT", str(len(entries)))
                                  .replace("MORE_COUNT", str(len(SCENES) - 12))
                                  .replace("PREVIEW_FILTER", preview_filter))
    archive = out.with_suffix(".zip")
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as bundle:
        for path in sorted(out.rglob("*")):
            if path.is_file():
                bundle.write(path, Path(out.name) / path.relative_to(out))
    print(f"Exported {len(SCENES)} screens + {len(PREVIEWS)} feature preview to {out}")
    print(f"Shareable archive: {archive} ({archive.stat().st_size:,} bytes)")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=ROOT / "dist/marketing" / f"deck-screenshots-{datetime.date.today()}")
    parser.add_argument("--source-dir", type=Path, help="Reuse PNGs from tools/preview-all.py --board hw-a")
    args = parser.parse_args()
    out = args.out.resolve()
    if out.exists() and any(out.iterdir()):
        parser.error(f"Output directory must be empty: {out}")
    out.mkdir(parents=True, exist_ok=True)
    if args.source_dir:
        package(args.source_dir.resolve(), out)
    else:
        with tempfile.TemporaryDirectory(prefix="deck-marketing-") as temporary:
            subprocess.run([sys.executable, str(ROOT / "tools/preview-all.py"), "--board", "hw-a",
                            "--out", temporary, *[s[0] for s in SCENES + PREVIEWS]], cwd=ROOT, check=True)
            package(Path(temporary), out)


if __name__ == "__main__":
    main()
