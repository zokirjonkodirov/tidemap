//! Geometry of edge ends: where a decoration sits on the curve, how the line is cut short
//! under it, and the rounded arrowhead outline. Shared by the SVG writer and the painter so
//! both draw the same thing.
//!
//! The direction of an end decoration follows the visible curve: it is the chord from the
//! point where the decoration's base meets the path to the tip, not the tangent of the last
//! control point. A Bézier that bends late (its last handle is axis-aligned) therefore gets
//! an arrowhead that lines up with the stroke the reader actually sees.

pub type Pt = [f32; 2];

/// Arrowhead length as a multiple of `StyleSheet::arrow_size`.
pub const ARROW_LENGTH: f32 = 1.4;
/// Arrowhead width as a fraction of its length.
pub const ARROW_WIDTH: f32 = 0.86;
/// Corner rounding of the arrowhead tip, as a fraction of its length.
const TIP_ROUND: f32 = 0.3;
/// Corner rounding of the two base corners, as a fraction of the half width.
const BASE_ROUND: f32 = 0.55;
/// The line runs this far (fraction of the decoration reach) under the decoration so no
/// gap shows between stroke and head.
pub const TUCK: f32 = 0.55;

fn sub(a: Pt, b: Pt) -> Pt {
    [a[0] - b[0], a[1] - b[1]]
}

fn dist(a: Pt, b: Pt) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

pub fn unit(v: Pt) -> Option<Pt> {
    let len = v[0].hypot(v[1]);
    (len > 1e-4 && len.is_finite()).then(|| [v[0] / len, v[1] / len])
}

fn lerp(a: Pt, b: Pt, t: f32) -> Pt {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn cubic(p: &[Pt; 4], t: f32) -> Pt {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    [
        a * p[0][0] + b * p[1][0] + c * p[2][0] + d * p[3][0],
        a * p[0][1] + b * p[1][1] + c * p[2][1] + d * p[3][1],
    ]
}

/// The part `[0, t]` of a cubic (de Casteljau).
fn split_head(p: &[Pt; 4], t: f32) -> [Pt; 4] {
    let a = lerp(p[0], p[1], t);
    let b = lerp(p[1], p[2], t);
    let c = lerp(p[2], p[3], t);
    let d = lerp(a, b, t);
    let e = lerp(b, c, t);
    let f = lerp(d, e, t);
    [p[0], a, d, f]
}

fn is_chain(points: &[Pt]) -> bool {
    points.len() >= 4 && (points.len() - 1) % 3 == 0
}

/// Cuts the end of `points` (a cubic chain or a polyline) where it is `reach` away from its
/// last point in a straight line, walking back from the end. Returns the shortened path and
/// the cut point, or `None` when the whole path lies within `reach` of its end.
pub fn cut_end(points: &[Pt], reach: f32) -> Option<(Vec<Pt>, Pt)> {
    let tip = *points.last()?;
    if points.len() < 2 || !(reach > 0.0) {
        return None;
    }
    let far = |p: Pt| dist(p, tip) >= reach;
    if is_chain(points) {
        let pieces = (points.len() - 1) / 3;
        for s in (0..pieces).rev() {
            let c = [points[3 * s], points[3 * s + 1], points[3 * s + 2], points[3 * s + 3]];
            const STEPS: usize = 40;
            // Walk t from the end of the piece back to its start; `inside` is the last t
            // still within reach.
            let mut inside = 1.0f32;
            let mut found = None;
            for k in (0..STEPS).rev() {
                let t = k as f32 / STEPS as f32;
                if far(cubic(&c, t)) {
                    found = Some((t, inside));
                    break;
                }
                inside = t;
            }
            if let Some((mut out_t, mut in_t)) = found {
                for _ in 0..24 {
                    let mid = (out_t + in_t) * 0.5;
                    if far(cubic(&c, mid)) {
                        out_t = mid;
                    } else {
                        in_t = mid;
                    }
                }
                let head = split_head(&c, out_t);
                let mut path = points[..=3 * s].to_vec();
                path.extend_from_slice(&head[1..]);
                return Some((path, head[3]));
            }
        }
        None
    } else {
        for i in (0..points.len() - 1).rev() {
            let (a, b) = (points[i], points[i + 1]);
            if far(a) {
                let (mut out_t, mut in_t) = (0.0f32, 1.0f32);
                for _ in 0..24 {
                    let mid = (out_t + in_t) * 0.5;
                    if far(lerp(a, b, mid)) {
                        out_t = mid;
                    } else {
                        in_t = mid;
                    }
                }
                let at = lerp(a, b, out_t);
                let mut path = points[..=i].to_vec();
                path.push(at);
                return Some((path, at));
            }
        }
        None
    }
}

/// Same as [`cut_end`] for the start of the path.
pub fn cut_start(points: &[Pt], reach: f32) -> Option<(Vec<Pt>, Pt)> {
    let reversed: Vec<Pt> = points.iter().rev().copied().collect();
    let (mut path, at) = cut_end(&reversed, reach)?;
    path.reverse();
    Some((path, at))
}

/// Tangent fallback: direction in which the path arrives at its last point.
fn end_tangent(points: &[Pt]) -> Option<Pt> {
    let last = *points.last()?;
    points.iter().rev().skip(1).find_map(|p| unit(sub(last, *p)))
}

/// Direction (unit, pointing at the tip) of a decoration of `reach` at the end of `points`.
pub fn end_dir(points: &[Pt], reach: f32) -> Option<Pt> {
    let tip = *points.last()?;
    cut_end(points, reach)
        .and_then(|(_, base)| unit(sub(tip, base)))
        .or_else(|| end_tangent(points))
}

/// Direction (unit, pointing outward at the first point) of a decoration at the start.
pub fn start_dir(points: &[Pt], reach: f32) -> Option<Pt> {
    let reversed: Vec<Pt> = points.iter().rev().copied().collect();
    end_dir(&reversed, reach)
}

/// The line itself, shortened under the end decorations. `cut_*` is the reach of the
/// decoration at that end (0 for none).
pub fn trimmed(points: &[Pt], cut_start_reach: f32, cut_end_reach: f32) -> Vec<Pt> {
    let mut path = points.to_vec();
    if cut_end_reach > 0.0
        && let Some((shorter, _)) = cut_end(&path, cut_end_reach * TUCK)
    {
        path = shorter;
    }
    if cut_start_reach > 0.0
        && let Some((shorter, _)) = cut_start(&path, cut_start_reach * TUCK)
    {
        path = shorter;
    }
    path
}

/// One rounded corner of an outline: the curve enters at `from`, bends around `corner`
/// (a quadratic control point) and leaves at `to`.
#[derive(Clone, Copy, Debug)]
pub struct Corner {
    pub from: Pt,
    pub corner: Pt,
    pub to: Pt,
}

/// The rounded arrowhead whose rounded tip touches `tip`, pointing along `dir`.
/// Returns the three corners in drawing order (tip, then the two base corners) and the
/// head's reach (how far back from the tip its base lies).
pub fn arrow_head(tip: Pt, dir: Pt, size: f32) -> ([Corner; 3], f32) {
    let length = size * ARROW_LENGTH;
    let half = length * ARROW_WIDTH * 0.5;
    let n = [-dir[1], dir[0]];
    // A rounded tip stops short of the sharp corner (its apex is the quadratic's midpoint,
    // half the tangent distance times the cosine of the half apex angle back); push the
    // outline forward by that much so the visible tip still lands on `tip`.
    let r_tip = length * TIP_ROUND;
    let gap = 0.5 * r_tip * (length / length.hypot(half));
    let t = [tip[0] + dir[0] * gap, tip[1] + dir[1] * gap];
    let at = |along: f32, across: f32| -> Pt {
        [
            t[0] - dir[0] * along + n[0] * across,
            t[1] - dir[1] * along + n[1] * across,
        ]
    };
    let points = [t, at(length, half), at(length, -half)];
    let radii = [r_tip, half * BASE_ROUND, half * BASE_ROUND];
    let mut corners = [Corner { from: t, corner: t, to: t }; 3];
    for i in 0..3 {
        let prev = points[(i + 2) % 3];
        let here = points[i];
        let next = points[(i + 1) % 3];
        let (to_prev, to_next) = (sub(prev, here), sub(next, here));
        let (lp, ln) = (to_prev[0].hypot(to_prev[1]), to_next[0].hypot(to_next[1]));
        let r = radii[i].min(lp * 0.45).min(ln * 0.45);
        corners[i] = Corner {
            from: lerp(here, prev, r / lp.max(1e-4)),
            corner: here,
            to: lerp(here, next, r / ln.max(1e-4)),
        };
    }
    (corners, length - gap)
}

/// How far back from the tip an arrowhead of `size` reaches along the path.
pub fn arrow_reach(size: f32) -> f32 {
    size * ARROW_LENGTH
}

/// The arrowhead outline flattened to a convex polygon, `steps` segments per corner.
pub fn arrow_polygon(corners: &[Corner; 3], steps: usize) -> Vec<Pt> {
    let steps = steps.max(1);
    let mut out = Vec::with_capacity(3 * (steps + 1));
    for c in corners {
        for k in 0..=steps {
            let t = k as f32 / steps as f32;
            let a = lerp(c.from, c.corner, t);
            let b = lerp(c.corner, c.to, t);
            out.push(lerp(a, b, t));
        }
    }
    out
}

/// One drawing primitive of a line-end marker. `true` means filled with the line colour,
/// `false` means filled with the background and outlined.
#[derive(Clone, Debug, PartialEq)]
pub enum Prim {
    Poly(Vec<Pt>, bool),
    Seg(Pt, Pt),
    Circle(Pt, f32, bool),
}

/// Whether `kind` is drawn from [`marker`] primitives rather than the classic arrow, circle
/// and cross.
pub fn is_marker(kind: crate::graph::ArrowKind) -> bool {
    use crate::graph::ArrowKind as A;
    !matches!(kind, A::None | A::Arrow | A::Circle | A::Cross)
}

fn mark_unit(size: f32) -> f32 {
    size * ARROW_LENGTH
}

/// How far back from the tip the marker reaches; the line is cut that far short.
pub fn marker_reach(kind: crate::graph::ArrowKind, size: f32) -> f32 {
    use crate::graph::ArrowKind as A;
    let u = mark_unit(size);
    match kind {
        A::Open => 0.0,
        A::Triangle => 1.1 * u,
        A::Diamond | A::DiamondFilled => 1.6 * u,
        A::ExactlyOne => 1.0 * u,
        A::ZeroOrOne => 1.65 * u,
        A::ZeroOrMore => 1.95 * u,
        A::OneOrMore => 1.2 * u,
        _ => 0.0,
    }
}

/// Primitives of the marker whose tip touches `tip`, pointing along `dir`.
pub fn marker(kind: crate::graph::ArrowKind, tip: Pt, dir: Pt, size: f32) -> Vec<Prim> {
    use crate::graph::ArrowKind as A;
    let u = mark_unit(size);
    let n = [-dir[1], dir[0]];
    let at = |along: f32, across: f32| -> Pt {
        [
            tip[0] - dir[0] * along + n[0] * across,
            tip[1] - dir[1] * along + n[1] * across,
        ]
    };
    let bar = |along: f32| Prim::Seg(at(along, -0.55 * u), at(along, 0.55 * u));
    let foot = |base: f32| {
        vec![
            Prim::Seg(at(base, 0.0), at(0.0, 0.0)),
            Prim::Seg(at(base, 0.0), at(0.0, 0.55 * u)),
            Prim::Seg(at(base, 0.0), at(0.0, -0.55 * u)),
        ]
    };
    match kind {
        A::Open => vec![
            Prim::Seg(at(0.9 * u, 0.5 * u), tip),
            Prim::Seg(at(0.9 * u, -0.5 * u), tip),
        ],
        A::Triangle => vec![Prim::Poly(
            vec![tip, at(1.1 * u, 0.55 * u), at(1.1 * u, -0.55 * u)],
            false,
        )],
        A::Diamond | A::DiamondFilled => vec![Prim::Poly(
            vec![
                tip,
                at(0.8 * u, 0.45 * u),
                at(1.6 * u, 0.0),
                at(0.8 * u, -0.45 * u),
            ],
            kind == A::DiamondFilled,
        )],
        A::ExactlyOne => vec![bar(0.5 * u), bar(1.0 * u)],
        A::ZeroOrOne => vec![bar(0.5 * u), Prim::Circle(at(1.25 * u, 0.0), 0.4 * u, false)],
        A::ZeroOrMore => {
            let mut v = foot(1.0 * u);
            v.push(Prim::Circle(at(1.55 * u, 0.0), 0.4 * u, false));
            v
        }
        A::OneOrMore => {
            let mut v = foot(1.0 * u);
            v.push(bar(1.2 * u));
            v
        }
        _ => Vec::new(),
    }
}
