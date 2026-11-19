//! The slim top bar: navigation on the left, the breadcrumb, quiet actions on the right.

use egui::{Align, Color32, Context, FontId, Id, Layout, Sense, UiBuilder, vec2};
use tidemap_model::Theme;

use super::{info, surface};
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

const MENU_OPEN: &str = "block-menu-open";
const MENU_INDEX: &str = "block-menu-index";
const MENU_BAND: &str = "block-menu-band";
const MENU_PRESENCE: &str = "block-menu-presence";
/// Height of a row in the diagram menu.
const MENU_ROW: f32 = 32.0;
/// Padding at the sides of a menu row.
const MENU_PAD: f32 = 12.0;
/// Narrowest and widest the diagram menu gets.
const MENU_WIDTH: std::ops::RangeInclusive<f32> = 240.0..=360.0;
/// Tallest the diagram menu gets before it scrolls.
const MENU_MAX_HEIGHT: f32 = 320.0;

/// Ctx-data keys of the menu's self-healing state.
const MENU_FILE: &str = "block-menu-file";
const MENU_DRAWN: &str = "block-menu-drawn";

/// Whether a menu flagged `open` is really up: its button drew on this or the previous pass
/// (so it was not removed by a view change) and it was opened for the file that is active now.
fn menu_visible(open: bool, last_drawn: Option<u64>, now: u64, opened_for: &str, active: &str) -> bool {
    open && last_drawn.is_some_and(|drawn| drawn + 1 >= now) && opened_for == active
}

/// Whether the diagram menu is open and visible for the file `active`.
pub fn block_menu_open(ctx: &Context, active: &str) -> bool {
    let now = ctx.cumulative_pass_nr();
    let (open, drawn, file) = ctx.data(|data| {
        (
            data.get_temp::<bool>(Id::new(MENU_OPEN)).unwrap_or(false),
            data.get_temp::<u64>(Id::new(MENU_DRAWN)),
            data.get_temp::<String>(Id::new(MENU_FILE)).unwrap_or_default(),
        )
    });
    menu_visible(open, drawn, now, &file, active)
}

fn set_block_menu_open(ctx: &Context, open: bool) {
    ctx.data_mut(|data| data.insert_temp(Id::new(MENU_OPEN), open));
}

/// Closes the diagram menu if it is visible. Returns whether it was (so Esc can stop there);
/// a stale flag is cleared but reported as not open, leaving Esc to whatever is really up.
pub fn close_block_menu(ctx: &Context, active: &str) -> bool {
    let was = block_menu_open(ctx, active);
    set_block_menu_open(ctx, false);
    was
}

/// The quiet button that names the active diagram and opens the list of the file's diagrams.
///
/// The menu is ours rather than egui's popup so it can grow from the button and be driven from
/// the keyboard: Up and Down move, Enter picks, Esc (see `palette::escape`) or a click outside
/// closes. Its keys are taken only while it is open.
fn block_button(
    app: &mut TidemapApp,
    ui: &mut egui::Ui,
    p: Palette,
    open: &Open,
    label: &str,
    font: &FontId,
) {
    let ctx = ui.ctx().clone();
    let count = open.blocks.len();
    let active = open.blocks.iter().position(|(id, _)| *id == app.active_block).unwrap_or(0);
    // A modal over the window takes the menu with it.
    if app.palette_open || app.dialog.is_some() {
        set_block_menu_open(&ctx, false);
    }
    let was_open = block_menu_open(&ctx, &app.active_file);
    if !was_open {
        // Stale (another file, or the button was away): forget it.
        set_block_menu_open(&ctx, false);
    }
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| {
        data.insert_temp(Id::new(MENU_DRAWN), pass);
        if !was_open {
            data.insert_temp(Id::new(MENU_FILE), app.active_file.clone());
        }
    });
    let mut index = ctx
        .data(|data| data.get_temp::<usize>(Id::new(MENU_INDEX)))
        .unwrap_or(active)
        .min(count.saturating_sub(1));

    // Keys come first: taking Enter here keeps the focused button from also reading it.
    let mut chosen = None;
    let mut moved = false;
    if was_open && count > 0 {
        let (down, up, enter) = ctx.input_mut(|input| {
            (
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            )
        });
        if down {
            index = (index + 1) % count;
        }
        if up {
            index = (index + count - 1) % count;
        }
        moved = down || up;
        if enter {
            chosen = open.blocks.get(index).map(|(id, _)| id.clone());
        }
    }

    let galley = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), p.secondary);
    let width = galley.size().x + 2.0 * BLOCK_PAD + ui.spacing().item_spacing.x + CHEVRON;
    let (rect, response) = ui.allocate_exact_size(vec2(width, BLOCK_HEIGHT), Sense::click());
    let mut is_open = was_open && chosen.is_none();
    if response.clicked() {
        is_open = !is_open;
        if is_open {
            // Every opening starts on the active diagram, with the band already there.
            index = active;
            ctx.data_mut(|data| data.insert_temp(Id::new(MENU_FILE), app.active_file.clone()));
            motion::restart(&ctx, Id::new(MENU_BAND));
        }
    }
    // The button keeps its hovered look for as long as its menu is open.
    let hover = motion::hover(&ctx, response.id, response.hovered() || is_open);
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
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if !is_open {
        response.on_hover_text("Diagrams in this file \u{b7} [ and ] step through them");
    }

    // The menu stays drawn while it leaves, but only an open one takes input.
    let mut outside = false;
    if let Some(t) = motion::presence(&ctx, Id::new(MENU_PRESENCE), is_open && count > 0) {
        let menu = MenuState { t, interactive: is_open, anchor: rect, moved };
        let (picked, area) = block_menu(&ctx, p, open, &app.active_block, &mut index, menu);
        chosen = chosen.or(picked);
        outside = is_open
            && ctx.input(|input| {
                input.pointer.any_click()
                    && input.pointer.interact_pos().is_some_and(|pos| !area.contains(pos) && !rect.contains(pos))
            });
    } else {
        // Gone: drop the layer transform so the next opening starts clean.
        let layer = egui::LayerId::new(egui::Order::Foreground, Id::new("block-menu"));
        surface::animate_layer(&ctx, layer, 1.0, rect.left_bottom(), surface::POPOVER);
    }
    if chosen.is_some() || outside {
        is_open = false;
    }
    ctx.data_mut(|data| data.insert_temp(Id::new(MENU_INDEX), index));
    set_block_menu_open(&ctx, is_open);
    if let Some(id) = chosen {
        app.select_block(id);
    }
}

/// What the diagram menu needs to know about this frame.
struct MenuState {
    /// Presence of the menu (0 to 1).
    t: f32,
    /// False while the menu leaves: it draws but takes no input.
    interactive: bool,
    /// The button the menu hangs from.
    anchor: egui::Rect,
    /// The keyboard moved the selection this frame (scroll it into view).
    moved: bool,
}

/// Draws the diagram menu under its button: a raised surface of 32 px rows with a band on the
/// selected one. It unfolds from the button's bottom-left corner. Returns the diagram that was
/// clicked and the menu's rectangle.
fn block_menu(
    ctx: &Context,
    p: Palette,
    open: &Open,
    active_block: &str,
    index: &mut usize,
    menu: MenuState,
) -> (Option<String>, egui::Rect) {
    let id = Id::new("block-menu");
    let layer = egui::LayerId::new(egui::Order::Foreground, id);
    let shown_t = if menu.interactive { menu.t } else { surface::exit_t(menu.t) };
    surface::animate_layer(ctx, layer, shown_t, menu.anchor.left_bottom(), surface::POPOVER);
    if !menu.interactive {
        surface::release_focus(ctx, layer);
    }
    let inset = token::SPACE[1];
    let font = FontId::proportional(token::FONT_SMALL);
    let widest = open
        .blocks
        .iter()
        .map(|(_, title)| ctx.fonts_mut(|fonts| fonts.layout_no_wrap(title.clone(), font.clone(), Color32::WHITE).size().x))
        .fold(0.0_f32, f32::max);
    // Room for the padding on both sides, the check mark and the inset of the surface.
    let width = (widest + 2.0 * MENU_PAD + 24.0 + 2.0 * inset).clamp(*MENU_WIDTH.start(), *MENU_WIDTH.end());
    let band_at = motion::spring(ctx, Id::new(MENU_BAND), *index as f32, motion::SOFT);
    let mut picked = None;
    let area = egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fade_in(false)
        .fixed_pos(menu.anchor.left_bottom() + vec2(0.0, token::SPACE[0]))
        .interactable(menu.interactive)
        .show(ctx, |ui| {
            ui.multiply_opacity(motion::fade(menu.t));
            theme::elevated(ui, p, theme::Level::Overlay, token::RADIUS_PANEL, egui::Margin::same(inset as i8), |ui| {
                ui.set_width(width - 2.0 * inset);
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                egui::ScrollArea::vertical().max_height(MENU_MAX_HEIGHT).auto_shrink([false, true]).show(ui, |ui| {
                    let top = ui.cursor().top() + band_at * MENU_ROW;
                    let band = egui::Rect::from_min_size(
                        egui::pos2(ui.max_rect().left(), top),
                        vec2(ui.available_width(), MENU_ROW),
                    );
                    ui.painter().rect_filled(band, token::RADIUS_CONTROL, p.accent_soft);
                    for (row, (block, title)) in open.blocks.iter().enumerate() {
                        let current = block == active_block;
                        let response = menu_row(ui, p, title, current, row == *index, &font);
                        if menu.interactive && response.hovered() && ui.input(|input| input.pointer.is_moving()) {
                            *index = row;
                        }
                        if menu.moved && row == *index {
                            response.scroll_to_me(None);
                        }
                        if menu.interactive && response.clicked() {
                            picked = Some(block.clone());
                        }
                    }
                });
            });
        });
    (picked, area.response.rect)
}

/// One row of the diagram menu. The active diagram is set in the accent colour and carries a
/// check mark.
fn menu_row(ui: &mut egui::Ui, p: Palette, title: &str, current: bool, selected: bool, font: &FontId) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), MENU_ROW), Sense::click());
    let mark = egui::Rect::from_center_size(egui::pos2(rect.right() - MENU_PAD - 8.0, rect.center().y), vec2(16.0, 16.0));
    let budget = (mark.left() - MENU_PAD - rect.left() - 4.0).max(0.0);
    let color = if current {
        p.accent
    } else if selected {
        p.ink
    } else {
        p.secondary
    };
    let galley = super::palette::elided(ui, title, font.clone(), color, budget, false);
    ui.painter().galley(egui::pos2(rect.left() + MENU_PAD, rect.center().y - galley.size().y * 0.5), galley, color);
    if current {
        icons::paint(ui.painter(), Icon::Check, mark, p.accent);
    }
    response
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

#[cfg(test)]
mod menu_tests {
    use super::menu_visible;

    #[test]
    fn a_menu_is_visible_only_while_its_button_draws_for_the_same_file() {
        assert!(menu_visible(true, Some(10), 11, "a.md", "a.md"));
        assert!(menu_visible(true, Some(11), 11, "a.md", "a.md"));
        // The button was not drawn last pass (single diagram, other view, closed file).
        assert!(!menu_visible(true, Some(8), 11, "a.md", "a.md"));
        assert!(!menu_visible(true, None, 11, "a.md", "a.md"));
        // Another file is active now.
        assert!(!menu_visible(true, Some(11), 11, "a.md", "b.md"));
        assert!(!menu_visible(false, Some(11), 11, "a.md", "a.md"));
    }
}
