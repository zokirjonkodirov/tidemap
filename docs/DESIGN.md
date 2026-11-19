# Design direction

Last updated 18 November 2026. The authoritative design is [`superpowers/specs/2026-11-18-own-engine-and-shell-redesign-design.md`](superpowers/specs/2026-11-18-own-engine-and-shell-redesign-design.md) (sections 6 and 7: shell redesign and tokens). This file keeps the design character and the final token values as read from the code. It replaces the earlier "warm clarity" direction (terracotta accent, warm paper surfaces, Manrope, Warm and Authored presets), which is no longer implemented.

## 1. Character

A quiet reading instrument. The diagram is the largest and strongest thing on screen. The shell is an inset, rounded set of cards on an off-white backdrop (near-black in dark mode), with one blue accent. Surfaces rise through four elevation levels (a fill, a contact shadow, a key shadow, a 1 px edge and, in dark mode, a 1 px top highlight); resize hairlines show only on hover or drag. Icons are painted vectors (`src/icons.rs`); no emoji or text glyphs are used as icons in the redesigned shell.

The Miro reference informed spacing, clear routing, shape diversity and readable role colours. Its toolbars, collaboration chrome, branding and node palette are not reproduced.

Layout, as built: a 40 px top bar (sidebar toggle, back and forward, breadcrumb, info, theme), a 260 px sidebar card with a virtualised tree, the canvas filling the rest (78.5% of a 1440x940 window), a floating pill for zoom, fit, source and export, a source card on the right that pushes the canvas, and a minimap that avoids the pill. Below 1000 px the sidebar is hidden and the source panel replaces the canvas. Source: `src/ui/`.

## 2. Tokens

Values below were read from `src/theme.rs` and `crates/diagram/src/style.rs` on 18 November 2026. If they differ from the code, the code wins.

### Shell palette (`src/theme.rs`, `Palette`)

| Token | Light | Dark | Use |
|---|---|---|---|
| `app` | `#FAFAF8` | `#1B1D20` | Window backdrop, canvas |
| `panel` | `#FFFFFF` | `#24272B` | Sidebar, source, popovers, pill |
| `hover` | ink `#1F2328` at alpha 13 | white at alpha 15 | Row and button hover |
| `ink` | `#1F2328` | `#E6E8EB` | Primary text |
| `secondary` | `#5F6672` | `#A9B0BA` | Supporting text |
| `muted` | `#6B7280` | `#98A1AC` | Hints, counts |
| `hairline` | ink at alpha 20 | white at alpha 20 | Divider shown on hover or drag |
| `accent` | `#41609E` | `#8FAEE0` | Selection, primary actions, focus ring |
| `accent_soft` | `#E9EEF7` | `#2A3447` | Active row, pressed controls |
| `on_accent` | `#FFFFFF` | `#14181C` | Text on accent |
| `danger` | `#A23B3B` | `#E19A9A` | Errors |

Contrast measured in the tokens report (WCAG): all text pairs are at least 4.5:1; the lowest is light `muted` on `app` at 4.63. Elevated surfaces carry a 1 px edge; the 2 px accent keyboard focus ring is the only widget stroke.

### Diagram palette (`crates/diagram/src/style.rs`, `DiagramColors`)

Fill and 1 px stroke per swatch.

| Token | Light fill / stroke | Dark fill / stroke | Default use |
|---|---|---|---|
| `canvas` | `#FAFAF8` | `#1B1D20` | Diagram surface |
| `text` | `#1F2328` | `#E6E8EB` | Labels |
| `edge` | `#8A919C` | `#7C848F` | Edges and arrowheads |
| `label_bg` | `#FFFFFF` | `#24272B` | Edge label pill |
| `sky` | `#DCEBFA` / `#8FB8E0` | `#26384A` / `#4F7396` | Process (rect, rounded) |
| `peach` | `#FFE3D1` / `#E8A982` | `#4A3327` / `#96684C` | Decision (diamond, hexagon) |
| `mint` | `#D9F2E3` / `#8CC9A6` | `#243F31` / `#4D8566` | Data store (cylinder) |
| `yellow` | `#FFF4C2` / `#E3CC6B` | `#453E22` / `#8F8040` | Start and end (stadium, circle, double circle) |
| `lavender` | `#E9E3FA` / `#B0A0DE` | `#352F4D` / `#6E61A0` | External (subroutine, asymmetric) |
| `gray` | `#EFF0F2` / `#B9BEC6` | `#2E3136` / `#5B616B` | Other (parallelograms, trapezoids) |
| `rose` | `#FBE0E0` / `#DE9C9C` | `#4A2A2C` / `#96575A` | Error role (defined; no shape maps to it) |

Colour by subgraph cycles sky, mint, peach, lavender, yellow over top-level subgraphs. Cluster tint is 0.35 by default and grows 40% per nesting depth, capped at 1. Authored fills win. Default text on every swatch has contrast of at least 8.7:1 (measured in the scene report).

Diagram defaults: edge width 1.5, arrow size 7, node stroke 1, cluster tint 0.35. All of these, and the palette, can be overridden in `style.json`.

### Geometry (`Metrics`, user-editable)

| Value | Default | Allowed range |
|---|---|---|
| `font_size` | 14 | 11 to 24 |
| `line_height` | 20 | at least 1.2 x font size |
| `max_label_width` | 220 | 80 to 600 |
| `pad_x`, `pad_y` | 18, 12 | 4 to 48 |
| `radius` | 12 | 4 to 32 |
| `rank_gap` | 64 | 16 to 240 |
| `node_gap` | 40 | 16 to 240 |
| `edge_label_font` | 13 | 13 to 20 |
| `cluster_pad` | 24 | 8 to 64 |
| `cluster_title` | 28 | 20 to 48 |
| `width_step` | 24 | 1 to 96 |

### Shell radii, spacing, type, sizes (`token` module)

| Group | Values |
|---|---|
| Spacing | 4, 8, 12, 16, 24, 32 px |
| Radii | control 8, panel 12, dialog 16, pill 999 px |
| Type sizes | UI 14, small 13, mono 13, title 20 px |
| Fonts | Inter Regular (UI), Inter Medium (node labels), Inter SemiBold (cluster titles, emphasis), IBM Plex Mono (source). Bundled in `assets/fonts` under the OFL |
| Heights | bar 40, row 32, control 32 px |
| Widths | sidebar 260 (220 to 360), source 420 (min 280), inset 8 px |
| Motion | Velocity-preserving springs (`src/motion.rs`): SNAP 0.16 s, SOFT 0.30 s, GLIDE 0.38 s, POP 0.34 s with a small overshoot; narrow-window breakpoint 1000 px |

`src/theme.rs` still holds a block of legacy tokens (`FONT_DISPLAY`, `HEIGHT_TOOLBAR`, `WIDTH_SEGMENT`, `GUTTER_TOP`, `GUTTER_PANEL`) used by dialogs, the welcome screen and the info popover until those are restyled.

### Elevation (`src/theme.rs`, `Level`, `elevation`, `elevated`)

Each level has a fill, a contact shadow, a key shadow, a 1 px edge drawn inside the shape and, in dark mode only, a 1 px top highlight inset by the corner radius. Shadow notation is `x, y, blur colour alpha`.

| Level | Used by | Light | Dark |
|---|---|---|---|
| `Card` | sidebar, source, outline cards | fill `panel`; 0,1,2 ink 5%; 0,2,8 ink 4%; edge ink 6% | fill `#24272B`; 0,1,2 black 30%; 0,4,12 black 20%; edge white 6%; highlight white 4% |
| `Raised` | pill, toast, minimap, LOD hint, search | fill `panel`; 0,1,3 ink 8%; 0,6,16 ink 8%; edge ink 7% | fill `#2A2D32`; 0,1,2 black 40%; 0,8,20 black 30%; edge white 8%; highlight white 6% |
| `Overlay` | popovers, menus, tooltips | fill `panel`; 0,1,3 ink 8%; 0,12,32 ink 12%; edge ink 8% | fill `#2D3136`; 0,1,3 black 45%; 0,12,32 black 40%; edge white 9%; highlight white 7% |
| `Modal` | palette, dialogs | fill `panel`; 0,2,6 ink 8%; 0,24,56 ink 18%; edge ink 8% | fill `#303439`; 0,2,6 black 50%; 0,24,56 black 50%; edge white 10%; highlight white 8% |

Dark `muted` is `#98A1AC` so it keeps 4.5:1 on the `Modal` fill. Tests in `theme.rs` hold the ordering, the dark edge separation and the text contrast on every fill.

Not used: gradients, glows, real blur.

## 3. Interactions

_Status as of 18 November 2026: the diagram rail, Trace and the "Fit updated diagram" action are design intent and not built. Click-to-highlight, Esc to clear, a camera per diagram and exact source selection are built._

**Diagram rail:** a slim title line beneath the file breadcrumb. Show the current block's heading and `2 of 6`, with previous/next controls. Expand to a searchable list on demand. File rows show small block counts only after discovery; unknown counts are never displayed as zero.

**Trace** (flowcharts; Next priority in the PRD, after the first release): select a supported node and press `T` or choose Trace. Preserve the full layout, emphasize connected edges, and gently reduce unrelated ink. A small contextual control expands one hop upstream/downstream. `Esc` returns to the full view. Do not make the whole canvas rearrange to focus. Cycles use visited sets; expansion has a bounded visible budget. Unsupported diagram families receive normal selection, not a misleading flow interpretation.

**Remembered places:** returning to a file restores its chosen block and camera. Updating labels preserves the camera anchor when possible. A quiet “Fit updated diagram” action appears if new content is offscreen. This makes repeated editing feel stable.

**Source certainty:** the status area independently communicates `Unsaved`, `Saved`, `Rendering`, `Preview current`, or `Showing previous valid preview`. Color supports these words. Clicking a diagnostic takes you to the correct full-file line.

## 4. Keyboard and accessibility

_The shortcuts as implemented are listed in the [README](../README.md#shortcuts). The table below is the intended set; Space-drag pan, `T` and remappable keys are not implemented._

| Action | Shortcut |
|---|---|
| Open folder | `Cmd/Ctrl+Shift+O` |
| Quick switch file/diagram | `Cmd/Ctrl+P` |
| Toggle source | `Cmd/Ctrl+E` |
| Save | `Cmd/Ctrl+S` |
| Find in source / active diagram | `Cmd/Ctrl+F`, scoped by focused pane |
| Previous/next diagram | `Alt+PageUp` / `Alt+PageDown`, remappable |
| Navigation history | Platform-standard back/forward, with menu alternatives |
| Fit / actual size | `F` / `1`, only when canvas is focused |
| Trace / clear focus | `T` / `Esc`, only when canvas is focused |
| Pan | Space-drag or middle-drag; trackpad gestures |

Every command has a menu alternative and accessible name. Shortcut letters never intercept normal editor typing. Check conflicts with window managers and localized keyboards. Use ≥32 px pointer targets and a comfortable 40 px target option; small icons can sit inside larger hit areas. All floating controls support focus trapping/escape correctly. Offer a text outline of diagram labels for screen readers and keyboard selection, with relationships included only for families that have a tested source/element mapping; canvas pixels alone cannot provide accessibility.

## 5. Design acceptance

Test file switching, multi-block discovery, error recovery, a 10,000-node overview, and editing at 800 × 600, 1,440 × 900, high DPI, dark mode, high contrast, and 200% text scaling. The strongest emphasis on each screen should be the diagram or current source task. Users should identify current file/block, save state, and preview freshness without opening a settings panel.
