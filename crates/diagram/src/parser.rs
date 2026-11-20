//! Flowchart parser: statements to `Graph`, with error recovery.

use std::collections::{HashMap, HashSet};

use crate::detect;
use crate::graph::{
    ClassDef, Cluster, ClusterId, Diagnostic, Direction, Edge, Endpoint, Family, Graph, Node,
    NodeId, Parsed, ShapeKind, Span, StyleProps,
};
use crate::lexer::{Lexer, LinkInfo, Tok, Token, skip_preamble, span};

/// Parses `source`. Non-flowchart families return an empty graph and no diagnostics.
pub fn parse(source: &str) -> Parsed {
    let family = detect(source);
    if family == Family::State {
        return crate::state::parse(source);
    }
    if matches!(
        family,
        Family::Pie | Family::Gantt | Family::Timeline | Family::Journey | Family::Quadrant
    ) {
        return crate::charts::parse(source, family);
    }
    if family == Family::Mindmap {
        return crate::mindmap::parse(source);
    }
    if family == Family::Class {
        return crate::class::parse(source);
    }
    if family == Family::Er {
        return crate::er::parse(source);
    }
    if family == Family::Sequence {
        return crate::seq::parse(source);
    }
    if family != Family::Flowchart {
        return Parsed {
            family,
            graph: Graph::default(),
            diagnostics: Vec::new(),
        };
    }
    let mut parser = Parser::new(source);
    parser.run();
    parser.finish();
    Parsed {
        family,
        graph: parser.graph,
        diagnostics: parser.diagnostics,
    }
}

/// A node mention in an edge statement.
struct NodeRef {
    endpoint: Endpoint,
    span: Span,
    /// Subgraph key whose cluster is not declared yet.
    pending: Option<String>,
}

/// An edge endpoint naming a subgraph declared later in the file.
struct Fixup {
    edge: usize,
    to_side: bool,
    key: String,
    span: Span,
}

/// A `linkStyle` statement, applied once all edges exist.
struct LinkStyleRule {
    indices: Vec<usize>,
    all: bool,
    style: StyleProps,
}

struct Parser<'a> {
    src: &'a str,
    lexer: Lexer<'a>,
    peeked: Option<Token>,
    graph: Graph,
    diagnostics: Vec<Diagnostic>,
    line_starts: Vec<usize>,
    nodes: HashMap<String, NodeId>,
    subgraph_keys: HashSet<String>,
    cluster_by_key: HashMap<String, ClusterId>,
    stack: Vec<ClusterId>,
    class_ids: HashMap<String, u16>,
    fixups: Vec<Fixup>,
    link_styles: Vec<LinkStyleRule>,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        let body_start = skip_preamble(src);
        let mut lexer = Lexer::new(src);
        lexer.seek(body_start);
        let mut line_starts = vec![0];
        line_starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
        let body = src.get(body_start..).unwrap_or("");
        Self {
            src,
            lexer,
            peeked: None,
            graph: Graph::default(),
            diagnostics: Vec::new(),
            line_starts,
            nodes: HashMap::new(),
            subgraph_keys: collect_subgraph_keys(body),
            cluster_by_key: HashMap::new(),
            stack: Vec::new(),
            class_ids: HashMap::new(),
            fixups: Vec::new(),
            link_styles: Vec::new(),
        }
    }

    // ---- token plumbing -------------------------------------------------

    fn peek(&mut self) -> Token {
        if let Some(t) = self.peeked {
            return t;
        }
        let t = self.lexer.next();
        self.peeked = Some(t);
        t
    }

    fn bump(&mut self) -> Token {
        match self.peeked.take() {
            Some(t) => t,
            None => self.lexer.next(),
        }
    }

    fn slice(&self, s: Span) -> &'a str {
        self.src.get(s.start as usize..s.end as usize).unwrap_or("")
    }

    /// Byte offset of `sub`, which must be a slice of the source.
    fn offset_of(&self, sub: &str) -> usize {
        let offset = (sub.as_ptr() as usize).saturating_sub(self.src.as_ptr() as usize);
        offset.min(self.src.len())
    }

    fn span_of_slice(&self, sub: &str) -> Span {
        let start = self.offset_of(sub);
        span(start, (start + sub.len()).min(self.src.len()))
    }

    fn locate(&self, offset: usize) -> (u32, u32) {
        let index = self
            .line_starts
            .partition_point(|&s| s <= offset)
            .saturating_sub(1);
        let line_start = self.line_starts.get(index).copied().unwrap_or(0);
        let column = self
            .src
            .get(line_start..offset)
            .map_or(0, |text| text.chars().count());
        (
            u32::try_from(index + 1).unwrap_or(u32::MAX),
            u32::try_from(column + 1).unwrap_or(u32::MAX),
        )
    }

    fn error(&mut self, at: Span, message: impl Into<String>) {
        let (line, column) = self.locate(at.start as usize);
        self.diagnostics.push(Diagnostic {
            line,
            column,
            span: at,
            message: message.into(),
        });
    }

    fn unexpected(&mut self, t: Token, expected: &str) {
        let message = match t.kind {
            Tok::Newline | Tok::Semi | Tok::Eof => format!("Expected {expected}"),
            _ => format!("Unexpected '{}'", self.slice(t.span)),
        };
        self.error(t.span, message);
    }

    /// Skips the rest of the current statement after an error on `offender`.
    fn recover_from(&mut self, offender: Token) {
        if matches!(offender.kind, Tok::Newline | Tok::Semi | Tok::Eof) {
            self.peeked = Some(offender);
            return;
        }
        if let Some(p) = self.peeked.take() {
            if matches!(p.kind, Tok::Newline | Tok::Semi | Tok::Eof) {
                self.peeked = Some(p);
                return;
            }
        }
        self.lexer.skip_line();
    }

    // ---- statements -----------------------------------------------------

    fn run(&mut self) {
        self.header();
        loop {
            let t = self.bump();
            match t.kind {
                Tok::Eof => break,
                Tok::Newline | Tok::Semi => {}
                Tok::Ident => {
                    self.statement(t);
                    let next = self.peek();
                    if !matches!(next.kind, Tok::Newline | Tok::Semi | Tok::Eof) {
                        let extra = self.bump();
                        self.unexpected(extra, "end of statement");
                        self.recover_from(extra);
                    }
                }
                _ => {
                    self.unexpected(t, "a statement");
                    self.recover_from(t);
                }
            }
        }
        for id in std::mem::take(&mut self.stack) {
            let at = self
                .graph
                .clusters
                .get(id.0 as usize)
                .map(|c| c.span)
                .unwrap_or_default();
            self.error(at, "Subgraph is missing end");
        }
    }

    fn header(&mut self) {
        let first = loop {
            let t = self.bump();
            if !matches!(t.kind, Tok::Newline | Tok::Semi) {
                break t;
            }
        };
        let text = self.slice(first.span);
        if first.kind != Tok::Ident || !(text == "graph" || text.starts_with("flowchart")) {
            self.peeked = Some(first);
            return;
        }
        let next = self.peek();
        if next.kind == Tok::Ident {
            if let Some(direction) = parse_direction(self.slice(next.span)) {
                self.bump();
                self.graph.direction = direction;
            }
        }
    }

    fn statement(&mut self, tok: Token) {
        let text = self.slice(tok.span);
        let spaced = matches!(
            self.src.as_bytes().get(tok.span.end as usize),
            Some(b' ' | b'\t')
        );
        match text {
            "subgraph" if spaced => self.subgraph(tok),
            "end" if self.rest_is_blank() => self.end_subgraph(tok),
            "direction" if spaced => {
                let rest = self.lexer.rest_of_line();
                if let (Some(direction), Some(id)) =
                    (parse_direction(self.slice(rest)), self.stack.last())
                {
                    if let Some(cluster) = self.graph.clusters.get_mut(id.0 as usize) {
                        cluster.direction = Some(direction);
                    }
                }
            }
            "classDef" if spaced => self.class_def(),
            "class" if spaced => self.class_statement(tok),
            "style" if spaced => self.style_statement(tok),
            "linkStyle" if spaced => self.link_style(),
            "click" if spaced => {
                self.lexer.rest_of_line();
            }
            "accTitle" | "accDescr" => self.skip_accessibility(),
            _ => self.node_statement(tok),
        }
    }

    fn rest_is_blank(&self) -> bool {
        let rest = self.lexer.remaining();
        let line = rest.split(['\n', ';']).next().unwrap_or("").trim();
        line.is_empty() || line.starts_with("%%")
    }

    fn skip_accessibility(&mut self) {
        let rest = self.lexer.rest_of_line();
        let text = self.slice(rest);
        if !text.starts_with('{') || text.contains('}') {
            return;
        }
        loop {
            if self.lexer.next().kind == Tok::Eof {
                break;
            }
            let rest = self.lexer.rest_of_line();
            if self.slice(rest).contains('}') {
                break;
            }
        }
    }

    // ---- subgraphs ------------------------------------------------------

    fn subgraph(&mut self, kw: Token) {
        let rest = self.lexer.rest_of_line();
        let (mut key, title) = subgraph_header(self.slice(rest));
        let id = ClusterId(u32::try_from(self.graph.clusters.len()).unwrap_or(u32::MAX));
        if key.is_empty() {
            key = format!("subgraph{}", id.0);
        }
        self.graph.clusters.push(Cluster {
            key: key.clone(),
            title: if title.is_empty() { key.clone() } else { title },
            parent: self.stack.last().copied(),
            direction: None,
            classes: Vec::new(),
            style: StyleProps::default(),
            span: span(kw.span.start as usize, rest.end as usize),
        });
        self.cluster_by_key.entry(key).or_insert(id);
        self.stack.push(id);
    }

    fn end_subgraph(&mut self, tok: Token) {
        match self.stack.pop() {
            Some(id) => {
                if let Some(cluster) = self.graph.clusters.get_mut(id.0 as usize) {
                    cluster.span.end = tok.span.end;
                }
            }
            None => self.error(tok.span, "Unexpected end"),
        }
    }

    // ---- nodes and edges ------------------------------------------------

    fn node_statement(&mut self, first: Token) {
        let Some(mut previous) = self.parse_group(first) else {
            return;
        };
        loop {
            let next = self.peek();
            match next.kind {
                Tok::Newline | Tok::Semi | Tok::Eof => return,
                Tok::Link => {}
                _ => {
                    let t = self.bump();
                    self.unexpected(t, "end of statement");
                    self.recover_from(t);
                    return;
                }
            }
            let link = self.bump();
            let info = self.lexer.link_info(&link);
            let mut label = info.inline_label.map(|s| clean_label(self.slice(s)));
            if label.is_none() && self.peek().kind == Tok::Pipe {
                self.bump();
                let (text, closed) = self.lexer.pipe_text();
                label = Some(clean_label(self.slice(text)));
                if closed {
                    self.bump();
                } else {
                    self.error(text, "Unclosed edge label");
                }
            }
            let target = self.bump();
            if target.kind != Tok::Ident {
                self.unexpected(target, "a node after the link");
                self.recover_from(target);
                return;
            }
            let Some(group) = self.parse_group(target) else {
                return;
            };
            let label = label.filter(|l| !l.is_empty());
            for from in &previous {
                for to in &group {
                    self.add_edge(from, to, &info, label.clone());
                }
            }
            previous = group;
        }
    }

    /// `node ('&' node)*`
    fn parse_group(&mut self, first: Token) -> Option<Vec<NodeRef>> {
        let mut refs = vec![self.parse_node(first)];
        while self.peek().kind == Tok::Amp {
            self.bump();
            let t = self.bump();
            if t.kind != Tok::Ident {
                self.unexpected(t, "a node after '&'");
                self.recover_from(t);
                return None;
            }
            refs.push(self.parse_node(t));
        }
        Some(refs)
    }

    /// `ident [shape] [@{...}] [:::class]*`
    fn parse_node(&mut self, ident: Token) -> NodeRef {
        let key = self.slice(ident.span);
        let mut end = ident.span.end;
        let mut label: Option<(String, Span)> = None;
        let mut shape: Option<ShapeKind> = None;
        let mut classes: Vec<&'a str> = Vec::new();

        if let Tok::Open(kind) = self.peek().kind {
            let open = self.bump();
            let (text_span, closed) = self.lexer.shape_text(kind);
            let mut final_shape = kind;
            if closed {
                let close = self.bump();
                final_shape = resolve_shape(kind, self.slice(close.span));
                end = close.span.end;
            } else {
                end = text_span.end;
                self.error(
                    span(open.span.start as usize, end as usize),
                    "Unclosed node label",
                );
            }
            label = Some((clean_label(self.slice(text_span)), text_span));
            shape = Some(final_shape);
        }

        if self.peek().kind == Tok::At {
            let at = self.bump();
            if self.peek().kind == Tok::LBrace {
                self.bump();
                let (body, closed) = self.lexer.brace_body();
                if closed {
                    end = self.bump().span.end;
                } else {
                    end = body.end;
                    self.error(body, "Unclosed '@{' block");
                }
                self.apply_at_props(self.slice(body), body, &mut label, &mut shape);
            } else {
                self.error(at.span, "Expected '{' after '@'");
            }
        }

        while self.peek().kind == Tok::Colon3 {
            self.bump();
            let name = self.peek();
            if name.kind == Tok::Ident {
                self.bump();
                classes.push(self.slice(name.span));
            } else {
                self.error(name.span, "Expected a class name after ':::'");
                break;
            }
        }

        let node_span = span(ident.span.start as usize, end as usize);
        if self.subgraph_keys.contains(key) {
            let pending = match self.cluster_by_key.get(key) {
                Some(_) => None,
                None => Some(key.to_string()),
            };
            let id = self.cluster_by_key.get(key).copied();
            for name in classes {
                if let (Some(id), Some(class)) = (id, self.class_id(name)) {
                    self.add_cluster_class(id, class);
                }
            }
            return NodeRef {
                endpoint: Endpoint::Cluster(id.unwrap_or(ClusterId(u32::MAX))),
                span: node_span,
                pending,
            };
        }
        let id = self.declare_node(key, node_span, label, shape);
        for name in classes {
            if let Some(class) = self.class_id(name) {
                self.add_node_class(id, class);
            }
        }
        NodeRef {
            endpoint: Endpoint::Node(id),
            span: node_span,
            pending: None,
        }
    }

    /// Applies `shape:` and `label:` entries of an `@{ ... }` block.
    fn apply_at_props(
        &self,
        text: &str,
        body: Span,
        label: &mut Option<(String, Span)>,
        shape: &mut Option<ShapeKind>,
    ) {
        for (key, value) in split_pairs(text) {
            match key {
                "shape" => *shape = Some(shape_from_name(value)),
                "label" => *label = Some((clean_label(value), body)),
                _ => {}
            }
        }
    }

    fn declare_node(
        &mut self,
        key: &str,
        node_span: Span,
        label: Option<(String, Span)>,
        shape: Option<ShapeKind>,
    ) -> NodeId {
        if let Some(&id) = self.nodes.get(key) {
            if let Some(node) = self.graph.nodes.get_mut(id.0 as usize) {
                if let Some((text, text_span)) = label {
                    node.label = text;
                    node.label_span = text_span;
                }
                if let Some(shape) = shape {
                    node.shape = shape;
                }
                // A node declared at the top level and later listed inside a
                // subgraph belongs to that subgraph (common in generated
                // diagrams: declare everything first, group afterwards). The
                // first subgraph to mention it wins.
                if node.cluster.is_none() {
                    node.cluster = self.stack.last().copied();
                }
            }
            return id;
        }
        let id = NodeId(u32::try_from(self.graph.nodes.len()).unwrap_or(u32::MAX));
        let (text, label_span) = label.unwrap_or_else(|| (key.to_string(), node_span));
        self.graph.nodes.push(Node {
            key: key.to_string(),
            label: text,
            shape: shape.unwrap_or_default(),
            cluster: self.stack.last().copied(),
            classes: Vec::new(),
            style: StyleProps::default(),
            span: node_span,
            label_span,
            sections: Vec::new(),
        });
        self.nodes.insert(key.to_string(), id);
        id
    }

    fn add_edge(&mut self, from: &NodeRef, to: &NodeRef, info: &LinkInfo, label: Option<String>) {
        let index = self.graph.edges.len();
        let start = from.span.start.min(to.span.start);
        let end = from.span.end.max(to.span.end);
        self.graph.edges.push(Edge {
            from: from.endpoint,
            to: to.endpoint,
            label,
            line: info.line,
            head: info.head,
            tail: info.tail,
            min_len: info.len.saturating_add(1),
            style: StyleProps::default(),
            span: Span { start, end },
        });
        for (node, to_side) in [(from, false), (to, true)] {
            if let Some(key) = &node.pending {
                self.fixups.push(Fixup {
                    edge: index,
                    to_side,
                    key: key.clone(),
                    span: node.span,
                });
            }
        }
    }

    // ---- classes and styles ---------------------------------------------

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

    fn add_node_class(&mut self, node: NodeId, class: u16) {
        if let Some(node) = self.graph.nodes.get_mut(node.0 as usize) {
            if !node.classes.contains(&class) {
                node.classes.push(class);
            }
        }
    }

    fn add_cluster_class(&mut self, cluster: ClusterId, class: u16) {
        if let Some(cluster) = self.graph.clusters.get_mut(cluster.0 as usize) {
            if !cluster.classes.contains(&class) {
                cluster.classes.push(class);
            }
        }
    }

    fn class_def(&mut self) {
        let rest = self.lexer.rest_of_line();
        let (names, props) = split_first_word(self.slice(rest));
        let mut style = StyleProps::default();
        self.apply_props(&mut style, props);
        for name in names.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            if let Some(id) = self.class_id(name) {
                if let Some(def) = self.graph.class_defs.get_mut(id as usize) {
                    merge_style(&mut def.props, &style);
                }
            }
        }
    }

    fn class_statement(&mut self, tok: Token) {
        let rest = self.lexer.rest_of_line();
        let mut words: Vec<&str> = self.slice(rest).split_whitespace().collect();
        let Some(class_name) = words.pop() else {
            return;
        };
        let Some(class) = self.class_id(class_name) else {
            return;
        };
        let targets = words.join(" ");
        for name in targets
            .split([',', ' '])
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            if let Some(&id) = self.cluster_by_key.get(name) {
                self.add_cluster_class(id, class);
            } else if !self.subgraph_keys.contains(name) {
                let id = self.declare_node(name, tok.span, None, None);
                self.add_node_class(id, class);
            }
        }
    }

    fn style_statement(&mut self, tok: Token) {
        let rest = self.lexer.rest_of_line();
        let (targets, props) = split_first_word(self.slice(rest));
        let mut style = StyleProps::default();
        self.apply_props(&mut style, props);
        for name in targets.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            if let Some(&id) = self.cluster_by_key.get(name) {
                if let Some(cluster) = self.graph.clusters.get_mut(id.0 as usize) {
                    merge_style(&mut cluster.style, &style);
                }
            } else if !self.subgraph_keys.contains(name) {
                let id = self.declare_node(name, tok.span, None, None);
                if let Some(node) = self.graph.nodes.get_mut(id.0 as usize) {
                    merge_style(&mut node.style, &style);
                }
            }
        }
    }

    fn link_style(&mut self) {
        let rest = self.lexer.rest_of_line();
        let (targets, props) = split_first_word(self.slice(rest));
        let mut style = StyleProps::default();
        self.apply_props(&mut style, props);
        let mut rule = LinkStyleRule {
            indices: Vec::new(),
            all: false,
            style,
        };
        for target in targets.split(',').map(str::trim) {
            if target == "default" {
                rule.all = true;
            } else if let Ok(index) = target.parse::<usize>() {
                rule.indices.push(index);
            }
        }
        self.link_styles.push(rule);
    }

    /// Parses `k:v,k:v` style properties into `style`, reporting bad colours.
    fn apply_props(&mut self, style: &mut StyleProps, text: &str) {
        for part in text.split(',') {
            let Some((key, value)) = part.split_once(':') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            match key {
                "fill" | "stroke" | "color" => match parse_color(value) {
                    Some(color) => match key {
                        "fill" => style.fill = Some(color),
                        "stroke" => style.stroke = Some(color),
                        _ => style.color = Some(color),
                    },
                    None => {
                        let at = self.span_of_slice(value);
                        self.error(at, format!("Invalid color '{value}'"));
                    }
                },
                "stroke-width" => {
                    let number = value.trim_end_matches("px").trim();
                    if let Ok(width) = number.parse::<f32>() {
                        if width.is_finite() {
                            style.stroke_width = Some(width);
                        }
                    }
                }
                "stroke-dasharray" => style.dashed = true,
                _ => {}
            }
        }
    }

    // ---- end of input ---------------------------------------------------

    fn finish(&mut self) {
        for fix in std::mem::take(&mut self.fixups) {
            let endpoint = match self.cluster_by_key.get(&fix.key) {
                Some(&id) => Endpoint::Cluster(id),
                None => {
                    let id = self.declare_node(&fix.key, fix.span, None, None);
                    Endpoint::Node(id)
                }
            };
            if let Some(edge) = self.graph.edges.get_mut(fix.edge) {
                if fix.to_side {
                    edge.to = endpoint;
                } else {
                    edge.from = endpoint;
                }
            }
        }
        for rule in std::mem::take(&mut self.link_styles) {
            for (index, edge) in self.graph.edges.iter_mut().enumerate() {
                if rule.all || rule.indices.contains(&index) {
                    merge_style(&mut edge.style, &rule.style);
                }
            }
        }
    }
}

// ---- free helpers -----------------------------------------------------------

pub(crate) fn parse_direction(word: &str) -> Option<Direction> {
    match word.trim() {
        "TB" | "TD" => Some(Direction::TB),
        "BT" => Some(Direction::BT),
        "LR" => Some(Direction::LR),
        "RL" => Some(Direction::RL),
        _ => None,
    }
}

/// Splits `text` into the first whitespace-delimited word and the trimmed rest.
pub(crate) fn split_first_word(text: &str) -> (&str, &str) {
    let text = text.trim();
    match text.find(char::is_whitespace) {
        Some(i) => (&text[..i], text[i..].trim_start()),
        None => (text, ""),
    }
}

/// Copies every property set in `from` onto `into`.
pub(crate) fn merge_style(into: &mut StyleProps, from: &StyleProps) {
    if from.fill.is_some() {
        into.fill = from.fill;
    }
    if from.stroke.is_some() {
        into.stroke = from.stroke;
    }
    if from.color.is_some() {
        into.color = from.color;
    }
    if from.stroke_width.is_some() {
        into.stroke_width = from.stroke_width;
    }
    into.dashed |= from.dashed;
}

/// Parses `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa` and a few CSS names.
pub(crate) fn parse_color(value: &str) -> Option<[u8; 4]> {
    let value = value.trim().to_ascii_lowercase();
    if let Some(hex) = value.strip_prefix('#') {
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let digit = |i: usize| u8::from_str_radix(hex.get(i..i + 1)?, 16).ok();
        let pair = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        return match hex.len() {
            3 => Some([digit(0)? * 17, digit(1)? * 17, digit(2)? * 17, 255]),
            4 => Some([
                digit(0)? * 17,
                digit(1)? * 17,
                digit(2)? * 17,
                digit(3)? * 17,
            ]),
            6 => Some([pair(0)?, pair(2)?, pair(4)?, 255]),
            8 => Some([pair(0)?, pair(2)?, pair(4)?, pair(6)?]),
            _ => None,
        };
    }
    let rgb = match value.as_str() {
        "white" => [255, 255, 255],
        "black" => [0, 0, 0],
        "red" => [255, 0, 0],
        "green" => [0, 128, 0],
        "blue" => [0, 0, 255],
        "gray" | "grey" => [128, 128, 128],
        "yellow" => [255, 255, 0],
        "orange" => [255, 165, 0],
        "purple" => [128, 0, 128],
        "pink" => [255, 192, 203],
        "none" | "transparent" => return Some([0, 0, 0, 0]),
        _ => return None,
    };
    Some([rgb[0], rgb[1], rgb[2], 255])
}

/// Chooses the final shape from the opener kind and the closer text.
fn resolve_shape(open: ShapeKind, closer: &str) -> ShapeKind {
    match open {
        ShapeKind::Parallelogram if closer.starts_with('\\') => ShapeKind::Trapezoid,
        ShapeKind::ParallelogramAlt if closer.starts_with('/') => ShapeKind::TrapezoidAlt,
        other => other,
    }
}

/// Maps an `@{ shape: name }` name to a shape; unknown names are `Rect`.
fn shape_from_name(name: &str) -> ShapeKind {
    match name.trim().to_ascii_lowercase().as_str() {
        "rounded" | "event" => ShapeKind::Rounded,
        "stadium" | "pill" | "terminal" => ShapeKind::Stadium,
        "subproc" | "subroutine" | "fr-rect" | "framed-rectangle" => ShapeKind::Subroutine,
        "cyl" | "cylinder" | "database" | "db" => ShapeKind::Cylinder,
        "circle" | "circ" => ShapeKind::Circle,
        "dbl-circ" | "double-circle" => ShapeKind::DoubleCircle,
        "diam" | "diamond" | "decision" | "question" => ShapeKind::Diamond,
        "hex" | "hexagon" | "prepare" => ShapeKind::Hexagon,
        "lean-r" => ShapeKind::Parallelogram,
        "lean-l" => ShapeKind::ParallelogramAlt,
        "trap-b" => ShapeKind::Trapezoid,
        "trap-t" => ShapeKind::TrapezoidAlt,
        "odd" => ShapeKind::Asymmetric,
        _ => ShapeKind::Rect,
    }
}

/// Splits `key: value, key: "value"` pairs, honouring quotes. Values lose their quotes.
fn split_pairs(text: &str) -> Vec<(&str, &str)> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quote: Option<char> = None;
    for (i, c) in text.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == ',' => {
                parts.push(&text[start..i]);
                start = i + 1;
            }
            None => {}
        }
    }
    parts.push(&text[start..]);
    parts
        .into_iter()
        .filter_map(|part| {
            let (key, value) = part.split_once(':')?;
            let value = value.trim();
            let unquoted = [('"', '"'), ('\'', '\'')]
                .iter()
                .find_map(|&(open, close)| value.strip_prefix(open)?.strip_suffix(close))
                .unwrap_or(value);
            Some((key.trim(), unquoted))
        })
        .collect()
}

/// Splits a subgraph header into `(key, title)`:
/// `id [Title]`, `id`, `"Title"` or `Some Title`.
fn subgraph_header(rest: &str) -> (String, String) {
    let rest = rest.trim();
    if rest.is_empty() {
        return (String::new(), String::new());
    }
    if rest.starts_with('"') {
        let title = clean_label(rest);
        return (title.clone(), title);
    }
    let id_end = rest
        .char_indices()
        .find(|&(_, c)| !(c.is_alphanumeric() || c == '_' || c == '-'))
        .map_or(rest.len(), |(i, _)| i);
    let id = &rest[..id_end];
    let tail = rest[id_end..].trim_start();
    if !id.is_empty() && tail.starts_with('[') {
        let inner = tail.strip_prefix('[').unwrap_or(tail);
        let inner = inner.strip_suffix(']').unwrap_or(inner);
        return (id.to_string(), clean_label(inner));
    }
    if !id.is_empty() && tail.is_empty() {
        return (id.to_string(), id.to_string());
    }
    let whole = clean_label(rest);
    (whole.clone(), whole)
}

/// Collects the keys of every `subgraph` header in `body`.
fn collect_subgraph_keys(body: &str) -> HashSet<String> {
    let mut keys = HashSet::new();
    for line in body.split(['\n', ';']) {
        let line = line.split("%%").next().unwrap_or("").trim_start();
        if let Some(rest) = line.strip_prefix("subgraph") {
            if rest.starts_with([' ', '\t']) {
                let (key, _) = subgraph_header(rest);
                if !key.is_empty() {
                    keys.insert(key);
                }
            }
        }
    }
    keys
}

/// Cleans label text: removes quotes and markdown markers, converts line-break
/// tags and `\n` to newlines, strips other HTML tags and decodes entities.
pub(crate) fn clean_label(raw: &str) -> String {
    let mut text = raw.trim();
    if let Some(inner) = text
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    {
        text = inner;
    }
    let mut markdown = false;
    if let Some(inner) = text
        .strip_prefix('`')
        .and_then(|rest| rest.strip_suffix('`'))
    {
        text = inner;
        markdown = true;
    }

    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let consumed = match c {
            '<' => clean_tag(rest, &mut out),
            '&' => clean_entity(rest, &mut out, false),
            '#' => clean_entity(rest, &mut out, true),
            '\\' if rest.get(1..).is_some_and(|r| r.starts_with('n')) => {
                out.push('\n');
                2
            }
            _ => {
                out.push(c);
                c.len_utf8()
            }
        };
        rest = rest.get(consumed..).unwrap_or("");
    }
    if markdown {
        out = out.replace("**", "").replace("__", "").replace('*', "");
    }
    out
}

/// Handles a `<` at the start of `rest`; returns the bytes consumed.
fn clean_tag(rest: &str, out: &mut String) -> usize {
    let Some(end) = rest.find('>') else {
        out.push('<');
        return 1;
    };
    let inner = rest.get(1..end).unwrap_or("");
    let compact: String = inner
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    if compact == "br" || compact == "br/" {
        out.push('\n');
        return end + 1;
    }
    let first = inner.chars().next();
    if first.is_some_and(|c| c.is_ascii_alphabetic() || c == '/' || c == '!') {
        return end + 1;
    }
    out.push('<');
    1
}

/// Handles `&name;`, `&#N;`, `&#xH;` and (with `hash_form`) `#N;` / `#name;`.
fn clean_entity(rest: &str, out: &mut String, hash_form: bool) -> usize {
    let fallback = |out: &mut String| {
        out.push(if hash_form { '#' } else { '&' });
        1
    };
    let Some(semi) = rest.find(';').filter(|&s| s <= 10) else {
        return fallback(out);
    };
    let name = rest.get(1..semi).unwrap_or("");
    let decoded = if hash_form {
        decode_hash_entity(name)
    } else {
        decode_amp_entity(name)
    };
    match decoded {
        Some(c) => {
            out.push(c);
            semi + 1
        }
        None => fallback(out),
    }
}

fn named_entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        _ => None,
    }
}

fn decode_amp_entity(name: &str) -> Option<char> {
    if let Some(number) = name.strip_prefix('#') {
        let code = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse::<u32>().ok()?,
        };
        return char::from_u32(code);
    }
    named_entity(name)
}

fn decode_hash_entity(name: &str) -> Option<char> {
    if !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()) {
        return char::from_u32(name.parse::<u32>().ok()?);
    }
    named_entity(name)
}
