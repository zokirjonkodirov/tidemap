//! State diagram parser: `stateDiagram` / `stateDiagram-v2` lowered onto the flowchart `Graph`.
//!
//! States are rounded nodes, composite states are clusters, `[*]` becomes a start or end
//! pseudo-state per scope, `<<choice>>`/`<<fork>>`/`<<join>>` become a small diamond or a bar,
//! and notes become note nodes joined to their state by a dotted edge.

use std::collections::{HashMap, HashSet};

use crate::graph::{
    ArrowKind, ClassDef, Cluster, ClusterId, Diagnostic, Edge, Endpoint, Family, Graph,
    LineKind, Node, NodeId, Parsed, ShapeKind, Span, StyleProps,
};
use crate::lexer::skip_preamble;
use crate::parser::{clean_label, merge_style, parse_color, parse_direction};

/// Longest accepted statement; longer lines are skipped.
const MAX_LINE: usize = 4096;

/// Parses a state diagram.
pub(crate) fn parse(source: &str) -> Parsed {
    let mut p = State::new(source);
    p.run();
    p.finish();
    Parsed {
        family: Family::State,
        graph: p.graph,
        diagnostics: p.diagnostics,
    }
}

/// A state mention inside a transition or note.
enum Ref {
    Node(NodeId),
    /// A composite state that may not be open yet.
    Cluster(String),
}

struct Fixup {
    edge: usize,
    to_side: bool,
    key: String,
}

struct State<'a> {
    src: &'a str,
    graph: Graph,
    diagnostics: Vec<Diagnostic>,
    nodes: HashMap<String, NodeId>,
    clusters: HashMap<String, ClusterId>,
    composites: HashSet<String>,
    stack: Vec<ClusterId>,
    class_ids: HashMap<String, u16>,
    /// Per node: authored `state "Title" as id` text.
    titles: HashMap<NodeId, String>,
    /// Per node: `id : description` lines.
    descriptions: HashMap<NodeId, Vec<String>>,
    fixups: Vec<Fixup>,
    notes: usize,
}

/// One source line with its byte offset.
#[derive(Clone, Copy)]
struct Line<'a> {
    text: &'a str,
    offset: usize,
}

impl<'a> Line<'a> {
    fn span(&self) -> Span {
        Span {
            start: self.offset as u32,
            end: (self.offset + self.text.len()) as u32,
        }
    }

    /// Span of `part`, which must be a sub-slice of this line.
    fn span_of(&self, part: &str) -> Span {
        let start = (part.as_ptr() as usize).saturating_sub(self.text.as_ptr() as usize);
        let start = start.min(self.text.len());
        let end = (start + part.len()).min(self.text.len());
        Span {
            start: (self.offset + start) as u32,
            end: (self.offset + end) as u32,
        }
    }
}

impl<'a> State<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            graph: Graph::default(),
            diagnostics: Vec::new(),
            nodes: HashMap::new(),
            clusters: HashMap::new(),
            composites: HashSet::new(),
            stack: Vec::new(),
            class_ids: HashMap::new(),
            titles: HashMap::new(),
            descriptions: HashMap::new(),
            fixups: Vec::new(),
            notes: 0,
        }
    }

    fn lines(&self) -> Vec<Line<'a>> {
        let start = skip_preamble(self.src);
        let mut out = Vec::new();
        let mut offset = start;
        for raw in self.src.get(start..).unwrap_or("").split_inclusive('\n') {
            let text = raw.trim_end_matches(['\n', '\r']);
            out.push(Line { text, offset });
            offset += raw.len();
        }
        out
    }

    fn error(&mut self, line: &Line, span: Span, message: impl Into<String>) {
        let before = self.src.get(..line.offset).unwrap_or("");
        let row = before.matches('\n').count() as u32 + 1;
        let column = self
            .src
            .get(line.offset..span.start as usize)
            .map_or(0, |s| s.chars().count()) as u32
            + 1;
        self.diagnostics.push(Diagnostic {
            line: row,
            column,
            span,
            message: message.into(),
        });
    }

    fn run(&mut self) {
        let lines = self.lines();
        // Composite names first, so transitions may name a state before it is opened.
        for line in &lines {
            if let Some(open) = composite_open(line.text.trim()) {
                self.composites.insert(open.key.to_string());
            }
        }
        let mut i = 0;
        let mut header = true;
        while i < lines.len() {
            let line = lines[i];
            i += 1;
            let text = strip_comment(line.text).trim();
            if text.is_empty() || text.len() > MAX_LINE {
                continue;
            }
            if header {
                header = false;
                if text.starts_with("stateDiagram") {
                    continue;
                }
            }
            if let Some(consumed) = self.statement(&lines, i - 1, text) {
                i = consumed;
            }
        }
    }

    /// Handles one statement; returns the index of the next line when it used several.
    fn statement(&mut self, lines: &[Line<'a>], at: usize, text: &'a str) -> Option<usize> {
        let line = lines[at];
        let (word, rest) = split_word(text);
        match word {
            "}" => {
                if self.stack.pop().is_none() {
                    self.error(&line, line.span_of(text), "Unmatched '}'");
                }
                return None;
            }
            "--" | "---" | "hide" | "scale" | "accTitle" | "accDescr" | "click" | "title" => {
                return None;
            }
            "direction" => {
                if let Some(d) = parse_direction(rest) {
                    match self.stack.last() {
                        Some(&c) => self.graph.clusters[c.0 as usize].direction = Some(d),
                        None => self.graph.direction = d,
                    }
                } else {
                    self.error(&line, line.span_of(rest), "Unknown direction");
                }
                return None;
            }
            "classDef" => {
                self.class_def(rest);
                return None;
            }
            "class" => {
                self.class_statement(&line, rest);
                return None;
            }
            "style" => {
                self.style_statement(&line, rest);
                return None;
            }
            "note" => return self.note(lines, at, rest),
            "state" if !rest.is_empty() => {
                self.state_statement(&line, rest);
                return None;
            }
            _ => {}
        }
        if text == "}" {
            self.stack.pop();
            return None;
        }
        if let Some(arrow) = text.find("-->") {
            self.transition(&line, &text[..arrow], &text[arrow + 3..]);
            return None;
        }
        if let Some((id, desc)) = split_colon(text) {
            if !id.contains(char::is_whitespace) {
                if let Some(node) = self.state_ref(&line, id) {
                    if let Ref::Node(n) = node {
                        let label = clean_label(desc);
                        if !label.is_empty() {
                            self.descriptions.entry(n).or_default().push(label);
                        }
                    }
                }
                return None;
            }
        }
        if text.contains(char::is_whitespace) && !text.contains(":::") {
            self.error(&line, line.span_of(text), "Unrecognised state statement");
            return None;
        }
        self.state_ref(&line, text);
        None
    }

    // ---- states ----------------------------------------------------------

    fn scope_key(&self) -> String {
        match self.stack.last() {
            Some(c) => format!("{}", c.0),
            None => "root".to_string(),
        }
    }

    fn declare(&mut self, key: &str, span: Span, shape: ShapeKind, label: String) -> NodeId {
        if let Some(&id) = self.nodes.get(key) {
            return id;
        }
        let id = NodeId(self.graph.nodes.len() as u32);
        self.graph.nodes.push(Node {
            key: key.to_string(),
            label,
            shape,
            cluster: self.stack.last().copied(),
            classes: Vec::new(),
            style: StyleProps::default(),
            span,
            label_span: span,
            sections: Vec::new(),
        });
        self.nodes.insert(key.to_string(), id);
        id
    }

    /// Resolves the text of a state mention (`id`, `id:::class`, `"quoted"`) to a state.
    fn state_ref(&mut self, line: &Line, raw: &str) -> Option<Ref> {
        self.state_ref_as(line, raw, None)
    }

    /// `end` is `Some(false)` for a transition source and `Some(true)` for its target, which
    /// decides whether `[*]` is a start or an end state.
    fn state_ref_as(&mut self, line: &Line, raw: &str, end: Option<bool>) -> Option<Ref> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        let span = line.span_of(raw);
        let (name, class) = match raw.split_once(":::") {
            Some((n, c)) => (n.trim(), Some(c.trim())),
            None => (raw, None),
        };
        let name = name.trim_matches('"');
        if name.is_empty() {
            return None;
        }
        let result = if name == "[*]" {
            let is_end = end.unwrap_or(false);
            let key = format!("[*]{}{}", if is_end { "end" } else { "start" }, self.scope_key());
            let shape = if is_end {
                ShapeKind::StateEnd
            } else {
                ShapeKind::StateStart
            };
            Ref::Node(self.declare(&key, span, shape, String::new()))
        } else if self.composites.contains(name) {
            Ref::Cluster(name.to_string())
        } else {
            Ref::Node(self.declare(name, span, ShapeKind::Rounded, name.to_string()))
        };
        if let Some(class) = class.filter(|c| !c.is_empty())
            && let Some(id) = self.class_id(class)
        {
            match &result {
                Ref::Node(n) => {
                    let node = &mut self.graph.nodes[n.0 as usize];
                    if !node.classes.contains(&id) {
                        node.classes.push(id);
                    }
                }
                Ref::Cluster(key) => {
                    if let Some(&c) = self.clusters.get(key) {
                        let cl = &mut self.graph.clusters[c.0 as usize];
                        if !cl.classes.contains(&id) {
                            cl.classes.push(id);
                        }
                    }
                }
            }
        }
        Some(result)
    }

    fn transition(&mut self, line: &Line, from: &str, rest: &str) {
        let (to, label) = match split_colon(rest) {
            Some((t, l)) => (t, Some(clean_label(l)).filter(|l| !l.is_empty())),
            None => (rest, None),
        };
        let (Some(a), Some(b)) = (
            self.state_ref_as(line, from, Some(false)),
            self.state_ref_as(line, to, Some(true)),
        ) else {
            self.error(line, line.span(), "A transition needs a state on both sides");
            return;
        };
        self.add_edge(a, b, label, LineKind::Solid, ArrowKind::Arrow, line.span());
    }

    fn add_edge(
        &mut self,
        from: Ref,
        to: Ref,
        label: Option<String>,
        line: LineKind,
        head: ArrowKind,
        span: Span,
    ) {
        let index = self.graph.edges.len();
        let mut pending = Vec::new();
        let mut endpoint = |r: Ref, to_side: bool| match r {
            Ref::Node(n) => Endpoint::Node(n),
            Ref::Cluster(key) => {
                pending.push((to_side, key));
                Endpoint::Node(NodeId(0))
            }
        };
        let from = endpoint(from, false);
        let to = endpoint(to, true);
        self.graph.edges.push(Edge {
            from,
            to,
            label,
            line,
            head,
            tail: ArrowKind::None,
            min_len: 1,
            style: StyleProps::default(),
            span,
        });
        for (to_side, key) in pending {
            self.fixups.push(Fixup {
                edge: index,
                to_side,
                key,
            });
        }
    }

    /// `state "Title" as id`, `state id {`, `state id <<choice>>` and `state id`.
    fn state_statement(&mut self, line: &Line, rest: &str) {
        let rest = rest.trim();
        if let Some(open) = composite_open(&format!("state {rest}")) {
            let span = line.span_of(rest);
            let title = open.title.unwrap_or(open.key).to_string();
            let parent = self.stack.last().copied();
            let id = ClusterId(self.graph.clusters.len() as u32);
            self.graph.clusters.push(Cluster {
                key: open.key.to_string(),
                title,
                parent,
                direction: None,
                classes: Vec::new(),
                style: StyleProps::default(),
                span,
            });
            self.clusters.insert(open.key.to_string(), id);
            self.stack.push(id);
            return;
        }
        let (name, title, tail) = if let Some(quoted) = rest.strip_prefix('"') {
            let Some(end) = quoted.find('"') else {
                self.error(line, line.span_of(rest), "Unterminated state name");
                return;
            };
            let title = &quoted[..end];
            let after = quoted[end + 1..].trim_start();
            let Some(after) = after.strip_prefix("as") else {
                self.error(line, line.span_of(rest), "Expected 'as' after the state name");
                return;
            };
            let (id, tail) = split_word(after.trim_start());
            (id, Some(clean_label(title)), tail)
        } else {
            let (id, tail) = split_word(rest);
            (id, None, tail)
        };
        if name.is_empty() {
            return;
        }
        let kind = tail
            .trim()
            .strip_prefix("<<")
            .and_then(|t| t.strip_suffix(">>"))
            .or_else(|| {
                tail.trim()
                    .strip_prefix("[[")
                    .and_then(|t| t.strip_suffix("]]"))
            })
            .map(|k| k.trim().to_ascii_lowercase());
        let span = line.span_of(name);
        let id = match self.nodes.get(name) {
            Some(&id) => id,
            None => self.declare(name, span, ShapeKind::Rounded, name.to_string()),
        };
        match kind.as_deref() {
            Some("choice") => {
                let node = &mut self.graph.nodes[id.0 as usize];
                node.shape = ShapeKind::Choice;
                node.label = String::new();
            }
            Some("fork" | "join") => {
                let node = &mut self.graph.nodes[id.0 as usize];
                node.shape = ShapeKind::Bar;
                node.label = String::new();
            }
            _ => {}
        }
        if let Some(title) = title {
            self.titles.insert(id, title);
        }
        if let Some(desc) = tail.trim().strip_prefix(':') {
            let label = clean_label(desc);
            if !label.is_empty() {
                self.descriptions.entry(id).or_default().push(label);
            }
        }
    }

    // ---- notes -----------------------------------------------------------

    /// `note left of X : text` or a multi-line block closed by `end note`.
    fn note(&mut self, lines: &[Line<'a>], at: usize, rest: &str) -> Option<usize> {
        let line = lines[at];
        let (side, after) = split_word(rest);
        let side = side.to_ascii_lowercase();
        let after = after.trim_start();
        let Some(after) = after.strip_prefix("of").or_else(|| after.strip_prefix("over")) else {
            self.error(&line, line.span_of(rest), "Expected 'note left of <state>'");
            return None;
        };
        let after = after.trim_start();
        let (target, text, next) = match split_colon(after) {
            Some((t, body)) => (t.trim(), clean_label(body), None),
            None => {
                let mut body = Vec::new();
                let mut next = at + 1;
                while next < lines.len() {
                    let t = lines[next].text.trim();
                    next += 1;
                    if t == "end note" {
                        break;
                    }
                    body.push(t);
                }
                (after.trim(), clean_label(&body.join("\n")), Some(next))
            }
        };
        let Some(state) = self.state_ref(&line, target) else {
            self.error(&line, line.span(), "A note needs a state");
            return next;
        };
        self.notes += 1;
        let key = format!("[note]{}", self.notes);
        let span = line.span();
        let note = self.declare(&key, span, ShapeKind::Note, text);
        let before = matches!(side.as_str(), "left" | "top" | "above");
        let (a, b) = if before {
            (Ref::Node(note), state)
        } else {
            (state, Ref::Node(note))
        };
        self.add_edge(a, b, None, LineKind::Dotted, ArrowKind::None, span);
        next
    }

    // ---- classes ---------------------------------------------------------

    fn class_id(&mut self, name: &str) -> Option<u16> {
        if let Some(&id) = self.class_ids.get(name) {
            return Some(id);
        }
        let id = u16::try_from(self.graph.class_defs.len()).ok()?;
        self.graph.class_defs.push(ClassDef {
            name: name.to_string(),
            props: StyleProps::default(),
        });
        self.class_ids.insert(name.to_string(), id);
        Some(id)
    }

    fn class_def(&mut self, rest: &str) {
        let (names, props) = split_word(rest);
        let style = style_props(props);
        for name in names.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            if let Some(id) = self.class_id(name) {
                merge_style(&mut self.graph.class_defs[id as usize].props, &style);
            }
        }
    }

    fn class_statement(&mut self, line: &Line, rest: &str) {
        let mut words: Vec<&str> = rest.split_whitespace().collect();
        let Some(class) = words.pop() else { return };
        let Some(id) = self.class_id(class) else { return };
        for name in words
            .join(" ")
            .split([',', ' '])
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            match self.state_ref(line, name) {
                Some(Ref::Node(n)) => {
                    let node = &mut self.graph.nodes[n.0 as usize];
                    if !node.classes.contains(&id) {
                        node.classes.push(id);
                    }
                }
                Some(Ref::Cluster(key)) => {
                    if let Some(&c) = self.clusters.get(&key) {
                        let cl = &mut self.graph.clusters[c.0 as usize];
                        if !cl.classes.contains(&id) {
                            cl.classes.push(id);
                        }
                    }
                }
                None => {}
            }
        }
    }

    fn style_statement(&mut self, line: &Line, rest: &str) {
        let (targets, props) = split_word(rest);
        let style = style_props(props);
        for name in targets.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            match self.state_ref(line, name) {
                Some(Ref::Node(n)) => merge_style(&mut self.graph.nodes[n.0 as usize].style, &style),
                Some(Ref::Cluster(key)) => {
                    if let Some(&c) = self.clusters.get(&key) {
                        merge_style(&mut self.graph.clusters[c.0 as usize].style, &style);
                    }
                }
                None => {}
            }
        }
    }

    // ---- end of input ----------------------------------------------------

    fn finish(&mut self) {
        // Final labels: description lines replace the id; a quoted title replaces it too.
        for i in 0..self.graph.nodes.len() {
            let id = NodeId(i as u32);
            if self.graph.nodes[i].shape != ShapeKind::Rounded {
                continue;
            }
            let head = self
                .titles
                .get(&id)
                .cloned()
                .unwrap_or_else(|| self.graph.nodes[i].key.clone());
            if let Some(desc) = self.descriptions.get(&id) {
                // Name on top, a divider, then the description lines.
                let node = &mut self.graph.nodes[i];
                node.label = format!("{head}\n{}", desc.join("\n"));
                node.shape = ShapeKind::Record;
                node.sections = vec![1, desc.len() as u16];
            } else if let Some(title) = self.titles.get(&id) {
                self.graph.nodes[i].label = title.clone();
            }
        }
        for fix in std::mem::take(&mut self.fixups) {
            let endpoint = match self.clusters.get(&fix.key) {
                Some(&c) => Endpoint::Cluster(c),
                None => {
                    let span = self.graph.edges[fix.edge].span;
                    Endpoint::Node(self.declare(&fix.key, span, ShapeKind::Rounded, fix.key.clone()))
                }
            };
            let edge = &mut self.graph.edges[fix.edge];
            if fix.to_side {
                edge.to = endpoint;
            } else {
                edge.from = endpoint;
            }
        }
        if !self.stack.is_empty() {
            let span = Span {
                start: self.src.len().saturating_sub(1) as u32,
                end: self.src.len() as u32,
            };
            let n = self.src.matches('\n').count() as u32 + 1;
            self.diagnostics.push(Diagnostic {
                line: n,
                column: 1,
                span,
                message: "A composite state is not closed with '}'".into(),
            });
        }
    }
}

/// A `state` statement that opens a composite block.
struct Open<'a> {
    key: &'a str,
    title: Option<&'a str>,
}

/// Parses `state X {` and `state "Title" as X {`.
fn composite_open(text: &str) -> Option<Open<'_>> {
    let rest = text.strip_prefix("state")?.trim_start();
    let body = rest.strip_suffix('{')?.trim_end();
    if let Some(quoted) = body.strip_prefix('"') {
        let end = quoted.find('"')?;
        let after = quoted[end + 1..].trim_start().strip_prefix("as")?.trim();
        if after.is_empty() || after.contains(char::is_whitespace) {
            return None;
        }
        return Some(Open {
            key: after,
            title: Some(&quoted[..end]),
        });
    }
    if body.is_empty() || body.contains(char::is_whitespace) {
        return None;
    }
    Some(Open {
        key: body,
        title: None,
    })
}

fn strip_comment(line: &str) -> &str {
    match line.find("%%") {
        Some(i) => &line[..i],
        None => line,
    }
}

fn split_word(text: &str) -> (&str, &str) {
    let text = text.trim();
    match text.find(char::is_whitespace) {
        Some(i) => (&text[..i], text[i..].trim_start()),
        None => (text, ""),
    }
}

/// Splits at the first `:` that is not part of `:::`.
fn split_colon(text: &str) -> Option<(&str, &str)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b':' {
            if bytes.get(i + 1) == Some(&b':') && bytes.get(i + 2) == Some(&b':') {
                i += 3;
                continue;
            }
            return Some((text[..i].trim(), text[i + 1..].trim()));
        }
        i += 1;
    }
    None
}

fn style_props(text: &str) -> StyleProps {
    let mut style = StyleProps::default();
    for part in text.split(',') {
        let Some((key, value)) = part.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "fill" => style.fill = parse_color(value).or(style.fill),
            "stroke" => style.stroke = parse_color(value).or(style.stroke),
            "color" => style.color = parse_color(value).or(style.color),
            "stroke-width" => {
                if let Ok(w) = value.trim_end_matches("px").trim().parse::<f32>()
                    && w.is_finite()
                {
                    style.stroke_width = Some(w);
                }
            }
            "stroke-dasharray" => style.dashed = true,
            _ => {}
        }
    }
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(src: &str) -> Parsed {
        parse(src)
    }

    #[test]
    fn transitions_start_end_and_labels() {
        let r = p("stateDiagram-v2\n  [*] --> Still\n  Still --> Moving : push\n  Moving --> [*]\n");
        assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
        let g = r.graph;
        assert_eq!(g.nodes.len(), 4);
        assert_eq!(g.nodes[0].shape, ShapeKind::StateStart);
        assert_eq!(g.nodes[3].shape, ShapeKind::StateEnd);
        assert_eq!(g.edges.len(), 3);
        assert_eq!(g.edges[1].label.as_deref(), Some("push"));
    }

    #[test]
    fn composite_states_are_clusters_with_scoped_pseudo_states() {
        let r = p("stateDiagram-v2\n[*] --> First\nstate First {\n  [*] --> Inner\n  Inner --> [*]\n}\nFirst --> Done\n");
        assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
        let g = r.graph;
        assert_eq!(g.clusters.len(), 1);
        assert_eq!(g.clusters[0].title, "First");
        // Edge into the composite targets the cluster, also before it is opened.
        assert_eq!(g.edges[0].to, Endpoint::Cluster(ClusterId(0)));
        assert_eq!(g.edges[3].from, Endpoint::Cluster(ClusterId(0)));
        let inner_start = g.nodes.iter().find(|n| n.key.starts_with("[*]start0")).unwrap();
        assert_eq!(inner_start.cluster, Some(ClusterId(0)));
        let root_start = g.nodes.iter().find(|n| n.key == "[*]startroot").unwrap();
        assert_eq!(root_start.cluster, None);
    }

    #[test]
    fn titles_descriptions_choice_fork_and_notes() {
        let r = p("stateDiagram\nstate \"Long name\" as s1\ns2 : first\ns2 : second\nstate c <<choice>>\nstate f <<fork>>\ns1 --> c\nnote right of s1 : hello\nnote left of s2\n  a\n  b\nend note\n");
        assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
        let g = r.graph;
        let by = |k: &str| g.nodes.iter().find(|n| n.key == k).unwrap();
        assert_eq!(by("s1").label, "Long name");
        assert_eq!(by("s2").label, "s2\nfirst\nsecond");
        assert_eq!(by("s2").sections, vec![1, 2]);
        assert_eq!(by("c").shape, ShapeKind::Choice);
        assert_eq!(by("f").shape, ShapeKind::Bar);
        assert_eq!(by("[note]1").label, "hello");
        assert_eq!(by("[note]2").label, "a\nb");
        assert_eq!(by("[note]2").shape, ShapeKind::Note);
        assert!(g.edges.iter().any(|e| e.line == LineKind::Dotted));
    }

    #[test]
    fn classes_direction_and_errors() {
        let r = p("stateDiagram-v2\ndirection LR\nclassDef hot fill:#f00,color:white\nA:::hot --> B\nclass B hot\nthis is nonsense\nstate X {\n");
        let g = r.graph;
        assert_eq!(g.direction, crate::graph::Direction::LR);
        assert_eq!(g.class_defs.len(), 1);
        assert_eq!(g.nodes[0].classes, vec![0]);
        assert_eq!(g.nodes[1].classes, vec![0]);
        assert_eq!(r.diagnostics.len(), 2, "{:?}", r.diagnostics);
    }

    #[test]
    fn nothing_hangs_on_garbage() {
        for src in ["stateDiagram", "stateDiagram\n}}}\n", "stateDiagram\n--> -->\n", "stateDiagram\nstate\nstate \"x\n"] {
            let _ = p(src);
        }
    }
}
