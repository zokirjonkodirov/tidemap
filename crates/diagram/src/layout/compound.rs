//! Compound (subgraph) support for the global path: cluster tree, border nodes,
//! nesting edges, cycle repair, rank tightening and cluster rectangles.

use super::{Frame, LEdge, LGraph, LKind, LNode};
use crate::graph::{ClusterId, EdgeId, Graph};
use crate::metrics::Metrics;
use crate::shape::Rect;

/// "No cluster" marker.
pub(crate) const NONE: u32 = u32::MAX;
/// Marker `orig` of working edges that are not an authored edge.
pub(crate) const NO_EDGE: EdgeId = EdgeId(u32::MAX);
/// Deepest supported subgraph nesting.
pub(crate) const MAX_DEPTH: u32 = 96;
/// Size of an empty subgraph box.
pub(crate) const EMPTY_BOX: [f32; 2] = [160.0, 72.0];
/// Extra width around a cluster title (12 px inset on each side).
pub(crate) const TITLE_INSET: f32 = 24.0;

/// Parent links and depths of the cluster forest (cycle-safe).
#[derive(Clone, Debug, Default)]
pub(crate) struct Tree {
    pub parent: Vec<u32>,
    pub depth: Vec<u32>,
}

impl Tree {
    pub(crate) fn new(g: &Graph) -> Self {
        let k = g.clusters.len();
        let mut parent: Vec<u32> = g
            .clusters
            .iter()
            .enumerate()
            .map(|(i, c)| match c.parent {
                Some(p) if (p.0 as usize) < k && p.0 as usize != i => p.0,
                _ => NONE,
            })
            .collect();
        loop {
            let mut changed = false;
            for i in 0..k {
                let mut x = i as u32;
                let mut steps = 0usize;
                while parent[x as usize] != NONE {
                    x = parent[x as usize];
                    steps += 1;
                    if steps > k {
                        break;
                    }
                }
                if steps > k {
                    parent[i] = NONE;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let mut depth = vec![0u32; k];
        for i in 0..k {
            let mut x = i as u32;
            let mut d = 0;
            while parent[x as usize] != NONE {
                x = parent[x as usize];
                d += 1;
            }
            depth[i] = d;
        }
        Self { parent, depth }
    }

    pub(crate) fn max_depth(&self) -> u32 {
        self.depth.iter().copied().max().unwrap_or(0)
    }

    fn d(&self, c: u32) -> i64 {
        if c == NONE {
            -1
        } else {
            i64::from(self.depth[c as usize])
        }
    }

    fn up(&self, c: u32) -> u32 {
        if c == NONE {
            NONE
        } else {
            self.parent[c as usize]
        }
    }

    /// Lowest common ancestor (`NONE` for the root).
    pub(crate) fn lca(&self, mut a: u32, mut b: u32) -> u32 {
        while a != b {
            let (da, db) = (self.d(a), self.d(b));
            if da >= db {
                a = self.up(a);
            }
            if db >= da {
                b = self.up(b);
            }
        }
        a
    }

    /// True when `c` equals `anc` or lies below it.
    pub(crate) fn inside(&self, mut c: u32, anc: u32) -> bool {
        while c != NONE {
            if c == anc {
                return true;
            }
            c = self.parent[c as usize];
        }
        false
    }
}

/// Creates the border nodes of every cluster (before ranking). A cluster without
/// members becomes a single box node. `title_w` is the title width per cluster.
pub(crate) fn add_borders(lg: &mut LGraph, g: &Graph, swap: bool, node_cluster: &[u32]) {
    let k = g.clusters.len();
    lg.tree = Tree::new(g);
    lg.cl_top = vec![NONE; k];
    lg.cl_bot = vec![NONE; k];
    lg.cl_leaf = vec![false; k];
    lg.cl_left = vec![Vec::new(); k];
    lg.cl_right = vec![Vec::new(); k];
    let mut count = vec![0u32; k];
    for &c in node_cluster {
        if c != NONE {
            count[c as usize] += 1;
        }
    }
    for &p in &lg.tree.parent {
        if p != NONE {
            count[p as usize] += 1;
        }
    }
    for c in 0..k {
        let cid = ClusterId(c as u32);
        let parent = lg.tree.parent[c];
        if count[c] == 0 {
            let d = if swap {
                [EMPTY_BOX[1], EMPTY_BOX[0]]
            } else {
                EMPTY_BOX
            };
            let id = lg.nodes.len() as u32;
            lg.nodes.push(LNode {
                kind: LKind::BorderTop(cid),
                w: d[0],
                h: d[1],
                rank: 0,
                order: 0,
                x: 0.0,
                y: 0.0,
                cluster: (parent != NONE).then_some(ClusterId(parent)),
            });
            lg.cl_top[c] = id;
            lg.cl_bot[c] = id;
            lg.cl_leaf[c] = true;
            continue;
        }
        for (kind, w) in [
            (LKind::BorderTop(cid), 0.0),
            (LKind::BorderBottom(cid), 0.0),
        ] {
            let id = lg.nodes.len() as u32;
            lg.nodes.push(LNode {
                kind,
                w,
                h: 0.0,
                rank: 0,
                order: 0,
                x: 0.0,
                y: 0.0,
                cluster: Some(cid),
            });
            if matches!(kind, LKind::BorderTop(_)) {
                lg.cl_top[c] = id;
            } else {
                lg.cl_bot[c] = id;
            }
        }
    }
}

/// Appends the zero-weight nesting edges (top -> members -> bottom). Returns the
/// number of working edges before them.
pub(crate) fn add_nesting(lg: &mut LGraph, node_cluster: &[u32]) -> usize {
    let first = lg.edges.len();
    let push = |lg: &mut LGraph, a: u32, b: u32| {
        lg.edges.push(LEdge {
            from: a,
            to: b,
            orig: NO_EDGE,
            reversed: false,
            weight: 0,
            min_len: 1,
        });
    };
    for (v, &c) in node_cluster.iter().enumerate() {
        if c != NONE && !lg.cl_leaf[c as usize] {
            let (t, b) = (lg.cl_top[c as usize], lg.cl_bot[c as usize]);
            push(lg, t, v as u32);
            push(lg, v as u32, b);
        }
    }
    for d in 0..lg.tree.parent.len() {
        let p = lg.tree.parent[d];
        if p != NONE && !lg.cl_leaf[p as usize] {
            let (t, b) = (lg.cl_top[p as usize], lg.cl_bot[p as usize]);
            let (dt, db) = (lg.cl_top[d], lg.cl_bot[d]);
            push(lg, t, dt);
            push(lg, db, b);
        }
    }
    first
}

/// Appends zero-weight "flow" edges: an edge between different clusters (or a cluster
/// and a node) makes the whole source cluster precede the target one, so connected
/// clusters form tiers instead of interleaving. Returns the index of the first one.
pub(crate) fn add_flow_edges(lg: &mut LGraph, real_edges: usize) -> usize {
    let first = lg.edges.len();
    let mut pairs: Vec<(u32, u32)> = Vec::new();
    let mut units: Vec<(u32, u32, u32, u32, usize)> = Vec::new();
    let mut flips: Vec<usize> = Vec::new();
    {
        let end = |v: u32| -> (u32, u32) {
            // (cluster the endpoint stands for or NONE, container)
            match lg.nodes[v as usize].kind {
                LKind::BorderTop(c) | LKind::BorderBottom(c) if !lg.cl_leaf[c.0 as usize] => {
                    (c.0, lg.tree.parent[c.0 as usize])
                }
                _ => {
                    let c = lg.nodes[v as usize].cluster.map_or(NONE, |c| c.0);
                    (NONE, c)
                }
            }
        };
        let unit = |own: u32, cont: u32, l: u32| -> u32 {
            let mut c = if own != NONE { own } else { cont };
            if c == NONE || c == l {
                return NONE;
            }
            while lg.tree.parent[c as usize] != l {
                c = lg.tree.parent[c as usize];
                if c == NONE {
                    return NONE;
                }
            }
            c
        };
        for (ei, e) in lg.edges[..real_edges].iter().enumerate() {
            let (oa, ca) = end(e.from);
            let (ob, cb) = end(e.to);
            let l = lg.tree.lca(ca, cb);
            let (ua, ub) = (unit(oa, ca, l), unit(ob, cb, l));
            if ua == NONE && ub == NONE {
                continue;
            }
            let a = if ua != NONE {
                lg.cl_bot[ua as usize]
            } else {
                e.from
            };
            let b = if ub != NONE {
                lg.cl_top[ub as usize]
            } else {
                e.to
            };
            if a != b {
                let key = |u: u32, v: u32| if u != NONE { u } else { v | 0x8000_0000 };
                units.push((key(ua, e.from), key(ub, e.to), a, b, ei));
            }
        }
    }
    // Opposite flows between the same pair of units: keep the better supported one.
    units.sort_unstable();
    let count = |x: u32, y: u32| -> usize {
        let lo = units.partition_point(|u| (u.0, u.1) < (x, y));
        let hi = units.partition_point(|u| (u.0, u.1) <= (x, y));
        hi - lo
    };
    for u in &units {
        let (c, r) = (count(u.0, u.1), count(u.1, u.0));
        if r > c || (r == c && u.0 > u.1) {
            if r > c {
                flips.push(u.4);
            }
            continue;
        }
        pairs.push((u.2, u.3));
    }
    // Real edges that run against the dominant cluster flow become back edges.
    for ei in flips {
        let ed = &mut lg.edges[ei];
        std::mem::swap(&mut ed.from, &mut ed.to);
        ed.reversed = !ed.reversed;
    }
    pairs.sort_unstable();
    pairs.dedup();
    for (a, b) in pairs {
        lg.edges.push(LEdge {
            from: a,
            to: b,
            orig: NO_EDGE,
            reversed: false,
            weight: 0,
            min_len: 1,
        });
    }
    first
}

/// Makes the working graph acyclic. Edges before `first_nesting` may be reversed,
/// nesting edges are fixed, flow edges (from `first_flow`) are dropped when needed.
pub(crate) fn break_cycles(lg: &mut LGraph, first_nesting: usize, first_flow: usize) {
    let n = lg.nodes.len();
    for _ in 0..64 {
        let m = lg.edges.len();
        let mut off = vec![0u32; n + 1];
        for e in &lg.edges {
            off[e.from as usize + 1] += 1;
        }
        for i in 0..n {
            off[i + 1] += off[i];
        }
        let mut fill = off.clone();
        let mut adj = vec![0u32; m];
        for (i, e) in lg.edges.iter().enumerate() {
            adj[fill[e.from as usize] as usize] = i as u32;
            fill[e.from as usize] += 1;
        }
        let mut color = vec![0u8; n];
        let mut kill = vec![false; m];
        let mut flip = vec![false; m];
        let mut any = false;
        for root in 0..n {
            if color[root] != 0 {
                continue;
            }
            let mut stack: Vec<(u32, u32, u32)> = vec![(root as u32, off[root], u32::MAX)];
            color[root] = 1;
            while let Some(top) = stack.len().checked_sub(1) {
                let (v, slot, _) = stack[top];
                if slot < off[v as usize + 1] {
                    stack[top].1 += 1;
                    let ei = adj[slot as usize];
                    if kill[ei as usize] {
                        continue;
                    }
                    let w = lg.edges[ei as usize].to;
                    match color[w as usize] {
                        0 => {
                            color[w as usize] = 1;
                            stack.push((w, off[w as usize], ei));
                        }
                        1 => {
                            any = true;
                            let start = stack.iter().position(|s| s.0 == w).unwrap_or(0);
                            let mut cyc: Vec<usize> =
                                stack[start + 1..].iter().map(|s| s.2 as usize).collect();
                            cyc.push(ei as usize);
                            if let Some(&f) = cyc.iter().find(|&&e| e >= first_flow) {
                                kill[f] = true;
                            } else if let Some(&f) = cyc
                                .iter()
                                .filter(|&&e| e < first_nesting)
                                .min_by_key(|&&e| (lg.edges[e].weight, e))
                            {
                                flip[f] = true;
                            }
                        }
                        _ => {}
                    }
                } else {
                    color[v as usize] = 2;
                    stack.pop();
                }
            }
        }
        if !any {
            return;
        }
        let mut i = 0;
        lg.edges.retain(|_| {
            i += 1;
            !kill[i - 1]
        });
        // `flip` indices refer to the old numbering; flipped edges precede every killed
        // one (they are below `first_nesting`), so the numbers are unchanged.
        for (e, f) in lg.edges.iter_mut().zip(&flip) {
            if *f {
                std::mem::swap(&mut e.from, &mut e.to);
                e.reversed = !e.reversed;
            }
        }
    }
}

/// Moves every cluster's top border to just above its first member and the bottom
/// border to just below its last one (deepest clusters first).
pub(crate) fn tighten(lg: &mut LGraph, node_cluster: &[u32]) {
    let k = lg.tree.parent.len();
    if k == 0 {
        return;
    }
    let mut lo = vec![i32::MAX; k];
    let mut hi = vec![i32::MIN; k];
    for (v, &c) in node_cluster.iter().enumerate() {
        if c != NONE {
            let r = lg.nodes[v].rank;
            lo[c as usize] = lo[c as usize].min(r);
            hi[c as usize] = hi[c as usize].max(r);
        }
    }
    let mut order: Vec<u32> = (0..k as u32).collect();
    order.sort_by_key(|&c| (std::cmp::Reverse(lg.tree.depth[c as usize]), c));
    for &c in &order {
        let ci = c as usize;
        let (t, b) = (lg.cl_top[ci] as usize, lg.cl_bot[ci] as usize);
        let (mut rl, mut rh) = (lo[ci], hi[ci]);
        if !lg.cl_leaf[ci] && rl != i32::MAX {
            lg.nodes[t].rank = rl - 1;
            lg.nodes[b].rank = rh + 1;
            rl = rl - 1;
            rh += 1;
        } else if lg.cl_leaf[ci] {
            rl = lg.nodes[t].rank;
            rh = rl;
        }
        let p = lg.tree.parent[ci];
        if p != NONE && rl != i32::MAX {
            lo[p as usize] = lo[p as usize].min(rl);
            hi[p as usize] = hi[p as usize].max(rh);
        }
    }
}

/// Adds one left and one right border node per rank spanned by each cluster.
pub(crate) fn add_side_borders(lg: &mut LGraph) {
    let k = lg.tree.parent.len();
    for c in 0..k {
        if lg.cl_leaf[c] {
            continue;
        }
        let cid = ClusterId(c as u32);
        let (tr, br) = (
            lg.nodes[lg.cl_top[c] as usize].rank,
            lg.nodes[lg.cl_bot[c] as usize].rank,
        );
        let mut prev: Option<(u32, u32)> = None;
        for r in (tr + 1).max(0)..br {
            if r as usize >= lg.ranks.len() {
                break;
            }
            let mut ids = [0u32; 2];
            for (slot, kind) in [LKind::BorderLeft(cid), LKind::BorderRight(cid)]
                .into_iter()
                .enumerate()
            {
                let id = lg.nodes.len() as u32;
                lg.nodes.push(LNode {
                    kind,
                    w: 0.0,
                    h: 0.0,
                    rank: r,
                    order: 0,
                    x: 0.0,
                    y: 0.0,
                    cluster: Some(cid),
                });
                lg.ranks[r as usize].push(id);
                ids[slot] = id;
            }
            lg.cl_left[c].push(ids[0]);
            lg.cl_right[c].push(ids[1]);
            if let Some((pl, pr)) = prev {
                for (a, b) in [(pl, ids[0]), (pr, ids[1])] {
                    lg.edges.push(LEdge {
                        from: a,
                        to: b,
                        orig: NO_EDGE,
                        reversed: false,
                        weight: 8,
                        min_len: 1,
                    });
                }
            }
            prev = Some((ids[0], ids[1]));
        }
    }
}

/// Final-coordinate rectangle of every cluster. The cross extent comes from the side
/// borders; the rank extent hugs the members (title room above, padding below).
pub(crate) fn cluster_rects(lg: &LGraph, frame: &Frame, title_w: &[f32], m: &Metrics) -> Vec<Rect> {
    let k = lg.tree.parent.len();
    let mut lo = vec![f32::INFINITY; k];
    let mut hi = vec![f32::NEG_INFINITY; k];
    for nd in &lg.nodes {
        if let (LKind::Real(_), Some(c)) = (nd.kind, nd.cluster) {
            let ci = c.0 as usize;
            lo[ci] = lo[ci].min(nd.y - nd.h / 2.0);
            hi[ci] = hi[ci].max(nd.y + nd.h / 2.0);
        }
    }
    let mut order: Vec<u32> = (0..k as u32).collect();
    order.sort_by_key(|&c| (std::cmp::Reverse(lg.tree.depth[c as usize]), c));
    let mut span = vec![[0.0f32; 2]; k];
    for &c in &order {
        let ci = c as usize;
        let (a, b) = if lg.cl_leaf[ci] {
            let nd = &lg.nodes[lg.cl_top[ci] as usize];
            (nd.y - nd.h / 2.0, nd.y + nd.h / 2.0)
        } else if lo[ci].is_finite() {
            let tt = match lg.dir {
                crate::graph::Direction::TB => (m.cluster_title, 0.0),
                crate::graph::Direction::BT => (0.0, m.cluster_title),
                _ => (0.0, 0.0),
            };
            (lo[ci] - tt.0 - m.cluster_pad, hi[ci] + tt.1 + m.cluster_pad)
        } else {
            let (t, b) = (
                &lg.nodes[lg.cl_top[ci] as usize],
                &lg.nodes[lg.cl_bot[ci] as usize],
            );
            (t.y, b.y.max(t.y))
        };
        span[ci] = [a, b];
        let p = lg.tree.parent[ci];
        if p != NONE {
            let pi = p as usize;
            lo[pi] = lo[pi].min(a);
            hi[pi] = hi[pi].max(b);
        }
    }
    let mut out = Vec::with_capacity(k);
    for c in 0..k {
        let rank = span[c];
        let cross = if lg.cl_leaf[c] {
            let nd = &lg.nodes[lg.cl_top[c] as usize];
            [nd.x - nd.w / 2.0, nd.x + nd.w / 2.0]
        } else {
            let lx = lg.cl_left[c]
                .iter()
                .map(|&v| lg.nodes[v as usize].x)
                .fold(f32::INFINITY, f32::min);
            let rx = lg.cl_right[c]
                .iter()
                .map(|&v| lg.nodes[v as usize].x)
                .fold(f32::NEG_INFINITY, f32::max);
            if lx.is_finite() && rx.is_finite() {
                [lx, rx.max(lx)]
            } else {
                let t = lg.nodes[lg.cl_top[c] as usize].x;
                [t, t]
            }
        };
        let a = frame.map([cross[0], rank[0]]);
        let b = frame.map([cross[1], rank[1]]);
        let mut r = Rect {
            x: a[0].min(b[0]),
            y: a[1].min(b[1]),
            w: (a[0] - b[0]).abs(),
            h: (a[1] - b[1]).abs(),
        };
        let min_w = title_w.get(c).copied().unwrap_or(0.0) + TITLE_INSET;
        if !lg.cl_leaf[c] && r.w < min_w {
            // Widen symmetrically so the title fits.
            r.x -= (min_w - r.w) / 2.0;
            r.w = min_w;
        }
        out.push(r);
    }
    out
}

/// Moves an attach coordinate `x` (cross axis, range `lo..hi`) off the title text of
/// a cluster whose title sits at the start of that range.
pub(crate) fn dodge_title(x: f32, lo: f32, hi: f32, title_w: f32) -> f32 {
    let zone_end = lo + 12.0 + title_w + 6.0;
    if x >= lo + 4.0 && x <= zone_end && zone_end + 8.0 <= hi - 14.0 {
        zone_end + 8.0
    } else {
        x
    }
}
