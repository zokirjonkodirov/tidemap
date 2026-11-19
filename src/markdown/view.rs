//! The rendered Markdown page: virtualized layout, painting, selection and inline diagrams.
//!
//! A document is a flat list of leaves (`parse::Leaf`). Every leaf has a height: exact once it
//! was laid out at the current width, an estimate before. Only leaves on or near the screen are
//! laid out and painted, so opening or scrolling a huge file costs the same as a small one. A
//! leaf's layout is cached by content, so typing in the source re-lays out only what changed.
//! Heights that change above the viewport (an estimate turning exact, a diagram finishing) move
//! the scroll offset with them, so the text under the reader's eyes stays put.

use std::{
    collections::HashMap,
    sync::Arc,
};

use egui::{
    Align2, Color32, Context, CursorIcon, EventFilter, FontFamily, FontId, Galley, Id, Key, Margin, Order, Pos2, Rect, RichText, Sense,
    Stroke, StrokeKind, TextFormat, Ui, pos2, text::{CCursor, CCursorRange, LayoutJob}, vec2,
};
use pulldown_cmark::Alignment;
use tidemap_diagram::{Family, ItemId, StyleSheet};
use tidemap_model::{Camera, DiagramBlock, RenderArtifact, RenderRequest};
use tidemap_renderer::Engine;

use super::highlight;
use super::images::{self, Entry as ImageEntry, Images};
use super::parse::{self, reparse, BOLD, CODE, Doc, ITALIC, Kind, LINK, Leaf, MATH, Marker, Role, Run, STRIKE, SUP, Table, parse};
use crate::{
    icons::{self, Icon},
    motion, paint,
    theme::{self, Palette},
};

const MARGIN_X: f32 = 32.0;
const MAX_WIDTH: f32 = 800.0;
const BODY: f32 = 15.0;
const BODY_LINE: f32 = 25.0;
const CODE_SIZE: f32 = 13.0;
const CODE_LINE: f32 = 20.0;
const CODE_PAD: f32 = 12.0;
/// Height of the strip that names the language above a code block.
const CODE_HEAD: f32 = 30.0;
const QUOTE_STEP: f32 = 18.0;
const LIST_STEP: f32 = 24.0;
const TOP_PAD: f32 = 28.0;
const BOTTOM_PAD: f32 = 96.0;
const DIAGRAM_PAD: f32 = 16.0;
const DIAGRAM_MAX: f32 = 760.0;
const DIAGRAM_PLACEHOLDER: f32 = 168.0;
const CELL_PAD_X: f32 = 12.0;
const CELL_PAD_Y: f32 = 8.0;
/// Layout work past the screen edge is bounded to this many milliseconds a frame.
const AHEAD_BUDGET_MS: f64 = 3.0;

const MAX_MATCHES: usize = 20_000;

/// Find on the page: the query, every match as (leaf, first char, end char), and the current one.
#[derive(Default)]
struct Find {
    open: bool,
    query: String,
    focus: bool,
    matches: Vec<(usize, usize, usize)>,
    current: usize,
    /// (document revision, lowercase query) the matches were computed for.
    stamp: Option<(u64, String)>,
    /// The query changed: jump to the first match at or below the top of the page.
    restart: bool,
    /// Leaf and char to bring into view; re-aimed every frame until the page has settled there.
    reveal: Option<(usize, usize)>,
    settled: u8,
}

/// What the page asks the app to do.
pub enum Action {
    SelectDiagram(String),
    OpenDiagram(String),
    OpenUrl(String),
    OpenPath(String),
    EditLine(usize),
    Copied,
}

pub struct Input<'a> {
    pub file_id: &'a str,
    pub content: &'a str,
    /// Folder of the document, for relative image paths.
    pub dir: Option<&'a std::path::Path>,
    pub revision: u64,
    pub blocks: &'a [DiagramBlock],
    pub active_block: &'a str,
    pub p: Palette,
    pub dark: bool,
    pub style: &'a StyleSheet,
    pub saved_scroll: f32,
}

pub struct Output {
    pub actions: Vec<Action>,
    pub scroll: f32,
}

/// What a laid-out leaf keeps between frames.
enum Slot {
    Text(Arc<Galley>),
    Code(Arc<Galley>),
    Table(TableLayout),
    Plain,
}

struct TableLayout {
    widths: Vec<f32>,
    /// Row height and the galley of each cell; the first row is the header.
    rows: Vec<(f32, Vec<Arc<Galley>>)>,
    align: Vec<Alignment>,
    width: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Pos {
    leaf: usize,
    ch: usize,
}

#[derive(Clone, Copy, Debug)]
struct Selection {
    anchor: Pos,
    head: Pos,
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    None,
    Text(u8),
    Bar(f32),
}

pub struct DocView {
    key: Option<(String, u64)>,
    doc: Arc<Doc>,
    width: f32,
    heights: Vec<f32>,
    tops: Vec<f32>,
    height_cache: HashMap<u64, f32>,
    slots: HashMap<u64, (Arc<Slot>, u64)>,
    artifacts: HashMap<u64, Arc<RenderArtifact>>,
    /// The newest picture of each diagram id, shown until its replacement is ready.
    stale: HashMap<String, Arc<RenderArtifact>>,
    requested: HashMap<u64, (f64, f64)>,
    frame: u64,
    scroll_y: f32,
    target_y: f32,
    file: String,
    last_active: String,
    sel: Option<Selection>,
    drag: Drag,
    anchor_word: Option<(Pos, Pos)>,
    last_click: Option<(f64, Pos2, u8)>,
    pending_link: Option<(u32, Pos2)>,
    scratch: Vec<ItemId>,
    bar_alpha_until: f64,
    scroll_to_leaf: Option<usize>,
    /// Sideways scroll of long code blocks, by leaf hash.
    hscroll: HashMap<u64, f32>,
    /// The code block under the pointer last frame and how far it can scroll.
    hover_code: Option<(u64, f32)>,
    /// A code block whose copy button was just used, and when.
    copied: Option<(u64, f64)>,
    hover_link: Option<(usize, u32)>,
    hover_url: Option<String>,
    last_url: Option<String>,
    /// The diagram the reader picked on this page; only that one wears the selection ring.
    chosen: String,
    images: Images,
    dir: Option<std::path::PathBuf>,
    /// The text `doc` was parsed from, to find what an edit changed.
    content: String,
    find: Find,
}

impl Default for DocView {
    fn default() -> Self {
        Self {
            key: None,
            doc: Arc::new(Doc::default()),
            width: 0.0,
            heights: Vec::new(),
            tops: vec![0.0],
            height_cache: HashMap::new(),
            slots: HashMap::new(),
            artifacts: HashMap::new(),
            stale: HashMap::new(),
            requested: HashMap::new(),
            frame: 0,
            scroll_y: 0.0,
            target_y: 0.0,
            file: String::new(),
            last_active: String::new(),
            sel: None,
            drag: Drag::None,
            anchor_word: None,
            last_click: None,
            pending_link: None,
            scratch: Vec::new(),
            bar_alpha_until: 0.0,
            scroll_to_leaf: None,
            hscroll: HashMap::new(),
            hover_code: None,
            copied: None,
            hover_link: None,
            hover_url: None,
            last_url: None,
            chosen: String::new(),
            images: Images::default(),
            dir: None,
            content: String::new(),
            find: Find::default(),
        }
    }
}

/// Fill of code boxes and table headers: a step away from the page in both themes.
fn well(p: Palette) -> Color32 {
    if p.ink.r() > 200 { theme::alpha(Color32::WHITE, 14) } else { p.field }
}

fn key_of(leaf: &Leaf, width: f32) -> u64 {
    leaf.hash ^ ((width.round() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

fn font_for(style: u8, size: f32, heading: bool) -> FontId {
    if style & CODE != 0 {
        FontId::monospace((size * 0.88).round())
    } else if style & BOLD != 0 || heading {
        FontId::new(size, theme::font_semibold())
    } else {
        FontId::new(size, FontFamily::Proportional)
    }
}

struct TextStyle {
    size: f32,
    line: f32,
    color: Color32,
    heading: bool,
}

/// Accent of a GitHub alert: note, tip, important, warning, caution.
fn alert_color(kind: u8, p: Palette) -> Color32 {
    let dark = p.ink.r() > 200;
    let pick = |l: (u8, u8, u8), d: (u8, u8, u8)| {
        let (r, g, b) = if dark { d } else { l };
        Color32::from_rgb(r, g, b)
    };
    match kind {
        1 => p.accent,
        2 => pick((0x2f, 0x7a, 0x4c), (0x7f, 0xc4, 0x93)),
        3 => pick((0x82, 0x50, 0xb8), (0xb9, 0x9a, 0xe6)),
        4 => pick((0xa8, 0x6a, 0x14), (0xe0, 0xb0, 0x6a)),
        _ => p.danger,
    }
}

fn text_style(role: Role, quote: bool, p: Palette) -> TextStyle {
    let (size, line) = match role {
        Role::Heading(1) => (30.0, 38.0),
        Role::Heading(2) => (23.0, 32.0),
        Role::Heading(3) => (19.0, 28.0),
        Role::Heading(4) => (16.5, 26.0),
        Role::Heading(_) => (15.0, 24.0),
        Role::Paragraph => (BODY, BODY_LINE),
        Role::Alert(_) => (13.5, 22.0),
        Role::Display => (17.0, 28.0),
    };
    let color = match role {
        Role::Heading(6) => p.secondary,
        Role::Heading(_) => p.ink,
        Role::Paragraph if quote => p.secondary,
        Role::Paragraph => p.ink,
        Role::Alert(kind) => alert_color(kind, p),
        Role::Display => p.ink,
    };
    TextStyle { size, line, color, heading: matches!(role, Role::Heading(_)) }
}

fn job_of(runs: &[Run], style: &TextStyle, p: Palette, width: f32, bold_all: bool) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width.max(20.0);
    for run in runs {
        let mut flags = run.style;
        if bold_all {
            flags |= BOLD;
        }
        let code = flags & CODE != 0;
        let link = flags & LINK != 0;
        let color = if link {
            p.accent
        } else if code || (flags & BOLD != 0 && style.color == p.secondary) {
            p.ink
        } else {
            style.color
        };
        let sup = flags & SUP != 0;
        let size = if sup { (style.size * 0.72).round() } else { style.size };
        let mut format = TextFormat {
                font_id: font_for(flags, size, style.heading),
                color,
                italics: flags & (ITALIC | MATH) != 0,
                underline: if link && !sup { Stroke::new(1.0_f32, theme::alpha(p.accent, 110)) } else { Stroke::NONE },
                strikethrough: if flags & STRIKE != 0 { Stroke::new(1.0_f32, style.color) } else { Stroke::NONE },
                line_height: Some(style.line),
                ..Default::default()
            };
        if sup {
            format.valign = egui::Align::TOP;
        }
        job.append(&run.text, 0.0, format);
    }
    job
}

fn plain_job(text: &str, font: FontId, color: Color32, line: f32, width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width.max(20.0);
    job.append(
        text,
        0.0,
        TextFormat { font_id: font, color, line_height: Some(line), ..Default::default() },
    );
    job
}

/// Space above the first code line: a header strip when the block names its language.
fn code_top(lang: &str) -> f32 {
    if lang.is_empty() { CODE_PAD } else { CODE_HEAD + 8.0 }
}

fn estimate(leaf: &Leaf, width: f32) -> f32 {
    let indent = leaf.quote as f32 * QUOTE_STEP + leaf.list as f32 * LIST_STEP;
    let wrap = (width - indent).max(60.0);
    let body = match &leaf.kind {
        Kind::Text { role, .. } => {
            let style = text_style(*role, false, theme::palette(false));
            let chars = leaf.plain.chars().count() as f32;
            let lines = (chars * style.size * 0.52 / wrap).ceil().max(1.0) + leaf.plain.matches('\n').count() as f32;
            lines * style.line
        }
        Kind::Code { lang, .. } => leaf.plain.lines().count().max(1) as f32 * CODE_LINE + code_top(lang) + CODE_PAD,
        Kind::Diagram { .. } => DIAGRAM_PLACEHOLDER,
        Kind::Rule => 1.0,
        Kind::Table(table) => (table.rows.len() + 1) as f32 * (BODY_LINE + 2.0 * CELL_PAD_Y),
        Kind::Html(text) => text.lines().count().max(1) as f32 * CODE_LINE + CODE_PAD,
        Kind::Image { .. } => 120.0,
    };
    leaf.gap + body
}

impl DocView {
    /// Opens the find bar (or puts the caret back into it).
    pub fn open_find(&mut self) {
        self.find.open = true;
        self.find.focus = true;
        self.find.stamp = None;
    }

    fn refresh_matches(&mut self, doc: &Doc) {
        let stamp = (self.key.as_ref().map_or(0, |key| key.1), self.find.query.to_lowercase());
        if self.find.stamp.as_ref() == Some(&stamp) {
            return;
        }
        self.find.matches.clear();
        let needle = stamp.1.clone();
        self.find.stamp = Some(stamp);
        if !self.find.open || needle.is_empty() {
            self.find.current = 0;
            return;
        }
        'leaves: for (index, leaf) in doc.leaves.iter().enumerate() {
            let text = match &leaf.kind {
                Kind::Text { .. } | Kind::Code { .. } | Kind::Html(_) | Kind::Table(_) => leaf.plain.as_str(),
                _ => continue,
            };
            if text.len() < needle.len() {
                continue;
            }
            let (mut rest, mut chars) = (text, 0usize);
            while let Some((from, to)) = crate::editor::find_fold(rest, &needle) {
                let start = chars + rest[..from].chars().count();
                let end = start + rest[from..to].chars().count();
                self.find.matches.push((index, start, end));
                chars = end;
                rest = &rest[to..];
                if self.find.matches.len() >= MAX_MATCHES {
                    break 'leaves;
                }
            }
        }
        if self.find.restart || self.find.current >= self.find.matches.len() {
            let top = self.tops.partition_point(|top| *top <= self.scroll_y).saturating_sub(1);
            self.find.current = self.find.matches.iter().position(|m| m.0 >= top).unwrap_or(0);
            self.find.restart = false;
        }
        self.find.reveal = self.find.matches.get(self.find.current).map(|m| (m.0, m.1));
        self.find.settled = 0;
    }

    fn step_match(&mut self, forward: bool) {
        let count = self.find.matches.len();
        if count == 0 {
            return;
        }
        self.find.current = if forward { (self.find.current + 1) % count } else { (self.find.current + count - 1) % count };
        let (leaf, from, _) = self.find.matches[self.find.current];
        self.find.reveal = Some((leaf, from));
        self.find.settled = 0;
    }

    /// Distance from the top of leaf `index` to char `ch`.
    fn char_y(&mut self, ctx: &Context, doc: &Doc, index: usize, ch: usize, width: f32, p: Palette) -> f32 {
        let leaf = &doc.leaves[index];
        let cursor = CCursor { index: ch, prefer_next_row: true };
        let slot = self.slot(ctx, doc, index, width, p);
        match (&leaf.kind, &*slot) {
            (Kind::Text { .. }, Slot::Text(galley)) => galley.pos_from_cursor(cursor).min.y,
            (Kind::Code { lang, .. }, Slot::Code(galley)) => code_top(lang) + galley.pos_from_cursor(cursor).min.y,
            _ => 0.0,
        }
    }

    /// The matches inside leaf `index` as (first char, end char, is the current one).
    fn leaf_matches(&self, index: usize) -> Vec<(usize, usize, bool)> {
        if !self.find.open || self.find.matches.is_empty() {
            return Vec::new();
        }
        let all = &self.find.matches;
        let from = all.partition_point(|m| m.0 < index);
        let mut out = Vec::new();
        for (offset, m) in all[from..].iter().take_while(|m| m.0 == index).enumerate() {
            out.push((m.1, m.2, from + offset == self.find.current));
        }
        out
    }

    fn find_bar(&mut self, ctx: &Context, rect: Rect, p: Palette) {
        let shown = motion::appear(ctx, Id::new("page-find"), self.find.open, motion::BASE);
        if shown < 0.01 {
            return;
        }
        let width = 380.0f32.min(rect.width() - 28.0);
        let pos = pos2(rect.right() - width - 24.0, rect.top() + 12.0 - (1.0 - shown) * 10.0);
        let field_id = Id::new("page-find-field");
        let (mut step, mut close, mut changed) = (None, false, false);
        let count = self.find.matches.len();
        let label = if self.find.query.is_empty() {
            String::new()
        } else if count == 0 {
            "No results".to_owned()
        } else {
            format!("{} of {}{}", self.find.current + 1, count, if count >= MAX_MATCHES { "+" } else { "" })
        };
        egui::Area::new(Id::new("page-find-bar")).order(Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
            ui.multiply_opacity(shown);
            theme::elevated(ui, p, theme::Level::Raised, 12.0, Margin::symmetric(10, 4), |ui| {
                ui.set_width(width - 20.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut self.find.query)
                            .id(field_id)
                            .frame(false)
                            .desired_width((width - 176.0).max(60.0))
                            .hint_text("Find on the page\u{2026}"),
                    );
                    if std::mem::take(&mut self.find.focus) {
                        field.request_focus();
                        if let Some(mut state) = egui::TextEdit::load_state(ctx, field_id) {
                            let end = self.find.query.chars().count();
                            state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(end))));
                            state.store(ctx, field_id);
                        }
                    }
                    changed = field.changed();
                    if field.lost_focus() {
                        let (enter, shift, escape) =
                            ctx.input(|i| (i.key_pressed(Key::Enter), i.modifiers.shift, i.key_pressed(Key::Escape)));
                        if enter {
                            step = Some(!shift);
                            field.request_focus();
                        }
                        close |= escape;
                    }
                    let color = if count == 0 && !self.find.query.is_empty() { p.danger } else { p.muted };
                    ui.label(RichText::new(label.as_str()).size(12.0).color(color));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close |= icons::button(ui, Icon::Close, "Close · Esc", false, true).clicked();
                        if icons::button(ui, Icon::Forward, "Next match · Enter", false, count > 0).clicked() {
                            step = Some(true);
                        }
                        if icons::button(ui, Icon::Back, "Previous match · Shift+Enter", false, count > 0).clicked() {
                            step = Some(false);
                        }
                    });
                });
            });
        });
        if changed {
            self.find.restart = true;
            self.find.stamp = None;
            ctx.request_repaint();
        }
        if let Some(forward) = step {
            self.step_match(forward);
            ctx.request_repaint();
        }
        if close {
            self.find.open = false;
            self.find.stamp = None;
            self.find.reveal = None;
        }
    }

    fn reload(&mut self, input: &Input, width: f32) {
        // Remember what is on screen, to put it back after the leaves change.
        let anchor = self.anchor();
        let same_file = self.key.as_ref().is_some_and(|(file, _)| file == input.file_id) && self.width == width;
        let spliced = if same_file && self.heights.len() == self.doc.leaves.len() {
            reparse(Arc::make_mut(&mut self.doc), &self.content, input.content, input.blocks)
        } else {
            None
        };
        self.key = Some((input.file_id.to_owned(), input.revision));
        self.content.clear();
        self.content.push_str(input.content);
        self.sel = None;
        let mapped = match spliced {
            Some(splice) => {
                let doc = self.doc.clone();
                let height_of = |this: &Self, index: usize| {
                    let leaf = &doc.leaves[index];
                    this.height_cache.get(&key_of(leaf, width)).copied().unwrap_or_else(|| estimate(leaf, width))
                };
                let fresh: Vec<f32> = (splice.at..splice.at + splice.inserted).map(|i| height_of(self, i)).collect();
                self.heights.splice(splice.at..splice.at + splice.removed, fresh);
                // The leaf after the edit may have a new gap above it.
                let after = splice.at + splice.inserted;
                if after < self.heights.len() {
                    self.heights[after] = height_of(self, after);
                }
                self.recompute_tops();
                anchor.map(|(hash, offset, index)| {
                    let found = if index < splice.at {
                        index
                    } else if index >= splice.at + splice.removed {
                        index + splice.inserted - splice.removed
                    } else {
                        (splice.at..splice.at + splice.inserted).find(|i| doc.leaves[*i].hash == hash).unwrap_or(splice.at)
                    };
                    (found, offset)
                })
            }
            None => {
                self.doc = Arc::new(parse(input.content, input.blocks));
                self.rebuild_heights(width);
                self.images.retry_failed();
                anchor.map(|(hash, offset, index)| {
                    let doc = self.doc.clone();
                    let count = doc.leaves.len();
                    let found = (0..64usize)
                        .flat_map(|d| [index.checked_sub(d), Some(index + d)])
                        .flatten()
                        .filter(|i| *i < count)
                        .find(|i| doc.leaves[*i].hash == hash)
                        .unwrap_or_else(|| index.min(count.saturating_sub(1)));
                    (found, offset)
                })
            }
        };
        if let Some((found, offset)) = mapped
            && !self.doc.leaves.is_empty()
        {
            let found = found.min(self.doc.leaves.len() - 1);
            self.scroll_y = self.tops[found] + offset;
            self.target_y = self.scroll_y;
        }
    }

    /// The leaf at the top of the viewport: its hash, the scroll offset into it and its index.
    fn anchor(&self) -> Option<(u64, f32, usize)> {
        let count = self.doc.leaves.len();
        if count == 0 || self.tops.len() != count + 1 {
            return None;
        }
        let index = self.tops.partition_point(|top| *top <= self.scroll_y).saturating_sub(1).min(count - 1);
        Some((self.doc.leaves[index].hash, self.scroll_y - self.tops[index], index))
    }

    fn rebuild_heights(&mut self, width: f32) {
        self.width = width;
        let doc = self.doc.clone();
        self.heights = doc
            .leaves
            .iter()
            .map(|leaf| self.height_cache.get(&key_of(leaf, width)).copied().unwrap_or_else(|| estimate(leaf, width)))
            .collect();
        self.recompute_tops();
    }

    fn recompute_tops(&mut self) {
        self.tops.clear();
        self.tops.reserve(self.heights.len() + 1);
        let mut y = TOP_PAD;
        self.tops.push(y);
        for height in &self.heights {
            y += height;
            self.tops.push(y);
        }
    }

    fn total(&self) -> f32 {
        self.tops.last().copied().unwrap_or(0.0) + BOTTOM_PAD
    }

    // -------------------------------------------------------------------------------------
    // Layout of single leaves
    // -------------------------------------------------------------------------------------

    fn inner_width(leaf: &Leaf, width: f32) -> f32 {
        (width - leaf.quote as f32 * QUOTE_STEP - leaf.list as f32 * LIST_STEP).max(80.0)
    }

    fn slot(&mut self, ctx: &Context, doc: &Doc, index: usize, width: f32, p: Palette) -> Arc<Slot> {
        let leaf = &doc.leaves[index];
        let key = key_of(leaf, width) ^ (p.ink.r() as u64) << 56;
        if let Some((slot, used)) = self.slots.get_mut(&key) {
            *used = self.frame;
            return slot.clone();
        }
        let inner = Self::inner_width(leaf, width);
        let slot = match &leaf.kind {
            Kind::Text { role, runs } => {
                let style = text_style(*role, leaf.quote > 0, p);
                let job = job_of(runs, &style, p, inner, false);
                Slot::Text(ctx.fonts_mut(|fonts| fonts.layout_job(job)))
            }
            Kind::Code { lang, text } => {
                let job = highlight::code_job(lang, text, p, FontId::monospace(CODE_SIZE), CODE_LINE);
                Slot::Code(ctx.fonts_mut(|fonts| fonts.layout_job(job)))
            }
            Kind::Html(text) => {
                let job = plain_job(text, FontId::monospace(CODE_SIZE - 1.0), p.muted, CODE_LINE, inner - 2.0 * CODE_PAD);
                Slot::Code(ctx.fonts_mut(|fonts| fonts.layout_job(job)))
            }
            Kind::Table(table) => Slot::Table(layout_table(ctx, table, inner, p)),
            Kind::Diagram { .. } | Kind::Rule | Kind::Image { .. } => Slot::Plain,
        };
        let slot = Arc::new(slot);
        self.slots.insert(key, (slot.clone(), self.frame));
        slot
    }

    /// Height of leaf `index` (gap included) at the current width, laying it out if needed.
    fn measure(&mut self, ctx: &Context, doc: &Doc, index: usize, width: f32, p: Palette, input: &Input) -> f32 {
        let leaf = &doc.leaves[index];
        let body = match &leaf.kind {
            Kind::Text { .. } | Kind::Code { .. } | Kind::Html(_) | Kind::Table(_) => {
                let slot = self.slot(ctx, doc, index, width, p);
                match &*slot {
                    Slot::Text(galley) => galley.size().y + if matches!(leaf.kind, Kind::Text { role: Role::Heading(1 | 2), .. }) { 10.0 } else { 0.0 },
                    Slot::Code(galley) => match &leaf.kind {
                        Kind::Code { lang, .. } => galley.size().y + code_top(lang) + CODE_PAD,
                        _ => galley.size().y + 2.0 * CODE_PAD,
                    },
                    Slot::Table(table) => table.rows.iter().map(|row| row.0).sum::<f32>() + 2.0,
                    Slot::Plain => 0.0,
                }
            }
            Kind::Rule => 1.0,
            Kind::Image { src, .. } => self.image_layout(src, Self::inner_width(leaf, width)).height(),
            Kind::Diagram { .. } => self.diagram_height(ctx, leaf, Self::inner_width(leaf, width), p, input),
        };
        leaf.gap + body
    }

    fn image_layout(&mut self, src: &str, inner: f32) -> ImageLayout {
        let Some(path) = images::resolve(self.dir.as_deref(), src) else { return ImageLayout::Remote };
        match self.images.get(&path) {
            ImageEntry::Loading => ImageLayout::Loading,
            ImageEntry::Failed(reason) => ImageLayout::Failed(reason.clone()),
            ImageEntry::Ready { texture, size, .. } => {
                let mut w = size.x.min(inner).max(1.0);
                let mut h = size.y * w / size.x.max(1.0);
                if h > IMAGE_MAX {
                    w *= IMAGE_MAX / h;
                    h = IMAGE_MAX;
                }
                ImageLayout::Ready { texture: texture.id(), size: vec2(w, h) }
            }
        }
    }

    fn diagram_height(&mut self, ctx: &Context, leaf: &Leaf, inner: f32, p: Palette, input: &Input) -> f32 {
        match self.artifact(leaf, input) {
            Some(artifact) => match diagram_fit(&artifact, inner) {
                Fit::Scene { height, .. } => height,
                Fit::Notice => notice_height(ctx, leaf, inner, p),
            },
            None => DIAGRAM_PLACEHOLDER,
        }
    }

    /// The finished layout of a diagram leaf, asking the engine for it when it is missing.
    fn artifact(&self, leaf: &Leaf, input: &Input) -> Option<Arc<RenderArtifact>> {
        self.artifacts.get(&leaf.hash).cloned().or_else(|| {
            // While a diagram is being redrawn after an edit, the last picture stays.
            let Kind::Diagram { block, .. } = &leaf.kind else { return None };
            let id = block.and_then(|i| input.blocks.get(i))?.id.as_str();
            self.stale.get(id).cloned()
        })
    }

    fn request_diagrams(&mut self, doc: &Doc, from: usize, to: usize, input: &Input, engine: &mut Engine, next_job: &mut u64, now: f64) {
        for index in from..to.min(doc.leaves.len()) {
            let leaf = &doc.leaves[index];
            if let Kind::Image { src, .. } = &leaf.kind {
                if let Some(path) = images::resolve(self.dir.as_deref(), src) {
                    self.images.get(&path);
                }
                continue;
            }
            let Kind::Diagram { block, source } = &leaf.kind else { continue };
            if self.artifacts.contains_key(&leaf.hash) {
                continue;
            }
            // The engine hashes the source on every lookup: ask a few times a second, not every frame.
            let (checked, asked) = self.requested.get(&leaf.hash).copied().unwrap_or((f64::MIN, f64::MIN));
            if now - checked < 0.1 {
                continue;
            }
            let id = block.and_then(|i| input.blocks.get(i)).map_or_else(|| format!("doc-{:x}", leaf.hash), |b| b.id.clone());
            if let Some(artifact) = engine.cached(&id, source) {
                self.stale.insert(id, artifact.clone());
                self.artifacts.insert(leaf.hash, artifact);
                continue;
            }
            if now - asked < 1.5 {
                self.requested.insert(leaf.hash, (now, asked));
                continue;
            }
            self.requested.insert(leaf.hash, (now, now));
            let job_id = *next_job;
            *next_job += 1;
            engine.prefetch(RenderRequest { job_id, diagram_id: id, revision: input.revision, source: source.clone() });
        }
    }

    // -------------------------------------------------------------------------------------
    // One frame
    // -------------------------------------------------------------------------------------

    pub fn show(&mut self, ui: &mut Ui, input: &Input, engine: &mut Engine, next_job: &mut u64) -> Output {
        let ctx = ui.ctx().clone();
        self.frame += 1;
        if self.dir.as_deref() != input.dir {
            self.dir = input.dir.map(std::path::Path::to_path_buf);
            self.images.retry_failed();
        }
        let p = input.p;
        let mut actions = Vec::new();
        let colors = input.style.colors(input.dark);
        let canvas = Color32::from_rgba_unmultiplied(colors.canvas[0], colors.canvas[1], colors.canvas[2], colors.canvas[3]);
        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, Sense::hover());
        let id = Id::new("document-view");
        let response = ui.interact(rect, id, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, canvas);

        let width = (rect.width() - 2.0 * MARGIN_X).clamp(160.0, MAX_WIDTH);
        let col_x = rect.left() + ((rect.width() - width) * 0.5).max(0.0);
        let now = ctx.input(|i| i.time);
        let view_h = rect.height();

        if self.file != input.file_id {
            self.file = input.file_id.to_owned();
            self.key = None;
            self.artifacts.clear();
            self.stale.clear();
            self.requested.clear();
            self.scroll_y = input.saved_scroll.max(0.0);
            self.target_y = self.scroll_y;
            self.last_active = input.active_block.to_owned();
            self.sel = None;
        }
        if self.key.as_ref().map(|(file, rev)| (file.as_str(), *rev)) != Some((input.file_id, input.revision)) {
            self.reload(input, width);
        } else if (self.width - width).abs() > 0.5 {
            let anchor = self.anchor();
            self.rebuild_heights(width);
            if let Some((_, offset, index)) = anchor {
                self.scroll_y = self.tops[index.min(self.tops.len() - 1)] + offset;
                self.target_y = self.scroll_y;
            }
        }
        let doc = self.doc.clone();
        let count = doc.leaves.len();
        self.images.poll(&ctx);
        self.refresh_matches(&doc);
        if let Some((leaf, ch)) = self.find.reveal
            && leaf < count
        {
            let max_scroll = (self.total() - view_h).max(0.0);
            let dy = self.char_y(&ctx, &doc, leaf, ch, width, p);
            let want = (self.tops[leaf] + doc.leaves[leaf].gap + dy - view_h * 0.38).clamp(0.0, max_scroll);
            self.target_y = want;
            if (self.scroll_y - want).abs() < 1.0 {
                self.find.settled += 1;
                if self.find.settled > 3 {
                    self.find.reveal = None;
                }
            } else {
                self.find.settled = 0;
            }
            // A match far along a long code line: slide the block sideways to it.
            if let Kind::Code { .. } = &doc.leaves[leaf].kind {
                let inner = Self::inner_width(&doc.leaves[leaf], width) - 2.0 * CODE_PAD;
                if let Slot::Code(galley) = &*self.slot(&ctx, &doc, leaf, width, p) {
                    let x = galley.pos_from_cursor(CCursor { index: ch, prefer_next_row: true }).min.x;
                    let at = self.hscroll.entry(doc.leaves[leaf].hash).or_insert(0.0);
                    if x < *at || x > *at + inner - 60.0 {
                        *at = (x - 60.0).max(0.0);
                    }
                }
            }
            ctx.request_repaint();
        } else {
            self.find.reveal = None;
        }

        // A diagram chosen elsewhere (sidebar, `[` and `]`) scrolls into view.
        if self.last_active != input.active_block {
            self.last_active = input.active_block.to_owned();
            self.scroll_to_leaf = doc.leaves.iter().position(|leaf| {
                matches!(&leaf.kind, Kind::Diagram { block: Some(b), .. } if input.blocks.get(*b).is_some_and(|block| block.id == input.active_block))
            });
        }
        if let Some(leaf) = self.scroll_to_leaf.take() {
            let top = self.tops[leaf] + doc.leaves[leaf].gap;
            let bottom = self.tops[leaf + 1];
            if top < self.scroll_y || bottom > self.scroll_y + view_h {
                self.target_y = (top - 32.0).max(0.0);
            }
        }

        // --- input: scrolling -------------------------------------------------------------
        let focused = ctx.memory(|m| m.has_focus(id));
        let hovered = response.hovered();
        if hovered {
            let (side, wheel) = ctx.input(|i| (i.smooth_scroll_delta.x, i.smooth_scroll_delta.y));
            if side != 0.0
                && let Some((hash, overflow)) = self.hover_code
            {
                let at = self.hscroll.entry(hash).or_insert(0.0);
                *at = (*at - side).clamp(0.0, overflow);
            }
            if wheel != 0.0 {
                self.find.reveal = None;
                self.scroll_y -= wheel;
                self.target_y = self.scroll_y;
                self.bar_alpha_until = now + 1.0;
            }
        }
        if focused {
            ctx.memory_mut(|m| {
                m.set_focus_lock_filter(id, EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() })
            });
            let page = view_h * 0.88;
            ctx.input_mut(|input| {
                let mut step = |key: Key, shift: bool, by: f32| {
                    if input.consume_key(if shift { egui::Modifiers::SHIFT } else { egui::Modifiers::NONE }, key) {
                        self.target_y += by;
                        self.bar_alpha_until = now + 1.0;
                    }
                };
                step(Key::ArrowDown, false, 48.0);
                step(Key::ArrowUp, false, -48.0);
                step(Key::PageDown, false, page);
                step(Key::PageUp, false, -page);
                step(Key::Space, false, page);
                step(Key::Space, true, -page);
                if input.consume_key(egui::Modifiers::NONE, Key::Home) || input.consume_key(egui::Modifiers::COMMAND, Key::Home) {
                    self.target_y = 0.0;
                }
                if input.consume_key(egui::Modifiers::NONE, Key::End) || input.consume_key(egui::Modifiers::COMMAND, Key::End) {
                    self.target_y = f32::MAX / 4.0;
                }
            });
            let events = ctx.input(|i| i.events.clone());
            for event in events {
                match event {
                    egui::Event::Copy => {
                        if let Some(text) = self.selected_text(&doc) {
                            ctx.copy_text(text);
                            actions.push(Action::Copied);
                        }
                    }
                    egui::Event::Key { key: Key::A, pressed: true, modifiers, .. } if modifiers.command && count > 0 => {
                        let last = count - 1;
                        self.sel = Some(Selection {
                            anchor: Pos { leaf: 0, ch: 0 },
                            head: Pos { leaf: last, ch: doc.leaves[last].plain.chars().count() },
                        });
                    }
                    _ => {}
                }
            }
        }
        let max_scroll = (self.total() - view_h).max(0.0);
        if (self.target_y - self.scroll_y).abs() > 0.4 {
            let dt = ctx.input(|i| i.stable_dt).min(0.05);
            let k = 1.0 - (-dt * 16.0).exp();
            self.target_y = self.target_y.clamp(0.0, max_scroll);
            self.scroll_y += (self.target_y - self.scroll_y) * k;
            ctx.request_repaint();
        } else {
            self.scroll_y = self.target_y.clamp(0.0, max_scroll);
        }
        self.scroll_y = self.scroll_y.clamp(0.0, max_scroll);
        if (self.scroll_y - self.target_y).abs() <= 0.4 {
            self.target_y = self.scroll_y;
        }

        // --- layout of what is on and near the screen -------------------------------------
        if count > 0 {
            let anchor = self.anchor();
            let first = self.tops.partition_point(|top| *top <= self.scroll_y).saturating_sub(1).min(count - 1);
            let mut last = first;
            while last + 1 < count && self.tops[last + 1] < self.scroll_y + view_h {
                last += 1;
            }
            let started = std::time::Instant::now();
            let mut changed = false;
            let mut layout = |this: &mut Self, index: usize| {
                let height = this.measure(&ctx, &doc, index, width, p, input);
                if (this.heights[index] - height).abs() > 0.01 {
                    this.heights[index] = height;
                    changed = true;
                }
                this.height_cache.insert(key_of(&doc.leaves[index], width), height);
            };
            self.request_diagrams(&doc, first.saturating_sub(4), (last + 12).min(count), input, engine, next_job, now);
            for index in first..=last {
                layout(self, index);
            }
            // Past the edges: as much as the budget allows, nearest first.
            let mut up = first;
            let mut down = last;
            while started.elapsed().as_secs_f64() * 1000.0 < AHEAD_BUDGET_MS {
                let mut worked = false;
                if down + 1 < count && self.tops[down + 1] < self.scroll_y + view_h * 2.0 {
                    down += 1;
                    layout(self, down);
                    worked = true;
                }
                if up > 0 && self.tops[up] > self.scroll_y - view_h {
                    up -= 1;
                    layout(self, up);
                    worked = true;
                }
                if !worked {
                    break;
                }
            }
            if changed {
                self.recompute_tops();
                if let Some((_, offset, index)) = anchor {
                    // Heights above the viewport changed: keep the anchor leaf where it was.
                    let moved = self.tops[index] + offset;
                    if (moved - self.scroll_y).abs() > 0.01 && index < first {
                        self.target_y += moved - self.scroll_y;
                        self.scroll_y = moved;
                    } else if index < self.tops.len() - 1 && (moved - self.scroll_y).abs() > 0.01 {
                        self.scroll_y = moved.min(self.tops[index + 1]);
                        self.target_y = self.scroll_y;
                    }
                }
                ctx.request_repaint();
            }
        }
        let max_scroll = (self.total() - view_h).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max_scroll);

        self.hover_code = None;
        // --- paint --------------------------------------------------------------------------
        let origin_y = rect.top() - self.scroll_y;
        let first = if count == 0 { 0 } else { self.tops.partition_point(|top| *top <= self.scroll_y).saturating_sub(1).min(count - 1) };
        let mut index = first;
        let mut hits: Vec<(usize, Rect)> = Vec::new();
        while index < count && self.tops[index] < self.scroll_y + view_h {
            let leaf = &doc.leaves[index];
            let top = origin_y + self.tops[index] + leaf.gap;
            let bottom = origin_y + self.tops[index + 1];
            let x = col_x + leaf.quote as f32 * QUOTE_STEP + leaf.list as f32 * LIST_STEP;
            let inner = Self::inner_width(leaf, width);
            let area = Rect::from_min_max(pos2(x, top), pos2(x + inner, bottom));
            self.paint_leaf(&ctx, &painter, &doc, index, area, col_x, width, input, &response, &mut hits, now);
            index += 1;
        }
        // Quote bars, drawn over whole runs of quoted leaves.
        {
            let mut i = first;
            while i < index.min(count) {
                let leaf = &doc.leaves[i];
                for depth in 0..leaf.quote {
                    let previous = i > 0 && doc.leaves[i - 1].quote > depth && doc.leaves[i - 1].alert == leaf.alert;
                    let top = origin_y + self.tops[i] + if previous { 0.0 } else { leaf.gap };
                    let bottom = origin_y + self.tops[i + 1];
                    let x = col_x + depth as f32 * QUOTE_STEP;
                    painter.rect_filled(
                        Rect::from_min_max(pos2(x, top), pos2(x + 3.0, bottom)),
                        1.5,
                        if leaf.alert > 0 { alert_color(leaf.alert, p) } else { theme::alpha(p.accent, 90) },
                    );
                }
                i += 1;
            }
        }

        // --- mouse --------------------------------------------------------------------------
        self.mouse(&ctx, &doc, input, &response, rect, col_x, width, origin_y, &mut actions, now, id);

        // --- scrollbar ------------------------------------------------------------------------
        self.scrollbar(&ctx, ui, &painter, rect, view_h, max_scroll, p, now);

        self.find_bar(&ctx, rect, p);
        let shown = motion::hover(&ctx, Id::new("link-chip"), self.hover_url.is_some());
        if shown > 0.01 {
            let url = self.hover_url.clone().or_else(|| self.last_url.clone()).unwrap_or_default();
            self.last_url = Some(url.clone());
            let shown_text = if url.chars().count() > 90 { format!("{}\u{2026}", url.chars().take(89).collect::<String>()) } else { url };
            let label = painter.layout_no_wrap(shown_text, FontId::proportional(12.0), p.secondary.gamma_multiply(shown));
            let pill = Rect::from_min_size(
                pos2(rect.left() + 12.0, rect.bottom() - 36.0 + (1.0 - shown) * 6.0),
                vec2(label.size().x + 20.0, 26.0),
            );
            painter.rect_filled(pill, 8.0, p.panel.gamma_multiply(shown));
            painter.rect_stroke(pill, 8.0, Stroke::new(1.0_f32, p.hairline.gamma_multiply(shown)), StrokeKind::Inside);
            painter.galley(pos2(pill.left() + 10.0, pill.center().y - label.size().y * 0.5), label, p.secondary);
        } else {
            self.last_url = None;
        }
        if self.scroll_y > max_scroll - 0.5 && self.target_y > max_scroll {
            self.target_y = max_scroll;
        }
        let _ = hits;
        Output { actions, scroll: self.scroll_y }
    }

    // -------------------------------------------------------------------------------------
    // Painting
    // -------------------------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn paint_leaf(
        &mut self,
        ctx: &Context,
        painter: &egui::Painter,
        doc: &Doc,
        index: usize,
        area: Rect,
        _col_x: f32,
        width: f32,
        input: &Input,
        response: &egui::Response,
        hits: &mut Vec<(usize, Rect)>,
        now: f64,
    ) {
        let p = input.p;
        let leaf = &doc.leaves[index];
        hits.push((index, area));
        let slot = self.slot(ctx, doc, index, width, p);
        let selection = self.selection_in(doc, index);
        match (&leaf.kind, &*slot) {
            (Kind::Text { role, runs }, Slot::Text(galley)) => {
                let at = text_origin(*role, galley, area);
                // Inline code sits in a soft rounded box.
                for (from, to) in code_ranges(runs) {
                    for (row, x0, x1) in spans(galley, from, to) {
                        let h = (row.height() * 0.8).min(22.0);
                        let top = at.y + row.min.y + (row.height() - h) * 0.5 - 2.0;
                        painter.rect_filled(
                            Rect::from_min_max(pos2(at.x + x0 - 2.5, top), pos2(at.x + x1 + 2.5, top + h)),
                            4.0,
                            theme::alpha(p.ink, if input.dark { 26 } else { 16 }),
                        );
                    }
                }
                if let Some((leaf_index, link)) = self.hover_link
                    && leaf_index == index
                    && let Some((from, to)) = link_range(runs, link)
                {
                    for (row, x0, x1) in spans(galley, from, to) {
                        painter.rect_filled(
                            Rect::from_min_max(pos2(at.x + x0 - 2.0, at.y + row.min.y + 1.0), pos2(at.x + x1 + 2.0, at.y + row.max.y - 1.0)),
                            4.0,
                            theme::alpha(p.accent, 30),
                        );
                    }
                }
                for (from, to, current) in self.leaf_matches(index) {
                    paint_selection(painter, galley, at, Some((from, to)), find_color(current, input.dark));
                }
                paint_selection(painter, galley, at, selection, selection_color(p, input.dark));
                painter.galley(at, galley.clone(), p.ink);
                if matches!(role, Role::Heading(1 | 2)) {
                    let y = at.y + galley.size().y + 6.0;
                    painter.hline(area.x_range(), y, Stroke::new(1.0_f32, p.hairline));
                }
                self.marker(painter, leaf, area, galley, p);
            }
            (Kind::Code { lang, .. } | Kind::Html(lang), Slot::Code(galley)) => {
                let is_html = matches!(leaf.kind, Kind::Html(_));
                let head = if is_html || lang.is_empty() { 0.0 } else { CODE_HEAD };
                let top_pad = if is_html { CODE_PAD } else { code_top(lang) };
                let boxr = Rect::from_min_size(area.min, vec2(area.width(), top_pad + galley.size().y + CODE_PAD));
                painter.rect_filled(boxr, 8.0, well(p));
                painter.rect_stroke(boxr, 8.0, Stroke::new(1.0_f32, theme::alpha(p.ink, if input.dark { 22 } else { 16 })), StrokeKind::Inside);
                if head > 0.0 {
                    painter.hline(boxr.left() + 1.0..=boxr.right() - 1.0, boxr.top() + head, Stroke::new(1.0_f32, theme::alpha(p.ink, if input.dark { 16 } else { 12 })));
                }
                let view = Rect::from_min_max(pos2(boxr.left() + CODE_PAD, boxr.top() + head), pos2(boxr.right() - CODE_PAD, boxr.bottom()));
                let overflow = (galley.size().x - view.width()).max(0.0);
                let hot = response.hovered() && ctx.pointer_hover_pos().is_some_and(|pos| boxr.contains(pos));
                if hot && overflow > 0.0 {
                    self.hover_code = Some((leaf.hash, overflow));
                }
                let scroll = self.hscroll.get(&leaf.hash).copied().unwrap_or(0.0).clamp(0.0, overflow);
                let at = pos2(view.left() - scroll, boxr.top() + top_pad);
                let clipped = painter.with_clip_rect(view.intersect(painter.clip_rect()));
                for (from, to, current) in self.leaf_matches(index) {
                    paint_selection(&clipped, galley, at, Some((from, to)), find_color(current, input.dark));
                }
                paint_selection(&clipped, galley, at, selection, selection_color(p, input.dark));
                clipped.galley(at, galley.clone(), p.ink);
                let alpha = motion::hover(ctx, Id::new(("code-chip", index)), hot);
                if overflow > 0.0 {
                    // A thin sideways scrollbar that shows while the pointer is over the block.
                    let track = Rect::from_min_max(pos2(view.left(), boxr.bottom() - 6.0), pos2(view.right(), boxr.bottom() - 3.0));
                    let thumb_w = (track.width() * view.width() / galley.size().x).max(28.0);
                    let x = track.left() + (track.width() - thumb_w) * (scroll / overflow);
                    painter.rect_filled(
                        Rect::from_min_size(pos2(x, track.top()), vec2(thumb_w, track.height())),
                        1.5,
                        theme::alpha(p.ink, 70).gamma_multiply(alpha),
                    );
                }
                if !is_html {
                    // The language sits on the left of the header; the copy button on the right.
                    // Blocks without a language show the copy button only while hovered.
                    if head > 0.0 {
                        let label = painter.layout_no_wrap(lang.to_owned(), FontId::new(12.0, theme::font_medium()), p.muted);
                        painter.galley(pos2(boxr.left() + CODE_PAD, boxr.top() + (head - label.size().y) * 0.5), label, p.muted);
                    }
                    let copied = self.copied.filter(|(hash, at)| *hash == leaf.hash && now - *at < 1.6);
                    if copied.is_some() {
                        ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    }
                    let shown = if copied.is_some() || head > 0.0 { if copied.is_some() { 1.0 } else { 0.55 + 0.45 * alpha } } else { alpha };
                    if shown > 0.01 {
                        let label = if copied.is_some() { "Copied" } else { "Copy" };
                        let color = if copied.is_some() { p.accent } else { p.secondary };
                        let text = painter.layout_no_wrap(label.to_owned(), FontId::new(12.0, theme::font_medium()), color.gamma_multiply(shown));
                        let height = if head > 0.0 { head - 8.0 } else { 24.0 };
                        let pill = Rect::from_min_size(
                            pos2(boxr.right() - 8.0 - text.size().x - 28.0, boxr.top() + if head > 0.0 { 4.0 } else { 8.0 }),
                            vec2(text.size().x + 28.0, height),
                        );
                        let hovered_pill = ctx.pointer_hover_pos().is_some_and(|pos| pill.contains(pos));
                        if head == 0.0 || hovered_pill || copied.is_some() {
                            painter.rect_filled(pill, 6.0, p.panel.gamma_multiply(if head > 0.0 { 1.0 } else { shown }));
                            painter.rect_stroke(pill, 6.0, Stroke::new(1.0_f32, p.hairline.gamma_multiply(shown)), StrokeKind::Inside);
                        }
                        let icon = if copied.is_some() { Icon::Check } else { Icon::File };
                        icons::paint(
                            painter,
                            icon,
                            Rect::from_center_size(pos2(pill.left() + 14.0, pill.center().y), vec2(12.0, 12.0)),
                            color.gamma_multiply(shown),
                        );
                        painter.galley(pos2(pill.left() + 23.0, pill.center().y - text.size().y * 0.5), text, color.gamma_multiply(shown));
                    }
                }
            }
            (Kind::Rule, _) => {
                painter.hline(area.x_range(), area.top() + 0.5, Stroke::new(1.0_f32, p.hairline));
            }
            (Kind::Table(_), Slot::Table(table)) => {
                self.paint_table(painter, table, area, p);
                let found = self.leaf_matches(index);
                if !found.is_empty() {
                    let outer = Rect::from_min_size(area.min, vec2(table.width, table.rows.iter().map(|r| r.0).sum::<f32>() + 2.0));
                    let current = found.iter().any(|m| m.2);
                    painter.rect_stroke(outer, 8.0, Stroke::new(if current { 2.0_f32 } else { 1.5 }, find_color(current, input.dark).gamma_multiply(if current { 1.0 } else { 0.8 })), StrokeKind::Outside);
                }
            }
            (Kind::Image { alt, src, link }, _) => {
                let layout = self.image_layout(src, area.width());
                let line = Stroke::new(1.0_f32, theme::alpha(p.ink, 20));
                match layout {
                    ImageLayout::Ready { texture, size } => {
                        let rect = Rect::from_min_size(area.min, size);
                        let shape = egui::epaint::RectShape::filled(rect, 8.0, Color32::WHITE)
                            .with_texture(texture, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));
                        painter.add(shape);
                        painter.rect_stroke(rect, 8.0, line, StrokeKind::Inside);
                        if link.is_some() && ctx.pointer_hover_pos().is_some_and(|pos| rect.contains(pos)) {
                            ctx.set_cursor_icon(CursorIcon::PointingHand);
                        }
                    }
                    ImageLayout::Loading => {
                        let boxr = Rect::from_min_size(area.min, vec2(area.width(), 120.0));
                        painter.rect_filled(boxr, 8.0, well(p));
                        let pulse = 0.55 + 0.25 * ((now * 3.0).sin() as f32);
                        painter.text(boxr.center(), Align2::CENTER_CENTER, "Loading image\u{2026}", FontId::proportional(13.0), p.muted.gamma_multiply(pulse));
                        ctx.request_repaint_after(std::time::Duration::from_millis(60));
                    }
                    ImageLayout::Remote | ImageLayout::Failed(_) => {
                        let boxr = Rect::from_min_size(area.min, vec2(area.width(), 52.0));
                        painter.rect_filled(boxr, 8.0, well(p));
                        painter.rect_stroke(boxr, 8.0, line, StrokeKind::Inside);
                        icons::paint(painter, Icon::File, Rect::from_center_size(pos2(boxr.left() + 24.0, boxr.center().y), vec2(16.0, 16.0)), p.muted);
                        let title = if alt.is_empty() { src.clone() } else { alt.clone() };
                        let note = match &layout {
                            ImageLayout::Failed(reason) => reason.clone(),
                            _ => "Remote images are not loaded: Tidemap works offline".to_owned(),
                        };
                        painter.text(pos2(boxr.left() + 44.0, boxr.center().y - 8.0), Align2::LEFT_CENTER, title, FontId::new(13.0, theme::font_medium()), p.secondary);
                        painter.text(pos2(boxr.left() + 44.0, boxr.center().y + 9.0), Align2::LEFT_CENTER, note, FontId::proportional(11.5), p.muted);
                    }
                }
            }
            (Kind::Diagram { block, source }, _) => {
                self.paint_diagram(ctx, painter, leaf, area, input, *block, source, now);
            }
            _ => {}
        }
    }

    fn marker(&self, painter: &egui::Painter, leaf: &Leaf, area: Rect, galley: &Galley, p: Palette) {
        let Some(marker) = &leaf.marker else { return };
        let row = galley.rows.first().map_or(BODY_LINE, |row| row.rect().height());
        let y = area.top() + row * 0.5;
        let right = area.left() - 8.0;
        match marker {
            Marker::Bullet(depth) => {
                let center = pos2(right - 3.0, y);
                match depth {
                    0 => {
                        painter.circle_filled(center, 2.6, p.secondary);
                    }
                    1 => {
                        painter.circle_stroke(center, 2.4, Stroke::new(1.2_f32, p.secondary));
                    }
                    _ => {
                        painter.rect_filled(Rect::from_center_size(center, vec2(5.0, 5.0)), 1.0, p.secondary);
                    }
                }
            }
            Marker::Number(text) => {
                painter.text(pos2(right, y - 1.0), Align2::RIGHT_CENTER, text, FontId::new(BODY - 1.0, FontFamily::Proportional), p.secondary);
            }
            Marker::Task(done) => {
                let boxr = Rect::from_center_size(pos2(right - 6.0, y), vec2(14.0, 14.0));
                if *done {
                    painter.rect_filled(boxr, 4.0, p.accent);
                    icons::paint(painter, Icon::Check, boxr.shrink(1.0), p.on_accent);
                } else {
                    painter.rect_stroke(boxr, 4.0, Stroke::new(1.3_f32, p.muted), StrokeKind::Inside);
                }
            }
        }
    }

    fn paint_table(&self, painter: &egui::Painter, table: &TableLayout, area: Rect, p: Palette) {
        let outer = Rect::from_min_size(area.min, vec2(table.width, table.rows.iter().map(|r| r.0).sum::<f32>() + 2.0));
        painter.rect_stroke(outer, 8.0, Stroke::new(1.0_f32, p.hairline), StrokeKind::Inside);
        let mut y = area.top() + 1.0;
        for (row_index, (height, cells)) in table.rows.iter().enumerate() {
            if row_index == 0 {
                painter.rect_filled(
                    Rect::from_min_size(pos2(area.left() + 1.0, y), vec2(table.width - 2.0, *height)),
                    egui::CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 },
                    well(p),
                );
            }
            if row_index > 0 {
                painter.hline(area.left()..=area.left() + table.width, y, Stroke::new(1.0_f32, p.hairline));
            }
            let mut x = area.left();
            for (column, galley) in cells.iter().enumerate() {
                let cell_w = table.widths[column];
                let align = table.align.get(column).copied().unwrap_or(Alignment::None);
                let offset = match align {
                    Alignment::Right => cell_w - 2.0 * CELL_PAD_X - galley.size().x,
                    Alignment::Center => (cell_w - 2.0 * CELL_PAD_X - galley.size().x) * 0.5,
                    _ => 0.0,
                };
                painter.galley(pos2(x + CELL_PAD_X + offset.max(0.0), y + CELL_PAD_Y), galley.clone(), p.ink);
                x += cell_w;
            }
            y += *height;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_diagram(
        &mut self,
        ctx: &Context,
        painter: &egui::Painter,
        leaf: &Leaf,
        area: Rect,
        input: &Input,
        block: Option<usize>,
        source: &str,
        now: f64,
    ) {
        let p = input.p;
        let active = block.and_then(|i| input.blocks.get(i)).is_some_and(|b| b.id == input.active_block && b.id == self.chosen);
        let colors = input.style.colors(input.dark);
        let canvas = Color32::from_rgba_unmultiplied(colors.canvas[0], colors.canvas[1], colors.canvas[2], colors.canvas[3]);
        let boxr = area;
        let hover = ctx.pointer_hover_pos().is_some_and(|pos| boxr.contains(pos) && painter.clip_rect().contains(pos));
        let lift = motion::hover(ctx, Id::new(("diagram-hover", leaf.hash)), hover);
        painter.rect_filled(boxr, 10.0, p.panel);
        painter.rect_filled(boxr, 10.0, canvas);
        let ring = if active { p.accent } else { theme::alpha(p.ink, 20 + (lift * 30.0) as u8) };
        painter.rect_stroke(boxr, 10.0, Stroke::new(if active { 1.5_f32 } else { 1.0 }, ring), StrokeKind::Inside);
        let inner = boxr.shrink(DIAGRAM_PAD);
        let Some(artifact) = self.artifact(leaf, input) else {
            // Not drawn yet: a quiet pulse, not a spinner.
            let pulse = 0.55 + 0.25 * ((now * 3.0).sin() as f32);
            painter.text(boxr.center(), Align2::CENTER_CENTER, "Drawing the diagram\u{2026}", FontId::proportional(13.0), p.muted.gamma_multiply(pulse));
            ctx.request_repaint_after(std::time::Duration::from_millis(60));
            return;
        };
        match diagram_fit(&artifact, inner.width()) {
            Fit::Scene { zoom, height, clipped } => {
                let scene = &artifact.rendered.scene;
                let b = scene.bounds;
                let camera = Camera {
                    zoom,
                    x: (inner.width() - b.w * zoom) * 0.5 - b.x * zoom,
                    y: -b.y * zoom,
                };
                let view = Rect::from_min_size(inner.min, vec2(inner.width(), (height - 2.0 * DIAGRAM_PAD).max(1.0)));
                let clip = painter.clip_rect().intersect(view);
                let sub = painter.with_clip_rect(clip);
                paint::paint_scene(
                    &sub,
                    view,
                    camera,
                    scene,
                    input.style,
                    input.dark,
                    p.accent,
                    &paint::Emphasis { selected: None, strong_nodes: &[], strong_edges: &[], matches: &[], dim: 0.0 },
                    &mut self.scratch,
                );
                if clipped {
                    let fade = Rect::from_min_max(pos2(boxr.left() + 1.0, boxr.bottom() - 44.0), pos2(boxr.right() - 1.0, boxr.bottom() - 1.0));
                    painter.rect_filled(fade, 9.0, canvas.gamma_multiply(0.0));
                    painter.text(pos2(boxr.center().x, boxr.bottom() - 16.0), Align2::CENTER_CENTER, "Double-click to see the whole diagram", FontId::proportional(12.0), p.muted);
                }
                let problems = artifact.rendered.diagnostics.len();
                if problems > 0 {
                    let label = format!("{problems} problem{}", if problems == 1 { "" } else { "s" });
                    painter.text(pos2(boxr.right() - 12.0, boxr.top() + 10.0), Align2::RIGHT_TOP, label, FontId::proportional(12.0), p.danger);
                }
            }
            Fit::Notice => {
                let title = unsupported_title(artifact.rendered.family);
                painter.text(pos2(inner.left(), inner.top()), Align2::LEFT_TOP, title, FontId::new(13.0, theme::font_medium()), p.secondary);
                let job = plain_job(source.trim_end(), FontId::monospace(CODE_SIZE - 0.5), p.muted, CODE_LINE - 2.0, inner.width());
                let galley = ctx.fonts_mut(|fonts| fonts.layout_job(job));
                painter.galley(pos2(inner.left(), inner.top() + 26.0), galley, p.muted);
            }
        }
        if lift > 0.0 {
            painter.text(
                pos2(boxr.right() - 12.0, boxr.bottom() - 10.0),
                Align2::RIGHT_BOTTOM,
                "Double-click to open",
                FontId::proportional(11.5),
                p.muted.gamma_multiply(lift),
            );
        }
    }

    fn scrollbar(&mut self, ctx: &Context, ui: &Ui, painter: &egui::Painter, rect: Rect, view_h: f32, max_scroll: f32, p: Palette, now: f64) {
        if max_scroll <= 1.0 {
            return;
        }
        let total = self.total();
        let track = Rect::from_min_max(pos2(rect.right() - 14.0, rect.top() + 4.0), pos2(rect.right() - 2.0, rect.bottom() - 4.0));
        let thumb_h = (track.height() * view_h / total).clamp(32.0, track.height());
        let travel = track.height() - thumb_h;
        let thumb_y = track.top() + travel * (self.scroll_y / max_scroll);
        let thumb = Rect::from_min_size(pos2(track.left(), thumb_y), vec2(track.width(), thumb_h));
        let bar = ui.interact(track, Id::new("document-bar"), Sense::click_and_drag());
        let near = ctx.pointer_hover_pos().is_some_and(|pos| pos.x > rect.right() - 36.0 && rect.contains(pos));
        let active = bar.dragged() || near || now < self.bar_alpha_until;
        let show = motion::toggle(ctx, Id::new("document-bar-show"), active, motion::BASE);
        if now < self.bar_alpha_until {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if bar.drag_started()
            && let Some(pos) = bar.interact_pointer_pos()
        {
            let grab = if thumb.contains(pos) { pos.y - thumb_y } else { thumb_h * 0.5 };
            self.drag = Drag::Bar(grab);
        }
        if let Drag::Bar(grab) = self.drag {
            if bar.dragged() && let Some(pos) = bar.interact_pointer_pos() {
                let fraction = ((pos.y - grab - track.top()) / travel.max(1.0)).clamp(0.0, 1.0);
                self.scroll_y = fraction * max_scroll;
                self.target_y = self.scroll_y;
            } else if !bar.dragged() {
                self.drag = Drag::None;
            }
        } else if bar.clicked()
            && let Some(pos) = bar.interact_pointer_pos()
            && !thumb.contains(pos)
        {
            self.target_y = (self.scroll_y + if pos.y < thumb_y { -view_h * 0.88 } else { view_h * 0.88 }).clamp(0.0, max_scroll);
        }
        if show > 0.01 {
            let width = 4.0 + 3.0 * show;
            let drawn = Rect::from_min_size(pos2(track.right() - width, thumb_y), vec2(width, thumb_h));
            painter.rect_filled(drawn, width * 0.5, theme::alpha(p.ink, (60.0 + 50.0 * show) as u8).gamma_multiply(show));
        }
    }

    // -------------------------------------------------------------------------------------
    // Mouse and selection
    // -------------------------------------------------------------------------------------

    fn leaf_at(&self, doc: &Doc, y: f32, origin_y: f32) -> Option<usize> {
        if doc.leaves.is_empty() {
            return None;
        }
        let local = y - origin_y;
        let index = self.tops.partition_point(|top| *top <= local).saturating_sub(1);
        Some(index.min(doc.leaves.len() - 1))
    }

    /// Char index in the leaf's text under `pos`, when the leaf is text.
    fn char_at(&mut self, ctx: &Context, doc: &Doc, index: usize, pos: Pos2, area_x: f32, top: f32, width: f32, p: Palette) -> Option<usize> {
        let leaf = &doc.leaves[index];
        let slot = self.slot(ctx, doc, index, width, p);
        match (&leaf.kind, &*slot) {
            (Kind::Text { role, .. }, Slot::Text(galley)) => {
                let area = Rect::from_min_size(pos2(area_x, top), vec2(Self::inner_width(leaf, width), 1.0));
                let at = text_origin(*role, galley, area);
                Some(galley.cursor_from_pos(pos - at).index)
            }
            (_, Slot::Code(galley)) => {
                let scroll = self.hscroll.get(&leaf.hash).copied().unwrap_or(0.0);
                let lang = if let Kind::Code { lang, .. } = &leaf.kind { lang.as_str() } else { "" };
                let at = pos2(area_x + CODE_PAD - scroll, top + code_top(lang));
                Some(galley.cursor_from_pos(pos - at).index)
            }
            _ => None,
        }
    }

    fn link_at(&self, doc: &Doc, index: usize, ch: usize) -> Option<u32> {
        let Kind::Text { runs, .. } = &doc.leaves[index].kind else { return None };
        let mut start = 0;
        for run in runs {
            let len = run.text.chars().count();
            if ch >= start && ch < start + len {
                return run.link;
            }
            start += len;
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn mouse(
        &mut self,
        ctx: &Context,
        doc: &Doc,
        input: &Input,
        response: &egui::Response,
        rect: Rect,
        col_x: f32,
        width: f32,
        origin_y: f32,
        actions: &mut Vec<Action>,
        now: f64,
        id: Id,
    ) {
        let p = input.p;
        let (pressed, down, released, pointer, modifiers) = ctx.input(|i| {
            (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.primary_released(), i.pointer.latest_pos(), i.modifiers)
        });
        let in_bar = pointer.is_some_and(|pos| pos.x > rect.right() - 16.0);
        self.hover_link = None;
        self.hover_url = None;
        let area_of = |this: &Self, leaf: &Leaf, index: usize| -> (f32, f32) {
            (
                col_x + leaf.quote as f32 * QUOTE_STEP + leaf.list as f32 * LIST_STEP,
                origin_y + this.tops[index] + leaf.gap,
            )
        };
        // Hover: links and diagrams show a hand.
        if response.hovered() && !in_bar && let Some(pos) = pointer && let Some(index) = self.leaf_at(doc, pos.y, origin_y) {
            let leaf = &doc.leaves[index];
            let (x, top) = area_of(self, leaf, index);
            match &leaf.kind {
                Kind::Diagram { .. } => ctx.set_cursor_icon(CursorIcon::PointingHand),
                Kind::Text { .. } => {
                    if let Some(ch) = self.char_at(ctx, doc, index, pos, x, top, width, p) {
                        let link = self.link_at(doc, index, ch);
                        ctx.set_cursor_icon(if link.is_some() { CursorIcon::PointingHand } else { CursorIcon::Text });
                        if let Some(link) = link {
                            self.hover_link = Some((index, link));
                            self.hover_url = doc.links.get(link as usize).cloned();
                        }
                    }
                }
                Kind::Code { .. } => ctx.set_cursor_icon(CursorIcon::Text),
                _ => {}
            }
        }
        if pressed && response.hovered() && !in_bar && let Some(pos) = pointer {
            ctx.memory_mut(|m| m.request_focus(id));
            let count = match self.last_click {
                Some((time, at, count)) if now - time < 0.45 && (at - pos).length() < 5.0 => (count % 3) + 1,
                _ => 1,
            };
            self.last_click = Some((now, pos, count));
            self.pending_link = None;
            if let Some(index) = self.leaf_at(doc, pos.y, origin_y) {
                let leaf = &doc.leaves[index];
                let (x, top) = area_of(self, leaf, index);
                if modifiers.alt {
                    actions.push(Action::EditLine(leaf.line));
                    return;
                }
                if matches!(leaf.kind, Kind::Code { .. })
                    && pos.x > x + Self::inner_width(leaf, width) - 100.0
                    && pos.y < top + if matches!(&leaf.kind, Kind::Code { lang, .. } if !lang.is_empty()) { CODE_HEAD } else { 36.0 }
                {
                    ctx.copy_text(leaf.plain.clone());
                    self.copied = Some((leaf.hash, now));
                    return;
                }
                match &leaf.kind {
                    Kind::Diagram { block, .. } => {
                        if let Some(block) = block.and_then(|b| input.blocks.get(b)) {
                            if count >= 2 {
                                actions.push(Action::OpenDiagram(block.id.clone()));
                            } else {
                                self.last_active = block.id.clone();
                                self.chosen = block.id.clone();
                                actions.push(Action::SelectDiagram(block.id.clone()));
                            }
                        }
                        self.sel = None;
                    }
                    Kind::Image { link, .. } => {
                        if let Some(link) = link {
                            self.pending_link = Some((*link, pos));
                        }
                        self.sel = None;
                    }
                    _ => {
                        if let Some(ch) = self.char_at(ctx, doc, index, pos, x, top, width, p) {
                            let here = Pos { leaf: index, ch };
                            if let Some(link) = self.link_at(doc, index, ch) {
                                self.pending_link = Some((link, pos));
                            }
                            match count {
                                1 => {
                                    self.sel = Some(if modifiers.shift {
                                        Selection { anchor: self.sel.map_or(here, |s| s.anchor), head: here }
                                    } else {
                                        Selection { anchor: here, head: here }
                                    });
                                    self.drag = Drag::Text(1);
                                    self.anchor_word = None;
                                }
                                2 => {
                                    let text = &leaf.plain;
                                    let byte = text.char_indices().nth(ch).map_or(text.len(), |(b, _)| b);
                                    let (a, b) = crate::editor::word_range(text, byte);
                                    let (a, b) = (text[..a].chars().count(), text[..b].chars().count());
                                    let (lo, hi) = (Pos { leaf: index, ch: a }, Pos { leaf: index, ch: b });
                                    self.sel = Some(Selection { anchor: lo, head: hi });
                                    self.anchor_word = Some((lo, hi));
                                    self.drag = Drag::Text(2);
                                }
                                _ => {
                                    let (lo, hi) = (Pos { leaf: index, ch: 0 }, Pos { leaf: index, ch: leaf.plain.chars().count() });
                                    self.sel = Some(Selection { anchor: lo, head: hi });
                                    self.anchor_word = Some((lo, hi));
                                    self.drag = Drag::Text(3);
                                }
                            }
                        } else {
                            self.sel = None;
                        }
                    }
                }
            }
        } else if let Drag::Text(mode) = self.drag {
            if down && let Some(pos) = pointer {
                let pos = pos2(pos.x, pos.y.clamp(rect.top() + 1.0, rect.bottom() - 1.0));
                if let Some(index) = self.leaf_at(doc, pos.y, origin_y) {
                    let leaf = &doc.leaves[index];
                    let (x, top) = area_of(self, leaf, index);
                    let ch = self.char_at(ctx, doc, index, pos, x, top, width, p).unwrap_or(if pos.y < origin_y + self.tops[index] { 0 } else { leaf.plain.chars().count() });
                    let here = Pos { leaf: index, ch };
                    if let Some(sel) = &mut self.sel {
                        match (mode, self.anchor_word) {
                            (1, _) | (_, None) => sel.head = here,
                            (_, Some((lo, hi))) => {
                                if here < lo {
                                    *sel = Selection { anchor: hi, head: here };
                                } else {
                                    *sel = Selection { anchor: lo, head: here.max(hi) };
                                }
                            }
                        }
                        if (pos.y - rect.top()).abs() < 2.0 || (rect.bottom() - pos.y).abs() < 2.0 {
                            self.target_y += if pos.y < rect.center().y { -30.0 } else { 30.0 };
                            ctx.request_repaint();
                        }
                    }
                }
                if pointer.is_some_and(|pos| pos.y < rect.top() + 8.0 || pos.y > rect.bottom() - 8.0) {
                    self.target_y += if pointer.is_some_and(|pos| pos.y < rect.center().y) { -24.0 } else { 24.0 };
                    ctx.request_repaint();
                }
            } else if !down {
                self.drag = Drag::None;
            }
        }
        if released {
            if let Some((link, at)) = self.pending_link.take()
                && pointer.is_some_and(|pos| (pos - at).length() < 5.0)
                && self.sel.is_none_or(|s| s.anchor == s.head)
                && let Some(url) = doc.links.get(link as usize)
            {
                self.open_link(doc, url, actions);
            }
            if matches!(self.drag, Drag::Text(_)) {
                self.drag = Drag::None;
            }
        }
    }

    fn open_link(&mut self, doc: &Doc, url: &str, actions: &mut Vec<Action>) {
        if let Some(anchor) = url.strip_prefix('#') {
            if let Some(leaf) = doc.anchors.get(&anchor.to_lowercase()) {
                self.target_y = (self.tops[*leaf] - 16.0).max(0.0);
                self.bar_alpha_until = 0.0;
            }
        } else if url.contains("://") || url.starts_with("mailto:") {
            actions.push(Action::OpenUrl(url.to_owned()));
        } else {
            actions.push(Action::OpenPath(url.to_owned()));
        }
    }

    fn selection_in(&self, doc: &Doc, index: usize) -> Option<(usize, usize)> {
        let sel = self.sel?;
        let (lo, hi) = if sel.anchor <= sel.head { (sel.anchor, sel.head) } else { (sel.head, sel.anchor) };
        if index < lo.leaf || index > hi.leaf || lo == hi {
            return None;
        }
        let total = doc.leaves[index].plain.chars().count();
        let from = if index == lo.leaf { lo.ch } else { 0 };
        let to = if index == hi.leaf { hi.ch } else { total };
        (from < to || (index != hi.leaf && total == 0)).then_some((from, to.max(from)))
    }

    fn selected_text(&self, doc: &Doc) -> Option<String> {
        let sel = self.sel?;
        let (lo, hi) = if sel.anchor <= sel.head { (sel.anchor, sel.head) } else { (sel.head, sel.anchor) };
        if lo == hi {
            return None;
        }
        let mut out = String::new();
        for index in lo.leaf..=hi.leaf.min(doc.leaves.len().saturating_sub(1)) {
            let leaf = &doc.leaves[index];
            let total = leaf.plain.chars().count();
            let from = if index == lo.leaf { lo.ch } else { 0 };
            let to = if index == hi.leaf { hi.ch.min(total) } else { total };
            if index > lo.leaf {
                let tight = leaf.list > 0 && doc.leaves[index - 1].list > 0;
                out.push_str(if tight { "\n" } else { "\n\n" });
            }
            out.extend(leaf.plain.chars().skip(from).take(to.saturating_sub(from)));
        }
        Some(out)
    }
}

/// Where a text galley is drawn inside its area: display formulas are centred.
fn text_origin(role: Role, galley: &Galley, area: Rect) -> Pos2 {
    if matches!(role, Role::Display) {
        pos2(area.min.x + ((area.width() - galley.size().x) * 0.5).max(0.0), area.min.y)
    } else {
        area.min
    }
}

/// Fill of a find match; the current one is stronger.
fn find_color(current: bool, dark: bool) -> Color32 {
    match (current, dark) {
        (true, false) => Color32::from_rgba_unmultiplied(255, 140, 30, 150),
        (true, true) => Color32::from_rgba_unmultiplied(255, 160, 50, 150),
        (false, false) => Color32::from_rgba_unmultiplied(255, 205, 50, 95),
        (false, true) => Color32::from_rgba_unmultiplied(255, 205, 70, 70),
    }
}

fn selection_color(p: Palette, dark: bool) -> Color32 {
    theme::alpha(p.accent, if dark { 95 } else { 70 })
}

/// For each row the char range `from..to` touches: the row's rectangle and the x range covered.
/// Positions are relative to the galley.
fn spans(galley: &Galley, from: usize, to: usize) -> Vec<(Rect, f32, f32)> {
    let mut out = Vec::new();
    let mut start = 0;
    for row in &galley.rows {
        let total = row.char_count_including_newline();
        let visible = row.char_count_excluding_newline();
        let (a, b) = (from.max(start), to.min(start + total));
        if a < b {
            let x0 = row.pos.x + row.x_offset(a - start);
            let mut x1 = row.pos.x + row.x_offset((b - start).min(visible));
            if b - start > visible {
                // The selection runs on past the line end: show the newline as a small nub.
                x1 += 5.0;
            }
            out.push((row.rect(), x0, x1));
        }
        start += total;
    }
    out
}

fn paint_selection(painter: &egui::Painter, galley: &Galley, at: Pos2, range: Option<(usize, usize)>, color: Color32) {
    let Some((from, to)) = range else { return };
    for (row, x0, x1) in spans(galley, from, to) {
        painter.rect_filled(
            Rect::from_min_max(pos2(at.x + x0, at.y + row.min.y), pos2(at.x + x1.max(x0 + 2.0), at.y + row.max.y)),
            3.0,
            color,
        );
    }
}

/// Char ranges of the inline code runs.
fn code_ranges(runs: &[Run]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut at = 0;
    for run in runs {
        let len = run.text.chars().count();
        if run.style & CODE != 0 && len > 0 {
            match out.last_mut() {
                Some(last) if last.1 == at => last.1 = at + len,
                _ => out.push((at, at + len)),
            }
        }
        at += len;
    }
    out
}

/// The char range of the link `index`, joining the runs that share it.
fn link_range(runs: &[Run], index: u32) -> Option<(usize, usize)> {
    let mut at = 0;
    let mut found: Option<(usize, usize)> = None;
    for run in runs {
        let len = run.text.chars().count();
        if run.link == Some(index) {
            found = Some(found.map_or((at, at + len), |(from, _)| (from, at + len)));
        }
        at += len;
    }
    found
}

const IMAGE_MAX: f32 = 720.0;

enum ImageLayout {
    Remote,
    Loading,
    Failed(String),
    Ready { texture: egui::TextureId, size: egui::Vec2 },
}

impl ImageLayout {
    fn height(&self) -> f32 {
        match self {
            Self::Remote | Self::Failed(_) => 52.0,
            Self::Loading => 120.0,
            Self::Ready { size, .. } => size.y,
        }
    }
}

enum Fit {
    Scene { zoom: f32, height: f32, clipped: bool },
    Notice,
}

fn unsupported_title(family: Family) -> &'static str {
    match family {
        Family::Flowchart => "Flowchart",
        Family::Sequence => "Sequence diagrams are not drawn yet",
        Family::State => "State diagrams are not drawn yet",
        Family::Class => "Class diagrams are not drawn yet",
        Family::Er => "ER diagrams are not drawn yet",
        Family::Other => "This diagram type is not drawn yet",
    }
}

fn notice_height(ctx: &Context, leaf: &Leaf, inner: f32, _p: Palette) -> f32 {
    let Kind::Diagram { source, .. } = &leaf.kind else { return DIAGRAM_PLACEHOLDER };
    let job = plain_job(source.trim_end(), FontId::monospace(CODE_SIZE - 0.5), Color32::WHITE, CODE_LINE - 2.0, inner - 2.0 * DIAGRAM_PAD);
    let galley = ctx.fonts_mut(|fonts| fonts.layout_job(job));
    galley.size().y + 26.0 + 2.0 * DIAGRAM_PAD
}

/// How a laid-out diagram fits a column of `inner_box` points: scale and box height.
fn diagram_fit(artifact: &RenderArtifact, inner_box: f32) -> Fit {
    let scene = &artifact.rendered.scene;
    if artifact.rendered.family != Family::Flowchart || scene.nodes.is_empty() {
        return Fit::Notice;
    }
    let b = scene.bounds;
    let avail = (inner_box - 2.0 * DIAGRAM_PAD).max(40.0);
    let by_width = avail / b.w.max(1.0);
    let mut zoom = by_width.min(1.35);
    // A tall diagram may shrink a little more to fit, but stays readable; beyond that it is clipped.
    let max_h = DIAGRAM_MAX;
    if b.h * zoom > max_h {
        zoom = (max_h / b.h.max(1.0)).max(0.55f32.min(by_width)).min(zoom);
    }
    let full = b.h * zoom;
    let clipped = full > max_h;
    let shown = full.min(max_h);
    Fit::Scene { zoom, height: shown + 2.0 * DIAGRAM_PAD + if clipped { 18.0 } else { 0.0 }, clipped }
}

fn layout_table(ctx: &Context, table: &Table, width: f32, p: Palette) -> TableLayout {
    let columns = table.head.len().max(table.rows.iter().map(Vec::len).max().unwrap_or(0)).max(1);
    let style = TextStyle { size: BODY - 1.0, line: BODY_LINE - 2.0, color: p.ink, heading: false };
    let mut natural = vec![0.0f32; columns];
    let all_rows: Vec<(&Vec<Vec<Run>>, bool)> =
        std::iter::once((&table.head, true)).chain(table.rows.iter().map(|r| (r, false))).collect();
    for (cells, head) in &all_rows {
        for (column, runs) in cells.iter().enumerate().take(columns) {
            let mut job = job_of(runs, &style, p, f32::INFINITY, *head);
            job.wrap.max_width = f32::INFINITY;
            let galley = ctx.fonts_mut(|fonts| fonts.layout_job(job));
            natural[column] = natural[column].max(galley.size().x + 2.0 * CELL_PAD_X);
        }
    }
    let total: f32 = natural.iter().sum();
    let mut widths = natural.clone();
    if total > width {
        // Shrink the wide columns first: every column keeps up to 120 points before any is cut.
        let floor = 96.0f32.min(width / columns as f32);
        let mut excess = total - width;
        for _ in 0..4 {
            let flexible: f32 = widths.iter().map(|w| (w - floor).max(0.0)).sum();
            if flexible <= 0.0 || excess <= 0.0 {
                break;
            }
            let share = (excess / flexible).min(1.0);
            for w in &mut widths {
                let give = (*w - floor).max(0.0) * share;
                *w -= give;
                excess -= give;
            }
        }
    }
    let table_width: f32 = widths.iter().sum();
    let mut rows = Vec::new();
    for (cells, head) in &all_rows {
        let mut galleys = Vec::new();
        let mut height = 0.0f32;
        for column in 0..columns {
            let empty = Vec::new();
            let runs = cells.get(column).unwrap_or(&empty);
            let job = job_of(runs, &style, p, widths[column] - 2.0 * CELL_PAD_X, *head);
            let galley = ctx.fonts_mut(|fonts| fonts.layout_job(job));
            height = height.max(galley.size().y);
            galleys.push(galley);
        }
        rows.push((height + 2.0 * CELL_PAD_Y, galleys));
    }
    TableLayout { widths, rows, align: table.align.clone(), width: table_width }
}

#[allow(dead_code)]
fn unused(_: &parse::Doc) {}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, RawInput};
    use std::time::Instant;

    fn big_doc(sections: usize) -> String {
        let mut md = String::new();
        for i in 0..sections {
            md.push_str(&format!(
                "## Section {i}\n\nSome **bold** text with `code`, a [link](#section-0) and more words to wrap around the line a few times over, long enough to need two or three rows at this width.\n\n- item one\n- item two\n  - nested\n\n```mermaid\ngraph TD\n  A{i}-->B\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n> quoted\n\n"
            ));
        }
        md
    }

    fn rig(content: &str) -> (Context, Engine, Vec<DiagramBlock>) {
        let ctx = Context::default();
        let mut fonts = egui::FontDefinitions::default();
        let base = fonts.families[&FontFamily::Proportional].clone();
        fonts.families.insert(theme::font_medium(), base.clone());
        fonts.families.insert(theme::font_semibold(), base);
        ctx.set_fonts(fonts);
        let font: Arc<[u8]> = Arc::from(&include_bytes!("../../assets/fonts/Inter-Medium.ttf")[..]);
        let engine = Engine::new(Arc::new(|| {}), font, StyleSheet::default());
        let blocks = tidemap_core::source::extract_diagrams(content, "t.md", &[]);
        (ctx, engine, blocks)
    }

    #[test]
    fn scrolls_a_huge_page_without_stalling() {
        let content = big_doc(7000);
        let (ctx, mut engine, blocks) = rig(&content);
        let mut view = DocView::default();
        let style = StyleSheet::default();
        let mut next_job = 1;
        let mut time = 0.0;
        let mut run = |view: &mut DocView, events: Vec<Event>, engine: &mut Engine, next_job: &mut u64| -> f64 {
            time += 0.016;
            let mut raw = RawInput::default();
            raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0)));
            raw.time = Some(time);
            raw.events = events;
            let started = Instant::now();
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                    let input = Input {
                        file_id: "f",
                        content: &content,
                        dir: None,
                        revision: 1,
                        blocks: &blocks,
                        active_block: "",
                        p: theme::palette(false),
                        dark: false,
                        style: &style,
                        saved_scroll: 0.0,
                    };
                    view.show(ui, &input, engine, next_job);
                });
            });
            started.elapsed().as_secs_f64() * 1000.0
        };
        let first = run(&mut view, vec![], &mut engine, &mut next_job);
        eprintln!("first frame on {} leaves: {first:.1} ms", view.doc.leaves.len());
        assert!(view.doc.leaves.len() > 40_000);
        let mut worst = 0.0f64;
        let at = Pos2::new(500.0, 400.0);
        run(&mut view, vec![Event::PointerMoved(at)], &mut engine, &mut next_job);
        for _ in 0..120 {
            let ms = run(
                &mut view,
                vec![Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: vec2(0.0, -40.0), modifiers: egui::Modifiers::NONE }],
                &mut engine,
                &mut next_job,
            );
            worst = worst.max(ms);
        }
        eprintln!("scrolled to {:.0} of {:.0}; worst frame {worst:.1} ms", view.scroll_y, view.total());
        assert!(view.scroll_y > 1000.0, "wheel scrolls the page");
        assert!(worst < 60.0, "a scroll frame took {worst:.1} ms");
    }

    #[test]
    fn find_on_the_page_jumps_to_a_match() {
        let content = big_doc(400);
        let (ctx, mut engine, blocks) = rig(&content);
        let mut view = DocView::default();
        let style = StyleSheet::default();
        let mut next_job = 1;
        let mut time = 0.0;
        let mut frame = |view: &mut DocView, engine: &mut Engine, next_job: &mut u64| {
            time += 0.016;
            let mut raw = RawInput::default();
            raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0)));
            raw.time = Some(time);
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                    let input = Input {
                        file_id: "f",
                        content: &content,
                        dir: None,
                        revision: 1,
                        blocks: &blocks,
                        active_block: "",
                        p: theme::palette(false),
                        dark: false,
                        style: &style,
                        saved_scroll: 0.0,
                    };
                    view.show(ui, &input, engine, next_job);
                });
            });
        };
        frame(&mut view, &mut engine, &mut next_job);
        view.open_find();
        view.find.query = "Section 300".to_owned();
        view.find.restart = true;
        for _ in 0..90 {
            frame(&mut view, &mut engine, &mut next_job);
        }
        assert_eq!(view.find.matches.len(), 1, "one heading says Section 300");
        let (leaf, _, _) = view.find.matches[0];
        let top = view.tops[leaf];
        assert!(view.scroll_y > 1000.0, "the page scrolled to the match");
        assert!(top >= view.scroll_y && top <= view.scroll_y + 800.0, "the match is on screen: top {top}, scroll {}", view.scroll_y);
    }
}
