//! Shape geometry: node sizes, outlines and boundary intersection.

use crate::graph::ShapeKind;
use crate::metrics::Metrics;
use crate::text::TextBlock;

/// Control-point factor for a quarter-circle cubic.
const KAPPA: f32 = 0.552_284_75;
/// Smallest node size.
const MIN_W: f32 = 72.0;
const MIN_H: f32 = 40.0;
/// Fillet length at polygon vertices.
const FILLET: f32 = 4.0;

/// Axis-aligned rectangle (top-left origin).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// Centre point.
    pub fn center(self) -> [f32; 2] {
        [self.x + self.w / 2.0, self.y + self.h / 2.0]
    }

    /// Right edge.
    pub fn right(self) -> f32 {
        self.x + self.w
    }

    /// Bottom edge.
    pub fn bottom(self) -> f32 {
        self.y + self.h
    }

    /// True when the rectangles overlap or touch.
    pub fn intersects(self, o: Rect) -> bool {
        self.x <= o.right() && o.x <= self.right() && self.y <= o.bottom() && o.y <= self.bottom()
    }

    /// True when `p` is inside or on the border.
    pub fn contains(self, p: [f32; 2]) -> bool {
        p[0] >= self.x && p[0] <= self.right() && p[1] >= self.y && p[1] <= self.bottom()
    }

    /// Smallest rectangle containing both.
    pub fn union(self, o: Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect {
            x,
            y,
            w: self.right().max(o.right()) - x,
            h: self.bottom().max(o.bottom()) - y,
        }
    }

    /// Grows the rectangle by `d` on every side.
    pub fn inflate(self, d: f32) -> Rect {
        Rect {
            x: self.x - d,
            y: self.y - d,
            w: self.w + 2.0 * d,
            h: self.h + 2.0 * d,
        }
    }
}

/// One drawing command of a closed outline.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PathCmd {
    Move([f32; 2]),
    Line([f32; 2]),
    Cubic([f32; 2], [f32; 2], [f32; 2]),
    Close,
}

/// Node size for a shape holding `text`.
pub fn size(kind: ShapeKind, text: &TextBlock, m: &Metrics) -> [f32; 2] {
    let w = text.width + 2.0 * m.pad_x;
    let h = text.height + 2.0 * m.pad_y;
    let [w, h] = match kind {
        ShapeKind::Rect | ShapeKind::Rounded => [w, h],
        ShapeKind::Subroutine => [w + 16.0, h],
        ShapeKind::Stadium => [w + h / 2.0, h],
        ShapeKind::Circle | ShapeKind::DoubleCircle => {
            let mut d = text.width.max(text.height) + 2.0 * m.pad_x;
            if kind == ShapeKind::DoubleCircle {
                d += 10.0;
            }
            let d = d.max(MIN_W);
            return [d, d];
        }
        ShapeKind::Diamond => [
            text.width + text.height + 2.0 * m.pad_x,
            (text.width * 0.5 + text.height + 2.0 * m.pad_y).max(56.0),
        ],
        ShapeKind::Hexagon
        | ShapeKind::Parallelogram
        | ShapeKind::ParallelogramAlt
        | ShapeKind::Trapezoid
        | ShapeKind::TrapezoidAlt
        | ShapeKind::Asymmetric => [w + h * 0.5, h],
        ShapeKind::Cylinder => [w, h + 16.0],
    };
    [w.max(MIN_W), h.max(MIN_H)]
}

/// Unrounded polygon vertices for polygonal shapes.
fn polygon(kind: ShapeKind, r: Rect) -> Option<Vec<[f32; 2]>> {
    let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
    let [cx, cy] = r.center();
    let quarter = r.h * 0.25;
    let skew = r.h * 0.5;
    Some(match kind {
        ShapeKind::Diamond => vec![[cx, y0], [x1, cy], [cx, y1], [x0, cy]],
        ShapeKind::Hexagon => vec![
            [x0 + quarter, y0],
            [x1 - quarter, y0],
            [x1, cy],
            [x1 - quarter, y1],
            [x0 + quarter, y1],
            [x0, cy],
        ],
        ShapeKind::Parallelogram => vec![[x0 + skew, y0], [x1, y0], [x1 - skew, y1], [x0, y1]],
        ShapeKind::ParallelogramAlt => vec![[x0, y0], [x1 - skew, y0], [x1, y1], [x0 + skew, y1]],
        ShapeKind::Trapezoid => vec![[x0 + quarter, y0], [x1 - quarter, y0], [x1, y1], [x0, y1]],
        ShapeKind::TrapezoidAlt => {
            vec![[x0, y0], [x1, y0], [x1 - quarter, y1], [x0 + quarter, y1]]
        }
        ShapeKind::Asymmetric => vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0 + quarter, cy]],
        _ => return None,
    })
}

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn length(v: [f32; 2]) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

/// Rounded rectangle path with corner radius `radius` (capped to half the sides).
fn rounded_rect(r: Rect, radius: f32) -> Vec<PathCmd> {
    let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
    let rad = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    let k = rad * KAPPA;
    vec![
        PathCmd::Move([x0 + rad, y0]),
        PathCmd::Line([x1 - rad, y0]),
        PathCmd::Cubic([x1 - rad + k, y0], [x1, y0 + rad - k], [x1, y0 + rad]),
        PathCmd::Line([x1, y1 - rad]),
        PathCmd::Cubic([x1, y1 - rad + k], [x1 - rad + k, y1], [x1 - rad, y1]),
        PathCmd::Line([x0 + rad, y1]),
        PathCmd::Cubic([x0 + rad - k, y1], [x0, y1 - rad + k], [x0, y1 - rad]),
        PathCmd::Line([x0, y0 + rad]),
        PathCmd::Cubic([x0, y0 + rad - k], [x0 + rad - k, y0], [x0 + rad, y0]),
        PathCmd::Close,
    ]
}

fn ellipse(r: Rect) -> Vec<PathCmd> {
    let [cx, cy] = r.center();
    let (rx, ry) = (r.w / 2.0, r.h / 2.0);
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    vec![
        PathCmd::Move([cx + rx, cy]),
        PathCmd::Cubic([cx + rx, cy + ky], [cx + kx, cy + ry], [cx, cy + ry]),
        PathCmd::Cubic([cx - kx, cy + ry], [cx - rx, cy + ky], [cx - rx, cy]),
        PathCmd::Cubic([cx - rx, cy - ky], [cx - kx, cy - ry], [cx, cy - ry]),
        PathCmd::Cubic([cx + kx, cy - ry], [cx + rx, cy - ky], [cx + rx, cy]),
        PathCmd::Close,
    ]
}

/// Polygon with every vertex rounded by a short cubic fillet.
fn filleted_polygon(points: &[[f32; 2]]) -> Vec<PathCmd> {
    let n = points.len();
    let mut path = Vec::with_capacity(n * 2 + 1);
    for (i, &v) in points.iter().enumerate() {
        let prev = points[(i + n - 1) % n];
        let next = points[(i + 1) % n];
        let to_prev = sub(prev, v);
        let to_next = sub(next, v);
        let (len_prev, len_next) = (length(to_prev), length(to_next));
        let d = FILLET.min(len_prev / 2.0).min(len_next / 2.0);
        let toward = |dir: [f32; 2], len: f32| {
            if len > 0.0 {
                [v[0] + dir[0] / len * d, v[1] + dir[1] / len * d]
            } else {
                v
            }
        };
        let a = toward(to_prev, len_prev);
        let b = toward(to_next, len_next);
        if i == 0 {
            path.push(PathCmd::Move(a));
        } else {
            path.push(PathCmd::Line(a));
        }
        path.push(PathCmd::Cubic(v, v, b));
    }
    path.push(PathCmd::Close);
    path
}

/// Closed outline of the shape in world units. Double circles and subroutines
/// return their outer outline only; inner rings and bars are decoration.
pub fn outline(kind: ShapeKind, r: Rect, radius: f32) -> Vec<PathCmd> {
    let r = Rect {
        w: r.w.max(0.0),
        h: r.h.max(0.0),
        ..r
    };
    match kind {
        ShapeKind::Rect | ShapeKind::Subroutine => rounded_rect(r, radius),
        ShapeKind::Rounded => rounded_rect(r, radius * 1.5),
        ShapeKind::Stadium => rounded_rect(r, r.h / 2.0),
        ShapeKind::Circle | ShapeKind::DoubleCircle => ellipse(r),
        ShapeKind::Cylinder => {
            let ry = 8.0f32.min(r.h / 4.0);
            let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
            let cx = r.center()[0];
            let k = ry * KAPPA;
            let kx = r.w / 2.0 * KAPPA;
            vec![
                PathCmd::Move([x0, y0 + ry]),
                PathCmd::Cubic([x0, y0 + ry - k], [cx - kx, y0], [cx, y0]),
                PathCmd::Cubic([cx + kx, y0], [x1, y0 + ry - k], [x1, y0 + ry]),
                PathCmd::Line([x1, y1 - ry]),
                PathCmd::Cubic([x1, y1 - ry + k], [cx + kx, y1], [cx, y1]),
                PathCmd::Cubic([cx - kx, y1], [x0, y1 - ry + k], [x0, y1 - ry]),
                PathCmd::Close,
            ]
        }
        _ => match polygon(kind, r) {
            Some(points) => filleted_polygon(&points),
            None => rounded_rect(r, radius),
        },
    }
}

/// Where the ray from the centre of `r` along `d` leaves the rectangle.
fn rect_hit(r: Rect, d: [f32; 2]) -> [f32; 2] {
    let [cx, cy] = r.center();
    let tx = if d[0].abs() > 1e-9 {
        (r.w / 2.0) / d[0].abs()
    } else {
        f32::INFINITY
    };
    let ty = if d[1].abs() > 1e-9 {
        (r.h / 2.0) / d[1].abs()
    } else {
        f32::INFINITY
    };
    let t = tx.min(ty);
    [cx + d[0] * t, cy + d[1] * t]
}

/// Point where the ray from the centre of `r` towards `toward` leaves the shape.
pub fn boundary(kind: ShapeKind, r: Rect, toward: [f32; 2]) -> [f32; 2] {
    let [cx, cy] = r.center();
    let d = sub(toward, [cx, cy]);
    if !d[0].is_finite() || !d[1].is_finite() || length(d) < 1e-6 {
        return [r.right(), cy];
    }
    let (hw, hh) = (r.w / 2.0, r.h / 2.0);
    match kind {
        ShapeKind::Circle | ShapeKind::DoubleCircle => {
            if hw <= 0.0 || hh <= 0.0 {
                return rect_hit(r, d);
            }
            let q = (d[0] / hw).powi(2) + (d[1] / hh).powi(2);
            let t = 1.0 / q.sqrt();
            [cx + d[0] * t, cy + d[1] * t]
        }
        ShapeKind::Stadium => stadium_hit(r, d),
        _ => match polygon(kind, r) {
            Some(points) => polygon_hit(&points, [cx, cy], d).unwrap_or_else(|| rect_hit(r, d)),
            None => rect_hit(r, d),
        },
    }
}

/// Ray hit for a stadium: rectangle middle with semicircular ends.
fn stadium_hit(r: Rect, d: [f32; 2]) -> [f32; 2] {
    let [cx, cy] = r.center();
    let hit = rect_hit(r, d);
    let hh = r.h / 2.0;
    let k = r.w / 2.0 - hh;
    if k <= 0.0 || (hit[0] - cx).abs() <= k {
        return hit;
    }
    let ox = if d[0] > 0.0 { cx + k } else { cx - k };
    let fx = cx - ox;
    let a = d[0] * d[0] + d[1] * d[1];
    let b = 2.0 * fx * d[0];
    let c = fx * fx - hh * hh;
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 || a <= 0.0 {
        return hit;
    }
    let t = (-b + disc.sqrt()) / (2.0 * a);
    if t <= 0.0 {
        return hit;
    }
    [cx + d[0] * t, cy + d[1] * t]
}

/// Nearest ray/polygon-edge intersection in the direction `d` from `c`.
fn polygon_hit(points: &[[f32; 2]], c: [f32; 2], d: [f32; 2]) -> Option<[f32; 2]> {
    let n = points.len();
    let mut best: Option<f32> = None;
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        let e = sub(b, a);
        let denom = d[0] * e[1] - d[1] * e[0];
        if denom.abs() < 1e-9 {
            continue;
        }
        let ac = sub(a, c);
        let t = (ac[0] * e[1] - ac[1] * e[0]) / denom;
        let u = (ac[0] * d[1] - ac[1] * d[0]) / denom;
        if t > 1e-6 && (-1e-4..=1.0 + 1e-4).contains(&u) && best.is_none_or(|prev| t < prev) {
            best = Some(t);
        }
    }
    best.map(|t| [c[0] + d[0] * t, c[1] + d[1] * t])
}
