# Website

The site lives in `site/` and is plain HTML, CSS and a little JavaScript: no framework, no build step to serve it.

- `site/index.html`: landing page. `site/guide/index.html`: the long-form guide.
- `site/llms.txt` (hand-written summary) and `site/llms-full.txt` (generated from the guide) are for language models.
- `site/assets/site.css` holds the tokens; they mirror `src/theme.rs` and `src/motion.rs`. Change the app's tokens, change these.

## The mark

`python tools/brand/icon.py` draws the app icon and writes `assets/icon*.svg`, `icon.png` and `icon.ico`
(needs Chrome, Chromium or Edge to rasterise). `build.py` then copies the small drawing to the site as the
favicon and top-bar brand, and the full one as `site/assets/mark.svg`. Meaning of each part: `docs/DESIGN.md`.

## Regenerating what comes from the app

Diagrams, screenshots, fonts, the social card and `llms-full.txt` are generated from the real app:

```text
cargo native build
python tools/site/build.py              everything (opens app windows for the screenshots)
python tools/site/build.py --no-shots   skip the screenshots
```

Needs Python 3 with `fontTools` and `Pillow`. The script rewrites the parts of `site/index.html` between
`<!-- build:… -->` markers; edit everything else by hand. Hero diagrams come from `tools/site/hero.md`,
gallery diagrams from `examples/`.

## Before a release

Version, file names and sizes are written in `site/index.html`, `site/guide/index.html`, `site/llms.txt` and
`BUILDS` in `site/assets/site.js`. Update them with each release.

## Publishing

`.github/workflows/pages.yml` publishes `site/` to GitHub Pages on every push to `main` that touches it.
Turn it on once in the repository: Settings, Pages, Source: GitHub Actions. The pages assume the address
`https://zokirjonkodirov.github.io/tidemap/`; with a custom domain, replace it in the canonical links, Open Graph
tags, JSON-LD, `sitemap.xml`, `robots.txt`, `llms.txt`, `404.html` and `build.py`.
