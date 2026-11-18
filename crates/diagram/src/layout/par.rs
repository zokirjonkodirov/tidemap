//! Structured fork-join over contiguous chunks (`std::thread::scope`).
//!
//! Every helper returns results in item order and reports the error of the lowest
//! failing item, so a parallel run gives exactly what the sequential loop gives.
//! With one worker nothing is spawned and the same code runs on the calling thread.

use std::num::NonZeroUsize;
use std::thread;

use super::LayoutError;

/// How many threads a layout may use, and whether small inputs are kept sequential.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Par {
    threads: usize,
    gated: bool,
}

impl Par {
    /// One thread per available core; levels with little work stay sequential.
    pub fn auto() -> Self {
        Self::with_threads(thread::available_parallelism().map_or(1, NonZeroUsize::get))
    }

    /// At most `threads` threads; levels with little work stay sequential.
    pub fn with_threads(threads: usize) -> Self {
        Self {
            threads: threads.max(1),
            gated: true,
        }
    }

    /// Everything on the calling thread.
    pub fn sequential() -> Self {
        Self::with_threads(1)
    }

    /// Up to `threads` workers at every level whatever the amount of work (tests).
    pub fn forced(threads: usize) -> Self {
        Self {
            threads: threads.max(1),
            gated: false,
        }
    }

    /// Workers for `items` independent tasks totalling `work` units. At most one per
    /// task and per thread; when gated also at most one per `per_worker` units of work,
    /// so that starting a thread is always paid for. One means: run on the calling thread.
    pub(crate) fn workers(self, items: usize, work: usize, per_worker: usize) -> usize {
        let cap = self.threads.min(items);
        let useful = if self.gated {
            cap.min(work / per_worker.max(1))
        } else {
            cap
        };
        useful.max(1)
    }

    /// True when small inputs are kept sequential (false for [`Par::forced`]).
    pub(crate) fn gated(self) -> bool {
        self.gated
    }

    /// Threads each of `workers` concurrent tasks may use for its own inner levels, so
    /// nested levels never run more threads than the outer budget.
    pub(crate) fn share(self, workers: usize) -> Self {
        Self {
            threads: (self.threads / workers.max(1)).max(1),
            ..self
        }
    }
}

/// Splits `items` into at most `workers` contiguous chunks and runs `f(first_index,
/// chunk)` on each, the first chunk on the calling thread. Returns one result per
/// chunk in order, or the error of the first failing chunk. A panic in a worker
/// propagates to the caller.
pub(crate) fn run_chunks<T, R, F>(items: &[T], workers: usize, f: F) -> Result<Vec<R>, LayoutError>
where
    T: Sync,
    R: Send,
    F: Fn(usize, &[T]) -> Result<R, LayoutError> + Sync,
{
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let size = items.len().div_ceil(workers.max(1));
    let chunks: Vec<(usize, &[T])> = items
        .chunks(size)
        .enumerate()
        .map(|(i, c)| (i * size, c))
        .collect();
    let Some((&(first_base, first), rest)) = chunks.split_first() else {
        return Ok(Vec::new());
    };
    if rest.is_empty() {
        return Ok(vec![f(first_base, first)?]);
    }
    thread::scope(|scope| {
        let f = &f;
        // A chunk whose thread cannot be started runs on the calling thread instead.
        let handles: Vec<_> = rest
            .iter()
            .map(|&(base, chunk)| {
                thread::Builder::new()
                    .spawn_scoped(scope, move || f(base, chunk))
                    .ok()
            })
            .collect();
        let mut out = Vec::with_capacity(chunks.len());
        out.push(f(first_base, first)?);
        for (handle, &(base, chunk)) in handles.into_iter().zip(rest) {
            let result = match handle {
                Some(h) => match h.join() {
                    Ok(r) => r,
                    Err(payload) => std::panic::resume_unwind(payload),
                },
                None => f(base, chunk),
            };
            out.push(result?);
        }
        Ok(out)
    })
}

/// Maps `f(index, item)` over `items` on up to `workers` threads. Results are in
/// item order; the error returned is that of the lowest failing index.
pub(crate) fn map_items<T, R, F>(items: &[T], workers: usize, f: F) -> Result<Vec<R>, LayoutError>
where
    T: Sync,
    R: Send,
    F: Fn(usize, &T) -> Result<R, LayoutError> + Sync,
{
    let chunks = run_chunks(items, workers, |base, chunk| {
        chunk
            .iter()
            .enumerate()
            .map(|(i, item)| f(base + i, item))
            .collect::<Result<Vec<R>, LayoutError>>()
    })?;
    Ok(chunks.into_iter().flatten().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_keep_item_order_for_every_worker_count() {
        let items: Vec<u32> = (0..103).collect();
        for workers in 1..=9 {
            let out = map_items(&items, workers, |i, &x| Ok((i, x * 2))).expect("no error");
            let want: Vec<(usize, u32)> = items.iter().map(|&x| (x as usize, x * 2)).collect();
            assert_eq!(out, want, "{workers} workers");
        }
    }

    #[test]
    fn lowest_failing_index_is_reported() {
        let items: Vec<u32> = (0..64).collect();
        for workers in 1..=9 {
            let out = map_items(&items, workers, |i, _| {
                if i == 17 || i == 40 || i == 63 {
                    Err(LayoutError::Budget(i.to_string()))
                } else {
                    Ok(i)
                }
            });
            assert_eq!(
                out,
                Err(LayoutError::Budget("17".into())),
                "{workers} workers"
            );
        }
    }

    #[test]
    fn empty_input_gives_empty_output() {
        let items: [u32; 0] = [];
        assert_eq!(map_items(&items, 4, |_, _| Ok(1u8)), Ok(Vec::new()));
    }

    #[test]
    fn worker_panic_reaches_the_caller() {
        let items: Vec<u32> = (0..8).collect();
        let caught = std::panic::catch_unwind(|| {
            map_items(&items, 4, |i, _| {
                assert!(i != 5, "boom");
                Ok(i)
            })
        });
        assert!(caught.is_err());
    }

    #[test]
    fn worker_policy() {
        // Sequential and single-item cases never use more than the calling thread.
        assert_eq!(Par::sequential().workers(8, 1_000_000, 1), 1);
        assert_eq!(Par::with_threads(8).workers(1, 1_000_000, 1), 1);
        // Gated: one worker per `per_worker` units, capped by items and threads.
        assert_eq!(Par::with_threads(8).workers(8, 299, 150), 1);
        assert_eq!(Par::with_threads(8).workers(8, 300, 150), 2);
        assert_eq!(Par::with_threads(8).workers(3, 1_000_000, 150), 3);
        assert_eq!(Par::with_threads(2).workers(8, 1_000_000, 150), 2);
        // Forced: ignores the amount of work.
        assert_eq!(Par::forced(8).workers(5, 0, 150), 5);
    }

    #[test]
    fn nested_levels_share_the_budget() {
        let outer = Par::with_threads(8);
        assert_eq!(outer.share(1), outer);
        assert_eq!(outer.share(2).threads, 4);
        assert_eq!(outer.share(8).threads, 1);
        assert_eq!(outer.share(100).threads, 1);
    }
}
