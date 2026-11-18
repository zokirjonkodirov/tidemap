//! Preview tool:
//! `cargo run --release -p mermaidlite-diagram --example render -- <in.mmd> <out.svg> [--dark] [--by-subgraph] [--embed-fonts] [--contrast]`
//!
//! Renders a Mermaid flowchart to a styled SVG and prints counts, bounds,
//! diagnostics and stage timings. `--embed-fonts` adds `@font-face` rules that
//! point at the bundled Inter files so a browser draws the same glyphs that
//! were measured.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use mermaidlite_diagram::style::contrast_ratio;
use mermaidlite_diagram::{
    ColorBy, Measurer, Metrics, Scene, StyleSheet, layout, parse, render, to_svg,
};

fn fonts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("assets")
        .join("fonts")
}

fn font_face_css(dir: &std::path::Path) -> String {
    let url = |file: &str| {
        let path = dir.join(file);
        let path = path.canonicalize().unwrap_or(path);
        let text = path
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        let text = text.trim_start_matches("//?/");
        format!("file:///{text}")
    };
    let mut css = String::new();
    for (weight, file) in [
        (400, "Inter-Regular.ttf"),
        (500, "Inter-Medium.ttf"),
        (600, "Inter-SemiBold.ttf"),
    ] {
        css.push_str(&format!(
            "@font-face {{ font-family: 'Inter'; font-weight: {weight}; src: url('{}'); }}\n",
            url(file)
        ));
    }
    css
}

fn print_contrast(style: &StyleSheet) {
    for (name, dark) in [("light", false), ("dark", true)] {
        let c = style.colors(dark);
        let rows = [
            ("sky", c.sky),
            ("peach", c.peach),
            ("mint", c.mint),
            ("yellow", c.yellow),
            ("lavender", c.lavender),
            ("gray", c.gray),
            ("rose", c.rose),
        ];
        for (swatch, s) in rows {
            println!(
                "contrast {name:5} {swatch:9} {:.2}",
                contrast_ratio(c.text, s.fill)
            );
        }
        println!(
            "contrast {name:5} {:9} {:.2}",
            "label_bg",
            contrast_ratio(c.text, c.label_bg)
        );
        println!(
            "contrast {name:5} {:9} {:.2}",
            "canvas",
            contrast_ratio(c.text, c.canvas)
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let files: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let mut style = StyleSheet::default();
    if flag("--contrast") {
        print_contrast(&style);
        if files.is_empty() {
            return;
        }
    }
    let (Some(input), Some(output)) = (files.first(), files.get(1)) else {
        eprintln!(
            "usage: render <in.mmd> <out.svg> [--dark] [--by-subgraph] [--embed-fonts] [--contrast]"
        );
        std::process::exit(2);
    };
    let dark = flag("--dark");
    if flag("--by-subgraph") {
        style.color_by = ColorBy::Subgraph;
    }

    let source = std::fs::read_to_string(input).expect("read input");
    let dir = fonts_dir();
    let font = std::fs::read(dir.join("Inter-Medium.ttf")).expect("read Inter-Medium.ttf");
    let measurer = Measurer::new(Arc::from(font)).expect("font");
    let metrics = Metrics::default();
    let cancel = AtomicBool::new(false);

    // Separate stage timings, then the real entry point.
    let t = Instant::now();
    let parsed = parse(&source);
    let parse_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let laid = layout::layout(&parsed.graph, &measurer, &metrics, None, &cancel);
    let layout_ms = t.elapsed().as_secs_f64() * 1e3;
    let scene_ms = laid.as_ref().ok().map(|l| {
        let t = Instant::now();
        let _ = Scene::build(&parsed.graph, l);
        t.elapsed().as_secs_f64() * 1e3
    });

    let t = Instant::now();
    let rendered = match render(&source, &measurer, &metrics, None, &cancel) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("render failed: {e}");
            std::process::exit(1);
        }
    };
    let render_ms = t.elapsed().as_secs_f64() * 1e3;

    let css = flag("--embed-fonts").then(|| font_face_css(&dir));
    let t = Instant::now();
    let svg = to_svg(&rendered.scene, &style, dark, true, "Inter", css.as_deref());
    let svg_ms = t.elapsed().as_secs_f64() * 1e3;
    std::fs::write(output, &svg).expect("write svg");

    let b = rendered.scene.bounds;
    println!(
        "nodes {}  edges {}  clusters {}  bounds {:.0} x {:.0}  svg {} bytes  scene {} bytes",
        rendered.scene.nodes.len(),
        rendered.scene.edges.len(),
        rendered.scene.clusters.len(),
        b.w,
        b.h,
        svg.len(),
        rendered.scene.byte_size()
    );
    println!("diagnostics {}", rendered.diagnostics.len());
    for d in &rendered.diagnostics {
        println!("  {}:{} {}", d.line, d.column, d.message);
    }
    println!(
        "timing ms  parse {parse_ms:.2}  layout {layout_ms:.2}  scene {:.2}  render {render_ms:.2}  svg {svg_ms:.2}",
        scene_ms.unwrap_or(0.0)
    );
}
