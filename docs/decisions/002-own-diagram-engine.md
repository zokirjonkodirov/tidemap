# ADR 002: Own diagram engine

Date: 18 November 2026

Status: accepted. Supersedes the rendering parts of [ADR 001](001-implementation-runtime.md). The rest of ADR 001 (native Rust application, egui/eframe, no WebView) stands.

Design: [`docs/superpowers/specs/2026-11-18-own-engine-and-shell-redesign-design.md`](../superpowers/specs/2026-11-18-own-engine-and-shell-redesign-design.md).

## Context

ADR 001 rendered Mermaid with the third-party `mermaid-rs-renderer` crate inside a supervised child process. The child existed so a renderer we could not interrupt could be killed. Results crossed a bincode protocol and were tessellated with lyon. We could not control the layout or the look of the output, and the process boundary added latency and copies on every edit.

## Decision

Replace the third-party renderer and the child process with an in-house engine, `crates/diagram`.

Pipeline: parser, graph, layered layout, scene, direct egui paint.

- **Parser.** A byte lexer and parser produce a `Graph` with UTF-8 byte spans. Malformed input yields diagnostics, not panics.
- **Layout.** Layered, compound-aware: cycle removal, network simplex ranking, median ordering, Brandes-Koepf positioning plus a compaction pass, routed cubic edges. Subgraphs are laid out as clusters, and a subgraph with its own `direction` is laid out recursively. Disconnected components are packed. A seed from the previous layout keeps node order stable while the user types.
- **Scene.** A typed display list (clusters, edges, nodes, labels) with a uniform grid index for culling and hit testing.
- **Paint.** `src/paint.rs` draws the scene with egui primitives and real text at the zoomed size. There is no tessellation step and no text-as-paths. Level of detail drops text and then detail at small zoom.
- **Threading.** `crates/renderer` is a thin job runner: one worker thread owns the text measurer, jobs carry a cooperative cancel flag, the latest foreground job wins, results are cached (LRU, 96 MiB). The worker catches panics and reports a diagnostic. Sources over 2 MiB are rejected. There is no child process.
- **Export.** SVG is written from the scene (`crates/diagram/src/svg.rs`). PNG rasterises that SVG with resvg on a background thread, limited to 8192 px per side and 16 megapixels. `usvg`, `resvg` and `tiny-skia` are kept for PNG export only.

Removed: `mermaid-rs-renderer`, the supervised child process, the bincode protocol, the lyon scene path, `worker.rs`, `supervisor.rs`, the old `scene.rs` and `protocol.rs`. `roxmltree` and `xmlwriter` remain in the lockfile only as dependencies of `usvg`/`resvg`.

## What is supported

| Family | Status |
|---|---|
| `flowchart` / `graph`, including subgraphs | Rendered by the own engine |
| `sequenceDiagram`, `stateDiagram`, `classDiagram`, `erDiagram`, everything else | Not supported yet. The canvas shows a card naming the family; the source panel stays available |

There is no fallback renderer. Syntax details are in [COMPATIBILITY.md](../COMPATIBILITY.md).

## Styling

- Authored `%%{init: ...}%%` directives and frontmatter themes are skipped, not honoured.
- Authored `classDef`, `class`, `:::`, `style` and `linkStyle` colours and stroke settings are honoured and win over the defaults.
- With no authored style, nodes are coloured by role (or optionally by subgraph) from a pastel palette.
- `style.json` in the application-data directory controls palette (light and dark), geometry (font size, padding, radius, rank and node gaps, label width), edge width, arrow size, node stroke and cluster tint. The file is created with defaults on first run and polled once per second. Invalid JSON keeps the last good style and shows the error in Settings and the info popover. A palette change repaints; a geometry change clears the cache and lays out again.

## Measured results

All numbers are from the build reports in `.superpowers/sdd/`. They were taken on the owner's machine in release builds. Nothing below was re-measured when this ADR was written.

Targets (parse + layout + scene build): 200 nodes under 5 ms, 2,000 under 60 ms, 10,000 under 800 ms.

**`cargo native bench`, best of 5, ms** (`scene-style-svg-report.md`). This run was taken before the two layout quality passes below.

| Graph | 200 nodes | 2,000 nodes | 10,000 nodes |
|---|---|---|---|
| chain | 0.4 | 4.4 | 21.8 |
| tree | 0.4 | 3.6 | 19.2 |
| layered_dense | 1.5 | 48.5 (parse 1.9, layout 45.7, scene 0.9) | 505.7 (parse 10.1, layout 483.2, scene 12.4) |
| clustered | 1.3 | 12.4 | 83.0 |

**Layout only, `dump` example, after the second layout pass** (`layout-pass2-report.md`):

| Sample | Layout ms |
|---|---|
| big2000 (flat, 2,000 nodes) | 48 |
| big2000c (clustered, 2,000 nodes) | 40 to 53 |
| big10000 (flat, 10,000 nodes) | 433 |
| big10000c (clustered, 10,000 nodes) | 374 |
| s9 / llm_architecture (about 60 nodes, 6 clusters) | 5.3 |
| c10 / c20 / c40 (generated, 10 / 20 / 40 clusters) | 9.1 / 20.5 / 49.6 |

The first layout quality pass (`layout-quality-report.md`) measured big2000 at 43 ms and big10000 at about 470 ms. Output was byte-identical on repeat runs for the samples checked in pass 2 (s9, c20, big2000c).

**Layout quality, `llm_architecture` sample** (`layout-quality-report.md`, `layout-pass2-report.md`): bounds 3239x2644 before the quality pass, 2220x2632 after it (area minus 32%), 2200x2632 after pass 2. Crossings 36, then 31, then 39. Edges through nodes 3, then 3, then 0. Edges through cluster titles 5.

**Cluster containment** (pass 2): nodes outside their cluster and overlapping unrelated clusters are 0 on all 52 samples. Before the pass, the 2,000-node clustered stress sample had 1,524 outsiders and 93 overlaps, and the 10,000-node one 95,781 and 5,834.

## Known limits

- Edges can still cross cluster titles. The `llm_architecture` sample has 5 such edges. Edge labels can overlap the title x-range of a cluster.
- No edge bundling. Long edges between distant tiers can still drift diagonally.
- Crossings on `llm_architecture` rose from 31 to 39 when the global sibling-cluster order was made a hard constraint to fix containment. Other cluster samples had fewer crossings (for example mid3 72 to 40, c40 166 to 73).
- Above 1,500 working nodes the order phase does not search alternative orders. Above 4,000 nodes the edge clearance repair is skipped. Layout time for mid-size cluster graphs rose 3 to 10 times in pass 2, still tens of milliseconds.
- Limits: 20,000 nodes, 20,000 edges, world coordinates up to 10,000,000. Beyond these the app shows a budget card.
- A node declared outside a subgraph and later listed inside it joins that subgraph; the first subgraph that lists it wins. Mermaid.js may differ in edge cases.
- Network simplex pivots are capped at min(4n, 400), so very large graphs get an approximate ranking.
- A cluster endpoint edge to one of its own members is not drawn.
- Recursive clusters with their own direction connect to inner nodes with straight stubs that can cross other inner nodes.
- Features that were written but not exercised interactively: real mouse clicks on nodes and edges, Esc behaviour, moving the editor cursor to highlight a node, hold-last-good timing while typing, prefetch of neighbouring diagrams, camera anchoring on live edits, the export file picker, the PNG size-limit message, and opening `style.json` in the default editor. Only paths reachable through the screenshot hooks and the export CLI were run. macOS and Linux were not run.
- Large diagrams can open wider than the canvas at the minimum label zoom and be cut on the left (reported in `plan3-tasks3-5-report.md`; not fixed there).

## Testing

The owner decided that no automated tests are written or run in this phase. The owner tests manually. Plans contain test code as a behaviour specification only, and implementers verified with `cargo check` and `cargo native build`. Passing compilation is not evidence of syntax parity.

Tools that exist instead:

- `crates/diagram/examples/dump.rs`: layout to a debug SVG. Prints bounds, crossings, edges through nodes and titles, cluster containment counters and invariants. `DUMP_VERBOSE=1` lists offenders. `--seed <prev.mmd>` tests stable re-layout.
- `crates/diagram/examples/render.rs`: scene to the final SVG. Flags `--dark`, `--by-subgraph`, `--embed-fonts`, `--contrast`.
- `cargo native bench`: times parse, layout and scene build on synthetic graphs.
- Sample inputs in `crates/diagram/tests/fixtures/` (data only, no test code).
- Screenshot and export hooks in the application, listed in the README (`MERMAIDLITE_SHOT*`, `MERMAIDLITE_EXPORT*`).

## Consequences

- Preview latency no longer includes process IPC or a mesh copy.
- We own the layout and look, and can fix defects in the repository. We also own the gaps listed above.
- Most Mermaid families are not rendered until they are written. This is a loss against ADR 001, which advertised 23 families through the third-party crate.

## Re-measured after both layout passes

`cargo native bench`, release build, best of 5, on the owner's machine, 18 November 2026. This table supersedes the earlier bench figures above.

| Topology | Nodes | Edges | Parse | Layout | Scene | Total |
|---|---|---|---|---|---|---|
| chain | 200 | 199 | 0.1 ms | 0.5 ms | 0.0 ms | 0.7 ms |
| chain | 2,000 | 1,999 | 1.1 ms | 5.7 ms | 0.5 ms | 7.2 ms |
| chain | 10,000 | 9,999 | 5.4 ms | 19.3 ms | 3.3 ms | 28.0 ms |
| tree | 200 | 199 | 0.1 ms | 0.5 ms | 0.0 ms | 0.7 ms |
| tree | 2,000 | 1,999 | 1.0 ms | 7.5 ms | 0.6 ms | 9.1 ms |
| tree | 10,000 | 9,999 | 5.7 ms | 15.3 ms | 3.5 ms | 24.5 ms |
| layered_dense | 200 | 360 | 0.2 ms | 3.9 ms | 0.1 ms | 4.1 ms |
| layered_dense | 2,000 | 3,960 | 1.8 ms | 47.7 ms | 0.9 ms | 50.5 ms |
| layered_dense | 10,000 | 19,960 | 10.0 ms | 405.9 ms | 10.2 ms | 426.1 ms |
| clustered | 200 | 212 | 0.1 ms | 11.6 ms | 0.1 ms | 11.8 ms |
| clustered | 2,000 | 2,120 | 0.8 ms | 41.4 ms | 0.7 ms | 42.9 ms |
| clustered | 10,000 | 10,600 | 4.7 ms | 117.2 ms | 4.3 ms | 126.2 ms |

Targets were 5 ms at 200 nodes, 60 ms at 2,000 and 800 ms at 10,000. One row misses: clustered at 200 nodes takes 11.8 ms, because small graphs try about 20 alternative node orders and keep the narrowest.
