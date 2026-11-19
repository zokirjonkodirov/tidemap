//! Markdown into a flat list of "leaves": the blocks that are laid out and painted one by one.
//!
//! Nesting (block quotes, lists) is flattened into an indent and a quote depth on each leaf, so
//! the view can find the leaves on screen with a binary search and never walks a tree per frame.

use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
};

use pulldown_cmark::{Alignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};
use tidemap_model::DiagramBlock;

pub const BOLD: u8 = 1;
pub const ITALIC: u8 = 2;
pub const CODE: u8 = 4;
pub const STRIKE: u8 = 8;
pub const LINK: u8 = 16;
/// Superscript (footnote references).
pub const SUP: u8 = 32;
/// Converted math: italic.
pub const MATH: u8 = 64;

/// A stretch of text with one style.
#[derive(Clone, Debug, PartialEq, Hash)]
pub struct Run {
    pub text: String,
    pub style: u8,
    /// Index into `Doc::links`.
    pub link: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Hash)]
pub enum Marker {
    Bullet(u8),
    Number(String),
    Task(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Heading(u8),
    Paragraph,
    /// The title line of a GitHub alert (`> [!NOTE]`): 1 note, 2 tip, 3 important, 4 warning, 5 caution.
    Alert(u8),
    /// A display formula, centred.
    Display,
}

#[derive(Clone, Debug)]
pub struct Table {
    pub align: Vec<Alignment>,
    pub head: Vec<Vec<Run>>,
    pub rows: Vec<Vec<Vec<Run>>>,
}

#[derive(Clone, Debug)]
pub enum Kind {
    Text { role: Role, runs: Vec<Run> },
    Code { lang: String, text: String },
    /// A fenced Mermaid block. `block` indexes the file's `DiagramBlock`s when they line up.
    Diagram { block: Option<usize>, source: String },
    Rule,
    Table(Table),
    #[allow(dead_code)]
    Html(String),
    Image { alt: String, src: String, link: Option<u32> },
}

#[derive(Clone, Debug)]
pub struct Leaf {
    pub kind: Kind,
    /// Block quote depth.
    pub quote: u8,
    /// List nesting depth (0 outside lists).
    pub list: u8,
    pub marker: Option<Marker>,
    /// Kind of the GitHub alert this leaf sits in (see `Role::Alert`), 0 for none.
    pub alert: u8,
    /// Space above the leaf in points.
    pub gap: f32,
    /// Source line, 1-based.
    pub line: usize,
    /// Hash of everything that decides how the leaf looks (not where it is).
    pub hash: u64,
    /// The text a selection copies; `Text` galleys hold exactly these chars.
    pub plain: String,
}

/// A top-level Markdown block: where it sits in the source and which leaves it produced. Edits
/// re-parse only the top-level blocks around them.
#[derive(Clone, Debug)]
pub struct Top {
    pub start: usize,
    pub end: usize,
    pub leaf_start: usize,
    pub leaf_end: usize,
    /// An unclosed fence or an HTML block may swallow what follows it: re-parsing it alone is not safe.
    pub risky: bool,
}

#[derive(Default, Clone)]
pub struct Doc {
    pub leaves: Vec<Leaf>,
    pub tops: Vec<Top>,
    /// Footnotes or reference links tie blocks together; such documents are always parsed whole.
    pub global: bool,
    pub links: Vec<String>,
    /// Heading slugs (GitHub style) to the leaf that holds them.
    pub anchors: HashMap<String, usize>,
}

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_GFM
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_MATH
}

/// Counts lines up to ever larger offsets without rescanning.
struct Lines<'a> {
    text: &'a str,
    at: usize,
    line: usize,
}

impl Lines<'_> {
    fn of(&mut self, byte: usize) -> usize {
        if byte < self.at {
            self.at = 0;
            self.line = 1;
        }
        let byte = byte.min(self.text.len());
        self.line += memchr::memchr_iter(b'\n', &self.text.as_bytes()[self.at..byte]).count();
        self.at = byte;
        self.line
    }
}

enum Container {
    Quote(u8),
    List(Option<u64>),
    Item,
}

struct Builder<'a> {
    doc: Doc,
    lines: Lines<'a>,
    blocks: &'a [DiagramBlock],
    bom: usize,
    stack: Vec<Container>,
    runs: Vec<Run>,
    style: u8,
    link: Vec<u32>,
    /// Source offset where the open text leaf started.
    text_at: usize,
    role: Role,
    pending: Option<Marker>,
    code: Option<(String, String, usize)>,
    html: Option<(String, usize)>,
    table: Option<(Table, usize)>,
    cell: Option<Vec<Run>>,
    row: Vec<Vec<Run>>,
    in_head: bool,
    image: Option<(String, String, usize)>,
    heading_text: String,
    html_style: u8,
    html_link: bool,
    html_skip: bool,
    notes: HashMap<String, usize>,
    footnote_anchor: Option<String>,
    depth: usize,
    top_start: usize,
    top_leaf: usize,
    top_kind: u8,
    tops: Vec<Top>,
    global: bool,
}

fn level_of(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for ch in text.trim().chars() {
        if ch.is_alphanumeric() || ch == '_' || ch == '-' {
            out.extend(ch.to_lowercase());
        } else if ch == ' ' {
            out.push('-');
        }
    }
    out
}

/// Space above a leaf, from what precedes it.
pub fn gap_between(previous: Option<&Leaf>, kind: &Kind, list: u8) -> f32 {
    let Some(previous) = previous else {
        return 0.0;
    };
    if let Kind::Text { role: Role::Heading(level), .. } = kind {
        if matches!(previous.kind, Kind::Text { role: Role::Heading(_), .. }) {
            return 10.0;
        }
        return match level {
            1 => 32.0,
            2 => 28.0,
            3 => 22.0,
            _ => 18.0,
        };
    }
    if matches!(previous.kind, Kind::Text { role: Role::Heading(_), .. }) {
        return 12.0;
    }
    if matches!(previous.kind, Kind::Text { role: Role::Alert(_), .. }) {
        return 4.0;
    }
    if list > 0 && previous.list > 0 {
        return 6.0;
    }
    16.0
}

fn hash_of(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

impl Builder<'_> {
    fn quote_depth(&self) -> u8 {
        self.stack.iter().filter(|c| matches!(c, Container::Quote(_))).count().min(8) as u8
    }

    fn list_depth(&self) -> u8 {
        self.stack.iter().filter(|c| matches!(c, Container::List(_))).count().min(8) as u8
    }

    fn gap_for(&self, kind: &Kind, list: u8) -> f32 {
        gap_between(self.doc.leaves.last(), kind, list)
    }

    fn push(&mut self, kind: Kind, plain: String, at: usize) {
        let list = self.list_depth();
        let quote = self.quote_depth();
        let marker = self.pending.take();
        let alert = self
            .stack
            .iter()
            .rev()
            .find_map(|c| match c {
                Container::Quote(kind) if *kind > 0 => Some(*kind),
                _ => None,
            })
            .unwrap_or(0);
        let gap = self.gap_for(&kind, list);
        let line = self.lines.of(at);
        let leaf_index = self.doc.leaves.len();
        if let Some(key) = self.footnote_anchor.take() {
            self.doc.anchors.insert(key, leaf_index);
        }
        if let Kind::Text { role: Role::Heading(_), .. } = &kind {
            let mut anchor = slug(&self.heading_text);
            let base = anchor.clone();
            let mut n = 1;
            while self.doc.anchors.contains_key(&anchor) {
                anchor = format!("{base}-{n}");
                n += 1;
            }
            self.doc.anchors.insert(anchor, leaf_index);
        }
        let hash = match &kind {
            Kind::Text { role, runs } => hash_of(&(0u8, role, runs, &marker, list, quote, alert)),
            Kind::Code { lang, text } => hash_of(&(1u8, lang, text, list, quote)),
            Kind::Diagram { source, .. } => hash_of(&(2u8, source, list, quote)),
            Kind::Rule => hash_of(&(3u8, list, quote)),
            Kind::Table(table) => {
                let rows: Vec<&Vec<Vec<Run>>> = table.rows.iter().collect();
                hash_of(&(4u8, &table.head, rows, list, quote))
            }
            Kind::Html(text) => hash_of(&(5u8, text, list, quote)),
            Kind::Image { alt, src, link } => hash_of(&(6u8, alt, src, link, list, quote)),
        };
        self.doc.leaves.push(Leaf { kind, quote, list, marker, alert, gap, line, hash, plain });
        self.heading_text.clear();
    }

    fn flush_text(&mut self) {
        if self.runs.is_empty() {
            return;
        }
        let mut runs = std::mem::take(&mut self.runs);
        while runs.last().is_some_and(|run| run.text.trim_end().is_empty() && run.style & CODE == 0) {
            runs.pop();
        }
        if let Some(last) = runs.last_mut() {
            if last.style & CODE == 0 {
                last.text.truncate(last.text.trim_end().len());
            }
        }
        if runs.is_empty() {
            return;
        }
        let plain = runs.iter().map(|run| run.text.as_str()).collect::<String>();
        let role = self.role;
        self.role = Role::Paragraph;
        let at = self.text_at;
        self.push(Kind::Text { role, runs }, plain, at);
    }

    /// Footnote numbers follow first appearance, reference or definition.
    fn note_number(&mut self, name: &str) -> usize {
        let next = self.notes.len() + 1;
        *self.notes.entry(name.to_lowercase()).or_insert(next)
    }

    fn push_run(&mut self, text: &str, style: u8, link: Option<u32>) {
        if text.is_empty() {
            return;
        }
        let target = match &mut self.cell {
            Some(cell) => cell,
            None => &mut self.runs,
        };
        if let Some(last) = target.last_mut()
            && last.style == style
            && last.link == link
        {
            last.text.push_str(text);
        } else {
            target.push(Run { text: text.to_owned(), style, link });
        }
    }

    fn text(&mut self, text: &str) {
        if self.html_skip {
            return;
        }
        if let Some((alt, _, _)) = &mut self.image {
            alt.push_str(text);
            return;
        }
        if matches!(self.role, Role::Heading(_)) {
            self.heading_text.push_str(text);
        }
        let style = self.style | self.html_style;
        let link = self.link.last().copied();
        let flags = style | if link.is_some() { LINK } else { 0 };
        if link.is_some() || style & CODE != 0 || !text.contains("://") {
            self.push_run(text, flags, link);
            return;
        }
        // Bare web addresses become links.
        let mut rest = text;
        while let Some(at) = ["https://", "http://"].iter().filter_map(|p| rest.find(p)).min() {
            self.push_run(&rest[..at], flags, None);
            let tail = &rest[at..];
            let mut end = tail.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"')).unwrap_or(tail.len());
            while end > 0
                && (matches!(tail.as_bytes()[end - 1], b'.' | b',' | b';' | b':' | b'!' | b'?' | b'\'' | b'*')
                    || (tail.as_bytes()[end - 1] == b')' && !tail[..end].contains('(')))
            {
                end -= 1;
            }
            let scheme = if tail.starts_with("https://") { 8 } else { 7 };
            if end <= scheme {
                let keep = scheme.min(tail.len());
                self.push_run(&tail[..keep], flags, None);
                rest = &tail[keep..];
                continue;
            }
            self.doc.links.push(tail[..end].to_owned());
            let index = self.doc.links.len() as u32 - 1;
            self.push_run(&tail[..end], flags | LINK, Some(index));
            rest = &tail[end..];
        }
        self.push_run(rest, flags, None);
    }

    /// Converts raw HTML (a block, or one inline tag) into runs and leaves.
    fn html(&mut self, html: &str, at: usize) {
        let mut rest = html;
        while !rest.is_empty() {
            let Some(lt) = rest.find('<') else {
                self.html_text(rest);
                break;
            };
            let (text, tail) = rest.split_at(lt);
            self.html_text(text);
            if let Some(comment) = tail.strip_prefix("<!--") {
                rest = comment.find("-->").map_or("", |end| &comment[end + 3..]);
                continue;
            }
            match tail.find('>') {
                Some(end) => {
                    self.html_tag(&tail[1..end], at);
                    rest = &tail[end + 1..];
                }
                None => {
                    self.html_text(tail);
                    break;
                }
            }
        }
    }

    fn html_text(&mut self, text: &str) {
        if text.is_empty() || self.html_skip {
            return;
        }
        let decoded = decode_entities(text);
        let mut collapsed = String::with_capacity(decoded.len());
        let mut space = false;
        let open = self.runs.last().is_some_and(|run| !run.text.ends_with([' ', '\n']));
        for ch in decoded.chars() {
            if ch.is_whitespace() {
                space = true;
            } else {
                if space && (!collapsed.is_empty() || open) {
                    collapsed.push(' ');
                }
                space = false;
                collapsed.push(ch);
            }
        }
        if space && !collapsed.is_empty() {
            collapsed.push(' ');
        }
        if !collapsed.is_empty() {
            self.text(&collapsed);
        }
    }

    fn html_tag(&mut self, tag: &str, at: usize) {
        let closing = tag.starts_with('/');
        let body = tag.trim_start_matches('/').trim_end_matches('/');
        let name = body.split(|c: char| c.is_whitespace()).next().unwrap_or("").to_ascii_lowercase();
        let toggle = |this: &mut Self, flag: u8| {
            if closing {
                this.html_style &= !flag;
            } else {
                this.html_style |= flag;
            }
        };
        match name.as_str() {
            "br" => self.text("\n"),
            "b" | "strong" => toggle(self, BOLD),
            "i" | "em" | "cite" | "var" => toggle(self, ITALIC),
            "s" | "strike" | "del" => toggle(self, STRIKE),
            "code" | "kbd" | "samp" | "tt" => toggle(self, CODE),
            "a" => {
                if closing {
                    if self.html_link {
                        self.link.pop();
                        self.html_link = false;
                    }
                } else if let Some(href) = html_attr(body, "href") {
                    self.doc.links.push(decode_entities(&href));
                    self.link.push(self.doc.links.len() as u32 - 1);
                    self.html_link = true;
                }
            }
            "img" => {
                let alt = html_attr(body, "alt").unwrap_or_default();
                let src = html_attr(body, "src").unwrap_or_default();
                if self.runs.is_empty() && self.cell.is_none() {
                    let link = self.link.last().copied();
                    self.push(Kind::Image { alt: alt.clone(), src, link }, alt, at);
                } else {
                    let label = if alt.is_empty() { "image".to_owned() } else { alt };
                    let style = self.style | ITALIC;
                    self.push_run(&format!("[{label}]"), style, None);
                }
            }
            "hr" => {
                self.flush_text();
                self.push(Kind::Rule, String::new(), at);
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush_text();
                if !closing {
                    self.text_at = at;
                    self.role = Role::Heading(name.as_bytes()[1] - b'0');
                    self.heading_text.clear();
                }
            }
            "summary" => {
                self.flush_text();
                toggle(self, BOLD);
            }
            "li" => {
                self.flush_text();
                if !closing {
                    self.text_at = at;
                    self.pending = Some(Marker::Bullet(self.list_depth().saturating_sub(1)));
                }
            }
            "td" | "th" => {
                if closing {
                    self.text("   ");
                }
            }
            "p" | "div" | "details" | "section" | "article" | "center" | "blockquote" | "ul" | "ol" | "table" | "tr" | "pre"
            | "figure" | "figcaption" | "header" | "footer" | "nav" | "main" | "aside" | "dl" | "dt" | "dd" => {
                self.flush_text();
                self.text_at = at;
            }
            "script" | "style" => self.html_skip = !closing,
            _ => {}
        }
    }
}

fn html_attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(name) {
        let at = from + found;
        from = at + name.len();
        let before_ok = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let after = tag[from..].trim_start();
        if before_ok && let Some(value) = after.strip_prefix('=') {
            let value = value.trim_start();
            return Some(match value.chars().next() {
                Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or("").to_owned(),
                _ => value.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or("").to_owned(),
            });
        }
    }
    None
}

fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let Some(end) = tail.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let entity = &tail[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "copy" => Some('\u{a9}'),
            "mdash" => Some('\u{2014}'),
            "ndash" => Some('\u{2013}'),
            "hellip" => Some('\u{2026}'),
            _ => entity.strip_prefix('#').and_then(|n| {
                n.strip_prefix(['x', 'X'])
                    .map_or_else(|| n.parse().ok(), |h| u32::from_str_radix(h, 16).ok())
                    .and_then(char::from_u32)
            }),
        };
        match decoded {
            Some(ch) => {
                out.push(ch);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Parses `content` (the whole file as stored, BOM and all). `blocks` are the file's diagrams.
pub fn parse(content: &str, blocks: &[DiagramBlock]) -> Doc {
    let bom = usize::from(content.starts_with('\u{feff}')) * 3;
    parse_text(&content[bom..], bom, 1, blocks)
}

/// Parses `text`, which starts at byte `bom` of the file and at line `first_line`.
fn parse_text(text: &str, bom: usize, first_line: usize, blocks: &[DiagramBlock]) -> Doc {
    let mut b = Builder {
        doc: Doc::default(),
        lines: Lines { text, at: 0, line: first_line },
        blocks,
        bom,
        stack: Vec::new(),
        runs: Vec::new(),
        style: 0,
        link: Vec::new(),
        text_at: 0,
        role: Role::Paragraph,
        pending: None,
        code: None,
        html: None,
        table: None,
        cell: None,
        row: Vec::new(),
        in_head: false,
        image: None,
        heading_text: String::new(),
        html_style: 0,
        html_link: false,
        html_skip: false,
        notes: HashMap::new(),
        footnote_anchor: None,
        depth: 0,
        top_start: 0,
        top_leaf: 0,
        top_kind: 0,
        tops: Vec::new(),
        global: false,
    };
    for (event, span) in Parser::new_ext(text, options()).into_offset_iter() {
        let block_start = matches!(&event, Event::Start(t) if !inline_tag(t));
        let block_end = matches!(&event, Event::End(t) if !inline_end(t));
        let rule = matches!(&event, Event::Rule);
        match &event {
            Event::Start(Tag::Link { link_type: LinkType::Reference | LinkType::Collapsed | LinkType::Shortcut, .. })
            | Event::FootnoteReference(_)
            | Event::Start(Tag::FootnoteDefinition(_)) => b.global = true,
            _ => {}
        }
        if block_start && b.depth == 0 {
            b.top_start = span.start;
            b.top_leaf = b.doc.leaves.len();
            b.top_kind = match &event {
                Event::Start(Tag::HtmlBlock) => 1,
                Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))) => 2,
                _ => 0,
            };
        }
        if block_start {
            b.depth += 1;
        }
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    b.flush_text();
                    b.text_at = span.start;
                    b.role = Role::Paragraph;
                }
                Tag::Heading { level, .. } => {
                    b.flush_text();
                    b.text_at = span.start;
                    b.role = Role::Heading(level_of(level));
                    b.heading_text.clear();
                }
                Tag::BlockQuote(kind) => {
                    b.flush_text();
                    let (code, label) = match kind {
                        Some(BlockQuoteKind::Note) => (1, "Note"),
                        Some(BlockQuoteKind::Tip) => (2, "Tip"),
                        Some(BlockQuoteKind::Important) => (3, "Important"),
                        Some(BlockQuoteKind::Warning) => (4, "Warning"),
                        Some(BlockQuoteKind::Caution) => (5, "Caution"),
                        None => (0, ""),
                    };
                    b.stack.push(Container::Quote(code));
                    if code > 0 {
                        let runs = vec![Run { text: label.to_owned(), style: BOLD, link: None }];
                        b.push(Kind::Text { role: Role::Alert(code), runs }, label.to_owned(), span.start);
                    }
                }
                Tag::CodeBlock(kind) => {
                    b.flush_text();
                    let lang = match kind {
                        CodeBlockKind::Fenced(info) => {
                            info.split(|c: char| c.is_whitespace() || c == '{' || c == ',').find(|w| !w.is_empty()).unwrap_or("").to_owned()
                        }
                        CodeBlockKind::Indented => String::new(),
                    };
                    b.code = Some((lang, String::new(), span.start));
                }
                Tag::List(start) => {
                    b.flush_text();
                    b.stack.push(Container::List(start));
                }
                Tag::Item => {
                    b.flush_text();
                    let depth = b.list_depth().saturating_sub(1);
                    let marker = match b.stack.iter_mut().rev().find_map(|c| match c {
                        Container::List(n) => Some(n),
                        _ => None,
                    }) {
                        Some(Some(n)) => {
                            let marker = Marker::Number(format!("{n}."));
                            *n += 1;
                            marker
                        }
                        _ => Marker::Bullet(depth),
                    };
                    b.pending = Some(marker);
                    b.text_at = span.start;
                    b.stack.push(Container::Item);
                }
                Tag::Table(align) => {
                    b.flush_text();
                    b.table = Some((Table { align, head: Vec::new(), rows: Vec::new() }, span.start));
                }
                Tag::TableHead => {
                    b.in_head = true;
                    b.row.clear();
                }
                Tag::TableRow => b.row.clear(),
                Tag::TableCell => b.cell = Some(Vec::new()),
                Tag::Emphasis => b.style |= ITALIC,
                Tag::Strong => b.style |= BOLD,
                Tag::Strikethrough => b.style |= STRIKE,
                Tag::Link { dest_url, .. } => {
                    b.doc.links.push(dest_url.to_string());
                    b.link.push(b.doc.links.len() as u32 - 1);
                }
                Tag::Image { dest_url, .. } => {
                    b.image = Some((String::new(), dest_url.to_string(), span.start));
                }
                Tag::HtmlBlock => {
                    b.flush_text();
                    b.html = Some((String::new(), span.start));
                }
                Tag::FootnoteDefinition(name) => {
                    b.flush_text();
                    let number = b.note_number(&name);
                    b.stack.push(Container::List(None));
                    b.pending = Some(Marker::Number(format!("{number}.")));
                    b.footnote_anchor = Some(format!("fn-{}", name.to_lowercase()));
                    b.text_at = span.start;
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph | TagEnd::Heading(_) => b.flush_text(),
                TagEnd::BlockQuote(_) => {
                    b.flush_text();
                    b.stack.pop();
                }
                TagEnd::List(_) => {
                    b.flush_text();
                    b.stack.pop();
                }
                TagEnd::Item => {
                    b.flush_text();
                    b.pending = None;
                    b.stack.pop();
                }
                TagEnd::CodeBlock => {
                    if let Some((lang, text, at)) = b.code.take() {
                        let plain = text.trim_end_matches('\n').to_owned();
                        if lang.eq_ignore_ascii_case("mermaid") {
                            let found = b.blocks.partition_point(|block| block.fence_start <= at + b.bom).checked_sub(1);
                            let block = found.filter(|index| at + b.bom < b.blocks[*index].end.max(b.blocks[*index].start + 1));
                            let source = block.map_or_else(|| plain.clone(), |index| b.blocks[index].source.clone());
                            b.push(Kind::Diagram { block, source }, plain, at);
                        } else {
                            b.push(Kind::Code { lang, text: plain.clone() }, plain, at);
                        }
                    }
                }
                TagEnd::Table => {
                    if let Some((table, at)) = b.table.take() {
                        let plain = table_text(&table);
                        b.push(Kind::Table(table), plain, at);
                    }
                }
                TagEnd::TableHead => {
                    let row = std::mem::take(&mut b.row);
                    if let Some((table, _)) = &mut b.table {
                        table.head = row;
                    }
                    b.in_head = false;
                }
                TagEnd::TableRow => {
                    let row = std::mem::take(&mut b.row);
                    if let Some((table, _)) = &mut b.table {
                        table.rows.push(row);
                    }
                }
                TagEnd::TableCell => {
                    if let Some(cell) = b.cell.take() {
                        b.row.push(cell);
                    }
                }
                TagEnd::Emphasis => b.style &= !ITALIC,
                TagEnd::Strong => b.style &= !BOLD,
                TagEnd::Strikethrough => b.style &= !STRIKE,
                TagEnd::Link => {
                    b.link.pop();
                }
                TagEnd::Image => {
                    if let Some((alt, src, at)) = b.image.take() {
                        // An image alone in its paragraph becomes a block; inside text it is its alt text.
                        let alone = b.runs.is_empty() && b.cell.is_none();
                        if alone {
                            b.flush_text();
                            let plain = alt.clone();
                            let link = b.link.last().copied();
                            b.push(Kind::Image { alt, src, link }, plain, at);
                        } else {
                            let label = if alt.is_empty() { "image".to_owned() } else { alt };
                            let style = b.style | ITALIC;
                            let target = match &mut b.cell {
                                Some(cell) => cell,
                                None => &mut b.runs,
                            };
                            target.push(Run { text: format!("[{label}]"), style, link: None });
                        }
                    }
                }
                TagEnd::FootnoteDefinition => {
                    b.flush_text();
                    b.pending = None;
                    b.footnote_anchor = None;
                    b.stack.pop();
                }
                TagEnd::HtmlBlock => {
                    if let Some((html, at)) = b.html.take() {
                        b.text_at = at;
                        b.role = Role::Paragraph;
                        b.html(&html, at);
                        b.flush_text();
                        b.html_style = 0;
                        b.html_skip = false;
                        if b.html_link {
                            b.link.pop();
                            b.html_link = false;
                        }
                    }
                }
                _ => {}
            },
            Event::Text(t) => {
                if let Some((_, code, _)) = &mut b.code {
                    code.push_str(&t);
                } else {
                    if b.runs.is_empty() && b.cell.is_none() && b.pending.is_some() {
                        b.text_at = span.start;
                    }
                    b.text(&t);
                }
            }
            Event::Code(t) => {
                let saved = b.style;
                b.style |= CODE;
                b.text(&t);
                b.style = saved;
            }
            Event::Html(t) => {
                if let Some((html, _)) = &mut b.html {
                    html.push_str(&t);
                }
            }
            Event::InlineHtml(t) => b.html(&t, span.start),
            Event::SoftBreak => b.text(" "),
            Event::HardBreak => b.text("\n"),
            Event::FootnoteReference(name) => {
                let number = b.note_number(&name);
                b.doc.links.push(format!("#fn-{}", name.to_lowercase()));
                let index = b.doc.links.len() as u32 - 1;
                let style = b.style | SUP | LINK;
                b.push_run(&number.to_string(), style, Some(index));
            }
            Event::InlineMath(t) => {
                let saved = b.style;
                b.style |= MATH;
                let text = super::latex::to_unicode(&t);
                b.text(&text);
                b.style = saved;
            }
            Event::DisplayMath(t) => {
                b.flush_text();
                let text = super::latex::to_unicode(&t);
                let runs = vec![Run { text: text.clone(), style: MATH, link: None }];
                b.push(Kind::Text { role: Role::Display, runs }, text, span.start);
            }
            Event::Rule => {
                b.flush_text();
                b.push(Kind::Rule, String::new(), span.start);
            }
            Event::TaskListMarker(done) => {
                b.pending = Some(Marker::Task(done));
            }
        }
        if block_end {
            b.depth = b.depth.saturating_sub(1);
            if b.depth == 0 {
                let risky = match b.top_kind {
                    1 => true,
                    2 => !fence_closed(&text[b.top_start..span.end.min(text.len())]),
                    _ => false,
                };
                b.tops.push(Top { start: b.top_start + b.bom, end: span.end + b.bom, leaf_start: b.top_leaf, leaf_end: b.doc.leaves.len(), risky });
            }
        } else if rule && b.depth == 0 {
            let end = b.doc.leaves.len();
            b.tops.push(Top { start: span.start + b.bom, end: span.end + b.bom, leaf_start: end.saturating_sub(1), leaf_end: end, risky: false });
        }
    }
    b.flush_text();
    b.doc.tops = std::mem::take(&mut b.tops);
    b.doc.global = b.global;
    b.doc
}

/// The text of a table: rows on lines, cells separated by tabs (for search and copy).
fn table_text(table: &Table) -> String {
    let row = |cells: &Vec<Vec<Run>>| cells.iter().map(|cell| cell.iter().map(|run| run.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("\t");
    std::iter::once(row(&table.head)).chain(table.rows.iter().map(row)).collect::<Vec<_>>().join("\n")
}

fn inline_tag(tag: &Tag) -> bool {
    matches!(tag, Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Superscript | Tag::Subscript | Tag::Link { .. } | Tag::Image { .. })
}

fn inline_end(tag: &TagEnd) -> bool {
    matches!(tag, TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Superscript | TagEnd::Subscript | TagEnd::Link | TagEnd::Image)
}

/// Whether a fenced code block's text ends with its closing fence.
fn fence_closed(block: &str) -> bool {
    let mut lines = block.trim_end().lines();
    let first = lines.next().is_some();
    match lines.last() {
        Some(last) => {
            first && {
                let t = last.trim_start_matches([' ', '\t', '>']);
                t.starts_with("```") || t.starts_with("~~~")
            }
        }
        None => false,
    }
}

// ---------------------------------------------------------------------------------------
// Incremental re-parse
// ---------------------------------------------------------------------------------------

/// What an edit changed in the leaf list: `removed` leaves at `at` became `inserted` leaves.
pub struct Splice {
    pub at: usize,
    pub removed: usize,
    pub inserted: usize,
}

fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    let n = a.len().min(b.len());
    let mut i = 0;
    while i + 8 <= n && a[i..i + 8] == b[i..i + 8] {
        i += 8;
    }
    while i < n && a[i] == b[i] {
        i += 1;
    }
    i
}

fn common_suffix(a: &[u8], b: &[u8], limit: usize) -> usize {
    let n = a.len().min(b.len()).min(limit);
    let mut i = 0;
    while i + 8 <= n && a[a.len() - i - 8..a.len() - i] == b[b.len() - i - 8..b.len() - i] {
        i += 8;
    }
    while i < n && a[a.len() - 1 - i] == b[b.len() - 1 - i] {
        i += 1;
    }
    i
}

fn rebuild_anchors(doc: &mut Doc) {
    doc.anchors.clear();
    for (index, leaf) in doc.leaves.iter().enumerate() {
        if let Kind::Text { role: Role::Heading(_), .. } = &leaf.kind {
            let base = slug(&leaf.plain);
            let mut anchor = base.clone();
            let mut n = 1;
            while doc.anchors.contains_key(&anchor) {
                anchor = format!("{base}-{n}");
                n += 1;
            }
            doc.anchors.insert(anchor, index);
        }
    }
}

fn shift_links(leaves: &mut [Leaf], by: u32) {
    if by == 0 {
        return;
    }
    let shift = |runs: &mut Vec<Run>| {
        for run in runs {
            if let Some(link) = &mut run.link {
                *link += by;
            }
        }
    };
    for leaf in leaves {
        match &mut leaf.kind {
            Kind::Text { runs, .. } => shift(runs),
            Kind::Table(table) => {
                table.head.iter_mut().for_each(shift);
                table.rows.iter_mut().flatten().for_each(shift);
            }
            Kind::Image { link: Some(link), .. } => *link += by,
            _ => {}
        }
    }
}

/// Updates `doc` (parsed from `old`) to match `new` by re-parsing only the top-level blocks the
/// edit touched, with one neighbour on each side. `None` when that is not provably safe: the
/// document uses footnotes or reference links, the edit touches a reference definition, or the
/// re-parsed part ends in an unclosed fence or HTML block. The caller then parses the whole text.
pub fn reparse(doc: &mut Doc, old: &str, new: &str, blocks: &[DiagramBlock]) -> Option<Splice> {
    if doc.global || doc.tops.is_empty() || doc.links.len() > 2048 + doc.leaves.len() * 2 {
        return None;
    }
    let mut prefix = common_prefix(old.as_bytes(), new.as_bytes());
    while !old.is_char_boundary(prefix) || !new.is_char_boundary(prefix) {
        prefix -= 1;
    }
    if prefix == old.len() && prefix == new.len() {
        return Some(Splice { at: 0, removed: 0, inserted: 0 });
    }
    let mut suffix = common_suffix(old.as_bytes(), new.as_bytes(), old.len().min(new.len()) - prefix);
    while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
        suffix -= 1;
    }
    let old_edit_end = old.len() - suffix;
    let tops = &doc.tops;
    let k = tops.partition_point(|t| t.start <= prefix).saturating_sub(1);
    let first = k.saturating_sub(1);
    let j = tops.partition_point(|t| t.end < old_edit_end).min(tops.len() - 1);
    let last = (j + 1).min(tops.len() - 1);
    let region_start = tops[first].start.min(prefix);
    let old_region_end = if last == tops.len() - 1 { old.len() } else { tops[last].end };
    if old_region_end < old_edit_end {
        return None;
    }
    let delta = new.len() as isize - old.len() as isize;
    let new_region_end = usize::try_from(old_region_end as isize + delta).ok()?;
    if new_region_end < region_start || new_region_end > new.len() || !new.is_char_boundary(new_region_end) {
        return None;
    }
    let (old_region, new_region) = (&old[region_start..old_region_end], &new[region_start..new_region_end]);
    if [old_region, new_region].iter().any(|text| text.contains("]:") || text.contains("[^")) {
        return None;
    }
    let first_line = 1 + memchr::memchr_iter(b'\n', &new.as_bytes()[..region_start]).count();
    let part = parse_text(new_region, region_start, first_line, blocks);
    if part.global || part.tops.last().is_some_and(|top| top.risky) {
        return None;
    }

    let leaf_start = doc.tops[first].leaf_start;
    let leaf_end = doc.tops[last].leaf_end;
    let removed = leaf_end - leaf_start;
    let mut fresh = part.leaves;
    shift_links(&mut fresh, doc.links.len() as u32);
    doc.links.extend(part.links);
    let inserted = fresh.len();
    let lines_in = |text: &str| memchr::memchr_iter(b'\n', text.as_bytes()).count() as isize;
    let line_delta = lines_in(new_region) - lines_in(old_region);
    if line_delta != 0 {
        for leaf in &mut doc.leaves[leaf_end..] {
            leaf.line = (leaf.line as isize + line_delta).max(1) as usize;
        }
    }
    doc.leaves.splice(leaf_start..leaf_end, fresh);
    let leaf_delta = inserted as isize - removed as isize;
    let mut new_tops = part.tops;
    for top in &mut new_tops {
        top.leaf_start += leaf_start;
        top.leaf_end += leaf_start;
    }
    for top in &mut doc.tops[last + 1..] {
        top.start = (top.start as isize + delta) as usize;
        top.end = (top.end as isize + delta) as usize;
        top.leaf_start = (top.leaf_start as isize + leaf_delta) as usize;
        top.leaf_end = (top.leaf_end as isize + leaf_delta) as usize;
    }
    doc.tops.splice(first..=last, new_tops);
    // The space above a leaf depends on its neighbour: refresh the two seams.
    for index in [leaf_start, leaf_start + inserted] {
        if index < doc.leaves.len() {
            let gap = gap_between(index.checked_sub(1).map(|i| &doc.leaves[i]), &doc.leaves[index].kind, doc.leaves[index].list);
            doc.leaves[index].gap = gap;
        }
    }
    rebuild_anchors(doc);
    Some(Splice { at: leaf_start, removed, inserted })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(doc: &Doc) -> Vec<&'static str> {
        doc.leaves
            .iter()
            .map(|leaf| match &leaf.kind {
                Kind::Text { role: Role::Heading(_), .. } => "h",
                Kind::Text { .. } => "p",
                Kind::Code { .. } => "code",
                Kind::Diagram { .. } => "diagram",
                Kind::Rule => "rule",
                Kind::Table(_) => "table",
                Kind::Html(_) => "html",
                Kind::Image { .. } => "image",
            })
            .collect()
    }

    #[test]
    fn flattens_blocks() {
        let md = "# Title\n\nHello **bold** and `code`.\n\n- one\n- two\n  - nested\n\n> quote\n\n```mermaid\ngraph TD\n A-->B\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n---\n";
        let doc = parse(md, &[]);
        assert_eq!(kinds(&doc), ["h", "p", "p", "p", "p", "p", "diagram", "table", "rule"]);
        assert_eq!(doc.leaves[0].line, 1);
        assert_eq!(doc.leaves[3].list, 1);
        assert_eq!(doc.leaves[4].list, 2);
        assert_eq!(doc.leaves[5].quote, 1);
        assert!(matches!(doc.leaves[2].marker, Some(Marker::Bullet(0))));
        assert_eq!(doc.anchors.get("title"), Some(&0));
        let Kind::Text { runs, .. } = &doc.leaves[1].kind else { panic!() };
        assert!(runs.iter().any(|run| run.style & BOLD != 0 && run.text == "bold"));
        assert!(runs.iter().any(|run| run.style & CODE != 0 && run.text == "code"));
    }

    #[test]
    fn mermaid_maps_to_extracted_blocks() {
        let md = "text\n\n```mermaid\ngraph TD\n A-->B\n```\n\n```mermaid\ngraph LR\n C-->D\n```\n";
        let blocks = tidemap_core::source::extract_diagrams(md, "a.md", &[]);
        assert_eq!(blocks.len(), 2);
        let doc = parse(md, &blocks);
        let found: Vec<Option<usize>> = doc
            .leaves
            .iter()
            .filter_map(|leaf| match leaf.kind {
                Kind::Diagram { block, .. } => Some(block),
                _ => None,
            })
            .collect();
        assert_eq!(found, [Some(0), Some(1)]);
    }

    #[test]
    fn numbered_and_task_items() {
        let doc = parse("3. a\n4. b\n\n- [x] done\n- [ ] todo\n", &[]);
        assert!(matches!(&doc.leaves[0].marker, Some(Marker::Number(n)) if n == "3."));
        assert!(matches!(&doc.leaves[1].marker, Some(Marker::Number(n)) if n == "4."));
        assert!(matches!(doc.leaves[2].marker, Some(Marker::Task(true))));
        assert!(matches!(doc.leaves[3].marker, Some(Marker::Task(false))));
    }
}

#[cfg(test)]
mod incremental {
    use super::*;

    fn signature(doc: &Doc) -> Vec<(u64, usize, u8, u8, u32)> {
        doc.leaves.iter().map(|l| (l.hash, l.line, l.quote, l.list, l.gap.to_bits())).collect()
    }

    #[test]
    fn edits_match_a_full_parse() {
        let mut text = String::new();
        for i in 0..40 {
            text.push_str(&format!("## Heading {i}\n\nSome *text* {i}.\n\n- a\n- b\n\n```rust\nlet x = {i};\n```\n\n"));
        }
        let edits: Vec<(usize, usize, &str)> = vec![
            (30, 0, "more "),
            (0, 0, "# Title\n\n"),
            (text.len() / 2, 5, ""),
            (text.len() / 3, 0, "\n\nnew paragraph\n\n"),
            (text.len() - 3, 0, "tail"),
            (100, 40, "- x\n- y\n"),
        ];
        let mut doc = parse(&text, &[]);
        for (at, remove, insert) in edits {
            let mut at = at.min(text.len());
            while !text.is_char_boundary(at) {
                at -= 1;
            }
            let end = (at + remove).min(text.len());
            let mut next = text.clone();
            next.replace_range(at..end, insert);
            let spliced = reparse(&mut doc, &text, &next, &[]);
            let full = parse(&next, &[]);
            if spliced.is_none() {
                doc = full;
            } else {
                assert_eq!(signature(&doc), signature(&full), "edit at {at} drifted");
                assert_eq!(doc.anchors, full.anchors);
            }
            text = next;
        }
    }

    #[test]
    fn breaking_a_fence_falls_back() {
        let text = "para\n\n```rust\nlet x = 1;\n```\n\nafter\n\nmore\n";
        let mut doc = parse(text, &[]);
        let broken = text.replace("```\n\nafter", "\n\nafter");
        assert!(reparse(&mut doc, text, &broken, &[]).is_none(), "an unclosed fence needs a full parse");
    }

    #[test]
    fn typing_in_a_big_file_is_fast() {
        let mut text = String::new();
        for i in 0..6000 {
            text.push_str(&format!("## Section {i}\n\nSome **bold** text with `code` and more words.\n\n- one\n- two\n\n"));
        }
        let mut doc = parse(&text, &[]);
        let at = text.len() / 2;
        let mut next = text.clone();
        next.insert(at, 'x');
        let started = std::time::Instant::now();
        let spliced = reparse(&mut doc, &text, &next, &[]);
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        eprintln!("incremental re-parse of {:.1} MB: {ms:.2} ms", text.len() as f64 / 1e6);
        assert!(spliced.is_some());
        assert!(ms < 8.0, "took {ms:.2} ms");
    }
}

#[cfg(test)]
mod speed {
    use super::*;
    use std::time::Instant;

    #[test]
    fn parses_megabytes_quickly() {
        let mut md = String::new();
        for i in 0..6000 {
            md.push_str(&format!(
                "## Section {i}\n\nSome **bold** text with `code`, a [link](https://example.com/{i}) and more words to wrap around the line a few times over.\n\n- item one\n- item two\n  - nested\n\n```mermaid\ngraph TD\n  A{i}-->B\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n"
            ));
        }
        let blocks = tidemap_core::source::extract_diagrams(&md, "big.md", &[]);
        let started = Instant::now();
        let doc = parse(&md, &blocks);
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        eprintln!("parsed {:.1} MB into {} leaves in {ms:.1} ms", md.len() as f64 / 1e6, doc.leaves.len());
        assert!(doc.leaves.len() > 30_000);
        assert!(ms < 400.0);
    }
}
