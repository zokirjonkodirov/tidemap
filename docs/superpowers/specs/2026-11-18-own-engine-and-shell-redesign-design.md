# Tidemap: own diagram engine and shell redesign

Date: 18 November 2026. Status: approved by owner, with the legacy renderer removed.
Supersedes the diagram and shell parts of `docs/DESIGN.md` and the rendering parts of `docs/ARCHITECTURE.md`. ADR 001 (Rust only, egui) stays in force.

## 1. Intent

The owner understands large LLM-built projects through flow diagrams, not code, and reads them here for hours. The app is a reading tool first and an editor second. Three things matter:

1. Diagrams look balanced and readable the moment a file opens, with no manual tuning.
2. Moving between files and diagrams is instant.
3. Shapes and styles are ours to control, not whatever a third-party renderer emits.

Today none of the three hold, and the cause is structural rather than cosmetic.

## 2. What is wrong today (measured from the code)

Current pipeline, per render, in a separate process:

```text
source → mermaid-rs-renderer parse + layout → SVG string
       → roxmltree re-parse + restyle rewrite → SVG string
       → usvg parse (text shaped into paths) → lyon tessellation of everything
       → 4096 px resvg raster → PNG encode
       → usvg re-serialise SVG → bincode frame (up to 128 MiB) → UI process
       → PNG decode on a thread → egui meshes
```

Consequences:

- **Look**: layout, shapes, padding and edge routing belong to the third-party crate. Our only lever is rewriting SVG attributes after layout, which cannot change geometry.
- **Speed**: three XML round trips, every glyph tessellated into triangles, a raster and PNG encode/decode on every keystroke-render, and a large IPC copy. A 180 ms debounce hides this.
- **Identity lost**: nodes and edges become anonymous paths. Links are matched back to nodes by label text (`DiagramLink { from, to }` keyed by text), so click-to-highlight and jump-to-source break on duplicate labels and cannot be exact.
- **Text**: labels are vector outlines, so they are not crisp at reading sizes and cannot be re-styled without a full re-render.
- **Shell**: palette tokens in `src/theme.rs` are already close to the target. The problems are layout (chrome and source panel take space from the diagram), duplicated controls, and status noise.

## 3. Goals, non-goals, anti-goals

**Goals**
- Own engine: parser → typed graph → layout → display list → direct egui paint. No SVG in the preview path.
- Balanced layouts by default; pastel, rounded, Miro-like nodes; smooth edges; labels never below 13 px at fit.
- Exact node/edge identity with source spans: click highlights neighbours, jumps to the source line.
- A style system (palette, shapes, spacing) in one data file, hot-reloaded.
- Shell rebuilt around the canvas: ≥75% of the window is diagram on open.
- Fast navigation: file switch and diagram switch show the new diagram in the next frame for ordinary sizes.

**Non-goals**
- Pixel parity with Mermaid.js layout. We aim for better-looking, not identical.
- Every Mermaid family in the own engine on day one (see §5.7).
- Real backdrop blur. egui cannot do it cheaply; softness comes from radius, tint and shadow.
- Collaboration, cloud, WYSIWYG node dragging.

**Anti-goals** (failure even if it technically works)
- A diagram that shuffles its whole layout while typing.
- Any path where the source panel or chrome covers the diagram.
- Silent dropping of syntax we do not understand. Unknown constructs produce a diagnostic with a line number.
- More than one accent colour, sharp corners, heavy borders, all-caps labels.

## 4. Approaches considered

| | Approach | Trade-off |
|---|---|---|
| A | Keep `mermaid-rs-renderer`, restyle harder | Cheapest. Cannot fix layout balance, shapes, identity or speed. Rejected: it is the current state. |
| B | Keep its parser, replace layout and paint | Saves the parser (about 15% of the work). Its AST has no source spans and no error recovery, so jump-to-source and partial previews stay weak. Rejected. |
| C | Own engine, with the legacy renderer kept as fallback for families not yet ported | Nothing regresses during migration, but two renderers, two looks and the whole SVG/process pipeline stay alive. Rejected by the owner. |
| D | **Own engine only; `mermaid-rs-renderer` and its SVG pipeline are deleted** | Most work up front and families not yet ported show an honest "not supported yet" state with the source. One renderer, one look, far less code. **Chosen by the owner.** |

## 5. The engine

New crate `crates/diagram`, no egui dependency, pure functions, fully testable. Four stages with a plain data hand-off between each.

```text
&str ──parse──▶ Graph ──layout──▶ Layout ──build──▶ Scene ──paint──▶ egui
              (spans, ids)     (positions)      (display list)   (in src/canvas.rs)
```

### 5.1 Parser

- Hand-written single-pass lexer and recursive-descent parser over bytes; no regex, no allocation per token; labels are slices of the source.
- Flowchart grammar: `flowchart`/`graph` with TB/TD/BT/LR/RL; all bracket shapes (`[]`, `()`, `([])`, `[[]]`, `[()]`, `(())`, `((()))`, `{}`, `{{}}`, `[/ /]`, `[\ \]`, `[/ \]`, `[\ /]`, `>]`) and the `@{ shape: … }` form; edges (`-->`, `---`, `-.->`, `==>`, `--o`, `--x`, `<-->`, `~~~`), extra-length edges, `|label|` and `-- label -->` forms, `&` fan-out chains; `subgraph … end` with nested `direction`; `classDef`, `class`, `:::`, `style`, `linkStyle`; `%%` comments; frontmatter and `%%{init}%%`; `<br>` and quoted/markdown strings (rendered as plain text with line breaks, bold/italic honoured).
- Every node, edge, subgraph and label records its UTF-8 byte span.
- Error recovery: an invalid statement yields a diagnostic with line and column, and parsing resumes at the next statement. The graph built from valid statements still renders, with the diagnostic shown.
- `click` and links are parsed and ignored with one notice (same policy as today).

### 5.2 Graph model

Arena-backed, `u32` ids, struct-of-arrays: nodes, edges, clusters (subgraph tree), adjacency in CSR form. Style is resolved to a small `StyleId` per element. This is the single identity used by layout, paint, selection, search and source mapping.

### 5.3 Text measurement

Advance widths read from the bundled font with `ttf-parser` (already in the dependency tree), cached per font size with an ASCII fast table. Labels wrap at a maximum width (default 220 px) with balanced line breaks, so nodes stay compact rather than becoming long strips. The same font file is used by egui for painting, so measured and painted widths agree within the node padding.

### 5.4 Layout: layered, compound-aware

| Step | Algorithm | Cost | Why |
|---|---|---|---|
| Cycle removal | Greedy feedback arc set (Eades–Lin–Smyth) | O(V+E) | Fewest reversed edges; reversed edges keep their original arrow. |
| Ranking | Network simplex, iteration-capped; longest-path fallback above a size threshold | near-linear in practice | Shortest total edge length gives compact, even ranks. |
| Subgraphs | Global compound layering with border nodes; a subgraph that declares its own `direction` is laid out recursively and treated as one node | O(V+E) extra | Cross-boundary edges route cleanly; per-subgraph direction works. |
| Ordering | Median/barycentre layer sweeps plus adjacent transpose, cluster-contiguous; crossings counted with an accumulator tree | O(sweeps · E log V) | Low crossings. Sweep count scales down with graph size. |
| Coordinates | Brandes–Köpf, four alignments, median-balanced | O(V+E) | Straight long edges, centred parents. This is the main source of the "balanced" look. |
| Components | Disconnected components packed toward the viewport aspect ratio | O(C log C) | No long thin strips of unrelated pieces. |
| Edge labels | Each label is a sized dummy node in its rank | — | Labels never overlap nodes or each other. |
| Routing | Ports spread along the node side; path through dummy nodes fitted as a smooth monotone cubic; clipped to the true shape outline; self-loops and parallel edges offset | O(E) | Smooth curves that enter nodes cleanly. |

Balance refinements on top: node widths within a rank snap to a small set of steps so siblings look uniform; rank and node spacing come from the style file.

**Stability while typing**: the previous layout's order within each rank seeds the next ordering pass, keyed by node id. Adding one node moves its neighbours, not the whole diagram. The camera keeps its anchor.

**Budgets**: every loop checks a cancellation flag. Hard caps on nodes, edges and world size produce a Budget diagnostic, as today.

### 5.5 Scene and paint

- `Scene` is a typed display list: clusters, edges, nodes, labels, in paint order, each carrying its graph id and world bounds, plus a uniform grid index for culling and hit-testing.
- No tessellation ahead of time and no text-as-paths. `src/canvas.rs` paints visible items with egui primitives: rounded rects and shape outlines, cubic béziers, and real text galleys at the zoomed font size (crisp at every zoom).
- Level of detail: below about 5 px text height labels are skipped; below about 2 px node height nodes become flat rects and edges straight polylines. When the visible item count passes a budget, the canvas draws a cached low-detail mesh and refines once the camera settles.
- Selection is by id: selected node, its edges and neighbours at full strength, everything else at reduced alpha. No re-layout, no re-render.
- Minimap draws from the same scene at the flat LOD. The 4096 px PNG overview is removed.

### 5.6 Threading and process model

The own engine runs **in-process on a worker thread** with cooperative cancellation. The supervised child process existed to kill a third-party engine we could not interrupt; our own bounded loops do not need it. This removes process IPC, bincode framing and the mesh copy from the preview path. A `Scene` is handed to the UI behind an `Arc`.

The supervised child process, the bincode protocol, `worker.rs`, `supervisor.rs`, `scene.rs` and the `mermaid-rs-renderer`, `usvg` scene path, `lyon`, `roxmltree`, `xmlwriter` and `bincode` dependencies are deleted. `crates/renderer` shrinks to a thin job runner: a worker thread, latest-wins scheduling, a byte-bounded scene cache, and export.

Edit debounce drops from 180 ms to about 30 ms. Neighbouring diagrams in the open file are laid out on idle, so Alt+PageUp/PageDown is a cache hit.

Targets on the owner's machine, parse + layout + scene build, to be measured not assumed: 200 nodes under 5 ms; 2,000 under 60 ms; 10,000 under 800 ms.

### 5.7 Diagram families

A family is one module implementing `parse(&str) -> Scene-ready model` behind a common trait. Detection is by the first keyword.

| Phase | Family | Engine |
|---|---|---|
| First | `flowchart` / `graph`, including subgraphs | Own layered layout |
| Second | `stateDiagram-v2` | Own; reuses the layered layout (composite states are clusters) |
| Second | `sequenceDiagram` | Own; column/row layout, trivial cost |
| Third | `classDiagram`, `erDiagram` | Own; layered layout with table-shaped nodes |
| Not supported | Everything else | No render. The canvas shows a calm "This diagram type is not supported yet" card naming the type, with the source panel one click away. |

There is no legacy fallback. The old renderer is removed in the canvas-integration phase, as soon as flowcharts render through the own engine.

### 5.8 Style system and shapes

One `style.json` in the app-data directory, created with defaults, hot-reloaded on change, also editable from Settings. It controls:

- **Palette**: the pastel set (§7), stroke darkness, text colour, canvas colour, light and dark variants.
- **Colour assignment** when the source gives none: by role. Process → sky, decision → peach, data store → mint, start/end → yellow, subroutine/external → lavender, everything else → gray. Subgraph containers get a very light tint of a rotating hue. Authored `classDef`/`style` always wins. A switch offers "by subgraph" colouring instead.
- **Geometry**: corner radius, padding, max label width, rank and node spacing, edge width, arrowhead size, font size.
- **Shapes**: a registry where each shape is an outline function, a text inset, and a boundary-intersection function for edge clipping. All Mermaid shape names map into it. Decision diamonds keep their shape with rounded joins. New shapes are added in one place and are immediately usable through `@{ shape: name }`.

Because style is applied at scene build and paint, a palette change repaints without re-layout; only geometry changes re-run layout.

### 5.9 Export

SVG is written directly from the `Scene` (real `<text>` with the bundled font embedded, rounded shapes, same colours). PNG rasterises that SVG with resvg on a background thread, same size limits as today (8192 px per side, 16 megapixels). resvg and usvg remain as export-only dependencies. Export therefore matches the screen.

## 6. Shell redesign

```text
┌──────────────────────────────────────────────────────────────────────────┐
│ ▤  ‹ ›   examples / system.mmd  ·  Workspace  2 of 3            ⓘ   ◐   │  slim bar, 40 px
├───────────────┬──────────────────────────────────────────┬───────────────┤
│ ⌕ Search      │                                          │ Source        │
│               │                                          │ (hidden by    │
│ ▾ docs        │                                          │  default;     │
│    orders.md  │              DIAGRAM CANVAS              │  resizable;   │
│  ▸ archive    │                                          │  pushes the   │
│ ▾ examples    │                                          │  canvas)      │
│  ● system.mmd │                                          │               │
│    welcome.md │      ╭──────────────────────────────╮    │               │
│               │      │  −   100%   +  │ ⤢ │ </> │ ⇩ │    │               │
│               │      ╰──────────────────────────────╯ ▣  │               │
└───────────────┴──────────────────────────────────────────┴───────────────┘
   260 px, collapsible        floating pill, bottom centre      minimap ▣
```

- **Sidebar**: search field, then a virtualised tree. Left-aligned names, 16 px indent per level, folder chevrons, active file on a soft accent-tinted pill, diagram count as quiet trailing text, tooltip with the full path when truncated. The sidebar is an inset rounded panel on the off-white backdrop, not a bordered column.
- **Top bar**: sidebar toggle, back/forward, breadcrumb (`folder / file · block title  2 of 3`, clicking the block part opens the diagram list), info button, theme toggle. No title block above the canvas.
- **Canvas**: fills everything else. Fit keeps labels ≥13 px; if that is impossible it fits to width and pans.
- **Floating pill**: zoom out, zoom level (click resets to 100%), zoom in, fit, toggle source, export. Every duplicate of these elsewhere is removed.
- **Source panel**: right side, hidden by default, draggable divider, pushes the canvas. Live preview. Clicking a node scrolls to and highlights its span; moving the cursor into a node's span highlights the node.
- **Info popover**: save state, preview freshness, diagnostics, compatibility notice, render time. Dismissible. A small dot on the info button signals an error or unsaved state; nothing else is permanently on screen.
- **Command palette** (Ctrl+P): centred, rounded, searches files and diagram titles together, fuzzy, top result preselected. Ctrl+Shift+P style commands (theme, export, fit) share it under a `>` prefix.
- **Navigation speed**: Alt+PageUp/PageDown between diagrams in a file; Alt+Left/Right history; camera remembered per diagram; neighbours pre-laid-out.
- **Interaction**: drag to pan, pinch or Ctrl+scroll to zoom, double-click empty canvas to fit, click node to highlight, Esc clears. Transitions 150–200 ms ease-out; none on layout changes while typing.
- **Minimap**: bottom-right, appears only when the diagram exceeds the viewport at current zoom.
- Narrow windows (<1000 px): sidebar auto-collapses; source becomes a toggle with the canvas rather than a third pane.

## 7. Design tokens

**Colour, light (primary theme)**

| Token | Value | Use |
|---|---|---|
| `bg.app` | `#FAFAF8` | Window backdrop |
| `bg.panel` | `#FFFFFF` | Sidebar, source panel, popovers, pill |
| `bg.canvas` | `#FAFAF8` | Diagram surface (same as app: no visible frame) |
| `bg.hover` | `#1F2328` at 5% | Row and button hover |
| `ink.primary` | `#1F2328` | Text (15.6:1 on white) |
| `ink.secondary` | `#5F6672` | Supporting text (5.9:1) |
| `ink.muted` | `#6B7280` | Counts, placeholders (4.8:1) |
| `line.hairline` | `#1F2328` at 8% | Only where separation by space fails |
| `accent` | `#41609E` | Selection, primary action, focus ring (6.2:1) |
| `accent.soft` | `#E9EEF7` | Active file pill, selected rows |

**Colour, dark (secondary theme)**: `bg.app #1B1D20`, `bg.panel #24272B`, `bg.canvas #1B1D20`, `ink.primary #E6E8EB`, `ink.secondary #A9B0BA`, `ink.muted #9099A4`, hairline white at 8%, `accent #8FAEE0`, `accent.soft #2A3447`. Neutral charcoal; no brown, no pure black.

**Diagram palette** (fill / 1 px stroke; text is always `ink.primary`; all pairs exceed AA)

| Name | Light fill | Light stroke | Dark fill | Dark stroke | Default role |
|---|---|---|---|---|---|
| Sky | `#DCEBFA` | `#8FB8E0` | `#26384A` | `#4F7396` | Process |
| Peach | `#FFE3D1` | `#E8A982` | `#4A3327` | `#96684C` | Decision |
| Mint | `#D9F2E3` | `#8CC9A6` | `#243F31` | `#4D8566` | Data store |
| Yellow | `#FFF4C2` | `#E3CC6B` | `#453E22` | `#8F8040` | Start / end |
| Lavender | `#E9E3FA` | `#B0A0DE` | `#352F4D` | `#6E61A0` | Subroutine / external |
| Gray | `#EFF0F2` | `#B9BEC6` | `#2E3136` | `#5B616B` | Other |
| Rose | `#FBE0E0` | `#DE9C9C` | `#4A2A2C` | `#96575A` | Error only |

Edges `#8A919C`, 1.5 px, arrowhead 7 px. Edge labels on a `bg.panel` pill, radius 8, 13 px text. Subgraph container: role hue at 35% of its fill strength, radius 16, title 13 px semibold top-left.

**Radii**: controls 8, cards and panels 12, dialogs and palette 16, nodes 12, subgraphs 16, pill toolbar full.
**Spacing**: 4, 8, 12, 16, 24, 32.
**Elevation**: one shadow for floating things only (pill, popover, palette): y 8, blur 24, `#14181C` at 14% light / 50% dark. Nodes have no shadow.
**Type**: Inter (bundled, OFL) for UI and diagram labels; JetBrains Mono or the existing IBM Plex Mono for source. UI 14/20; small 13/18; diagram label 14/20 medium; subgraph title 13 semibold; source 13/21. Sentence case everywhere. No text below 13 px.
**Motion**: 160 ms ease-out for panels, popovers, camera; off under reduced motion.

## 8. Phases

Each phase ends with something usable. The shell (phase 4) does not depend on the engine and can be pulled earlier if the owner wants the visible change first.

1. **Engine core**: `crates/diagram` with flowchart parser, graph, text measure, layout, scene. Invariant tests and a timing harness on a fixture corpus (§9).
2. **Canvas integration**: in-process worker thread, direct paint, LOD, id-based selection, jump-to-source, stable re-layout, the "not supported yet" card for other families, and deletion of the legacy renderer, child process and their dependencies.
3. **Style and export**: `style.json`, role colours, shape registry, hot reload, SVG writer, PNG via resvg.
4. **Shell**: tokens into `src/theme.rs`, Inter, sidebar tree, slim bar, floating pill, push-style source panel, info popover, palette, minimap, removal of duplicate controls.
5. **More families**: state and sequence, then class and ER.
6. **Hardening**: huge-diagram LOD mesh cache, neighbour pre-layout, contrast audit, 800×600 and 200% scale checks, ADR 002 recording the engine change.

## 9. Verification

ADR 001 deferred tests at the owner's request. An own parser and layout engine cannot be trusted without them, so this design asks to lift that for `crates/diagram` only:

- Parser: fixtures per syntax form; every diagnostic carries the right line; no panic on arbitrary bytes (fuzz-style loop).
- Layout invariants on every fixture: no node overlaps another; every node inside its subgraph; edges end on shape outlines; ranks respect edge direction except reversed cycle edges; edge labels overlap nothing.
- Stability: adding one node to a fixture moves fewer than a set fraction of existing nodes.
- Timing harness prints parse/layout/scene milliseconds for 200, 2,000 and 10,000-node generated graphs (chain, tree, dense, single connected component).
- Done-when from the brief: diagram ≥75% of window on open and readable without zoom; no sharp corner or heavy border; AA contrast (computed from the tokens above); one accent plus the pastel palette; existing features (save, recovery, conflict handling, watch, export, pins, history, multi-block Markdown) still work.

## 10. Decisions made on the owner's behalf

Proposed by the designer and accepted by the owner on 18 November 2026 (item 1 as amended by the owner):

1. Flowchart first; state and sequence second; class and ER third. Owner decision: no legacy renderer; unported families show "not supported yet".
2. Own engine runs in-process on a thread; the child process is removed entirely.
3. Stay on egui; no real blur, Craft softness approximated with radius, tint and one shadow.
4. Inter replaces Manrope.
5. Default node colour by shape role, with authored styles winning.
6. Light theme is the default on first run regardless of system setting; dark is opt-in.
7. Tests are written for the engine crate.
8. Style lives in `style.json` in app data, not in the repository and not in diagram source.

## 11. Left to the builder

Exact data layouts, the network-simplex iteration cap and the size threshold for its fallback, sweep counts, grid cell size, LOD thresholds, node-width snap steps, and the internal module split of `crates/diagram`. All are tuned against the timing harness and the fixture corpus.
