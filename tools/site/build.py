#!/usr/bin/env python3
"""Builds the generated parts of the Tidemap website in `site/`.

Everything the site shows of the app comes from the app itself:

- Diagrams: every `examples/*.mmd` and the three blocks of `tools/site/hero.md` are exported
  with `TIDEMAP_EXPORT`, once with the light palette and once with the dark one. The two
  exports are merged into one SVG whose colours follow the reader's colour scheme.
  Gallery files embed a subset of Inter so they render the same inside `<img>`; the hero
  diagrams are written inline into `site/index.html` and use the page's fonts.
- Screenshots: `TIDEMAP_SHOT` captures of the app, saved as WebP.
- Fonts: Inter and IBM Plex Mono from `assets/fonts`, subset and saved as WOFF.
- Social card and icons.

Needs a release build of the app (`cargo native build`) and Python 3 with fontTools and Pillow.

    python tools/site/build.py            everything
    python tools/site/build.py --no-shots skip the screenshots (they open app windows)
"""

from __future__ import annotations

import html
import io
import os
import re
import shutil
import subprocess
import sys
import tempfile
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urljoin

from fontTools import subset
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[2]
SITE = ROOT / "site"
FONTS = ROOT / "assets" / "fonts"
EXE = ROOT / "target" / "release" / ("tidemap.exe" if os.name == "nt" else "tidemap")

INTER = {"400": FONTS / "Inter-Regular.ttf", "500": FONTS / "Inter-Medium.ttf", "600": FONTS / "Inter-SemiBold.ttf"}
MONO = FONTS / "IBMPlexMono-Regular.ttf"

# Gallery order: the families people reach for first come first.
# (slug, name, keyword, example file, alt text)
FAMILIES = [
    ("flowchart", "Flowchart", "flowchart", "system.mmd",
     "Flowchart: Markdown and Mermaid files feed a project index, search and the source editor, which hands off to an isolated renderer and local recovery."),
    ("sequence", "Sequence", "sequenceDiagram", "sequence-basic.mmd",
     "Sequence diagram: a user, a web app and a database exchange numbered messages inside loop and alt frames, with notes."),
    ("class", "Class", "classDiagram", "class-basic.mmd",
     "Class diagram: an abstract Animal with Dog and Cat subclasses, attributes, methods and UML relation markers."),
    ("state", "State", "stateDiagram-v2", "state-basic.mmd",
     "State diagram with a composite Active state, a choice, a fork and labelled transitions."),
    ("er", "Entity relationship", "erDiagram", "er-basic.mmd",
     "Entity relationship diagram: CUSTOMER, ORDER, PRODUCT and LINE_ITEM tables with keys and crow's foot cardinalities."),
    ("gantt", "Gantt", "gantt", "gantt-basic.mmd",
     "Gantt chart of a release plan: research, wireframes, backend and frontend tasks, a blocker and a release milestone."),
    ("pie", "Pie", "pie", "pie-basic.mmd",
     "Pie chart of pets adopted: dogs 73%, cats 16%, birds 8%, rats 3%."),
    ("mindmap", "Mindmap", "mindmap", "mindmap-basic.mmd",
     "Mindmap of Tidemap branching into diagrams, editor and export."),
    ("timeline", "Timeline", "timeline", "timeline-basic.mmd",
     "Timeline of social media from 2002 to 2006 in two sections."),
    ("journey", "User journey", "journey", "journey-basic.mmd",
     "User journey of a working day, each task scored by two actors."),
    ("gitgraph", "Git graph", "gitGraph", "gitgraph-basic.mmd",
     "Git graph with main, develop and hotfix branches, a tag, merges and a reverted commit."),
    ("quadrant", "Quadrant chart", "quadrantChart", "quadrant-basic.mmd",
     "Quadrant chart placing three campaigns by reach and engagement."),
    ("xychart", "XY chart", "xychart-beta", "xychart-basic.mmd",
     "XY chart of monthly sales revenue from January to June, bars with a line."),
    ("sankey", "Sankey", "sankey-beta", "sankey-basic.mmd",
     "Sankey diagram of energy flowing from coal, solar and gas through electricity and heating to industry, homes and losses."),
    ("c4", "C4", "C4Context", "c4-basic.mmd",
     "C4 system context diagram of an internet banking system, its customer, mainframe and e-mail system."),
    ("architecture", "Architecture", "architecture-beta", "architecture-basic.mmd",
     "Architecture diagram: a gateway, a server, a database and storage inside an API group."),
    ("block", "Block", "block-beta", "block-basic.mmd",
     "Block diagram with columns, a spanning block, a rounded block and a decision."),
    ("requirement", "Requirement", "requirementDiagram", "requirement-basic.mmd",
     "Requirement diagram: a test entity satisfies a requirement that contains a functional requirement."),
    ("packet", "Packet", "packet-beta", "packet-basic.mmd",
     "Packet diagram of a TCP header: ports, sequence and acknowledgment numbers, flags and window."),
    ("kanban", "Kanban", "kanban", "kanban-basic.mmd",
     "Kanban board with Todo, In progress and Done columns."),
    ("radar", "Radar", "radar-beta", "radar-basic.mmd",
     "Radar chart comparing two students' grades in five subjects."),
    ("treemap", "Treemap", "treemap-beta", "treemap-basic.mmd",
     "Treemap of products and services by size."),
]

TAG = re.compile(r'<([a-zA-Z][\w:-]*)((?:\s+[\w:-]+="[^"]*")*)\s*(/?)>')
ATTR = re.compile(r'([\w:-]+)="([^"]*)"')
PAINT_DEFAULTS = {"fill": "#000", "stroke": "none", "fill-opacity": "1", "stroke-opacity": "1", "opacity": "1", "stop-color": "#000"}

# Characters the page fonts keep: ASCII plus the typography the copy uses.
PAGE_TEXT = "".join(chr(c) for c in range(0x20, 0x7F)) + " ©·×–—‘’“”…• ←↑→↓−⌘⇧⌥⌃↵ "


# ---------------------------------------------------------------------------
# Running the app
# ---------------------------------------------------------------------------

def run_app(env: dict[str, str], *args: str) -> None:
    full = {**os.environ, **env}
    result = subprocess.run([str(EXE), *args], env=full, cwd=ROOT, capture_output=True, text=True, timeout=120)
    if result.returncode != 0:
        sys.exit(f"tidemap {' '.join(args)} failed: {result.stderr.strip()}")


def export(source: Path, out: Path, data_dir: Path, dark: bool = False, scale: float = 1) -> str:
    env = {"TIDEMAP_EXPORT": str(out), "TIDEMAP_DATA_DIR": str(data_dir), "TIDEMAP_EXPORT_SCALE": str(scale)}
    if dark:
        env["TIDEMAP_SHOT_THEME"] = "dark"
    run_app(env, str(source))
    return out.read_text(encoding="utf-8") if out.suffix == ".svg" else ""


# ---------------------------------------------------------------------------
# Light + dark export -> one SVG
# ---------------------------------------------------------------------------

def strip_export(svg: str) -> str:
    """Drops the embedded font and the background rectangle: the page supplies both."""
    svg = re.sub(r"<style>.*?</style>\s*", "", svg, count=1, flags=re.S)
    svg = re.sub(r'(<svg[^>]*>\s*)<rect x="[^"]*" y="[^"]*" width="[^"]*" height="[^"]*" fill="[^"]*"/>\s*', r"\1", svg, count=1)
    return svg


def merge(light: str, dark: str, prefix: str, attrs=None) -> tuple[str, str]:
    """Returns the light SVG with a class on every element whose paint differs in the dark
    export, and the CSS rules that apply the dark paint. `attrs(tag, attributes)` may return
    extra attributes for an element."""
    light, dark = strip_export(light), strip_export(dark)
    light_tags, dark_tags = list(TAG.finditer(light)), list(TAG.finditer(dark))
    if len(light_tags) != len(dark_tags):
        sys.exit(f"{prefix}: the light and dark exports differ in shape")
    rules: dict[tuple[str, str], str] = {}
    out, position = [], 0
    for a, b in zip(light_tags, dark_tags):
        if a.group(1) != b.group(1):
            sys.exit(f"{prefix}: <{a.group(1)}> in light, <{b.group(1)}> in dark")
        la, da = dict(ATTR.findall(a.group(2))), dict(ATTR.findall(b.group(2)))
        classes = []
        for key in sorted(set(la) | set(da)):
            lv, dv = la.get(key, PAINT_DEFAULTS.get(key)), da.get(key, PAINT_DEFAULTS.get(key))
            if lv == dv:
                continue
            if key not in PAINT_DEFAULTS:
                sys.exit(f"{prefix}: {key} differs between themes ({lv} / {dv})")
            name = rules.setdefault((key, dv), f"{prefix}{len(rules)}")
            classes.append(name)
        extra = attrs(a.group(1), la) if attrs else {}
        if classes:
            extra["class"] = " ".join(filter(None, [la.get("class"), *classes]))
        attributes = a.group(2)
        for key, value in extra.items():
            if f' {key}="' in attributes:
                attributes = re.sub(rf' {key}="[^"]*"', f' {key}="{html.escape(value)}"', attributes)
            else:
                attributes += f' {key}="{html.escape(value)}"'
        out.append(light[position:a.start()])
        out.append(f"<{a.group(1)}{attributes}{a.group(3)}>")
        position = a.end()
    out.append(light[position:])
    css = "".join(f".{name}{{{key}:{value}}}" for (key, value), name in rules.items())
    return "".join(out), css


def view_box(svg: str) -> tuple[float, float, float, float]:
    x, y, w, h = (float(v) for v in re.search(r'viewBox="([^"]+)"', svg).group(1).split())
    return x, y, w, h


def text_by_weight(svg: str) -> dict[str, str]:
    """Characters drawn at each font weight, so each weight is subset to what it needs."""
    used: dict[str, set[str]] = {}
    for match in re.finditer(r'<text([^>]*)>(.*?)</text>', svg, re.S):
        declared = re.search(r'font-weight="(\d+)"', match.group(1))
        weight = declared.group(1) if declared else "400"
        content = html.unescape(re.sub(r"<[^>]+>", "", match.group(2)))
        used.setdefault(weight, set()).update(content)
    return {weight: "".join(sorted(chars)) for weight, chars in used.items()}


def woff(path: Path, text: str, features: list[str]) -> bytes:
    options = subset.Options()
    options.flavor = "woff"
    options.layout_features = features
    options.hinting = False
    options.desubroutinize = True
    options.name_IDs = [0, 1, 2, 3, 4, 5, 6]
    options.name_languages = [0x0409]
    font = subset.load_font(str(path), options)
    subsetter = subset.Subsetter(options)
    subsetter.populate(text=text + " ")
    subsetter.subset(font)
    buffer = io.BytesIO()
    subset.save_font(font, buffer, options)
    return buffer.getvalue()


def standalone(svg: str, css: str) -> str:
    """A self-contained SVG for `<img>`: its own fonts and its own dark-scheme rule."""
    import base64

    faces = []
    for weight, chars in sorted(text_by_weight(svg).items()):
        data = base64.b64encode(woff(INTER.get(weight, INTER["500"]), chars, ["kern"])).decode()
        faces.append(f'@font-face{{font-family:"Inter";font-weight:{weight};src:url(data:font/woff;base64,{data}) format("woff")}}')
    style = "".join(faces) + (f"@media (prefers-color-scheme:dark){{{css}}}" if css else "")
    return re.sub(r"(<svg[^>]*>)", lambda m: f"{m.group(1)}\n<style>{style}</style>", svg, count=1)


# ---------------------------------------------------------------------------
# Hero: three diagrams from one Markdown file, with source links
# ---------------------------------------------------------------------------

def hero_blocks() -> list[tuple[str, str]]:
    text = (ROOT / "tools" / "site" / "hero.md").read_text(encoding="utf-8")
    return re.findall(r"^## (.+?)\n\n```mermaid\n(.*?)```", text, re.S | re.M)


def hero_links(source: str) -> tuple[dict[str, set[int]], list[tuple[str, str, int]]]:
    """Node id -> source lines that name it, and the edges in the order the engine numbers them."""
    lines = source.rstrip("\n").split("\n")
    kind = lines[0].split()[0]
    nodes: dict[str, set[int]] = {}
    edges: list[tuple[str, str, int]] = []

    def mention(node: str, line: int) -> None:
        nodes.setdefault(node, set()).add(line)

    if kind == "flowchart":
        for number, line in enumerate(lines[1:], 2):
            parts = re.split(r"\s*-->\s*(?:\|[^|]*\|\s*)?", line.strip())
            ids = [re.match(r"\w+", part).group(0) for part in parts if re.match(r"\w+", part)]
            for node in ids:
                mention(node, number)
            edges += [(a, b, number) for a, b in zip(ids, ids[1:])]
    elif kind == "sequenceDiagram":
        messages = []
        for number, line in enumerate(lines[1:], 2):
            declared = re.match(r"\s*(?:participant|actor)\s+(\w+)", line)
            if declared:
                mention(declared.group(1), number)
                mention(f"{declared.group(1)} (end)", number)
                edges.append((declared.group(1), f"{declared.group(1)} (end)", number))
            message = re.match(r"\s*(\w+)\s*-+>>?\s*(\w+)\s*:", line)
            if message:
                a, b = message.groups()
                for node in (a, b, f"{a} (end)", f"{b} (end)"):
                    mention(node, number)
                messages.append((a, b, number))
        edges += messages
    elif kind.startswith("stateDiagram"):
        for number, line in enumerate(lines[1:], 2):
            transition = re.match(r"\s*(\S+)\s*-->\s*([^\s:]+)", line)
            if not transition:
                continue
            a, b = transition.groups()
            a = "[*]startroot" if a == "[*]" else a
            b = "[*]endroot" if b == "[*]" else b
            mention(a, number)
            mention(b, number)
            edges.append((a, b, number))
    return nodes, edges


KEYWORDS = r"\b(?:flowchart|sequenceDiagram|stateDiagram-v2|direction|participant|actor|as|LR|TB|classDef|class|subgraph|end)\b"


def highlight(line: str) -> str:
    """Escapes one source line and marks keywords, arrows and edge labels like the editor does."""
    tokens = re.split(r"(-->>|->>|-->|\|[^|]*\||\[\*\]|" + KEYWORDS + r")", line)
    out = []
    for token in tokens:
        if not token:
            continue
        escaped = html.escape(token)
        if re.fullmatch(KEYWORDS, token):
            out.append(f'<span class="tk-k">{escaped}</span>')
        elif token in ("-->>", "->>", "-->") or token == "[*]":
            out.append(f'<span class="tk-o">{escaped}</span>')
        elif token.startswith("|") and token.endswith("|"):
            out.append(f'<span class="tk-s">{escaped}</span>')
        else:
            out.append(escaped)
    return "".join(out) or " "


def build_hero(work: Path, data_dir: Path) -> tuple[str, str]:
    """The diagrams for the canvas and the numbered source lists for the source card."""
    blocks = hero_blocks()
    stages, codes, styles = [], [], []
    for index, (title, source) in enumerate(blocks):
        path = work / f"hero-{index}.mmd"
        path.write_text(source, encoding="utf-8")
        light = export(path, work / f"hero-{index}.svg", data_dir)
        dark = export(path, work / f"hero-{index}-dark.svg", data_dir, dark=True)
        nodes, edges = hero_links(source)

        def attrs(tag: str, a: dict[str, str]) -> dict[str, str]:
            if tag != "g" or "data-id" not in a:
                return {}
            if a.get("class") == "node":
                lines = nodes.get(a["data-id"], set())
                return {"data-lines": " ".join(map(str, sorted(lines)))}
            if a.get("class") in ("edge", "edge-label"):
                number = int(a["data-id"].lstrip("e"))
                if number >= len(edges):
                    sys.exit(f"hero {index}: the engine drew more edges than the source parser found")
                start, end, line = edges[number]
                return {"data-from": start, "data-to": end, "data-lines": str(line)}
            return {}

        svg, css = merge(light, dark, f"h{index}-", attrs)
        plain, plain_css = merge(light, dark, "c")
        (SITE / "diagrams" / f"hero-{index}.svg").write_text(standalone(plain, plain_css), encoding="utf-8")
        _, _, width, height = view_box(svg)
        svg = re.sub(r'<svg xmlns="http://www.w3.org/2000/svg" viewBox="([^"]+)" width="[^"]+" height="[^"]+">',
                     lambda m: f'<svg class="dia" viewBox="{m.group(1)}" width="{width:g}" height="{height:g}" role="img" aria-label="{html.escape(title)}: diagram drawn by Tidemap">',
                     svg, count=1)
        styles.append(css)
        hidden = "" if index == 0 else " hidden"
        stages.append(f'<div class="stage" data-index="{index}" data-title="{html.escape(title)}" data-w="{width:g}" data-h="{height:g}"{hidden}>\n{svg.strip()}\n</div>')
        lines = source.rstrip("\n").split("\n")
        rows = "\n".join(f'<li data-line="{n}"><span class="ln" aria-hidden="true">{n}</span><code>{highlight(line)}</code></li>' for n, line in enumerate(lines, 1))
        codes.append(f'<ol class="code" data-index="{index}" aria-label="Source of {html.escape(title)}"{hidden}>\n{rows}\n</ol>')
    style = f'<style>@media (prefers-color-scheme:dark){{{"".join(styles)}}}</style>'
    return "\n".join([style, *stages]), "\n".join(codes)


# ---------------------------------------------------------------------------
# Gallery
# ---------------------------------------------------------------------------

def build_gallery(work: Path, data_dir: Path) -> str:
    items = []
    for slug, name, keyword, file, alt in FAMILIES:
        source_path = ROOT / "examples" / file
        light = export(source_path, work / f"{slug}.svg", data_dir)
        dark = export(source_path, work / f"{slug}-dark.svg", data_dir, dark=True)
        svg, css = merge(light, dark, "c")
        (SITE / "diagrams" / f"{slug}.svg").write_text(standalone(svg, css), encoding="utf-8")
        _, _, width, height = view_box(svg)
        source = html.escape(source_path.read_text(encoding="utf-8").rstrip())
        items.append(
            f'<li class="tile" id="{slug}">\n'
            f'  <button type="button" class="tile-hit" data-slug="{slug}" aria-haspopup="dialog" aria-label="{html.escape(name)} diagram, open larger">\n'
            f'    <span class="tile-canvas"><img src="diagrams/{slug}.svg" width="{width:g}" height="{height:g}" loading="lazy" decoding="async" alt="{html.escape(alt)}"></span>\n'
            f'  </button>\n'
            f'  <p class="tile-name">{html.escape(name)}</p>\n'
            f'  <p class="tile-key"><code>{html.escape(keyword)}</code></p>\n'
            f'  <template><pre><code>{source}</code></pre></template>\n'
            f"</li>"
        )
    return '<ul class="gallery" role="list">\n' + "\n".join(items) + "\n</ul>"


# ---------------------------------------------------------------------------
# Screenshots, fonts, icons, social card
# ---------------------------------------------------------------------------

SHOTS = [
    # (name, file to open, extra environment)
    ("page", "examples/welcome.md", {}),
    ("palette", "examples/system.mmd", {"TIDEMAP_SHOT_PALETTE": ">"}),
]


def build_shots(work: Path, data_dir: Path) -> None:
    out = SITE / "assets" / "shots"
    out.mkdir(parents=True, exist_ok=True)
    for name, file, extra in SHOTS:
        for theme in ("light", "dark"):
            png = work / f"{name}-{theme}.png"
            env = {"TIDEMAP_SHOT": str(png), "TIDEMAP_SHOT_SIZE": "1440x900", "TIDEMAP_SHOT_FOLDER": "examples",
                   "TIDEMAP_DATA_DIR": str(data_dir), **extra}
            if theme == "dark":
                env["TIDEMAP_SHOT_THEME"] = "dark"
            run_app(env, file)
            image = Image.open(png).convert("RGB")
            for width in (1440, 960):
                scaled = image if width == image.width else image.resize((width, round(image.height * width / image.width)), Image.LANCZOS)
                scaled.save(out / f"{name}-{theme}-{width}.webp", "WEBP", quality=84, method=6)


def build_fonts() -> None:
    out = SITE / "assets" / "fonts"
    out.mkdir(parents=True, exist_ok=True)
    features = ["kern", "liga", "calt", "tnum", "case", "ccmp", "locl"]
    for weight, path in INTER.items():
        (out / f"inter-{weight}.woff").write_bytes(woff(path, PAGE_TEXT, features))
    (out / "plex-mono-400.woff").write_bytes(woff(MONO, PAGE_TEXT, ["kern"]))
    for license_file in ("Inter-OFL.txt", "IBMPlexMono-OFL.txt"):
        shutil.copy(FONTS / license_file, out / license_file)


def build_icons() -> None:
    """The mark comes from tools/brand/icon.py. The small drawing serves the favicon and the
    28 px brand in the top bar; the full one is shown at app-icon size by the downloads."""
    out = SITE / "assets"
    shutil.copy(ROOT / "assets" / "icon-small.svg", out / "icon.svg")
    shutil.copy(ROOT / "assets" / "icon.svg", out / "mark.svg")
    shutil.copy(ROOT / "assets" / "icon.ico", SITE / "favicon.ico")
    icon = Image.open(ROOT / "assets" / "icon.png").convert("RGBA")
    icon.resize((180, 180), Image.LANCZOS).save(out / "apple-touch-icon.png")


def build_card(work: Path, data_dir: Path) -> None:
    """The 1200x630 social card: name, one line, and the hero flowchart as the engine draws it."""
    source = work / "hero-0.mmd"
    png = work / "card-diagram.png"
    export(source, png, data_dir, scale=2)
    card = Image.new("RGB", (1200, 630), "#F1F2F4")
    draw = ImageDraw.Draw(card)
    icon = Image.open(ROOT / "assets" / "icon.png").convert("RGBA").resize((64, 64), Image.LANCZOS)
    card.paste(icon, (64, 64), icon)
    draw.text((148, 66), "Tidemap", font=ImageFont.truetype(str(INTER["600"]), 48), fill="#1F2328")
    draw.text((64, 164), "Read and edit the Mermaid diagrams", font=ImageFont.truetype(str(INTER["600"]), 44), fill="#1F2328")
    draw.text((64, 220), "in your Markdown. Native, offline.", font=ImageFont.truetype(str(INTER["600"]), 44), fill="#5F6672")
    diagram = Image.open(png).convert("RGB")
    sheet_w, sheet_h = 1072, 272
    sheet = Image.new("RGB", (sheet_w, sheet_h), "#FAFAF8")
    diagram.thumbnail((sheet_w - 48, sheet_h - 32), Image.LANCZOS)
    sheet.paste(diagram, ((sheet_w - diagram.width) // 2, (sheet_h - diagram.height) // 2))
    mask = Image.new("L", (sheet_w, sheet_h), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, sheet_w - 1, sheet_h - 1), radius=16, fill=255)
    draw.rounded_rectangle((63, 309, 64 + sheet_w, 310 + sheet_h), radius=17, fill="#E3E5E8")
    card.paste(sheet, (64, 310), mask)
    card.save(SITE / "assets" / "card.png", optimize=True)


# ---------------------------------------------------------------------------
# llms-full.txt: the guide as Markdown, so the two never drift apart
# ---------------------------------------------------------------------------

class GuideToMarkdown(HTMLParser):
    BLOCKS = {"p", "li", "pre", "h1", "h2", "h3", "summary", "tr"}

    def __init__(self, base: str):
        super().__init__(convert_charrefs=True)
        self.base = base
        self.out: list[str] = []
        self.inside_main = False
        self.skip = 0
        self.lists: list[list] = []  # [kind, counter]
        self.pre = False
        self.href: list[str | None] = []
        self.kbd_end = -1
        self.row: list[str] | None = None
        self.cell: list[str] | None = None
        self.header_done = False

    def text(self) -> str:
        return "".join(self.out)

    def emit(self, value: str) -> None:
        if self.cell is not None:
            self.cell.append(value)
        else:
            self.out.append(value)

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == "main":
            self.inside_main = True
        if not self.inside_main:
            return
        # Icons and screen-reader-only text stay out; `skip` counts open tags inside them.
        if self.skip:
            self.skip += 1
            return
        if tag == "svg" or "vh" in (attrs.get("class") or "").split():
            self.skip = 1
            return
        if tag in ("h1", "h2", "h3"):
            self.emit("\n\n" + "#" * int(tag[1]) + " ")
        elif tag == "summary":
            self.emit("\n\n### ")
        elif tag == "p":
            self.emit("\n\n")
        elif tag in ("ul", "ol"):
            self.lists.append([tag, 0])
            self.emit("\n")
        elif tag == "li":
            kind = self.lists[-1]
            kind[1] += 1
            self.emit("\n" + ("- " if kind[0] == "ul" else f"{kind[1]}. "))
        elif tag == "pre":
            self.pre = True
            self.emit("\n\n```\n")
        elif tag == "code" and not self.pre:
            self.emit("`")
        elif tag == "strong":
            self.emit("**")
        elif tag == "kbd":
            # <kbd>Ctrl</kbd> <kbd>P</kbd> reads as Ctrl+P
            target = self.cell if self.cell is not None else self.out
            joined = "".join(target)
            if self.kbd_end >= 0 and self.kbd_end <= len(joined) and joined[self.kbd_end:].strip() == "":
                target[:] = [joined[: self.kbd_end], "+"]
        elif tag == "a":
            href = attrs.get("href", "")
            self.href.append(None if href.startswith("#") else urljoin(self.base, href))
            if self.href[-1]:
                self.emit("[")
        elif tag == "table":
            self.header_done = False
            self.emit("\n")
        elif tag == "tr":
            self.row = []
        elif tag in ("td", "th"):
            self.cell = []
            self.kbd_end = -1

    def handle_endtag(self, tag):
        if not self.inside_main:
            return
        if tag == "main":
            self.inside_main = False
            return
        if self.skip:
            self.skip -= 1
            return
        if tag in ("ul", "ol"):
            self.lists.pop()
        elif tag == "pre":
            self.pre = False
            self.emit("\n```")
        elif tag == "code" and not self.pre:
            self.emit("`")
        elif tag == "strong":
            self.emit("**")
        elif tag == "kbd":
            self.kbd_end = len("".join(self.cell if self.cell is not None else self.out))
        elif tag == "a":
            href = self.href.pop() if self.href else None
            if href:
                self.emit(f"]({href})")
        elif tag in ("td", "th") and self.cell is not None:
            self.row.append(" ".join("".join(self.cell).split()))
            self.cell = None
            self.kbd_end = -1
        elif tag == "tr" and self.row is not None:
            self.out.append("\n| " + " | ".join(self.row) + " |")
            if not self.header_done:
                self.out.append("\n|" + "---|" * len(self.row))
                self.header_done = True
            self.row = None

    def handle_data(self, data):
        if not self.inside_main or self.skip:
            return
        if self.pre:
            self.emit(data)
        else:
            self.emit(re.sub(r"\s+", " ", data))


def build_llms_full() -> None:
    page = (SITE / "guide" / "index.html").read_text(encoding="utf-8")
    parser = GuideToMarkdown("https://zokirjonkodirov.github.io/tidemap/guide/")
    parser.feed(page)
    body = re.sub(r"[ \t]+\n", "\n", parser.text())
    body = re.sub(r"\n{3,}", "\n\n", body).strip()
    head = "Source: https://zokirjonkodirov.github.io/tidemap/guide/ (generated from that page by tools/site/build.py).\n\n"
    (SITE / "llms-full.txt").write_text(head + body + "\n", encoding="utf-8", newline="\n")


# ---------------------------------------------------------------------------

def inject(path: Path, name: str, content: str) -> None:
    text = path.read_text(encoding="utf-8")
    start, end = f"<!-- build:{name} -->", f"<!-- /build:{name} -->"
    i, j = text.index(start) + len(start), text.index(end)
    indent = re.search(r"([ \t]*)$", text[:text.index(start)]).group(1)
    # Not re-indented: the content holds <pre> blocks whose whitespace is part of the source.
    path.write_text(text[:i] + "\n" + content + "\n" + indent + text[j:], encoding="utf-8", newline="\n")


def main() -> None:
    if not EXE.exists():
        sys.exit(f"{EXE} not found: run `cargo native build` first")
    (SITE / "diagrams").mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as temp:
        work = Path(temp)
        data_dir = work / "data"  # default style.json, whatever the local one says
        data_dir.mkdir()
        stages, sources = build_hero(work, data_dir)
        inject(SITE / "index.html", "hero", stages)
        inject(SITE / "index.html", "hero-source", sources)
        inject(SITE / "index.html", "gallery", build_gallery(work, data_dir))
        build_fonts()
        build_icons()
        build_card(work, data_dir)
        build_llms_full()
        if "--no-shots" not in sys.argv:
            build_shots(work, data_dir)
    print("site/ is up to date")


if __name__ == "__main__":
    main()
