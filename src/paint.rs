//! Paints a diagram `Scene` with egui primitives and real text.
//!
//! Geometry mirrors `tidemap_diagram::svg`: outlines come from
//! `shape::outline`, details (rims, bars, rings), arrowheads and label pills
//! are the same, and colours come from the `*_paint` resolvers.

use egui::epaint::PathShape;
use egui::{Color32, FontFamily, FontId, Mesh, Painter, Pos2, Shape, Stroke, Vec2};
use tidemap_diagram::shape::{self, PathCmd};
use tidemap_diagram::ends;
use tidemap_diagram::svg::{ARROW_LENGTH, end_reach};
use tidemap_diagram::{
    ArrowKind, ClusterItem, EdgeItem, ItemId, LineKind, NodeId, NodeItem, Paint, Rect, Rgba, Scene,
    ShapeKind, StyleSheet, cluster_paint, edge_paint, node_paint,
};
use tidemap_model::Camera;

use crate::{text, theme};

/// Items outside the viewport by less than this many world units are still painted.
const CULL_MARGIN: f32 = 32.0;
/// Text smaller than this on screen is not painted.
const MIN_TEXT_PX: f32 = 5.0;
/// Share of the colour kept by items outside the selection.
const DIM: f32 = 0.28;
/// Least share of its colour a label keeps while everything else is dimmed.
const TEXT_KEEP: f32 = 0.62;
const CLUSTER_RADIUS: f32 = 16.0;
const CLUSTER_TITLE_SIZE: f32 = 13.0;
const LABEL_RADIUS: f32 = 8.0;
/// Smallest on-screen corner radius of any painted rectangle.
const MIN_RADIUS_PX: f32 = 4.0;

/// How much detail is painted at the current zoom.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lod {
    Full,
    NoText,
    Flat,
}

/// Level of detail for the given zoom, node font size and number of visible items.
pub fn lod(zoom: f32, font_size: f32, visible_items: usize) -> Lod {
    let text_px = zoom * font_size;
    if text_px < 2.5 || visible_items > 6000 {
        Lod::Flat
    } else if text_px < 5.0 || visible_items > 2500 {
        Lod::NoText
    } else {
        Lod::Full
    }
}

/// Which items stand out. With a selection everything else is dimmed.
pub struct Emphasis<'a> {
    pub selected: Option<NodeId>,
    pub strong_nodes: &'a [NodeId],
    pub strong_edges: &'a [tidemap_diagram::EdgeId],
    pub matches: &'a [NodeId],
    /// Progress of the dimming of everything else, 0 (none) to 1 (full).
    pub dim: f32,
}

/// What a frame painted.
#[derive(Clone, Copy, Debug)]
pub struct PaintStats {
    pub items: usize,
    pub texts: usize,
    pub lod: Lod,
}

fn color(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

fn stroke_of(width: f32, c: Color32) -> Stroke {
    if width <= 0.0 || c.a() == 0 {
        Stroke::NONE
    } else {
        Stroke::new(width, c)
    }
}

type Pt = [f32; 2];

/// Camera transform from world units to screen points.
#[derive(Clone, Copy)]
struct View {
    origin: Pos2,
    camera: Camera,
}

impl View {
    fn pt(&self, p: Pt) -> Pos2 {
        Pos2::new(
            self.origin.x + self.camera.x + p[0] * self.camera.zoom,
            self.origin.y + self.camera.y + p[1] * self.camera.zoom,
        )
    }

    fn rect(&self, r: Rect) -> egui::Rect {
        egui::Rect::from_min_size(self.pt([r.x, r.y]), Vec2::new(r.w, r.h) * self.camera.zoom)
    }

    fn zoom(&self) -> f32 {
        self.camera.zoom
    }
}

/// Number of line segments for a cubic whose control polygon is `length` px long.
fn segments(length: f32) -> usize {
    ((length.max(0.0).sqrt() * 1.3).ceil() as usize).clamp(2, 64)
}

fn cubic_point(p0: Pos2, p1: Pos2, p2: Pos2, p3: Pos2, t: f32) -> Pos2 {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    Pos2::new(
        a * p0.x + b * p1.x + c * p2.x + d * p3.x,
        a * p0.y + b * p1.y + c * p2.y + d * p3.y,
    )
}

/// Appends a flattened cubic (without its start point).
fn push_cubic(out: &mut Vec<Pos2>, p0: Pos2, p1: Pos2, p2: Pos2, p3: Pos2) {
    let length = p0.distance(p1) + p1.distance(p2) + p2.distance(p3);
    let n = segments(length);
    for i in 1..=n {
        out.push(cubic_point(p0, p1, p2, p3, i as f32 / n as f32));
    }
}

/// Removes consecutive duplicates; for closed contours also the repeated start point.
fn tidy(points: &mut Vec<Pos2>, closed: bool) {
    points.dedup_by(|a, b| a.distance(*b) < 0.01);
    if closed {
        while points.len() > 1 && points[0].distance(points[points.len() - 1]) < 0.01 {
            points.pop();
        }
    }
}

/// Flattens a closed outline into screen points.
fn flatten_outline(cmds: &[PathCmd], view: &View) -> Vec<Pos2> {
    let mut out: Vec<Pos2> = Vec::with_capacity(cmds.len() * 6);
    let mut current = Pos2::ZERO;
    for cmd in cmds {
        match *cmd {
            PathCmd::Move(p) => {
                current = view.pt(p);
                out.push(current);
            }
            PathCmd::Line(p) => {
                current = view.pt(p);
                out.push(current);
            }
            PathCmd::Cubic(a, b, p) => {
                let end = view.pt(p);
                push_cubic(&mut out, current, view.pt(a), view.pt(b), end);
                current = end;
            }
            PathCmd::Close => {}
        }
    }
    tidy(&mut out, true);
    out
}

/// Flattens an edge's cubic chain (or polyline when it is not a valid chain).
fn flatten_chain(points: &[Pt], view: &View) -> Vec<Pos2> {
    let mut out = Vec::new();
    let Some(first) = points.first() else {
        return out;
    };
    out.push(view.pt(*first));
    if points.len() >= 4 && (points.len() - 1) % 3 == 0 {
        let mut current = view.pt(*first);
        for c in points[1..].chunks_exact(3) {
            let end = view.pt(c[2]);
            push_cubic(&mut out, current, view.pt(c[0]), view.pt(c[1]), end);
            current = end;
        }
    } else {
        out.extend(points[1..].iter().map(|p| view.pt(*p)));
    }
    tidy(&mut out, false);
    out
}

fn cross(o: Pos2, a: Pos2, b: Pos2) -> f32 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

fn is_convex(points: &[Pos2]) -> bool {
    let n = points.len();
    if n < 4 {
        return true;
    }
    let mut sign = 0.0f32;
    for i in 0..n {
        let c = cross(points[i], points[(i + 1) % n], points[(i + 2) % n]);
        if c.abs() < 1e-4 {
            continue;
        }
        if sign == 0.0 {
            sign = c.signum();
        } else if c.signum() != sign {
            return false;
        }
    }
    true
}

fn signed_area(points: &[Pos2]) -> f32 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum::<f32>()
        / 2.0
}

fn in_triangle(p: Pos2, a: Pos2, b: Pos2, c: Pos2) -> bool {
    let (d1, d2, d3) = (cross(a, b, p), cross(b, c, p), cross(c, a, p));
    let negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(negative && positive)
}

/// Ear-clipping triangulation of a simple polygon.
fn triangulate(points: &[Pos2]) -> Vec<[u32; 3]> {
    let n = points.len();
    let mut triangles = Vec::with_capacity(n.saturating_sub(2));
    if n < 3 {
        return triangles;
    }
    let orientation = if signed_area(points) >= 0.0 {
        1.0
    } else {
        -1.0
    };
    let mut ring: Vec<usize> = (0..n).collect();
    let mut guard = n * n + 8;
    while ring.len() > 3 && guard > 0 {
        guard -= 1;
        let m = ring.len();
        let mut clipped = false;
        for i in 0..m {
            let (ia, ib, ic) = (ring[(i + m - 1) % m], ring[i], ring[(i + 1) % m]);
            let (a, b, c) = (points[ia], points[ib], points[ic]);
            if cross(a, b, c) * orientation <= 1e-6 {
                continue;
            }
            let blocked = ring
                .iter()
                .any(|&j| j != ia && j != ib && j != ic && in_triangle(points[j], a, b, c));
            if blocked {
                continue;
            }
            triangles.push([ia as u32, ib as u32, ic as u32]);
            ring.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            // Degenerate input: fan out the rest so something is still filled.
            break;
        }
    }
    if ring.len() >= 3 {
        for k in 1..ring.len() - 1 {
            triangles.push([ring[0] as u32, ring[k] as u32, ring[k + 1] as u32]);
        }
    }
    triangles
}

struct Frame<'a> {
    painter: &'a Painter,
    view: View,
    style: &'a StyleSheet,
    scene: &'a Scene,
    dark: bool,
    accent: Color32,
    canvas: Color32,
    selected: Option<NodeId>,
    strong_nodes: Vec<u32>,
    strong_edges: Vec<u32>,
    matches: Vec<u32>,
    texts: usize,
    /// How far dimming has progressed, 0 to 1 (animated by the canvas).
    dim: f32,
    /// Share of colour kept by dimmed items at this point of the animation.
    keep: f32,
    /// Few enough items on screen that far-away edges keep their curves.
    curved_far_edges: bool,
}

impl Frame<'_> {
    fn dimming(&self) -> bool {
        self.selected.is_some() || !self.strong_edges.is_empty() || !self.strong_nodes.is_empty()
    }

    fn node_strong(&self, id: NodeId) -> bool {
        !self.dimming()
            || self.selected == Some(id)
            || self.strong_nodes.binary_search(&id.0).is_ok()
    }

    fn edge_strong(&self, id: tidemap_diagram::EdgeId) -> bool {
        !self.dimming() || self.strong_edges.binary_search(&id.0).is_ok()
    }

    /// A colour at full strength, or dimmed against the canvas when `strong` is false.
    fn tone(&self, c: Color32, strong: bool) -> Color32 {
        if strong { c } else { c.gamma_multiply(self.keep) }
    }

    /// Text colour: dimmed less than shapes, so labels of de-emphasised items stay readable.
    fn text_tone(&self, c: Color32, strong: bool) -> Color32 {
        if strong || c.a() == 0 {
            c
        } else {
            self.canvas.lerp_to_gamma(c, self.keep.max(TEXT_KEEP))
        }
    }

    /// Dimmed opaque fill, so dimmed items never show what lies beneath them.
    fn fill_tone(&self, c: Color32, strong: bool) -> Color32 {
        if strong || c.a() == 0 {
            c
        } else {
            self.canvas.lerp_to_gamma(c, self.keep)
        }
    }

    /// Fills and strokes a closed screen polygon; handles concave outlines.
    fn closed(&self, points: Vec<Pos2>, fill: Color32, stroke: Stroke, dash: Option<(f32, f32)>) {
        if points.len() < 3 {
            return;
        }
        let line_stroke = if dash.is_some() { Stroke::NONE } else { stroke };
        if fill.a() > 0 {
            if is_convex(&points) {
                self.painter
                    .add(PathShape::convex_polygon(points.clone(), fill, line_stroke));
            } else {
                let mut mesh = Mesh::default();
                for p in &points {
                    mesh.colored_vertex(*p, fill);
                }
                for [a, b, c] in triangulate(&points) {
                    mesh.add_triangle(a, b, c);
                }
                self.painter.add(Shape::mesh(mesh));
                if line_stroke != Stroke::NONE {
                    self.painter
                        .add(PathShape::closed_line(points.clone(), line_stroke));
                }
            }
        } else if line_stroke != Stroke::NONE {
            self.painter
                .add(PathShape::closed_line(points.clone(), line_stroke));
        }
        if let Some((dash, gap)) = dash
            && stroke != Stroke::NONE
        {
            let mut path = points;
            path.push(path[0]);
            self.painter
                .extend(Shape::dashed_line(&path, stroke, dash, gap));
        }
    }

    fn outline(&self, kind: ShapeKind, rect: Rect, radius: f32) -> Vec<Pos2> {
        flatten_outline(&shape::outline(kind, rect, radius), &self.view)
    }

    /// Paints `lines` centred on `center` (world units).
    #[allow(clippy::too_many_arguments)]
    fn text_block(
        &mut self,
        lines: &[String],
        center: Pt,
        line_height: f32,
        size: f32,
        family: FontFamily,
        c: Color32,
    ) {
        let zoom = self.view.zoom();
        let px = size * zoom;
        if lines.is_empty() || px < MIN_TEXT_PX || c.a() == 0 {
            return;
        }
        let (raster, scale) = text::raster(px);
        let first = center[1] - lines.len() as f32 * line_height / 2.0 + line_height / 2.0;
        for (i, line) in lines.iter().enumerate() {
            if line.is_empty() {
                continue;
            }
            let middle = self.view.pt([center[0], first + i as f32 * line_height]);
            let galley =
                self.painter
                    .layout_no_wrap(line.clone(), FontId::new(raster, family.clone()), c);
            let pos = middle - galley.size() * (0.5 * scale);
            text::paint_scaled(&self.painter, pos, galley, c, scale);
            self.texts += 1;
        }
    }

    fn cluster(&mut self, c: &ClusterItem, level: Lod) {
        let paint = cluster_paint(c, self.scene, self.style, self.dark);
        let strong = !self.dimming();
        let fill = self.tone(color(paint.fill), strong);
        if level == Lod::Flat {
            let rect = self.view.rect(c.rect);
            let radius = (CLUSTER_RADIUS * self.view.zoom()).clamp(1.0, 6.0);
            self.painter.rect_filled(rect, radius, fill);
            if rect.width() > 6.0 && rect.height() > 6.0 {
                let edge = self.tone(color(paint.stroke), strong).gamma_multiply(0.7);
                self.painter.rect_stroke(rect, radius, Stroke::new(1.0_f32, edge), egui::StrokeKind::Inside);
            }
            return;
        }
        let zoom = self.view.zoom();
        let r = c.rect;
        let radius = CLUSTER_RADIUS
            .max(MIN_RADIUS_PX / zoom)
            .min(r.w / 2.0)
            .min(r.h / 2.0);
        let points = self.outline(ShapeKind::Rect, r, radius);
        let stroke = stroke_of(
            node_stroke_px(&paint, zoom),
            self.tone(color(paint.stroke), strong),
        );
        let dash = paint.dashed.then_some((6.0 * zoom, 4.0 * zoom));
        self.closed(points, fill, stroke, dash);
        if level == Lod::Full && !c.title.is_empty() {
            let zone = self.style.metrics.cluster_title.min(r.h);
            let text_color = self.text_tone(color(paint.text), strong);
            let px = CLUSTER_TITLE_SIZE * zoom;
            if px >= MIN_TEXT_PX {
                let middle = self.view.pt([r.x + 12.0, r.y + zone / 2.0]);
                let (raster, scale) = text::raster(px);
                let galley = self.painter.layout_no_wrap(
                    c.title.clone(),
                    FontId::new(raster, theme::font_semibold()),
                    text_color,
                );
                let pos = Pos2::new(middle.x, middle.y - galley.size().y * 0.5 * scale);
                text::paint_scaled(&self.painter, pos, galley, text_color, scale);
                self.texts += 1;
            }
        }
    }

    /// Decoration at a line end. `dir` points at the tip and follows the visible curve.
    fn end_mark(
        &self,
        kind: ArrowKind,
        tip: Pt,
        dir: Pt,
        size: f32,
        stroke_w: f32,
        c: Color32,
    ) -> Vec<Shape> {
        let zoom = self.view.zoom();
        let n = [-dir[1], dir[0]];
        let at = |along: f32, across: f32| -> Pt {
            [
                tip[0] - dir[0] * along + n[0] * across,
                tip[1] - dir[1] * along + n[1] * across,
            ]
        };
        match kind {
            ArrowKind::None => Vec::new(),
            ArrowKind::Arrow => {
                // Rounded triangle: soft tip on the outline, soft base corners.
                let (corners, _) = ends::arrow_head(tip, dir, size);
                let steps = if size * zoom > 14.0 { 6 } else { 3 };
                let points: Vec<Pos2> = ends::arrow_polygon(&corners, steps)
                    .into_iter()
                    .map(|p| self.view.pt(p))
                    .collect();
                vec![Shape::Path(PathShape::convex_polygon(points, c, Stroke::NONE))]
            }
            ArrowKind::Circle => {
                let r = size * 0.45 * ARROW_LENGTH;
                let center = self.view.pt(at(r, 0.0));
                vec![Shape::circle_filled(center, r * zoom, c)]
            }
            ArrowKind::Cross => {
                let h = size * 0.7 * ARROW_LENGTH;
                let k = std::f32::consts::FRAC_1_SQRT_2;
                let center = at(h * k, 0.0);
                let u = [(dir[0] + n[0]) * h * k, (dir[1] + n[1]) * h * k];
                let v = [(dir[0] - n[0]) * h * k, (dir[1] - n[1]) * h * k];
                let width = ((stroke_w * 1.1).max(1.2) * zoom).max(1.0);
                let seg = |p: Pt, q: Pt| {
                    let (a, b) = (self.view.pt(p), self.view.pt(q));
                    // Round caps so the cross has no sharp ends either.
                    vec![
                        Shape::line_segment([a, b], Stroke::new(width, c)),
                        Shape::circle_filled(a, width * 0.5, c),
                        Shape::circle_filled(b, width * 0.5, c),
                    ]
                };
                let mut shapes = seg(
                    [center[0] - u[0], center[1] - u[1]],
                    [center[0] + u[0], center[1] + u[1]],
                );
                shapes.extend(seg(
                    [center[0] - v[0], center[1] - v[1]],
                    [center[0] + v[0], center[1] + v[1]],
                ));
                shapes
            }
        }
    }

    fn edge(&mut self, e: &EdgeItem, level: Lod) {
        if e.line == LineKind::Invisible || e.points.len() < 2 {
            return;
        }
        let paint = edge_paint(e, self.scene, self.style, self.dark);
        let strong = self.edge_strong(e.id);
        let highlighted = self.dimming() && strong;
        let mut stroke_color = color(paint.stroke);
        if highlighted {
            stroke_color = stroke_color.lerp_to_gamma(self.accent, 0.7 * self.dim);
        }
        // Opaque dimming: the line tucks under its arrowhead, so a translucent colour would
        // show the overlap.
        let stroke_color = self.fill_tone(stroke_color, strong);
        let zoom = self.view.zoom();
        let width_world = paint.stroke_width * if highlighted { 1.3 } else { 1.0 };
        let width = (width_world * zoom).max(1.0);

        if level == Lod::Flat {
            // Far away, lines are a hint of structure: thin and quiet, still following their curves.
            let far = Stroke::new(1.0_f32, stroke_color.gamma_multiply(0.6));
            if self.curved_far_edges {
                let path = flatten_chain(&e.points, &self.view);
                if path.len() >= 2 {
                    self.painter.add(Shape::line(path, far));
                }
            } else if let (Some(a), Some(b)) = (e.points.first(), e.points.last()) {
                self.painter.line_segment([self.view.pt(*a), self.view.pt(*b)], far);
            }
            return;
        }

        let size = self.style.arrow_size;
        let mut marks = Vec::new();
        let (reach_end, reach_start) = (end_reach(e.head, size), end_reach(e.tail, size));
        if let (Some(dir), Some(tip)) = (ends::end_dir(&e.points, reach_end), e.points.last()) {
            marks.extend(self.end_mark(e.head, *tip, dir, size, paint.stroke_width, stroke_color));
        }
        if let (Some(dir), Some(tip)) = (ends::start_dir(&e.points, reach_start), e.points.first()) {
            marks.extend(self.end_mark(e.tail, *tip, dir, size, paint.stroke_width, stroke_color));
        }

        // The line runs a little way under its decorations so no seam shows.
        let pts = ends::trimmed(&e.points, reach_start, reach_end);
        let path = flatten_chain(&pts, &self.view);
        let stroke = Stroke::new(width, stroke_color);
        if path.len() >= 2 {
            if paint.dashed {
                let dash = (width * 1.6).max(3.0);
                let gap = (width * 3.0).max(4.0);
                self.painter
                    .extend(Shape::dashed_line(&path, stroke, dash, gap));
            } else {
                self.painter.add(Shape::line(path, stroke));
            }
        }
        self.painter.extend(marks);
    }

    fn edge_label(&mut self, e: &EdgeItem) {
        if e.line == LineKind::Invisible {
            return;
        }
        let Some((r, text)) = &e.label else {
            return;
        };
        let paint = edge_paint(e, self.scene, self.style, self.dark);
        let strong = self.edge_strong(e.id);
        let zoom = self.view.zoom();
        let radius = LABEL_RADIUS
            .max(MIN_RADIUS_PX / zoom)
            .min(r.h / 2.0)
            .min(r.w / 2.0);
        let mut border = paint.stroke;
        border[3] = 90;
        let points = self.outline(ShapeKind::Rect, *r, radius);
        let fill = self.fill_tone(color(paint.fill), strong);
        let stroke = stroke_of(1.0, self.tone(color(border), strong));
        self.closed(points, fill, stroke, None);
        let size = self.style.metrics.edge_label_font;
        let line_height = self.style.metrics.line_height.min(size * 1.4).max(size);
        let text_color = self.text_tone(color(paint.text), strong);
        self.text_block(
            &text.lines,
            r.center(),
            line_height,
            size,
            FontFamily::Proportional,
            text_color,
        );
    }

    /// Selection and search rings around a node, drawn above it.
    fn rings(&self, n: &NodeItem, level: Lod) {
        let selected = self.selected == Some(n.id);
        let matched = self.matches.binary_search(&n.id.0).is_ok();
        if !selected && !matched {
            return;
        }
        let zoom = self.view.zoom();
        let ring = if selected {
            // The ring fades in with the dimming of everything else.
            self.accent.gamma_multiply(self.dim.max(0.05))
        } else {
            self.accent.gamma_multiply(0.6)
        };
        let stroke = Stroke::new(2.0_f32, ring);
        let offset = 3.0 / zoom;
        let grown = n.rect.inflate(offset);
        if level == Lod::Flat {
            self.painter
                .rect_stroke(self.view.rect(grown), 2.0, stroke, egui::StrokeKind::Middle);
            return;
        }
        let radius = self.style.metrics.radius + offset;
        let points = self.outline(n.shape, grown, radius);
        self.closed(points, Color32::TRANSPARENT, stroke, None);
    }

    fn node(&mut self, n: &NodeItem, level: Lod) {
        let paint = node_paint(n, self.scene, self.style, self.dark);
        let strong = self.node_strong(n.id);
        let fill = self.fill_tone(color(paint.fill), strong);
        if level == Lod::Flat {
            // A soft rounded tile with a hairline edge and bars where the text would be.
            let rect = self.view.rect(n.rect);
            let radius = (self.style.metrics.radius * self.view.zoom()).clamp(1.5, 6.0).min(rect.height() * 0.5);
            self.painter.rect_filled(rect, radius, fill);
            if rect.width() > 5.0 && rect.height() > 5.0 {
                let edge = self.tone(color(paint.stroke), strong).gamma_multiply(0.75);
                self.painter.rect_stroke(rect, radius, Stroke::new(1.0_f32, edge), egui::StrokeKind::Inside);
            }
            self.skeleton(n, self.text_tone(color(paint.text), strong));
            self.rings(n, level);
            return;
        }
        let zoom = self.view.zoom();
        let m = &self.style.metrics;
        let (radius, font_size, line_height) = (m.radius, m.font_size, m.line_height);
        let r = n.rect;
        let stroke_color = self.tone(color(paint.stroke), strong);
        let width = node_stroke_px(&paint, zoom);
        let stroke = stroke_of(width, stroke_color);
        let dash = paint.dashed.then_some((3.0 * zoom, 4.0 * zoom));
        let points = self.outline(n.shape, r, radius);
        self.closed(points, fill, stroke, dash);

        let mut text_dy = 0.0;
        match n.shape {
            ShapeKind::Cylinder => {
                // Front half of the top rim; the body outline already curves top and bottom.
                let ry = 8.0f32.min(r.h / 4.0);
                let (x0, x1, cx) = (r.x, r.right(), r.center()[0]);
                let y = r.y + ry;
                let k = ry * 0.552_284_75;
                let kx = r.w / 2.0 * 0.552_284_75;
                let cmds = [
                    PathCmd::Move([x0, y]),
                    PathCmd::Cubic([x0, y + k], [cx - kx, y + ry], [cx, y + ry]),
                    PathCmd::Cubic([cx + kx, y + ry], [x1, y + k], [x1, y]),
                ];
                self.detail_line(&cmds, stroke);
                text_dy = ry / 2.0;
            }
            ShapeKind::Subroutine => {
                let corner = radius.min(r.h / 2.0).min(r.w / 2.0);
                let inset = 8.0;
                // Keep the bars inside the rounded corners.
                let dy = if corner > inset {
                    corner - (corner * corner - (corner - inset).powi(2)).max(0.0).sqrt() + 0.5
                } else {
                    0.0
                };
                for x in [r.x + inset, r.right() - inset] {
                    let cmds = [
                        PathCmd::Move([x, r.y + dy]),
                        PathCmd::Line([x, r.bottom() - dy]),
                    ];
                    self.detail_line(&cmds, stroke);
                }
            }
            ShapeKind::DoubleCircle => {
                let inner = Rect {
                    x: r.x + 5.0,
                    y: r.y + 5.0,
                    w: (r.w - 10.0).max(0.0),
                    h: (r.h - 10.0).max(0.0),
                };
                let points = self.outline(ShapeKind::Circle, inner, radius);
                self.closed(points, Color32::TRANSPARENT, stroke, None);
            }
            _ => {}
        }

        if level == Lod::NoText {
            self.skeleton(n, self.text_tone(color(paint.text), strong));
        }
        if level == Lod::Full {
            let [cx, cy] = r.center();
            let text_color = self.text_tone(color(paint.text), strong);
            self.text_block(
                &n.text.lines,
                [cx, cy + text_dy],
                line_height,
                font_size,
                theme::font_medium(),
                text_color,
            );
        }
        self.rings(n, level);
    }

    /// Quiet rounded bars where a node's text lines would be, so a zoomed-out diagram still shows
    /// what is label and what is empty space.
    fn skeleton(&self, n: &NodeItem, text: Color32) {
        let zoom = self.view.zoom();
        let m = &self.style.metrics;
        let tile = self.view.rect(n.rect);
        if tile.width() < 8.0 || tile.height() < 6.0 || n.text.lines.is_empty() {
            return;
        }
        let bar_h = (m.font_size * 0.42 * zoom).clamp(1.6, 5.0);
        let pitch = (m.line_height * zoom).max(bar_h * 1.9);
        let lines = n.text.lines.iter().filter(|line| !line.is_empty()).count().max(1);
        let block = pitch * (lines as f32 - 1.0) + bar_h;
        if block > tile.height() - 2.0 {
            // Too many lines for the tile: one bar for all of them.
            let width = (n.text.width * zoom).min(tile.width() - 6.0).max(bar_h);
            let bar = egui::Rect::from_center_size(tile.center(), egui::vec2(width, bar_h));
            self.painter.rect_filled(bar, bar_h * 0.5, text.gamma_multiply(0.34));
            return;
        }
        let mut y = tile.center().y - block * 0.5 + bar_h * 0.5;
        for line in n.text.lines.iter().filter(|line| !line.is_empty()) {
            let chars = line.chars().count() as f32;
            let width = (chars * m.font_size * 0.5 * zoom).min(tile.width() - 6.0).max(bar_h);
            let bar = egui::Rect::from_center_size(Pos2::new(tile.center().x, y), egui::vec2(width, bar_h));
            self.painter.rect_filled(bar, bar_h * 0.5, text.gamma_multiply(0.34));
            y += pitch;
        }
    }

    /// Open stroked path of decoration lines.
    fn detail_line(&self, cmds: &[PathCmd], stroke: Stroke) {
        if stroke == Stroke::NONE {
            return;
        }
        let mut out: Vec<Pos2> = Vec::new();
        let mut current = Pos2::ZERO;
        for cmd in cmds {
            match *cmd {
                PathCmd::Move(p) | PathCmd::Line(p) => {
                    current = self.view.pt(p);
                    out.push(current);
                }
                PathCmd::Cubic(a, b, p) => {
                    let end = self.view.pt(p);
                    push_cubic(&mut out, current, self.view.pt(a), self.view.pt(b), end);
                    current = end;
                }
                PathCmd::Close => {}
            }
        }
        tidy(&mut out, false);
        if out.len() >= 2 {
            self.painter.add(Shape::line(out, stroke));
        }
    }
}

/// On-screen stroke width of a node or cluster outline.
fn node_stroke_px(paint: &Paint, zoom: f32) -> f32 {
    if paint.stroke_width <= 0.0 {
        0.0
    } else {
        (paint.stroke_width * zoom.min(2.0)).max(1.0)
    }
}

/// Paints the visible part of `scene`. `viewport` is the screen rectangle the camera maps
/// world `(0, 0)` into at offset `(camera.x, camera.y)`. `scratch` is reused between frames.
#[allow(clippy::too_many_arguments)]
pub fn paint_scene(
    painter: &Painter,
    viewport: egui::Rect,
    camera: Camera,
    scene: &Scene,
    style: &StyleSheet,
    dark: bool,
    accent: Color32,
    emphasis: &Emphasis,
    scratch: &mut Vec<ItemId>,
) -> PaintStats {
    scratch.clear();
    let zoom = camera.zoom;
    if !(zoom.is_finite() && zoom > 0.0) {
        return PaintStats {
            items: 0,
            texts: 0,
            lod: Lod::Flat,
        };
    }
    let world = Rect {
        x: -camera.x / zoom,
        y: -camera.y / zoom,
        w: viewport.width() / zoom,
        h: viewport.height() / zoom,
    }
    .inflate(CULL_MARGIN);
    scene.query(world, scratch);
    let level = lod(zoom, style.metrics.font_size, scratch.len());

    let sorted = |ids: &mut Vec<u32>| ids.sort_unstable();
    let mut strong_nodes: Vec<u32> = emphasis.strong_nodes.iter().map(|n| n.0).collect();
    let mut strong_edges: Vec<u32> = emphasis.strong_edges.iter().map(|e| e.0).collect();
    let mut matches: Vec<u32> = emphasis.matches.iter().map(|n| n.0).collect();
    sorted(&mut strong_nodes);
    sorted(&mut strong_edges);
    sorted(&mut matches);

    let mut frame = Frame {
        painter,
        view: View {
            origin: viewport.min,
            camera,
        },
        style,
        scene,
        dark,
        accent,
        canvas: color(style.colors(dark).canvas),
        selected: emphasis.selected,
        strong_nodes,
        strong_edges,
        matches,
        texts: 0,
        dim: emphasis.dim.clamp(0.0, 1.0),
        keep: 1.0 - (1.0 - DIM) * emphasis.dim.clamp(0.0, 1.0),
        curved_far_edges: scratch.len() <= 2500,
    };

    // Paint order: clusters, edges, edge labels, nodes.
    for id in scratch.iter() {
        if let ItemId::Cluster(c) = id
            && let Some(item) = scene.clusters.get(c.0 as usize)
        {
            frame.cluster(item, level);
        }
    }
    for id in scratch.iter() {
        if let ItemId::Edge(e) = id
            && let Some(item) = scene.edges.get(e.0 as usize)
        {
            frame.edge(item, level);
        }
    }
    if level == Lod::Full {
        for id in scratch.iter() {
            if let ItemId::Edge(e) = id
                && let Some(item) = scene.edges.get(e.0 as usize)
            {
                frame.edge_label(item);
            }
        }
    }
    for id in scratch.iter() {
        if let ItemId::Node(n) = id
            && let Some(item) = scene.nodes.get(n.0 as usize)
        {
            frame.node(item, level);
        }
    }
    PaintStats {
        items: scratch.len(),
        texts: frame.texts,
        lod: level,
    }
}
