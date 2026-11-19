//! Per-line syntax colouring for the source editor.
//!
//! Colouring is presentation only; the diagram engine owns Mermaid parsing. Each line is
//! coloured from its text and a small `State` (inside a Mermaid fence, another fence, or plain
//! Markdown), so one edit only changes the lines it touches.

use egui::{Color32, FontId, TextFormat, text::LayoutJob};

use crate::theme::Palette;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Text,
    Mermaid,
    Code,
}

/// What a line starts in. `fence` is the char and length of the open fence.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct State {
    pub mode: Mode,
    fence: (u8, u16),
}

impl State {
    pub const MERMAID: State = State { mode: Mode::Mermaid, fence: (0, 0) };
    pub const TEXT: State = State { mode: Mode::Text, fence: (0, 0) };
}

/// A fence line found in `line`: char, run length and what follows the run.
fn fence_of(line: &str) -> Option<(u8, usize, &str)> {
    let mut rest = line.trim_start_matches([' ', '\t', '>']);
    // List item that holds the fence: "- ", "* ", "+ ", "1. ".
    let bytes = rest.as_bytes();
    if matches!(bytes.first(), Some(b'-' | b'*' | b'+')) && bytes.get(1) == Some(&b' ') {
        rest = rest[2..].trim_start();
    } else if let Some(dot) = rest.find(['.', ')'])
        && dot > 0
        && dot <= 9
        && rest[..dot].bytes().all(|b| b.is_ascii_digit())
        && rest[dot + 1..].starts_with(' ')
    {
        rest = rest[dot + 2..].trim_start();
    }
    let first = *rest.as_bytes().first()?;
    if first != b'`' && first != b'~' {
        return None;
    }
    let run = rest.bytes().take_while(|b| *b == first).count();
    (run >= 3).then(|| (first, run, &rest[run..]))
}

/// State at the start of the next line.
pub fn advance(state: State, line: &str, markdown: bool) -> State {
    if !markdown {
        return State::MERMAID;
    }
    if memchr::memchr2(b'`', b'~', line.as_bytes()).is_none() {
        return state;
    }
    let Some((ch, run, tail)) = fence_of(line) else {
        return state;
    };
    match state.mode {
        Mode::Text => {
            let lang = tail
                .trim()
                .trim_start_matches('{')
                .split(|c: char| c.is_whitespace() || c == '}')
                .next()
                .unwrap_or("");
            let mode = if lang.eq_ignore_ascii_case("mermaid") { Mode::Mermaid } else { Mode::Code };
            State { mode, fence: (ch, run.min(u16::MAX as usize) as u16) }
        }
        _ => {
            if ch == state.fence.0 && run >= state.fence.1 as usize && tail.trim().is_empty() {
                State::TEXT
            } else {
                state
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Colors {
    ink: Color32,
    keyword: Color32,
    string: Color32,
    arrow: Color32,
    punct: Color32,
    muted: Color32,
    code: Color32,
    link: Color32,
    find: Color32,
}

fn colors(p: Palette) -> Colors {
    let dark = p.ink.r() > 200;
    let string = if dark { Color32::from_rgb(142, 186, 150) } else { Color32::from_rgb(60, 105, 75) };
    Colors {
        ink: p.ink,
        keyword: p.accent,
        string,
        arrow: p.accent,
        punct: p.secondary,
        muted: p.muted,
        code: string,
        link: p.accent,
        find: p.accent_soft,
    }
}

const KEYWORDS: &[&str] = &[
    "flowchart", "graph", "sequenceDiagram", "classDiagram", "stateDiagram-v2", "stateDiagram",
    "erDiagram", "gantt", "pie", "journey", "mindmap", "timeline", "gitGraph", "quadrantChart",
    "requirementDiagram", "subgraph", "end", "direction", "classDef", "class", "style", "click",
    "participant", "actor", "loop", "alt", "else", "opt", "par", "and", "Note", "note", "activate",
    "deactivate", "title", "section", "dateFormat", "accTitle", "accDescr", "linkStyle", "autonumber",
    "rect", "critical", "break", "over", "of", "left", "right", "TD", "TB", "BT", "LR", "RL",
];

struct Builder<'a> {
    job: LayoutJob,
    font: &'a FontId,
    query: &'a str,
    colors: &'a Colors,
}

impl Builder<'_> {
    fn push(&mut self, text: &str, color: Color32) {
        if text.is_empty() {
            return;
        }
        // Matches of the find text get a background; the rest of the token keeps its colour.
        if !self.query.is_empty() {
            let mut rest = text;
            while !rest.is_empty() {
                match find_fold(rest, self.query) {
                    Some((start, end)) => {
                        self.raw(&rest[..start], color, Color32::TRANSPARENT);
                        self.raw(&rest[start..end], color, self.colors.find);
                        rest = &rest[end..];
                    }
                    None => {
                        self.raw(rest, color, Color32::TRANSPARENT);
                        break;
                    }
                }
            }
        } else {
            self.raw(text, color, Color32::TRANSPARENT);
        }
    }

    fn raw(&mut self, text: &str, color: Color32, background: Color32) {
        if text.is_empty() {
            return;
        }
        self.job.append(
            text,
            0.0,
            TextFormat { font_id: self.font.clone(), color, background, ..Default::default() },
        );
    }
}

/// First occurrence of `needle` in `hay` ignoring case; byte range in `hay`. `needle` is
/// already lowercase. Only the first char of a case fold is compared, which is exact for
/// everything but a few ligatures.
pub fn find_fold(hay: &str, needle: &str) -> Option<(usize, usize)> {
    if needle.is_empty() {
        return None;
    }
    if needle.is_ascii() && hay.is_ascii() {
        let (hay_bytes, needle_bytes) = (hay.as_bytes(), needle.as_bytes());
        let first = needle_bytes[0];
        let mut from = 0;
        while from + needle_bytes.len() <= hay_bytes.len() {
            let candidate = hay_bytes[from..=hay_bytes.len() - needle_bytes.len()]
                .iter()
                .position(|b| b.to_ascii_lowercase() == first)?;
            let at = from + candidate;
            if hay_bytes[at..at + needle_bytes.len()].eq_ignore_ascii_case(needle_bytes) {
                return Some((at, at + needle_bytes.len()));
            }
            from = at + 1;
        }
        return None;
    }
    let needle_chars: Vec<char> = needle.chars().collect();
    let indices: Vec<(usize, char)> = hay.char_indices().collect();
    'outer: for start in 0..indices.len() {
        if start + needle_chars.len() > indices.len() {
            break;
        }
        for (offset, wanted) in needle_chars.iter().enumerate() {
            let have = indices[start + offset].1.to_lowercase().next().unwrap_or(' ');
            if have != *wanted {
                continue 'outer;
            }
        }
        let last = indices[start + needle_chars.len() - 1];
        return Some((indices[start].0, last.0 + last.1.len_utf8()));
    }
    None
}

/// Lays out the colours of one line. `query` is the lowercase find text (may be empty).
pub fn highlight(
    line: &str,
    state: State,
    markdown: bool,
    p: Palette,
    query: &str,
    font: &FontId,
) -> LayoutJob {
    let colors = colors(p);
    let mut b = Builder { job: LayoutJob::default(), font, query, colors: &colors };
    b.job.wrap.max_width = f32::INFINITY;
    if line.is_empty() {
        // An empty galley still needs the font so the row has a height.
        b.job.append("", 0.0, TextFormat { font_id: font.clone(), ..Default::default() });
        return b.job;
    }
    let fence = markdown.then(|| fence_of(line)).flatten();
    if let Some((_, _, tail)) = fence {
        let split = line.len() - tail.len();
        b.push(&line[..split], colors.keyword);
        b.push(tail, colors.muted);
        return b.job;
    }
    match state.mode {
        Mode::Mermaid => mermaid_line(&mut b, line, markdown),
        Mode::Code => b.push(line, colors.string),
        Mode::Text => markdown_line(&mut b, line),
    }
    b.job
}

fn mermaid_line(b: &mut Builder, line: &str, markdown: bool) {
    let c = *b.colors;
    let trimmed = line.trim_start();
    if trimmed.starts_with("%%") {
        b.push(line, c.muted);
        return;
    }
    let _ = markdown;
    let mut offset = 0;
    while offset < line.len() {
        let tail = &line[offset..];
        let first = tail.chars().next().unwrap_or(' ');
        let (length, color) = if first == '"' || first == '\'' {
            let mut escaped = false;
            let mut end = first.len_utf8();
            for ch in tail[first.len_utf8()..].chars() {
                end += ch.len_utf8();
                if ch == first && !escaped {
                    break;
                }
                escaped = ch == '\\' && !escaped;
            }
            (end, c.string)
        } else if first.is_alphanumeric() || first == '_' {
            let length = tail
                .char_indices()
                .find(|(_, ch)| !(ch.is_alphanumeric() || *ch == '_'))
                .map_or(tail.len(), |(index, _)| index);
            let mut word = &tail[..length];
            let mut length = length;
            // `stateDiagram-v2` is one keyword.
            if word == "stateDiagram" && tail[length..].starts_with("-v2") {
                length += 3;
                word = &tail[..length];
            }
            let color = if KEYWORDS.contains(&word) {
                c.keyword
            } else if word.chars().all(|ch| ch.is_ascii_digit()) {
                c.arrow
            } else {
                c.ink
            };
            (length, color)
        } else if matches!(first, '-' | '=' | '.' | '<' | '>' | '~') {
            let length = tail.find(|ch: char| !matches!(ch, '-' | '=' | '.' | '<' | '>' | '~' | 'o' | 'x')).unwrap_or(tail.len());
            // A run that is only `o` or `x` at its end is part of an arrow head; a lone `.` or `-` is text.
            let run = &tail[..length];
            let arrowish = run.len() >= 2 || matches!(first, '<' | '>');
            if arrowish { (length, c.arrow) } else { (first.len_utf8(), c.punct) }
        } else if matches!(first, '|' | ':') {
            (1, c.arrow)
        } else if first.is_whitespace() {
            let length = tail.find(|ch: char| !ch.is_whitespace()).unwrap_or(tail.len());
            (length, c.ink)
        } else {
            (first.len_utf8(), c.punct)
        };
        b.push(&tail[..length], color);
        offset += length;
    }
}

fn markdown_line(b: &mut Builder, line: &str) {
    let c = *b.colors;
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    if trimmed.starts_with("<!--") {
        b.push(line, c.muted);
        return;
    }
    let hashes = trimmed.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
        b.push(&line[..indent], c.ink);
        b.push(&trimmed[..hashes], c.muted);
        inline(b, &trimmed[hashes..], c.keyword, &c);
        return;
    }
    if trimmed.starts_with("---") && trimmed.trim_end().bytes().all(|ch| ch == b'-') {
        b.push(line, c.muted);
        return;
    }
    let mut rest = line;
    if let Some(quote) = trimmed.strip_prefix('>') {
        b.push(&line[..indent + 1], c.muted);
        rest = quote;
        let _ = rest;
        inline(b, quote, c.punct, &c);
        return;
    }
    // List marker.
    let bytes = trimmed.as_bytes();
    if matches!(bytes.first(), Some(b'-' | b'*' | b'+')) && bytes.get(1) == Some(&b' ') {
        b.push(&line[..indent], c.ink);
        b.push(&trimmed[..1], c.keyword);
        rest = &trimmed[1..];
    } else if let Some(dot) = trimmed.find(['.', ')'])
        && dot > 0
        && dot <= 9
        && trimmed[..dot].bytes().all(|b| b.is_ascii_digit())
        && trimmed[dot + 1..].starts_with(' ')
    {
        b.push(&line[..indent], c.ink);
        b.push(&trimmed[..dot + 1], c.keyword);
        rest = &trimmed[dot + 1..];
    }
    inline(b, rest, c.ink, &c);
}

/// Inline Markdown: `code`, **strong**, [links](url).
fn inline(b: &mut Builder, text: &str, base: Color32, c: &Colors) {
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'`' => {
                let ticks = bytes[i..].iter().take_while(|b| **b == b'`').count();
                let marker = &text[i..i + ticks];
                if let Some(close) = text[i + ticks..].find(marker) {
                    b.push(&text[start..i], base);
                    let end = i + ticks + close + ticks;
                    b.push(&text[i..end], c.code);
                    i = end;
                    start = end;
                    continue;
                }
                i += ticks;
            }
            b'[' => {
                if let Some(label_end) = text[i..].find("](")
                    && let Some(url_end) = text[i + label_end..].find(')')
                {
                    b.push(&text[start..i], base);
                    let end = i + label_end + url_end + 1;
                    b.push(&text[i..i + label_end + 1], c.link);
                    b.push(&text[i + label_end + 1..end], c.muted);
                    i = end;
                    start = end;
                    continue;
                }
                i += 1;
            }
            b'*' | b'_' => {
                let marker = bytes[i];
                let run = bytes[i..].iter().take_while(|b| **b == marker).count();
                if run <= 3 && (marker == b'*' || i == 0 || !bytes[i - 1].is_ascii_alphanumeric()) {
                    let closing = &text[i..i + run];
                    if let Some(close) = text[i + run..].find(closing)
                        && close > 0
                        && !text[i + run..i + run + close].starts_with(' ')
                    {
                        b.push(&text[start..i], base);
                        let end = i + run + close + run;
                        b.push(&text[i..i + run], c.muted);
                        b.push(&text[i + run..i + run + close], c.ink);
                        b.push(&text[i + run + close..end], c.muted);
                        i = end;
                        start = end;
                        continue;
                    }
                }
                i += run;
            }
            _ => i += 1,
        }
    }
    b.push(&text[start..], base);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fences_track_mermaid_blocks() {
        let mut state = State::TEXT;
        let seen: Vec<Mode> = ["# t", "```mermaid", "graph TD", "```", "text", "~~~rust", "```", "~~~"]
            .iter()
            .map(|line| {
                let at = state.mode;
                state = advance(state, line, true);
                at
            })
            .collect();
        assert_eq!(
            seen,
            [Mode::Text, Mode::Text, Mode::Mermaid, Mode::Mermaid, Mode::Text, Mode::Text, Mode::Code, Mode::Code]
        );
        assert_eq!(state.mode, Mode::Text);
    }

    #[test]
    fn find_ignores_case() {
        assert_eq!(find_fold("Hello World", "world"), Some((6, 11)));
        assert_eq!(find_fold("ÄÖ x", "öx"), None);
        assert_eq!(find_fold("ÄÖ x", "ö"), Some((2, 4)));
    }
}
