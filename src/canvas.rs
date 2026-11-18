//! Native diagram viewport: camera, input and selection. Painting lives in `paint`.

use std::sync::Arc;
use std::time::Instant;

use egui::{Align2, Color32, FontId, Pos2, Sense, Stroke, StrokeKind, Vec2};
use tidemap_diagram::{EdgeId, Endpoint, ItemId, NodeId, Rect, Scene, StyleSheet};
use tidemap_model::{Camera, RenderArtifact};

use crate::motion;
use crate::paint::{self, Emphasis, Lod};
use crate::theme::{self, Palette, token};

/// What the canvas reports back to the app after a frame.
#[derive(Default)]
pub struct CanvasResponse {
    pub camera_changed: bool,
    /// Item under a click (or picked with Tab), if any.
    pub clicked: Option<ItemId>,
    /// Item the user asked to open in the source (Enter or a double click on a node).
    pub activated: Option<ItemId>,
    /// The selection was cleared by a click on empty canvas or by Escape.
    pub cleared: bool,
}

/// The last emphasis, kept so the dimming can ease out after the selection is dropped.
#[derive(Default)]
struct Fade {
    selected: Option<NodeId>,
    edges: Vec<EdgeId>,
    nodes: Vec<NodeId>,
}

struct CameraAnim {
    from: Camera,
    to: Camera,
    started: Instant,
}

const MIN_LABEL_ZOOM: f32 = token::FONT_MONO / token::FONT_UI;
const MAX_ZOOM: f32 = 16.0;
const MIN_ZOOM: f32 = 0.000001;
/// Click tolerance in screen pixels.
const HIT_SLOP_PX: f32 = 6.0;

/// Space kept free at the bottom of the viewport for the floating toolbar.
const FIT_BOTTOM: f32 = 84.0;
/// Space kept free on the other three sides when fitting.
const FIT_MARGIN: f32 = 24.0;

/// Camera that shows all of `bounds` inside a viewport of `viewport` size.
///
/// The diagram stays clear of the floating toolbar (84 px at the bottom, 24 px elsewhere).
/// Text stays readable: a huge diagram fits to width (and is panned vertically)
/// instead of shrinking below `min_zoom`. Small diagrams are not magnified past 1.6.
pub fn fit_camera(bounds: Rect, viewport: Vec2, min_zoom: f32) -> Camera {
    let available_w = (viewport.x - 2.0 * FIT_MARGIN).max(20.0);
    let available_h = (viewport.y - FIT_MARGIN - FIT_BOTTOM).max(20.0);
    let width_zoom = available_w / bounds.w.max(1.0);
    let height_zoom = available_h / bounds.h.max(1.0);
    let raw = width_zoom.min(height_zoom);
    let zoom = if raw < min_zoom {
        // Too large to show whole at a readable size: fit the width when that
        // is readable, but never magnify a tall, narrow diagram past 100%.
        width_zoom.clamp(min_zoom, 1.0)
    } else {
        raw.clamp(min_zoom, 1.6)
    };
    // A diagram taller than the free area starts just below the top edge instead of its middle.
    let y = if bounds.h * zoom > available_h {
        FIT_MARGIN - bounds.y * zoom
    } else {
        FIT_MARGIN + (available_h - bounds.h * zoom) * 0.5 - bounds.y * zoom
    };
    // A diagram wider than the free area starts at the left margin instead of being centred
    // (and cut off on both sides).
    let x = if bounds.w * zoom > available_w {
        FIT_MARGIN - bounds.x * zoom
    } else {
        (viewport.x - bounds.w * zoom) * 0.5 - bounds.x * zoom
    };
    Camera { zoom, x, y }
}

pub struct DiagramCanvas {
    pub camera: Camera,
    pub selected: Option<NodeId>,
    /// Edge picked by a click; it and its two end nodes stand out.
    pub selected_edge: Option<EdgeId>,
    /// Nodes matching the diagram search; ringed on the canvas.
    pub matches: Vec<NodeId>,
    pub show_minimap: bool,
    /// Where the minimap was painted last frame, in screen coordinates.
    pub minimap_rect: Option<egui::Rect>,
    artifact_id: String,
    diagram_id: String,
    /// The artifact painted last, to carry the selection over to a new layout.
    shown: Option<Arc<RenderArtifact>>,
    bounds: Option<Rect>,
    size: Vec2,
    restore_pending: bool,
    fit_pending: bool,
    anim: Option<CameraAnim>,
    scratch: Vec<ItemId>,
    /// Consecutive frames in which the current artifact was painted.
    pub painted_frames: u32,
    /// What the last frame painted, for the diagnostics dialog.
    pub last_stats: Option<paint::PaintStats>,
    fade: Fade,
    /// A diagram fading in, or dimming easing, was still moving last frame.
    settling: bool,
    /// Glide velocity (screen points per second) after a fast drag.
    fling: Option<Vec2>,
    /// Smoothed drag velocity, measured while dragging.
    drag_velocity: Vec2,
    /// The camera is still where the last fit put it (no pan or zoom since).
    follow_fit: bool,
}

/// How quickly a fling slows down (per second, exponential).
const FLING_DECAY: f32 = 5.5;
/// Release speed below which a drag simply stops.
const FLING_MIN_SPEED: f32 = 240.0;

impl Default for DiagramCanvas {
    fn default() -> Self {
        Self {
            camera: Camera::default(),
            selected: None,
            selected_edge: None,
            matches: Vec::new(),
            show_minimap: false,
            minimap_rect: None,
            artifact_id: String::new(),
            diagram_id: String::new(),
            shown: None,
            bounds: None,
            size: Vec2::ZERO,
            restore_pending: false,
            fit_pending: true,
            anim: None,
            scratch: Vec::new(),
            painted_frames: 0,
            last_stats: None,
            fade: Fade::default(),
            settling: false,
            fling: None,
            drag_velocity: Vec2::ZERO,
            follow_fit: false,
        }
    }
}

const APPEAR_ID: &str = "canvas-appear";
const DIM_ID: &str = "canvas-dim";

impl DiagramCanvas {
    pub fn restore_camera(&mut self, camera: Option<Camera>) {
        self.anim = None;
        self.fling = None;
        self.follow_fit = false;
        self.restore_pending = camera.is_some();
        self.fit_pending = camera.is_none();
        if let Some(camera) = camera
            .filter(|c| c.x.is_finite() && c.y.is_finite() && c.zoom.is_finite() && c.zoom > 0.0)
        {
            self.camera = Camera {
                zoom: camera.zoom.clamp(MIN_ZOOM, MAX_ZOOM),
                ..camera
            };
        }
        self.selected = None;
        self.selected_edge = None;
        self.matches.clear();
    }

    fn animate_to(&mut self, to: Camera) {
        self.fling = None;
        self.follow_fit = false;
        let from = self.camera;
        if (to.x - from.x).abs() < 0.1
            && (to.y - from.y).abs() < 0.1
            && (to.zoom - from.zoom).abs() < 0.001
        {
            self.anim = None;
            return;
        }
        self.anim = Some(CameraAnim {
            from,
            to,
            started: Instant::now(),
        });
    }

    fn tick_anim(&mut self) {
        let Some(anim) = &self.anim else { return };
        let t = (anim.started.elapsed().as_secs_f32() / motion::CAMERA).min(1.0);
        let eased = motion::ease_out(t);
        let (from, to) = (anim.from, anim.to);
        self.camera = Camera {
            x: from.x + (to.x - from.x) * eased,
            y: from.y + (to.y - from.y) * eased,
            zoom: from.zoom * (to.zoom / from.zoom).powf(eased),
        };
        if t >= 1.0 {
            self.anim = None;
        }
    }

    pub fn is_animating(&self) -> bool {
        self.anim.is_some() || self.settling || self.fling.is_some()
    }

    /// Selects the next (or previous) node in reading order and brings it into view.
    fn step_selection(&mut self, scene: &Scene, forward: bool) -> Option<NodeId> {
        let count = scene.nodes.len() as u32;
        if count == 0 {
            return None;
        }
        let next = match self.selected {
            Some(NodeId(at)) if forward => (at + 1) % count,
            Some(NodeId(at)) => (at + count - 1) % count,
            None if forward => 0,
            None => count - 1,
        };
        let id = NodeId(next);
        self.selected_edge = None;
        self.focus_node(scene, id);
        Some(id)
    }

    /// Drops the node and edge selection; true when there was one.
    pub fn clear_selection(&mut self) -> bool {
        let had = self.selected.is_some() || self.selected_edge.is_some();
        self.selected = None;
        self.selected_edge = None;
        had
    }

    pub fn fit(&mut self) {
        let Some(bounds) = self.bounds else {
            self.fit_pending = true;
            return;
        };
        if self.size.x < 2.0 || self.size.y < 2.0 {
            self.fit_pending = true;
            return;
        }
        self.animate_to(fit_camera(bounds, self.size, MIN_LABEL_ZOOM));
        self.fit_pending = false;
        self.follow_fit = true;
    }

    pub fn actual_size(&mut self) {
        self.zoom_to(1.0, self.size * 0.5, true);
    }

    pub fn zoom_by(&mut self, factor: f32) {
        self.zoom_to(self.target().zoom * factor, self.size * 0.5, true);
    }

    /// Advances a fling (the glide after a fast drag) by one frame.
    fn tick_fling(&mut self, dt: f32) {
        let Some(velocity) = self.fling else { return };
        let dt = dt.clamp(0.0, 0.05);
        self.camera.x += velocity.x * dt;
        self.camera.y += velocity.y * dt;
        let velocity = velocity * (-dt * FLING_DECAY).exp();
        self.fling = (velocity.length() > 12.0).then_some(velocity);
    }

    /// Centres and zooms on node `n` and selects it.
    pub fn focus_node(&mut self, scene: &Scene, n: NodeId) {
        let Some(node) = scene.nodes.get(n.0 as usize) else {
            return;
        };
        let r = node.rect;
        let zoom = ((self.size.x - 160.0) / r.w.max(80.0))
            .min((self.size.y - 160.0) / r.h.max(60.0))
            .clamp(MIN_LABEL_ZOOM, 2.0);
        let [cx, cy] = r.center();
        self.animate_to(Camera {
            zoom,
            x: self.size.x * 0.5 - cx * zoom,
            y: self.size.y * 0.5 - cy * zoom,
        });
        self.selected = Some(n);
    }

    /// Where the camera is heading: the end of a running animation, or where it is now.
    fn target(&self) -> Camera {
        self.anim.as_ref().map_or(self.camera, |anim| anim.to)
    }

    fn zoom_to(&mut self, zoom: f32, anchor: Vec2, animate: bool) {
        if !zoom.is_finite() {
            return;
        }
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        if animate {
            // Steps pressed in quick succession compound from where the camera is heading.
            let base = self.target();
            let ratio = zoom / base.zoom;
            self.animate_to(Camera {
                x: anchor.x - (anchor.x - base.x) * ratio,
                y: anchor.y - (anchor.y - base.y) * ratio,
                zoom,
            });
        } else {
            self.zoom_at(zoom, anchor);
        }
    }

    fn zoom_at(&mut self, zoom: f32, anchor: Vec2) {
        if !zoom.is_finite() {
            return;
        }
        self.anim = None;
        self.fling = None;
        self.follow_fit = false;
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let ratio = zoom / self.camera.zoom;
        self.camera.x = anchor.x - (anchor.x - self.camera.x) * ratio;
        self.camera.y = anchor.y - (anchor.y - self.camera.y) * ratio;
        self.camera.zoom = zoom;
    }

    /// World coordinates under a screen position: `(screen - viewport.min - camera.xy) / zoom`.
    fn world_point(&self, viewport: egui::Rect, screen: Pos2) -> [f32; 2] {
        [
            (screen.x - viewport.min.x - self.camera.x) / self.camera.zoom,
            (screen.y - viewport.min.y - self.camera.y) / self.camera.zoom,
        ]
    }

    fn visible_world(&self) -> Rect {
        Rect {
            x: -self.camera.x / self.camera.zoom,
            y: -self.camera.y / self.camera.zoom,
            w: self.size.x / self.camera.zoom,
            h: self.size.y / self.camera.zoom,
        }
    }

    /// A new layout of the same diagram: the selected node (found again by its key) stays
    /// selected and keeps its place on screen; otherwise the camera is left untouched, which
    /// keeps the world point under the viewport centre.
    fn carry_over(&mut self, old: &Scene, new: &Scene) {
        self.selected_edge = None;
        let Some(node) = self.selected.take() else {
            return;
        };
        if node.0 as usize >= old.nodes.len() {
            return;
        }
        let key = old.node_key(node);
        let Some(found) = (0..new.nodes.len() as u32)
            .map(NodeId)
            .find(|id| new.node_key(*id) == key)
        else {
            return;
        };
        self.selected = Some(found);
        let before = old.nodes[node.0 as usize].rect.center();
        let after = new.nodes[found.0 as usize].rect.center();
        self.camera.x += (before[0] - after[0]) * self.camera.zoom;
        self.camera.y += (before[1] - after[1]) * self.camera.zoom;
    }

    /// Shows the diagram. `artifact` is `None` when nothing has been drawn yet.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        artifact: Option<&Arc<RenderArtifact>>,
        style: &StyleSheet,
        dark: bool,
        p: Palette,
    ) -> CanvasResponse {
        self.tick_anim();
        let dt = ui.input(|i| i.stable_dt);
        self.tick_fling(dt);
        if self.anim.is_some() || self.fling.is_some() {
            ui.ctx().request_repaint();
        }
        let old_camera = self.camera;
        let mut result = CanvasResponse::default();
        let (viewport, response) = ui.allocate_exact_size(
            ui.available_size().max(Vec2::splat(1.0)),
            Sense::click_and_drag(),
        );
        let painter = ui.painter_at(viewport);
        let old_size = self.size;
        self.size = viewport.size();
        if old_size.x > 1.0 && old_size.y > 1.0 && old_size != self.size {
            // Keep the view centred. A camera move in flight (a fit while a panel slides in)
            // is carried along rather than cancelled, or it would stop halfway.
            let shift = (self.size - old_size) * 0.5;
            let fitted = self.follow_fit.then_some(()).and(self.bounds).map(|bounds| fit_camera(bounds, self.size, MIN_LABEL_ZOOM));
            match (fitted, &mut self.anim) {
                // Untouched since the last fit: stay fitted, so the diagram reflows smoothly
                // while a panel slides or the window is resized.
                (Some(target), Some(anim)) => anim.to = target,
                (Some(target), None) => self.camera = target,
                (None, anim) => {
                    self.camera.x += shift.x;
                    self.camera.y += shift.y;
                    if let Some(anim) = anim {
                        for camera in [&mut anim.from, &mut anim.to] {
                            camera.x += shift.x;
                            camera.y += shift.y;
                        }
                    }
                }
            }
        }
        let colors = style.colors(dark);
        let background = Color32::from_rgba_unmultiplied(
            colors.canvas[0],
            colors.canvas[1],
            colors.canvas[2],
            colors.canvas[3],
        );
        painter.rect_filled(viewport, 0.0, background);
        self.minimap_rect = None;
        let Some(artifact) = artifact else {
            self.painted_frames = 0;
            painter.text(
                viewport.center(),
                Align2::CENTER_CENTER,
                "Your diagram appears here",
                FontId::proportional(token::FONT_UI),
                p.secondary,
            );
            return result;
        };
        let scene = &artifact.rendered.scene;

        if self.artifact_id != artifact.id {
            let different_diagram = self.diagram_id != artifact.diagram_id;
            self.artifact_id.clone_from(&artifact.id);
            self.diagram_id.clone_from(&artifact.diagram_id);
            self.bounds = Some(scene.bounds);
            self.painted_frames = 0;
            let previous = self.shown.take();
            if different_diagram {
                self.selected = None;
                self.selected_edge = None;
                self.fade = Fade::default();
                motion::restart(ui.ctx(), egui::Id::new(DIM_ID));
                // A different diagram fades and rises in instead of popping.
                motion::restart(ui.ctx(), egui::Id::new(APPEAR_ID));
                if !self.restore_pending {
                    self.fit_pending = true;
                }
            } else if let Some(previous) = previous {
                self.carry_over(&previous.rendered.scene, scene);
            }
            self.restore_pending = false;
        }
        self.shown = Some(artifact.clone());
        if self.fit_pending {
            self.fit();
        }

        let auto_minimap =
            scene.nodes.len() > 36 || scene.bounds.w > 3200.0 || scene.bounds.h > 2400.0;
        let minimap_rect = (self.show_minimap || auto_minimap).then(|| {
            egui::Rect::from_min_size(
                viewport.right_bottom() - Vec2::new(188.0, 130.0),
                Vec2::new(172.0, 112.0),
            )
        });
        self.minimap_rect = minimap_rect;
        let pointer_in_minimap = minimap_rect.is_some_and(|rect| {
            ui.input(|i| i.pointer.hover_pos().is_some_and(|pos| rect.contains(pos)))
        });

        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        if response.drag_started() {
            self.fling = None;
            self.drag_velocity = Vec2::ZERO;
        }
        if response.dragged() && !pointer_in_minimap {
            self.anim = None;
            self.fling = None;
            self.follow_fit = false;
            let delta = response.drag_delta();
            self.camera.x += delta.x;
            self.camera.y += delta.y;
            if dt > 0.0 {
                // Exponentially smoothed so one jittery frame does not decide the glide.
                self.drag_velocity = self.drag_velocity * 0.6 + (delta / dt) * 0.4;
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if response.drag_stopped() {
            let still = ui.input(|i| i.pointer.time_since_last_movement()) > 0.06;
            if !still && self.drag_velocity.length() > FLING_MIN_SPEED {
                self.fling = Some(self.drag_velocity);
                ui.ctx().request_repaint();
            }
            self.drag_velocity = Vec2::ZERO;
        } else if response.hovered() && !pointer_in_minimap {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        if response.hovered() && !pointer_in_minimap {
            let (factor, scroll, modifiers, pointer) = ui.input(|i| {
                (
                    i.zoom_delta(),
                    i.smooth_scroll_delta,
                    i.modifiers,
                    i.pointer.hover_pos(),
                )
            });
            let anchor = pointer.map_or(viewport.size() * 0.5, |pos| pos - viewport.min);
            // Only real wheel or pinch input takes over from a running camera animation;
            // merely hovering must not freeze a fit or zoom that is gliding.
            if (factor - 1.0).abs() > 0.0001 {
                self.zoom_at(self.camera.zoom * factor, anchor);
            } else if scroll != Vec2::ZERO {
                self.anim = None;
                self.fling = None;
                self.follow_fit = false;
                if modifiers.ctrl || modifiers.command {
                    if scroll.y != 0.0 {
                        self.zoom_at(self.camera.zoom * (scroll.y * 0.004).exp(), anchor);
                    }
                } else {
                    self.camera.x += scroll.x;
                    self.camera.y += scroll.y;
                }
            }
            ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
        if response.has_focus() {
            // Keep arrows, Tab and Esc on the canvas; egui would otherwise move focus away
            // on the first press and every following key would go nowhere.
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    response.id,
                    egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true },
                )
            });
            let mut escape = false;
            let (mut tab, mut enter) = (None, false);
            let mut pan = Vec2::ZERO;
            ui.input_mut(|i| {
                escape = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
                if i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab) {
                    tab = Some(false);
                } else if i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
                    tab = Some(true);
                }
                enter = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                let step = if i.modifiers.shift { 240.0 } else { 64.0 };
                for (key, delta) in [
                    (egui::Key::ArrowLeft, Vec2::new(step, 0.0)),
                    (egui::Key::ArrowRight, Vec2::new(-step, 0.0)),
                    (egui::Key::ArrowUp, Vec2::new(0.0, step)),
                    (egui::Key::ArrowDown, Vec2::new(0.0, -step)),
                ] {
                    if i.key_pressed(key) && !i.modifiers.alt && !i.modifiers.command {
                        pan += delta;
                    }
                }
            });
            if pan != Vec2::ZERO {
                // Arrow panning glides instead of jumping.
                let base = self.anim.as_ref().map_or(self.camera, |anim| anim.to);
                self.animate_to(Camera { x: base.x + pan.x, y: base.y + pan.y, zoom: base.zoom });
            }
            if let Some(forward) = tab
                && let Some(node) = self.step_selection(scene, forward)
            {
                result.clicked = Some(ItemId::Node(node));
            }
            if enter {
                if let Some(node) = self.selected {
                    result.activated = Some(ItemId::Node(node));
                } else if let Some(edge) = self.selected_edge {
                    result.activated = Some(ItemId::Edge(edge));
                }
            }
            if escape && self.clear_selection() {
                result.cleared = true;
            }
        }

        if !pointer_in_minimap
            && (response.clicked() || response.double_clicked())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let hit = scene.hit(
                self.world_point(viewport, pointer),
                HIT_SLOP_PX / self.camera.zoom,
            );
            match hit {
                Some(ItemId::Node(n)) => {
                    self.selected = Some(n);
                    self.selected_edge = None;
                    result.clicked = hit;
                    // Double-clicking a node opens its line in the source.
                    if response.double_clicked() {
                        result.activated = hit;
                    }
                }
                Some(ItemId::Edge(e)) => {
                    self.selected = None;
                    self.selected_edge = Some(e);
                    result.clicked = hit;
                }
                other => {
                    result.clicked = other;
                    if self.clear_selection() || other.is_none() {
                        result.cleared = true;
                    }
                    // Double-clicking the background (or a container) fits the diagram.
                    if response.double_clicked() {
                        self.fit();
                    }
                }
            }
        }

        let edge_only;
        let edge_ends;
        let (strong_edges, strong_nodes) = match (self.selected, self.selected_edge) {
            (Some(n), _) => scene.neighbours(n),
            (None, Some(e)) if scene.edges.get(e.0 as usize).is_some() => {
                let item = &scene.edges[e.0 as usize];
                edge_only = [e];
                edge_ends = [item.from, item.to]
                    .into_iter()
                    .filter_map(|end| match end {
                        Endpoint::Node(n) => Some(n),
                        Endpoint::Cluster(_) => None,
                    })
                    .collect::<Vec<_>>();
                (&edge_only[..], &edge_ends[..])
            }
            _ => (&[][..], &[][..]),
        };
        // Dimming eases in with a selection and out after it, painting the last emphasis
        // while it fades.
        let active = self.selected.is_some() || self.selected_edge.is_some();
        let dim = motion::toggle(ui.ctx(), egui::Id::new(DIM_ID), active, motion::BASE);
        if active {
            self.fade = Fade { selected: self.selected, edges: strong_edges.to_vec(), nodes: strong_nodes.to_vec() };
        } else if dim <= 0.0 {
            self.fade = Fade::default();
        }
        let emphasis = if active {
            Emphasis { selected: self.selected, strong_nodes, strong_edges, matches: &self.matches, dim }
        } else {
            Emphasis {
                selected: self.fade.selected,
                strong_nodes: &self.fade.nodes,
                strong_edges: &self.fade.edges,
                matches: &self.matches,
                dim,
            }
        };
        let appear = motion::appear(ui.ctx(), egui::Id::new(APPEAR_ID), true, motion::BASE + 0.04);
        self.settling = appear < 1.0 || motion::moving(ui.ctx(), egui::Id::new(DIM_ID));
        let mut scene_painter = painter.clone();
        scene_painter.multiply_opacity(appear);
        let camera = Camera { y: self.camera.y + (1.0 - appear) * 10.0, ..self.camera };
        let stats = paint::paint_scene(
            &scene_painter,
            viewport,
            camera,
            scene,
            style,
            dark,
            p.accent,
            &emphasis,
            &mut self.scratch,
        );
        self.painted_frames = self.painted_frames.saturating_add(1);
        self.last_stats = Some(stats);
        // A quiet pill fades in while labels are hidden, above the zoom pill.
        let hint = motion::toggle(ui.ctx(), egui::Id::new("canvas-lod-hint"), stats.lod != Lod::Full, motion::BASE);
        if hint > 0.0 {
            let galley = painter.layout_no_wrap(
                "Zoom in for labels".to_owned(),
                FontId::proportional(token::FONT_SMALL),
                p.secondary.gamma_multiply(hint),
            );
            let size = galley.size() + Vec2::new(20.0, 10.0);
            let rise = 6.0 * (1.0 - hint);
            let bottom = viewport.center_bottom() - Vec2::new(0.0, 76.0 - rise);
            let pill = egui::Rect::from_center_size(bottom - Vec2::new(0.0, size.y * 0.5), size);
            painter.rect_filled(pill, size.y * 0.5, p.field.gamma_multiply(0.92 * hint));
            painter.galley(pill.center() - galley.size() * 0.5, galley, p.secondary);
        }
        if let Some(rect) = minimap_rect {
            self.paint_minimap(ui, &painter, viewport, rect, scene, style, dark, p);
        }
        result.camera_changed = self.camera.x != old_camera.x
            || self.camera.y != old_camera.y
            || self.camera.zoom != old_camera.zoom;
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_minimap(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        viewport: egui::Rect,
        panel: egui::Rect,
        scene: &Scene,
        style: &StyleSheet,
        dark: bool,
        p: Palette,
    ) {
        let painter = painter.with_clip_rect(panel.intersect(viewport));
        painter.rect_filled(panel, token::RADIUS_PANEL, p.panel);
        let bounds = scene.bounds;
        let scale = ((panel.width() - 14.0) / bounds.w.max(1.0))
            .min((panel.height() - 14.0) / bounds.h.max(1.0));
        let drawing =
            egui::Rect::from_center_size(panel.center(), Vec2::new(bounds.w, bounds.h) * scale);
        let to_panel = |r: Rect| {
            egui::Rect::from_min_size(
                drawing.min + Vec2::new(r.x - bounds.x, r.y - bounds.y) * scale,
                Vec2::new(r.w, r.h) * scale,
            )
        };
        let colors = style.colors(dark);
        let node_color =
            Color32::from_rgba_unmultiplied(colors.edge[0], colors.edge[1], colors.edge[2], 200);
        // Large diagrams are sampled so the minimap costs the same for any size.
        let stride = (scene.nodes.len() / 1500).max(1);
        for node in scene.nodes.iter().step_by(stride) {
            painter.rect_filled(to_panel(node.rect), 0.5, node_color);
        }
        let view_rect = to_panel(self.visible_world()).intersect(drawing);
        painter.rect_filled(
            view_rect,
            token::RADIUS_CONTROL / 2.0,
            p.accent.gamma_multiply(0.10),
        );
        painter.rect_stroke(
            view_rect,
            token::RADIUS_CONTROL / 2.0,
            Stroke::new(1.3_f32, p.accent),
            StrokeKind::Inside,
        );
        let interaction = ui.interact(
            panel,
            ui.id().with("diagram-minimap"),
            Sense::click_and_drag(),
        );
        interaction
            .clone()
            .on_hover_text("Click or drag to move the viewport");
        if (interaction.clicked() || interaction.dragged())
            && let Some(position) = interaction.interact_pointer_pos()
        {
            let world_x = bounds.x + (position.x - drawing.left()) / scale;
            let world_y = bounds.y + (position.y - drawing.top()) / scale;
            let zoom = self.camera.zoom;
            self.animate_to(Camera {
                x: viewport.width() * 0.5 - world_x * zoom,
                y: viewport.height() * 0.5 - world_y * zoom,
                zoom,
            });
        }
    }
}

/// Fills the viewport with the canvas colour and shows a centred card: title, body and one
/// button. Returns true when the button was pressed.
pub fn paint_card(
    ui: &mut egui::Ui,
    style: &StyleSheet,
    dark: bool,
    p: Palette,
    title: &str,
    body: &str,
    button: &str,
) -> bool {
    let (viewport, _) =
        ui.allocate_exact_size(ui.available_size().max(Vec2::splat(1.0)), Sense::hover());
    let c = style.colors(dark).canvas;
    ui.painter_at(viewport).rect_filled(
        viewport,
        0.0,
        Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]),
    );
    let mut pressed = false;
    egui::Area::new(ui.id().with("diagram-card"))
        .order(egui::Order::Middle)
        .fade_in(false)
        .pivot(Align2::CENTER_CENTER)
        .fixed_pos(viewport.center())
        .show(ui.ctx(), |ui| {
            theme::floating(p, token::RADIUS_DIALOG)
                .inner_margin(token::SPACE[4])
                .show(ui, |ui| {
                    ui.set_max_width(((viewport.width() - 80.0).clamp(160.0, 400.0)).max(160.0));
                    ui.label(
                        egui::RichText::new(title)
                            .size(token::FONT_TITLE)
                            .strong()
                            .color(p.ink),
                    );
                    ui.add_space(token::SPACE[1]);
                    ui.label(
                        egui::RichText::new(body)
                            .size(token::FONT_UI)
                            .color(p.secondary),
                    );
                    ui.add_space(token::SPACE[3]);
                    pressed = ui.button(button).clicked();
                });
        });
    pressed
}
