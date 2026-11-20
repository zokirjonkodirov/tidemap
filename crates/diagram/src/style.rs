//! Style sheet, pastel palette and paint resolution (colour by role or subgraph).

use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use crate::graph::{ShapeKind, StyleProps};
use crate::metrics::Metrics;
use crate::scene::{ClusterItem, EdgeItem, NodeItem, Scene};

/// 8-bit RGBA colour.
pub type Rgba = [u8; 4];

/// A fill and its matching 1 px stroke.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Swatch {
    pub fill: Rgba,
    pub stroke: Rgba,
}

/// Semantic colour role of a node.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Process,
    Decision,
    Store,
    Terminal,
    External,
    Other,
    Error,
    /// Pseudo-states drawn in the line colour (state start, end, fork bars).
    Ink,
    /// Sticky note.
    Note,
}

/// What decides a node's swatch.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum ColorBy {
    #[default]
    Role,
    Subgraph,
}

/// Canvas, text, edge and swatch colours of one theme.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DiagramColors {
    pub canvas: Rgba,
    pub text: Rgba,
    pub edge: Rgba,
    pub label_bg: Rgba,
    pub sky: Swatch,
    pub peach: Swatch,
    pub mint: Swatch,
    pub yellow: Swatch,
    pub lavender: Swatch,
    pub gray: Swatch,
    pub rose: Swatch,
}

/// Everything that changes how a diagram looks.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleSheet {
    pub metrics: Metrics,
    pub light: DiagramColors,
    pub dark: DiagramColors,
    pub color_by: ColorBy,
    pub edge_width: f32,
    pub arrow_size: f32,
    pub node_stroke: f32,
    pub cluster_tint: f32,
}

/// Resolved colours and line style of one item. For edges `fill` is the label pill.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Paint {
    pub fill: Rgba,
    pub stroke: Rgba,
    pub text: Rgba,
    pub stroke_width: f32,
    pub dashed: bool,
}

const fn rgb(hex: u32) -> Rgba {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8, 255]
}

const fn swatch(fill: u32, stroke: u32) -> Swatch {
    Swatch {
        fill: rgb(fill),
        stroke: rgb(stroke),
    }
}

impl DiagramColors {
    /// Light theme from the design tokens.
    pub fn default_light() -> Self {
        DiagramColors {
            canvas: rgb(0xFAFAF8),
            text: rgb(0x1F2328),
            edge: rgb(0x8A919C),
            label_bg: rgb(0xFFFFFF),
            sky: swatch(0xDCEBFA, 0x8FB8E0),
            peach: swatch(0xFFE3D1, 0xE8A982),
            mint: swatch(0xD9F2E3, 0x8CC9A6),
            yellow: swatch(0xFFF4C2, 0xE3CC6B),
            lavender: swatch(0xE9E3FA, 0xB0A0DE),
            gray: swatch(0xEFF0F2, 0xB9BEC6),
            rose: swatch(0xFBE0E0, 0xDE9C9C),
        }
    }

    /// Dark theme from the design tokens.
    pub fn default_dark() -> Self {
        DiagramColors {
            canvas: rgb(0x1B1D20),
            text: rgb(0xE6E8EB),
            edge: rgb(0x7C848F),
            label_bg: rgb(0x24272B),
            sky: swatch(0x26384A, 0x4F7396),
            peach: swatch(0x4A3327, 0x96684C),
            mint: swatch(0x243F31, 0x4D8566),
            yellow: swatch(0x453E22, 0x8F8040),
            lavender: swatch(0x352F4D, 0x6E61A0),
            gray: swatch(0x2E3136, 0x5B616B),
            rose: swatch(0x4A2A2C, 0x96575A),
        }
    }
}

impl Default for DiagramColors {
    fn default() -> Self {
        Self::default_light()
    }
}

impl Default for StyleSheet {
    fn default() -> Self {
        StyleSheet {
            metrics: Metrics::default(),
            light: DiagramColors::default_light(),
            dark: DiagramColors::default_dark(),
            color_by: ColorBy::Role,
            edge_width: 1.5,
            arrow_size: 7.0,
            node_stroke: 1.0,
            cluster_tint: 0.35,
        }
    }
}

/// Clamps to `lo..=hi`; non-finite values become `fallback`.
fn limit(v: f32, lo: f32, hi: f32, fallback: f32) -> f32 {
    if v.is_finite() {
        v.clamp(lo, hi)
    } else {
        fallback
    }
}

impl StyleSheet {
    /// Forces every numeric value into its allowed range.
    pub fn clamped(mut self) -> StyleSheet {
        let d = Metrics::default();
        let m = &mut self.metrics;
        m.font_size = limit(m.font_size, 11.0, 24.0, d.font_size);
        m.line_height = limit(m.line_height, 0.0, 200.0, d.line_height).max(m.font_size * 1.2);
        m.max_label_width = limit(m.max_label_width, 80.0, 600.0, d.max_label_width);
        m.pad_x = limit(m.pad_x, 4.0, 48.0, d.pad_x);
        m.pad_y = limit(m.pad_y, 4.0, 48.0, d.pad_y);
        m.radius = limit(m.radius, 4.0, 32.0, d.radius);
        m.rank_gap = limit(m.rank_gap, 16.0, 240.0, d.rank_gap);
        m.node_gap = limit(m.node_gap, 16.0, 240.0, d.node_gap);
        m.edge_label_font = limit(m.edge_label_font, 13.0, 20.0, d.edge_label_font);
        m.cluster_pad = limit(m.cluster_pad, 8.0, 64.0, d.cluster_pad);
        m.cluster_title = limit(m.cluster_title, 20.0, 48.0, d.cluster_title);
        m.width_step = limit(m.width_step, 1.0, 96.0, d.width_step);
        self.edge_width = limit(self.edge_width, 0.5, 4.0, 1.5);
        self.arrow_size = limit(self.arrow_size, 4.0, 16.0, 7.0);
        self.node_stroke = limit(self.node_stroke, 0.5, 3.0, 1.0);
        self.cluster_tint = limit(self.cluster_tint, 0.05, 1.0, 0.35);
        self
    }

    /// Colours of the requested theme.
    pub fn colors(&self, dark: bool) -> &DiagramColors {
        if dark { &self.dark } else { &self.light }
    }

    /// Hash of `metrics` only: when it changes the diagram must be laid out again.
    pub fn layout_key(&self) -> u64 {
        let m = &self.metrics;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for v in [
            m.font_size,
            m.line_height,
            m.max_label_width,
            m.pad_x,
            m.pad_y,
            m.radius,
            m.rank_gap,
            m.node_gap,
            m.edge_label_font,
            m.cluster_pad,
            m.cluster_title,
            m.width_step,
        ] {
            v.to_bits().hash(&mut h);
        }
        h.finish()
    }
}

/// Colour role of a node shape.
pub fn role(shape: ShapeKind) -> Role {
    match shape {
        ShapeKind::Rect | ShapeKind::Rounded | ShapeKind::Record => Role::Process,
        ShapeKind::Diamond | ShapeKind::Hexagon | ShapeKind::Choice => Role::Decision,
        ShapeKind::Cylinder => Role::Store,
        ShapeKind::Stadium | ShapeKind::Circle | ShapeKind::DoubleCircle => Role::Terminal,
        ShapeKind::Subroutine | ShapeKind::Asymmetric => Role::External,
        ShapeKind::Parallelogram
        | ShapeKind::ParallelogramAlt
        | ShapeKind::Trapezoid
        | ShapeKind::TrapezoidAlt => Role::Other,
        ShapeKind::StateStart | ShapeKind::StateEnd | ShapeKind::Bar => Role::Ink,
        ShapeKind::Note => Role::Note,
    }
}

fn role_swatch(c: &DiagramColors, r: Role) -> Swatch {
    match r {
        Role::Process => c.sky,
        Role::Decision => c.peach,
        Role::Store => c.mint,
        Role::Terminal => c.yellow,
        Role::External => c.lavender,
        Role::Other => c.gray,
        Role::Error => c.rose,
        Role::Note => c.yellow,
        Role::Ink => Swatch {
            fill: c.edge,
            stroke: c.edge,
        },
    }
}

/// Swatches handed out to top-level subgraphs, in order.
fn cycle_swatch(c: &DiagramColors, index: usize) -> Swatch {
    [c.sky, c.mint, c.peach, c.lavender, c.yellow][index % 5]
}

/// Linear blend of two colours; `t` is the weight of `b`. Result is opaque.
fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let ch = |i: usize| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8;
    [ch(0), ch(1), ch(2), 255]
}

/// Composites `c` over an opaque background.
fn over(c: Rgba, background: Rgba) -> Rgba {
    mix(background, c, c[3] as f32 / 255.0)
}

fn linear(v: u8) -> f32 {
    let s = v as f32 / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance of an opaque colour.
pub fn luminance(c: Rgba) -> f32 {
    0.2126 * linear(c[0]) + 0.7152 * linear(c[1]) + 0.0722 * linear(c[2])
}

/// WCAG contrast ratio between two opaque colours (1 to 21).
pub fn contrast_ratio(a: Rgba, b: Rgba) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

const INK_ON_LIGHT: Rgba = rgb(0x1F2328);
const INK_ON_DARK: Rgba = rgb(0xF5F6F7);

/// Applies authored overrides to `paint`; returns whether a fill was authored.
fn apply(
    paint: &mut Paint,
    props: &StyleProps,
    color: &mut Option<Rgba>,
    stroke_set: &mut bool,
) -> bool {
    let mut authored_fill = false;
    if let Some(f) = props.fill {
        paint.fill = f;
        authored_fill = true;
    }
    if let Some(s) = props.stroke {
        paint.stroke = s;
        *stroke_set = true;
    }
    if let Some(c) = props.color {
        *color = Some(c);
    }
    if let Some(w) = props.stroke_width
        && w.is_finite()
    {
        paint.stroke_width = w.clamp(0.0, 16.0);
    }
    paint.dashed |= props.dashed;
    authored_fill
}

/// Resolves authored style (class definitions in order, then inline) onto `paint`.
fn resolve_authored(
    paint: &mut Paint,
    classes: &[u16],
    inline: &StyleProps,
    scene: &Scene,
    colors: &DiagramColors,
) {
    let mut color = None;
    let mut stroke_set = false;
    let mut authored_fill = false;
    for &ci in classes {
        if let Some(def) = scene.class_defs.get(ci as usize) {
            authored_fill |= apply(paint, &def.props, &mut color, &mut stroke_set);
        }
    }
    authored_fill |= apply(paint, inline, &mut color, &mut stroke_set);

    if authored_fill {
        let flat = over(paint.fill, colors.canvas);
        if paint.fill[3] == 255 && !stroke_set {
            // Derive a border from an authored fill so it never keeps a clashing pastel stroke.
            paint.stroke = if luminance(flat) > 0.4 {
                mix(flat, [0, 0, 0, 255], 0.22)
            } else {
                mix(flat, [255, 255, 255, 255], 0.28)
            };
        }
        paint.text = if contrast_ratio(INK_ON_LIGHT, flat) >= contrast_ratio(INK_ON_DARK, flat) {
            INK_ON_LIGHT
        } else {
            INK_ON_DARK
        };
    }
    if let Some(c) = color {
        paint.text = c;
    }
}

fn is_error_class(name: &str) -> bool {
    ["error", "danger", "fail", "failed"]
        .iter()
        .any(|k| name.eq_ignore_ascii_case(k))
}

/// Colours of a node: role swatch, then class definitions, then inline style.
pub fn node_paint(n: &NodeItem, scene: &Scene, s: &StyleSheet, dark: bool) -> Paint {
    let colors = s.colors(dark);
    let has_error_class = n.classes.iter().any(|&c| {
        scene
            .class_defs
            .get(c as usize)
            .is_some_and(|d| is_error_class(&d.name))
    });
    let base = if has_error_class {
        colors.rose
    } else if s.color_by == ColorBy::Subgraph {
        match scene.node_top_level_index(n.id) {
            Some(i) => cycle_swatch(colors, i),
            None => colors.gray,
        }
    } else {
        role_swatch(colors, role(n.role_shape))
    };
    let mut paint = Paint {
        fill: base.fill,
        stroke: base.stroke,
        text: colors.text,
        stroke_width: s.node_stroke,
        dashed: false,
    };
    if n.shape == ShapeKind::StateEnd {
        // Ring only; the inner disc is drawn in the stroke colour.
        paint.fill = [0, 0, 0, 0];
    }
    resolve_authored(&mut paint, &n.classes, &n.style, scene, colors);
    paint
}

/// Colours of a subgraph container: its top-level swatch thinned toward the canvas.
pub fn cluster_paint(c: &ClusterItem, scene: &Scene, s: &StyleSheet, dark: bool) -> Paint {
    let colors = s.colors(dark);
    let base = cycle_swatch(colors, scene.top_level_index(c.id));
    // Nested containers get slightly more tint so they stay visible on their parent.
    let tint = (s.cluster_tint * (1.0 + 0.4 * c.depth as f32)).min(1.0);
    let mut paint = Paint {
        fill: mix(colors.canvas, base.fill, tint),
        stroke: mix(colors.canvas, base.stroke, 0.6),
        text: colors.text,
        stroke_width: s.node_stroke,
        dashed: false,
    };
    resolve_authored(&mut paint, &c.classes, &c.style, scene, colors);
    paint
}

/// Colours of an edge. `fill` is the label pill background.
pub fn edge_paint(e: &EdgeItem, scene: &Scene, s: &StyleSheet, dark: bool) -> Paint {
    use crate::graph::LineKind;
    let _ = scene;
    let colors = s.colors(dark);
    let thick = if e.line == LineKind::Thick { 1.6 } else { 1.0 };
    Paint {
        fill: colors.label_bg,
        stroke: e.style.stroke.unwrap_or(colors.edge),
        text: e.style.color.unwrap_or(colors.text),
        stroke_width: e
            .style
            .stroke_width
            .filter(|w| w.is_finite())
            .map_or(s.edge_width * thick, |w| w.clamp(0.0, 16.0)),
        dashed: e.line == LineKind::Dotted || e.style.dashed,
    }
}
