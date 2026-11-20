//! `block-beta` diagrams: blocks on a column grid with arrows between them.

use crate::charts::{line, rect, text};
use crate::decor::{Anchor, Decor, Tone};
use crate::linewise::{Line, split_word};
use crate::parser::clean_label;
use crate::shape::Rect;
use crate::text::Measurer;

const MAX_BLOCKS: usize = 5_000;

#[derive(Clone, Debug)]
pub struct Blk {
    pub id: String,
    pub label: String,
    /// 0 rectangle, 1 rounded, 2 circle, 3 diamond.
    pub shape: u8,
    pub span: usize,
    pub space: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Block {
    pub columns: usize,
    pub blocks: Vec<Blk>,
    pub edges: Vec<(String, String, String)>,
}

/// Splits a row of block tokens at spaces outside quotes and brackets.
fn tokens(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut quoted = false;
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '[' | '(' | '{' if !quoted => depth += 1,
            ']' | ')' | '}' if !quoted => depth -= 1,
            _ => {}
        }
        if c.is_whitespace() && !quoted && depth <= 0 {
            if let Some(s) = start.take() {
                out.push(&text[s..i]);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        out.push(&text[s..]);
    }
    out
}

fn block(token: &str) -> Option<Blk> {
    let (main, span) = match token.rsplit_once(':') {
        Some((m, s)) if s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty() => {
            (m, s.parse::<usize>().unwrap_or(1).clamp(1, 24))
        }
        _ => (token, 1),
    };
    if main == "space" {
        return Some(Blk { id: String::new(), label: String::new(), shape: 0, span, space: true });
    }
    let at = main.find(['[', '(', '{', '>']).unwrap_or(main.len());
    let id = main[..at].to_string();
    if id.is_empty() {
        return None;
    }
    let rest = &main[at..];
    let strip = |s: &str, o: &str, c: &str| -> Option<String> {
        s.strip_prefix(o)?.strip_suffix(c).map(|i| clean_label(i.trim().trim_matches('"')))
    };
    let (shape, label) = if let Some(l) = strip(rest, "((", "))") {
        (2, l)
    } else if let Some(l) = strip(rest, "(", ")") {
        (1, l)
    } else if let Some(l) = strip(rest, "{", "}") {
        (3, l)
    } else if let Some(l) = strip(rest, "[", "]").or_else(|| strip(rest, ">", "]")) {
        (0, l)
    } else {
        (0, id.clone())
    };
    Some(Blk { id, label, shape, span, space: false })
}

pub(crate) fn parse(body: &[(Line, &str)]) -> Block {
    let mut b = Block::default();
    for (_, text) in body {
        let (word, rest) = split_word(text);
        if word == "columns" {
            b.columns = rest.trim().parse().unwrap_or(0);
            continue;
        }
        if word == "end" || word.starts_with("block:") || word == "block" || word == "style" || word == "classDef" || word == "class" {
            continue;
        }
        if text.contains("-->") {
            // `a --> b` or `a -- "label" --> b`
            let Some((left, right)) = text.split_once("-->") else { continue };
            let (a, label) = match left.split_once("--") {
                Some((a, l)) => (a.trim(), clean_label(l.trim().trim_matches('"'))),
                None => (left.trim(), String::new()),
            };
            b.edges.push((a.to_string(), right.trim().to_string(), label));
            continue;
        }
        for t in tokens(text) {
            if b.blocks.len() >= MAX_BLOCKS {
                break;
            }
            if let Some(blk) = block(t) {
                b.blocks.push(blk);
            }
        }
    }
    b
}

/// Where the ray from the centre of `r` towards `to` leaves the rectangle.
fn clip(r: Rect, to: [f32; 2]) -> [f32; 2] {
    let c = r.center();
    let d = [to[0] - c[0], to[1] - c[1]];
    let tx = if d[0].abs() > 1e-6 { r.w / 2.0 / d[0].abs() } else { f32::INFINITY };
    let ty = if d[1].abs() > 1e-6 { r.h / 2.0 / d[1].abs() } else { f32::INFINITY };
    let t = tx.min(ty);
    if t.is_finite() { [c[0] + d[0] * t, c[1] + d[1] * t] } else { c }
}

pub(crate) fn layout(b: &Block, me: &Measurer) -> Vec<Decor> {
    let mut d = Vec::new();
    if b.blocks.is_empty() {
        return d;
    }
    let cols = if b.columns > 0 { b.columns } else { b.blocks.iter().map(|k| k.span).sum::<usize>().min(8) }.max(1);
    let cell_w = b
        .blocks
        .iter()
        .filter(|k| !k.space)
        .map(|k| (me.width(&k.label, 13.0) + 40.0) / k.span as f32)
        .fold(96.0, f32::max);
    let (cell_h, gap) = (54.0f32, 26.0f32);
    let mut col = 0usize;
    let mut row = 0usize;
    let mut rects: Vec<(String, Rect)> = Vec::new();
    for k in &b.blocks {
        let span = k.span.min(cols);
        if col + span > cols {
            col = 0;
            row += 1;
        }
        let r = Rect {
            x: col as f32 * (cell_w + gap),
            y: row as f32 * (cell_h + gap),
            w: span as f32 * cell_w + (span - 1) as f32 * gap,
            h: cell_h,
        };
        col += span;
        if col >= cols {
            col = 0;
            row += 1;
        }
        if k.space {
            continue;
        }
        let tone = Tone::Swatch(0);
        match k.shape {
            3 => d.push(Decor::Poly {
                pts: vec![[r.x + r.w / 2.0, r.y], [r.right(), r.y + r.h / 2.0], [r.x + r.w / 2.0, r.bottom()], [r.x, r.y + r.h / 2.0]],
                fill: tone,
                stroke: Some(tone),
                alpha: 255,
            }),
            s => d.push(rect(r.x, r.y, r.w, r.h, [4.0, 12.0, r.h / 2.0][s.min(2) as usize], tone, Some(tone))),
        }
        d.push(text(r.center(), &k.label, 13.0, Anchor::Middle, false, Tone::Ink));
        rects.push((k.id.clone(), r));
    }
    for (a, c, label) in &b.edges {
        let (Some((_, ra)), Some((_, rb))) = (rects.iter().find(|(i, _)| i == a), rects.iter().find(|(i, _)| i == c)) else {
            continue;
        };
        let (ca, cb) = (ra.center(), rb.center());
        let (p, q) = (clip(*ra, cb), clip(*rb, ca));
        let dir = [q[0] - p[0], q[1] - p[1]];
        let len = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt().max(1e-3);
        let u = [dir[0] / len, dir[1] / len];
        let n = [-u[1], u[0]];
        let base = [q[0] - u[0] * 10.0, q[1] - u[1] * 10.0];
        d.push(line(vec![p, base], 1.6, false, Tone::Muted));
        d.push(Decor::Poly {
            pts: vec![q, [base[0] + n[0] * 4.5, base[1] + n[1] * 4.5], [base[0] - n[0] * 4.5, base[1] - n[1] * 4.5]],
            fill: Tone::Muted,
            stroke: None,
            alpha: 255,
        });
        if !label.is_empty() {
            d.push(text([(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0 - 9.0], label, 12.0, Anchor::Middle, false, Tone::Ink));
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_shapes_and_edges() {
        let src = "columns 3\n a[\"Alpha beta\"] b:2\n c((Circle)) space d{Choice}\n a --> c\n";
        let mut off = 0;
        let mut body = Vec::new();
        for raw in src.split_inclusive('\n') {
            let text = raw.trim_end_matches('\n');
            body.push((Line { text, offset: off }, text.trim()));
            off += raw.len();
        }
        let b = parse(&body);
        assert_eq!(b.columns, 3);
        assert_eq!(b.blocks.len(), 5);
        assert_eq!(b.blocks[0].label, "Alpha beta");
        assert_eq!(b.blocks[1].span, 2);
        assert_eq!(b.blocks[2].shape, 2);
        assert!(b.blocks[3].space);
        assert_eq!(b.blocks[4].shape, 3);
        assert_eq!(b.edges.len(), 1);
    }
}
