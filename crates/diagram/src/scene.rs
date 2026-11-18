//! Display list with a spatial index: what the canvas paints and the SVG writer exports.

use crate::graph::{
    ArrowKind, ClassDef, ClusterId, EdgeId, Endpoint, Graph, LineKind, NodeId, ShapeKind, Span,
    StyleProps,
};
use crate::layout::Layout;
use crate::shape::Rect;
use crate::text::TextBlock;

/// Any paintable, selectable item.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ItemId {
    Node(NodeId),
    Edge(EdgeId),
    Cluster(ClusterId),
}

/// A positioned node.
#[derive(Clone, Debug)]
pub struct NodeItem {
    pub id: NodeId,
    pub rect: Rect,
    pub shape: ShapeKind,
    pub text: TextBlock,
    pub span: Span,
    /// The shape that decides the colour role (same as `shape` for flowcharts).
    pub role_shape: ShapeKind,
    pub classes: Vec<u16>,
    pub style: StyleProps,
}

/// A routed edge.
#[derive(Clone, Debug)]
pub struct EdgeItem {
    pub id: EdgeId,
    /// Cubic Bézier chain, `1 + 3k` points. Empty when the edge is not routed.
    pub points: Vec<[f32; 2]>,
    /// Bounding box of `points` inflated by 4, united with the label rectangle.
    pub bounds: Rect,
    pub line: LineKind,
    pub head: ArrowKind,
    pub tail: ArrowKind,
    pub label: Option<(Rect, TextBlock)>,
    pub from: Endpoint,
    pub to: Endpoint,
    pub span: Span,
    pub style: StyleProps,
}

/// A subgraph container.
#[derive(Clone, Debug)]
pub struct ClusterItem {
    pub id: ClusterId,
    pub rect: Rect,
    pub title: String,
    /// Nesting level, 0 for top-level subgraphs.
    pub depth: u8,
    pub span: Span,
    pub classes: Vec<u16>,
    pub style: StyleProps,
}

/// Everything needed to paint, hit-test and export one diagram.
///
/// Paint order is clusters (outer first), then edges, then nodes.
#[derive(Clone, Debug)]
pub struct Scene {
    pub bounds: Rect,
    pub clusters: Vec<ClusterItem>,
    pub edges: Vec<EdgeItem>,
    pub nodes: Vec<NodeItem>,
    pub class_defs: Vec<ClassDef>,
    grid: Grid,
    adjacency: Adjacency,
    meta: Meta,
}

/// Source-level data the scene keeps for painting and search.
#[derive(Clone, Debug, Default)]
struct Meta {
    node_keys: Vec<String>,
    cluster_keys: Vec<String>,
    labels_lower: Vec<String>,
    node_cluster: Vec<Option<ClusterId>>,
    /// Per cluster: index of its top-level ancestor among the top-level clusters.
    top_index: Vec<usize>,
}

/// Compressed adjacency lists, both directions, deduplicated.
#[derive(Clone, Debug, Default)]
struct Adjacency {
    edge_start: Vec<u32>,
    edges: Vec<EdgeId>,
    node_start: Vec<u32>,
    nodes: Vec<NodeId>,
}

/// Uniform grid over global item indices (clusters, then edges, then nodes).
#[derive(Clone, Debug, Default)]
struct Grid {
    origin: [f32; 2],
    cell: f32,
    nx: usize,
    ny: usize,
    cells: Vec<Vec<u32>>,
}

fn finite_rect(r: Rect) -> bool {
    r.x.is_finite() && r.y.is_finite() && r.w.is_finite() && r.h.is_finite()
}

impl Grid {
    fn build(bounds: Rect, item_bounds: &[Option<Rect>]) -> Grid {
        let count = item_bounds.iter().flatten().count();
        let (w, h) = if finite_rect(bounds) {
            (bounds.w.max(0.0), bounds.h.max(0.0))
        } else {
            (0.0, 0.0)
        };
        let area = w * h;
        let cell = (256.0f32).max((area / count.max(1) as f32).sqrt() * 2.0);
        let nx = ((w / cell).ceil() as usize).clamp(1, 4096);
        let ny = ((h / cell).ceil() as usize).clamp(1, 4096);
        let origin = if finite_rect(bounds) {
            [bounds.x, bounds.y]
        } else {
            [0.0, 0.0]
        };
        let mut grid = Grid {
            origin,
            cell,
            nx,
            ny,
            cells: vec![Vec::new(); nx * ny],
        };
        for (i, b) in item_bounds.iter().enumerate() {
            if let Some(b) = b {
                let (x0, y0, x1, y1) = grid.range(*b);
                for cy in y0..=y1 {
                    for cx in x0..=x1 {
                        grid.cells[cy * grid.nx + cx].push(i as u32);
                    }
                }
            }
        }
        grid
    }

    fn col(&self, v: f32, axis: usize) -> usize {
        let n = if axis == 0 { self.nx } else { self.ny };
        let c = ((v - self.origin[axis]) / self.cell).floor();
        if c.is_nan() || c < 0.0 {
            0
        } else {
            (c as usize).min(n - 1)
        }
    }

    /// Inclusive cell range covering `r`.
    fn range(&self, r: Rect) -> (usize, usize, usize, usize) {
        (
            self.col(r.x, 0),
            self.col(r.y, 1),
            self.col(r.right(), 0),
            self.col(r.bottom(), 1),
        )
    }

    /// Sorted, deduplicated candidate indices for `r`.
    fn candidates(&self, r: Rect) -> Vec<u32> {
        let (x0, y0, x1, y1) = self.range(r);
        let mut out = Vec::new();
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                out.extend_from_slice(&self.cells[cy * self.nx + cx]);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    fn byte_size(&self) -> usize {
        self.cells.capacity() * std::mem::size_of::<Vec<u32>>()
            + self
                .cells
                .iter()
                .map(|c| c.capacity() * std::mem::size_of::<u32>())
                .sum::<usize>()
    }
}

/// Flattens a cubic chain into points (12 segments per cubic).
fn flatten(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    let Some(first) = points.first() else {
        return out;
    };
    out.push(*first);
    for c in points[1..].chunks(3) {
        let p0 = out[out.len() - 1];
        if c.len() < 3 {
            for p in c {
                out.push(*p);
            }
            break;
        }
        for i in 1..=12 {
            let t = i as f32 / 12.0;
            let u = 1.0 - t;
            let (a, b, cc, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            out.push([
                a * p0[0] + b * c[0][0] + cc * c[1][0] + d * c[2][0],
                a * p0[1] + b * c[0][1] + cc * c[1][1] + d * c[2][1],
            ]);
        }
    }
    out
}

fn point_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= f32::EPSILON {
        0.0
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    };
    let (qx, qy) = (a[0] + t * dx, a[1] + t * dy);
    ((p[0] - qx).powi(2) + (p[1] - qy).powi(2)).sqrt()
}

fn edge_bounds(points: &[[f32; 2]], label: Option<Rect>) -> Option<Rect> {
    let mut bb: Option<Rect> = None;
    if !points.is_empty() {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in points {
            x0 = x0.min(p[0]);
            y0 = y0.min(p[1]);
            x1 = x1.max(p[0]);
            y1 = y1.max(p[1]);
        }
        bb = Some(
            Rect {
                x: x0,
                y: y0,
                w: x1 - x0,
                h: y1 - y0,
            }
            .inflate(4.0),
        );
    }
    match (bb, label) {
        (Some(b), Some(l)) => Some(b.union(l)),
        (Some(b), None) => Some(b),
        (None, Some(l)) => Some(l),
        (None, None) => None,
    }
}

impl Scene {
    /// Builds the display list and its indexes from a graph and its layout.
    pub fn build(g: &Graph, l: &Layout) -> Scene {
        let (depths, top_index) = cluster_levels(g);

        let clusters: Vec<ClusterItem> = g
            .clusters
            .iter()
            .enumerate()
            .map(|(i, c)| ClusterItem {
                id: ClusterId(i as u32),
                rect: l.clusters.get(i).copied().unwrap_or_default(),
                title: c.title.clone(),
                depth: depths[i],
                span: c.span,
                classes: c.classes.clone(),
                style: c.style.clone(),
            })
            .collect();

        let edges: Vec<EdgeItem> = g
            .edges
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let path = l.edges.get(i);
                let points = path.map(|p| p.points.clone()).unwrap_or_default();
                let label_rect = path.and_then(|p| p.label);
                let label = match (label_rect, l.edge_text.get(i).and_then(|t| t.clone())) {
                    (Some(r), Some(t)) => Some((r, t)),
                    _ => None,
                };
                let bounds =
                    edge_bounds(&points, label.as_ref().map(|(r, _)| *r)).unwrap_or_default();
                EdgeItem {
                    id: EdgeId(i as u32),
                    points,
                    bounds,
                    line: e.line,
                    head: e.head,
                    tail: e.tail,
                    label,
                    from: e.from,
                    to: e.to,
                    span: e.span,
                    style: e.style.clone(),
                }
            })
            .collect();

        let nodes: Vec<NodeItem> = g
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| NodeItem {
                id: NodeId(i as u32),
                rect: l.nodes.get(i).copied().unwrap_or_default(),
                shape: n.shape,
                text: l.node_text.get(i).cloned().unwrap_or(TextBlock {
                    lines: Vec::new(),
                    width: 0.0,
                    height: 0.0,
                }),
                span: n.span,
                role_shape: n.shape,
                classes: n.classes.clone(),
                style: n.style.clone(),
            })
            .collect();

        let bounds = if finite_rect(l.bounds) {
            l.bounds
        } else {
            Rect::default()
        };

        // Global item bounds in paint order.
        let mut item_bounds: Vec<Option<Rect>> =
            Vec::with_capacity(clusters.len() + edges.len() + nodes.len());
        let visible = |r: Rect| finite_rect(r).then_some(r);
        item_bounds.extend(clusters.iter().map(|c| visible(c.rect)));
        item_bounds.extend(edges.iter().map(|e| {
            let has_geometry = !e.points.is_empty() || e.label.is_some();
            if has_geometry {
                visible(e.bounds)
            } else {
                None
            }
        }));
        item_bounds.extend(nodes.iter().map(|n| visible(n.rect)));
        let grid = Grid::build(bounds, &item_bounds);

        let adjacency = Adjacency::build(g);
        let meta = Meta {
            node_keys: g.nodes.iter().map(|n| n.key.clone()).collect(),
            cluster_keys: g.clusters.iter().map(|c| c.key.clone()).collect(),
            labels_lower: g.nodes.iter().map(|n| n.label.to_lowercase()).collect(),
            node_cluster: g.nodes.iter().map(|n| n.cluster).collect(),
            top_index,
        };

        Scene {
            bounds,
            clusters,
            edges,
            nodes,
            class_defs: g.class_defs.clone(),
            grid,
            adjacency,
            meta,
        }
    }

    fn item_id(&self, global: u32) -> ItemId {
        let i = global as usize;
        let (c, e) = (self.clusters.len(), self.edges.len());
        if i < c {
            ItemId::Cluster(ClusterId(global))
        } else if i < c + e {
            ItemId::Edge(EdgeId((i - c) as u32))
        } else {
            ItemId::Node(NodeId((i - c - e) as u32))
        }
    }

    fn item_rect(&self, global: u32) -> Rect {
        let i = global as usize;
        let (c, e) = (self.clusters.len(), self.edges.len());
        if i < c {
            self.clusters[i].rect
        } else if i < c + e {
            self.edges[i - c].bounds
        } else {
            self.nodes[i - c - e].rect
        }
    }

    /// Appends, in paint order, every item whose bounds intersect `view`.
    pub fn query(&self, view: Rect, out: &mut Vec<ItemId>) {
        if !finite_rect(view) {
            return;
        }
        for global in self.grid.candidates(view) {
            if self.item_rect(global).intersects(view) {
                out.push(self.item_id(global));
            }
        }
    }

    /// Item under `p`: nodes first, then edge labels, then edge curves within
    /// `tolerance`, then the innermost cluster.
    pub fn hit(&self, p: [f32; 2], tolerance: f32) -> Option<ItemId> {
        if !p[0].is_finite() || !p[1].is_finite() {
            return None;
        }
        let tol = if tolerance.is_finite() {
            tolerance.max(0.0)
        } else {
            0.0
        };
        let probe = Rect {
            x: p[0],
            y: p[1],
            w: 0.0,
            h: 0.0,
        }
        .inflate(tol);
        let candidates = self.grid.candidates(probe);
        let (c, e) = (self.clusters.len(), self.edges.len());

        // Nodes, topmost (last painted) first.
        for &g in candidates.iter().rev() {
            let i = g as usize;
            if i >= c + e && self.nodes[i - c - e].rect.contains(p) {
                return Some(ItemId::Node(NodeId((i - c - e) as u32)));
            }
        }
        // Edge labels.
        for &g in candidates.iter().rev() {
            let i = g as usize;
            if (c..c + e).contains(&i)
                && let Some((r, _)) = &self.edges[i - c].label
                && r.contains(p)
            {
                return Some(ItemId::Edge(EdgeId((i - c) as u32)));
            }
        }
        // Edge curves: nearest within tolerance.
        let mut best: Option<(f32, usize)> = None;
        for &g in &candidates {
            let i = g as usize;
            if !(c..c + e).contains(&i) {
                continue;
            }
            let edge = &self.edges[i - c];
            if edge.line == LineKind::Invisible
                || edge.points.is_empty()
                || !edge.bounds.inflate(tol).contains(p)
            {
                continue;
            }
            let flat = flatten(&edge.points);
            let d = flat
                .windows(2)
                .map(|w| point_segment(p, w[0], w[1]))
                .fold(f32::MAX, f32::min);
            if d <= tol && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, i - c));
            }
        }
        if let Some((_, i)) = best {
            return Some(ItemId::Edge(EdgeId(i as u32)));
        }
        // Innermost cluster: deepest, later in paint order wins ties.
        let mut inner: Option<&ClusterItem> = None;
        for &g in &candidates {
            let i = g as usize;
            if i < c {
                let cl = &self.clusters[i];
                if cl.rect.contains(p) && inner.is_none_or(|b| cl.depth >= b.depth) {
                    inner = Some(cl);
                }
            }
        }
        inner.map(|cl| ItemId::Cluster(cl.id))
    }

    /// Incident edges and adjacent nodes of `n`, both directions, deduplicated.
    pub fn neighbours(&self, n: NodeId) -> (&[EdgeId], &[NodeId]) {
        let i = n.0 as usize;
        let range = |start: &[u32]| -> Option<(usize, usize)> {
            Some((*start.get(i)? as usize, *start.get(i + 1)? as usize))
        };
        let edges = range(&self.adjacency.edge_start)
            .and_then(|(a, b)| self.adjacency.edges.get(a..b))
            .unwrap_or(&[]);
        let nodes = range(&self.adjacency.node_start)
            .and_then(|(a, b)| self.adjacency.nodes.get(a..b))
            .unwrap_or(&[]);
        (edges, nodes)
    }

    /// Node whose source span contains `byte`; the smallest span wins.
    pub fn node_at_offset(&self, byte: u32) -> Option<NodeId> {
        self.nodes
            .iter()
            .filter(|n| n.span.start <= byte && byte < n.span.end)
            .min_by_key(|n| (n.span.end - n.span.start, n.id))
            .map(|n| n.id)
    }

    /// Case-insensitive substring search over node labels, in reading order.
    pub fn find(&self, needle: &str) -> Vec<NodeId> {
        if needle.is_empty() {
            return Vec::new();
        }
        let needle = needle.to_lowercase();
        let mut hits: Vec<&NodeItem> = self
            .nodes
            .iter()
            .filter(|n| {
                self.meta
                    .labels_lower
                    .get(n.id.0 as usize)
                    .is_some_and(|l| l.contains(&needle))
            })
            .collect();
        hits.sort_by(|a, b| {
            a.rect
                .y
                .total_cmp(&b.rect.y)
                .then(a.rect.x.total_cmp(&b.rect.x))
                .then(a.id.cmp(&b.id))
        });
        hits.into_iter().map(|n| n.id).collect()
    }

    /// Source key of a node (the identifier used in the diagram text).
    pub fn node_key(&self, n: NodeId) -> &str {
        self.meta
            .node_keys
            .get(n.0 as usize)
            .map_or("", String::as_str)
    }

    /// Source key of a cluster.
    pub fn cluster_key(&self, c: ClusterId) -> &str {
        self.meta
            .cluster_keys
            .get(c.0 as usize)
            .map_or("", String::as_str)
    }

    /// Index of the top-level cluster that contains `c` (or is `c`), counted
    /// among top-level clusters in declaration order.
    pub fn top_level_index(&self, c: ClusterId) -> usize {
        self.meta.top_index.get(c.0 as usize).copied().unwrap_or(0)
    }

    /// Top-level cluster index of the cluster holding node `n`, if any.
    pub fn node_top_level_index(&self, n: NodeId) -> Option<usize> {
        let c = (*self.meta.node_cluster.get(n.0 as usize)?)?;
        self.meta.top_index.get(c.0 as usize).copied()
    }

    /// Approximate heap size, for cache budgets.
    pub fn byte_size(&self) -> usize {
        use std::mem::size_of;
        let strings =
            |v: &[String]| -> usize { v.iter().map(|s| s.capacity() + size_of::<String>()).sum() };
        let text = |t: &TextBlock| -> usize {
            t.lines
                .iter()
                .map(|s| s.capacity() + size_of::<String>())
                .sum::<usize>()
        };
        size_of::<Scene>()
            + self.clusters.capacity() * size_of::<ClusterItem>()
            + self
                .clusters
                .iter()
                .map(|c| c.title.capacity())
                .sum::<usize>()
            + self.edges.capacity() * size_of::<EdgeItem>()
            + self
                .edges
                .iter()
                .map(|e| {
                    e.points.capacity() * size_of::<[f32; 2]>()
                        + e.label.as_ref().map_or(0, |(_, t)| text(t))
                })
                .sum::<usize>()
            + self.nodes.capacity() * size_of::<NodeItem>()
            + self
                .nodes
                .iter()
                .map(|n| text(&n.text) + n.classes.capacity() * 2)
                .sum::<usize>()
            + self.class_defs.capacity() * size_of::<ClassDef>()
            + self.grid.byte_size()
            + self.adjacency.edge_start.capacity() * 4
            + self.adjacency.node_start.capacity() * 4
            + self.adjacency.edges.capacity() * size_of::<EdgeId>()
            + self.adjacency.nodes.capacity() * size_of::<NodeId>()
            + strings(&self.meta.node_keys)
            + strings(&self.meta.cluster_keys)
            + strings(&self.meta.labels_lower)
            + self.meta.node_cluster.capacity() * size_of::<Option<ClusterId>>()
            + self.meta.top_index.capacity() * size_of::<usize>()
    }
}

/// Nesting depth and top-level-ancestor index for every cluster.
fn cluster_levels(g: &Graph) -> (Vec<u8>, Vec<usize>) {
    let n = g.clusters.len();
    let mut depth = vec![0u8; n];
    let mut top = vec![0usize; n];
    let mut next_top = 0usize;
    let mut root_slot = vec![usize::MAX; n];
    for i in 0..n {
        // Walk to the root with a step cap so malformed parent links cannot loop.
        let mut d = 0usize;
        let mut cur = i;
        while let Some(p) = g.clusters[cur].parent {
            let p = p.0 as usize;
            if p >= n || p == cur || d > 64 {
                break;
            }
            cur = p;
            d += 1;
        }
        depth[i] = d.min(255) as u8;
        if root_slot[cur] == usize::MAX {
            root_slot[cur] = next_top;
            next_top += 1;
        }
        top[i] = root_slot[cur];
    }
    (depth, top)
}

impl Adjacency {
    fn build(g: &Graph) -> Adjacency {
        let n = g.nodes.len();
        let mut edge_lists: Vec<Vec<EdgeId>> = vec![Vec::new(); n];
        let mut node_lists: Vec<Vec<NodeId>> = vec![Vec::new(); n];
        for (i, e) in g.edges.iter().enumerate() {
            let id = EdgeId(i as u32);
            let from = match e.from {
                Endpoint::Node(a) if (a.0 as usize) < n => Some(a),
                _ => None,
            };
            let to = match e.to {
                Endpoint::Node(b) if (b.0 as usize) < n => Some(b),
                _ => None,
            };
            if let Some(a) = from {
                edge_lists[a.0 as usize].push(id);
            }
            if let Some(b) = to
                && from != Some(b)
            {
                edge_lists[b.0 as usize].push(id);
            }
            if let (Some(a), Some(b)) = (from, to)
                && a != b
            {
                node_lists[a.0 as usize].push(b);
                node_lists[b.0 as usize].push(a);
            }
        }
        let mut adj = Adjacency::default();
        adj.edge_start.push(0);
        adj.node_start.push(0);
        for (es, ns) in edge_lists.iter_mut().zip(node_lists.iter_mut()) {
            es.sort_unstable();
            es.dedup();
            ns.sort_unstable();
            ns.dedup();
            adj.edges.extend_from_slice(es);
            adj.nodes.extend_from_slice(ns);
            adj.edge_start.push(adj.edges.len() as u32);
            adj.node_start.push(adj.nodes.len() as u32);
        }
        adj
    }
}
