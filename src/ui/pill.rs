//! The floating pill over the canvas: zoom, fit, source and export.

use egui::{Align2, Area, Context, FontId, Id, Order, Pos2, Rect, Sense, Stroke, Ui, vec2};

use crate::{
    app::{Dialog, TidemapApp},
    icons::{self, Icon},
    theme::{self, Palette, token},
};

const PADDING: f32 = 6.0;
const GAP: f32 = 4.0;
const HEIGHT: f32 = 44.0;
const ZOOM_LABEL_WIDTH: f32 = 56.0;
const DIVIDER_HEIGHT: f32 = 16.0;
/// Distance between the pill and the bottom edge of the canvas.
const BOTTOM_MARGIN: f32 = 16.0;
/// Free space kept between the pill and the minimap or the canvas edge.
const CLEARANCE: f32 = 8.0;
/// Buttons: zoom out, zoom in, fit, source, export.
const BUTTONS: f32 = 5.0;
/// Items in the row: five buttons, the zoom label and the divider.
const ITEMS: f32 = 7.0;

/// Width of the pill's content, without its padding.
fn content_width() -> f32 {
    BUTTONS * token::CONTROL + ZOOM_LABEL_WIDTH + 1.0 + (ITEMS - 1.0) * GAP
}

/// Outer width of the pill.
pub fn pill_width() -> f32 {
    content_width() + 2.0 * PADDING
}

/// Bottom-centre anchor of the pill: centred on the canvas, 16 px up, moved left to leave
/// 8 px to the minimap, and kept inside the canvas. When the canvas is too narrow for both
/// side by side, the pill sits above the minimap instead.
pub fn pill_pos(canvas: Rect, minimap: Option<Rect>) -> Pos2 {
    let half = pill_width() * 0.5;
    let mut bottom = canvas.bottom() - BOTTOM_MARGIN;
    let mut x = canvas.center().x;
    let overlaps_row = |bottom: f32, minimap: Rect| bottom - HEIGHT < minimap.bottom() && bottom > minimap.top();
    if let Some(minimap) = minimap.filter(|minimap| overlaps_row(bottom, *minimap)) {
        if x + half > minimap.left() - CLEARANCE {
            x = minimap.left() - CLEARANCE - half;
        }
    }
    let lowest = canvas.left() + CLEARANCE + half;
    let highest = canvas.right() - CLEARANCE - half;
    x = if lowest <= highest { x.clamp(lowest, highest) } else { canvas.center().x };
    if let Some(minimap) = minimap {
        let right = x + half;
        if overlaps_row(bottom, minimap) && right > minimap.left() - CLEARANCE {
            bottom = minimap.top() - CLEARANCE;
        }
    }
    Pos2::new(x, bottom)
}

pub fn show_pill(app: &mut TidemapApp, ctx: &Context, canvas: Rect, p: Palette) {
    if app.active_file.is_empty() {
        return;
    }
    let has_preview = app.active_preview().is_some();
    let export_unavailable = app.export_unavailable();
    let has_block = app.block().is_some();
    let minimap = app.canvas.minimap_rect;
    let position = pill_pos(canvas, minimap);

    Area::new(Id::new("canvas-pill"))
        .order(Order::Foreground)
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(position)
        .show(ctx, |ui| {
            theme::floating(p, token::RADIUS_PILL)
                .inner_margin(egui::Margin::same(PADDING as i8))
                .show(ui, |ui| {
                    ui.set_width(content_width());
                    ui.set_height(HEIGHT - 2.0 * PADDING);
                    ui.spacing_mut().item_spacing = vec2(GAP, 0.0);
                    ui.horizontal_centered(|ui| {
                        row(app, ui, p, has_preview, has_block, export_unavailable);
                    });
                });
        });
}

fn row(
    app: &mut TidemapApp,
    ui: &mut Ui,
    p: Palette,
    has_preview: bool,
    has_block: bool,
    export_unavailable: Option<&'static str>,
) {
    if icons::button(ui, Icon::Minus, "Zoom out · -", false, has_preview).clicked() {
        app.canvas.zoom_by(0.8);
    }
    let percent = (app.canvas.camera.zoom * 100.0).round() as i32;
    if zoom_label(ui, p, &format!("{percent}%"), has_preview).clicked() {
        app.canvas.actual_size();
    }
    if icons::button(ui, Icon::Plus, "Zoom in · =", false, has_preview).clicked() {
        app.canvas.zoom_by(1.25);
    }
    divider(ui, p);
    if icons::button(ui, Icon::Fit, "Fit · F", false, has_preview).clicked() {
        app.canvas.fit();
    }
    let source_open = app.editor_open;
    if icons::button(ui, Icon::Code, "Source · Ctrl+E", source_open, has_block).clicked() {
        app.editor_open = !app.editor_open;
        app.record_state();
    }
    let export_tooltip = export_unavailable.unwrap_or("Export as SVG or PNG · Ctrl+Shift+X");
    if icons::button(ui, Icon::Export, export_tooltip, false, export_unavailable.is_none()).clicked() {
        app.dialog = Some(Dialog::Export);
    }
}

/// The zoom percentage; a click returns to 100%.
fn zoom_label(ui: &mut Ui, p: Palette, text: &str, enabled: bool) -> egui::Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(ZOOM_LABEL_WIDTH, token::CONTROL), sense);
    let hover = crate::motion::toggle(ui.ctx(), response.id.with("hover"), enabled && response.hovered(), crate::motion::FAST);
    if hover > 0.0 {
        ui.painter().rect_filled(rect, token::RADIUS_CONTROL, p.hover.gamma_multiply(hover));
    }
    let color = if enabled { p.ink } else { p.muted };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(token::FONT_SMALL),
        color,
    );
    if enabled {
        response.on_hover_text("Actual size · 1")
    } else {
        response
    }
}

/// A 1 px hairline, 16 px tall, with the usual gap on both sides.
fn divider(ui: &mut Ui, p: Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, token::CONTROL), Sense::hover());
    let top = rect.center().y - DIVIDER_HEIGHT * 0.5;
    ui.painter().line_segment(
        [Pos2::new(rect.center().x, top), Pos2::new(rect.center().x, top + DIVIDER_HEIGHT)],
        Stroke::new(1.0_f32, p.hairline),
    );
}
