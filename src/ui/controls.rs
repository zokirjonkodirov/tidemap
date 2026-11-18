//! Hand-painted controls shared by the dialogs, the palette and the empty states.
//!
//! The global egui visuals draw no widget strokes, so anything that has to read as a control
//! (switch, segmented choice, key cap, button) is painted here from the palette.

use egui::{Align2, Color32, FontId, Rect, Response, Sense, Stroke, StrokeKind, Ui, vec2};

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
const SEGMENT_PAD: f32 = 2.0;
const SEGMENT_HEIGHT: f32 = 28.0;
const SEGMENT_TEXT_PAD: f32 = 16.0;
const KEY_HEIGHT: f32 = 24.0;

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

/// A rounded text button. `fill` is painted always; `hover` eases in while hovered and a press
/// sinks the label half a point.
fn text_button(ui: &mut Ui, text: &str, fill: Color32, hover: Color32, color: Color32) -> Response {
    let p = palette(ui);
    let enabled = ui.is_enabled();
    let galley = ui.painter().layout_no_wrap(text.to_owned(), medium(), color);
    let size = vec2(galley.size().x + 2.0 * BUTTON_PAD, BUTTON_HEIGHT);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let ctx = ui.ctx().clone();
    let over = motion::toggle(&ctx, response.id.with("hover"), enabled && response.hovered(), motion::FAST);
    let press = motion::toggle(&ctx, response.id.with("press"), enabled && response.is_pointer_button_down_on(), 0.08);
    ui.painter().rect_filled(rect, token::RADIUS_CONTROL, fill.lerp_to_gamma(hover, over));
    let color = if enabled { color } else { p.muted };
    let pos = rect.center() - galley.size() * 0.5 + vec2(0.0, 0.5 * press);
    ui.painter().galley(pos, galley, color);
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

/// A 36 x 20 pill switch with a 16 px knob. Returns true when toggled.
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
    let travel_t = motion::tween(ui.ctx(), response.id.with("switch"), f32::from(u8::from(*value)), motion::BASE + 0.04, motion::ease_out_back);
    let on = travel_t.clamp(0.0, 1.0);
    let track = Rect::from_center_size(area.center(), SWITCH_SIZE);
    let track_color = p.track_off.lerp_to_gamma(p.accent, on);
    ui.painter().rect_filled(track, SWITCH_SIZE.y * 0.5, track_color);
    let travel = SWITCH_SIZE.x - SWITCH_SIZE.y;
    let knob = egui::pos2(track.left() + SWITCH_SIZE.y * 0.5 + travel * travel_t.clamp(-0.06, 1.06), track.center().y);
    let knob_color = p.knob_off.lerp_to_gamma(p.on_accent, on);
    ui.painter().circle_filled(knob, KNOB_RADIUS, knob_color);
    focus_ring(ui, &response, track, SWITCH_SIZE.y * 0.5, p);
    changed
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
        let left = motion::tween(&ctx, id.with("left"), lefts[chosen], motion::BASE, motion::ease_out);
        let w = motion::tween(&ctx, id.with("width"), widths[chosen], motion::BASE, motion::ease_out);
        let band = Rect::from_min_size(egui::pos2(left, track.top() + SEGMENT_PAD), vec2(w, SEGMENT_HEIGHT));
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
        let over = motion::toggle(ui.ctx(), response.id.with("hover"), response.hovered() && !selected, motion::FAST);
        if over > 0.0 {
            ui.painter().rect_filled(rect, inner_radius, p.hover.gamma_multiply(over));
        }
        let (family, color) = if selected {
            (theme::font_medium(), p.ink)
        } else {
            (egui::FontFamily::Proportional, p.secondary)
        };
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            *label,
            FontId::new(token::FONT_UI, family),
            color,
        );
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
