#!/usr/bin/env python3
"""Draws the Tidemap mark and renders every file made from it.

The mark is the app's design language at icon size:

- The tile is the window frame: a deep tone of the accent, lit from above. A 5 px rim of light
  along the top and a 7 px rim of shade along the bottom give it thickness, the way every
  elevated surface in the app has a 1 px top highlight and a contact shadow.
- Three tide bands rise from the bottom like the app's elevation levels (Card, Raised, Overlay):
  each lighter than the one behind, each casting a soft shadow and carrying a highlight on its
  crest. They are also the "tide" in the name.
- Two nodes in the diagram palette sit on top: a start node (yellow stadium) and a process node
  (sky rectangle) floating on the first crest: the "map".
- The edge between them follows the tide's S-curve and ends in a rounded head that follows the
  curve's approach angle. No sharp arrowheads, as in the engine.

Small sizes (48 px and under) use a simpler drawing: one band, no hairlines, fatter shapes.

Writes assets/icon.svg, assets/icon-small.svg, assets/icon.png (512 px, the window and launcher
icon) and assets/icon.ico (16 to 256 px). Rendering uses headless Chrome or Edge.

    python tools/brand/icon.py
"""

from __future__ import annotations

import math
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
ASSETS = ROOT / "assets"

# Palette: accent family for the frame and the tide, diagram swatches for the nodes
# (src/theme.rs, crates/diagram/src/style.rs).
FRAME = "#2D477A"
TIDE = ("#3A5893", "#4A69A7", "#5E7FBE")
SHADE = "#081530"
START = ("#FFF4C2", "#E3CC6B")    # yellow swatch: start and end
PROCESS = ("#DCEBFA", "#8FB8E0")  # sky swatch: process
EDGE = "#FFFFFF"


def squircle(r: float, dy: float = 0.0, n: float = 5.0, steps: int = 180) -> str:
    """A superellipse tile centred in the 512 canvas: continuous corners, no visible join."""
    points = []
    for i in range(steps):
        t = 2 * math.pi * i / steps
        c, s = math.cos(t), math.sin(t)
        x = 256 + r * math.copysign(abs(c) ** (2 / n), c)
        y = 256 + dy + r * math.copysign(abs(s) ** (2 / n), s)
        points.append(f"{x:.1f} {y:.1f}")
    return "M" + "L".join(points) + "Z"


def crest(y: float, amp: float) -> str:
    """One tide line across the tile: high on the left, a trough, high again on the right."""
    return (f"M-20 {y - amp * 0.2:.0f}C90 {y - amp:.0f} 170 {y + amp:.0f} 270 {y + amp * 0.15:.0f}"
            f"S430 {y - amp * 0.9:.0f} 540 {y - amp * 0.1:.0f}")


def head(x: float, y: float, angle: float, length: float, width: float, rounding: float) -> str:
    """A rounded arrowhead with its tip at (x, y), pointing along `angle`."""
    ca, sa = math.cos(angle), math.sin(angle)
    corners = [(x + px * ca - py * sa, y + px * sa + py * ca) for px, py in ((0, 0), (-length, width / 2), (-length, -width / 2))]
    d = "M" + "L".join(f"{cx:.1f} {cy:.1f}" for cx, cy in corners) + "Z"
    return f'<path d="{d}" fill="{EDGE}" stroke="{EDGE}" stroke-width="{rounding}" stroke-linejoin="round"/>'


TILE = squircle(224)
RIM_LIGHT = squircle(224, dy=5)
RIM_SHADE = squircle(224, dy=-7)


def frame(defs: str, inside: str) -> str:
    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512">
  <title>Tidemap</title>
  <defs>
    <clipPath id="tile"><path d="{TILE}"/></clipPath>{defs}
  </defs>
  <path d="{TILE}" fill="{FRAME}"/>
  <g clip-path="url(#tile)">
{inside}
    <path d="{TILE}{RIM_LIGHT}" fill="#FFFFFF" fill-opacity="0.2" fill-rule="evenodd"/>
    <path d="{TILE}{RIM_SHADE}" fill="#000000" fill-opacity="0.18" fill-rule="evenodd"/>
  </g>
</svg>
'''


def large() -> str:
    defs = f'''
    <filter id="band" x="-10%" y="-30%" width="120%" height="160%">
      <feDropShadow dx="0" dy="-2" stdDeviation="9" flood-color="{SHADE}" flood-opacity="0.34"/>
    </filter>
    <filter id="lift" x="-30%" y="-40%" width="160%" height="200%">
      <feDropShadow dx="0" dy="2" stdDeviation="2" flood-color="{SHADE}" flood-opacity="0.3"/>
      <feDropShadow dx="0" dy="10" stdDeviation="12" flood-color="{SHADE}" flood-opacity="0.32"/>
    </filter>
    <filter id="line" x="-20%" y="-20%" width="140%" height="150%">
      <feDropShadow dx="0" dy="3" stdDeviation="4" flood-color="{SHADE}" flood-opacity="0.28"/>
    </filter>'''
    bands = "\n".join(
        f'    <path d="{crest(y, amp)}L540 560L-20 560Z" fill="{fill}" filter="url(#band)"/>\n'
        f'    <path d="{crest(y, amp)}" fill="none" stroke="#FFFFFF" stroke-opacity="0.24" stroke-width="3"/>'
        for (y, amp), fill in zip(((300, 36), (368, 30), (430, 24)), TIDE)
    )
    inside = f'''{bands}
    <g filter="url(#line)">
      <path d="M196 160C268 160 250 298 312 298" fill="none" stroke="{EDGE}" stroke-width="15" stroke-linecap="round"/>
      {head(334, 298, 0.0, 26, 28, 12)}
    </g>
    <rect x="62" y="126" width="128" height="68" rx="34" fill="{START[0]}" stroke="{START[1]}" stroke-width="5" filter="url(#lift)"/>
    <rect x="340" y="264" width="112" height="68" rx="20" fill="{PROCESS[0]}" stroke="{PROCESS[1]}" stroke-width="5" filter="url(#lift)"/>'''
    return frame(defs, inside)


def small() -> str:
    inside = f'''    <path d="{crest(352, 34)}L540 560L-20 560Z" fill="{TIDE[1]}"/>
    <path d="M200 164C290 164 250 316 320 316" fill="none" stroke="{EDGE}" stroke-width="34" stroke-linecap="round"/>
    <rect x="56" y="112" width="160" height="104" rx="52" fill="{START[0]}"/>
    <rect x="300" y="264" width="156" height="104" rx="30" fill="{PROCESS[0]}"/>'''
    return frame("", inside)


def browser() -> str:
    candidates = [
        os.environ.get("CHROME", ""),
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        shutil.which("google-chrome") or "", shutil.which("chromium") or "", shutil.which("chromium-browser") or "",
    ]
    for candidate in candidates:
        if candidate and Path(candidate).exists():
            return candidate
    sys.exit("No Chrome, Chromium or Edge found: set CHROME to its path")


def render(svg: Path, size: int, out: Path, work: Path) -> Image.Image:
    """Rasterises the SVG at `size` px with a transparent background, so small sizes are drawn
    at their size rather than scaled down from a large bitmap."""
    page = work / f"render-{svg.stem}-{size}.html"
    page.write_text(f'<!doctype html><style>html,body{{margin:0;background:transparent}}img{{display:block}}</style>'
                    f'<img src="{svg.as_uri()}" width="{size}" height="{size}">', encoding="utf-8")
    subprocess.run([browser(), "--headless=new", "--disable-gpu", "--hide-scrollbars", "--default-background-color=00000000",
                    f"--window-size={size},{size}", f"--screenshot={out}", page.as_uri()],
                   check=True, capture_output=True, timeout=60)
    return Image.open(out).convert("RGBA")


def main() -> None:
    big, little = ASSETS / "icon.svg", ASSETS / "icon-small.svg"
    big.write_text(large(), encoding="utf-8", newline="\n")
    little.write_text(small(), encoding="utf-8", newline="\n")
    with tempfile.TemporaryDirectory() as temp:
        work = Path(temp)
        render(big, 512, ASSETS / "icon.png", work)
        frames = [render(little if size <= 48 else big, size, work / f"ico-{size}.png", work) for size in (16, 24, 32, 48, 64, 128, 256)]
        frames[-1].save(ASSETS / "icon.ico", format="ICO", sizes=[f.size for f in frames], append_images=frames[:-1])
    print("assets/icon.svg, icon-small.svg, icon.png and icon.ico written")


if __name__ == "__main__":
    main()
