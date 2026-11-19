# Motion and depth: design review and target

Date: 19 November 2026. Status: proposed. Owner decisions D1 and D2 at the end.
Reference feel: Craft Docs (soft depth, springy popovers, calm) and Arc (spatial continuity, things come from where you clicked, tinted chrome around an elevated content card).

## 1. What the shell is for

- **Question:** "What does this diagram look like now, and where am I in my files?"
- **Answer object:** the diagram on the canvas, with the breadcrumb naming it.
- **One action:** open the source (or jump to another file or diagram).

Motion and depth have one job here: tell the user that the app heard them, where a surface came from, where it went, and which layer is on top. Today the motion exists but does not do that job.

## 2. Findings (interaction designer chair)

`surface · chair · Q · severity · what is wrong · the move`

1. All surfaces · interaction · Q8 · High · Every tween uses quartic ease-out (`src/motion.rs:20-23`). Its starting speed is 4× its average, so the first 60 Hz frame covers 45% of a 120 ms hover and 32% of a 180 ms popover, and the second frame 73% / 56%. The eye sees a cut followed by a smear: "available but cheap". · Replace with springs that start at zero velocity and settle over 200–350 ms.
2. All surfaces · interaction · Q8 · High · Retargeting restarts from the current value with zero velocity (`src/motion.rs:72-74`, camera `src/canvas.rs:191-210`). Hover jitter, Ctrl+B spam, held arrow keys in the palette and repeated zoom clicks all hitch. · Springs keep velocity across retargets.
3. Palette, dialogs, info popover, block menu · interaction · Q8 · High · Entrances animate, exits are a one-frame cut (`src/ui/palette.rs:481-485`, `src/ui/dialogs.rs:63-81`, `src/ui/info.rs:352-355`; the block menu is stock `egui::Popup::menu`, `src/ui/topbar.rs:278-289`). Half of every transition is missing, which is most of what makes motion feel physical. · Presence model: a surface keeps drawing, non-interactive, while it leaves.
4. All floating surfaces · interaction · Q8 · High · One move for everything: fade + rise of 6–10 px (`dialogs.rs:83-97`, `palette.rs:489-490`, `info.rs:356-366`, toast `info.rs:395-405`, canvas `canvas.rs:675-679`). Too small to read as travel, no scale, no origin. Nothing comes from where you clicked. · Scale from the anchor (0.94–0.96 → 1) with a small spring; opacity reaches 1 early, the transform carries the rest.
5. Cards · typographer/founder · Q8, Q10 · High · Figure and ground are nearly identical: light `panel` #FFFFFF on `app` #FAFAF8 is 1.04:1, dark #24272B on #1B1D20 is 1.12:1, with no edge and no shadow (`src/theme.rs:251-254`). The sidebar, source and outline cards dissolve into the backdrop (see `docs/screenshots/workspace.png`). · Elevation levels with a contact shadow, a hairline edge, and (dark) a lighter fill and a top highlight.
6. Floating surfaces · interaction · Q8 · High · One shadow for every floating thing (`src/theme.rs:154-157`), so a toast, the pill, a popover and a modal sit at the same height. In dark mode the shadow (#14181C at 50% over #1B1D20) differs from the backdrop by 1.03:1: the palette in `docs/screenshots/palette.png` has no visible edge. · Four elevation levels; dark mode separates with fill lift, edge and highlight, not shadow.
7. Design system · founder · Q6 · High · The flatness is specified: `docs/DESIGN.md:7` ("hairline borders only on hover or drag … one soft shadow") and `:88` ("Not used: … glass blur, decorative glows"); plan 3 constraints repeat it. · Amend the direction (D1).
8. Buttons, icon buttons · interaction · Q8 · Medium · Press feedback is a 0.5 px sink over 80 ms (`src/ui/controls.rs:48-51`, `src/icons.rs:459-473`). A normal click is down 50–90 ms, so the press barely starts before it is released. · Latched press: a click always plays a full 4% press and springs back.
9. Lists · interaction · Q8 · Medium · Hover in and out take the same 120 ms; moving across rows flickers row by row. · Asymmetric hover: in fast, out slow, so the pointer leaves a short trail.
10. Side panels · interaction · Q8 · Medium · A sliding card also fades (`src/ui/mod.rs:196-206`). Fading a moving opaque card double-encodes and looks ghostly. · Slide only; the card is solid and its shadow leads the edge.
11. Diagram switching · interaction · Q8 · Medium · Next diagram, previous diagram, back, forward and opening a file all play the same fade-rise (`src/canvas.rs:448-455`). No sense of direction. · Directional slide for step and history; a gentle scale-in for jumps.
12. Page ↔ diagram, theme · interaction · Q8 · Medium · Toggling the page view (`src/app.rs:981-989`) and the theme (`src/ui/topbar.rs:305-307`) are hard cuts of the whole window. · Snapshot crossfade; a circular theme reveal from the button; an embedded diagram grows into the canvas.
13. Stock egui leaks · interaction · Q5 · Medium · Tooltips (egui default, 0.5 s delay, no motion), the block menu (`selectable_label`), the sidebar empty-state `ui.button` (`src/ui/sidebar.rs:706`) speak a different motion and shape vocabulary. · Route them through the shared surface and controls.
14. Accessibility · person doing the task · Q8 · High · No reduced-motion support anywhere, although plan 3 required it. · Settings choice (System / Reduced / Full) and the Windows system flag.
15. Verification · founder · — · Medium · Motion cannot be reviewed: stills only, no slow motion, no frame-time evidence. · `TIDEMAP_SLOWMO` and `TIDEMAP_FRAME_LOG` dev hooks.

**Root cause, in one line:** the motion system is a fixed-duration, front-loaded tween applied as fade-plus-nudge on enter only, on top of a design direction that bans depth. Fix the engine first, then elevation, then choreography.

## 3. Target vocabulary

### Springs (`src/motion.rs`)

Response is the period in seconds, damping the damping fraction (1 = critically damped).

| Name | Response | Damping | Use |
|---|---|---|---|
| `SNAP` | 0.16 | 1.0 | Hover in, press down, focus ring, every exit |
| `SOFT` | 0.30 | 1.0 | Hover out (trail), selection bands, fades |
| `GLIDE` | 0.38 | 1.0 | Panels, camera, diagram transitions, folds |
| `POP` | 0.34 | 0.74 | Popover, palette and dialog entrances, press release, dirty dot, switch knob (overshoot ≈ 3%) |

Rules: opacity reaches 1 at 60% progress (`motion::fade`), the transform carries the rest. Exits use `SNAP`, so they are faster than entrances. At rest every transform is exactly identity (crisp text, exact hit-testing).

### Choreography

| Surface | Enter | Exit | Origin |
|---|---|---|---|
| Info popover, block menu | scale 0.94 → 1, y −4 → 0, `POP` | to 0.97, fade, `SNAP` | anchor button |
| Palette | scale 0.96 → 1, y −8 → 0, `POP`; background pushes back to 0.99 | fade, `SNAP` | palette top centre |
| Dialogs | scale 0.95 → 1, y 6 → 0, `POP`; background pushes back to 0.99 | fade, to 0.98, `SNAP` | dialog centre |
| Toast | scale 0.96 → 1, y 12 → 0, `POP` | fade, y +6 | bottom left |
| Side panels | slide, no fade, `GLIDE` | slide, `GLIDE` | moving edge |
| Diagram step / history | slide 32 px in the direction of travel, `GLIDE`; outgoing slides the other way and fades in the first half | — | canvas |
| Diagram jump | scale 0.985 → 1, fade, `GLIDE` | old fades | canvas centre |
| Page → diagram from an embed | embed rect grows into the fitted canvas (camera spring) while the page snapshot fades | — | embed rect |
| Theme | old frame revealed away by a growing circle from the button, `GLIDE` | — | theme button |
| Hover | `SNAP` in, `SOFT` out | | |
| Press | latched 4% scale-down, `SNAP` down, `POP` up | | control centre |
| Selection bands | leading edge `SNAP`, trailing edge `SOFT` (the band stretches while it travels) | | |
| Folder unfold | rows grow with `GLIDE`, staggered 12 ms per row, at most 8 steps | together, `SNAP` | |

### Elevation (`src/theme.rs`)

Each level: fill, a contact shadow, a key shadow, a 1 px edge, and (dark only) a 1 px top highlight. Shadow = `x, y, blur, colour`.

| Level | Used by | Light | Dark |
|---|---|---|---|
| `Card` | sidebar, source, outline cards | fill `panel`; 0,1,2 ink 5%; 0,2,8 ink 4%; edge ink 6% | fill #24272B; 0,1,2 black 30%; 0,4,12 black 20%; edge white 6%; highlight white 4% |
| `Raised` | pill, toast, minimap, LOD hint | fill `panel`; 0,1,3 ink 8%; 0,6,16 ink 8%; edge ink 7% | fill #2A2D32; 0,1,2 black 40%; 0,8,20 black 30%; edge white 8%; highlight white 6% |
| `Overlay` | popovers, block menu, tooltips | fill `panel`; 0,1,3 ink 8%; 0,12,32 ink 12%; edge ink 8% | fill #2D3136; 0,1,3 black 45%; 0,12,32 black 40%; edge white 9%; highlight white 7% |
| `Modal` | palette, dialogs | fill `panel`; 0,2,6 ink 8%; 0,24,56 ink 18%; edge ink 8% | fill #303439; 0,2,6 black 50%; 0,24,56 black 50%; edge white 10%; highlight white 8% |

Dark `muted` moves from #9099A4 to #98A1AC so it keeps 4.5:1 on the `Modal` fill (4.34:1 before).

Still not used: gradients, glows, real blur (egui has no blur pass; the 0.99 push-back plus scrim stands in for it).

## 4. Decisions for the owner

- **D1 (recommended: yes).** Replace "hairline borders only on hover or drag … one soft shadow" and the "Not used" line in `docs/DESIGN.md` with the elevation table above. Without it, findings 5–7 stay.
- **D2 (recommended: yes, as the last task).** Arc-style frame: tint the backdrop (light #F1F2F4, dark #141619) and make the canvas an inset card at `Card` level, so chrome reads as frame and the diagram as the content sheet. Costs about 1 percentage point of canvas area (8 px gutters) and requires `muted` light to darken to #636A76 to keep 4.5:1 on the tinted backdrop. Without D2 the backdrop stays #FAFAF8 and only the side cards gain depth.

## 5. Out of scope

Real background blur, per-node diagram shadows, sound, haptics, animated diagram re-layout on edit (node morphing between layouts).
