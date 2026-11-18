//! Application contracts. All source offsets and spans are UTF-8 bytes.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    pub include_ignored: bool,
    pub autosave: bool,
    pub show_minimap: bool,
    pub sidebar_width: f32,
    pub editor_width: f32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            include_ignored: false,
            autosave: false,
            show_minimap: false,
            sidebar_width: 258.0,
            editor_width: 440.0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileEntry {
    pub id: String,
    pub path: String,
    pub name: String,
    pub size: u64,
    pub modified: u64,
    pub diagram_count: Option<usize>,
    pub titles: Vec<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    pub id: String,
    pub name: String,
    pub root: PathBuf,
    pub files: Vec<FileEntry>,
    pub indexing: bool,
    pub scanned: usize,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceLine {
    pub start: usize,
    pub end: usize,
    pub prefix: String,
    pub newline: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagramBlock {
    pub id: String,
    pub title: String,
    pub heading: String,
    pub source: String,
    pub start: usize,
    pub end: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub fence_start: usize,
    pub fence_end: usize,
    pub fence: String,
    pub indent: String,
    pub lines: Vec<SourceLine>,
    pub standalone: bool,
    pub closed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryRecord {
    pub file_id: String,
    pub path: PathBuf,
    pub content: String,
    pub base_hash: String,
    pub updated_at: u64,
}
#[derive(Debug, Clone)]
pub struct DocumentSnapshot {
    pub file_id: String,
    pub path: String,
    pub name: String,
    pub content: String,
    pub disk_hash: String,
    pub read_only: bool,
    pub disk_missing: bool,
    pub newline: String,
    pub blocks: Vec<DiagramBlock>,
    pub recovery: Option<RecoveryRecord>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Camera {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewState {
    pub camera: Option<Camera>,
    pub cursor: usize,
    pub scroll: f32,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceState {
    pub active_file_id: Option<String>,
    pub active_path: Option<String>,
    pub active_block_id: Option<String>,
    pub editor_open: bool,
    pub full_file: bool,
    pub pins: Vec<String>,
    pub views: BTreeMap<String, ViewState>,
    pub selected_blocks: BTreeMap<String, String>,
    pub history: Vec<NavigationEntry>,
    pub history_index: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavigationEntry {
    pub file_id: String,
    pub block_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentWorkspace {
    pub root: PathBuf,
    pub name: String,
    pub last_opened: u64,
}
#[derive(Debug, Clone)]
pub struct BootstrapState {
    pub settings: Settings,
    pub recent_workspaces: Vec<RecentWorkspace>,
    pub last_workspace: Option<PathBuf>,
    pub recoveries: Vec<RecoveryRecord>,
}
#[derive(Debug, Clone)]
pub struct SaveRequest {
    pub file_id: String,
    pub content: String,
    pub expected_hash: String,
}
#[derive(Debug, Clone)]
pub enum SaveResult {
    Saved {
        path: PathBuf,
        disk_hash: String,
        copy: bool,
    },
    Conflict {
        message: String,
        disk_content: Option<String>,
        disk_hash: Option<String>,
    },
    Failed(String),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Changed,
    Deleted,
}
#[derive(Debug, Clone)]
pub enum CoreEvent {
    Workspace(WorkspaceSnapshot),
    Document {
        file_id: String,
        result: Result<DocumentSnapshot, String>,
    },
    Saved {
        file_id: String,
        result: SaveResult,
    },
    External {
        file_id: String,
        kind: ChangeKind,
        disk_hash: Option<String>,
    },
    Error(String),
}

/// One finished layout of a diagram, shared between the engine cache and the canvas.
#[derive(Debug, Clone)]
pub struct RenderArtifact {
    pub id: String,
    pub diagram_id: String,
    pub revision: u64,
    pub source_hash: String,
    pub rendered: mermaidlite_diagram::Rendered,
    pub layout_key: u64,
    pub bytes: usize,
    pub duration_us: u64,
}

#[derive(Debug, Clone)]
pub struct RenderRequest {
    pub job_id: u64,
    pub diagram_id: String,
    pub revision: u64,
    pub source: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiagnosticKind {
    Syntax,
    Budget,
    Crash,
    Canceled,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderDiagnostic {
    pub kind: DiagnosticKind,
    pub message: String,
    pub line: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportFormat {
    Svg,
    Png,
}

#[derive(Debug, Clone)]
pub struct ExportRequest {
    pub job_id: u64,
    pub artifact: Arc<RenderArtifact>,
    pub path: PathBuf,
    pub format: ExportFormat,
    pub background: bool,
    pub scale: f32,
    pub dark: bool,
}

#[derive(Debug, Clone)]
pub enum RenderEvent {
    Completed {
        job_id: u64,
        diagram_id: String,
        result: Result<Arc<RenderArtifact>, RenderDiagnostic>,
    },
    Exported {
        job_id: u64,
        result: Result<PathBuf, String>,
    },
}
