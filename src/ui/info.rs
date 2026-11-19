//! The info popover, the status dot on its button and the toast.

use egui::{
    Align2, Area, Color32, Context, FontId, Id, LayerId, Margin, Order, Painter, Pos2, Rect, RichText, Sense, Ui,
    UiBuilder, pos2, vec2,
};
use egui::emath::TSTransform;
use tidemap_diagram::Span;
use tidemap_model::DiagnosticKind;

use super::{
    controls::{primary_button, quiet_button},
    surface,
};
use crate::{
    app::{Dialog, TidemapApp},
    icons::{self, Icon},
    motion,
    theme::{self, Palette, token},
};

/// Width of the popover content (340 minus the 16 px padding on both sides).
const CONTENT_WIDTH: f32 = 308.0;
const PADDING: i8 = 16;
/// Most problems listed before "and k more".
const MAX_PROBLEMS: usize = 5;
const DOT_SIZE: f32 = 6.0;
const TOAST_WIDTH: f32 = 420.0;

/// What the status dot on the Info button says.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dot {
    None,
    Unsaved,
    Attention,
}

/// Attention (danger colour) beats unsaved (accent); otherwise no dot.
pub fn dot(dirty: bool, saving: bool, external_change: bool, diagnostics: usize, style_error: bool) -> Dot {
    if external_change || diagnostics > 0 || style_error {
        Dot::Attention
    } else if dirty || saving {
        Dot::Unsaved
    } else {
        Dot::None
    }
}

/// The one-line save status shown at the top of the popover.
pub fn save_line(dirty: bool, saving: bool, external_change: bool, read_only: bool) -> &'static str {
    if saving {
        "Saving\u{2026}"
    } else if external_change {
        "This file changed on disk. Your edits are kept."
    } else if read_only {
        "Read-only file"
    } else if dirty {
        "Unsaved changes"
    } else {
        "Saved"
    }
}

/// The dot for the active file, as the top bar needs it.
pub fn status(app: &TidemapApp) -> Dot {
    let buffer = app.buffers.get(&app.active_file);
    dot(
        buffer.is_some_and(|buffer| buffer.dirty()),
        buffer.is_some_and(|buffer| buffer.saving.is_some()),
        buffer.is_some_and(|buffer| buffer.external.is_some()),
        app.problem_count(),
        app.style_file.error.is_some(),
    )
}

/// Paints the 6 px status dot at the top-right of the Info button.
pub fn paint_dot(painter: &Painter, button: Rect, dot: Dot, p: Palette) {
    let color = match dot {
        Dot::None => return,
        Dot::Unsaved => p.accent,
        Dot::Attention => p.danger,
    };
    let centre = button.center() + vec2(7.0, -7.0);
    // A ring in the backdrop colour keeps the dot apart from the icon.
    painter.circle_filled(centre, DOT_SIZE * 0.5 + 1.5, p.app);
    painter.circle_filled(centre, DOT_SIZE * 0.5, color);
}

/// One line in the "problems" list.
struct Problem {
    text: String,
    line: usize,
    span: Option<Span>,
}

/// What the popover asks for; applied after it is drawn.
#[derive(Default)]
struct Actions {
    save: bool,
    review: bool,
    save_copy: bool,
    cancel_render: bool,
    retry: bool,
    goto: Option<(usize, Option<Span>)>,
    outline: bool,
    shortcuts: bool,
    diagnostics: bool,
    close: bool,
}

/// Everything the popover shows, read from the app before drawing.
struct Facts {
    file_name: String,
    save_line: &'static str,
    dirty: bool,
    external: bool,
    drawing: bool,
    drawn: Option<String>,
    held: bool,
    problems: Vec<Problem>,
    can_retry: bool,
    style_error: Option<String>,
    details: Option<String>,
}

fn facts(app: &TidemapApp) -> Facts {
    let buffer = app.buffers.get(&app.active_file);
    let dirty = buffer.is_some_and(|buffer| buffer.dirty());
    let saving = buffer.is_some_and(|buffer| buffer.saving.is_some());
    let external = buffer.is_some_and(|buffer| buffer.external.is_some());
    let read_only = buffer.is_some_and(|buffer| buffer.document.read_only);
    let key = app.diagram_key();

    let mut problems = Vec::new();
    if let Some(artifact) = app.notes_artifact() {
        for item in &artifact.rendered.diagnostics {
            problems.push(Problem {
                text: format!("Line {}: {}", item.line, item.message),
                line: item.line as usize,
                span: Some(item.span),
            });
        }
    }
    let failure = app.diagnostics.get(&key);
    if let Some(failure) = failure {
        let text = match failure.line {
            Some(line) => format!("Line {line}: {}", failure.message),
            None => failure.message.clone(),
        };
        problems.push(Problem { text, line: failure.line.unwrap_or(1), span: None });
    }

    let drawn = app.active_preview().map(|preview| {
        format!(
            "Drawn in {:.1} ms · {} nodes · {} links",
            preview.artifact.duration_us as f64 / 1000.0,
            preview.artifact.rendered.scene.nodes.len(),
            preview.artifact.rendered.scene.edges.len()
        )
    });
    let details = buffer.map(|buffer| {
        let lines = buffer.document.content.split('\n').count();
        let newline = if buffer.document.newline == "\r\n" { "CRLF" } else { "LF" };
        format!("{lines} lines · UTF-8 · {newline}")
    });
    Facts {
        file_name: buffer.map(|buffer| buffer.document.name.clone()).unwrap_or_default(),
        save_line: save_line(dirty, saving, external, read_only),
        dirty,
        external,
        drawing: app.active_job.is_some() || app.pending_render.is_some(),
        drawn,
        held: app.held_for_active(),
        problems,
        can_retry: failure.is_some_and(|failure| {
            matches!(failure.kind, DiagnosticKind::Budget | DiagnosticKind::Crash)
        }),
        style_error: app.style_file.error.clone(),
        details,
    }
}

/// A line of text that reacts to hover and click; returns true when clicked.
fn clickable_text(ui: &mut Ui, p: Palette, text: &str, color: Color32) -> bool {
    let galley = ui.painter().layout(
        text.to_owned(),
        FontId::proportional(token::FONT_SMALL),
        color,
        ui.available_width(),
    );
    let size = vec2(ui.available_width(), galley.size().y + 8.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect.expand2(vec2(8.0, 0.0)), token::RADIUS_CONTROL, p.hover);
    }
    ui.painter().galley(rect.min + vec2(0.0, 4.0), galley, color);
    response.clicked()
}

fn muted(ui: &mut Ui, p: Palette, text: &str) {
    ui.label(RichText::new(text).size(token::FONT_SMALL).color(p.muted));
}

/// A small quiet text button, 13 px, for the links row.
fn link(ui: &mut Ui, p: Palette, text: &str) -> bool {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        FontId::new(token::FONT_SMALL, theme::font_medium()),
        p.secondary,
    );
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 16.0, token::CONTROL), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, token::RADIUS_CONTROL, p.hover);
    }
    ui.painter().galley(rect.center() - galley.size() * 0.5, galley, p.secondary);
    response.clicked()
}

fn contents(ui: &mut Ui, p: Palette, facts: &Facts, actions: &mut Actions) {
    ui.set_width(CONTENT_WIDTH);
    ui.spacing_mut().item_spacing.y = 0.0;

    // File.
    egui::containers::Sides::new().shrink_left().truncate().show(
        ui,
        |ui| {
            ui.label(
                RichText::new(&facts.file_name)
                    .font(FontId::new(token::FONT_UI, theme::font_medium()))
                    .color(p.ink),
            );
        },
        |ui| {
            if icons::button(ui, Icon::Close, "Close", false, true).clicked() {
                actions.close = true;
            }
        },
    );
    ui.label(RichText::new(facts.save_line).size(token::FONT_UI).color(p.secondary));
    if facts.dirty || facts.external {
        ui.add_space(token::SPACE[1]);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = token::SPACE[1];
            if facts.external {
                if quiet_button(ui, "Review changes").clicked() {
                    actions.review = true;
                }
                if quiet_button(ui, "Save a copy").clicked() {
                    actions.save_copy = true;
                }
            } else if primary_button(ui, "Save").clicked() {
                actions.save = true;
            }
        });
    }

    // Diagram.
    ui.add_space(token::SPACE[2]);
    if facts.drawing {
        ui.horizontal(|ui| {
            muted(ui, p, "Drawing\u{2026}");
            if link(ui, p, "Cancel") {
                actions.cancel_render = true;
            }
        });
    } else if let Some(drawn) = &facts.drawn {
        ui.label(RichText::new(drawn).size(token::FONT_SMALL).color(p.secondary));
    }
    if facts.held {
        muted(ui, p, "Showing the last version that drew cleanly.");
    }

    // Problems.
    if !facts.problems.is_empty() || facts.style_error.is_some() {
        ui.add_space(token::SPACE[2]);
        for problem in facts.problems.iter().take(MAX_PROBLEMS) {
            if clickable_text(ui, p, &problem.text, p.danger) {
                actions.goto = Some((problem.line, problem.span));
            }
        }
        let more = facts.problems.len().saturating_sub(MAX_PROBLEMS);
        if more > 0 {
            ui.label(RichText::new(format!("and {more} more")).size(token::FONT_SMALL).color(p.muted));
        }
        if facts.can_retry && link(ui, p, "Try again") {
            actions.retry = true;
        }
        if let Some(error) = &facts.style_error {
            ui.add(egui::Label::new(RichText::new(error).size(token::FONT_SMALL).color(p.danger)).wrap());
        }
    }

    // Details and links.
    if let Some(details) = &facts.details {
        ui.add_space(token::SPACE[2]);
        muted(ui, p, details);
    }
    ui.add_space(token::SPACE[2]);
    // The links carry 8 px of padding; shift them left so their text lines up with the rest.
    let mut rect = ui.available_rect_before_wrap();
    rect.min.x -= token::SPACE[1];
    ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if link(ui, p, "Outline") {
                actions.outline = true;
            }
            if link(ui, p, "Keyboard shortcuts") {
                actions.shortcuts = true;
            }
            if link(ui, p, "Diagnostics") {
                actions.diagnostics = true;
            }
        });
    });
}

fn apply(app: &mut TidemapApp, actions: Actions) {
    let id = app.active_file.clone();
    if actions.save {
        app.save(id.clone(), false);
    }
    if actions.review {
        app.conflict_file = Some(id.clone());
        app.info_open = false;
    }
    if actions.save_copy {
        app.save(id, true);
    }
    if actions.cancel_render {
        app.cancel_render();
    }
    if actions.retry {
        app.schedule_render(true);
    }
    if let Some((line, span)) = actions.goto {
        app.show_problem(line, span);
    }
    if actions.outline {
        app.outline_open = !app.outline_open;
        app.info_open = false;
    }
    if actions.shortcuts {
        app.dialog = Some(Dialog::Help);
    }
    if actions.diagnostics {
        app.open_diagnostics();
    }
    if actions.close {
        app.info_open = false;
    }
}

/// The popover under the Info button. `anchor` is that button's rectangle. It unfolds from the
/// button's bottom-right corner and folds back into it.
pub fn show(app: &mut TidemapApp, ctx: &Context, anchor: Rect, p: Palette) {
    let id = Id::new("info-popover");
    let layer = LayerId::new(Order::Foreground, id);
    let open = app.info_open && app.dialog.is_none() && !app.palette_open;
    let Some(t) = motion::presence(ctx, id.with("presence"), open) else {
        // Gone: drop the layer transform so the next opening starts clean.
        surface::animate_layer(ctx, layer, 1.0, Pos2::ZERO, surface::POPOVER);
        return;
    };
    let shown_t = if open { t } else { surface::exit_t(t) };
    surface::animate_layer(ctx, layer, shown_t, anchor.right_bottom(), surface::POPOVER);
    if !open {
        surface::release_focus(ctx, layer);
    }
    let facts = facts(app);
    let mut actions = Actions::default();
    let area = Area::new(id)
        .order(Order::Foreground)
        .fade_in(false)
        .pivot(Align2::RIGHT_TOP)
        .fixed_pos(pos2(ctx.content_rect().right() - token::INSET, anchor.bottom() + token::SPACE[0]))
        .interactable(open)
        .show(ctx, |ui| {
            ui.multiply_opacity(motion::fade(t));
            theme::elevated(ui, p, theme::Level::Overlay, token::RADIUS_PANEL, Margin::same(PADDING), |ui| {
                contents(ui, p, &facts, &mut actions)
            });
        });
    if !open {
        return;
    }
    let clicked_outside = ctx.input(|input| {
        input.pointer.any_click()
            && input
                .pointer
                .interact_pos()
                .is_some_and(|pos| !area.response.rect.contains(pos) && !anchor.contains(pos))
    });
    if clicked_outside {
        app.info_open = false;
    }
    apply(app, actions);
}

/// How far a leaving toast sinks while it fades.
const TOAST_SINK: f32 = 6.0;

/// The toast that is on screen (or leaving): its text and when it was shown.
type ToastMemo = (String, std::time::Instant);

/// The toast for one-off results, at the bottom-left of the canvas. It grows from its corner
/// and, once its time is up, fades while sinking a few pixels instead of vanishing.
pub fn toast(app: &TidemapApp, ctx: &Context, canvas: Rect, p: Palette) {
    let memo_key = Id::new("toast-memo");
    let presence_key = Id::new("toast-presence");
    // The app drops the toast when its time is up; keep the last one to draw it leaving.
    let current = app.toast.clone();
    let remembered = ctx.data(|data| data.get_temp::<ToastMemo>(memo_key));
    let shown = current.clone().or_else(|| remembered.clone());
    if let Some(memo) = &current {
        if remembered.as_ref().is_none_or(|(_, previous)| *previous != memo.1) {
            // A new toast: enter again from the start.
            motion::restart(ctx, presence_key);
        }
        ctx.data_mut(|data| data.insert_temp(memo_key, memo.clone()));
    }
    let open = current.is_some();
    let layer = surface::toast_layer();
    let Some(t) = motion::presence(ctx, presence_key, open) else {
        ctx.set_transform_layer(layer, surface::pushed(ctx));
        return;
    };
    let Some((text, _)) = shown else {
        return;
    };
    let origin = canvas.left_bottom() + vec2(token::SPACE[3], -token::SPACE[3]);
    // Entering: grow from the corner. Leaving: no scale, just sink a little.
    let own = if open {
        surface::transform(t, origin, surface::TOAST)
    } else {
        TSTransform::from_translation(vec2(0.0, TOAST_SINK * (1.0 - t)))
    };
    ctx.set_transform_layer(layer, surface::pushed(ctx) * own);
    Area::new(Id::new("toast"))
        .order(Order::Tooltip)
        .fade_in(false)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(origin)
        .interactable(false)
        .show(ctx, |ui| {
            ui.multiply_opacity(motion::fade(t));
            let margin = Margin::symmetric(PADDING, token::SPACE[2] as i8);
            theme::elevated(ui, p, theme::Level::Raised, token::RADIUS_PANEL, margin, |ui| {
                ui.set_max_width(TOAST_WIDTH);
                ui.add(egui::Label::new(RichText::new(&text).size(token::FONT_UI).color(p.ink)).wrap());
            });
        });
    if let Some((_, since)) = &app.toast {
        ctx.request_repaint_after(crate::app::TOAST_LIFETIME.saturating_sub(since.elapsed()));
    }
}
