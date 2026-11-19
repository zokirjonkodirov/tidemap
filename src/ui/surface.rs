//! Floating surfaces: how the palette, dialogs, popovers and the toast enter and leave.
//!
//! Every surface grows from an origin (the button that opened it, or the screen centre) with
//! a spring, and shrinks back a little while it fades on the way out. Surfaces share one
//! recipe, so the palette, dialogs and popovers cannot drift apart:
//!
//! 1. [`crate::motion::presence`] gives a value `t` that springs in (with a small pop) and
//!    out, and is `None` once the surface is gone.
//! 2. [`transform`] turns `t` into a scale-and-offset about the origin; [`exit_t`] softens it
//!    for leaving so a surface only shrinks a few percent.
//! 3. Modal surfaces ([`modal`]) apply it to their shapes only; small popovers apply it to the
//!    whole layer ([`animate_layer`]), which also maps the pointer.
//! 4. A leaving surface is inert: it takes no clicks, no keys and no focus.
//! 5. While a modal is up the window behind sinks slightly ([`push_back`]).

use egui::{
    Align2, Area, Context, Frame, Id, LayerId, Order, Painter, Pos2, Rect, Ui, Vec2, emath::TSTransform, vec2,
};

use crate::{motion, theme::Palette};

/// How a surface starts: its scale and where it sits relative to its resting place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Enter {
    /// Scale at `t == 0`; 1 at rest.
    pub scale: f32,
    /// Offset at `t == 0`; zero at rest.
    pub offset: Vec2,
}

/// Info popover and block menu: they unfold from their button, a touch above their place.
pub const POPOVER: Enter = Enter { scale: 0.94, offset: vec2(0.0, -4.0) };
/// Palette: drops in from slightly above.
pub const PALETTE: Enter = Enter { scale: 0.96, offset: vec2(0.0, -8.0) };
/// Dialogs: rise into place from slightly below.
pub const DIALOG: Enter = Enter { scale: 0.95, offset: vec2(0.0, 6.0) };
/// Toast: rises from the bottom.
pub const TOAST: Enter = Enter { scale: 0.96, offset: vec2(0.0, 12.0) };

/// Scale of the window behind a fully open modal.
pub const PUSH_SCALE: f32 = 0.99;

/// The scale and offset of a surface at presence `t`, about `origin` (in the coordinates of
/// the layer the surface is drawn in). Exactly [`TSTransform::IDENTITY`] when `t >= 1`, so a
/// settled surface is drawn untransformed and stays crisp; the small overshoot of a popping
/// spring is clamped away.
pub fn transform(t: f32, origin: Pos2, enter: Enter) -> TSTransform {
    if t >= 1.0 {
        return TSTransform::IDENTITY;
    }
    let t = t.max(0.0);
    let scale = enter.scale + (1.0 - enter.scale) * t;
    let offset = enter.offset * (1.0 - t);
    TSTransform::new(origin.to_vec2() * (1.0 - scale) + offset, scale)
}

/// Maps the presence of a *leaving* surface into the range fed to [`transform`], so it only
/// shrinks to a few percent below its resting size instead of all the way to its entrance pose.
pub fn exit_t(t: f32) -> f32 {
    0.6 + 0.4 * t
}

/// [`transform`] for the current motion preference: under reduced motion nothing scales or
/// slides, so a surface only fades.
pub fn placed(ctx: &Context, t: f32, origin: Pos2, enter: Enter) -> TSTransform {
    if motion::reduced(ctx) { TSTransform::IDENTITY } else { transform(t, origin, enter) }
}

/// Sets the transform of `layer` for presence `t`; at rest (or under reduced motion) it
/// removes the transform.
pub fn animate_layer(ctx: &Context, layer: LayerId, t: f32, origin: Pos2, enter: Enter) {
    ctx.set_transform_layer(layer, placed(ctx, t, origin, enter));
}

/// The layer of the pill that floats over the canvas.
fn pill_layer() -> LayerId {
    LayerId::new(Order::Foreground, Id::new("canvas-pill"))
}

/// The layer of the toast.
pub fn toast_layer() -> LayerId {
    LayerId::new(Order::Tooltip, Id::new("toast"))
}

const PUSH_KEY: &str = "surface-push";

/// The transform that sinks the window about `centre` for modal presence `t` (identity at 0).
fn push_transform(t: f32, centre: Pos2) -> TSTransform {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 {
        return TSTransform::IDENTITY;
    }
    let scale = 1.0 + (PUSH_SCALE - 1.0) * t;
    TSTransform::new(centre.to_vec2() * (1.0 - scale), scale)
}

/// Sinks the window behind a modal: the background and the pill scale about the screen centre
/// to [`PUSH_SCALE`] as `t` goes from 0 to 1. Call once per frame with the strongest modal
/// presence; once `t` has been 0 for a moment (or under reduced motion) the transforms are
/// removed. A sudden drop of `t` eases out rather than snapping.
///
/// The toast layer is deliberately left alone: it has its own entrance transform, so it reads
/// this one back with [`pushed`] and composes `pushed * enter` itself. That way neither
/// overwrites the other.
pub fn push_back(ctx: &Context, t: f32) {
    // Sinking follows `t` at once; only a drop (a modal that vanishes in a frame) eases out, so
    // the window never snaps back. The spring is exactly 0 at rest, keeping the identity.
    let t = t.clamp(0.0, 1.0);
    let eased = motion::spring(ctx, Id::new(PUSH_KEY).with("ease"), t, motion::SNAP).clamp(0.0, 1.0).max(t);
    let push = push_transform(if motion::reduced(ctx) { 0.0 } else { eased }, ctx.content_rect().center());
    ctx.data_mut(|data| data.insert_temp(Id::new(PUSH_KEY), push));
    ctx.set_transform_layer(LayerId::background(), push);
    ctx.set_transform_layer(pill_layer(), push);
}

/// The transform [`push_back`] applied this frame, for layers that compose it with their own.
pub fn pushed(ctx: &Context) -> TSTransform {
    ctx.data(|data| data.get_temp::<TSTransform>(Id::new(PUSH_KEY))).unwrap_or(TSTransform::IDENTITY)
}

/// Paints the scrim for a modal that is leaving, clipped to `screen`. It is drawn inside the
/// leaving surface's own area (under its content) so it stays below everything that sits above
/// the modal layer.
pub fn exit_scrim(painter: &Painter, screen: Rect, p: Palette, t: f32) {
    painter.with_clip_rect(screen).rect_filled(screen, 0.0, p.scrim.gamma_multiply(motion::fade(t)));
}

/// Gives up keyboard focus if the focused widget lives in `layer`, so a leaving surface cannot
/// keep eating keystrokes.
pub fn release_focus(ctx: &Context, layer: LayerId) {
    let Some(id) = ctx.memory(|memory| memory.focused()) else {
        return;
    };
    if ctx.read_response(id).is_some_and(|response| response.layer_id == layer) {
        ctx.memory_mut(|memory| memory.surrender_focus(id));
    }
}

/// Where a modal surface sits and where it grows from.
#[derive(Clone, Copy)]
pub struct Placement {
    /// Id of the surface's area (the same id while it enters and leaves).
    pub id: Id,
    /// Which point of the screen the surface is anchored to.
    pub anchor: Align2,
    /// Offset from the anchor point.
    pub offset: Vec2,
    /// The point the surface grows from and shrinks toward, in screen coordinates.
    pub origin: Pos2,
    /// Entrance pose.
    pub enter: Enter,
}

/// What a modal surface reports back.
pub struct Shown<R> {
    /// What the content closure returned.
    pub inner: R,
    /// The scrim was clicked (only ever while the surface is open).
    pub backdrop_clicked: bool,
}

/// Shows a modal surface at presence `t`.
///
/// While `open` it is an [`egui::Modal`] over a scrim of `p.scrim` faded by `t`; the content
/// is drawn with a visual transform (shapes only, so input is not moved during the short
/// entrance). While leaving it is a plain, inert [`Area`] at the same place (it keeps its layer
/// order, and lets pointer and keys through to the window) with the scrim painted inside it. `add` gets `open` as its `interactive` flag: when false it must not take
/// keys, request focus or build editable widgets.
pub fn modal<R>(
    ctx: &Context,
    p: Palette,
    place: Placement,
    t: f32,
    open: bool,
    add: impl FnOnce(&mut Ui, bool) -> R,
) -> Shown<R> {
    let alpha = motion::fade(t);
    let shown_t = if open { t } else { exit_t(t) };
    let content = |ui: &mut Ui| {
        ui.multiply_opacity(alpha);
        ui.with_visual_transform(placed(ctx, shown_t, place.origin, place.enter), |ui| add(ui, open)).inner
    };
    if open {
        let area = egui::Modal::default_area(place.id).fade_in(false).anchor(place.anchor, place.offset);
        let modal = egui::Modal::new(place.id)
            .area(area)
            .frame(Frame::NONE)
            .backdrop_color(p.scrim.gamma_multiply(alpha))
            .show(ctx, content);
        return Shown { inner: modal.inner, backdrop_clicked: modal.backdrop_response.clicked() };
    }
    let layer = LayerId::new(Order::Foreground, place.id);
    release_focus(ctx, layer);
    let area = Area::new(place.id)
        .order(Order::Foreground)
        .fade_in(false)
        .anchor(place.anchor, place.offset)
        .interactable(false)
        .show(ctx, |ui| {
            exit_scrim(ui.painter(), ctx.content_rect(), p, t);
            content(ui)
        });
    Shown { inner: area.inner, backdrop_clicked: false }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{LayerId, emath::TSTransform};

    #[test]
    fn transform_is_identity_at_rest_and_scales_about_origin() {
        let o = egui::pos2(100.0, 50.0);
        assert_eq!(transform(1.0, o, DIALOG), TSTransform::IDENTITY);
        let t0 = transform(0.0, o, POPOVER);
        let mapped = t0 * o;
        assert!((mapped - (o + POPOVER.offset)).length() < 1e-4);
        assert!((t0.scaling - 0.94).abs() < 1e-6);
    }

    #[test]
    fn layer_transform_cleared_at_rest() {
        let ctx = egui::Context::default();
        let layer = LayerId::new(egui::Order::Foreground, egui::Id::new("x"));
        let _ = ctx.run(Default::default(), |c| animate_layer(c, layer, 0.5, egui::Pos2::ZERO, POPOVER));
        assert!(ctx.layer_transform_to_global(layer).is_some());
        let _ = ctx.run(Default::default(), |c| animate_layer(c, layer, 1.0, egui::Pos2::ZERO, POPOVER));
        assert!(ctx.layer_transform_to_global(layer).is_none());
    }

    #[test]
    fn overshoot_and_undershoot_are_clamped() {
        let o = egui::pos2(10.0, 10.0);
        assert_eq!(transform(1.03, o, PALETTE), TSTransform::IDENTITY);
        assert_eq!(transform(-0.2, o, PALETTE), transform(0.0, o, PALETTE));
    }

    #[test]
    fn leaving_surfaces_only_shrink_a_little() {
        let o = egui::pos2(0.0, 0.0);
        let floor = transform(exit_t(0.0), o, DIALOG).scaling;
        assert!(floor > 0.96 && floor < 1.0, "{floor}");
        assert_eq!(exit_t(1.0), 1.0);
    }

    #[test]
    fn push_back_sinks_about_the_centre_and_clears_at_zero() {
        let ctx = egui::Context::default();
        let pill = pill_layer();
        let _ = ctx.run(Default::default(), |c| push_back(c, 1.0));
        let sunk = ctx.layer_transform_to_global(LayerId::background()).expect("background sunk");
        assert!((sunk.scaling - PUSH_SCALE).abs() < 1e-6);
        let centre = ctx.content_rect().center();
        assert!(((sunk * centre) - centre).length() < 1e-3, "centre must stay put");
        assert!(ctx.layer_transform_to_global(pill).is_some());
        // A drop eases out (see the next test); a fresh context at 0 is the identity.
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |c| push_back(c, 0.0));
        assert!(ctx.layer_transform_to_global(LayerId::background()).is_none());
        assert!(ctx.layer_transform_to_global(pill).is_none());
        assert_eq!(pushed(&ctx), TSTransform::IDENTITY);
    }

    #[test]
    fn push_back_eases_out_when_the_modal_vanishes_at_once() {
        let ctx = egui::Context::default();
        let frame = |ctx: &egui::Context, time: f64, t: f32| {
            let input = egui::RawInput { time: Some(time), ..Default::default() };
            let _ = ctx.run(input, |c| push_back(c, t));
            ctx.layer_transform_to_global(LayerId::background()).map_or(1.0, |transform| transform.scaling)
        };
        assert!((frame(&ctx, 0.0, 1.0) - PUSH_SCALE).abs() < 1e-6, "sinking follows the modal at once");
        let first = frame(&ctx, 0.016, 0.0);
        assert!(first < 1.0 && first > PUSH_SCALE, "the window must not snap back: {first}");
        let mut last = first;
        for step in 2..200 {
            last = frame(&ctx, 0.016 * f64::from(step), 0.0);
        }
        assert_eq!(last, 1.0);
        assert!(ctx.layer_transform_to_global(LayerId::background()).is_none(), "identity exactly at rest");
    }

    #[test]
    fn reduced_motion_drops_movement_but_keeps_the_layer_plain() {
        let ctx = egui::Context::default();
        let layer = LayerId::new(egui::Order::Foreground, egui::Id::new("r"));
        let _ = ctx.run(Default::default(), |c| {
            crate::motion::set_reduced(c, true);
            assert_eq!(placed(c, 0.3, egui::Pos2::ZERO, POPOVER), TSTransform::IDENTITY);
            animate_layer(c, layer, 0.3, egui::Pos2::ZERO, POPOVER);
            push_back(c, 1.0);
        });
        assert!(ctx.layer_transform_to_global(layer).is_none());
        assert!(ctx.layer_transform_to_global(LayerId::background()).is_none());
    }

    #[test]
    fn push_back_leaves_the_toast_layer_to_the_toast() {
        let ctx = egui::Context::default();
        let toast = LayerId::new(egui::Order::Tooltip, egui::Id::new("toast"));
        ctx.set_transform_layer(toast, TSTransform::new(egui::vec2(0.0, 5.0), 1.0));
        let _ = ctx.run(Default::default(), |c| push_back(c, 1.0));
        assert_eq!(ctx.layer_transform_to_global(toast), Some(TSTransform::new(egui::vec2(0.0, 5.0), 1.0)));
    }
}
