#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod canvas;
mod editor;
mod export_cli;
mod icons;
mod motion;
mod paint;
mod source_map;
mod style_file;
mod theme;
mod ui;

use eframe::egui;
use std::sync::Arc;

fn main() -> eframe::Result {
    if let Some(path) = std::env::var_os("MERMAIDLITE_ICON_PREVIEW") {
        if let Err(error) = std::fs::write(&path, icons::to_svg_preview()) {
            eprintln!("Icon preview: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Some(out) = std::env::var_os("MERMAIDLITE_EXPORT") {
        // Dev mode: render the file given on the command line, write the export and exit.
        if let Err(reason) = export_cli::run(std::path::Path::new(&out)) {
            eprintln!("Export: {reason}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let icon = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .ok()
        .map(|image| {
            let pixels = image.into_rgba8();
            egui::IconData { width: pixels.width(), height: pixels.height(), rgba: pixels.into_raw() }
        });
    let options = start_options();
    let size = if options.shot.is_some() { shot_size() } else { [1440.0, 940.0] };
    let mut viewport = egui::ViewportBuilder::default().with_title("MermaidLite").with_inner_size(size);
    if options.shot.is_none() {
        viewport = viewport.with_min_inner_size([800.0, 600.0]);
    }
    if let Some(icon) = icon { viewport = viewport.with_icon(Arc::new(icon)); }
    let native = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native("MermaidLite", native, Box::new(move |cc| {
        let mut fonts = egui::FontDefinitions::default();
        let faces: [(&str, &[u8]); 4] = [
            ("Inter", include_bytes!("../assets/fonts/Inter-Regular.ttf")),
            ("Inter Medium", include_bytes!("../assets/fonts/Inter-Medium.ttf")),
            ("Inter SemiBold", include_bytes!("../assets/fonts/Inter-SemiBold.ttf")),
            ("IBM Plex Mono", include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf")),
        ];
        for (name, bytes) in faces {
            fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_static(bytes)));
        }
        // Egui's bundled fonts stay behind each family so missing glyphs still render.
        let fallback = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
        let with_fallback = |first: &str| {
            let mut list = vec![first.to_owned()];
            list.extend(fallback.iter().cloned());
            list
        };
        fonts.families.insert(egui::FontFamily::Proportional, with_fallback("Inter"));
        fonts.families.insert(theme::font_medium(), with_fallback("Inter Medium"));
        fonts.families.insert(theme::font_semibold(), with_fallback("Inter SemiBold"));
        fonts.families.entry(egui::FontFamily::Monospace).or_default().insert(0, "IBM Plex Mono".into());
        cc.egui_ctx.set_fonts(fonts);
        Ok(Box::new(app::MermaidLiteApp::new(cc, options)))
    }))
}

/// Reads the command line: an optional file to open first and the screenshot dev mode.
///
/// `MERMAIDLITE_SHOT=<png path>` with a file argument opens that file, saves the window as a PNG
/// once the diagram is painted and exits. `MERMAIDLITE_SHOT_THEME=dark` picks the dark theme.
/// More dev hooks, honoured only in screenshot mode: `MERMAIDLITE_SHOT_SELECT=<label text>` selects the
/// first node whose label contains the text, `MERMAIDLITE_SHOT_SOURCE=1|full` opens the source panel
/// (`full` for the whole file), `MERMAIDLITE_SHOT_FOLDER=<dir>` opens that folder as the workspace first,
/// `MERMAIDLITE_SHOT_DIALOG=export|settings|help|close|diagnostics` opens a dialog, `MERMAIDLITE_SHOT_INFO=1` the info popover,
/// `MERMAIDLITE_SHOT_PALETTE=<query>` the palette (`>` for commands), `MERMAIDLITE_SHOT_SEARCH=<text>` the diagram search,
/// `MERMAIDLITE_SHOT_OUTLINE=1` the outline, and `MERMAIDLITE_SHOT_EMPTY=1` without a file the no-workspace welcome state.
fn start_options() -> app::StartOptions {
    let open = std::env::args_os()
        .skip(1)
        .find(|argument| !argument.to_string_lossy().starts_with("--"))
        .map(std::path::PathBuf::from);
    let flag = |name: &str| std::env::var(name).is_ok_and(|value| !value.is_empty() && value != "0");
    let text = |name: &str| std::env::var(name).ok();
    let folder = std::env::var_os("MERMAIDLITE_SHOT_FOLDER").map(std::path::PathBuf::from);
    let empty = flag("MERMAIDLITE_SHOT_EMPTY");
    let shot = match std::env::var_os("MERMAIDLITE_SHOT") {
        Some(path) if open.is_some() || folder.is_some() || empty => Some(app::ShotOptions {
            path: path.into(),
            folder,
            dark: text("MERMAIDLITE_SHOT_THEME").is_some_and(|theme| theme.eq_ignore_ascii_case("dark")),
            select: text("MERMAIDLITE_SHOT_SELECT").filter(|value| !value.is_empty()),
            source: text("MERMAIDLITE_SHOT_SOURCE").filter(|value| !value.is_empty() && value != "0"),
            dialog: text("MERMAIDLITE_SHOT_DIALOG").map(|value| value.to_lowercase()).filter(|value| !value.is_empty()),
            info: flag("MERMAIDLITE_SHOT_INFO"),
            palette: text("MERMAIDLITE_SHOT_PALETTE"),
            search: text("MERMAIDLITE_SHOT_SEARCH"),
            outline: flag("MERMAIDLITE_SHOT_OUTLINE"),
            empty,
        }),
        _ => None,
    };
    app::StartOptions { open, shot }
}

/// Window size from `MERMAIDLITE_SHOT_SIZE=WxH`, default 1440x940.
fn shot_size() -> [f32; 2] {
    std::env::var("MERMAIDLITE_SHOT_SIZE")
        .ok()
        .and_then(|value| {
            let (w, h) = value.split_once(['x', 'X'])?;
            Some([w.trim().parse().ok()?, h.trim().parse().ok()?])
        })
        .unwrap_or([1440.0, 940.0])
}
