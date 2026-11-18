//! Edge routing: ports, cubic Bézier chains, clipping, self-loops, label rectangles.

use super::{EdgePath, Frame, LGraph, LKind};
use crate::graph::{Endpoint, Graph};
use crate::metrics::Metrics;
use crate::shape::{self, Rect};

/// Tangent handle length as a fraction of the rank distance of a segment.
const HANDLE: f32 = 0.4;
/// Share of a node side used to spread ports.
const PORT_SPREAD: f32 = 0.6;

/// Outward bulge of the `k`-th self-loop of a node.
pub(crate) fn loop_bulge(m: &Metrics, k: u32) -> f32 {
    m.node_gap * 0.8 + 14.0 * k as f32
}

struct Router<'a> {
    g: &'a Graph,
    rects: &'a [Rect],
    frame: &'a Frame,
}

impl Router<'_> {
    /// Normalised centre of a real node.
    fn centre(&self, v: usize) -> [f32; 2] {
        self.frame.unmap(self.rects[v].center())
    }

    /// Normalised (cross, rank) half extents of a real node.
    fn half(&self, v: usize) -> [f32; 2] {
        let r = self.rects[v];
        if self.frame.swaps() {
            [r.h / 2.0, r.w / 2.0]
        } else {
            [r.w / 2.0, r.h / 2.0]
        }
    }

    /// Projects a normalised point on the outline of real node `v` (ray from its centre).
    fn clip(&self, v: usize, q: [f32; 2]) -> [f32; 2] {
        let kind = self.g.nodes[v].shape;
        let b = shape::boundary(kind, self.rects[v], self.frame.map(q));
        self.frame.unmap(b)
    }
}

/// Waypoints worth keeping: the ends, anything flagged `pin`, and every point that
/// is not (within `tol`) on the straight line between its kept neighbours.
fn simplify(w: &[[f32; 2]], pin: &[bool], tol: f32) -> Vec<usize> {
    let k = w.len();
    let mut keep = vec![0usize];
    let mut a = 0usize;
    while a + 1 < k {
        // Furthest j such that every point strictly between a and j lies on line a-j.
        let mut best = a + 1;
        for j in a + 2..k {
            let dr = w[j][1] - w[a][1];
            let ok = dr > 1e-3
                && (a + 1..j).all(|i| {
                    let t = (w[i][1] - w[a][1]) / dr;
                    !pin[i] && (w[i][0] - (w[a][0] + t * (w[j][0] - w[a][0]))).abs() <= tol
                });
            if ok {
                best = j;
            } else {
                break;
            }
        }
        keep.push(best);
        a = best;
    }
    keep
}

/// How far the tangent at the two ends of an edge leans from the rank axis toward the
/// straight line to its neighbouring waypoint (0 = always along the rank axis).
const END_LEAN: f32 = 0.55;

/// Unit vector halfway (by `lean`) between the rank axis and `chord`.
fn leaned(chord: [f32; 2], lean: f32) -> [f32; 2] {
    let len = chord[0].hypot(chord[1]);
    if !(len > 1e-3) {
        return [0.0, 1.0];
    }
    let v = [chord[0] / len * lean, (1.0 - lean) + chord[1] / len * lean];
    let l = v[0].hypot(v[1]).max(1e-3);
    [v[0] / l, v[1] / l]
}

/// Builds a cubic chain through normalised `[cross, rank]` waypoints. Interior waypoints
/// have a tangent along the rank axis, so the path is a sequence of smooth S-bends. The two
/// ends lean toward the direction the edge actually travels, so an edge that comes in at an
/// angle meets its node (and its arrowhead) at that angle instead of bending sharply upright
/// in its last few points.
fn curve(w: &[[f32; 2]], bow: f32) -> Vec<[f32; 2]> {
    let k = w.len();
    let mut pts = Vec::with_capacity(3 * (k - 1) + 1);
    pts.push(w[0]);
    for i in 0..k - 1 {
        let (a, b) = (w[i], w[i + 1]);
        let dr = b[1] - a[1];
        let d = dr.max(0.0) * HANDLE;
        let chord = [b[0] - a[0], b[1] - a[1]];
        let start = if i == 0 && dr > 0.0 {
            let dir = leaned(chord, END_LEAN);
            [a[0] + bow + dir[0] * d, a[1] + dir[1] * d]
        } else {
            [a[0] + bow, a[1] + d]
        };
        let end = if i + 2 == k && dr > 0.0 {
            let dir = leaned(chord, END_LEAN);
            [b[0] + bow - dir[0] * d, b[1] - dir[1] * d]
        } else {
            [b[0] + bow, b[1] - d]
        };
        pts.push(start);
        pts.push(end);
        pts.push(b);
    }
    pts
}

/// Routes every authored edge. Points are in final coordinates, ordered from the
/// authored source to the authored target.
pub(crate) fn route(
    lg: &LGraph,
    g: &Graph,
    rects: &[Rect],
    frame: &Frame,
    label_dims: &[Option<[f32; 2]>],
    crects: &[Rect],
    title_w: &[f32],
    m: &Metrics,
) -> Vec<EdgePath> {
    let ne = g.edges.len();
    let n = g.nodes.len();
    let rt = Router { g, rects, frame };
    let mut out: Vec<EdgePath> = vec![
        EdgePath {
            points: Vec::new(),
            label: None,
        };
        ne
    ];

    // Chains: contiguous ranges of lg.edges per authored edge.
    let mut range: Vec<(u32, u32)> = vec![(0, 0); ne];
    let mut i = 0;
    while i < lg.edges.len() {
        let orig = lg.edges[i].orig.0 as usize;
        let mut j = i;
        while j < lg.edges.len() && lg.edges[j].orig.0 as usize == orig {
            j += 1;
        }
        if orig < ne {
            range[orig] = (i as u32, j as u32);
        }
        i = j;
    }
    let chain_nodes = |e: usize| -> Vec<u32> {
        let (a, b) = range[e];
        let mut v = Vec::with_capacity((b - a) as usize + 1);
        if b > a {
            v.push(lg.edges[a as usize].from);
            for k in a..b {
                v.push(lg.edges[k as usize].to);
            }
        }
        v
    };
    let node_c = |v: u32| -> [f32; 2] {
        let nd = &lg.nodes[v as usize];
        if (v as usize) < n {
            rt.centre(v as usize)
        } else {
            [nd.x, nd.y]
        }
    };

    // Point on a cluster rectangle for an edge that ends at a cluster border node.
    let cluster_point = |v: u32, nb: [f32; 2]| -> [f32; 2] {
        let c = match lg.nodes[v as usize].kind {
            LKind::BorderTop(c) | LKind::BorderBottom(c) => c.0 as usize,
            _ => return nb,
        };
        let Some(r) = crects.get(c) else {
            return nb;
        };
        let a = frame.unmap([r.x, r.y]);
        let b = frame.unmap([r.right(), r.bottom()]);
        let (c0, c1) = (a[0].min(b[0]), a[0].max(b[0]));
        let (r0, r1) = (a[1].min(b[1]), a[1].max(b[1]));
        let margin = 14.0f32.min((c1 - c0) / 2.0).max(0.0);
        let mut x = nb[0].clamp(c0 + margin, (c1 - margin).max(c0 + margin));
        let top = nb[1] < (r0 + r1) / 2.0;
        let title_side = match frame.dir {
            crate::graph::Direction::TB => top,
            crate::graph::Direction::BT => !top,
            _ => false,
        };
        if title_side {
            x = super::compound::dodge_title(x, c0, c1, title_w.get(c).copied().unwrap_or(0.0));
        }
        [x, if top { r0 } else { r1 }]
    };

    // Ports.
    let mut out_list: Vec<Vec<(f32, u32)>> = vec![Vec::new(); n];
    let mut in_list: Vec<Vec<(f32, u32)>> = vec![Vec::new(); n];
    let mut chains: Vec<Vec<u32>> = vec![Vec::new(); ne];
    for e in 0..ne {
        let ch = chain_nodes(e);
        if ch.len() >= 2 {
            let s = ch[0] as usize;
            let t = ch[ch.len() - 1] as usize;
            if s < n {
                out_list[s].push((node_c(ch[1])[0], e as u32));
            }
            if t < n {
                in_list[t].push((node_c(ch[ch.len() - 2])[0], e as u32));
            }
        }
        chains[e] = ch;
    }
    let mut start_off = vec![0.0f32; ne];
    let mut end_off = vec![0.0f32; ne];
    let is_rev = |e: u32| -> bool {
        let (a, b) = range[e as usize];
        b > a && lg.edges[a as usize].reversed
    };
    let assign = |list: &mut Vec<(f32, u32)>, v: usize, off: &mut [f32]| {
        list.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let width = rt.half(v)[0] * 2.0;
        // Ordinary edges share the middle of the side; cycle-reversed edges (which
        // loop around the outside) take the outer end on their own side.
        let ordinary: Vec<u32> = list.iter().filter(|x| !is_rev(x.1)).map(|x| x.1).collect();
        let k = ordinary.len();
        if k >= 2 {
            let spread = PORT_SPREAD * width;
            for (idx, &e) in ordinary.iter().enumerate() {
                off[e as usize] = -spread / 2.0 + spread * (idx as f32 + 0.5) / k as f32;
            }
        }
        let cx = rt.centre(v)[0];
        let both = k > 0;
        for &(nx, e) in list.iter() {
            if is_rev(e) {
                let side = if nx >= cx { 1.0 } else { -1.0 };
                off[e as usize] = if both { side * 0.36 * width } else { 0.0 };
            }
        }
        if !both {
            let k = list.len();
            if k >= 2 {
                let spread = PORT_SPREAD * width;
                for (idx, &(_, e)) in list.iter().enumerate() {
                    off[e as usize] = -spread / 2.0 + spread * (idx as f32 + 0.5) / k as f32;
                }
            }
        }
    };
    for v in 0..n {
        assign(&mut out_list[v], v, &mut start_off);
        assign(&mut in_list[v], v, &mut end_off);
    }

    // Parallel adjacent edges (same layout pair, no interior waypoint).
    let mut dup_bow = vec![0.0f32; ne];
    {
        let mut keyed: Vec<(u32, u32, u32)> = Vec::new();
        for (e, ch) in chains.iter().enumerate() {
            if ch.len() == 2 {
                keyed.push((ch[0], ch[1], e as u32));
            }
        }
        keyed.sort_unstable();
        let mut i = 0;
        while i < keyed.len() {
            let mut j = i;
            while j < keyed.len() && keyed[j].0 == keyed[i].0 && keyed[j].1 == keyed[i].1 {
                j += 1;
            }
            let cnt = j - i;
            if cnt > 1 {
                for (k, item) in keyed[i..j].iter().enumerate() {
                    dup_bow[item.2 as usize] = (k as f32 - (cnt as f32 - 1.0) / 2.0) * 16.0;
                }
            }
            i = j;
        }
    }

    let index = (n <= 4000).then(|| NodeIndex::new(rects));
    let mut loop_count = vec![0u32; n];
    for (e, edge) in g.edges.iter().enumerate() {
        match (edge.from, edge.to) {
            (Endpoint::Node(a), Endpoint::Node(b)) if a == b => {
                let v = a.0 as usize;
                let k = loop_count[v];
                loop_count[v] += 1;
                out[e] = self_loop(&rt, v, k, label_dims[e], m);
            }
            _ => {
                let ch = &chains[e];
                if ch.len() < 2 {
                    continue;
                }
                let (s, t) = (ch[0] as usize, ch[ch.len() - 1] as usize);
                let sq = if s < n {
                    let (cs, hs) = (rt.centre(s), rt.half(s));
                    rt.clip(s, [cs[0] + start_off[e], cs[1] + hs[1]])
                } else {
                    cluster_point(ch[0], node_c(ch[1]))
                };
                let eq = if t < n {
                    let (ct, ht) = (rt.centre(t), rt.half(t));
                    rt.clip(t, [ct[0] + end_off[e], ct[1] - ht[1]])
                } else {
                    cluster_point(ch[ch.len() - 1], node_c(ch[ch.len() - 2]))
                };
                let mut wp: Vec<[f32; 2]> = Vec::with_capacity(ch.len());
                wp.push(sq);
                for &v in &ch[1..ch.len() - 1] {
                    wp.push(node_c(v));
                }
                wp.push(eq);
                let mut label = None;
                if let Some(d) = label_dims[e] {
                    let c = ch[1..ch.len() - 1]
                        .iter()
                        .find(|&&v| matches!(lg.nodes[v as usize].kind, LKind::Label(_)))
                        .map(|&v| node_c(v))
                        .unwrap_or([(sq[0] + eq[0]) / 2.0, (sq[1] + eq[1]) / 2.0]);
                    let fc = frame.map(c);
                    label = Some(Rect {
                        x: fc[0] - d[0] / 2.0,
                        y: fc[1] - d[1] / 2.0,
                        w: d[0],
                        h: d[1],
                    });
                }
                // Waypoints next to a node or label stay (the curve must keep clear of it).
                let mut pin: Vec<bool> = vec![false; wp.len()];
                for (k, &v) in ch[1..ch.len() - 1].iter().enumerate() {
                    let nd = &lg.nodes[v as usize];
                    if !matches!(nd.kind, LKind::Dummy(_)) {
                        pin[k + 1] = true;
                        continue;
                    }
                    let layer = &lg.ranks[nd.rank.max(0) as usize];
                    let Some(pos) = layer.iter().position(|&u| u == v) else {
                        continue;
                    };
                    for nb in [pos.checked_sub(1), Some(pos + 1)].into_iter().flatten() {
                        let Some(&u) = layer.get(nb) else { continue };
                        let un = &lg.nodes[u as usize];
                        if matches!(un.kind, LKind::Real(_) | LKind::Label(_))
                            && (un.x - nd.x).abs() - un.w / 2.0 < 160.0
                        {
                            pin[k + 1] = true;
                        }
                    }
                }
                let kept = simplify(&wp, &pin, 2.0);
                let simple: Vec<[f32; 2]> = kept.iter().map(|&i| wp[i]).collect();
                let build = |w: &[[f32; 2]]| -> Vec<[f32; 2]> {
                    curve(w, dup_bow[e])
                        .into_iter()
                        .map(|p| frame.map(p))
                        .collect()
                };
                let mut pts = build(&simple);
                if let Some(ix) = &index {
                    let skip = [(s < n).then_some(s), (t < n).then_some(t)];
                    if let Some(first) = ix.hits(&pts, &skip) {
                        let mut cur = simple.clone();
                        if kept.len() < wp.len() {
                            cur = wp.clone();
                            pts = build(&cur);
                        }
                        let mut hit = ix.hits(&pts, &skip).or(Some(first).filter(|_| false));
                        let mut guard = 0;
                        while let Some(piece) = hit {
                            guard += 1;
                            if guard > 8 || piece + 1 >= cur.len() {
                                break;
                            }
                            let (a, b) = (cur[piece], cur[piece + 1]);
                            let dr = b[1] - a[1];
                            let mut fixed = false;
                            for f in [0.5f32, 0.65, 0.35, 0.8] {
                                for late in [true, false] {
                                    let mid = if late {
                                        [a[0], a[1] + f * dr]
                                    } else {
                                        [b[0], b[1] - f * dr]
                                    };
                                    let mut cand = cur.clone();
                                    cand.insert(piece + 1, mid);
                                    let cp = build(&cand);
                                    if ix.hits(&cp, &skip).is_none_or(|q| q > piece + 1) {
                                        cur = cand;
                                        pts = cp;
                                        fixed = true;
                                        break;
                                    }
                                }
                                if fixed {
                                    break;
                                }
                            }
                            if !fixed {
                                break;
                            }
                            hit = ix.hits(&pts, &skip);
                        }
                    }
                }
                if lg.edges[range[e].0 as usize].reversed {
                    pts.reverse();
                }
                out[e] = EdgePath { points: pts, label };
            }
        }
    }
    out
}

/// A loop on the cross+ side of the node, leaving at 30% and returning at 70% of its height.
fn self_loop(rt: &Router<'_>, v: usize, k: u32, label: Option<[f32; 2]>, m: &Metrics) -> EdgePath {
    let c = rt.centre(v);
    let [hc, hr] = rt.half(v);
    let bulge = loop_bulge(m, k);
    let (ra, rb) = (c[1] - hr + 0.6 * hr, c[1] - hr + 1.4 * hr);
    let s = rt.clip(v, [c[0] + hc, ra]);
    let e = rt.clip(v, [c[0] + hc, rb]);
    let reach = bulge / 0.75;
    let pts = [s, [s[0] + reach, ra], [e[0] + reach, rb], e];
    let label = label.map(|d| {
        let cross = if rt.frame.swaps() { d[1] } else { d[0] };
        let lc = [c[0] + hc + bulge + 4.0 + cross / 2.0, c[1]];
        let fc = rt.frame.map(lc);
        Rect {
            x: fc[0] - d[0] / 2.0,
            y: fc[1] - d[1] / 2.0,
            w: d[0],
            h: d[1],
        }
    });
    EdgePath {
        points: pts.iter().map(|&p| rt.frame.map(p)).collect(),
        label,
    }
}

/// Node rectangles sorted by top edge, for curve-versus-node clearance tests.
struct NodeIndex {
    /// `(top, bottom, left, right, node)` sorted by top.
    items: Vec<(f32, f32, f32, f32, usize)>,
    max_h: f32,
}

impl NodeIndex {
    fn new(rects: &[Rect]) -> Self {
        let mut items: Vec<_> = rects
            .iter()
            .enumerate()
            .map(|(i, r)| (r.y, r.bottom(), r.x, r.right(), i))
            .collect();
        items.sort_by(|a, b| a.0.total_cmp(&b.0));
        let max_h = rects.iter().map(|r| r.h).fold(0.0, f32::max);
        Self { items, max_h }
    }

    /// Index of the first cubic piece that comes within 3 px of a node outside `skip`.
    fn hits(&self, pts: &[[f32; 2]], skip: &[Option<usize>; 2]) -> Option<usize> {
        const CLEAR: f32 = 3.0;
        let mut prev = pts[0];
        for (piece, c) in pts[1..].chunks(3).enumerate() {
            if c.len() < 3 {
                break;
            }
            let p0 = prev;
            for i in 1..=10 {
                let t = i as f32 / 10.0;
                let u = 1.0 - t;
                let (a, b, cc, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                let q = [
                    a * p0[0] + b * c[0][0] + cc * c[1][0] + d * c[2][0],
                    a * p0[1] + b * c[0][1] + cc * c[1][1] + d * c[2][1],
                ];
                let (y0, y1) = (prev[1].min(q[1]), prev[1].max(q[1]));
                let (x0, x1) = (prev[0].min(q[0]), prev[0].max(q[0]));
                let lo = self
                    .items
                    .partition_point(|it| it.0 < y0 - self.max_h - CLEAR);
                for it in &self.items[lo..] {
                    if it.0 - CLEAR > y1 {
                        break;
                    }
                    if skip.contains(&Some(it.4))
                        || it.1 + CLEAR < y0
                        || it.3 + CLEAR < x0
                        || it.2 - CLEAR > x1
                    {
                        continue;
                    }
                    if seg_hits_rect(
                        prev,
                        q,
                        [it.2 - CLEAR, it.0 - CLEAR, it.3 + CLEAR, it.1 + CLEAR],
                    ) {
                        return Some(piece);
                    }
                }
                prev = q;
            }
            prev = c[2];
        }
        None
    }
}

/// Liang-Barsky segment versus `[x0, y0, x1, y1]`.
fn seg_hits_rect(a: [f32; 2], b: [f32; 2], r: [f32; 4]) -> bool {
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    let d = [b[0] - a[0], b[1] - a[1]];
    for (p, q) in [
        (-d[0], a[0] - r[0]),
        (d[0], r[2] - a[0]),
        (-d[1], a[1] - r[1]),
        (d[1], r[3] - a[1]),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}
