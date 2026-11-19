//! The source editor: a virtualized code view with its own caret, selection and undo history.
//!
//! egui's `TextEdit` lays the whole text out on every frame, which is what made typing,
//! selecting and moving through a big file slow. This editor keeps a line index
//! (`model::LineIndex`) and lays out only the lines on screen, each cached by content, so a
//! frame costs the same for a 10-line and a 1,000,000-line file. All offsets inside are UTF-8
//! bytes; the app sees `char` indices through `EditorResponse::cursor` and `select_range`.

mod model;
mod syntax;
#[cfg(test)]
mod tests;

use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::Arc,
    time::Duration,
};

use egui::{
    Context, CursorIcon, Event, EventFilter, FontId, Galley, Id, Key, Modifiers, Pos2, Rect, RichText, Sense, Ui,
    text::CCursor, vec2,
};

use model::{
    Change, History, Kind, LineIndex, Selection, byte_of_char, chars_before, indent_of, next_grapheme, normalize,
    prev_grapheme, word_at, word_left, word_right,
};
use syntax::State;

use crate::theme::{self, Palette, token};

/// Space between the line numbers and the text.
const TEXT_PAD: f32 = 10.0;
/// Space around the line numbers.
const GUTTER_PAD: f32 = 12.0;
const INDENT: &str = "    ";
const BLINK: f64 = 0.53;

pub struct EditorResponse {
    pub changed: bool,
    pub cursor: usize,
    pub scroll: f32,
    pub focused: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum DragMode {
    Char,
    Word,
    Line,
}

struct Drag {
    mode: DragMode,
    /// The word or line that was pressed; a drag always keeps it selected.
    origin: (usize, usize),
}

#[derive(Clone, Copy)]
struct Reveal {
    center: bool,
    animate: bool,
}

/// Everything a frame needs to lay out and paint lines.
struct Env {
    markdown: bool,
    p: Palette,
    font: FontId,
    query: String,
    line_h: f32,
    glyph_w: f32,
}

#[derive(Default)]
pub struct SourceEditor {
    pub find_open: bool,
    pub find: String,
    find_focus: bool,
    jump_line: Option<usize>,
    select: Option<(usize, usize)>,

    loaded: Option<String>,
    text_len: usize,
    markdown: bool,
    lines: LineIndex,
    history: History,
    sel: Selection,
    /// Pixel column kept while moving up and down.
    want_x: Option<f32>,
    /// `states[i]` is the syntax state at the start of line `i`; valid up to `states.len()`.
    states: Vec<State>,
    longest: usize,
    longest_dirty: bool,
    revision: u64,
    changed: bool,

    galleys: HashMap<u64, (Arc<Galley>, u64)>,
    /// The font-atlas epoch `galleys` were laid out under.
    galley_epoch: u64,
    frame: u64,
    reveal: Option<Reveal>,
    drag: Option<Drag>,
    last_press: Option<(f64, Pos2, u8)>,
    blink_from: f64,
    view_lines: usize,
    chars_cache: (usize, u64, usize),
    matches: Option<(u64, String, usize)>,
}

impl SourceEditor {
    /// Opens the find bar and puts the caret into it.
    pub fn find(&mut self) {
        self.find_open = true;
        self.find_focus = true;
    }

    pub fn jump_to_line(&mut self, line: usize) {
        self.jump_line = Some(line.max(1));
    }

    /// Selects the char range `chars` and scrolls it into view on the next `show`.
    pub fn select_range(&mut self, chars: (usize, usize)) {
        self.select = Some(chars);
    }

    /// Forgets the loaded text; the next `show` reads the buffer afresh. Call it whenever the
    /// text behind the editor was replaced.
    pub fn reset(&mut self) {
        self.loaded = None;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut Ui,
        key: &str,
        source: &mut String,
        markdown: bool,
        read_only: bool,
        p: Palette,
        initial_cursor: usize,
        initial_scroll: f32,
    ) -> EditorResponse {
        self.frame += 1;
        self.changed = false;
        let epoch = crate::text::epoch(ui.ctx());
        if epoch != self.galley_epoch {
            self.galleys.clear();
            self.galley_epoch = epoch;
        }
        let first_open = self.loaded.as_deref() != Some(key);
        if first_open {
            self.loaded = Some(key.to_owned());
            self.markdown = markdown;
            self.rebuild(source);
            self.history.clear();
            self.sel = Selection::caret(byte_of_char(source, initial_cursor));
            self.want_x = None;
            self.drag = None;
            self.reveal = None;
        } else {
            if self.text_len != source.len() {
                self.rebuild(source);
                self.sel = Selection { anchor: self.sel.anchor.min(source.len()), head: self.sel.head.min(source.len()) };
            }
            if self.markdown != markdown {
                self.markdown = markdown;
                self.states.clear();
            }
        }
        let font = FontId::monospace(token::FONT_MONO);
        let (row_h, glyph_w) = ui.fonts_mut(|fonts| (fonts.row_height(&font), fonts.glyph_width(&font, '0')));
        let env_query = self.find.to_lowercase();
        let mut env = Env {
            markdown,
            p,
            font,
            query: if self.find_open { env_query } else { String::new() },
            line_h: (row_h + 5.0).round(),
            glyph_w,
        };

        let text_id = Id::new(("source", key));
        if self.find_open {
            self.find_bar(ui, source, &mut env, text_id);
        }

        if let Some(line) = self.jump_line.take() {
            let at = self.lines.start((line - 1).min(self.lines.len() - 1));
            self.sel = Selection::caret(at);
            self.reveal = Some(Reveal { center: true, animate: true });
            ui.memory_mut(|memory| memory.request_focus(text_id));
        }
        if let Some((start, end)) = self.select.take() {
            self.sel = Selection { anchor: byte_of_char(source, start), head: byte_of_char(source, end) };
            self.want_x = None;
            self.reveal = Some(Reveal { center: true, animate: true });
        }

        let mut area = egui::ScrollArea::both()
            .id_salt(("editor-scroll", key))
            .auto_shrink([false, false])
            .scroll_source(egui::containers::scroll_area::ScrollSource { drag: false, ..Default::default() });
        if first_open {
            area = area.vertical_scroll_offset(initial_scroll);
        }
        let view_w = ui.available_width();
        let output = area.show_viewport(ui, |ui, viewport| {
            self.surface(ui, viewport, source, &env, text_id, read_only, view_w);
        });
        self.evict();

        let focused = ui.memory(|memory| memory.has_focus(text_id));
        let cursor = self.cursor_char(source);
        EditorResponse { changed: self.changed, cursor, scroll: output.state.offset.y, focused }
    }

    fn rebuild(&mut self, text: &str) {
        self.lines = LineIndex::build(text);
        self.states.clear();
        self.text_len = text.len();
        self.longest = self.lines.longest(text);
        self.longest_dirty = false;
        self.revision += 1;
    }

    fn cursor_char(&mut self, text: &str) -> usize {
        let (head, revision, chars) = self.chars_cache;
        if head == self.sel.head && revision == self.revision {
            return chars;
        }
        let chars = chars_before(text, self.sel.head);
        self.chars_cache = (self.sel.head, self.revision, chars);
        chars
    }

    fn evict(&mut self) {
        if self.galleys.len() > 3000 {
            let keep = self.frame.saturating_sub(2);
            self.galleys.retain(|_, (_, used)| *used >= keep);
        }
    }

    // -----------------------------------------------------------------------------------
    // Find
    // -----------------------------------------------------------------------------------

    fn find_bar(&mut self, ui: &mut Ui, source: &str, env: &mut Env, editor_id: Id) {
        let p = env.p;
        let mut close = false;
        let mut step: Option<bool> = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Find").small().color(p.secondary));
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.find)
                    .id_salt("source-find")
                    .desired_width((ui.available_width() - 70.0).max(80.0))
                    .hint_text("Find in source\u{2026}"),
            );
            if std::mem::take(&mut self.find_focus) {
                field.request_focus();
            }
            if field.changed() {
                step = Some(true);
                // Typing keeps the current match when it still fits: search from its start.
                let (start, _) = self.sel.range();
                self.sel = Selection::caret(start);
            }
            if field.lost_focus() {
                let (enter, shift, escape) =
                    ui.input(|input| (input.key_pressed(Key::Enter), input.modifiers.shift, input.key_pressed(Key::Escape)));
                if enter {
                    step = Some(!shift);
                    field.request_focus();
                }
                if escape {
                    close = true;
                }
            }
            if crate::icons::button(ui, crate::icons::Icon::Close, "Close find", false, true).clicked() {
                close = true;
            }
        });
        if !self.find.is_empty() {
            let count = self.match_count(source);
            ui.label(
                RichText::new(format!("{count} matches · Enter jumps to the next, Shift+Enter to the previous"))
                    .small()
                    .color(p.muted),
            );
        }
        ui.add_space(token::SPACE[1]);
        env.query = self.find.to_lowercase();
        if let Some(forward) = step
            && !self.find.is_empty()
        {
            self.find_step(source, forward);
        }
        if close {
            self.find_open = false;
            ui.memory_mut(|memory| memory.request_focus(editor_id));
        }
    }

    fn match_count(&mut self, text: &str) -> usize {
        let query = self.find.to_lowercase();
        if let Some((revision, cached, count)) = &self.matches
            && *revision == self.revision
            && *cached == query
        {
            return *count;
        }
        let mut count = 0;
        for line in text.split('\n') {
            let mut rest = line;
            while let Some((_, end)) = syntax::find_fold(rest, &query) {
                count += 1;
                rest = &rest[end..];
            }
        }
        self.matches = Some((self.revision, query, count));
        count
    }

    /// Selects the next or previous match of the find text, wrapping around the text.
    fn find_step(&mut self, text: &str, forward: bool) {
        let query = self.find.to_lowercase();
        let (start, end) = self.sel.range();
        let found = if forward {
            self.find_forward(text, &query, end).or_else(|| self.find_forward(text, &query, 0))
        } else {
            self.find_backward(text, &query, start).or_else(|| self.find_backward(text, &query, text.len()))
        };
        if let Some((from, to)) = found {
            self.sel = Selection { anchor: from, head: to };
            self.want_x = None;
            self.reveal = Some(Reveal { center: true, animate: true });
        }
    }

    fn find_forward(&self, text: &str, query: &str, from: usize) -> Option<(usize, usize)> {
        let mut line = self.lines.line_of(from);
        let mut offset = from - self.lines.start(line);
        while line < self.lines.len() {
            let content = self.lines.line(text, line);
            if offset <= content.len()
                && let Some((a, b)) = syntax::find_fold(&content[offset..], query)
            {
                let base = self.lines.start(line) + offset;
                return Some((base + a, base + b));
            }
            line += 1;
            offset = 0;
        }
        None
    }

    fn find_backward(&self, text: &str, query: &str, before: usize) -> Option<(usize, usize)> {
        let mut line = self.lines.line_of(before) as isize;
        while line >= 0 {
            let at = line as usize;
            let content = self.lines.line(text, at);
            let limit = if at == self.lines.line_of(before) { before - self.lines.start(at) } else { content.len() };
            let mut best = None;
            let mut base = 0;
            let mut rest = &content[..limit.min(content.len())];
            while let Some((a, b)) = syntax::find_fold(rest, query) {
                best = Some((base + a, base + b));
                base += b;
                rest = &rest[b..];
            }
            if let Some((a, b)) = best {
                let origin = self.lines.start(at);
                return Some((origin + a, origin + b));
            }
            line -= 1;
        }
        None
    }

    // -----------------------------------------------------------------------------------
    // Layout
    // -----------------------------------------------------------------------------------

    fn state_at(&mut self, text: &str, line: usize) -> State {
        if self.states.is_empty() {
            self.states.push(if self.markdown { State::TEXT } else { State::MERMAID });
        }
        while self.states.len() <= line {
            let at = self.states.len() - 1;
            let next = syntax::advance(self.states[at], self.lines.line(text, at), self.markdown);
            self.states.push(next);
        }
        self.states[line]
    }

    fn galley(&mut self, ctx: &Context, env: &Env, text: &str, line: usize) -> Arc<Galley> {
        let state = self.state_at(text, line);
        let content = self.lines.line(text, line);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        content.hash(&mut hasher);
        (state.mode as u8).hash(&mut hasher);
        env.query.hash(&mut hasher);
        env.markdown.hash(&mut hasher);
        env.p.ink.hash(&mut hasher);
        let key = hasher.finish();
        if let Some((galley, used)) = self.galleys.get_mut(&key) {
            *used = self.frame;
            return galley.clone();
        }
        let job = syntax::highlight(content, state, env.markdown, env.p, &env.query, &env.font);
        let galley = ctx.fonts_mut(|fonts| fonts.layout_job(job));
        self.galleys.insert(key, (galley.clone(), self.frame));
        galley
    }

    /// Pixel x of `byte` inside its line, measured from the start of the text.
    fn x_of(&mut self, ctx: &Context, env: &Env, text: &str, byte: usize) -> f32 {
        let line = self.lines.line_of(byte);
        let start = self.lines.start(line);
        let column = chars_before(self.lines.line(text, line), byte - start);
        self.galley(ctx, env, text, line).pos_from_cursor(CCursor::new(column)).min.x
    }

    /// Byte offset on `line` nearest to pixel `x`.
    fn byte_at(&mut self, ctx: &Context, env: &Env, text: &str, line: usize, x: f32) -> usize {
        let galley = self.galley(ctx, env, text, line);
        let y = galley.rows.first().map_or(0.0, |row| row.rect().center().y);
        let cursor = galley.cursor_from_pos(vec2(x.max(0.0), y));
        self.lines.start(line) + byte_of_char(self.lines.line(text, line), cursor.index)
    }

    // -----------------------------------------------------------------------------------
    // One frame of the editing surface
    // -----------------------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn surface(&mut self, ui: &mut Ui, viewport: Rect, text: &mut String, env: &Env, id: Id, read_only: bool, view_w: f32) {
        let ctx = ui.ctx().clone();
        let count = self.lines.len();
        let line_h = env.line_h;
        let digits = count.to_string().len().max(2);
        let gutter_w = (digits as f32 * env.glyph_w).ceil() + 2.0 * GUTTER_PAD;
        let text_x = gutter_w + TEXT_PAD;
        self.view_lines = (viewport.height() / line_h).floor().max(1.0) as usize;

        if self.longest_dirty {
            self.longest = self.lines.longest(text);
            self.longest_dirty = false;
        }
        let content_w = (text_x + self.longest as f32 * env.glyph_w + 96.0).max(view_w);
        let content_h = count as f32 * line_h + (viewport.height() * 0.5).max(0.0);
        let origin = ui.cursor().min;
        let rect = Rect::from_min_size(origin, vec2(content_w, content_h));
        ui.allocate_rect(rect, Sense::hover());
        let response = ui.interact(rect, id, Sense::click_and_drag());
        let gutter_right = origin.x + viewport.min.x + gutter_w;
        let now = ctx.input(|input| input.time);

        // --- mouse ---------------------------------------------------------------------
        let (pressed, down, pointer, modifiers) = ctx.input(|input| {
            (
                input.pointer.primary_pressed(),
                input.pointer.primary_down(),
                input.pointer.latest_pos(),
                input.modifiers,
            )
        });
        if response.hovered() && pointer.is_some_and(|pos| pos.x >= gutter_right) {
            ctx.set_cursor_icon(CursorIcon::Text);
        }
        if pressed
            && response.hovered()
            && let Some(pos) = pointer
        {
            ctx.memory_mut(|memory| memory.request_focus(id));
            self.press(&ctx, env, text, pos, origin, gutter_right, text_x, modifiers, now);
        } else if self.drag.is_some() {
            if down && let Some(pos) = pointer {
                self.drag_to(&ctx, env, text, pos, origin, text_x);
                self.auto_scroll(ui, pos, origin, viewport, gutter_w);
            } else if !down {
                self.drag = None;
            }
        }
        if pressed && !response.hovered() && ctx.memory(|memory| memory.has_focus(id)) {
            ctx.memory_mut(|memory| memory.surrender_focus(id));
        }

        // --- keyboard ------------------------------------------------------------------
        let focused = ctx.memory(|memory| memory.has_focus(id));
        if focused {
            ctx.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    id,
                    EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false },
                );
            });
            ctx.data_mut(|data| data.insert_temp(Id::new("editor-focus"), id));
            let events = ctx.input(|input| input.events.clone());
            for event in events {
                self.event(&ctx, env, text, event, read_only, now);
            }
        }
        if self.longest_dirty {
            self.longest = self.lines.longest(text);
            self.longest_dirty = false;
        }

        // --- reveal the caret ----------------------------------------------------------
        let head_line = self.lines.line_of(self.sel.head);
        let head_x = self.x_of(&ctx, env, text, self.sel.head);
        if let Some(reveal) = self.reveal.take() {
            let caret = Rect::from_min_size(
                origin + vec2(text_x + head_x, head_line as f32 * line_h),
                vec2(2.0, line_h),
            );
            let target = caret.expand2(vec2(0.0, line_h)).with_min_x(caret.min.x - gutter_w - 3.0 * env.glyph_w).with_max_x(caret.max.x + 3.0 * env.glyph_w);
            let align = reveal.center.then_some(egui::Align::Center);
            let animation = if reveal.animate {
                ui.style().scroll_animation
            } else {
                egui::style::ScrollAnimation::none()
            };
            ui.scroll_to_rect_animation(target, align, animation);
        }

        // --- paint ---------------------------------------------------------------------
        let first = (viewport.min.y / line_h).floor().max(0.0) as usize;
        let last = (((viewport.max.y / line_h).ceil()) as usize + 1).min(count);
        let screen = Rect::from_min_max(origin + viewport.min.to_vec2(), origin + viewport.max.to_vec2());
        let text_clip = Rect::from_min_max(pos2(gutter_right, screen.top()), screen.max);
        let painter = ui.painter_at(screen);
        let text_painter = ui.painter_at(text_clip);
        let p = env.p;
        let (sel_start, sel_end) = self.sel.range();
        let dark = p.ink.r() > 200;
        let selection_color = theme::alpha(p.accent, match (focused, dark) {
            (true, true) => 80,
            (true, false) => 60,
            (false, true) => 45,
            (false, false) => 32,
        });
        if focused && self.sel.is_empty() {
            let y = origin.y + head_line as f32 * line_h;
            painter.rect_filled(
                Rect::from_min_max(pos2(gutter_right, y), pos2(screen.right(), y + line_h)),
                0.0,
                p.hover,
            );
        }
        let head_row = head_line;
        for line in first..last {
            let galley = self.galley(&ctx, env, text, line);
            let top = origin.y + line as f32 * line_h;
            let at = pos2(origin.x + text_x, top + ((line_h - galley.size().y) * 0.5).max(0.0));
            let (line_start, line_end) = (self.lines.start(line), self.lines.end(text, line));
            if sel_start < sel_end && sel_start <= line_end && sel_end > line_start {
                let content = self.lines.line(text, line);
                let from = sel_start.max(line_start) - line_start;
                let to = sel_end.min(line_end) - line_start;
                let x0 = galley.pos_from_cursor(CCursor::new(chars_before(content, from))).min.x;
                let mut x1 = galley.pos_from_cursor(CCursor::new(chars_before(content, to))).min.x;
                if sel_end > line_end {
                    x1 += env.glyph_w * 0.7;
                }
                text_painter.rect_filled(
                    Rect::from_min_max(pos2(at.x + x0, top), pos2(at.x + x1.max(x0 + 2.0), top + line_h)),
                    0.0,
                    selection_color,
                );
            }
            text_painter.galley(at, galley, p.ink);
            let _ = head_row;
        }
        // Caret.
        if focused {
            let phase = ((now - self.blink_from) / BLINK) as i64;
            if phase % 2 == 0 || !self.sel.is_empty() && false {
                let x = origin.x + text_x + head_x;
                let y = origin.y + head_line as f32 * line_h;
                text_painter.rect_filled(
                    Rect::from_min_max(pos2(x - 0.5, y + 2.0), pos2(x + 1.0, y + line_h - 2.0)),
                    0.0,
                    p.ink,
                );
            }
            let next = BLINK - (now - self.blink_from) % BLINK;
            ctx.request_repaint_after(Duration::from_secs_f64(next.max(0.016)));
            ctx.output_mut(|output| {
                let caret = Rect::from_min_size(
                    pos2(origin.x + text_x + head_x, origin.y + head_line as f32 * line_h),
                    vec2(1.0, line_h),
                );
                output.ime = Some(egui::output::IMEOutput { rect: screen, cursor_rect: caret });
            });
        }
        // Gutter: opaque, so text scrolled sideways slides under it.
        painter.rect_filled(
            Rect::from_min_max(pos2(gutter_right - gutter_w, screen.top()), pos2(gutter_right, screen.bottom())),
            0.0,
            p.panel,
        );
        for line in first..last {
            let y = origin.y + line as f32 * line_h + line_h * 0.5;
            let color = if line == head_line && focused { p.secondary } else { p.muted };
            painter.text(
                pos2(gutter_right - GUTTER_PAD, y),
                egui::Align2::RIGHT_CENTER,
                (line + 1).to_string(),
                env.font.clone(),
                color,
            );
        }
    }

    fn auto_scroll(&self, ui: &Ui, pos: Pos2, origin: Pos2, viewport: Rect, gutter_w: f32) {
        let top = origin.y + viewport.min.y;
        let bottom = origin.y + viewport.max.y;
        let left = origin.x + viewport.min.x + gutter_w;
        let right = origin.x + viewport.max.x;
        let speed = |distance: f32| -(distance.min(120.0) * 0.35).max(2.0);
        let mut delta = vec2(0.0, 0.0);
        if pos.y < top {
            delta.y = -speed(top - pos.y);
        } else if pos.y > bottom {
            delta.y = speed(pos.y - bottom);
        }
        if pos.x < left {
            delta.x = -speed(left - pos.x);
        } else if pos.x > right {
            delta.x = speed(pos.x - right);
        }
        if delta != vec2(0.0, 0.0) {
            ui.scroll_with_delta_animation(delta, egui::style::ScrollAnimation::none());
            ui.ctx().request_repaint();
        }
    }

    // -----------------------------------------------------------------------------------
    // Mouse
    // -----------------------------------------------------------------------------------

    fn locate(&mut self, ctx: &Context, env: &Env, text: &str, pos: Pos2, origin: Pos2, text_x: f32) -> (usize, usize) {
        let count = self.lines.len();
        let line = (((pos.y - origin.y) / env.line_h).floor().max(0.0) as usize).min(count - 1);
        let byte = self.byte_at(ctx, env, text, line, pos.x - origin.x - text_x);
        (line, byte)
    }

    #[allow(clippy::too_many_arguments)]
    fn press(
        &mut self,
        ctx: &Context,
        env: &Env,
        text: &str,
        pos: Pos2,
        origin: Pos2,
        gutter_right: f32,
        text_x: f32,
        modifiers: Modifiers,
        now: f64,
    ) {
        let count = match self.last_press {
            Some((time, at, count)) if now - time < 0.45 && (at - pos).length() < 5.0 => (count % 3) + 1,
            _ => 1,
        };
        self.last_press = Some((now, pos, count));
        self.blink_from = now;
        self.want_x = None;
        let (line, byte) = self.locate(ctx, env, text, pos, origin, text_x);
        let line_range = |this: &Self, line: usize| {
            let start = this.lines.start(line);
            let end = (this.lines.end(text, line) + 1).min(text.len());
            (start, end)
        };
        if pos.x < gutter_right {
            // A click on the line numbers selects the line.
            let range = line_range(self, line);
            self.sel = Selection { anchor: range.0, head: range.1 };
            self.drag = Some(Drag { mode: DragMode::Line, origin: range });
            return;
        }
        match count {
            1 => {
                if modifiers.shift {
                    self.sel.head = byte;
                } else {
                    self.sel = Selection::caret(byte);
                }
                self.drag = Some(Drag { mode: DragMode::Char, origin: (self.sel.anchor, self.sel.anchor) });
            }
            2 => {
                let range = word_at(text, byte);
                self.sel = Selection { anchor: range.0, head: range.1 };
                self.drag = Some(Drag { mode: DragMode::Word, origin: range });
            }
            _ => {
                let range = line_range(self, line);
                self.sel = Selection { anchor: range.0, head: range.1 };
                self.drag = Some(Drag { mode: DragMode::Line, origin: range });
            }
        }
    }

    fn drag_to(&mut self, ctx: &Context, env: &Env, text: &str, pos: Pos2, origin: Pos2, text_x: f32) {
        let Some(drag) = &self.drag else { return };
        let (mode, anchor_range) = (drag.mode, drag.origin);
        let (line, byte) = self.locate(ctx, env, text, pos, origin, text_x);
        match mode {
            DragMode::Char => self.sel.head = byte,
            DragMode::Word => {
                let (start, end) = word_at(text, byte);
                if byte < anchor_range.0 {
                    self.sel = Selection { anchor: anchor_range.1, head: start };
                } else {
                    self.sel = Selection { anchor: anchor_range.0, head: end.max(anchor_range.1) };
                }
            }
            DragMode::Line => {
                let start = self.lines.start(line);
                let end = (self.lines.end(text, line) + 1).min(text.len());
                if start < anchor_range.0 {
                    self.sel = Selection { anchor: anchor_range.1, head: start };
                } else {
                    self.sel = Selection { anchor: anchor_range.0, head: end };
                }
            }
        }
    }

    // -----------------------------------------------------------------------------------
    // Keyboard
    // -----------------------------------------------------------------------------------

    fn move_head(&mut self, head: usize, extend: bool) {
        if extend {
            self.sel.head = head;
        } else {
            self.sel = Selection::caret(head);
        }
        self.want_x = None;
        self.reveal = Some(Reveal { center: false, animate: false });
    }

    fn vertical(&mut self, ctx: &Context, env: &Env, text: &str, delta: isize, extend: bool) {
        let line = self.lines.line_of(self.sel.head);
        let x = match self.want_x {
            Some(x) => x,
            None => self.x_of(ctx, env, text, self.sel.head),
        };
        let target = line as isize + delta;
        let head = if target < 0 {
            0
        } else if target as usize >= self.lines.len() {
            text.len()
        } else {
            self.byte_at(ctx, env, text, target as usize, x)
        };
        self.move_head(head, extend);
        self.want_x = Some(x);
    }

    /// Replaces `[at, at + remove)` and puts the selection at `after`.
    fn change(&mut self, text: &mut String, at: usize, remove: usize, insert: &str, after: Selection, kind: Kind, now: f64) {
        let before = self.sel;
        model::replace(text, &mut self.lines, &mut self.history, Change { at, remove, insert, before, after, kind, time: now });
        self.touched(text, at);
        self.sel = after;
        self.want_x = None;
    }

    fn touched(&mut self, text: &str, at: usize) {
        self.revision += 1;
        self.text_len = text.len();
        self.changed = true;
        self.longest_dirty = true;
        let line = self.lines.line_of(at.min(text.len()));
        self.states.truncate(line + 1);
        self.reveal = Some(Reveal { center: false, animate: false });
    }

    fn replace_selection(&mut self, text: &mut String, insert: &str, kind: Kind, now: f64) {
        let (start, end) = self.sel.range();
        self.change(text, start, end - start, insert, Selection::caret(start + insert.len()), kind, now);
    }

    fn selected<'a>(&self, text: &'a str) -> &'a str {
        let (start, end) = self.sel.range();
        &text[start..end]
    }

    /// The lines touched by the selection as `(first, last)`; a selection that ends at the very
    /// start of a line does not touch it.
    fn touched_lines(&self) -> (usize, usize) {
        let (start, end) = self.sel.range();
        let first = self.lines.line_of(start);
        let mut last = self.lines.line_of(end);
        if end > start && end == self.lines.start(last) && last > first {
            last -= 1;
        }
        (first, last)
    }

    fn event(&mut self, ctx: &Context, env: &Env, text: &mut String, event: Event, read_only: bool, now: f64) {
        match event {
            Event::Text(inserted) if !read_only => {
                let inserted: String = inserted.chars().filter(|c| !c.is_control() || *c == '\t').collect();
                if !inserted.is_empty() {
                    self.blink_from = now;
                    self.replace_selection(text, &inserted, Kind::Typing, now);
                }
            }
            Event::Ime(egui::ImeEvent::Commit(inserted)) if !read_only => {
                if !inserted.is_empty() {
                    self.replace_selection(text, &inserted, Kind::Other, now);
                }
            }
            Event::Paste(pasted) if !read_only => {
                let pasted = normalize(&pasted);
                if !pasted.is_empty() {
                    self.history.seal();
                    self.replace_selection(text, &pasted, Kind::Other, now);
                    self.history.seal();
                }
            }
            Event::Copy | Event::Cut => {
                let cut = matches!(event, Event::Cut);
                if self.sel.is_empty() {
                    // Like most editors: copy or cut the whole line.
                    let line = self.lines.line_of(self.sel.head);
                    let (start, end) = (self.lines.start(line), (self.lines.end(text, line) + 1).min(text.len()));
                    let mut copied = text[start..end].to_owned();
                    if !copied.ends_with('\n') {
                        copied.push('\n');
                    }
                    ctx.copy_text(copied);
                    if cut && !read_only {
                        self.change(text, start, end - start, "", Selection::caret(start), Kind::Other, now);
                    }
                } else {
                    ctx.copy_text(self.selected(text).to_owned());
                    if cut && !read_only {
                        self.replace_selection(text, "", Kind::Other, now);
                    }
                }
            }
            Event::Key { key, pressed: true, modifiers, .. } => {
                if modifiers.ctrl && modifiers.alt {
                    return;
                }
                self.blink_from = now;
                self.key(ctx, env, text, key, modifiers, read_only, now);
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn key(&mut self, ctx: &Context, env: &Env, text: &mut String, key: Key, m: Modifiers, read_only: bool, now: f64) {
        let extend = m.shift;
        let word = if cfg!(target_os = "macos") { m.alt } else { m.ctrl };
        let head = self.sel.head;
        let (start, end) = self.sel.range();
        let line = self.lines.line_of(head);
        let page = self.view_lines.saturating_sub(2).max(1) as isize;
        match key {
            Key::ArrowLeft | Key::ArrowRight => {
                if m.alt && !cfg!(target_os = "macos") {
                    return;
                }
                let left = key == Key::ArrowLeft;
                if m.mac_cmd {
                    let at = if left { self.lines.start(line) } else { self.lines.end(text, line) };
                    self.move_head(at, extend);
                } else if !extend && !self.sel.is_empty() && !word {
                    self.move_head(if left { start } else { end }, false);
                } else {
                    let to = match (left, word) {
                        (true, true) => word_left(text, head),
                        (true, false) => prev_grapheme(text, head),
                        (false, true) => word_right(text, head),
                        (false, false) => next_grapheme(text, head),
                    };
                    self.move_head(to, extend);
                }
            }
            Key::ArrowUp | Key::ArrowDown => {
                let down = key == Key::ArrowDown;
                if m.alt && !read_only {
                    self.move_lines(text, down, now);
                } else if m.alt {
                } else {
                    self.vertical(ctx, env, text, if down { 1 } else { -1 }, extend);
                }
            }
            Key::PageUp => self.vertical(ctx, env, text, -page, extend),
            Key::PageDown => self.vertical(ctx, env, text, page, extend),
            Key::Home => {
                let at = if m.command {
                    0
                } else {
                    let first = self.lines.start(line);
                    let content = self.lines.line(text, line);
                    let smart = first + indent_of(content).len();
                    if head != smart && smart < self.lines.end(text, line) { smart } else { first }
                };
                self.move_head(at, extend);
            }
            Key::End => {
                let at = if m.command { text.len() } else { self.lines.end(text, line) };
                self.move_head(at, extend);
            }
            Key::A if m.command => {
                self.sel = Selection { anchor: 0, head: text.len() };
                self.want_x = None;
            }
            Key::Z if m.command && !read_only => {
                let restored = if m.shift {
                    model::redo(text, &mut self.lines, &mut self.history)
                } else {
                    model::undo(text, &mut self.lines, &mut self.history)
                };
                self.restored(text, restored);
            }
            Key::Y if m.command && !read_only => {
                let restored = model::redo(text, &mut self.lines, &mut self.history);
                self.restored(text, restored);
            }
            Key::Backspace | Key::Delete if !read_only => {
                let forward = key == Key::Delete;
                if !self.sel.is_empty() {
                    self.replace_selection(text, "", Kind::Other, now);
                } else {
                    let (from, to) = match (forward, word) {
                        (false, true) => (word_left(text, head), head),
                        (false, false) => (prev_grapheme(text, head), head),
                        (true, true) => (head, word_right(text, head)),
                        (true, false) => (head, next_grapheme(text, head)),
                    };
                    if from < to {
                        let kind = if word { Kind::Other } else if forward { Kind::Delete } else { Kind::Backspace };
                        self.change(text, from, to - from, "", Selection::caret(from), kind, now);
                    }
                }
            }
            Key::Enter if !read_only => self.enter(text, m, now),
            Key::Tab if !read_only => {
                let (first, last) = self.touched_lines();
                if m.shift {
                    self.shift_lines(text, first, last, true, now);
                } else if first != last {
                    self.shift_lines(text, first, last, false, now);
                } else {
                    let column = chars_before(&text[self.lines.start(line)..start], start - self.lines.start(line));
                    let pad = INDENT.len() - column % INDENT.len();
                    self.replace_selection(text, &INDENT[..pad], Kind::Other, now);
                }
            }
            Key::OpenBracket | Key::CloseBracket if m.command && !read_only => {
                let (first, last) = self.touched_lines();
                self.shift_lines(text, first, last, key == Key::OpenBracket, now);
            }
            Key::K if m.command && m.shift && !read_only => self.delete_lines(text, now),
            Key::L if m.command => {
                let (first, last) = self.touched_lines();
                let to = (self.lines.end(text, last) + 1).min(text.len());
                self.sel = Selection { anchor: self.lines.start(first), head: to };
                self.reveal = Some(Reveal { center: false, animate: false });
            }
            _ => {}
        }
    }

    fn restored(&mut self, text: &str, selection: Option<Selection>) {
        if let Some(selection) = selection {
            self.revision += 1;
            self.text_len = text.len();
            self.changed = true;
            self.longest_dirty = true;
            self.states.clear();
            self.sel = Selection { anchor: selection.anchor.min(text.len()), head: selection.head.min(text.len()) };
            self.want_x = None;
            self.reveal = Some(Reveal { center: false, animate: false });
        }
    }

    /// Enter: keeps the indentation and continues Markdown lists.
    fn enter(&mut self, text: &mut String, m: Modifiers, now: f64) {
        let (start, end) = self.sel.range();
        let line = self.lines.line_of(start);
        let line_start = self.lines.start(line);
        let content = self.lines.line(text, line);
        if m.command {
            // Ctrl+Enter opens a line below, Ctrl+Shift+Enter above.
            let indent = indent_of(content).to_owned();
            let at = if m.shift { line_start } else { self.lines.end(text, line) };
            let insert = if m.shift { format!("{indent}\n") } else { format!("\n{indent}") };
            let caret = if m.shift { at + indent.len() } else { at + insert.len() };
            self.history.seal();
            self.change(text, at, 0, &insert, Selection::caret(caret), Kind::Other, now);
            return;
        }
        let before = &content[..start - line_start];
        let mut prefix = indent_of(before).to_owned();
        let in_text = self.markdown && self.state_at(text, line).mode == syntax::Mode::Text;
        if in_text && let Some((marker_len, next, empty)) = list_marker(content) {
            if empty && end == line_start + content.len() {
                // An empty item ends the list.
                self.history.seal();
                self.change(text, line_start, content.len(), "", Selection::caret(line_start), Kind::Other, now);
                return;
            }
            if start - line_start >= marker_len {
                prefix = next;
            }
        }
        let insert = format!("\n{prefix}");
        self.history.seal();
        self.replace_selection(text, &insert, Kind::Other, now);
        self.history.seal();
    }

    /// Indents or outdents the lines `first..=last` by one level as a single undo step.
    fn shift_lines(&mut self, text: &mut String, first: usize, last: usize, outdent: bool, now: f64) {
        let block_start = self.lines.start(first);
        let block_end = self.lines.end(text, last);
        let mut out = String::with_capacity(block_end - block_start + INDENT.len() * (last - first + 1));
        // (old line start, bytes added (+) or removed (-) at its start)
        let mut shifts: Vec<(usize, isize)> = Vec::with_capacity(last - first + 1);
        for line in first..=last {
            let content = self.lines.line(text, line);
            let delta = if outdent {
                let spaces = content.bytes().take_while(|b| *b == b' ').count().min(INDENT.len());
                let cut = if spaces == 0 && content.starts_with('\t') { 1 } else { spaces };
                out.push_str(&content[cut..]);
                -(cut as isize)
            } else if content.is_empty() {
                0
            } else {
                out.push_str(INDENT);
                out.push_str(content);
                INDENT.len() as isize
            };
            shifts.push((self.lines.start(line), delta));
            if line != last {
                out.push('\n');
            }
        }
        let map = |offset: usize| -> usize {
            let mut moved = offset as isize;
            let mut total = 0isize;
            for (start, delta) in &shifts {
                if *start > offset {
                    break;
                }
                if *delta < 0 {
                    let column = (offset - start) as isize;
                    moved = offset as isize + total + (*delta).max(-column);
                    total += *delta;
                } else {
                    moved = offset as isize + total + *delta;
                    total += *delta;
                }
            }
            moved.max(0) as usize
        };
        let after = Selection { anchor: map(self.sel.anchor), head: map(self.sel.head) };
        self.history.seal();
        self.change(text, block_start, block_end - block_start, &out, after, Kind::Other, now);
        self.history.seal();
    }

    /// Alt+Up / Alt+Down: moves the touched lines one line up or down.
    fn move_lines(&mut self, text: &mut String, down: bool, now: f64) {
        let (first, last) = self.touched_lines();
        if (!down && first == 0) || (down && last + 1 >= self.lines.len()) {
            return;
        }
        let (block_start, block_end) = (self.lines.start(first), self.lines.end(text, last));
        let block = text[block_start..block_end].to_owned();
        let (from, to, replacement, shift): (usize, usize, String, isize) = if down {
            let next = self.lines.line(text, last + 1).to_owned();
            let to = self.lines.end(text, last + 1);
            (block_start, to, format!("{next}\n{block}"), next.len() as isize + 1)
        } else {
            let previous = self.lines.line(text, first - 1).to_owned();
            let from = self.lines.start(first - 1);
            (from, block_end, format!("{block}\n{previous}"), -(previous.len() as isize + 1))
        };
        let shifted = |offset: usize| (offset as isize + shift) as usize;
        let after = Selection { anchor: shifted(self.sel.anchor), head: shifted(self.sel.head) };
        self.history.seal();
        self.change(text, from, to - from, &replacement, after, Kind::Other, now);
        self.history.seal();
    }

    fn delete_lines(&mut self, text: &mut String, now: f64) {
        let (first, last) = self.touched_lines();
        let mut start = self.lines.start(first);
        let mut end = (self.lines.end(text, last) + 1).min(text.len());
        if last + 1 == self.lines.len() && start > 0 {
            start -= 1; // the last line takes the newline before it
            end = text.len();
        }
        self.history.seal();
        self.change(text, start, end - start, "", Selection::caret(start.min(text.len())), Kind::Other, now);
        self.history.seal();
    }
}

/// A Markdown list item: `(bytes up to and including the marker and its space, the prefix of the
/// next item, whether the item is empty)`.
fn list_marker(line: &str) -> Option<(usize, String, bool)> {
    let indent = indent_of(line);
    let rest = &line[indent.len()..];
    let bytes = rest.as_bytes();
    let (marker_len, next_marker) = if matches!(bytes.first(), Some(b'-' | b'*' | b'+')) && bytes.get(1) == Some(&b' ') {
        (2, rest[..2].to_owned())
    } else {
        let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
        if digits == 0 || digits > 9 || !matches!(bytes.get(digits), Some(b'.' | b')')) || bytes.get(digits + 1) != Some(&b' ') {
            return None;
        }
        let number: u64 = rest[..digits].parse().ok()?;
        (digits + 2, format!("{}{} ", number + 1, &rest[digits..digits + 1]))
    };
    let after = &rest[marker_len..];
    let (task_len, task) = if after.starts_with("[ ] ") || after.starts_with("[x] ") || after.starts_with("[X] ") {
        (4, "[ ] ")
    } else {
        (0, "")
    };
    let empty = after[task_len..].trim().is_empty();
    Some((indent.len() + marker_len, format!("{indent}{next_marker}{task}"), empty))
}

/// First case-insensitive match of the lowercase `needle` in `hay`, as a byte range.
pub use syntax::find_fold;

/// Byte range of the word around `at`, for double-click selection in other views.
pub fn word_range(text: &str, at: usize) -> (usize, usize) {
    word_at(text, at)
}

/// The id of the focused editor, for `typing`: set every frame the editor has the keyboard.
pub fn focused_id(ctx: &Context) -> Option<Id> {
    ctx.data(|data| data.get_temp::<Id>(Id::new("editor-focus")))
}

use egui::pos2;
