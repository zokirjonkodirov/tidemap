//! Text measurement and word wrapping using font advance widths.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Measures text with the advance widths of one font (no kerning).
pub struct Measurer {
    font: Arc<[u8]>,
    units_per_em: f32,
    ascii: [u16; 128],
    fallback: u16,
    cache: Mutex<HashMap<char, u16>>,
}

/// Wrapped text and its size.
#[derive(Clone, Debug, PartialEq)]
pub struct TextBlock {
    pub lines: Vec<String>,
    pub width: f32,
    pub height: f32,
}

impl Measurer {
    /// Parses the font and precomputes ASCII advances.
    pub fn new(font: Arc<[u8]>) -> Result<Self, String> {
        let face = ttf_parser::Face::parse(&font, 0).map_err(|e| e.to_string())?;
        let units = face.units_per_em();
        if units == 0 {
            return Err("font has zero units per em".to_string());
        }
        let advance = |c: char| face.glyph_index(c).and_then(|g| face.glyph_hor_advance(g));
        let fallback = advance('?').unwrap_or(units / 2);
        let mut ascii = [0u16; 128];
        for (code, slot) in (0u8..).zip(ascii.iter_mut()) {
            let c = char::from(code);
            *slot = if c.is_control() {
                0
            } else {
                advance(c).unwrap_or(fallback)
            };
        }
        Ok(Self {
            font,
            units_per_em: f32::from(units),
            ascii,
            fallback,
            cache: Mutex::new(HashMap::new()),
        })
    }

    fn advance_units(&self, c: char) -> u16 {
        if let Some(&units) = self.ascii.get(c as usize) {
            return units;
        }
        if c.is_control() {
            return 0;
        }
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(&units) = cache.get(&c) {
            return units;
        }
        let units = ttf_parser::Face::parse(&self.font, 0)
            .ok()
            .and_then(|face| face.glyph_index(c).and_then(|g| face.glyph_hor_advance(g)))
            .unwrap_or(self.fallback);
        cache.insert(c, units);
        units
    }

    /// Width of `text` at `size`: the sum of advances. Missing glyphs use the advance of `?`.
    pub fn width(&self, text: &str, size: f32) -> f32 {
        let units: u32 = text.chars().map(|c| u32::from(self.advance_units(c))).sum();
        units as f32 * size / self.units_per_em
    }

    /// Wraps `text` to `max_width` without splitting words, then balances line lengths.
    pub fn wrap(&self, text: &str, size: f32, line_height: f32, max_width: f32) -> TextBlock {
        let max_width = max_width.max(0.0);
        let space = self.width(" ", size);
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            let words: Vec<(&str, f32)> = paragraph
                .split_whitespace()
                .map(|w| (w, self.width(w, size)))
                .collect();
            if words.is_empty() {
                lines.push(String::new());
                continue;
            }
            let count = greedy_count(&words, space, max_width);
            let mut limit = max_width;
            if count > 1 {
                let mut lo = words.iter().map(|&(_, w)| w).fold(0.0, f32::max);
                let mut hi = max_width;
                for _ in 0..8 {
                    if lo >= hi {
                        break;
                    }
                    let mid = (lo + hi) / 2.0;
                    if greedy_count(&words, space, mid) <= count {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                limit = hi;
            }
            lines.extend(greedy_lines(&words, space, limit));
        }
        let width = lines
            .iter()
            .map(|l| self.width(l, size))
            .fold(0.0, f32::max);
        TextBlock {
            height: lines.len() as f32 * line_height,
            lines,
            width,
        }
    }
}

/// Greedy line breaking; returns the lines as strings.
fn greedy_lines(words: &[(&str, f32)], space: f32, limit: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_width = 0.0;
    for &(word, w) in words {
        if current.is_empty() {
            current.push_str(word);
            current_width = w;
        } else if current_width + space + w <= limit {
            current.push(' ');
            current.push_str(word);
            current_width += space + w;
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
            current_width = w;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Number of lines greedy breaking produces at `limit`.
fn greedy_count(words: &[(&str, f32)], space: f32, limit: f32) -> usize {
    let mut count = 0;
    let mut current_width = 0.0;
    let mut empty = true;
    for &(_, w) in words {
        if empty {
            count += 1;
            current_width = w;
            empty = false;
        } else if current_width + space + w <= limit {
            current_width += space + w;
        } else {
            count += 1;
            current_width = w;
        }
    }
    count
}
