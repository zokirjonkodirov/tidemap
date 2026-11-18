//! Debug tool: `cargo run --release -p mermaidlite-diagram --example dump -- <in.mmd> <out.svg>`
//! Parses and lays out a Mermaid flowchart and writes a plain debug SVG.

use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use mermaidlite_diagram::{Measurer, Metrics, PathCmd, layout, parse, shape};

type Pt = [f32; 2];

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn flatten(points: &[Pt]) -> Vec<Pt> {
    let mut out = Vec::new();
    if points.len() < 4 {
        return out;
    }
    out.push(points[0]);
    for c in points[1..].chunks(3) {
        if c.len() < 3 {
            break;
        }
        let p0 = *out.last().unwrap();
        for i in 1..=12 {
            let t = i as f32 / 12.0;
            let u = 1.0 - t;
            let (a, b, cc, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            out.push([
                a * p0[0] + b * c[0][0] + cc * c[1][0] + d * c[2][0],
                a * p0[1] + b * c[0][1] + cc * c[1][1] + d * c[2][1],
            ]);
        }
    }
    out
}

fn seg_cross(a: Pt, b: Pt, c: Pt, d: Pt) -> bool {
    let o = |p: Pt, q: Pt, r: Pt| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    let (d1, d2, d3, d4) = (o(a, b, c), o(a, b, d), o(c, d, a), o(c, d, b));
    ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0)) && d1 != 0.0 && d2 != 0.0
}

fn seg_rect(a: Pt, b: Pt, r: [f32; 4]) -> bool {
    // Liang-Barsky against [x0, y0, x1, y1].
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    let d = [b[0] - a[0], b[1] - a[1]];
    for (p, q) in [
        (-d[0], a[0] - r[0]),
        (d[0], r[2] - a[0]),
        (-d[1], a[1] - r[1]),
        (d[1], r[3] - a[1]),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

fn quality_line(
    g: &mermaidlite_diagram::Graph,
    lay: &mermaidlite_diagram::Layout,
    measurer: &Measurer,
) -> String {
    use mermaidlite_diagram::Endpoint;
    let b = lay.bounds;
    let polys: Vec<Vec<Pt>> = lay.edges.iter().map(|e| flatten(&e.points)).collect();
    let mut total = 0.0f32;
    for p in &polys {
        for w in p.windows(2) {
            total += (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        }
    }
    let ends: Vec<[Endpoint; 2]> = g.edges.iter().map(|e| [e.from, e.to]).collect();
    let share = |i: usize, j: usize| ends[i].iter().any(|a| ends[j].contains(a));
    let bbox = |p: &Vec<Pt>| {
        let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for q in p {
            r[0] = r[0].min(q[0]);
            r[1] = r[1].min(q[1]);
            r[2] = r[2].max(q[0]);
            r[3] = r[3].max(q[1]);
        }
        r
    };
    let boxes: Vec<[f32; 4]> = polys.iter().map(bbox).collect();
    let big = g.nodes.len() > 1500;
    let mut crossings = 0usize;
    if !big {
        for i in 0..polys.len() {
            for j in i + 1..polys.len() {
                if polys[i].is_empty()
                    || polys[j].is_empty()
                    || share(i, j)
                    || boxes[i][0] > boxes[j][2]
                    || boxes[j][0] > boxes[i][2]
                    || boxes[i][1] > boxes[j][3]
                    || boxes[j][1] > boxes[i][3]
                {
                    continue;
                }
                let mut hit = false;
                'o: for a in polys[i].windows(2) {
                    for c in polys[j].windows(2) {
                        if seg_cross(a[0], a[1], c[0], c[1]) {
                            hit = true;
                            break 'o;
                        }
                    }
                }
                crossings += usize::from(hit);
            }
        }
    }
    let mut overlap = 0usize;
    if !big {
        for i in 0..lay.nodes.len() {
            for j in i + 1..lay.nodes.len() {
                let (p, q) = (lay.nodes[i], lay.nodes[j]);
                if p.x < q.right() && q.x < p.right() && p.y < q.bottom() && q.y < p.bottom() {
                    overlap += 1;
                }
            }
        }
    }
    let mut through = 0usize;
    if !big {
        for (i, p) in polys.iter().enumerate() {
            let mut hit = false;
            for (k, r) in lay.nodes.iter().enumerate() {
                if ends[i].contains(&Endpoint::Node(mermaidlite_diagram::NodeId(k as u32))) {
                    continue;
                }
                let rr = [r.x + 2.0, r.y + 2.0, r.right() - 2.0, r.bottom() - 2.0];
                if rr[0] >= rr[2]
                    || rr[1] >= rr[3]
                    || boxes[i][0] > rr[2]
                    || boxes[i][2] < rr[0]
                    || boxes[i][1] > rr[3]
                    || boxes[i][3] < rr[1]
                {
                    continue;
                }
                if p.windows(2).any(|w| seg_rect(w[0], w[1], rr)) {
                    hit = true;
                    if std::env::var("DUMP_VERBOSE").is_ok() {
                        let w = p.windows(2).find(|w| seg_rect(w[0], w[1], rr)).unwrap();
                        eprintln!(
                            "edge {} through node {} {} rect {:?} seg {:?} pts {:?}",
                            i, k, g.nodes[k].label, r, w, lay.edges[i].points
                        );
                    }
                    break;
                }
            }
            through += usize::from(hit);
        }
    }
    let mut title_hits = 0usize;
    for (c, cl) in g.clusters.iter().enumerate() {
        let r = lay.clusters[c];
        let tw = measurer.width(&cl.title, 13.0) * 1.06;
        let tr = [r.x + 10.0, r.y + 4.0, r.x + 14.0 + tw, r.y + 24.0];
        for (ei, p) in polys.iter().enumerate() {
            if p.windows(2).any(|w| seg_rect(w[0], w[1], tr)) {
                title_hits += 1;
                if std::env::var("DUMP_VERBOSE").is_ok() {
                    eprintln!("edge {ei} through title of {}", cl.title);
                }
            }
        }
    }
    // Containment counters: node rects inside a cluster rect they are not a member of
    // (directly or via nesting), and pairs of unrelated clusters that intersect.
    let kc = g.clusters.len();
    let cl_anc = |mut c: usize, a: usize| -> bool {
        for _ in 0..300 {
            if c == a {
                return true;
            }
            match g.clusters[c].parent {
                Some(p) => c = p.0 as usize,
                None => return false,
            }
        }
        false
    };
    let hit = |p: &mermaidlite_diagram::Rect, q: &mermaidlite_diagram::Rect| {
        p.x < q.right() && q.x < p.right() && p.y < q.bottom() && q.y < p.bottom()
    };
    let verbose = std::env::var("DUMP_VERBOSE").is_ok();
    let mut outsiders = 0usize;
    for (i, nd) in g.nodes.iter().enumerate() {
        let hit_any = (0..kc).any(|c| {
            let member = nd.cluster.is_some_and(|x| cl_anc(x.0 as usize, c));
            let bad = !member && hit(&lay.nodes[i], &lay.clusters[c]);
            if bad && verbose {
                eprintln!(
                    "outsider node {} in cluster {}",
                    nd.label, g.clusters[c].title
                );
            }
            bad
        });
        outsiders += usize::from(hit_any);
    }
    let mut cl_overlaps = 0usize;
    for a in 0..kc {
        for b in a + 1..kc {
            if !cl_anc(a, b) && !cl_anc(b, a) && hit(&lay.clusters[a], &lay.clusters[b]) {
                cl_overlaps += 1;
                if verbose {
                    eprintln!(
                        "clusters overlap {} {:?} / {} {:?}",
                        g.clusters[a].title, lay.clusters[a], g.clusters[b].title, lay.clusters[b]
                    );
                }
            }
        }
    }
    let skip = |v: usize| {
        if big {
            "n/a".to_string()
        } else {
            v.to_string()
        }
    };
    let top: Vec<usize> = (0..g.clusters.len())
        .filter(|&c| g.clusters[c].parent.is_none())
        .collect();
    let carea: f32 = top
        .iter()
        .map(|&c| lay.clusters[c].w * lay.clusters[c].h)
        .sum();
    let narea: f32 = g
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, nd)| nd.cluster.is_some())
        .map(|(i, _)| lay.nodes[i].w * lay.nodes[i].h)
        .sum();
    let fill = if carea > 0.0 { narea / carea } else { 0.0 };
    format!(
        "bounds {:.0}x{:.0} area={:.0} aspect={:.2} total_edge_length={:.0} crossings={} nodes_overlapping={} edges_through_nodes={} edges_through_titles={} cluster_fill={:.3} outsiders_in_clusters={} cluster_overlaps={}",
        b.w,
        b.h,
        b.w * b.h,
        b.w / b.h.max(1.0),
        total,
        skip(crossings),
        skip(overlap),
        skip(through),
        title_hits,
        fill,
        outsiders,
        cl_overlaps
    )
}

fn invariants_line(
    g: &mermaidlite_diagram::Graph,
    lay: &mermaidlite_diagram::Layout,
    m: &Metrics,
) -> String {
    let k = g.clusters.len();
    let anc = |mut c: usize, a: usize| -> bool {
        for _ in 0..300 {
            if c == a {
                return true;
            }
            match g.clusters[c].parent {
                Some(p) => c = p.0 as usize,
                None => return false,
            }
        }
        false
    };
    let hit = |p: &mermaidlite_diagram::Rect, q: &mermaidlite_diagram::Rect| {
        p.x < q.right() && q.x < p.right() && p.y < q.bottom() && q.y < p.bottom()
    };
    let (mut outside, mut intruder, mut sib, mut nest, mut lab) = (0, 0, 0, 0, 0);
    for (i, nd) in g.nodes.iter().enumerate() {
        let r = lay.nodes[i];
        for c in 0..k {
            let cr = lay.clusters[c];
            let member = nd.cluster.is_some_and(|x| anc(x.0 as usize, c));
            if member {
                let pad = m.cluster_pad - 0.5;
                if r.x < cr.x + pad
                    || r.right() > cr.right() - pad
                    || r.y < cr.y + pad
                    || r.bottom() > cr.bottom() - pad
                {
                    outside += 1;
                }
            } else if hit(&r, &cr) {
                intruder += 1;
            }
        }
    }
    for a in 0..k {
        for b in a + 1..k {
            if anc(a, b) || anc(b, a) {
                if anc(a, b) {
                    let (p, c) = (lay.clusters[b], lay.clusters[a]);
                    if c.x < p.x || c.y < p.y || c.right() > p.right() || c.bottom() > p.bottom() {
                        nest += 1;
                    }
                } else {
                    let (p, c) = (lay.clusters[a], lay.clusters[b]);
                    if c.x < p.x || c.y < p.y || c.right() > p.right() || c.bottom() > p.bottom() {
                        nest += 1;
                    }
                }
            } else if hit(&lay.clusters[a], &lay.clusters[b]) {
                sib += 1;
            }
        }
    }
    let labels: Vec<_> = lay.edges.iter().filter_map(|e| e.label).collect();
    for (i, l) in labels.iter().enumerate() {
        for r in &lay.nodes {
            if hit(l, r) {
                lab += 1;
            }
        }
        for l2 in &labels[i + 1..] {
            if hit(l, l2) {
                lab += 1;
            }
        }
    }
    format!(
        "invariants members_outside={outside} intruders={intruder} sibling_overlap={sib} nest_violations={nest} label_overlaps={lab}"
    )
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: dump <input.mmd> <output.svg> [--seed <previous.mmd>]");
        std::process::exit(2);
    }
    let src = std::fs::read_to_string(&args[1]).expect("read input");
    let font_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/fonts/Inter-Medium.ttf"
    );
    let font = std::fs::read(font_path).expect("read font");
    let measurer = Measurer::new(Arc::from(font)).expect("font");
    let m = Metrics::default();

    let t0 = Instant::now();
    let parsed = parse(&src);
    let parse_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let g = parsed.graph;
    let cancel = AtomicBool::new(false);
    // Optional stable re-layout: lay out the previous source first and pass it as the seed.
    let prev = args
        .iter()
        .position(|a| a == "--seed")
        .and_then(|i| args.get(i + 1))
        .map(|p| {
            let psrc = std::fs::read_to_string(p).expect("read seed input");
            let pg = parse(&psrc).graph;
            let pl = layout(&pg, &measurer, &m, None, &cancel).expect("seed layout");
            (pg, pl)
        });
    let t1 = Instant::now();
    let lay = match layout(
        &g,
        &measurer,
        &m,
        prev.as_ref().map(|(a, b)| (a, b)),
        &cancel,
    ) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("layout failed: {e}");
            std::process::exit(1);
        }
    };
    let layout_ms = t1.elapsed().as_secs_f64() * 1000.0;

    println!("nodes: {}", g.nodes.len());
    println!("edges: {}", g.edges.len());
    println!(
        "bounds: x={:.1} y={:.1} w={:.1} h={:.1}",
        lay.bounds.x, lay.bounds.y, lay.bounds.w, lay.bounds.h
    );
    println!("{}", quality_line(&g, &lay, &measurer));
    if g.nodes.len() <= 20_000 {
        println!("{}", invariants_line(&g, &lay, &m));
    }
    println!("parse: {parse_ms:.2} ms");
    println!("layout: {layout_ms:.2} ms");
    for d in &parsed.diagnostics {
        println!("diagnostic {}:{}: {}", d.line, d.column, d.message);
    }

    let margin = 24.0;
    let mut s = String::new();
    let _ = write!(
        s,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"{} {} {} {}\" font-family=\"Inter\" font-size=\"14\">\n",
        lay.bounds.w + 2.0 * margin,
        lay.bounds.h + 2.0 * margin,
        -margin,
        -margin,
        lay.bounds.w + 2.0 * margin,
        lay.bounds.h + 2.0 * margin
    );
    let _ = writeln!(
        s,
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#fff\"/>",
        -margin,
        -margin,
        lay.bounds.w + 2.0 * margin,
        lay.bounds.h + 2.0 * margin
    );
    let _ = writeln!(
        s,
        "<style>@font-face{{font-family:Inter;src:url('file:///{}')}}</style>",
        std::path::Path::new(font_path)
            .canonicalize()
            .map(|p| p
                .display()
                .to_string()
                .trim_start_matches("\\\\?\\")
                .replace('\\', "/"))
            .unwrap_or_default()
    );
    // Clusters first (outer before inner), then edges, then nodes.
    let mut corder: Vec<usize> = (0..g.clusters.len()).collect();
    let depth = |mut c: usize| {
        let mut d = 0;
        while let Some(p) = g.clusters[c].parent {
            c = p.0 as usize;
            d += 1;
            if d > 200 {
                break;
            }
        }
        d
    };
    corder.sort_by_key(|&c| (depth(c), c));
    for c in corder {
        let r = lay.clusters[c];
        let _ = writeln!(
            s,
            "<rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" rx=\"16\" fill=\"#F3F7FC\" stroke=\"#C9D8EA\"/>",
            r.x, r.y, r.w, r.h
        );
        let _ = writeln!(
            s,
            "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"13\" font-weight=\"600\" fill=\"#4B5A6D\">{}</text>",
            r.x + 12.0,
            r.y + 20.0,
            esc(&g.clusters[c].title)
        );
    }
    // Edges first so nodes draw over any accidental overlap.
    for (i, e) in lay.edges.iter().enumerate() {
        if e.points.len() < 4 {
            continue;
        }
        let mut d = format!("M{:.2} {:.2}", e.points[0][0], e.points[0][1]);
        for c in e.points[1..].chunks(3) {
            if c.len() == 3 {
                let _ = write!(
                    d,
                    " C{:.2} {:.2} {:.2} {:.2} {:.2} {:.2}",
                    c[0][0], c[0][1], c[1][0], c[1][1], c[2][0], c[2][1]
                );
            }
        }
        let _ = writeln!(
            s,
            "<path d=\"{d}\" fill=\"none\" stroke=\"#8A919C\" stroke-width=\"1.5\"/>"
        );
        let n = e.points.len();
        let tip = e.points[n - 1];
        let mut from = e.points[n - 2];
        if (from[0] - tip[0]).hypot(from[1] - tip[1]) < 0.01 {
            from = e.points[n - 3];
        }
        let (dx, dy) = (tip[0] - from[0], tip[1] - from[1]);
        let len = dx.hypot(dy).max(1e-6);
        let (ux, uy) = (dx / len, dy / len);
        let (len_a, half_w) = (9.0, 4.5);
        let base = [tip[0] - ux * len_a, tip[1] - uy * len_a];
        let _ = writeln!(
            s,
            "<polygon points=\"{:.2},{:.2} {:.2},{:.2} {:.2},{:.2}\" fill=\"#8A919C\"/>",
            tip[0],
            tip[1],
            base[0] - uy * half_w,
            base[1] + ux * half_w,
            base[0] + uy * half_w,
            base[1] - ux * half_w
        );
        let _ = i;
    }
    for (i, r) in lay.nodes.iter().enumerate() {
        let mut d = String::new();
        for c in shape::outline(g.nodes[i].shape, *r, m.radius) {
            match c {
                PathCmd::Move(p) => {
                    let _ = write!(d, "M{:.2} {:.2} ", p[0], p[1]);
                }
                PathCmd::Line(p) => {
                    let _ = write!(d, "L{:.2} {:.2} ", p[0], p[1]);
                }
                PathCmd::Cubic(a, b, c) => {
                    let _ = write!(
                        d,
                        "C{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} ",
                        a[0], a[1], b[0], b[1], c[0], c[1]
                    );
                }
                PathCmd::Close => d.push('Z'),
            }
        }
        let _ = writeln!(
            s,
            "<path d=\"{d}\" fill=\"#DCEBFA\" stroke=\"#8FB8E0\" stroke-width=\"1.5\"/>"
        );
        let tb = &lay.node_text[i];
        let [cx, cy] = r.center();
        let y0 = cy - tb.height / 2.0;
        let lh = if tb.lines.is_empty() {
            m.line_height
        } else {
            tb.height / tb.lines.len() as f32
        };
        for (k, line) in tb.lines.iter().enumerate() {
            let _ = writeln!(
                s,
                "<text x=\"{cx:.2}\" y=\"{:.2}\" text-anchor=\"middle\" dominant-baseline=\"central\" font-size=\"14\" fill=\"#1f2937\">{}</text>",
                y0 + lh * (k as f32 + 0.5),
                esc(line)
            );
        }
    }
    for (i, e) in lay.edges.iter().enumerate() {
        let (Some(r), Some(tb)) = (e.label, lay.edge_text[i].as_ref()) else {
            continue;
        };
        let _ = writeln!(
            s,
            "<rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" rx=\"4\" fill=\"#fff\" stroke=\"#E5E7EB\"/>",
            r.x, r.y, r.w, r.h
        );
        let [cx, cy] = r.center();
        let lh = if tb.lines.is_empty() {
            18.0
        } else {
            tb.height / tb.lines.len() as f32
        };
        let y0 = cy - tb.height / 2.0;
        for (k, line) in tb.lines.iter().enumerate() {
            let _ = writeln!(
                s,
                "<text x=\"{cx:.2}\" y=\"{:.2}\" text-anchor=\"middle\" dominant-baseline=\"central\" font-size=\"13\" fill=\"#374151\">{}</text>",
                y0 + lh * (k as f32 + 0.5),
                esc(line)
            );
        }
    }
    s.push_str("</svg>\n");
    std::fs::write(&args[2], s).expect("write output");
}
