# Shell Redesign Implementation Plan (plan 3 of 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild the application chrome around the canvas: light, soft, Craft-like, with the diagram taking at least 75% of the window and every control in exactly one place.

**Architecture:** Chrome moves out of the 565-line `src/app.rs` into `src/ui/*` modules, one per surface, each a function over `&mut TidemapApp`. Decisions that can be wrong (tokens, contrast, layout mode, tree rows, breadcrumb, palette ranking, status dot) are pure functions with tests; pixels are checked with the screenshot mode from plan 2. Icons are painted vectors, not font glyphs.

**Tech Stack:** Rust 1.88, egui/eframe 0.33.3, Inter (OFL) static TTFs.

**Spec:** `docs/superpowers/specs/2026-11-18-own-engine-and-shell-redesign-design.md` (§6, §7, §9 done-when).

**Depends on:** plan 2 complete. Tasks 1–2 can run earlier; they touch only `src/theme.rs`, `src/icons.rs`, `src/main.rs`.

## Global Constraints

- Tokens are exactly the values in spec §7. No colour literal outside `src/theme.rs` and the diagram `StyleSheet`.
- One accent (`#41609E` light, `#8FAEE0` dark), used only for selection, the primary action and focus rings.
- Radii: controls 8, cards and panels 12, dialogs and palette 16, pill toolbar full. Nothing painted with radius 0.
- Borders: none by default. Where separation by space fails, a 1 px hairline at `ink` 8% (white 8% in dark).
- One shadow, floating surfaces only (pill, popovers, palette, dialogs, minimap): y 8, blur 24, `#14181C` at 14% light / 50% dark.
- Type: Inter. UI 14/20, small 13/18, source 13/21 mono. Sentence case. No text below 13 px. No all-caps.
- No Unicode symbol glyphs as icons (`◇ ☾ ⚙ ⌕ ▸ ●` …): every icon comes from `src/icons.rs`.
- Motion: 160 ms ease-out for panels, popovers and camera; none when the OS asks for reduced motion.
- Pointer targets ≥ 32 px.
- Minimum window 800 × 600. Below 1000 px wide: sidebar auto-collapses and the source panel replaces the canvas instead of sitting beside it.
- Each control exists once: zoom out, zoom level, zoom in, fit, toggle source, export live only in the floating pill (plus keyboard shortcuts and palette commands).
- Every existing feature keeps working: open folder/file, recents, pins, history, multi-block Markdown, save, save-copy, autosave, recovery, conflict dialog, external-change handling, export, diagram search, outline, settings.
- Tests: `cargo test -p tidemap`. Visual check after each task: `TIDEMAP_SHOT=<scratch>/tN.png target/release/tidemap.exe examples/system.mmd` at the default 1440x940, then read the PNG; repeat with `TIDEMAP_SHOT_THEME=dark`.
- Commit steps apply only if the owner chose `git init` in plan 1.

## Review Focus

1. **Long names and deep folders in a 260 px sidebar**: a 90-character file name ten folders deep stays readable: indentation stops growing after 6 levels, the name truncates with an ellipsis, the tooltip shows the full path. Tests in Task 4.
2. **Small and scaled windows**: at 800 × 600 and at 200% UI scale the pill does not run off-screen or cover the minimap, and the top bar does not clip its right-hand buttons. Tests in Tasks 3 and 5.
3. **Shortcuts versus typing**: single-key shortcuts (`F`, `1`, `Esc` behaviour) never fire while a text field has focus; Ctrl+P works from anywhere, including inside the editor. Test in Task 7.
4. **Empty states**: no workspace, a workspace with zero matching files, a file with no diagrams, a search with no results: each shows a calm, specific message and one next action, never a blank panel. Checked in Tasks 4 and 8.
5. **Existing settings files**: a settings file written by the current build (theme `System`, a `preset` field, `editor_width` from the old left-side panel) loads without error and keeps the user's theme; light-by-default applies only when no settings file exists. Test in Task 8.

---

## File structure

```text
assets/fonts/Inter-Regular.ttf, Inter-Medium.ttf, Inter-SemiBold.ttf, Inter-OFL.txt   new
assets/fonts/Manrope.ttf, Manrope-OFL.txt                                             removed
src/theme.rs        tokens, palette, contrast(), surface helpers
src/icons.rs        painted icons (new)
src/ui/mod.rs       frame(): decides layout mode, calls the surfaces in order (new)
src/ui/topbar.rs    slim bar and breadcrumb (new)
src/ui/sidebar.rs   search + tree (new)
src/ui/tree.rs      pure row building (new)
src/ui/pill.rs      floating toolbar (new)
src/ui/source.rs    source panel (new)
src/ui/info.rs      info popover + status dot (new)
src/ui/palette.rs   command palette (new)
src/ui/dialogs.rs   settings, export, help, recovery, conflict, close (moved, restyled)
src/ui/states.rs    welcome and empty states (new)
src/app.rs          state, events, commands only; no painting
```

---

### Task 1: Tokens, Inter, contrast

**Files:** Modify `src/theme.rs`, `src/main.rs`, `crates/renderer` font wiring (`Engine::new` receives Inter Medium), `THIRD_PARTY_NOTICES.md`; add Inter files; remove Manrope.

**Interfaces — Produces:**

```rust
pub mod token { // replaces the current values
    pub const SPACE: [f32; 6] = [4.0, 8.0, 12.0, 16.0, 24.0, 32.0];
    pub const RADIUS_CONTROL: f32 = 8.0; pub const RADIUS_PANEL: f32 = 12.0; pub const RADIUS_DIALOG: f32 = 16.0; pub const RADIUS_PILL: f32 = 999.0;
    pub const FONT_UI: f32 = 14.0; pub const FONT_SMALL: f32 = 13.0; pub const FONT_MONO: f32 = 13.0; pub const FONT_TITLE: f32 = 20.0;
    pub const BAR_HEIGHT: f32 = 40.0; pub const ROW_HEIGHT: f32 = 32.0; pub const CONTROL: f32 = 32.0;
    pub const SIDEBAR: f32 = 260.0; pub const SIDEBAR_MIN: f32 = 220.0; pub const SIDEBAR_MAX: f32 = 360.0;
    pub const SOURCE: f32 = 420.0; pub const SOURCE_MIN: f32 = 280.0; pub const INSET: f32 = 8.0;
    pub const EASE_SECONDS: f32 = 0.16; pub const NARROW: f32 = 1000.0;
}
pub struct Palette { pub app, pub panel, pub hover, pub ink, pub secondary, pub muted, pub hairline, pub accent, pub accent_soft, pub on_accent, pub danger, pub shadow: Color32 }
pub fn palette(dark: bool) -> Palette;
pub fn contrast(a: Color32, b: Color32) -> f32;        // WCAG ratio; translucent colours must be flattened by the caller
pub fn over(top: Color32, bottom: Color32) -> Color32;  // alpha-composite
pub fn floating(p: Palette, radius: f32) -> egui::Frame; // panel fill, the one shadow, no stroke
pub fn inset_panel(p: Palette) -> egui::Frame;           // panel fill, RADIUS_PANEL, no stroke, no shadow
pub fn font_medium() -> egui::FontFamily;  pub fn font_semibold() -> egui::FontFamily;
```

Values: light `app #FAFAF8`, `panel #FFFFFF`, `hover` ink at 5%, `ink #1F2328`, `secondary #5F6672`, `muted #6B7280`, `hairline` ink at 8%, `accent #41609E`, `accent_soft #E9EEF7`, `on_accent #FFFFFF`, `danger #A23B3B`, shadow `#14181C` at 14%. Dark `app #1B1D20`, `panel #24272B`, `hover` white at 6%, `ink #E6E8EB`, `secondary #A9B0BA`, `muted #9099A4`, `hairline` white at 8%, `accent #8FAEE0`, `accent_soft #2A3447`, `on_accent #14181C`, `danger #E19A9A`, shadow at 50%. The `canvas` and `success`/`line` fields are removed; the canvas colour comes from the diagram `StyleSheet`.

egui visuals: no widget strokes in any state (`bg_stroke` = `Stroke::NONE`) except keyboard focus, which is a 2 px accent ring; inactive widgets have transparent fill; hovered `hover`; active/selected `accent_soft`; text edits use `app` fill with radius 8 and no stroke. `ui.separator()` is not used anywhere after this plan. Fonts: Proportional = Inter Regular, named families `inter-medium`, `inter-semibold`; Monospace = IBM Plex Mono. Download the three static TTFs and `LICENSE.txt` from the Inter 4.x release at github.com/rsms/inter into `assets/fonts/`. `Engine::new` gets Inter Medium (diagram labels are medium weight per spec §7); `paint.rs` draws node labels with `font_medium()`, cluster titles with `font_semibold()`.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn tokens_match_spec() { let l = palette(false); assert_eq!(l.app, Color32::from_rgb(0xFA,0xFA,0xF8)); assert_eq!(l.ink, Color32::from_rgb(0x1F,0x23,0x28)); assert_eq!(l.accent, Color32::from_rgb(0x41,0x60,0x9E)); let d = palette(true); assert_eq!(d.app, Color32::from_rgb(0x1B,0x1D,0x20)); assert_eq!(d.panel, Color32::from_rgb(0x24,0x27,0x2B)); }
#[test] fn all_text_pairs_meet_aa() { for dark in [false, true] { let p = palette(dark); let hover_panel = over(p.hover, p.panel);
    for (fg, bg, name) in [(p.ink, p.app, "ink/app"), (p.ink, p.panel, "ink/panel"), (p.secondary, p.app, "secondary/app"), (p.secondary, p.panel, "secondary/panel"), (p.muted, p.app, "muted/app"), (p.muted, p.panel, "muted/panel"),
        (p.ink, p.accent_soft, "ink/accent_soft"), (p.accent, p.panel, "accent/panel"), (p.accent, p.accent_soft, "accent/accent_soft"), (p.on_accent, p.accent, "on_accent/accent"), (p.danger, p.panel, "danger/panel"), (p.ink, hover_panel, "ink/hover")] {
        assert!(contrast(fg, bg) >= 4.5, "{name} dark={dark}: {}", contrast(fg, bg)); } } }
#[test] fn contrast_known_values() { assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01); assert!((contrast(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 0.01); }
#[test] fn no_text_token_below_13() { assert!(token::FONT_SMALL >= 13.0 && token::FONT_MONO >= 13.0 && token::FONT_UI >= 13.0); }
```

- [ ] **Step 2:** Run → fail.
- [ ] **Step 3:** Implement. If a pair fails AA, darken (light) or lighten (dark) the foreground token in 2% lightness steps until it passes and record the final hex in the spec §7 table.
- [ ] **Step 4:** Run → pass. `cargo build --release` (call sites of removed fields updated minimally; full restyle follows in later tasks).
- [ ] **Step 5:** Screenshot → text is Inter, backdrop off-white, no brown anywhere. Commit.

---

### Task 2: Painted icons

**Files:** Create `src/icons.rs`.

**Interfaces — Produces:**

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon { Sidebar, Back, Forward, Search, Info, Sun, Moon, Settings, Plus, Minus, Fit, Code, Export, Folder, FolderOpen, File, Diagram, ChevronRight, ChevronDown, Pin, Close, Check, Warning }
pub const ALL: [Icon; 23] = [/* every variant */];
pub fn paint(painter: &egui::Painter, icon: Icon, rect: egui::Rect, color: egui::Color32);   // 16-unit grid scaled to rect, 1.5 px round-capped strokes
pub fn button(ui: &mut egui::Ui, icon: Icon, tooltip: &str, selected: bool, enabled: bool) -> egui::Response; // 32×32 hit area, 16 px glyph, radius 8, hover/selected fills from tokens
pub fn shapes(icon: Icon) -> Vec<Stroke16>;   // geometry in the 16-unit grid: polylines, arcs, circles
pub enum Stroke16 { Line(Vec<[f32;2]>), Circle([f32;2], f32), Arc([f32;2], f32, f32, f32), Rect([f32;4], f32) }
```

Design: outline style, all geometry inside a 1-unit margin (coordinates 1..15), stroke only (no fills) so every icon has the same visual weight. `Fit` is four corner brackets; `Code` is `</>`; `Export` is a tray with a down arrow; `Diagram` is two small rounded rects joined by a line; `Sidebar` is a rounded rect with a vertical divider at x=6.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn every_icon_has_geometry_inside_the_grid() { for i in ALL { let s = shapes(i); assert!(!s.is_empty(), "{i:?}"); for sh in s { for [x, y] in points_of(&sh) { assert!((1.0..=15.0).contains(&x) && (1.0..=15.0).contains(&y), "{i:?} {x},{y}"); } } } }
#[test] fn icons_are_distinct() { let mut seen = std::collections::HashSet::new(); for i in ALL { assert!(seen.insert(format!("{:?}", shapes(i))), "{i:?} duplicates another icon"); } }
#[test] fn button_is_32px() { /* headless frame; assert response.rect.size() == vec2(32.0, 32.0) */ }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run → pass.
- [ ] **Step 5:** Temporary dev check: paint `ALL` in a row at 16, 24 and 48 px in the welcome screen, screenshot, read it, confirm each icon is recognisable and evenly weighted; remove the row. Commit.

---

### Task 3: Frame, slim top bar, breadcrumb

**Files:** Create `src/ui/mod.rs`, `src/ui/topbar.rs`; modify `src/app.rs` (`update` calls `ui::frame`), `src/main.rs`.

**Interfaces — Produces:**

```rust
// ui/mod.rs
#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum SourceMode { Hidden, Side, Replace }
#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub struct FrameMode { pub sidebar: bool, pub source: SourceMode }
pub fn frame_mode(window_width: f32, sidebar_open: bool, source_open: bool, has_workspace: bool) -> FrameMode;
pub fn canvas_fraction(window: egui::Vec2, mode: FrameMode, sidebar_w: f32, source_w: f32) -> f32;   // canvas area / window area
pub fn frame(app: &mut TidemapApp, ctx: &egui::Context);
// ui/topbar.rs
pub struct Crumb { pub folder: String, pub file: String, pub block: Option<String>, pub position: Option<(usize, usize)> }
pub fn crumb(workspace_name: &str, path: &str, blocks: &[(String, String)], active_block: &str) -> Crumb;
pub fn show(app: &mut TidemapApp, ctx: &egui::Context, p: Palette);
```

Layout: the whole window is `p.app`. Top bar: 40 px, no fill of its own, no border. Left to right: `Sidebar` icon, `Back`, `Forward`, 12 px gap, breadcrumb. Right-aligned: `Info` (with status dot, Task 6), theme toggle (`Sun`/`Moon`), `Settings`. Nothing else: Save, pin and the "⌕ Ctrl P" button leave the bar (save is Ctrl+S and a palette command, shown as state in the info popover; pin moves to the sidebar row context menu and a palette command; search is the sidebar field and Ctrl+P).

Breadcrumb: `folder / file` in `secondary`, 13 px; `folder` is the file's parent directory relative to the workspace (workspace name when at the root); when the file has more than one diagram, ` · {block title}  {i} of {n}` follows and that part is a button opening a rounded menu of the file's diagrams. A 6 px accent dot follows the file name when the buffer is dirty. The breadcrumb truncates from the left (folder first) when space is short.

Panels: sidebar and source are `inset_panel` cards with `INSET` (8 px) margin from the window edge and from the canvas, so the off-white backdrop shows between them. Sidebar left; source **right**; canvas is the remainder with no frame. `frame_mode`: `sidebar = sidebar_open && has_workspace && window_width >= NARROW`; `source = Hidden` if closed, `Replace` if `window_width < NARROW`, else `Side`. Panel open/close animates width over 160 ms.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn mode_rules() { assert_eq!(frame_mode(1440.0, true, false, true), FrameMode { sidebar: true, source: SourceMode::Hidden }); assert_eq!(frame_mode(1440.0, true, true, true).source, SourceMode::Side);
    assert_eq!(frame_mode(900.0, true, true, true), FrameMode { sidebar: false, source: SourceMode::Replace }); assert!(!frame_mode(1440.0, true, false, false).sidebar); }
#[test] fn diagram_gets_three_quarters_on_open() { let m = frame_mode(1440.0, true, false, true); assert!(canvas_fraction(egui::vec2(1440.0, 940.0), m, token::SIDEBAR, token::SOURCE) >= 0.75);
    let m = frame_mode(1100.0, true, false, true); assert!(canvas_fraction(egui::vec2(1100.0, 700.0), m, token::SIDEBAR, token::SOURCE) >= 0.70); }
#[test] fn crumbs() { let b = vec![("a".to_string(), "Overview".to_string()), ("b".to_string(), "Order flow".to_string())];
    let c = crumb("proj", "docs/arch/orders.md", &b, "b"); assert_eq!((c.folder.as_str(), c.file.as_str()), ("docs/arch", "orders.md")); assert_eq!(c.block.as_deref(), Some("Order flow")); assert_eq!(c.position, Some((2, 2)));
    let c = crumb("proj", "system.mmd", &b[..1], "a"); assert_eq!(c.folder, "proj"); assert_eq!(c.block, None); assert_eq!(c.position, None); }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement; delete `top_bar` and `content` from `app.rs`.
- [ ] **Step 4:** Run → pass. Screenshot at 1440x940 and at `TIDEMAP_SHOT_SIZE=800x600`: bar is one quiet line, right buttons fully visible at 800 wide, no header block above the canvas.
- [ ] **Step 5:** Commit.

---

### Task 4: Sidebar tree

**Files:** Create `src/ui/tree.rs`, `src/ui/sidebar.rs`; remove `build_tree_rows`, `tree_rows`, `sidebar` from `app.rs`.

**Interfaces — Produces:**

```rust
// ui/tree.rs
pub struct TreeRow { pub path: String, pub name: String, pub depth: usize, pub directory: bool, pub expanded: bool, pub file_id: Option<String>, pub count: Option<usize> }
pub fn build(files: &[FileEntry], query: &str, collapsed: &BTreeSet<String>) -> Vec<TreeRow>;
pub fn indent(depth: usize) -> f32;      // 16 px per level, capped at 6 levels
// ui/sidebar.rs
pub fn show(app: &mut TidemapApp, ui: &mut egui::Ui, p: Palette);
```

`build`: directories before files at each level, each group sorted case-insensitively by name; a non-empty `query` matches case-insensitively against path and diagram titles, keeps all ancestor folders of a match, ignores `collapsed`, and drops folders with no matches; an empty query honours `collapsed`.

Sidebar content, top to bottom, 12 px inner padding: search field (radius 8, `app` fill, `Search` icon inside, placeholder "Search files"); 12 px gap; when pins exist and the query is empty, a "Pinned" group (13 px `muted` sentence-case label, then rows); then the tree with no "Explorer" label. Row: 32 px high, full-width hit area, `indent(depth)` then a 16 px icon (`ChevronRight`/`ChevronDown` for folders, `Diagram` for `.mmd`, `File` for Markdown) then the name, left-aligned, 14 px, `ink` for files and folders alike; truncated with an ellipsis; hover fill `hover`; the active file has an `accent_soft` pill (radius 8) and `accent` text in medium weight; trailing diagram count in `muted` 13 px only when > 1; trailing 6 px accent dot when dirty. Tooltip (full path) only when the name is truncated. Right-click on a file row: "Pin" / "Unpin", "Reveal in file manager". Footer row, 13 px `muted`: "{n} files" or "Indexing… {scanned}", with `Plus` (open folder) and `File` (open file) icon buttons right-aligned. The "Local files · Offline ready" line, the workspace title row and the collapse button inside the sidebar are removed (the collapse control is the top-bar `Sidebar` icon).

Empty states inside the panel: indexing → "Looking for diagrams…"; no files → "No Markdown or Mermaid files in this folder." with a text button "Open another folder"; query without results → "No files match “{query}”."

- [ ] **Step 1: Write failing tests**

```rust
fn f(path: &str) -> FileEntry { FileEntry { id: path.into(), path: path.into(), name: path.rsplit('/').next().unwrap().into(), ..Default::default() } }
fn names(rows: &[TreeRow]) -> Vec<String> { rows.iter().map(|r| format!("{}{}", "  ".repeat(r.depth), r.name)).collect() }
#[test] fn folders_first_sorted_case_insensitive() { let rows = build(&[f("zeta.md"), f("Docs/b.md"), f("docs2/a.md"), f("Alpha.mmd"), f("Docs/A.md")], "", &BTreeSet::new());
    assert_eq!(names(&rows), ["Docs", "  A.md", "  b.md", "docs2", "  a.md", "Alpha.mmd", "zeta.md"]); }
#[test] fn collapsed_hides_children_only_without_query() { let c: BTreeSet<String> = ["docs".to_string()].into(); let files = [f("docs/a.md"), f("docs/deep/b.md"), f("c.md")];
    assert_eq!(names(&build(&files, "", &c)), ["docs", "c.md"]); assert!(!build(&files, "", &c)[0].expanded); assert_eq!(names(&build(&files, "b.md", &c)), ["docs", "  deep", "    b.md"]); }
#[test] fn query_matches_titles_and_drops_empty_folders() { let mut a = f("docs/a.md"); a.titles = vec!["Order flow".into()]; let rows = build(&[a, f("other/x.md")], "order", &BTreeSet::new()); assert_eq!(names(&rows), ["docs", "  a.md"]); }
#[test] fn indent_caps() { assert_eq!(indent(0), 0.0); assert_eq!(indent(3), 48.0); assert_eq!(indent(6), 96.0); assert_eq!(indent(10), 96.0); }
#[test] fn deep_long_name_row_exists() { let p = format!("{}/{}.md", (0..10).map(|i| format!("d{i}")).collect::<Vec<_>>().join("/"), "n".repeat(90)); let rows = build(&[f(&p)], "", &BTreeSet::new()); assert_eq!(rows.last().unwrap().depth, 10); assert_eq!(rows.len(), 11); }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run → pass.
- [ ] **Step 5:** Screenshot with the `examples` folder open and with a scratch folder containing a 10-deep, 90-character file name: names left-aligned, active pill visible, long name ends in an ellipsis inside the panel. Commit.

---

### Task 5: Floating pill and source panel

**Files:** Create `src/ui/pill.rs`, `src/ui/source.rs`; remove `floating_toolbar`, `source_panel` from `app.rs`.

**Interfaces — Produces:**

```rust
pub fn pill_width() -> f32;                                                   // fixed content width of the pill
pub fn pill_pos(canvas: egui::Rect, minimap: Option<egui::Rect>) -> egui::Pos2; // centre-bottom of the canvas, 16 px up; shifted left to keep an 8 px gap from the minimap; clamped inside the canvas
pub fn show_pill(app: &mut TidemapApp, ctx: &egui::Context, canvas: egui::Rect, p: Palette);
pub fn show_source(app: &mut TidemapApp, ui: &mut egui::Ui, p: Palette);
```

Pill: `floating(p, RADIUS_PILL)`, height 44, inner padding 6; items: `Minus` · zoom label (56 px wide, 13 px, click → 100%) · `Plus` · 1 px hairline 16 px tall · `Fit` · `Code` (selected state when the source panel is open) · `Export`. All icon buttons from `icons::button`; tooltips carry the shortcut ("Fit · F", "Source · Ctrl+E"). Disabled state when there is no artifact. It is centred on the **canvas** rect, not the window, so it never sits under a panel. No text buttons.

Source panel: `inset_panel`, on the right, resizable by a 6 px invisible grab strip on its left edge (cursor changes; a hairline appears on hover), width persisted in `Settings::editor_width`, range `SOURCE_MIN..=window_width − 480`. Header row 36 px: "Source" in 13 px medium `secondary`; a two-option segmented control "Diagram" / "Full file" (radius 8 track in `app`, selected segment `panel` with no shadow) only for Markdown files; `Close` icon. Editor below with line numbers in `muted`, IBM Plex Mono 13/21. Footer removed: line count, encoding and newline style move to the info popover. In `SourceMode::Replace` the panel fills the canvas area and the pill stays visible above it.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn pill_centres_on_canvas() { let c = egui::Rect::from_min_size(egui::pos2(276.0, 40.0), egui::vec2(1156.0, 892.0)); let p = pill_pos(c, None); assert!((p.x - c.center().x).abs() < 0.5); assert_eq!(p.y, c.bottom() - 16.0); }
#[test] fn pill_avoids_minimap_and_stays_inside_at_800() { let c = egui::Rect::from_min_size(egui::pos2(0.0, 40.0), egui::vec2(800.0, 560.0)); let mm = egui::Rect::from_min_max(egui::pos2(604.0, 464.0), egui::pos2(784.0, 584.0)); let p = pill_pos(c, Some(mm));
    let left = p.x - pill_width() / 2.0; let right = p.x + pill_width() / 2.0; assert!(left >= c.left() + 8.0); assert!(right <= mm.left() - 8.0); }
#[test] fn pill_fits_narrowest_canvas() { assert!(pill_width() <= 800.0 - 180.0 - 16.0 - 16.0 - 8.0); }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run → pass.
- [ ] **Step 5:** Audit for duplicates: `grep -n "zoom_by\|canvas.fit()\|actual_size\|Dialog::Export\|editor_open=" src` → each action is triggered from `pill.rs`, the shortcut handler and the palette command list only. Remove anything else.
- [ ] **Step 6:** Screenshots: source closed; source open at 1440 (canvas pushed, not covered: the diagram's right edge is left of the panel); 800x600 with source open (panel replaces canvas). Commit.

---

### Task 6: Info popover and status dot

**Files:** Create `src/ui/info.rs`; remove `info_popover` and the toast-style status noise from `app.rs`.

**Interfaces — Produces:**

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum Dot { None, Unsaved, Attention }
pub fn dot(dirty: bool, saving: bool, external_change: bool, diagnostics: usize, style_error: bool) -> Dot;
pub fn save_line(dirty: bool, saving: bool, external_change: bool, read_only: bool) -> &'static str;
pub fn show(app: &mut TidemapApp, ctx: &egui::Context, anchor: egui::Rect, p: Palette);
```

`dot`: `Attention` (danger colour) when `external_change || diagnostics > 0 || style_error`; else `Unsaved` (accent) when `dirty || saving`; else `None`. The dot is 6 px, drawn at the top-right of the `Info` icon.

`save_line`: saving → "Saving…"; external_change → "This file changed on disk. Your edits are kept."; read_only → "Read-only file"; dirty → "Unsaved changes"; else "Saved".

Popover: `floating(p, RADIUS_PANEL)`, 340 px wide, anchored under the `Info` button, closes on Esc, on click outside, and on its `Close` icon. Sections separated by 12 px of space, no rules:
1. File: `save_line`; when dirty a primary button "Save" (accent fill, `on_accent` text, radius 8); when external change: "Review changes" and "Save a copy".
2. Diagram: "Drawn in {x.x} ms · {n} nodes · {m} links", or "Drawing…". If held by the hold-last-good rule: "Showing the last version that drew cleanly."
3. Problems (only when present): each diagnostic as "Line {n}: {message}", clickable → opens source and selects the span; max 5, then "and {k} more". Style-file error line if any.
4. Details, 13 px `muted`: "{lines} lines · UTF-8 · LF|CRLF".
5. Links row: "Outline", "Keyboard shortcuts", "Diagnostics".

Toasts remain only for one-off results (export finished, save failed) and use `floating` with radius 12 at the bottom-left of the canvas, auto-dismiss after 4 s.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn dot_priority() { assert_eq!(dot(false, false, false, 0, false), Dot::None); assert_eq!(dot(true, false, false, 0, false), Dot::Unsaved); assert_eq!(dot(false, true, false, 0, false), Dot::Unsaved);
    assert_eq!(dot(true, false, false, 2, false), Dot::Attention); assert_eq!(dot(false, false, true, 0, false), Dot::Attention); assert_eq!(dot(false, false, false, 0, true), Dot::Attention); }
#[test] fn save_lines() { assert_eq!(save_line(false, false, false, false), "Saved"); assert_eq!(save_line(true, false, false, false), "Unsaved changes"); assert_eq!(save_line(true, true, false, false), "Saving…"); assert_eq!(save_line(true, false, true, false), "This file changed on disk. Your edits are kept."); assert_eq!(save_line(false, false, false, true), "Read-only file"); }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run → pass.
- [ ] **Step 5:** Manual: introduce a syntax error → red dot appears, popover lists "Line n: …", clicking it selects the text in the source panel. Screenshot the open popover. Commit.

---

### Task 7: Command palette and shortcuts

**Files:** Create `src/ui/palette.rs`; remove `quick_results`, `quick_switcher`, `fuzzy_score` from `app.rs` (move `fuzzy_score` into `palette.rs`); modify the shortcut handler.

**Interfaces — Produces:**

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command { Save, SaveCopy, ToggleSource, ToggleSidebar, Fit, ActualSize, ExportDiagram, ToggleTheme, OpenFolder, OpenFile, PinFile, NextDiagram, PreviousDiagram, FindInDiagram, Settings, Shortcuts }
pub const COMMANDS: [(Command, &str, &str); 16] = [/* (command, title, shortcut text) */];
#[derive(Clone, PartialEq, Debug)]
pub enum Hit { File { file_id: String, name: String, path: String }, Diagram { file_id: String, title: String, path: String, index: usize }, Command(Command) }
pub fn results(query: &str, files: &[FileEntry], pins: &[String], recent: &[String], limit: usize) -> Vec<Hit>;
pub fn fuzzy_score(query: &str, text: &str) -> Option<i64>;
pub fn show(app: &mut TidemapApp, ctx: &egui::Context, p: Palette);
pub fn single_key_allowed(ctx: &egui::Context) -> bool;   // false while any text field has keyboard focus or a modal is open
```

`results`: a query starting with `>` searches `COMMANDS` titles only (empty remainder lists all commands in declaration order). Otherwise: empty query → `recent` files (most recent first, deduplicated), then pinned files not already listed, up to `limit`; non-empty → files scored on name (×3 weight) and path, diagrams scored on title; pinned files +5; ties broken by shorter path, then alphabetical; top `limit` (50).

Palette: centred horizontally, top at 18% of window height, 600 px wide (window width − 48 at most), `floating(p, RADIUS_DIALOG)`, a scrim of `ink` at 12% behind it. Input row 48 px with `Search` icon, 16 px text, placeholder "Find a file or diagram, or type > for commands". Results: 40 px rows, icon (`File`/`Diagram`/command-specific), title in `ink`, path or shortcut right-aligned in `muted` 13 px; selected row `accent_soft` with radius 8; max 9 visible rows then scroll. Up/Down move, Enter opens, Esc or scrim click closes, typing resets selection to the first row. No header, no footer hints.

Shortcuts: Ctrl+P palette; Ctrl+Shift+P palette pre-filled with `>`; Ctrl+E source; Ctrl+S save; Ctrl+Shift+S save copy; Ctrl+F find (editor when it has focus, else diagram); Ctrl+Shift+O open folder; Ctrl+O open file; Ctrl+B sidebar; Alt+Left/Right history; Alt+PageUp/PageDown diagram; `F` fit and `1` 100% only when `single_key_allowed`; Esc closes, in order: palette, dialog, info popover, diagram search, selection. Double-click on empty canvas fits.

- [ ] **Step 1: Write failing tests**

```rust
fn file(path: &str, titles: &[&str]) -> FileEntry { FileEntry { id: path.into(), path: path.into(), name: path.rsplit('/').next().unwrap().into(), titles: titles.iter().map(|t| t.to_string()).collect(), ..Default::default() } }
#[test] fn empty_query_lists_recent_then_pins() { let files = [file("a.md", &[]), file("b.md", &[]), file("c.md", &[])]; let r = results("", &files, &["c.md".into()], &["b.md".into(), "a.md".into(), "b.md".into()], 50);
    let ids: Vec<_> = r.iter().map(|h| match h { Hit::File { file_id, .. } => file_id.as_str(), _ => "" }).collect(); assert_eq!(ids, ["b.md", "a.md", "c.md"]); }
#[test] fn name_beats_path_and_diagram_titles_are_found() { let files = [file("orders/readme.md", &[]), file("docs/orders.md", &["Order flow", "Refunds"])]; let r = results("orders", &files, &[], &[], 50);
    assert!(matches!(&r[0], Hit::File { file_id, .. } if file_id == "docs/orders.md")); let r = results("refund", &files, &[], &[], 50); assert!(matches!(&r[0], Hit::Diagram { title, index: 1, .. } if title == "Refunds")); }
#[test] fn command_mode() { let files = [file("fit.md", &[])]; let r = results(">fit", &files, &[], &[], 50); assert_eq!(r[0], Hit::Command(Command::Fit)); assert!(r.iter().all(|h| matches!(h, Hit::Command(_)))); assert_eq!(results(">", &files, &[], &[], 50).len(), COMMANDS.len()); }
#[test] fn no_match_is_empty_not_everything() { assert!(results("zzzzqq", &[file("a.md", &["x"])], &[], &[], 50).is_empty()); }
#[test] fn single_keys_blocked_while_typing() { /* headless frame with a focused TextEdit: assert !single_key_allowed(ctx); frame without focus: assert single_key_allowed(ctx) */ }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run → pass.
- [ ] **Step 5:** Manual: Ctrl+P from inside the source editor opens the palette; typing `f` in the editor does not fit the canvas; `>theme` + Enter toggles the theme. Screenshot the open palette. Commit.

---

### Task 8: Dialogs, welcome, empty states, first-run theme

**Files:** Create `src/ui/dialogs.rs`, `src/ui/states.rs`; remove `dialogs`, `recovery_dialog`, `conflict_dialog`, `welcome`, `outline`, `diagram_search_overlay` from `app.rs` (moved and restyled); modify `crates/core` bootstrap only to expose whether a settings file existed.

**Interfaces — Produces:**

```rust
pub fn initial_theme(settings_file_existed: bool, stored: Theme) -> Theme;   // no file → Theme::Light; otherwise `stored`
pub fn dialog_frame(p: Palette) -> egui::Frame;                               // floating, RADIUS_DIALOG, 24 px padding
pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response;      // accent fill, on_accent text, radius 8, height 32
pub fn quiet_button(ui: &mut egui::Ui, text: &str) -> egui::Response;        // no fill until hover, ink text
```

Rules: every dialog uses `dialog_frame`, a 12% `ink` scrim, a 20 px semibold title in sentence case, body in 14 px, actions right-aligned with exactly one `primary_button`; destructive choices are `quiet_button` with `danger` text. No separators; sections are spaced 16 px. Settings: groups "Appearance" (theme: Light / Dark / Match system), "Diagram style" (from plan 2 Task 8), "Files" (autosave, include ignored files), "Canvas" (minimap). Export dialog: format segmented control (SVG / PNG), scale (1×, 2×, 3×) for PNG, background toggle, primary "Export".

States (`states.rs`), each centred in the canvas area, title 20 px semibold, one 14 px `secondary` sentence, one action:
- No workspace: "Open a folder of diagrams" / "Tidemap reads Markdown and .mmd files and keeps everything on this computer." / primary "Open folder", quiet "Open a file"; below, up to 5 recent workspaces as quiet rows with `Folder` icons.
- Workspace, no file selected: "Pick a file to start reading" / "Or press Ctrl+P to search." / none.
- File without diagrams: "No diagrams in this file yet" / "Add a fenced mermaid block to see it here." / quiet "Open source".
- Diagram search overlay: `floating` pill at top-centre of the canvas, 300 px field, "{i} of {n}" in `muted`, `Close` icon; "No matches" when empty.
- Outline: right-side `inset_panel` list of node labels, 32 px rows, same row style as the sidebar.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn light_only_on_first_run() { assert_eq!(initial_theme(false, Theme::System), Theme::Light); assert_eq!(initial_theme(true, Theme::System), Theme::System); assert_eq!(initial_theme(true, Theme::Dark), Theme::Dark); }
#[test] fn old_settings_file_still_loads() { let old = r#"{"theme":"System","preset":"Warm","include_ignored":false,"autosave":true,"show_minimap":false,"sidebar_width":258.0,"editor_width":440.0}"#; let s: Settings = serde_json::from_str(old).unwrap(); assert_eq!(s.theme, Theme::System); assert!(s.autosave); assert_eq!(s.editor_width, 440.0); }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run → pass.
- [ ] **Step 5:** Screenshots: welcome (no workspace), settings dialog, export dialog, conflict dialog (edit a file externally while dirty), in light and dark. Confirm: no sharp corners, one primary button each, no separators. Commit.

---

### Task 9: Acceptance pass

**Files:** Modify whatever the checks expose; update `docs/DESIGN.md` (replace §2, §4, §5, §6 with a pointer to the spec and the final token table), `README.md` feature list and shortcuts.

- [ ] **Step 1: Leftover sweep.** Each of these greps over `src` must return nothing:
  - `grep -rn "separator()" src`
  - `grep -rnE "[◇◆☾☀⚙⌕▸▾●♧↥＋◧≡ⓘ×]" src` (icons are painted, the dirty dot is a painted circle)
  - `grep -rnE "Color32::from_rgb|from_rgba" src --include=*.rs | grep -v "src/theme.rs"`
  - `grep -rnE "corner_radius\(0|CornerRadius::ZERO|\.size\((7|8|9|10|11|12)\.0\)" src`
  - `grep -rnE "\.to_uppercase\(\)" src/ui`
- [ ] **Step 2: Size matrix.** Screenshot `examples/system.mmd` and `crates/diagram/tests/fixtures/llm_architecture.mmd` at 1440x940, 1100x700, 800x600, in light and dark (12 images). For each, read the PNG and check: diagram readable without zooming; nothing overlaps; pill fully visible; top-bar buttons unclipped.
- [ ] **Step 3: Scale.** Run with `ctx.set_zoom_factor(2.0)` via a temporary `TIDEMAP_SHOT_SCALE=2` env hook at 1440x940; same checks. Keep the env hook (documented in README under development).
- [ ] **Step 4: Done-when from the brief.** Record each result in a short table at the bottom of this file:
  1. Canvas ≥ 75% of the window on open at 1440x940 (from `canvas_fraction` and by measuring the screenshot).
  2. Node label text ≥ 13 px on screen at the opening zoom for both fixtures (zoom × 14 from the pill's zoom label).
  3. No sharp corner, no heavy border (Step 1 greps + screenshots).
  4. AA contrast (`cargo test -p tidemap all_text_pairs_meet_aa` and `cargo test -p tidemap-diagram default_palette_matches_spec_and_passes_aa`).
  5. One accent plus the pastel palette (Step 1 colour grep).
- [ ] **Step 5: Feature regression walk.** In the running app, with a scratch copy of `examples/`: open folder; open file; switch files; Alt+PageDown through `welcome.md`; back/forward; pin and unpin; edit and Ctrl+S; edit and close the window (close dialog appears); edit the file in another editor while dirty (conflict dialog); kill the app while dirty and reopen (recovery dialog); export SVG and PNG; diagram search; outline; theme toggle; restart and confirm the last file, diagram and camera are restored. Note any failure, fix it, repeat that item.
- [ ] **Step 6:** `cargo test --workspace` green, `cargo build --release` clean, `cargo native package` produces the portable build. Update the docs. Commit.

## Acceptance results

(filled in by Task 9)
