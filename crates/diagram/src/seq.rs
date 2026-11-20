//! Sequence diagrams: parser and layout, lowered onto the shared `Graph`/`Layout`/`Scene`.
//!
//! Participants are nodes (a box at the top and one at the bottom), lifelines are dotted
//! edges between the two, messages are edges with fixed coordinates, activation bars are
//! narrow nodes, `loop`/`alt`/`opt`/`par`... frames are clusters and `else`/`and` dividers
//! are dotted edges across a frame. The layout is a single top-to-bottom pass.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::graph::{
    ArrowKind, Cluster, ClusterId, Diagnostic, Edge, Endpoint, Family, Graph, LineKind, Node,
    NodeId, Parsed, ShapeKind, Span, StyleProps,
};
use crate::layout::{EdgePath, Layout, LayoutError, check_cancel};
use crate::lexer::skip_preamble;
use crate::metrics::Metrics;
use crate::parser::clean_label;
use crate::shape::Rect;
use crate::text::{Measurer, TextBlock};

/// Statement budget; later lines are ignored.
const MAX_EVENTS: usize = 20_000;

/// One step of the diagram, in source order. Indices point into the `Graph`.
#[derive(Clone, Debug)]
pub enum Ev {
    Msg { edge: u32, from: u32, to: u32 },
    /// `side`: 0 left of `a`, 1 right of `a`, 2 over `a` (and `b`).
    Note { node: u32, side: u8, a: u32, b: u32 },
    Act { node: u32, p: u32 },
    Deact { node: u32, p: u32 },
    Block { cluster: u32 },
    Branch { edge: u32, cluster: u32 },
    End { cluster: u32 },
}

/// What layout needs beyond the graph.
#[derive(Clone, Debug, Default)]
pub struct Model {
    pub participants: u32,
    pub events: Vec<Ev>,
}

struct Part {
    key: String,
    label: String,
    actor: bool,
    span: Span,
}

enum Raw {
    Msg {
        from: usize,
        to: usize,
        label: String,
        line: LineKind,
        head: ArrowKind,
        tail: ArrowKind,
        act: bool,
        deact: bool,
        span: Span,
    },
    Note {
        side: u8,
        a: usize,
        b: usize,
        text: String,
        span: Span,
    },
    Act(usize),
    Deact(usize),
    Block {
        kw: String,
        label: String,
        span: Span,
    },
    Branch {
        label: String,
        span: Span,
    },
    End,
}

const OPS: [(&str, LineKind, ArrowKind, ArrowKind); 10] = [
    ("<<-->>", LineKind::Dotted, ArrowKind::Arrow, ArrowKind::Arrow),
    ("<<->>", LineKind::Solid, ArrowKind::Arrow, ArrowKind::Arrow),
    ("-->>", LineKind::Dotted, ArrowKind::Arrow, ArrowKind::None),
    ("->>", LineKind::Solid, ArrowKind::Arrow, ArrowKind::None),
    ("--x", LineKind::Dotted, ArrowKind::Cross, ArrowKind::None),
    ("-x", LineKind::Solid, ArrowKind::Cross, ArrowKind::None),
    ("--)", LineKind::Dotted, ArrowKind::Open, ArrowKind::None),
    ("-)", LineKind::Solid, ArrowKind::Open, ArrowKind::None),
    ("-->", LineKind::Dotted, ArrowKind::None, ArrowKind::None),
    ("->", LineKind::Solid, ArrowKind::None, ArrowKind::None),
];

fn find_op(text: &str) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for (i, (op, ..)) in OPS.iter().enumerate() {
        if let Some(at) = text.find(op)
            && best.is_none_or(|(b, _)| at < b)
        {
            best = Some((at, i));
        }
    }
    best
}

struct P<'a> {
    src: &'a str,
    parts: Vec<Part>,
    index: HashMap<String, usize>,
    raws: Vec<(Raw, Span)>,
    diagnostics: Vec<Diagnostic>,
    autonumber: bool,
}

fn span_in(src: &str, offset: usize, part: &str, line: &str) -> Span {
    let start = (part.as_ptr() as usize).saturating_sub(line.as_ptr() as usize);
    let start = start.min(line.len());
    let _ = src;
    Span {
        start: (offset + start) as u32,
        end: (offset + (start + part.len()).min(line.len())) as u32,
    }
}

impl<'a> P<'a> {
    fn part(&mut self, key: &str, span: Span) -> usize {
        if let Some(&i) = self.index.get(key) {
            return i;
        }
        let i = self.parts.len();
        self.parts.push(Part {
            key: key.to_string(),
            label: key.to_string(),
            actor: false,
            span,
        });
        self.index.insert(key.to_string(), i);
        i
    }

    fn error(&mut self, line_offset: usize, span: Span, message: &str) {
        let row = self.src[..line_offset].matches('\n').count() as u32 + 1;
        let column = self
            .src
            .get(line_offset..span.start as usize)
            .map_or(0, |s| s.chars().count()) as u32
            + 1;
        self.diagnostics.push(Diagnostic {
            line: row,
            column,
            span,
            message: message.to_string(),
        });
    }

    fn run(&mut self) {
        let start = skip_preamble(self.src);
        let mut offset = start;
        let mut header = true;
        let mut kinds: Vec<bool> = Vec::new(); // true = box (ignored grouping)
        let src = self.src;
        let mut pending_note: Option<(usize, u8, usize, usize, Vec<String>, Span)> = None;
        for raw in src.get(start..).unwrap_or("").split_inclusive('\n') {
            let line_offset = offset;
            offset += raw.len();
            let line = raw.trim_end_matches(['\n', '\r']);
            let text = line.split("%%").next().unwrap_or("").trim();
            if text.is_empty() {
                continue;
            }
            if self.raws.len() > MAX_EVENTS {
                break;
            }
            let _ = &mut pending_note;
            if header {
                header = false;
                if text.starts_with("sequenceDiagram") {
                    continue;
                }
            }
            let (word, rest) = split_word(text);
            let span = span_in(src, line_offset, text, line);
            match word {
                "participant" | "actor" => self.declare(word == "actor", rest, line, line_offset),
                "create" => {
                    let (kind, rest2) = split_word(rest);
                    if kind == "participant" || kind == "actor" {
                        self.declare(kind == "actor", rest2, line, line_offset);
                    }
                }
                "destroy" | "title" | "accTitle" | "accDescr" | "link" | "links"
                | "properties" | "details" | "theme" => {}
                "autonumber" => self.autonumber = !rest.starts_with("off"),
                "box" => kinds.push(true),
                "end" => {
                    if kinds.pop() == Some(false) {
                        self.raws.push((Raw::End, span));
                    }
                }
                "activate" => {
                    let p = self.part(rest.trim(), span);
                    self.raws.push((Raw::Act(p), span));
                }
                "deactivate" => {
                    let p = self.part(rest.trim(), span);
                    self.raws.push((Raw::Deact(p), span));
                }
                "loop" | "alt" | "opt" | "par" | "critical" | "break" | "rect" => {
                    kinds.push(false);
                    let label = if word == "rect" {
                        String::new()
                    } else {
                        clean_label(rest)
                    };
                    self.raws.push((
                        Raw::Block {
                            kw: word.to_string(),
                            label,
                            span,
                        },
                        span,
                    ));
                }
                "else" | "and" | "option" => {
                    let label = clean_label(rest);
                    self.raws.push((Raw::Branch { label, span }, span));
                }
                "Note" | "note" => self.note(rest, line, line_offset, span),
                _ => self.message(text, line, line_offset, span),
            }
        }
    }

    fn declare(&mut self, actor: bool, rest: &str, line: &str, line_offset: usize) {
        let (id, label) = match rest.split_once(" as ") {
            Some((id, label)) => (id.trim(), Some(clean_label(label))),
            None => (rest.trim().split('@').next().unwrap_or("").trim(), None),
        };
        if id.is_empty() {
            return;
        }
        let span = span_in(self.src, line_offset, id, line);
        let i = self.part(id, span);
        let p = &mut self.parts[i];
        p.actor |= actor;
        if let Some(l) = label {
            p.label = l;
        }
    }

    fn note(&mut self, rest: &str, line: &str, line_offset: usize, span: Span) {
        let (side_word, after) = split_word(rest);
        let side = match side_word.to_ascii_lowercase().as_str() {
            "left" => 0u8,
            "right" => 1,
            "over" => 2,
            _ => {
                self.error(line_offset, span, "Expected 'Note left of', 'right of' or 'over'");
                return;
            }
        };
        let after = after.trim_start();
        let after = after.strip_prefix("of ").unwrap_or(after);
        let Some((targets, text)) = after.split_once(':') else {
            self.error(line_offset, span, "A note needs ':' and text");
            return;
        };
        let mut ids = targets.split(',').map(str::trim).filter(|t| !t.is_empty());
        let Some(first) = ids.next() else {
            self.error(line_offset, span, "A note needs a participant");
            return;
        };
        let a = self.part(first, span_in(self.src, line_offset, first, line));
        let b = match ids.next() {
            Some(second) => self.part(second, span_in(self.src, line_offset, second, line)),
            None => a,
        };
        self.raws.push((
            Raw::Note {
                side,
                a,
                b,
                text: clean_label(text),
                span,
            },
            span,
        ));
    }

    fn message(&mut self, text: &str, line: &str, line_offset: usize, span: Span) {
        let Some((at, op)) = find_op(text) else {
            self.error(line_offset, span, "Unrecognised sequence statement");
            return;
        };
        let (name, _, _, _) = OPS[op];
        let (line_kind, head, tail) = (OPS[op].1, OPS[op].2, OPS[op].3);
        let from = text[..at].trim();
        let mut rest = text[at + name.len()..].trim_start();
        let (mut act, mut deact) = (false, false);
        if let Some(r) = rest.strip_prefix('+') {
            act = true;
            rest = r.trim_start();
        } else if let Some(r) = rest.strip_prefix('-') {
            deact = true;
            rest = r.trim_start();
        }
        let (to, label) = match rest.split_once(':') {
            Some((t, l)) => (t.trim(), clean_label(l)),
            None => (rest.trim(), String::new()),
        };
        if from.is_empty() || to.is_empty() {
            self.error(line_offset, span, "A message needs a participant on both sides");
            return;
        }
        let a = self.part(from, span_in(self.src, line_offset, from, line));
        let b = self.part(to, span_in(self.src, line_offset, to, line));
        self.raws.push((
            Raw::Msg {
                from: a,
                to: b,
                label,
                line: line_kind,
                head,
                tail,
                act,
                deact,
                span,
            },
            span,
        ));
    }
}

fn split_word(text: &str) -> (&str, &str) {
    let text = text.trim();
    match text.find(char::is_whitespace) {
        Some(i) => (&text[..i], text[i..].trim_start()),
        None => (text, ""),
    }
}

/// Parses a sequence diagram.
pub(crate) fn parse(source: &str) -> Parsed {
    let mut p = P {
        src: source,
        parts: Vec::new(),
        index: HashMap::new(),
        raws: Vec::new(),
        diagnostics: Vec::new(),
        autonumber: false,
    };
    p.run();
    let (graph, model) = lower(&p);
    let mut graph = graph;
    graph.seq = Some(Arc::new(model));
    Parsed {
        family: Family::Sequence,
        graph,
        diagnostics: p.diagnostics,
    }
}

fn blank_node(key: String, label: String, shape: ShapeKind, span: Span) -> Node {
    Node {
        key,
        label,
        shape,
        cluster: None,
        classes: Vec::new(),
        style: StyleProps::default(),
        span,
        label_span: span,
            sections: Vec::new(),
    }
}

fn blank_edge(from: Endpoint, to: Endpoint, line: LineKind, span: Span) -> Edge {
    Edge {
        from,
        to,
        label: None,
        line,
        head: ArrowKind::None,
        tail: ArrowKind::None,
        min_len: 1,
        style: StyleProps::default(),
        span,
    }
}

fn lower(p: &P) -> (Graph, Model) {
    let n = p.parts.len();
    let mut g = Graph::default();
    let mut model = Model {
        participants: n as u32,
        events: Vec::new(),
    };
    let shape = |a: bool| if a { ShapeKind::Rounded } else { ShapeKind::Rect };
    for part in &p.parts {
        g.nodes.push(blank_node(
            part.key.clone(),
            part.label.clone(),
            shape(part.actor),
            part.span,
        ));
    }
    for part in &p.parts {
        g.nodes.push(blank_node(
            format!("{} (end)", part.key),
            part.label.clone(),
            shape(part.actor),
            part.span,
        ));
    }
    for (i, part) in p.parts.iter().enumerate() {
        g.edges.push(blank_edge(
            Endpoint::Node(NodeId(i as u32)),
            Endpoint::Node(NodeId((n + i) as u32)),
            LineKind::Dotted,
            part.span,
        ));
    }
    let mut stacks: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut blocks: Vec<u32> = Vec::new();
    let mut number = 0u32;
    let mut acts = 0u32;
    let mut notes = 0u32;
    let mut new_act = |g: &mut Graph, p: usize, span: Span, stacks: &mut Vec<Vec<u32>>| -> u32 {
        acts += 1;
        let id = g.nodes.len() as u32;
        g.nodes.push(blank_node(
            format!("{}#activation{}", p_key(p, &g.nodes), acts),
            String::new(),
            ShapeKind::Rect,
            span,
        ));
        stacks[p].push(id);
        id
    };
    for (raw, _) in &p.raws {
        match raw {
            Raw::Msg {
                from,
                to,
                label,
                line,
                head,
                tail,
                act,
                deact,
                span,
            } => {
                let mut text = label.clone();
                if p.autonumber {
                    number += 1;
                    text = if text.is_empty() {
                        number.to_string()
                    } else {
                        format!("{number}. {text}")
                    };
                }
                let edge = g.edges.len() as u32;
                let mut e = blank_edge(
                    Endpoint::Node(NodeId(*from as u32)),
                    Endpoint::Node(NodeId(*to as u32)),
                    *line,
                    *span,
                );
                e.head = *head;
                e.tail = *tail;
                e.label = (!text.is_empty()).then_some(text);
                g.edges.push(e);
                model.events.push(Ev::Msg {
                    edge,
                    from: *from as u32,
                    to: *to as u32,
                });
                if *act {
                    let node = new_act(&mut g, *to, *span, &mut stacks);
                    model.events.push(Ev::Act {
                        node,
                        p: *to as u32,
                    });
                }
                if *deact && let Some(node) = stacks[*from].pop() {
                    model.events.push(Ev::Deact {
                        node,
                        p: *from as u32,
                    });
                }
            }
            Raw::Note {
                side,
                a,
                b,
                text,
                span,
            } => {
                notes += 1;
                let node = g.nodes.len() as u32;
                g.nodes.push(blank_node(
                    format!("[note]{notes}"),
                    text.clone(),
                    ShapeKind::Note,
                    *span,
                ));
                model.events.push(Ev::Note {
                    node,
                    side: *side,
                    a: *a as u32,
                    b: *b as u32,
                });
            }
            Raw::Act(pi) => {
                let node = new_act(&mut g, *pi, Span::default(), &mut stacks);
                model.events.push(Ev::Act {
                    node,
                    p: *pi as u32,
                });
            }
            Raw::Deact(pi) => {
                if let Some(node) = stacks[*pi].pop() {
                    model.events.push(Ev::Deact {
                        node,
                        p: *pi as u32,
                    });
                }
            }
            Raw::Block { kw, label, span } => {
                let id = g.clusters.len() as u32;
                let title = match (kw.as_str(), label.is_empty()) {
                    ("rect", _) => String::new(),
                    (_, true) => kw.clone(),
                    (_, false) => format!("{kw} [{label}]"),
                };
                g.clusters.push(Cluster {
                    key: format!("{kw}{id}"),
                    title,
                    parent: blocks.last().map(|&c| ClusterId(c)),
                    direction: None,
                    classes: Vec::new(),
                    style: StyleProps::default(),
                    span: *span,
                });
                blocks.push(id);
                model.events.push(Ev::Block { cluster: id });
            }
            Raw::Branch { label, span } => {
                if let Some(&c) = blocks.last() {
                    let edge = g.edges.len() as u32;
                    let mut e = blank_edge(
                        Endpoint::Cluster(ClusterId(c)),
                        Endpoint::Cluster(ClusterId(c)),
                        LineKind::Dotted,
                        *span,
                    );
                    e.label = (!label.is_empty()).then(|| format!("[{label}]"));
                    g.edges.push(e);
                    model.events.push(Ev::Branch { edge, cluster: c });
                }
            }
            Raw::End => {
                if let Some(c) = blocks.pop() {
                    model.events.push(Ev::End { cluster: c });
                }
            }
        }
    }
    for (pi, stack) in stacks.iter_mut().enumerate() {
        while let Some(node) = stack.pop() {
            model.events.push(Ev::Deact {
                node,
                p: pi as u32,
            });
        }
    }
    while let Some(c) = blocks.pop() {
        model.events.push(Ev::End { cluster: c });
    }
    (g, model)
}

fn p_key(p: usize, nodes: &[Node]) -> String {
    nodes.get(p).map(|n| n.key.clone()).unwrap_or_default()
}

// ---- layout -------------------------------------------------------------------

/// `[p0, p1, ...]` as a chain of straight cubic segments.
fn chain(pts: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut out = vec![pts[0]];
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        out.push([a[0] + (b[0] - a[0]) / 3.0, a[1] + (b[1] - a[1]) / 3.0]);
        out.push([a[0] + (b[0] - a[0]) * 2.0 / 3.0, a[1] + (b[1] - a[1]) * 2.0 / 3.0]);
        out.push(b);
    }
    out
}

fn union(a: Option<Rect>, b: Rect) -> Rect {
    a.map_or(b, |a| a.union(b))
}

/// Lays out a sequence diagram.
pub(crate) fn layout(
    g: &Graph,
    model: &Model,
    measurer: &Measurer,
    m: &Metrics,
    cancel: &AtomicBool,
) -> Result<Layout, LayoutError> {
    let n = model.participants as usize;
    let font = m.edge_label_font;
    let node_text: Vec<TextBlock> = g
        .nodes
        .iter()
        .map(|nd| {
            let width = if nd.shape == ShapeKind::Note { 220.0 } else { 200.0 };
            measurer.wrap(&nd.label, m.font_size, m.line_height, width)
        })
        .collect();
    let edge_text: Vec<Option<TextBlock>> = g
        .edges
        .iter()
        .map(|e| {
            e.label
                .as_ref()
                .filter(|l| !l.is_empty())
                .map(|l| measurer.wrap(l, font, font + 5.0, 360.0))
        })
        .collect();
    check_cancel(cancel)?;

    let pw: Vec<f32> = (0..n)
        .map(|i| (node_text[i].width + 2.0 * m.pad_x).max(84.0))
        .collect();
    let ph = (0..n)
        .map(|i| node_text[i].height + 2.0 * m.pad_y)
        .fold(40.0f32, f32::max);
    let note_w = |node: u32| node_text[node as usize].width + 24.0;
    let label_w = |edge: u32| edge_text[edge as usize].as_ref().map_or(0.0, |t| t.width + 12.0);

    // Horizontal spacing: centre distances between neighbouring lifelines.
    let mut dist: Vec<f32> = (0..n.saturating_sub(1))
        .map(|i| (pw[i] + pw[i + 1]) / 2.0 + 24.0)
        .collect();
    let mut x0 = 24.0 + pw.first().copied().unwrap_or(0.0) / 2.0;
    let mut spans: Vec<(usize, usize, f32)> = Vec::new();
    for ev in &model.events {
        match *ev {
            Ev::Msg { edge, from, to } => {
                let (a, b) = (from.min(to) as usize, from.max(to) as usize);
                if a == b {
                    if a + 1 < n {
                        spans.push((a, a + 1, label_w(edge) + 56.0));
                    }
                } else {
                    spans.push((a, b, label_w(edge) + 24.0));
                }
            }
            Ev::Note { node, side, a, b } => {
                let w = note_w(node);
                let (a, b) = (a.min(b) as usize, a.max(b) as usize);
                match side {
                    0 => {
                        if a > 0 {
                            spans.push((a - 1, a, w + 20.0));
                        } else {
                            x0 = x0.max(w + 24.0);
                        }
                    }
                    1 => {
                        if a + 1 < n {
                            spans.push((a, a + 1, w + 20.0));
                        }
                    }
                    _ => {
                        if a == b {
                            if a > 0 {
                                spans.push((a - 1, a, w / 2.0 + 14.0));
                            } else {
                                x0 = x0.max(w / 2.0 + 24.0);
                            }
                            if a + 1 < n {
                                spans.push((a, a + 1, w / 2.0 + 14.0));
                            }
                        } else {
                            spans.push((a, b, w + 20.0));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    spans.sort_by_key(|&(a, b, _)| b - a);
    for (a, b, need) in spans {
        let have: f32 = dist[a..b].iter().sum();
        if have < need {
            dist[b - 1] += need - have;
        }
    }
    let mut xs = Vec::with_capacity(n);
    let mut x = x0;
    for i in 0..n {
        xs.push(x);
        if i + 1 < n {
            x += dist[i];
        }
    }
    check_cancel(cancel)?;

    // Vertical pass.
    let nn = g.nodes.len();
    let mut nodes = vec![Rect::default(); nn];
    let mut edges: Vec<EdgePath> = (0..g.edges.len())
        .map(|_| EdgePath {
            points: Vec::new(),
            label: None,
        })
        .collect();
    let mut clusters = vec![Rect::default(); g.clusters.len()];
    let top = 16.0;
    let mut y = top + ph + 26.0;
    let mut depth = vec![0usize; n];
    let mut act_start: HashMap<u32, f32> = HashMap::new();
    let mut act_level: HashMap<u32, usize> = HashMap::new();
    let mut open: Vec<u32> = Vec::new();
    let mut frame_top = vec![0.0f32; g.clusters.len()];
    let mut frame_bottom = vec![0.0f32; g.clusters.len()];
    let mut frame_x: Vec<Option<(f32, f32)>> = vec![None; g.clusters.len()];
    let mut branches: Vec<(u32, u32, f32)> = Vec::new();
    let mut last_ly: Option<f32> = None;
    let mut bars: Vec<(u32, u32, f32, f32)> = Vec::new(); // node, p, y0, y1
    let widen = |open: &[u32], frame_x: &mut Vec<Option<(f32, f32)>>, lo: f32, hi: f32| {
        for &c in open {
            let e = &mut frame_x[c as usize];
            *e = Some(e.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
        }
    };
    for (k, ev) in model.events.iter().enumerate() {
        if k % 256 == 0 {
            check_cancel(cancel)?;
        }
        if !matches!(ev, Ev::Act { .. } | Ev::Deact { .. }) {
            last_ly = None;
        }
        match *ev {
            Ev::Msg { edge, from, to } => {
                let (f, t) = (from as usize, to as usize);
                let tb = edge_text[edge as usize].as_ref();
                let lh = tb.map_or(0.0, |t| t.height + 10.0);
                let rightward = xs[t] >= xs[f];
                let right_edge = |p: usize, d: usize| xs[p] + if d > 0 { 4.0 * d as f32 } else { 0.0 };
                let left_edge = |p: usize, d: usize| xs[p] - if d > 0 { 4.0 } else { 0.0 };
                if f == t {
                    let ly = y + 10.0;
                    let xr = right_edge(f, depth[f]);
                    let pts = [
                        [xr, ly],
                        [xr + 28.0, ly],
                        [xr + 28.0, ly + 22.0],
                        [xr, ly + 22.0],
                    ];
                    edges[edge as usize].points = chain(&pts);
                    let mut hi = xr + 28.0;
                    if let Some(tb) = tb {
                        let r = Rect {
                            x: xr + 36.0,
                            y: ly + 11.0 - (tb.height + 6.0) / 2.0,
                            w: tb.width + 12.0,
                            h: tb.height + 6.0,
                        };
                        hi = r.right();
                        edges[edge as usize].label = Some(r);
                        y = y.max(r.bottom() - 10.0);
                    }
                    widen(&open, &mut frame_x, xs[f], hi);
                    y = y.max(ly + 22.0) + 14.0;
                    last_ly = Some(ly);
                } else {
                    let d_to = depth[t]
                        + usize::from(matches!(model.events.get(k + 1), Some(Ev::Act { p, .. }) if *p == to));
                    let (sx, ex) = if rightward {
                        (right_edge(f, depth[f]), left_edge(t, d_to))
                    } else {
                        (left_edge(f, depth[f]), right_edge(t, d_to))
                    };
                    let ly = y + lh + 12.0;
                    edges[edge as usize].points = chain(&[[sx, ly], [ex, ly]]);
                    if let Some(tb) = tb {
                        let w = tb.width + 12.0;
                        let h = tb.height + 6.0;
                        edges[edge as usize].label = Some(Rect {
                            x: (sx + ex) / 2.0 - w / 2.0,
                            y: ly - 4.0 - h,
                            w,
                            h,
                        });
                    }
                    widen(&open, &mut frame_x, sx.min(ex), sx.max(ex));
                    y = ly + 12.0;
                    last_ly = Some(ly);
                }
            }
            Ev::Note { node, side, a, b } => {
                let tb = &node_text[node as usize];
                let w = tb.width + 24.0;
                let h = tb.height + 14.0;
                let (a, b) = (a.min(b) as usize, a.max(b) as usize);
                let x = match side {
                    0 => xs[a] - 10.0 - w,
                    1 => xs[a] + 10.0,
                    _ if a == b => xs[a] - w / 2.0,
                    _ => (xs[a] + xs[b]) / 2.0 - w.max(xs[b] - xs[a] + 20.0) / 2.0,
                };
                let w = if side == 2 && a != b {
                    w.max(xs[b] - xs[a] + 20.0)
                } else {
                    w
                };
                nodes[node as usize] = Rect { x, y, w, h };
                widen(&open, &mut frame_x, x, x + w);
                y += h + 12.0;
            }
            Ev::Act { node, p } => {
                let p = p as usize;
                act_start.insert(node, last_ly.unwrap_or(y));
                act_level.insert(node, depth[p]);
                depth[p] += 1;
            }
            Ev::Deact { node, p } => {
                let p = p as usize;
                depth[p] = depth[p].saturating_sub(1);
                let y0 = act_start.get(&node).copied().unwrap_or(y);
                let y1 = last_ly.unwrap_or(y);
                bars.push((node, p as u32, y0, y1.max(y0 + 10.0)));
            }
            Ev::Block { cluster } => {
                frame_top[cluster as usize] = y;
                open.push(cluster);
                y += 30.0;
            }
            Ev::Branch { edge, cluster } => {
                y += 6.0;
                let tb = edge_text[edge as usize].as_ref();
                branches.push((edge, cluster, y));
                y += tb.map_or(0.0, |t| t.height + 12.0) + 10.0;
            }
            Ev::End { cluster } => {
                frame_bottom[cluster as usize] = y + 6.0;
                open.retain(|&c| c != cluster);
                y += 20.0;
            }
        }
    }
    // Any frame still open (cannot happen after lowering) ends here.
    for &c in &open {
        frame_bottom[c as usize] = y;
    }
    let y_end = y + 6.0;
    let y_bottom = y_end + 20.0;

    // Frames: nesting level below decides padding so nested frames stay visible.
    let k = g.clusters.len();
    let mut below = vec![0usize; k];
    for c in (0..k).rev() {
        if let Some(p) = g.clusters[c].parent {
            let p = p.0 as usize;
            below[p] = below[p].max(below[c] + 1);
        }
    }
    let full = (xs.first().copied().unwrap_or(0.0), xs.last().copied().unwrap_or(0.0));
    for c in 0..k {
        let pad = 14.0 + 8.0 * below[c] as f32;
        let (lo, hi) = frame_x[c].unwrap_or(full);
        let (lo, mut hi) = (lo - pad, hi + pad);
        let title_w = measurer.width(&g.clusters[c].title, 13.0) * 1.06 + 28.0;
        hi = hi.max(lo + title_w);
        clusters[c] = Rect {
            x: lo,
            y: frame_top[c],
            w: hi - lo,
            h: (frame_bottom[c] - frame_top[c]).max(20.0),
        };
    }
    for &(edge, cluster, sy) in &branches {
        let r = clusters[cluster as usize];
        edges[edge as usize].points = chain(&[[r.x, sy], [r.right(), sy]]);
        if let Some(tb) = &edge_text[edge as usize] {
            edges[edge as usize].label = Some(Rect {
                x: r.x + 10.0,
                y: sy + 4.0,
                w: tb.width + 12.0,
                h: tb.height + 6.0,
            });
        }
    }
    for &(node, p, y0, y1) in &bars {
        let level = act_level.get(&node).copied().unwrap_or(0) as f32;
        nodes[node as usize] = Rect {
            x: xs[p as usize] - 4.0 + 4.0 * level,
            y: y0,
            w: 8.0,
            h: y1 - y0,
        };
    }
    for i in 0..n {
        nodes[i] = Rect {
            x: xs[i] - pw[i] / 2.0,
            y: top,
            w: pw[i],
            h: ph,
        };
        nodes[n + i] = Rect {
            x: xs[i] - pw[i] / 2.0,
            y: y_bottom,
            w: pw[i],
            h: ph,
        };
        edges[i].points = chain(&[[xs[i], top + ph], [xs[i], y_bottom]]);
    }

    // Bounds and shift so everything sits at positive coordinates.
    let mut bb: Option<Rect> = None;
    for r in nodes.iter().chain(clusters.iter()) {
        if r.w > 0.0 || r.h > 0.0 {
            bb = Some(union(bb, *r));
        }
    }
    for e in &edges {
        if let Some(l) = e.label {
            bb = Some(union(bb, l));
        }
    }
    let bb = bb.unwrap_or_default();
    let (dx, dy) = (24.0 - bb.x, 16.0 - bb.y.min(top));
    let shift = |r: &mut Rect| {
        r.x += dx;
        r.y += dy;
    };
    nodes.iter_mut().for_each(shift);
    clusters.iter_mut().for_each(shift);
    for e in &mut edges {
        if let Some(l) = &mut e.label {
            shift(l);
        }
        for p in &mut e.points {
            p[0] += dx;
            p[1] += dy;
        }
    }
    Ok(Layout {
        nodes,
        node_text,
        records: vec![None; nn],
        clusters,
        edges,
        edge_text,
        bounds: Rect {
            x: 0.0,
            y: 0.0,
            w: bb.w + 48.0,
            h: bb.h + 32.0 - (top - bb.y.min(top)).max(0.0) * 0.0,
        },
        decor: Vec::new(),
        slots: (0..nn).map(|i| (0, i as u32)).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_messages_notes_blocks_and_activation() {
        let r = parse(
            "sequenceDiagram\nautonumber\nparticipant A as Alice\nactor B\nA->>+B: hi\nB-->>-A: ok\nA-)B: async\nNote over A,B: both\nloop every sec\n  A->>A: self\nelse other\n  B-xA: no\nend\n",
        );
        assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
        let g = &r.graph;
        let m = g.seq.as_ref().unwrap();
        assert_eq!(m.participants, 2);
        assert_eq!(g.nodes[0].label, "Alice");
        assert_eq!(g.edges[2].label.as_deref(), Some("1. hi"));
        assert_eq!(g.edges[2].head, ArrowKind::Arrow);
        assert_eq!(g.edges[3].line, LineKind::Dotted);
        assert_eq!(g.edges[4].head, ArrowKind::Open);
        assert_eq!(g.clusters.len(), 1);
        assert!(m.events.iter().any(|e| matches!(e, Ev::Branch { .. })));
        let acts = m.events.iter().filter(|e| matches!(e, Ev::Act { .. })).count();
        let deacts = m.events.iter().filter(|e| matches!(e, Ev::Deact { .. })).count();
        assert_eq!((acts, deacts), (1, 1));
    }

    #[test]
    fn garbage_is_survivable() {
        for s in ["sequenceDiagram", "sequenceDiagram\nend\nend\n", "sequenceDiagram\n->>\nA->>\nNote x\nloop\n"] {
            let _ = parse(s);
        }
    }
}
