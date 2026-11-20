//! SVG export of a [`Scene`], using the same paint functions as the canvas.

use std::fmt::Write as _;

use crate::ends;
use crate::decor::{Anchor, Decor, Tone, tone_colors};
use crate::graph::{ArrowKind, LineKind, ShapeKind};
use crate::scene::{ClusterItem, EdgeItem, NodeItem, Scene};
use crate::shape::{self, PathCmd, Rect};
use crate::style::{Paint, Rgba, StyleSheet, cluster_paint, edge_paint, node_paint};

type Pt = [f32; 2];

/// Margin around the diagram inside the viewBox.
const MARGIN: f32 = 24.0;
/// Baseline offset from the visual centre of a line, as a fraction of the font size.
const BASELINE: f32 = 0.36;
const CLUSTER_RADIUS: f32 = 16.0;
const LABEL_RADIUS: f32 = 8.0;

/// Formats a number with at most two decimals and no trailing zeros.
fn num(v: f32) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        "0".into()
    } else {
        s.to_owned()
    }
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Control characters are not allowed in XML 1.0.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

fn hex(c: Rgba) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

/// `fill="#RRGGBB"` plus `fill-opacity` when the colour is translucent.
fn paint_attr(name: &str, c: Rgba) -> String {
    if c[3] == 0 {
        format!("{name}=\"none\"")
    } else if c[3] == 255 {
        format!("{name}=\"{}\"", hex(c))
    } else {
        format!(
            "{name}=\"{}\" {name}-opacity=\"{}\"",
            hex(c),
            num(c[3] as f32 / 255.0)
        )
    }
}

/// Appends path commands as an SVG `d` string.
fn push_cmds(d: &mut String, cmds: &[PathCmd]) {
    for c in cmds {
        match c {
            PathCmd::Move(p) => {
                let _ = write!(d, "M{} {}", num(p[0]), num(p[1]));
            }
            PathCmd::Line(p) => {
                let _ = write!(d, "L{} {}", num(p[0]), num(p[1]));
            }
            PathCmd::Cubic(a, b, p) => {
                let _ = write!(
                    d,
                    "C{} {} {} {} {} {}",
                    num(a[0]),
                    num(a[1]),
                    num(b[0]),
                    num(b[1]),
                    num(p[0]),
                    num(p[1])
                );
            }
            PathCmd::Close => d.push('Z'),
        }
    }
}

fn outline_d(kind: ShapeKind, r: Rect, radius: f32) -> String {
    let mut d = String::new();
    push_cmds(&mut d, &shape::outline(kind, r, radius));
    d
}

fn dash_attr(paint: &Paint, dash: f32, gap: f32) -> String {
    if paint.dashed {
        format!(" stroke-dasharray=\"{} {}\"", num(dash), num(gap))
    } else {
        String::new()
    }
}

/// One `<text>` with a `<tspan>` per line, centred on `(cx, cy)`.
#[allow(clippy::too_many_arguments)]
fn text_block(
    out: &mut String,
    lines: &[String],
    cx: f32,
    cy: f32,
    line_height: f32,
    size: f32,
    weight: u32,
    color: Rgba,
    family: &str,
) {
    if lines.is_empty() {
        return;
    }
    let first = cy - lines.len() as f32 * line_height / 2.0 + line_height / 2.0 + size * BASELINE;
    let _ = write!(
        out,
        "<text text-anchor=\"middle\" font-family=\"{}\" font-size=\"{}\" font-weight=\"{}\" {}>",
        esc(family),
        num(size),
        weight,
        paint_attr("fill", color)
    );
    for (i, line) in lines.iter().enumerate() {
        let _ = write!(
            out,
            "<tspan x=\"{}\" y=\"{}\">{}</tspan>",
            num(cx),
            num(first + i as f32 * line_height),
            esc(line)
        );
    }
    out.push_str("</text>\n");
}

pub use crate::ends::{ARROW_LENGTH, ARROW_WIDTH};

/// How far back from its tip the decoration of `kind` reaches along the path.
pub fn end_reach(kind: ArrowKind, size: f32) -> f32 {
    match kind {
        ArrowKind::None => 0.0,
        ArrowKind::Arrow => ends::arrow_reach(size),
        ArrowKind::Circle => 2.0 * size * 0.45 * ARROW_LENGTH,
        ArrowKind::Cross => 2.0 * size * 0.7 * ARROW_LENGTH * std::f32::consts::FRAC_1_SQRT_2,
        other => ends::marker_reach(other, size),
    }
}

/// Decoration at a line end. `dir` points at the tip and follows the visible curve.
fn marker_svg(prims: &[ends::Prim], stroke_w: f32, color: Rgba, bg: Rgba) -> String {
    let stroke = paint_attr("stroke", color);
    let w = num(stroke_w.max(1.0));
    let mut out = String::new();
    for prim in prims {
        match prim {
            ends::Prim::Seg(a, b) => {
                let _ = writeln!(
                    out,
                    "<path d=\"M{} {}L{} {}\" fill=\"none\" {stroke} stroke-width=\"{w}\" stroke-linecap=\"round\"/>",
                    num(a[0]),
                    num(a[1]),
                    num(b[0]),
                    num(b[1])
                );
            }
            ends::Prim::Poly(points, filled) => {
                let mut d = String::new();
                for (i, p) in points.iter().enumerate() {
                    let _ = write!(d, "{}{} {}", if i == 0 { "M" } else { "L" }, num(p[0]), num(p[1]));
                }
                let fill = paint_attr("fill", if *filled { color } else { bg });
                let _ = writeln!(
                    out,
                    "<path d=\"{d}Z\" {fill} {stroke} stroke-width=\"{w}\" stroke-linejoin=\"round\"/>"
                );
            }
            ends::Prim::Circle(c, r, filled) => {
                let fill = paint_attr("fill", if *filled { color } else { bg });
                let _ = writeln!(
                    out,
                    "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" {fill} {stroke} stroke-width=\"{w}\"/>",
                    num(c[0]),
                    num(c[1]),
                    num(*r)
                );
            }
        }
    }
    out
}

/// Decoration at a line end. `dir` points at the tip and follows the visible curve.
fn arrow_end(
    kind: ArrowKind,
    tip: Pt,
    dir: Pt,
    size: f32,
    stroke_w: f32,
    color: Rgba,
    bg: Rgba,
) -> String {
    if ends::is_marker(kind) {
        return marker_svg(&ends::marker(kind, tip, dir, size), stroke_w, color, bg);
    }
    arrow_end_classic(kind, tip, dir, size, stroke_w, color)
}

fn arrow_end_classic(kind: ArrowKind, tip: Pt, dir: Pt, size: f32, stroke_w: f32, color: Rgba) -> String {
    let n = [-dir[1], dir[0]];
    let at = |along: f32, across: f32| -> Pt {
        [
            tip[0] - dir[0] * along + n[0] * across,
            tip[1] - dir[1] * along + n[1] * across,
        ]
    };
    let fill = paint_attr("fill", color);
    let stroke = paint_attr("stroke", color);
    match kind {
        ArrowKind::None => String::new(),
        ArrowKind::Arrow => {
            // Rounded triangle: soft tip on the outline, soft base corners, no outline stroke.
            let ([c0, c1, c2], _) = ends::arrow_head(tip, dir, size);
            let p = |q: Pt| format!("{} {}", num(q[0]), num(q[1]));
            format!(
                "<path d=\"M{}L{}Q{} {}L{}Q{} {}L{}Q{} {}Z\" {fill}/>\n",
                p(c0.to),
                p(c1.from),
                p(c1.corner),
                p(c1.to),
                p(c2.from),
                p(c2.corner),
                p(c2.to),
                p(c0.from),
                p(c0.corner),
                p(c0.to),
            )
        }
        ArrowKind::Circle => {
            let r = size * 0.45 * ARROW_LENGTH;
            let c = at(r, 0.0);
            format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" {fill}/>\n",
                num(c[0]),
                num(c[1]),
                num(r)
            )
        }
        ArrowKind::Cross => {
            let h = size * 0.7 * ARROW_LENGTH;
            let k = std::f32::consts::FRAC_1_SQRT_2;
            let c = at(h * k, 0.0);
            let (u, v) = (
                [(dir[0] + n[0]) * h * k, (dir[1] + n[1]) * h * k],
                [(dir[0] - n[0]) * h * k, (dir[1] - n[1]) * h * k],
            );
            format!(
                "<path d=\"M{} {}L{} {}M{} {}L{} {}\" fill=\"none\" {stroke} stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>\n",
                num(c[0] - u[0]),
                num(c[1] - u[1]),
                num(c[0] + u[0]),
                num(c[1] + u[1]),
                num(c[0] - v[0]),
                num(c[1] - v[1]),
                num(c[0] + v[0]),
                num(c[1] + v[1]),
                num((stroke_w * 1.1).max(1.2)),
            )
        }
        _ => String::new(),
    }
}

/// The edge's own line as a `d` string. `pts` is already shortened under its decorations.
fn edge_d(pts: &[Pt]) -> String {
    let mut d = String::new();
    let Some(first) = pts.first() else {
        return d;
    };
    let _ = write!(d, "M{} {}", num(first[0]), num(first[1]));
    if pts.len() >= 4 && (pts.len() - 1) % 3 == 0 {
        for c in pts[1..].chunks_exact(3) {
            let _ = write!(
                d,
                "C{} {} {} {} {} {}",
                num(c[0][0]),
                num(c[0][1]),
                num(c[1][0]),
                num(c[1][1]),
                num(c[2][0]),
                num(c[2][1])
            );
        }
    } else {
        for p in &pts[1..] {
            let _ = write!(d, "L{} {}", num(p[0]), num(p[1]));
        }
    }
    d
}

fn write_cluster(
    out: &mut String,
    scene: &Scene,
    c: &ClusterItem,
    style: &StyleSheet,
    dark: bool,
    family: &str,
) {
    let paint = cluster_paint(c, scene, style, dark);
    let r = c.rect;
    let _ = writeln!(
        out,
        "<g class=\"cluster\" data-id=\"{}\">",
        esc(scene.cluster_key(c.id))
    );
    let _ = writeln!(
        out,
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" {} {} stroke-width=\"{}\" stroke-linejoin=\"round\" stroke-linecap=\"round\"{}/>",
        num(r.x),
        num(r.y),
        num(r.w),
        num(r.h),
        num(CLUSTER_RADIUS.min(r.w / 2.0).min(r.h / 2.0)),
        paint_attr("fill", paint.fill),
        paint_attr("stroke", paint.stroke),
        num(paint.stroke_width),
        dash_attr(&paint, 6.0, 4.0),
    );
    if !c.title.is_empty() {
        let size = 13.0;
        let zone = style.metrics.cluster_title.min(r.h);
        let y = r.y + zone / 2.0 + size * BASELINE;
        let _ = writeln!(
            out,
            "<text x=\"{}\" y=\"{}\" font-family=\"{}\" font-size=\"{}\" font-weight=\"600\" {}>{}</text>",
            num(r.x + 12.0),
            num(y),
            esc(family),
            num(size),
            paint_attr("fill", paint.text),
            esc(&c.title)
        );
    }
    out.push_str("</g>\n");
}

fn write_decor(out: &mut String, d: &Decor, style: &StyleSheet, dark: bool, family: &str) {
    let colors = style.colors(dark);
    let col = |t: Tone| tone_colors(t, colors);
    match d {
        Decor::Poly { pts, fill, stroke, alpha } => {
            let (f, _) = col(*fill);
            let mut path = String::new();
            for (i, p) in pts.iter().enumerate() {
                let _ = write!(path, "{}{} {}", if i == 0 { "M" } else { "L" }, num(p[0]), num(p[1]));
            }
            let st = match stroke {
                Some(t) => format!(" {} stroke-width=\"1\" stroke-linejoin=\"round\"", paint_attr("stroke", col(*t).1)),
                None => String::new(),
            };
            let mut fc = f;
            fc[3] = ((u16::from(fc[3]) * u16::from(*alpha)) / 255) as u8;
            let _ = writeln!(out, "<path d=\"{path}Z\" {}{st}/>", paint_attr("fill", fc));
        }
        Decor::Line { pts, width, dashed, tone } => {
            let (_, c) = col(*tone);
            let mut path = String::new();
            for (i, p) in pts.iter().enumerate() {
                let _ = write!(path, "{}{} {}", if i == 0 { "M" } else { "L" }, num(p[0]), num(p[1]));
            }
            let dash = if *dashed { " stroke-dasharray=\"3 4\"" } else { "" };
            let _ = writeln!(
                out,
                "<path d=\"{path}\" fill=\"none\" {} stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{dash}/>",
                paint_attr("stroke", c),
                num(*width)
            );
        }
        Decor::Rect { rect, radius, fill, stroke } => {
            let (f, _) = col(*fill);
            let st = match stroke {
                Some(t) => format!("{} stroke-width=\"1\"", paint_attr("stroke", col(*t).1)),
                None => String::new(),
            };
            let _ = writeln!(
                out,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" {} {st}/>",
                num(rect.x),
                num(rect.y),
                num(rect.w),
                num(rect.h),
                num(*radius),
                paint_attr("fill", f)
            );
        }
        Decor::Wedge { c, r, a0, a1, tone } => {
            let (f, _) = col(*tone);
            let p0 = [c[0] + r * a0.cos(), c[1] + r * a0.sin()];
            let p1 = [c[0] + r * a1.cos(), c[1] + r * a1.sin()];
            let large = i32::from((a1 - a0).abs() > std::f32::consts::PI);
            let _ = writeln!(
                out,
                "<path d=\"M{} {}L{} {}A{} {} 0 {large} 1 {} {}Z\" {} {} stroke-width=\"1.5\" stroke-linejoin=\"round\"/>",
                num(c[0]),
                num(c[1]),
                num(p0[0]),
                num(p0[1]),
                num(*r),
                num(*r),
                num(p1[0]),
                num(p1[1]),
                paint_attr("fill", f),
                paint_attr("stroke", colors.canvas)
            );
        }
        Decor::Circle { c, r, fill, stroke } => {
            let (f, _) = col(*fill);
            let st = match stroke {
                Some(t) => format!("{} stroke-width=\"1\"", paint_attr("stroke", col(*t).1)),
                None => String::new(),
            };
            let _ = writeln!(
                out,
                "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" {} {st}/>",
                num(c[0]),
                num(c[1]),
                num(*r),
                paint_attr("fill", f)
            );
        }
        Decor::Text { at, text, size, anchor, bold, tone } => {
            let (c, _) = col(*tone);
            let anchor = match anchor {
                Anchor::Start => "start",
                Anchor::Middle => "middle",
                Anchor::End => "end",
            };
            let _ = writeln!(
                out,
                "<text x=\"{}\" y=\"{}\" text-anchor=\"{anchor}\" font-family=\"{}\" font-size=\"{}\" font-weight=\"{}\" {}>{}</text>",
                num(at[0]),
                num(at[1] + size * BASELINE),
                esc(family),
                num(*size),
                if *bold { 600 } else { 400 },
                paint_attr("fill", c),
                esc(text)
            );
        }
    }
}

fn write_edge(out: &mut String, scene: &Scene, e: &EdgeItem, style: &StyleSheet, dark: bool) {
    if e.line == LineKind::Invisible || e.points.len() < 2 {
        return;
    }
    let paint = edge_paint(e, scene, style, dark);
    let size = style.arrow_size;
    let (mut head, mut tail) = (String::new(), String::new());
    let (reach_end, reach_start) = (end_reach(e.head, size), end_reach(e.tail, size));
    if let (Some(dir), Some(tip)) = (ends::end_dir(&e.points, reach_end), e.points.last()) {
        head = arrow_end(e.head, *tip, dir, size, paint.stroke_width, paint.stroke, paint.fill);
    }
    if let (Some(dir), Some(tip)) = (ends::start_dir(&e.points, reach_start), e.points.first()) {
        tail = arrow_end(e.tail, *tip, dir, size, paint.stroke_width, paint.stroke, paint.fill);
    }
    let line = ends::trimmed(&e.points, reach_start, reach_end);
    let dash = if paint.dashed {
        format!(
            " stroke-dasharray=\"{} {}\"",
            num(paint.stroke_width * 1.6),
            num(paint.stroke_width * 3.0)
        )
    } else {
        String::new()
    };
    let _ = writeln!(out, "<g class=\"edge\" data-id=\"e{}\">", e.id.0);
    let _ = writeln!(
        out,
        "<path d=\"{}\" fill=\"none\" {} stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{dash}/>",
        edge_d(&line),
        paint_attr("stroke", paint.stroke),
        num(paint.stroke_width),
    );
    out.push_str(&head);
    out.push_str(&tail);
    out.push_str("</g>\n");
}

fn write_edge_label(
    out: &mut String,
    scene: &Scene,
    e: &EdgeItem,
    style: &StyleSheet,
    dark: bool,
    family: &str,
) {
    if e.line == LineKind::Invisible {
        return;
    }
    let Some((r, text)) = &e.label else {
        return;
    };
    let paint = edge_paint(e, scene, style, dark);
    let size = style.metrics.edge_label_font;
    let mut border = paint.stroke;
    border[3] = 90; // 35% of the edge colour
    let _ = writeln!(out, "<g class=\"edge-label\" data-id=\"e{}\">", e.id.0);
    let _ = writeln!(
        out,
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" {} {} stroke-width=\"1\"/>",
        num(r.x),
        num(r.y),
        num(r.w),
        num(r.h),
        num(LABEL_RADIUS.min(r.h / 2.0)),
        paint_attr("fill", paint.fill),
        paint_attr("stroke", border),
    );
    let [cx, cy] = r.center();
    text_block(
        out,
        &text.lines,
        cx,
        cy,
        style.metrics.line_height.min(size * 1.4).max(size),
        size,
        500,
        paint.text,
        family,
    );
    out.push_str("</g>\n");
}

fn write_node(
    out: &mut String,
    scene: &Scene,
    n: &NodeItem,
    style: &StyleSheet,
    dark: bool,
    family: &str,
) {
    let paint = node_paint(n, scene, style, dark);
    let m = &style.metrics;
    let r = n.rect;
    let sw = num(paint.stroke_width);
    let stroke = paint_attr("stroke", paint.stroke);
    let _ = writeln!(
        out,
        "<g class=\"node\" data-id=\"{}\">",
        esc(scene.node_key(n.id))
    );
    let _ = writeln!(
        out,
        "<path d=\"{}\" {} {} stroke-width=\"{sw}\" stroke-linejoin=\"round\" stroke-linecap=\"round\"{}/>",
        outline_d(n.shape, r, m.radius),
        paint_attr("fill", paint.fill),
        stroke,
        dash_attr(&paint, 3.0, 4.0),
    );
    let line = |out: &mut String, d: &str| {
        let _ = writeln!(
            out,
            "<path d=\"{d}\" fill=\"none\" {stroke} stroke-width=\"{sw}\" stroke-linejoin=\"round\" stroke-linecap=\"round\"/>"
        );
    };
    let mut text_dy = 0.0;
    match n.shape {
        ShapeKind::Cylinder => {
            // Front half of the top rim; the body outline already curves top and bottom.
            let ry = 8.0f32.min(r.h / 4.0);
            let (x0, x1, cx) = (r.x, r.right(), r.center()[0]);
            let y = r.y + ry;
            let k = ry * 0.552_284_75;
            let kx = r.w / 2.0 * 0.552_284_75;
            line(
                out,
                &format!(
                    "M{} {}C{} {} {} {} {} {}C{} {} {} {} {} {}",
                    num(x0),
                    num(y),
                    num(x0),
                    num(y + k),
                    num(cx - kx),
                    num(y + ry),
                    num(cx),
                    num(y + ry),
                    num(cx + kx),
                    num(y + ry),
                    num(x1),
                    num(y + k),
                    num(x1),
                    num(y)
                ),
            );
            text_dy = ry / 2.0;
        }
        ShapeKind::Subroutine => {
            let radius = m.radius.min(r.h / 2.0).min(r.w / 2.0);
            let inset = 8.0;
            // Keep the bars inside the rounded corners.
            let dy = if radius > inset {
                radius - (radius * radius - (radius - inset).powi(2)).max(0.0).sqrt() + 0.5
            } else {
                0.0
            };
            for x in [r.x + inset, r.right() - inset] {
                line(
                    out,
                    &format!(
                        "M{} {}L{} {}",
                        num(x),
                        num(r.y + dy),
                        num(x),
                        num(r.bottom() - dy)
                    ),
                );
            }
        }
        ShapeKind::Record => {
            if let Some(rec) = &n.record {
                for &y in &rec.dividers {
                    line(
                        out,
                        &format!("M{} {}L{} {}", num(r.x), num(r.y + y), num(r.right()), num(r.y + y)),
                    );
                }
            }
        }
        ShapeKind::StateEnd => {
            let inner = Rect {
                x: r.x + 4.0,
                y: r.y + 4.0,
                w: (r.w - 8.0).max(0.0),
                h: (r.h - 8.0).max(0.0),
            };
            let _ = writeln!(
                out,
                "<path d=\"{}\" {} stroke=\"none\"/>",
                outline_d(ShapeKind::Circle, inner, m.radius),
                paint_attr("fill", paint.stroke),
            );
        }
        ShapeKind::Note => {
            let cut = 10.0f32.min(r.w / 2.0).min(r.h / 2.0);
            line(
                out,
                &format!(
                    "M{} {}L{} {}L{} {}",
                    num(r.right() - cut),
                    num(r.y),
                    num(r.right() - cut),
                    num(r.y + cut),
                    num(r.right()),
                    num(r.y + cut)
                ),
            );
        }
        ShapeKind::DoubleCircle => {
            let inner = Rect {
                x: r.x + 5.0,
                y: r.y + 5.0,
                w: (r.w - 10.0).max(0.0),
                h: (r.h - 10.0).max(0.0),
            };
            line(out, &outline_d(ShapeKind::Circle, inner, m.radius));
        }
        _ => {}
    }
    let [cx, cy] = r.center();
    if let Some(rec) = &n.record {
        for row in &rec.rows {
            for (x, cell) in &row.cells {
                if row.header {
                    text_block(
                        out,
                        std::slice::from_ref(cell),
                        cx,
                        r.y + row.y,
                        m.line_height,
                        m.font_size,
                        600,
                        paint.text,
                        family,
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "<text x=\"{}\" y=\"{}\" font-family=\"{}\" font-size=\"{}\" font-weight=\"400\" {}>{}</text>",
                        num(r.x + x),
                        num(r.y + row.y + rec.member_font * BASELINE),
                        esc(family),
                        num(rec.member_font),
                        paint_attr("fill", paint.text),
                        esc(cell)
                    );
                }
            }
        }
    } else {
        text_block(
            out,
            &n.text.lines,
            cx,
            cy + text_dy,
            m.line_height,
            m.font_size,
            500,
            paint.text,
            family,
        );
    }
    out.push_str("</g>\n");
}

/// Renders the scene as a standalone SVG document.
///
/// Colours come from the same `*_paint` functions as the canvas. `font_css`
/// (for example `@font-face` rules) is embedded verbatim in a `<style>` element.
pub fn to_svg(
    scene: &Scene,
    style: &StyleSheet,
    dark: bool,
    background: bool,
    font_family: &str,
    font_css: Option<&str>,
) -> String {
    let b = scene.bounds;
    let (w, h) = (b.w.max(0.0) + 2.0 * MARGIN, b.h.max(0.0) + 2.0 * MARGIN);
    let (vx, vy) = (b.x - MARGIN, b.y - MARGIN);
    let mut out = String::with_capacity(1024 + scene.nodes.len() * 700 + scene.edges.len() * 600);
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {} {}\" width=\"{}\" height=\"{}\">",
        num(vx),
        num(vy),
        num(w),
        num(h),
        num(w),
        num(h)
    );
    if let Some(css) = font_css {
        // `]]>` cannot appear inside CDATA; split it.
        let _ = writeln!(
            out,
            "<style><![CDATA[{}]]></style>",
            css.replace("]]>", "]]]]><![CDATA[>")
        );
    }
    if background {
        let _ = writeln!(
            out,
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" {}/>",
            num(vx),
            num(vy),
            num(w),
            num(h),
            paint_attr("fill", style.colors(dark).canvas)
        );
    }
    for d in &scene.decor {
        write_decor(&mut out, d, style, dark, font_family);
    }
    for c in &scene.clusters {
        write_cluster(&mut out, scene, c, style, dark, font_family);
    }
    for e in &scene.edges {
        write_edge(&mut out, scene, e, style, dark);
    }
    for e in &scene.edges {
        write_edge_label(&mut out, scene, e, style, dark, font_family);
    }
    for n in &scene.nodes {
        write_node(&mut out, scene, n, style, dark, font_family);
    }
    out.push_str("</svg>\n");
    out
}
