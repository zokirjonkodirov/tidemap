//! Hand-painted controls shared by the dialogs, the palette and the empty states.
//!
//! The global egui visuals draw no widget strokes, so anything that has to read as a control
//! (switch, segmented choice, key cap, button) is painted here from the palette.

use egui::{Color32, Context, FontId, Id, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, emath::TSTransform, vec2};

use crate::motion;
use crate::theme::{self, Palette, token};

/// The palette of the theme the `ui` is drawn in.
fn palette(ui: &Ui) -> Palette {
    theme::palette(ui.visuals().dark_mode)
}

const BUTTON_HEIGHT: f32 = 32.0;
const BUTTON_PAD: f32 = 16.0;
const SWITCH_SIZE: egui::Vec2 = egui::vec2(36.0, 20.0);
const KNOB_RADIUS: f32 = 8.0;
/// Extra knob width at full press.
const KNOB_SQUISH: f32 = 4.0;
const SEGMENT_PAD: f32 = 2.0;
const SEGMENT_HEIGHT: f32 = 28.0;
const SEGMENT_TEXT_PAD: f32 = 16.0;
const KEY_HEIGHT: f32 = 24.0;

/// How much a fully pressed control shrinks: 4 % about its centre.
pub const PRESS_DEPTH: f32 = 0.04;

/// Scale of a control at press amount `press` (0 idle, 1 fully pressed). The release spring
/// overshoots `press` slightly below 0, which reads as a small rebound above full size.
pub fn press_scale(press: f32) -> f32 {
    1.0 - PRESS_DEPTH * press
}

/// `rect` scaled about its centre for press amount `press`. Hit-testing and the focus ring
/// keep using the unscaled rect.
pub fn pressed_rect(rect: Rect, press: f32) -> Rect {
    Rect::from_center_size(rect.center(), rect.size() * press_scale(press))
}

/// Paints `text` centred on `centre`, scaled about that point for press amount `press`.
///
/// The galley is laid out once at the normal size and the finished shape is scaled as
/// geometry, so a press never rasterises new glyph sizes into the font atlas. At rest the
/// text is painted untransformed.
pub fn paint_pressed_label(painter: &egui::Painter, centre: Pos2, text: &str, font: FontId, color: Color32, press: f32) {
    let galley = painter.layout_no_wrap(text.to_owned(), font, color);
    let pos = centre - galley.size() * 0.5;
    let mut shape = egui::epaint::TextShape::new(pos, galley, color);
    if press != 0.0 {
        let scale = press_scale(press);
        shape.transform(TSTransform::new(centre.to_vec2() * (1.0 - scale), scale));
    }
    painter.add(shape);
}

fn medium() -> FontId {
    FontId::new(token::FONT_UI, theme::font_medium())
}

/// Draws the keyboard focus ring around `rect` when `response` has focus.
fn focus_ring(ui: &Ui, response: &Response, rect: Rect, radius: f32, p: Palette) {
    if response.has_focus() {
        ui.painter()
            .rect_stroke(rect, radius, Stroke::new(2.0_f32, p.accent), StrokeKind::Outside);
    }
}

/// A rounded text button. `fill` is painted always; `hover` springs in while hovered and a
/// press scales the fill and label down about the centre.
fn text_button(ui: &mut Ui, text: &str, fill: Color32, hover: Color32, color: Color32) -> Response {
    let p = palette(ui);
    let enabled = ui.is_enabled();
    let width = ui.painter().layout_no_wrap(text.to_owned(), medium(), color).size().x;
    let size = vec2(width + 2.0 * BUTTON_PAD, BUTTON_HEIGHT);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let ctx = ui.ctx().clone();
    let over = motion::hover(&ctx, response.id, enabled && response.hovered());
    let press = motion::press(&ctx, response.id, enabled && response.is_pointer_button_down_on());
    let radius = token::RADIUS_CONTROL * press_scale(press);
    ui.painter().rect_filled(pressed_rect(rect, press), radius, fill.lerp_to_gamma(hover, over));
    let color = if enabled { color } else { p.muted };
    paint_pressed_label(ui.painter(), rect.center(), text, medium(), color, press);
    focus_ring(ui, &response, rect, token::RADIUS_CONTROL, p);
    response
}

/// The one main action of a surface: accent fill, `on_accent` text.
pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui);
    let fill = if ui.is_enabled() { p.accent } else { p.field };
    let hover = theme::over(theme::alpha(p.on_accent, 36), p.accent);
    text_button(ui, text, fill, hover, p.on_accent)
}

/// A secondary action: no fill until hovered, `ink` text.
pub fn quiet_button(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui);
    text_button(ui, text, Color32::TRANSPARENT, p.hover, p.ink)
}

/// A quiet button for a destructive choice: `danger` text.
pub fn danger_button(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui);
    text_button(ui, text, Color32::TRANSPARENT, p.hover, p.danger)
}

/// A 36 x 20 pill switch with a 16 px knob that widens by 4 px while pressed. Returns true
/// when toggled.
pub fn switch(ui: &mut Ui, value: &mut bool) -> bool {
    let p = palette(ui);
    let (area, response) =
        ui.allocate_exact_size(vec2(SWITCH_SIZE.x, token::CONTROL), Sense::click());
    let mut changed = false;
    if response.clicked() {
        *value = !*value;
        changed = true;
    }
    // The knob travels with a slight overshoot; the colour follows without one.
    let travel_t = motion::spring(ui.ctx(), response.id.with("switch"), f32::from(u8::from(*value)), motion::POP);
    let on = travel_t.clamp(0.0, 1.0);
    let track = Rect::from_center_size(area.center(), SWITCH_SIZE);
    let track_color = p.track_off.lerp_to_gamma(p.accent, on);
    ui.painter().rect_filled(track, SWITCH_SIZE.y * 0.5, track_color);
    let travel = SWITCH_SIZE.x - SWITCH_SIZE.y;
    let knob = egui::pos2(track.left() + SWITCH_SIZE.y * 0.5 + travel * travel_t.clamp(-0.06, 1.06), track.center().y);
    let knob_color = p.knob_off.lerp_to_gamma(p.on_accent, on);
    let press = motion::press(ui.ctx(), response.id, response.is_pointer_button_down_on());
    let knob_size = vec2(2.0 * KNOB_RADIUS + KNOB_SQUISH * press, 2.0 * KNOB_RADIUS);
    ui.painter().rect_filled(Rect::from_center_size(knob, knob_size), KNOB_RADIUS, knob_color);
    focus_ring(ui, &response, track, SWITCH_SIZE.y * 0.5, p);
    changed
}

/// Where a band was last asked to be, and which way it was travelling.
#[derive(Clone, Copy)]
struct BandMemo {
    /// The leading-axis start (top or left) the band last targeted.
    target: f32,
    /// Whether the band last moved toward larger values (down or right).
    forward: bool,
}

fn band_memo_key(id: Id) -> Id {
    id.with("band-memo")
}

/// A selection band that stretches as it travels: the edge on the side of travel chases its
/// target quickly ([`motion::SNAP`]) while the trailing edge follows slowly ([`motion::SOFT`]),
/// so the band elongates in motion and contracts on arrival.
///
/// `top` and `bottom` are the target edges in any unit (points, row indices; for a horizontal
/// band pass left and right). Returns the band's current `(top, bottom)` in the same unit. The
/// first call, and the first call after [`restart_band`], returns the target exactly.
pub fn stretch_band(ctx: &Context, id: Id, top: f32, bottom: f32) -> (f32, f32) {
    let key = band_memo_key(id);
    let memo = ctx.data(|data| data.get_temp::<BandMemo>(key));
    let forward = match memo {
        Some(memo) if memo.target == top => memo.forward,
        Some(memo) => top > memo.target,
        None => true,
    };
    ctx.data_mut(|data| data.insert_temp(key, BandMemo { target: top, forward }));
    let (leading, trailing) = (motion::SNAP, motion::SOFT);
    let (top_feel, bottom_feel) = if forward { (trailing, leading) } else { (leading, trailing) };
    (
        motion::spring(ctx, id.with("top"), top, top_feel),
        motion::spring(ctx, id.with("bottom"), bottom, bottom_feel),
    )
}

/// Resets a [`stretch_band`] so its next call snaps to its target instead of travelling.
pub fn restart_band(ctx: &Context, id: Id) {
    motion::restart(ctx, id.with("top"));
    motion::restart(ctx, id.with("bottom"));
    ctx.data_mut(|data| data.remove::<BandMemo>(band_memo_key(id)));
}

/// A segmented choice: a recessed track with the selected option as a raised segment.
/// Options read left to right in the order given. Returns true when the choice changed.
pub fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    let p = palette(ui);
    let selected_font = medium();
    let widths: Vec<f32> = options
        .iter()
        .map(|(_, label)| {
            let galley = ui
                .painter()
                .layout_no_wrap((*label).to_owned(), selected_font.clone(), p.ink);
            galley.size().x + 2.0 * SEGMENT_TEXT_PAD
        })
        .collect();
    let width = widths.iter().sum::<f32>() + 2.0 * SEGMENT_PAD;
    let (track, track_response) = ui.allocate_exact_size(vec2(width, SEGMENT_HEIGHT + 2.0 * SEGMENT_PAD), Sense::hover());
    ui.painter().rect_filled(track, token::RADIUS_CONTROL, p.field);

    // The raised segment slides to the chosen option.
    let lefts: Vec<f32> = widths
        .iter()
        .scan(track.left() + SEGMENT_PAD, |x, w| {
            let left = *x;
            *x += w;
            Some(left)
        })
        .collect();
    let chosen = options.iter().position(|(candidate, _)| candidate == value);
    let inner_radius = token::RADIUS_CONTROL - SEGMENT_PAD;
    if let Some(chosen) = chosen {
        let ctx = ui.ctx().clone();
        let id = track_response.id.with("segment-slide");
        let (left, right) = stretch_band(&ctx, id, lefts[chosen], lefts[chosen] + widths[chosen]);
        let band = Rect::from_min_size(egui::pos2(left, track.top() + SEGMENT_PAD), vec2(right - left, SEGMENT_HEIGHT));
        ui.painter().rect_filled(band, inner_radius, p.raised);
    }

    let mut changed = false;
    for (index, ((candidate, label), segment_width)) in options.iter().zip(&widths).enumerate() {
        let rect = Rect::from_min_size(
            egui::pos2(lefts[index], track.top() + SEGMENT_PAD),
            vec2(*segment_width, SEGMENT_HEIGHT),
        );
        let response = ui.interact(rect, ui.id().with(("segment", index)), Sense::click());
        let selected = *value == *candidate;
        let over = motion::hover(ui.ctx(), response.id, response.hovered() && !selected);
        let press = motion::press(ui.ctx(), response.id, response.is_pointer_button_down_on());
        if over > 0.0 {
            ui.painter().rect_filled(
                pressed_rect(rect, press),
                inner_radius * press_scale(press),
                p.hover.gamma_multiply(over),
            );
        }
        let (family, color) = if selected {
            (theme::font_medium(), p.ink)
        } else {
            (egui::FontFamily::Proportional, p.secondary)
        };
        let font = FontId::new(token::FONT_UI, family);
        paint_pressed_label(ui.painter(), rect.center(), label, font, color, press);
        focus_ring(ui, &response, rect, inner_radius, p);
        if response.clicked() && !selected {
            *value = *candidate;
            changed = true;
        }
    }
    changed
}

/// A small rounded key cap, as used in the shortcut list.
pub fn key_cap(ui: &mut Ui, text: &str) {
    let p = palette(ui);
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        FontId::new(token::FONT_SMALL, theme::font_medium()),
        p.secondary,
    );
    let width = (galley.size().x + 16.0).max(KEY_HEIGHT);
    let (rect, _) = ui.allocate_exact_size(vec2(width, KEY_HEIGHT), Sense::hover());
    ui.painter().rect_filled(rect, 6.0, p.field);
    let pos = rect.center() - galley.size() * 0.5;
    ui.painter().galley(pos, galley, p.secondary);
}

/// Draws a shortcut such as "Ctrl+Shift+O" as a row of key caps.
pub fn key_caps(ui: &mut Ui, shortcut: &str) {
    ui.spacing_mut().item_spacing.x = token::SPACE[0];
    for key in shortcut.split('+') {
        key_cap(ui, key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_band(ctx: &Context, id: Id, time: f64, top: f32) -> (f32, f32) {
        let mut out = (0.0, 0.0);
        let _ = ctx.run(egui::RawInput { time: Some(time), ..Default::default() }, |c| {
            out = stretch_band(c, id, top, top + 32.0);
        });
        out
    }

    #[test]
    fn band_stretches_while_travelling_and_rests_exact() {
        let ctx = Context::default();
        let id = Id::new("band");
        run_band(&ctx, id, 0.0, 0.0);
        let (a, b) = run_band(&ctx, id, 0.05, 320.0);
        assert!(b - a > 32.0, "no stretch: {a} {b}");
        let mut rest = (0.0, 0.0);
        for f in 4..120 {
            rest = run_band(&ctx, id, f as f64 / 60.0, 320.0);
        }
        assert_eq!(rest, (320.0, 352.0));
    }

    #[test]
    fn band_leads_with_the_edge_on_the_side_of_travel_when_moving_up() {
        let ctx = Context::default();
        let id = Id::new("band-up");
        run_band(&ctx, id, 0.0, 320.0);
        let (a, b) = run_band(&ctx, id, 0.05, 0.0);
        assert!(b - a > 32.0, "no stretch: {a} {b}");
        assert!(320.0 - a > 352.0 - b, "top should lead: {a} {b}");
    }

    #[test]
    fn restarted_band_snaps_to_its_target() {
        let ctx = Context::default();
        let id = Id::new("band-restart");
        run_band(&ctx, id, 0.0, 0.0);
        run_band(&ctx, id, 0.05, 320.0);
        restart_band(&ctx, id);
        assert_eq!(run_band(&ctx, id, 0.1, 64.0), (64.0, 96.0));
    }

    #[test]
    fn press_scale_spans_four_percent() {
        assert_eq!(press_scale(0.0), 1.0);
        assert!((press_scale(1.0) - 0.96).abs() < 1e-6);
    }

    #[test]
    fn pressed_rect_is_identity_at_rest_and_keeps_its_centre() {
        let rect = Rect::from_min_size(egui::pos2(10.0, 20.0), vec2(80.0, 32.0));
        assert_eq!(pressed_rect(rect, 0.0), rect);
        let pressed = pressed_rect(rect, 1.0);
        assert_eq!(pressed.center(), rect.center());
        assert!((pressed.width() - 80.0 * 0.96).abs() < 1e-4);
    }
}
