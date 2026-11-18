//! Pure construction of the sidebar tree: which rows exist, in which order, at which depth.

use std::{collections::BTreeSet, sync::Arc};

use tidemap_model::FileEntry;

/// Levels of indentation after which nested folders stop moving to the right.
const MAX_INDENT_LEVELS: usize = 6;
/// Horizontal step per level, in points.
const INDENT_STEP: f32 = 14.0;

/// One visible line of the tree.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeRow {
    /// Path relative to the workspace root, `/` separated.
    pub path: String,
    pub name: String,
    pub depth: usize,
    pub directory: bool,
    /// Only meaningful for directories: whether the children are listed.
    pub expanded: bool,
    pub file_id: Option<String>,
    pub count: Option<usize>,
}

/// The last tree that was built, with what it was built from.
pub struct TreeCache {
    pub revision: u64,
    pub query: String,
    pub collapsed: BTreeSet<String>,
    pub rows: Arc<Vec<TreeRow>>,
}

/// Left offset of a row at `depth`: 14 px per level, capped at six levels.
pub fn indent(depth: usize) -> f32 {
    depth.min(MAX_INDENT_LEVELS) as f32 * INDENT_STEP
}

/// A file that passed the query, with its path split into components.
struct Candidate<'a> {
    file: &'a FileEntry,
    parts: Vec<&'a str>,
}

/// Builds the visible rows. Folders come before files at every level, each group sorted
/// case-insensitively. A non-empty `query` keeps only matching files (by path or diagram
/// title) with all their ancestor folders. `collapsed` holds the folded folders; the caller
/// passes a separate set while searching, so folding a folder in the results never disturbs
/// the folding of the plain tree.
pub fn build(files: &[FileEntry], query: &str, collapsed: &BTreeSet<String>) -> Vec<TreeRow> {
    let query = query.trim().to_lowercase();
    let searching = !query.is_empty();
    let candidates: Vec<Candidate> = files
        .iter()
        .filter(|file| {
            !searching
                || file.path.to_lowercase().contains(&query)
                || file.titles.iter().any(|title| title.to_lowercase().contains(&query))
        })
        .map(|file| Candidate {
            file,
            parts: file.path.split('/').collect(),
        })
        .collect();

    let all: Vec<&Candidate> = candidates.iter().collect();
    let mut rows = Vec::new();
    push_level(&all, 0, "", collapsed, &mut rows);
    rows
}

fn sort_key(name: &str) -> (String, String) {
    (name.to_lowercase(), name.to_owned())
}

fn push_level(
    candidates: &[&Candidate],
    depth: usize,
    prefix: &str,
    collapsed: &BTreeSet<String>,
    rows: &mut Vec<TreeRow>,
) {
    let mut folders: Vec<&str> = candidates
        .iter()
        .filter(|candidate| candidate.parts.len() > depth + 1)
        .map(|candidate| candidate.parts[depth])
        .collect();
    folders.sort_by_key(|name| sort_key(name));
    folders.dedup();

    for folder in folders {
        let path = if prefix.is_empty() {
            folder.to_owned()
        } else {
            format!("{prefix}/{folder}")
        };
        let expanded = !collapsed.contains(&path);
        rows.push(TreeRow {
            path: path.clone(),
            name: folder.to_owned(),
            depth,
            directory: true,
            expanded,
            file_id: None,
            count: None,
        });
        if expanded {
            let inside: Vec<&Candidate> = candidates
                .iter()
                .copied()
                .filter(|candidate| candidate.parts.len() > depth + 1 && candidate.parts[depth] == folder)
                .collect();
            push_level(&inside, depth + 1, &path, collapsed, rows);
        }
    }

    let mut files: Vec<&Candidate> = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.parts.len() == depth + 1)
        .collect();
    files.sort_by_key(|candidate| sort_key(&candidate.file.name));
    for candidate in files {
        rows.push(file_row(candidate, depth));
    }
}

fn file_row(candidate: &Candidate, depth: usize) -> TreeRow {
    TreeRow {
        path: candidate.file.path.clone(),
        name: candidate.file.name.clone(),
        depth,
        directory: false,
        expanded: false,
        file_id: Some(candidate.file.id.clone()),
        count: candidate.file.diagram_count,
    }
}
