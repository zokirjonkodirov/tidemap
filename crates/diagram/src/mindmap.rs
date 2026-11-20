//! Mindmaps lowered onto a left-to-right flowchart: one node per line, edges from parent to
//! child, nesting taken from indentation.

use crate::graph::{
    ArrowKind, Direction, Endpoint, Family, Graph, LineKind, NodeId, Parsed, ShapeKind, Span,
};
use crate::lexer::skip_preamble;
use crate::linewise::{lines, new_edge, new_node, strip_comment};
use crate::parser::clean_label;

const MAX_LINES: usize = 20_000;

/// Parses a mindmap.
pub(crate) fn parse(source: &str) -> Parsed {
    let start = skip_preamble(source);
    let mut g = Graph {
        direction: Direction::LR,
        ..Graph::default()
    };
    // (indent, node) of the open ancestors.
    let mut stack: Vec<(usize, NodeId)> = Vec::new();
    let mut header = true;
    for line in lines(source, start).iter().take(MAX_LINES) {
        let body = strip_comment(line.text);
        let text = body.trim();
        if text.is_empty() {
            continue;
        }
        if header {
            header = false;
            if text.starts_with("mindmap") {
                continue;
            }
        }
        if text.starts_with("::") {
            continue;
        }
        let indent = body
            .chars()
            .take_while(|c| c.is_whitespace())
            .map(|c| if c == '\t' { 4 } else { 1 })
            .sum::<usize>();
        let (shape, label) = parse_node(text);
        let id = NodeId(g.nodes.len() as u32);
        let span = line.span_of(text);
        let shape = if stack.is_empty() && shape == ShapeKind::Rounded {
            ShapeKind::Circle
        } else {
            shape
        };
        let mut node = new_node(&format!("n{}", id.0), label, shape, span);
        node.label_span = Span { ..span };
        g.nodes.push(node);
        while stack.last().is_some_and(|&(i, _)| i >= indent) {
            stack.pop();
        }
        if let Some(&(_, parent)) = stack.last() {
            g.edges.push(new_edge(
                Endpoint::Node(parent),
                Endpoint::Node(id),
                None,
                LineKind::Solid,
                ArrowKind::None,
                ArrowKind::None,
                span,
            ));
        }
        stack.push((indent, id));
    }
    Parsed {
        family: Family::Mindmap,
        graph: g,
        diagnostics: Vec::new(),
    }
}

/// `id((text))`, `id(text)`, `id[text]`, `id))text((`, `id)text(`, `id{{text}}` or plain text.
fn parse_node(text: &str) -> (ShapeKind, String) {
    let open = text.find(['(', '[', ')', '{']);
    let Some(at) = open else {
        return (ShapeKind::Rounded, clean_label(text));
    };
    let rest = &text[at..];
    let table: [(&str, &str, ShapeKind); 7] = [
        ("((", "))", ShapeKind::Circle),
        ("))", "((", ShapeKind::Asymmetric),
        ("{{", "}}", ShapeKind::Hexagon),
        (")", "(", ShapeKind::Stadium),
        ("(", ")", ShapeKind::Rounded),
        ("[", "]", ShapeKind::Rect),
        ("{", "}", ShapeKind::Diamond),
    ];
    for (o, c, shape) in table {
        if let Some(inner) = rest.strip_prefix(o)
            && let Some(inner) = inner.trim_end().strip_suffix(c)
        {
            return (shape, clean_label(inner));
        }
    }
    (ShapeKind::Rounded, clean_label(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indentation_builds_a_tree() {
        let p = parse("mindmap\n  root((Root))\n    A\n      A1\n      A2\n    B[Box]\n");
        let g = p.graph;
        assert_eq!(g.nodes.len(), 5);
        assert_eq!(g.nodes[0].shape, ShapeKind::Circle);
        assert_eq!(g.nodes[0].label, "Root");
        assert_eq!(g.nodes[4].shape, ShapeKind::Rect);
        assert_eq!(g.edges.len(), 4);
        assert_eq!(g.edges[3].from, Endpoint::Node(NodeId(0)));
        assert_eq!(g.edges[2].from, Endpoint::Node(NodeId(1)));
    }
}
