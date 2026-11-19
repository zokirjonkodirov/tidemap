//! The sidebar card: project switcher, search field, pinned files, the file tree and a quiet
//! footer.
//!
//! The tree is fully keyboard driven. From the search field, Up and Down walk the matching
//! files and Enter opens one (the keyboard then stays in the tree). In the tree, Up and Down
//! move, Left and Right fold and unfold (or go to the parent), Enter or Space opens, and
//! Home, End, Page Up and Page Down jump. `/` goes to the search field.

use std::{collections::BTreeSet, sync::Arc};

use egui::{Context, EventFilter, FontId, Id, Key, Modifiers, Rect, Sense, Ui, UiBuilder, text::LayoutJob, vec2};
use tidemap_model::FileEntry;

use super::controls;
use super::tree::{self, TreeCache, TreeRow};
use crate::{
    app::TidemapApp,
    icons::{self, Icon},
    motion,
    theme::{self, Palette, token},
};

const HEADER_HEIGHT: f32 = 36.0;
const SEARCH_HEIGHT: f32 = 32.0;
const FOOTER_HEIGHT: f32 = 36.0;
const ICON_SIZE: f32 = 16.0;
/// Space between a row's edge and its content.
const ROW_PAD: f32 = 8.0;
/// Gap between a row's icon and its name.
const ICON_GAP: f32 = 6.0;
const DOT: f32 = 6.0;
/// How long a file that appeared on disk keeps its glow.
const GLOW_SECONDS: f32 = 2.4;
/// How long a folder takes to fold or unfold.
const FOLD_SECONDS: f32 = 0.2;
/// Above this many rows a fold is not animated (every row would be laid out each frame).
const FOLD_ANIMATION_LIMIT: usize = 2500;
/// Height of a tree row. Tighter than a control: a tree is read, not aimed at.
const TREE_ROW: f32 = 28.0;
/// Size of the folder chevron.
const CHEVRON: f32 = 14.0;

const SEARCH_ID: &str = "sidebar-search";
const TREE_ID: &str = "sidebar-tree";
const SEARCH_FOCUSED_LAST: &str = "sidebar-search-focused-last";
const FOLD_ANIM: &str = "sidebar-fold-anim";
const LAST_QUERY: &str = "sidebar-last-query";
const ACTIVE_BAND: &str = "sidebar-active-band";

/// Where a shortcut asks the keyboard to go in the sidebar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FocusTarget {
    Search,
    Tree,
}

/// What a click, key or menu choice in the tree asks for; applied after drawing.
#[derive(Default)]
struct Actions {
    open: Option<String>,
    toggle: Option<String>,
    pin: Option<String>,
    unpin: Option<String>,
    reveal: Option<String>,
    /// The tree takes the keyboard (a row was clicked, or a file opened from the search).
    focus_tree: bool,
    /// The keyboard moved the cursor: bring its row into view.
    scroll_to_cursor: bool,
    /// A click put the keyboard cursor on this row.
    cursor: Option<String>,
    /// The query changed: the results start at the top.
    reset_scroll: bool,
}

/// One frame of keyboard input for the tree.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Nav {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Left,
    Right,
    Activate,
}

/// A folder that is folding or unfolding, and since when.
#[derive(Clone)]
struct Fold {
    path: String,
    opening: bool,
    start: f64,
}

pub fn show(app: &mut TidemapApp, ui: &mut Ui, p: Palette) {
    let ctx = ui.ctx().clone();
    let rows = cached_rows(app);
    let mut actions = Actions::default();

    match app.focus_request.take() {
        Some(FocusTarget::Search) => ctx.memory_mut(|memory| memory.request_focus(Id::new(SEARCH_ID))),
        Some(FocusTarget::Tree) => {
            ctx.memory_mut(|memory| memory.request_focus(Id::new(TREE_ID)));
            if cursor_index(app, &rows).is_none() {
                app.tree_cursor = active_row(app, &rows).or(Some(0)).and_then(|index| rows.get(index)).map(|row| row.path.clone());
            }
            actions.scroll_to_cursor = true;
        }
        None => {}
    }
    let search_focused = ctx.memory(|memory| memory.has_focus(Id::new(SEARCH_ID)));
    let tree_focused = ctx.memory(|memory| memory.has_focus(Id::new(TREE_ID)));
    keyboard(app, &ctx, &rows, search_focused, tree_focused, &mut actions);

    let area = ui.available_rect_before_wrap();
    header(app, ui, p);
    ui.add_space(token::SPACE[1]);
    search_field(app, ui, &ctx, p);
    ui.add_space(token::SPACE[2]);
    // The search field and Esc may have changed the query this frame: show the matching tree now,
    // not one frame late, and start the results from a clean fold state at the top.
    let last_query: Option<String> = ctx.data(|data| data.get_temp(Id::new(LAST_QUERY)));
    let query_changed = last_query.as_deref().is_some_and(|last| last != app.tree_query);
    if query_changed {
        app.search_collapsed.clear();
        ctx.data_mut(|data| data.remove::<Fold>(Id::new(FOLD_ANIM)));
    }
    ctx.data_mut(|data| data.insert_temp(Id::new(LAST_QUERY), app.tree_query.clone()));
    let rows = cached_rows(app);
    actions.reset_scroll = query_changed;

    let footer = Rect::from_min_max(egui::pos2(area.left(), area.bottom() - FOOTER_HEIGHT), area.max);
    let list_height = (footer.top() - ui.cursor().top() - token::SPACE[1]).max(token::ROW_HEIGHT);
    ui.allocate_ui(vec2(area.width(), list_height), |ui| {
        ui.set_min_height(list_height);
        list(app, ui, p, &rows, &mut actions);
    });
    ui.scope_builder(UiBuilder::new().max_rect(footer), |ui| footer_row(app, ui, p));

    apply(app, &ctx, actions);
    // Only the search field holds back the app's Esc cascade (its first Esc clears the query).
    app.sidebar_focused = ctx.memory(|memory| memory.has_focus(Id::new(SEARCH_ID)));
}

/// Rebuilds the rows only when the workspace, the filter or the folding changed.
/// The folded folders that apply now: the search results fold on their own.
fn folded(app: &TidemapApp) -> &BTreeSet<String> {
    if app.tree_query.trim().is_empty() { &app.collapsed } else { &app.search_collapsed }
}

fn cached_rows(app: &mut TidemapApp) -> Arc<Vec<TreeRow>> {
    let fresh = app.tree_cache.as_ref().is_some_and(|cache| {
        cache.revision == app.workspace_revision && cache.query == app.tree_query && cache.collapsed == *folded(app)
    });
    if fresh {
        if let Some(cache) = &app.tree_cache {
            return Arc::clone(&cache.rows);
        }
    }
    let files: &[FileEntry] = app
        .workspace
        .as_ref()
        .map(|workspace| workspace.files.as_slice())
        .unwrap_or(&[]);
    let rows = Arc::new(tree::build(files, &app.tree_query, folded(app)));
    app.tree_cache = Some(TreeCache {
        revision: app.workspace_revision,
        query: app.tree_query.clone(),
        collapsed: folded(app).clone(),
        rows: Arc::clone(&rows),
    });
    rows
}

fn cursor_index(app: &TidemapApp, rows: &[TreeRow]) -> Option<usize> {
    let cursor = app.tree_cursor.as_ref()?;
    rows.iter().position(|row| &row.path == cursor)
}

fn active_row(app: &TidemapApp, rows: &[TreeRow]) -> Option<usize> {
    rows.iter().position(|row| row.file_id.as_ref() == Some(&app.active_file))
}

// ---------------------------------------------------------------------------
// Keyboard
// ---------------------------------------------------------------------------

fn keyboard(
    app: &mut TidemapApp,
    ctx: &Context,
    rows: &[TreeRow],
    search_focused: bool,
    tree_focused: bool,
    actions: &mut Actions,
) {
    let searching = !app.tree_query.trim().is_empty();
    // egui drops a text field's focus on Esc before we see the key; remember last frame.
    let search_was_focused = ctx.data(|data| data.get_temp::<bool>(Id::new(SEARCH_FOCUSED_LAST)).unwrap_or(false));
    if search_was_focused && !search_focused && ctx.input(|input| input.key_pressed(Key::Escape)) && searching {
        // First Esc clears the query and keeps the field; the next one leaves it.
        app.tree_query.clear();
        app.tree_cursor = None;
        ctx.memory_mut(|memory| memory.request_focus(Id::new(SEARCH_ID)));
        ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape));
    }
    ctx.data_mut(|data| data.insert_temp(Id::new(SEARCH_FOCUSED_LAST), search_focused));

    if !search_focused && !tree_focused {
        return;
    }
    if tree_focused {
        // Keep arrows on the tree; egui would otherwise move focus to a neighbouring widget.
        ctx.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                Id::new(TREE_ID),
                EventFilter { tab: false, horizontal_arrows: true, vertical_arrows: true, escape: false },
            )
        });
    }
    let nav = ctx.input_mut(|input| {
        let mut take = |key: Key| input.consume_key(Modifiers::NONE, key);
        if take(Key::ArrowDown) {
            Some(Nav::Down)
        } else if take(Key::ArrowUp) {
            Some(Nav::Up)
        } else if take(Key::Enter) {
            Some(Nav::Activate)
        } else if !tree_focused {
            None
        } else if take(Key::ArrowLeft) {
            Some(Nav::Left)
        } else if take(Key::ArrowRight) {
            Some(Nav::Right)
        } else if take(Key::Space) {
            Some(Nav::Activate)
        } else if take(Key::Home) {
            Some(Nav::Home)
        } else if take(Key::End) {
            Some(Nav::End)
        } else if take(Key::PageDown) {
            Some(Nav::PageDown)
        } else if take(Key::PageUp) {
            Some(Nav::PageUp)
        } else {
            None
        }
    });
    // Letters are left alone in the tree: the bare-key shortcuts (F, 1, [, ], /) keep working
    // there, and `/` is the way into the search field.
    let Some(nav) = nav else { return };

    // From the search field only files are stops; in the tree every row is.
    let stops: Vec<usize> = if search_focused {
        rows.iter().enumerate().filter(|(_, row)| !row.directory).map(|(index, _)| index).collect()
    } else {
        (0..rows.len()).collect()
    };
    if search_focused && !searching && nav == Nav::Down {
        // Down from an empty search field steps into the tree.
        ctx.memory_mut(|memory| memory.request_focus(Id::new(TREE_ID)));
        if cursor_index(app, rows).is_none() {
            app.tree_cursor = active_row(app, rows).or(Some(0)).and_then(|index| rows.get(index)).map(|row| row.path.clone());
        }
        actions.scroll_to_cursor = true;
        return;
    }
    if stops.is_empty() {
        return;
    }
    let current = cursor_index(app, rows).and_then(|index| stops.iter().position(|stop| *stop == index));
    let page = 8;
    let last = stops.len() - 1;
    let target_stop = match (nav, current) {
        (Nav::Down, None) | (Nav::Home, _) => Some(0),
        (Nav::Up, None) | (Nav::End, _) => Some(last),
        (Nav::Down, Some(at)) => Some((at + 1).min(last)),
        (Nav::Up, Some(at)) => Some(at.saturating_sub(1)),
        (Nav::PageDown, at) => Some((at.unwrap_or(0) + page).min(last)),
        (Nav::PageUp, at) => Some(at.unwrap_or(0).saturating_sub(page)),
        _ => None,
    };
    if let Some(stop) = target_stop {
        app.tree_cursor = rows.get(stops[stop]).map(|row| row.path.clone());
        actions.scroll_to_cursor = true;
        return;
    }
    let Some(index) = cursor_index(app, rows).or_else(|| (nav == Nav::Activate).then(|| stops[0])) else {
        return;
    };
    let row = &rows[index];
    match nav {
        Nav::Activate => {
            if row.directory {
                actions.toggle = Some(row.path.clone());
            } else {
                actions.open = row.file_id.clone();
                app.tree_cursor = Some(row.path.clone());
                actions.focus_tree = true;
            }
        }
        Nav::Right if row.directory && !row.expanded => actions.toggle = Some(row.path.clone()),
        Nav::Right if row.directory => {
            if let Some(child) = rows.get(index + 1).filter(|child| child.depth > row.depth) {
                app.tree_cursor = Some(child.path.clone());
                actions.scroll_to_cursor = true;
            }
        }
        Nav::Left if row.directory && row.expanded => actions.toggle = Some(row.path.clone()),
        Nav::Left => {
            if let Some((parent, _)) = row.path.rsplit_once('/') {
                if rows.iter().any(|candidate| candidate.directory && candidate.path == parent) {
                    app.tree_cursor = Some(parent.to_owned());
                    actions.scroll_to_cursor = true;
                }
            }
        }
        _ => {}
    }
}

fn apply(app: &mut TidemapApp, ctx: &Context, actions: Actions) {
    if let Some(path) = actions.toggle {
        let searching = !app.tree_query.trim().is_empty();
        let set = if searching { &mut app.search_collapsed } else { &mut app.collapsed };
        let opening = set.remove(&path);
        if !opening {
            set.insert(path.clone());
        }
        // The rows slide and fade as the folder folds or unfolds.
        let start = ctx.input(|input| input.time);
        ctx.data_mut(|data| data.insert_temp(Id::new(FOLD_ANIM), Fold { path, opening, start }));
        ctx.request_repaint();
    }
    if let Some(cursor) = actions.cursor {
        app.tree_cursor = Some(cursor);
    }
    if actions.focus_tree {
        ctx.memory_mut(|memory| memory.request_focus(Id::new(TREE_ID)));
    }
    if let Some(id) = actions.open {
        app.open_document(id, None, true);
    }
    if let Some(id) = actions.pin {
        if !app.state.pins.contains(&id) {
            app.state.pins.push(id);
        }
        app.record_state();
    }
    if let Some(id) = actions.unpin {
        app.state.pins.retain(|pin| pin != &id);
        app.record_state();
    }
    if let Some(path) = actions.reveal {
        if let Some(workspace) = &app.workspace {
            reveal(&workspace.root.join(path));
        }
    }
}

/// Shows `path` selected in the system file manager.
fn reveal(path: &std::path::Path) {
    use std::process::Command;
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let native = path.to_string_lossy().replace('/', "\\");
        let _ = Command::new("explorer")
            .raw_arg(format!("/select,\"{native}\""))
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg("-R").arg(path).spawn();
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        if let Some(parent) = path.parent() {
            let _ = Command::new("xdg-open").arg(parent).spawn();
        }
    }
}

// ---------------------------------------------------------------------------
// Header: the project switcher
// ---------------------------------------------------------------------------

fn header(app: &mut TidemapApp, ui: &mut Ui, p: Palette) {
    let name = app.workspace.as_ref().map(|workspace| workspace.name.clone()).unwrap_or_default();
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_HEIGHT), Sense::click());
    let open = app.palette_open && app.palette_mode == super::palette::PaletteMode::Projects;
    let hover = motion::hover(ui.ctx(), response.id, response.hovered() || open);
    let painter = ui.painter();
    if hover > 0.0 {
        painter.rect_filled(rect, token::RADIUS_CONTROL, p.hover.gamma_multiply(hover));
    }
    let icon = Rect::from_center_size(egui::pos2(rect.left() + ROW_PAD + ICON_SIZE * 0.5, rect.center().y), vec2(ICON_SIZE, ICON_SIZE));
    icons::paint(painter, Icon::Stack, icon, p.secondary.lerp_to_gamma(p.ink, hover));
    let chevrons = Rect::from_center_size(egui::pos2(rect.right() - ROW_PAD - 7.0, rect.center().y), vec2(14.0, 14.0));
    icons::paint(painter, Icon::ChevronsUpDown, chevrons, p.muted.lerp_to_gamma(p.secondary, hover));
    let text_left = icon.right() + ROW_PAD;
    let mut job = LayoutJob::simple_singleline(name, FontId::new(token::FONT_UI, theme::font_semibold()), p.ink);
    job.wrap.max_width = (chevrons.left() - ROW_PAD - text_left).max(0.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('\u{2026}');
    let galley = painter.layout_job(job);
    painter.galley(egui::pos2(text_left, rect.center().y - galley.size().y * 0.5), galley, p.ink);
    let response = response
        .on_hover_text("Switch project · Ctrl+R")
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        app.toggle_projects();
    }
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

fn search_field(app: &mut TidemapApp, ui: &mut Ui, ctx: &Context, p: Palette) {
    let id = Id::new(SEARCH_ID);
    // The whole field is the target, not only the text: a click on the icon or the padding focuses it.
    let (rect, field) = ui.allocate_exact_size(vec2(ui.available_width(), SEARCH_HEIGHT), Sense::click());
    if field.clicked() {
        ctx.memory_mut(|memory| memory.request_focus(id));
    }
    let focused = ctx.memory(|memory| memory.has_focus(id));
    let focus = motion::toggle(ctx, id.with("focus-ring"), focused, motion::FAST);
    let hover = motion::hover(ctx, id, field.hovered() && !focused);
    ui.painter().rect_filled(rect, token::RADIUS_CONTROL, p.app);
    if hover > 0.0 {
        ui.painter().rect_filled(rect, token::RADIUS_CONTROL, p.hover.gamma_multiply(hover * 0.6));
    }
    let icon = Rect::from_center_size(
        egui::pos2(rect.left() + 10.0 + ICON_SIZE * 0.5, rect.center().y),
        vec2(ICON_SIZE, ICON_SIZE),
    );
    icons::paint(ui.painter(), Icon::Search, icon, p.muted.lerp_to_gamma(p.accent, focus));

    // The text area never changes size: focus, the key cap and the clear button come and go
    // inside the same reserved strip, so typing never makes the field jump.
    let strip = 30.0;
    let text_rect = Rect::from_min_max(
        egui::pos2(icon.right() + 8.0, rect.top()),
        egui::pos2(rect.right() - strip, rect.bottom()),
    );
    let before = app.tree_query.clone();
    let edit = egui::TextEdit::singleline(&mut app.tree_query)
        .id(id)
        .hint_text(egui::RichText::new("Search files").color(p.muted))
        .frame(false)
        .margin(egui::Margin::ZERO)
        .vertical_align(egui::Align::Center)
        .desired_width(text_rect.width());
    ui.put(text_rect, edit);
    if app.tree_query != before {
        // A new query starts the keyboard on its first match.
        app.tree_cursor = None;
    }

    let slot = Rect::from_center_size(egui::pos2(rect.right() - 17.0, rect.center().y), vec2(22.0, 22.0));
    let has_text = !app.tree_query.is_empty();
    // The shortcut as a quiet key cap while the field is idle and empty; a clear button once there is text.
    let cap = motion::toggle(ctx, id.with("cap"), !has_text && !focused, motion::BASE);
    if cap > 0.0 {
        ui.painter().rect_filled(slot, 6.0, p.field.gamma_multiply(cap));
        ui.painter().text(slot.center(), egui::Align2::CENTER_CENTER, "/", FontId::proportional(token::FONT_SMALL), p.muted.gamma_multiply(cap));
    }
    let clear = motion::toggle(ctx, id.with("clear"), has_text, motion::BASE);
    if clear > 0.0 {
        let response = ui.interact(slot, id.with("clear-hit"), if has_text { Sense::click() } else { Sense::hover() });
        let over = motion::hover(ctx, id.with("clear-hover"), response.hovered() && has_text);
        ui.painter().rect_filled(slot, 6.0, p.hover.gamma_multiply(over * clear));
        let glyph = Rect::from_center_size(slot.center(), vec2(12.0, 12.0));
        icons::paint(ui.painter(), Icon::Close, glyph, p.muted.lerp_to_gamma(p.ink, over).gamma_multiply(clear));
        if has_text && response.on_hover_text("Clear · Esc").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            app.tree_query.clear();
            app.tree_cursor = None;
            ctx.memory_mut(|memory| memory.request_focus(id));
        }
    }
    if focus > 0.0 {
        ui.painter().rect_stroke(
            rect,
            token::RADIUS_CONTROL,
            egui::Stroke::new(1.0_f32, p.accent.gamma_multiply(focus)),
            egui::StrokeKind::Inside,
        );
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

/// How a row is drawn this frame.
struct RowLook {
    /// The keyboard is on this row and the tree (or search) has focus.
    cursor: bool,
    /// Draw the active fill here (pinned rows; tree rows use the sliding band instead).
    active_fill: bool,
    /// How much of the row is shown: 1 normally, less while a folder folds or unfolds.
    reveal: f32,
}

/// The rows to draw and how far each is revealed. During a fold the folder's children are
/// part of the list, shrinking away (or growing in) with the same progress.
struct Display {
    rows: Arc<Vec<TreeRow>>,
    reveal: Vec<f32>,
    animating: bool,
}

fn display(app: &TidemapApp, ctx: &Context, rows: &Arc<Vec<TreeRow>>) -> Display {
    let plain = || Display { rows: Arc::clone(rows), reveal: Vec::new(), animating: false };
    let now = ctx.input(|input| input.time);
    let fold: Option<Fold> = ctx.data(|data| data.get_temp(Id::new(FOLD_ANIM)));
    let Some(fold) = fold.filter(|fold| ((now - fold.start) as f32) < FOLD_SECONDS) else {
        return plain();
    };
    let progress = motion::ease_out(((now - fold.start) as f32 / FOLD_SECONDS).clamp(0.0, 1.0));
    let share = if fold.opening { progress } else { 1.0 - progress };
    let prefix = format!("{}/", fold.path);
    let shown: Arc<Vec<TreeRow>> = if fold.opening {
        Arc::clone(rows)
    } else {
        // The folder is already folded in the data; list its children once more while they leave.
        let mut set = folded(app).clone();
        set.remove(&fold.path);
        let files: &[FileEntry] = app.workspace.as_ref().map(|workspace| workspace.files.as_slice()).unwrap_or(&[]);
        let mut full = tree::build(files, &app.tree_query, &set);
        if let Some(row) = full.iter_mut().find(|row| row.path == fold.path) {
            row.expanded = false;
        }
        Arc::new(full)
    };
    if shown.len() > FOLD_ANIMATION_LIMIT {
        return plain();
    }
    let reveal = shown.iter().map(|row| if row.path.starts_with(&prefix) { share } else { 1.0 }).collect();
    ctx.request_repaint();
    Display { rows: shown, reveal, animating: true }
}

fn list(app: &mut TidemapApp, ui: &mut Ui, p: Palette, rows: &Arc<Vec<TreeRow>>, actions: &mut Actions) {
    let indexing = app.workspace.as_ref().is_some_and(|workspace| workspace.indexing);
    let searching = !app.tree_query.trim().is_empty();

    if rows.is_empty() {
        empty_state(app, ui, p, indexing, searching);
        return;
    }
    let ctx = ui.ctx().clone();
    let tree_focused = ctx.memory(|memory| memory.has_focus(Id::new(TREE_ID)));
    let search_focused = ctx.memory(|memory| memory.has_focus(Id::new(SEARCH_ID)));
    // While searching, the first matching file is where Enter goes; show it.
    if searching && cursor_index(app, rows).is_none() {
        app.tree_cursor = rows.iter().find(|row| !row.directory).map(|row| row.path.clone());
    }
    let show_cursor = tree_focused || (search_focused && searching);
    let shown = display(app, &ctx, rows);
    let rows = &shown.rows;
    let cursor = cursor_index(app, rows);
    let active = active_row(app, rows);
    // Top of each row, in points below the first row. Uniform unless a fold is moving.
    let tops: Option<Vec<f32>> = shown.animating.then(|| {
        let mut tops = Vec::with_capacity(rows.len() + 1);
        let mut y = 0.0;
        for factor in &shown.reveal {
            tops.push(y);
            y += TREE_ROW * factor;
        }
        tops.push(y);
        tops
    });
    let top_of = |index: usize| -> f32 {
        match &tops {
            Some(tops) => tops[index.min(tops.len() - 1)],
            None => index as f32 * TREE_ROW,
        }
    };

    let pinned = pinned_files(app);
    let list_rect = ui.max_rect();
    // The tree's own focus target: rows are clickable, the tree as a whole takes the keyboard.
    ui.interact(list_rect, Id::new(TREE_ID), Sense::focusable_noninteractive());
    let mut area = egui::ScrollArea::vertical().id_salt("file-tree").auto_shrink([false, false]);
    if actions.reset_scroll {
        area = area.vertical_scroll_offset(0.0);
    }
    area.show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if !pinned.is_empty() && !searching {
            ui.label(egui::RichText::new("Pinned").size(token::FONT_SMALL).color(p.muted));
            ui.add_space(token::SPACE[0]);
            for file in &pinned {
                let row = TreeRow {
                    path: file.path.clone(),
                    name: file.name.clone(),
                    depth: 0,
                    directory: false,
                    expanded: false,
                    file_id: Some(file.id.clone()),
                    count: file.diagram_count,
                };
                let look = RowLook { cursor: false, active_fill: true, reveal: 1.0 };
                file_or_folder_row(app, ui, p, &row, &look, actions);
            }
            ui.add_space(token::SPACE[2]);
        }
        // Rows are cheap to lay out but a workspace can hold thousands: draw only the visible ones
        // (while a fold moves every row is drawn; FOLD_ANIMATION_LIMIT caps that).
        let base = ui.cursor().top();
        let (start, end) = if shown.animating {
            (0, rows.len())
        } else {
            let first_visible = ((ui.clip_rect().top() - base) / TREE_ROW).floor().max(0.0) as usize;
            let visible = (ui.clip_rect().height() / TREE_ROW).ceil() as usize + 2;
            if !actions.scroll_to_cursor {
                scroll_to_active(app, ui, rows, first_visible, visible);
            }
            let end = (first_visible + visible).min(rows.len());
            (first_visible.min(end), end)
        };
        if actions.scroll_to_cursor {
            if let Some(index) = cursor {
                let rect = Rect::from_min_size(
                    egui::pos2(ui.max_rect().left(), base + top_of(index)),
                    vec2(ui.available_width(), TREE_ROW),
                );
                ui.scroll_to_rect(rect, None);
            }
        }
        // The active row's fill glides when the open file changes or rows move; while a fold
        // moves it simply stays on its row.
        if let Some(index) = active {
            let duration = if shown.animating { 0.0 } else { motion::BASE };
            let at = motion::tween(&ctx, Id::new(ACTIVE_BAND), top_of(index), duration, motion::ease_out);
            let height = TREE_ROW * shown.reveal.get(index).copied().unwrap_or(1.0);
            let band = Rect::from_min_size(egui::pos2(ui.max_rect().left(), base + at), vec2(ui.available_width(), height));
            if ui.is_rect_visible(band) {
                ui.painter().rect_filled(band, token::RADIUS_CONTROL, p.accent_soft);
            }
        }
        ui.add_space(top_of(start));
        for index in start..end {
            let row = &rows[index];
            let reveal = shown.reveal.get(index).copied().unwrap_or(1.0);
            let look = RowLook { cursor: show_cursor && cursor == Some(index), active_fill: false, reveal };
            file_or_folder_row(app, ui, p, row, &look, actions);
        }
        ui.add_space((top_of(rows.len()) - top_of(end)).max(0.0));
    });
}

/// When a different file becomes active, brings its row into view if it is outside.
fn scroll_to_active(app: &TidemapApp, ui: &mut Ui, rows: &[TreeRow], first_visible: usize, visible: usize) {
    let marker = egui::Id::new("tree-scrolled-to");
    let last: Option<String> = ui.data(|data| data.get_temp(marker));
    if last.as_deref() == Some(app.active_file.as_str()) {
        return;
    }
    let Some(index) = rows
        .iter()
        .position(|row| row.file_id.as_ref() == Some(&app.active_file))
    else {
        return;
    };
    ui.data_mut(|data| data.insert_temp(marker, app.active_file.clone()));
    let comfortably_visible = first_visible + 1..first_visible + visible.saturating_sub(3);
    if comfortably_visible.contains(&index) {
        return;
    }
    let top = ui.cursor().top() + index as f32 * TREE_ROW;
    let rect = Rect::from_min_size(egui::pos2(ui.max_rect().left(), top), vec2(ui.available_width(), TREE_ROW));
    ui.scroll_to_rect(rect, Some(egui::Align::Center));
}

fn pinned_files(app: &TidemapApp) -> Vec<FileEntry> {
    let Some(workspace) = &app.workspace else {
        return Vec::new();
    };
    app.state
        .pins
        .iter()
        .filter_map(|id| workspace.files.iter().find(|file| &file.id == id))
        .cloned()
        .collect()
}

fn empty_state(app: &mut TidemapApp, ui: &mut Ui, p: Palette, indexing: bool, searching: bool) {
    ui.add_space(token::SPACE[1]);
    let message = if indexing {
        "Looking for diagrams\u{2026}".to_owned()
    } else if searching {
        format!("No files match \u{201c}{}\u{201d}.", app.tree_query.trim())
    } else {
        "No Markdown or Mermaid files in this folder.".to_owned()
    };
    ui.add(
        egui::Label::new(egui::RichText::new(message).size(token::FONT_SMALL).color(p.muted)).wrap(),
    );
    if !indexing && !searching {
        ui.add_space(token::SPACE[1]);
        if controls::quiet_button(ui, "Open another folder").clicked() {
            app.open_folder(None);
        }
    }
}

/// One tree row, painted by hand so that text is left-aligned and truncated exactly. While a
/// folder folds or unfolds, `look.reveal` shrinks the row's height and fades it; the content keeps
/// its place and is clipped, so rows slide away rather than squash.
fn file_or_folder_row(
    app: &TidemapApp,
    ui: &mut Ui,
    p: Palette,
    row: &TreeRow,
    look: &RowLook,
    actions: &mut Actions,
) {
    let moving = look.reveal < 0.999;
    let height = TREE_ROW * look.reveal.clamp(0.0, 1.0);
    let sense = if moving { Sense::hover() } else { Sense::click() };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), sense);
    let active = row.file_id.as_ref() == Some(&app.active_file);
    let dirty = row
        .file_id
        .as_ref()
        .and_then(|id| app.buffers.get(id))
        .is_some_and(|buffer| buffer.dirty());

    if height >= 0.5 && ui.is_rect_visible(rect) {
        let ctx = ui.ctx().clone();
        let row_id = Id::new(("tree-row", look.active_fill, &row.path));
        let hover = motion::hover(&ctx, row_id, response.hovered() && !active && !moving);
        let mut painter = ui.painter().clone();
        painter.set_clip_rect(painter.clip_rect().intersect(rect));
        if moving {
            painter.multiply_opacity(look.reveal.clamp(0.0, 1.0).powi(2));
        }
        // Content keeps its full height; the clip does the revealing.
        let rect = Rect::from_min_size(rect.min, vec2(rect.width(), TREE_ROW));
        if active && look.active_fill {
            painter.rect_filled(rect, token::RADIUS_CONTROL, p.accent_soft);
        }
        if hover > 0.0 {
            painter.rect_filled(rect, token::RADIUS_CONTROL, p.hover.gamma_multiply(hover));
        }
        // A file that just appeared on disk glows, then settles.
        if let Some(seen) = row.file_id.as_ref().and_then(|id| app.appeared.get(id)) {
            let age = seen.elapsed().as_secs_f32();
            if age < GLOW_SECONDS {
                let fade = 1.0 - motion::ease_out(age / GLOW_SECONDS);
                painter.rect_filled(rect, token::RADIUS_CONTROL, p.accent.gamma_multiply(0.18 * fade));
                ctx.request_repaint();
            }
        }
        if look.cursor {
            painter.rect_stroke(
                rect.shrink(0.5),
                token::RADIUS_CONTROL,
                egui::Stroke::new(1.0_f32, p.accent.gamma_multiply(0.8)),
                egui::StrokeKind::Inside,
            );
        }

        // Indent guides: a faint line under each ancestor folder's chevron, so depth reads at a glance.
        if !look.active_fill {
            for level in 1..=row.depth.min(6) {
                let x = rect.left() + ROW_PAD + tree::indent(level - 1) + CHEVRON * 0.5;
                painter.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    egui::Stroke::new(1.0_f32, p.hairline.gamma_multiply(0.7)),
                );
            }
        }

        let ink = if active { p.accent } else { p.ink };
        let left = rect.left() + ROW_PAD + tree::indent(row.depth);
        let icon_rect = Rect::from_center_size(
            egui::pos2(left + ICON_SIZE * 0.5, rect.center().y),
            vec2(ICON_SIZE, ICON_SIZE),
        );
        let icon_color = if active { p.accent } else { p.secondary.lerp_to_gamma(p.ink, hover) };
        let lead = if row.directory {
            // The chevron turns as the folder folds and unfolds. It is a little lighter and smaller
            // than a file icon: the name is what matters.
            let turn = motion::toggle(&ctx, row_id.with("chevron"), row.expanded, motion::BASE);
            let chevron = Rect::from_center_size(egui::pos2(left + CHEVRON * 0.5, rect.center().y), vec2(CHEVRON, CHEVRON));
            icons::paint_rotated(
                &painter,
                Icon::ChevronRight,
                chevron,
                p.muted.lerp_to_gamma(p.ink, hover.max(turn * 0.6)),
                turn * std::f32::consts::FRAC_PI_2,
            );
            chevron.right()
        } else {
            let icon = if row.name.to_lowercase().ends_with(".mmd") { Icon::Diagram } else { Icon::File };
            icons::paint(&painter, icon, icon_rect, icon_color);
            icon_rect.right()
        };

        // Trailing items, from the right edge inwards.
        let mut right = rect.right() - ROW_PAD;
        if dirty {
            painter.circle_filled(egui::pos2(right - DOT * 0.5, rect.center().y), DOT * 0.5, p.accent);
            right -= DOT + ROW_PAD;
        }
        if let Some(count) = row.count.filter(|count| *count > 1) {
            let galley = painter.layout_no_wrap(
                count.to_string(),
                FontId::proportional(token::FONT_SMALL),
                p.muted,
            );
            let pos = egui::pos2(right - galley.size().x, rect.center().y - galley.size().y * 0.5);
            right = pos.x - ROW_PAD;
            painter.galley(pos, galley, p.muted);
        }

        let text_left = lead + ICON_GAP;
        let family = if active || row.directory { theme::font_medium() } else { egui::FontFamily::Proportional };
        let mut job = LayoutJob::simple_singleline(
            row.name.clone(),
            FontId::new(token::FONT_UI, family),
            ink,
        );
        job.wrap.max_width = (right - text_left).max(0.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('\u{2026}');
        let galley = painter.layout_job(job);
        let truncated = galley.elided;
        let pos = egui::pos2(text_left, rect.center().y - galley.size().y * 0.5);
        painter.galley(pos, galley, ink);

        if truncated && response.hovered() && !moving {
            response.clone().on_hover_text(&row.path);
        }
    }

    if moving {
        return;
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked() {
        actions.focus_tree = true;
        if row.directory {
            actions.toggle = Some(row.path.clone());
        } else {
            actions.open = row.file_id.clone();
        }
        // The keyboard continues from the clicked row (pinned rows are outside the tree's order).
        if !look.active_fill {
            actions.cursor = Some(row.path.clone());
        }
    }
    if let Some(id) = &row.file_id {
        let pinned = app.state.pins.contains(id);
        response.context_menu(|ui| {
            let label = if pinned { "Unpin" } else { "Pin" };
            if ui.button(label).clicked() {
                if pinned {
                    actions.unpin = Some(id.clone());
                } else {
                    actions.pin = Some(id.clone());
                }
                ui.close();
            }
            if ui.button("Reveal in file manager").clicked() {
                actions.reveal = Some(row.path.clone());
                ui.close();
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Footer
// ---------------------------------------------------------------------------

/// Three separate actions, each with its own glyph: a new diagram, an existing file, a folder.
fn footer_row(app: &mut TidemapApp, ui: &mut Ui, p: Palette) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if icons::button(ui, Icon::FolderOpen, "Open folder · Ctrl+Shift+O", false, true).clicked() {
            app.open_folder(None);
        }
        if icons::button(ui, Icon::File, "Open file · Ctrl+O", false, true).clicked() {
            app.open_external(None);
        }
        if icons::button(ui, Icon::FilePlus, "New diagram · Ctrl+N", false, app.workspace.is_some()).clicked() {
            app.new_diagram();
        }
        let (text, busy) = match &app.workspace {
            Some(workspace) if workspace.indexing && workspace.files.is_empty() => {
                (format!("Indexing\u{2026} {}", workspace.scanned), true)
            }
            Some(workspace) if workspace.files.len() == 1 => ("1 file".to_owned(), workspace.indexing),
            Some(workspace) => (format!("{} files", workspace.files.len()), workspace.indexing),
            None => (String::new(), false),
        };
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(text).size(token::FONT_SMALL).color(p.muted));
            // A re-index after a change on disk: a soft breathing dot, not a word that flickers.
            let shown = motion::toggle(ui.ctx(), Id::new("sidebar-busy"), busy, motion::BASE);
            if shown > 0.0 {
                let time = ui.input(|input| input.time) as f32;
                let breathe = 0.45 + 0.35 * (time * 5.0).sin();
                let (rect, _) = ui.allocate_exact_size(vec2(DOT, DOT), Sense::hover());
                ui.painter().circle_filled(rect.center(), DOT * 0.5, p.accent.gamma_multiply(breathe * shown));
                ui.ctx().request_repaint();
            }
        });
    });
}

impl TidemapApp {
    /// Creates `untitled.mmd` (or the next free name) in the project and opens it with the source
    /// pane showing, as soon as the index has picked it up.
    pub(crate) fn new_diagram(&mut self) {
        let Some(workspace) = &self.workspace else {
            self.notify("Open a folder first");
            return;
        };
        let root = workspace.root.clone();
        let mut name = "untitled.mmd".to_owned();
        let mut n = 2;
        while root.join(&name).exists() {
            name = format!("untitled-{n}.mmd");
            n += 1;
        }
        match std::fs::write(root.join(&name), "flowchart TD\n    A[Start] --> B[Next]\n") {
            Ok(()) => self.pending_new = Some((name, std::time::Instant::now())),
            Err(reason) => self.notify(format!("Could not create {name}: {reason}")),
        }
    }

    /// Opens the diagram made by [`Self::new_diagram`] once it shows up in the index.
    pub(crate) fn open_pending_new(&mut self) {
        let Some((path, since)) = self.pending_new.clone() else {
            return;
        };
        let id = self
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.files.iter().find(|file| file.path == path))
            .map(|file| file.id.clone());
        match id {
            Some(id) => {
                self.pending_new = None;
                self.open_document(id, None, true);
                self.editor_open = true;
                self.record_state();
            }
            None if since.elapsed() > std::time::Duration::from_secs(8) => self.pending_new = None,
            None => {}
        }
    }
}
