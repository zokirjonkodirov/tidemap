# Implementation status

Last updated 19 November 2026. Version 0.4.0 (Tidemap).

The earlier Electron/TypeScript attempt is discarded; see [BUILD_REPORT.md](BUILD_REPORT.md) for its history only. The active implementation is a native Rust/egui app with an in-house diagram engine ([ADR 001](decisions/001-implementation-runtime.md), [ADR 002](decisions/002-own-diagram-engine.md)).

## Delivered

| Area | State |
|---|---|
| Diagram engine (`crates/diagram`) | Flowchart/graph parser, layered compound-aware layout, scene with spatial index, SVG writer. Multithreaded layout. |
| Files and index (`crates/core`) | Folder scan, Markdown block extraction with source maps, safe save, recovery, watcher. |
| Render jobs (`crates/renderer`) | Single worker thread, cooperative cancel, LRU cache (96 MiB), SVG and PNG export (max 8192 px side, 16 MP). |
| App (`src/`) | File tree, search, pins, recent projects, live preview, node-to-source jumps, minimap, outline, light/dark, source panel, command palette, `style.json` live reload. |
| Packaging | `cargo native package`: Windows zip, macOS/Linux tar.gz, Linux `.desktop`. GitHub Actions release on `v*` tags. |

## Tests

9 automated tests in `crates/diagram`: golden layout (`tests/golden.rs`), parallel equals sequential, cancel flag, and the worker-pool policy tests. Run `cargo test --workspace`. There are no UI, file-handling or export tests.

## Known gaps

- Only flowcharts render. Sequence, state, class, ER and other families show a "not supported" card.
- `%%{init}%%` is ignored. Layout differs from Mermaid.js.
- CI (`.github/workflows/ci.yml`) builds only; it runs no tests and triggers on `main` while the local branch is `master`.
- Releases are unsigned. macOS first run needs right-click, Open.
- Not validated on macOS or Linux by hand; those builds come from CI only.
- Limits: 20,000 nodes/edges, 2 MiB source.
