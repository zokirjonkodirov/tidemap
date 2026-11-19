//! Dialogs: settings, export, shortcuts, diagnostics, close, recovery and conflict.
//!
//! Every dialog is a rounded floating surface over a scrim, with a 20 px title, sections spaced
//! by whitespace and exactly one primary button, right-aligned.

use egui::{Align, Context, Frame, Id, Layout, Margin, RichText, Ui, vec2};
use tidemap_diagram::ColorBy;
use tidemap_model::{ExportFormat, MotionPref, Theme};
use tidemap_renderer::export;

pub use super::controls::{danger_button, primary_button, quiet_button};
use super::{
    controls::{key_caps, segmented, solid_scroll, switch},
    surface,
};
use crate::{
    app::{Dialog, TidemapApp},
    motion,
    theme::{self, Palette, token},
};

const DIALOG_PADDING: i8 = 24;
const WIDTH: f32 = 480.0;
const SHORTCUTS_WIDTH: f32 = 520.0;
const DIAGNOSTICS_WIDTH: f32 = 600.0;
const GROUP_GAP: f32 = 24.0;
const ROW_GAP: f32 = 8.0;
/// Space kept above and below a dialog in a short window.
const WINDOW_MARGIN: f32 = 96.0;

/// Light on the very first run, the stored choice otherwise.
pub fn initial_theme(settings_file_existed: bool, stored: Theme) -> Theme {
    if settings_file_existed {
        stored
    } else {
        Theme::Light
    }
}

/// The surface of a dialog: `Modal` elevation, radius 16, 24 px padding.
pub fn dialog_surface<R>(ui: &mut Ui, p: Palette, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    theme::elevated(ui, p, theme::Level::Modal, token::RADIUS_DIALOG, Margin::same(DIALOG_PADDING), add)
}

/// Ctx-data key of the dialog that was open last, so it can be drawn while it leaves.
fn last_dialog_key() -> Id {
    Id::new("workspace-dialog").with("last")
}

/// Id of the presence spring of the dialog area `id`.
fn presence_id(id: &'static str) -> Id {
    Id::new(id).with("presence")
}

/// Shows whichever dialogs are open (or still leaving). Returns the strongest presence among
/// them (0 when none is showing), so the frame can sink the window behind it.
pub fn show(app: &mut TidemapApp, ctx: &Context, p: Palette) -> f32 {
    let mut strongest = 0.0_f32;
    if let Some(dialog) = app.dialog {
        ctx.data_mut(|data| data.insert_temp(last_dialog_key(), dialog));
    }
    let leaving = ctx.data(|data| data.get_temp::<Dialog>(last_dialog_key()));
    let open = app.dialog.is_some();
    if let Some(t) = motion::presence(ctx, presence_id("workspace-dialog"), open) {
        // A leaving dialog is drawn from the last one that was open.
        if let Some(dialog) = app.dialog.or(leaving) {
            let width = match dialog {
                Dialog::Help => SHORTCUTS_WIDTH,
                Dialog::Diagnostics => DIAGNOSTICS_WIDTH,
                _ => WIDTH,
            };
            let shown = surface_modal(ctx, "workspace-dialog", p, t, open, |ui, interactive| {
                ui.set_width(width);
                match dialog {
                    Dialog::Settings => settings(app, ui, p),
                    Dialog::Help => shortcuts(app, ui, p),
                    Dialog::Diagnostics => diagnostics(app, ui, ctx, p, interactive),
                    Dialog::Export => export_dialog(app, ui, p),
                    Dialog::Close => close_dialog(app, ui, ctx, p),
                }
            });
            if open && shown.backdrop_clicked {
                app.dialog = None;
            }
        }
        strongest = strongest.max(t);
    }
    // Recovery and conflict are enter-only: their state is gone once they close.
    if let Some(id) = app.recovery_file.clone() {
        strongest = strongest.max(recovery(app, ctx, p, id));
    } else {
        rest(ctx, "recover-source");
    }
    if let Some(id) = app.conflict_file.clone() {
        strongest = strongest.max(conflict(app, ctx, p, id));
    } else {
        rest(ctx, "external-conflict");
    }
    strongest
}

/// A closed enter-only dialog lets its presence spring settle at 0, so the next opening
/// animates in again.
fn rest(ctx: &Context, id: &'static str) {
    let _ = motion::presence(ctx, presence_id(id), false);
}

/// A dialog over the scrim, growing from the screen centre while open and shrinking back as it
/// leaves.
fn surface_modal<R>(
    ctx: &Context,
    id: &'static str,
    p: Palette,
    t: f32,
    open: bool,
    add: impl FnOnce(&mut Ui, bool) -> R,
) -> surface::Shown<R> {
    let place = surface::Placement {
        id: Id::new(id),
        anchor: egui::Align2::CENTER_CENTER,
        offset: egui::Vec2::ZERO,
        origin: ctx.content_rect().center(),
        enter: surface::DIALOG,
    };
    surface::modal(ctx, p, place, t, open, |ui, interactive| dialog_surface(ui, p, |ui| add(ui, interactive)).inner)
}

/// A dialog that only ever enters: its state is gone the moment it closes. Returns its
/// presence along with what the content returned.
fn show_modal<R>(
    ctx: &Context,
    id: &'static str,
    p: Palette,
    add: impl FnOnce(&mut Ui) -> R,
) -> (f32, surface::Shown<R>) {
    let t = motion::presence(ctx, presence_id(id), true).unwrap_or(1.0);
    (t, surface_modal(ctx, id, p, t, true, |ui, _| add(ui)))
}

fn title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(token::FONT_TITLE).family(theme::font_semibold()));
    ui.add_space(token::SPACE[1]);
}

fn body(ui: &mut Ui, p: Palette, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(token::FONT_UI).color(p.secondary)).wrap());
}

fn note(ui: &mut Ui, text: &str, color: egui::Color32) {
    ui.add(egui::Label::new(RichText::new(text).size(token::FONT_SMALL).color(color)).wrap());
}

/// The group name above a block of rows.
fn group(ui: &mut Ui, p: Palette, name: &str) {
    ui.add_space(GROUP_GAP - ROW_GAP);
    ui.label(
        RichText::new(name)
            .font(egui::FontId::new(token::FONT_SMALL, theme::font_medium()))
            .color(p.muted),
    );
    ui.add_space(ROW_GAP);
}

/// A name (with an optional second line) painted as one block, so it centres against its control.
fn label_block(ui: &mut Ui, p: Palette, name: &str, detail: &str) {
    let width = ui.available_width();
    let name = ui.painter().layout(name.to_owned(), egui::FontId::proportional(token::FONT_UI), p.ink, width);
    let detail = (!detail.is_empty()).then(|| {
        ui.painter().layout(detail.to_owned(), egui::FontId::proportional(token::FONT_SMALL), p.muted, width)
    });
    let height = name.size().y + detail.as_ref().map_or(0.0, |galley| galley.size().y);
    let wide = detail.as_ref().map_or(name.size().x, |galley| galley.size().x.max(name.size().x));
    let (rect, _) = ui.allocate_exact_size(vec2(wide, height), egui::Sense::hover());
    let below = rect.top() + name.size().y;
    ui.painter().galley(rect.min, name, p.ink);
    if let Some(detail) = detail {
        ui.painter().galley(egui::pos2(rect.left(), below), detail, p.muted);
    }
}

/// A label (with an optional second line) on the left and its control on the right.
fn row(ui: &mut Ui, p: Palette, name: &str, detail: &str, control: impl FnOnce(&mut Ui)) {
    let height = if detail.is_empty() { token::CONTROL } else { 44.0 };
    egui::containers::Sides::new()
        .height(height)
        .spacing(token::SPACE[3])
        .shrink_left()
        .wrap()
        .show(ui, |ui| label_block(ui, p, name, detail), control);
}

/// Right-aligned action buttons. The first one added ends up rightmost.
fn actions(ui: &mut Ui, add_buttons: impl FnOnce(&mut Ui)) {
    ui.add_space(token::SPACE[3]);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = token::SPACE[1];
        add_buttons(ui);
    });
}

/// A scroll area that never makes the dialog taller than the window, with a bar of its own width
/// beside the content (see [`solid_scroll`]).
fn scroll(ui: &mut Ui, ctx: &Context, add_contents: impl FnOnce(&mut Ui)) {
    let height = (ctx.content_rect().height() - 2.0 * WINDOW_MARGIN - 120.0).max(160.0);
    solid_scroll(ui, |ui| egui::ScrollArea::vertical().max_height(height).auto_shrink([false, true]).show(ui, add_contents));
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

fn settings(app: &mut TidemapApp, ui: &mut Ui, p: Palette) {
    title(ui, "Settings");
    let ctx = ui.ctx().clone();
    let mut changed = false;
    let mut close = false;
    scroll(ui, &ctx, |ui| {
        ui.spacing_mut().item_spacing.y = ROW_GAP;
        group(ui, p, "Appearance");
        row(ui, p, "Theme", "", |ui| {
            let options = [(Theme::Light, "Light"), (Theme::Dark, "Dark"), (Theme::System, "Match system")];
            changed |= segmented(ui, &mut app.settings.theme, &options);
        });

        row(ui, p, "Motion", "Reduced keeps fades but drops movement", |ui| {
            let options = [(MotionPref::System, "System"), (MotionPref::Reduced, "Reduced"), (MotionPref::Full, "Full")];
            changed |= segmented(ui, &mut app.settings.motion, &options);
        });

        group(ui, p, "Diagram style");
        let mut color_by = app.style_file.sheet.color_by;
        row(ui, p, "Colour nodes by", "", |ui| {
            segmented(ui, &mut color_by, &[(ColorBy::Role, "Role"), (ColorBy::Subgraph, "Subgraph")]);
        });
        let mut open_file = false;
        row(ui, p, "Style file", "Colours, spacing and shapes.", |ui| {
            open_file = quiet_button(ui, "Open style file").clicked();
        });
        let (mut ask_reset, mut keep, mut reset) = (false, false, false);
        if app.style_confirm {
            row(ui, p, "Replace your style file with the defaults?", "", |ui| {
                reset = danger_button(ui, "Reset").clicked();
                keep = quiet_button(ui, "Keep my style").clicked();
            });
        } else {
            row(ui, p, "Reset style", "Go back to the built-in look.", |ui| {
                ask_reset = quiet_button(ui, "Reset to defaults").clicked();
            });
        }
        if let Some(error) = &app.style_file.error {
            note(ui, error, p.danger);
        }
        app.apply_style_choices(color_by, open_file, ask_reset, keep, reset);

        group(ui, p, "Files");
        row(ui, p, "Autosave", "Save after a pause. Conflicts always ask first.", |ui| {
            changed |= switch(ui, &mut app.settings.autosave);
        });
        row(ui, p, "Include ignored files", "Show files that .gitignore excludes.", |ui| {
            changed |= switch(ui, &mut app.settings.include_ignored);
        });

        group(ui, p, "Canvas");
        row(ui, p, "Minimap", "Keep the shape of a large diagram in view.", |ui| {
            changed |= switch(ui, &mut app.settings.show_minimap);
        });

        if !app.recoveries.is_empty() {
            group(ui, p, "Recover unsaved work");
            let records: Vec<(String, String)> = app
                .recoveries
                .iter()
                .take(6)
                .map(|record| {
                    let name = record.path.file_name().unwrap_or(record.path.as_os_str());
                    (record.file_id.clone(), name.to_string_lossy().into_owned())
                })
                .collect();
            for (id, name) in records {
                if quiet_button(ui, &name).clicked() {
                    app.open_document(id, None, true);
                    close = true;
                }
            }
        }
        ui.add_space(token::SPACE[1]);
        note(ui, "Preferences and recovery copies live in app data. Your folders hold only your files.", p.muted);
    });
    if changed {
        app.save_settings();
    }

    let (mut shortcuts, mut diagnostics) = (false, false);
    ui.add_space(token::SPACE[3]);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = token::SPACE[1];
        shortcuts = quiet_button(ui, "Keyboard shortcuts").clicked();
        diagnostics = quiet_button(ui, "Diagnostics").clicked();
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            close |= primary_button(ui, "Done").clicked();
        });
    });
    if shortcuts {
        app.dialog = Some(Dialog::Help);
    } else if diagnostics {
        app.open_diagnostics();
    } else if close {
        app.dialog = None;
    }
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

fn export_dialog(app: &mut TidemapApp, ui: &mut Ui, p: Palette) {
    title(ui, "Export diagram");
    ui.spacing_mut().item_spacing.y = ROW_GAP;
    let artifact = app.active_preview().map(|preview| preview.artifact.clone());
    let png = app.export.format == ExportFormat::Png;
    let limit = artifact
        .as_ref()
        .filter(|_| png)
        .and_then(|artifact| export::png_limit_error(artifact, app.export.scale as f32));

    ui.add_space(token::SPACE[1]);
    row(ui, p, "Format", "SVG stays sharp at any size. PNG is a picture.", |ui| {
        segmented(ui, &mut app.export.format, &[(ExportFormat::Svg, "SVG"), (ExportFormat::Png, "PNG")]);
    });
    if png {
        row(ui, p, "Scale", "Pixels per diagram unit.", |ui| {
            segmented(ui, &mut app.export.scale, &[(1, "1x"), (2, "2x"), (3, "3x")]);
        });
    }
    row(ui, p, "Background", "Paint the canvas colour behind the diagram.", |ui| {
        switch(ui, &mut app.export.background);
    });
    ui.add_space(token::SPACE[1]);
    if let Some(message) = &limit {
        note(ui, message, p.danger);
    } else if let (true, Some(artifact)) = (png, &artifact) {
        let (width, height) = export::png_size(artifact, app.export.scale as f32);
        note(ui, &format!("Image size: {width} x {height} px"), p.muted);
    } else {
        note(ui, "Exported with the current appearance.", p.muted);
    }

    let ready = artifact.is_some() && limit.is_none() && app.export.job.is_none();
    let (mut start, mut cancel) = (false, false);
    actions(ui, |ui| {
        ui.add_enabled_ui(ready, |ui| start = primary_button(ui, "Export").clicked());
        cancel = quiet_button(ui, "Cancel").clicked();
    });
    if start {
        app.start_export();
    } else if cancel {
        app.dialog = None;
    }
}

// ---------------------------------------------------------------------------
// Keyboard shortcuts
// ---------------------------------------------------------------------------

/// Groups of (action, key chord), in the order shown. Alternatives are separated by " or ".
const SHORTCUT_GROUPS: [(&str, &[(&str, &str)]); 7] = [
    ("Go anywhere", &[
        ("Find a file or diagram", "Ctrl+P or Ctrl+K"),
        ("Run a command", "Ctrl+Shift+P"),
        ("Switch project", "Ctrl+R"),
        ("Previous file", "Ctrl+Tab"),
        ("Back", "Alt+Left"),
        ("Forward", "Alt+Right"),
    ]),
    ("Files", &[
        ("Search files in the sidebar", "/ or Ctrl+Shift+F"),
        ("Go to the file tree", "Ctrl+Shift+E"),
        ("Move in the tree", "Up or Down"),
        ("Fold, unfold, go to parent", "Left or Right"),
        ("Open the file under the cursor", "Enter"),
        ("Pin or unpin the file", "Ctrl+D"),
        ("Open folder", "Ctrl+Shift+O"),
        ("Open file", "Ctrl+O"),
        ("New diagram", "Ctrl+N"),
    ]),
    ("Diagram", &[
        ("Previous or next diagram", "[ or ]"),
        ("Find in the diagram or source", "Ctrl+F"),
        ("Fit to window", "F or 0"),
        ("Actual size", "1"),
        ("Zoom in or out", "= or -"),
        ("Select the next or previous node", "Tab or Shift+Tab"),
        ("Open the selected node in the source", "Enter"),
        ("Pan", "Arrows or Drag"),
        ("Clear the selection", "Esc"),
    ]),
    ("Edit", &[
        ("Show or hide source", "Ctrl+E"),
        ("Save", "Ctrl+S"),
        ("Save a copy", "Ctrl+Shift+S"),
        ("Export diagram", "Ctrl+Shift+X"),
    ]),
    ("Source editor", &[
        ("Select a word, a line", "Double click, triple click"),
        ("Move by word", "Ctrl+Left or Ctrl+Right"),
        ("Select to the start or end", "Shift+Home or Shift+End"),
        ("Select all, select line", "Ctrl+A or Ctrl+L"),
        ("Undo or redo", "Ctrl+Z or Ctrl+Y"),
        ("Indent or outdent lines", "Tab or Shift+Tab"),
        ("Move the lines up or down", "Alt+Up or Alt+Down"),
        ("Delete the line", "Ctrl+Shift+K"),
        ("Open a line below or above", "Ctrl+Enter or Ctrl+Shift+Enter"),
    ]),
    ("Page", &[
        ("Switch between the page and the diagram", "Ctrl+Shift+M"),
        ("Scroll", "Space, Page Up or Page Down"),
        ("Open a diagram from the page", "Double click"),
        ("Edit the text under the pointer", "Alt+Click"),
    ]),
    ("Window", &[
        ("Show or hide sidebar", "Ctrl+B"),
        ("Show or hide the outline", "Ctrl+Shift+L"),
        ("Switch light or dark", "Ctrl+Shift+D"),
        ("Toggle reduced motion in the command palette", "Ctrl+Shift+P"),
        ("Settings", "Ctrl+,"),
        ("This list", "Ctrl+/ or ? or F1"),
    ]),
];

/// Key caps for "Ctrl+Shift+P", with " or " between alternatives.
fn chord(ui: &mut Ui, p: Palette, keys: &str) {
    ui.spacing_mut().item_spacing.x = token::SPACE[0];
    for (index, alternative) in keys.split(" or ").enumerate() {
        if index > 0 {
            ui.label(RichText::new("or").size(token::FONT_SMALL).color(p.muted));
        }
        key_caps(ui, alternative);
    }
}

fn shortcuts(app: &mut TidemapApp, ui: &mut Ui, p: Palette) {
    title(ui, "Keyboard shortcuts");
    let ctx = ui.ctx().clone();
    ui.add_space(token::SPACE[1]);
    scroll(ui, &ctx, |ui| {
        for (index, (name, list)) in SHORTCUT_GROUPS.iter().enumerate() {
            if index > 0 {
                ui.add_space(token::SPACE[1]);
            }
            group(ui, p, name);
            egui::Grid::new(("shortcut-list", index))
                .num_columns(2)
                .spacing(vec2(token::SPACE[4], ROW_GAP))
                .min_col_width(240.0)
                .show(ui, |ui| {
                    for (action, keys) in *list {
                        ui.label(RichText::new(*action).size(token::FONT_UI).color(p.ink));
                        ui.horizontal(|ui| chord(ui, p, keys));
                        ui.end_row();
                    }
                });
        }
    });
    ui.add_space(token::SPACE[2]);
    note(ui, "On macOS use Command in place of Ctrl.", p.muted);
    let mut done = false;
    actions(ui, |ui| done = primary_button(ui, "Done").clicked());
    if done {
        app.dialog = None;
    }
}

// ---------------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------------

fn diagnostics(app: &mut TidemapApp, ui: &mut Ui, ctx: &Context, p: Palette, interactive: bool) {
    title(ui, "Diagnostics");
    note(ui, "Local runtime information. Diagram source is not included.", p.muted);
    ui.add_space(token::SPACE[2]);
    Frame::new().fill(p.field).corner_radius(token::RADIUS_PANEL).inner_margin(Margin::same(12)).show(ui, |ui| {
        egui::ScrollArea::both().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut app.diagnostics_text)
                    .interactive(interactive)
                    .font(egui::TextStyle::Monospace)
                    .frame(false)
                    .desired_width(f32::INFINITY)
                    .desired_rows(14),
            );
        });
    });
    let (mut copy, mut done) = (false, false);
    actions(ui, |ui| {
        done = primary_button(ui, "Done").clicked();
        copy = quiet_button(ui, "Copy report").clicked();
    });
    if copy {
        ctx.copy_text(app.diagnostics_text.clone());
        app.notify("Diagnostics copied");
    }
    if done {
        app.dialog = None;
    }
}

// ---------------------------------------------------------------------------
// Close with unsaved changes
// ---------------------------------------------------------------------------

fn close_dialog(app: &mut TidemapApp, ui: &mut Ui, ctx: &Context, p: Palette) {
    title(ui, "Keep your changes?");
    let count = app.unsaved_count();
    let files = if count == 1 { "file" } else { "files" };
    body(ui, p, &format!("You have unsaved work in {count} {files}."));
    note(ui, "Save everything, throw the recovery copies away, or go back to editing.", p.muted);
    let (mut save, mut discard, mut cancel) = (false, false, false);
    actions(ui, |ui| {
        save = primary_button(ui, "Save all and close").clicked();
        cancel = quiet_button(ui, "Cancel").clicked();
        discard = danger_button(ui, "Discard and close").clicked();
    });
    if save {
        app.save_and_close(ctx);
    } else if discard {
        app.discard_and_close(ctx);
    } else if cancel {
        app.dialog = None;
    }
}

// ---------------------------------------------------------------------------
// Recovery and conflict
// ---------------------------------------------------------------------------

fn recovery(app: &mut TidemapApp, ctx: &Context, p: Palette, id: String) -> f32 {
    let Some(info) = app.recovery_info(&id) else {
        app.recovery_file = None;
        return 0.0;
    };
    let (mut recover, mut discard) = (false, false);
    let (t, _) = show_modal(ctx, "recover-source", p, |ui| {
        ui.set_width(WIDTH);
        title(ui, "Pick up where you left off");
        body(ui, p, &format!("There are unsaved changes for {}.", info.name));
        ui.add_space(token::SPACE[1]);
        note(ui, "Recover them into the editor, or use the file as it is on disk.", p.muted);
        if info.disk_changed {
            ui.add_space(token::SPACE[1]);
            note(ui, "The file also changed on disk. Both versions are kept for review.", p.muted);
        }
        actions(ui, |ui| {
            recover = primary_button(ui, "Recover changes").clicked();
            discard = quiet_button(ui, "Use file on disk").clicked();
        });
    });
    if recover {
        app.recover_changes(id);
    } else if discard {
        app.use_disk_version(id);
    }
    t
}

fn conflict(app: &mut TidemapApp, ctx: &Context, p: Palette, id: String) -> f32 {
    let Some(info) = app.conflict_info(&id) else {
        app.conflict_file = None;
        return 0.0;
    };
    let width = (ctx.content_rect().width() - 2.0 * WINDOW_MARGIN).clamp(WIDTH, 980.0);
    let (mut keep, mut reload, mut copy) = (false, false, false);
    let (t, modal) = show_modal(ctx, "external-conflict", p, |ui| {
        ui.set_width(width);
        title(ui, "This file changed on disk");
        body(ui, p, &format!("{} has a different version on disk. Your work is safe; review both before continuing.", info.name));
        if let Some(hash) = &info.disk_hash {
            note(ui, &format!("Disk version {}", hash.chars().take(12).collect::<String>()), p.muted);
        }
        ui.add_space(token::SPACE[2]);
        ui.columns(2, |columns| {
            pane(&mut columns[0], p, "Your unsaved source", &info.ours);
            pane(&mut columns[1], p, "File on disk", &info.theirs);
        });
        actions(ui, |ui| {
            copy = primary_button(ui, "Save my work as a copy").clicked();
            keep = quiet_button(ui, "Keep editing").clicked();
            ui.add_enabled_ui(!info.deleted, |ui| reload = danger_button(ui, "Discard my edits and reload").clicked());
        });
    });
    if keep || modal.backdrop_clicked {
        app.conflict_file = None;
    }
    if reload {
        app.reload_conflict(id.clone());
    }
    if copy {
        app.save(id, true);
    }
    t
}

/// A read-only, scrollable box with a caption, used to compare two versions.
fn pane(ui: &mut Ui, p: Palette, caption: &str, text: &str) {
    ui.label(RichText::new(caption).font(egui::FontId::new(token::FONT_SMALL, theme::font_medium())).color(p.secondary));
    ui.add_space(token::SPACE[0]);
    Frame::new().fill(p.field).corner_radius(token::RADIUS_PANEL).inner_margin(Margin::same(12)).show(ui, |ui| {
        egui::ScrollArea::both()
            .id_salt(caption)
            .max_height(280.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(RichText::new(text).monospace().size(token::FONT_MONO).color(p.ink))
                        .wrap_mode(egui::TextWrapMode::Extend)
                        .selectable(false),
                );
            });
    });
}
