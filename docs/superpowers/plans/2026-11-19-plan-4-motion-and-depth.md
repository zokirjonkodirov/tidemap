# Motion and Depth Implementation Plan (plan 4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the shell feel physical and layered (Craft/Arc): springs instead of front-loaded tweens, entrances and exits from an origin, four elevation levels, directional transitions, a theme reveal, and reduced-motion support.

**Architecture:** `src/motion.rs` becomes a spring engine (stateful per `egui::Id`, velocity-preserving, plus an analytic step curve for time-based uses). `src/theme.rs` gains elevation levels painted by one helper. A new `src/ui/surface.rs` owns enter/exit presence and transforms for floating surfaces (layer transforms for `Area`s, visual transforms inside `Modal`s). A new `src/ui/snapshot.rs` crossfades a screenshot of the previous frame for the theme and page/diagram switches. Every existing surface migrates to these four modules; no surface does its own time math.

**Tech Stack:** Rust 1.88, egui/eframe =0.33.3 (`Context::set_transform_layer`, `Ui::with_visual_transform`, `ViewportCommand::Screenshot` with `UserData`), Inter.

**Spec:** `docs/superpowers/specs/2026-11-19-motion-and-depth-design.md` (§3 values, §4 decisions D1/D2).

**Depends on:** plans 1–3 done. Start from a clean tree: commit or stash the current editor/markdown work, then branch `motion-depth` from it.

## Global Constraints

- egui/eframe stay at `=0.33.3`. Only new dependency: `windows-sys = "0.61"` (already in `Cargo.lock`) under `[target.'cfg(windows)'.dependencies]` with feature `Win32_UI_WindowsAndMessaging`.
- All motion goes through `src/motion.rs`. No `ctx.animate_*`, no `Instant` or `input.time` arithmetic in surfaces.
- Springs, exactly: `SNAP` 0.16/1.0, `SOFT` 0.30/1.0, `GLIDE` 0.38/1.0, `POP` 0.34/0.74 (response s / damping fraction).
- Entrances use `POP` (surfaces) or `GLIDE` (panels, canvas). Exits use `SNAP`.
- Opacity = `motion::fade(t)` = `(t / 0.6).clamp(0, 1)`; transforms carry the rest of the progress.
- At rest every layer and visual transform is exactly `TSTransform::IDENTITY` (layer transform removed).
- Shadows only via `theme::Level`; no `Shadow { .. }` literal outside `src/theme.rs`. Colours only in `src/theme.rs` and the diagram `StyleSheet` (existing rule).
- Choreography values (scales, offsets, origins) exactly as spec §3 table.
- Owner rules: everything moves; light icons; no sharp arrowheads; the keyboard reaches everything (the new block menu takes arrows, Enter, Esc).
- Reduced motion: spatial springs resolve in one frame; opacity changes stay, capped at 150 ms.
- Motion frames: p95 CPU ≤ 8 ms at 1440×940, release build, `examples/` workspace (measured with `TIDEMAP_FRAME_LOG=1`).
- Tests: `cargo test -p tidemap` (and `-p tidemap-model` for Task 9). Stills: `TIDEMAP_SHOT=<scratch>/tN.png target/release/tidemap.exe examples/welcome.md`, light and `TIDEMAP_SHOT_THEME=dark`. Motion: run with `TIDEMAP_SLOWMO=8` and watch.

## Review Focus

1. **Rapid toggling** (Ctrl+B pressed 4× fast, pointer sweeping across tree rows, holding ↓ in the palette, clicking zoom-in 5×): values reverse without a jump and keep moving the way they were going for a frame. Test `retarget_keeps_position_and_momentum` in Task 1; `camera_retarget_keeps_velocity` in Task 6.
2. **Long frame gaps** (window minimised, laptop asleep, first frame after idle): no overshoot explosion, no surface stuck half-open. Test `long_gap_stays_bounded` in Task 1.
3. **Reopen while closing / close while opening** (Esc then Ctrl+P within 100 ms): the surface continues from its current scale and opacity, never restarts from zero. Test `presence_reopens_from_current_value` in Task 1.
4. **Huge diagrams during transitions** (10,000-node file, step to next diagram, slide the sidebar): the outgoing scene is not painted above 1,500 nodes. Test `outgoing_scene_skipped_when_large` in Task 7.
5. **Screenshot mode and settling**: `motion::busy` turns false after springs rest, so `TIDEMAP_SHOT` never hangs, and at rest no layer keeps a transform. Tests `springs_rest_exactly_and_busy_clears` (Task 1) and `layer_transform_cleared_at_rest` (Task 4).

---

## File structure

```text
src/motion.rs          spring engine, hover, press, presence, reduced motion, slow motion (rewritten)
src/theme.rs           Palette.dark, Level, Elevation, elevation(), elevated(), contrast()
src/ui/surface.rs      enter/exit transforms, Area/Modal helpers, background push-back (new)
src/ui/snapshot.rs     screenshot crossfade and circular reveal (new)
src/ui/controls.rs     press/hover migration, stretch_band()
src/ui/{palette,dialogs,info,pill,topbar,sidebar,states,mod}.rs   migrate to the above
src/icons.rs           button(): press scale, asymmetric hover
src/canvas.rs          spring camera, Enter direction, outgoing scene, camera_into()
src/markdown/view.rs   Action::OpenDiagram carries the embed rect
src/app.rs             theme/page requests go through snapshot; Enter on step/history; frame log
crates/model/src/lib.rs  Settings.motion: MotionPref
docs/DESIGN.md         elevation and motion sections (D1)
```

---

### Task 1: Spring engine

**Files:**
- Modify: `src/motion.rs` (whole file), `src/app.rs:1061` (`update`: set time scale once)
- Test: `src/motion.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces:
  - `#[derive(Clone, Copy)] pub struct Spring { pub response: f32, pub damping: f32 }`; `pub const SNAP, SOFT, GLIDE, POP: Spring`
  - `#[derive(Clone, Copy, Default)] pub struct SpringState { pub value: f32, pub velocity: f32 }`
  - `pub fn advance(state: &mut SpringState, target: f32, spring: Spring, dt: f32)` — semi-implicit Euler, substeps ≤ 1/240 s, `dt` clamped to 1/30 s; stiffness `(2π/response)²`, damping coefficient `4π·damping/response`; snaps to target when `|value−target| < 5e-4·max(1,|target|)` and `|velocity|` below the same ×10.
  - `pub fn step(spring: Spring, elapsed: f32) -> f32` — analytic 0→1 step response (critically and under-damped closed forms).
  - `pub fn spring(ctx: &Context, id: Id, target: f32, spring: Spring) -> f32` (first call returns `target`) and `pub fn spring_from(ctx, id, initial: f32, target: f32, spring: Spring) -> f32`
  - `pub fn hover(ctx: &Context, id: Id, on: bool) -> f32` — `SNAP` toward 1, `SOFT` toward 0
  - `pub fn press(ctx: &Context, id: Id, down: bool) -> f32` — first call starts at 0; target 1 while down or for 0.08 s after the press started; `SNAP` down, `POP` up
  - `pub fn presence(ctx: &Context, id: Id, open: bool) -> Option<f32>` — `POP` from 0 on first open, `SNAP` toward 0 on close, `None` once closed and at rest
  - `pub fn fade(t: f32) -> f32`
  - `pub fn set_reduced(ctx: &Context, on: bool)`, `pub fn reduced(ctx: &Context) -> bool` — when on, `spring*`/`presence` positions resolve at once
  - `pub fn set_time_scale(ctx: &Context, scale: f32)` — divides the motion clock (`TIDEMAP_SLOWMO`)
  - Keep `busy`, `restart` (now also clears spring state), `moving`. Keep `tween`/`toggle`/`appear` and `FAST/BASE/PANEL/CAMERA` until Task 9 removes them.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Context, Id, RawInput};

    fn at(ctx: &Context, time: f64, mut f: impl FnMut(&Context)) {
        let _ = ctx.run(RawInput { time: Some(time), ..Default::default() }, |ctx| f(ctx));
    }

    #[test]
    fn first_frame_moves_less_than_a_fifth() {
        for s in [SNAP, SOFT, GLIDE, POP] { assert!(step(s, 1.0 / 60.0) < 0.2); }
    }

    #[test]
    fn critically_damped_never_overshoots_and_pop_barely_does() {
        let peak = |s| (0..400).map(|i| step(s, i as f32 / 200.0)).fold(0.0_f32, f32::max);
        assert!(peak(GLIDE) <= 1.0 + 1e-4);
        assert!(peak(POP) > 1.0 && peak(POP) < 1.05);
    }

    #[test]
    fn retarget_keeps_position_and_momentum() {
        let ctx = Context::default();
        let id = Id::new("r");
        let mut v = vec![];
        at(&ctx, 0.0, |c| { spring(c, id, 0.0, SOFT); });
        for f in 1..=4 { at(&ctx, f as f64 / 60.0, |c| v.push(spring(c, id, 1.0, SOFT))); }
        at(&ctx, 5.0 / 60.0, |c| v.push(spring(c, id, 0.0, SOFT)));
        assert!(v[4] >= v[3] - 1e-4, "reversed instantly: {v:?}");
        assert!((v[4] - v[3]).abs() < 0.2);
    }

    #[test]
    fn long_gap_stays_bounded() {
        let ctx = Context::default();
        let id = Id::new("g");
        at(&ctx, 0.0, |c| { spring(c, id, 0.0, POP); });
        at(&ctx, 0.016, |c| { spring(c, id, 1.0, POP); });
        let mut late = 0.0;
        at(&ctx, 10.0, |c| late = spring(c, id, 1.0, POP));
        assert!((0.0..=1.05).contains(&late));
    }

    #[test]
    fn springs_rest_exactly_and_busy_clears() {
        let ctx = Context::default();
        let id = Id::new("s");
        at(&ctx, 0.0, |c| { spring(c, id, 0.0, GLIDE); });
        let mut last = 0.0;
        for f in 1..=120 { at(&ctx, f as f64 / 60.0, |c| last = spring(c, id, 1.0, GLIDE)); }
        assert_eq!(last, 1.0);
        let mut busy_now = true;
        at(&ctx, 2.1, |c| { spring(c, id, 1.0, GLIDE); });
        at(&ctx, 2.2, |c| busy_now = busy(c));
        assert!(!busy_now);
    }

    #[test]
    fn hover_leaves_slower_than_it_arrives() {
        let ctx = Context::default();
        let id = Id::new("h");
        let frames_until = |on: bool, done: fn(f32) -> bool, start: u32| {
            let mut f = start;
            loop { f += 1; let mut v = 0.0; at(&ctx, f as f64 / 60.0, |c| v = hover(c, id, on)); if done(v) { return f - start; } }
        };
        at(&ctx, 0.0, |c| { hover(c, id, false); });
        let arrive = frames_until(true, |v| v >= 0.9, 0);
        let leave = frames_until(false, |v| v <= 0.1, 200);
        assert!(leave * 2 > arrive * 3, "arrive {arrive} leave {leave}");
    }

    #[test]
    fn short_click_still_presses() {
        let ctx = Context::default();
        let id = Id::new("p");
        at(&ctx, 0.0, |c| { press(c, id, true); });
        let mut peak = 0.0_f32;
        for f in 1..30 { at(&ctx, f as f64 / 60.0, |c| peak = peak.max(press(c, id, false))); }
        assert!(peak >= 0.5);
    }

    #[test]
    fn presence_reopens_from_current_value() {
        let ctx = Context::default();
        let id = Id::new("o");
        for f in 0..=60 { at(&ctx, f as f64 / 60.0, |c| { presence(c, id, true); }); }
        let mut mid = None;
        for f in 61..=63 { at(&ctx, f as f64 / 60.0, |c| mid = presence(c, id, false)); }
        let mut again = None;
        at(&ctx, 64.0 / 60.0, |c| again = presence(c, id, true));
        let (mid, again) = (mid.unwrap(), again.unwrap());
        assert!(mid > 0.2 && mid < 1.0 && (again - mid).abs() < 0.15);
        let mut gone = Some(1.0);
        for f in 65..=200 { at(&ctx, f as f64 / 60.0, |c| gone = presence(c, id, false)); }
        assert_eq!(gone, None);
    }

    #[test]
    fn reduced_motion_resolves_positions_at_once() {
        let ctx = Context::default();
        let id = Id::new("m");
        at(&ctx, 0.0, |c| { set_reduced(c, true); spring(c, id, 0.0, GLIDE); });
        let mut v = 0.0;
        at(&ctx, 0.016, |c| v = spring(c, id, 1.0, GLIDE));
        assert_eq!(v, 1.0);
    }

    #[test]
    fn slow_motion_scales_the_clock() {
        let ctx = Context::default();
        let id = Id::new("t");
        at(&ctx, 0.0, |c| { set_time_scale(c, 10.0); spring(c, id, 0.0, GLIDE); });
        let mut v = 0.0;
        at(&ctx, 0.1, |c| v = spring(c, id, 1.0, GLIDE));
        assert!((v - step(GLIDE, 0.01)).abs() < 0.02);
    }
}
```

- [ ] **Step 2: Run, expect compile failures** — `cargo test -p tidemap motion::` → FAIL ("cannot find function `step`" …)
- [ ] **Step 3: Implement the Interfaces above in `src/motion.rs`.** Spring state is stored with `insert_temp` under `id.with("motion-spring")` with its last update time; mark `ACTIVE_PASS` and `request_repaint` while not at rest (same as tweens today). The motion clock is `input.time / scale`. Reduced mode: positions jump to target; `presence` still returns `None` after close.
- [ ] **Step 4: In `TidemapApp::update` (first frame only), read `TIDEMAP_SLOWMO` (f32, default 1) and call `motion::set_time_scale`.**
- [ ] **Step 5: Run** `cargo test -p tidemap motion::` → all PASS; `cargo test -p tidemap` → PASS.
- [ ] **Step 6: Commit** `git commit -am "feat(motion): velocity-preserving spring engine"`

---

### Task 2: Elevation levels

**Files:**
- Modify: `src/theme.rs:43-67` (Palette gets `pub dark: bool`), `:99-144`, `:154-157`, `:194-254`; `src/ui/mod.rs:156-166` (`card`), `src/ui/pill.rs:75`, `src/ui/info.rs:370,409`, `src/ui/dialogs.rs:37-40`, `src/ui/palette.rs:501`, `src/canvas.rs` (minimap and LOD hint fills), `docs/DESIGN.md` §1, §2 Motion row, "Not used" line (D1)
- Test: `src/theme.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces:
  - `#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum Level { Card, Raised, Overlay, Modal }`
  - `#[derive(Clone, Copy)] pub struct Elevation { pub fill: Color32, pub contact: Shadow, pub key: Shadow, pub edge: Stroke, pub highlight: Color32 }` (`highlight` transparent in light)
  - `pub fn elevation(p: Palette, level: Level) -> Elevation` — values exactly spec §3 table
  - `pub fn elevated<R>(ui: &mut Ui, p: Palette, level: Level, radius: f32, margin: Margin, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R>` — reserves a `Shape::Noop` slot before the frame, draws `Frame` with fill and edge (`StrokeKind::Inside`), then fills the slot with `contact` and `key` shapes (`Shadow::as_shape`) and paints the highlight as a 1 px line inset by `radius` along the top edge.
  - `pub fn contrast(a: Color32, b: Color32) -> f32` (WCAG ratio)
  - `visuals.popup_shadow` / `window_shadow` = `Overlay` key; `window_stroke` = `Overlay` edge (stock tooltips get the edge).
  - `floating()` and `inset_panel()` are deleted; callers use `elevated()`.
  - Dark `muted` becomes #98A1AC: the old #9099A4 is 4.34:1 on the `Modal` fill #303439 and fails `text_contrast_holds_on_every_fill`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn elevation_rises_with_level() {
    for dark in [false, true] {
        let p = palette(dark);
        let levels = [Level::Card, Level::Raised, Level::Overlay, Level::Modal];
        for pair in levels.windows(2) {
            let (a, b) = (elevation(p, pair[0]), elevation(p, pair[1]));
            assert!(b.key.blur > a.key.blur);
            assert!(b.key.offset[1] >= a.key.offset[1]);
        }
    }
}

#[test]
fn dark_surfaces_lighten_with_level() {
    let p = palette(true);
    let fills: Vec<f32> = [Level::Card, Level::Raised, Level::Overlay, Level::Modal]
        .iter().map(|l| contrast(elevation(p, *l).fill, Color32::BLACK)).collect();
    assert!(fills.windows(2).all(|w| w[1] > w[0]));
}

#[test]
fn dark_edges_separate_from_backdrop() {
    let p = palette(true);
    for (level, floor) in [(Level::Card, 1.25), (Level::Overlay, 1.4), (Level::Modal, 1.4)] {
        let e = elevation(p, level);
        assert!(contrast(over(e.edge.color, e.fill), p.app) >= floor, "{level:?}");
    }
}

#[test]
fn text_contrast_holds_on_every_fill() {
    for dark in [false, true] {
        let p = palette(dark);
        for level in [Level::Card, Level::Raised, Level::Overlay, Level::Modal] {
            assert!(contrast(p.muted, elevation(p, level).fill) >= 4.5);
        }
    }
}
```

- [ ] **Step 2: Run** `cargo test -p tidemap theme::` → FAIL (no `Level`).
- [ ] **Step 3: Implement the Interfaces; migrate callers:** side/source/outline cards → `Card`; pill, toast, minimap frame, LOD hint → `Raised`; info popover → `Overlay`; palette, dialogs → `Modal`.
- [ ] **Step 4: Run** `cargo test -p tidemap` → PASS. `cargo build --release`, take light and dark stills of `examples/welcome.md` with `TIDEMAP_SHOT_FOLDER=examples` and `TIDEMAP_SHOT_PALETTE=sys`; read them: sidebar and outline cards have visible edges; in dark the palette has a visible edge and lighter fill than the cards.
- [ ] **Step 5: Update `docs/DESIGN.md`** (D1): replace the shadow/border sentences in §1, the shadow row in the palette table, the Motion row, and the "Not used" line with the spec §3 elevation table and spring names.
- [ ] **Step 6: Commit** `git commit -am "feat(theme): four elevation levels with edges and dark lift"`

---

### Task 3: Hover and press micro-interactions

**Files:**
- Modify: `src/ui/controls.rs:37-99`, `src/icons.rs:443-482`, `src/ui/pill.rs:122-142`, `src/ui/topbar.rs:177-199,250-272`, `src/ui/sidebar.rs:388-415,421-491,737`, `src/ui/palette.rs:703`, `src/ui/states.rs:61,297-298`, `src/ui/sidebar.rs:706` (stock `ui.button` → `controls::quiet_button`), `src/ui/topbar.rs:239-242` (dirty dot)
- Test: `src/ui/controls.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `motion::hover`, `motion::press`, `motion::spring`, `motion::POP`.
- Produces: `pub const PRESS_DEPTH: f32 = 0.04;` and `pub fn press_scale(press: f32) -> f32` (= `1 − PRESS_DEPTH·press`) in `controls.rs`, used by every pressable control. Text buttons, icon buttons, pill buttons, segmented options and the zoom label scale their fill and content about the rect centre by `press_scale`; the switch knob widens by 4 px while pressed (iOS squish).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn press_scale_spans_four_percent() {
    assert_eq!(press_scale(0.0), 1.0);
    assert!((press_scale(1.0) - 0.96).abs() < 1e-6);
}
```

- [ ] **Step 2: Run** `cargo test -p tidemap controls::` → FAIL.
- [ ] **Step 3: Implement `press_scale`; replace every hover `motion::toggle(…, hovered…, FAST)` with `motion::hover` and every press toggle with `motion::press`.** The dirty dot pops in with `motion::spring_from(…, 0.0, 1.0, POP)` as its radius factor.
- [ ] **Step 4: Verify no old hover/press calls remain:** `rg -n "toggle\(.*(hover|press)" src` → no output. `cargo test -p tidemap` → PASS.
- [ ] **Step 5: Look:** `TIDEMAP_SLOWMO=8 target/release/tidemap.exe examples/welcome.md`; sweep the pointer down the tree (rows leave a fading trail), click pill buttons (each click visibly dips and springs back).
- [ ] **Step 6: Commit** `git commit -am "feat(ui): asymmetric hover and latched press"`

---

### Task 4: Surfaces enter and leave from an origin

**Files:**
- Create: `src/ui/surface.rs` (+ `pub mod surface;` in `src/ui/mod.rs`)
- Modify: `src/ui/palette.rs:480-514` and `contents`/`input_row` (take `interactive: bool`), `src/ui/dialogs.rs:42-97`, `src/ui/info.rs:349-419`, `src/ui/topbar.rs:249-293` (own block menu), `src/theme.rs:226-238` (`tooltip_delay` 0.35, `tooltip_grace_time` 0.6)
- Test: `src/ui/surface.rs`

**Interfaces:**
- Consumes: `motion::presence`, `motion::fade`, `theme::elevated`, `Level`.
- Produces:
  - `#[derive(Clone, Copy)] pub struct Enter { pub scale: f32, pub offset: Vec2 }` and consts `POPOVER {0.94, (0,−4)}`, `PALETTE {0.96, (0,−8)}`, `DIALOG {0.95, (0,6)}`, `TOAST {0.96, (0,12)}`
  - `pub fn transform(t: f32, origin: Pos2, enter: Enter) -> TSTransform` — scale `lerp(enter.scale, 1, t)` about `origin`, translated by `enter.offset·(1−t)`; exactly `IDENTITY` when `t >= 1`. On exit the caller passes `exit_t(t)` so the surface shrinks only to 0.97–0.98 (`pub fn exit_t(t: f32) -> f32 { 0.6 + 0.4·t }`).
  - `pub fn animate_layer(ctx: &Context, layer: LayerId, t: f32, origin: Pos2, enter: Enter)` — `set_transform_layer`, identity removes it.
  - `pub fn push_back(ctx: &Context, t: f32)` — scales `LayerId::background()` and the pill and toast layers about the screen centre to `lerp(1, 0.99, t)`.
  - `pub fn exit_scrim(ctx: &Context, p: Palette, t: f32)` — paints the scrim on `Order::Middle` while a modal is leaving.
- Behaviour per surface (spec §3 choreography):
  - Palette, dialogs: while `presence` is `Some(t)` and open → `egui::Modal` with `ui.with_visual_transform(transform(…))` around the frame, scrim `p.scrim·fade(t)`, `push_back(t)`. While closing → plain `Area` (same position, `interactable(false)`), `exit_scrim`, contents drawn with `interactive = false` (no key consumption, no focus requests). The last open `Dialog` is kept in ctx temp data under `Id::new("workspace-dialog").with("last")` so it can be drawn while leaving. Recovery and conflict dialogs keep enter-only (their state is gone on close).
  - Info popover and the block menu: `Area` + `animate_layer` with `POPOVER`, origin = anchor button's bottom-right (info) / bottom-left (block).
  - Block menu replaces `egui::Popup::menu`: open state in ctx temp data; rows painted like palette rows with the sliding band; ↑/↓ move, Enter selects, Esc or outside click closes.
  - Toast: `TOAST` enter via `motion::step(POP, age)`, exit fade with y +6.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn transform_is_identity_at_rest_and_scales_about_origin() {
    let o = egui::pos2(100.0, 50.0);
    assert_eq!(transform(1.0, o, DIALOG), TSTransform::IDENTITY);
    let t0 = transform(0.0, o, POPOVER);
    let mapped = t0 * o;
    assert!((mapped - (o + POPOVER.offset)).length() < 1e-4);
    assert!((t0.scaling - 0.94).abs() < 1e-6);
}

#[test]
fn layer_transform_cleared_at_rest() {
    let ctx = egui::Context::default();
    let layer = LayerId::new(egui::Order::Foreground, egui::Id::new("x"));
    let _ = ctx.run(Default::default(), |c| animate_layer(c, layer, 0.5, egui::Pos2::ZERO, POPOVER));
    assert!(ctx.layer_transform_to_global(layer).is_some());
    let _ = ctx.run(Default::default(), |c| animate_layer(c, layer, 1.0, egui::Pos2::ZERO, POPOVER));
    assert!(ctx.layer_transform_to_global(layer).is_none());
}
```

- [ ] **Step 2: Run** `cargo test -p tidemap surface::` → FAIL.
- [ ] **Step 3: Implement `surface.rs`; migrate palette, dialogs, info popover, toast; build the block menu.**
- [ ] **Step 4: Run** `cargo test -p tidemap` → PASS. Stills for `TIDEMAP_SHOT_DIALOG=settings`, `TIDEMAP_SHOT_INFO=1`, `TIDEMAP_SHOT_PALETTE=` still render settled (identity) surfaces.
- [ ] **Step 5: Look (slow-mo 8):** Ctrl+P / Esc / Ctrl+P quickly: palette never restarts from zero; dialogs leave as well as arrive; the window behind sinks slightly; the info popover grows from its button; the block menu works with the keyboard alone.
- [ ] **Step 6: Commit** `git commit -am "feat(ui): surfaces enter and exit from their origin"`

---

### Task 5: Selection bands and folds

**Files:**
- Modify: `src/ui/controls.rs:123-141` (segmented), `src/ui/palette.rs:664-693`, `src/ui/sidebar.rs:515-545,635-645`, `src/ui/states.rs:290-313` (outline rows)
- Test: `src/ui/controls.rs`

**Interfaces:**
- Consumes: `motion::spring`, `motion::step`, `SNAP`, `SOFT`, `GLIDE`.
- Produces: `pub fn stretch_band(ctx: &Context, id: Id, top: f32, bottom: f32) -> (f32, f32)` — the edge on the side of travel follows with `SNAP`, the other with `SOFT`; returns the band's current top and bottom. Used by the tree active band, palette highlight, block menu, segmented control (horizontal: pass left/right) and outline.
- Folds: `display()` uses `motion::step(GLIDE, elapsed − delay)` with `delay = min(k, 8) · 0.012` for the k-th revealed child when opening, no delay when closing (`SNAP`); `FOLD_SECONDS` becomes the time at which `step(GLIDE, ·)` ≥ 0.999 plus the largest delay.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn band_stretches_while_travelling_and_rests_exact() {
    let ctx = egui::Context::default();
    let id = egui::Id::new("band");
    let run = |t: f64, top: f32| {
        let mut out = (0.0, 0.0);
        let _ = ctx.run(egui::RawInput { time: Some(t), ..Default::default() }, |c| out = stretch_band(c, id, top, top + 32.0));
        out
    };
    run(0.0, 0.0);
    let (a, b) = run(0.05, 320.0);
    assert!(b - a > 32.0, "no stretch: {a} {b}");
    let mut rest = (0.0, 0.0);
    for f in 4..120 { rest = run(f as f64 / 60.0, 320.0); }
    assert_eq!(rest, (320.0, 352.0));
}
```

- [ ] **Step 2: Run** → FAIL. **Step 3: Implement and migrate the four bands and the fold.** **Step 4: Run** `cargo test -p tidemap` → PASS; look in slow-mo: holding ↓ in the palette glides continuously; folders unfold in a short cascade.
- [ ] **Step 5: Commit** `git commit -am "feat(ui): stretching selection bands, staggered folds"`

---

### Task 6: Panels and camera

**Files:**
- Modify: `src/ui/mod.rs:110-118,196-264`, `src/canvas.rs:180-215` (`CameraAnim`), callers of `motion::CAMERA`
- Test: `src/canvas.rs` (existing or new `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `motion::spring`, `motion::advance`, `SpringState`, `GLIDE`.
- Produces: `CameraAnim { x: SpringState, y: SpringState, log_zoom: SpringState, to: Camera }`; `fn tick_anim(&mut self, dt: f32)` advances the three springs with `GLIDE` (dt from the motion clock, so slow-mo and reduced motion apply); `animate_to` retargets without resetting velocity.
- Panels: `sidebar_t` / `source_t` = `motion::spring(…, open as f32, GLIDE)`; `sliding_card` no longer multiplies opacity; the card keeps `Level::Card` so its shadow travels with the edge.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn camera_retarget_keeps_velocity() {
    let mut canvas = CanvasView::default();
    canvas.camera = Camera { x: 0.0, y: 0.0, zoom: 1.0 };
    canvas.animate_to(Camera { x: 0.0, y: 0.0, zoom: 1.25 });
    for _ in 0..3 { canvas.tick_anim(1.0 / 60.0); }
    let z3 = canvas.camera.zoom;
    canvas.animate_to(Camera { x: 0.0, y: 0.0, zoom: 1.5625 });
    canvas.tick_anim(1.0 / 60.0);
    let z4 = canvas.camera.zoom;
    canvas.tick_anim(1.0 / 60.0);
    assert!(z4 > z3 && canvas.camera.zoom - z4 >= z4 - z3, "momentum lost");
}
```
(Use the canvas struct's real name and constructor.)

- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** `cargo test -p tidemap` → PASS; look: Ctrl+B ×4 fast reverses smoothly, no ghosting; `+` five times zooms in one continuous motion.
- [ ] **Step 5: Commit** `git commit -am "feat(canvas): spring camera; panels slide without fading"`

---

### Task 7: Directional diagram transitions

**Files:**
- Modify: `src/canvas.rs:440-462,655-690`, `src/app.rs:168-170` (`select_block`, `step_diagram`, `history_step`), `src/ui/topbar.rs:201-247` (breadcrumb label crossfade)
- Test: `src/canvas.rs`

**Interfaces:**
- Produces:
  - `#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)] pub enum Travel { Forward, Back, #[default] Jump }`; `pub fn set_travel(&mut self, travel: Travel)` on the canvas (consumed by the next different-diagram artifact, then reset to `Jump`).
  - `pub const OUTGOING_LIMIT: usize = 1500;` and `pub fn keeps_outgoing(nodes: usize) -> bool`
  - `pub fn travel_offsets(travel: Travel, p: f32) -> (Vec2, Vec2)` — (incoming, outgoing) in points: Forward `(32·(1−p), 0)` / `(−32·p, 0)`, Back mirrored, Jump `(0,0)` / `(0,0)`.
  - Incoming opacity `fade(p)`; outgoing opacity `1 − fade(min(1, 2p))`; Jump incoming scale `lerp(0.985, 1, p)` about the viewport centre; `p` from `motion::spring_from(…, 0, 1, GLIDE)` restarted on each switch.
- App: `step_diagram(+1)` and `history_step(+1)` → `Forward`; negative → `Back`; everything else `Jump`.
- Breadcrumb: the file name and block label crossfade with a 6 px shift in the travel direction (`SOFT`).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn travel_offsets_meet_at_rest() {
    assert_eq!(travel_offsets(Travel::Forward, 1.0), (Vec2::ZERO, Vec2::new(-32.0, 0.0)));
    assert_eq!(travel_offsets(Travel::Back, 0.0).0, Vec2::new(-32.0, 0.0));
    assert_eq!(travel_offsets(Travel::Jump, 0.3), (Vec2::ZERO, Vec2::ZERO));
}

#[test]
fn outgoing_scene_skipped_when_large() {
    assert!(keeps_outgoing(1500));
    assert!(!keeps_outgoing(1501));
}
```

- [ ] **Step 2: Run** → FAIL. **Step 3: Implement**; paint the outgoing scene first with its own camera offset and opacity, then the incoming one. **Step 4: Run** `cargo test -p tidemap` → PASS; look: `]` slides left-to-right travel, `[` and Alt+Left slide back, opening a file from the tree scales in.
- [ ] **Step 5: Commit** `git commit -am "feat(canvas): directional diagram transitions"`

---

### Task 8: Snapshot crossfade — theme reveal and page ↔ diagram

**Files:**
- Create: `src/ui/snapshot.rs` (+ `pub mod snapshot;`)
- Modify: `src/ui/topbar.rs:300-308` (theme button), `src/app.rs` (theme shortcut, `toggle_document_view` at `:981-989`, `Event::Screenshot` filter at `:930` accepts only `user_data.data.is_none()`), `src/markdown/view.rs:76,1562` (`OpenDiagram(String, egui::Rect)`), `src/app.rs:1027` (handle it), `src/canvas.rs` (`camera_into`)
- Test: `src/ui/snapshot.rs`, `src/canvas.rs`

**Interfaces:**
- Produces:
  - `pub enum Purpose { Theme { to: Theme, from: Pos2 }, Page { clip: Rect } }`
  - `pub fn request(ctx: &Context, purpose: Purpose)` — sends `ViewportCommand::Screenshot(UserData::new(purpose))`
  - `pub fn take(ctx: &Context) -> Option<Purpose>` — consumes our `Event::Screenshot`, uploads it as texture `"snapshot"`, starts the reveal, returns the purpose so the app applies the change in that same frame
  - `pub fn paint(ctx: &Context)` — paints the old frame on `Order::Debug` (non-interactive): Theme → everything outside a circle from `from`, radius `step(GLIDE, age) · farthest-corner distance`; Page → the clipped image fading with `1 − fade(p)` and scaling 1 → 0.98; reduced motion → a 150 ms fade for both.
  - `pub fn ring_mesh(center: Pos2, radius: f32, screen: Rect, segments: usize, texture: TextureId) -> Mesh` — quads between the circle and a circle of the farthest-corner radius, UV = `(pos − screen.min) / screen.size()`.
  - `canvas::camera_into(bounds: Rect, target: egui::Rect) -> Camera` — the camera that places `bounds` inside `target` (viewport-relative). Opening a diagram from a page embed sets the camera to `camera_into(bounds, embed_rect)` at once, then `animate_to(fit)` while the page snapshot fades: the embed grows into the canvas.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn ring_is_empty_once_radius_covers_screen() {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 940.0));
    let corner = egui::vec2(1440.0, 940.0).length();
    assert!(ring_mesh(egui::Pos2::ZERO, corner + 1.0, screen, 96, egui::TextureId::default()).indices.is_empty());
    assert_eq!(ring_mesh(egui::Pos2::ZERO, 0.0, screen, 96, egui::TextureId::default()).indices.len(), 96 * 6);
}

#[test]
fn camera_into_places_bounds_in_target() {
    let bounds = Rect { x: 0.0, y: 0.0, w: 400.0, h: 200.0 };
    let target = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(200.0, 100.0));
    let cam = camera_into(bounds, target);
    assert!((cam.zoom - 0.5).abs() < 1e-4);
    assert!((cam.x - 100.0).abs() < 1e-3 && (cam.y - 50.0).abs() < 1e-3);
}
```
(`Rect` here is the scene bounds type `fit_camera` takes.)

- [ ] **Step 2: Run** → FAIL. **Step 3: Implement; route the theme button, Ctrl+Shift+D and page toggle through `request`/`take`.** **Step 4: Run** `cargo test -p tidemap` → PASS. Shot mode still saves PNGs (its screenshot has no user data). Look: the theme sweeps out from the button; clicking an embedded diagram on a page grows it into the canvas.
- [ ] **Step 5: Commit** `git commit -am "feat(ui): theme reveal and page-to-diagram growth"`

---

### Task 9: Reduced motion, frame evidence, cleanup, acceptance

**Files:**
- Modify: `crates/model/src/lib.rs:15-35`, `src/ui/dialogs.rs` (settings row), `src/app.rs:1061` (apply preference, frame log), `src/motion.rs` (remove `tween`, `toggle`, `appear`, `FAST`, `BASE`, `PANEL`, `CAMERA` once unused), `Cargo.toml` (windows-sys), `docs/DESIGN.md` §2 Motion row, README shortcuts/env table
- Test: `crates/model/src/lib.rs`, `src/motion.rs`

**Interfaces:**
- Produces: `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)] pub enum MotionPref { #[default] System, Reduced, Full }`; `Settings.motion: MotionPref`. `pub fn system_reduced() -> bool` in `src/motion.rs` — Windows: `SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION)` returns false animations → true; other platforms false. Settings row "Motion" with `segmented` System / Reduced / Full. `TIDEMAP_FRAME_LOG=1` prints `frame <ms>` from `frame.info().cpu_usage` for every frame where `motion::busy`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn old_settings_default_motion_to_system() {
    let s: Settings = serde_json::from_str(r#"{"theme":"Light","autosave":true}"#).unwrap();
    assert_eq!(s.motion, MotionPref::System);
}
```

- [ ] **Step 2: Run** `cargo test -p tidemap-model` → FAIL. **Step 3: Implement**; `update` calls `motion::set_reduced(ctx, pref == Reduced || (pref == System && system_reduced()))`.
- [ ] **Step 4: Remove the tween API;** `rg -n "motion::(tween|toggle|appear|FAST|BASE|PANEL|CAMERA)\b" src` → no output; `cargo test --workspace` → PASS; `cargo clippy --workspace -- -D warnings` → clean.
- [ ] **Step 5: Frame budget:** `TIDEMAP_FRAME_LOG=1 target/release/tidemap.exe examples/welcome.md 2> <scratch>/frames.txt`; toggle sidebar ×5, open/close palette ×5, step diagrams ×5, switch theme ×2; p95 of logged values ≤ 8 ms. Record the number in the Acceptance section below.
- [ ] **Step 6: Acceptance stills** (light + dark): workspace, palette, settings dialog, info popover, source open. All settled, edges visible, no stock egui menu left (`rg -n "Popup::menu|selectable_label|ui\.button\(" src` → no output).
- [ ] **Step 7: Commit** `git commit -am "feat: reduced motion, frame log, remove tween API"`

---

### Task 10 (only if the owner approves D2): Tinted frame and canvas card

**Files:**
- Modify: `src/theme.rs:99-144` (light `app` #F1F2F4, `muted` #636A76; dark `app` #141619), `src/ui/mod.rs:267-301` (canvas inside `elevated(Level::Card, RADIUS_PANEL)` with `inset_margin(token::INSET)`), `src/canvas.rs:421-426` (clip to the card radius), `docs/DESIGN.md` §1 layout, `src/ui/mod.rs:126-143` canvas share check
- Test: `src/theme.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn muted_reads_on_tinted_backdrop() {
    for dark in [false, true] {
        let p = palette(dark);
        assert!(contrast(p.muted, p.app) >= 4.5);
        assert!(contrast(p.secondary, p.app) >= 4.5);
    }
}
```

- [ ] **Step 2: Run** → FAIL (light muted on #F1F2F4 is 4.32). **Step 3: Implement.** **Step 4: Run** `cargo test -p tidemap` → PASS; the shot log line "canvas … %" stays ≥ 75% at 1440×940; pill and minimap stay inside the card.
- [ ] **Step 5: Commit** `git commit -am "feat(ui): tinted frame, canvas as content card"`

## Acceptance results

Recorded by Task 9 (release build, 1440x940, Windows).

**Frame log (smoke run, screenshot mode).** `TIDEMAP_FRAME_LOG=1 TIDEMAP_SHOT_DIALOG=settings TIDEMAP_SHOT_FOLDER=examples tidemap.exe examples/welcome.md`: 38 busy frames (the dialog entrance and the page-to-workspace settle), median 0.87 ms, p95 5.49 ms, max 22.19 ms. The max is the first frame after launch (font and layout warm-up), not steady-state motion. A run on `welcome.md` alone logged no busy frames (nothing animates once it settles). This is a smoke figure only; it is not the interactive budget below.

**Stills** (all settled, light and dark) in `.superpowers/sdd/2026-11-19-plan-4-motion-and-depth/shots/`: `t9-workspace-{light,dark}.png`, `t9-palette-{light,dark}.png` (query `>motion`, shows the new Toggle reduced motion command), `t9-settings-{light,dark}.png` (new Motion row), `t9-info-{light,dark}.png`, `t9-source-{light,dark}.png`; frame log in `t9-frames.txt`. Nothing wrong seen. The Motion row's detail wraps to three lines at the dialog width and sits close to the Theme row.

**Checks.** `cargo build` and `cargo test --workspace` warning-free; `rg "Popup::menu|selectable_label|ui\.button\(" src` prints nothing; no `motion::(tween|toggle|appear|FAST|BASE|PANEL|CAMERA)` left. Clippy is not installed for this toolchain, so it was not run.

**Owner checks still outstanding.**
- Interactive frame budget: toggle the sidebar x5, palette open/close x5, step diagrams x5, switch theme x2 with `TIDEMAP_FRAME_LOG=1`, then confirm p95 of the `frame` lines is at most 8 ms.
- Slow-motion feel with `TIDEMAP_SLOWMO=4` across panels, dialogs, camera glides and the theme reveal.
- Reduced motion in the real app: Settings > Motion > Reduced, and the Windows "Show animations" switch with the window refocused.
