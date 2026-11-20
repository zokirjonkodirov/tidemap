# Changelog

All notable changes to Tidemap are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **Website** in `site/`: a landing page with a live, clickable copy of the window showing the
  engine's own output, a guide (install, syntax, shortcuts, `style.json`, export, FAQ), `llms.txt`
  and `llms-full.txt`, structured data and a sitemap. `tools/site/build.py` regenerates every
  diagram, screenshot and font from the app; `.github/workflows/pages.yml` publishes it.
- `Tidemap.exe` carries the app icon and version details, so Explorer and Task Manager show them.

### Changed

- **New mark**: a tide of three elevation layers under a start node and a process node joined by a
  rounded-head edge, in the accent family. It replaces the terracotta icon of the retired "warm
  clarity" direction and is drawn by `tools/brand/icon.py`, with a simpler drawing for 48 px and
  under.
- The welcome state shows the mark instead of the generic diagram tile.

### Fixed

- `cargo native build` and `package` link again on the GNU toolchain: the rust-mingw
  `self-contained` folder no longer goes on PATH, where its bare `x86_64-w64-mingw32-gcc` made
  rustc drop its own crt objects and import libraries.

## [0.4.0] - 2026-11-20

### Added

- **Git graph** (`gitGraph`): branches as lanes, commits with ids, tags, highlight and reverse
  styles, merges, cherry-picks, horizontal and vertical orientation.
- **Sankey** (`sankey-beta`): column layout with crossing reduction and curved ribbons.
- **XY chart** (`xychart-beta`): bar and line series with axes and grid.
- **Requirement diagrams**: requirement and element boxes with labelled relationships.
- **C4 diagrams** (`C4Context`, `C4Container`, `C4Component`, `C4Dynamic`, `C4Deployment`): C4
  colours, boundaries, labelled relationships.
- **Architecture diagrams** (`architecture-beta`): nested groups, services, database shapes, edges.
- **Block diagrams** (`block-beta`): column grid with spans, shapes and arrows.
- **Packet diagrams** (`packet-beta`), **kanban** boards, **radar** charts and **treemaps**.
- Polygon primitive and stronger line colours in the free-form drawing layer.

### Changed

- Every diagram family the engine knows is now drawn; only unknown keywords show the "not
  supported" card.

## [0.3.0] - 2026-11-20

### Added

- **State diagrams** (`stateDiagram`, `stateDiagram-v2`): transitions with labels, `[*]` start and
  end states per scope, composite states, `state "Title" as id`, descriptions drawn under the state
  name, `<<choice>>`, `<<fork>>` and `<<join>>`, notes, `direction`, `classDef`/`class`/`style`.
- **Sequence diagrams**: participants and actors with aliases, every message arrow (solid, dotted,
  open, cross, bidirectional), activation bars (`+`/`-`, `activate`/`deactivate`), notes left, right
  and over, `loop`/`alt`/`opt`/`par`/`critical`/`break`/`rect` frames with `else`/`and`/`option`
  dividers, `autonumber`.
- **Class diagrams**: annotations, attributes and methods in compartments, generics, all UML relation
  markers (inheritance, composition, aggregation, association, dependency, realization),
  cardinalities, notes, namespaces.
- **ER diagrams**: entity tables with type, name, key and comment columns, crow's foot cardinality
  markers, identifying and non-identifying relationships, aliases.
- **Pie, Gantt, timeline, user journey and quadrant charts**, drawn with a new free-form drawing
  layer that follows the light and dark themes.
- **Mindmaps**: indentation tree with node shapes, laid out left to right.
- SVG and PNG export for every new diagram type.
- Sample diagrams for each type in `examples/`.
- CI now runs the test suite on Linux, Windows and macOS.

### Changed

- Diagram families that are not drawn yet are listed in the README and
  [compatibility notes](docs/COMPATIBILITY.md).

### Known limits

- State diagram notes are placed above or below their state, not beside it, and concurrent regions
  (`--`) are ignored.
- Chart-style diagrams (pie, Gantt, timeline, journey, quadrant) are not selectable on the canvas.
- Git graph, sankey, xychart, requirement, block, C4, architecture, packet, kanban, radar and
  treemap diagrams arrived in 0.4.0.

## [0.2.0] - 2026-11-19

### Added

- Native Rust viewer and editor (egui) with an in-house flowchart engine: all node shapes, edge
  kinds, subgraphs, `classDef`, `style`, `linkStyle`.
- Markdown pages with inline Mermaid diagrams, workspace tree, find, minimap, outline and export to
  SVG and PNG.
- Motion and depth system, reduced-motion support, crash recovery and safe save.
- Cross-platform CI and tagged release packaging for Windows, macOS and Linux.

[0.4.0]: https://github.com/zokirjonkodirov/tidemap/releases/tag/v0.4.0
[0.3.0]: https://github.com/zokirjonkodirov/tidemap/releases/tag/v0.3.0
[0.2.0]: https://github.com/zokirjonkodirov/tidemap/releases/tag/v0.2.0
