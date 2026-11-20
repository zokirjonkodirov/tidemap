# Changelog

All notable changes to Tidemap are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

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
- Not drawn yet: git graph, sankey, xychart, requirement, block, C4, architecture, packet, kanban,
  radar and treemap diagrams. They show a "not supported" card and the source stays editable.

## [0.2.0] - 2026-11-19

### Added

- Native Rust viewer and editor (egui) with an in-house flowchart engine: all node shapes, edge
  kinds, subgraphs, `classDef`, `style`, `linkStyle`.
- Markdown pages with inline Mermaid diagrams, workspace tree, find, minimap, outline and export to
  SVG and PNG.
- Motion and depth system, reduced-motion support, crash recovery and safe save.
- Cross-platform CI and tagged release packaging for Windows, macOS and Linux.

[0.3.0]: https://github.com/zokirjonkodirov/tidemap/releases/tag/v0.3.0
[0.2.0]: https://github.com/zokirjonkodirov/tidemap/releases/tag/v0.2.0
