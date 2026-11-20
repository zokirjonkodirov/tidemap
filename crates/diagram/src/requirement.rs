//! Requirement diagrams lowered onto the shared graph: requirements and elements are record
//! boxes, relationships are labelled arrows.

use std::collections::HashMap;

use crate::graph::{ArrowKind, Endpoint, Family, Graph, LineKind, NodeId, Parsed, ShapeKind};
use crate::lexer::skip_preamble;
use crate::linewise::{diagnostic, lines, new_edge, new_node, split_word, strip_comment};
use crate::parser::{clean_label, parse_direction};

const MAX_LINES: usize = 20_000;

struct Item {
    kind: String,
    fields: Vec<(String, String)>,
}

/// Parses a requirement diagram.
pub(crate) fn parse(source: &str) -> Parsed {
    let start = skip_preamble(source);
    let mut g = Graph::default();
    let mut diags = Vec::new();
    let mut index: HashMap<String, NodeId> = HashMap::new();
    let mut items: Vec<Item> = Vec::new();
    let mut body: Option<usize> = None;
    let mut header = true;
    let mut node_of = |g: &mut Graph, items: &mut Vec<Item>, name: &str, kind: &str, span| -> NodeId {
        let name = name.trim().trim_matches('"');
        if let Some(&id) = index.get(name) {
            return id;
        }
        let id = NodeId(g.nodes.len() as u32);
        g.nodes
            .push(new_node(name, name.to_string(), ShapeKind::Record, span));
        items.push(Item {
            kind: kind.to_string(),
            fields: Vec::new(),
        });
        index.insert(name.to_string(), id);
        id
    };
    for line in lines(source, start).iter().take(MAX_LINES) {
        let text = strip_comment(line.text).trim();
        if text.is_empty() {
            continue;
        }
        if header {
            header = false;
            if text.starts_with("requirementDiagram") {
                continue;
            }
        }
        if let Some(i) = body {
            if text.starts_with('}') {
                body = None;
            } else if let Some((k, v)) = text.split_once(':') {
                items[i]
                    .fields
                    .push((k.trim().to_ascii_lowercase(), clean_label(v.trim().trim_matches('"'))));
            }
            continue;
        }
        let (word, rest) = split_word(text);
        if word == "direction" {
            if let Some(d) = parse_direction(rest) {
                g.direction = d;
            }
            continue;
        }
        let kind = match word {
            "requirement" => Some("requirement"),
            "functionalRequirement" => Some("functional requirement"),
            "interfaceRequirement" => Some("interface requirement"),
            "performanceRequirement" => Some("performance requirement"),
            "physicalRequirement" => Some("physical requirement"),
            "designConstraint" => Some("design constraint"),
            "element" => Some("element"),
            _ => None,
        };
        if let Some(kind) = kind {
            let name = rest.trim_end_matches('{').trim();
            let id = node_of(&mut g, &mut items, name, kind, line.span_of(name));
            if rest.trim_end().ends_with('{') {
                body = Some(id.0 as usize);
            }
            continue;
        }
        // `a - satisfies -> b` or `b <- satisfies - a`
        if let Some((left, tail)) = text.split_once(" - ")
            && let Some((rel, right)) = tail.split_once(" -> ")
        {
            let a = node_of(&mut g, &mut items, left, "", line.span_of(left));
            let b = node_of(&mut g, &mut items, right, "", line.span_of(right));
            g.edges.push(new_edge(
                Endpoint::Node(a),
                Endpoint::Node(b),
                Some(format!("\u{ab}{}\u{bb}", rel.trim())),
                LineKind::Dotted,
                ArrowKind::None,
                ArrowKind::Arrow,
                line.span_of(text),
            ));
            continue;
        }
        if let Some((left, tail)) = text.split_once(" <- ")
            && let Some((rel, right)) = tail.split_once(" - ")
        {
            let a = node_of(&mut g, &mut items, right, "", line.span_of(right));
            let b = node_of(&mut g, &mut items, left, "", line.span_of(left));
            g.edges.push(new_edge(
                Endpoint::Node(a),
                Endpoint::Node(b),
                Some(format!("\u{ab}{}\u{bb}", rel.trim())),
                LineKind::Dotted,
                ArrowKind::None,
                ArrowKind::Arrow,
                line.span_of(text),
            ));
            continue;
        }
        if !text.starts_with("style") && !text.starts_with("classDef") && !text.starts_with("class ") {
            diags.push(diagnostic(
                source,
                line,
                line.span_of(text),
                "Unrecognised requirement statement",
            ));
        }
    }
    for (i, item) in items.iter().enumerate() {
        let node = &mut g.nodes[i];
        let mut lines = Vec::new();
        if !item.kind.is_empty() {
            lines.push(format!("\u{ab}{}\u{bb}", item.kind));
        }
        lines.push(node.key.clone());
        let head = lines.len();
        let mut rows = Vec::new();
        for (k, v) in &item.fields {
            let label = match k.as_str() {
                "id" => "Id",
                "text" => "Text",
                "risk" => "Risk",
                "verifymethod" => "Verification",
                "type" => "Type",
                "docref" => "Doc Ref",
                other => other,
            };
            rows.push(format!("{label}: {v}"));
        }
        let n = rows.len();
        lines.extend(rows);
        node.label = lines.join("\n");
        node.sections = if n == 0 {
            vec![head as u16]
        } else {
            vec![head as u16, n as u16]
        };
    }
    Parsed {
        family: Family::Requirement,
        graph: g,
        diagnostics: diags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requirements_elements_and_relations() {
        let r = parse("requirementDiagram\n requirement test_req {\n id: 1\n text: the text\n risk: high\n verifymethod: test\n }\n element ent {\n type: simulation\n }\n ent - satisfies -> test_req\n");
        assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
        let g = r.graph;
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.nodes[0].sections, vec![2, 4]);
        assert!(g.nodes[0].label.contains("Verification: test"));
        assert_eq!(g.edges.len(), 1);
        assert_eq!(g.edges[0].label.as_deref(), Some("\u{ab}satisfies\u{bb}"));
    }
}
