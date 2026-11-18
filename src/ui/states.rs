//! Empty states, the diagram search overlay and the outline panel.

use egui::{
    Align, Align2, Area, Context, FontId, Id, Layout, Margin, Order, Rect, RichText, Sense, Ui, UiBuilder,
    pos2, vec2,
};

use super::{
    controls::{primary_button, quiet_button},
    palette::elided,
};
use crate::{
    app::MermaidLiteApp,
    icons::{self, Icon},
    theme::{self, Palette, token},
};

/// Width of the text column of an empty state.
const COLUMN_WIDTH: f32 = 420.0;
/// Width of the list rows (recent folders, recoverable files).
const LIST_WIDTH: f32 = 360.0;
const LIST_ROW: f32 = 40.0;
const LOGO_SIZE: f32 = 56.0;
const SEARCH_FIELD_WIDTH: f32 = 300.0;
const SEARCH_TOP: f32 = 16.0;
/// How many recent folders the welcome state lists.
const RECENT_LIMIT: usize = 5;

/// A column centred in `area`, starting `top_fraction` of the way down.
fn column(ui: &mut Ui, top_fraction: f32, add_contents: impl FnOnce(&mut Ui)) {
    let area = ui.available_rect_before_wrap();
    let x = (area.center().x - COLUMN_WIDTH * 0.5).max(area.left());
    let y = area.top() + (area.height() * top_fraction).round();
    let rect = Rect::from_min_max(pos2(x, y), pos2(x + COLUMN_WIDTH.min(area.width()), area.bottom()));
    let builder = UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Center));
    ui.scope_builder(builder, add_contents);
}

/// Title and one sentence, centred.
fn heading(ui: &mut Ui, p: Palette, title: &str, sentence: &str) {
    ui.label(RichText::new(title).size(token::FONT_TITLE).family(theme::font_semibold()).color(p.ink));
    ui.add_space(token::SPACE[1]);
    ui.add(
        egui::Label::new(RichText::new(sentence).size(token::FONT_UI).color(p.secondary))
            .wrap()
            .halign(Align::Center),
    );
}

/// A calm 56 px tile with the diagram icon.
fn logo(ui: &mut Ui, p: Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(LOGO_SIZE, LOGO_SIZE), Sense::hover());
    ui.painter().rect_filled(rect, token::RADIUS_DIALOG, p.accent_soft);
    let icon = Rect::from_center_size(rect.center(), vec2(24.0, 24.0));
    icons::paint(ui.painter(), Icon::Diagram, icon, p.accent);
}

/// A quiet list row: icon, name, and a muted detail on the right. Returns the row's response.
fn list_row(ui: &mut Ui, p: Palette, icon: Icon, name: &str, detail: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(LIST_WIDTH.min(ui.available_width()), LIST_ROW), Sense::click());
    let hover = crate::motion::toggle(ui.ctx(), response.id.with("hover"), response.hovered(), crate::motion::FAST);
    if hover > 0.0 {
        ui.painter().rect_filled(rect, token::RADIUS_CONTROL, p.hover.gamma_multiply(hover));
    }
    let icon_rect = Rect::from_center_size(pos2(rect.left() + 12.0 + 8.0, rect.center().y), vec2(16.0, 16.0));
    icons::paint(ui.painter(), icon, icon_rect, p.secondary.lerp_to_gamma(p.ink, hover));
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let left = icon_rect.right() + 12.0;
    let detail_galley = elided(ui, detail, FontId::proportional(token::FONT_SMALL), p.muted, rect.width() * 0.4, true);
    let detail_pos = pos2(rect.right() - 12.0 - detail_galley.size().x, rect.center().y - detail_galley.size().y * 0.5);
    let budget = (detail_pos.x - 12.0 - left).max(0.0);
    ui.painter().galley(detail_pos, detail_galley, p.muted);
    let name_galley = elided(ui, name, FontId::proportional(token::FONT_UI), p.ink, budget, false);
    ui.painter().galley(pos2(left, rect.center().y - name_galley.size().y * 0.5), name_galley, p.ink);
    response
}

/// No workspace and no file: invite the user to open a folder.
pub fn welcome(app: &mut MermaidLiteApp, ui: &mut Ui, p: Palette) {
    let recents: Vec<_> = app.recents.iter().take(RECENT_LIMIT).cloned().collect();
    let recoveries: Vec<(String, String, String)> = app
        .recoveries
        .iter()
        .take(RECENT_LIMIT)
        .map(|record| {
            let name = record.path.file_name().unwrap_or(record.path.as_os_str()).to_string_lossy().into_owned();
            (record.file_id.clone(), name, record.path.display().to_string())
        })
        .collect();
    let mut open_folder = None;
    let mut recover = None;
    column(ui, 0.2, |ui| {
        logo(ui, p);
        ui.add_space(token::SPACE[3]);
        heading(
            ui,
            p,
            "Open a folder of diagrams",
            "MermaidLite reads Markdown and .mmd files and keeps everything on this computer.",
        );
        ui.add_space(token::SPACE[4]);
        ui.spacing_mut().item_spacing.y = token::SPACE[0];
        if primary_button(ui, "Open folder").clicked() {
            open_folder = Some(None);
        }
        if quiet_button(ui, "Open a file").clicked() {
            app.open_external(None);
        }
        if !recents.is_empty() {
            ui.add_space(token::SPACE[4]);
            section_label(ui, p, "Recent folders");
            for recent in &recents {
                let detail = super::palette::display_path(&recent.root);
                if list_row(ui, p, Icon::Folder, &recent.name, &detail).clicked() {
                    open_folder = Some(Some(recent.root.clone()));
                }
            }
        }
        if !recoveries.is_empty() {
            ui.add_space(token::SPACE[4]);
            section_label(ui, p, "Recover unsaved work");
            for (id, name, path) in &recoveries {
                if list_row(ui, p, Icon::File, name, path).clicked() {
                    recover = Some(id.clone());
                }
            }
        }
    });
    if let Some(root) = open_folder {
        app.open_folder(root);
    }
    if let Some(id) = recover {
        app.open_document(id, None, true);
    }
}

fn section_label(ui: &mut Ui, p: Palette, text: &str) {
    ui.label(
        RichText::new(text)
            .font(FontId::new(token::FONT_SMALL, theme::font_medium()))
            .color(p.muted),
    );
    ui.add_space(token::SPACE[0]);
}

/// A workspace is open but no file is selected.
pub fn idle(app: &MermaidLiteApp, ui: &mut Ui, p: Palette) {
    let indexing = app.workspace.as_ref().is_some_and(|workspace| workspace.indexing);
    column(ui, 0.3, |ui| {
        if indexing {
            heading(ui, p, "Finding your diagrams\u{2026}", "Your files appear in the sidebar as they are found.");
        } else {
            heading(ui, p, "Pick a file to start reading", "Or press Ctrl+P to search, Ctrl+R to switch project.");
        }
    });
}

/// The open file has no Mermaid block.
pub fn no_diagrams(app: &mut MermaidLiteApp, ui: &mut Ui, p: Palette) {
    let mut open_source = false;
    let source_open = app.editor_open;
    column(ui, 0.3, |ui| {
        heading(ui, p, "No diagrams in this file yet", "Add a fenced mermaid block to see it here.");
        if !source_open {
            ui.add_space(token::SPACE[3]);
            open_source = quiet_button(ui, "Open source").clicked();
        }
    });
    if open_source {
        app.show_full_source();
    }
}

/// The find-in-diagram pill at the top centre of the canvas.
pub fn search_overlay(app: &mut MermaidLiteApp, ctx: &Context, canvas: Rect, p: Palette) {
    if !app.search_open || app.active_file.is_empty() {
        return;
    }
    let count = app.search_hits.len();
    let position = app.search_cursor.map_or(1, |cursor| cursor + 1);
    let has_query = !app.diagram_search.trim().is_empty();
    let mut close = false;
    let mut next = false;
    Area::new(Id::new("diagram-search"))
        .order(Order::Foreground)
        .pivot(Align2::CENTER_TOP)
        .fixed_pos(pos2(canvas.center().x, canvas.top() + SEARCH_TOP))
        .show(ctx, |ui| {
            theme::floating(p, token::RADIUS_PILL)
                .inner_margin(Margin::symmetric(token::SPACE[2] as i8, token::SPACE[0] as i8))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = token::SPACE[1];
                    ui.horizontal(|ui| {
                        let (icon, _) = ui.allocate_exact_size(vec2(16.0, token::CONTROL), Sense::hover());
                        icons::paint(ui.painter(), Icon::Search, Rect::from_center_size(icon.center(), vec2(16.0, 16.0)), p.muted);
                        let edit = egui::TextEdit::singleline(&mut app.diagram_search)
                            .id(Id::new("diagram-search-input"))
                            .font(FontId::proportional(token::FONT_UI))
                            .hint_text(RichText::new("Find in diagram").color(p.muted))
                            .frame(false)
                            .margin(Margin::ZERO)
                            .desired_width(SEARCH_FIELD_WIDTH);
                        let response = ui.add(edit);
                        if app.search_fresh {
                            app.search_fresh = false;
                            response.request_focus();
                        }
                        if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                            next = true;
                            response.request_focus();
                        }
                        let status = if !has_query {
                            String::new()
                        } else if count == 0 {
                            "No matches".to_owned()
                        } else {
                            format!("{position} of {count}")
                        };
                        if !status.is_empty() {
                            ui.label(RichText::new(status).size(token::FONT_SMALL).color(p.muted));
                        }
                        if icons::button(ui, Icon::Close, "Close search", false, true).clicked() {
                            close = true;
                        }
                    });
                });
        });
    if next {
        app.focus_next_match();
    }
    if close {
        app.close_search();
    }
}

/// The outline: every node of the diagram as a row; choosing one finds it on the canvas.
pub fn outline(app: &mut MermaidLiteApp, ui: &mut Ui, p: Palette) {
    let mut close = false;
    egui::containers::Sides::new().show(
        ui,
        |ui| {
            ui.label(
                RichText::new("Outline")
                    .font(FontId::new(token::FONT_SMALL, theme::font_medium()))
                    .color(p.secondary),
            );
        },
        |ui| close = icons::button(ui, Icon::Close, "Close outline", false, true).clicked(),
    );
    if close {
        app.outline_open = false;
    }
    let Some(artifact) = app.active_preview().map(|preview| preview.artifact.clone()) else {
        return;
    };
    let scene = &artifact.rendered.scene;
    let query = if app.search_open { app.diagram_search.trim().to_owned() } else { String::new() };
    let ids = if query.is_empty() {
        scene.nodes.iter().map(|node| node.id).collect::<Vec<_>>()
    } else {
        scene.find(&query)
    };
    ui.label(RichText::new(format!("{} nodes", scene.nodes.len())).size(token::FONT_SMALL).color(p.muted));
    ui.add_space(token::SPACE[1]);
    if ids.is_empty() {
        ui.label(RichText::new("No matching nodes.").size(token::FONT_SMALL).color(p.muted));
        return;
    }
    let mut focus = None;
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, token::ROW_HEIGHT, ids.len(), |ui, range| {
        for index in range {
            let id = ids[index];
            let Some(node) = scene.nodes.get(id.0 as usize) else {
                continue;
            };
            let text = node.text.lines.join(" ");
            let text = if text.is_empty() { scene.node_key(id).to_owned() } else { text };
            let active = app.canvas.selected == Some(id);
            if outline_row(ui, p, &text, active) {
                focus = Some(id);
            }
        }
    });
    if let Some(id) = focus {
        app.canvas.focus_node(scene, id);
        if let Some(span) = scene.nodes.get(id.0 as usize).map(|node| node.span) {
            app.select_in_source(span);
        }
    }
}

/// One 32 px outline row with the same look as a sidebar row.
fn outline_row(ui: &mut Ui, p: Palette, text: &str, active: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), token::ROW_HEIGHT), Sense::click());
    let ctx = ui.ctx().clone();
    let on = crate::motion::toggle(&ctx, response.id.with("active"), active, crate::motion::BASE);
    let hover = crate::motion::toggle(&ctx, response.id.with("hover"), response.hovered() && !active, crate::motion::FAST);
    if on > 0.0 {
        ui.painter().rect_filled(rect, token::RADIUS_CONTROL, p.accent_soft.gamma_multiply(on));
    }
    if hover > 0.0 {
        ui.painter().rect_filled(rect, token::RADIUS_CONTROL, p.hover.gamma_multiply(hover));
    }
    let (family, color) = if active {
        (theme::font_medium(), p.accent)
    } else {
        (egui::FontFamily::Proportional, p.ink)
    };
    let galley = elided(ui, text, FontId::new(token::FONT_UI, family), color, rect.width() - 16.0, false);
    ui.painter().galley(pos2(rect.left() + 8.0, rect.center().y - galley.size().y * 0.5), galley, color);
    response.clicked()
}
