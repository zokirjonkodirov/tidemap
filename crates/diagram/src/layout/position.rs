//! Coordinate assignment: rank-axis offsets and Brandes–Köpf cross-axis positions.

use std::sync::atomic::AtomicBool;

use super::par::{self, Par};
use super::{Csr, LGraph, LKind, LayoutError, check_cancel};
use crate::graph::Direction;
use crate::metrics::Metrics;

/// Fills `LNode::x` (cross-axis centre) and `LNode::y` (rank-axis centre).
/// The minimum left edge is translated to 0. Stops with `Canceled` when `cancel` is set.
pub(crate) fn position(
    lg: &mut LGraph,
    m: &Metrics,
    cancel: &AtomicBool,
    par: Par,
) -> Result<(), LayoutError> {
    if lg.nodes.is_empty() {
        return Ok(());
    }
    assign_y(lg, m);
    let (xs, failed) = assign_x(lg, m, cancel, par)?;
    lg.forced_failed = failed;
    let mut min_left = f32::INFINITY;
    for (nd, &x) in lg.nodes.iter().zip(&xs) {
        min_left = min_left.min(x - nd.w / 2.0);
    }
    for (nd, &x) in lg.nodes.iter_mut().zip(&xs) {
        nd.x = x - min_left;
    }
    Ok(())
}

fn assign_y(lg: &mut LGraph, m: &Metrics) {
    let rc = lg.ranks.len();
    let mut hh = vec![0.0f32; rc];
    let mut has_boxy = vec![false; rc];
    let mut has_label = vec![false; rc];
    let mut has_top = vec![false; rc];
    let mut has_bot = vec![false; rc];
    for (r, layer) in lg.ranks.iter().enumerate() {
        for &v in layer {
            let nd = &lg.nodes[v as usize];
            match nd.kind {
                LKind::Dummy(_) | LKind::BorderLeft(_) | LKind::BorderRight(_) => {}
                LKind::Label(_) => {
                    has_label[r] = true;
                    hh[r] = hh[r].max(nd.h / 2.0);
                }
                LKind::BorderTop(c) => {
                    if lg.cl_leaf.get(c.0 as usize).copied().unwrap_or(false) {
                        has_boxy[r] = true;
                        hh[r] = hh[r].max(nd.h / 2.0);
                    } else {
                        has_top[r] = true;
                    }
                }
                LKind::BorderBottom(_) => has_bot[r] = true,
                LKind::Real(_) => {
                    has_boxy[r] = true;
                    hh[r] = hh[r].max(nd.h / 2.0);
                }
            }
        }
    }
    let thin = |r: usize| !has_boxy[r] && !has_label[r] && !has_top[r] && !has_bot[r];
    // Ranks holding only edge labels are packed tighter; ranks holding only cluster
    // borders get the room the border needs.
    let tight = |r: usize| (has_label[r] && !has_boxy[r]) || thin(r);
    let border_only = |r: usize| !has_boxy[r] && !has_label[r] && (has_top[r] || has_bot[r]);
    let edge_room = m.rank_gap * 0.625;
    let mut yc = vec![0.0f32; rc];
    let mut top = 0.0f32;
    for r in 0..rc {
        if r > 0 {
            let p = r - 1;
            let f = if tight(r) || tight(p) { 0.5 } else { 1.0 };
            let base = if !border_only(p) && !border_only(r) {
                m.rank_gap * f
            } else if (border_only(p) && has_top[p]) || (border_only(r) && has_bot[r]) {
                0.0
            } else {
                edge_room
            };
            let top_room = if lg.dir == Direction::TB {
                m.cluster_title + m.cluster_pad
            } else {
                m.cluster_pad
            };
            let bot_room = if lg.dir == Direction::BT {
                m.cluster_title + m.cluster_pad
            } else {
                m.cluster_pad
            };
            let req_top = if has_top[p] { top_room - hh[p] } else { 0.0 };
            let req_bot = if has_bot[r] { bot_room - hh[r] } else { 0.0 };
            // A cluster ending in rank `r` right below a rank that starts another one
            // (members one rank apart) needs both paddings between those member rows.
            let req_both = if has_top[p] && has_bot[r] {
                top_room + bot_room
            } else {
                0.0
            };
            top += base.max(req_top).max(req_bot).max(req_both);
        }
        yc[r] = top + hh[r];
        top += 2.0 * hh[r];
    }
    for (r, layer) in lg.ranks.iter().enumerate() {
        for &v in layer {
            lg.nodes[v as usize].y = yc[r];
        }
    }
}

/// Half of the gap contributed by a node when it sits next to another.
fn half_gap(kind: LKind, m: &Metrics) -> f32 {
    if matches!(kind, LKind::Dummy(_)) {
        m.node_gap * DUMMY_GAP
    } else {
        m.node_gap / 2.0
    }
}

struct Ctx<'a> {
    lg: &'a LGraph,
    preds: Csr<u32>,
    succs: Csr<u32>,
    conflict: Vec<bool>,
    half: Vec<f32>,
    pad: f32,
    /// Extra room before the left border (title sits on the cross-min side).
    left_extra: f32,
    /// Minimum cluster widths as `(left border, right border, width)`.
    widths: Vec<(u32, u32, f32)>,
}

impl Ctx<'_> {
    fn sep(&self, a: u32, b: u32) -> f32 {
        let (na, nb) = (&self.lg.nodes[a as usize], &self.lg.nodes[b as usize]);
        let inner = (na.w + nb.w) / 2.0;
        match (na.kind, nb.kind) {
            (LKind::BorderLeft(_), _) => inner + self.pad + self.left_extra,
            (_, LKind::BorderRight(_)) => inner + self.pad,
            _ => inner + self.half[a as usize] + self.half[b as usize],
        }
    }
}

fn assign_x(
    lg: &LGraph,
    m: &Metrics,
    cancel: &AtomicBool,
    par: Par,
) -> Result<(Vec<f32>, bool), LayoutError> {
    let n = lg.nodes.len();
    let mut p = Vec::with_capacity(lg.edges.len());
    let mut s = Vec::with_capacity(lg.edges.len());
    for (i, e) in lg.edges.iter().enumerate() {
        p.push((e.to, e.from, i as u32));
        s.push((e.from, e.to, i as u32));
    }
    let ctx = Ctx {
        lg,
        preds: Csr::build(n, &p),
        succs: Csr::build(n, &s),
        conflict: vec![false; lg.edges.len()],
        half: lg.nodes.iter().map(|nd| half_gap(nd.kind, m)).collect(),
        pad: m.cluster_pad,
        left_extra: if matches!(lg.dir, Direction::LR | Direction::RL) {
            m.cluster_title
        } else {
            0.0
        },
        widths: min_widths(lg),
    };
    let mut ctx = ctx;
    mark_type1_conflicts(&mut ctx);

    // Four alignments, in the order ul, ur, dl, dr. They only read `ctx`, so they may
    // run concurrently; results stay in this order.
    let workers = par.workers(
        ALIGNMENTS.len(),
        n + lg.edges.len(),
        PAR_ALIGN_WORK_PER_WORKER,
    );
    let aligned = par::map_items(&ALIGNMENTS, workers, |_, &(up, left)| {
        check_cancel(cancel)?;
        Ok(align_one(&ctx, up, left))
    })?;
    let failed = aligned.iter().any(|a| a.1);
    let mut xss: Vec<Vec<f32>> = aligned.into_iter().map(|a| a.0).collect();

    // Align to the narrowest layout, then take the average of the two medians.
    let width = |xs: &[f32]| -> f32 {
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for (nd, &x) in lg.nodes.iter().zip(xs) {
            lo = lo.min(x - nd.w / 2.0);
            hi = hi.max(x + nd.w / 2.0);
        }
        hi - lo
    };
    let mut smallest = 0;
    let mut smallest_w = f32::INFINITY;
    for (i, xs) in xss.iter().enumerate() {
        let w = width(xs);
        if w < smallest_w {
            smallest_w = w;
            smallest = i;
        }
    }
    let (ref_min, ref_max) = min_max(&xss[smallest]);
    for (i, xs) in xss.iter_mut().enumerate() {
        if i == smallest {
            continue;
        }
        let (lo, hi) = min_max(xs);
        // Index order is ul, ur, dl, dr: even = left aligned.
        let delta = if i % 2 == 0 {
            ref_min - lo
        } else {
            ref_max - hi
        };
        for x in xs.iter_mut() {
            *x += delta;
        }
    }
    let mut xs: Vec<f32> = (0..n)
        .map(|v| {
            let mut a = [xss[0][v], xss[1][v], xss[2][v], xss[3][v]];
            a.sort_by(f32::total_cmp);
            (a[1] + a[2]) / 2.0
        })
        .collect();
    // Keep the compacted layout only when it is clearly better: not wider and either
    // much narrower or with much shorter edges (otherwise the straight BK spines win).
    let mut ys = xs.clone();
    refine(&ctx, &mut ys, cancel)?;
    let edge_len = |xs: &[f32]| -> f32 {
        lg.edges
            .iter()
            .map(|e| (xs[e.from as usize] - xs[e.to as usize]).abs())
            .sum()
    };
    let (wx, wy) = (width(&xs), width(&ys));
    if wy <= wx + 0.5 && (wy < wx * 0.9 || edge_len(&ys) < edge_len(&xs) * 0.85) {
        xs = ys;
    }
    Ok((xs, failed))
}

/// The four Brandes-Koepf alignments as `(up, left)`.
const ALIGNMENTS: [(bool, bool); 4] = [(true, true), (true, false), (false, true), (false, false)];

/// One alignment: node x coordinates for the given sweep direction (mirrored back
/// when right-aligned) and whether the straight side borders had to be given up.
fn align_one(ctx: &Ctx<'_>, up: bool, left: bool) -> (Vec<f32>, bool) {
    let lg = ctx.lg;
    let n = lg.nodes.len();
    let mut layers: Vec<Vec<u32>> = lg.ranks.clone();
    if !up {
        layers.reverse();
    }
    if !left {
        for l in &mut layers {
            l.reverse();
        }
    }
    let mut pos = vec![0u32; n];
    for l in &layers {
        for (i, &v) in l.iter().enumerate() {
            pos[v as usize] = i as u32;
        }
    }
    let mut failed = false;
    let (root, _align) = vertical_alignment(ctx, &layers, &pos, up, true);
    let mut xs = match horizontal_compaction(ctx, &layers, &root, left) {
        Some(xs) => xs,
        None => {
            failed = true;
            // Straight side borders crossed another chain: align without forcing them.
            let (root, _) = vertical_alignment(ctx, &layers, &pos, up, false);
            horizontal_compaction(ctx, &layers, &root, left).unwrap_or_else(|| vec![0.0; n])
        }
    };
    if !left {
        for x in &mut xs {
            *x = -*x;
        }
    }
    (xs, failed)
}

/// Minimum width of every cluster box so its title fits.
fn min_widths(lg: &LGraph) -> Vec<(u32, u32, f32)> {
    let mut out = Vec::new();
    if matches!(lg.dir, Direction::LR | Direction::RL) {
        // The title sits on the rank axis there; the cluster rect widening handles it.
        return out;
    }
    for c in 0..lg.cl_left.len() {
        if let (Some(&l), Some(&r)) = (lg.cl_left[c].first(), lg.cl_right[c].first()) {
            let t = lg.title_w.get(c).copied().unwrap_or(0.0);
            out.push((l, r, t + super::compound::TITLE_INSET));
        }
    }
    out
}

/// Edge weights of the compaction (real-real 1, dummy-dummy, real-dummy, intra-cluster).
const W_DUMMY_DUMMY: f32 = 2.0;
const W_REAL_DUMMY: f32 = 2.0;
const W_INTRA: f32 = 3.0;
/// Extra weight of edges that are already straight in the BK layout.
const STRAIGHT_BONUS: f32 = 4.0;
/// Pull of a node towards the middle of its cluster.
const CLUSTER_SPRING: f32 = 2.0;
/// Pull of whole clusters towards the middle of the layout.
const CENTRE_PULL: f32 = 20.0;
/// Clearance (times `node_gap`) on each side of an edge dummy.
const DUMMY_GAP: f32 = 0.25;
/// Work (nodes plus edges) one thread needs before the four alignments run concurrently.
/// Measured on a 28-thread desktop CPU (best of 15/5): on trees of 500-4,000 nodes (about
/// 2,000-16,000 units) concurrent was 0-15% slower or equal; random layered graphs gained
/// 3% at 4,000 nodes (about 13,000) and 14% at 8,000 nodes (about 40,000 units).
const PAR_ALIGN_WORK_PER_WORKER: usize = 8_000;
/// `refine` looks at the cancel flag once per this many sweeps.
const CANCEL_EVERY_SWEEPS: usize = 4;

/// Weighted least-squares compaction: every block (a node, or the straight side
/// border column of a cluster) moves to the weighted mean of its neighbours inside
/// the interval its rank neighbours leave free. Keeps the BK layout feasible.
fn refine(ctx: &Ctx<'_>, xs: &mut [f32], cancel: &AtomicBool) -> Result<(), LayoutError> {
    let sweeps = (500_000 / (xs.len() + ctx.lg.edges.len()).max(1)).clamp(3, 60);
    let Some(mut st) = Refiner::new(ctx, xs) else {
        return Ok(());
    };
    let mut before = xs.to_vec();
    let mut quiet = 0u8;
    for sweep in 0..sweeps {
        if sweep % CANCEL_EVERY_SWEEPS == 0 {
            check_cancel(cancel)?;
        }
        before.copy_from_slice(xs);
        st.sweep(xs, sweep % 2 == 0);
        // A sweep is a pure function of `xs` and its direction (the scratch buffers
        // are rewritten before they are read). Two unchanged sweeps in a row, one per
        // direction, are therefore fixed points of both: every later sweep is a no-op.
        quiet = if same_bits(&before, xs) { quiet + 1 } else { 0 };
        if quiet == 2 {
            break;
        }
    }
    st.finish(xs);
    Ok(())
}

/// Exact (bit-for-bit) equality of two coordinate slices; no tolerance.
fn same_bits(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}

/// Constant state of one `refine` call plus the scratch buffers its sweeps reuse.
struct Refiner<'a, 'c> {
    ctx: &'a Ctx<'c>,
    /// Block of every node: the head of its side-border column, else the node itself.
    bid: Vec<u32>,
    /// Per block head: 1 = left column, 2 = right column, 0 = free node.
    side: Vec<u8>,
    /// Separation constraints between blocks, incoming and outgoing.
    cin: Csr<f32>,
    cout: Csr<f32>,
    /// Weighted neighbours per node.
    adj: Csr<f32>,
    /// Nodes of every cluster (nested ones included) for rigid cluster moves.
    members: Vec<Vec<u32>>,
    /// Cluster ids, deepest first.
    corder: Vec<usize>,
    stamp: Vec<u32>,
    /// Rank bands that can slide as a whole (no cluster column crosses their borders).
    bands: Vec<(usize, usize)>,
    in_band: Vec<u32>,
}

/// Block id and side marker of every node (see [`Refiner`]).
fn block_ids(lg: &LGraph) -> (Vec<u32>, Vec<u8>) {
    let n = lg.nodes.len();
    let mut bid: Vec<u32> = (0..n as u32).collect();
    let mut side = vec![0u8; n];
    for (sd, cols) in [(1u8, &lg.cl_left), (2u8, &lg.cl_right)] {
        for chain in cols {
            if let Some(&head) = chain.first() {
                for &v in chain {
                    bid[v as usize] = head;
                }
                side[head as usize] = sd;
            }
        }
    }
    (bid, side)
}

/// Separation constraints between blocks as `(incoming, outgoing)` triples.
type Constraints = Vec<(u32, u32, f32)>;

fn block_constraints(ctx: &Ctx<'_>, bid: &[u32]) -> (Constraints, Constraints) {
    let mut cin: Constraints = Vec::new();
    let mut cout: Constraints = Vec::new();
    let mut add = |a: u32, b: u32, d: f32| {
        if a != b {
            cin.push((b, a, d));
            cout.push((a, b, d));
        }
    };
    for layer in &ctx.lg.ranks {
        for w in layer.windows(2) {
            add(bid[w[0] as usize], bid[w[1] as usize], ctx.sep(w[0], w[1]));
        }
    }
    for &(l, r, d) in &ctx.widths {
        add(bid[l as usize], bid[r as usize], d);
    }
    (cin, cout)
}

/// True when the input keeps straight blocks straight and respects every separation.
fn is_feasible(xs: &[f32], bid: &[u32], cout: &[(u32, u32, f32)]) -> bool {
    for (v, &x) in xs.iter().enumerate() {
        if (x - xs[bid[v] as usize]).abs() > 0.01 {
            return false;
        }
    }
    // Written as `any` (not `all(>=)`) so a NaN gap counts as feasible, as before.
    !cout
        .iter()
        .any(|&(a, b, d)| xs[b as usize] - xs[a as usize] < d - 0.01)
}

/// Pull class of a node: 0 = real, 1 = edge part, 2 = cluster border.
fn node_class(lg: &LGraph, v: u32) -> u8 {
    match lg.nodes[v as usize].kind {
        LKind::Real(_) => 0,
        LKind::BorderTop(c) if lg.cl_leaf[c.0 as usize] => 0,
        LKind::Dummy(_) | LKind::Label(_) => 1,
        _ => 2,
    }
}

/// Weighted attraction between connected nodes, both directions. Edges that are
/// already straight in `xs` weigh more.
fn weighted_edges(lg: &LGraph, bid: &[u32], xs: &[f32]) -> Vec<(u32, u32, f32)> {
    let (w_dd, w_rd, w_intra) = (W_DUMMY_DUMMY, W_REAL_DUMMY, W_INTRA);
    let mut adj: Vec<(u32, u32, f32)> = Vec::new();
    for e in &lg.edges {
        let (a, b) = (node_class(lg, e.from), node_class(lg, e.to));
        if a == 2 || b == 2 || bid[e.from as usize] == bid[e.to as usize] {
            continue;
        }
        let intra = lg.nodes[e.from as usize].cluster == lg.nodes[e.to as usize].cluster
            && lg.nodes[e.from as usize].cluster.is_some();
        let w = match (a, b) {
            (0, 0) => 1.0 * if intra { w_intra } else { 1.0 },
            (1, 1) => w_dd,
            _ => w_rd,
        } * if e.reversed { 0.5 } else { 1.0 }
            * if (xs[e.from as usize] - xs[e.to as usize]).abs() < 0.5 {
                STRAIGHT_BONUS
            } else {
                1.0
            };
        adj.push((e.from, e.to, w));
        adj.push((e.to, e.from, w));
    }
    adj
}

/// Nodes of every cluster, nested clusters' nodes included.
fn cluster_members(lg: &LGraph) -> Vec<Vec<u32>> {
    let kc = lg.tree.parent.len();
    let mut members: Vec<Vec<u32>> = vec![Vec::new(); kc];
    for (v, nd) in lg.nodes.iter().enumerate() {
        let mut c = nd.cluster.map_or(super::compound::NONE, |c| c.0);
        let mut guard = 0;
        while c != super::compound::NONE && guard < 400 {
            members[c as usize].push(v as u32);
            c = lg.tree.parent[c as usize];
            guard += 1;
        }
    }
    members
}

/// Rank bands that can slide as a whole: no cluster column crosses their borders.
fn rank_bands(lg: &LGraph) -> Vec<(usize, usize)> {
    let nr_all = lg.ranks.len();
    let mut cut_bad = vec![false; nr_all];
    for chain in &lg.cl_left {
        let (mut lo, mut hi) = (i32::MAX, i32::MIN);
        for &v in chain {
            let r = lg.nodes[v as usize].rank;
            lo = lo.min(r);
            hi = hi.max(r);
        }
        for r in lo.max(0)..hi.max(0) {
            if (r as usize) < nr_all {
                cut_bad[r as usize] = true;
            }
        }
    }
    let mut bands: Vec<(usize, usize)> = Vec::new();
    let mut start = 0usize;
    for r in 0..nr_all {
        if r + 1 == nr_all || !cut_bad[r] {
            bands.push((start, r));
            start = r + 1;
        }
    }
    bands
}

/// Mean x of the two side columns of cluster `c`, if it has them.
fn cluster_centre(lg: &LGraph, xs: &[f32], c: usize) -> Option<f32> {
    let (l, r) = (lg.cl_left[c].first()?, lg.cl_right[c].first()?);
    Some((xs[*l as usize] + xs[*r as usize]) / 2.0)
}

impl<'a, 'c> Refiner<'a, 'c> {
    /// `None` when `xs` is not a feasible starting layout (nothing to refine).
    fn new(ctx: &'a Ctx<'c>, xs: &[f32]) -> Option<Self> {
        let lg = ctx.lg;
        let n = lg.nodes.len();
        let (bid, side) = block_ids(lg);
        let (cin, cout) = block_constraints(ctx, &bid);
        if !is_feasible(xs, &bid, &cout) {
            return None;
        }
        let kc = lg.tree.parent.len();
        let mut corder: Vec<usize> = (0..kc).collect();
        corder.sort_by_key(|&c| (std::cmp::Reverse(lg.tree.depth[c]), c));
        Some(Self {
            ctx,
            cin: Csr::build(n, &cin),
            cout: Csr::build(n, &cout),
            adj: Csr::build(n, &weighted_edges(lg, &bid, xs)),
            members: cluster_members(lg),
            corder,
            stamp: vec![u32::MAX; n],
            bands: rank_bands(lg),
            in_band: vec![u32::MAX; n],
            bid,
            side,
        })
    }

    /// One sweep: slide bands, fit every rank (top-down when `down`), move clusters.
    fn sweep(&mut self, xs: &mut [f32], down: bool) {
        self.slide_bands(xs);
        self.fit_ranks(xs, down);
        self.move_clusters(xs);
    }

    /// Writes every node's block coordinate back to the node.
    fn finish(&self, xs: &mut [f32]) {
        for v in 0..self.bid.len() {
            xs[v] = xs[self.bid[v] as usize];
        }
    }

    /// Slides each rank band by the mean pull of its outside neighbours.
    fn slide_bands(&mut self, xs: &mut [f32]) {
        let lg = self.ctx.lg;
        for (bi, &(r0, r1)) in self.bands.iter().enumerate() {
            for r in r0..=r1 {
                for &v in &lg.ranks[r] {
                    self.in_band[v as usize] = bi as u32;
                }
            }
            let (mut f, mut wsum) = (0.0f32, 0.0f32);
            for r in r0..=r1 {
                for &v in &lg.ranks[r] {
                    for &(o, w) in self.adj.row(v) {
                        if self.in_band[o as usize] != bi as u32 {
                            let dx = xs[self.bid[o as usize] as usize]
                                - xs[self.bid[v as usize] as usize];
                            f += w * dx;
                            wsum += w;
                        }
                    }
                }
            }
            if wsum > 0.0 && (f / wsum).abs() > 1e-3 {
                let delta = f / wsum;
                for r in r0..=r1 {
                    for &v in &lg.ranks[r] {
                        xs[v as usize] += delta;
                    }
                }
            }
        }
    }

    /// Fits every rank in sweep order.
    fn fit_ranks(&self, xs: &mut [f32], down: bool) {
        let lg = self.ctx.lg;
        let nr = lg.ranks.len();
        let mut offs: Vec<f32> = Vec::new();
        let mut stack: Vec<(f32, f32, u32)> = Vec::new();
        for ri in 0..nr {
            let r = if down { ri } else { nr - 1 - ri };
            let layer = &lg.ranks[r];
            self.hug_side_columns(layer, xs);
            // Free nodes between side columns: weighted isotonic regression.
            let len = layer.len();
            let mut i = 0;
            while i < len {
                if self.side[self.bid[layer[i] as usize] as usize] != 0 {
                    i += 1;
                    continue;
                }
                let mut j = i;
                while j + 1 < len && self.side[self.bid[layer[j + 1] as usize] as usize] == 0 {
                    j += 1;
                }
                self.fit_run(layer, (i, j), xs, &mut offs, &mut stack);
                i = j + 1;
            }
        }
    }

    /// Side columns hug the cluster contents.
    fn hug_side_columns(&self, layer: &[u32], xs: &mut [f32]) {
        for &v in layer {
            let b = self.bid[v as usize];
            let sd = self.side[b as usize];
            if sd == 0 {
                continue;
            }
            let (mut lo, mut hi) = (f32::NEG_INFINITY, f32::INFINITY);
            for &(a, d) in self.cin.row(b) {
                lo = lo.max(xs[a as usize] + d);
            }
            for &(c, d) in self.cout.row(b) {
                hi = hi.min(xs[c as usize] - d);
            }
            if sd == 1 && hi.is_finite() {
                xs[b as usize] = hi;
            } else if sd == 2 && lo.is_finite() {
                xs[b as usize] = lo;
            }
        }
    }

    /// Target position and weight of free node `v`: its neighbours plus the pull
    /// towards the middle of its cluster.
    fn target(&self, v: u32, xs: &[f32]) -> (f32, f32) {
        let lg = self.ctx.lg;
        let (mut num, mut den) = (0.0f32, 0.0f32);
        for &(o, w) in self.adj.row(v) {
            let xo = xs[self.bid[o as usize] as usize];
            num += w * xo;
            den += w;
        }
        let nd = &lg.nodes[v as usize];
        let sc = match nd.kind {
            LKind::BorderTop(c) | LKind::BorderBottom(c) if !lg.cl_leaf[c.0 as usize] => {
                Some((c.0 as usize, 1.0))
            }
            LKind::Real(_) | LKind::BorderTop(_) => {
                nd.cluster.map(|c| (c.0 as usize, CLUSTER_SPRING))
            }
            _ => None,
        };
        if let Some((c, w)) = sc
            && let Some(cx) = cluster_centre(lg, xs, c)
        {
            num += w * cx;
            den += w;
        }
        if den > 0.0 {
            (num / den, den)
        } else {
            (xs[v as usize], 1e-3)
        }
    }

    /// Fits the free nodes `layer[i..=j]` (no side column among them) by pool-adjacent-
    /// violators regression, kept inside the room the neighbouring blocks leave.
    fn fit_run(
        &self,
        layer: &[u32],
        (i, j): (usize, usize),
        xs: &mut [f32],
        offs: &mut Vec<f32>,
        stack: &mut Vec<(f32, f32, u32)>,
    ) {
        let ctx = self.ctx;
        let len = layer.len();
        offs.clear();
        stack.clear();
        let mut off = 0.0f32;
        for k in i..=j {
            let v = layer[k];
            if k > i {
                off += ctx.sep(layer[k - 1], v);
            }
            offs.push(off);
            let (t, w) = self.target(v, xs);
            // Pool adjacent violators on z = x - offset.
            stack.push((w * (t - off), w, 1u32));
            while stack.len() >= 2 {
                let (a, b) = (stack[stack.len() - 2], stack[stack.len() - 1]);
                if a.0 / a.1 > b.0 / b.1 {
                    stack.pop();
                    let top = stack.len() - 1;
                    stack[top] = (a.0 + b.0, a.1 + b.1, a.2 + b.2);
                } else {
                    break;
                }
            }
        }
        let mut zlo = f32::NEG_INFINITY;
        let mut zhi = f32::INFINITY;
        if i > 0 {
            let f = layer[i - 1];
            zlo = xs[self.bid[f as usize] as usize] + ctx.sep(f, layer[i]);
        }
        if j + 1 < len {
            let f = layer[j + 1];
            zhi = xs[self.bid[f as usize] as usize] - ctx.sep(layer[j], f) - offs[j - i];
        }
        if zlo <= zhi {
            let mut k = i;
            for &(s, w, c) in stack.iter() {
                let z = (s / w).clamp(zlo, zhi);
                for _ in 0..c {
                    xs[layer[k] as usize] = z + offs[k - i];
                    k += 1;
                }
            }
        }
    }

    /// Moves whole clusters (deepest first) by the mean pull of their outside links.
    fn move_clusters(&mut self, xs: &mut [f32]) {
        let (gl, gh) = min_max(xs);
        let gc = (gl + gh) / 2.0;
        for k in 0..self.corder.len() {
            let c = self.corder[k];
            self.move_cluster(c, gc, xs);
        }
    }

    fn move_cluster(&mut self, c: usize, gc: f32, xs: &mut [f32]) {
        let lg = self.ctx.lg;
        let set = &self.members[c];
        for &v in set {
            self.stamp[v as usize] = c as u32;
        }
        let (mut dlo, mut dhi) = (f32::NEG_INFINITY, f32::INFINITY);
        let (mut f, mut wsum) = (0.0f32, 0.0f32);
        for &v in set {
            let vi = v as usize;
            if self.bid[vi] == v {
                for &(a, d) in self.cin.row(v) {
                    if self.stamp[a as usize] != c as u32 {
                        dlo = dlo.max(xs[a as usize] + d - xs[vi]);
                    }
                }
                for &(b, d) in self.cout.row(v) {
                    if self.stamp[b as usize] != c as u32 {
                        dhi = dhi.min(xs[b as usize] - d - xs[vi]);
                    }
                }
            }
            for &(o, w) in self.adj.row(v) {
                if self.stamp[o as usize] != c as u32 {
                    let dx = xs[self.bid[o as usize] as usize] - xs[self.bid[vi] as usize];
                    f += w * dx;
                    wsum += w;
                }
            }
        }
        if let Some(cx) = cluster_centre(lg, xs, c) {
            f += CENTRE_PULL * (gc - cx);
            wsum += CENTRE_PULL;
        }
        if wsum > 0.0 && dlo <= dhi {
            let delta = (f / wsum).clamp(dlo, dhi);
            if delta.abs() > 1e-3 {
                for &v in set {
                    xs[v as usize] += delta;
                }
            }
        }
    }
}

fn min_max(xs: &[f32]) -> (f32, f32) {
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for &x in xs {
        lo = lo.min(x);
        hi = hi.max(x);
    }
    (lo, hi)
}

/// Marks non-inner segments that cross inner (dummy-dummy) segments.
fn mark_type1_conflicts(ctx: &mut Ctx<'_>) {
    let lg = ctx.lg;
    let mut pos = vec![0u32; lg.nodes.len()];
    for l in &lg.ranks {
        for (i, &v) in l.iter().enumerate() {
            pos[v as usize] = i as u32;
        }
    }
    let is_dummy = |v: u32| lg.nodes[v as usize].kind.is_inner_part();
    for i in 1..lg.ranks.len() {
        let prev_len = lg.ranks[i - 1].len() as u32;
        let layer = &lg.ranks[i];
        let mut k0 = 0u32;
        let mut scan = 0usize;
        for (idx, &v) in layer.iter().enumerate() {
            let other = if is_dummy(v) {
                ctx.preds
                    .row(v)
                    .iter()
                    .map(|&(u, _)| u)
                    .find(|&u| is_dummy(u))
            } else {
                None
            };
            let k1 = other.map_or(prev_len, |w| pos[w as usize]);
            if other.is_some() || idx + 1 == layer.len() {
                for &s in &layer[scan..=idx] {
                    for &(u, eid) in ctx.preds.row(s) {
                        let up = pos[u as usize];
                        if (up < k0 || k1 < up) && !(is_dummy(u) && is_dummy(s)) {
                            ctx.conflict[eid as usize] = true;
                        }
                    }
                }
                scan = idx + 1;
                k0 = k1;
            }
        }
    }
}

/// Aligns each node with a median neighbour of the previously processed layer.
fn vertical_alignment(
    ctx: &Ctx<'_>,
    layers: &[Vec<u32>],
    pos: &[u32],
    up: bool,
    forced: bool,
) -> (Vec<u32>, Vec<u32>) {
    let n = ctx.lg.nodes.len();
    let mut root: Vec<u32> = (0..n as u32).collect();
    let mut align: Vec<u32> = (0..n as u32).collect();
    let mut ws: Vec<(u32, u32, u32)> = Vec::new();
    // Side borders of a cluster form one straight block each.
    for chains in ctx.lg.cl_left.iter().chain(&ctx.lg.cl_right) {
        if !forced {
            break;
        }
        if let Some(&head) = chains.first() {
            for &v in chains {
                root[v as usize] = head;
                align[v as usize] = head;
            }
        }
    }
    // Forced column segments: for every column node its partner in the previously
    // processed layer, used by the crossing test below.
    let mut col_pred = vec![u32::MAX; n];
    if forced {
        for chains in ctx.lg.cl_left.iter().chain(&ctx.lg.cl_right) {
            for (i, &v) in chains.iter().enumerate() {
                let p = if up { i.checked_sub(1) } else { Some(i + 1) };
                if let Some(&w) = p.and_then(|p| chains.get(p)) {
                    col_pred[v as usize] = w;
                }
            }
        }
    }
    let column_pred = |v: u32| -> Option<u32> {
        let w = col_pred[v as usize];
        (w != u32::MAX).then_some(w)
    };
    let mut right_bound: Vec<i64> = Vec::new();
    for layer in layers {
        let mut prev_idx: i64 = -1;
        // Per layer position: position of the partner of the next forced column node at
        // or right of it (so ordinary alignments cannot cross the column).
        right_bound.clear();
        right_bound.resize(layer.len() + 1, i64::MAX);
        if forced {
            for i in (0..layer.len()).rev() {
                right_bound[i] = match column_pred(layer[i]) {
                    Some(w) => i64::from(pos[w as usize]),
                    None => right_bound[i + 1],
                };
            }
        }
        for (li, &v) in layer.iter().enumerate() {
            if let Some(w) = column_pred(v) {
                prev_idx = prev_idx.max(i64::from(pos[w as usize]));
                continue;
            }
            let hi_bound = right_bound[li];
            let row = if up {
                ctx.preds.row(v)
            } else {
                ctx.succs.row(v)
            };
            if row.is_empty() {
                continue;
            }
            ws.clear();
            // Ignore neighbours reached through cycle-reversed edges when an ordinary one exists.
            let ordinary = row.iter().any(|&(_, e)| !ctx.lg.edges[e as usize].reversed);
            ws.extend(
                row.iter()
                    .filter(|&&(_, e)| !ordinary || !ctx.lg.edges[e as usize].reversed)
                    .map(|&(w, e)| (pos[w as usize], w, e)),
            );
            ws.sort_unstable();
            let k = ws.len();
            let (lo, hi) = ((k - 1) / 2, k / 2);
            for i in lo..=hi {
                let (wp, w, e) = ws[i];
                if align[v as usize] == v
                    && prev_idx < i64::from(wp)
                    && i64::from(wp) < hi_bound
                    && !ctx.conflict[e as usize]
                {
                    align[w as usize] = v;
                    root[v as usize] = root[w as usize];
                    align[v as usize] = root[v as usize];
                    prev_idx = i64::from(wp);
                }
            }
        }
    }
    (root, align)
}

/// Places blocks at their leftmost feasible coordinates (longest path in the block graph).
fn horizontal_compaction(
    ctx: &Ctx<'_>,
    layers: &[Vec<u32>],
    root: &[u32],
    left: bool,
) -> Option<Vec<f32>> {
    let n = ctx.lg.nodes.len();
    let mut edges: Vec<(u32, u32, f32)> = Vec::new();
    for layer in layers {
        for w in layer.windows(2) {
            let (u, v) = (w[0], w[1]);
            let d = if left { ctx.sep(u, v) } else { ctx.sep(v, u) };
            edges.push((root[u as usize], root[v as usize], d));
        }
    }
    for &(l, r, d) in &ctx.widths {
        let (a, b) = (root[l as usize], root[r as usize]);
        if a != b {
            edges.push(if left { (a, b, d) } else { (b, a, d) });
        }
    }
    let mut indeg = vec![0u32; n];
    for e in &edges {
        indeg[e.1 as usize] += 1;
    }
    let out = Csr::build(n, &edges);
    let mut topo: Vec<u32> = (0..n as u32)
        .filter(|&v| root[v as usize] == v && indeg[v as usize] == 0)
        .collect();
    let mut head = 0;
    while head < topo.len() {
        let v = topo[head];
        head += 1;
        for &(w, _) in out.row(v) {
            indeg[w as usize] -= 1;
            if indeg[w as usize] == 0 {
                topo.push(w);
            }
        }
    }
    let roots = (0..n).filter(|&v| root[v] as usize == v).count();
    if topo.len() < roots {
        return None;
    }
    let mut xs = vec![0.0f32; n];
    for &v in &topo {
        let xv = xs[v as usize];
        for &(w, sep) in out.row(v) {
            if xs[w as usize] < xv + sep {
                xs[w as usize] = xv + sep;
            }
        }
    }
    for v in 0..n {
        xs[v] = xs[root[v] as usize];
    }
    Some(xs)
}
