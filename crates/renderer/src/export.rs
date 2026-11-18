//! Export of a drawn diagram as SVG or PNG.
//!
//! Both formats come from `mermaidlite_diagram::to_svg`, so they match the canvas. SVG files
//! embed the font; PNG files are rasterised by resvg with a font database that holds only the
//! bundled font.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use mermaidlite_diagram::{StyleSheet, to_svg};
use mermaidlite_model::RenderArtifact;
use resvg::tiny_skia;

/// Space the SVG writer adds around the scene bounds on every side.
const MARGIN: f32 = 24.0;
/// Longest side of an exported PNG in pixels.
const MAX_SIDE: u32 = 8192;
/// Largest exported PNG in pixels.
const MAX_PIXELS: u64 = 16_000_000;

/// `@font-face` rule that embeds `font` as a base64 data URI under `family`.
///
/// The rule covers every weight, so a browser draws the one bundled face instead of
/// synthesising bold, matching what the canvas shows.
pub fn font_css(family: &str, font: &[u8]) -> String {
    let family = family.replace(['"', '\\', '\n', '\r'], "");
    format!(
        "@font-face {{ font-family: \"{family}\"; font-weight: 100 900; font-style: normal; src: url(data:font/ttf;base64,{}) format('truetype'); }}",
        STANDARD.encode(font)
    )
}

/// The diagram as a standalone SVG document with the font embedded.
pub fn svg_bytes(
    a: &RenderArtifact,
    style: &StyleSheet,
    dark: bool,
    background: bool,
    family: &str,
    font: &[u8],
) -> Vec<u8> {
    let css = font_css(family, font);
    to_svg(
        &a.rendered.scene,
        style,
        dark,
        background,
        family,
        Some(&css),
    )
    .into_bytes()
}

/// Pixel size of a PNG export of `a` at `scale`.
pub fn png_size(a: &RenderArtifact, scale: f32) -> (u64, u64) {
    let bounds = a.rendered.scene.bounds;
    let side = |length: f32| ((length.max(0.0) + 2.0 * MARGIN) * scale.max(0.0)).ceil() as u64;
    (side(bounds.w), side(bounds.h))
}

/// The error text for an image that would exceed the size limits, if it would.
pub fn png_limit_error(a: &RenderArtifact, scale: f32) -> Option<String> {
    let (w, h) = png_size(a, scale);
    (w > u64::from(MAX_SIDE) || h > u64::from(MAX_SIDE) || w * h > MAX_PIXELS).then(|| {
        format!("Image would be {w} \u{d7} {h} px. Lower the scale or export SVG.")
    })
}

/// The diagram as a PNG at `scale`. Fails when the image would exceed 8192 px on a side or
/// 16 megapixels.
pub fn png_bytes(
    a: &RenderArtifact,
    style: &StyleSheet,
    dark: bool,
    background: bool,
    scale: f32,
    family: &str,
    font: &[u8],
) -> Result<Vec<u8>, String> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err("The export scale is not valid.".into());
    }
    if let Some(message) = png_limit_error(a, scale) {
        return Err(message);
    }
    let svg = to_svg(&a.rendered.scene, style, dark, background, family, None);

    let mut options = resvg::usvg::Options {
        font_family: family.to_owned(),
        // Nothing outside the document may be loaded.
        image_href_resolver: resvg::usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let mut fonts = resvg::usvg::fontdb::Database::new();
    fonts.load_font_data(font.to_vec());
    fonts.set_sans_serif_family(family);
    options.fontdb = Arc::new(fonts);

    let tree = resvg::usvg::Tree::from_str(&svg, &options)
        .map_err(|reason| format!("The diagram could not be converted: {reason}"))?;
    let (w, h) = png_size(a, scale);
    let mut pixmap = tiny_skia::Pixmap::new(w as u32, h as u32)
        .ok_or_else(|| "The image would be empty.".to_owned())?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|reason| format!("The PNG could not be encoded: {reason}"))
}

/// Writes `bytes` to `path` through a temporary file in the same folder, so an interrupted
/// export never leaves a half-written file under the final name.
pub fn write_export(path: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| "The export path has no file name.".to_owned())?;
    let temp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let written = std::fs::write(&temp, bytes).and_then(|()| std::fs::rename(&temp, path));
    match written {
        Ok(()) => Ok(path.to_path_buf()),
        Err(reason) => {
            let _ = std::fs::remove_file(&temp);
            Err(format!("Could not write {}: {reason}", path.display()))
        }
    }
}
