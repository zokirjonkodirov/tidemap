use crate::{
    Internal,
    files::{Resource, decode, eligible, hash, read_bounded, resolve},
    source::extract_diagrams,
    store::{EDIT_LIMIT, Store, atomic_write, now, valid_hash},
};
use anyhow::{Context, Result, anyhow, bail};
use tidemap_model::{CoreEvent, DocumentSnapshot, RecoveryRecord, SaveRequest, SaveResult};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Sender},
    },
};

pub(crate) enum Task {
    Read {
        id: String,
        resource: Resource,
    },
    Save {
        request: SaveRequest,
        resource: Resource,
    },
    Copy {
        id: String,
        content: String,
        destination: PathBuf,
        resource: Resource,
    },
    Recovery(String),
    State,
    Flush(Sender<Result<(), String>>),
    Stop,
}

pub(crate) fn spawn(
    store: Arc<Store>,
    sender: Sender<Internal>,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> std::io::Result<Sender<Task>> {
    let (tasks, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("tidemap-files".into())
        .spawn(move || {
            let mut previous = HashMap::<String, Vec<tidemap_model::DiagramBlock>>::new();
            let mut previous_order = Vec::<String>::new();
            let send = |event: CoreEvent| {
                let _ = sender.send(Internal::Core(event));
                wake();
            };
            let mut journal_error: Option<String> = None;
            while let Ok(task) = receiver.recv() {
                match task {
                    Task::Read { id, resource } => {
                        let result = read_document(
                            &store,
                            &id,
                            &resource,
                            previous.get(&id).map(Vec::as_slice).unwrap_or(&[]),
                        );
                        if let Ok(document) = &result {
                            previous.insert(id.clone(), document.blocks.clone());
                            previous_order.retain(|key| key != &id);
                            previous_order.push(id.clone());
                            let mut bytes: usize = previous
                                .values()
                                .flat_map(|blocks| blocks.iter())
                                .map(|block| block.source.len() + block.lines.len() * 64)
                                .sum();
                            while previous_order.len() > 32
                                || (bytes > 32 * 1024 * 1024 && previous_order.len() > 1)
                            {
                                let first = previous_order.remove(0);
                                if let Some(blocks) = previous.remove(&first) {
                                    bytes = bytes.saturating_sub(
                                        blocks
                                            .iter()
                                            .map(|block| {
                                                block.source.len() + block.lines.len() * 64
                                            })
                                            .sum::<usize>(),
                                    );
                                }
                            }
                        }
                        send(CoreEvent::Document {
                            file_id: id,
                            result: result.map_err(|error| format!("{error:#}")),
                        });
                    }
                    Task::Save { request, resource } => {
                        let id = request.file_id.clone();
                        let result = save(&store, &request, &resource);
                        send(CoreEvent::Saved {
                            file_id: id,
                            result,
                        });
                    }
                    Task::Copy {
                        id,
                        content,
                        destination,
                        resource,
                    } => {
                        send(CoreEvent::Saved {
                            file_id: id,
                            result: save_copy(&resource, &content, destination),
                        });
                    }
                    Task::Recovery(id) => {
                        if let Err(error) = store.persist_recovery(&id) {
                            let text = format!("Recovery could not be written: {error:#}");
                            journal_error = Some(text.clone());
                            send(CoreEvent::Error(text));
                        }
                    }
                    Task::State => {
                        if let Err(error) = store.persist_state() {
                            let text = format!("Workspace state could not be written: {error:#}");
                            journal_error = Some(text.clone());
                            send(CoreEvent::Error(text));
                        }
                    }
                    Task::Flush(reply) => {
                        // Retry every accepted in-memory journal so a transient disk
                        // error cannot be hidden by a later successful queue item.
                        let ids = store
                            .memory
                            .lock()
                            .map(|memory| memory.recoveries.keys().cloned().collect::<Vec<_>>());
                        let result = (|| -> Result<()> {
                            for id in ids.map_err(|_| anyhow!("Recovery store lock failed"))? {
                                store.persist_recovery(&id)?;
                            }
                            store.persist_state()?;
                            Ok(())
                        })();
                        if result.is_ok() {
                            journal_error = None;
                        }
                        let result = result
                            .map_err(|error| format!("{error:#}"))
                            .and_then(|_| journal_error.clone().map_or(Ok(()), Err));
                        let _ = reply.send(result);
                    }
                    Task::Stop => break,
                }
            }
        })?;
    Ok(tasks)
}

pub(crate) fn read_document(
    store: &Store,
    id: &str,
    resource: &Resource,
    previous: &[tidemap_model::DiagramBlock],
) -> Result<DocumentSnapshot> {
    let recovery = {
        let mut memory = store
            .memory
            .lock()
            .map_err(|_| anyhow!("Recovery store lock failed"))?;
        let recovery = memory.recoveries.get(id).cloned();
        if recovery.is_some() {
            memory.touched.insert(id.to_owned());
        }
        recovery
    };
    let read = resolve(resource).and_then(|absolute| {
        read_bounded(&absolute, EDIT_LIMIT, resource.root.as_deref()).map(|bytes| (absolute, bytes))
    });
    let (absolute, bytes) = match read {
        Ok(read) => read,
        Err(error) => {
            if let Some(record) = recovery {
                return Ok(DocumentSnapshot {
                    file_id: id.to_owned(),
                    path: resource.relative.clone(),
                    name: resource
                        .absolute
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    content: String::new(),
                    disk_hash: record.base_hash.clone(),
                    read_only: true,
                    disk_missing: true,
                    newline: "\n".into(),
                    blocks: extract_diagrams("", &resource.relative, previous),
                    recovery: Some(record),
                });
            }
            return Err(error);
        }
    };
    let disk_hash = hash(&bytes);
    let content = decode(bytes)?;
    let blocks = extract_diagrams(&content, &resource.relative, previous);
    let metadata = fs::metadata(&absolute)?;
    let newline = if content.contains("\r\n") {
        "\r\n"
    } else if content.contains('\n') {
        "\n"
    } else if content.contains('\r') {
        "\r"
    } else {
        "\n"
    }
    .to_owned();
    Ok(DocumentSnapshot {
        file_id: id.to_owned(),
        path: resource.relative.clone(),
        name: absolute
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        content,
        disk_hash,
        read_only: metadata.permissions().readonly(),
        disk_missing: false,
        newline,
        blocks,
        recovery,
    })
}

pub(crate) fn save(store: &Store, request: &SaveRequest, resource: &Resource) -> SaveResult {
    match save_inner(store, request, resource) {
        Ok((path, disk_hash)) => SaveResult::Saved {
            path,
            disk_hash,
            copy: false,
        },
        Err(error) => {
            if let Some(conflict) = error.downcast_ref::<Conflict>() {
                SaveResult::Conflict {
                    message: conflict.message.clone(),
                    disk_content: conflict.content.clone(),
                    disk_hash: conflict.hash.clone(),
                }
            } else {
                SaveResult::Failed(format!("{error:#}"))
            }
        }
    }
}

#[derive(Debug)]
struct Conflict {
    message: String,
    content: Option<String>,
    hash: Option<String>,
}
impl std::fmt::Display for Conflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}
impl std::error::Error for Conflict {}
fn conflict(bytes: &[u8]) -> anyhow::Error {
    Conflict {
        message: "This file changed on disk. Compare or reload it, or save your edits as a copy."
            .into(),
        content: String::from_utf8(bytes.to_vec()).ok(),
        hash: Some(hash(bytes)),
    }
    .into()
}

fn save_inner(
    store: &Store,
    request: &SaveRequest,
    resource: &Resource,
) -> Result<(PathBuf, String)> {
    if request.content.len() > EDIT_LIMIT || !valid_hash(&request.expected_hash) {
        bail!("Invalid source size or base revision. Reload the file or save a copy.");
    }
    let absolute = resolve(resource)?;
    let metadata = fs::metadata(&absolute)?;
    if metadata.permissions().readonly() {
        bail!("This file is read-only. Save a copy to a writable location.");
    }
    let disk = read_bounded(&absolute, EDIT_LIMIT, resource.root.as_deref())?;
    if hash(&disk) != request.expected_hash {
        return Err(conflict(&disk));
    }
    // The UI's latest accepted draft remains authoritative while an older save
    // is in flight. Never replace that draft with the older outgoing snapshot.
    let has_record = store
        .memory
        .lock()
        .map_err(|_| anyhow!("Recovery store lock failed"))?
        .recoveries
        .contains_key(&request.file_id);
    if !has_record {
        store.record(RecoveryRecord {
            file_id: request.file_id.clone(),
            path: resource.absolute.clone(),
            content: request.content.clone(),
            base_hash: request.expected_hash.clone(),
            updated_at: now(),
        })?;
    }
    store.persist_recovery(&request.file_id)?;
    atomic_write(
        &absolute,
        request.content.as_bytes(),
        Some(metadata.permissions()),
        || {
            let latest_absolute = resolve(resource)?;
            let latest = read_bounded(&latest_absolute, EDIT_LIMIT, resource.root.as_deref())?;
            if latest_absolute != absolute || hash(&latest) != request.expected_hash {
                return Err(conflict(&latest));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                let current = fs::metadata(&latest_absolute)?;
                if current.ino() != metadata.ino() || current.dev() != metadata.dev() {
                    return Err(conflict(&latest));
                }
            }
            Ok(())
        },
    )?;
    let disk_hash = hash(request.content.as_bytes());
    store
        .mark_saved(&request.file_id, &request.content, &disk_hash)
        .context(
            "The file was saved but recovery cleanup failed; the recovery copy remains available",
        )?;
    Ok((absolute, disk_hash))
}

fn save_copy(resource: &Resource, content: &str, destination: PathBuf) -> SaveResult {
    let operation = || -> Result<(PathBuf, String)> {
        if content.len() > EDIT_LIMIT || !destination.is_absolute() || !eligible(&destination) {
            bail!("Choose a .md or .mmd copy destination for source smaller than 16 MiB.");
        }
        let parent = fs::canonicalize(
            destination
                .parent()
                .context("Choose a parent folder for the copy")?,
        )?;
        let target = parent.join(
            destination
                .file_name()
                .context("Choose a filename for the copy")?,
        );
        if target == resource.absolute {
            bail!(
                "Choose another filename for the copy. Saving over the original requires a current disk revision."
            );
        }
        let previous = match fs::symlink_metadata(&target) {
            Ok(metadata) => {
                if metadata.permissions().readonly() {
                    bail!("The copy destination is read-only.");
                }
                Some((
                    hash(&read_bounded(&target, EDIT_LIMIT, None)?),
                    metadata.permissions(),
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        atomic_write(
            &target,
            content.as_bytes(),
            previous
                .as_ref()
                .map(|(_, permissions)| permissions.clone()),
            || {
                match fs::symlink_metadata(&target) {
                    Ok(_) => {
                        let latest = hash(&read_bounded(&target, EDIT_LIMIT, None)?);
                        if previous.as_ref().map(|(hash, _)| hash.as_str()) != Some(latest.as_str())
                        {
                            bail!("The copy destination changed while saving. Choose it again.");
                        }
                    }
                    Err(error)
                        if error.kind() == std::io::ErrorKind::NotFound && previous.is_none() => {}
                    Err(error) => return Err(error.into()),
                }
                Ok(())
            },
        )?;
        Ok((target, hash(content.as_bytes())))
    };
    match operation() {
        Ok((path, disk_hash)) => SaveResult::Saved {
            path,
            disk_hash,
            copy: true,
        },
        Err(error) => SaveResult::Failed(format!("{error:#}")),
    }
}
