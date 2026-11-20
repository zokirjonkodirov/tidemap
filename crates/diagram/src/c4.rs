//! C4 diagrams (`C4Context`, `C4Container`, ...) lowered onto the shared graph: elements are
//! record boxes in the C4 colours, boundaries are dashed clusters, `Rel` calls are arrows.

use std::collections::HashMap;

use crate::graph::{
    ArrowKind, Cluster, ClusterId, Endpoint, Family, Graph, LineKind, NodeId, Parsed, ShapeKind,
    Span, StyleProps,
};
use crate::lexer::skip_preamble;
use crate::linewise::{Line, diagnostic, lines, new_edge, new_node, strip_comment};
use crate::parser::clean_label;

const MAX_LINES: usize = 20_000;

/// Splits the arguments of a macro call, honouring double quotes.
fn args(inner: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut depth = 0;
    for c in inner.chars() {
        match c {
            '"' => quoted = !quoted,
            '(' | '[' | '{' if !quoted => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' | '}' if !quoted => {
                depth -= 1;
                cur.push(c);
            }
            ',' if !quoted && depth == 0 => out.push(std::mem::take(&mut cur).trim().to_string()),
            c => cur.push(c),
        }
    }
    out.push(cur.trim().to_string());
    out.into_iter()
        .filter(|a| !a.starts_with('$'))
        .map(|a| a.trim_matches('"').to_string())
        .collect()
}

fn rgb(hex: u32, alpha: u8) -> [u8; 4] {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8, alpha]
}

/// Fill, stroke and text colour of an element macro.
fn colors(name: &str) -> ([u8; 4], [u8; 4], [u8; 4]) {
    let white = rgb(0xffffff, 255);
    let dark = rgb(0x1f2937, 255);
    if name.starts_with("Person") {
        let ext = name.ends_with("_Ext");
        return if ext {
            (rgb(0x686868, 255), rgb(0x4b4b4b, 255), white)
        } else {
            (rgb(0x08427b, 255), rgb(0x073b6f, 255), white)
        };
    }
    let ext = name.ends_with("_Ext");
    if ext {
        return (rgb(0x999999, 255), rgb(0x8a8a8a, 255), white);
    }
    if name.starts_with("System") {
        (rgb(0x1168bd, 255), rgb(0x3c7fc0, 255), white)
    } else if name.starts_with("Container") {
        (rgb(0x438dd5, 255), rgb(0x3c7fc0, 255), white)
    } else {
        (rgb(0x85bbf0, 255), rgb(0x78a8d8, 255), dark)
    }
}

/// Parses a C4 diagram.
pub(crate) fn parse(source: &str) -> Parsed {
    let start = skip_preamble(source);
    let mut g = Graph::default();
    let mut diags = Vec::new();
    let mut index: HashMap<String, NodeId> = HashMap::new();
    let mut stack: Vec<ClusterId> = Vec::new();
    let mut header = true;
    let mut pending_open: Option<()> = None;
    let mut rels: Vec<(String, String, String, String, bool, Span)> = Vec::new();
    for line in lines(source, start).iter().take(MAX_LINES) {
        let text = strip_comment(line.text).trim();
        if text.is_empty() {
            continue;
        }
        if header {
            header = false;
            if text.starts_with("C4") {
                continue;
            }
        }
        if text == "}" {
            stack.pop();
            continue;
        }
        if text == "{" {
            pending_open = Some(());
            continue;
        }
        let _ = &pending_open;
        let Some(open) = text.find('(') else {
            if text.starts_with("title") {
                continue;
            }
            diags.push(diagnostic(source, line, line.span_of(text), "Unrecognised C4 statement"));
            continue;
        };
        let name = text[..open].trim();
        let close = text.rfind(')').unwrap_or(text.len());
        let a = args(&text[open + 1..close.max(open + 1)]);
        let tail = text[close.min(text.len())..].trim_start_matches(')').trim();
        let span = line.span_of(text);
        let get = |i: usize| a.get(i).cloned().unwrap_or_default();
        if name.ends_with("Boundary") || name == "Node" || name == "Deployment_Node" || name == "Node_L" || name == "Node_R" {
            let id = ClusterId(g.clusters.len() as u32);
            let label = clean_label(&get(1));
            g.clusters.push(Cluster {
                key: get(0),
                title: if label.is_empty() { get(0) } else { label },
                parent: stack.last().copied(),
                direction: None,
                classes: Vec::new(),
                style: StyleProps {
                    dashed: true,
                    ..StyleProps::default()
                },
                span,
            });
            if tail.starts_with('{') || tail.is_empty() {
                stack.push(id);
            }
            continue;
        }
        if name.starts_with("Rel") || name == "BiRel" {
            // Rel(from, to, label, tech)
            rels.push((get(0), get(1), clean_label(&get(2)), get(3), name == "BiRel", span));
            continue;
        }
        if name.starts_with("Update") || name.starts_with("Add") || name.starts_with("Layout") || name == "title" || name == "SHOW_LEGEND" {
            continue;
        }
        let known = name.starts_with("Person")
            || name.starts_with("System")
            || name.starts_with("Container")
            || name.starts_with("Component");
        if !known {
            diags.push(diagnostic(source, line, span, "Unknown C4 element"));
            continue;
        }
        let key = get(0);
        let label = clean_label(&get(1));
        let (tech, descr) = if name.starts_with("Person") || name.starts_with("System") {
            (String::new(), get(2))
        } else {
            (get(2), get(3))
        };
        let kind = if name.starts_with("Person") {
            "Person"
        } else if name.starts_with("System") {
            "System"
        } else if name.starts_with("Container") {
            "Container"
        } else {
            "Component"
        };
        let mut lines = vec![if label.is_empty() { key.clone() } else { label }];
        let mut tag = format!("[{kind}");
        if !tech.is_empty() {
            tag.push_str(&format!(": {tech}"));
        }
        tag.push(']');
        lines.push(tag);
        let mut n_rows = 1;
        if !descr.is_empty() {
            lines.extend(
                clean_label(&descr)
                    .split(' ')
                    .collect::<Vec<_>>()
                    .chunks(6)
                    .map(|c| c.join(" ")),
            );
            n_rows = lines.len() - 1;
        }
        let id = NodeId(g.nodes.len() as u32);
        let mut node = new_node(&key, lines.join("\n"), ShapeKind::Record, span);
        node.sections = vec![1, n_rows as u16];
        node.cluster = stack.last().copied();
        let (fill, stroke, color) = colors(name);
        node.style = StyleProps {
            fill: Some(fill),
            stroke: Some(stroke),
            color: Some(color),
            ..StyleProps::default()
        };
        g.nodes.push(node);
        index.insert(key, id);
    }
    for (from, to, label, tech, bi, span) in rels {
        let (Some(&a), Some(&b)) = (index.get(&from), index.get(&to)) else {
            continue;
        };
        let mut text = label;
        if !tech.is_empty() {
            text.push_str(&format!("\n[{tech}]"));
        }
        g.edges.push(new_edge(
            Endpoint::Node(a),
            Endpoint::Node(b),
            (!text.is_empty()).then_some(text),
            LineKind::Solid,
            if bi { ArrowKind::Arrow } else { ArrowKind::None },
            ArrowKind::Arrow,
            span,
        ));
    }
    let _: Option<&Line> = None;
    Parsed {
        family: Family::C4,
        graph: g,
        diagnostics: diags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elements_boundaries_and_relations() {
        let r = parse("C4Context\n title Demo\n Person(user, \"User\", \"A customer\")\n Enterprise_Boundary(b, \"Bank\") {\n System(sys, \"Banking\", \"Does things\")\n }\n System_Ext(mail, \"Mail\")\n Rel(user, sys, \"Uses\", \"HTTPS\")\n Rel(sys, mail, \"Sends\")\n");
        assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
        let g = r.graph;
        assert_eq!(g.nodes.len(), 3);
        assert_eq!(g.clusters.len(), 1);
        assert_eq!(g.nodes[1].cluster, Some(ClusterId(0)));
        assert_eq!(g.edges.len(), 2);
        assert_eq!(g.edges[0].label.as_deref(), Some("Uses\n[HTTPS]"));
        assert!(g.nodes[0].style.fill.is_some());
    }
}
