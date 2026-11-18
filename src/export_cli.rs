//! Dev mode `MERMAIDLITE_EXPORT=<out.svg|out.png> mermaidlite <file>`: renders the first diagram
//! of the file, writes the export through the same code the Export dialog uses and exits.
//!
//! `MERMAIDLITE_EXPORT_SCALE` sets the PNG scale (default 1); `MERMAIDLITE_SHOT_THEME=dark`
//! exports with the dark palette.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mermaidlite_core::{default_data_dir, source::extract_diagrams};
use mermaidlite_model::{ExportFormat, ExportRequest, RenderEvent, RenderRequest};
use mermaidlite_renderer::Engine;

use crate::style_file::StyleFile;

const FONT_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
const TIMEOUT: Duration = Duration::from_secs(60);

pub fn run(out: &Path) -> Result<(), String> {
    let input = std::env::args_os()
        .skip(1)
        .find(|argument| !argument.to_string_lossy().starts_with("--"))
        .ok_or("pass the file to export as the first argument")?;
    let content = std::fs::read_to_string(&input).map_err(|reason| reason.to_string())?;
    let blocks = extract_diagrams(&content, &input.to_string_lossy(), &[]);
    let block = blocks.first().ok_or("the file has no diagram")?;
    let format = match out.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("png") => ExportFormat::Png,
        Some(extension) if extension.eq_ignore_ascii_case("svg") => ExportFormat::Svg,
        _ => return Err("the output path must end in .svg or .png".into()),
    };
    let scale = std::env::var("MERMAIDLITE_EXPORT_SCALE")
        .ok()
        .and_then(|value| value.trim().trim_end_matches(['x', 'X']).parse::<f32>().ok())
        .unwrap_or(1.0);
    let dark = std::env::var("MERMAIDLITE_SHOT_THEME").is_ok_and(|theme| theme.eq_ignore_ascii_case("dark"));

    let style = StyleFile::load(&default_data_dir()).sheet;
    let mut engine = Engine::new(Arc::new(|| {}), Arc::from(FONT_MEDIUM), style);
    engine.render(RenderRequest {
        job_id: 1,
        diagram_id: block.id.clone(),
        revision: 0,
        source: block.source.clone(),
    });
    let started = Instant::now();
    let artifact = loop {
        let mut finished = None;
        for event in engine.poll() {
            if let RenderEvent::Completed { result, .. } = event {
                finished = Some(result.map_err(|diagnostic| diagnostic.message)?);
            }
        }
        if let Some(artifact) = finished {
            break artifact;
        }
        if started.elapsed() > TIMEOUT {
            return Err("the diagram did not finish drawing".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    };

    engine.export(ExportRequest {
        job_id: 2,
        artifact,
        path: out.to_path_buf(),
        format,
        background: true,
        scale,
        dark,
    });
    loop {
        for event in engine.poll() {
            if let RenderEvent::Exported { result, .. } = event {
                return result.map(|_| ());
            }
        }
        if started.elapsed() > TIMEOUT {
            return Err("the export did not finish".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
