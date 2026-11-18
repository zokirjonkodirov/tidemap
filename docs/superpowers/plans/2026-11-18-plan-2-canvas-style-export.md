# Canvas Integration, Style System and Export Implementation Plan (plan 2 of 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The app renders flowcharts through `tidemap-diagram` with direct egui painting, id-based selection, a controllable style file and scene-based export; the old renderer, its child process and its dependencies are deleted.

**Architecture:** `crates/renderer` becomes a thin in-process job runner (`Engine`): one worker thread, latest-wins scheduling, cooperative cancel, a byte-bounded cache of `Arc<RenderArtifact>`. `src/paint.rs` draws a `Scene` with egui primitives and real text; `src/canvas.rs` keeps camera and input. Colours are resolved at paint time from a `StyleSheet`, so theme and palette changes never re-run layout.

**Tech Stack:** Rust 1.88, egui/eframe 0.33.3, `tidemap-diagram` (plan 1), usvg/resvg 0.47 and tiny-skia for PNG export only, `base64` for font embedding.

**Spec:** `docs/superpowers/specs/2026-11-18-own-engine-and-shell-redesign-design.md` (§5.5, §5.6, §5.7 "Not supported", §5.8, §5.9, §7 diagram palette).

**Depends on:** plan 1 complete (`cargo test -p tidemap-diagram` green).

## Global Constraints

- No SVG, raster or IPC in the preview path. `mermaid-rs-renderer`, `lyon`, `roxmltree`, `xmlwriter`, `bincode` leave `Cargo.toml`; `usvg`, `resvg`, `tiny-skia`, `base64` stay, used only by export.
- No child process: the `--render-worker` branch in `src/main.rs` is removed.
- One accent colour in chrome; the diagram uses only the palette in spec §7. Text on every swatch is `ink.primary` unless the author set `color`.
- No sharp corners: minimum radius 4 for any painted rect; node radius from `Metrics::radius`.
- No text painted below 13 px on screen at fit zoom (existing `MIN_LABEL_ZOOM` rule stays); below 5 px on-screen text height, text is not painted at all.
- Edit debounce 30 ms. Camera animation 160 ms ease-out. No animation of layout changes.
- PNG export limits unchanged: 8192 px per side, 16 megapixels.
- Tests: `cargo test -p tidemap-diagram`, `cargo test -p tidemap-renderer`, `cargo test -p tidemap`. Build check: `cargo build --release`.
- Visual check: every UI task ends by running the screenshot mode added in Task 5 and reading the PNG.
- Commit steps apply only if the owner chose `git init` in plan 1.

## Review Focus

1. **Theme switch and authored colours**: toggling light/dark repaints without a new layout job; a node with authored dark `fill` and no `color` gets light text (luminance-based), never unreadable dark-on-dark. Tests in Task 1.
2. **Bad `style.json`**: invalid JSON, missing fields, or absurd values (font size 0, negative gaps) never blank the canvas: values are clamped, missing fields take defaults, a parse error keeps the last good sheet and reports in the info popover. Tests in Tasks 1 and 8.
3. **Stale results**: switching file or diagram while a layout runs must never show the old diagram under the new name; a result is accepted only if job id and diagram id match the active ones. Tests in Task 3.
4. **Jump-to-source inside Markdown**: a diagram fenced inside a list item or blockquote (line prefixes) must select the right characters in the full-file editor, including with CRLF and non-ASCII before the block. Tests in Task 6.
5. **Huge diagram at fit zoom**: 10,000 nodes visible must not lay out 10,000 text galleys per frame; LOD drops text and simplifies shapes. Test in Task 4 (item counts per LOD), measured in Task 10.

---

## File structure

```text
crates/diagram/src/style.rs     StyleSheet, palette, Role, paint resolution (new)
crates/diagram/src/svg.rs       Scene → SVG string (new)
crates/model/src/lib.rs         RenderArtifact/RenderRequest/RenderEvent reshaped; Preset removed
crates/renderer/src/lib.rs      re-exports Engine
crates/renderer/src/engine.rs   worker thread, scheduling, cache (new)
crates/renderer/src/export.rs   SVG font embedding, PNG rasterisation (new)
crates/renderer/src/{worker,supervisor,scene,protocol}.rs   deleted
src/paint.rs                    Scene → egui shapes, LOD (new)
src/canvas.rs                   camera, input, selection; calls paint
src/minimap.rs                  minimap from Scene (new)
src/style_file.rs               load, clamp, watch style.json (new)
src/app.rs                      engine wiring, hold-last-good rule, prefetch, source sync
src/editor.rs                   select_range
src/main.rs                     worker branch removed, screenshot mode
docs/decisions/002-own-diagram-engine.md, README.md, docs/COMPATIBILITY.md, THIRD_PARTY_NOTICES.md
```

---

### Task 1: Style sheet and paint resolution

**Files:** Create `crates/diagram/src/style.rs`; re-export from `lib.rs`.

**Interfaces — Produces:**

```rust
pub type Rgba = [u8; 4];
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)] pub struct Swatch { pub fill: Rgba, pub stroke: Rgba }
#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum Role { Process, Decision, Store, Terminal, External, Other, Error }
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)] pub enum ColorBy { #[default] Role, Subgraph }
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)] #[serde(default)]
pub struct DiagramColors { pub canvas: Rgba, pub text: Rgba, pub edge: Rgba, pub label_bg: Rgba,
    pub sky: Swatch, pub peach: Swatch, pub mint: Swatch, pub yellow: Swatch, pub lavender: Swatch, pub gray: Swatch, pub rose: Swatch }
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)] #[serde(default)]
pub struct StyleSheet { pub metrics: Metrics, pub light: DiagramColors, pub dark: DiagramColors, pub color_by: ColorBy,
    pub edge_width: f32, pub arrow_size: f32, pub node_stroke: f32, pub cluster_tint: f32 }   // defaults 1.5, 7.0, 1.0, 0.35
#[derive(Clone, Copy, PartialEq, Debug)] pub struct Paint { pub fill: Rgba, pub stroke: Rgba, pub text: Rgba, pub stroke_width: f32, pub dashed: bool }
impl StyleSheet {
    pub fn clamped(self) -> StyleSheet;                       // forces every value into its allowed range
    pub fn colors(&self, dark: bool) -> &DiagramColors;
    pub fn layout_key(&self) -> u64;                          // hash of `metrics` only: changes ⇒ re-layout
}
impl DiagramColors { pub fn default_light() -> Self; pub fn default_dark() -> Self; }   // `Default` = light
pub fn role(shape: ShapeKind) -> Role;
pub fn node_paint(n: &NodeItem, scene: &Scene, s: &StyleSheet, dark: bool) -> Paint;
pub fn cluster_paint(c: &ClusterItem, scene: &Scene, s: &StyleSheet, dark: bool) -> Paint;
pub fn edge_paint(e: &EdgeItem, scene: &Scene, s: &StyleSheet, dark: bool) -> Paint;   // fill = label pill background
```

Exact values: `DiagramColors` light and dark defaults are the table in spec §7 (canvas `#FAFAF8` / `#1B1D20`, text `#1F2328` / `#E6E8EB`, edge `#8A919C` / `#7C848F`, label_bg `#FFFFFF` / `#24272B`, seven swatches as listed).

Role mapping: Rect, Rounded → Process (sky); Diamond, Hexagon → Decision (peach); Cylinder → Store (mint); Stadium, Circle, DoubleCircle → Terminal (yellow); Subroutine, Asymmetric → External (lavender); Parallelogram(+Alt), Trapezoid(+Alt) → Other (gray). A node carrying a class named `error`, `danger`, `fail` or `failed` (case-insensitive) with no authored fill → Error (rose).

Resolution order for a node: role swatch → each `classDef` in `classes` order → inline `style`. With `ColorBy::Subgraph`, the base swatch is instead the node's top-level cluster index mod 5 over [sky, mint, peach, lavender, yellow]; nodes outside any cluster use gray. Text colour: authored `color` if present; else if the final fill was authored, pick `#1F2328` or `#F5F6F7` by WCAG relative luminance of the fill (threshold 0.4); else `colors.text`. Cluster fill = base swatch for (depth-0 cluster index mod 5) blended toward `canvas` so that `cluster_tint` of the swatch remains; stroke = same blend of the swatch stroke at 0.6. Edge stroke = authored `linkStyle` stroke or `colors.edge`; width `edge_width`, ×1.6 for `LineKind::Thick`; `dashed` for Dotted.

Clamp ranges: font_size 11–24; line_height ≥ font_size × 1.2; max_label_width 80–600; pad_x, pad_y 4–48; radius 4–32; rank_gap, node_gap 16–240; edge_label_font 13–20; cluster_pad 8–64; cluster_title 20–48; width_step 1–96; edge_width 0.5–4; arrow_size 4–16; node_stroke 0.5–3; cluster_tint 0.05–1.

- [ ] **Step 1: Write failing tests** (helper `scene(src) -> Scene` = parse + layout with default metrics + `Scene::build`; define it once in a `#[cfg(test)] pub(crate) mod testutil` in `lib.rs` and reuse it in `svg.rs`)

```rust
fn wcag(a: Rgba, b: Rgba) -> f32 { /* relative-luminance contrast ratio */ }
#[test] fn default_palette_matches_spec_and_passes_aa() { let s = StyleSheet::default(); assert_eq!(s.light.sky, Swatch { fill: [0xDC,0xEB,0xFA,255], stroke: [0x8F,0xB8,0xE0,255] }); assert_eq!(s.light.canvas, [0xFA,0xFA,0xF8,255]); assert_eq!(s.dark.canvas, [0x1B,0x1D,0x20,255]);
    for c in [&s.light, &s.dark] { for sw in [c.sky, c.peach, c.mint, c.yellow, c.lavender, c.gray, c.rose] { assert!(wcag(c.text, sw.fill) >= 4.5); } assert!(wcag(c.text, c.label_bg) >= 4.5); } }
#[test] fn roles() { use ShapeKind::*; assert_eq!(role(Rect), Role::Process); assert_eq!(role(Diamond), Role::Decision); assert_eq!(role(Cylinder), Role::Store); assert_eq!(role(Stadium), Role::Terminal); assert_eq!(role(Subroutine), Role::External); assert_eq!(role(Trapezoid), Role::Other); }
#[test] fn authored_wins_and_text_stays_readable() { let sc = scene("flowchart TB\nclassDef k fill:#102030\nA:::k-->B\nstyle B fill:#ffffff,color:#ff0000\nC:::error"); let s = StyleSheet::default();
    let a = node_paint(&sc.nodes[0], &sc, &s, false); assert_eq!(a.fill, [0x10,0x20,0x30,255]); assert!(wcag(a.text, a.fill) >= 4.5);
    assert_eq!(node_paint(&sc.nodes[1], &sc, &s, false).text, [255,0,0,255]); assert_eq!(node_paint(&sc.nodes[2], &sc, &s, false).fill, s.light.rose.fill); }
#[test] fn dark_uses_dark_swatches_without_new_scene() { let sc = scene("flowchart TB\nA{ok}"); let s = StyleSheet::default(); assert_eq!(node_paint(&sc.nodes[0], &sc, &s, true).fill, s.dark.peach.fill); }
#[test] fn color_by_subgraph() { let sc = scene("flowchart TB\nsubgraph a\nA\nend\nsubgraph b\nB\nend\nC"); let s = StyleSheet { color_by: ColorBy::Subgraph, ..Default::default() };
    assert_eq!(node_paint(&sc.nodes[0], &sc, &s, false).fill, s.light.sky.fill); assert_eq!(node_paint(&sc.nodes[1], &sc, &s, false).fill, s.light.mint.fill); assert_eq!(node_paint(&sc.nodes[2], &sc, &s, false).fill, s.light.gray.fill); }
#[test] fn clamps_and_partial_json() { let s: StyleSheet = serde_json::from_str(r#"{"metrics":{"font_size":0,"node_gap":-5,"radius":0},"edge_width":99}"#).unwrap(); let s = s.clamped();
    assert_eq!(s.metrics.font_size, 11.0); assert_eq!(s.metrics.node_gap, 16.0); assert_eq!(s.metrics.radius, 4.0); assert_eq!(s.edge_width, 4.0); assert_eq!(s.metrics.rank_gap, 64.0); assert_eq!(s.light, DiagramColors::default_light()); }
#[test] fn layout_key_ignores_colours() { let a = StyleSheet::default(); let mut b = a.clone(); b.light.sky.fill = [0,0,0,255]; assert_eq!(a.layout_key(), b.layout_key()); b.metrics.node_gap = 50.0; assert_ne!(a.layout_key(), b.layout_key()); }
```

(`serde_json` as a dev-dependency of `tidemap-diagram`.)

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run → pass. **Step 5:** Commit.

---

### Task 2: SVG writer

**Files:** Create `crates/diagram/src/svg.rs`.

**Interfaces — Produces:** `pub fn to_svg(scene: &Scene, style: &StyleSheet, dark: bool, background: bool, font_family: &str, font_css: Option<&str>) -> String`

Output contract: root `<svg xmlns="http://www.w3.org/2000/svg" viewBox="-24 -24 {w+48} {h+48}" width height>`; optional `<style>{font_css}</style>`; background `<rect>` with the canvas colour when `background`; one `<g class="cluster|edge|node" data-id="{key}">` per item in paint order; shapes as `<path d>` built from `shape::outline`; edges as one `<path>` with `fill="none"` plus arrowheads as small closed `<path>`s (no `<marker>`); text as `<text>` with one `<tspan>` per line, `text-anchor="middle"`, `font-family="{font_family}"`, sizes from metrics; colours from the same `*_paint` functions as the canvas. All text content and attribute values XML-escaped (`& < > " '`). `LineKind::Invisible` edges are not written.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn well_formed_and_complete() { let sc = scene("flowchart TB\nsubgraph s[Group]\nA[One]-->|go|B{Two}\nend"); let svg = to_svg(&sc, &StyleSheet::default(), false, true, "Inter", None);
    assert!(svg.starts_with("<svg ") && svg.trim_end().ends_with("</svg>")); assert_eq!(svg.matches("class=\"node\"").count(), 2); assert_eq!(svg.matches("class=\"edge\"").count(), 1); assert_eq!(svg.matches("class=\"cluster\"").count(), 1);
    for t in [">One<", ">Two<", ">go<", ">Group<"] { assert!(svg.contains(t), "{t}"); } assert!(svg.contains("#DCEBFA") || svg.contains("#dcebfa")); assert!(svg.find("class=\"cluster\"").unwrap() < svg.find("class=\"node\"").unwrap()); }
#[test] fn escapes_text() { let sc = scene("flowchart TB\nA[\"a < b & \\\"c\\\"\"]"); let svg = to_svg(&sc, &StyleSheet::default(), false, false, "Inter", None); assert!(svg.contains("a &lt; b &amp;")); assert!(!svg.contains("a < b")); }
#[test] fn background_and_theme() { let sc = scene("flowchart TB\nA"); let s = StyleSheet::default(); assert!(to_svg(&sc, &s, true, true, "Inter", None).to_lowercase().contains("#1b1d20")); assert!(!to_svg(&sc, &s, false, false, "Inter", None).to_lowercase().contains("#fafaf8")); }
#[test] fn empty_scene_is_valid() { let svg = to_svg(&scene("flowchart TB"), &StyleSheet::default(), false, true, "Inter", None); assert!(svg.contains("viewBox=\"-24 -24 48 48\"")); }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run → pass. **Step 5:** Commit.

---

### Task 3: Engine (in-process job runner), built beside the old supervisor

**Files:** Create `crates/renderer/src/engine.rs`; modify `crates/renderer/src/lib.rs`, `crates/renderer/Cargo.toml` (add `tidemap-diagram`), `crates/model/src/lib.rs`, `crates/model/Cargo.toml` (add `tidemap-diagram`).

The old types stay compiling in this task; new types get new names and replace the old ones in Task 5.

**Interfaces — Produces:**

```rust
// crates/model
pub struct DiagramArtifact { pub id: String, pub diagram_id: String, pub revision: u64, pub source_hash: String,
    pub rendered: tidemap_diagram::Rendered, pub layout_key: u64, pub bytes: usize, pub duration_us: u64 }
pub struct DiagramRequest { pub job_id: u64, pub diagram_id: String, pub revision: u64, pub source: String }
pub enum EngineEvent { Completed { job_id: u64, diagram_id: String, result: Result<Arc<DiagramArtifact>, RenderDiagnostic> },
                       Exported { job_id: u64, result: Result<PathBuf, String> } }
// crates/renderer/src/engine.rs
pub struct Engine { /* tx to worker, rx of events, cache, style, last artifact per diagram_id */ }
impl Engine {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>, font: Arc<[u8]>, style: StyleSheet) -> Self;
    pub fn render(&mut self, request: DiagramRequest);      // cancels the running foreground job, replaces any queued one
    pub fn prefetch(&mut self, request: DiagramRequest);    // runs only when idle; dropped if a foreground job arrives; emits no event, fills the cache
    pub fn cached(&self, diagram_id: &str, source: &str) -> Option<Arc<DiagramArtifact>>;
    pub fn set_style(&mut self, style: StyleSheet);         // if layout_key changed: clear cache
    pub fn cancel(&mut self);
    pub fn poll(&mut self) -> Vec<EngineEvent>;
    pub fn is_busy(&self) -> bool;
    pub fn diagnostics(&self) -> serde_json::Value;          // cache bytes, entries, last duration_us
}
```

Decisions: one long-lived thread owning the `Measurer`; each job gets a fresh `Arc<AtomicBool>` cancel flag; `render` sets the previous flag. Cache key = `sha256(source)` + `layout_key`; byte-accounted LRU, limit 96 MiB (existing `CACHE_LIMIT`), size from `Scene::byte_size()` + source length. A cache hit emits `Completed` on the next `poll` without touching the thread. The seed passed to `tidemap_diagram::render` is the most recent artifact for the same `diagram_id` (kept in a small map, max 32 entries). `LayoutError::Budget(m)` → `RenderDiagnostic { kind: Budget, message: m, line: None }`; `Canceled` emits nothing. Source over 2 MiB → Budget diagnostic "Diagram source exceeds 2 MiB." without starting a job. A panic inside the job is caught with `catch_unwind` and reported as `DiagnosticKind::Crash` with message "The diagram engine hit an internal error."; the thread keeps running.

- [ ] **Step 1: Write failing tests** (`engine.rs`; helper `wait(&mut Engine) -> Vec<EngineEvent>` polls up to 2 s)

```rust
#[test] fn renders_and_reports_ids() { let mut e = engine(); e.render(req(1, "d1", "flowchart TB\nA-->B")); let ev = wait(&mut e);
    let EngineEvent::Completed { job_id, diagram_id, result } = &ev[0] else { panic!() }; assert_eq!((*job_id, diagram_id.as_str()), (1, "d1")); assert_eq!(result.as_ref().unwrap().rendered.scene.nodes.len(), 2); }
#[test] fn latest_wins() { let mut e = engine(); let big = bench_source(4000); e.render(req(1, "d1", &big)); e.render(req(2, "d2", "flowchart TB\nX")); let ev = wait(&mut e);
    assert!(ev.iter().all(|x| !matches!(x, EngineEvent::Completed { job_id: 1, .. }))); assert!(ev.iter().any(|x| matches!(x, EngineEvent::Completed { job_id: 2, .. }))); }
#[test] fn cache_hit_is_same_arc() { let mut e = engine(); e.render(req(1, "d", "flowchart TB\nA")); let a = first_ok(wait(&mut e)); e.render(req(2, "d", "flowchart TB\nA")); let b = first_ok(wait(&mut e)); assert!(Arc::ptr_eq(&a, &b)); assert!(e.cached("d", "flowchart TB\nA").is_some()); }
#[test] fn style_metrics_change_invalidates_colours_do_not() { let mut e = engine(); e.render(req(1, "d", "flowchart TB\nA")); wait(&mut e);
    let mut s = StyleSheet::default(); s.light.sky.fill = [1,2,3,255]; e.set_style(s.clone()); assert!(e.cached("d", "flowchart TB\nA").is_some()); s.metrics.node_gap = 80.0; e.set_style(s); assert!(e.cached("d", "flowchart TB\nA").is_none()); }
#[test] fn prefetch_fills_cache_silently() { let mut e = engine(); e.prefetch(req(9, "n", "flowchart TB\nN")); std::thread::sleep(Duration::from_millis(300)); assert!(e.poll().is_empty()); assert!(e.cached("n", "flowchart TB\nN").is_some()); }
#[test] fn budget_and_oversize() { let mut e = engine(); e.render(req(1, "d", &"x".repeat(2 * 1024 * 1024 + 1))); let ev = wait(&mut e); assert!(matches!(&ev[0], EngineEvent::Completed { result: Err(d), .. } if d.kind == DiagnosticKind::Budget)); }
#[test] fn unsupported_family_is_ok_with_empty_scene() { let mut e = engine(); e.render(req(1, "d", "sequenceDiagram\nA->>B: hi")); let a = first_ok(wait(&mut e)); assert_eq!(a.rendered.family, Family::Sequence); }
```

- [ ] **Step 2:** Run `cargo test -p tidemap-renderer` → fail. **Step 3:** Implement. **Step 4:** Run → pass; `cargo build --release` still succeeds with the old path intact. **Step 5:** Commit.

---

### Task 4: Scene painter with level of detail

**Files:** Create `src/paint.rs`; add `mod paint;` in `src/main.rs`.

**Interfaces — Produces:**

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum Lod { Full, NoText, Flat }
pub fn lod(zoom: f32, font_size: f32, visible_items: usize) -> Lod;
pub struct Emphasis<'a> { pub selected: Option<NodeId>, pub strong_nodes: &'a [NodeId], pub strong_edges: &'a [EdgeId], pub matches: &'a [NodeId] }
pub struct PaintStats { pub items: usize, pub texts: usize, pub lod: Lod }
pub fn paint_scene(painter: &egui::Painter, viewport: egui::Rect, camera: Camera, scene: &Scene, style: &StyleSheet, dark: bool, accent: egui::Color32, emphasis: &Emphasis, scratch: &mut Vec<ItemId>) -> PaintStats;
```

Rules: `lod` = `Flat` when `zoom · font_size < 2.5` or `visible_items > 6000`; `NoText` when `zoom · font_size < 5.0` or `visible_items > 2500`; else `Full`. Visible set comes from `scene.query(view_rect_in_world inflated by 32)`. Full: shape outline via `egui::Shape::Path`/`CubicBezierShape` built from `shape::outline` transformed by the camera, fill + stroke from `style::node_paint`; text via `painter.layout_no_wrap` per line at `font_size · zoom`, centred; edges as chained `CubicBezierShape` with stroke from `edge_paint`, dashed edges via `Shape::dashed_line` on a 16-segment flattening; arrowheads as filled triangles of `arrow_size · zoom` aligned to the end tangent, circle and cross ends drawn as such; edge label = rounded pill (radius 8·zoom, min 4) in `label_bg` + text; cluster = rounded rect radius 16·zoom + title at top-left inset 12, 13 px semibold-weight substitute (regular face, `ink` at full strength). NoText: same without any text and without label pills. Flat: nodes as `rect_filled` with radius 2, edges as a single line segment start→end, clusters as filled rects, no strokes.

Emphasis: when `selected` is `Some`, every item not in `strong_nodes`/`strong_edges` (and not the selected node) is painted with all colours multiplied to 28% opacity; the selected node gets a 2 px `accent` ring offset 3 px outside its outline; strong edges get `accent`-tinted stroke at 1.3× width. `matches` (search hits) get a 2 px accent ring at 60% opacity.

- [ ] **Step 1: Write failing tests** (run a headless frame: `egui::Context::default().run(RawInput { screen_rect: Some(rect 1200×800), ..Default::default() }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| { stats = paint_scene(ui.painter(), …) }); })`; fonts default)

```rust
#[test] fn lod_thresholds() { assert_eq!(lod(1.0, 14.0, 100), Lod::Full); assert_eq!(lod(0.3, 14.0, 100), Lod::NoText); assert_eq!(lod(0.1, 14.0, 100), Lod::Flat); assert_eq!(lod(1.0, 14.0, 3000), Lod::NoText); assert_eq!(lod(1.0, 14.0, 7000), Lod::Flat); }
#[test] fn full_paints_every_label() { let sc = scene("flowchart TB\nsubgraph s[G]\nA-->|x|B\nend"); let st = frame(&sc, Camera { x: 40.0, y: 40.0, zoom: 1.0 }); assert_eq!(st.lod, Lod::Full); assert_eq!(st.items, 4); assert_eq!(st.texts, 4); }
#[test] fn culls_offscreen() { let sc = scene(&chain_source(400)); let st = frame(&sc, Camera { x: 40.0, y: 40.0, zoom: 1.0 }); assert!(st.items < 60, "{}", st.items); }
#[test] fn big_overview_paints_no_text() { let sc = scene(&bench_source(10_000)); let zoom = 800.0 / sc.bounds.h.max(sc.bounds.w); let st = frame(&sc, Camera { x: 0.0, y: 0.0, zoom }); assert_ne!(st.lod, Lod::Full); assert_eq!(st.texts, 0); }
```

- [ ] **Step 2:** Run `cargo test -p tidemap paint` → fail. **Step 3:** Implement. **Step 4:** Run → pass. **Step 5:** Commit.

---

### Task 5: Switch the app to the engine; delete the legacy renderer

**Files:**
- Modify: `src/app.rs`, `src/canvas.rs`, `src/main.rs`, `crates/model/src/lib.rs`, `crates/renderer/src/lib.rs`, `crates/renderer/Cargo.toml`, `tools/xtask/src/main.rs` (only if it references the worker)
- Delete: `crates/renderer/src/worker.rs`, `supervisor.rs`, `scene.rs`, `protocol.rs`

**Interfaces — Consumes:** `Engine`, `DiagramArtifact`, `paint_scene`, `StyleSheet`.
**Produces:**

```rust
// src/canvas.rs
pub struct CanvasResponse { pub camera_changed: bool, pub clicked: Option<ItemId>, pub cleared: bool }
impl DiagramCanvas {
    pub selected: Option<NodeId>,                         // replaces selected_label
    pub fn show(&mut self, ui: &mut egui::Ui, artifact: Option<&Arc<DiagramArtifact>>, style: &StyleSheet, dark: bool, p: Palette) -> CanvasResponse;
    pub fn focus_node(&mut self, scene: &Scene, n: NodeId);   // replaces focus_label
}
pub fn fit_camera(bounds: Rect, viewport: egui::Vec2, min_zoom: f32) -> Camera;   // pure; body = existing `fit` maths
```

Changes:
- `crates/model`: remove `Preset`, `Scene`, `ScenePrimitive`, `SpatialNode`, `DiagramLabel`, `DiagramLink`, the old `RenderArtifact`, `RenderRequest`, `RenderEvent`, `Settings::preset`. Rename `DiagramArtifact`/`DiagramRequest`/`EngineEvent` to `RenderArtifact`/`RenderRequest`/`RenderEvent` so call sites keep their names. `ExportRequest` keeps `artifact`, `path`, `format`, `background`, `scale` and gains `dark: bool`.
- `src/app.rs`: `renderer: Engine`; `signature()` becomes the source text only (theme and preset no longer part of it); `Progress` handling and `render_stage` removed; `Completed` accepted only when `job_id` and `diagram_id` equal the active ones; the Authored/Warm preset control in Settings removed; export temporarily disabled with tooltip "Export returns in the next step" until Task 9.
- `src/canvas.rs`: overview texture, PNG decode thread, `IMAGE_DECODE_LOCK`, label search state removed; painting delegated to `paint_scene`; click → `scene.hit(world_point, 6.0 / zoom)`.
- `src/main.rs`: `--render-worker` branch removed. New dev mode: when env `TIDEMAP_SHOT=<png path>` is set and a file path is the first CLI argument, open that file, wait until its first artifact has been painted for two frames, request `egui::ViewportCommand::Screenshot`, save the image to the path, and exit. Optional `TIDEMAP_SHOT_SIZE=WxH` (default 1440x940) and `TIDEMAP_SHOT_THEME=dark`.
- `Cargo.toml` of renderer: drop `mermaid-rs-renderer`, `lyon`, `roxmltree`, `xmlwriter`, `bincode`.

- [ ] **Step 1: Write failing tests** in `src/canvas.rs`

```rust
#[test] fn fit_small_diagram_caps_zoom_and_centres() { let c = fit_camera(Rect { x: 0.0, y: 0.0, w: 200.0, h: 100.0 }, egui::vec2(1200.0, 800.0), 13.0 / 14.0); assert_eq!(c.zoom, 1.6); assert!((c.x - (1200.0 - 320.0) / 2.0).abs() < 0.5); }
#[test] fn fit_huge_diagram_keeps_text_readable_and_fits_width_if_possible() { let min = 13.0 / 14.0; let c = fit_camera(Rect { x: 0.0, y: 0.0, w: 1000.0, h: 9000.0 }, egui::vec2(1200.0, 800.0), min); assert!(c.zoom >= min); assert!(1000.0 * c.zoom <= 1200.0); }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Make the changes above; delete the four files.
- [ ] **Step 4:** `cargo build --release` → succeeds with no warnings about unused legacy code. `cargo tree -p tidemap | grep -E "mermaid-rs-renderer|lyon|bincode|roxmltree"` → no output. `cargo test --workspace` → pass.
- [ ] **Step 5:** Visual check: `TIDEMAP_SHOT=<scratch>/t5.png target/release/tidemap.exe examples/system.mmd`, then read the PNG. Expected: pastel rounded nodes, a tinted "Your local workspace" container with its title top-left, smooth grey edges with arrowheads, crisp text. Repeat with `crates/diagram/tests/fixtures/llm_architecture.mmd`.
- [ ] **Step 6:** Commit.

---

### Task 6: Selection, neighbour highlight, source sync, search, outline

**Files:** Modify `src/app.rs`, `src/canvas.rs`, `src/editor.rs`; pure helpers in a new `src/source_map.rs`.

**Interfaces — Produces:**

```rust
// src/source_map.rs — map between a diagram block's own source and the text shown in the editor
pub fn block_to_editor_chars(block: &DiagramBlock, full_file: bool, file_content: &str, span: Span) -> Option<(usize, usize)>;  // char range in the editor buffer
pub fn editor_char_to_block_byte(block: &DiagramBlock, full_file: bool, file_content: &str, editor_char: usize) -> Option<u32>;
// src/editor.rs
impl SourceEditor { pub fn select_range(&mut self, chars: (usize, usize)); }   // scrolls into view and selects; applied on the next `show`
```

`block_to_editor_chars`: in diagram-only mode the editor buffer is `block.source`, so convert byte span → char indices directly. In full-file mode each block line `i` lives at file bytes `block.lines[i].start..end` after its `prefix`; find the line containing `span.start` by walking `block.source` line starts, add the offset within the line to `lines[i].start + prefix.len()`, do the same for `span.end` (clamped to the end of its line), then convert file byte offsets → char indices. The reverse function inverts this and returns `None` outside the block or inside a prefix.

Behaviour: click node → `canvas.selected = Some(id)`, `Emphasis` built from `scene.neighbours(id)`; if the source panel is open, `editor.select_range(node span)`. Click edge → `canvas.selected` stays `None`; the edge's span is selected in the editor and the edge plus both endpoint nodes are emphasised (rest dimmed). Click empty canvas or Esc → clear. When the editor cursor moves (not on every frame: only when `EditorResponse::cursor` changed) and maps to a node via `scene.node_at_offset`, that node becomes selected without moving the camera. Diagram search (`Ctrl+F` with canvas focus) uses `scene.find`; Enter cycles matches with `focus_node`. The outline panel lists `scene.nodes` labels in reading order; click → `focus_node`. `source_line_for_label` and `last_source_jump` are deleted.

- [ ] **Step 1: Write failing tests** (`src/source_map.rs`; build `DiagramBlock`s with `tidemap_core::source::extract_diagrams`)

```rust
#[test] fn standalone_mmd_maps_directly() { let src = "flowchart TB\nAlpha[Привет] --> Beta"; /* block.source == src */ let r = block_to_editor_chars(&block, false, src, span_of(src, "Beta")).unwrap(); assert_eq!(chars(src, r), "Beta"); }
#[test] fn markdown_fence_full_file() { let md = "# T\n\n```mermaid\nflowchart TB\nA[One] --> B\n```\n"; let b = &extract_diagrams(md)[0]; let r = block_to_editor_chars(b, true, md, span_of(&b.source, "A[One]")).unwrap(); assert_eq!(chars(md, r), "A[One]"); }
#[test] fn fence_inside_list_and_quote_with_crlf_and_unicode_before() { let md = "Žluťoučký 🚀\r\n\r\n- item\r\n  > ```mermaid\r\n  > flowchart TB\r\n  > A[One] --> B[Two]\r\n  > ```\r\n"; let b = &extract_diagrams(md)[0];
    let r = block_to_editor_chars(b, true, md, span_of(&b.source, "B[Two]")).unwrap(); assert_eq!(chars(md, r), "B[Two]");
    let back = editor_char_to_block_byte(b, true, md, r.0).unwrap(); assert_eq!(&b.source[back as usize..back as usize + 6], "B[Two]"); }
#[test] fn outside_block_is_none() { let md = "intro\n```mermaid\nflowchart TB\nA\n```\n"; let b = &extract_diagrams(md)[0]; assert_eq!(editor_char_to_block_byte(b, true, md, 1), None); }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement helpers and wiring. **Step 4:** Run → pass.
- [ ] **Step 5:** Manual check in the running app with `examples/welcome.md` in full-file mode: click a node → its text is selected in the editor; put the cursor on another node's line → that node highlights; click empty canvas → dimming clears.
- [ ] **Step 6:** Commit.

---

### Task 7: Typing behaviour, unsupported card, neighbour prefetch

**Files:** Modify `src/app.rs`, `src/canvas.rs`.

**Interfaces — Produces:** `pub fn should_show(new_diag_count: usize, have_clean_previous: bool, since_last_edit: Duration) -> bool` in `src/app.rs` (pure).

Rules:
- Debounce 30 ms flat (remove the 180/450 ms adaptive values).
- Hold-last-good: a completed artifact with diagnostics is shown immediately only if there is no earlier diagnostic-free artifact for this diagram; otherwise it is held and shown once 1.2 s pass without an edit. `should_show(0, _, _) = true`; `should_show(n>0, false, _) = true`; `should_show(n>0, true, d) = d >= 1200 ms`. The info popover shows the diagnostics at once in every case.
- Camera: a new artifact for the same diagram never refits; it keeps the world point under the viewport centre. Only a different diagram (or first render) fits. If the previous selection's node key still exists, it stays selected.
- Unsupported family (`rendered.family != Family::Flowchart`): the canvas shows a centred card (panel fill, radius 16, one floating shadow) with title "{Name} diagrams are not supported yet", body "Flowcharts render today. The source is still here to read and edit.", and one button "Open source". Names: Sequence, State, Class, ER; `Family::Other` → "This diagram type is not supported yet".
- `Budget`/`Crash` diagnostic with no artifact: same card style, title "This diagram is too large to draw" / "Something went wrong drawing this diagram", body = the diagnostic message.
- Prefetch: after a `Completed` for the active diagram and when `!engine.is_busy()`, call `engine.prefetch` for the next and previous block of the active file (if not `cached`). `activate()` first checks `engine.cached(...)` and, on a hit, installs the artifact in the same frame.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn hold_last_good_rule() { let ms = Duration::from_millis; assert!(should_show(0, true, ms(0))); assert!(should_show(2, false, ms(0))); assert!(!should_show(1, true, ms(300))); assert!(should_show(1, true, ms(1200))); }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run → pass.
- [ ] **Step 5:** Manual check: open `examples/welcome.md`; Alt+PageDown to the sequence diagram shows the card; back to the flowchart is instant; type `X --> ` (incomplete) in the flowchart: the diagram does not jump; after about a second the info dot appears and the partial diagram shows; finish the line: it updates within a frame or two and nothing else moves.
- [ ] **Step 6:** Commit.

---

### Task 8: `style.json`, hot reload, settings

**Files:** Create `src/style_file.rs`; modify `src/app.rs` (Settings dialog, poll), `crates/core/src/lib.rs` only if `default_data_dir` is not already public.

**Interfaces — Produces:**

```rust
pub struct StyleFile { pub path: PathBuf, pub sheet: StyleSheet, pub error: Option<String>, /* mtime, last_check */ }
impl StyleFile {
    pub fn load(dir: &Path) -> StyleFile;                 // creates style.json with pretty-printed defaults if missing
    pub fn poll(&mut self, now: Instant) -> bool;          // checks mtime at most once per second; true when `sheet` changed
    pub fn save(&mut self, sheet: &StyleSheet) -> Result<(), String>;   // temp file + rename
    pub fn reset(&mut self) -> Result<(), String>;
}
```

Rules: file is `<app data dir>/style.json`. Parse with `serde_json`, then `.clamped()`. On a parse error: keep the previous `sheet`, set `error` to "style.json line {l}: {message}"; clear `error` on the next good load. `poll() == true` → `engine.set_style(sheet.clone())` and, if `layout_key` changed, `schedule_render`. Settings dialog gains a "Diagram style" card: colour by (Role / Subgraph) segmented control, "Open style file" (opens with the OS default handler), "Reset to defaults" (confirmation inline), and the error line when present. The same error is listed in the info popover.

- [ ] **Step 1: Write failing tests** (temp dir under `std::env::temp_dir()` with a unique name)

```rust
#[test] fn creates_defaults_when_missing() { let d = tmp(); let f = StyleFile::load(&d); assert!(d.join("style.json").exists()); assert_eq!(f.sheet, StyleSheet::default()); assert!(f.error.is_none()); }
#[test] fn bad_json_keeps_last_good_and_reports_line() { let d = tmp(); let mut f = StyleFile::load(&d); let mut s = StyleSheet::default(); s.metrics.node_gap = 72.0; f.save(&s).unwrap();
    std::fs::write(d.join("style.json"), "{\n  \"metrics\": { oops\n}").unwrap(); bump_mtime(&d.join("style.json")); assert!(!f.poll(later())); assert_eq!(f.sheet.metrics.node_gap, 72.0); assert!(f.error.as_deref().unwrap().starts_with("style.json line 2")); }
#[test] fn edit_is_picked_up_and_clamped() { let d = tmp(); let mut f = StyleFile::load(&d); std::fs::write(d.join("style.json"), r#"{"metrics":{"font_size":100}}"#).unwrap(); bump_mtime(&d.join("style.json")); assert!(f.poll(later())); assert_eq!(f.sheet.metrics.font_size, 24.0); assert!(f.error.is_none()); }
#[test] fn poll_is_rate_limited() { let d = tmp(); let mut f = StyleFile::load(&d); let t = Instant::now(); f.poll(t); std::fs::write(d.join("style.json"), r#"{"edge_width":3}"#).unwrap(); assert!(!f.poll(t + Duration::from_millis(200))); }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run → pass.
- [ ] **Step 5:** Manual check: with the app open, change `"radius"` to 20 and `light.sky.fill` in the file and save: corners change after a re-layout, colour changes with no layout job (render time in the info popover does not change).
- [ ] **Step 6:** Commit.

---

### Task 9: Export from the scene

**Files:** Create `crates/renderer/src/export.rs`; modify `engine.rs` (`pub fn export(&mut self, request: ExportRequest)`), `src/app.rs` (re-enable export).

**Interfaces — Produces:**

```rust
pub fn font_css(family: &str, font: &[u8]) -> String;     // @font-face with base64 data URI, format('truetype')
pub fn svg_bytes(a: &RenderArtifact, style: &StyleSheet, dark: bool, background: bool, family: &str, font: &[u8]) -> Vec<u8>;
pub fn png_bytes(a: &RenderArtifact, style: &StyleSheet, dark: bool, background: bool, scale: f32, family: &str, font: &[u8]) -> Result<Vec<u8>, String>;
```

Rules: PNG = `to_svg(.., font_css: None)` parsed by usvg with a `fontdb` containing only the bundled font and external image/font resolvers disabled, rendered by resvg at `scale`, encoded by tiny-skia. Before rendering: if either side × scale > 8192 or pixels > 16,000,000 → `Err("Image would be {w} × {h} px. Lower the scale or export SVG.")`. Export runs on its own short-lived thread; result arrives as `RenderEvent::Exported`; the file is written with the existing temp-then-rename helper (move `write_export` from the deleted supervisor into `export.rs`). Export is disabled, with a tooltip, when the active artifact is an unsupported family or has zero nodes.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn svg_embeds_font_and_text() { let a = artifact("flowchart TB\nA[Hello]-->B"); let s = String::from_utf8(svg_bytes(&a, &StyleSheet::default(), false, true, "Inter", FONT)).unwrap(); assert!(s.contains("@font-face") && s.contains("base64,") && s.contains(">Hello<")); }
#[test] fn png_has_expected_size_and_is_not_blank() { let a = artifact("flowchart TB\nA[Hello]-->B"); let png = png_bytes(&a, &StyleSheet::default(), false, true, 2.0, "Inter", FONT).unwrap(); let img = image::load_from_memory(&png).unwrap().into_rgba8();
    let b = a.rendered.scene.bounds; assert_eq!(img.width(), ((b.w + 48.0) * 2.0).ceil() as u32); let distinct: std::collections::HashSet<_> = img.pixels().map(|p| p.0).collect(); assert!(distinct.len() > 20); }
#[test] fn png_text_is_rendered_not_dropped() { /* render "A[IIIIIIII]" and "A[        ]" at same node size: pixel buffers must differ inside the node rect */ }
#[test] fn png_limit() { let a = artifact(&chain_source(300)); let e = png_bytes(&a, &StyleSheet::default(), false, true, 16.0, "Inter", FONT).unwrap_err(); assert!(e.contains("Lower the scale")); }
#[test] fn transparent_background() { let a = artifact("flowchart TB\nA"); let png = png_bytes(&a, &StyleSheet::default(), false, false, 1.0, "Inter", FONT).unwrap(); let img = image::load_from_memory(&png).unwrap().into_rgba8(); assert_eq!(img.get_pixel(0, 0).0[3], 0); }
```

(`image` with the `png` feature as a dev-dependency of the renderer crate.)

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run → pass.
- [ ] **Step 5:** Manual check: export `examples/system.mmd` as SVG and PNG from the app; open both; they match the canvas.
- [ ] **Step 6:** Commit.

---

### Task 10: Minimap, large-diagram check, documentation

**Files:** Create `src/minimap.rs`, `docs/decisions/002-own-diagram-engine.md`; modify `src/canvas.rs`, `README.md`, `docs/COMPATIBILITY.md`, `THIRD_PARTY_NOTICES.md`, `tools/xtask/src/bench.rs`.

**Interfaces — Produces:**

```rust
pub fn minimap_visible(bounds: Rect, camera: Camera, viewport: egui::Vec2) -> bool;      // true when the diagram is larger than the viewport on either axis at this zoom
pub fn minimap_rect(viewport: egui::Rect, bounds: Rect) -> egui::Rect;                  // max 180 × 120, aspect-preserving, 16 px from the bottom-right corner
pub fn show_minimap(ui: &mut egui::Ui, viewport: egui::Rect, scene: &Scene, style: &StyleSheet, dark: bool, camera: &mut Camera, p: Palette) -> bool; // true if the camera moved
```

Minimap: panel-coloured rounded card (radius 12, floating shadow), nodes as flat role-coloured rects (skip nodes smaller than 1 px), clusters as tinted rects, no edges above 3,000 edges, an accent-outlined rounded rect for the viewport; click or drag recentres the camera. The flat geometry is cached as one `egui::Mesh` per (artifact id, dark) and reused every frame. `Settings::show_minimap` becomes "auto" behaviour: shown only when `minimap_visible`; the settings toggle hides it entirely.

Large-diagram check: add `frame` rows to `cargo native bench`: build the 10,000-node `layered_dense` scene, run `paint_scene` headless at fit-to-all zoom and at zoom 1.0, best of 5, print milliseconds. Targets: both under 8 ms. If the fit-zoom frame is over, cache the Flat-LOD geometry as a mesh keyed by artifact id (same approach as the minimap) and draw it with a camera transform.

Docs: ADR 002 records: own engine, in-process thread, legacy renderer and child process removed, flowchart-only support with the "not supported yet" state, tests reinstated for `crates/diagram`, measured timings copied from plan 1 "Measured" and this task. README "Rendering contract" and "Layout" sections rewritten to match. `docs/COMPATIBILITY.md` replaced by a short table: family → supported / not yet; flowchart syntax supported; known gaps (`click`, HTML labels beyond `<br>`, icons, authored themes). `THIRD_PARTY_NOTICES.md`: remove dropped crates, add `ttf-parser`.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn minimap_only_when_diagram_exceeds_view() { let b = Rect { x: 0.0, y: 0.0, w: 2000.0, h: 500.0 }; let v = egui::vec2(1200.0, 800.0); assert!(minimap_visible(b, Camera { x: 0.0, y: 0.0, zoom: 1.0 }, v)); assert!(!minimap_visible(b, Camera { x: 0.0, y: 0.0, zoom: 0.5 }, v)); }
#[test] fn minimap_rect_fits_corner() { let vp = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 800.0)); let r = minimap_rect(vp, Rect { x: 0.0, y: 0.0, w: 4000.0, h: 1000.0 }); assert!(r.width() <= 180.0 && r.height() <= 120.0); assert!((r.width() / r.height() - 4.0).abs() < 0.05); assert_eq!((r.right(), r.bottom()), (1184.0, 784.0)); }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement minimap and bench rows. **Step 4:** Run tests → pass; run `cargo native bench` → record the frame rows in ADR 002.
- [ ] **Step 5:** Write the docs.
- [ ] **Step 6:** Final check: `cargo test --workspace` green; `cargo build --release` clean; screenshot mode on `examples/system.mmd` and `llm_architecture.mmd` in light and dark; read all four PNGs and confirm rounded pastel nodes, readable text, no overlap.
- [ ] **Step 7:** Commit.
