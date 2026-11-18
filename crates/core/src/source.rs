//! CommonMark extraction and reversible, UTF-8-byte source maps.
use anyhow::{Result, bail};
use mermaidlite_model::{DiagramBlock, SourceLine};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

#[derive(Clone)]
struct Line<'a> {
    start: usize,
    end: usize,
    text: &'a str,
    newline: &'a str,
}

fn lines(text: &str) -> Vec<Line<'_>> {
    let mut result = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\r' && bytes[i] != b'\n' {
            i += 1;
            continue;
        }
        let end = i;
        let length = if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            2
        } else {
            1
        };
        result.push(Line {
            start,
            end,
            text: &text[start..end],
            newline: &text[i..i + length],
        });
        i += length;
        start = i;
    }
    if start < text.len() || result.is_empty() {
        result.push(Line {
            start,
            end: text.len(),
            text: &text[start..],
            newline: "",
        });
    }
    result
}

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))[..24].to_owned()
}
fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}
fn source_title(source: &str) -> Option<String> {
    let mut frontmatter = false;
    for (index, line) in source.lines().take(60).enumerate() {
        let trimmed = line.trim();
        if index == 0 && trimmed == "---" {
            frontmatter = true;
            continue;
        }
        if frontmatter && trimmed == "---" {
            frontmatter = false;
        }
        let title = if frontmatter {
            trimmed.strip_prefix("title:")
        } else {
            trimmed
                .strip_prefix("title ")
                .or_else(|| trimmed.strip_prefix("%% title:"))
        };
        if let Some(title) = title {
            let title = title.trim().trim_matches(['\'', '"']);
            if !title.is_empty() {
                return Some(title.to_owned());
            }
        }
    }
    None
}

fn continuation_prefix(prefix: &str) -> String {
    let mut result = String::new();
    let mut chars = prefix.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(character) = chars.next() {
        if matches!(character, '-' | '+' | '*')
            && chars.peek().is_some_and(|next| next.is_whitespace())
        {
            result.push(' ');
        } else if character.is_ascii_digit() {
            let mut run = String::from(character);
            while chars.peek().is_some_and(|next| next.is_ascii_digit()) {
                run.push(chars.next().unwrap());
            }
            if chars.peek().is_some_and(|next| matches!(next, '.' | ')')) {
                run.push(chars.next().unwrap());
                if chars.peek().is_some_and(|next| next.is_whitespace()) {
                    result.extend(std::iter::repeat_n(' ', run.len()));
                } else {
                    result.push_str(&run);
                }
            } else {
                result.push_str(&run);
            }
        } else {
            result.push(character);
        }
    }
    result
}

fn marker(text: &str) -> Option<(usize, String)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' || bytes[i] == b'~' {
            let first = i;
            let character = bytes[i];
            while i < bytes.len() && bytes[i] == character {
                i += 1;
            }
            if i - first >= 3 {
                return Some((first, text[first..i].to_owned()));
            }
        } else {
            i += 1;
        }
    }
    None
}

fn reconcile(blocks: &mut [DiagramBlock], previous: &[DiagramBlock], path: &str) {
    let key = |block: &DiagramBlock| digest(&format!("{}\0{}", block.heading, block.source));
    let mut current: HashMap<String, Vec<usize>> = HashMap::new();
    let mut prior: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, block) in blocks.iter().enumerate() {
        current.entry(key(block)).or_default().push(index);
    }
    for (index, block) in previous.iter().enumerate() {
        prior.entry(key(block)).or_default().push(index);
    }
    let mut used = HashSet::new();
    let mut assigned = HashSet::new();
    let old_ids: HashSet<&str> = previous.iter().map(|block| block.id.as_str()).collect();
    for (content, indices) in &current {
        let Some(old) = prior.get(content) else {
            continue;
        };
        let safe = (indices.len() == 1 && old.len() == 1)
            || (indices.len() == old.len()
                && indices.iter().zip(old).all(|(a, b)| {
                    blocks[*a].start == previous[*b].start && blocks[*a].end == previous[*b].end
                }));
        if safe {
            for (new_index, old_index) in indices.iter().zip(old) {
                blocks[*new_index].id = previous[*old_index].id.clone();
                used.insert(*old_index);
                assigned.insert(previous[*old_index].id.clone());
            }
        }
    }
    let mut current_headings: HashMap<String, Vec<usize>> = HashMap::new();
    let mut old_headings: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, block) in blocks.iter().enumerate() {
        if block.id.is_empty() {
            current_headings
                .entry(block.heading.clone())
                .or_default()
                .push(index);
        }
    }
    for (index, block) in previous.iter().enumerate() {
        if !used.contains(&index) {
            old_headings
                .entry(block.heading.clone())
                .or_default()
                .push(index);
        }
    }
    for (heading, indices) in current_headings {
        if let Some(old) = old_headings.get(&heading) {
            if indices.len() == 1 && old.len() == 1 {
                let id = previous[old[0]].id.clone();
                blocks[indices[0]].id = id.clone();
                assigned.insert(id);
            }
        }
    }
    let file_hash = digest(&path.replace('\\', "/"));
    let mut occurrences: HashMap<String, usize> = HashMap::new();
    for block in blocks {
        if !block.id.is_empty() {
            continue;
        }
        let base = format!("d-{file_hash}-{}", key(block));
        let occurrence = occurrences.entry(base.clone()).or_default();
        loop {
            *occurrence += 1;
            let id = format!("{base}-{occurrence}");
            if !assigned.contains(&id) && !old_ids.contains(id.as_str()) {
                assigned.insert(id.clone());
                block.id = id;
                break;
            }
        }
    }
}

struct Capture {
    range: Range<usize>,
    source: String,
    heading: String,
}

/// Extract only actual CommonMark fenced Mermaid code blocks. All offsets are UTF-8 bytes.
pub fn extract_diagrams(content: &str, path: &str, previous: &[DiagramBlock]) -> Vec<DiagramBlock> {
    let raw = lines(content);
    if path.to_ascii_lowercase().ends_with(".mmd") {
        let start = usize::from(content.starts_with('\u{feff}')) * 3;
        let source = &content[start..];
        return vec![DiagramBlock {
            id: previous
                .first()
                .map(|block| block.id.clone())
                .unwrap_or_else(|| format!("d-{}-standalone", digest(&path.replace('\\', "/")))),
            title: source_title(source).unwrap_or_else(|| {
                path.rsplit(['/', '\\'])
                    .next()
                    .unwrap_or("Diagram")
                    .trim_end_matches(".mmd")
                    .trim_end_matches(".MMD")
                    .to_owned()
            }),
            heading: String::new(),
            source: source.to_owned(),
            start,
            end: content.len(),
            start_line: 1,
            end_line: raw.len(),
            fence_start: start,
            fence_end: content.len(),
            fence: String::new(),
            indent: String::new(),
            lines: raw
                .iter()
                .map(|line| SourceLine {
                    start: line.start,
                    end: line.end,
                    prefix: String::new(),
                    newline: line.newline.to_owned(),
                })
                .collect(),
            standalone: true,
            closed: true,
        }];
    }
    let bom = usize::from(content.starts_with('\u{feff}')) * 3;
    let parsed = &content[bom..];
    let mut headings: Vec<(u8, String)> = Vec::new();
    let mut pending_heading: Option<(u8, String)> = None;
    let mut capture: Option<Capture> = None;
    let mut result = Vec::new();
    for (event, span) in Parser::new_ext(parsed, Options::empty()).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                let level = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                pending_heading = Some((level, String::new()));
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, text)) = pending_heading.take() {
                    while headings.last().is_some_and(|heading| heading.0 >= level) {
                        headings.pop();
                    }
                    headings.push((level, text));
                }
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info)))
                if info
                    .split_whitespace()
                    .next()
                    .is_some_and(|token| token.eq_ignore_ascii_case("mermaid")) =>
            {
                capture = Some(Capture {
                    range: span.start + bom..span.end + bom,
                    source: String::new(),
                    heading: headings
                        .iter()
                        .map(|heading| heading.1.as_str())
                        .collect::<Vec<_>>()
                        .join(" › "),
                });
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some(code) = &mut capture {
                    code.source.push_str(&text);
                } else if let Some((_, heading)) = &mut pending_heading {
                    heading.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some((_, heading)) = &mut pending_heading {
                    heading.push(' ');
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                let Some(code) = capture.take() else {
                    continue;
                };
                let span_end = code.range.end.max(span.end + bom).min(content.len());
                let first = raw
                    .partition_point(|line| line.start <= code.range.start)
                    .saturating_sub(1);
                let after = raw.partition_point(|line| line.start < span_end);
                let opening = &raw[first];
                let Some((marker_at, fence)) = marker(opening.text) else {
                    continue;
                };
                let source = normalize(&code.source);
                let body_rows: Vec<&str> = if source.is_empty() {
                    Vec::new()
                } else {
                    source.split_terminator('\n').collect()
                };
                let closed = after.saturating_sub(first + 1) > body_rows.len();
                let body_after = after.saturating_sub(usize::from(closed));
                let start = raw
                    .get(first + 1)
                    .map(|line| line.start)
                    .unwrap_or(opening.end + opening.newline.len());
                let end = raw
                    .get(body_after)
                    .map(|line| line.start)
                    .unwrap_or(content.len());
                let mapped = (first + 1..body_after)
                    .filter_map(|index| {
                        raw.get(index).map(|line| {
                            let body = body_rows.get(index - first - 1).copied().unwrap_or("");
                            SourceLine {
                                start: line.start,
                                end: line.end,
                                prefix: line.text.strip_suffix(body).unwrap_or("\0").to_owned(),
                                newline: line.newline.to_owned(),
                            }
                        })
                    })
                    .collect();
                result.push(DiagramBlock {
                    id: String::new(),
                    title: source_title(&source)
                        .or_else(|| headings.last().map(|heading| heading.1.clone()))
                        .unwrap_or_else(|| format!("Diagram {}", result.len() + 1)),
                    heading: code.heading,
                    source,
                    start,
                    end,
                    start_line: first + 2,
                    end_line: body_after.max(first + 2),
                    fence_start: opening.start,
                    fence_end: raw
                        .get(after)
                        .map(|line| line.start)
                        .unwrap_or(content.len()),
                    fence,
                    indent: continuation_prefix(&opening.text[..marker_at]),
                    lines: mapped,
                    standalone: false,
                    closed,
                });
            }
            _ => {}
        }
    }
    reconcile(&mut result, previous, path);
    result
}

fn longest_run(source: &str, marker: u8) -> usize {
    let mut longest = 0;
    let mut run = 0;
    for byte in source.bytes() {
        if byte == marker {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    longest
}

/// Preserve all bytes outside the mapped body and any deliberately lengthened fence.
pub fn patch_diagram(content: &str, block: &DiagramBlock, new_source: &str) -> Result<String> {
    if new_source == block.source {
        return Ok(content.to_owned());
    }
    if block.start > block.end
        || block.end > content.len()
        || !content.is_char_boundary(block.start)
        || !content.is_char_boundary(block.end)
    {
        bail!("This source map is stale. Select the diagram again.");
    }
    if block.standalone {
        return Ok(format!("{}{}", &content[..block.start], new_source));
    }
    if block.fence.is_empty() || block.lines.iter().any(|line| line.prefix.contains('\0')) {
        bail!(
            "This fence crosses a tab indentation boundary. Use full-file editing to preserve its Markdown container."
        );
    }
    let normalized_old = normalize(&block.source);
    let old: Vec<&str> = if normalized_old.is_empty() {
        Vec::new()
    } else {
        normalized_old.split_terminator('\n').collect()
    };
    for (index, line) in block.lines.iter().enumerate() {
        let expected = format!("{}{}", line.prefix, old.get(index).copied().unwrap_or(""));
        if content.get(line.start..line.end) != Some(expected.as_str()) {
            bail!("The Markdown source map changed. Select this diagram again before editing.");
        }
    }
    let newline = block
        .lines
        .iter()
        .find_map(|line| (!line.newline.is_empty()).then_some(line.newline.as_str()))
        .unwrap_or_else(|| {
            if content.contains("\r\n") {
                "\r\n"
            } else if content.contains('\n') {
                "\n"
            } else if content.contains('\r') {
                "\r"
            } else {
                "\n"
            }
        });
    let normalized = normalize(new_source);
    let pieces: Vec<&str> = if normalized.is_empty() {
        Vec::new()
    } else {
        normalized.split_terminator('\n').collect()
    };
    let default_prefix = block
        .lines
        .iter()
        .find_map(|line| (!line.prefix.is_empty()).then_some(line.prefix.as_str()))
        .unwrap_or(&block.indent);
    let mut body = String::new();
    for (index, piece) in pieces.iter().enumerate() {
        if index > 0 {
            body.push_str(newline);
        }
        let mapped = block.lines.get(index).map(|line| line.prefix.as_str());
        let prefix = if !piece.is_empty() {
            mapped
                .filter(|prefix| !prefix.is_empty())
                .unwrap_or(default_prefix)
        } else {
            mapped.unwrap_or(default_prefix)
        };
        body.push_str(prefix);
        body.push_str(piece);
    }
    if !pieces.is_empty() && (block.closed || normalized.ends_with('\n')) {
        body.push_str(newline);
    }
    let mut before = content[..block.start].to_owned();
    let mut after = content[block.end..].to_owned();
    let marker_byte = block.fence.as_bytes()[0];
    let longest = longest_run(&normalized, marker_byte);
    if longest >= block.fence.len() {
        let longer = (marker_byte as char).to_string().repeat(longest + 1);
        let opening = content
            .get(block.fence_start..block.start)
            .ok_or_else(|| anyhow::anyhow!("The fence source map is invalid."))?;
        let position = opening.find(&block.fence).ok_or_else(|| {
            anyhow::anyhow!("The Markdown fence changed. Select this diagram again.")
        })?;
        before = format!(
            "{}{}{}{}",
            &content[..block.fence_start],
            &opening[..position],
            longer,
            &opening[position + block.fence.len()..]
        );
        if block.closed {
            let end = after.find(['\r', '\n']).unwrap_or(after.len());
            let closing = &after[..end];
            let (position, old_fence) = marker(closing).ok_or_else(|| {
                anyhow::anyhow!("The closing fence changed. Use full-file editing.")
            })?;
            after = format!(
                "{}{}{}",
                &closing[..position],
                longer,
                &after[position + old_fence.len()..]
            );
        }
    }
    if !body.is_empty() && !before.ends_with(['\n', '\r']) {
        before.push_str(newline);
    }
    Ok(before + &body + &after)
}
