//! `gitGraph` diagrams: branches as lanes, commits as dots, merges as joins.

use crate::charts::{line, rect, text};
use crate::decor::{Anchor, Decor, Tone};
use crate::linewise::{Line, split_word};
use crate::parser::clean_label;
use crate::text::Measurer;

const MAX_COMMITS: usize = 5_000;

#[derive(Clone, Debug)]
pub struct Commit {
    pub branch: usize,
    pub id: Option<String>,
    pub tag: Option<String>,
    /// 0 normal, 1 reverse, 2 highlight.
    pub kind: u8,
    pub parents: Vec<usize>,
    pub merge: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Git {
    pub branches: Vec<String>,
    pub commits: Vec<Commit>,
    pub vertical: bool,
}

/// Value of `key: "v"` or `key: v` in `rest`.
fn attr(rest: &str, key: &str) -> Option<String> {
    let at = rest.find(&format!("{key}:"))?;
    let after = rest[at + key.len() + 1..].trim_start();
    if let Some(q) = after.strip_prefix('"') {
        return Some(clean_label(q.split('"').next().unwrap_or("")));
    }
    Some(after.split_whitespace().next().unwrap_or("").to_string())
}

fn kind(rest: &str) -> u8 {
    match attr(rest, "type").as_deref() {
        Some("REVERSE") => 1,
        Some("HIGHLIGHT") => 2,
        _ => 0,
    }
}

pub(crate) fn parse(header_rest: &str, body: &[(Line, &str)]) -> Git {
    let mut g = Git {
        vertical: header_rest.trim_start().starts_with("TB") || header_rest.contains("BT"),
        ..Git::default()
    };
    g.branches.push("main".into());
    let mut head: Vec<Option<usize>> = vec![None];
    let mut cur = 0usize;
    let branch_of = |g: &Git, n: &str| g.branches.iter().position(|b| b == n);
    for (_, text) in body {
        if g.commits.len() >= MAX_COMMITS {
            break;
        }
        let (word, rest) = split_word(text);
        match word {
            "commit" => {
                let c = Commit {
                    branch: cur,
                    id: attr(rest, "id"),
                    tag: attr(rest, "tag"),
                    kind: kind(rest),
                    parents: head[cur].into_iter().collect(),
                    merge: false,
                };
                g.commits.push(c);
                head[cur] = Some(g.commits.len() - 1);
            }
            "branch" => {
                let name = clean_label(rest.split_whitespace().next().unwrap_or("").trim_matches('"'));
                if name.is_empty() {
                    continue;
                }
                let at = match branch_of(&g, &name) {
                    Some(i) => i,
                    None => {
                        g.branches.push(name);
                        head.push(head[cur]);
                        g.branches.len() - 1
                    }
                };
                cur = at;
            }
            "checkout" | "switch" => {
                if let Some(i) = branch_of(&g, rest.trim().trim_matches('"')) {
                    cur = i;
                }
            }
            "merge" => {
                let name = rest.split_whitespace().next().unwrap_or("").trim_matches('"');
                if let Some(other) = branch_of(&g, name) {
                    let mut parents: Vec<usize> = head[cur].into_iter().collect();
                    parents.extend(head[other]);
                    g.commits.push(Commit {
                        branch: cur,
                        id: attr(rest, "id"),
                        tag: attr(rest, "tag"),
                        kind: kind(rest),
                        parents,
                        merge: true,
                    });
                    head[cur] = Some(g.commits.len() - 1);
                }
            }
            "cherry-pick" => {
                let from = attr(rest, "id").unwrap_or_default();
                g.commits.push(Commit {
                    branch: cur,
                    id: None,
                    tag: Some(format!("cherry-pick:{from}")),
                    kind: 0,
                    parents: head[cur].into_iter().collect(),
                    merge: false,
                });
                head[cur] = Some(g.commits.len() - 1);
            }
            _ => {}
        }
    }
    g
}

pub(crate) fn layout(g: &Git, me: &Measurer) -> Vec<Decor> {
    let mut d = Vec::new();
    let label_w = g.branches.iter().map(|b| me.width(b, 13.0)).fold(0.0, f32::max) + 28.0;
    let pos = |idx: usize, lane: usize| -> [f32; 2] {
        if g.vertical {
            [lane as f32 * 90.0 + 50.0, 60.0 + idx as f32 * 50.0]
        } else {
            [label_w + 40.0 + idx as f32 * 58.0, lane as f32 * 56.0 + 60.0]
        }
    };
    let n = g.commits.len().max(1);
    for (bi, name) in g.branches.iter().enumerate() {
        let tone = Tone::Swatch(bi as u8);
        let start = pos(0, bi);
        let end = pos(n - 1, bi);
        if g.vertical {
            d.push(line(vec![[start[0], 40.0], [end[0], end[1] + 20.0]], 1.0, true, Tone::Grid));
            let w = me.width(name, 13.0) + 20.0;
            d.push(rect(start[0] - w / 2.0, 6.0, w, 26.0, 6.0, tone, Some(tone)));
            d.push(text([start[0], 19.0], name, 13.0, Anchor::Middle, true, Tone::Ink));
        } else {
            d.push(line(vec![[label_w + 20.0, start[1]], [end[0] + 24.0, end[1]]], 1.0, true, Tone::Grid));
            d.push(rect(0.0, start[1] - 13.0, label_w, 26.0, 6.0, tone, Some(tone)));
            d.push(text([label_w / 2.0, start[1]], name, 13.0, Anchor::Middle, true, Tone::Ink));
        }
    }
    for (i, c) in g.commits.iter().enumerate() {
        let to = pos(i, c.branch);
        for (k, &p) in c.parents.iter().enumerate() {
            let pc = &g.commits[p];
            let from = pos(p, pc.branch);
            let lane = if c.merge && k == 1 { pc.branch } else { c.branch };
            let tone = Tone::Swatch(lane as u8);
            let pts = if pc.branch == c.branch {
                vec![from, to]
            } else if g.vertical {
                vec![from, [from[0], from[1] + 16.0], [to[0], to[1] - 16.0], to]
            } else {
                vec![from, [from[0] + 16.0, from[1]], [to[0] - 16.0, to[1]], to]
            };
            d.push(line(pts, 2.5, false, tone));
        }
    }
    for (i, c) in g.commits.iter().enumerate() {
        let p = pos(i, c.branch);
        let tone = Tone::Swatch(c.branch as u8);
        match (c.kind, c.merge) {
            (2, _) => d.push(rect(p[0] - 9.0, p[1] - 9.0, 18.0, 18.0, 2.0, tone, Some(Tone::Ink))),
            (_, true) => {
                d.push(Decor::Circle { c: p, r: 10.0, fill: tone, stroke: Some(Tone::Ink) });
                d.push(Decor::Circle { c: p, r: 5.0, fill: Tone::Canvas, stroke: Some(Tone::Ink) });
            }
            _ => d.push(Decor::Circle { c: p, r: 8.5, fill: tone, stroke: Some(Tone::Ink) }),
        }
        if c.kind == 1 {
            d.push(line(vec![[p[0] - 4.5, p[1] - 4.5], [p[0] + 4.5, p[1] + 4.5]], 1.8, false, Tone::Ink));
            d.push(line(vec![[p[0] - 4.5, p[1] + 4.5], [p[0] + 4.5, p[1] - 4.5]], 1.8, false, Tone::Ink));
        }
        if let Some(id) = &c.id {
            if g.vertical {
                d.push(text([p[0] + 16.0, p[1]], id, 11.5, Anchor::Start, false, Tone::Muted));
            } else {
                d.push(text([p[0], p[1] + 24.0], id, 11.5, Anchor::Middle, false, Tone::Muted));
            }
        }
        if let Some(tag) = &c.tag {
            let w = me.width(tag, 11.5) + 14.0;
            let (x, y) = if g.vertical { (p[0] + 16.0, p[1] - 22.0) } else { (p[0] - w / 2.0, p[1] - 36.0) };
            d.push(rect(x, y, w, 20.0, 5.0, Tone::Swatch(3), Some(Tone::Swatch(3))));
            d.push(text([x + w / 2.0, y + 10.0], tag, 11.5, Anchor::Middle, false, Tone::Ink));
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(src: &str) -> Vec<(Line<'_>, &str)> {
        let mut off = 0;
        let mut out = Vec::new();
        for raw in src.split_inclusive('\n') {
            let text = raw.trim_end_matches(['\n', '\r']);
            if !text.trim().is_empty() {
                out.push((Line { text, offset: off }, text.trim()));
            }
            off += raw.len();
        }
        out
    }

    #[test]
    fn branches_merges_and_tags() {
        let b = body("commit\ncommit id: \"A\" tag: \"v1\"\nbranch dev\ncheckout dev\ncommit\ncheckout main\nmerge dev id: \"M\"\ncommit type: REVERSE\n");
        let g = parse("LR:", &b);
        assert_eq!(g.branches, vec!["main", "dev"]);
        assert_eq!(g.commits.len(), 5);
        assert_eq!(g.commits[1].tag.as_deref(), Some("v1"));
        assert_eq!(g.commits[2].parents, vec![1]);
        assert!(g.commits[3].merge);
        assert_eq!(g.commits[3].parents, vec![1, 2]);
        assert_eq!(g.commits[4].kind, 1);
    }
}
