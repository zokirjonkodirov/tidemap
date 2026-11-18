//! Timing harness: `cargo native bench`.
//!
//! Builds synthetic diagrams as source text and times parse, layout and scene
//! build separately (best of 5). Targets (parse + layout + scene, release):
//! 200 nodes < 5 ms, 2,000 < 60 ms, 10,000 < 800 ms.
//!
//! The layout column uses every available core. The right-hand stage split (rank,
//! order, position, route, other) comes from a separate single-threaded run, so its
//! parts add up to that run's wall time and show where the work is.

use mermaidlite_diagram::{Measurer, Metrics, Par, Scene, Stage, StageTimes, layout, parse};
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Small deterministic generator (xorshift) so runs are comparable.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn chain(n: usize) -> String {
    let mut s = String::from("flowchart TB\n");
    for i in 1..n {
        let _ = writeln!(s, "N{}[Step {}] --> N{}[Step {}]", i - 1, i - 1, i, i);
    }
    s
}

fn tree(n: usize) -> String {
    let mut s = String::from("flowchart TB\n");
    for i in 1..n {
        let _ = writeln!(
            s,
            "N{}[Node {}] --> N{}[Node {}]",
            (i - 1) / 3,
            (i - 1) / 3,
            i,
            i
        );
    }
    s
}

fn layered_dense(n: usize) -> String {
    let mut rng = Rng(42);
    let mut s = String::from("flowchart TB\n");
    for i in 0..n {
        let rank = i / 20;
        let next_start = (rank + 1) * 20;
        if next_start >= n {
            let _ = writeln!(s, "N{i}[Node {i}]");
            continue;
        }
        let span = 20.min(n - next_start);
        for _ in 0..2 {
            let t = next_start + rng.below(span);
            let _ = writeln!(s, "N{i}[Node {i}] --> N{t}[Node {t}]");
        }
    }
    s
}

fn clustered(n: usize) -> String {
    let mut rng = Rng(7);
    let mut s = String::from("flowchart TB\n");
    for (c, start) in (0..n).step_by(25).enumerate() {
        let end = (start + 25).min(n);
        let _ = writeln!(s, "subgraph C{c}[Group {c}]");
        for i in start + 1..end {
            let _ = writeln!(s, "N{} --> N{i}[Node {i}]", start + (i - start - 1) / 2);
        }
        if end - start == 1 {
            let _ = writeln!(s, "N{start}[Node {start}]");
        }
        s.push_str("end\n");
    }
    for _ in 0..n / 10 {
        let (a, b) = (rng.below(n), rng.below(n));
        if a != b {
            let _ = writeln!(s, "N{a} --> N{b}");
        }
    }
    s
}

fn best_of<T>(runs: usize, mut f: impl FnMut() -> T) -> (f64, T) {
    let mut best = f64::MAX;
    let mut last = None;
    for _ in 0..runs {
        let t = Instant::now();
        let v = f();
        best = best.min(t.elapsed().as_secs_f64() * 1e3);
        last = Some(v);
    }
    (best, last.expect("at least one run"))
}

/// Per-stage milliseconds of the fastest of `runs` timed layouts: rank, order,
/// position, route and the unattributed remainder (text, setup, packing, bounds).
fn stage_split(
    runs: usize,
    parsed: &mermaidlite_diagram::graph::Graph,
    measurer: &Measurer,
    metrics: &Metrics,
    cancel: &AtomicBool,
) -> Result<[f64; 5]> {
    let mut best = f64::MAX;
    let mut split = [0.0; 5];
    for _ in 0..runs {
        let times = StageTimes::default();
        let t = Instant::now();
        layout::layout_with(
            parsed,
            measurer,
            metrics,
            None,
            cancel,
            Some(&times),
            Par::sequential(),
        )?;
        let wall = t.elapsed().as_secs_f64() * 1e3;
        if wall < best {
            best = wall;
            let ms = |s: Stage| times.get(s).as_secs_f64() * 1e3;
            let parts = [
                ms(Stage::Rank),
                ms(Stage::Order),
                ms(Stage::Position),
                ms(Stage::Route),
            ];
            let other = (wall - parts.iter().sum::<f64>()).max(0.0);
            split = [parts[0], parts[1], parts[2], parts[3], other];
        }
    }
    Ok(split)
}

pub fn run() -> Result<()> {
    let font = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/fonts/Inter-Medium.ttf"
    ))?;
    let measurer = Measurer::new(Arc::from(font))?;
    let metrics = Metrics::default();
    let cancel = AtomicBool::new(false);
    type Generator = fn(usize) -> String;
    let generators: [(&str, Generator); 4] = [
        ("chain", chain),
        ("tree", tree),
        ("layered_dense", layered_dense),
        ("clustered", clustered),
    ];
    println!(
        "{:<14}{:>7}{:>9}{:>10}{:>10}{:>10}{:>10}  |{:>8}{:>8}{:>8}{:>8}{:>8}",
        "topology",
        "nodes",
        "edges",
        "parse",
        "layout",
        "scene",
        "total",
        "rank",
        "order",
        "pos",
        "route",
        "other"
    );
    for (name, make) in generators {
        for n in [200usize, 2_000, 10_000] {
            let source = make(n);
            let (parse_ms, parsed) = best_of(5, || parse(&source));
            let (layout_ms, laid) = best_of(5, || {
                layout::layout(&parsed.graph, &measurer, &metrics, None, &cancel)
            });
            let laid = laid?;
            let (scene_ms, scene) = best_of(5, || Scene::build(&parsed.graph, &laid));
            let sp = stage_split(5, &parsed.graph, &measurer, &metrics, &cancel)?;
            println!(
                "{:<14}{:>7}{:>9}{:>8.1}ms{:>8.1}ms{:>8.1}ms{:>8.1}ms  |{:>8.1}{:>8.1}{:>8.1}{:>8.1}{:>8.1}",
                name,
                scene.nodes.len(),
                scene.edges.len(),
                parse_ms,
                layout_ms,
                scene_ms,
                parse_ms + layout_ms + scene_ms,
                sp[0],
                sp[1],
                sp[2],
                sp[3],
                sp[4]
            );
        }
    }
    Ok(())
}
