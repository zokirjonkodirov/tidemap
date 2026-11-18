//! Graph model produced by the parser and consumed by layout.

/// Byte range into the source text (UTF-8 offsets, always on char boundaries).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

/// Index of a node in `Graph::nodes`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct NodeId(pub u32);

/// Index of an edge in `Graph::edges`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct EdgeId(pub u32);

/// Index of a cluster (subgraph) in `Graph::clusters`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ClusterId(pub u32);

/// Diagram family detected from the header.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Family {
    Flowchart,
    State,
    Sequence,
    Class,
    Er,
    Other,
}

/// Main layout direction.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Direction {
    #[default]
    TB,
    BT,
    LR,
    RL,
}

/// Node outline.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ShapeKind {
    #[default]
    Rect,
    Rounded,
    Stadium,
    Subroutine,
    Cylinder,
    Circle,
    DoubleCircle,
    Diamond,
    Hexagon,
    Parallelogram,
    ParallelogramAlt,
    Trapezoid,
    TrapezoidAlt,
    Asymmetric,
}

/// Edge line style.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LineKind {
    #[default]
    Solid,
    Dotted,
    Thick,
    Invisible,
}

/// Edge end decoration.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ArrowKind {
    None,
    #[default]
    Arrow,
    Circle,
    Cross,
}

/// An edge end: a node or a whole cluster.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Endpoint {
    Node(NodeId),
    Cluster(ClusterId),
}

/// Authored style overrides (RGBA colours).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleProps {
    pub fill: Option<[u8; 4]>,
    pub stroke: Option<[u8; 4]>,
    pub color: Option<[u8; 4]>,
    pub stroke_width: Option<f32>,
    pub dashed: bool,
}

/// A flowchart node.
#[derive(Clone, Debug)]
pub struct Node {
    pub key: String,
    pub label: String,
    pub shape: ShapeKind,
    pub cluster: Option<ClusterId>,
    pub classes: Vec<u16>,
    pub style: StyleProps,
    /// First declaration, from identifier to closing bracket.
    pub span: Span,
    /// Raw label text in the source.
    pub label_span: Span,
}

/// A flowchart edge.
#[derive(Clone, Debug)]
pub struct Edge {
    pub from: Endpoint,
    pub to: Endpoint,
    pub label: Option<String>,
    pub line: LineKind,
    pub head: ArrowKind,
    pub tail: ArrowKind,
    pub min_len: u8,
    pub style: StyleProps,
    pub span: Span,
}

/// A subgraph.
#[derive(Clone, Debug)]
pub struct Cluster {
    pub key: String,
    pub title: String,
    pub parent: Option<ClusterId>,
    pub direction: Option<Direction>,
    pub classes: Vec<u16>,
    pub style: StyleProps,
    pub span: Span,
}

/// A `classDef` entry. Referenced-but-undefined classes get empty props.
#[derive(Clone, Debug)]
pub struct ClassDef {
    pub name: String,
    pub props: StyleProps,
}

/// The parsed diagram.
#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub direction: Direction,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub clusters: Vec<Cluster>,
    pub class_defs: Vec<ClassDef>,
}

/// A parse problem. `line` and `column` are 1-based; columns count chars.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub line: u32,
    pub column: u32,
    pub span: Span,
    pub message: String,
}

/// Result of `parse`.
#[derive(Clone, Debug)]
pub struct Parsed {
    pub family: Family,
    pub graph: Graph,
    pub diagnostics: Vec<Diagnostic>,
}

impl Graph {
    /// Edges whose source is the node `n` (linear scan).
    pub fn out_edges(&self, n: NodeId) -> impl Iterator<Item = EdgeId> + '_ {
        self.edges
            .iter()
            .enumerate()
            .filter(move |(_, e)| e.from == Endpoint::Node(n))
            .map(|(i, _)| EdgeId(i as u32))
    }

    /// Edges whose target is the node `n` (linear scan).
    pub fn in_edges(&self, n: NodeId) -> impl Iterator<Item = EdgeId> + '_ {
        self.edges
            .iter()
            .enumerate()
            .filter(move |(_, e)| e.to == Endpoint::Node(n))
            .map(|(i, _)| EdgeId(i as u32))
    }
}
