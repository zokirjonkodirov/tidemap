//! Free-form drawing primitives for chart-style diagrams (pie, gantt, timeline, journey,
//! quadrant). They are painted under clusters, edges and nodes and carry theme tones, not
//! colours, so light and dark themes both work.

use crate::shape::Rect;
use crate::style::{DiagramColors, Rgba, Swatch};

/// Theme colour reference.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tone {
    /// Text colour.
    Ink,
    /// Line colour.
    Muted,
    /// Faint grid line.
    Grid,
    /// Page background.
    Canvas,
    /// Pastel swatch `n` (wraps around): fill and matching stroke.
    Swatch(u8),
}

/// Horizontal text anchor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Anchor {
    Start,
    Middle,
    End,
}

/// One primitive, in world units.
#[derive(Clone, PartialEq, Debug)]
pub enum Decor {
    Line {
        pts: Vec<[f32; 2]>,
        width: f32,
        dashed: bool,
        tone: Tone,
    },
    Rect {
        rect: Rect,
        radius: f32,
        fill: Tone,
        stroke: Option<Tone>,
    },
    /// Pie slice from angle `a0` to `a1` (radians, clockwise from 3 o'clock, y points down).
    Wedge {
        c: [f32; 2],
        r: f32,
        a0: f32,
        a1: f32,
        tone: Tone,
    },
    Circle {
        c: [f32; 2],
        r: f32,
        fill: Tone,
        stroke: Option<Tone>,
    },
    /// Filled polygon; `alpha` scales the fill opacity (255 = opaque).
    Poly {
        pts: Vec<[f32; 2]>,
        fill: Tone,
        stroke: Option<Tone>,
        alpha: u8,
    },
    Text {
        at: [f32; 2],
        text: String,
        size: f32,
        anchor: Anchor,
        bold: bool,
        tone: Tone,
    },
}

impl Decor {
    /// Conservative bounds; text is estimated from its length.
    pub fn bounds(&self) -> Rect {
        match self {
            Decor::Poly { pts, .. } => {
                let mut r: Option<Rect> = None;
                for p in pts {
                    let pr = Rect { x: p[0], y: p[1], w: 0.0, h: 0.0 };
                    r = Some(r.map_or(pr, |r| r.union(pr)));
                }
                r.unwrap_or_default()
            }
            Decor::Line { pts, width, .. } => {
                let mut r: Option<Rect> = None;
                for p in pts {
                    let pr = Rect {
                        x: p[0],
                        y: p[1],
                        w: 0.0,
                        h: 0.0,
                    };
                    r = Some(r.map_or(pr, |r| r.union(pr)));
                }
                r.unwrap_or_default().inflate(width / 2.0)
            }
            Decor::Rect { rect, .. } => *rect,
            Decor::Wedge { c, r, .. } | Decor::Circle { c, r, .. } => Rect {
                x: c[0] - r,
                y: c[1] - r,
                w: 2.0 * r,
                h: 2.0 * r,
            },
            Decor::Text {
                at,
                text,
                size,
                anchor,
                ..
            } => {
                let w = text.chars().count() as f32 * size * 0.55;
                let x = match anchor {
                    Anchor::Start => at[0],
                    Anchor::Middle => at[0] - w / 2.0,
                    Anchor::End => at[0] - w,
                };
                Rect {
                    x,
                    y: at[1] - size * 0.6,
                    w,
                    h: size * 1.2,
                }
            }
        }
    }

    /// Moves the primitive.
    pub fn translate(&mut self, dx: f32, dy: f32) {
        match self {
            Decor::Line { pts, .. } | Decor::Poly { pts, .. } => {
                for p in pts {
                    p[0] += dx;
                    p[1] += dy;
                }
            }
            Decor::Rect { rect, .. } => {
                rect.x += dx;
                rect.y += dy;
            }
            Decor::Wedge { c, .. } | Decor::Circle { c, .. } => {
                c[0] += dx;
                c[1] += dy;
            }
            Decor::Text { at, .. } => {
                at[0] += dx;
                at[1] += dy;
            }
        }
    }
}

const CYCLE: usize = 7;

fn swatch(c: &DiagramColors, n: u8) -> Swatch {
    [c.sky, c.peach, c.mint, c.yellow, c.lavender, c.rose, c.gray][n as usize % CYCLE]
}

/// Fill and stroke colour of `tone`.
pub fn tone_colors(tone: Tone, c: &DiagramColors) -> (Rgba, Rgba) {
    match tone {
        Tone::Ink => (c.text, c.text),
        Tone::Muted => (c.edge, c.edge),
        Tone::Grid => {
            let mut g = c.edge;
            g[3] = 34;
            (g, g)
        }
        Tone::Canvas => (c.canvas, c.canvas),
        Tone::Swatch(n) => {
            let s = swatch(c, n);
            (s.fill, s.stroke)
        }
    }
}

/// Points of the outline of a wedge, `step` radians apart.
pub fn wedge_points(c: [f32; 2], r: f32, a0: f32, a1: f32) -> Vec<[f32; 2]> {
    let sweep = a1 - a0;
    let steps = ((sweep.abs() / 0.07).ceil() as usize).clamp(2, 200);
    let mut pts = Vec::with_capacity(steps + 2);
    pts.push(c);
    for i in 0..=steps {
        let a = a0 + sweep * i as f32 / steps as f32;
        pts.push([c[0] + r * a.cos(), c[1] + r * a.sin()]);
    }
    pts
}
