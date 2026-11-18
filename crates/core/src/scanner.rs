use crate::{
    Internal,
    files::{decode, eligible, identity, read_bounded, relative_path},
    source::extract_diagrams,
    store::{EDIT_LIMIT, Store, atomic_write},
};
use ignore::WalkBuilder;
use tidemap_model::FileEntry;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    time::{Instant, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize)]
struct Cache {
    schema: u32,
    root: PathBuf,
    include_ignored: bool,
    files: Vec<FileEntry>,
}
pub(crate) fn generated(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "node_modules"
            | "target"
            | "dist"
            | "build"
            | "coverage"
            | ".next"
            | ".nuxt"
            | ".cache"
            | ".turbo"
            | ".svelte-kit"
            | ".parcel-cache"
            | ".venv"
            | "venv"
    )
}

pub(crate) fn spawn(
    root: PathBuf,
    workspace_id: String,
    include_ignored: bool,
    generation: u64,
    cancel: Arc<AtomicBool>,
    sender: Sender<Internal>,
    wake: Arc<dyn Fn() + Send + Sync>,
    store: Arc<Store>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("tidemap-index".into())
        .spawn(move || {
            let send = |event: Internal| {
                let _ = sender.send(event);
                wake();
            };
            let cache_path = store
                .directory
                .join("indexes")
                .join(format!("{workspace_id}.json"));
            if let Ok(bytes) = read_bounded(&cache_path, 32 * 1024 * 1024, None) {
                if let Ok(cache) = serde_json::from_slice::<Cache>(&bytes) {
                    if cache.schema == 1
                        && cache.root == root
                        && cache.include_ignored == include_ignored
                    {
                        send(Internal::Cached {
                            generation,
                            files: cache.files,
                        });
                    }
                }
            }
            let mut builder = WalkBuilder::new(&root);
            builder
                .hidden(false)
                .follow_links(false)
                .parents(false)
                .require_git(false)
                .git_ignore(!include_ignored)
                .git_exclude(false)
                .git_global(false)
                .ignore(!include_ignored);
            let root_for_filter = root.clone();
            builder.filter_entry(move |entry| {
                if entry.path() == root_for_filter {
                    return true;
                }
                let name = entry.file_name().to_string_lossy();
                !name.eq_ignore_ascii_case(".git")
                    && !(entry.file_type().is_some_and(|kind| kind.is_dir())
                        && !include_ignored
                        && generated(&name))
            });
            let mut files = Vec::new();
            let mut pending = Vec::new();
            let mut directories = Vec::new();
            let mut scanned = 0;
            let mut inaccessible = 0;
            let mut last = Instant::now();
            for entry in builder.build() {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                scanned += 1;
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(_) => {
                        inaccessible += 1;
                        continue;
                    }
                };
                if entry.file_type().is_some_and(|kind| kind.is_symlink()) {
                    continue;
                }
                if entry.file_type().is_some_and(|kind| kind.is_dir()) {
                    directories.push(entry.path().to_owned());
                } else if entry.file_type().is_some_and(|kind| kind.is_file())
                    && eligible(entry.path())
                {
                    let relative = match relative_path(&root, entry.path()) {
                        Ok(relative) => relative,
                        Err(_) => {
                            inaccessible += 1;
                            continue;
                        }
                    };
                    let metadata = match fs::symlink_metadata(entry.path()) {
                        Ok(metadata) => metadata,
                        Err(_) => {
                            inaccessible += 1;
                            continue;
                        }
                    };
                    let file = FileEntry {
                        id: identity(&root, &relative),
                        path: relative,
                        name: entry.file_name().to_string_lossy().into_owned(),
                        size: metadata.len(),
                        modified: metadata
                            .modified()
                            .ok()
                            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                            .map(|time| time.as_millis().min(u64::MAX as u128) as u64)
                            .unwrap_or_default(),
                        diagram_count: None,
                        titles: Vec::new(),
                    };
                    pending.push(file.clone());
                    files.push(file);
                }
                if directories.len() >= 64 {
                    send(Internal::Directories {
                        generation,
                        directories: std::mem::take(&mut directories),
                    });
                }
                if pending.len() >= 64 || last.elapsed().as_millis() >= 80 {
                    send(Internal::Batch {
                        generation,
                        files: std::mem::take(&mut pending),
                        scanned,
                        metadata: false,
                    });
                    last = Instant::now();
                }
            }
            if !directories.is_empty() {
                send(Internal::Directories {
                    generation,
                    directories,
                });
            }
            if !pending.is_empty() {
                send(Internal::Batch {
                    generation,
                    files: pending,
                    scanned,
                    metadata: false,
                });
            }
            let mut metadata_batch = Vec::new();
            for file in &mut files {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let absolute = root.join(Path::new(&file.path));
                if file.size > EDIT_LIMIT as u64 {
                    inaccessible += 1;
                    continue;
                }
                match read_bounded(&absolute, EDIT_LIMIT, Some(&root)).and_then(decode) {
                    Ok(content) => {
                        let blocks = extract_diagrams(&content, &file.path, &[]);
                        file.diagram_count = Some(blocks.len());
                        file.titles = blocks
                            .iter()
                            .take(250)
                            .map(|block| block.title.clone())
                            .collect();
                        metadata_batch.push(file.clone());
                    }
                    Err(_) => {
                        inaccessible += 1;
                    }
                }
                if metadata_batch.len() >= 32 || last.elapsed().as_millis() >= 100 {
                    send(Internal::Batch {
                        generation,
                        files: std::mem::take(&mut metadata_batch),
                        scanned,
                        metadata: true,
                    });
                    last = Instant::now();
                }
            }
            if !metadata_batch.is_empty() {
                send(Internal::Batch {
                    generation,
                    files: metadata_batch,
                    scanned,
                    metadata: true,
                });
            }
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            if let Ok(bytes) = serde_json::to_vec(&Cache {
                schema: 1,
                root,
                include_ignored,
                files,
            }) {
                if let Ok(_guard) = store.disk_lock.lock() {
                    let _ = atomic_write(&cache_path, &bytes, None, || Ok(()));
                }
            }
            send(Internal::ScanDone {
                generation,
                scanned,
                inaccessible,
            });
        })
        .map(|_| ())
}
