//! The command palette and the keyboard shortcuts.

use std::path::{Path, PathBuf};

use egui::{
    Align2, Context, FontId, Id, Key, Margin, Modifiers, Rect, RichText, Sense, Ui, text::LayoutJob, vec2,
};
use tidemap_model::{FileEntry, RecentWorkspace};

use super::{controls::{self, key_cap}, surface};
use crate::{
    app::TidemapApp,
    icons::{self, Icon},
    motion,
    theme::{self, Palette, token},
};

/// Everything the palette can do besides opening files.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    Save,
    SaveCopy,
    ToggleSource,
    ToggleSidebar,
    Fit,
    ActualSize,
    ZoomIn,
    ZoomOut,
    ExportDiagram,
    ToggleTheme,
    ToggleReducedMotion,
    SwitchProject,
    OpenFolder,
    OpenFile,
    NewDiagram,
    PreviousFile,
    FocusFiles,
    SearchFiles,
    PinFile,
    NextDiagram,
    PreviousDiagram,
    FindInDiagram,
    ToggleOutline,
    ToggleDocument,
    Settings,
    Shortcuts,
}

/// Command, title and shortcut text (empty when there is no shortcut), in display order.
pub const COMMANDS: [(Command, &str, &str); 26] = [
    (Command::SwitchProject, "Switch project", "Ctrl+R"),
    (Command::Save, "Save", "Ctrl+S"),
    (Command::SaveCopy, "Save a copy", "Ctrl+Shift+S"),
    (Command::ToggleSource, "Show or hide source", "Ctrl+E"),
    (Command::ToggleSidebar, "Show or hide sidebar", "Ctrl+B"),
    (Command::FocusFiles, "Go to the file tree", "Ctrl+Shift+E"),
    (Command::SearchFiles, "Search files in the sidebar", "Ctrl+Shift+F"),
    (Command::PreviousFile, "Go to the previous file", "Ctrl+Tab"),
    (Command::Fit, "Fit diagram to window", "F"),
    (Command::ActualSize, "Actual size", "1"),
    (Command::ZoomIn, "Zoom in", "="),
    (Command::ZoomOut, "Zoom out", "-"),
    (Command::NextDiagram, "Next diagram", "]"),
    (Command::PreviousDiagram, "Previous diagram", "["),
    (Command::FindInDiagram, "Find in diagram", "Ctrl+F"),
    (Command::ToggleOutline, "Show or hide the outline", "Ctrl+Shift+L"),
    (Command::ToggleDocument, "Switch between the page and the diagram", "Ctrl+Shift+M"),
    (Command::ExportDiagram, "Export diagram", "Ctrl+Shift+X"),
    (Command::ToggleTheme, "Switch light or dark appearance", "Ctrl+Shift+D"),
    (Command::ToggleReducedMotion, "Toggle reduced motion", ""),
    (Command::OpenFolder, "Open folder", "Ctrl+Shift+O"),
    (Command::OpenFile, "Open file", "Ctrl+O"),
    (Command::NewDiagram, "New diagram", "Ctrl+N"),
    (Command::PinFile, "Pin or unpin file", "Ctrl+D"),
    (Command::Settings, "Settings", "Ctrl+,"),
    (Command::Shortcuts, "Keyboard shortcuts", "Ctrl+/"),
];

/// What the palette lists.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PaletteMode {
    /// Files, diagrams and (after `>`) commands.
    #[default]
    Files,
    /// Recently opened project folders.
    Projects,
}

/// One row of the result list.
#[derive(Clone, PartialEq, Debug)]
pub enum Hit {
    File { file_id: String, name: String, path: String },
    Diagram { file_id: String, title: String, path: String, index: usize },
    Command(Command),
    Project { root: PathBuf, name: String },
    /// "Open another folder" at the end of the project list.
    OpenFolder,
}

/// Rows shown at once; more scroll.
const VISIBLE_ROWS: usize = 9;
const ROW_HEIGHT: f32 = 40.0;
const INPUT_HEIGHT: f32 = 48.0;
const PANEL_WIDTH: f32 = 600.0;
const MAX_RESULTS: usize = 50;
const PLACEHOLDER: &str = "Find a file or diagram, or type > for commands";
const PROJECT_PLACEHOLDER: &str = "Switch to a recent project";
const FOOTER_HEIGHT: f32 = 36.0;

/// How well `text` matches `query`: a contiguous match scores highest, then a subsequence match.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i64> {
    let query = query.trim().to_lowercase();
    let text = text.to_lowercase();
    if query.is_empty() {
        return Some(1);
    }
    if let Some(index) = text.find(&query) {
        return Some(10_000 - index as i64 * 2 - text.len() as i64 / 4);
    }
    let chars: Vec<char> = text.chars().collect();
    let mut cursor = 0;
    let mut score = 0;
    for wanted in query.chars() {
        let offset = chars.get(cursor..)?.iter().position(|ch| *ch == wanted)?;
        let index = cursor + offset;
        score += if offset == 0 && cursor > 0 { 12 } else { 2 };
        if index == 0 || matches!(chars[index - 1], '/' | ' ' | '-' | '_') {
            score += 8;
        }
        cursor = index + 1;
    }
    Some(score - chars.len() as i64 / 20)
}

fn file_hit(file: &FileEntry) -> Hit {
    Hit::File { file_id: file.id.clone(), name: file.name.clone(), path: file.path.clone() }
}

/// The rows for `query`. `pins` and `recent` hold file ids, `recent` most recent first.
pub fn results(query: &str, files: &[FileEntry], pins: &[String], recent: &[String], limit: usize) -> Vec<Hit> {
    if let Some(command_query) = query.strip_prefix('>') {
        return command_results(command_query, limit);
    }
    if query.trim().is_empty() {
        return default_results(files, pins, recent, limit);
    }

    // (score, path length, path, diagram index, hit); sorted best first.
    let mut scored: Vec<(i64, usize, &str, usize, Hit)> = Vec::new();
    for file in files {
        let name = fuzzy_score(query, &file.name).map(|score| score * 3);
        let path = fuzzy_score(query, &file.path);
        if let Some(score) = name.max(path) {
            let bonus = if pins.contains(&file.id) { 5 } else { 0 };
            scored.push((score + bonus, file.path.len(), &file.path, 0, file_hit(file)));
        }
        for (index, title) in file.titles.iter().enumerate() {
            if let Some(score) = fuzzy_score(query, title) {
                let hit = Hit::Diagram {
                    file_id: file.id.clone(),
                    title: title.clone(),
                    path: file.path.clone(),
                    index,
                };
                scored.push((score, file.path.len(), &file.path, index + 1, hit));
            }
        }
    }
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.cmp(&b.1))
            .then(a.2.cmp(b.2))
            .then(a.3.cmp(&b.3))
    });
    scored.into_iter().take(limit).map(|entry| entry.4).collect()
}

fn command_results(query: &str, limit: usize) -> Vec<Hit> {
    if query.trim().is_empty() {
        return COMMANDS.iter().map(|(command, ..)| Hit::Command(*command)).take(limit).collect();
    }
    let mut scored: Vec<(i64, Command)> = COMMANDS
        .iter()
        .filter_map(|(command, title, _)| fuzzy_score(query, title).map(|score| (score, *command)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().take(limit).map(|(_, command)| Hit::Command(command)).collect()
}

/// `path` with the home folder shown as `~`, for display.
pub fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let text = text.strip_prefix("//?/").unwrap_or(&text).to_owned();
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|home| home.to_string_lossy().replace('\\', "/"));
    match home {
        Some(home) if !home.is_empty() && text.to_lowercase().starts_with(&home.to_lowercase()) => {
            format!("~{}", &text[home.len()..])
        }
        _ => text,
    }
}

/// Recent projects other than the open one, most recent first, then "Open another folder".
pub fn project_results(query: &str, recents: &[RecentWorkspace], current: Option<&Path>) -> Vec<Hit> {
    let others = recents.iter().filter(|recent| Some(recent.root.as_path()) != current);
    let mut hits: Vec<Hit> = if query.trim().is_empty() {
        others.map(|recent| Hit::Project { root: recent.root.clone(), name: recent.name.clone() }).collect()
    } else {
        let mut scored: Vec<(i64, usize, Hit)> = others
            .enumerate()
            .filter_map(|(order, recent)| {
                let name = fuzzy_score(query, &recent.name).map(|score| score * 3);
                let path = fuzzy_score(query, &display_path(&recent.root));
                let score = name.max(path)?;
                Some((score, order, Hit::Project { root: recent.root.clone(), name: recent.name.clone() }))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().map(|entry| entry.2).collect()
    };
    if query.trim().is_empty() || fuzzy_score(query, "open another folder").is_some() {
        hits.push(Hit::OpenFolder);
    }
    hits
}

/// Recent files, then pinned ones. A brand-new workspace has neither, so it lists its files instead.
fn default_results(files: &[FileEntry], pins: &[String], recent: &[String], limit: usize) -> Vec<Hit> {
    let mut ids: Vec<&str> = Vec::new();
    for id in recent.iter().chain(pins) {
        if !ids.contains(&id.as_str()) && files.iter().any(|file| &file.id == id) {
            ids.push(id);
        }
    }
    let mut hits: Vec<Hit> = ids
        .iter()
        .filter_map(|id| files.iter().find(|file| file.id == *id))
        .map(file_hit)
        .collect();
    if hits.is_empty() {
        hits = files.iter().map(file_hit).collect();
    }
    hits.truncate(limit);
    hits
}

/// Whether a text field has the keyboard. (egui's `wants_keyboard_input` is true for any
/// focused widget, the canvas included, which would switch every bare-key shortcut off as
/// soon as the diagram was clicked.)
pub fn typing(ctx: &Context) -> bool {
    let focused = ctx.memory(|memory| memory.focused());
    focused.is_some_and(|id| {
        egui::TextEdit::load_state(ctx, id).is_some() || crate::editor::focused_id(ctx) == Some(id)
    })
}

/// Whether a bare key such as `F` may act as a shortcut: not while typing and not under a modal.
pub fn single_key_allowed(ctx: &Context) -> bool {
    !typing(ctx) && ctx.memory(|memory| memory.top_modal_layer().is_none())
}

/// The sidebar's icons: a diagram for Mermaid files, a document for Markdown.
fn file_icon(name: &str) -> Icon {
    if name.to_lowercase().ends_with(".mmd") {
        Icon::Diagram
    } else {
        Icon::File
    }
}

fn command_icon(command: Command, dark: bool) -> Icon {
    match command {
        Command::Save | Command::SaveCopy => Icon::Save,
        Command::ToggleSource => Icon::Code,
        Command::ToggleSidebar | Command::FocusFiles => Icon::Sidebar,
        Command::SearchFiles => Icon::Search,
        Command::PreviousFile => Icon::Clock,
        Command::SwitchProject => Icon::Stack,
        Command::Fit => Icon::Fit,
        Command::ActualSize => Icon::ActualSize,
        Command::ZoomIn => Icon::Plus,
        Command::ZoomOut => Icon::Minus,
        Command::ToggleOutline => Icon::File,
        Command::ToggleDocument => Icon::Diagram,
        Command::ExportDiagram => Icon::Export,
        Command::ToggleTheme => {
            if dark {
                Icon::Sun
            } else {
                Icon::Moon
            }
        }
        Command::ToggleReducedMotion => Icon::Settings,
        Command::OpenFolder => Icon::FolderOpen,
        Command::OpenFile => Icon::File,
        Command::NewDiagram => Icon::FilePlus,
        Command::PinFile => Icon::Pin,
        Command::NextDiagram => Icon::Forward,
        Command::PreviousDiagram => Icon::Back,
        Command::FindInDiagram => Icon::Search,
        Command::Settings => Icon::Settings,
        Command::Shortcuts => Icon::Keyboard,
    }
}

// ---------------------------------------------------------------------------
// Shortcuts
// ---------------------------------------------------------------------------

/// A key chord and what it does.
enum Action {
    Palette(&'static str),
    Projects,
    Command(Command),
    Find,
    Back,
    Forward,
}

/// Chords with more modifiers come first so that Ctrl+Shift+S is not taken for Ctrl+S
/// (egui ignores extra Shift and Alt when matching).
fn chords() -> Vec<(Modifiers, Key, Action)> {
    let ctrl = Modifiers::COMMAND;
    let ctrl_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    vec![
        (ctrl_shift, Key::P, Action::Palette(">")),
        (ctrl_shift, Key::O, Action::Command(Command::OpenFolder)),
        (ctrl_shift, Key::S, Action::Command(Command::SaveCopy)),
        (ctrl_shift, Key::E, Action::Command(Command::FocusFiles)),
        (ctrl_shift, Key::F, Action::Command(Command::SearchFiles)),
        (ctrl_shift, Key::X, Action::Command(Command::ExportDiagram)),
        (ctrl_shift, Key::D, Action::Command(Command::ToggleTheme)),
        (ctrl_shift, Key::L, Action::Command(Command::ToggleOutline)),
        (ctrl_shift, Key::M, Action::Command(Command::ToggleDocument)),
        (ctrl, Key::P, Action::Palette("")),
        (ctrl, Key::K, Action::Palette("")),
        (ctrl, Key::R, Action::Projects),
        (ctrl, Key::O, Action::Command(Command::OpenFile)),
        (ctrl, Key::N, Action::Command(Command::NewDiagram)),
        (ctrl, Key::E, Action::Command(Command::ToggleSource)),
        (ctrl, Key::S, Action::Command(Command::Save)),
        (ctrl, Key::B, Action::Command(Command::ToggleSidebar)),
        (ctrl, Key::D, Action::Command(Command::PinFile)),
        (ctrl, Key::Tab, Action::Command(Command::PreviousFile)),
        (ctrl, Key::Comma, Action::Command(Command::Settings)),
        (ctrl, Key::Slash, Action::Command(Command::Shortcuts)),
        (ctrl, Key::F, Action::Find),
        (Modifiers::NONE, Key::F1, Action::Command(Command::Shortcuts)),
        (Modifiers::ALT, Key::ArrowLeft, Action::Back),
        (Modifiers::ALT, Key::ArrowRight, Action::Forward),
        (Modifiers::ALT, Key::PageUp, Action::Command(Command::PreviousDiagram)),
        (Modifiers::ALT, Key::PageDown, Action::Command(Command::NextDiagram)),
    ]
}

/// Bare keys, active only while nothing takes text and no modal is up.
fn single_keys() -> [(Key, Command); 10] {
    [
        (Key::F, Command::Fit),
        (Key::Num0, Command::Fit),
        (Key::Num1, Command::ActualSize),
        (Key::Equals, Command::ZoomIn),
        (Key::Plus, Command::ZoomIn),
        (Key::Minus, Command::ZoomOut),
        (Key::OpenBracket, Command::PreviousDiagram),
        (Key::CloseBracket, Command::NextDiagram),
        (Key::Questionmark, Command::Shortcuts),
        (Key::Slash, Command::SearchFiles),
    ]
}

/// Handles the keyboard for one frame: chords, the single keys and the Esc cascade.
pub fn shortcuts(app: &mut TidemapApp, ctx: &Context) {
    let modal = app.palette_open
        || app.dialog.is_some()
        || app.recovery_file.is_some()
        || app.conflict_file.is_some();
    if escape(app, ctx) {
        return;
    }
    let pressed = ctx.input_mut(|input| {
        chords()
            .into_iter()
            .find(|(modifiers, key, _)| input.consume_key(*modifiers, *key))
            .map(|(_, _, action)| action)
    });
    if matches!(pressed, Some(Action::Command(Command::PreviousFile))) {
        // Ctrl+Tab must not also move egui's keyboard focus to the next widget.
        ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
    }
    match pressed {
        Some(Action::Palette(prefill)) => app.toggle_palette(prefill),
        Some(Action::Projects) => app.toggle_projects(),
        Some(_) if modal => {}
        Some(Action::Command(command)) => app.run_command(command),
        Some(Action::Find) => {
            if app.editor_open && typing(ctx) && !app.sidebar_focused {
                app.editor.find();
            } else {
                app.open_search();
            }
        }
        Some(Action::Back) => app.history_step(-1),
        Some(Action::Forward) => app.history_step(1),
        None => {}
    }
    if !modal && single_key_allowed(ctx) {
        let hit = ctx.input_mut(|input| {
            // `?` arrives as Shift+Slash on most layouts: check it before the bare slash.
            if input.consume_key(Modifiers::SHIFT, Key::Slash) {
                return Some(Command::Shortcuts);
            }
            single_keys()
                .into_iter()
                .find(|(key, _)| input.consume_key(Modifiers::NONE, *key))
                .map(|(_, command)| command)
        });
        if let Some(command) = hit {
            app.run_command(command);
        }
    }
}

/// Esc closes one thing per press: the diagram menu, palette, dialog, info popover, diagram search, selection.
/// Returns true when it did. The sidebar handles its own Esc while it has the keyboard.
fn escape(app: &mut TidemapApp, ctx: &Context) -> bool {
    // The diagram menu is the topmost thing: Esc closes it before anything else.
    if super::topbar::block_menu_open(ctx, &app.active_file)
        && !app.palette_open
        && app.dialog.is_none()
        && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
    {
        super::topbar::close_block_menu(ctx, &app.active_file);
        return true;
    }
    if app.sidebar_focused && !app.palette_open && app.dialog.is_none() {
        return false;
    }
    let selection = app.canvas.selected.is_some() || app.canvas.selected_edge.is_some();
    let anything = app.palette_open
        || app.dialog.is_some()
        || app.conflict_file.is_some()
        || app.info_open
        || app.search_open
        || app.outline_open
        || selection;
    if !anything || !ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
        return false;
    }
    if app.palette_open {
        app.palette_open = false;
    } else if app.dialog.is_some() {
        app.dialog = None;
    } else if app.conflict_file.is_some() {
        app.conflict_file = None;
    } else if app.info_open {
        app.info_open = false;
    } else if app.search_open {
        app.close_search();
    } else if selection {
        app.canvas.clear_selection();
    } else {
        app.outline_open = false;
    }
    true
}

// ---------------------------------------------------------------------------
// Palette
// ---------------------------------------------------------------------------

/// The rows for the palette as it is now.
fn current_results(app: &TidemapApp) -> Vec<Hit> {
    if app.palette_mode == PaletteMode::Projects {
        let current = app.workspace.as_ref().map(|workspace| workspace.root.as_path());
        return project_results(&app.palette_query, &app.recents, current);
    }
    let files: &[FileEntry] = app
        .workspace
        .as_ref()
        .map(|workspace| workspace.files.as_slice())
        .unwrap_or(&[]);
    let recent: Vec<String> = app.state.history.iter().rev().map(|entry| entry.file_id.clone()).collect();
    results(&app.palette_query, files, &app.state.pins, &recent, MAX_RESULTS)
}

const PRESENCE_ID: &str = "palette-presence";
const HIGHLIGHT_ID: &str = "palette-highlight";
const INPUT_ID: &str = "palette-input";

/// Shows the palette while it is open or still leaving. Returns its presence (0 when closed)
/// so the frame can sink the window behind it.
pub fn show(app: &mut TidemapApp, ctx: &Context, p: Palette) -> f32 {
    let Some(t) = motion::presence(ctx, Id::new(PRESENCE_ID), app.palette_open) else {
        return 0.0;
    };
    let open = app.palette_open;
    let screen = ctx.content_rect();
    let width = PANEL_WIDTH.min(screen.width() - 2.0 * token::SPACE[4]);
    let top = (screen.height() * 0.18).round();
    let place = surface::Placement {
        id: Id::new("palette"),
        anchor: Align2::CENTER_TOP,
        offset: vec2(0.0, top),
        // Grows from the top centre, where the eye already is.
        origin: egui::pos2(screen.center().x, top),
        enter: surface::PALETTE,
    };
    let shown = surface::modal(ctx, p, place, t, open, |ui, interactive| {
        theme::elevated(ui, p, theme::Level::Modal, token::RADIUS_DIALOG, Margin::ZERO, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            contents(app, ui, ctx, p, interactive)
        })
        .inner
    });
    if open {
        if shown.backdrop_clicked {
            app.palette_open = false;
        }
        if let Some(hit) = shown.inner {
            app.palette_open = false;
            app.open_hit(hit);
        }
    }
    t
}

/// The palette's contents. While `interactive` is false (the palette is leaving) it only
/// draws: no keys are taken, no focus is requested and the text field is not editable.
fn contents(app: &mut TidemapApp, ui: &mut Ui, ctx: &Context, p: Palette, interactive: bool) -> Option<Hit> {
    // Keys are taken before the text field sees them (it would move its cursor instead).
    let empty_query = app.palette_query.is_empty();
    let (down, up, page_down, page_up, enter, delete, back) = if !interactive {
        (false, false, false, false, false, false, false)
    } else {
        ctx.input_mut(|input| {
        (
            input.consume_key(Modifiers::NONE, Key::ArrowDown) || input.consume_key(Modifiers::CTRL, Key::N),
            input.consume_key(Modifiers::NONE, Key::ArrowUp),
            input.consume_key(Modifiers::NONE, Key::PageDown),
            input.consume_key(Modifiers::NONE, Key::PageUp),
            input.consume_key(Modifiers::NONE, Key::Enter),
            app.palette_mode == PaletteMode::Projects && input.consume_key(Modifiers::NONE, Key::Delete),
            app.palette_mode == PaletteMode::Projects && empty_query && input.consume_key(Modifiers::NONE, Key::Backspace),
        )
        })
    };
    if interactive && app.palette_fresh {
        // A fresh opening: the highlight starts on its row instead of gliding from the last one.
        controls::restart_band(ctx, Id::new(HIGHLIGHT_ID));
    }
    if back {
        // Backspace on an empty project query goes back to finding files.
        app.palette_mode = PaletteMode::Files;
        app.palette_index = 0;
        controls::restart_band(ctx, Id::new(HIGHLIGHT_ID));
    }
    let changed = input_row(app, ui, ctx, p, interactive);
    if changed {
        app.palette_index = 0;
        controls::restart_band(ctx, Id::new(HIGHLIGHT_ID));
    }
    let hits = current_results(app);
    let last = hits.len().saturating_sub(1);
    if down {
        app.palette_index = if app.palette_index >= last { 0 } else { app.palette_index + 1 };
    }
    if up {
        app.palette_index = if app.palette_index == 0 { last } else { app.palette_index - 1 };
    }
    if page_down {
        app.palette_index = (app.palette_index + VISIBLE_ROWS).min(last);
    }
    if page_up {
        app.palette_index = app.palette_index.saturating_sub(VISIBLE_ROWS);
    }
    app.palette_index = app.palette_index.min(last);
    if delete {
        if let Some(Hit::Project { root, name }) = hits.get(app.palette_index).cloned() {
            app.forget_project(&root, &name);
        }
    }

    ui.add_space(token::SPACE[1]);
    let mut chosen = None;
    if hits.is_empty() {
        let message = empty_message(app);
        ui.add_space(token::SPACE[2]);
        ui.horizontal(|ui| {
            ui.add_space(token::SPACE[3]);
            ui.label(RichText::new(message).size(token::FONT_UI).color(p.muted));
        });
        ui.add_space(token::SPACE[3]);
    } else {
        chosen = result_list(app, ui, p, &hits, down || up || page_down || page_up);
        if enter && chosen.is_none() {
            chosen = hits.get(app.palette_index).cloned();
        }
        ui.add_space(token::SPACE[1]);
    }
    footer(app, ui, p);
    chosen
}

/// A quiet strip of key hints under the list.
fn footer(app: &TidemapApp, ui: &mut Ui, p: Palette) {
    let hints: &[(&str, &str)] = match app.palette_mode {
        PaletteMode::Projects => &[("Enter", "open"), ("Del", "remove from list"), ("Backspace", "files")],
        PaletteMode::Files if app.palette_query.starts_with('>') => &[("Enter", "run"), ("Esc", "close")],
        PaletteMode::Files => &[("Enter", "open"), (">", "commands"), ("Ctrl+R", "projects")],
    };
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), FOOTER_HEIGHT), Sense::hover());
    ui.painter().hline(rect.x_range().shrink(token::SPACE[2]), rect.top(), egui::Stroke::new(1.0_f32, p.hairline));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(token::SPACE[3], 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.spacing_mut().item_spacing.x = token::SPACE[0];
    for (keys, what) in hints {
        for key in keys.split('+') {
            key_cap(&mut child, key);
        }
        child.label(RichText::new(*what).size(token::FONT_SMALL).color(p.muted));
        child.add_space(token::SPACE[2]);
    }
}

fn empty_message(app: &TidemapApp) -> String {
    let query = app.palette_query.trim();
    if app.palette_mode == PaletteMode::Projects {
        return if query.is_empty() {
            "No other projects yet. Open a folder to add one.".to_owned()
        } else {
            format!("No project matches \u{201c}{query}\u{201d}.")
        };
    }
    if let Some(command) = query.strip_prefix('>') {
        format!("No command matches \u{201c}{}\u{201d}.", command.trim())
    } else if query.is_empty() {
        "Open a folder to search its files.".to_owned()
    } else {
        format!("Nothing matches \u{201c}{query}\u{201d}.")
    }
}

/// The search icon and the text field. Returns true when the text changed.
fn input_row(app: &mut TidemapApp, ui: &mut Ui, ctx: &Context, p: Palette, interactive: bool) -> bool {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), INPUT_HEIGHT), Sense::hover());
    let icon = Rect::from_center_size(egui::pos2(row.left() + 16.0 + 8.0, row.center().y), vec2(16.0, 16.0));
    let (glyph, placeholder) = match app.palette_mode {
        PaletteMode::Projects => (Icon::Stack, PROJECT_PLACEHOLDER),
        PaletteMode::Files => (Icon::Search, PLACEHOLDER),
    };
    icons::paint(ui.painter(), glyph, icon, p.muted);
    let field = Rect::from_min_max(egui::pos2(icon.right() + 12.0, row.top()), egui::pos2(row.right() - 16.0, row.bottom()));
    let id = Id::new(INPUT_ID);
    let edit = egui::TextEdit::singleline(&mut app.palette_query)
        .id(id)
        .interactive(interactive)
        .font(FontId::proportional(16.0))
        .hint_text(RichText::new(placeholder).size(16.0).color(p.muted))
        .frame(false)
        .margin(Margin::ZERO)
        .vertical_align(egui::Align::Center)
        .desired_width(field.width());
    let response = ui.put(field, edit);
    if !interactive {
        return false;
    }
    if app.palette_fresh {
        app.palette_fresh = false;
        // Pre-filled text ("> ") is not selected: the cursor goes to its end.
        if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
            let end = app.palette_query.chars().count();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(end))));
            state.store(ctx, id);
        }
    }
    response.request_focus();
    response.changed()
}

/// The scrolling list. Returns the hit that was clicked. The selection highlight glides
/// between rows instead of jumping.
fn result_list(app: &TidemapApp, ui: &mut Ui, p: Palette, hits: &[Hit], keyboard: bool) -> Option<Hit> {
    let mut chosen = None;
    let height = ROW_HEIGHT * hits.len().min(VISIBLE_ROWS) as f32;
    let ctx = ui.ctx().clone();
    // In row units: the band's two edges, stretching as the selection travels.
    let index = app.palette_index as f32;
    let (band_top, band_bottom) = controls::stretch_band(&ctx, Id::new(HIGHLIGHT_ID), index, index + 1.0);
    egui::ScrollArea::vertical()
        .id_salt("palette-results")
        .max_height(height)
        .auto_shrink([false, true])
        .show_rows(ui, ROW_HEIGHT, hits.len(), |ui, range| {
            let origin = ui.cursor().top() - range.start as f32 * ROW_HEIGHT;
            let inset = token::SPACE[1];
            let band = Rect::from_min_max(
                egui::pos2(ui.max_rect().left() + inset, origin + band_top * ROW_HEIGHT),
                egui::pos2(ui.max_rect().right() - inset, origin + band_bottom * ROW_HEIGHT),
            );
            ui.painter().rect_filled(band, token::RADIUS_CONTROL, p.accent_soft);
            for index in range {
                let selected = index == app.palette_index;
                let response = result_row(ui, p, &hits[index], selected);
                if selected && keyboard {
                    response.scroll_to_me(None);
                }
                if response.clicked() {
                    chosen = Some(hits[index].clone());
                }
            }
        });
    chosen
}

fn result_row(ui: &mut Ui, p: Palette, hit: &Hit, selected: bool) -> egui::Response {
    let inset = token::SPACE[1];
    let width = ui.available_width() - 2.0 * inset;
    let (area, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    let rect = Rect::from_min_max(
        egui::pos2(area.left() + inset, area.top()),
        egui::pos2(area.right() - inset, area.bottom()),
    );
    let hover = motion::hover(ui.ctx(), response.id, response.hovered() && !selected);
    if hover > 0.0 {
        ui.painter().rect_filled(rect, token::RADIUS_CONTROL, p.hover.gamma_multiply(hover));
    }

    let (icon, title, trailing) = match hit {
        Hit::File { name, path, .. } => (file_icon(name), name.clone(), path.clone()),
        Hit::Diagram { title, path, .. } => (Icon::Diagram, title.clone(), path.clone()),
        Hit::Command(command) => {
            let (_, title, shortcut) = COMMANDS.iter().find(|(candidate, ..)| candidate == command).copied().unwrap_or((*command, "", ""));
            (command_icon(*command, ui.visuals().dark_mode), title.to_owned(), shortcut.replace('+', " + "))
        }
        Hit::Project { root, name } => (Icon::Folder, name.clone(), display_path(root)),
        Hit::OpenFolder => (Icon::FolderOpen, "Open another folder\u{2026}".to_owned(), "Ctrl + Shift + O".to_owned()),
    };
    let icon_rect = Rect::from_center_size(egui::pos2(rect.left() + 12.0 + 8.0, rect.center().y), vec2(16.0, 16.0));
    let icon_color = if selected { p.accent } else { p.secondary };
    icons::paint(ui.painter(), icon, icon_rect, icon_color);

    // The path (or shortcut) takes at most 45% of the row and is cut from the left when long.
    let small = FontId::proportional(token::FONT_SMALL);
    let trailing_budget = width * 0.45;
    let trailing_galley = elided(ui, &trailing, small, p.muted, trailing_budget, true);
    let trailing_pos = egui::pos2(rect.right() - 12.0 - trailing_galley.size().x, rect.center().y - trailing_galley.size().y * 0.5);
    let title_left = icon_rect.right() + 12.0;
    let title_budget = (trailing_pos.x - 12.0 - title_left).max(0.0);
    ui.painter().galley(trailing_pos, trailing_galley, p.muted);
    let title_galley = elided(ui, &title, FontId::proportional(token::FONT_UI), p.ink, title_budget, false);
    let title_pos = egui::pos2(title_left, rect.center().y - title_galley.size().y * 0.5);
    ui.painter().galley(title_pos, title_galley, p.ink);
    response
}

/// One line of text shortened with an ellipsis to fit `budget`. `from_left` keeps the end.
pub(super) fn elided(ui: &Ui, text: &str, font: FontId, color: egui::Color32, budget: f32, from_left: bool) -> std::sync::Arc<egui::Galley> {
    let shown = if from_left { tail_that_fits(ui, text, &font, budget) } else { text.to_owned() };
    let mut job = LayoutJob::simple_singleline(shown, font, color);
    job.wrap.max_width = budget;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('\u{2026}');
    ui.painter().layout_job(job)
}

/// `text`, or an ellipsis followed by its end when the whole does not fit `budget`.
fn tail_that_fits(ui: &Ui, text: &str, font: &FontId, budget: f32) -> String {
    let width = |candidate: &str| ui.painter().layout_no_wrap(candidate.to_owned(), font.clone(), egui::Color32::WHITE).size().x;
    if width(text) <= budget {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    for dropped in 1..chars.len() {
        let candidate: String = std::iter::once('\u{2026}').chain(chars[dropped..].iter().copied()).collect();
        if width(&candidate) <= budget {
            return candidate;
        }
    }
    "\u{2026}".to_owned()
}

impl TidemapApp {
    /// Opens the file, diagram or command a palette row stands for.
    pub(crate) fn open_hit(&mut self, hit: Hit) {
        match hit {
            Hit::File { file_id, .. } => self.open_document(file_id, None, true),
            Hit::Diagram { file_id, index, .. } => {
                self.open_document(file_id, Some(format!("index\0{index}")), true)
            }
            Hit::Command(command) => self.run_command(command),
            Hit::Project { root, .. } => self.switch_project(root),
            Hit::OpenFolder => self.open_folder(None),
        }
    }
}
