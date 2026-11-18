# Implementation status

> Historical record: this document describes the earlier Electron/TypeScript attempt. The active implementation is the native Rust/egui application described in [ADR 001](decisions/001-implementation-runtime.md) and the [README](../README.md), including a persistent warm render worker. Validation policy below still applies: build only; tests deferred by owner.

Date: 18 November 2026. Validation policy: build only; tests deferred by owner.

## Scope delivered in code

The repository now contains a desktop implementation rather than specifications alone. Native core, UI, rendering helper, typed protocol, packaging configuration, sample project, and deterministic corpus generator are connected in one npm workspace.

Production compilation and Windows x64 packaging completed. See [build report](BUILD_REPORT.md) for artifacts; this does not change the pending validation policy below.

| Area | Implementation |
|---|---|
| Navigation | Native folder/file opening, ignore-aware background discovery, progressive snapshots, metadata/title indexing, virtualized tree, quick switcher, pins, history, recent workspaces. |
| Markdown | CommonMark token maps, multi-block extraction, headings, stable-identity reconciliation, UTF-16 and UTF-8 spans, container/newline mappings, conservative patch rejection, fence lengthening. |
| Files | Root-scoped IDs, explicit external-file grants, symlink checks, UTF-8 validation, read-only handling, conflict hashes, sibling replacement, recovery journals. |
| Watching | Ignore-aware directory subscriptions, coalesced events, external-file watches, reconciliation fallback. |
| Editing | CodeMirror block/full-file modes, highlighting, undo/redo/find, retained buffers, autosave setting, recovery/conflict choices, window-close journal handshake. |
| Rendering | Exact Mermaid version, sandboxed helper in distinct session, typed revision/job IDs, stale rejection, cancellation by helper destruction, finite source/time/memory/output budgets, byte-bounded cache. |
| Presentation | Warm/Authored presets, local fonts, light/dark shell, SVG preflight/sanitization, bounded SVG, static scene normalization and spatial culling, overview/compatibility path, pan/zoom/selection/search/outline. |
| Export | Identified valid SVG revision; bounded PNG dimensions/area; background/scale options; native destination picker. |
| Tooling | Locked graph, compile/build/package commands, font hashes and license inventory, Windows application icon, original examples, optional corpus generator. |

## Explicit limits and assumptions

- Electron/TypeScript replaces provisional Tauri/Rust. This is an implementation assumption made with no Rust toolchain and an instruction to defer tests, not P0 benchmark evidence.
- Atomic JSON metadata/recovery files replace proposed SQLite persistence. Search ranks in-memory metadata rather than using a database trigram index.
- Graph-aware Trace and exact node-to-source navigation remain future enhancements. Active label mapping uses measured SVG text geometry; graph relationships are not fabricated.
- Scenes have an explicit static feature subset. Unsupported features use a labeled bounded raster compatibility path when faithful raster output is available. Unsupported rich HTML/math rasterization produces an explanatory state. Fine detail cannot be recovered from an overview alone; SVG export preserves accepted upstream geometry.
- Source editing is limited to 16 MiB per file. Rendering, labels, geometry, output dimensions, and caches have additional finite limits. These are safeguards, not measured capacity claims.
- Fonts cover scripts included in bundled Manrope/Plex assets; other Unicode uses system fallback. No deterministic CJK font-metric guarantee is claimed.
- File identities are path-derived. Moves produce filesystem changes; identity across arbitrary external renames is not guaranteed.
- Disk hash/identity comparisons reduce overwrite races without providing atomic compare-and-swap against arbitrary external writers.
- Full-file source extraction on editing uses the shared parser. Large single-document responsiveness still requires measurement and may justify incremental/background extraction.
- PNG export is bounded rather than tiled; SVG remains available for output exceeding the raster area limit.
- Folder drag/drop and a minimap are not implemented; native pickers/recent projects and fit/pan/zoom are available. Accessible outlines currently list labels without inferred relationships.
- Installers are unsigned. macOS signing/notarization and Linux distribution acceptance require native release work.

## Validation intentionally pending

No unit, integration, property, browser, native smoke, accessibility, performance, crash, or cross-platform tests have been added or run. The app has not been interactively validated. P0–P5 acceptance gates remain open until evidence exists; build output alone does not satisfy them.

The dependency installer reports advisories in development/build-tool glob dependencies. These tools process this project's trusted build configuration; review patch availability before distribution. No forced major-version downgrade was applied to mask that report. The bundled runtime dependency graph still requires a release review.

When validation starts, use the existing test plan and record actual failures/numbers before changing advertised limits or performance objectives.
