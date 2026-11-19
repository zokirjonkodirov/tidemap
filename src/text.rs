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

use egui::{Color32, Context, Id, Painter, Pos2, emath::TSTransform, epaint::TextShape};

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
}

fn epoch_id() -> Id {
    Id::new("tidemap-font-epoch")
}

fn settled_id() -> Id {
    Id::new("tidemap-font-settled")
}


/// Call once at the start of every frame, before any galley is used. Bumps the [epoch](epoch)
/// when the atlas was rebuilt or its scale changed since the previous [`settle`].
///
/// egui only rebuilds the atlas in `begin_pass`, between our `settle` and this call, and an
/// atlas only ever grows otherwise: a lower fill ratio therefore means a rebuild (this also
/// covers a rebuild for a changed `text_alpha_from_coverage`, which refills from scratch).
pub fn observe(ctx: &Context) {
    let now = current(ctx);
    let was = ctx.data(|data| data.get_temp::<Settled>(settled_id()));
    let rebuilt = was.is_some_and(|was| now.fill < was.fill || now.ppp != was.ppp || now.side != was.side);
    if rebuilt {
        ctx.data_mut(|data| *data.get_temp_mut_or_insert_with(epoch_id(), || 0_u64) += 1);
    }
}

/// Call once at the end of every frame: records the atlas state that [`observe`] compares with.
pub fn settle(ctx: &Context) {
    let now = current(ctx);
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
    let (fill, side) = ctx.fonts(|fonts| (fonts.font_atlas_fill_ratio(), fonts.max_texture_side()));
    Settled { fill, ppp: ctx.pixels_per_point(), side }
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
/// At scale 1 (what [`raster`] returns for a resting view) the galley is painted as it is: no
/// transform, no copy. Otherwise the shape is scaled as geometry, so no new glyph size reaches the
/// atlas; egui's `TextShape::transform` copies the (cache-shared) galley, which is accepted only
/// while the zoom is moving. Uvs stay in texel space and egui normalises them when it tessellates
/// at the end of the frame, so an atlas that grows mid-frame cannot misplace glyphs.
pub fn paint_scaled(painter: &Painter, top_left: Pos2, galley: Arc<egui::Galley>, color: Color32, scale: f32) {
    let mut shape = TextShape::new(top_left, galley, color);
    if (scale - 1.0).abs() >= 1e-4 {
        shape.transform(TSTransform::new(top_left.to_vec2() * (1.0 - scale), scale));
    }
    painter.add(shape);
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
    fn a_resting_label_is_never_scaled_and_is_painted_as_a_plain_text_shape() {
        let mut px = 1.0_f32;
        while px <= MAX_RASTER_PX {
            assert_eq!(raster(px, true).1, 1.0, "{px}");
            px += 0.07;
        }
        let ctx = Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            let galley = painter.layout_no_wrap("Hi".to_owned(), egui::FontId::proportional(14.0), Color32::RED);
            paint_scaled(&painter, egui::pos2(10.0, 20.0), galley.clone(), Color32::RED, 1.0);
            // The shape holds the very galley (shared, not copied) at the given position.
            let shapes = ctx.graphics_mut(|g| g.entry(egui::LayerId::background()).all_entries().map(|entry| entry.shape.clone()).collect::<Vec<_>>());
            let Some(egui::Shape::Text(text)) = shapes.last() else { panic!("a text shape: {shapes:?}") };
            assert!(Arc::ptr_eq(&text.galley, &galley), "no copy");
            assert_eq!(text.pos, egui::pos2(10.0, 20.0));
        });
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
