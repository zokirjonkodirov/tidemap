//! Layered (Sugiyama-style) layout for flowcharts.
//!
//! Pipeline: acyclic -> rank -> dummies -> order -> position -> route.
//! Everything is computed in a direction-normalised space (top to bottom, "cross"
//! axis = x, "rank" axis = y) and transformed once at the end with [`Frame`].

mod acyclic;
mod compound;
mod order;
mod pack;
mod par;
mod position;
mod rank;
mod route;
mod timing;

pub use par::Par;
pub use timing::{Stage, StageTimes};

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::graph::{ClusterId, Direction, EdgeId, Endpoint, Graph, Node, NodeId, ShapeKind};
use crate::metrics::Metrics;
use crate::shape::{self, Rect};
use crate::text::{Measurer, TextBlock};

/// Width (px) one crossing is worth when choosing between node orders.
const CROSS_PX: f32 = 12.0;
/// Work (working-graph nodes times alternative orders) one thread needs before it is
/// worth positioning alternatives concurrently. Measured on a 28-thread desktop CPU (best
/// of 15, random layered graphs): 34 nodes with 5 alternatives (170) was 5-35% slower
/// concurrently, 48 nodes (about 290) broke even, 78 nodes with 8 alternatives (620) was
/// 30% faster, 450 nodes with 12 alternatives (5,400) 2x faster.
const PAR_ALT_WORK_PER_WORKER: usize = 150;
/// Work (nodes plus edges of the sub-graphs) one thread needs before laying out
/// components or direction-changing clusters concurrently. Measured as above on graphs of
/// disconnected trees: 114 units was 10% slower with 2 threads and 3x slower with 28,
/// 190 units 25% faster with 2 threads, 3,000 units 3x faster with 8 threads.
const PAR_PARTS_WORK_PER_WORKER: usize = 150;

/// Node budget.
pub const MAX_NODES: usize = 20_000;
/// Edge budget.
pub const MAX_EDGES: usize = 20_000;
/// World size budget per side.
pub const MAX_WORLD: f32 = 10_000_000.0;

/// Why a layout did not complete.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutError {
    Canceled,
    Budget(String),
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutError::Canceled => write!(f, "layout canceled"),
            LayoutError::Budget(s) => write!(f, "layout budget exceeded: {s}"),
        }
    }
}

impl std::error::Error for LayoutError {}

/// Routed edge: cubic Bézier chain (1 + 3k points) from the authored source to the
/// authored target, plus the label rectangle if any. Empty for unrouted edges.
#[derive(Clone, Debug)]
pub struct EdgePath {
    pub points: Vec<[f32; 2]>,
    pub label: Option<Rect>,
}

/// Positioned diagram.
#[derive(Clone, Debug)]
pub struct Layout {
    pub nodes: Vec<Rect>,
    pub node_text: Vec<TextBlock>,
    /// Per node: geometry of a `Record` box.
    pub records: Vec<Option<std::sync::Arc<shape::Record>>>,
    pub clusters: Vec<Rect>,
    pub edges: Vec<EdgePath>,
    pub edge_text: Vec<Option<TextBlock>>,
    pub bounds: Rect,
    /// Free-form primitives of chart-style diagrams.
    pub decor: Vec<crate::decor::Decor>,
    /// Per node: (rank, position in rank), the stability seed.
    pub slots: Vec<(u32, u32)>,
}

/// Kind of a working-graph node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LKind {
    Real(NodeId),
    Dummy(EdgeId),
    Label(EdgeId),
    BorderTop(ClusterId),
    BorderBottom(ClusterId),
    BorderLeft(ClusterId),
    BorderRight(ClusterId),
}

impl LKind {
    /// Dummy or edge-label node (part of a long edge).
    pub(crate) fn is_edge_part(self) -> bool {
        matches!(self, LKind::Dummy(_) | LKind::Label(_))
    }

    /// Edge part or side border: segments between two such nodes are "inner" segments.
    pub(crate) fn is_inner_part(self) -> bool {
        matches!(
            self,
            LKind::Dummy(_) | LKind::Label(_) | LKind::BorderLeft(_) | LKind::BorderRight(_)
        )
    }
}

/// Working-graph node. `x` is the cross-axis centre, `y` the rank-axis centre.
#[derive(Clone, Debug)]
pub(crate) struct LNode {
    pub kind: LKind,
    pub w: f32,
    pub h: f32,
    pub rank: i32,
    pub order: u32,
    pub x: f32,
    pub y: f32,
    /// Cluster this node is a direct member of (ordering container).
    pub cluster: Option<ClusterId>,
}

/// Working-graph edge. After `insert_dummies` every edge spans exactly one rank and
/// the chain of one authored edge is contiguous and ordered in `LGraph::edges`.
#[derive(Clone, Debug)]
pub(crate) struct LEdge {
    pub from: u32,
    pub to: u32,
    pub orig: EdgeId,
    pub reversed: bool,
    pub weight: u32,
    pub min_len: u32,
}

/// Working graph. The first `Graph::nodes.len()` nodes are `LKind::Real` in order.
#[derive(Clone, Debug, Default)]
pub(crate) struct LGraph {
    pub nodes: Vec<LNode>,
    pub edges: Vec<LEdge>,
    pub ranks: Vec<Vec<u32>>,
    /// Cluster forest (empty for flat graphs).
    pub tree: compound::Tree,
    /// Per cluster: top / bottom border node (the same node for an empty cluster).
    pub cl_top: Vec<u32>,
    pub cl_bot: Vec<u32>,
    pub cl_leaf: Vec<bool>,
    /// Per cluster: side border nodes, one per spanned rank.
    pub cl_left: Vec<Vec<u32>>,
    pub cl_right: Vec<Vec<u32>>,
    /// Layout direction (decides on which side cluster titles need room).
    pub dir: Direction,
    /// Title width per cluster.
    pub title_w: Vec<f32>,
    /// Chains may run inside the clusters of their end nodes.
    pub chains_in_clusters: bool,
    /// Set by `position` when straight cluster columns could not be kept.
    pub forced_failed: bool,
    /// Alternative node orders (`ranks`-shaped) to try when positioning small graphs.
    pub alt_orders: Vec<Vec<Vec<u32>>>,
}

/// Compressed adjacency rows with a payload per entry.
pub(crate) struct Csr<T> {
    off: Vec<u32>,
    data: Vec<(u32, T)>,
}

impl<T: Copy + Default> Csr<T> {
    /// Builds rows `0..n` from `(row, neighbour, payload)` triples, keeping input order.
    pub(crate) fn build(n: usize, edges: &[(u32, u32, T)]) -> Self {
        let mut off = vec![0u32; n + 1];
        for e in edges {
            off[e.0 as usize + 1] += 1;
        }
        for i in 0..n {
            off[i + 1] += off[i];
        }
        let mut fill: Vec<u32> = off[..n].to_vec();
        let mut data = vec![(0u32, T::default()); edges.len()];
        for e in edges {
            let slot = &mut fill[e.0 as usize];
            data[*slot as usize] = (e.1, e.2);
            *slot += 1;
        }
        Self { off, data }
    }

    pub(crate) fn row(&self, v: u32) -> &[(u32, T)] {
        &self.data[self.off[v as usize] as usize..self.off[v as usize + 1] as usize]
    }
}

/// Maps between normalised (cross, rank) space and final (x, y) space.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Frame {
    pub dir: Direction,
    /// Total length of the rank axis (used for mirroring).
    pub extent: f32,
}

impl Frame {
    /// True when the final width/height are swapped relative to normalised space.
    pub(crate) fn swaps(&self) -> bool {
        matches!(self.dir, Direction::LR | Direction::RL)
    }

    /// Normalised `[cross, rank]` to final `[x, y]`.
    pub(crate) fn map(&self, p: [f32; 2]) -> [f32; 2] {
        let [c, r] = p;
        match self.dir {
            Direction::TB => [c, r],
            Direction::BT => [c, self.extent - r],
            Direction::LR => [r, c],
            Direction::RL => [self.extent - r, c],
        }
    }

    /// Final `[x, y]` to normalised `[cross, rank]`.
    pub(crate) fn unmap(&self, p: [f32; 2]) -> [f32; 2] {
        let [x, y] = p;
        match self.dir {
            Direction::TB => [x, y],
            Direction::BT => [x, self.extent - y],
            Direction::LR => [y, x],
            Direction::RL => [y, self.extent - x],
        }
    }
}

pub(crate) fn check_cancel(cancel: &AtomicBool) -> Result<(), LayoutError> {
    if cancel.load(Ordering::Relaxed) {
        Err(LayoutError::Canceled)
    } else {
        Ok(())
    }
}

use compound::{NONE, Tree};

/// Settings shared by every stage of one layout call.
#[derive(Clone, Copy)]
struct Run<'a> {
    m: &'a Metrics,
    cancel: &'a AtomicBool,
    /// Optional per-stage timing sink (benchmarks only).
    times: Option<&'a StageTimes>,
    /// Threads this level may use.
    par: Par,
}

/// Everything the layout stages need for one (sub)graph. Dimensions are in final
/// orientation.
struct Inputs<'a> {
    /// Borrowed for the whole diagram, owned for sub-graphs built by `sub_inputs`.
    g: Cow<'a, Graph>,
    dims: Vec<[f32; 2]>,
    labels: Vec<Option<[f32; 2]>>,
    title_w: Vec<f32>,
    seed_x: Vec<Option<f32>>,
}

/// Layout of one (sub)graph, translated so its bounds start at (0, 0).
struct Core {
    rects: Vec<Rect>,
    clusters: Vec<Rect>,
    edges: Vec<EdgePath>,
    slots: Vec<(u32, u32)>,
    size: [f32; 2],
}

impl Core {
    fn empty(n: usize, k: usize, ne: usize) -> Self {
        Self {
            rects: vec![Rect::default(); n],
            clusters: vec![Rect::default(); k],
            edges: vec![
                EdgePath {
                    points: Vec::new(),
                    label: None
                };
                ne
            ],
            slots: vec![(0, 0); n],
            size: [0.0, 0.0],
        }
    }

    fn translate(&mut self, dx: f32, dy: f32) {
        for r in self.rects.iter_mut().chain(self.clusters.iter_mut()) {
            r.x += dx;
            r.y += dy;
        }
        for e in &mut self.edges {
            shift_path(e, dx, dy);
        }
    }
}

fn shift_path(e: &mut EdgePath, dx: f32, dy: f32) {
    if let Some(l) = &mut e.label {
        l.x += dx;
        l.y += dy;
    }
    for p in &mut e.points {
        p[0] += dx;
        p[1] += dy;
    }
}

fn empty_path() -> EdgePath {
    EdgePath {
        points: Vec::new(),
        label: None,
    }
}

/// Lays out `g`. With a `seed` (previous graph and its layout) nodes keep their
/// relative left-to-right order unless a reorder removes crossings.
pub fn layout(
    g: &Graph,
    measurer: &Measurer,
    m: &Metrics,
    seed: Option<(&Graph, &Layout)>,
    cancel: &AtomicBool,
) -> Result<Layout, LayoutError> {
    layout_with(g, measurer, m, seed, cancel, None, Par::auto())
}

/// Like [`layout`], with an explicit thread budget and an optional sink that is
/// charged the time of each pipeline stage. The result does not depend on `par`.
pub fn layout_with(
    g: &Graph,
    measurer: &Measurer,
    m: &Metrics,
    seed: Option<(&Graph, &Layout)>,
    cancel: &AtomicBool,
    times: Option<&StageTimes>,
    par: Par,
) -> Result<Layout, LayoutError> {
    if let Some(chart) = &g.chart {
        return crate::charts::layout(chart, measurer, m, cancel);
    }
    if let Some(model) = &g.seq {
        return crate::seq::layout(g, model, measurer, m, cancel);
    }
    let n = g.nodes.len();
    if n > MAX_NODES {
        return Err(LayoutError::Budget(format!(
            "{n} nodes exceed the limit of {MAX_NODES}"
        )));
    }
    if g.edges.len() > MAX_EDGES {
        return Err(LayoutError::Budget(format!(
            "{} edges exceed the limit of {MAX_EDGES}",
            g.edges.len()
        )));
    }
    if Tree::new(g).max_depth() > compound::MAX_DEPTH {
        return Err(LayoutError::Budget(
            "subgraphs are nested too deeply".into(),
        ));
    }
    check_cancel(cancel)?;

    let swap = matches!(g.direction, Direction::LR | Direction::RL);

    // Text and sizes (final orientation). Widths snap up so similar labels match.
    let mut node_text = Vec::with_capacity(n);
    let mut records: Vec<Option<std::sync::Arc<shape::Record>>> = vec![None; n];
    let mut dims: Vec<[f32; 2]> = Vec::with_capacity(n);
    for (i, node) in g.nodes.iter().enumerate() {
        if i % 512 == 0 {
            check_cancel(cancel)?;
        }
        if node.shape == ShapeKind::Record && !node.sections.is_empty() {
            let lines: Vec<String> = node.label.split('\n').map(str::to_owned).collect();
            let rec = shape::record(&lines, &node.sections, measurer, m);
            dims.push(rec.size);
            let flat: Vec<String> = lines.iter().map(|l| l.replace('\t', "  ")).collect();
            node_text.push(TextBlock {
                width: rec.size[0],
                height: rec.size[1],
                lines: flat,
            });
            records[i] = Some(std::sync::Arc::new(rec));
            continue;
        }
        let tb = measurer.wrap(&node.label, m.font_size, m.line_height, m.max_label_width);
        let mut d = shape::size(node.shape, &tb, m);
        if node.shape == ShapeKind::Bar && swap {
            d = [d[1], d[0]];
        }
        if !matches!(
            node.shape,
            ShapeKind::Circle
                | ShapeKind::DoubleCircle
                | ShapeKind::StateStart
                | ShapeKind::StateEnd
                | ShapeKind::Bar
                | ShapeKind::Choice
                | ShapeKind::Record
        ) {
            d[0] = pack::snap_width(d[0], m.width_step);
        }
        dims.push(d);
        node_text.push(tb);
    }
    let mut edge_text: Vec<Option<TextBlock>> = Vec::with_capacity(g.edges.len());
    let mut labels: Vec<Option<[f32; 2]>> = Vec::with_capacity(g.edges.len());
    for (i, e) in g.edges.iter().enumerate() {
        if i % 512 == 0 {
            check_cancel(cancel)?;
        }
        match &e.label {
            Some(l) if !l.is_empty() => {
                let f = m.edge_label_font;
                let tb = measurer.wrap(l, f, f + 5.0, 160.0);
                labels.push(Some([tb.width + 12.0, tb.height + 6.0]));
                edge_text.push(Some(tb));
            }
            _ => {
                labels.push(None);
                edge_text.push(None);
            }
        }
    }
    let title_w: Vec<f32> = g
        .clusters
        .iter()
        .map(|c| measurer.width(&c.title, 13.0) * 1.06)
        .collect();

    // Stable re-layout: cross-axis coordinate each node had before (same direction only).
    let mut seed_x: Vec<Option<f32>> = vec![None; n];
    if let Some((og, ol)) = seed
        && ol.nodes.len() == og.nodes.len()
        && og.direction == g.direction
    {
        let map: std::collections::HashMap<&str, usize> = og
            .nodes
            .iter()
            .enumerate()
            .map(|(i, nd)| (nd.key.as_str(), i))
            .collect();
        for (i, nd) in g.nodes.iter().enumerate() {
            if let Some(&j) = map.get(nd.key.as_str()) {
                let c = ol.nodes[j].center();
                seed_x[i] = Some(if swap { c[1] } else { c[0] });
            }
        }
    }

    let inp = Inputs {
        g: Cow::Borrowed(g),
        dims,
        labels,
        title_w,
        seed_x,
    };
    let run = Run {
        m,
        cancel,
        times,
        par,
    };
    let core = layout_graph(&inp, &run)?;
    if core.size[0] > MAX_WORLD || core.size[1] > MAX_WORLD {
        return Err(LayoutError::Budget("diagram exceeds the world size".into()));
    }
    Ok(Layout {
        nodes: core.rects,
        node_text,
        records,
        clusters: core.clusters,
        edges: core.edges,
        edge_text,
        bounds: Rect {
            x: 0.0,
            y: 0.0,
            w: core.size[0],
            h: core.size[1],
        },
        decor: Vec::new(),
        slots: core.slots,
    })
}

/// Splits into weakly connected components (a cluster and everything in it count as
/// connected), lays each out and packs the results.
fn layout_graph(inp: &Inputs, run: &Run) -> Result<Core, LayoutError> {
    let (m, cancel) = (run.m, run.cancel);
    let g = &inp.g;
    let n = g.nodes.len();
    let k = g.clusters.len();
    let ne = g.edges.len();
    if n + k == 0 {
        return Ok(Core::empty(0, 0, ne));
    }
    check_cancel(cancel)?;
    let mut parent: Vec<u32> = (0..(n + k) as u32).collect();
    fn find(p: &mut [u32], mut v: u32) -> u32 {
        while p[v as usize] != v {
            p[v as usize] = p[p[v as usize] as usize];
            v = p[v as usize];
        }
        v
    }
    let union = |p: &mut Vec<u32>, a: usize, b: usize| {
        let (ra, rb) = (find(p, a as u32), find(p, b as u32));
        if ra != rb {
            p[ra.max(rb) as usize] = ra.min(rb);
        }
    };
    let item = |ep: Endpoint| -> Option<usize> {
        match ep {
            Endpoint::Node(a) if (a.0 as usize) < n => Some(a.0 as usize),
            Endpoint::Cluster(c) if (c.0 as usize) < k => Some(n + c.0 as usize),
            _ => None,
        }
    };
    for (i, nd) in g.nodes.iter().enumerate() {
        if let Some(c) = nd.cluster
            && (c.0 as usize) < k
        {
            union(&mut parent, i, n + c.0 as usize);
        }
    }
    for (i, c) in g.clusters.iter().enumerate() {
        if let Some(p) = c.parent
            && (p.0 as usize) < k
        {
            union(&mut parent, n + i, n + p.0 as usize);
        }
    }
    for e in &g.edges {
        if let (Some(a), Some(b)) = (item(e.from), item(e.to)) {
            union(&mut parent, a, b);
        }
    }
    let mut comp_of_root = vec![u32::MAX; n + k];
    let mut comps = 0usize;
    let mut comp = vec![0u32; n + k];
    for v in 0..n + k {
        let r = find(&mut parent, v as u32) as usize;
        if comp_of_root[r] == u32::MAX {
            comp_of_root[r] = comps as u32;
            comps += 1;
        }
        comp[v] = comp_of_root[r];
    }
    if comps == 1 {
        return core_one(inp, run);
    }
    let mut c_nodes: Vec<Vec<u32>> = vec![Vec::new(); comps];
    let mut c_clusters: Vec<Vec<u32>> = vec![Vec::new(); comps];
    let mut c_edges: Vec<Vec<u32>> = vec![Vec::new(); comps];
    for v in 0..n {
        c_nodes[comp[v] as usize].push(v as u32);
    }
    for c in 0..k {
        c_clusters[comp[n + c] as usize].push(c as u32);
    }
    for (i, e) in g.edges.iter().enumerate() {
        if let (Some(a), Some(_)) = (item(e.from), item(e.to)) {
            c_edges[comp[a] as usize].push(i as u32);
        }
    }
    let parts: Vec<Part> = (0..comps)
        .map(|ci| Part {
            nodes: &c_nodes[ci],
            clusters: &c_clusters[ci],
            edges: &c_edges[ci],
            dir: g.direction,
        })
        .collect();
    let cores = layout_parts(inp, run, &parts, core_one)?;
    let sizes: Vec<[f32; 2]> = cores.iter().map(|c| c.size).collect();
    let offs = pack::pack(&sizes, m.rank_gap, 1.6);
    let mut out = Core::empty(n, k, ne);
    let (mut tw, mut th) = (0.0f32, 0.0f32);
    for (ci, mut core) in cores.into_iter().enumerate() {
        core.translate(offs[ci][0], offs[ci][1]);
        tw = tw.max(offs[ci][0] + core.size[0]);
        th = th.max(offs[ci][1] + core.size[1]);
        for (i, &v) in c_nodes[ci].iter().enumerate() {
            out.rects[v as usize] = core.rects[i];
            out.slots[v as usize] = core.slots[i];
        }
        for (i, &c) in c_clusters[ci].iter().enumerate() {
            out.clusters[c as usize] = core.clusters[i];
        }
        for (i, &e) in c_edges[ci].iter().enumerate() {
            out.edges[e as usize] = std::mem::replace(&mut core.edges[i], empty_path());
        }
    }
    out.size = [tw, th];
    Ok(out)
}

/// Builds the inputs of the sub-graph made of the given nodes, clusters and edges.
fn sub_inputs(
    inp: &Inputs,
    nodes: &[u32],
    clusters: &[u32],
    edges: &[u32],
    dir: Direction,
    nmap: &mut [u32],
    cmap: &mut [u32],
) -> Inputs<'static> {
    let g = &inp.g;
    for (i, &v) in nodes.iter().enumerate() {
        nmap[v as usize] = i as u32;
    }
    for (i, &c) in clusters.iter().enumerate() {
        cmap[c as usize] = i as u32;
    }
    let cl = |c: Option<ClusterId>| -> Option<ClusterId> {
        c.and_then(|c| match cmap.get(c.0 as usize) {
            Some(&x) if x != NONE => Some(ClusterId(x)),
            _ => None,
        })
    };
    let sub_nodes: Vec<Node> = nodes
        .iter()
        .map(|&v| {
            let mut nd = g.nodes[v as usize].clone();
            nd.cluster = cl(nd.cluster);
            nd
        })
        .collect();
    let sub_clusters = clusters
        .iter()
        .map(|&c| {
            let mut cc = g.clusters[c as usize].clone();
            cc.parent = cl(cc.parent);
            cc
        })
        .collect();
    let ep = |e: Endpoint| -> Option<Endpoint> {
        match e {
            Endpoint::Node(a) => nmap
                .get(a.0 as usize)
                .filter(|&&x| x != NONE)
                .map(|&x| Endpoint::Node(NodeId(x))),
            Endpoint::Cluster(c) => cmap
                .get(c.0 as usize)
                .filter(|&&x| x != NONE)
                .map(|&x| Endpoint::Cluster(ClusterId(x))),
        }
    };
    let mut sub_edges = Vec::with_capacity(edges.len());
    let mut kept: Vec<u32> = Vec::with_capacity(edges.len());
    for &e in edges {
        let src = &g.edges[e as usize];
        if let (Some(f), Some(t)) = (ep(src.from), ep(src.to)) {
            let mut ed = src.clone();
            ed.from = f;
            ed.to = t;
            sub_edges.push(ed);
            kept.push(e);
        }
    }
    let out = Inputs {
        g: Cow::Owned(Graph {
            direction: dir,
            nodes: sub_nodes,
            edges: sub_edges,
            clusters: sub_clusters,
            class_defs: Vec::new(),
            seq: None,
            chart: None,
        }),
        dims: nodes.iter().map(|&v| inp.dims[v as usize]).collect(),
        labels: kept.iter().map(|&e| inp.labels[e as usize]).collect(),
        title_w: clusters.iter().map(|&c| inp.title_w[c as usize]).collect(),
        seed_x: nodes.iter().map(|&v| inp.seed_x[v as usize]).collect(),
    };
    for &v in nodes {
        nmap[v as usize] = NONE;
    }
    for &c in clusters {
        cmap[c as usize] = NONE;
    }
    // Callers pass only edges with both endpoints inside; keep the edge list aligned.
    debug_assert_eq!(kept.len(), edges.len());
    out
}

/// Node, cluster and edge indices (into the parent inputs) of one sub-graph, and the
/// direction it is laid out in.
struct Part<'a> {
    nodes: &'a [u32],
    clusters: &'a [u32],
    edges: &'a [u32],
    dir: Direction,
}

/// Lays out independent sub-graphs of `inp` with `layout_one`. The sub-inputs are built
/// one after the other (they share scratch maps); the layouts then run concurrently when
/// the parts hold enough work. Cores come back in part order.
fn layout_parts(
    inp: &Inputs,
    run: &Run,
    parts: &[Part],
    layout_one: fn(&Inputs, &Run) -> Result<Core, LayoutError>,
) -> Result<Vec<Core>, LayoutError> {
    let mut nmap = vec![NONE; inp.g.nodes.len()];
    let mut cmap = vec![NONE; inp.g.clusters.len()];
    let mut subs: Vec<Inputs<'static>> = Vec::with_capacity(parts.len());
    for p in parts {
        check_cancel(run.cancel)?;
        subs.push(sub_inputs(
            inp, p.nodes, p.clusters, p.edges, p.dir, &mut nmap, &mut cmap,
        ));
    }
    let sizes: Vec<usize> = parts
        .iter()
        .map(|p| p.nodes.len() + p.edges.len())
        .collect();
    let work: usize = sizes.iter().sum();
    let biggest = sizes.iter().copied().max().unwrap_or(0);
    // One dominant part bounds the gain at 2x and would take the threads its own inner
    // levels (alternatives, alignments) can use better.
    let workers = if run.par.gated() && biggest * 2 > work {
        1
    } else {
        run.par.workers(subs.len(), work, PAR_PARTS_WORK_PER_WORKER)
    };
    let inner = Run {
        par: run.par.share(workers),
        ..*run
    };
    par::map_items(&subs, workers, |_, sub| {
        check_cancel(inner.cancel)?;
        layout_one(sub, &inner)
    })
}

/// Where a crossing edge ends inside a recursively laid out cluster.
struct Stub {
    orig: usize,
    pg_edge: usize,
    from_root: Option<usize>,
    to_root: Option<usize>,
}

/// One connected piece: clusters with their own direction are laid out recursively and
/// stand in as a single node, everything else goes through the layered pipeline.
fn core_one(inp: &Inputs, run: &Run) -> Result<Core, LayoutError> {
    let m = run.m;
    let g = &inp.g;
    let n = g.nodes.len();
    let k = g.clusters.len();
    let ne = g.edges.len();
    if k == 0 {
        return pipeline(inp, run);
    }
    let tree = Tree::new(g);
    let mut eff = vec![g.direction; k];
    let mut is_rec = vec![false; k];
    let mut order: Vec<usize> = (0..k).collect();
    order.sort_by_key(|&c| (tree.depth[c], c));
    for &c in &order {
        let p = tree.parent[c];
        let pd = if p == NONE {
            g.direction
        } else {
            eff[p as usize]
        };
        match g.clusters[c].direction {
            Some(d) => {
                eff[c] = d;
                is_rec[c] = d != pd;
            }
            None => eff[c] = pd,
        }
    }
    let mut absorbed = vec![NONE; k];
    for &c in &order {
        let p = tree.parent[c];
        absorbed[c] = if p != NONE && absorbed[p as usize] != NONE {
            absorbed[p as usize]
        } else if is_rec[c] {
            c as u32
        } else {
            NONE
        };
    }
    let roots: Vec<usize> = (0..k).filter(|&c| absorbed[c] == c as u32).collect();
    if roots.is_empty() {
        return pipeline(inp, run);
    }
    let mut rix = vec![usize::MAX; k];
    for (i, &c) in roots.iter().enumerate() {
        rix[c] = i;
    }
    let nabs: Vec<u32> = g
        .nodes
        .iter()
        .map(|nd| match nd.cluster {
            Some(c) if (c.0 as usize) < k => absorbed[c.0 as usize],
            _ => NONE,
        })
        .collect();
    // Class of an endpoint: the recursive root it lies in (or NONE); None if invalid.
    let class = |ep: Endpoint| -> Option<u32> {
        match ep {
            Endpoint::Node(a) if (a.0 as usize) < n => Some(nabs[a.0 as usize]),
            Endpoint::Cluster(c) if (c.0 as usize) < k => Some(absorbed[c.0 as usize]),
            _ => None,
        }
    };

    // Sub-layouts.
    let mut sub_nodes: Vec<Vec<u32>> = vec![Vec::new(); roots.len()];
    let mut sub_clusters: Vec<Vec<u32>> = vec![Vec::new(); roots.len()];
    let mut sub_edges: Vec<Vec<u32>> = vec![Vec::new(); roots.len()];
    for v in 0..n {
        if nabs[v] != NONE {
            sub_nodes[rix[nabs[v] as usize]].push(v as u32);
        }
    }
    for c in 0..k {
        if absorbed[c] != NONE && absorbed[c] != c as u32 {
            sub_clusters[rix[absorbed[c] as usize]].push(c as u32);
        }
    }
    // 0 = plain, 1 = internal to a root, 2 = dropped
    let mut kind = vec![0u8; ne];
    for (i, e) in g.edges.iter().enumerate() {
        let (Some(cf), Some(ct)) = (class(e.from), class(e.to)) else {
            kind[i] = 2;
            continue;
        };
        if cf != NONE && cf == ct {
            let own = |ep: Endpoint| matches!(ep, Endpoint::Cluster(c) if c.0 == cf);
            if own(e.from) || own(e.to) {
                kind[i] = 2;
            } else {
                kind[i] = 1;
                sub_edges[rix[cf as usize]].push(i as u32);
            }
        }
    }
    let parts: Vec<Part> = roots
        .iter()
        .enumerate()
        .map(|(ri, &c)| Part {
            nodes: &sub_nodes[ri],
            clusters: &sub_clusters[ri],
            edges: &sub_edges[ri],
            dir: g.clusters[c].direction.unwrap_or(g.direction),
        })
        .collect();
    let mut subs = layout_parts(inp, run, &parts, layout_graph)?;
    let stand: Vec<[f32; 2]> = roots
        .iter()
        .zip(&subs)
        .map(|(&c, core)| {
            let w =
                (core.size[0] + 2.0 * m.cluster_pad).max(inp.title_w[c] + compound::TITLE_INSET);
            [w, core.size[1] + m.cluster_title + 2.0 * m.cluster_pad]
        })
        .collect();

    // Parent graph with one stand-in node per recursive cluster.
    let mut pg_node_of = vec![NONE; n];
    let mut pg_cl_of = vec![NONE; k];
    let mut pg_clusters = Vec::new();
    for c in 0..k {
        if absorbed[c] == NONE {
            pg_cl_of[c] = pg_clusters.len() as u32;
            pg_clusters.push(c);
        }
    }
    let map_cl = |c: Option<ClusterId>| -> Option<ClusterId> {
        c.and_then(|c| match pg_cl_of.get(c.0 as usize) {
            Some(&x) if x != NONE => Some(ClusterId(x)),
            _ => None,
        })
    };
    let mut pnodes: Vec<Node> = Vec::new();
    let mut pdims: Vec<[f32; 2]> = Vec::new();
    let mut pseed: Vec<Option<f32>> = Vec::new();
    for v in 0..n {
        if nabs[v] == NONE {
            pg_node_of[v] = pnodes.len() as u32;
            let mut nd = g.nodes[v].clone();
            nd.cluster = map_cl(nd.cluster);
            pnodes.push(nd);
            pdims.push(inp.dims[v]);
            pseed.push(inp.seed_x[v]);
        }
    }
    let synth0 = pnodes.len();
    for (ri, &c) in roots.iter().enumerate() {
        let p = tree.parent[c];
        pnodes.push(Node {
            key: format!("\u{1}cluster{c}"),
            label: String::new(),
            shape: ShapeKind::Rect,
            cluster: map_cl((p != NONE).then_some(ClusterId(p))),
            classes: Vec::new(),
            style: Default::default(),
            span: Default::default(),
            label_span: Default::default(),
            sections: Vec::new(),
        });
        pdims.push(stand[ri]);
        pseed.push(None);
    }
    let pclusters: Vec<_> = pg_clusters
        .iter()
        .map(|&c| {
            let mut cc = g.clusters[c].clone();
            cc.parent = map_cl(cc.parent);
            cc
        })
        .collect();
    let ptitle: Vec<f32> = pg_clusters.iter().map(|&c| inp.title_w[c]).collect();
    let mut pedges = Vec::new();
    let mut plabels = Vec::new();
    let mut stubs: Vec<Stub> = Vec::new();
    let map_ep = |ep: Endpoint| -> (Endpoint, Option<usize>) {
        match ep {
            Endpoint::Node(a) => {
                let r = nabs[a.0 as usize];
                if r != NONE {
                    (
                        Endpoint::Node(NodeId((synth0 + rix[r as usize]) as u32)),
                        Some(rix[r as usize]),
                    )
                } else {
                    (Endpoint::Node(NodeId(pg_node_of[a.0 as usize])), None)
                }
            }
            Endpoint::Cluster(c) => {
                let r = absorbed[c.0 as usize];
                if r != NONE {
                    (
                        Endpoint::Node(NodeId((synth0 + rix[r as usize]) as u32)),
                        Some(rix[r as usize]),
                    )
                } else {
                    (Endpoint::Cluster(ClusterId(pg_cl_of[c.0 as usize])), None)
                }
            }
        }
    };
    for (i, e) in g.edges.iter().enumerate() {
        if kind[i] != 0 {
            continue;
        }
        let (f, fr) = map_ep(e.from);
        let (t, tr) = map_ep(e.to);
        let mut ed = e.clone();
        ed.from = f;
        ed.to = t;
        stubs.push(Stub {
            orig: i,
            pg_edge: pedges.len(),
            from_root: fr,
            to_root: tr,
        });
        pedges.push(ed);
        plabels.push(inp.labels[i]);
    }
    let pinp = Inputs {
        g: Cow::Owned(Graph {
            direction: g.direction,
            nodes: pnodes,
            edges: pedges,
            clusters: pclusters,
            class_defs: Vec::new(),
            seq: None,
            chart: None,
        }),
        dims: pdims,
        labels: plabels,
        title_w: ptitle,
        seed_x: pseed,
    };
    let pg = pipeline(&pinp, run)?;

    // Put everything back.
    let mut out = Core::empty(n, k, ne);
    out.size = pg.size;
    for v in 0..n {
        if nabs[v] == NONE {
            out.rects[v] = pg.rects[pg_node_of[v] as usize];
            out.slots[v] = pg.slots[pg_node_of[v] as usize];
        }
    }
    for c in 0..k {
        if absorbed[c] == NONE {
            out.clusters[c] = pg.clusters[pg_cl_of[c] as usize];
        }
    }
    let mut stand_rect: Vec<Rect> = Vec::with_capacity(roots.len());
    for (ri, &c) in roots.iter().enumerate() {
        let s = pg.rects[synth0 + ri];
        stand_rect.push(s);
        out.clusters[c] = s;
        let core = &mut subs[ri];
        let off = [
            s.x + (s.w - core.size[0]) / 2.0,
            s.y + m.cluster_title + m.cluster_pad,
        ];
        core.translate(off[0], off[1]);
        for (i, &v) in sub_nodes[ri].iter().enumerate() {
            out.rects[v as usize] = core.rects[i];
            out.slots[v as usize] = core.slots[i];
        }
        for (i, &cc) in sub_clusters[ri].iter().enumerate() {
            out.clusters[cc as usize] = core.clusters[i];
        }
        for (i, &e) in sub_edges[ri].iter().enumerate() {
            out.edges[e as usize] = std::mem::replace(&mut core.edges[i], empty_path());
        }
    }
    for st in &stubs {
        let mut path = pg.edges[st.pg_edge].clone();
        let e = &g.edges[st.orig];
        for (at_end, ep, root) in [(false, e.from, st.from_root), (true, e.to, st.to_root)] {
            let Some(ri) = root else { continue };
            let (inner, shp) = match ep {
                Endpoint::Node(a) => (out.rects[a.0 as usize], g.nodes[a.0 as usize].shape),
                Endpoint::Cluster(c) => (out.clusters[c.0 as usize], ShapeKind::Rect),
            };
            attach(
                &mut path,
                at_end,
                inner,
                shp,
                stand_rect[ri],
                g.direction,
                inp.title_w[roots[ri]],
            );
        }
        out.edges[st.orig] = path;
    }
    Ok(out)
}

/// Extends the end of `path` from the border of a stand-in cluster to the inner target.
fn attach(
    path: &mut EdgePath,
    at_end: bool,
    inner: Rect,
    shp: ShapeKind,
    stand: Rect,
    dir: Direction,
    title_w: f32,
) {
    let pts = &mut path.points;
    if pts.len() < 4 {
        return;
    }
    let (pi, ci) = if at_end {
        (pts.len() - 1, pts.len() - 2)
    } else {
        (0, 1)
    };
    let axis = usize::from(matches!(dir, Direction::LR | Direction::RL));
    let (lo, hi) = if axis == 0 {
        (stand.x, stand.right())
    } else {
        (stand.y, stand.bottom())
    };
    let margin = 14.0f32.min((hi - lo) / 2.0).max(0.0);
    let mut want = inner.center()[axis].clamp(lo + margin, (hi - margin).max(lo + margin));
    // Entering on the side that carries the title: keep clear of the text.
    let end_y = pts[pi][1];
    let title_side = match dir {
        Direction::TB => (end_y - stand.y).abs() < 1.0,
        Direction::BT => (end_y - stand.bottom()).abs() < 1.0,
        _ => false,
    };
    if title_side {
        want = compound::dodge_title(want, lo, hi, title_w);
    }
    let d = want - pts[pi][axis];
    pts[pi][axis] += d;
    pts[ci][axis] += d;
    let p = pts[pi];
    let q = shape::boundary(shp, inner, p);
    let (a, b) = if at_end { (p, q) } else { (q, p) };
    let c1 = [a[0] + (b[0] - a[0]) / 3.0, a[1] + (b[1] - a[1]) / 3.0];
    let c2 = [
        a[0] + 2.0 * (b[0] - a[0]) / 3.0,
        a[1] + 2.0 * (b[1] - a[1]) / 3.0,
    ];
    if at_end {
        pts.extend([c1, c2, b]);
    } else {
        pts.splice(0..0, [a, c1, c2]);
    }
}

/// Working graph after ranking: everything the two layout attempts have in common.
#[derive(Clone)]
struct Ranked {
    lg: LGraph,
    /// Extra node width reserved for self-loops.
    loop_extra: Vec<f32>,
}

/// The layered pipeline for one graph (no recursive clusters left). Chains run inside
/// their end clusters; when that makes the cluster columns impossible to keep straight
/// the graph is laid out again with chains outside. Ranking does not depend on that
/// choice (only `insert_dummies` reads it), so the ranked graph is computed once.
fn pipeline(inp: &Inputs, run: &Run) -> Result<Core, LayoutError> {
    let g = &inp.g;
    let (n, k, ne) = (g.nodes.len(), g.clusters.len(), g.edges.len());
    if n + k == 0 {
        return Ok(Core::empty(0, 0, ne));
    }
    let ranked = rank_graph(inp, run)?;
    // Only a graph with clusters can fail the first attempt.
    let spare = (k > 0).then(|| ranked.clone());
    if let Some(core) = finish_pass(inp, run, ranked, true)? {
        return Ok(core);
    }
    debug_assert!(spare.is_some(), "only graphs with clusters can fail a pass");
    let Some(ranked) = spare else {
        return Ok(Core::empty(n, k, ne));
    };
    Ok(finish_pass(inp, run, ranked, false)?.unwrap_or_else(|| Core::empty(n, k, ne)))
}

/// Layout width of the positioned working graph.
fn layout_width(l: &LGraph) -> f32 {
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for nd in &l.nodes {
        lo = lo.min(nd.x - nd.w / 2.0);
        hi = hi.max(nd.x + nd.w / 2.0);
    }
    hi - lo
}

/// Score of a positioned working graph (lower is better).
fn alt_score(l: &LGraph) -> f32 {
    layout_width(l)
        + if l.forced_failed { 1e6 } else { 0.0 }
        + CROSS_PX * order::crossings(l) as f32
}

/// A positioned copy of `base` using the node order `alt`. Positioning rewrites every
/// coordinate from the ranks alone, so the result does not depend on which earlier
/// positioning `base` came from.
fn with_order(base: &LGraph, alt: &[Vec<u32>], run: &Run) -> Result<LGraph, LayoutError> {
    let mut l2 = base.clone();
    for (r, layer) in alt.iter().enumerate() {
        for (i, &v) in layer.iter().enumerate() {
            l2.nodes[v as usize].order = i as u32;
        }
        l2.ranks[r].clone_from(layer);
    }
    position::position(&mut l2, run.m, run.cancel, run.par)?;
    Ok(l2)
}

/// The first alternative in `alts` that scores strictly below `threshold` and below
/// every earlier alternative (`None` when there is none).
fn best_of_chunk(
    base: &LGraph,
    alts: &[Vec<Vec<u32>>],
    threshold: f32,
    run: &Run,
) -> Result<Option<(f32, LGraph)>, LayoutError> {
    let mut best: Option<(f32, LGraph)> = None;
    let mut best_w = threshold;
    for alt in alts {
        check_cancel(run.cancel)?;
        let l2 = with_order(base, alt, run)?;
        let w = alt_score(&l2);
        if w < best_w {
            best_w = w;
            best = Some((w, l2));
        }
    }
    Ok(best)
}

/// Tries the alternative node orders and keeps the best-scoring positioning in `lg`.
/// The alternatives are independent, so chunks of them run concurrently; picking the
/// winner by strict `<` in index order (chunks and their members) keeps the lowest
/// index on ties, exactly as the sequential scan does.
fn pick_alternative(lg: &mut LGraph, run: &Run) -> Result<(), LayoutError> {
    if lg.alt_orders.is_empty() || lg.nodes.len() > 3_000 {
        return Ok(());
    }
    let alts = std::mem::take(&mut lg.alt_orders);
    let base_w = alt_score(lg);
    let workers = run.par.workers(
        alts.len(),
        lg.nodes.len() * alts.len(),
        PAR_ALT_WORK_PER_WORKER,
    );
    let inner = Run {
        par: run.par.share(workers),
        ..*run
    };
    let base: &LGraph = lg;
    let bests = par::run_chunks(&alts, workers, |_, chunk| {
        best_of_chunk(base, chunk, base_w, &inner)
    })?;
    let mut best_w = base_w;
    let mut winner = None;
    for (w, cand) in bests.into_iter().flatten() {
        if w < best_w {
            best_w = w;
            winner = Some(cand);
        }
    }
    if let Some(l) = winner {
        *lg = l;
    }
    Ok(())
}

/// Cluster of every node (`NONE` = top level or invalid).
fn node_clusters(g: &Graph) -> Vec<u32> {
    let k = g.clusters.len();
    g.nodes
        .iter()
        .map(|nd| match nd.cluster {
            Some(c) if (c.0 as usize) < k => c.0,
            _ => NONE,
        })
        .collect()
}

/// Room for self-loops on the cross+ side of each node.
fn loop_extras(inp: &Inputs, m: &Metrics, swap: bool) -> Vec<f32> {
    let g = &inp.g;
    let n = g.nodes.len();
    let norm = |d: [f32; 2]| if swap { [d[1], d[0]] } else { d };
    let mut loop_extra = vec![0.0f32; n];
    let mut loop_count = vec![0u32; n];
    for (ei, e) in g.edges.iter().enumerate() {
        if let (Endpoint::Node(a), Endpoint::Node(b)) = (e.from, e.to)
            && a == b
            && (a.0 as usize) < n
        {
            let a = a.0 as usize;
            let bulge = route::loop_bulge(m, loop_count[a]);
            loop_count[a] += 1;
            let lab = inp.labels[ei].map_or(0.0, |d| norm(d)[0] + 8.0);
            loop_extra[a] = loop_extra[a].max(bulge + lab).max(m.node_gap);
        }
    }
    loop_extra
}

/// Working graph with one node per authored node and the cluster border nodes.
fn base_graph(inp: &Inputs, node_cluster: &[u32], loop_extra: &[f32], swap: bool) -> LGraph {
    let g = &inp.g;
    let norm = |d: [f32; 2]| if swap { [d[1], d[0]] } else { d };
    let mut lg = LGraph {
        dir: g.direction,
        title_w: inp.title_w.clone(),
        ..LGraph::default()
    };
    for (i, _) in g.nodes.iter().enumerate() {
        let d = norm(inp.dims[i]);
        lg.nodes.push(LNode {
            kind: LKind::Real(NodeId(i as u32)),
            w: d[0] + loop_extra[i],
            h: d[1],
            rank: 0,
            order: 0,
            x: 0.0,
            y: 0.0,
            cluster: (node_cluster[i] != NONE).then_some(ClusterId(node_cluster[i])),
        });
    }
    if !g.clusters.is_empty() {
        compound::add_borders(&mut lg, g, swap, node_cluster);
    }
    lg
}

/// Appends one working edge per usable authored edge. Self-loops (handled by the
/// router) and edges between a cluster and its own members are skipped.
fn add_work_edges(inp: &Inputs, lg: &mut LGraph, node_cluster: &[u32]) {
    let g = &inp.g;
    let (n, k) = (g.nodes.len(), g.clusters.len());
    for (ei, e) in g.edges.iter().enumerate() {
        let ends = |ep: Endpoint| -> Option<(u32, u32)> {
            // (as source node, as target node)
            match ep {
                Endpoint::Node(a) if (a.0 as usize) < n => Some((a.0, a.0)),
                Endpoint::Cluster(c) if (c.0 as usize) < k => {
                    Some((lg.cl_bot[c.0 as usize], lg.cl_top[c.0 as usize]))
                }
                _ => None,
            }
        };
        let (Some(f), Some(t)) = (ends(e.from), ends(e.to)) else {
            continue;
        };
        let inside = |ep: Endpoint, other: Endpoint| -> bool {
            let cl_of = |x: Endpoint| -> u32 {
                match x {
                    Endpoint::Node(a) => node_cluster[a.0 as usize],
                    Endpoint::Cluster(c) => c.0,
                }
            };
            match ep {
                Endpoint::Cluster(c) => lg.tree.inside(cl_of(other), c.0),
                Endpoint::Node(_) => false,
            }
        };
        if inside(e.from, e.to) || inside(e.to, e.from) {
            continue;
        }
        if let (Endpoint::Node(a), Endpoint::Node(b)) = (e.from, e.to)
            && a == b
        {
            continue;
        }
        let (a, b) = (f.0, t.1);
        if a == b {
            continue;
        }
        let mut min_len = u32::from(e.min_len).max(1);
        if inp.labels[ei].is_some() {
            min_len = min_len.max(2);
        }
        lg.edges.push(LEdge {
            from: a,
            to: b,
            orig: EdgeId(ei as u32),
            reversed: false,
            weight: 1,
            min_len,
        });
    }
}

/// Builds the working graph and ranks it (cycle removal, cluster nesting, network
/// simplex, tightened cluster borders).
fn rank_graph(inp: &Inputs, run: &Run) -> Result<Ranked, LayoutError> {
    let (m, cancel, times) = (run.m, run.cancel, run.times);
    let g = &inp.g;
    let k = g.clusters.len();
    let swap = matches!(g.direction, Direction::LR | Direction::RL);
    let node_cluster = node_clusters(g);
    let loop_extra = loop_extras(inp, m, swap);
    let mut lg = base_graph(inp, &node_cluster, &loop_extra, swap);
    add_work_edges(inp, &mut lg, &node_cluster);
    let real_edges = StageTimes::time(times, Stage::Rank, || {
        acyclic::make_acyclic(&mut lg);
        let real_edges = lg.edges.len();
        if k > 0 {
            let first = compound::add_nesting(&mut lg, &node_cluster);
            debug_assert_eq!(first, real_edges);
            let flow = compound::add_flow_edges(&mut lg, real_edges);
            compound::break_cycles(&mut lg, first, flow);
        }
        real_edges
    });
    check_cancel(cancel)?;
    StageTimes::time(times, Stage::Rank, || rank::assign_ranks(&mut lg, cancel))?;
    if k > 0 {
        StageTimes::time(times, Stage::Rank, || {
            lg.edges.truncate(real_edges);
            compound::tighten(&mut lg, &node_cluster);
        });
    }
    Ok(Ranked { lg, loop_extra })
}

/// Orders and positions the ranked graph (dummies, ordering, coordinates, best
/// alternative order).
fn order_and_position(inp: &Inputs, run: &Run, lg: &mut LGraph) -> Result<(), LayoutError> {
    let (m, cancel, times) = (run.m, run.cancel, run.times);
    let swap = matches!(inp.g.direction, Direction::LR | Direction::RL);
    StageTimes::time(times, Stage::Rank, || {
        let norm_labels: Vec<Option<[f32; 2]>> = inp
            .labels
            .iter()
            .map(|d| d.map(|d| if swap { [d[1], d[0]] } else { d }))
            .collect();
        rank::insert_dummies(lg, &inp.g, &norm_labels);
        if !inp.g.clusters.is_empty() {
            compound::add_side_borders(lg);
        }
    });
    check_cancel(cancel)?;
    let seed = inp
        .seed_x
        .iter()
        .any(Option::is_some)
        .then_some(&inp.seed_x[..]);
    StageTimes::time(times, Stage::Order, || order::order(lg, seed, cancel))?;
    check_cancel(cancel)?;
    StageTimes::time(times, Stage::Position, || {
        position::position(lg, m, cancel, run.par)?;
        pick_alternative(lg, run)
    })?;
    check_cancel(cancel)
}

/// One layout attempt from the ranked graph. `None` when chains inside clusters made
/// the straight cluster columns impossible (the caller then retries without).
fn finish_pass(
    inp: &Inputs,
    run: &Run,
    ranked: Ranked,
    chains_in_clusters: bool,
) -> Result<Option<Core>, LayoutError> {
    let Ranked { mut lg, loop_extra } = ranked;
    lg.chains_in_clusters = chains_in_clusters;
    order_and_position(inp, run, &mut lg)?;
    if lg.forced_failed && chains_in_clusters && !inp.g.clusters.is_empty() {
        return Ok(None);
    }
    assemble(inp, run, &lg, &loop_extra).map(Some)
}

/// Final-frame rectangle of every authored node.
fn node_rects(inp: &Inputs, lg: &LGraph, frame: &Frame, loop_extra: &[f32]) -> Vec<Rect> {
    let n = inp.g.nodes.len();
    let mut rects = Vec::with_capacity(n);
    for (i, nd) in lg.nodes[..n].iter().enumerate() {
        let c = frame.map([nd.x - loop_extra[i] / 2.0, nd.y]);
        let [fw, fh] = inp.dims[i];
        rects.push(Rect {
            x: c[0] - fw / 2.0,
            y: c[1] - fh / 2.0,
            w: fw,
            h: fh,
        });
    }
    rects
}

/// Union of every rectangle, label and path point.
fn content_bounds(rects: &[Rect], crects: &[Rect], edges: &[EdgePath]) -> Rect {
    let mut bounds: Option<Rect> = None;
    let mut grow = |r: Rect| {
        bounds = Some(bounds.map_or(r, |b| b.union(r)));
    };
    for r in rects.iter().chain(crects) {
        grow(*r);
    }
    for e in edges {
        if let Some(l) = e.label {
            grow(l);
        }
        for p in &e.points {
            grow(Rect {
                x: p[0],
                y: p[1],
                w: 0.0,
                h: 0.0,
            });
        }
    }
    bounds.unwrap_or_default()
}

/// Final frame, routing, bounds and translation to the origin.
fn assemble(inp: &Inputs, run: &Run, lg: &LGraph, loop_extra: &[f32]) -> Result<Core, LayoutError> {
    let (m, cancel, times) = (run.m, run.cancel, run.times);
    let g = &inp.g;
    let n = g.nodes.len();
    let extent = lg
        .nodes
        .iter()
        .map(|nd| nd.y + nd.h / 2.0)
        .fold(0.0f32, f32::max);
    let frame = Frame {
        dir: g.direction,
        extent,
    };
    let mut rects = node_rects(inp, lg, &frame, loop_extra);
    let (mut crects, mut edges) = StageTimes::time(times, Stage::Route, || {
        let crects = if g.clusters.is_empty() {
            Vec::new()
        } else {
            compound::cluster_rects(lg, &frame, &inp.title_w, m)
        };
        let edges = route::route(lg, g, &rects, &frame, &inp.labels, &crects, &inp.title_w, m);
        (crects, edges)
    });
    check_cancel(cancel)?;
    let bounds = content_bounds(&rects, &crects, &edges);
    let (dx, dy) = (-bounds.x, -bounds.y);
    for r in rects.iter_mut().chain(crects.iter_mut()) {
        r.x += dx;
        r.y += dy;
    }
    for e in &mut edges {
        shift_path(e, dx, dy);
    }
    let slots = lg.nodes[..n]
        .iter()
        .map(|nd| (nd.rank.max(0) as u32, nd.order))
        .collect();
    Ok(Core {
        rects,
        clusters: crects,
        edges,
        slots,
        size: [bounds.w, bounds.h],
    })
}
