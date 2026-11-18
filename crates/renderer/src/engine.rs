//! In-process diagram engine: one worker thread, latest-wins scheduling,
//! cooperative cancellation and a byte-bounded cache of finished layouts.

use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use mermaidlite_diagram::{LayoutError, Measurer, Metrics, StyleSheet, render};
use mermaidlite_model::{
    DiagnosticKind, ExportFormat, ExportRequest, RenderArtifact, RenderDiagnostic, RenderEvent,
    RenderRequest,
};
use sha2::{Digest, Sha256};

use crate::export;

/// Family name the exported text refers to; the bundled font is registered under it.
const EXPORT_FAMILY: &str = "Inter";

/// Cache budget in bytes.
const CACHE_LIMIT: usize = 96 * 1024 * 1024;
/// Largest accepted diagram source.
const MAX_SOURCE: usize = 2 * 1024 * 1024;
/// Number of per-diagram seeds (previous results) kept for stable node order.
const MAX_SEEDS: usize = 32;

type Wake = Arc<dyn Fn() + Send + Sync>;
type Key = (String, String, u64);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn diagnostic(kind: DiagnosticKind, message: &str) -> RenderDiagnostic {
    RenderDiagnostic {
        kind,
        message: message.to_owned(),
        line: None,
    }
}

fn hash_source(source: &str) -> String {
    Sha256::digest(source.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

struct CacheEntry {
    artifact: Arc<RenderArtifact>,
    tick: u64,
}

/// State shared between the app thread and the worker.
#[derive(Default)]
struct Shared {
    cache: HashMap<Key, CacheEntry>,
    cache_bytes: usize,
    tick: u64,
    /// Most recent artifact per diagram, newest last.
    seeds: Vec<(String, Arc<RenderArtifact>)>,
    last_duration_us: u64,
}

impl Shared {
    fn get(&mut self, key: &Key) -> Option<Arc<RenderArtifact>> {
        self.tick += 1;
        let tick = self.tick;
        self.cache.get_mut(key).map(|entry| {
            entry.tick = tick;
            entry.artifact.clone()
        })
    }

    fn insert(&mut self, key: Key, artifact: Arc<RenderArtifact>) {
        self.tick += 1;
        self.cache_bytes += artifact.bytes;
        let entry = CacheEntry {
            artifact,
            tick: self.tick,
        };
        if let Some(old) = self.cache.insert(key, entry) {
            self.cache_bytes = self.cache_bytes.saturating_sub(old.artifact.bytes);
        }
        // Evict least recently used entries, always keeping the newest one.
        while self.cache_bytes > CACHE_LIMIT && self.cache.len() > 1 {
            let oldest = self
                .cache
                .iter()
                .min_by_key(|(_, e)| e.tick)
                .map(|(k, _)| k.clone());
            let Some(oldest) = oldest else { break };
            if let Some(removed) = self.cache.remove(&oldest) {
                self.cache_bytes = self.cache_bytes.saturating_sub(removed.artifact.bytes);
            }
        }
    }

    fn set_seed(&mut self, diagram_id: &str, artifact: Arc<RenderArtifact>) {
        self.seeds.retain(|(id, _)| id != diagram_id);
        self.seeds.push((diagram_id.to_owned(), artifact));
        if self.seeds.len() > MAX_SEEDS {
            self.seeds.remove(0);
        }
    }

    fn seed(&self, diagram_id: &str) -> Option<Arc<RenderArtifact>> {
        self.seeds
            .iter()
            .find(|(id, _)| id == diagram_id)
            .map(|(_, a)| a.clone())
    }
}

struct Job {
    request: RenderRequest,
    /// Cache key of the request, hashed once when the job is created.
    key: Key,
    metrics: Metrics,
    layout_key: u64,
    foreground: bool,
    cancel: Arc<AtomicBool>,
}

struct Completion {
    job_id: u64,
    diagram_id: String,
    result: Result<Arc<RenderArtifact>, RenderDiagnostic>,
}

/// Runs layout jobs on a background thread and owns the result cache.
pub struct Engine {
    jobs: Option<Sender<Job>>,
    completions: Receiver<Completion>,
    shared: Arc<Mutex<Shared>>,
    pending_jobs: Arc<AtomicUsize>,
    artifact_counter: Arc<AtomicU64>,
    wake: Wake,
    style: StyleSheet,
    /// Cancel flags of every job handed to the worker that may still be running.
    flags: Vec<Arc<AtomicBool>>,
    /// Immediate results (cache hits, rejected sources) delivered on the next poll.
    queued: VecDeque<Completion>,
    latest_foreground: u64,
    font: Arc<[u8]>,
    exports_tx: Sender<RenderEvent>,
    exports_rx: Receiver<RenderEvent>,
    exports_running: Arc<AtomicUsize>,
}

impl Engine {
    /// Starts the worker thread. `font` is the face used for text measurement;
    /// `wake` is called whenever a result is ready.
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>, font: Arc<[u8]>, style: StyleSheet) -> Self {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done_rx) = mpsc::channel::<Completion>();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let pending_jobs = Arc::new(AtomicUsize::new(0));
        let artifact_counter = Arc::new(AtomicU64::new(0));
        let worker = Worker {
            jobs: job_rx,
            done: done_tx,
            shared: shared.clone(),
            pending: pending_jobs.clone(),
            counter: artifact_counter.clone(),
            wake: wake.clone(),
            font: font.clone(),
        };
        let (exports_tx, exports_rx) = mpsc::channel();
        // If the thread cannot start, jobs are never answered; the app keeps its last picture.
        let _ = std::thread::Builder::new()
            .name("diagram-engine".into())
            .spawn(move || worker.run());
        Self {
            jobs: Some(job_tx),
            completions: done_rx,
            shared,
            pending_jobs,
            artifact_counter,
            wake,
            style: style.clamped(),
            flags: Vec::new(),
            queued: VecDeque::new(),
            latest_foreground: 0,
            font,
            exports_tx,
            exports_rx,
            exports_running: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn cancel_flags(&mut self) {
        for flag in self.flags.drain(..) {
            flag.store(true, Ordering::Relaxed);
        }
    }

    fn key(&self, diagram_id: &str, source: &str) -> Key {
        (
            diagram_id.to_owned(),
            hash_source(source),
            self.style.layout_key(),
        )
    }

    fn dispatch(&mut self, request: RenderRequest, key: Key, foreground: bool) {
        let cancel = Arc::new(AtomicBool::new(false));
        self.flags.retain(|f| Arc::strong_count(f) > 1);
        self.flags.push(cancel.clone());
        let job = Job {
            request,
            key,
            metrics: self.style.metrics.clone(),
            layout_key: self.style.layout_key(),
            foreground,
            cancel,
        };
        self.pending_jobs.fetch_add(1, Ordering::SeqCst);
        let sent = self.jobs.as_ref().is_some_and(|tx| tx.send(job).is_ok());
        if !sent {
            self.pending_jobs.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Starts a foreground job: cancels the running job and replaces any queued one.
    pub fn render(&mut self, request: RenderRequest) {
        self.cancel_flags();
        self.latest_foreground = request.job_id;
        if request.source.len() > MAX_SOURCE {
            self.queued.push_back(Completion {
                job_id: request.job_id,
                diagram_id: request.diagram_id,
                result: Err(diagnostic(
                    DiagnosticKind::Budget,
                    "Diagram source exceeds 2 MiB.",
                )),
            });
            (self.wake)();
            return;
        }
        let key = self.key(&request.diagram_id, &request.source);
        if let Some(artifact) = lock(&self.shared).get(&key) {
            self.queued.push_back(Completion {
                job_id: request.job_id,
                diagram_id: request.diagram_id,
                result: Ok(artifact),
            });
            (self.wake)();
            return;
        }
        self.dispatch(request, key, true);
    }

    /// Fills the cache when the engine is idle. Dropped when a foreground job arrives; emits no event.
    pub fn prefetch(&mut self, request: RenderRequest) {
        if request.source.len() > MAX_SOURCE {
            return;
        }
        let key = self.key(&request.diagram_id, &request.source);
        if lock(&self.shared).get(&key).is_some() {
            return;
        }
        self.dispatch(request, key, false);
    }

    /// Cached result for this exact source, if any.
    pub fn cached(&self, diagram_id: &str, source: &str) -> Option<Arc<RenderArtifact>> {
        let key = self.key(diagram_id, source);
        lock(&self.shared).get(&key)
    }

    /// Applies a new style sheet; a change of layout metrics clears the cache.
    pub fn set_style(&mut self, style: StyleSheet) {
        let style = style.clamped();
        let changed = style.layout_key() != self.style.layout_key();
        self.style = style;
        if changed {
            let mut shared = lock(&self.shared);
            shared.cache.clear();
            shared.cache_bytes = 0;
        }
    }

    /// Exports a finished diagram on a short-lived thread. The outcome arrives from `poll` as
    /// `RenderEvent::Exported`. The engine's current style sheet is used.
    pub fn export(&mut self, request: ExportRequest) {
        let job_id = request.job_id;
        let style = self.style.clone();
        let font = self.font.clone();
        let tx = self.exports_tx.clone();
        let wake = self.wake.clone();
        let running = self.exports_running.clone();
        running.fetch_add(1, Ordering::SeqCst);
        let failed_tx = self.exports_tx.clone();
        let spawned = std::thread::Builder::new()
            .name("diagram-export".into())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    run_export(&request, &style, &font)
                }))
                .unwrap_or_else(|_| Err("The export hit an internal error.".to_owned()));
                let _ = tx.send(RenderEvent::Exported { job_id, result });
                running.fetch_sub(1, Ordering::SeqCst);
                wake();
            });
        if spawned.is_err() {
            self.exports_running.fetch_sub(1, Ordering::SeqCst);
            let _ = failed_tx.send(RenderEvent::Exported {
                job_id,
                result: Err("The export could not be started.".to_owned()),
            });
            (self.wake)();
        }
    }

    /// Cancels every running and queued job.
    pub fn cancel(&mut self) {
        self.cancel_flags();
        self.latest_foreground = 0;
    }

    /// Collects finished results. Results of superseded foreground jobs are dropped.
    pub fn poll(&mut self) -> Vec<RenderEvent> {
        let mut events = Vec::new();
        let mut push = |c: Completion, latest: u64| {
            if c.job_id == latest {
                events.push(RenderEvent::Completed {
                    job_id: c.job_id,
                    diagram_id: c.diagram_id,
                    result: c.result,
                });
            }
        };
        let latest = self.latest_foreground;
        while let Some(c) = self.queued.pop_front() {
            push(c, latest);
        }
        while let Ok(c) = self.completions.try_recv() {
            push(c, latest);
        }
        events.extend(self.exports_rx.try_iter());
        events
    }

    /// True while a job (foreground or prefetch) is queued or running.
    pub fn is_busy(&self) -> bool {
        self.pending_jobs.load(Ordering::SeqCst) > 0
            || !self.queued.is_empty()
            || self.exports_running.load(Ordering::SeqCst) > 0
    }

    /// Cache statistics for the diagnostics dialog.
    pub fn diagnostics(&self) -> serde_json::Value {
        let shared = lock(&self.shared);
        serde_json::json!({
            "cache_bytes": shared.cache_bytes,
            "cache_entries": shared.cache.len(),
            "last_duration_us": shared.last_duration_us,
            "artifacts_built": self.artifact_counter.load(Ordering::Relaxed),
        })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.cancel_flags();
        // Closing the channel ends the worker loop once the current job stops.
        self.jobs = None;
    }
}

/// Renders and writes one export file.
fn run_export(
    request: &ExportRequest,
    style: &StyleSheet,
    font: &[u8],
) -> Result<std::path::PathBuf, String> {
    let artifact = &request.artifact;
    let bytes = match request.format {
        ExportFormat::Svg => export::svg_bytes(
            artifact,
            style,
            request.dark,
            request.background,
            EXPORT_FAMILY,
            font,
        ),
        ExportFormat::Png => export::png_bytes(
            artifact,
            style,
            request.dark,
            request.background,
            request.scale,
            EXPORT_FAMILY,
            font,
        )?,
    };
    export::write_export(&request.path, &bytes)
}

struct Worker {
    jobs: Receiver<Job>,
    done: Sender<Completion>,
    shared: Arc<Mutex<Shared>>,
    pending: Arc<AtomicUsize>,
    counter: Arc<AtomicU64>,
    wake: Wake,
    font: Arc<[u8]>,
}

impl Worker {
    fn run(self) {
        let measurer = Measurer::new(self.font.clone());
        let mut queue: VecDeque<Job> = VecDeque::new();
        loop {
            if queue.is_empty() {
                match self.jobs.recv() {
                    Ok(job) => queue.push_back(job),
                    Err(_) => return,
                }
            }
            while let Ok(job) = self.jobs.try_recv() {
                queue.push_back(job);
            }
            // Latest foreground wins; it also replaces every queued prefetch.
            if let Some(last) = queue.iter().rposition(|j| j.foreground) {
                let keep = queue.remove(last);
                let dropped = queue.len();
                queue.clear();
                self.pending.fetch_sub(dropped, Ordering::SeqCst);
                queue.extend(keep);
            }
            let Some(job) = queue.pop_front() else {
                continue;
            };
            if !job.cancel.load(Ordering::Relaxed) {
                self.process(&job, measurer.as_ref());
            }
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
    }

    fn process(&self, job: &Job, measurer: Result<&Measurer, &String>) {
        let request = &job.request;
        let key = job.key.clone();
        let result = match measurer {
            Err(_) => Some(Err(diagnostic(
                DiagnosticKind::Crash,
                "The diagram font could not be loaded.",
            ))),
            Ok(measurer) => self.build(job, measurer, key),
        };
        let Some(result) = result else { return };
        if job.foreground {
            let _ = self.done.send(Completion {
                job_id: request.job_id,
                diagram_id: request.diagram_id.clone(),
                result,
            });
            (self.wake)();
        }
    }

    /// Renders one job. `None` means it was canceled and nothing is reported.
    fn build(
        &self,
        job: &Job,
        measurer: &Measurer,
        key: Key,
    ) -> Option<Result<Arc<RenderArtifact>, RenderDiagnostic>> {
        let request = &job.request;
        let seed = lock(&self.shared).seed(&request.diagram_id);
        let started = Instant::now();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            render(
                &request.source,
                measurer,
                &job.metrics,
                seed.as_ref()
                    .map(|a| (&a.rendered.graph, &a.rendered.layout)),
                &job.cancel,
            )
        }));
        match outcome {
            Err(_) => Some(Err(diagnostic(
                DiagnosticKind::Crash,
                "The diagram engine hit an internal error.",
            ))),
            Ok(Err(LayoutError::Canceled)) => None,
            Ok(Err(LayoutError::Budget(message))) => {
                Some(Err(diagnostic(DiagnosticKind::Budget, &message)))
            }
            Ok(Ok(rendered)) => {
                let duration_us = started.elapsed().as_micros() as u64;
                let id = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
                let artifact = Arc::new(RenderArtifact {
                    id: format!("{}:{id}", &key.1[..12]),
                    diagram_id: request.diagram_id.clone(),
                    revision: request.revision,
                    source_hash: key.1.clone(),
                    layout_key: job.layout_key,
                    bytes: rendered.scene.byte_size() + request.source.len(),
                    duration_us,
                    rendered,
                });
                let mut shared = lock(&self.shared);
                shared.last_duration_us = duration_us;
                shared.set_seed(&request.diagram_id, artifact.clone());
                shared.insert(key, artifact.clone());
                Some(Ok(artifact))
            }
        }
    }
}
