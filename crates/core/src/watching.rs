use crate::Internal;
use tidemap_model::ChangeKind;
use notify::{
    Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
    event::{ModifyKind, RenameMode},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError, Sender},
    },
    time::{Duration, Instant},
};

pub(crate) struct Watching {
    watcher: RecommendedWatcher,
    stop: Arc<AtomicBool>,
}
impl Watching {
    pub fn new(
        sender: Sender<Internal>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> notify::Result<Self> {
        let (changes, receiver) = mpsc::channel::<notify::Result<notify::Event>>();
        let watcher = RecommendedWatcher::new(
            move |event| {
                let _ = changes.send(event);
            },
            Config::default(),
        )?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        std::thread::Builder::new()
            .name("tidemap-watch".into())
            .spawn(move || {
                let mut pending = BTreeMap::<PathBuf, ChangeKind>::new();
                let mut since = Instant::now();
                while !stopped.load(Ordering::Relaxed) {
                    let timeout = if pending.is_empty() {
                        Duration::from_millis(500)
                    } else {
                        Duration::from_millis(180).saturating_sub(since.elapsed())
                    };
                    match receiver.recv_timeout(timeout) {
                        Ok(Ok(event)) => {
                            if matches!(event.kind, EventKind::Access(_)) {
                                continue;
                            }
                            if pending.is_empty() {
                                since = Instant::now();
                            }
                            for (index, path) in event.paths.into_iter().enumerate() {
                                let name = path.file_name().unwrap_or_default().to_string_lossy();
                                if (name.starts_with(".tidemap-") && name.ends_with(".tmp"))
                                    || path.components().any(|component| {
                                        component.as_os_str().eq_ignore_ascii_case(".git")
                                    })
                                {
                                    continue;
                                }
                                let deleted = matches!(
                                    event.kind,
                                    EventKind::Remove(_)
                                        | EventKind::Modify(ModifyKind::Name(RenameMode::From))
                                ) || (matches!(
                                    event.kind,
                                    EventKind::Modify(ModifyKind::Name(RenameMode::Both))
                                ) && index == 0);
                                pending.insert(
                                    path,
                                    if deleted {
                                        ChangeKind::Deleted
                                    } else {
                                        ChangeKind::Changed
                                    },
                                );
                            }
                            if pending.len() > 5000 {
                                pending.clear();
                                let _ = sender.send(Internal::Rescan);
                                wake();
                            }
                        }
                        Ok(Err(error)) => {
                            let _ = sender.send(Internal::WatchError(error.to_string()));
                            wake();
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            if !pending.is_empty() {
                                let _ = sender.send(Internal::Filesystem(
                                    std::mem::take(&mut pending).into_iter().collect(),
                                ));
                                wake();
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                    if !pending.is_empty() && since.elapsed() >= Duration::from_millis(180) {
                        let _ = sender.send(Internal::Filesystem(
                            std::mem::take(&mut pending).into_iter().collect(),
                        ));
                        wake();
                    }
                }
            })
            .map_err(|error| {
                notify::Error::generic(&format!("Cannot start the watcher event worker: {error}"))
            })?;
        Ok(Self { watcher, stop })
    }
    pub fn add(&mut self, path: &Path) -> notify::Result<()> {
        self.watcher.watch(path, RecursiveMode::NonRecursive)
    }
}
impl Drop for Watching {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
