//! Helpers shared by the line-oriented diagram parsers (class, ER).

use crate::graph::{Diagnostic, Edge, Endpoint, Node, ShapeKind, Span, StyleProps, ArrowKind, LineKind};

/// A source line and its byte offset.
#[derive(Clone, Copy)]
pub(crate) struct Line<'a> {
    pub text: &'a str,
    pub offset: usize,
}

impl Line<'_> {
    /// Span of `part`, which must be a sub-slice of this line.
    pub fn span_of(&self, part: &str) -> Span {
        let start = (part.as_ptr() as usize).saturating_sub(self.text.as_ptr() as usize);
        let start = start.min(self.text.len());
        let end = (start + part.len()).min(self.text.len());
        Span {
            start: (self.offset + start) as u32,
            end: (self.offset + end) as u32,
        }
    }
}

/// Lines of `src` starting at `start`.
pub(crate) fn lines(src: &str, start: usize) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    let mut offset = start;
    for raw in src.get(start..).unwrap_or("").split_inclusive('\n') {
        out.push(Line {
            text: raw.trim_end_matches(['\n', '\r']),
            offset,
        });
        offset += raw.len();
    }
    out
}

/// Text before a `%%` comment.
pub(crate) fn strip_comment(line: &str) -> &str {
    line.split("%%").next().unwrap_or("")
}

pub(crate) fn split_word(text: &str) -> (&str, &str) {
    let text = text.trim();
    match text.find(char::is_whitespace) {
        Some(i) => (&text[..i], text[i..].trim_start()),
        None => (text, ""),
    }
}

/// Records a diagnostic located at `span` on `line`.
pub(crate) fn diagnostic(src: &str, line: &Line, span: Span, message: &str) -> Diagnostic {
    let row = src.get(..line.offset).unwrap_or("").matches('\n').count() as u32 + 1;
    let column = src
        .get(line.offset..span.start as usize)
        .map_or(0, |s| s.chars().count()) as u32
        + 1;
    Diagnostic {
        line: row,
        column,
        span,
        message: message.to_string(),
    }
}

pub(crate) fn new_node(key: &str, label: String, shape: ShapeKind, span: Span) -> Node {
    Node {
        key: key.to_string(),
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn new_edge(
    from: Endpoint,
    to: Endpoint,
    label: Option<String>,
    line: LineKind,
    tail: ArrowKind,
    head: ArrowKind,
    span: Span,
) -> Edge {
    Edge {
        from,
        to,
        label,
        line,
        head,
        tail,
        min_len: 1,
        style: StyleProps::default(),
        span,
    }
}

/// Splits `text` into whitespace separated tokens; double-quoted runs stay in one token
/// with their quotes removed.
pub(crate) fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut had = false;
    for c in text.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                had = true;
            }
            c if c.is_whitespace() && !quoted => {
                if had || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    had = false;
                }
            }
            c => cur.push(c),
        }
    }
    if had || !cur.is_empty() {
        out.push(cur);
    }
    out
}
