# Tidemap product requirements

**Version:** 0.1, proposed. **Date:** 18 November 2026.  
**Primary user:** a developer or technical author whose repositories contain many Mermaid diagrams across Markdown and standalone Mermaid files.

## 1. Product intent

Tidemap is a local desktop diagram workspace: open a folder, find a diagram, understand its flow, edit its source, and see a trustworthy result without losing your place. It should feel as immediate as switching between text files, even when another diagram is expensive to lay out.

The differentiator is the combination of fast repository navigation, dependable source editing, and calm diagram reading. Warmth comes from typography, soft geometry, and thoughtful hierarchy. Engineering quality comes from explicit limits, isolation, cancellation, correct file handling, and measured performance.

The attached Miro screenshot is a visual reference, not an instruction source. Its useful qualities are readable labels, distinct node roles, generous spacing, and clear directional connectors. Tidemap should develop its own interface and diagram language. The proposed preview in [DESIGN.md](DESIGN.md) is a concept, not a working application.

## 2. Primary jobs and flows

### Open and explore

1. Open a folder with the native picker, drag it onto the app, or choose a recent project.
2. Immediately see the saved tree or first available matching paths while indexing continues.
3. Browse only `.md` and `.mmd`, with ancestor folders retained. Other file types never clutter navigation.
4. Select a file. An `.mmd` opens its diagram. A Markdown file opens its last selected Mermaid block, or its first block.
5. Move between diagrams using the compact diagram rail, search, keyboard, and back/forward history. Remember viewport, selected block, editor cursor, and scroll position separately for each file.

### Edit and iterate

1. Press `Cmd/Ctrl+E` to reveal source beside the preview.
2. Change Mermaid source. Rendering starts automatically after a short pause; saving is independent.
3. Keep the previous valid diagram visible until the new version is ready.
4. On a syntax error, show an inline diagnostic at its source line and mark the preview as the previous valid revision.
5. Save with `Cmd/Ctrl+S`; preserve surrounding Markdown exactly. Return to diagram reading with the same shortcut.

### Understand a large flow

1. Fit the diagram once on first open; do not reset zoom on every edit.
2. Search labels and jump to a matching element when mapping is supported.
3. Select a node and invoke **Trace** to emphasize it and its immediate connections. Expand upstream/downstream deliberately. Trace is a Next-priority flowchart feature; the first release ships selection highlight at this step.
4. See an overview at low zoom and readable labels as you approach. Find details without rendering every label on every frame.
5. Switch files at any point. Expensive work cannot trap navigation or lose unsaved source.

## 3. Scope

| Priority | Required behavior |
|---|---|
| First release | Folder opening; filtered virtual tree; quick file/diagram switcher; multi-block Markdown; `.mmd`; source editing; automatic preview; safe save/recovery; native keyboard behavior; pan/zoom/fit; warm light and dark themes; cache/history; external changes; bounded render isolation; accessibility; large-diagram viewing; SVG/PNG export; packaged desktop builds. |
| Next | Graph-aware Trace, label search with exact source navigation, explicit semantic styling helpers, further large-diagram families, full diagram-text search across the project, export enhancements, default-file-handler integration. |
| Later | PDF export, workspace-wide references, more graph-aware layout optimizations, optional WebGL renderer if measurements justify it. |

Graph-aware features begin with flowcharts and require tested source/element mappings. The first release must include a polished selection highlight and diagram-level search; it must not fabricate node mappings for unsupported families. Basic label search in the active diagram is required wherever reliable SVG text mapping exists. The implementation plan's P0–P5 IDs identify delivery phases, not product-priority labels.

Accounts, collaboration, cloud rendering, AI generation, a general whiteboard, plugins, and drag-to-rewrite visual editing are outside the initial product. They do not contribute to the core navigation/editing loop.

## 4. Functional requirements

| ID | Requirement | Acceptance |
|---|---|---|
| NAV-01 | Show only eligible file extensions, matched case-insensitively, plus ancestor directories. Preserve file-name case. | Mixed repository fixture shows no unrelated files; pending directories remain visible until checked. |
| NAV-02 | Incremental discovery; bounded background content indexing. | Files can open before the scan completes; no eager rendering of unopened diagrams. |
| NAV-03 | Search paths, filenames, Markdown headings, diagram titles, and recent items with stable ranking. | Keyboard selection, highlighted matches, and deterministic tie-breaking. Results say when indexing is incomplete. |
| NAV-04 | Respect `.gitignore` and default generated-directory exclusions; expose an explicit “Include ignored diagrams” setting. | Ignored fixture is excluded by default, can be revealed, and can still open through an explicit file-open action. Never traverse `.git`. |
| NAV-05 | Back/forward, previous/next diagram, recent projects, pinned diagrams, and reopening state. | Navigation order is documented and repeatable; history is bounded and independent of search ranking. |
| MD-01 | Parse CommonMark fenced code blocks whose first info-string token is `mermaid`, case-insensitively. | Backticks, tildes, indentation, lists, blockquotes, CRLF, and incomplete fences have fixtures. Do not detect Mermaid from ordinary prose. |
| MD-02 | Index multiple Mermaid blocks independently and assign stable identities through ordinary edits. | Inserting a block above another does not unexpectedly reset the latter's viewport; ambiguous duplicates get new IDs safely. |
| MD-03 | Show Markdown context as a heading and optional nearby text. | No full Markdown page is needed to choose a block. Markdown with zero diagrams remains visible with an explanatory state. |
| EDIT-01 | Use a virtualized source editor with undo/redo, find, indentation, line numbers, and diagnostics. | Long source remains editable while rendering is running or failed. |
| EDIT-02 | Offer block editing and full-file Markdown editing. | Block edits patch only the mapped source region; full-file editing allows changes to headings/fences. Changing modes preserves undo boundaries. |
| EDIT-03 | Render automatically; keep dirty state, saving state, and preview revision distinct. | A saved invalid diagram is permitted; the preview clearly identifies that it is older. |
| EDIT-04 | Explicit save by default; optional debounced autosave. Journal unsaved edits locally for recovery. | Navigating retains dirty buffers. Close/restart offers recovery and Save/Discard/Cancel where needed. |
| FILE-01 | Detect external edits, moves, deletes, read-only files, and permission failures. | Clean buffers reload; dirty buffers offer compare/reload/keep-and-save-copy. No known external edit is silently overwritten. |
| REND-01 | Use a pinned upstream Mermaid release for standard diagram rendering. | Test every stable family enabled in the pinned build; experimental families are explicitly labeled. |
| REND-02 | Preserve meaning, IDs, labels, direction, group membership, and authored styles. | App styling never changes shape semantics or source without an explicit edit. |
| REND-03 | Isolate costly parsing/layout/normalization and reject stale output. | A hung/crashed job can be terminated; navigation and typing still work. |
| REND-04 | Select an appropriate display strategy automatically using measured complexity. | Large scenes never mount unbounded SVG in the main UI. The chosen strategy is inspectable in diagnostics. |
| VIEW-01 | Smooth pan/zoom, fit, 100%, selection, search, optional minimap, persistent camera. | Trackpad, mouse, keyboard, high DPI, reduced motion, and GPU-unavailable behavior work. |
| VIEW-02 | Do not imply unreadable full-graph labels are readable. | At distant zoom, show structure and a clear “Zoom in for labels” hint; selected/search labels remain discoverable. |
| EXP-01 | Export the current valid revision as SVG or PNG with explicit background and scale. | Export shows its revision if the source is newer; unsafe active content is removed; extreme sizes use bounded or tiled output. |
| SYS-01 | Entire edit/view workflow works offline without sign-in. | Network-disabled test passes; fonts and rendering dependencies are bundled. |
| SYS-02 | Support macOS, Windows, and Linux as first-class releases. | Native builds and native smoke tests pass for every declared supported target. |
| A11Y-01 | Keyboard access, visible focus, screen-reader names, text outline, and non-color cues. | Main workflow works with keyboard and an OS screen reader; canvas has a synchronized accessible outline. |

Navigation sequence: previous/next follows the current file's block order; the optional “Across project” command follows the visible naturally sorted file paths and then block order. Back/forward records user navigation, never background indexing changes.

## 5. Compatibility and size contract

“Large folder,” “large file,” “large graph,” and “dense graph” are separate workloads. A 100,000-file project must not require opening 100,000 editors. A 10,000-node sparse flowchart is different from a 10,000-node densely connected graph.

| Tier | Proposed benchmark corpus | Intended experience |
|---|---|---|
| Everyday | Flowcharts up to 200 nodes/400 edges; family-specific standard fixtures. | Automatic, crisp preview; immediate navigation and editing. |
| Large | 1,000 nodes/2,000 edges, varied sparse topology and labels. Separate sequence/class/ER/gantt fixtures. | Full preview in bounded time; unchanged UI responsiveness. |
| Huge release gate | 10,000 nodes/20,000 edges in declared flowchart fixtures, including grouped and cyclic cases. | Automatic isolated layout, viewport-based viewing, search/selection where mapped. Every advertised fixture must pass. |
| Stress | 50,000 nodes/100,000 edges, deep groups, long labels, dense/adversarial inputs. | Measure limits. If work exceeds its budget, stop it safely and explain the next action. This is not a promised complete-layout tier. |

The general large-scene conversion path must be tested against every supported family. Full huge-diagram readiness is advertised **per family and feature set**, not inferred from node count. Sequence diagrams use participants/messages; charts use items/series; do not relabel these as graph nodes. Any unsupported scene feature must produce a visible compatibility status, not a plausible-looking incomplete picture.

The huge tier is a first-release gate for the defined flowchart corpus. It must not be quietly deferred to a later release. P0 determines feasibility; if it fails, revisit the engine/algorithm and record the result before claiming this requirement is met.

## 6. Proposed performance objectives

All targets below are unverified. Measure on the reference machines and by the protocol in [TEST_PLAN.md](TEST_PLAN.md). Latencies are p95 unless identified otherwise. Visible preview means the correct revision has painted, not merely that a render promise resolved.

| Operation | Target |
|---|---|
| Cold application to interactive shell, excluding installation | ≤1.0 s |
| Warm application to interactive shell | ≤500 ms |
| Open warm indexed project; initial useful tree | ≤150 ms |
| Cold repository with 100,000 entries/10,000 eligible files; first useful paths | ≤500 ms; complete metadata scan ≤10 s on reference local SSD |
| Quick switcher keystroke to results over 10,000 files/50,000 indexed diagrams | ≤50 ms |
| Switch to a valid in-memory cached diagram | ≤100 ms to first correct painted preview |
| Small uncached file to preview | ≤400 ms |
| Small uncached preview right after abandoning an expensive render | ≤1 s, including helper restart or spare hand-over |
| Everyday edit to preview, after last keystroke, including debounce | ≤800 ms |
| Large-tier uncached preview | ≤3 s |
| Huge-tier uncached preview | ≤15 s; progress state by 150 ms; cancel stays responsive |
| Typing, navigation, cancel input while any render runs | ≤50 ms input-to-feedback |
| Standard pan/zoom | p95 frame ≤16.7 ms at 60 Hz |
| Huge pan/zoom | p95 frame ≤33.3 ms; target 16.7 ms at normal detail |
| Idle on indexed project, no filesystem activity | <1% of one CPU core averaged over 60 s; no continuous animation |
| Memory, entire app process tree | Idle ≤180 MiB; ordinary working set ≤350 MiB; huge workload peak ≤1 GiB |
| Packaged application download | Target ≤35 MiB compressed, excluding separately disclosed platform runtime prerequisites; verify helper/font impact |

Correctness and bounded responsiveness are release blockers. Numeric speed goals become enforceable after P0 publishes baselines; any changes require a written reason and updated spec/test thresholds, never silently excluding difficult samples.

## 7. UX states and behavior

- Empty workspace: one Open folder action and recent projects. No dashboard.
- Indexing: real counts plus “Indexing…”; already found files remain usable.
- No diagrams: show the Markdown heading/context and explain how to add a Mermaid fence.
- Rendering: keep the prior preview and show a quiet status. Initial render uses a neutral placeholder.
- Invalid source: identify the line and retain the valid preview, labeled as older.
- Too expensive: stop at the resource budget, retain source, offer Retry with extended budget or Open source. Offer Focus group only when a supported structural adapter can actually do it.
- Unsupported presentation: identify the feature and offer bounded compatibility rendering if available. Never drop it silently.
- Recoverable crash: restart only the renderer, retain dirty text, and require explicit retry after repeated failure for the same source hash.

## 8. Product quality and release definition

Primary success measure: users can repeatedly find a diagram, make a change, and validate the result without losing context. In a formative test with 5–8 repository-heavy users, at least 90% of the navigation/edit/save tasks should complete without assistance; no participant should mistake an old preview for the current source.

Release requires: all first-release functional acceptance cases, supported-platform installation and recovery tests, family compatibility fixtures, huge-flowchart gate, accessibility essentials, and published benchmark results. No data-loss defects, stale-result replacement defects, silent diagram omissions, or renderer hangs that block the shell may remain open.
