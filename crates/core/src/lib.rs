//! Native repository navigation, source integrity, and local draft persistence.
mod files;
mod io_worker;
mod scanner;
pub mod source;
mod store;
mod watching;

use anyhow::{Context, Result, anyhow, bail};
use directories::ProjectDirs;
use files::{Resource, eligible, hash, identity, naturally_compare, relative_path, valid_relative};
use io_worker::Task;
use mermaidlite_model::*;
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant},
};
use store::{Store, clamp_settings, now, valid_hash};
use watching::Watching;

pub use source::{extract_diagrams, patch_diagram};

pub(crate) enum Internal {
    Core(CoreEvent),
    Cached {
        generation: u64,
        files: Vec<FileEntry>,
    },
    Batch {
        generation: u64,
        files: Vec<FileEntry>,
        scanned: usize,
        metadata: bool,
    },
    Directories {
        generation: u64,
        directories: Vec<PathBuf>,
    },
    ScanDone {
        generation: u64,
        scanned: usize,
        inaccessible: usize,
    },
    Filesystem(Vec<(PathBuf, ChangeKind)>),
    WatchError(String),
    Rescan,
}

/// Where preferences, journals and indexes live. `MERMAIDLITE_DATA_DIR` overrides it (dev and
/// screenshot runs use this so they never touch the real app data).
pub fn default_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("MERMAIDLITE_DATA_DIR").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    ProjectDirs::from("dev", "MermaidLite", "MermaidLite")
        .map(|directories| directories.data_local_dir().to_owned())
        .unwrap_or_else(|| std::env::temp_dir().join("MermaidLite-local"))
}

fn path_key(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    {
        let value = value
            .strip_prefix("//?/UNC/")
            .map(|rest| format!("//{rest}"))
            .unwrap_or_else(|| value.strip_prefix("//?/").unwrap_or(&value).to_owned());
        value.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        value
    }
}

pub struct WorkspaceCore {
    store: Arc<Store>,
    tasks: Sender<Task>,
    sender: Sender<Internal>,
    receiver: Receiver<Internal>,
    wake: Arc<dyn Fn() + Send + Sync>,
    workspace: Option<WorkspaceSnapshot>,
    files: HashMap<String, FileEntry>,
    resources: HashMap<String, Resource>,
    resource_paths: HashMap<String, String>,
    explicit_files: HashSet<PathBuf>,
    watcher: Option<Watching>,
    watching_dirs: HashSet<PathBuf>,
    pending_dirs: VecDeque<PathBuf>,
    scan_cancel: Arc<AtomicBool>,
    generation: u64,
    scan_seen: HashSet<String>,
    pending_events: Vec<CoreEvent>,
    workspace_dirty: bool,
    last_emit: Instant,
    last_scan: Instant,
    degraded: bool,
    rescan_pending: bool,
    fallback_poll: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
}

impl WorkspaceCore {
    pub fn new(data_dir: PathBuf, wake: Arc<dyn Fn() + Send + Sync>) -> Result<Self> {
        let store = Arc::new(Store::new(data_dir)?);
        let (sender, receiver) = mpsc::channel();
        let tasks = io_worker::spawn(store.clone(), sender.clone(), wake.clone())
            .context("Start native file worker")?;
        let fallback_poll = Arc::new(AtomicBool::new(false));
        let shutdown = Arc::new(AtomicBool::new(false));
        let poll_enabled = fallback_poll.clone();
        let stopped = shutdown.clone();
        let tick_sender = sender.clone();
        let tick_wake = wake.clone();
        std::thread::Builder::new()
            .name("mermaidlite-reconcile".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(30));
                    if stopped.load(Ordering::Relaxed) {
                        break;
                    }
                    if poll_enabled.load(Ordering::Relaxed) {
                        if tick_sender.send(Internal::Rescan).is_err() {
                            break;
                        }
                        tick_wake();
                    }
                }
            })
            .context("Start filesystem reconciliation clock")?;
        let mut core = Self {
            store,
            tasks,
            sender,
            receiver,
            wake,
            workspace: None,
            files: HashMap::new(),
            resources: HashMap::new(),
            resource_paths: HashMap::new(),
            explicit_files: HashSet::new(),
            watcher: None,
            watching_dirs: HashSet::new(),
            pending_dirs: VecDeque::new(),
            scan_cancel: Arc::new(AtomicBool::new(false)),
            generation: 0,
            scan_seen: HashSet::new(),
            pending_events: Vec::new(),
            workspace_dirty: false,
            last_emit: Instant::now(),
            last_scan: Instant::now(),
            degraded: false,
            rescan_pending: false,
            fallback_poll,
            shutdown,
        };
        let bootstrap = core.bootstrap();
        for record in bootstrap.recoveries {
            if !record.path.is_absolute() {
                continue;
            }
            let matched = bootstrap.recent_workspaces.iter().find_map(|workspace| {
                relative_path(&workspace.root, &record.path)
                    .ok()
                    .filter(|relative| identity(&workspace.root, relative) == record.file_id)
                    .map(|relative| (workspace.root.clone(), relative))
            });
            let (root, relative) = matched.unwrap_or_else(|| {
                (
                    record.path.parent().unwrap_or(Path::new("/")).to_owned(),
                    record.path.to_string_lossy().into_owned(),
                )
            });
            core.install_resource(
                record.file_id,
                Resource {
                    absolute: record.path,
                    root: Some(root),
                    relative,
                },
            );
        }
        Ok(core)
    }

    pub fn bootstrap(&self) -> BootstrapState {
        let memory = self
            .store
            .memory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        BootstrapState {
            settings: memory.app.settings.clone(),
            recent_workspaces: memory.app.recent_workspaces.clone(),
            last_workspace: memory.app.last_workspace.clone(),
            recoveries: memory.recoveries.values().cloned().collect(),
        }
    }

    fn snapshot(&self) -> WorkspaceSnapshot {
        let mut snapshot = self.workspace.clone().unwrap_or_default();
        snapshot.files = self.files.values().cloned().collect();
        snapshot
            .files
            .sort_by(|a, b| naturally_compare(&a.path, &b.path));
        snapshot
    }

    pub fn open_workspace(&mut self, root: PathBuf) -> Result<WorkspaceSnapshot> {
        let root =
            fs::canonicalize(root).context("This project folder is missing or unavailable")?;
        if !root.is_dir() {
            bail!("Choose a project folder.");
        }
        let id = hash(root.as_os_str().as_encoded_bytes())[..32].to_owned();
        let name = root
            .file_name()
            .unwrap_or(root.as_os_str())
            .to_string_lossy()
            .into_owned();
        {
            let mut memory = self
                .store
                .memory
                .lock()
                .map_err(|_| anyhow!("Workspace state lock failed"))?;
            memory.app.last_workspace = Some(root.clone());
            memory
                .app
                .recent_workspaces
                .retain(|workspace| workspace.root != root);
            memory.app.recent_workspaces.insert(
                0,
                RecentWorkspace {
                    root: root.clone(),
                    name: name.clone(),
                    last_opened: now(),
                },
            );
            memory.app.recent_workspaces.truncate(12);
        }
        self.tasks
            .send(Task::State)
            .context("Workspace persistence worker stopped")?;
        self.files.clear();
        self.workspace = Some(WorkspaceSnapshot {
            id,
            root,
            name,
            files: Vec::new(),
            indexing: true,
            scanned: 0,
            error: None,
        });
        self.start_scan();
        Ok(self.snapshot())
    }

    /// Removes `root` from the recent projects. The folder and its files are not touched.
    pub fn forget_workspace(&mut self, root: &Path) -> Result<()> {
        {
            let mut memory = self
                .store
                .memory
                .lock()
                .map_err(|_| anyhow!("Workspace state lock failed"))?;
            memory
                .app
                .recent_workspaces
                .retain(|workspace| workspace.root != root);
        }
        self.tasks
            .send(Task::State)
            .context("Workspace persistence worker stopped")?;
        Ok(())
    }

    fn install_resource(&mut self, id: String, resource: Resource) {
        self.resource_paths
            .insert(path_key(&resource.absolute), id.clone());
        self.resources.insert(id, resource);
    }

    fn rebuild_watcher(&mut self) {
        self.watcher = None;
        self.watching_dirs.clear();
        self.pending_dirs.clear();
        match Watching::new(self.sender.clone(), self.wake.clone()) {
            Ok(watcher) => {
                self.watcher = Some(watcher);
                self.degraded = false;
                self.fallback_poll.store(false, Ordering::Relaxed);
            }
            Err(error) => {
                self.degraded = true;
                self.fallback_poll.store(true, Ordering::Relaxed);
                self.pending_events.push(CoreEvent::Error(format!("Live watching is unavailable: {error}. The project will reconcile periodically.")));
                return;
            }
        }
        if let Some(workspace) = &self.workspace {
            self.pending_dirs.push_back(workspace.root.clone());
        }
        for path in &self.explicit_files {
            if let Some(parent) = path.parent() {
                self.pending_dirs.push_back(parent.to_owned());
            }
        }
        let recovery_paths = self.active_recoveries();
        for record in recovery_paths {
            if let Some(parent) = record.path.parent() {
                self.pending_dirs.push_back(parent.to_owned());
            }
        }
    }

    fn start_scan(&mut self) {
        let Some(workspace) = self.workspace.as_mut() else {
            return;
        };
        self.scan_cancel.store(true, Ordering::Relaxed);
        self.scan_cancel = Arc::new(AtomicBool::new(false));
        self.generation += 1;
        self.scan_seen.clear();
        workspace.indexing = true;
        workspace.scanned = 0;
        workspace.error = None;
        let root = workspace.root.clone();
        let id = workspace.id.clone();
        let settings = self.bootstrap().settings;
        self.rebuild_watcher();
        self.last_scan = Instant::now();
        self.workspace_dirty = true;
        self.rescan_pending = false;
        if let Err(error) = scanner::spawn(
            root,
            id,
            settings.include_ignored,
            self.generation,
            self.scan_cancel.clone(),
            self.sender.clone(),
            self.wake.clone(),
            self.store.clone(),
        ) {
            if let Some(workspace) = &mut self.workspace {
                workspace.indexing = false;
                workspace.error = Some(format!("Cannot start directory indexing: {error}"));
            }
        }
    }

    pub fn request_read(&mut self, file_id: String) {
        if let Some(resource) = self.resources.get(&file_id).cloned() {
            if self
                .tasks
                .send(Task::Read {
                    id: file_id.clone(),
                    resource,
                })
                .is_err()
            {
                self.pending_events.push(CoreEvent::Document {
                    file_id,
                    result: Err("The native file worker stopped. Reopen the application.".into()),
                });
            }
        } else {
            self.pending_events.push(CoreEvent::Document {
                file_id,
                result: Err(
                    "This file is no longer registered. Open it from the project again.".into(),
                ),
            });
        }
        (self.wake)();
    }

    pub fn open_external(&mut self, path: PathBuf) -> Result<String> {
        if !eligible(&path) {
            bail!("Choose a .md or .mmd file.");
        }
        let canonical = fs::canonicalize(path)?;
        if !canonical.is_file() {
            bail!("Choose a regular Mermaid or Markdown file.");
        }
        let (id, resource) = if let Some(workspace) = &self.workspace {
            if canonical.starts_with(&workspace.root) {
                let relative = relative_path(&workspace.root, &canonical)?;
                (
                    identity(&workspace.root, &relative),
                    Resource {
                        absolute: canonical.clone(),
                        root: Some(workspace.root.clone()),
                        relative,
                    },
                )
            } else {
                (
                    identity(&canonical, "external"),
                    Resource {
                        absolute: canonical.clone(),
                        root: None,
                        relative: canonical.to_string_lossy().into_owned(),
                    },
                )
            }
        } else {
            (
                identity(&canonical, "external"),
                Resource {
                    absolute: canonical.clone(),
                    root: None,
                    relative: canonical.to_string_lossy().into_owned(),
                },
            )
        };
        self.install_resource(id.clone(), resource);
        self.explicit_files.insert(canonical.clone());
        if self.watcher.is_none() {
            self.rebuild_watcher();
        }
        if let Some(parent) = canonical.parent() {
            self.pending_dirs.push_back(parent.to_owned());
        }
        self.request_read(id.clone());
        Ok(id)
    }

    pub fn journal(&mut self, file_id: &str, content: &str, base_hash: &str) -> Result<()> {
        let resource = self
            .resources
            .get(file_id)
            .context("Open this file before editing")?;
        if !valid_hash(base_hash) {
            bail!("The recovery base revision is invalid. Reload or save a copy.");
        }
        self.store.record(RecoveryRecord {
            file_id: file_id.to_owned(),
            path: resource.absolute.clone(),
            content: content.to_owned(),
            base_hash: base_hash.to_owned(),
            updated_at: now(),
        })?;
        self.tasks
            .send(Task::Recovery(file_id.to_owned()))
            .context("The recovery worker stopped")?;
        Ok(())
    }

    pub fn discard_recovery(&mut self, file_id: &str) -> Result<()> {
        {
            let mut memory = self
                .store
                .memory
                .lock()
                .map_err(|_| anyhow!("Recovery store lock failed"))?;
            memory.recoveries.remove(file_id);
            memory.touched.remove(file_id);
        }
        self.tasks
            .send(Task::Recovery(file_id.to_owned()))
            .context("The recovery worker stopped")?;
        Ok(())
    }

    pub fn request_save(&mut self, request: SaveRequest) {
        let file_id = request.file_id.clone();
        let result = self.journal(&file_id, &request.content, &request.expected_hash);
        if let Err(error) = result {
            self.pending_events.push(CoreEvent::Saved {
                file_id,
                result: SaveResult::Failed(format!("{error:#}")),
            });
            (self.wake)();
            return;
        }
        if let Some(resource) = self.resources.get(&file_id).cloned() {
            if self.tasks.send(Task::Save { request, resource }).is_err() {
                self.pending_events.push(CoreEvent::Saved {
                    file_id,
                    result: SaveResult::Failed("The native save worker stopped.".into()),
                });
            }
        }
        (self.wake)();
    }

    pub fn save_copy(&mut self, file_id: String, content: String, destination: PathBuf) {
        if let Some(resource) = self.resources.get(&file_id).cloned() {
            if self
                .tasks
                .send(Task::Copy {
                    id: file_id.clone(),
                    content,
                    destination,
                    resource,
                })
                .is_err()
            {
                self.pending_events.push(CoreEvent::Saved {
                    file_id,
                    result: SaveResult::Failed("The native save worker stopped.".into()),
                });
            }
        } else {
            self.pending_events.push(CoreEvent::Saved {
                file_id,
                result: SaveResult::Failed("Open the source file before saving a copy.".into()),
            });
        }
        (self.wake)();
    }

    pub fn save_now(&mut self, request: SaveRequest) -> SaveResult {
        if let Err(error) = self.flush() {
            return SaveResult::Failed(format!("Flush draft recovery before saving: {error:#}"));
        }
        let Some(resource) = self.resources.get(&request.file_id).cloned() else {
            return SaveResult::Failed("The file is no longer registered.".into());
        };
        io_worker::save(&self.store, &request, &resource)
    }

    pub fn active_recoveries(&self) -> Vec<RecoveryRecord> {
        let memory = self
            .store
            .memory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        memory
            .recoveries
            .iter()
            .filter(|(id, _)| memory.touched.contains(*id))
            .map(|(_, record)| record.clone())
            .collect()
    }

    pub fn save_workspace_state(&mut self, mut state: WorkspaceState) -> Result<()> {
        let Some(workspace) = &self.workspace else {
            return Ok(());
        };
        state.pins.truncate(100);
        if state.history.len() > 150 {
            let removed = state.history.len() - 150;
            state.history.drain(..removed);
            state.history_index = state.history_index.saturating_sub(removed);
        }
        state.history_index = state
            .history_index
            .min(state.history.len().saturating_sub(1));
        while state.views.len() > 500 {
            state.views.pop_first();
        }
        if serde_json::to_vec(&state)?.len() > 2 * 1024 * 1024 {
            bail!("Workspace view state exceeds its 2 MiB limit.");
        }
        self.store
            .memory
            .lock()
            .map_err(|_| anyhow!("Workspace store lock failed"))?
            .app
            .workspaces
            .insert(workspace.id.clone(), state);
        self.tasks
            .send(Task::State)
            .context("The workspace persistence worker stopped")?;
        Ok(())
    }

    pub fn workspace_state(&self) -> Option<WorkspaceState> {
        let workspace = self.workspace.as_ref()?;
        self.store
            .memory
            .lock()
            .ok()?
            .app
            .workspaces
            .get(&workspace.id)
            .cloned()
    }

    pub fn update_settings(&mut self, settings: Settings) -> Result<()> {
        let old = self.bootstrap().settings;
        let settings = clamp_settings(settings);
        let ignored_changed = old.include_ignored != settings.include_ignored;
        self.store
            .memory
            .lock()
            .map_err(|_| anyhow!("Settings store lock failed"))?
            .app
            .settings = settings;
        self.tasks
            .send(Task::State)
            .context("The settings persistence worker stopped")?;
        if ignored_changed {
            self.start_scan();
        }
        Ok(())
    }

    fn apply_files(&mut self, entries: Vec<FileEntry>, authoritative: bool, metadata: bool) {
        let Some(root) = self
            .workspace
            .as_ref()
            .map(|workspace| workspace.root.clone())
        else {
            return;
        };
        for mut entry in entries {
            if !valid_relative(&entry.path)
                || !eligible(Path::new(&entry.path))
                || entry.id != identity(&root, &entry.path)
            {
                continue;
            }
            if authoritative {
                self.scan_seen.insert(entry.id.clone());
            }
            if !metadata {
                if let Some(old) = self.files.get(&entry.id) {
                    if old.modified == entry.modified && old.size == entry.size {
                        entry.diagram_count = old.diagram_count;
                        entry.titles = old.titles.clone();
                    } else if self.resources.contains_key(&entry.id) {
                        self.pending_events.push(CoreEvent::External {
                            file_id: entry.id.clone(),
                            kind: ChangeKind::Changed,
                            disk_hash: None,
                        });
                    }
                }
            }
            self.install_resource(
                entry.id.clone(),
                Resource {
                    absolute: root.join(&entry.path),
                    root: Some(root.clone()),
                    relative: entry.path.clone(),
                },
            );
            self.files.insert(entry.id.clone(), entry);
        }
        self.workspace_dirty = true;
    }

    pub fn poll(&mut self) -> Vec<CoreEvent> {
        let mut rescan = false;
        for _ in 0..256 {
            let Ok(event) = self.receiver.try_recv() else {
                break;
            };
            match event {
                Internal::Core(event) => {
                    self.pending_events.push(event);
                }
                Internal::Cached { generation, files } if generation == self.generation => {
                    self.apply_files(files, false, true);
                }
                Internal::Batch {
                    generation,
                    files,
                    scanned,
                    metadata,
                } if generation == self.generation => {
                    self.apply_files(files, true, metadata);
                    if let Some(workspace) = &mut self.workspace {
                        workspace.scanned = scanned;
                    }
                }
                Internal::Directories {
                    generation,
                    directories,
                } if generation == self.generation => self.pending_dirs.extend(directories),
                Internal::ScanDone {
                    generation,
                    scanned,
                    inaccessible,
                } if generation == self.generation => {
                    let missing: Vec<_> = self
                        .files
                        .keys()
                        .filter(|id| !self.scan_seen.contains(*id))
                        .cloned()
                        .collect();
                    for id in missing {
                        self.files.remove(&id);
                    }
                    if let Some(workspace) = &mut self.workspace {
                        workspace.indexing = false;
                        workspace.scanned = scanned;
                        workspace.error = (inaccessible > 0).then(|| format!("{inaccessible} unreadable, unsupported, or over-limit entries were skipped."));
                    }
                    self.workspace_dirty = true;
                }
                Internal::Filesystem(changes) => {
                    for (path, kind) in changes {
                        let key = path_key(&path);
                        if let Some(id) = self.resource_paths.get(&key) {
                            self.pending_events.push(CoreEvent::External {
                                file_id: id.clone(),
                                kind,
                                disk_hash: None,
                            });
                        }
                        if self.workspace.as_ref().is_some_and(|workspace| {
                            let root = path_key(&workspace.root);
                            key == root || key.starts_with(&(root + "/"))
                        }) {
                            rescan = true;
                        }
                    }
                }
                Internal::Rescan => rescan = true,
                Internal::WatchError(error) => {
                    self.degraded = true;
                    self.fallback_poll.store(true, Ordering::Relaxed);
                    self.pending_events.push(CoreEvent::Error(format!(
                        "Live watching degraded: {error}. Periodic reconciliation remains enabled."
                    )));
                }
                _ => {}
            }
        }
        for _ in 0..48 {
            let Some(directory) = self.pending_dirs.pop_front() else {
                break;
            };
            if !self.watching_dirs.insert(directory.clone()) {
                continue;
            }
            if let Some(watcher) = &mut self.watcher {
                if let Err(error) = watcher.add(&directory) {
                    self.degraded = true;
                    self.fallback_poll.store(true, Ordering::Relaxed);
                    if let Some(workspace) = &mut self.workspace {
                        workspace.error = Some(format!(
                            "A folder cannot be watched: {error}. Periodic reconciliation is enabled."
                        ));
                        self.workspace_dirty = true;
                    }
                }
            }
        }
        if self.degraded && self.last_scan.elapsed() > Duration::from_secs(30) {
            rescan = true;
        }
        self.rescan_pending |= rescan;
        if self.rescan_pending
            && self
                .workspace
                .as_ref()
                .is_some_and(|workspace| !workspace.indexing)
        {
            self.start_scan();
        }
        if self.workspace_dirty
            && (self.last_emit.elapsed() >= Duration::from_millis(50)
                || self
                    .workspace
                    .as_ref()
                    .is_some_and(|workspace| !workspace.indexing))
        {
            self.pending_events
                .push(CoreEvent::Workspace(self.snapshot()));
            self.workspace_dirty = false;
            self.last_emit = Instant::now();
        }
        if !self.pending_dirs.is_empty() || self.workspace_dirty {
            (self.wake)();
        }
        std::mem::take(&mut self.pending_events)
    }

    pub fn flush(&mut self) -> Result<()> {
        let (reply, result) = mpsc::channel();
        self.tasks
            .send(Task::Flush(reply))
            .context("The native persistence worker stopped")?;
        result
            .recv()
            .context("The native persistence worker stopped before flushing")?
            .map_err(anyhow::Error::msg)
    }

    pub fn diagnostics(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            (
                "implementation".into(),
                "native Rust filesystem / pulldown-cmark / notify".into(),
            ),
            (
                "persistence".into(),
                "Atomic JSON state and recovery journals; SQLite deferred".into(),
            ),
            ("source_offsets".into(), "UTF-8 bytes".into()),
            ("source_limit".into(), "16 MiB per editable file".into()),
            ("files".into(), self.files.len().to_string()),
            (
                "watching".into(),
                if self.degraded {
                    "degraded"
                } else {
                    "native, nonrecursive ignore-aware directories"
                }
                .into(),
            ),
        ])
    }
}

impl Drop for WorkspaceCore {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.scan_cancel.store(true, Ordering::Relaxed);
        let _ = self.flush();
        let _ = self.tasks.send(Task::Stop);
    }
}
