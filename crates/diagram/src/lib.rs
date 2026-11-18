//! Pure-Rust Mermaid diagram engine: parser, text measurement and shape geometry.

pub mod ends;
pub mod graph;
pub mod layout;
pub mod lexer;
pub mod metrics;
pub mod parser;
pub mod scene;
pub mod shape;
pub mod style;
pub mod svg;
pub mod text;

pub use graph::*;
pub use layout::{EdgePath, Layout, LayoutError, Par, Stage, StageTimes, layout, layout_with};
pub use metrics::Metrics;
pub use parser::parse;
pub use scene::{ClusterItem, EdgeItem, ItemId, NodeItem, Scene};
pub use shape::{PathCmd, Rect};
pub use style::{
    ColorBy, DiagramColors, Paint, Rgba, Role, StyleSheet, Swatch, cluster_paint, edge_paint,
    node_paint, role,
};
pub use svg::to_svg;
pub use text::{Measurer, TextBlock};

use std::sync::atomic::AtomicBool;

/// Detects the diagram family from the first meaningful word of `source`.
///
/// Frontmatter, `%%` comments and `%%{...}%%` directives are skipped.
pub fn detect(source: &str) -> Family {
    let start = lexer::skip_preamble(source);
    let rest = source.get(start..).unwrap_or("");
    let word = rest
        .split(|c: char| c.is_whitespace() || c == ';')
        .next()
        .unwrap_or("");
    if word == "graph" || word.starts_with("flowchart") {
        Family::Flowchart
    } else if word.starts_with("stateDiagram") {
        Family::State
    } else if word == "sequenceDiagram" {
        Family::Sequence
    } else if word.starts_with("classDiagram") {
        Family::Class
    } else if word == "erDiagram" {
        Family::Er
    } else {
        Family::Other
    }
}

/// Everything produced from one diagram source.
#[derive(Clone, Debug)]
pub struct Rendered {
    pub family: Family,
    pub scene: Scene,
    pub layout: Layout,
    pub graph: Graph,
    pub diagnostics: Vec<Diagnostic>,
}

/// Parses, lays out and builds the display list for `source`.
///
/// Sources of unsupported families produce an empty scene. With a `seed` (the
/// previous graph and layout) node order stays stable while the user types.
pub fn render(
    source: &str,
    measurer: &Measurer,
    m: &Metrics,
    seed: Option<(&Graph, &Layout)>,
    cancel: &AtomicBool,
) -> Result<Rendered, LayoutError> {
    let parsed = parse(source);
    let positioned = layout::layout(&parsed.graph, measurer, m, seed, cancel)?;
    let scene = Scene::build(&parsed.graph, &positioned);
    Ok(Rendered {
        family: parsed.family,
        scene,
        layout: positioned,
        graph: parsed.graph,
        diagnostics: parsed.diagnostics,
    })
}
