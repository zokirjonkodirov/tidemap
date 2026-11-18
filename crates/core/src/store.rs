use anyhow::{Context, Result, anyhow, bail};
use tidemap_model::{RecentWorkspace, RecoveryRecord, Settings, WorkspaceState};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, Permissions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const EDIT_LIMIT: usize = 16 * 1024 * 1024;
const RECOVERY_LIMIT: usize = 96 * 1024 * 1024;

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AppState {
    pub schema: u32,
    pub settings: Settings,
    pub recent_workspaces: Vec<RecentWorkspace>,
    pub last_workspace: Option<PathBuf>,
    pub workspaces: BTreeMap<String, WorkspaceState>,
}
impl Default for AppState {
    fn default() -> Self {
        Self {
            schema: 1,
            settings: Settings::default(),
            recent_workspaces: Vec::new(),
            last_workspace: None,
            workspaces: BTreeMap::new(),
        }
    }
}

pub(crate) struct Memory {
    pub app: AppState,
    pub recoveries: BTreeMap<String, RecoveryRecord>,
    pub touched: HashSet<String>,
}

pub(crate) struct Store {
    pub directory: PathBuf,
    pub memory: Mutex<Memory>,
    pub disk_lock: Mutex<()>,
}

impl Store {
    pub fn new(directory: PathBuf) -> Result<Self> {
        fs::create_dir_all(directory.join("recovery"))
            .context("Create local recovery directory")?;
        fs::create_dir_all(directory.join("indexes")).context("Create local index directory")?;
        let state_path = directory.join("state.json");
        let app = match fs::read(&state_path) {
            Ok(bytes) if bytes.len() <= 4 * 1024 * 1024 => {
                match serde_json::from_slice::<AppState>(&bytes) {
                    Ok(mut state) if state.schema == 1 => {
                        state.recent_workspaces.truncate(12);
                        state.settings = clamp_settings(state.settings);
                        state
                    }
                    _ => {
                        let _ = fs::copy(
                            &state_path,
                            directory.join(format!("state.corrupt-{}.json", now())),
                        );
                        AppState::default()
                    }
                }
            }
            _ => AppState::default(),
        };
        let mut recoveries = BTreeMap::new();
        let mut total = 0;
        for entry in fs::read_dir(directory.join("recovery"))?
            .filter_map(std::result::Result::ok)
            .take(512)
        {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".json") else {
                continue;
            };
            if !valid_id(id)
                || entry
                    .file_type()
                    .is_ok_and(|kind| !kind.is_file() || kind.is_symlink())
            {
                continue;
            }
            if entry
                .metadata()
                .is_ok_and(|meta| meta.len() > (EDIT_LIMIT + 65536) as u64)
            {
                continue;
            }
            let Ok(bytes) = fs::read(entry.path()) else {
                continue;
            };
            let Ok(record) = serde_json::from_slice::<RecoveryRecord>(&bytes) else {
                continue;
            };
            if record.file_id != id
                || !valid_hash(&record.base_hash)
                || record.content.len() > EDIT_LIMIT
                || total + record.content.len() > RECOVERY_LIMIT
            {
                continue;
            }
            total += record.content.len();
            recoveries.insert(record.file_id.clone(), record);
        }
        Ok(Self {
            directory,
            memory: Mutex::new(Memory {
                app,
                recoveries,
                touched: HashSet::new(),
            }),
            disk_lock: Mutex::new(()),
        })
    }

    pub fn record(&self, record: RecoveryRecord) -> Result<()> {
        if !valid_id(&record.file_id) || record.content.len() > EDIT_LIMIT {
            bail!("Invalid recovery identity or source larger than 16 MiB.");
        }
        let mut memory = self
            .memory
            .lock()
            .map_err(|_| anyhow!("Recovery store lock failed"))?;
        let previous = memory
            .recoveries
            .get(&record.file_id)
            .map(|item| item.content.len())
            .unwrap_or_default();
        let total: usize = memory
            .recoveries
            .values()
            .map(|item| item.content.len())
            .sum();
        if total - previous + record.content.len() > RECOVERY_LIMIT
            || (!memory.recoveries.contains_key(&record.file_id) && memory.recoveries.len() >= 256)
        {
            bail!(
                "Recovery storage reached its 96 MiB / 256-file limit. Save an existing draft before editing another file."
            );
        }
        memory.touched.insert(record.file_id.clone());
        memory.recoveries.insert(record.file_id.clone(), record);
        Ok(())
    }

    pub fn persist_state(&self) -> Result<()> {
        let _guard = self
            .disk_lock
            .lock()
            .map_err(|_| anyhow!("State disk lock failed"))?;
        let state = self
            .memory
            .lock()
            .map_err(|_| anyhow!("State lock failed"))?
            .app
            .clone();
        atomic_write(
            &self.directory.join("state.json"),
            &serde_json::to_vec_pretty(&state)?,
            None,
            || Ok(()),
        )
    }

    pub fn persist_recovery(&self, id: &str) -> Result<()> {
        if !valid_id(id) {
            bail!("Invalid recovery identity.");
        }
        let _guard = self
            .disk_lock
            .lock()
            .map_err(|_| anyhow!("Recovery disk lock failed"))?;
        let record = self
            .memory
            .lock()
            .map_err(|_| anyhow!("Recovery lock failed"))?
            .recoveries
            .get(id)
            .cloned();
        let path = self.directory.join("recovery").join(format!("{id}.json"));
        if let Some(record) = record {
            atomic_write(&path, &serde_json::to_vec(&record)?, None, || Ok(()))
        } else {
            match fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.into()),
            }
        }
    }

    pub fn mark_saved(&self, id: &str, content: &str, disk_hash: &str) -> Result<()> {
        {
            let mut memory = self
                .memory
                .lock()
                .map_err(|_| anyhow!("Recovery lock failed"))?;
            match memory.recoveries.get_mut(id) {
                Some(record) if record.content != content => {
                    record.base_hash = disk_hash.to_owned();
                }
                Some(_) => {
                    memory.recoveries.remove(id);
                    memory.touched.remove(id);
                }
                None => {}
            }
        }
        self.persist_recovery(id)
    }
}

pub(crate) fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
pub(crate) fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
pub(crate) fn clamp_settings(mut settings: Settings) -> Settings {
    settings.sidebar_width = if settings.sidebar_width.is_finite() {
        settings.sidebar_width.clamp(180.0, 480.0)
    } else {
        258.0
    };
    settings.editor_width = if settings.editor_width.is_finite() {
        settings.editor_width.clamp(240.0, 1000.0)
    } else {
        440.0
    };
    settings
}

/// Flush a temporary sibling and atomically replace the destination. No in-place fallback.
pub(crate) fn atomic_write(
    path: &Path,
    bytes: &[u8],
    permissions: Option<Permissions>,
    before_replace: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let parent = path
        .parent()
        .context("A save destination needs a parent folder")?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".tidemap-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .context("Create temporary save file")?;
    temporary
        .write_all(bytes)
        .context("Write temporary save file")?;
    temporary.as_file_mut().flush()?;
    temporary
        .as_file()
        .sync_all()
        .context("Flush temporary save file")?;
    if let Some(permissions) = permissions {
        temporary.as_file().set_permissions(permissions)?;
    }
    before_replace()?;
    temporary
        .persist(path)
        .map_err(|error| anyhow!("Replace destination atomically: {}", error.error))?;
    #[cfg(unix)]
    {
        if let Ok(directory) = fs::File::open(parent) {
            let _ = directory.sync_all();
        }
    }
    Ok(())
}
