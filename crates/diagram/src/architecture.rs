//! `architecture-beta` diagrams lowered onto the shared graph: groups are clusters, services
//! are nodes (databases are cylinders), connections are edges.

use std::collections::HashMap;

use crate::graph::{
    ArrowKind, Cluster, ClusterId, Direction, Endpoint, Family, Graph, LineKind, NodeId, Parsed,
    ShapeKind, StyleProps,
};
use crate::lexer::skip_preamble;
use crate::linewise::{diagnostic, lines, new_edge, new_node, split_word, strip_comment};
use crate::parser::clean_label;

const MAX_LINES: usize = 20_000;

/// `id(icon)[Title] in parent` -> (id, icon, title, parent)
fn declaration(rest: &str) -> Option<(String, String, String, Option<String>)> {
    let rest = rest.trim();
    let id_end = rest.find(['(', '[', ' ']).unwrap_or(rest.len());
    let id = rest[..id_end].to_string();
    if id.is_empty() {
        return None;
    }
    let mut tail = rest[id_end..].trim_start();
    let mut icon = String::new();
    if let Some(r) = tail.strip_prefix('(')
        && let Some((i, after)) = r.split_once(')')
    {
        icon = i.trim().to_string();
        tail = after.trim_start();
    }
    let mut title = String::new();
    if let Some(r) = tail.strip_prefix('[')
        && let Some((t, after)) = r.split_once(']')
    {
        title = clean_label(t);
        tail = after.trim_start();
    }
    let parent = tail.strip_prefix("in ").map(|p| p.trim().to_string());
    Some((id, icon, title, parent))
}

/// Parses an architecture diagram.
pub(crate) fn parse(source: &str) -> Parsed {
    let start = skip_preamble(source);
    let mut g = Graph {
        direction: Direction::LR,
        ..Graph::default()
    };
    let mut diags = Vec::new();
    let mut groups: HashMap<String, ClusterId> = HashMap::new();
    let mut nodes: HashMap<String, NodeId> = HashMap::new();
    let mut header = true;
    for line in lines(source, start).iter().take(MAX_LINES) {
        let text = strip_comment(line.text).trim();
        if text.is_empty() {
            continue;
        }
        if header {
            header = false;
            if text.starts_with("architecture") {
                continue;
            }
        }
        let (word, rest) = split_word(text);
        let span = line.span_of(text);
        match word {
            "group" => {
                let Some((id, _icon, title, parent)) = declaration(rest) else {
                    continue;
                };
                let cid = ClusterId(g.clusters.len() as u32);
                g.clusters.push(Cluster {
                    key: id.clone(),
                    title: if title.is_empty() { id.clone() } else { title },
                    parent: parent.and_then(|p| groups.get(&p).copied()),
                    direction: None,
                    classes: Vec::new(),
                    style: StyleProps::default(),
                    span,
                });
                groups.insert(id, cid);
            }
            "service" | "junction" => {
                let Some((id, icon, title, parent)) = declaration(rest) else {
                    continue;
                };
                let shape = match (word, icon.as_str()) {
                    ("junction", _) => ShapeKind::Circle,
                    (_, "database") => ShapeKind::Cylinder,
                    (_, "disk") => ShapeKind::Cylinder,
                    (_, "internet") | (_, "cloud") => ShapeKind::Stadium,
                    _ => ShapeKind::Rounded,
                };
                let label = if word == "junction" {
                    String::new()
                } else if title.is_empty() {
                    id.clone()
                } else {
                    title
                };
                let nid = NodeId(g.nodes.len() as u32);
                let mut node = new_node(&id, label, shape, span);
                node.cluster = parent.and_then(|p| groups.get(&p).copied());
                g.nodes.push(node);
                nodes.insert(id, nid);
            }
            _ => {
                // `a:R -- L:b`, `a:R --> L:b`, `a:R <--> L:b`
                let arrow_at = text.find("--");
                let Some(at) = arrow_at else {
                    diags.push(diagnostic(source, line, span, "Unrecognised architecture statement"));
                    continue;
                };
                let left = text[..at].trim_end();
                let tail = &text[at..];
                let dashes = tail.chars().take_while(|c| *c == '-').count();
                let after = &tail[dashes..];
                let head = after.starts_with('>');
                let right = after.trim_start_matches('>').trim_start();
                let tail_arrow = left.ends_with('<');
                let left = left.trim_end_matches('<').trim_end();
                let name = |s: &str| -> String {
                    let s = s.trim();
                    // `a:R` or `L:b`, with optional `{group}`.
                    let s = s.trim_end_matches("{group}").trim();
                    match s.split_once(':') {
                        Some((a, b)) if a.len() == 1 && a.chars().all(|c| "LRTB".contains(c)) => {
                            b.trim().to_string()
                        }
                        Some((a, _)) => a.trim().to_string(),
                        None => s.to_string(),
                    }
                };
                let (Some(&a), Some(&b)) = (nodes.get(&name(left)), nodes.get(&name(right))) else {
                    continue;
                };
                g.edges.push(new_edge(
                    Endpoint::Node(a),
                    Endpoint::Node(b),
                    None,
                    LineKind::Solid,
                    if tail_arrow { ArrowKind::Arrow } else { ArrowKind::None },
                    if head { ArrowKind::Arrow } else { ArrowKind::None },
                    span,
                ));
            }
        }
    }
    Parsed {
        family: Family::Architecture,
        graph: g,
        diagnostics: diags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_services_and_edges() {
        let r = parse("architecture-beta\n group api(cloud)[API]\n service db(database)[Database] in api\n service server(server)[Server] in api\n db:L -- R:server\n server:T --> B:db\n");
        assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
        let g = r.graph;
        assert_eq!(g.clusters.len(), 1);
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.nodes[0].shape, ShapeKind::Cylinder);
        assert_eq!(g.nodes[0].cluster, Some(ClusterId(0)));
        assert_eq!(g.edges.len(), 2);
        assert_eq!(g.edges[1].head, ArrowKind::Arrow);
    }
}
