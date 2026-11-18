//! Maps between a diagram block's own source (what the engine sees) and the text shown in the editor.
//!
//! Engine spans are UTF-8 byte offsets into `block.source`. The editor addresses text by
//! `char` index into its buffer, which is either `block.source` ("Diagram" mode) or the whole
//! file ("Full file" mode). Both buffers are shown with `\r\n` collapsed to `\n` and without a
//! leading byte order mark. In Markdown a block's lines may carry a prefix (list indent,
//! blockquote marker); the prefix exists in the file but not in `block.source`.

use mermaidlite_diagram::Span;
use mermaidlite_model::DiagramBlock;

/// Length in bytes of the byte order mark that the editor never shows.
fn bom_len(file_content: &str) -> usize {
    if file_content.starts_with('\u{feff}') { 3 } else { 0 }
}

/// Number of editor chars in `text[..byte]`, where `\r\n` is one char.
fn editor_chars_before(text: &str, byte: usize) -> usize {
    let head = text.get(..byte.min(text.len())).unwrap_or("");
    head.chars().count() - head.matches("\r\n").count()
}

/// Byte offset in `text` of the editor char at `index`; `None` past the end of the text.
/// An index equal to the number of chars maps to `text.len()`.
fn byte_of_editor_char(text: &str, index: usize) -> Option<usize> {
    let mut count = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((byte, ch)) = chars.next() {
        if count == index {
            return Some(byte);
        }
        if ch == '\r' && matches!(chars.peek(), Some((_, '\n'))) {
            chars.next();
        }
        count += 1;
    }
    (count == index).then_some(text.len())
}

/// Index of the line of `source` that holds `byte`, and the byte where that line starts.
fn line_of(source: &str, byte: usize) -> (usize, usize) {
    let head = &source.as_bytes()[..byte.min(source.len())];
    let index = head.iter().filter(|b| **b == b'\n').count();
    let start = head.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    (index, start)
}

/// Byte offset in `source` where line `index` starts.
fn line_start(source: &str, index: usize) -> Option<usize> {
    if index == 0 {
        return Some(0);
    }
    source
        .match_indices('\n')
        .nth(index - 1)
        .map(|(at, _)| at + 1)
}

/// File byte offset for a byte offset inside `block.source`. `end` marks an exclusive range end,
/// which belongs to the previous line when it sits exactly at a line start.
fn source_byte_to_file(block: &DiagramBlock, byte: usize, end: bool) -> Option<usize> {
    let source = &block.source;
    let byte = byte.min(source.len());
    let (mut index, mut start) = line_of(source, byte);
    if end && byte == start && index > 0 {
        index -= 1;
        start = line_start(source, index)?;
    }
    let line = block.lines.get(index).or_else(|| block.lines.last())?;
    if line.prefix.contains('\0') {
        return None;
    }
    let body = (line.end - line.start).checked_sub(line.prefix.len())?;
    let offset = if block.lines.get(index).is_some() {
        (byte - start).min(body)
    } else {
        body
    };
    Some(line.start + line.prefix.len() + offset)
}

/// Char range in the editor buffer for `span` of `block`.
///
/// `file_content` is the whole file as stored on disk (BOM and CRLF included).
pub fn block_to_editor_chars(
    block: &DiagramBlock,
    full_file: bool,
    file_content: &str,
    span: Span,
) -> Option<(usize, usize)> {
    let (start, end) = (span.start as usize, span.end as usize);
    if start > end || end > block.source.len() {
        return None;
    }
    if !full_file {
        let source = &block.source;
        if !source.is_char_boundary(start) || !source.is_char_boundary(end) {
            return None;
        }
        return Some((
            editor_chars_before(source, start),
            editor_chars_before(source, end),
        ));
    }
    let from = source_byte_to_file(block, start, false)?;
    let to = source_byte_to_file(block, end, end > start)?.max(from);
    if !file_content.is_char_boundary(from) || !file_content.is_char_boundary(to) {
        return None;
    }
    let bom = bom_len(file_content);
    let shown = &file_content[bom..];
    Some((
        editor_chars_before(shown, from.checked_sub(bom)?),
        editor_chars_before(shown, to.checked_sub(bom)?),
    ))
}

/// Byte offset in `block.source` under the editor char `editor_char`.
/// `None` when the char lies outside the block or inside a line prefix.
pub fn editor_char_to_block_byte(
    block: &DiagramBlock,
    full_file: bool,
    file_content: &str,
    editor_char: usize,
) -> Option<u32> {
    let byte = if full_file {
        let bom = bom_len(file_content);
        let file_byte = bom + byte_of_editor_char(&file_content[bom..], editor_char)?;
        let (index, line) = block
            .lines
            .iter()
            .enumerate()
            .find(|(_, line)| line.start <= file_byte && file_byte <= line.end)?;
        let body_start = line.start + line.prefix.len();
        if line.prefix.contains('\0') || file_byte < body_start {
            return None;
        }
        line_start(&block.source, index)? + (file_byte - body_start)
    } else {
        byte_of_editor_char(&block.source, editor_char)?
    };
    (byte <= block.source.len())
        .then_some(byte)
        .and_then(|byte| u32::try_from(byte).ok())
}
