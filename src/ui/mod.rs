//! The window chrome: top bar, sidebar card, source card, canvas area and the floating pill.
//!
//! `frame` decides the layout once per frame and calls each surface in order. Cards float on
//! the `app` backdrop: they are inset from the window edges and from the canvas, so the gaps
//! are real backdrop rather than borders.

use egui::{Context, CursorIcon, Frame, Id, Margin, Rect, Sense, Stroke, Ui, UiBuilder, Vec2};

use tidemap_diagram::ItemId;

use crate::{
    app::{CardAction, TidemapApp},
    canvas, motion,
    theme::{self, Palette, token},
};

pub mod controls;
pub mod dialogs;
pub mod info;
pub mod palette;
pub mod pill;
pub mod sidebar;
pub mod snapshot;
pub mod source;
pub mod states;
pub mod surface;
pub mod topbar;
pub mod tree;

/// Width of the strip that grabs a panel edge, centred in the backdrop gap.
const GRAB_WIDTH: f32 = token::INSET;
/// Room kept under the source card (in `Replace` mode) so the pill sits on the backdrop.
const PILL_RESERVE: f32 = 60.0;
/// A side panel never takes the last of this much canvas width.
const MIN_CANVAS_WIDTH: f32 = 480.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SourceMode {
    Hidden,
    /// Beside the canvas, which shrinks.
    Side,
    /// Instead of the canvas (narrow windows).
    Replace,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FrameMode {
    pub sidebar: bool,
    pub source: SourceMode,
}

/// Which panels exist at this window width.
pub fn frame_mode(
    window_width: f32,
    sidebar_open: bool,
    source_open: bool,
    has_workspace: bool,
) -> FrameMode {
    let wide = window_width >= token::NARROW;
    let source = match (source_open, wide) {
        (false, _) => SourceMode::Hidden,
        (true, true) => SourceMode::Side,
        (true, false) => SourceMode::Replace,
    };
    FrameMode {
        sidebar: sidebar_open && has_workspace && wide,
        source,
    }
}

/// Content card (below the bar, beside the panels) as a fraction of the window area. A side
/// panel's width already includes its own backdrop gap; the card adds one on each side that has
/// no panel, and one under it.
pub fn canvas_fraction(window: Vec2, mode: FrameMode, sidebar_w: f32, source_w: f32) -> f32 {
    let mut width = window.x;
    for (open, panel_w) in [(mode.sidebar, sidebar_w), (mode.source == SourceMode::Side, source_w)] {
        width -= if open { panel_w } else { token::INSET };
    }
    let height = window.y - token::BAR_HEIGHT - token::INSET;
    (width.max(0.0) * height.max(0.0)) / (window.x * window.y).max(1.0)
}

fn sidebar_width(app: &TidemapApp) -> f32 {
    let stored = app.settings.sidebar_width;
    let width = if stored > 0.0 { stored } else { token::SIDEBAR };
    width.clamp(token::SIDEBAR_MIN, token::SIDEBAR_MAX)
}

fn source_width(app: &TidemapApp, window_width: f32) -> f32 {
    let stored = app.settings.editor_width;
    let width = if stored > 0.0 { stored } else { token::SOURCE };
    let max = (window_width - MIN_CANVAS_WIDTH).max(token::SOURCE_MIN);
    width.clamp(token::SOURCE_MIN, max)
}

/// Lays out the whole window for one frame.
pub fn frame(app: &mut TidemapApp, ctx: &Context) {
    let p = theme::palette(app.dark);
    let screen = ctx.content_rect();
    ctx.layer_painter(egui::LayerId::background())
        .rect_filled(screen, 0.0, p.app);

    let has_file = !app.active_file.is_empty();
    let mode = frame_mode(
        screen.width(),
        app.sidebar_open,
        app.editor_open && has_file,
        app.workspace.is_some(),
    );

    topbar::show(app, ctx, p);
    // Panels glide on a spring: their width follows it and the solid card slides with the moving edge.
    let sidebar_t = motion::spring(ctx, Id::new("sidebar-reveal"), f32::from(mode.sidebar), motion::GLIDE).clamp(0.0, 1.0);
    if sidebar_t > 0.002 && app.workspace.is_some() && screen.width() >= token::NARROW {
        sidebar_panel(app, ctx, p, sidebar_t);
    }
    let source_t = motion::spring(ctx, Id::new("source-reveal"), f32::from(mode.source == SourceMode::Side), motion::GLIDE).clamp(0.0, 1.0);
    if source_t > 0.002 && has_file && screen.width() >= token::NARROW {
        source_panel(app, ctx, p, source_t);
    }

    let mut canvas = screen;
    // How far each side panel has opened: the content card's gap on that side shrinks to
    // nothing as the panel's own gap takes over.
    let reveal = (sidebar_t, source_t);
    egui::CentralPanel::default()
        .frame(Frame::NONE)
        .show(ctx, |ui| canvas = central(app, ui, p, mode, reveal));

    if app.shot.is_some() && has_file && app.canvas.painted_frames == 3 {
        let share = canvas.area() / screen.area().max(1.0);
        let planned = canvas_fraction(
            screen.size(),
            mode,
            sidebar_width(app),
            source_width(app, screen.width()),
        );
        eprintln!(
            "canvas {:.0}x{:.0} of {:.0}x{:.0} window: {:.1}% (planned {:.1}%)",
            canvas.width(),
            canvas.height(),
            screen.width(),
            screen.height(),
            share * 100.0,
            planned * 100.0
        );
    }
    app.canvas_area = canvas;
    if has_file && !app.document_view() {
        pill::show_pill(app, ctx, canvas, p);
    }
    states::search_overlay(app, ctx, canvas, p);
    info::show(app, ctx, app.info_anchor, p);
    let palette = palette::show(app, ctx, p);
    let dialogs = dialogs::show(app, ctx, p);
    // The window sinks behind whichever modal is more present. This runs before the toast,
    // which composes the same transform with its own entrance.
    surface::push_back(ctx, palette.max(dialogs).clamp(0.0, 1.0));
    info::toast(app, ctx, canvas, p);
}

/// A rounded white card with the standard padding. `margin` is the backdrop gap around it.
fn card(ui: &mut Ui, p: Palette, margin: Margin, add_contents: impl FnOnce(&mut Ui)) {
    Frame::NONE.outer_margin(margin).show(ui, |ui| {
        theme::elevated(ui, p, theme::Level::Card, token::RADIUS_PANEL, Margin::same(12), |ui| {
            ui.set_min_size(ui.available_size());
            // Nothing may paint outside the rounded card.
            ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
            add_contents(ui);
        });
    });
}

/// Draws a hairline and sets the resize cursor while the strip is hovered or dragged.
/// Returns the pointer x while dragging.
fn resize_strip(ui: &mut Ui, id: &'static str, strip: Rect, p: Palette) -> Option<f32> {
    let response = ui.interact(strip, egui::Id::new(id), Sense::drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        ui.painter()
            .vline(strip.center().x, strip.y_range(), Stroke::new(1.0_f32, p.hairline));
    }
    if response.dragged() {
        response.interact_pointer_pos().map(|pos| pos.x)
    } else {
        None
    }
}

/// Backdrop gap around a card that hangs from the top bar.
fn inset_margin(bottom: f32) -> Margin {
    Margin {
        left: token::INSET as i8,
        right: token::INSET as i8,
        top: 0,
        bottom: bottom as i8,
    }
}

/// Lays a panel's card out at its full width inside `panel` (which may still be opening),
/// anchored to the panel's inner edge so the solid card slides with it.
fn sliding_card(ui: &mut Ui, panel: Rect, full: f32, from_left: bool, add: impl FnOnce(&mut Ui)) {
    let left = if from_left { panel.right() - full } else { panel.left() };
    let content = Rect::from_min_size(egui::pos2(left, panel.top()), Vec2::new(full, panel.height()));
    ui.set_clip_rect(panel.intersect(ui.clip_rect()));
    ui.scope_builder(UiBuilder::new().max_rect(content), add);
}

fn sidebar_panel(app: &mut TidemapApp, ctx: &Context, p: Palette, t: f32) {
    let full = sidebar_width(app);
    let width = (full * t).max(1.0);
    let mut dragged = None;
    egui::SidePanel::left("sidebar")
        .exact_width(width)
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::NONE)
        .show(ctx, |ui| {
            let panel = ui.max_rect();
            sliding_card(ui, panel, full, true, |ui| {
                card(ui, p, inset_margin(token::INSET), |ui| sidebar::show(app, ui, p));
            });
            if t >= 1.0 {
                let strip = Rect::from_min_max(
                    egui::pos2(panel.right() - GRAB_WIDTH, panel.top()),
                    egui::pos2(panel.right(), panel.bottom() - token::INSET),
                );
                dragged = resize_strip(ui, "sidebar-resize", strip, p);
            }
        });
    if let Some(x) = dragged {
        app.settings.sidebar_width =
            (x + GRAB_WIDTH * 0.5).clamp(token::SIDEBAR_MIN, token::SIDEBAR_MAX);
    }
}

fn source_panel(app: &mut TidemapApp, ctx: &Context, p: Palette, t: f32) {
    let window_width = ctx.content_rect().width();
    let full = source_width(app, window_width);
    let width = (full * t).max(1.0);
    let mut dragged = None;
    egui::SidePanel::right("source-panel")
        .exact_width(width)
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::NONE)
        .show(ctx, |ui| {
            let panel = ui.max_rect();
            sliding_card(ui, panel, full, false, |ui| {
                card(ui, p, inset_margin(token::INSET), |ui| source::show_source(app, ui, p));
            });
            if t >= 1.0 {
                let strip = Rect::from_min_max(
                    egui::pos2(panel.left(), panel.top()),
                    egui::pos2(panel.left() + GRAB_WIDTH, panel.bottom() - token::INSET),
                );
                dragged = resize_strip(ui, "source-resize", strip, p);
            }
        });
    if let Some(x) = dragged {
        let max = (window_width - MIN_CANVAS_WIDTH).max(token::SOURCE_MIN);
        app.settings.editor_width =
            (window_width - x + GRAB_WIDTH * 0.5).clamp(token::SOURCE_MIN, max);
    }
}

/// Backdrop gap on the side of the content card that faces a panel (`reveal` is how far that
/// panel has opened): the panel's own gap fills it once open.
fn side_gap(reveal: f32) -> i8 {
    (token::INSET * (1.0 - reveal.clamp(0.0, 1.0))).round() as i8
}

/// The area between the panels. Returns the rectangle the pill is centred on: the content card
/// when a file is open, otherwise the whole area. `reveal` is how far the sidebar and the source
/// panel have opened.
fn central(app: &mut TidemapApp, ui: &mut Ui, p: Palette, mode: FrameMode, reveal: (f32, f32)) -> Rect {
    let area = ui.available_rect_before_wrap();
    if app.active_file.is_empty() {
        if app.workspace.is_none() {
            states::welcome(app, ui, p);
        } else {
            states::idle(app, ui, p);
        }
        return area;
    }
    if mode.source == SourceMode::Replace {
        let margin = inset_margin(token::INSET + PILL_RESERVE);
        card(ui, p, margin, |ui| source::show_source(app, ui, p));
        return area;
    }
    if app.outline_open {
        let margin = Margin {
            left: 0,
            right: side_gap(reveal.1),
            ..inset_margin(token::INSET)
        };
        egui::SidePanel::right("diagram-outline")
            .exact_width(240.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::NONE)
            .show_inside(ui, |ui| card(ui, p, margin, |ui| states::outline(app, ui, p)));
    }
    // The outline card, when open, sits against the content card's right.
    let margin = Margin {
        left: side_gap(reveal.0),
        right: if app.outline_open { token::INSET as i8 } else { side_gap(reveal.1) },
        top: 0,
        bottom: token::INSET as i8,
    };
    theme::content_card(ui, p, margin, |ui| {
        if app.document_view() {
            app.show_document(ui, p);
        } else {
            canvas_view(app, ui, p);
        }
    })
    .0
}

/// The diagram of the active block, or a card or empty state when there is nothing to draw.
fn canvas_view(app: &mut TidemapApp, ui: &mut Ui, p: Palette) {
    app.canvas.show_minimap = app.settings.show_minimap;
    let key = app.diagram_key();
    let artifact = app.previews.get(&key).map(|preview| preview.artifact.clone());
    if app.block().is_none() {
        states::no_diagrams(app, ui, p);
        return;
    }
    app.update_search_hits(artifact.as_ref());
    if let Some((title, body, action)) = app.canvas_message(artifact.as_ref(), &key) {
        app.card_frames = app.card_frames.saturating_add(1);
        let label = match action {
            CardAction::OpenSource => "Open source",
            CardAction::Retry => "Try again",
        };
        if canvas::paint_card(ui, &app.style, app.dark, p, &title, &body, label) {
            match action {
                CardAction::OpenSource => app.show_source(),
                CardAction::Retry => app.schedule_render(true),
            }
        }
        return;
    }
    app.card_frames = 0;
    let response = app.canvas.show(ui, artifact.as_ref(), &app.style, app.dark, p);
    if response.camera_changed {
        app.state.views.entry(key).or_default().camera = Some(app.canvas.camera);
        app.record_state();
    }
    let span_of = |item: Option<ItemId>| {
        artifact.as_ref().and_then(|artifact| {
            let scene = &artifact.rendered.scene;
            match item {
                Some(ItemId::Node(node)) => scene.nodes.get(node.0 as usize).map(|item| item.span),
                Some(ItemId::Edge(edge)) => scene.edges.get(edge.0 as usize).map(|item| item.span),
                _ => None,
            }
        })
    };
    if let Some(span) = span_of(response.activated) {
        // Enter or a double click: open the source at this item.
        app.show_source();
        app.select_in_source(span);
    } else if let Some(span) = span_of(response.clicked) {
        app.select_in_source(span);
    }
}
