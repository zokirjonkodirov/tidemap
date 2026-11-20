//! Byte lexer for flowchart source. Never panics; spans are UTF-8 byte offsets.

use crate::graph::{ArrowKind, LineKind, ShapeKind, Span};

/// Longest inline link label (`A -- text --> B`) we search for, in bytes.
const MAX_INLINE_LABEL: usize = 4096;

/// Builds a span from byte offsets, saturating at `u32::MAX`.
pub(crate) fn span(start: usize, end: usize) -> Span {
    Span {
        start: u32::try_from(start).unwrap_or(u32::MAX),
        end: u32::try_from(end).unwrap_or(u32::MAX),
    }
}

/// Token kinds.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tok {
    Ident,
    Text,
    /// Shape opener; follow with `Lexer::shape_text`.
    Open(ShapeKind),
    /// Shape closer; only produced after `shape_text` reported a closed label.
    Close,
    Link,
    Pipe,
    Amp,
    Colon3,
    Semi,
    Newline,
    At,
    LBrace,
    RBrace,
    Eof,
}

/// A token and its source span.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Token {
    pub kind: Tok,
    pub span: Span,
}

/// Decoded link token.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LinkInfo {
    pub line: LineKind,
    pub head: ArrowKind,
    pub tail: ArrowKind,
    /// Extra length beyond the minimum link (`-->` is 0, `--->` is 1).
    pub len: u8,
    /// Text of an inline label (`A -- text --> B`), trimmed.
    pub inline_label: Option<Span>,
}

/// Flowchart lexer.
#[derive(Clone, Debug)]
pub struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    after_at: bool,
    pending_close: Option<ShapeKind>,
}

/// Returns the offset of the first meaningful content: skips a leading
/// `---` frontmatter block, blank lines, `%%` comments and `%%{...}%%` directives.
pub(crate) fn skip_preamble(src: &str) -> usize {
    let mut pos = if src.starts_with('\u{feff}') { 3 } else { 0 };
    let mut first = true;
    loop {
        let rest = src.get(pos..).unwrap_or("");
        let trimmed = rest.trim_start();
        pos += rest.len() - trimmed.len();
        if first && trimmed.starts_with("---") {
            if let Some(after) = frontmatter_end(trimmed) {
                pos += after;
                first = false;
                continue;
            }
        }
        first = false;
        if trimmed.starts_with("%%") {
            pos += comment_len(trimmed);
            continue;
        }
        return pos;
    }
}

/// Length of a frontmatter block at the start of `s`, if it is closed.
fn frontmatter_end(s: &str) -> Option<usize> {
    let mut offset = 0;
    for (index, line) in s.split_inclusive('\n').enumerate() {
        let trimmed = line.trim();
        if index == 0 {
            if trimmed != "---" {
                return None;
            }
        } else if trimmed == "---" {
            return Some(offset + line.len());
        }
        offset += line.len();
    }
    None
}

/// Length of the comment or directive at the start of `s` (which begins with `%%`).
fn comment_len(s: &str) -> usize {
    if s.starts_with("%%{") {
        if let Some(i) = s.find("}%%") {
            return i + 3;
        }
    }
    s.find('\n').unwrap_or(s.len())
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// True when the byte at `i` could continue an identifier (non-ASCII counts).
fn ident_byte_at(b: &[u8], i: usize) -> bool {
    b.get(i)
        .is_some_and(|&x| x.is_ascii_alphanumeric() || x == b'_' || x >= 0x80)
}

fn to_u8(n: usize) -> u8 {
    u8::try_from(n).unwrap_or(u8::MAX)
}

/// Closing delimiters accepted for a shape opener.
fn closers(open: ShapeKind) -> &'static [&'static str] {
    match open {
        ShapeKind::Rounded => &[")"],
        ShapeKind::Circle => &["))"],
        ShapeKind::DoubleCircle => &[")))"],
        ShapeKind::Stadium => &["])"],
        ShapeKind::Subroutine => &["]]"],
        ShapeKind::Cylinder => &[")]"],
        ShapeKind::Rect | ShapeKind::Asymmetric => &["]"],
        ShapeKind::StateStart
        | ShapeKind::StateEnd
        | ShapeKind::Bar
        | ShapeKind::Note
        | ShapeKind::Record
        | ShapeKind::Choice => &[],
        ShapeKind::Diamond => &["}"],
        ShapeKind::Hexagon => &["}}"],
        ShapeKind::Parallelogram | ShapeKind::Trapezoid => &["/]", "\\]"],
        ShapeKind::ParallelogramAlt | ShapeKind::TrapezoidAlt => &["\\]", "/]"],
    }
}

/// Trims spaces, tabs and CRs from both ends of `b[start..end]`.
fn trimmed(b: &[u8], mut start: usize, mut end: usize) -> Span {
    let is_blank = |x: u8| matches!(x, b' ' | b'\t' | b'\r');
    while start < end && b.get(start).copied().is_some_and(is_blank) {
        start += 1;
    }
    while end > start && b.get(end - 1).copied().is_some_and(is_blank) {
        end -= 1;
    }
    span(start, end)
}

/// Parses an arrow head at `j`; returns the head and the end offset.
fn scan_head(b: &[u8], j: usize) -> (ArrowKind, usize) {
    match b.get(j) {
        Some(b'>') => (ArrowKind::Arrow, j + 1),
        Some(b'o') if !ident_byte_at(b, j + 1) => (ArrowKind::Circle, j + 1),
        Some(b'x') if !ident_byte_at(b, j + 1) => (ArrowKind::Cross, j + 1),
        _ => (ArrowKind::None, j),
    }
}

fn tail_ok(tail: ArrowKind, head: ArrowKind) -> bool {
    match tail {
        ArrowKind::Circle | ArrowKind::Cross => {
            matches!(head, ArrowKind::Circle | ArrowKind::Cross)
        }
        _ => true,
    }
}

/// Searches for the closing run of an inline `-- text -->` or `== text ==>` link.
fn scan_inline(b: &[u8], label_start: usize, line: LineKind, c: u8) -> Option<(usize, LinkInfo)> {
    let limit = label_start.saturating_add(MAX_INLINE_LABEL).min(b.len());
    let mut k = label_start;
    while k < limit {
        let ch = b.get(k).copied()?;
        if ch == b'\n' {
            return None;
        }
        if ch != c {
            k += 1;
            continue;
        }
        let mut e = k;
        while b.get(e) == Some(&c) {
            e += 1;
        }
        let run = e - k;
        let (head, end) = scan_head(b, e);
        let min = if head == ArrowKind::None { 3 } else { 2 };
        if run >= min {
            let info = LinkInfo {
                line,
                head,
                tail: ArrowKind::None,
                len: to_u8(run - min),
                inline_label: Some(trimmed(b, label_start, k)),
            };
            return Some((end, info));
        }
        k = e;
    }
    None
}

/// Searches for the closing `.->` or `.-` of an inline dotted link `-. text .-> B`.
fn scan_inline_dotted(b: &[u8], label_start: usize) -> Option<(usize, LinkInfo)> {
    let limit = label_start.saturating_add(MAX_INLINE_LABEL).min(b.len());
    let mut k = label_start;
    while k < limit {
        let ch = b.get(k).copied()?;
        if ch == b'\n' {
            return None;
        }
        if ch != b'.' {
            k += 1;
            continue;
        }
        let mut e = k;
        while b.get(e) == Some(&b'.') {
            e += 1;
        }
        if b.get(e) == Some(&b'-') {
            let (head, end) = scan_head(b, e + 1);
            let info = LinkInfo {
                line: LineKind::Dotted,
                head,
                tail: ArrowKind::None,
                len: to_u8(e - k - 1),
                inline_label: Some(trimmed(b, label_start, k)),
            };
            return Some((end, info));
        }
        k = e;
    }
    None
}

/// Scans a link token starting at `start`; returns its end offset and decoded info.
fn scan_link(b: &[u8], start: usize) -> Option<(usize, LinkInfo)> {
    let at = |i: usize| b.get(i).copied();
    let mut i = start;
    let mut tail = ArrowKind::None;
    match at(i)? {
        b'<' => tail = ArrowKind::Arrow,
        b'o' => tail = ArrowKind::Circle,
        b'x' => tail = ArrowKind::Cross,
        _ => {}
    }
    if tail != ArrowKind::None {
        i += 1;
    }
    let c = at(i)?;
    if !matches!(c, b'-' | b'=' | b'~') {
        return None;
    }

    if c == b'-' && at(i + 1) == Some(b'.') {
        let mut j = i + 1;
        while at(j) == Some(b'.') {
            j += 1;
        }
        let dots = j - (i + 1);
        if at(j) == Some(b'-') {
            let (head, end) = scan_head(b, j + 1);
            if !tail_ok(tail, head) {
                return None;
            }
            let info = LinkInfo {
                line: LineKind::Dotted,
                head,
                tail,
                len: to_u8(dots - 1),
                inline_label: None,
            };
            return Some((end, info));
        }
        if tail != ArrowKind::None {
            return None;
        }
        return scan_inline_dotted(b, j);
    }

    let mut j = i;
    while at(j) == Some(c) {
        j += 1;
    }
    let run = j - i;
    let line = match c {
        b'-' => LineKind::Solid,
        b'=' => LineKind::Thick,
        _ => LineKind::Invisible,
    };
    let (head, end) = if c == b'~' {
        (ArrowKind::None, j)
    } else {
        scan_head(b, j)
    };
    if !tail_ok(tail, head) {
        return None;
    }
    let min = if head == ArrowKind::None { 3 } else { 2 };
    if run >= min {
        let info = LinkInfo {
            line,
            head,
            tail,
            len: to_u8(run - min),
            inline_label: None,
        };
        return Some((end, info));
    }
    if run == 2 && head == ArrowKind::None && tail == ArrowKind::None && c != b'~' {
        return scan_inline(b, j, line, c);
    }
    None
}

impl<'a> Lexer<'a> {
    /// Creates a lexer at the start of `src`.
    pub fn new(src: &'a str) -> Self {
        Self {
            src,
            pos: 0,
            after_at: false,
            pending_close: None,
        }
    }

    /// Current byte offset.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Moves to `pos` (clamped to the source length and rounded up to a char boundary).
    pub fn seek(&mut self, pos: usize) {
        let mut pos = pos.min(self.src.len());
        while !self.src.is_char_boundary(pos) {
            pos += 1;
        }
        self.pos = pos;
        self.after_at = false;
        self.pending_close = None;
    }

    /// Unconsumed source text.
    pub fn remaining(&self) -> &'a str {
        self.src.get(self.pos..).unwrap_or("")
    }

    fn skip_trivia(&mut self) {
        loop {
            let rest = self.remaining();
            let Some(c) = rest.chars().next() else {
                return;
            };
            if c == '\n' {
                return;
            }
            if c.is_whitespace() || c == '\u{feff}' {
                self.pos += c.len_utf8();
            } else if rest.starts_with("%%") {
                self.pos += comment_len(rest);
            } else {
                return;
            }
        }
    }

    /// Returns the next token. Unknown characters come back as `Tok::Text` of one char.
    pub fn next(&mut self) -> Token {
        if let Some(open) = self.pending_close.take() {
            let rest = self.remaining();
            for closer in closers(open) {
                if rest.starts_with(closer) {
                    let start = self.pos;
                    self.pos += closer.len();
                    return Token {
                        kind: Tok::Close,
                        span: span(start, self.pos),
                    };
                }
            }
        }
        self.skip_trivia();
        let after_at = std::mem::take(&mut self.after_at);
        let start = self.pos;
        let rest = self.remaining();
        let Some(c) = rest.chars().next() else {
            return Token {
                kind: Tok::Eof,
                span: span(start, start),
            };
        };

        let simple = |lexer: &mut Self, kind: Tok, len: usize| {
            lexer.pos += len;
            Token {
                kind,
                span: span(start, start + len),
            }
        };

        match c {
            '\n' => return simple(self, Tok::Newline, 1),
            ';' => return simple(self, Tok::Semi, 1),
            '&' => return simple(self, Tok::Amp, 1),
            '|' => return simple(self, Tok::Pipe, 1),
            '}' => return simple(self, Tok::RBrace, 1),
            '@' => {
                self.after_at = true;
                return simple(self, Tok::At, 1);
            }
            ':' if rest.starts_with(":::") => return simple(self, Tok::Colon3, 3),
            '{' if after_at => return simple(self, Tok::LBrace, 1),
            '{' => {
                return if rest.starts_with("{{") {
                    simple(self, Tok::Open(ShapeKind::Hexagon), 2)
                } else {
                    simple(self, Tok::Open(ShapeKind::Diamond), 1)
                };
            }
            '(' => {
                return if rest.starts_with("(((") {
                    simple(self, Tok::Open(ShapeKind::DoubleCircle), 3)
                } else if rest.starts_with("((") {
                    simple(self, Tok::Open(ShapeKind::Circle), 2)
                } else if rest.starts_with("([") {
                    simple(self, Tok::Open(ShapeKind::Stadium), 2)
                } else {
                    simple(self, Tok::Open(ShapeKind::Rounded), 1)
                };
            }
            '[' => {
                let (kind, len) = if rest.starts_with("[[") {
                    (ShapeKind::Subroutine, 2)
                } else if rest.starts_with("[(") {
                    (ShapeKind::Cylinder, 2)
                } else if rest.starts_with("[/") {
                    (ShapeKind::Parallelogram, 2)
                } else if rest.starts_with("[\\") {
                    (ShapeKind::ParallelogramAlt, 2)
                } else {
                    (ShapeKind::Rect, 1)
                };
                return simple(self, Tok::Open(kind), len);
            }
            '>' => return simple(self, Tok::Open(ShapeKind::Asymmetric), 1),
            _ => {}
        }

        if matches!(c, '-' | '=' | '~' | '<' | 'o' | 'x') {
            if let Some((end, _)) = scan_link(self.src.as_bytes(), start) {
                self.pos = end;
                return Token {
                    kind: Tok::Link,
                    span: span(start, end),
                };
            }
        }

        if is_ident_char(c) {
            return self.ident(start);
        }
        simple(self, Tok::Text, c.len_utf8())
    }

    /// Lexes an identifier. A `-` belongs to it only when an identifier char follows.
    fn ident(&mut self, start: usize) -> Token {
        let rest = self.remaining();
        let mut end = start;
        let mut chars = rest.char_indices().peekable();
        while let Some((i, ch)) = chars.next() {
            if is_ident_char(ch) {
                end = start + i + ch.len_utf8();
            } else if ch == '-' && chars.peek().is_some_and(|&(_, next)| is_ident_char(next)) {
                end = start + i + 1;
            } else {
                break;
            }
        }
        self.pos = end;
        Token {
            kind: Tok::Ident,
            span: span(start, end),
        }
    }

    /// Decodes a `Tok::Link` token.
    pub fn link_info(&self, t: &Token) -> LinkInfo {
        scan_link(self.src.as_bytes(), t.span.start as usize)
            .map(|(_, info)| info)
            .unwrap_or(LinkInfo {
                line: LineKind::Solid,
                head: ArrowKind::Arrow,
                tail: ArrowKind::None,
                len: 0,
                inline_label: None,
            })
    }

    /// Reads a node label after an opener, up to (not including) the matching closer.
    /// Returns the trimmed label span (quotes excluded) and whether a closer was found.
    /// When closed, the next `next()` call returns `Tok::Close`.
    pub fn shape_text(&mut self, open: ShapeKind) -> (Span, bool) {
        let closers = closers(open);
        let b = self.src.as_bytes();
        let starts_closer = |i: usize| {
            closers.iter().any(|c| {
                b.get(i..)
                    .is_some_and(|rest| rest.starts_with(c.as_bytes()))
            })
        };

        let mut i = self.pos;
        while matches!(b.get(i), Some(b' ' | b'\t')) {
            i += 1;
        }
        if b.get(i) == Some(&b'"') {
            let quote_end = b
                .get(i + 1..)
                .and_then(|rest| rest.iter().position(|&x| x == b'"'))
                .map(|rel| i + 1 + rel);
            if let Some(q_end) = quote_end {
                let mut j = q_end + 1;
                while matches!(b.get(j), Some(b' ' | b'\t')) {
                    j += 1;
                }
                if starts_closer(j) {
                    self.pos = j;
                    self.pending_close = Some(open);
                    return (span(i + 1, q_end), true);
                }
            }
        }

        let start = i;
        let mut j = start;
        let mut closed = false;
        while let Some(&byte) = b.get(j) {
            if byte == b'\n' {
                break;
            }
            if starts_closer(j) {
                closed = true;
                break;
            }
            j += 1;
        }
        let label = trimmed(b, start, j);
        self.pos = j;
        if closed {
            self.pending_close = Some(open);
        }
        (label, closed)
    }

    /// Reads an edge label after an opening `|`, up to the closing `|` on the same line.
    /// When closed, the closing pipe is left to be returned by `next()`.
    pub fn pipe_text(&mut self) -> (Span, bool) {
        let b = self.src.as_bytes();
        let mut i = self.pos;
        while matches!(b.get(i), Some(b' ' | b'\t')) {
            i += 1;
        }
        if b.get(i) == Some(&b'"') {
            let quote_end = b
                .get(i + 1..)
                .and_then(|rest| rest.iter().position(|&x| x == b'"'))
                .map(|rel| i + 1 + rel);
            if let Some(q_end) = quote_end {
                let mut j = q_end + 1;
                while matches!(b.get(j), Some(b' ' | b'\t')) {
                    j += 1;
                }
                if b.get(j) == Some(&b'|') {
                    self.pos = j;
                    return (span(i + 1, q_end), true);
                }
            }
        }
        let start = i;
        let mut j = start;
        while let Some(&byte) = b.get(j) {
            if byte == b'|' || byte == b'\n' {
                break;
            }
            j += 1;
        }
        let closed = b.get(j) == Some(&b'|');
        self.pos = j;
        (trimmed(b, start, j), closed)
    }

    /// Reads the body of an `@{ ... }` block up to its closing brace (quotes respected).
    /// The closing brace is left to be returned by `next()` as `Tok::RBrace`.
    pub fn brace_body(&mut self) -> (Span, bool) {
        let b = self.src.as_bytes();
        let start = self.pos;
        let mut j = start;
        let mut in_quote = false;
        let mut closed = false;
        while let Some(&byte) = b.get(j) {
            if byte == b'"' {
                in_quote = !in_quote;
            } else if byte == b'}' && !in_quote {
                closed = true;
                break;
            }
            j += 1;
        }
        self.pos = j;
        (trimmed(b, start, j), closed)
    }

    /// Reads the rest of the current line (up to newline, `;` or a `%%` comment), trimmed.
    pub fn rest_of_line(&mut self) -> Span {
        let b = self.src.as_bytes();
        let start = self.pos;
        let mut j = start;
        while let Some(&byte) = b.get(j) {
            if byte == b'\n' || byte == b';' || (byte == b'%' && b.get(j + 1) == Some(&b'%')) {
                break;
            }
            j += 1;
        }
        self.pos = j;
        trimmed(b, start, j)
    }

    /// Advances to the next newline or `;` without consuming it.
    pub fn skip_line(&mut self) {
        let b = self.src.as_bytes();
        let mut j = self.pos;
        while let Some(&byte) = b.get(j) {
            if byte == b'\n' || byte == b';' {
                break;
            }
            j += 1;
        }
        self.pos = j;
        self.pending_close = None;
        self.after_at = false;
    }
}
