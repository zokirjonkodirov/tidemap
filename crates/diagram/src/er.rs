//! Entity-relationship diagrams lowered onto the shared graph: entities are record boxes with
//! one attribute per row, relationships are edges with crow's foot markers.

use std::collections::HashMap;

use crate::graph::{ArrowKind, Endpoint, Family, Graph, LineKind, NodeId, Parsed, ShapeKind};
use crate::lexer::skip_preamble;
use crate::linewise::{Line, diagnostic, lines, new_edge, new_node, strip_comment, tokens};
use crate::parser::{clean_label, parse_direction};

const MAX_LINES: usize = 50_000;

struct E {
    g: Graph,
    index: HashMap<String, NodeId>,
    attrs: Vec<Vec<String>>,
}

impl E {
    fn entity(&mut self, line: &Line, name: &str) -> NodeId {
        let name = name.trim().trim_matches('"');
        if let Some(&id) = self.index.get(name) {
            return id;
        }
        let id = NodeId(self.g.nodes.len() as u32);
        self.g.nodes.push(new_node(
            name,
            name.to_string(),
            ShapeKind::Record,
            line.span_of(name),
        ));
        self.attrs.push(Vec::new());
        self.index.insert(name.to_string(), id);
        id
    }
}

/// Parses an ER diagram.
pub(crate) fn parse(source: &str) -> Parsed {
    let mut e = E {
        g: Graph::default(),
        index: HashMap::new(),
        attrs: Vec::new(),
    };
    let mut diags = Vec::new();
    let start = skip_preamble(source);
    let all = lines(source, start);
    let mut body: Option<NodeId> = None;
    let mut header = true;

    for line in all.iter().take(MAX_LINES) {
        let text = strip_comment(line.text).trim();
        if text.is_empty() {
            continue;
        }
        if header {
            header = false;
            if text.starts_with("erDiagram") {
                continue;
            }
        }
        if let Some(id) = body {
            if text.starts_with('}') {
                body = None;
            } else if let Some(row) = attribute(text) {
                e.attrs[id.0 as usize].push(row);
            }
            continue;
        }
        if let Some(rest) = text.strip_prefix("direction ") {
            if let Some(d) = parse_direction(rest) {
                e.g.direction = d;
            }
            continue;
        }
        if ["title", "accTitle", "accDescr", "style", "classDef", "class ", "click"]
            .iter()
            .any(|k| text.starts_with(k))
        {
            continue;
        }
        if let Some((open, inline)) = text.split_once('{')
            && !text.contains("--")
            && !text.contains("..")
        {
            let open = open.trim();
            let (name, alias) = match open.split_once('[') {
                Some((n, a)) => (n.trim(), Some(clean_label(a.trim_end().trim_end_matches(']')))),
                None => (open, None),
            };
            if name.is_empty() {
                continue;
            }
            let id = e.entity(line, name);
            if let Some(alias) = alias {
                e.g.nodes[id.0 as usize].label = alias;
            }
            if let Some((members, _)) = inline.split_once('}') {
                for m in members.split(';') {
                    if let Some(row) = attribute(m.trim()) {
                        e.attrs[id.0 as usize].push(row);
                    }
                }
            } else {
                body = Some(id);
            }
            continue;
        }
        if let Some(rel) = relationship(text) {
            let a = e.entity(line, rel.left);
            let b = e.entity(line, rel.right);
            e.g.edges.push(new_edge(
                Endpoint::Node(a),
                Endpoint::Node(b),
                (!rel.label.is_empty()).then_some(rel.label),
                if rel.identifying {
                    LineKind::Solid
                } else {
                    LineKind::Dotted
                },
                rel.left_card,
                rel.right_card,
                line.span_of(text),
            ));
            continue;
        }
        if !text.contains(char::is_whitespace) {
            e.entity(line, text);
            continue;
        }
        diags.push(diagnostic(
            source,
            line,
            line.span_of(text),
            "Unrecognised ER statement",
        ));
    }
    let mut g = e.g;
    for (i, rows) in e.attrs.iter().enumerate() {
        let node = &mut g.nodes[i];
        let mut lines = vec![node.label.clone()];
        lines.extend(rows.iter().cloned());
        node.label = lines.join("\n");
        node.sections = if rows.is_empty() {
            vec![1]
        } else {
            vec![1, rows.len() as u16]
        };
    }
    Parsed {
        family: Family::Er,
        graph: g,
        diagnostics: diags,
    }
}

/// `type name [PK, FK] "comment"` as tab separated columns.
fn attribute(text: &str) -> Option<String> {
    let quoted_comment = text.rfind('"').is_some() && text.matches('"').count() >= 2;
    let (body, comment) = if quoted_comment {
        let first = text.find('"')?;
        (&text[..first], text[first..].trim().trim_matches('"').to_string())
    } else {
        (text, String::new())
    };
    let t = tokens(body);
    if t.len() < 2 {
        return None;
    }
    let keys = t[2..].join(" ");
    let mut row = format!("{}\t{}", t[0], t[1]);
    if !keys.is_empty() || !comment.is_empty() {
        row.push('\t');
        row.push_str(&keys);
    }
    if !comment.is_empty() {
        row.push('\t');
        row.push_str(&comment);
    }
    Some(row)
}

struct Rel<'a> {
    left: &'a str,
    right: &'a str,
    left_card: ArrowKind,
    right_card: ArrowKind,
    identifying: bool,
    label: String,
}

fn left_card(s: &str) -> Option<ArrowKind> {
    Some(match s {
        "||" => ArrowKind::ExactlyOne,
        "|o" => ArrowKind::ZeroOrOne,
        "}o" => ArrowKind::ZeroOrMore,
        "}|" => ArrowKind::OneOrMore,
        _ => return None,
    })
}

fn right_card(s: &str) -> Option<ArrowKind> {
    Some(match s {
        "||" => ArrowKind::ExactlyOne,
        "o|" => ArrowKind::ZeroOrOne,
        "o{" => ArrowKind::ZeroOrMore,
        "|{" => ArrowKind::OneOrMore,
        _ => return None,
    })
}

/// `A ||--o{ B : label`
fn relationship(text: &str) -> Option<Rel<'_>> {
    let (rel, label) = match text.split_once(':') {
        Some((r, l)) => (r.trim(), clean_label(l.trim().trim_matches('"'))),
        None => (text.trim(), String::new()),
    };
    let (at, identifying) = match (rel.find("--"), rel.find("..")) {
        (Some(a), Some(b)) if b < a => (b, false),
        (Some(a), _) => (a, true),
        (None, Some(b)) => (b, false),
        (None, None) => return None,
    };
    let left_part = rel[..at].trim_end();
    let right_part = rel[at + 2..].trim_start();
    if left_part.len() < 2 || right_part.len() < 2 {
        return None;
    }
    let (left, lc) = left_part.split_at(left_part.len().checked_sub(2)?);
    let (rc, right) = right_part.split_at(2);
    Some(Rel {
        left: left.trim(),
        right: right.trim(),
        left_card: left_card(lc)?,
        right_card: right_card(rc)?,
        identifying,
        label,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_attributes_and_relationships() {
        let r = parse(
            "erDiagram\nCUSTOMER ||--o{ ORDER : places\nORDER ||..|{ LINE : contains\nCUSTOMER {\n  string name PK\n  string email UK \"login\"\n  int age\n}\n",
        );
        assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
        let g = r.graph;
        assert_eq!(g.nodes.len(), 3);
        assert_eq!(g.edges[0].tail, ArrowKind::ExactlyOne);
        assert_eq!(g.edges[0].head, ArrowKind::ZeroOrMore);
        assert_eq!(g.edges[0].label.as_deref(), Some("places"));
        assert_eq!(g.edges[1].line, LineKind::Dotted);
        assert_eq!(g.edges[1].head, ArrowKind::OneOrMore);
        let c = &g.nodes[0];
        assert_eq!(c.sections, vec![1, 3]);
        assert!(c.label.contains("string\tname\tPK"), "{:?}", c.label);
        assert!(c.label.contains("string\temail\tUK\tlogin"), "{:?}", c.label);
    }

    #[test]
    fn garbage_is_survivable() {
        for s in ["erDiagram", "erDiagram\n}\nA ||--\n||--|| B\n{{\n"] {
            let _ = parse(s);
        }
    }
}
