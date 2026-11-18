//! Crossing minimisation: DFS (or seeded) initial order, weighted-median sweeps,
//! adjacent transposition, best ordering kept. Cluster members stay contiguous in
//! every rank (side borders first/last, recursively); back-edge dummies are moved
//! to the outside of their group when that costs no crossings.

use std::sync::atomic::AtomicBool;

use super::compound::NONE;
use super::{Csr, LGraph, LKind, LayoutError, check_cancel};

/// Cluster information needed while ordering.
struct Hier {
    parent: Vec<u32>,
    depth: Vec<u32>,
    /// Container cluster per node (`NONE` = top level).
    cluster: Vec<u32>,
    /// 1 = left border, 2 = right border.
    side: Vec<u8>,
    /// Dummy or label node of a cycle-reversed edge.
    back: Vec<bool>,
}

#[derive(Clone, Copy)]
enum Item {
    Node(u32),
    Group(u32),
}

struct Group {
    cluster: u32,
    left: u32,
    right: u32,
    items: Vec<Item>,
    agg: (f32, f32, u32),
}

#[derive(Default)]
struct Work {
    groups: Vec<Group>,
    count: usize,
    gidx: Vec<u32>,
    used: Vec<u32>,
    chain: Vec<u32>,
    nkey: Vec<Option<(f32, f32)>>,
    /// Global sort key per cluster (identical in every rank, so sibling clusters keep
    /// the same left-to-right order everywhere).
    gkey: Vec<f32>,
    /// Scale turning neighbour positions into rank-independent relative positions.
    scale: f32,
    /// Position of every cluster in the order of the global keys.
    gorder: Vec<u32>,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Plain,
    Sort,
    Outward,
}

impl Work {
    fn new_group(&mut self, cluster: u32) -> usize {
        if self.count == self.groups.len() {
            self.groups.push(Group {
                cluster,
                left: NONE,
                right: NONE,
                items: Vec::new(),
                agg: (0.0, 0.0, 0),
            });
        } else {
            let g = &mut self.groups[self.count];
            g.cluster = cluster;
            g.left = NONE;
            g.right = NONE;
            g.items.clear();
            g.agg = (0.0, 0.0, 0);
        }
        self.count += 1;
        self.count - 1
    }

    /// Groups the nodes of `layer` by cluster (first appearance order).
    fn build(&mut self, layer: &[u32], h: &Hier) {
        self.count = 0;
        self.new_group(NONE);
        for &v in layer {
            self.chain.clear();
            let mut c = h.cluster[v as usize];
            while c != NONE {
                self.chain.push(c);
                c = h.parent[c as usize];
            }
            let mut cur = 0usize;
            for i in (0..self.chain.len()).rev() {
                let cl = self.chain[i];
                if self.gidx[cl as usize] == NONE {
                    let ng = self.new_group(cl);
                    self.groups[cur].items.push(Item::Group(ng as u32));
                    self.gidx[cl as usize] = ng as u32;
                    self.used.push(cl);
                }
                cur = self.gidx[cl as usize] as usize;
            }
            match h.side[v as usize] {
                1 => self.groups[cur].left = v,
                2 => self.groups[cur].right = v,
                _ => self.groups[cur].items.push(Item::Node(v)),
            }
        }
        for &cl in &self.used {
            self.gidx[cl as usize] = NONE;
        }
        self.used.clear();
    }

    /// Mean (median, barycentre) of the keyed nodes of each group (children first).
    fn aggregate(&mut self) {
        for gi in (0..self.count).rev() {
            let mut agg = (0.0f32, 0.0f32, 0u32);
            for side in [self.groups[gi].left, self.groups[gi].right] {
                if side != NONE
                    && let Some((a, b)) = self.nkey[side as usize]
                {
                    agg.0 += a;
                    agg.1 += b;
                    agg.2 += 1;
                }
            }
            for it in &self.groups[gi].items {
                match *it {
                    Item::Node(v) => {
                        if let Some((a, b)) = self.nkey[v as usize] {
                            agg.0 += a;
                            agg.1 += b;
                            agg.2 += 1;
                        }
                    }
                    Item::Group(c) => {
                        let o = self.groups[c as usize].agg;
                        agg.0 += o.0;
                        agg.1 += o.1;
                        agg.2 += o.2;
                    }
                }
            }
            self.groups[gi].agg = agg;
        }
    }

    /// Keys of the child clusters of group `gi`: their rank-local keys, reassigned in
    /// the order of the global keys so that sibling clusters keep one order everywhere.
    fn cluster_keys(&self, gi: usize, out: &mut Vec<(u32, (f32, f32))>) {
        out.clear();
        let g = &self.groups[gi];
        let mut idx: Vec<(u32, u32)> = Vec::new();
        let mut keys: Vec<(f32, f32)> = Vec::new();
        for it in &g.items {
            if let Item::Group(c) = *it {
                let cg = &self.groups[c as usize];
                let k = if cg.agg.2 > 0 {
                    let n = cg.agg.2 as f32;
                    (cg.agg.0 / n * self.scale, cg.agg.1 / n * self.scale)
                } else {
                    let v = self.gkey[cg.cluster as usize];
                    (v, v)
                };
                idx.push((self.gorder[cg.cluster as usize], c));
                keys.push(k);
            }
        }
        if idx.is_empty() {
            return;
        }
        keys.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        idx.sort_unstable();
        for (t, &(_, c)) in idx.iter().enumerate() {
            out.push((c, keys[t]));
        }
    }

    /// Pairs of sibling clusters that are neighbours in some group of the last `build`.
    fn adjacent_clusters(&self, out: &mut Vec<(u32, u32)>) {
        for g in &self.groups[..self.count] {
            for w in g.items.windows(2) {
                if let (Item::Group(a), Item::Group(b)) = (w[0], w[1]) {
                    out.push((
                        self.groups[a as usize].cluster,
                        self.groups[b as usize].cluster,
                    ));
                }
            }
        }
    }

    /// Exchanges the positions of sibling clusters `a` and `b` in their parent group.
    fn swap_clusters(&mut self, a: u32, b: u32) {
        let mut ga = usize::MAX;
        let mut gb = usize::MAX;
        for (i, g) in self.groups[..self.count].iter().enumerate() {
            if g.cluster == a {
                ga = i;
            } else if g.cluster == b {
                gb = i;
            }
        }
        if ga == usize::MAX || gb == usize::MAX {
            return;
        }
        for gi in 0..self.count {
            let items = &mut self.groups[gi].items;
            let pa = items
                .iter()
                .position(|it| matches!(it, Item::Group(x) if *x as usize == ga));
            let pb = items
                .iter()
                .position(|it| matches!(it, Item::Group(x) if *x as usize == gb));
            if let (Some(pa), Some(pb)) = (pa, pb) {
                items.swap(pa, pb);
                return;
            }
        }
    }

    fn emit(&self, gi: usize, mode: Mode, back: &[bool], out: &mut Vec<u32>) {
        let g = &self.groups[gi];
        if g.left != NONE {
            out.push(g.left);
        }
        let k = g.items.len();
        let items: Vec<Item> = match mode {
            Mode::Plain => g.items.clone(),
            Mode::Sort => {
                let mut ck: Vec<(u32, (f32, f32))> = Vec::new();
                self.cluster_keys(gi, &mut ck);
                let mut keyed: Vec<(f32, f32, u32, Item)> = Vec::with_capacity(k);
                let mut fixed = vec![false; k];
                for (i, &it) in g.items.iter().enumerate() {
                    match it {
                        Item::Node(v) => match self.nkey[v as usize] {
                            Some((a, b)) => {
                                keyed.push((a * self.scale, b * self.scale, i as u32, it));
                            }
                            None => fixed[i] = true,
                        },
                        Item::Group(c) => {
                            let (a, b) = ck.iter().find(|x| x.0 == c).map_or((0.0, 0.0), |x| x.1);
                            let tie =
                                0x4000_0000 + self.gorder[self.groups[c as usize].cluster as usize];
                            keyed.push((a, b, tie, it));
                        }
                    }
                }
                keyed.sort_by(|a, b| {
                    a.0.total_cmp(&b.0)
                        .then(a.1.total_cmp(&b.1))
                        .then(a.2.cmp(&b.2))
                });
                let mut res = Vec::with_capacity(k);
                let mut next = 0;
                for (i, &it) in g.items.iter().enumerate() {
                    if fixed[i] {
                        res.push(it);
                    } else {
                        res.push(keyed[next].3);
                        next += 1;
                    }
                }
                res
            }
            Mode::Outward => {
                let mut left = Vec::new();
                let mut right = Vec::new();
                let mut mid = Vec::with_capacity(k);
                for (i, &it) in g.items.iter().enumerate() {
                    match it {
                        Item::Node(v) if back[v as usize] => {
                            if (i as f32) < (k as f32 - 1.0) / 2.0 {
                                left.push(it);
                            } else {
                                right.push(it);
                            }
                        }
                        _ => mid.push(it),
                    }
                }
                left.extend(mid);
                left.extend(right);
                left
            }
        };
        for it in items {
            match it {
                Item::Node(v) => out.push(v),
                Item::Group(c) => self.emit(c as usize, mode, back, out),
            }
        }
        if g.right != NONE {
            out.push(g.right);
        }
        let _ = g.cluster;
    }
}

/// Edge crossings between rank `r` and `r + 1` (accumulator tree). `pos` is the
/// position of every node in its rank, `down` the edges towards the next rank.
fn pair_crossings(
    ranks: &[Vec<u32>],
    pos: &[u32],
    down: &Csr<()>,
    r: usize,
    tree: &mut Vec<u32>,
    buf: &mut Vec<u32>,
) -> u64 {
    let lower = ranks[r + 1].len();
    tree.clear();
    tree.resize(lower + 1, 0);
    let mut total = 0u64;
    let mut inserted = 0u64;
    for &u in &ranks[r] {
        buf.clear();
        buf.extend(down.row(u).iter().map(|&(v, _)| pos[v as usize]));
        buf.sort_unstable();
        for &p in buf.iter() {
            // Edges already inserted whose lower endpoint is right of `p`.
            let mut i = p as usize + 1;
            let mut le = 0u64;
            while i > 0 {
                le += u64::from(tree[i]);
                i -= i & i.wrapping_neg();
            }
            total += inserted - le;
        }
        for &p in buf.iter() {
            let mut i = p as usize + 1;
            while i <= lower {
                tree[i] += 1;
                i += i & i.wrapping_neg();
            }
        }
        inserted += buf.len() as u64;
    }
    total
}

/// Total crossings over all adjacent rank pairs.
fn count_all(ranks: &[Vec<u32>], pos: &[u32], down: &Csr<()>) -> u64 {
    let mut tree = Vec::new();
    let mut buf = Vec::new();
    (0..ranks.len().saturating_sub(1))
        .map(|r| pair_crossings(ranks, pos, down, r, &mut tree, &mut buf))
        .sum()
}

fn rel_scale(len: usize) -> f32 {
    1.0 / (len.saturating_sub(1)).max(1) as f32
}

/// Ordered layers plus adjacency between neighbouring ranks.
struct Layers {
    ranks: Vec<Vec<u32>>,
    pos: Vec<u32>,
    up: Csr<()>,
    down: Csr<()>,
    hier: Option<Hier>,
    clustered: bool,
    work: Work,
    /// Rank of every node.
    rank: Vec<u32>,
    /// Per cluster: ranks of its top and bottom border.
    cl_range: Vec<(u32, u32)>,
}

impl Layers {
    fn new(lg: &LGraph) -> Self {
        let n = lg.nodes.len();
        let mut ups = Vec::with_capacity(lg.edges.len());
        let mut downs = Vec::with_capacity(lg.edges.len());
        for e in &lg.edges {
            if lg.nodes[e.to as usize].rank == lg.nodes[e.from as usize].rank + 1 {
                downs.push((e.from, e.to, ()));
                ups.push((e.to, e.from, ()));
            }
        }
        let clustered = !lg.tree.parent.is_empty();
        let mut back = vec![false; n];
        let mut any_back = false;
        for e in &lg.edges {
            if e.reversed {
                for v in [e.from, e.to] {
                    if lg.nodes[v as usize].kind.is_edge_part() {
                        back[v as usize] = true;
                        any_back = true;
                    }
                }
            }
        }
        let hier = (clustered || any_back).then(|| Hier {
            parent: lg.tree.parent.clone(),
            depth: lg.tree.depth.clone(),
            cluster: lg
                .nodes
                .iter()
                .map(|nd| nd.cluster.map_or(NONE, |c| c.0))
                .collect(),
            side: lg
                .nodes
                .iter()
                .map(|nd| match nd.kind {
                    LKind::BorderLeft(_) => 1,
                    LKind::BorderRight(_) => 2,
                    _ => 0,
                })
                .collect(),
            back,
        });
        let mut work = Work::default();
        work.gidx = vec![NONE; lg.tree.parent.len()];
        work.nkey = vec![None; n];
        work.gkey = vec![0.0; lg.tree.parent.len()];
        work.scale = 1.0;
        work.gorder = vec![0; lg.tree.parent.len()];
        let mut layers = Self {
            ranks: lg.ranks.clone(),
            pos: vec![0; n],
            up: Csr::build(n, &ups),
            down: Csr::build(n, &downs),
            hier,
            clustered,
            work,
            rank: lg.nodes.iter().map(|nd| nd.rank.max(0) as u32).collect(),
            cl_range: (0..lg.tree.parent.len())
                .map(|c| {
                    (
                        lg.nodes[lg.cl_top[c] as usize].rank.max(0) as u32,
                        lg.nodes[lg.cl_bot[c] as usize].rank.max(0) as u32,
                    )
                })
                .collect(),
        };
        layers.refresh_pos();
        layers
    }

    fn refresh_pos(&mut self) {
        for layer in &self.ranks {
            for (i, &v) in layer.iter().enumerate() {
                self.pos[v as usize] = i as u32;
            }
        }
    }

    fn refresh_rank(&mut self, r: usize) {
        for (i, &v) in self.ranks[r].iter().enumerate() {
            self.pos[v as usize] = i as u32;
        }
    }

    /// Edge crossings between rank `r` and `r + 1` (accumulator tree).
    fn count_pair(&self, r: usize, tree: &mut Vec<u32>, buf: &mut Vec<u32>) -> u64 {
        pair_crossings(&self.ranks, &self.pos, &self.down, r, tree, buf)
    }

    fn count(&self) -> u64 {
        count_all(&self.ranks, &self.pos, &self.down)
    }

    /// (weighted median, barycentre) of the neighbour positions of `v`.
    fn key_of(&self, v: u32, use_up: bool, scratch: &mut Vec<u32>) -> Option<(f32, f32)> {
        let row = if use_up {
            self.up.row(v)
        } else {
            self.down.row(v)
        };
        if row.is_empty() {
            return None;
        }
        scratch.clear();
        scratch.extend(row.iter().map(|&(w, _)| self.pos[w as usize]));
        scratch.sort_unstable();
        let k = scratch.len();
        let median = if k % 2 == 1 {
            scratch[k / 2] as f32
        } else if k == 2 {
            (scratch[0] + scratch[1]) as f32 / 2.0
        } else {
            let l = (scratch[k / 2 - 1] - scratch[0]) as f32;
            let rr = (scratch[k - 1] - scratch[k / 2]) as f32;
            if l + rr == 0.0 {
                (scratch[k / 2 - 1] + scratch[k / 2]) as f32 / 2.0
            } else {
                (scratch[k / 2 - 1] as f32 * rr + scratch[k / 2] as f32 * l) / (l + rr)
            }
        };
        let bary = scratch.iter().map(|&p| p as f32).sum::<f32>() / k as f32;
        Some((median, bary))
    }

    /// Reorders rank `r` by the weighted median of neighbour positions in the
    /// adjacent rank (`use_up` selects the rank above). Nodes without neighbours keep their slot.
    fn reorder(&mut self, r: usize, use_up: bool, scratch: &mut Vec<u32>) {
        if self.clustered {
            self.reorder_hier(r, use_up, scratch);
            return;
        }
        let layer = &self.ranks[r];
        let mut keyed: Vec<(f32, f32, u32, u32)> = Vec::with_capacity(layer.len());
        let mut slots: Vec<u32> = Vec::with_capacity(layer.len());
        for (i, &v) in layer.iter().enumerate() {
            let Some((median, bary)) = self.key_of(v, use_up, scratch) else {
                continue;
            };
            keyed.push((median, bary, i as u32, v));
            slots.push(i as u32);
        }
        keyed.sort_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
        });
        let layer = &mut self.ranks[r];
        for (slot, k) in slots.iter().zip(&keyed) {
            layer[*slot as usize] = k.3;
            self.pos[k.3 as usize] = *slot;
        }
    }

    fn reorder_hier(&mut self, r: usize, use_up: bool, scratch: &mut Vec<u32>) {
        let Some(hier) = self.hier.take() else {
            return;
        };
        let mut work = std::mem::take(&mut self.work);
        let layer = std::mem::take(&mut self.ranks[r]);
        let adj = if use_up {
            r.checked_sub(1)
        } else {
            (r + 1 < self.ranks.len()).then_some(r + 1)
        };
        work.scale = adj.map_or(1.0, |a| rel_scale(self.ranks[a].len()));
        for &v in &layer {
            work.nkey[v as usize] = self.key_of(v, use_up, scratch);
        }
        work.build(&layer, &hier);
        work.aggregate();
        let mut out = layer;
        out.clear();
        work.emit(0, Mode::Sort, &hier.back, &mut out);
        self.ranks[r] = out;
        self.refresh_rank(r);
        self.work = work;
        self.hier = Some(hier);
    }

    /// Relative (rank-independent) position of node `w` in its rank.
    fn rel(&self, w: u32) -> f32 {
        let r = self.rank[w as usize] as usize;
        self.pos[w as usize] as f32 * rel_scale(self.ranks[r].len())
    }

    /// Global key of every cluster: mean relative position of the outside neighbours of
    /// its members (all ranks, nested members included). `own` uses the members' own
    /// positions instead (initial order).
    fn update_gkeys(&mut self, mode: u8) {
        let own = mode != 0;
        let Some(hier) = self.hier.take() else {
            return;
        };
        let k = hier.parent.len();
        let mut sum = vec![0.0f64; k];
        let mut cnt = vec![0u32; k];
        let mut up_a: Vec<u32> = Vec::new();
        let n = self.pos.len();
        for v in 0..n as u32 {
            let cv = hier.cluster[v as usize];
            if cv == NONE {
                continue;
            }
            if own {
                let x = f64::from(self.rel(v));
                let mut a = cv;
                while a != NONE {
                    let ai = a as usize;
                    match mode {
                        2 => {
                            if cnt[ai] == 0 || x < sum[ai] {
                                sum[ai] = x;
                            }
                        }
                        3 => {
                            if cnt[ai] == 0 || x > sum[ai] {
                                sum[ai] = x;
                            }
                        }
                        _ => sum[ai] += x,
                    }
                    cnt[ai] += 1;
                    a = hier.parent[ai];
                }
                continue;
            }
            for row in [self.up.row(v), self.down.row(v)] {
                for &(w, _) in row {
                    let x = f64::from(self.rel(w));
                    let cw = hier.cluster[w as usize];
                    // Climb from v's cluster up to the lowest common ancestor.
                    let (mut a, mut b) = (cv, cw);
                    let d = |c: u32| {
                        if c == NONE {
                            -1
                        } else {
                            i64::from(hier.depth[c as usize])
                        }
                    };
                    up_a.clear();
                    while a != b {
                        if d(a) >= d(b) {
                            up_a.push(a);
                            a = if a == NONE {
                                NONE
                            } else {
                                hier.parent[a as usize]
                            };
                        } else {
                            b = if b == NONE {
                                NONE
                            } else {
                                hier.parent[b as usize]
                            };
                        }
                    }
                    for &c in &up_a {
                        sum[c as usize] += x;
                        cnt[c as usize] += 1;
                    }
                }
            }
        }
        for c in 0..k {
            if cnt[c] > 0 {
                let v = if mode == 2 || mode == 3 {
                    sum[c]
                } else {
                    sum[c] / f64::from(cnt[c])
                };
                self.work.gkey[c] = if mode == 4 { -v as f32 } else { v as f32 };
            }
        }
        let mut ord: Vec<u32> = (0..k as u32).collect();
        ord.sort_by(|&a, &b| {
            self.work.gkey[a as usize]
                .total_cmp(&self.work.gkey[b as usize])
                .then(a.cmp(&b))
        });
        for (t, &c) in ord.iter().enumerate() {
            self.work.gorder[c as usize] = t as u32;
        }
        self.hier = Some(hier);
    }

    /// Gives every cluster one global key and re-establishes contiguity of every
    /// cluster in every rank, siblings in the same order everywhere.
    fn normalise(&mut self, mode: u8) {
        self.update_gkeys(mode);
        for r in 0..self.ranks.len() {
            let Some(hier) = self.hier.take() else {
                return;
            };
            let mut work = std::mem::take(&mut self.work);
            let layer = std::mem::take(&mut self.ranks[r]);
            work.scale = 1.0;
            let sc = rel_scale(layer.len());
            for (i, &v) in layer.iter().enumerate() {
                let x = i as f32 * sc;
                work.nkey[v as usize] = Some((x, x));
            }
            work.build(&layer, &hier);
            work.aggregate();
            let mut out = layer;
            out.clear();
            work.emit(0, Mode::Sort, &hier.back, &mut out);
            self.ranks[r] = out;
            self.refresh_rank(r);
            self.work = work;
            self.hier = Some(hier);
        }
    }

    fn regroup(&mut self, r: usize, mode: Mode) {
        let Some(hier) = self.hier.take() else {
            return;
        };
        let mut work = std::mem::take(&mut self.work);
        let layer = std::mem::take(&mut self.ranks[r]);
        work.build(&layer, &hier);
        let mut out = layer;
        out.clear();
        work.emit(0, mode, &hier.back, &mut out);
        self.ranks[r] = out;
        self.refresh_rank(r);
        self.work = work;
        self.hier = Some(hier);
    }

    /// Exchanges neighbouring sibling clusters in every rank they share when that
    /// lowers the crossing count (the global sibling order stays consistent).
    fn cluster_swaps(&mut self, passes: usize, cancel: &AtomicBool) -> Result<(), LayoutError> {
        let Some(hier) = self.hier.take() else {
            return Ok(());
        };
        let mut work = std::mem::take(&mut self.work);
        let mut tree = Vec::new();
        let mut buf = Vec::new();
        let mut pairs: Vec<(u32, u32)> = Vec::new();
        let mut saved: Vec<Vec<u32>> = Vec::new();
        for _ in 0..passes {
            check_cancel(cancel)?;
            pairs.clear();
            for r in 0..self.ranks.len() {
                work.build(&self.ranks[r], &hier);
                work.adjacent_clusters(&mut pairs);
            }
            pairs.sort_unstable();
            pairs.dedup();
            let mut improved = false;
            for &(a, b) in &pairs {
                let (ta, ba) = self.cl_range[a as usize];
                let (tb, bb) = self.cl_range[b as usize];
                let (lo, hi) = (ta.max(tb) as usize, ba.min(bb) as usize);
                if lo > hi || hi >= self.ranks.len() {
                    continue;
                }
                // Skip when an earlier swap already put `b` before `a`.
                let first_left = |s: &Self| -> Option<bool> {
                    let layer = &s.ranks[lo];
                    let pos_of = |c: u32| {
                        layer.iter().position(|&v| {
                            let mut x = hier.cluster[v as usize];
                            while x != NONE && x != c {
                                x = hier.parent[x as usize];
                            }
                            x == c
                        })
                    };
                    Some(pos_of(a)? < pos_of(b)?)
                };
                if first_left(self) != Some(true) {
                    continue;
                }
                let (c0, c1) = (lo.saturating_sub(1), hi.min(self.ranks.len() - 2));
                let cost = |s: &Self, tree: &mut Vec<u32>, buf: &mut Vec<u32>| -> u64 {
                    (c0..=c1).map(|r| s.count_pair(r, tree, buf)).sum()
                };
                if self.ranks.len() < 2 {
                    continue;
                }
                let before = cost(self, &mut tree, &mut buf);
                saved.clear();
                for r in lo..=hi {
                    saved.push(self.ranks[r].clone());
                }
                for r in lo..=hi {
                    let layer = std::mem::take(&mut self.ranks[r]);
                    work.build(&layer, &hier);
                    work.swap_clusters(a, b);
                    let mut o = layer;
                    o.clear();
                    work.emit(0, Mode::Plain, &hier.back, &mut o);
                    self.ranks[r] = o;
                    self.refresh_rank(r);
                }
                let after = cost(self, &mut tree, &mut buf);
                if after < before {
                    improved = true;
                } else {
                    for (i, r) in (lo..=hi).enumerate() {
                        self.ranks[r].clone_from(&saved[i]);
                        self.refresh_rank(r);
                    }
                }
            }
            if !improved {
                break;
            }
        }
        self.work = work;
        self.hier = Some(hier);
        Ok(())
    }

    /// Orders that differ from `base` by exchanging one pair of neighbouring sibling
    /// clusters in all their shared ranks (alternatives for the width-based choice).
    fn swap_variants(&mut self, base: &[Vec<u32>], limit: usize) -> Vec<Vec<Vec<u32>>> {
        let Some(hier) = self.hier.take() else {
            return Vec::new();
        };
        let mut work = std::mem::take(&mut self.work);
        self.ranks = base.to_vec();
        self.refresh_pos();
        let mut pairs: Vec<(u32, u32)> = Vec::new();
        for r in 0..self.ranks.len() {
            work.build(&self.ranks[r], &hier);
            work.adjacent_clusters(&mut pairs);
        }
        pairs.sort_unstable();
        pairs.dedup();
        let mut res = Vec::new();
        for &(a, b) in pairs.iter().take(limit) {
            let (ta, ba) = self.cl_range[a as usize];
            let (tb, bb) = self.cl_range[b as usize];
            let (lo, hi) = (ta.max(tb) as usize, ba.min(bb) as usize);
            if lo > hi || hi >= self.ranks.len() {
                continue;
            }
            for r in lo..=hi {
                let layer = std::mem::take(&mut self.ranks[r]);
                work.build(&layer, &hier);
                work.swap_clusters(a, b);
                let mut o = layer;
                o.clear();
                work.emit(0, Mode::Plain, &hier.back, &mut o);
                self.ranks[r] = o;
            }
            res.push(self.ranks.clone());
            self.ranks = base.to_vec();
        }
        self.refresh_pos();
        self.work = work;
        self.hier = Some(hier);
        res
    }

    /// Moves back-edge dummies to the outer end of their group when it costs no crossings.
    fn outward(&mut self, cancel: &AtomicBool) -> Result<(), LayoutError> {
        let any = self
            .hier
            .as_ref()
            .is_some_and(|h| h.back.iter().any(|&b| b));
        if !any {
            return Ok(());
        }
        let mut tree = Vec::new();
        let mut buf = Vec::new();
        for r in 0..self.ranks.len() {
            if r % 64 == 0 {
                check_cancel(cancel)?;
            }
            let local = |s: &Self, tree: &mut Vec<u32>, buf: &mut Vec<u32>| -> u64 {
                let mut c = 0;
                if r > 0 {
                    c += s.count_pair(r - 1, tree, buf);
                }
                if r + 1 < s.ranks.len() {
                    c += s.count_pair(r, tree, buf);
                }
                c
            };
            let before = local(self, &mut tree, &mut buf);
            let old = self.ranks[r].clone();
            self.regroup(r, Mode::Outward);
            if self.ranks[r] == old {
                continue;
            }
            let after = local(self, &mut tree, &mut buf);
            if after > before {
                self.ranks[r] = old;
                self.refresh_rank(r);
            }
        }
        Ok(())
    }

    /// Crossings of the pair (u left of v, v left of u) against both adjacent ranks.
    fn pair_cross(&self, u: u32, v: u32, a: &mut Vec<u32>, b: &mut Vec<u32>) -> (u64, u64) {
        let mut c_uv = 0u64;
        let mut c_vu = 0u64;
        for use_up in [true, false] {
            let (ru, rv) = if use_up {
                (self.up.row(u), self.up.row(v))
            } else {
                (self.down.row(u), self.down.row(v))
            };
            if ru.is_empty() || rv.is_empty() {
                continue;
            }
            a.clear();
            a.extend(ru.iter().map(|&(w, _)| self.pos[w as usize]));
            b.clear();
            b.extend(rv.iter().map(|&(w, _)| self.pos[w as usize]));
            a.sort_unstable();
            b.sort_unstable();
            let (mut lt, mut le) = (0usize, 0usize);
            for &x in a.iter() {
                while lt < b.len() && b[lt] < x {
                    lt += 1;
                }
                while le < b.len() && b[le] <= x {
                    le += 1;
                }
                c_uv += lt as u64;
                c_vu += (b.len() - le) as u64;
            }
        }
        (c_uv, c_vu)
    }

    /// A loose node next to a cluster block of its container may hop over the whole
    /// block when that lowers the crossings (keeps every block contiguous).
    fn hop_block(&mut self, r: usize, i: usize, a: &mut Vec<u32>, b: &mut Vec<u32>) -> bool {
        let Some(h) = &self.hier else {
            return false;
        };
        let layer = &self.ranks[r];
        let (x, y) = (layer[i], layer[i + 1]);
        // (loose node index, block start, block end) with the loose node left of the block.
        let find = |loose: usize, start: usize, forward: bool| -> Option<(usize, usize)> {
            let l = layer[loose];
            let first = layer[start];
            let want_side = if forward { 1 } else { 2 };
            if h.side[l as usize] != 0 || h.side[first as usize] != want_side {
                return None;
            }
            let ac = h.cluster[first as usize];
            if ac == NONE || h.parent[ac as usize] != h.cluster[l as usize] {
                return None;
            }
            let mut j = start;
            loop {
                let w = layer[j];
                if h.side[w as usize] == 3 - want_side && h.cluster[w as usize] == ac {
                    break;
                }
                if forward {
                    j += 1;
                    if j >= layer.len() {
                        return None;
                    }
                } else {
                    if j == 0 {
                        return None;
                    }
                    j -= 1;
                }
            }
            Some(if forward { (start, j) } else { (j, start) })
        };
        let (lo, hi, loose_left) = if let Some((s0, e0)) = find(i, i + 1, true) {
            (s0, e0, true)
        } else if let Some((s0, e0)) = find(i + 1, i, false) {
            (s0, e0, false)
        } else {
            return false;
        };
        let u = if loose_left { x } else { y };
        let (mut c_left, mut c_right) = (0u64, 0u64);
        for k in lo..=hi {
            let w = self.ranks[r][k];
            let (c_uw, c_wu) = self.pair_cross(u, w, a, b);
            c_left += c_uw;
            c_right += c_wu;
        }
        let layer = &mut self.ranks[r];
        if loose_left && c_right < c_left {
            layer.remove(i);
            layer.insert(hi, u);
            for k in i..=hi {
                self.pos[self.ranks[r][k] as usize] = k as u32;
            }
            true
        } else if !loose_left && c_left < c_right {
            layer.remove(i + 1);
            layer.insert(lo, u);
            for k in lo..=i + 1 {
                self.pos[self.ranks[r][k] as usize] = k as u32;
            }
            true
        } else {
            false
        }
    }

    fn transpose(&mut self, max_passes: usize, cancel: &AtomicBool) -> Result<(), LayoutError> {
        let mut a = Vec::new();
        let mut b = Vec::new();
        for _ in 0..max_passes {
            check_cancel(cancel)?;
            let mut improved = false;
            for r in 0..self.ranks.len() {
                for i in 0..self.ranks[r].len().saturating_sub(1) {
                    let (u, v) = (self.ranks[r][i], self.ranks[r][i + 1]);
                    if let Some(h) = &self.hier
                        && (h.side[u as usize] != 0
                            || h.side[v as usize] != 0
                            || h.cluster[u as usize] != h.cluster[v as usize])
                    {
                        if self.hop_block(r, i, &mut a, &mut b) {
                            improved = true;
                        }
                        continue;
                    }
                    let (c_uv, c_vu) = self.pair_cross(u, v, &mut a, &mut b);
                    if c_vu < c_uv {
                        self.ranks[r].swap(i, i + 1);
                        self.pos[u as usize] = i as u32 + 1;
                        self.pos[v as usize] = i as u32;
                        improved = true;
                    }
                }
            }
            if !improved {
                break;
            }
        }
        Ok(())
    }
}

/// Total number of edge crossings of the current ordering.
pub(crate) fn crossings(lg: &LGraph) -> u64 {
    let n = lg.nodes.len();
    let mut downs = Vec::with_capacity(lg.edges.len());
    for e in &lg.edges {
        if lg.nodes[e.to as usize].rank == lg.nodes[e.from as usize].rank + 1 {
            downs.push((e.from, e.to, ()));
        }
    }
    let mut pos = vec![0u32; n];
    for layer in &lg.ranks {
        for (i, &v) in layer.iter().enumerate() {
            pos[v as usize] = i as u32;
        }
    }
    count_all(&lg.ranks, &pos, &Csr::build(n, &downs))
}

/// Fills `lg.ranks` (ordered) and `LNode::order`. `seed` holds, per real node, the
/// cross-axis coordinate it had in a previous layout (stable re-layout).
pub(crate) fn order(
    lg: &mut LGraph,
    seed: Option<&[Option<f32>]>,
    cancel: &AtomicBool,
) -> Result<(), LayoutError> {
    let n = lg.nodes.len();
    if n == 0 {
        return Ok(());
    }
    match seed {
        Some(s) if s.iter().any(Option::is_some) => init_seeded(lg, s),
        _ => init_order(lg),
    }
    let mut layers = Layers::new(lg);
    let init_ranks = layers.ranks.clone();
    let starts: &[u8] = if layers.clustered {
        if n <= 3_000 { &[1, 2, 3, 4] } else { &[1] }
    } else {
        &[0]
    };
    let sweeps = if n <= 500 {
        24
    } else if n <= 5_000 {
        8
    } else {
        4
    };
    let transpose_passes = if n <= 5_000 { 8 } else { 2 };
    let mut scratch = Vec::new();
    let mut best_c = u64::MAX;
    let mut cands: Vec<(u64, Vec<Vec<u32>>)> = Vec::new();
    for &mode in starts {
        let mut sbest: Vec<Vec<u32>> = Vec::new();
        let mut sbest_c;
        if mode != starts[0] {
            layers.ranks.clone_from(&init_ranks);
            layers.refresh_pos();
        }
        if layers.clustered {
            layers.normalise(mode);
        }
        let c0 = layers.count();
        sbest_c = c0;
        sbest.clone_from(&layers.ranks);
        if c0 < best_c {
            best_c = c0;
        }
        let mut stale = 0;
        for it in 0..sweeps {
            check_cancel(cancel)?;
            if layers.clustered {
                layers.update_gkeys(0);
            }
            let first = usize::from(!layers.clustered);
            if it % 2 == 0 {
                for r in first..layers.ranks.len() {
                    layers.reorder(r, true, &mut scratch);
                }
            } else {
                let last = layers
                    .ranks
                    .len()
                    .saturating_sub(usize::from(!layers.clustered));
                for r in (0..last).rev() {
                    layers.reorder(r, false, &mut scratch);
                }
            }
            layers.transpose(transpose_passes, cancel)?;
            let c = layers.count();
            if n <= 1_500 && !cands.iter().any(|(_, r)| *r == layers.ranks) {
                cands.push((c, layers.ranks.clone()));
            }
            if c < sbest_c {
                sbest_c = c;
                sbest.clone_from(&layers.ranks);
                stale = 0;
                if c < best_c {
                    best_c = c;
                }
                if c == 0 {
                    break;
                }
            } else {
                stale += 1;
                if stale >= 4 {
                    break;
                }
            }
        }
        if !cands.iter().any(|(_, r)| *r == sbest) {
            cands.push((sbest_c, sbest));
        }
        if best_c == 0 {
            break;
        }
    }
    // Best candidate first; the others are alternatives the caller may try.
    cands.sort_by_key(|c| c.0);
    cands.truncate(10);
    let mut finals: Vec<Vec<Vec<u32>>> = Vec::new();
    for (_, ranks) in cands {
        layers.ranks = ranks;
        layers.refresh_pos();
        if layers.clustered && n <= 3_000 {
            layers.cluster_swaps(6, cancel)?;
        }
        layers.outward(cancel)?;
        finals.push(layers.ranks.clone());
    }
    if layers.clustered && n <= 1_500 {
        let base = finals[0].clone();
        let extra = layers.swap_variants(&base, 12);
        for v in extra {
            if !finals.contains(&v) {
                finals.push(v);
            }
        }
    }
    let first = finals.remove(0);
    for (r, layer) in first.iter().enumerate() {
        for (i, &v) in layer.iter().enumerate() {
            lg.nodes[v as usize].order = i as u32;
        }
        lg.ranks[r].clone_from(layer);
    }
    lg.alt_orders = finals;
    Ok(())
}

/// Initial order from a previous layout: seeded nodes by their old cross coordinate,
/// others by the barycentre of already keyed neighbours, then by index.
fn init_seeded(lg: &mut LGraph, seed: &[Option<f32>]) {
    let n = lg.nodes.len();
    let mut key = vec![f32::NAN; n];
    for (i, s) in seed.iter().enumerate().take(n) {
        if let Some(x) = s {
            key[i] = *x;
        }
    }
    let mut ups = Vec::new();
    let mut downs = Vec::new();
    for e in &lg.edges {
        if lg.nodes[e.to as usize].rank == lg.nodes[e.from as usize].rank + 1 {
            ups.push((e.to, e.from, ()));
            downs.push((e.from, e.to, ()));
        }
    }
    let up = Csr::build(n, &ups);
    let down = Csr::build(n, &downs);
    let avg = |row: &[(u32, ())], key: &[f32]| -> Option<f32> {
        let (mut s, mut c) = (0.0f32, 0u32);
        for &(w, _) in row {
            let k = key[w as usize];
            if !k.is_nan() {
                s += k;
                c += 1;
            }
        }
        (c > 0).then(|| s / c as f32)
    };
    for r in 0..lg.ranks.len() {
        for &v in &lg.ranks[r] {
            if key[v as usize].is_nan()
                && let Some(k) = avg(up.row(v), &key)
            {
                key[v as usize] = k;
            }
        }
    }
    for r in (0..lg.ranks.len()).rev() {
        for &v in &lg.ranks[r] {
            if key[v as usize].is_nan()
                && let Some(k) = avg(down.row(v), &key)
            {
                key[v as usize] = k;
            }
        }
    }
    for layer in &mut lg.ranks {
        layer.sort_by(|&a, &b| {
            let (ka, kb) = (key[a as usize], key[b as usize]);
            let (ka, kb) = (
                if ka.is_nan() { f32::MAX } else { ka },
                if kb.is_nan() { f32::MAX } else { kb },
            );
            ka.total_cmp(&kb).then(a.cmp(&b))
        });
    }
    for layer in &lg.ranks {
        for (i, &v) in layer.iter().enumerate() {
            lg.nodes[v as usize].order = i as u32;
        }
    }
}

/// Initial order: DFS along out-edges from every unvisited node, sorted by (rank, index).
fn init_order(lg: &mut LGraph) {
    let n = lg.nodes.len();
    let mut outs = Vec::with_capacity(lg.edges.len());
    for e in &lg.edges {
        outs.push((e.from, e.to, ()));
    }
    let down = Csr::build(n, &outs);
    let mut roots: Vec<u32> = (0..n as u32).collect();
    roots.sort_by_key(|&v| (lg.nodes[v as usize].rank, v));
    let mut seen = vec![false; n];
    for layer in &mut lg.ranks {
        layer.clear();
    }
    let mut stack: Vec<(u32, usize)> = Vec::new();
    for &root in &roots {
        if seen[root as usize] {
            continue;
        }
        seen[root as usize] = true;
        lg.ranks[lg.nodes[root as usize].rank as usize].push(root);
        stack.push((root, 0));
        while let Some(top) = stack.len().checked_sub(1) {
            let (v, i) = stack[top];
            let row = down.row(v);
            if i < row.len() {
                stack[top].1 += 1;
                let w = row[i].0;
                if !seen[w as usize] {
                    seen[w as usize] = true;
                    lg.ranks[lg.nodes[w as usize].rank as usize].push(w);
                    stack.push((w, 0));
                }
            } else {
                stack.pop();
            }
        }
    }
    for layer in &lg.ranks {
        for (i, &v) in layer.iter().enumerate() {
            lg.nodes[v as usize].order = i as u32;
        }
    }
}
