//! Class diagrams lowered onto the shared graph: classes are record boxes, relations are edges
//! with UML end markers, namespaces are clusters.

use std::collections::HashMap;

use crate::graph::{
    ArrowKind, Cluster, ClusterId, Diagnostic, Endpoint, Family, Graph, LineKind, NodeId, Parsed,
    ShapeKind, Span, StyleProps,
};
use crate::lexer::skip_preamble;
use crate::linewise::{Line, diagnostic, lines, new_edge, new_node, split_word, strip_comment};
use crate::parser::{clean_label, parse_direction};

const MAX_LINES: usize = 50_000;

/// Parses a class diagram.
pub(crate) fn parse(source: &str) -> Parsed {
    let mut p = C {
        src: source,
        g: Graph::default(),
        diags: Vec::new(),
        index: HashMap::new(),
        data: Vec::new(),
        stack: Vec::new(),
        notes: 0,
    };
    p.run();
    p.finish();
    Parsed {
        family: Family::Class,
        graph: p.g,
        diagnostics: p.diags,
    }
}

#[derive(Default)]
struct ClassData {
    display: String,
    annotations: Vec<String>,
    attrs: Vec<String>,
    methods: Vec<String>,
}

struct C<'a> {
    src: &'a str,
    g: Graph,
    diags: Vec<Diagnostic>,
    index: HashMap<String, NodeId>,
    /// Per node (notes included): class text.
    data: Vec<ClassData>,
    stack: Vec<ClusterId>,
    notes: usize,
}

/// Splits `Name~T~` into the key and the display name.
fn split_generic(name: &str) -> (String, String) {
    match name.split_once('~') {
        Some((base, rest)) => {
            let inner = rest.trim_end_matches('~').replace('~', "<");
            (base.to_string(), format!("{base}<{inner}>"))
        }
        None => (name.to_string(), name.to_string()),
    }
}

impl<'a> C<'a> {
    fn class(&mut self, raw: &str, span: Span) -> NodeId {
        let (key, display) = split_generic(raw.trim().trim_matches('"'));
        if let Some(&id) = self.index.get(&key) {
            if display != key {
                self.data[id.0 as usize].display = display;
            }
            return id;
        }
        let id = NodeId(self.g.nodes.len() as u32);
        let mut node = new_node(&key, display.clone(), ShapeKind::Record, span);
        node.cluster = self.stack.last().copied();
        self.g.nodes.push(node);
        self.data.push(ClassData {
            display,
            ..ClassData::default()
        });
        self.index.insert(key, id);
        id
    }

    fn member(&mut self, id: NodeId, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        if let Some(a) = text.strip_prefix("<<").and_then(|t| t.strip_suffix(">>")) {
            self.data[id.0 as usize]
                .annotations
                .push(format!("\u{ab}{}\u{bb}", a.trim()));
            return;
        }
        let cleaned = clean_label(text).replace('~', "");
        let cleaned = cleaned.trim_end_matches(['$', '*']).to_string();
        let d = &mut self.data[id.0 as usize];
        if cleaned.contains('(') {
            d.methods.push(cleaned);
        } else {
            d.attrs.push(cleaned);
        }
    }

    fn run(&mut self) {
        let start = skip_preamble(self.src);
        let all = lines(self.src, start);
        let mut body: Option<NodeId> = None;
        let mut header = true;
        for line in all.iter().take(MAX_LINES) {
            let text = strip_comment(line.text).trim();
            if text.is_empty() {
                continue;
            }
            if header {
                header = false;
                if text.starts_with("classDiagram") {
                    continue;
                }
            }
            if let Some(id) = body {
                if text.starts_with('}') {
                    body = None;
                } else {
                    self.member(id, text);
                }
                continue;
            }
            self.statement(line, text, &mut body);
        }
    }

    fn statement(&mut self, line: &Line, text: &str, body: &mut Option<NodeId>) {
        let (word, rest) = split_word(text);
        match word {
            "}" => {
                self.stack.pop();
                return;
            }
            "direction" => {
                if let Some(d) = parse_direction(rest) {
                    self.g.direction = d;
                }
                return;
            }
            "class" => {
                self.class_statement(line, rest, body);
                return;
            }
            "namespace" => {
                let name = rest.trim_end_matches('{').trim();
                let id = ClusterId(self.g.clusters.len() as u32);
                self.g.clusters.push(Cluster {
                    key: name.to_string(),
                    title: name.to_string(),
                    parent: self.stack.last().copied(),
                    direction: None,
                    classes: Vec::new(),
                    style: StyleProps::default(),
                    span: line.span_of(text),
                });
                self.stack.push(id);
                return;
            }
            "note" => {
                self.note(line, rest);
                return;
            }
            "click" | "callback" | "link" | "style" | "classDef" | "cssClass" | "accTitle"
            | "accDescr" | "title" | "hide" | "href" => return,
            _ => {}
        }
        if let Some(rel) = find_rel(text) {
            self.relation(line, text, rel);
            return;
        }
        if let Some(rest) = text.strip_prefix("<<")
            && let Some((a, name)) = rest.split_once(">>")
        {
            let id = self.class(name, line.span_of(name));
            self.member(id, &format!("<<{}>>", a.trim()));
            return;
        }
        if let Some((name, member)) = text.split_once(':')
            && !name.trim().contains(char::is_whitespace)
        {
            let id = self.class(name, line.span_of(name));
            self.member(id, member);
            return;
        }
        if !text.contains(char::is_whitespace) {
            self.class(text, line.span_of(text));
            return;
        }
        self.diags.push(diagnostic(
            self.src,
            line,
            line.span_of(text),
            "Unrecognised class statement",
        ));
    }

    fn class_statement(&mut self, line: &Line, rest: &str, body: &mut Option<NodeId>) {
        let rest = rest.trim();
        let name_end = rest
            .find(|c: char| c.is_whitespace() || c == '{' || c == '[' || c == ':')
            .unwrap_or(rest.len());
        let name = &rest[..name_end];
        if name.is_empty() {
            return;
        }
        let id = self.class(name, line.span_of(name));
        let mut after = rest[name_end..].trim();
        if let Some(label) = after.strip_prefix('[')
            && let Some((l, tail)) = label.split_once(']')
        {
            self.data[id.0 as usize].display = clean_label(l);
            after = tail.trim();
        }
        if let Some(inline) = after.strip_prefix('{') {
            match inline.split_once('}') {
                Some((members, _)) => {
                    for m in members.split(';') {
                        self.member(id, m);
                    }
                }
                None => {
                    for m in inline.split(';') {
                        self.member(id, m);
                    }
                    *body = Some(id);
                }
            }
        }
    }

    fn note(&mut self, line: &Line, rest: &str) {
        let (target, text) = match rest.strip_prefix("for ") {
            Some(r) => {
                let r = r.trim();
                match r.split_once(char::is_whitespace) {
                    Some((t, tx)) => (Some(t), tx),
                    None => (None, r),
                }
            }
            None => (None, rest),
        };
        let text = clean_label(text.trim().trim_matches('"'));
        self.notes += 1;
        let key = format!("[note]{}", self.notes);
        let span = line.span_of(rest);
        let note = NodeId(self.g.nodes.len() as u32);
        self.g
            .nodes
            .push(new_node(&key, text, ShapeKind::Note, span));
        self.data.push(ClassData::default());
        if let Some(t) = target {
            let class = self.class(t, line.span_of(t));
            self.g.edges.push(new_edge(
                Endpoint::Node(class),
                Endpoint::Node(note),
                None,
                LineKind::Dotted,
                ArrowKind::None,
                ArrowKind::None,
                span,
            ));
        }
    }

    fn relation(&mut self, line: &Line, text: &str, rel: Rel) {
        let left = &text[..rel.at];
        let right = &text[rel.at + 2..];
        let (left_marker, left) = left_marker(left);
        let (right_marker, right) = right_marker(right);
        let left = left.trim();
        let (card_l, name_l) = if left.ends_with('"')
            && let Some(open) = left[..left.len() - 1].rfind('"')
        {
            (
                left[open + 1..left.len() - 1].to_string(),
                left[..open].trim(),
            )
        } else {
            (String::new(), left.rsplit(char::is_whitespace).next().unwrap_or(left))
        };
        let right = right.trim();
        let (card_r, rest) = match right.strip_prefix('"') {
            Some(r) => match r.split_once('"') {
                Some((c, rest)) => (c.to_string(), rest.trim_start()),
                None => (String::new(), right),
            },
            None => (String::new(), right),
        };
        let (name_r, label) = match rest.split_once(':') {
            Some((n, l)) => (n.trim(), clean_label(l)),
            None => (rest.trim(), String::new()),
        };
        if name_l.is_empty() || name_r.is_empty() {
            self.diags.push(diagnostic(
                self.src,
                line,
                line.span_of(text),
                "A relation needs a class on both sides",
            ));
            return;
        }
        let a = self.class(name_l, line.span_of(name_l));
        let b = self.class(name_r, line.span_of(name_r));
        let label = [card_l.as_str(), label.as_str(), card_r.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("  ");
        self.g.edges.push(new_edge(
            Endpoint::Node(a),
            Endpoint::Node(b),
            (!label.is_empty()).then_some(label),
            if rel.dotted {
                LineKind::Dotted
            } else {
                LineKind::Solid
            },
            left_marker,
            right_marker,
            line.span_of(text),
        ));
    }

    fn finish(&mut self) {
        for (i, d) in self.data.iter().enumerate() {
            let node = &mut self.g.nodes[i];
            if node.shape != ShapeKind::Record {
                continue;
            }
            let mut lines: Vec<String> = d.annotations.clone();
            lines.push(d.display.clone());
            let head = lines.len();
            lines.extend(d.attrs.iter().cloned());
            lines.extend(d.methods.iter().cloned());
            node.label = lines.join("\n");
            node.sections = vec![head as u16, d.attrs.len() as u16, d.methods.len() as u16];
        }
    }
}

struct Rel {
    at: usize,
    dotted: bool,
}

/// First `--` or `..` outside quotes.
fn find_rel(text: &str) -> Option<Rel> {
    let b = text.as_bytes();
    let mut quoted = false;
    let mut i = 0;
    while i + 1 < b.len() {
        match b[i] {
            b'"' => quoted = !quoted,
            b'-' if !quoted && b[i + 1] == b'-' => {
                return Some(Rel {
                    at: i,
                    dotted: false,
                });
            }
            b'.' if !quoted && b[i + 1] == b'.' => {
                return Some(Rel {
                    at: i,
                    dotted: true,
                });
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn left_marker(left: &str) -> (ArrowKind, &str) {
    let t = left.trim_end();
    let separated =
        |rest: &str| rest.is_empty() || rest.ends_with(char::is_whitespace) || rest.ends_with('"');
    if let Some(r) = t.strip_suffix("<|") {
        return (ArrowKind::Triangle, r);
    }
    if let Some(r) = t.strip_suffix('*') {
        return (ArrowKind::DiamondFilled, r);
    }
    if let Some(r) = t.strip_suffix('o')
        && separated(r)
    {
        return (ArrowKind::Diamond, r);
    }
    if let Some(r) = t.strip_suffix('<') {
        return (ArrowKind::Arrow, r);
    }
    (ArrowKind::None, left)
}

fn right_marker(right: &str) -> (ArrowKind, &str) {
    let separated = |rest: &str| rest.is_empty() || rest.starts_with(char::is_whitespace);
    if let Some(r) = right.strip_prefix("|>") {
        return (ArrowKind::Triangle, r);
    }
    if let Some(r) = right.strip_prefix('*') {
        return (ArrowKind::DiamondFilled, r);
    }
    if let Some(r) = right.strip_prefix('o')
        && separated(r)
    {
        return (ArrowKind::Diamond, r);
    }
    if let Some(r) = right.strip_prefix('>') {
        return (ArrowKind::Arrow, r);
    }
    (ArrowKind::None, right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_members_and_relations() {
        let r = parse(
            "classDiagram\nclass Animal {\n  <<abstract>>\n  +String name\n  +eat() void\n}\nAnimal <|-- Dog\nDog : +bark()\nDog \"1\" *-- \"many\" Tail : has\nFoo ..> Bar\nFoo ..|> Baz\nclass Box~T~\n",
        );
        assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
        let g = r.graph;
        let animal = &g.nodes[0];
        assert_eq!(animal.sections, vec![2, 1, 1]);
        assert!(
            animal
                .label
                .starts_with("\u{ab}abstract\u{bb}\nAnimal\n+String name")
        );
        assert_eq!(g.edges[0].tail, ArrowKind::Triangle);
        assert_eq!(g.edges[0].head, ArrowKind::None);
        assert_eq!(g.edges[1].tail, ArrowKind::DiamondFilled);
        assert_eq!(g.edges[1].label.as_deref(), Some("1  has  many"));
        assert_eq!(g.edges[2].head, ArrowKind::Arrow);
        assert_eq!(g.edges[2].line, LineKind::Dotted);
        assert_eq!(g.edges[3].head, ArrowKind::Triangle);
        assert_eq!(g.nodes.last().unwrap().label, "Box<T>");
        let dog = g.nodes.iter().find(|n| n.key == "Dog").unwrap();
        assert_eq!(dog.sections, vec![1, 0, 1]);
    }

    #[test]
    fn garbage_is_survivable() {
        for s in [
            "classDiagram",
            "classDiagram\n}\n}\n",
            "classDiagram\nclass\nA <|--\n-- B\nnote for\n",
        ] {
            let _ = parse(s);
        }
    }
}
