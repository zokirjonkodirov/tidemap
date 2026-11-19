//! The slim top bar: navigation on the left, the breadcrumb, quiet actions on the right.

use egui::{Align, Color32, Context, FontId, Id, Layout, Sense, UiBuilder, vec2};
use tidemap_model::Theme;

use super::info;
use crate::{
    app::TidemapApp,
    icons::{self, Icon},
    motion,
    theme::{self, Palette, token},
};

/// Width of the three right-hand buttons and the gaps between them.
const RIGHT_CLUSTER: f32 = 4.0 * token::CONTROL + 3.0 * 4.0;
/// Space kept between the breadcrumb and the right-hand buttons.
const RIGHT_GAP: f32 = token::SPACE[3];
/// Height of the diagram button in the breadcrumb.
const BLOCK_HEIGHT: f32 = 26.0;
/// Padding inside the diagram button.
const BLOCK_PAD: f32 = 8.0;
/// Size of the chevron inside the diagram button.
const CHEVRON: f32 = 12.0;
/// Diameter of the unsaved-changes dot.
const DOT: f32 = 6.0;

/// The pieces of the breadcrumb, as plain text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crumb {
    pub folder: String,
    pub file: String,
    /// Title of the active diagram; only when the file has more than one.
    pub block: Option<String>,
    /// One-based position of the active diagram and the number of diagrams.
    pub position: Option<(usize, usize)>,
}

/// Builds the breadcrumb for `path` (relative to the workspace) and its diagrams
/// (`blocks` holds id and title pairs).
pub fn crumb(workspace_name: &str, path: &str, blocks: &[(String, String)], active_block: &str) -> Crumb {
    let (folder, file) = match path.rsplit_once(['/', '\\']) {
        Some((folder, file)) => (folder.to_owned(), file.to_owned()),
        None => (workspace_name.to_owned(), path.to_owned()),
    };
    let active = blocks.iter().position(|(id, _)| id == active_block);
    let (block, position) = match active {
        Some(index) if blocks.len() > 1 => {
            (Some(blocks[index].1.clone()), Some((index + 1, blocks.len())))
        }
        _ => (None, None),
    };
    Crumb {
        folder,
        file,
        block,
        position,
    }
}

/// Shortens `text` to fit `budget` points, replacing the dropped end with an ellipsis.
/// `keep_end` drops from the left (for folders); otherwise from the right (for names).
fn fit_text(measure: &dyn Fn(&str) -> f32, text: &str, budget: f32, keep_end: bool) -> String {
    if measure(text) <= budget {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    for dropped in 1..chars.len() {
        let candidate: String = if keep_end {
            std::iter::once('\u{2026}').chain(chars[dropped..].iter().copied()).collect()
        } else {
            chars[..chars.len() - dropped].iter().copied().chain(std::iter::once('\u{2026}')).collect()
        };
        if measure(&candidate) <= budget {
            return candidate;
        }
    }
    "\u{2026}".to_owned()
}

/// The last `count` components of a path, joined with `/`.
fn last_components(path: &str, count: usize) -> String {
    let parts: Vec<&str> = path
        .split(['/', '\\'])
        .filter(|part| !part.is_empty() && *part != "?")
        .collect();
    parts[parts.len().saturating_sub(count)..].join("/")
}

/// What the bar needs to know about the open file.
struct Open {
    crumb: Crumb,
    dirty: bool,
    blocks: Vec<(String, String)>,
}

fn open_file(app: &TidemapApp) -> Option<Open> {
    let buffer = app.buffers.get(&app.active_file)?;
    let blocks: Vec<(String, String)> = buffer
        .document
        .blocks
        .iter()
        .map(|block| (block.id.clone(), block.title.clone()))
        .collect();
    let (workspace_name, path) = match &app.workspace {
        Some(workspace) => (workspace.name.as_str(), buffer.document.path.clone()),
        // A single file outside any workspace: show only its folder and name, not the full path.
        None => ("", last_components(&buffer.document.path, 2)),
    };
    Some(Open {
        crumb: crumb(workspace_name, &path, &blocks, &app.active_block),
        dirty: buffer.dirty(),
        blocks,
    })
}

pub fn show(app: &mut TidemapApp, ctx: &Context, p: Palette) {
    let narrow_ok = ctx.content_rect().width() >= token::NARROW;
    egui::TopBottomPanel::top("topbar")
        .exact_height(token::BAR_HEIGHT)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::NONE)
        .show(ctx, |ui| {
            let bar = ui.max_rect().shrink2(vec2(token::INSET, 0.0));
            let left = UiBuilder::new()
                .max_rect(bar)
                .layout(Layout::left_to_right(Align::Center));
            ui.scope_builder(left, |ui| left_side(app, ui, p, narrow_ok));
            let right = UiBuilder::new()
                .max_rect(bar)
                .layout(Layout::right_to_left(Align::Center));
            ui.scope_builder(right, |ui| right_side(app, ui, p));
        });
}

fn left_side(app: &mut TidemapApp, ui: &mut egui::Ui, p: Palette, sidebar_fits: bool) {
    ui.spacing_mut().item_spacing.x = 4.0;
    let open = open_file(app);
    let has_workspace = app.workspace.is_some();

    if has_workspace
        && icons::button(ui, Icon::Sidebar, "Show or hide the sidebar · Ctrl+B", false, sidebar_fits).clicked()
    {
        app.sidebar_open = !app.sidebar_open;
    }
    if has_workspace || open.is_some() {
        let can_back = app.state.history_index > 0;
        let can_forward = app.state.history_index + 1 < app.state.history.len();
        if icons::button(ui, Icon::Back, "Back · Alt+Left", false, can_back).clicked() {
            app.history_step(-1);
        }
        if icons::button(ui, Icon::Forward, "Forward · Alt+Right", false, can_forward).clicked() {
            app.history_step(1);
        }
    }
    // With the sidebar out of sight, the project name leads the breadcrumb and switches projects.
    let sidebar_visible = app.sidebar_open && sidebar_fits;
    let mut open = open;
    if has_workspace && !sidebar_visible {
        ui.add_space(token::SPACE[2] - 4.0);
        project_crumb(app, ui, p);
        if let Some(open) = open.as_mut() {
            ui.label(egui::RichText::new("/").font(FontId::proportional(token::FONT_SMALL)).color(p.muted));
            // A file at the project root would repeat the project name as its folder.
            if app.workspace.as_ref().is_some_and(|workspace| workspace.name == open.crumb.folder) {
                open.crumb.folder.clear();
            }
        }
    }
    let Some(open) = open else { return };
    ui.add_space(token::SPACE[2] - 4.0);
    let budget = ui.available_width() - RIGHT_CLUSTER - RIGHT_GAP;
    breadcrumb(app, ui, p, &open, budget);
}

/// The project name as a quiet button that opens the project switcher.
fn project_crumb(app: &mut TidemapApp, ui: &mut egui::Ui, p: Palette) {
    let name = app.workspace.as_ref().map(|workspace| workspace.name.clone()).unwrap_or_default();
    let font = FontId::new(token::FONT_SMALL, theme::font_medium());
    let shown = fit_text(&|text: &str| ui.painter().layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE).size().x, &name, 180.0, false);
    let galley = ui.painter().layout_no_wrap(shown, font, p.secondary);
    let width = galley.size().x + 2.0 * BLOCK_PAD + 4.0 + CHEVRON;
    let (rect, response) = ui.allocate_exact_size(vec2(width, BLOCK_HEIGHT), Sense::click());
    let hover = motion::hover(ui.ctx(), response.id, response.hovered());
    if hover > 0.0 {
        ui.painter().rect_filled(rect, token::RADIUS_CONTROL, p.hover.gamma_multiply(hover));
    }
    let color = p.secondary.lerp_to_gamma(p.ink, hover);
    ui.painter().galley(egui::pos2(rect.left() + BLOCK_PAD, rect.center().y - galley.size().y * 0.5), galley, color);
    let chevrons = egui::Rect::from_center_size(
        egui::pos2(rect.right() - BLOCK_PAD - CHEVRON * 0.5, rect.center().y),
        vec2(CHEVRON, CHEVRON),
    );
    icons::paint(ui.painter(), Icon::ChevronsUpDown, chevrons, p.muted.lerp_to_gamma(p.secondary, hover));
    let response = response.on_hover_text("Switch project · Ctrl+R").on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        app.toggle_projects();
    }
}

fn breadcrumb(app: &mut TidemapApp, ui: &mut egui::Ui, p: Palette, open: &Open, budget: f32) {
    let small = FontId::proportional(token::FONT_SMALL);
    let name_font = FontId::new(token::FONT_SMALL, theme::font_medium());
    let painter = ui.painter().clone();
    let width_of = |font: &FontId| {
        let font = font.clone();
        let painter = painter.clone();
        move |text: &str| painter.layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE).size().x
    };
    let small_width = width_of(&small);
    let name_width = width_of(&name_font);
    let spacing = ui.spacing().item_spacing.x;

    let crumb = &open.crumb;
    let block_label = crumb.block.as_ref().zip(crumb.position).map(|(title, (index, count))| {
        format!("{title}  {index} of {count}")
    });
    let separator = "/";
    let dot_width = if open.dirty { DOT + spacing } else { 0.0 };
    let block_width = block_label
        .as_ref()
        .map(|label| small_width(label) + 2.0 * BLOCK_PAD + spacing + CHEVRON + spacing + small_width("·"))
        .unwrap_or(0.0);

    // Everything but the folder has priority; the folder gives way first, from the left.
    let separator_width = small_width(separator) + 2.0 * spacing;
    let fixed = name_width(&crumb.file) + dot_width + block_width + spacing;
    let folder_budget = budget - fixed - separator_width;
    let show_folder = !crumb.folder.is_empty() && folder_budget > small_width("\u{2026}") + 24.0;
    let file_budget = budget - dot_width - block_width - spacing;

    if show_folder {
        let folder = fit_text(&small_width, &crumb.folder, folder_budget, true);
        ui.label(egui::RichText::new(folder).font(small.clone()).color(p.secondary));
        ui.label(egui::RichText::new(separator).font(small.clone()).color(p.muted));
    }
    let file = fit_text(&name_width, &crumb.file, file_budget, false);
    ui.label(egui::RichText::new(file).font(name_font).color(p.ink));
    if open.dirty {
        let (rect, _) = ui.allocate_exact_size(vec2(DOT, token::CONTROL), Sense::hover());
        // Pops in the first time the file turns dirty.
        let pop = motion::spring_from(ui.ctx(), Id::new("crumb-dirty-dot"), 0.0, 1.0, motion::POP);
        ui.painter().circle_filled(rect.center(), DOT * 0.5 * pop.max(0.0), p.accent);
    }
    if let Some(label) = block_label {
        ui.label(egui::RichText::new("·").font(small.clone()).color(p.muted));
        block_button(app, ui, p, open, &label, &small);
    }
}

/// The quiet button that names the active diagram and opens the list of the file's diagrams.
fn block_button(
    app: &mut TidemapApp,
    ui: &mut egui::Ui,
    p: Palette,
    open: &Open,
    label: &str,
    font: &FontId,
) {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), p.secondary);
    let width = galley.size().x + 2.0 * BLOCK_PAD + ui.spacing().item_spacing.x + CHEVRON;
    let (rect, response) = ui.allocate_exact_size(vec2(width, BLOCK_HEIGHT), Sense::click());
    let popup_open = egui::Popup::is_id_open(ui.ctx(), response.id.with("popup"));
    let hover = motion::hover(ui.ctx(), response.id, response.hovered() || popup_open);
    if hover > 0.0 {
        ui.painter().rect_filled(rect, token::RADIUS_CONTROL, p.hover.gamma_multiply(hover));
    }
    let text_pos = egui::pos2(rect.left() + BLOCK_PAD, rect.center().y - galley.size().y * 0.5);
    ui.painter().galley(text_pos, galley, p.secondary);
    let chevron = egui::Rect::from_center_size(
        egui::pos2(rect.right() - BLOCK_PAD - CHEVRON * 0.5, rect.center().y),
        vec2(CHEVRON, CHEVRON),
    );
    icons::paint(ui.painter(), Icon::ChevronDown, chevron, p.secondary);
    let response = response
        .on_hover_text("Diagrams in this file · [ and ] step through them")
        .on_hover_cursor(egui::CursorIcon::PointingHand);

    let mut chosen = None;
    egui::Popup::menu(&response).id(response.id.with("popup")).show(|ui| {
        ui.set_min_width(240.0);
        ui.set_max_width(360.0);
        for (id, title) in &open.blocks {
            let active = *id == app.active_block;
            let text = egui::RichText::new(title).size(token::FONT_SMALL);
            if ui.selectable_label(active, text).clicked() {
                chosen = Some(id.clone());
                ui.close();
            }
        }
    });
    if let Some(id) = chosen {
        app.select_block(id);
    }
}

fn right_side(app: &mut TidemapApp, ui: &mut egui::Ui, p: Palette) {
    ui.spacing_mut().item_spacing.x = 4.0;
    if icons::button(ui, Icon::Settings, "Settings · Ctrl+,", false, true).clicked() {
        app.dialog = Some(crate::app::Dialog::Settings);
    }
    let (icon, tooltip) = if app.dark {
        (Icon::Sun, "Switch to the light appearance · Ctrl+Shift+D")
    } else {
        (Icon::Moon, "Switch to the dark appearance · Ctrl+Shift+D")
    };
    if icons::button(ui, icon, tooltip, false, true).clicked() {
        app.settings.theme = if app.dark { Theme::Light } else { Theme::Dark };
        app.save_settings();
    }
    let markdown = app.buffers.get(&app.active_file).is_some_and(|buffer| !buffer.document.name.to_lowercase().ends_with(".mmd"));
    if markdown {
        let page = app.document_view();
        let (icon, tooltip) = if page {
            (Icon::Diagram, "Show the diagram · Ctrl+Shift+M")
        } else {
            (Icon::File, "Show the page · Ctrl+Shift+M")
        };
        if icons::button(ui, icon, tooltip, false, true).clicked() {
            app.toggle_document_view();
        }
    }
    let info = icons::button(ui, Icon::Info, "Status and diagram notes", app.info_open, true);
    app.info_anchor = info.rect;
    info::paint_dot(ui.painter(), info.rect, info::status(app), p);
    if info.clicked() {
        app.info_open = !app.info_open;
    }
}
