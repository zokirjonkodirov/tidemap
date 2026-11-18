//! Optional per-stage timing sink for benchmarks. Normal runs pass `None` and pay
//! nothing: no clock is read and no atomic is touched.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Pipeline stage a measurement is attributed to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Cycle removal, cluster nesting, ranking, dummy insertion.
    Rank,
    /// Crossing minimisation.
    Order,
    /// Coordinate assignment (including the alternative-order search).
    Position,
    /// Cluster rectangles and edge routing.
    Route,
}

impl Stage {
    const COUNT: usize = 4;

    fn index(self) -> usize {
        self as usize
    }
}

/// Accumulated time per stage. Shared by reference, so concurrent workers add to the
/// same totals (the sum is CPU time across threads, not wall time).
#[derive(Debug, Default)]
pub struct StageTimes {
    nanos: [AtomicU64; Stage::COUNT],
}

impl StageTimes {
    /// Total time recorded for `stage`.
    pub fn get(&self, stage: Stage) -> Duration {
        Duration::from_nanos(self.nanos[stage.index()].load(Ordering::Relaxed))
    }

    /// Runs `f`, charging its duration to `stage` when a sink is present.
    pub(crate) fn time<T>(times: Option<&Self>, stage: Stage, f: impl FnOnce() -> T) -> T {
        let Some(sink) = times else {
            return f();
        };
        let start = Instant::now();
        let out = f();
        let ns = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        sink.nanos[stage.index()].fetch_add(ns, Ordering::Relaxed);
        out
    }
}
