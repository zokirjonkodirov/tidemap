//! Text model of the source editor: line index, selection arithmetic and undo history.
//!
//! The text itself lives in a `String` owned by the app; this module only keeps what makes
//! navigation and editing O(log n) in the number of lines instead of O(n) in the text size.
//! Every offset is a UTF-8 byte offset on a `char` boundary. Text uses `\n` line ends only.

use std::collections::VecDeque;
use unicode_segmentation::GraphemeCursor;

/// Byte offsets of line starts. Line `i` is `text[starts[i]..end]`, `end` being the offset of
/// its `\n` (or the end of the text). A text always has at least one line.
#[derive(Clone, Debug)]
pub struct LineIndex {
    starts: Vec<usize>,
}

impl Default for LineIndex {
    fn default() -> Self {
        Self { starts: vec![0] }
    }
}

impl LineIndex {
    pub fn build(text: &str) -> Self {
        let mut starts = Vec::with_capacity(text.len() / 32 + 1);
        starts.push(0);
        starts.extend(memchr::memchr_iter(b'\n', text.as_bytes()).map(|at| at + 1));
        Self { starts }
    }

    pub fn len(&self) -> usize {
        self.starts.len()
    }

    pub fn start(&self, line: usize) -> usize {
        self.starts[line.min(self.starts.len() - 1)]
    }

    /// Offset just before the `\n` that ends `line` (or the text length for the last line).
    pub fn end(&self, text: &str, line: usize) -> usize {
        match self.starts.get(line + 1) {
            Some(next) => next - 1,
            None => text.len(),
        }
    }

    pub fn line<'a>(&self, text: &'a str, line: usize) -> &'a str {
        &text[self.start(line)..self.end(text, line)]
    }

    /// Line that holds `byte`; the offset of a `\n` belongs to the line it ends.
    pub fn line_of(&self, byte: usize) -> usize {
        self.starts.partition_point(|start| *start <= byte).saturating_sub(1)
    }

    /// Updates the index after `text[at..at + removed]` was replaced by `inserted`.
    pub fn apply(&mut self, at: usize, removed: usize, inserted: &str) {
        let first = self.starts.partition_point(|start| *start <= at);
        let last = self.starts.partition_point(|start| *start <= at + removed);
        let created: Vec<usize> = memchr::memchr_iter(b'\n', inserted.as_bytes())
            .map(|offset| at + offset + 1)
            .collect();
        let tail = first + created.len();
        self.starts.splice(first..last, created);
        let delta = inserted.len() as isize - removed as isize;
        if delta != 0 {
            for start in &mut self.starts[tail..] {
                *start = (*start as isize + delta) as usize;
            }
        }
    }

    /// Longest line in bytes: an upper bound for its width in columns.
    pub fn longest(&self, text: &str) -> usize {
        (0..self.len())
            .map(|line| self.end(text, line) - self.start(line))
            .max()
            .unwrap_or(0)
    }
}

/// Anchor and head of the selection; the head is the caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn caret(at: usize) -> Self {
        Self { anchor: at, head: at }
    }

    pub fn range(self) -> (usize, usize) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }

    pub fn is_empty(self) -> bool {
        self.anchor == self.head
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Typing,
    Backspace,
    Delete,
    Other,
}

#[derive(Clone, Debug)]
struct Edit {
    at: usize,
    removed: String,
    inserted: String,
    before: Selection,
    after: Selection,
    kind: Kind,
    time: f64,
}

impl Edit {
    fn cost(&self) -> usize {
        self.inserted.len() + self.removed.len() + 64
    }
}

/// Undo and redo stacks. Edits are stored as replacements, so memory follows the size of what
/// changed, not of the document.
#[derive(Default)]
pub struct History {
    undo: VecDeque<Edit>,
    redo: Vec<Edit>,
    bytes: usize,
    /// The next edit must open a new undo step.
    seal: bool,
}

const HISTORY_BYTES: usize = 24 << 20;
/// Typing within this many seconds of the last keystroke joins the same undo step.
const MERGE_SECONDS: f64 = 1.0;

impl History {
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.bytes = 0;
        self.seal = false;
    }

    /// The next edit starts a new undo step even if it would merge.
    pub fn seal(&mut self) {
        self.seal = true;
    }

    fn record(&mut self, edit: Edit) {
        self.redo.clear();
        let seal = std::mem::take(&mut self.seal);
        if !seal && let Some(last) = self.undo.back_mut() {
            let close = edit.time - last.time <= MERGE_SECONDS;
            let merged = close
                && match (last.kind, edit.kind) {
                    (Kind::Typing, Kind::Typing) => {
                        last.removed.is_empty()
                            && edit.removed.is_empty()
                            && edit.at == last.at + last.inserted.len()
                            && !edit.inserted.contains('\n')
                            && last.inserted.len() < 200
                            && (edit.inserted.chars().all(char::is_alphanumeric)
                                || !last.inserted.ends_with(char::is_alphanumeric))
                    }
                    (Kind::Backspace, Kind::Backspace) => {
                        last.inserted.is_empty()
                            && edit.inserted.is_empty()
                            && edit.at + edit.removed.len() == last.at
                    }
                    (Kind::Delete, Kind::Delete) => {
                        last.inserted.is_empty() && edit.inserted.is_empty() && edit.at == last.at
                    }
                    _ => false,
                };
            if merged {
                self.bytes += edit.inserted.len() + edit.removed.len();
                match edit.kind {
                    Kind::Typing => last.inserted.push_str(&edit.inserted),
                    Kind::Backspace => {
                        last.at = edit.at;
                        last.removed.insert_str(0, &edit.removed);
                    }
                    _ => last.removed.push_str(&edit.removed),
                }
                last.after = edit.after;
                last.time = edit.time;
                return;
            }
        }
        self.bytes += edit.cost();
        self.undo.push_back(edit);
        while self.bytes > HISTORY_BYTES && self.undo.len() > 1 {
            if let Some(old) = self.undo.pop_front() {
                self.bytes = self.bytes.saturating_sub(old.cost());
            }
        }
    }
}

/// One replacement of the text: `text[at..at + remove]` becomes `insert`.
pub struct Change<'a> {
    pub at: usize,
    pub remove: usize,
    pub insert: &'a str,
    pub before: Selection,
    pub after: Selection,
    pub kind: Kind,
    pub time: f64,
}

/// Applies `change` and keeps `lines` and `history` in step.
pub fn replace(text: &mut String, lines: &mut LineIndex, history: &mut History, change: Change) {
    let Change { at, remove, insert, before, after, kind, time } = change;
    let removed = text[at..at + remove].to_owned();
    text.replace_range(at..at + remove, insert);
    lines.apply(at, remove, insert);
    history.record(Edit { at, removed, inserted: insert.to_owned(), before, after, kind, time });
}

/// Reverts the last step. Returns the selection to restore.
pub fn undo(text: &mut String, lines: &mut LineIndex, history: &mut History) -> Option<Selection> {
    let edit = history.undo.pop_back()?;
    history.bytes = history.bytes.saturating_sub(edit.cost());
    text.replace_range(edit.at..edit.at + edit.inserted.len(), &edit.removed);
    lines.apply(edit.at, edit.inserted.len(), &edit.removed);
    let selection = edit.before;
    history.redo.push(edit);
    history.seal = true;
    Some(selection)
}

pub fn redo(text: &mut String, lines: &mut LineIndex, history: &mut History) -> Option<Selection> {
    let edit = history.redo.pop()?;
    text.replace_range(edit.at..edit.at + edit.removed.len(), &edit.inserted);
    lines.apply(edit.at, edit.removed.len(), &edit.inserted);
    let selection = edit.after;
    history.bytes += edit.cost();
    history.undo.push_back(edit);
    history.seal = true;
    Some(selection)
}

// ---------------------------------------------------------------------------------------
// Movement
// ---------------------------------------------------------------------------------------

/// Offset of the next grapheme boundary after `at` (the text length at the end).
pub fn next_grapheme(text: &str, at: usize) -> usize {
    if at >= text.len() {
        return text.len();
    }
    // One line at a time: a grapheme never spans a line end in editable text.
    let end = text[at..].find('\n').map_or(text.len(), |n| at + n);
    if at == end {
        return at + 1;
    }
    let mut cursor = GraphemeCursor::new(at, end, true);
    match cursor.next_boundary(&text[..end], 0) {
        Ok(Some(next)) => next,
        _ => text[at..].chars().next().map_or(text.len(), |c| at + c.len_utf8()),
    }
}

pub fn prev_grapheme(text: &str, at: usize) -> usize {
    if at == 0 {
        return 0;
    }
    if text.as_bytes()[at - 1] == b'\n' {
        return at - 1;
    }
    let start = text[..at].rfind('\n').map_or(0, |n| n + 1);
    let mut cursor = GraphemeCursor::new(at - start, at - start, true);
    match cursor.prev_boundary(&text[start..at], 0) {
        Ok(Some(prev)) => start + prev,
        _ => text[..at].chars().next_back().map_or(0, |c| at - c.len_utf8()),
    }
}

#[derive(PartialEq, Clone, Copy)]
enum Class {
    Word,
    Space,
    Other,
}

fn class(ch: char) -> Class {
    if ch.is_alphanumeric() || ch == '_' || ('\u{300}'..='\u{36f}').contains(&ch) || matches!(ch, '\u{200d}' | '\u{fe0f}') {
        Class::Word
    } else if ch == ' ' || ch == '\t' {
        Class::Space
    } else {
        Class::Other
    }
}

/// Start of the word before `at` (Ctrl+Left).
pub fn word_left(text: &str, at: usize) -> usize {
    let mut chars = text[..at].char_indices().rev().peekable();
    let mut position = at;
    while let Some(&(index, ch)) = chars.peek() {
        if class(ch) == Class::Space {
            position = index;
            chars.next();
        } else {
            break;
        }
    }
    match chars.peek() {
        None => 0,
        Some(&(index, '\n')) => {
            if position == at {
                index
            } else {
                position
            }
        }
        Some(&(_, ch)) => {
            let wanted = class(ch);
            while let Some(&(index, ch)) = chars.peek() {
                if class(ch) == wanted && ch != '\n' {
                    position = index;
                    chars.next();
                } else {
                    break;
                }
            }
            position
        }
    }
}

/// End of the word after `at` (Ctrl+Right).
pub fn word_right(text: &str, at: usize) -> usize {
    let mut chars = text[at..].char_indices().peekable();
    let mut position = at;
    match chars.peek() {
        None => return text.len(),
        Some(&(_, '\n')) => return at + 1,
        Some(&(_, ch)) if class(ch) != Class::Space => {
            let wanted = class(ch);
            while let Some(&(index, ch)) = chars.peek() {
                if class(ch) == wanted && ch != '\n' {
                    position = at + index + ch.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
        }
        _ => {}
    }
    while let Some(&(index, ch)) = chars.peek() {
        if class(ch) == Class::Space {
            position = at + index + ch.len_utf8();
            chars.next();
        } else {
            break;
        }
    }
    position
}

/// The word (or run of spaces or punctuation) around `at`, for a double click.
pub fn word_at(text: &str, at: usize) -> (usize, usize) {
    let after = text[at..].chars().next().filter(|c| *c != '\n');
    let before = text[..at].chars().next_back().filter(|c| *c != '\n');
    let probe = match (after, before) {
        (Some(a), Some(b)) if class(a) == Class::Word || class(b) != Class::Word => a,
        (Some(a), None) => a,
        (_, Some(b)) => b,
        (None, None) => return (at, at),
    };
    let wanted = class(probe);
    let mut start = at;
    for (index, ch) in text[..at].char_indices().rev() {
        if class(ch) == wanted && ch != '\n' {
            start = index;
        } else {
            break;
        }
    }
    let mut end = at;
    for (index, ch) in text[at..].char_indices() {
        if class(ch) == wanted && ch != '\n' {
            end = at + index + ch.len_utf8();
        } else {
            break;
        }
    }
    (start, end)
}

/// Number of chars in `text[..byte]`: counts every byte that is not a UTF-8 continuation byte.
pub fn chars_before(text: &str, byte: usize) -> usize {
    text.as_bytes()[..byte.min(text.len())]
        .iter()
        .filter(|b| (**b & 0xC0) != 0x80)
        .count()
}

/// Byte offset of char `index`, clamped to the text length.
pub fn byte_of_char(text: &str, index: usize) -> usize {
    text.char_indices().nth(index).map_or(text.len(), |(byte, _)| byte)
}

/// Leading spaces and tabs of `line`.
pub fn indent_of(line: &str) -> &str {
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// Normalises pasted text to `\n` line ends.
pub fn normalize(text: &str) -> String {
    if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(at: usize, remove: usize, insert: &str, kind: Kind, time: f64) -> Change<'_> {
        Change {
            at,
            remove,
            insert,
            before: Selection::caret(at),
            after: Selection::caret(at + insert.len()),
            kind,
            time,
        }
    }

    fn check(text: &str, lines: &LineIndex) {
        assert_eq!(lines.starts, LineIndex::build(text).starts, "index drifted for {text:?}");
    }

    #[test]
    fn line_index_follows_edits() {
        let mut text = String::from("alpha\nbeta\n\ngamma");
        let mut lines = LineIndex::build(&text);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines.line(&text, 1), "beta");
        assert_eq!(lines.line_of(5), 0);
        assert_eq!(lines.line_of(6), 1);
        let mut history = History::default();
        replace(&mut text, &mut lines, &mut history, change(3, 5, "X\nY\nZ", Kind::Other, 0.0));
        check(&text, &lines);
        let all = text.len();
        replace(&mut text, &mut lines, &mut history, change(0, all, "", Kind::Other, 0.0));
        check(&text, &lines);
        replace(&mut text, &mut lines, &mut history, change(0, 0, "a\nb\n", Kind::Other, 0.0));
        check(&text, &lines);
        replace(&mut text, &mut lines, &mut history, change(1, 1, "", Kind::Other, 0.0));
        check(&text, &lines);
    }

    #[test]
    fn undo_redo_roundtrip_and_merge() {
        let mut text = String::new();
        let mut lines = LineIndex::build(&text);
        let mut history = History::default();
        for (i, ch) in "hello".chars().enumerate() {
            replace(
                &mut text,
                &mut lines,
                &mut history,
                change(i, 0, &ch.to_string(), Kind::Typing, 0.1 * i as f64),
            );
        }
        assert_eq!(history.undo.len(), 1, "typing merges into one step");
        undo(&mut text, &mut lines, &mut history);
        assert_eq!(text, "");
        redo(&mut text, &mut lines, &mut history);
        assert_eq!(text, "hello");
        check(&text, &lines);
    }

    #[test]
    fn graphemes_and_words() {
        let text = "a\u{301}bc def\nxyz";
        assert_eq!(next_grapheme(text, 0), 3);
        assert_eq!(prev_grapheme(text, 3), 0);
        assert_eq!(word_right(text, 0), 6);
        assert_eq!(word_left(text, 9), 6);
        assert_eq!(word_at(text, 7), (6, 9));
        assert_eq!(chars_before("héé", 5), 3);
        assert_eq!(byte_of_char("héé", 2), 3);
    }
}
