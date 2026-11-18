//! Golden-snapshot test for the layout engine.
//!
//! Every output coordinate is stored as an exact f32 bit pattern, so any change in
//! layout results (even one ulp) fails. Regenerate with `UPDATE_GOLDEN=1`.

use tidemap_diagram::{Graph, Layout, LayoutError, Measurer, Metrics, Par, layout, parse};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// Number of random graphs in the snapshot set.
const RANDOM_CASES: u64 = 120;
/// Seeds whose graph makes the original layout panic (index -1 in `order::init_order`);
/// left out so the snapshot set stays valid for both old and new code.
const PANICKING_SEEDS: [u64; 1] = [16];
/// Seeds of random graphs on which the first layout attempt fails (`forced_failed`) and
/// the pipeline retries with chains outside the clusters; they cover that path.
const RETRY_SEEDS: [u64; 12] = [
    168, 585, 799, 968, 1142, 1149, 1513, 1872, 1900, 1914, 2213, 2321,
];

/// Small deterministic generator (xorshift), copied from the bench.
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

/// Random flowchart with nested subgraphs, cluster edges, labels, cycles, self-loops
/// and per-subgraph directions. Deterministic in `seed`.
fn random_graph(seed: u64) -> String {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    for _ in 0..4 {
        rng.next();
    }
    let n = 6 + rng.below(45);
    let k = rng.below(7);
    // Cluster parent: none or an earlier cluster (so the nesting is a forest).
    let parent: Vec<Option<usize>> = (0..k)
        .map(|c| (c > 0 && rng.below(3) == 0).then(|| rng.below(c)))
        .collect();
    let dirs = ["TB", "LR", "BT", "RL"];
    let cdir: Vec<Option<&str>> = (0..k)
        .map(|_| (rng.below(5) == 0).then(|| dirs[rng.below(4)]))
        .collect();
    // Node home: top level (k) or a cluster.
    let home: Vec<usize> = (0..n)
        .map(|_| {
            if k == 0 || rng.below(4) == 0 {
                k
            } else {
                rng.below(k)
            }
        })
        .collect();
    let mut s = format!("flowchart {}\n", dirs[rng.below(4)]);
    for top in (0..=k).filter(|&c| c == k || parent[c].is_none()) {
        emit_cluster(top, k, &parent, &cdir, &home, &mut s);
    }
    let edges = n + n / 3 + rng.below(n / 2 + 1);
    let arrows = ["-->", "---", "-.->", "==>"];
    for _ in 0..edges {
        let end = |rng: &mut Rng| {
            if k > 0 && rng.below(12) == 0 {
                format!("C{}", rng.below(k))
            } else {
                format!("N{}", rng.below(n))
            }
        };
        let (a, b) = (end(&mut rng), end(&mut rng));
        let arrow = arrows[rng.below(4)];
        if rng.below(6) == 0 {
            let _ = writeln!(s, "{a} {arrow}|lbl {}| {b}", rng.below(99));
        } else {
            let _ = writeln!(s, "{a} {arrow} {b}");
        }
    }
    s
}

/// Emits cluster `c` (or the top level when `c == k`) with its members and children.
fn emit_cluster(
    c: usize,
    k: usize,
    parent: &[Option<usize>],
    cdir: &[Option<&str>],
    home: &[usize],
    s: &mut String,
) {
    if c < k {
        let _ = writeln!(s, "subgraph C{c}[Group {c}]");
        if let Some(d) = cdir[c] {
            let _ = writeln!(s, "direction {d}");
        }
    }
    for (v, &h) in home.iter().enumerate() {
        if h == c {
            let _ = writeln!(s, "N{v}[Node {v}]");
        }
    }
    for child in (0..k).filter(|&x| parent[x] == Some(c)) {
        emit_cluster(child, k, parent, cdir, home, s);
    }
    if c < k {
        s.push_str("end\n");
    }
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn measurer() -> Measurer {
    let font = std::fs::read(manifest_dir().join("../../assets/fonts/Inter-Medium.ttf"))
        .expect("font file");
    Measurer::new(Arc::from(font)).expect("font parses")
}

fn bits(v: f32) -> String {
    format!("{:08x}", v.to_bits())
}

fn rect_line(tag: &str, i: usize, r: &tidemap_diagram::Rect) -> String {
    format!(
        "{tag} {i} {} {} {} {}",
        bits(r.x),
        bits(r.y),
        bits(r.w),
        bits(r.h)
    )
}

/// Serialises every output coordinate of `l` as one line per item.
fn serialise(l: &Layout) -> String {
    let mut out = String::new();
    for (i, r) in l.nodes.iter().enumerate() {
        let _ = writeln!(out, "{}", rect_line("node", i, r));
    }
    for (i, r) in l.clusters.iter().enumerate() {
        let _ = writeln!(out, "{}", rect_line("cluster", i, r));
    }
    for (i, e) in l.edges.iter().enumerate() {
        let _ = write!(out, "edge {i} n={}", e.points.len());
        for p in &e.points {
            let _ = write!(out, " {},{}", bits(p[0]), bits(p[1]));
        }
        out.push('\n');
        match &e.label {
            Some(r) => {
                let _ = writeln!(out, "{}", rect_line("edgelabel", i, r));
            }
            None => {
                let _ = writeln!(out, "edgelabel {i} none");
            }
        }
    }
    let _ = writeln!(out, "{}", rect_line("bounds", 0, &l.bounds));
    for (i, s) in l.slots.iter().enumerate() {
        let _ = writeln!(out, "slot {i} {} {}", s.0, s.1);
    }
    out
}

/// All cases: fixtures plus generated graphs.
fn cases() -> Vec<(String, String)> {
    let mut v = Vec::new();
    let dir = manifest_dir().join("tests/fixtures");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("fixtures dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "mmd"))
        .collect();
    files.sort();
    for f in files {
        let name = f.file_stem().expect("stem").to_string_lossy().into_owned();
        v.push((
            format!("fixture_{name}"),
            std::fs::read_to_string(&f).expect("fixture"),
        ));
    }
    for seed in (0..RANDOM_CASES).filter(|s| !PANICKING_SEEDS.contains(s)) {
        v.push((format!("rand_{seed:03}"), random_graph(seed)));
    }
    for seed in RETRY_SEEDS {
        v.push((format!("retry_{seed}"), random_graph(seed)));
    }
    type Generator = fn(usize) -> String;
    let gens: [(&str, Generator); 4] = [
        ("chain", chain),
        ("tree", tree),
        ("layered_dense", layered_dense),
        ("clustered", clustered),
    ];
    for (name, make) in gens {
        for n in [200usize, 2_000] {
            v.push((format!("gen_{name}_{n}"), make(n)));
        }
    }
    v
}

fn golden_path(name: &str) -> PathBuf {
    manifest_dir()
        .join("tests/golden")
        .join(format!("{name}.txt"))
}

fn first_diff(want: &str, got: &str) -> Option<String> {
    let (mut a, mut b) = (want.lines(), got.lines());
    let mut n = 1usize;
    loop {
        match (a.next(), b.next()) {
            (None, None) => return None,
            (x, y) if x == y => n += 1,
            (x, y) => {
                return Some(format!(
                    "line {n}:\n  golden:  {}\n  current: {}",
                    x.unwrap_or("<end>"),
                    y.unwrap_or("<end>")
                ));
            }
        }
    }
}

/// Thread settings every case is laid out with; the result must not depend on them.
/// `None` is the public default (`layout()`), the others fix the thread budget and
/// ignore the size gates so even tiny graphs take the parallel code paths.
fn modes() -> [(&'static str, Option<Par>); 5] {
    [
        ("default", None),
        ("sequential", Some(Par::sequential())),
        ("forced-2", Some(Par::forced(2))),
        ("forced-8", Some(Par::forced(8))),
        ("forced-64", Some(Par::forced(64))),
    ]
}

fn run_layout(m: &Measurer, parsed: &Graph, par: Option<Par>) -> Layout {
    let cancel = AtomicBool::new(false);
    let metrics = Metrics::default();
    match par {
        None => layout::layout(parsed, m, &metrics, None, &cancel),
        Some(p) => layout::layout_with(parsed, m, &metrics, None, &cancel, None, p),
    }
    .expect("layout succeeds")
}

/// Compares every thread mode against the golden file (written first when `update`).
fn check_case(m: &Measurer, name: &str, source: &str, update: bool) -> Option<String> {
    let parsed = parse(source);
    let path: &Path = &golden_path(name);
    if update {
        let got = serialise(&run_layout(m, &parsed.graph, Some(Par::sequential())));
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, &got).expect("write golden");
    }
    let Ok(want) = std::fs::read_to_string(path) else {
        return Some(format!("{name}: missing golden file {}", path.display()));
    };
    for (label, par) in modes() {
        let got = serialise(&run_layout(m, &parsed.graph, par));
        if let Some(d) = first_diff(&want, &got) {
            return Some(format!("{name} [{label}]: {d}"));
        }
    }
    None
}

#[test]
fn layout_matches_golden() {
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let m = measurer();
    let mut failures = Vec::new();
    for (name, source) in cases() {
        if let Some(f) = check_case(&m, &name, &source, update) {
            failures.push(f);
        }
    }
    assert!(
        failures.is_empty(),
        "layout differs from golden files:\n{}",
        failures.join("\n")
    );
}

/// The parallel code paths (forced on every input) give the sequential result,
/// independently of the golden files.
#[test]
fn parallel_equals_sequential() {
    let m = measurer();
    for (name, source) in cases() {
        let parsed = parse(&source);
        let seq = serialise(&run_layout(&m, &parsed.graph, Some(Par::sequential())));
        for threads in [2, 3, 8, 64] {
            let par = serialise(&run_layout(&m, &parsed.graph, Some(Par::forced(threads))));
            assert_eq!(seq, par, "{name}: {threads} threads differ from sequential");
        }
    }
}

/// A raised cancel flag ends every thread mode with `Canceled`.
#[test]
fn cancel_flag_is_honoured_in_every_mode() {
    let m = measurer();
    let parsed = parse(&clustered(200));
    let cancel = AtomicBool::new(true);
    for (label, par) in modes() {
        let par = par.unwrap_or_else(Par::auto);
        let r = layout::layout_with(
            &parsed.graph,
            &m,
            &Metrics::default(),
            None,
            &cancel,
            None,
            par,
        );
        assert_eq!(r.err(), Some(LayoutError::Canceled), "{label}");
    }
}
