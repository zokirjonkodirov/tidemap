//! Text that survives egui's font atlas.
//!
//! egui rasterises every glyph at every size it is asked for into one atlas that is never
//! evicted. When the atlas fills up (or the display scale changes) it is thrown away and
//! rebuilt, and any `Galley` the app kept from an earlier frame then points into the old
//! atlas and draws the wrong glyphs. This module gives the app two tools against that:
//!
//! * a font-atlas *epoch* ([`observe`], [`settle`], [`epoch`]) that changes whenever cached
//!   galleys became invalid, so holders of galleys can drop them, and
//! * [`raster`] and [`paint_scaled`], which keep zoomed diagram labels on a small fixed ladder
//!   of glyph sizes while the zoom moves (and at the exact size once it rests), so the atlas
//!   fills slowly.

use std::sync::Arc;

use egui::{Color32, Context, Id, Painter, Pos2, Shape, epaint::{Mesh, TextShape, Vertex}, vec2};

/// Largest glyph size, in points, that is ever rasterised for a zoomed label. Bigger text is
/// the 96 point raster scaled up as geometry.
pub const MAX_RASTER_PX: f32 = 96.0;

/// Raster sizes per doubling of the font size: neighbouring rasters differ by about 6%.
const STEPS_PER_OCTAVE: f32 = 12.0;

/// What the atlas looked like when the last frame ended.
#[derive(Clone, Copy, PartialEq)]
struct Settled {
    fill: f32,
    ppp: f32,
    side: usize,
    /// Size of the atlas image in texels.
    size: [usize; 2],
}

fn epoch_id() -> Id {
    Id::new("tidemap-font-epoch")
}

fn settled_id() -> Id {
    Id::new("tidemap-font-settled")
}

fn started_id() -> Id {
    Id::new("tidemap-font-size-at-start")
}

/// Call once at the start of every frame, before any galley is used. Bumps the [epoch](epoch)
/// when the atlas was rebuilt or its scale changed since the previous [`settle`].
///
/// egui only rebuilds the atlas in `begin_pass`, between our `settle` and this call, and an
/// atlas only ever grows otherwise: a lower fill ratio therefore means a rebuild (this also
/// covers a rebuild for a changed `text_alpha_from_coverage`, which refills from scratch).
pub fn observe(ctx: &Context) {
    let now = current(ctx);
    ctx.data_mut(|data| data.insert_temp(started_id(), now.size));
    let was = ctx.data(|data| data.get_temp::<Settled>(settled_id()));
    let rebuilt = was.is_some_and(|was| now.fill < was.fill || now.ppp != was.ppp || now.side != was.side);
    if rebuilt {
        ctx.data_mut(|data| *data.get_temp_mut_or_insert_with(epoch_id(), || 0_u64) += 1);
    }
}

/// Call once at the end of every frame: records the atlas state that [`observe`] compares with.
///
/// An atlas that grew during the frame invalidates the texel scale [`paint_scaled`] used for the
/// meshes it painted before the growth: one more frame is asked for to draw them again.
pub fn settle(ctx: &Context) {
    let now = current(ctx);
    let started = ctx.data(|data| data.get_temp::<[usize; 2]>(started_id()));
    if started.is_some_and(|size| size != now.size) {
        ctx.request_repaint();
    }
    ctx.data_mut(|data| data.insert_temp(settled_id(), now));
}

/// A number that changes whenever galleys laid out in earlier frames can no longer be drawn.
/// Keep it next to any `Arc<Galley>` held across frames and drop them when it differs.
pub fn epoch(ctx: &Context) -> u64 {
    ctx.data(|data| data.get_temp::<u64>(epoch_id())).unwrap_or(0)
}

fn current(ctx: &Context) -> Settled {
    // Read egui's state first: its context lock is not re-entrant, so it must not be held
    // while the data lock is taken.
    let (fill, side, size) = ctx.fonts(|fonts| (fonts.font_atlas_fill_ratio(), fonts.max_texture_side(), fonts.font_image_size()));
    Settled { fill, ppp: ctx.pixels_per_point(), side, size }
}

/// Splits a wanted glyph size `px` into `(raster_px, scale)` with `raster_px * scale == px`.
///
/// While the zoom moves (`settled` false), `raster_px` lies on a fixed geometric ladder (12 steps
/// per octave), at most [`MAX_RASTER_PX`] and at least 1, so only a few dozen sizes are ever
/// rasterised however the zoom moves. Once it rests (`settled`), the text is laid out at `px`
/// itself, rounded to a quarter point, with scale 1: unscaled glyphs stay on the pixel grid and
/// crisp, and one resting size costs the atlas one size. Sizes beyond [`MAX_RASTER_PX`] always use
/// the ladder. Lay the text out at `raster_px` and draw it with [`paint_scaled`] by `scale`.
pub fn raster(px: f32, settled: bool) -> (f32, f32) {
    let px = px.max(f32::EPSILON);
    if settled {
        let exact = (px * 4.0).round() / 4.0;
        if (1.0..=MAX_RASTER_PX).contains(&exact) {
            return (exact, 1.0);
        }
    }
    let step = (px.log2() * STEPS_PER_OCTAVE).round() / STEPS_PER_OCTAVE;
    let raster = step.exp2().clamp(1.0, MAX_RASTER_PX);
    (raster, px / raster)
}

/// Paints `galley` with its top-left corner at `top_left`, `scale` times its laid-out size.
///
/// A scaled galley is drawn as one mesh built from its rows with the vertices scaled as
/// geometry: nothing is laid out again (no new glyph size reaches the atlas) and the galley,
/// which egui's cache shares, is neither copied nor changed.
pub fn paint_scaled(painter: &Painter, top_left: Pos2, galley: Arc<egui::Galley>, color: Color32, scale: f32) {
    if (scale - 1.0).abs() < 1e-4 {
        painter.add(TextShape::new(top_left, galley, color));
        return;
    }
    let atlas = painter.ctx().fonts(|fonts| fonts.font_image_size());
    painter.add(Shape::mesh(scaled_mesh(&galley, top_left, color, scale, atlas)));
}

/// The glyphs of `galley` as one mesh of the font atlas (`atlas` texels across and down), scaled
/// by `scale` about `top_left`, where the galley's own top-left corner is put.
fn scaled_mesh(galley: &egui::Galley, top_left: Pos2, color: Color32, scale: f32, atlas: [usize; 2]) -> Mesh {
    // Galleys keep texel uvs; egui's tessellator divides them by the atlas size, and so do we.
    let normalise = vec2(1.0 / atlas[0] as f32, 1.0 / atlas[1] as f32);
    let mut mesh = Mesh::default();
    mesh.vertices.reserve(galley.num_vertices);
    mesh.indices.reserve(galley.num_indices);
    for placed in &galley.rows {
        let visuals = &placed.row.visuals.mesh;
        let first = mesh.vertices.len() as u32;
        mesh.indices.extend(visuals.indices.iter().map(|index| index + first));
        mesh.vertices.extend(visuals.vertices.iter().map(|vertex| Vertex {
            pos: top_left + (placed.pos.to_vec2() + vertex.pos.to_vec2()) * scale,
            uv: (vertex.uv.to_vec2() * normalise).to_pos2(),
            color: if vertex.color == Color32::PLACEHOLDER { color } else { vertex.color },
        }));
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raster_sizes_stay_close_to_the_wanted_size_and_few() {
        let mut distinct = std::collections::BTreeSet::new();
        let mut px = 1.0_f32;
        while px < 400.0 {
            let (raster_px, scale) = raster(px, false);
            assert!((raster_px * scale - px).abs() <= px * 1e-5, "{px}: {raster_px} * {scale}");
            assert!(raster_px <= MAX_RASTER_PX && raster_px >= 1.0, "{px}: raster {raster_px}");
            if px <= MAX_RASTER_PX {
                assert!((scale - 1.0).abs() <= 0.03, "{px}: scale {scale}");
            }
            if (5.0..=96.0).contains(&px) {
                distinct.insert(raster_px.to_bits());
            }
            px += 0.05;
        }
        // 12 steps per octave over log2(96 / 5) = 4.26 octaves is 52 sizes.
        assert!(distinct.len() <= 54, "{} distinct raster sizes between 5 and 96 px", distinct.len());
        assert_eq!(raster(500.0, false).0, MAX_RASTER_PX);
    }

    #[test]
    fn settled_labels_are_laid_out_at_their_exact_size() {
        assert_eq!(raster(14.0, true), (14.0, 1.0), "100% zoom is crisp");
        assert_eq!(raster(14.1, true), (14.0, 1.0));
        assert_eq!(raster(14.2, true), (14.25, 1.0));
        assert_eq!(raster(9.0, true), (9.0, 1.0));
        // Too big for one glyph size: the ladder and a scale, settled or not.
        let (big, scale) = raster(200.0, true);
        assert_eq!(big, MAX_RASTER_PX);
        assert!((big * scale - 200.0).abs() < 1e-3);
        // Moving: the ladder, as before.
        let (moving, scale) = raster(14.2, false);
        assert!(moving != 14.25 && (scale - 1.0).abs() <= 0.03);
    }

    #[test]
    fn a_scaled_label_is_one_mesh_at_scaled_positions_and_leaves_the_galley_alone() {
        let ctx = Context::default();
        let mut checked = false;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            let galley = ctx.fonts_mut(|fonts| fonts.layout_no_wrap("Hi there".to_owned(), egui::FontId::proportional(20.0), Color32::RED));
            let before: Vec<_> = galley.rows.iter().flat_map(|row| row.row.visuals.mesh.vertices.iter().map(|v| v.pos)).collect();
            let (top_left, scale, atlas) = (egui::pos2(100.0, 50.0), 1.5, [256_usize, 128]);
            let mesh = scaled_mesh(&galley, top_left, Color32::BLUE, scale, atlas);
            assert!(!mesh.is_empty());
            assert_eq!(mesh.texture_id, egui::TextureId::default(), "the font atlas");
            let mut at = 0;
            for row in &galley.rows {
                for vertex in &row.row.visuals.mesh.vertices {
                    let out = mesh.vertices[at];
                    at += 1;
                    let want = top_left + (row.pos.to_vec2() + vertex.pos.to_vec2()) * scale;
                    assert!((out.pos - want).length() < 1e-3, "{:?} vs {want:?}", out.pos);
                    assert!((out.uv.x - vertex.uv.x / 256.0).abs() < 1e-6 && (out.uv.y - vertex.uv.y / 128.0).abs() < 1e-6);
                    assert_eq!(out.color, vertex.color);
                }
            }
            assert_eq!(at, mesh.vertices.len());
            assert!(mesh.indices.iter().all(|&i| (i as usize) < mesh.vertices.len()));
            let after: Vec<_> = galley.rows.iter().flat_map(|row| row.row.visuals.mesh.vertices.iter().map(|v| v.pos)).collect();
            assert_eq!(before, after, "the shared galley was not touched");
            checked = true;
        });
        assert!(checked);
    }

    fn frame(ctx: &Context) {
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            observe(ctx);
            egui::CentralPanel::default().show(ctx, |ui| ui.label("some text for the atlas"));
            settle(ctx);
        });
    }

    #[test]
    fn the_epoch_bumps_when_the_scale_changes_and_only_then() {
        let ctx = Context::default();
        frame(&ctx);
        frame(&ctx);
        let before = epoch(&ctx);
        frame(&ctx);
        assert_eq!(epoch(&ctx), before, "an unchanged frame keeps the epoch");
        ctx.set_pixels_per_point(2.0);
        frame(&ctx);
        assert!(epoch(&ctx) > before, "a new scale rebuilds the atlas");
        let after = epoch(&ctx);
        frame(&ctx);
        assert_eq!(epoch(&ctx), after);
    }
}
