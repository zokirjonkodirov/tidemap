# Test and benchmark plan

**Status:** proposed; no app exists and no performance or native-platform tests have run. This plan defines what implementation must demonstrate. Machine-readable initial budgets are in [benchmark-spec.json](benchmark-spec.json).

## 1. Measurement contract

Test optimized release builds with production assets and tracing normally disabled. Record git revision, toolchain/dependency lock hashes, OS, WebView version, GPU/driver, CPU, RAM, storage, display scale/refresh, power mode, dataset seed and effective rendering configuration.

Proposed physical reference machines:

| Platform | Baseline |
|---|---|
| Windows | Windows 11, Intel i5-1135G7-class 4-core CPU, 16 GiB RAM, NVMe, integrated GPU, WebView2. |
| macOS | Apple M1, 8 GiB RAM, internal SSD, WKWebView meeting the pinned Mermaid feature requirements. |
| Linux | Ubuntu 24.04 LTS, i5-1135G7-class CPU, 8 GiB RAM, NVMe, integrated GPU, supported WebKitGTK; test X11 and Wayland. |

Also build and smoke-test macOS Intel. Proposed OS floors are Windows 11, macOS 14.4, and Ubuntu 24.04; P0 verifies them against the exact pinned dependencies and records any change. Additional distributions/Windows ARM64 are not claimed supported until built and tested. Validate current Linux dependency availability instead of assuming an AppImage removes all platform requirements.

Use plugged-in balanced power, a 60 Hz 1,440 × 900 logical viewport, no debugger, and a known background-load policy for reference runs. Include separate battery, high-DPI and reduced-memory checks. Compare platforms against their own baseline and the common product targets; do not claim identical browser-engine behavior.

Definitions:

- **Cold app:** no app/helper processes; render/index caches cleared for the selected scenario. Record OS page-cache state. Do not claim cold-disk I/O unless deliberately controlled.
- **Warm app:** retained valid caches; no existing app process for startup tests.
- **Cached diagram switch:** already verified content and artifact in the bounded in-memory cache, not a disk read disguised as a memory hit.
- **First useful tree:** real eligible paths visible and selectable. Skeleton rows are not useful output.
- **Preview ready:** correct revision has completed the first meaningful paint. Instrument both handoff and paint/OS capture; resolved promises are insufficient evidence.
- **Input feedback:** actual selection/caret/cancel indication is visible, not merely receipt of the input event.
- **Memory:** entire app/helper/descendant process tree; report private/working-set metrics per OS plus the closest comparable total. Browser shared memory is identified, never omitted to improve the number.

Run at least 50 startup observations, 100 navigation/search/everyday render observations, and 100 huge-tier observations before a release candidate. During development, 30 huge observations are an explicitly preliminary estimate. Include cold/warm distributions, p50/p95/max, failures, timeout counts and confidence/variance. Discard only documented harness errors, never slow valid diagrams. Randomize fixture order with a reproducible seed.

Shared CI runners enforce correctness and broad regression alarms; stable reference hardware determines tight performance acceptance. A confirmed >10% p95 regression against the accepted baseline requires investigation even when the absolute budget still passes. A failed absolute target cannot be waived by an improved average. Record exceptions explicitly with requirement changes and evidence.

## 2. Corpus

Generate fixtures deterministically, retain manifests, and combine synthetic graphs with representative user-owned repositories when available. Real source never enters public reports without permission. Baseline work does not require the user to provide private content.

| Corpus | Contents | Purpose |
|---|---|---|
| Repo small | 1,000 entries, 100 eligible files, 500 diagrams. | Basic path/block behavior. |
| Repo release | 100,000 entries, 10,000 eligible `.md`/`.mmd`, 50,000 Mermaid blocks. | Discovery, indexing, search, navigation budgets. |
| Repo stress | 1 million entries, 100,000 eligible files, nested/ignored/generated content. | Bounded traversal, watch fallback, cancellation and memory; exploratory limits. |
| Everyday graphs | 20/40 and 200/400 node/edge flowcharts. | Editing and rendering. |
| Large graphs | 1,000 nodes/2,000 edges. | Full pipeline latency and geometry parity. |
| Huge release graphs | 10,000 nodes/20,000 edges; multiple topology seeds. | Required huge-viewer gate. |
| Stress graphs | 50,000 nodes/100,000 edges plus dense/pathological cases. | Time/memory caps, recovery and honest limitation states. |
| Long source | 1/5/20 MiB Markdown, many blocks, huge plain text, long single lines. | Editor/source map/index limits independent of graph complexity. |
| Family corpus | Each stable family enabled in the pinned Mermaid build. | Compatibility; use family-native counts and limits. |

Topology sweep: chain, balanced tree, wide layered DAG, multiple disconnected components, cycles/SCCs, compound groups, crossing-group edges, parallel edges, high-degree hubs, and heavily crossing layers. Vary label length, multiline text, Unicode/RTL/emoji, style definitions and edge labels. Dense graphs have separate edge budgets; node count alone does not define difficulty.

Family-specific examples include sequence participants/messages/notes/loops, class members/relations, ER entities/attributes/relations, Gantt tasks/date spans, and chart series/items. Publish the actual largest passing fixture for each family. No “all Mermaid types at 10,000 nodes” claim.

## 3. Functional and correctness suites

| Suite ID (`T-` prefix; distinct from PRD requirement IDs) | Scenarios | Blocking invariant |
|---|---|---|
| T-FS-01 discovery | Mixed extensions/case, empty directories, ignored diagrams, deep trees, unknown counts during scan. | Only eligible files and required ancestors visible; no premature pruning. |
| T-FS-02 identity | Rename, case-only rename, move, delete/recreate, duplicate names, Unicode names, UNC/network paths, lost volume. | Correct file/buffer identity; no source saved to the wrong file. |
| T-FS-03 watching | Burst changes, self-save, `.gitignore` updates, queue overflow, watch quota and sleep/resume. | No event loop storm; bounded reconciliation eventually matches disk. |
| T-MD-01 extraction | All supported CommonMark fence/container forms, CRLF/BOM, incomplete fences, fake fences in code, info strings, zero/many/duplicate blocks. | Correct spans and source lines; ordinary prose is not detected as a diagram. |
| T-MD-02 editing | Insert/remove/reorder blocks, heading changes, indentation, backtick content, undo across mode switches. | Re-extraction matches intended block; unchanged bytes preserved. |
| T-SAVE-01 safe write | External dirty/clean edits, deleted file, disk full, locks, read-only, temp-write failure, interrupted rename. | No known silent overwrite; both versions/recovery available; never destructive fallback. |
| T-SAVE-02 recovery | App/core/helper crash before/after save steps, restart with dirty buffers, corrupt recovery entry. | Recoverable text is offered; successful saves do not reappear as dirty erroneously. |
| T-REND-01 revision races | File A slow → file B fast; source revision 1 slow → revision 2 fast; close workspace mid-job; change preset/font. | Only the exact current job key can replace the preview. |
| T-REND-02 failure | Invalid syntax, helper hang, forced OOM/exit, giant returned SVG, malformed geometry. | Shell responsive, old preview labeled, cleanup/retry bounded. |
| T-REND-03 semantics | Every supported family, source styles/config, arrows, groups, labels, explicit roles. | No missing or direction-reversed relationships; source untouched by presentation. |
| T-SCENE-01 conversion | Transforms, text, markers, clip paths, masks, filters, rich labels, math, icons and assets. | Supported features match; unsupported ones are detected before silent loss. |
| T-CACHE-01 cache | Same mtime/size with changed contents; stale DB; corrupt files; version/font/config changes; eviction. | Invalid artifact never marked current; memory remains bounded. |
| T-EXP-01 export | Current/old preview revision, light/dark, font packaging, transparent background, extreme dimensions. | Clear revision and safe faithful output; bounded memory. |
| T-UI-01 commands | Tree/switcher/rail/history, editor focus, modifier differences, IME composition, clipboard. | Predictable navigation; canvas shortcuts never steal source typing. |
| T-VIEW-01 viewing | Fit on first open, camera retained across edits and file switches, trackpad/mouse/keyboard zoom, high DPI, reduced motion, GPU unavailable, distant-zoom label hint, active-diagram label search. | Camera never resets on ordinary edits; unreadable labels are never presented as readable; selected/search labels stay discoverable. |
| T-A11Y-01 access | Keyboard-only, VoiceOver, NVDA, Orca, contrast, 200% text, reduced motion. | Core workflow and accessible diagram outline usable; freshness not color-only. |

Use property-based generation for Markdown extract/patch round trips, graph connectivity/arrow invariants, cache identities, and scheduler interleavings. Use upstream examples under their licenses and compare with the exact pinned upstream renderer. Prefer these behavior checks over tests that simply repeat implementation logic.

Visual comparison has two layers: semantic checks for nodes/edges/text and raster comparison under controlled fonts/engine. Permit small antialiasing differences with reviewed thresholds; never use pixel tolerance to excuse missing text or geometry. Standard SVG and large-scene output of the same fixture must be compared directly, including paint order and fonts. Human review of representative giant overviews remains necessary.

## 4. Performance experiments

| Experiment | Method | Target / gate |
|---|---|---|
| Startup | OS launch to interactive shell, separately time fonts/helper warm-up. | Cold ≤1 s, warm ≤500 ms. |
| Warm project open | Verify retained metadata, reopen Repo release and paint selectable initial paths. | First paths ≤150 ms; stale metadata reconciles visibly. |
| Repository open | Clear metadata cache, open Repo release, timestamp initial paths and complete metadata scan. | First paths ≤500 ms; metadata scan ≤10 s. Content indexing reported separately. |
| Search | Script varied short/broad/specific queries while indexing and rendering. | Key-to-result p95 ≤50 ms; correct stable ranking. |
| Repeated switching | Replay 1,000 selections across cached/uncached files and blocks. | In-memory ≤100 ms; small uncached ≤400 ms; no wrong preview flashes. |
| Editing | Replay valid/incomplete/error/fix source changes with IME and rapid revisions. | Everyday last-keystroke-to-paint ≤800 ms including debounce; typing ≤50 ms. |
| Layout probe (P0.0) | Run pinned Mermaid parse/layout alone on the large and huge topology sweep, per target engine and available layout engine; record time and peak memory per topology, connected graphs separately. | No budget of its own. Decides whether the huge gate is reachable with upstream layout before scene work starts. |
| Large and huge | Time every pipeline stage; include full render, transfer, normalization, preparation, paint. | Large ≤3 s; huge ≤15 s for every declared release-fixture group, p95 over runs. |
| Pan/zoom | Fixed camera path at overview, dense center, sparse region and label-level zoom. | Standard p95 frame ≤16.7 ms, huge ≤33.3 ms; report missed-frame rates and longest stall. |
| Cancellation | Cancel during parse/layout/normalization/transfer/preparation/export; navigate away. | UI feedback ≤50 ms, helper tree cleanup ≤500 ms; stale result ignored. |
| Post-cancel preview | Abandon a huge render by navigating to a small uncached diagram. | Correct preview ≤1 s, including helper restart or spare hand-over. |
| Memory/soak | Switch/edit for 60 min; evict/reload scenes; close workspace; sample process tree/GPU resources. | Idle ≤180 MiB, ordinary ≤350 MiB, huge peak ≤1 GiB. No growing retained-resource trend after equivalent cycles. |
| Idle/energy | 60 s idle after completed index; also watcher fallback and an inactive project. | <1% of one CPU core; no unnecessary redraw loop; fallback separately documented. |
| Package size | Inspect production archives and clean-machine prerequisites. | Target ≤35 MiB compressed excluding disclosed runtime prerequisites. |

Frame budget applies to the specified camera paths and detail policy, not an empty viewport. Capture visible-content counts, primitive counts and zoom levels so results cannot be improved by hiding the diagram. Every timeout is reported as a failure for its promised tier and as a bounded-failure result for stress tests.

For long sources, initial proposal: normal editing up to 5 MiB; above that offer a deliberate large-file mode with reduced syntax/index work, and preserve source access for bounded failure cases. Determine final thresholds from P0 rather than silently truncating source or disabling navigation.

## 5. Native tooling and platform matrix

Use Rust tests for filesystem/extraction/cache/scheduler logic and frontend component tests for UI state. Run native end-to-end tests against the packaged app. Tauri currently documents WebdriverIO's Tauri service with embedded driver support across Windows, Linux, and macOS; raw `tauri-driver` alone has a different platform matrix. Verify the pinned setup, ensure instrumentation is test-only, and perform manual native checks where automation cannot cover OS behavior. See [Tauri WebDriver documentation](https://v2.tauri.app/develop/tests/webdriver/).

Native smoke scenario on every supported target: install → open folder → switch Markdown blocks → edit → automatic preview → save → external update → export → cancel huge render → restart and recover. Also test offline first launch, missing/old runtime, fonts, file dialogs, helper lifecycle, multiple displays, scaling and permission denial. Browser-only tests cannot validate these.

## 6. Input boundaries

Test untrusted diagram input as data: script/callback directives, external URLs/images/fonts, SVG/CSS resource references, source-controlled security overrides, recursive references, deep nesting, oversized paths/images, and malformed encodings. Expected behavior is a safe render or specific diagnostic within bounded resources. No network requests, arbitrary app commands, or outside-workspace reads are permitted from source content.

Verify symlink traversal/races and explicit outside-root files separately. Stress tests use disposable fixture directories only. Cross-platform safe-save tests document filesystem limitations rather than assuming Linux rename behavior applies everywhere.

## 7. Release checklist and traceability

| Product promise | PRD requirements | Test suites / experiments | Phase |
|---|---|---|---|
| Fast project/diagram navigation | NAV-01–05, MD-03 | T-FS-01/02/03, T-UI-01, repository/search/switch experiments | P1 |
| Correct Markdown editing and safe iteration | MD-01/02, EDIT-01–04, FILE-01 | T-MD-01/02, T-SAVE-01/02, T-UI-01 | P2 |
| Faithful automatic preview | REND-01–04, EXP-01 | T-REND-01/02/03, T-SCENE-01, T-CACHE-01, T-EXP-01 | P2–P3 |
| Readable, stable viewing | VIEW-01/02 | T-VIEW-01, pan/zoom experiment | P2–P3 |
| Huge diagrams without shell freezes | REND-03/04, PRD §5 huge tier | Layout probe; huge, frame, memory, cancel and post-cancel experiments; stress recovery | P0 and P3 |
| Warm clear presentation and access | A11Y-01, DESIGN §8 | T-A11Y-01, visual parity, formative task tests | P2 outline; P4 |
| Reliable offline desktop distribution | SYS-01/02 | Native smoke including network-disabled run, package/runtime/license inspection | Every phase exit from P1; P5 |

Release blockers: data loss; source changed by viewing; stale artifact shown as current; silent missing diagram content; unbounded helper work; main-window hangs; failed declared huge corpus; inaccessible essential workflow; or missing native validation on a claimed platform. Publish measured results, compatibility limits, and known issues alongside the release.
