//! Ranking (longest path + network simplex) and dummy-node insertion.

use std::sync::atomic::AtomicBool;

use super::compound::NONE as NO_CLUSTER;
use super::{LGraph, LKind, LNode, LayoutError, check_cancel};
use crate::graph::{ClusterId, Graph};

/// Above this node count only longest-path ranking plus a pull pass is used.
const SIMPLEX_NODE_LIMIT: usize = 2_000;
const NONE: u32 = u32::MAX;
/// Hard cap on simplex pivots (each pivot is O(n + m)); keeps big graphs fast.
const MAX_PIVOTS: usize = 400;

/// Assigns `LNode::rank` (normalised to start at 0 in every connected component).
pub(crate) fn assign_ranks(lg: &mut LGraph, cancel: &AtomicBool) -> Result<(), LayoutError> {
    let n = lg.nodes.len();
    let mut rank = longest_path(lg);
    check_cancel(cancel)?;
    if n <= SIMPLEX_NODE_LIMIT {
        simplex_components(lg, &mut rank, cancel)?;
        balance(lg, &mut rank);
    } else {
        pull_sources(lg, &mut rank);
    }
    for (nd, r) in lg.nodes.iter_mut().zip(rank) {
        nd.rank = r;
    }
    Ok(())
}

/// Longest-path ranking over the (acyclic) edge set.
fn longest_path(lg: &LGraph) -> Vec<i32> {
    let n = lg.nodes.len();
    let mut indeg = vec![0u32; n];
    let mut off = vec![0usize; n + 1];
    for e in &lg.edges {
        indeg[e.to as usize] += 1;
        off[e.from as usize + 1] += 1;
    }
    for i in 0..n {
        off[i + 1] += off[i];
    }
    let mut fill = off.clone();
    let mut adj = vec![0u32; lg.edges.len()];
    for (i, e) in lg.edges.iter().enumerate() {
        adj[fill[e.from as usize]] = i as u32;
        fill[e.from as usize] += 1;
    }
    let mut rank = vec![0i32; n];
    let mut queue: Vec<u32> = (0..n as u32).filter(|&v| indeg[v as usize] == 0).collect();
    let mut head = 0;
    while head < queue.len() {
        let v = queue[head] as usize;
        head += 1;
        for &ei in &adj[off[v]..off[v + 1]] {
            let e = &lg.edges[ei as usize];
            let t = e.to as usize;
            rank[t] = rank[t].max(rank[v] + e.min_len as i32);
            indeg[t] -= 1;
            if indeg[t] == 0 {
                queue.push(t as u32);
            }
        }
    }
    rank
}

/// Pulls every source toward its nearest successor (keeps edges short for big graphs).
fn pull_sources(lg: &LGraph, rank: &mut [i32]) {
    let n = lg.nodes.len();
    let mut has_in = vec![false; n];
    let mut best = vec![i32::MAX; n];
    for e in &lg.edges {
        has_in[e.to as usize] = true;
    }
    for e in &lg.edges {
        let v = e.from as usize;
        best[v] = best[v].min(rank[e.to as usize] - e.min_len as i32);
    }
    for v in 0..n {
        if !has_in[v] && best[v] != i32::MAX && best[v] > rank[v] {
            rank[v] = best[v];
        }
    }
}

/// Moves nodes with equal in/out weight to the least crowded feasible rank.
fn balance(lg: &LGraph, rank: &mut [i32]) {
    let n = lg.nodes.len();
    let max_rank = rank.iter().copied().max().unwrap_or(0).max(0) as usize;
    let mut count = vec![0u32; max_rank + 1];
    for &r in rank.iter() {
        count[r as usize] += 1;
    }
    let mut in_w = vec![0i64; n];
    let mut out_w = vec![0i64; n];
    // Feasible range depends on neighbours' current ranks, so recompute per node.
    let mut ins: Vec<Vec<(u32, i32)>> = vec![Vec::new(); n];
    let mut outs: Vec<Vec<(u32, i32)>> = vec![Vec::new(); n];
    for e in &lg.edges {
        let w = i64::from(e.weight);
        out_w[e.from as usize] += w;
        in_w[e.to as usize] += w;
        outs[e.from as usize].push((e.to, e.min_len as i32));
        ins[e.to as usize].push((e.from, e.min_len as i32));
    }
    for v in 0..n {
        if in_w[v] == 0 || in_w[v] != out_w[v] {
            continue;
        }
        let lo = ins[v]
            .iter()
            .map(|&(u, l)| rank[u as usize] + l)
            .max()
            .unwrap_or(i32::MIN);
        let hi = outs[v]
            .iter()
            .map(|&(w, l)| rank[w as usize] - l)
            .min()
            .unwrap_or(i32::MAX);
        if lo >= hi {
            continue;
        }
        let cur = rank[v];
        let mut best = cur;
        let mut best_count = count[cur as usize];
        for r in lo..=hi {
            if r >= 0 && (r as usize) < count.len() && count[r as usize] + 1 < best_count {
                best = r;
                best_count = count[r as usize] + 1;
            }
        }
        if best != cur {
            count[cur as usize] -= 1;
            count[best as usize] += 1;
            rank[v] = best;
        }
    }
}

/// Runs network simplex on every connected component and normalises ranks to 0.
fn simplex_components(
    lg: &LGraph,
    rank: &mut [i32],
    cancel: &AtomicBool,
) -> Result<(), LayoutError> {
    let n = lg.nodes.len();
    // Components via union-find.
    let mut parent: Vec<u32> = (0..n as u32).collect();
    fn find(p: &mut [u32], mut v: u32) -> u32 {
        while p[v as usize] != v {
            p[v as usize] = p[p[v as usize] as usize];
            v = p[v as usize];
        }
        v
    }
    for e in &lg.edges {
        let a = find(&mut parent, e.from);
        let b = find(&mut parent, e.to);
        if a != b {
            let (lo, hi) = (a.min(b), a.max(b));
            parent[hi as usize] = lo;
        }
    }
    let mut comp_of = vec![0u32; n];
    for v in 0..n {
        comp_of[v] = find(&mut parent, v as u32);
    }
    let mut local = vec![0u32; n];
    let mut sizes = vec![0u32; n];
    for v in 0..n {
        let c = comp_of[v] as usize;
        local[v] = sizes[c];
        sizes[c] += 1;
    }
    let mut comp_edges: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (i, e) in lg.edges.iter().enumerate() {
        comp_edges[comp_of[e.from as usize] as usize].push(i as u32);
    }
    let mut comp_nodes: Vec<Vec<u32>> = vec![Vec::new(); n];
    for v in 0..n {
        comp_nodes[comp_of[v] as usize].push(v as u32);
    }
    for c in 0..n {
        if comp_nodes[c].len() < 2 {
            continue;
        }
        check_cancel(cancel)?;
        let nodes = &comp_nodes[c];
        let mut ns = Simplex::new(nodes.len());
        for &ei in &comp_edges[c] {
            let e = &lg.edges[ei as usize];
            ns.add_edge(
                local[e.from as usize],
                local[e.to as usize],
                i64::from(e.weight),
                e.min_len as i32,
            );
        }
        for (k, &v) in nodes.iter().enumerate() {
            ns.rank[k] = rank[v as usize];
        }
        ns.run(cancel)?;
        let min = ns.rank.iter().copied().min().unwrap_or(0);
        for (k, &v) in nodes.iter().enumerate() {
            rank[v as usize] = ns.rank[k] - min;
        }
    }
    Ok(())
}

/// Network simplex state for one connected component (Gansner et al.).
#[derive(Clone)]
struct Simplex {
    n: usize,
    eu: Vec<u32>,
    ev: Vec<u32>,
    ew: Vec<i64>,
    el: Vec<i32>,
    inc: Vec<Vec<u32>>,
    rank: Vec<i32>,
    in_tree: Vec<bool>,
    tadj: Vec<Vec<u32>>,
    cut: Vec<i64>,
    low: Vec<u32>,
    lim: Vec<u32>,
    par_edge: Vec<u32>,
    post: Vec<u32>,
}

impl Simplex {
    fn new(n: usize) -> Self {
        Self {
            n,
            eu: Vec::new(),
            ev: Vec::new(),
            ew: Vec::new(),
            el: Vec::new(),
            inc: vec![Vec::new(); n],
            rank: vec![0; n],
            in_tree: Vec::new(),
            tadj: vec![Vec::new(); n],
            cut: Vec::new(),
            low: vec![0; n],
            lim: vec![0; n],
            par_edge: vec![NONE; n],
            post: Vec::new(),
        }
    }

    fn add_edge(&mut self, u: u32, v: u32, w: i64, l: i32) {
        let id = self.eu.len() as u32;
        self.eu.push(u);
        self.ev.push(v);
        self.ew.push(w);
        self.el.push(l);
        self.inc[u as usize].push(id);
        self.inc[v as usize].push(id);
        self.in_tree.push(false);
        self.cut.push(0);
    }

    fn slack(&self, e: usize) -> i32 {
        self.rank[self.ev[e] as usize] - self.rank[self.eu[e] as usize] - self.el[e]
    }

    fn other(&self, e: u32, v: u32) -> u32 {
        if self.eu[e as usize] == v {
            self.ev[e as usize]
        } else {
            self.eu[e as usize]
        }
    }

    fn run(&mut self, cancel: &AtomicBool) -> Result<(), LayoutError> {
        if self.eu.is_empty() {
            return Ok(());
        }
        self.feasible_tree();
        self.number_tree();
        self.compute_cuts();
        let max_pivots = (4 * self.n).min(MAX_PIVOTS);
        let m = self.eu.len();
        let mut start = 0usize;
        for pivot in 0..max_pivots {
            if pivot % 16 == 0 {
                check_cancel(cancel)?;
            }
            // Leave edge: cyclic search for a negative cut value.
            let mut leave = None;
            for k in 0..m {
                let e = (start + k) % m;
                if self.in_tree[e] && self.cut[e] < 0 {
                    leave = Some(e);
                    start = e + 1;
                    break;
                }
            }
            let Some(leave) = leave else { break };
            let Some(enter) = self.enter_edge(leave) else {
                break;
            };
            self.exchange(leave, enter);
        }
        Ok(())
    }

    /// Grows a tight spanning tree, shifting the tree to make more edges tight.
    fn feasible_tree(&mut self) {
        let n = self.n;
        let mut in_node = vec![false; n];
        let mut count = 1usize;
        in_node[0] = true;
        let mut stack: Vec<u32> = vec![0];
        loop {
            // Expand over tight edges.
            while let Some(v) = stack.pop() {
                for k in 0..self.inc[v as usize].len() {
                    let e = self.inc[v as usize][k];
                    let w = self.other(e, v);
                    if !in_node[w as usize] && self.slack(e as usize) == 0 {
                        in_node[w as usize] = true;
                        count += 1;
                        self.in_tree[e as usize] = true;
                        self.tadj[v as usize].push(e);
                        self.tadj[w as usize].push(e);
                        stack.push(w);
                    }
                }
            }
            if count >= n {
                break;
            }
            // Cheapest edge joining the tree to the rest.
            let mut best = i32::MAX;
            let mut cands: Vec<u32> = Vec::new();
            for e in 0..self.eu.len() {
                let a = in_node[self.eu[e] as usize];
                let b = in_node[self.ev[e] as usize];
                if a != b {
                    let s = self.slack(e);
                    if s < best {
                        best = s;
                        cands.clear();
                        cands.push(e as u32);
                    } else if s == best {
                        cands.push(e as u32);
                    }
                }
            }
            if cands.is_empty() {
                break; // disconnected; cannot happen per component
            }
            let first = cands[0] as usize;
            let delta = if in_node[self.eu[first] as usize] {
                best
            } else {
                -best
            };
            for v in 0..n {
                if in_node[v] {
                    self.rank[v] += delta;
                }
            }
            for &e in &cands {
                let e = e as usize;
                if self.slack(e) != 0 {
                    continue;
                }
                let (a, b) = (self.eu[e], self.ev[e]);
                let (inside, outside) = if in_node[a as usize] { (a, b) } else { (b, a) };
                if in_node[outside as usize] == in_node[inside as usize] {
                    continue;
                }
                in_node[outside as usize] = true;
                count += 1;
                self.in_tree[e] = true;
                self.tadj[inside as usize].push(e as u32);
                self.tadj[outside as usize].push(e as u32);
                stack.push(outside);
            }
        }
    }

    /// Assigns low/lim numbers, parent edges and a postorder from root 0.
    fn number_tree(&mut self) {
        self.post.clear();
        self.par_edge[0] = NONE;
        self.low[0] = 1;
        self.number_subtree(0);
    }

    /// Numbers the subtree below `root`: parent edges, low/lim and postorder slots,
    /// reusing the range of numbers `root` already starts at (`low[root]`). Children are
    /// visited in `tadj` order, so renumbering a subtree gives the numbers a renumbering
    /// of the whole tree would give it.
    fn number_subtree(&mut self, root: u32) {
        let mut counter = self.low[root as usize];
        let mut stack: Vec<(u32, usize)> = vec![(root, 0)];
        while let Some(top) = stack.len().checked_sub(1) {
            let (v, idx) = stack[top];
            let vi = v as usize;
            if idx < self.tadj[vi].len() {
                let e = self.tadj[vi][idx];
                stack[top].1 += 1;
                if e == self.par_edge[vi] {
                    continue;
                }
                let w = self.other(e, v);
                self.par_edge[w as usize] = e;
                self.low[w as usize] = counter;
                stack.push((w, 0));
            } else {
                self.lim[vi] = counter;
                // Slots are appended when numbering from scratch, overwritten otherwise.
                match self.post.get_mut(counter as usize - 1) {
                    Some(slot) => *slot = v,
                    None => self.post.push(v),
                }
                counter += 1;
                stack.pop();
            }
        }
    }

    fn compute_cuts(&mut self) {
        for k in 0..self.post.len() {
            let v = self.post[k];
            let pe = self.par_edge[v as usize];
            if pe == NONE {
                continue;
            }
            let child_is_tail = self.eu[pe as usize] == v;
            let mut cut = self.ew[pe as usize];
            for i in 0..self.inc[v as usize].len() {
                let e = self.inc[v as usize][i];
                if e == pe {
                    continue;
                }
                let is_out = self.eu[e as usize] == v;
                let points_to_head = is_out == child_is_tail;
                let w = self.ew[e as usize];
                cut += if points_to_head { w } else { -w };
                if self.in_tree[e as usize] {
                    let oc = self.cut[e as usize];
                    cut += if points_to_head { -oc } else { oc };
                }
            }
            self.cut[pe as usize] = cut;
        }
    }

    fn enter_edge(&self, leave: usize) -> Option<usize> {
        let (u, v) = (self.eu[leave] as usize, self.ev[leave] as usize);
        let (sub_root, flip) = if self.lim[u] > self.lim[v] {
            (v, true)
        } else {
            (u, false)
        };
        let (lo, hi) = (self.low[sub_root], self.lim[sub_root]);
        let desc = |x: u32| {
            let l = self.lim[x as usize];
            lo <= l && l <= hi
        };
        let mut best: Option<(i32, usize)> = None;
        for e in 0..self.eu.len() {
            if self.in_tree[e] {
                continue;
            }
            if flip == desc(self.eu[e]) && flip != desc(self.ev[e]) {
                let s = self.slack(e);
                if best.is_none_or(|(bs, _)| s < bs) {
                    best = Some((s, e));
                }
            }
        }
        best.map(|(_, e)| e)
    }

    /// Child end of tree edge `e`: the one farther from the root.
    fn child_end(&self, e: usize) -> u32 {
        let (u, v) = (self.eu[e], self.ev[e]);
        if self.lim[u as usize] > self.lim[v as usize] {
            v
        } else {
            u
        }
    }

    /// Parent of `v` in the tree, `NONE` for the root.
    fn parent_of(&self, v: u32) -> u32 {
        match self.par_edge[v as usize] {
            NONE => NONE,
            pe => self.other(pe, v),
        }
    }

    /// Lowest common ancestor of `a` and `b` (a parent always has a larger `lim`).
    fn lca(&self, mut a: u32, mut b: u32) -> u32 {
        while a != b {
            let up = if self.lim[a as usize] < self.lim[b as usize] {
                &mut a
            } else {
                &mut b
            };
            let p = self.parent_of(*up);
            if p == NONE {
                return *up;
            }
            *up = p;
        }
        a
    }

    /// Replaces tree edge `leave` by `enter`, updating only what changes (Gansner et al.):
    /// the ranks of the subtree that moves, the cut values on the tree path between the
    /// endpoints of `enter`, and the numbering of the subtree below their common ancestor.
    fn exchange(&mut self, leave: usize, enter: usize) {
        let moved = self.child_end(leave);
        let (a, b) = (self.eu[enter], self.ev[enter]);
        let top = self.lca(a, b);
        let leave_cut = self.cut[leave];
        self.shift_subtree_ranks(moved, enter);
        self.walk_cuts(a, top, leave_cut, true);
        self.walk_cuts(b, top, leave_cut, false);
        self.cut[enter] = -leave_cut;
        self.swap_tree_edges(leave, enter);
        self.number_subtree(top);
        #[cfg(debug_assertions)]
        self.check_against_full();
    }

    /// Makes `enter` tight by moving the subtree below `moved` (it holds one end of
    /// `enter`; the root side stays where it is).
    fn shift_subtree_ranks(&mut self, moved: u32, enter: usize) {
        let (lo, hi) = (self.low[moved as usize], self.lim[moved as usize]);
        let tail_lim = self.lim[self.eu[enter] as usize];
        let slack = self.slack(enter);
        let delta = if lo <= tail_lim && tail_lim <= hi {
            slack
        } else {
            -slack
        };
        if delta == 0 {
            return;
        }
        for k in lo as usize - 1..hi as usize {
            let v = self.post[k] as usize;
            self.rank[v] += delta;
        }
    }

    /// Adds `delta` to (or, by edge direction, subtracts it from) the cut value of every
    /// tree edge on the way from `v` up to `top`. `dir` is true when walking from the
    /// tail of the entering edge.
    fn walk_cuts(&mut self, mut v: u32, top: u32, delta: i64, dir: bool) {
        while v != top {
            let pe = self.par_edge[v as usize];
            if pe == NONE {
                return;
            }
            let from_tail = self.eu[pe as usize] == v;
            let add = from_tail == dir;
            self.cut[pe as usize] += if add { delta } else { -delta };
            v = self.other(pe, v);
        }
    }

    /// Swaps `leave` out of and `enter` into the tree adjacency.
    fn swap_tree_edges(&mut self, leave: usize, enter: usize) {
        self.in_tree[leave] = false;
        for end in [self.eu[leave], self.ev[leave]] {
            let t = &mut self.tadj[end as usize];
            if let Some(p) = t.iter().position(|&x| x == leave as u32) {
                t.swap_remove(p);
            }
        }
        self.in_tree[enter] = true;
        let (a, b) = (self.eu[enter], self.ev[enter]);
        self.tadj[a as usize].push(enter as u32);
        self.tadj[b as usize].push(enter as u32);
    }

    /// Ranks recomputed from the root along tight tree edges (preorder = reverse
    /// postorder). Reference for the incremental update.
    #[cfg(debug_assertions)]
    fn recompute_ranks(&mut self) {
        for k in (0..self.post.len()).rev() {
            let v = self.post[k];
            let pe = self.par_edge[v as usize];
            if pe == NONE {
                continue;
            }
            let p = self.other(pe, v);
            let l = self.el[pe as usize];
            self.rank[v as usize] = if self.eu[pe as usize] == p {
                self.rank[p as usize] + l
            } else {
                self.rank[p as usize] - l
            };
        }
    }

    /// Debug check: the incrementally maintained numbering, cut values and ranks equal
    /// what a full recomputation over the whole tree gives.
    #[cfg(debug_assertions)]
    fn check_against_full(&self) {
        let mut full = self.clone();
        full.number_tree();
        full.compute_cuts();
        full.recompute_ranks();
        debug_assert_eq!(self.low, full.low, "low");
        debug_assert_eq!(self.lim, full.lim, "lim");
        debug_assert_eq!(self.par_edge, full.par_edge, "parent edges");
        debug_assert_eq!(self.post, full.post, "postorder");
        debug_assert_eq!(self.rank, full.rank, "ranks");
        for e in 0..self.eu.len() {
            if self.in_tree[e] {
                debug_assert_eq!(self.cut[e], full.cut[e], "cut value of edge {e}");
            }
        }
    }
}

/// Replaces long edges by chains of unit edges. A labelled edge gets one `Label`
/// node at its middle rank; every other interior node is a `Dummy`.
/// `labels` holds the normalised (w, h) of each authored edge's label, if any.
pub(crate) fn insert_dummies(lg: &mut LGraph, _g: &Graph, labels: &[Option<[f32; 2]>]) {
    let old = std::mem::take(&mut lg.edges);
    let mut edges = Vec::with_capacity(old.len());
    let seg_weight = |nodes: &[LNode], a: u32, b: u32| -> u32 {
        match (
            nodes[a as usize].kind.is_edge_part(),
            nodes[b as usize].kind.is_edge_part(),
        ) {
            (false, false) => 1,
            (true, true) => 8,
            _ => 2,
        }
    };
    // Ranks that hold a cluster's title edge: labels avoid them (they would sit on the title).
    let mut title_rank =
        vec![false; (lg.nodes.iter().map(|nd| nd.rank).max().unwrap_or(0) + 2).max(0) as usize];
    for c in 0..lg.tree.parent.len() {
        if lg.cl_leaf[c] {
            continue;
        }
        let v = if lg.dir == crate::graph::Direction::BT {
            lg.cl_bot[c]
        } else {
            lg.cl_top[c]
        };
        if let Some(f) = title_rank.get_mut(lg.nodes[v as usize].rank.max(0) as usize) {
            *f = true;
        }
    }
    for e in old {
        let r0 = lg.nodes[e.from as usize].rank;
        let r1 = lg.nodes[e.to as usize].rank;
        let span = r1 - r0;
        let label = labels.get(e.orig.0 as usize).copied().flatten();
        if span <= 1 {
            edges.push(super::LEdge {
                weight: 1,
                min_len: 1,
                ..e
            });
            continue;
        }
        let container = chain_container(lg, e.from, e.to);
        let (anc_a, anc_b) = chain_ancestors(lg, e.from, e.to);
        let label_rank = label.map(|_| {
            let mid = r0 + span / 2;
            let mut best = mid;
            for d in 0..span {
                let cands = [mid + d, mid - d];
                if let Some(&r) = cands.iter().find(|&&r| {
                    r > r0 && r < r1 && !title_rank.get(r as usize).copied().unwrap_or(false)
                }) {
                    best = r;
                    break;
                }
            }
            best
        });
        let mut prev = e.from;
        for r in r0 + 1..r1 {
            let id = lg.nodes.len() as u32;
            let (kind, w, h) = match (label_rank, label) {
                (Some(lr), Some(d)) if lr == r => (LKind::Label(e.orig), d[0], d[1]),
                _ => (LKind::Dummy(e.orig), 0.0, 0.0),
            };
            let cont = rank_container(lg, &anc_a, &anc_b, r, container);
            lg.nodes.push(LNode {
                kind,
                w,
                h,
                rank: r,
                order: 0,
                x: 0.0,
                y: 0.0,
                cluster: (cont != NONE).then_some(ClusterId(cont)),
            });
            edges.push(super::LEdge {
                from: prev,
                to: id,
                orig: e.orig,
                reversed: e.reversed,
                weight: 1,
                min_len: 1,
            });
            prev = id;
        }
        edges.push(super::LEdge {
            from: prev,
            to: e.to,
            orig: e.orig,
            reversed: e.reversed,
            weight: 1,
            min_len: 1,
        });
    }
    for e in &mut edges {
        e.weight = seg_weight(&lg.nodes, e.from, e.to);
    }
    lg.edges = edges;
    let max_rank = lg.nodes.iter().map(|nd| nd.rank).max().unwrap_or(-1);
    let mut ranks: Vec<Vec<u32>> = vec![Vec::new(); (max_rank + 1).max(0) as usize];
    for (i, nd) in lg.nodes.iter().enumerate() {
        ranks[nd.rank.max(0) as usize].push(i as u32);
    }
    lg.ranks = ranks;
}

/// Cluster whose members the chain between `a` and `b` belongs to (`NONE` = top level).
fn chain_container(lg: &LGraph, a: u32, b: u32) -> u32 {
    if lg.tree.parent.is_empty() {
        return NONE;
    }
    let end = |v: u32| -> u32 {
        match lg.nodes[v as usize].kind {
            LKind::BorderTop(c) | LKind::BorderBottom(c)
                if !lg.cl_leaf.get(c.0 as usize).copied().unwrap_or(false) =>
            {
                lg.tree.parent[c.0 as usize]
            }
            _ => lg.nodes[v as usize].cluster.map_or(NO_CLUSTER, |c| c.0),
        }
    };
    lg.tree.lca(end(a), end(b))
}

/// Clusters entered below the chain's common container, from each end upwards.
fn chain_ancestors(lg: &LGraph, a: u32, b: u32) -> (Vec<u32>, Vec<u32>) {
    if lg.tree.parent.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let end = |v: u32| -> u32 {
        match lg.nodes[v as usize].kind {
            LKind::BorderTop(c) | LKind::BorderBottom(c)
                if !lg.cl_leaf.get(c.0 as usize).copied().unwrap_or(false) =>
            {
                lg.tree.parent[c.0 as usize]
            }
            _ => lg.nodes[v as usize].cluster.map_or(NO_CLUSTER, |c| c.0),
        }
    };
    let (ea, eb) = (end(a), end(b));
    let l = lg.tree.lca(ea, eb);
    let up = |mut c: u32| -> Vec<u32> {
        let mut out = Vec::new();
        while c != NONE && c != l {
            out.push(c);
            c = lg.tree.parent[c as usize];
        }
        out
    };
    (up(ea), up(eb))
}

/// Container of the chain node at rank `r`: the deepest endpoint cluster whose side
/// borders exist at that rank, else the common container.
fn rank_container(lg: &LGraph, anc_a: &[u32], anc_b: &[u32], r: i32, common: u32) -> u32 {
    let pick = |anc: &[u32]| -> u32 {
        for &c in anc {
            let ci = c as usize;
            if lg.cl_leaf[ci] {
                continue;
            }
            let (t, b) = (
                lg.nodes[lg.cl_top[ci] as usize].rank,
                lg.nodes[lg.cl_bot[ci] as usize].rank,
            );
            if t < r && r < b {
                return c;
            }
        }
        NONE
    };
    if !lg.chains_in_clusters {
        return common;
    }
    let (pa, pb) = (pick(anc_a), pick(anc_b));
    match (pa, pb) {
        (NONE, NONE) => common,
        (a, NONE) => a,
        (NONE, b) => b,
        _ => common,
    }
}
