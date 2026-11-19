//! Hand-drawn vector icon set.
//!
//! Every icon is described on a 16-unit grid (all geometry inside 1..15) as
//! stroke-only shapes and painted with round caps and joins, so the whole set
//! shares one visual weight. No text glyphs or emoji are used anywhere.
#![allow(dead_code)]

use std::f32::consts::{PI, TAU};

use eframe::egui;

use crate::{motion, ui::controls};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Sidebar,
    Back,
    Forward,
    Search,
    Info,
    Sun,
    Moon,
    Settings,
    Plus,
    Minus,
    Fit,
    Code,
    Export,
    Folder,
    FolderOpen,
    File,
    /// A document with a plus: a new diagram.
    FilePlus,
    Diagram,
    ChevronRight,
    ChevronDown,
    Pin,
    Close,
    Check,
    Warning,
    Save,
    Keyboard,
    ActualSize,
    /// Two stacked layers: projects.
    Stack,
    /// Up and down chevrons: a switcher.
    ChevronsUpDown,
    /// A clock face: recent.
    Clock,
}

pub const ALL: [Icon; 30] = [
    Icon::Sidebar,
    Icon::Back,
    Icon::Forward,
    Icon::Search,
    Icon::Info,
    Icon::Sun,
    Icon::Moon,
    Icon::Settings,
    Icon::Plus,
    Icon::Minus,
    Icon::Fit,
    Icon::Code,
    Icon::Export,
    Icon::Folder,
    Icon::FolderOpen,
    Icon::File,
    Icon::FilePlus,
    Icon::Diagram,
    Icon::ChevronRight,
    Icon::ChevronDown,
    Icon::Pin,
    Icon::Close,
    Icon::Check,
    Icon::Warning,
    Icon::Save,
    Icon::Keyboard,
    Icon::ActualSize,
    Icon::Stack,
    Icon::ChevronsUpDown,
    Icon::Clock,
];

/// One stroked shape in the 16-unit grid (y grows downwards).
#[derive(Clone, Debug)]
pub enum Stroke16 {
    /// Open polyline. A closed outline repeats its first point at the end.
    Line(Vec<[f32; 2]>),
    /// Centre, radius.
    Circle([f32; 2], f32),
    /// Centre, radius, start angle, end angle in radians. The arc runs from
    /// start to end with increasing angle, which is clockwise on screen.
    Arc([f32; 2], f32, f32, f32),
    /// Corners `[x0, y0, x1, y1]`, corner radius.
    Rect([f32; 4], f32),
}

const GRID: f32 = 16.0;
/// Stroke width in grid units. Light on purpose: the icons sit beside 13-14 px text and
/// must read as quieter than it.
const BASE_WIDTH: f32 = 1.25;
const CIRCLE_SEGMENTS: f32 = 32.0;

// ---------------------------------------------------------------------------
// Geometry helpers
// ---------------------------------------------------------------------------

/// Polyline through `points` with every interior corner (and, when `closed`,
/// every corner) replaced by a circular fillet of radius `radius`.
fn rounded(points: &[[f32; 2]], radius: f32, closed: bool) -> Stroke16 {
    let n = points.len();
    let mut out: Vec<[f32; 2]> = Vec::new();
    let corner_range = if closed { 0..n } else { 1..n - 1 };
    if !closed {
        out.push(points[0]);
    }
    for i in corner_range {
        let prev = points[(i + n - 1) % n];
        let corner = points[i];
        let next = points[(i + 1) % n];
        fillet(prev, corner, next, radius, &mut out);
    }
    if !closed {
        out.push(points[n - 1]);
    } else {
        out.push(out[0]);
    }
    Stroke16::Line(out)
}

fn fillet(a: [f32; 2], p: [f32; 2], b: [f32; 2], radius: f32, out: &mut Vec<[f32; 2]>) {
    let to_a = [a[0] - p[0], a[1] - p[1]];
    let to_b = [b[0] - p[0], b[1] - p[1]];
    let len_a = to_a[0].hypot(to_a[1]);
    let len_b = to_b[0].hypot(to_b[1]);
    let u = [to_a[0] / len_a, to_a[1] / len_a];
    let v = [to_b[0] / len_b, to_b[1] / len_b];
    let cos = (u[0] * v[0] + u[1] * v[1]).clamp(-1.0, 1.0);
    let theta = cos.acos();
    if theta < 0.05 || theta > PI - 0.05 {
        out.push(p);
        return;
    }
    let half_tan = (theta / 2.0).tan();
    let tangent = (radius / half_tan).min(len_a * 0.5).min(len_b * 0.5);
    let r = tangent * half_tan;
    let t1 = [p[0] + u[0] * tangent, p[1] + u[1] * tangent];
    let t2 = [p[0] + v[0] * tangent, p[1] + v[1] * tangent];
    let bis = [u[0] + v[0], u[1] + v[1]];
    let bis_len = bis[0].hypot(bis[1]);
    let dist = r / (theta / 2.0).sin();
    let c = [
        p[0] + bis[0] / bis_len * dist,
        p[1] + bis[1] / bis_len * dist,
    ];
    let a1 = (t1[1] - c[1]).atan2(t1[0] - c[0]);
    let a2 = (t2[1] - c[1]).atan2(t2[0] - c[0]);
    let mut delta = a2 - a1;
    while delta > PI {
        delta -= TAU;
    }
    while delta < -PI {
        delta += TAU;
    }
    let steps = ((delta.abs() / (PI / 2.0)) * 8.0).ceil().max(2.0) as usize;
    for s in 0..=steps {
        let angle = a1 + delta * s as f32 / steps as f32;
        out.push([c[0] + r * angle.cos(), c[1] + r * angle.sin()]);
    }
}

fn line(points: &[[f32; 2]]) -> Stroke16 {
    Stroke16::Line(points.to_vec())
}

/// A dot: a zero-length line, rendered as a single round cap.
fn dot(x: f32, y: f32) -> Stroke16 {
    Stroke16::Line(vec![[x, y], [x, y]])
}

fn on_circle(centre: [f32; 2], radius: f32, angle: f32) -> [f32; 2] {
    [
        centre[0] + radius * angle.cos(),
        centre[1] + radius * angle.sin(),
    ]
}

/// Crescent made from two arcs: the part of the outer circle that lies
/// outside the cutting circle, and the part of the cutting circle that lies
/// inside the outer one.
fn crescent() -> Vec<Stroke16> {
    let c1: [f32; 2] = [7.6, 8.4];
    let r1: f32 = 6.4;
    let c2: [f32; 2] = [11.2, 5.0];
    let r2: f32 = 5.0;
    let dx = c2[0] - c1[0];
    let dy = c2[1] - c1[1];
    let d = dx.hypot(dy);
    let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h = (r1 * r1 - a * a).sqrt();
    let toward = dy.atan2(dx);
    let phi = h.atan2(a);
    let psi = h.atan2(d - a);
    vec![
        Stroke16::Arc(c1, r1, toward + phi, toward + TAU - phi),
        Stroke16::Arc(c2, r2, toward + PI - psi, toward + PI + psi),
    ]
}

// ---------------------------------------------------------------------------
// Icon geometry
// ---------------------------------------------------------------------------

pub fn shapes(icon: Icon) -> Vec<Stroke16> {
    use Stroke16::{Circle, Rect};
    match icon {
        Icon::Sidebar => vec![Rect([2.0, 3.0, 14.0, 13.0], 2.5), line(&[[6.0, 3.5], [6.0, 12.5]])],
        Icon::Back => vec![
            line(&[[13.0, 8.0], [3.0, 8.0]]),
            line(&[[7.5, 3.5], [3.0, 8.0], [7.5, 12.5]]),
        ],
        Icon::Forward => vec![
            line(&[[3.0, 8.0], [13.0, 8.0]]),
            line(&[[8.5, 3.5], [13.0, 8.0], [8.5, 12.5]]),
        ],
        Icon::Search => vec![Circle([7.0, 7.0], 4.8), line(&[[10.5, 10.5], [14.0, 14.0]])],
        Icon::Info => vec![Circle([8.0, 8.0], 6.5), line(&[[8.0, 7.4], [8.0, 11.0]]), dot(8.0, 5.1)],
        Icon::Sun => {
            let mut shapes = vec![Circle([8.0, 8.0], 2.8)];
            for ray in 0..8 {
                let angle = ray as f32 * PI / 4.0;
                shapes.push(Stroke16::Line(vec![
                    on_circle([8.0, 8.0], 5.2, angle),
                    on_circle([8.0, 8.0], 6.8, angle),
                ]));
            }
            shapes
        }
        Icon::Moon => crescent(),
        // Two sliders with open knobs: calmer than three at 16 px.
        Icon::Settings => vec![
            line(&[[2.5, 5.0], [7.6, 5.0]]),
            line(&[[12.4, 5.0], [13.5, 5.0]]),
            Circle([10.0, 5.0], 2.1),
            line(&[[2.5, 11.0], [3.6, 11.0]]),
            line(&[[8.4, 11.0], [13.5, 11.0]]),
            Circle([6.0, 11.0], 2.1),
        ],
        Icon::Plus => vec![line(&[[8.0, 3.0], [8.0, 13.0]]), line(&[[3.0, 8.0], [13.0, 8.0]])],
        Icon::Minus => vec![line(&[[3.0, 8.0], [13.0, 8.0]])],
        // Four soft corners: a frame to fit the diagram into.
        Icon::Fit => vec![
            rounded(&[[2.5, 6.0], [2.5, 2.5], [6.0, 2.5]], 1.75, false),
            rounded(&[[10.0, 2.5], [13.5, 2.5], [13.5, 6.0]], 1.75, false),
            rounded(&[[13.5, 10.0], [13.5, 13.5], [10.0, 13.5]], 1.75, false),
            rounded(&[[6.0, 13.5], [2.5, 13.5], [2.5, 10.0]], 1.75, false),
            Rect([6.2, 6.2, 9.8, 9.8], 1.2),
        ],
        // Angle brackets with a slash that leans like text: source.
        Icon::Code => vec![
            rounded(&[[5.4, 4.6], [2.2, 8.0], [5.4, 11.4]], 0.9, false),
            rounded(&[[10.6, 4.6], [13.8, 8.0], [10.6, 11.4]], 0.9, false),
            line(&[[9.0, 3.4], [7.0, 12.6]]),
        ],
        // A tray with an arrow dropping into it: download.
        Icon::Export => vec![
            rounded(&[[2.5, 9.8], [2.5, 12.0], [13.5, 12.0], [13.5, 9.8]], 1.6, false),
            line(&[[8.0, 2.6], [8.0, 9.6]]),
            rounded(&[[5.0, 6.8], [8.0, 9.8], [11.0, 6.8]], 0.8, false),
        ],
        Icon::Folder => vec![rounded(
            &[[2.0, 12.5], [2.0, 4.0], [6.2, 4.0], [8.0, 6.0], [14.0, 6.0], [14.0, 12.5]],
            1.5,
            true,
        )],
        Icon::FolderOpen => vec![
            rounded(
                &[[2.0, 12.5], [2.0, 4.0], [6.2, 4.0], [8.0, 6.0], [12.5, 6.0], [12.5, 7.5]],
                1.5,
                false,
            ),
            rounded(&[[4.2, 7.5], [14.5, 7.5], [12.5, 12.5], [2.0, 12.5]], 1.2, true),
        ],
        Icon::File => vec![
            rounded(
                &[[3.5, 2.0], [9.5, 2.0], [12.5, 5.0], [12.5, 14.0], [3.5, 14.0]],
                1.3,
                true,
            ),
            rounded(&[[9.5, 2.0], [9.5, 5.0], [12.5, 5.0]], 0.8, false),
            line(&[[6.0, 8.5], [10.0, 8.5]]),
            line(&[[6.0, 11.0], [10.0, 11.0]]),
        ],
        Icon::FilePlus => vec![
            rounded(
                &[[3.5, 2.0], [9.5, 2.0], [12.5, 5.0], [12.5, 14.0], [3.5, 14.0]],
                1.3,
                true,
            ),
            rounded(&[[9.5, 2.0], [9.5, 5.0], [12.5, 5.0]], 0.8, false),
            line(&[[8.0, 8.0], [8.0, 12.0]]),
            line(&[[6.0, 10.0], [10.0, 10.0]]),
        ],
        Icon::Diagram => vec![
            Rect([2.0, 2.5, 7.5, 6.5], 1.3),
            Rect([8.5, 9.5, 14.0, 13.5], 1.3),
            rounded(&[[4.75, 6.5], [4.75, 11.5], [8.5, 11.5]], 1.5, false),
        ],
        Icon::ChevronRight => vec![line(&[[6.0, 3.5], [10.5, 8.0], [6.0, 12.5]])],
        Icon::ChevronDown => vec![line(&[[3.5, 6.0], [8.0, 10.5], [12.5, 6.0]])],
        Icon::Pin => vec![
            rounded(
                &[[6.0, 2.5], [10.0, 2.5], [10.0, 6.5], [12.0, 9.0], [4.0, 9.0], [6.0, 6.5]],
                0.9,
                true,
            ),
            line(&[[8.0, 9.0], [8.0, 14.0]]),
        ],
        Icon::Close => vec![
            line(&[[3.5, 3.5], [12.5, 12.5]]),
            line(&[[12.5, 3.5], [3.5, 12.5]]),
        ],
        Icon::Check => vec![line(&[[2.5, 8.5], [6.2, 12.0], [13.5, 4.5]])],
        Icon::Warning => vec![
            rounded(&[[8.0, 2.2], [14.5, 13.5], [1.5, 13.5]], 1.8, true),
            line(&[[8.0, 6.5], [8.0, 9.5]]),
            dot(8.0, 11.5),
        ],
        Icon::Save => vec![
            rounded(
                &[[2.5, 2.5], [10.5, 2.5], [13.5, 5.5], [13.5, 13.5], [2.5, 13.5]],
                1.8,
                true,
            ),
            line(&[[5.5, 2.5], [5.5, 6.0], [10.0, 6.0]]),
            line(&[[4.5, 13.5], [4.5, 9.5], [11.5, 9.5], [11.5, 13.5]]),
        ],
        Icon::Keyboard => vec![
            Rect([1.5, 3.5, 14.5, 12.5], 2.5),
            dot(4.8, 6.6),
            dot(8.0, 6.6),
            dot(11.2, 6.6),
            line(&[[5.0, 9.6], [11.0, 9.6]]),
        ],
        Icon::ActualSize => vec![
            Rect([2.5, 2.5, 13.5, 13.5], 2.5),
            line(&[[6.4, 6.6], [8.2, 5.6], [8.2, 10.6]]),
        ],
        Icon::Stack => vec![
            rounded(&[[8.0, 2.5], [14.0, 5.6], [8.0, 8.7], [2.0, 5.6]], 1.1, true),
            rounded(&[[2.0, 9.0], [8.0, 12.1], [14.0, 9.0]], 1.1, false),
        ],
        Icon::ChevronsUpDown => vec![
            line(&[[5.0, 6.0], [8.0, 3.0], [11.0, 6.0]]),
            line(&[[5.0, 10.0], [8.0, 13.0], [11.0, 10.0]]),
        ],
        Icon::Clock => vec![Circle([8.0, 8.0], 6.0), line(&[[8.0, 4.8], [8.0, 8.0], [10.4, 9.6]])],
    }
}

/// Flattens a shape into a polyline in grid units. The flag is true when the
/// outline is closed (no end caps needed).
pub fn flatten(shape: &Stroke16) -> (Vec<[f32; 2]>, bool) {
    match shape {
        Stroke16::Line(points) => {
            let closed = points.len() > 2 && points.first() == points.last();
            (points.clone(), closed)
        }
        Stroke16::Circle(centre, radius) => {
            let steps = CIRCLE_SEGMENTS as usize;
            let points = (0..=steps)
                .map(|i| on_circle(*centre, *radius, TAU * i as f32 / steps as f32))
                .collect();
            (points, true)
        }
        Stroke16::Arc(centre, radius, start, end) => {
            let steps = (((end - start).abs() / TAU) * CIRCLE_SEGMENTS).ceil().max(4.0) as usize;
            let points = (0..=steps)
                .map(|i| on_circle(*centre, *radius, start + (end - start) * i as f32 / steps as f32))
                .collect();
            (points, false)
        }
        Stroke16::Rect([x0, y0, x1, y1], radius) => {
            let corners = [[*x0, *y0], [*x1, *y0], [*x1, *y1], [*x0, *y1]];
            match rounded(&corners, *radius, true) {
                Stroke16::Line(points) => (points, true),
                _ => unreachable!(),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------

/// Paints `icon` scaled into `rect` (assumed square).
pub fn paint(painter: &egui::Painter, icon: Icon, rect: egui::Rect, color: egui::Color32) {
    paint_rotated(painter, icon, rect, color, 0.0);
}

/// Paints `icon` turned by `angle` radians (clockwise on screen) about the centre of `rect`.
pub fn paint_rotated(painter: &egui::Painter, icon: Icon, rect: egui::Rect, color: egui::Color32, angle: f32) {
    let scale = rect.width() / GRID;
    let width = (BASE_WIDTH * scale).clamp(1.1, 2.2);
    let centre = rect.center();
    let (sin, cos) = angle.sin_cos();
    let map = |p: [f32; 2]| {
        let (x, y) = ((p[0] - GRID * 0.5) * scale, (p[1] - GRID * 0.5) * scale);
        centre + egui::vec2(x * cos - y * sin, x * sin + y * cos)
    };
    let stroke = egui::Stroke::new(width, color);
    let cap_radius = width * 0.5;

    for shape in shapes(icon) {
        let (points, closed) = flatten(&shape);
        let screen: Vec<egui::Pos2> = points.iter().copied().map(map).collect();
        let is_dot = points.len() == 2 && points[0] == points[1];
        if is_dot {
            painter.add(egui::Shape::circle_filled(screen[0], cap_radius * 1.15, color));
            continue;
        }
        if closed {
            let mut ring = screen;
            while ring.len() > 2 && ring[0].distance(ring[ring.len() - 1]) < 0.01 {
                ring.pop();
            }
            painter.add(egui::Shape::closed_line(ring, stroke));
        } else {
            // egui strokes have butt caps; round the two open ends only. Joins stay as
            // tessellated so corners do not swell into blobs.
            let (first, last) = (screen[0], screen[screen.len() - 1]);
            painter.add(egui::Shape::line(screen, stroke));
            painter.add(egui::Shape::circle_filled(first, cap_radius, color));
            painter.add(egui::Shape::circle_filled(last, cap_radius, color));
        }
    }
}

/// A 32x32 icon button with a 16 px glyph. Idle glyphs are quieter than text; hover eases
/// the fill and the glyph to full strength, a press scales the fill and glyph down.
pub fn button(
    ui: &mut egui::Ui,
    icon: Icon,
    tooltip: &str,
    selected: bool,
    enabled: bool,
) -> egui::Response {
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), sense);
    if ui.is_rect_visible(rect) {
        let p = crate::theme::palette(ui.visuals().dark_mode);
        let ctx = ui.ctx().clone();
        let hovered = enabled && response.hovered();
        let pressed = enabled && response.is_pointer_button_down_on();
        let hover = motion::hover(&ctx, response.id, hovered);
        let on = motion::spring(&ctx, response.id.with("selected"), f32::from(u8::from(selected)), motion::SOFT);
        let press = motion::press(&ctx, response.id, pressed);
        let body = controls::pressed_rect(rect, press);
        let painter = ui.painter();
        if on > 0.0 {
            painter.rect_filled(body, 8.0 * controls::press_scale(press), p.accent_soft.gamma_multiply(on));
        }
        if hover > 0.0 {
            painter.rect_filled(body, 8.0 * controls::press_scale(press), p.hover.gamma_multiply(hover * (1.0 + 0.8 * press.max(0.0))));
        }
        let glyph = if !enabled {
            p.muted.gamma_multiply(0.55)
        } else {
            p.secondary.lerp_to_gamma(p.ink, hover).lerp_to_gamma(p.accent, on)
        };
        let size = 16.0 * controls::press_scale(press);
        paint(painter, icon, egui::Rect::from_center_size(rect.center(), egui::vec2(size, size)), glyph);
    }
    let response = response.on_hover_text(tooltip);
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

// ---------------------------------------------------------------------------
// Preview
// ---------------------------------------------------------------------------

/// SVG contact sheet of every icon at 48 px and 16 px, generated from the
/// same flattened geometry that `paint` strokes.
pub fn to_svg_preview() -> String {
    use std::fmt::Write;

    let columns = 6;
    let cell_w = 150.0;
    let cell_h = 100.0;
    let rows = ALL.len().div_ceil(columns);
    let width = cell_w * columns as f32;
    let height = cell_h * rows as f32;
    let mut svg = String::new();
    let _ = writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\">"
    );
    let _ = writeln!(svg, "<rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>");
    for (index, icon) in ALL.iter().enumerate() {
        let x = (index % columns) as f32 * cell_w;
        let y = (index / columns) as f32 * cell_h;
        for (size, offset) in [(48.0_f32, 14.0_f32), (16.0, 86.0)] {
            let scale = size / GRID;
            let stroke = (BASE_WIDTH * scale).clamp(1.1, 2.2);
            let top = y + if size > 20.0 { 10.0 } else { 26.0 };
            let _ = writeln!(
                svg,
                "<g transform=\"translate({} {}) scale({scale})\" fill=\"none\" stroke=\"#1d2630\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\">",
                x + offset,
                top,
                stroke / scale
            );
            for shape in shapes(*icon) {
                let (points, _) = flatten(&shape);
                let mut d = String::new();
                for (i, p) in points.iter().enumerate() {
                    let _ = write!(d, "{}{:.3} {:.3} ", if i == 0 { "M" } else { "L" }, p[0], p[1]);
                }
                if points.len() == 2 && points[0] == points[1] {
                    let _ = write!(d, "l 0.001 0");
                }
                let _ = writeln!(svg, "<path d=\"{}\"/>", d.trim_end());
            }
            let _ = writeln!(svg, "</g>");
        }
        let _ = writeln!(
            svg,
            "<text x=\"{}\" y=\"{}\" font-family=\"Segoe UI, Arial, sans-serif\" font-size=\"12\" text-anchor=\"middle\" fill=\"#44505c\">{:?}</text>",
            x + cell_w / 2.0,
            y + 82.0,
            icon
        );
    }
    let _ = writeln!(svg, "</svg>");
    svg
}
