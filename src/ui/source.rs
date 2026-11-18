//! The source card: a header with the scope switch and the code editor.

use egui::{Align, Layout, Rect, Sense, Ui, vec2};
use mermaidlite_model::ViewState;

use crate::{
    app::MermaidLiteApp,
    icons::{self, Icon},
    theme::{self, Palette, token},
};

const HEADER_HEIGHT: f32 = 36.0;
const SEGMENT_WIDTH: f32 = 72.0;
const SEGMENT_HEIGHT: f32 = 28.0;
const TRACK_PAD: f32 = 2.0;

pub fn show_source(app: &mut MermaidLiteApp, ui: &mut Ui, p: Palette) {
    let (is_markdown_file, read_only) = app
        .buffers
        .get(&app.active_file)
        .map(|buffer| {
            (
                !buffer.document.name.to_lowercase().ends_with(".mmd"),
                buffer.document.read_only,
            )
        })
        .unwrap_or_default();

    header(app, ui, p, is_markdown_file);

    app.sync_editor();
    let markdown = app.full_file && is_markdown_file;
    let view_key = format!("editor:{}", app.editor_key);
    let position = app.state.views.get(&view_key).cloned().unwrap_or_default();

    if read_only {
        ui.horizontal(|ui| {
            theme::caption(ui, p, "Read-only source");
            if ui.small_button("Save a copy").clicked() {
                app.save(app.active_file.clone(), true);
            }
        });
    }

    // The editor's scroll area takes exactly the rest of the card.
    let rest = ui.available_rect_before_wrap();
    ui.scope_builder(egui::UiBuilder::new().max_rect(rest), |ui| {
        ui.set_clip_rect(rest.intersect(ui.clip_rect()));
        let response = app.editor.show(
            ui,
            &app.editor_key,
            &mut app.source_text,
            markdown,
            read_only,
            p,
            position.cursor,
            position.scroll,
        );
        app.state.views.insert(
            view_key,
            ViewState {
                cursor: response.cursor,
                scroll: response.scroll,
                ..position
            },
        );
        if response.changed {
            app.apply_source();
        } else if response.focused && app.cursor_seen.is_some_and(|seen| seen != response.cursor) {
            app.select_from_cursor(response.cursor);
        }
        app.cursor_seen = Some(response.cursor);
    });
}

fn header(app: &mut MermaidLiteApp, ui: &mut Ui, p: Palette, is_markdown_file: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_HEIGHT), Sense::hover());
    let layout = Layout::left_to_right(Align::Center);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(layout), |ui| {
        ui.label(
            egui::RichText::new("Source")
                .font(egui::FontId::new(token::FONT_SMALL, theme::font_medium()))
                .color(p.secondary),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = token::SPACE[1];
            if icons::button(ui, Icon::Close, "Close source · Ctrl+E", false, true).clicked() {
                app.editor_open = false;
                app.record_state();
            }
            if is_markdown_file {
                let has_block = app.block().is_some();
                if scope_switch(ui, p, &mut app.full_file, has_block) {
                    app.editor_key.clear();
                    app.record_state();
                }
            }
        });
    });
}

/// "Diagram" / "Full file" switch: a track in the backdrop colour with a white thumb.
/// Returns true when the choice changed.
fn scope_switch(ui: &mut Ui, p: Palette, full_file: &mut bool, diagram_enabled: bool) -> bool {
    let size = vec2(2.0 * SEGMENT_WIDTH + 2.0 * TRACK_PAD, SEGMENT_HEIGHT + 2.0 * TRACK_PAD);
    let (track, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(track, token::RADIUS_CONTROL, p.app);

    let mut changed = false;
    let options = [(false, "Diagram", diagram_enabled), (true, "Full file", true)];
    for (index, (value, label, enabled)) in options.into_iter().enumerate() {
        let origin = track.min + vec2(TRACK_PAD + index as f32 * SEGMENT_WIDTH, TRACK_PAD);
        let rect = Rect::from_min_size(origin, vec2(SEGMENT_WIDTH, SEGMENT_HEIGHT));
        let sense = if enabled { Sense::click() } else { Sense::hover() };
        let response = ui.interact(rect, ui.id().with(("scope", index)), sense);
        let selected = *full_file == value;
        if selected {
            ui.painter().rect_filled(rect, token::RADIUS_CONTROL - 2.0, p.panel);
        } else if enabled && response.hovered() {
            ui.painter().rect_filled(rect, token::RADIUS_CONTROL - 2.0, p.hover);
        }
        let color = if selected {
            p.ink
        } else if enabled {
            p.secondary
        } else {
            p.muted
        };
        let family = if selected { theme::font_medium() } else { egui::FontFamily::Proportional };
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::new(token::FONT_SMALL, family),
            color,
        );
        if enabled && response.clicked() && !selected {
            *full_file = value;
            changed = true;
        }
    }
    changed
}
