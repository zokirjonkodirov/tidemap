# Implementation plan

> **Historical.** This plan targeted Tauri/Svelte and a proposed `apps/desktop` layout that was never built. The real layout is in the [README](../README.md); executed plans are in [superpowers/plans](superpowers/plans/).

## 1. Delivery strategy

Build a benchmarked vertical slice before investing in the complete interface. Every phase produces a reviewable artifact and has explicit exit criteria. The core promise—large folders and large diagrams without freezing—is required for the first full release.

The workspace is currently empty. The structure and commands below are proposed; no starter application has been generated.

```text
apps/desktop/              shell UI, source editor, canvas, desktop integration
crates/core/               files, watcher, scheduler, persistence, recovery
crates/render-host/        restricted Mermaid WebView host and supervisor protocol
crates/scene/              SVG preflight, normalization, geometry, spatial metadata
packages/contracts/        versioned generated Rust/TypeScript protocol types
packages/design-tokens/    shared shell/diagram/export tokens
fixtures/                  licensed examples and deterministic generated corpus
bench/                     generation, replay, measurement and report tools
docs/                      requirements, decisions, compatibility and benchmark reports
```

Use one Rust workspace and one frontend workspace. Do not introduce a backend server, monorepo orchestrator, plugin API, or custom layout engine until justified by an actual dependency or benchmark.

## 2. Phases and exit gates

### P0 — Prove the difficult parts

**Order:** first. **Output:** runnable research spike, repeatable fixtures, platform measurements, architecture decision.

| Work item | Concrete output | Exit gate |
|---|---|---|
| P0.0 Probe upstream layout first | One to two days: pinned Mermaid parse/layout alone on the large and huge topology sweep, in each target engine and for each available layout engine; time and peak memory per topology, connected graphs reported separately. | Numbers published before scene work starts. They decide whether P0.5 targets presentation only, also needs the Architecture §3C adapter, or the huge corpus needs an owner decision. |
| P0.1 Pin toolchain and baseline | Exact Rust, Node/package manager, Tauri, WebView and Mermaid versions; bundled font assets; lockfiles; CI build matrix. | Minimal packaged window runs on Windows, macOS ARM/Intel, and selected Linux targets. |
| P0.2 Prove isolated rendering | Render-only helper protocol; forced hang, crash, cancel and restart scenarios. | Main window remains interactive; cancel feedback ≤50 ms and cleanup ≤500 ms; no orphan descendants; helper cold start to first-render-ready measured, and the spare-helper question in Architecture §5 answered. |
| P0.3 Extract Markdown accurately | Small CommonMark extraction/source-map library with fixtures. | Multi-block, indentation/container, CRLF, duplicate and incomplete-fence tests pass; block edit round-trips preserve other bytes. |
| P0.4 Establish performance baselines | Generated repository and diagram corpus; stage timings and total memory. | Publish all runs and failures for everyday/large/huge tiers on reference machines. |
| P0.5 Prove large presentation | Bounded SVG threshold, normalized scene + culling/overview prototype, text and search parity. The accessible outline is built in P2/P3, not in the spike. | Huge corpus can be explored without an unbounded main-window DOM; supported SVG features are declared and visually checked. |
| P0.6 Resolve runtime/engine choice | Decision record comparing actual Tauri results and a fallback spike only if needed. | One shell selected; OS/runtime prerequisites and realistic resource budgets recorded. |

P0 must include all stages, not just layout time. Test transferring the result, preparing paths, painting, selecting, searching, and switching away mid-render. If global layout is the failing stage, insert the narrow structural-adapter work in Architecture §3C and repeat the benchmark before leaving P0. If no approach passes, record an unmet requirement rather than relabeling stress handling as huge-diagram support.

**Stop condition for P0:** enough evidence to select a feasible path or identify a specific failed promise. Avoid an open-ended framework comparison; compare alternatives only against the failed gate.

### P1 — Ship the navigation spine internally

**Depends on:** P0 shell/protocol/extraction decisions.

- Implement native folder/file open and one active workspace with recent-workspace memory.
- Add progressive discovery, ignore handling, virtualized tree, directory pruning after scan completion, watch reconciliation, and missing-root recovery.
- Persist metadata and heading/block index in SQLite. Add stable file/diagram identities.
- Build the quick switcher with bounded candidate ranking, keyboard control and incomplete-index status.
- Add diagram rail, history, pinning, previous/next, camera restoration and bounded artifact cache.

**Exit:** navigate the 100,000-entry corpus while indexing and while a helper is intentionally hung; meet NAV acceptance and search/cached-switch budgets. This can use a simple preview but must use the real desktop filesystem path.

### P2 — Complete the source-to-preview loop

**Depends on:** P1 buffers/identity and P0 rendering.

- Integrate CodeMirror transactions, Mermaid highlighting/diagnostics, block/full-file modes and reversible source maps.
- Implement explicit save, dirty buffers across navigation, optional autosave, recovery journal, external-change conflict handling, and crash restoration.
- Add latest-revision render scheduling, adaptive debounce, stale-response rejection, status copy and retained valid previews.
- Integrate bounded SVG path, font-ready measurement, per-artifact IDs, strict configuration and safe assets.
- Plumb the Authored/Warm preset through render configuration and cache keys (Architecture §3D), with provisional token values.
- Add the accessible text outline for the SVG path.
- Implement fit/zoom/pan, source/preview toggle, active-diagram search and revision-aware export.

**Exit:** open → choose block → edit → render → error → repair → save → external conflict → restart works without losing data. Verify correct source lines, unchanged Markdown bytes, no camera reset on ordinary edits, and no stale artifact replacing the current diagram.

**Milestone M1 — core loop:** this exit is the first point where the README goal is demonstrable. Run it as a packaged build on Windows, macOS and Linux with the everyday and large tiers. It does not depend on the huge-diagram work.

### P3 — Make huge diagrams a supported experience

**Depends on:** P0 scene proof; P2 revision/supervision interfaces.

- Productionize scene normalization, feature preflight, font shaping, paint-order parity, conservative bounds and chunked transfer.
- Add spatial culling, overview/detail levels, bounded path preparation, geometry hit testing, label search and text outline.
- Implement resource accounting, helper termination, cancellation under memory pressure and cache eviction.
- Verify the large corpus for every supported diagram family; publish feature/family limits.
- Complete any structural layout optimization demonstrated necessary by P0. Never partition across semantic constraints silently.

**Exit:** huge flowchart corpus meets the PRD latency/memory/interaction budgets with faithful output; unsupported-feature states are tested; no silent missing labels, arrows, nodes or relationships. Switching away during huge work remains fast.

### P4 — Apply the crafted visual language

**Depends on:** functioning navigation, editing and large presentation. Token/font plumbing should start in P1; final visual refinement happens here.

- Apply warm/light/dark tokens, bundled typefaces, rhythm, rounded controls, meaningful line weights and low visual noise.
- Tune Authored/Warm preset values and verify the style-precedence policy; the plumbing landed in P2.
- Polish compact diagram rail, empty/loading/error/conflict states, resize behavior, focus, menus and keyboard help.
- Add graph-aware Trace and source navigation for tested flowchart mappings as a next-priority enhancement if capacity permits; avoid holding core release for unsupported diagram families.
- Offer an explicit undoable role-class helper only after source integrity tests exist.
- Conduct 5–8 user sessions on finding diagrams, recognizing freshness, editing and understanding dense flows.

**Exit:** design acceptance sizes/scales/themes pass; primary tasks meet the formative usability threshold; users distinguish dirty, saved, rendering and old-preview states.

### P5 — Harden and distribute

**Depends on:** all release gates above.

- Run sustained switching/editing, filesystem stress, power/resume, high-DPI and GPU-fallback tests.
- Validate native keyboard/IME/clipboard/accessibility and OS-specific filesystem cases.
- Package Windows installer, signed/notarized macOS app and DMG, and declared Linux formats. Start with AppImage plus `.deb`; do not promise untested distributions.
- Verify helper and font distribution, license notices, clean-machine installation, uninstall and offline runtime prerequisites.
- Produce benchmark report, compatibility matrix, known limitations, recovery documentation and release checklist.
- OS default associations are optional after validation; `.md` association is always explicit because it affects non-diagram Markdown files.

**Exit:** all declared supported platform builds install and pass native smoke tests; every release blocker in the PRD/Test Plan is closed or the release is held. Do not call a debug-browser-only build a desktop release.

## 3. Critical path and schedule assumptions

Critical path: **P0 renderer isolation + faithful huge scene → P1 file identities/navigation → P2 source correctness → P3 huge support → P4 usability → P5 native release**. Some visual design and fixture work can overlap, but that does not remove the gates.

From P1 onward, every phase exit runs the native smoke scenario on packaged builds for all three OS families. P5 hardens distribution; it is not the first cross-platform check.

Planning estimate for one experienced desktop/full-stack engineer with part-time design/testing support:

| Phase | Initial estimate |
|---|---|
| P0 | 2–3 weeks |
| P1 | 2 weeks |
| P2 | 2–3 weeks |
| P3 | 3–5 weeks |
| P4 | 2 weeks |
| P5 | 2–3 weeks |

Sequential total: approximately **13–18 weeks**, with meaningful prototype results earlier. A custom structural adapter or major cross-engine mismatch may add 3–6 weeks. These are rough planning estimates; re-estimate after P0. M1 at the end of P2 (about 6–8 weeks) delivers the core navigate/edit/preview goal; the huge-diagram gate (P0.5, P3 and any adapter) is about a third of the total and carries most of the schedule risk. Code-signing accounts, native runners, representative corpora and access to all three OS families are prerequisites for a public release, not reasons to claim cross-platform validation from one machine.

## 4. Proposed developer commands

Implement these entry points as the corresponding tooling exists:

```text
pnpm dev                  run the selected desktop shell in development
pnpm check                format, lint, frontend types and contract generation check
pnpm test                 meaningful frontend/core behavior tests
cargo test --workspace    Rust extraction/files/cache/scheduler tests
pnpm test:desktop         native desktop smoke workflow on the current OS
pnpm bench:generate       deterministic fixture corpus and manifest
pnpm bench:run            release-build latency/frame/memory measurements
pnpm bench:report         compare to baseline, include failures and variance
pnpm package              reproducible package for the current declared target
```

Separate browser component tests from actual native desktop tests. Add targeted property tests for extraction/source patches, graph invariants and scheduler races; avoid snapshotting every cosmetic implementation detail.

## 5. Risk register

| Risk | Early evidence | Response |
|---|---|---|
| Tauri helper behaves differently across engines | P0 hang/cancel/font/layout tests | Adjust helper design or select measured Electron fallback. |
| Large scene conversion changes meaning | Differential family fixtures and pixel/structure checks | Narrow supported profile or implement missing feature; never drop it. |
| Layout dominates huge latency | Stage breakdown, topology sweep | Compare actual layouts/settings; then components/SCC adapter if needed. |
| Upstream layout cannot place connected huge graphs in budget | P0.0 probe, connected-topology results | Try available layout engines/settings; adapter only where graphs split; otherwise an owner decision on the huge corpus or engine, recorded as an ADR. |
| UI freezes preparing a result | Transfer/path-preparation/main-thread traces | Chunk work, move preparation, reduce mounted DOM, cap detail. |
| External edits or Markdown patching damage content | Byte-preservation and race tests | Reversible mappings, conflict checks, recovery and safe write failure. |
| Folder watch limits/large indexing consume CPU | Event storm/overflow/idle tests | Coalescing, bounded queue, priority and adaptive reconciliation. |
| “Minimal” UI hides essential state | Freshness/error/conflict user tasks | Persistent concise status and progressive details. |

## 6. Decisions to record during implementation

Create short ADRs for shell/helper selection, pinned Mermaid and OS floors, supported scene subset, source-map strategy, save/recovery semantics, actual benchmark thresholds, and any structural layout adapter. Each records the measured problem, options considered, decision, and follow-up test. Do not leave critical product promises as undocumented assumptions.
