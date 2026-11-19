//! Native diagram viewport: camera, input and selection. Painting lives in `paint`.

use std::sync::Arc;

use egui::{Align2, Color32, FontId, Pos2, Sense, Stroke, StrokeKind, Vec2};
use tidemap_diagram::{EdgeId, Endpoint, ItemId, NodeId, Rect, Scene, StyleSheet};
use tidemap_model::{Camera, RenderArtifact};

use crate::motion::{self, SpringState};
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

/// A camera glide: one critically damped spring per component, retargetable mid-flight.
///
/// Zoom is sprung in log space so each step covers the same ratio, matching how zoom feels.
struct CameraAnim {
    x: SpringState,
    y: SpringState,
    log_zoom: SpringState,
    to: Camera,
}

/// Which way the user is moving through diagrams, so a switch can show it.
///
/// Set with [`DiagramCanvas::set_travel`] just before the switch; anything that is not a step
/// (tree click, palette, restore) is a [`Travel::Jump`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Travel {
    /// To the next diagram or later history: the new diagram arrives from the right.
    Forward,
    /// To the previous diagram or earlier history: the new diagram arrives from the left.
    Back,
    /// Anywhere else: the new diagram scales in without sliding.
    #[default]
    Jump,
}

impl Travel {
    /// The travel of a step of `delta` places (positive is forward); no step is a jump.
    pub fn from_step(delta: isize) -> Self {
        match delta.signum() {
            1 => Self::Forward,
            -1 => Self::Back,
            _ => Self::Jump,
        }
    }

    /// `1.0` forward, `-1.0` back, `0.0` for a jump: the sign of the horizontal motion.
    pub fn direction(self) -> f32 {
        match self {
            Self::Forward => 1.0,
            Self::Back => -1.0,
            Self::Jump => 0.0,
        }
    }
}

/// The travel announced for one navigation: which way, and the diagram it leads to.
type Announced = Option<(Travel, String)>;

/// Takes the travel announced for `diagram` out of `pending`; any other announcement stays
/// (an artifact for some other diagram, such as a re-render of the one being left, neither
/// uses nor discards it). Unannounced diagrams are jumps.
fn take_travel(pending: &mut Announced, diagram: &str) -> Travel {
    pending.take_if(|(_, target)| target == diagram).map_or(Travel::Jump, |(travel, _)| travel)
}

/// The travel for opening `file`, given what a step announced: only a step into that very file
/// counts, so a step that never opened anything cannot leak into an unrelated navigation.
pub fn navigation_travel(announced: Option<(Travel, String)>, file: &str) -> Travel {
    announced.filter(|(_, target)| target == file).map_or(Travel::Jump, |(travel, _)| travel)
}

/// Largest diagram (in nodes) that is kept to slide out; bigger ones just give way, so a
/// transition never paints two heavy scenes.
pub const OUTGOING_LIMIT: usize = 1500;

/// Whether a scene of `nodes` nodes is worth keeping to animate out.
pub fn keeps_outgoing(nodes: usize) -> bool {
    nodes <= OUTGOING_LIMIT
}

/// How far a slide covers, in points.
const TRAVEL_DISTANCE: f32 = 32.0;
/// Scale a jump starts from, about the viewport centre.
const JUMP_FROM_SCALE: f32 = 0.985;

/// Offsets, in points, of the (incoming, outgoing) diagrams at transition progress `p`
/// (0 at the switch, 1 at rest). The incoming one slides in from the side of travel while the
/// outgoing one leaves the other way; a jump does not slide.
pub fn travel_offsets(travel: Travel, p: f32) -> (Vec2, Vec2) {
    let d = travel.direction();
    (Vec2::new(TRAVEL_DISTANCE * (1.0 - p) * d, 0.0), Vec2::new(-TRAVEL_DISTANCE * p * d, 0.0))
}

/// `camera` zoomed by `scale` so that the point `centre` (viewport-local) keeps showing the
/// same world point.
fn scaled_about(camera: Camera, centre: Vec2, scale: f32) -> Camera {
    Camera {
        x: centre.x - (centre.x - camera.x) * scale,
        y: centre.y - (centre.y - camera.y) * scale,
        zoom: camera.zoom * scale,
    }
}

/// The diagram that is leaving: its picture, and the camera and travel it left with.
struct Outgoing {
    artifact: Arc<RenderArtifact>,
    camera: Camera,
    travel: Travel,
}

/// A scene as one layer of a frame: where its camera is and how opaque it is.
struct Layer<'a> {
    scene: &'a Scene,
    camera: Camera,
    opacity: f32,
}

/// Natural log of a zoom for the camera spring, with anything that is not a positive finite
/// zoom read as the smallest one, so the spring can always come to rest.
fn log_zoom(zoom: f32) -> f32 {
    if zoom.is_finite() { zoom.max(MIN_ZOOM).ln() } else { MIN_ZOOM.ln() }
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

/// The camera that places all of `bounds` inside `target`, a rectangle relative to the
/// viewport's min corner: the zoom that fits it, centred. A diagram opened from a page embed
/// starts here, so the embed appears to grow into the canvas.
pub fn camera_into(bounds: Rect, target: egui::Rect) -> Camera {
    let zoom = (target.width() / bounds.w.max(1.0)).min(target.height() / bounds.h.max(1.0)).max(f32::EPSILON);
    Camera {
        x: target.center().x - (bounds.x + bounds.w * 0.5) * zoom,
        y: target.center().y - (bounds.y + bounds.h * 0.5) * zoom,
        zoom,
    }
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
    /// Travel announced for the navigation to a diagram, until its artifact arrives.
    pending_travel: Announced,
    /// Travel of the transition that is running (or ran last).
    travel: Travel,
    /// The diagram sliding out while the new one arrives.
    outgoing: Option<Outgoing>,
    /// The camera the scene was last painted with, which is what an outgoing scene leaves with.
    painted_camera: Camera,
    bounds: Option<Rect>,
    size: Vec2,
    restore_pending: bool,
    fit_pending: bool,
    anim: Option<CameraAnim>,
    /// Motion-clock time of the last camera tick; `None` while no glide is running.
    last_tick: Option<f64>,
    /// Reduced motion was on last frame: camera moves land at once.
    reduced_motion: bool,
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
    /// Pass number of the last frame this canvas painted, to tell a fresh appearance from a repaint.
    last_pass: Option<u64>,
    /// The page embed a diagram grows out of; see [`DiagramCanvas::enter_from`].
    enter: Option<Enter>,
    /// The running transition is a growth from an embed: no scale-in, no outgoing scene.
    growing: bool,
}

/// A pending growth: the diagram (block id) it is for, the embed's screen rectangle, and when it
/// was announced (motion clock).
struct Enter {
    diagram: String,
    from: egui::Rect,
    at: f64,
}

/// Takes the growth announced for `diagram`, unless it has expired. A growth for another
/// diagram stays until its own arrives or it expires; an expired one is dropped.
fn take_enter(enter: &mut Option<Enter>, diagram: &str, now: f64) -> Option<egui::Rect> {
    if enter.as_ref().is_some_and(|e| now - e.at >= ENTER_TTL) {
        *enter = None;
    }
    enter.take_if(|e| e.diagram == diagram).map(|e| e.from)
}

/// How long an unused [`DiagramCanvas::enter_from`] stays valid, in motion-clock seconds. A
/// diagram that never arrives (a render error) must not leak its growth into a later navigation.
const ENTER_TTL: f64 = 2.0;

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
            pending_travel: None,
            travel: Travel::Jump,
            outgoing: None,
            painted_camera: Camera::default(),
            bounds: None,
            size: Vec2::ZERO,
            restore_pending: false,
            fit_pending: true,
            anim: None,
            last_tick: None,
            reduced_motion: false,
            scratch: Vec::new(),
            painted_frames: 0,
            last_stats: None,
            fade: Fade::default(),
            settling: false,
            fling: None,
            drag_velocity: Vec2::ZERO,
            follow_fit: false,
            last_pass: None,
            enter: None,
            growing: false,
        }
    }
}

/// Progress of the diagram transition, 0 at the switch and 1 at rest.
const TRANSITION_ID: &str = "canvas-transition";
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

    /// Glides the camera to `to`.
    ///
    /// A glide already in flight is retargeted and keeps its velocity, so repeated zoom steps
    /// flow into one motion instead of restarting from rest. Under reduced motion the camera
    /// lands at once.
    fn animate_to(&mut self, to: Camera) {
        self.fling = None;
        self.follow_fit = false;
        let from = self.camera;
        // A target that is not a real camera must never keep the spring from resting.
        let finite = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };
        let to = Camera {
            x: finite(to.x, from.x),
            y: finite(to.y, from.y),
            zoom: if to.zoom.is_finite() { to.zoom.max(MIN_ZOOM) } else { from.zoom },
        };
        if (to.x - from.x).abs() < 0.1
            && (to.y - from.y).abs() < 0.1
            && (to.zoom - from.zoom).abs() < 0.001
        {
            self.anim = None;
            return;
        }
        if self.reduced_motion {
            self.camera = to;
            self.anim = None;
            return;
        }
        let anim = self.anim.get_or_insert_with(|| CameraAnim {
            x: SpringState { value: from.x, velocity: 0.0 },
            y: SpringState { value: from.y, velocity: 0.0 },
            log_zoom: SpringState { value: log_zoom(from.zoom), velocity: 0.0 },
            to,
        });
        anim.to = to;
    }

    /// Advances the camera glide by `dt` seconds of motion-clock time.
    fn tick_anim(&mut self, dt: f32) {
        let Some(anim) = &mut self.anim else { return };
        let to = anim.to;
        motion::advance(&mut anim.x, to.x, motion::GLIDE, dt);
        motion::advance(&mut anim.y, to.y, motion::GLIDE, dt);
        motion::advance(&mut anim.log_zoom, log_zoom(to.zoom), motion::GLIDE, dt);
        let at_rest = |state: &SpringState, target: f32| state.value == target && state.velocity == 0.0;
        if at_rest(&anim.x, to.x) && at_rest(&anim.y, to.y) && at_rest(&anim.log_zoom, log_zoom(to.zoom)) {
            self.camera = to;
            self.anim = None;
        } else {
            self.camera = Camera { x: anim.x.value, y: anim.y.value, zoom: anim.log_zoom.value.exp() };
        }
    }

    /// Announces how the navigation to `diagram` (a block id) is made. Every navigation
    /// announces its own travel, which replaces any earlier announcement, so none can leak into
    /// a later switch; only the artifact of `diagram` uses it.
    pub fn set_travel(&mut self, travel: Travel, diagram: &str) {
        self.pending_travel = Some((travel, diagram.to_owned()));
    }

    /// Travel of the navigation now under way: the announced one until its diagram arrives,
    /// then the one its transition runs with. The top bar reads it, in the frame the label
    /// changes, to slide the breadcrumb the same way.
    pub fn travel(&self) -> Travel {
        self.pending_travel.as_ref().map_or(self.travel, |(travel, _)| *travel)
    }

    /// Announces that `diagram` (a block id) grows out of `from`, the on-screen rectangle of its
    /// page embed. The canvas starts that diagram's camera inside `from` and glides it to the
    /// fit, in place of the transition the navigation would otherwise run. Only that diagram's
    /// artifact uses it, however long the canvas keeps showing another one meanwhile.
    pub fn enter_from(&mut self, ctx: &egui::Context, diagram: &str, from: egui::Rect) {
        self.enter = Some(Enter { diagram: diagram.to_owned(), from, at: motion::now(ctx) });
    }

    /// Whether the camera glides, the diagram transition runs, or dimming eases.
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
        // The glide follows the motion clock, so slow-motion and reduced motion apply to it.
        let now = motion::now(ui.ctx());
        self.reduced_motion = motion::reduced(ui.ctx());
        if self.anim.is_some() {
            let last = self.last_tick.replace(now).unwrap_or(now);
            self.tick_anim((now - last) as f32);
        } else {
            self.last_tick = None;
        }
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
                        anim.x.value += shift.x;
                        anim.y.value += shift.y;
                        anim.to.x += shift.x;
                        anim.to.y += shift.y;
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

        // After being hidden (the page view) the canvas has no sensible camera to glide from:
        // a fit then lands at once instead of sliding in from the corner.
        let pass = ui.ctx().cumulative_pass_nr();
        let fresh = self.last_pass.is_none_or(|last| pass > last + 2);
        self.last_pass = Some(pass);

        let grow_from = take_enter(&mut self.enter, &artifact.diagram_id, motion::now(ui.ctx()));
        if self.artifact_id != artifact.id {
            let different_diagram = self.diagram_id != artifact.diagram_id;
            self.artifact_id.clone_from(&artifact.id);
            self.diagram_id.clone_from(&artifact.diagram_id);
            self.bounds = Some(scene.bounds);
            self.painted_frames = 0;
            let previous = self.shown.take();
            if different_diagram {
                self.travel = take_travel(&mut self.pending_travel, &artifact.diagram_id);
                self.growing = false;
                self.outgoing = previous
                    .filter(|old| !fresh && grow_from.is_none() && keeps_outgoing(old.rendered.scene.nodes.len()))
                    .map(|artifact| Outgoing { artifact, camera: self.painted_camera, travel: self.travel });
                motion::restart(ui.ctx(), egui::Id::new(TRANSITION_ID));
                self.selected = None;
                self.selected_edge = None;
                self.fade = Fade::default();
                motion::restart(ui.ctx(), egui::Id::new(DIM_ID));
                if !self.restore_pending {
                    self.fit_pending = true;
                }
            } else if let Some(previous) = previous {
                self.carry_over(&previous.rendered.scene, scene);
            }
            self.restore_pending = false;
        }
        self.shown = Some(artifact.clone());
        if let Some(from) = grow_from {
            // The embed grows into the canvas: start inside it and glide to the fit.
            self.growing = true;
            self.camera = camera_into(scene.bounds, from.translate(-viewport.min.to_vec2()));
            self.anim = None;
            self.fit_pending = false;
            self.fit();
        } else if self.fit_pending {
            self.fit();
            if fresh && let Some(anim) = self.anim.take() {
                self.camera = anim.to;
            }
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
        let progress = motion::spring_from(ui.ctx(), egui::Id::new(TRANSITION_ID), 0.0, 1.0, motion::GLIDE);
        self.settling = progress < 1.0 || motion::moving(ui.ctx(), egui::Id::new(DIM_ID));
        if progress >= 1.0 {
            self.outgoing = None;
        }
        // The leaving diagram goes underneath, without any selection emphasis.
        if let Some(out) = &self.outgoing {
            let opacity = 1.0 - motion::fade((2.0 * progress).min(1.0));
            if opacity > 0.0 {
                let (_, offset) = travel_offsets(out.travel, progress);
                let layer = Layer {
                    scene: &out.artifact.rendered.scene,
                    camera: Camera { x: out.camera.x + offset.x, y: out.camera.y + offset.y, ..out.camera },
                    opacity,
                };
                let plain = Emphasis { selected: None, strong_nodes: &[], strong_edges: &[], matches: &[], dim: 0.0 };
                paint_layer(&painter, viewport, &layer, style, dark, p.accent, &plain, &mut self.scratch);
            }
        }
        let (offset, _) = travel_offsets(self.travel, progress);
        let mut camera = Camera { x: self.camera.x + offset.x, y: self.camera.y + offset.y, ..self.camera };
        if self.travel == Travel::Jump && !self.growing {
            camera = scaled_about(camera, viewport.size() * 0.5, egui::lerp(JUMP_FROM_SCALE..=1.0, progress));
        }
        let layer = Layer { scene, camera, opacity: motion::fade(progress) };
        let stats = paint_layer(&painter, viewport, &layer, style, dark, p.accent, &emphasis, &mut self.scratch);
        self.painted_camera = self.camera;
        self.painted_frames = self.painted_frames.saturating_add(1);
        self.last_stats = Some(stats);
        // A quiet pill fades in while labels are hidden, above the zoom pill.
        let hint = motion::toggle(ui.ctx(), egui::Id::new("canvas-lod-hint"), stats.lod != Lod::Full, motion::BASE);
        if hint > 0.0 {
            let galley = painter.layout_no_wrap(
                "Zoom in for labels".to_owned(),
                FontId::proportional(token::FONT_SMALL),
                p.secondary,
            );
            let size = galley.size() + Vec2::new(20.0, 10.0);
            let rise = 6.0 * (1.0 - hint);
            let bottom = viewport.center_bottom() - Vec2::new(0.0, 76.0 - rise);
            let pill = egui::Rect::from_center_size(bottom - Vec2::new(0.0, size.y * 0.5), size);
            let mut hint_painter = painter.clone();
            hint_painter.multiply_opacity(hint);
            let e = theme::elevation(p, theme::Level::Raised);
            theme::paint_elevation(&hint_painter, pill, size.y * 0.5, &e);
            hint_painter.galley(pill.center() - galley.size() * 0.5, galley, p.secondary);
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
        let e = theme::elevation(p, theme::Level::Raised);
        theme::paint_elevation(painter, panel, token::RADIUS_PANEL, &e);
        let painter = painter.with_clip_rect(panel.intersect(viewport));
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
            let margin = egui::Margin::same(token::SPACE[4] as i8);
            theme::elevated(ui, p, theme::Level::Overlay, token::RADIUS_DIALOG, margin, |ui| {
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

/// Paints one layer of the frame: its scene through its camera at its opacity.
#[allow(clippy::too_many_arguments)]
fn paint_layer(
    painter: &egui::Painter,
    viewport: egui::Rect,
    layer: &Layer,
    style: &StyleSheet,
    dark: bool,
    accent: Color32,
    emphasis: &Emphasis,
    scratch: &mut Vec<ItemId>,
) -> paint::PaintStats {
    let mut painter = painter.clone();
    painter.multiply_opacity(layer.opacity);
    paint::paint_scene(&painter, viewport, layer.camera, layer.scene, style, dark, accent, emphasis, scratch)
}

#[cfg(test)]
mod tests {
    #[test]
    fn growth_is_taken_only_by_its_own_diagram() {
        let rect = egui::Rect::from_min_size(egui::pos2(1.0, 2.0), egui::vec2(3.0, 4.0));
        let mut enter = Some(Enter { diagram: "b".into(), from: rect, at: 10.0 });
        assert_eq!(take_enter(&mut enter, "a", 10.1), None);
        assert!(enter.is_some(), "another diagram's arrival leaves it waiting");
        assert_eq!(take_enter(&mut enter, "b", 10.2), Some(rect));
        assert!(enter.is_none());
        let mut stale = Some(Enter { diagram: "b".into(), from: rect, at: 10.0 });
        assert_eq!(take_enter(&mut stale, "b", 10.0 + ENTER_TTL), None);
        assert!(stale.is_none(), "an expired growth is dropped");
    }

    #[test]
    fn camera_into_places_bounds_in_target() {
        let bounds = Rect { x: 0.0, y: 0.0, w: 400.0, h: 200.0 };
        let target = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(200.0, 100.0));
        let cam = camera_into(bounds, target);
        assert!((cam.zoom - 0.5).abs() < 1e-4);
        assert!((cam.x - 100.0).abs() < 1e-3 && (cam.y - 50.0).abs() < 1e-3);
    }

    use super::*;

    #[test]
    fn camera_retarget_keeps_velocity() {
        let mut canvas = DiagramCanvas::default();
        canvas.camera = Camera { x: 0.0, y: 0.0, zoom: 1.0 };
        canvas.animate_to(Camera { x: 0.0, y: 0.0, zoom: 1.25 });
        for _ in 0..3 {
            canvas.tick_anim(1.0 / 60.0);
        }
        let z3 = canvas.camera.zoom;
        canvas.animate_to(Camera { x: 0.0, y: 0.0, zoom: 1.5625 });
        canvas.tick_anim(1.0 / 60.0);
        let z4 = canvas.camera.zoom;
        canvas.tick_anim(1.0 / 60.0);
        assert!(z4 > z3 && canvas.camera.zoom - z4 >= z4 - z3, "momentum lost");
    }

    #[test]
    fn travel_offsets_meet_at_rest() {
        assert_eq!(travel_offsets(Travel::Forward, 1.0), (Vec2::ZERO, Vec2::new(-32.0, 0.0)));
        assert_eq!(travel_offsets(Travel::Back, 0.0).0, Vec2::new(-32.0, 0.0));
        assert_eq!(travel_offsets(Travel::Jump, 0.3), (Vec2::ZERO, Vec2::ZERO));
    }

    #[test]
    fn outgoing_scene_skipped_when_large() {
        assert!(keeps_outgoing(1500));
        assert!(!keeps_outgoing(1501));
    }

    #[test]
    fn announced_travel_does_not_leak_into_other_navigation() {
        // A forward step announced for file A that opened nothing; then a jump to file B.
        let stale = Some((Travel::Forward, "A".to_owned()));
        assert_eq!(navigation_travel(stale.clone(), "A"), Travel::Forward);
        assert_eq!(navigation_travel(stale, "B"), Travel::Jump);
        assert_eq!(navigation_travel(None, "B"), Travel::Jump);

        // The canvas: a later navigation's announcement replaces the earlier one...
        let mut canvas = DiagramCanvas::default();
        canvas.set_travel(Travel::Forward, "a");
        canvas.set_travel(Travel::Jump, "b");
        assert_eq!(canvas.travel(), Travel::Jump);
        assert_eq!(take_travel(&mut canvas.pending_travel, "b"), Travel::Jump);

        // ...and only the announced diagram's artifact takes it.
        let mut pending = Some((Travel::Back, "b".to_owned()));
        assert_eq!(take_travel(&mut pending, "other"), Travel::Jump);
        assert!(pending.is_some(), "a re-render of another diagram must not discard it");
        assert_eq!(take_travel(&mut pending, "b"), Travel::Back);
        assert!(pending.is_none());
    }

    #[test]
    fn stepping_maps_to_travel() {
        assert_eq!(Travel::from_step(1), Travel::Forward);
        assert_eq!(Travel::from_step(-1), Travel::Back);
        assert_eq!(Travel::from_step(0), Travel::Jump);
    }

    #[test]
    fn scaling_keeps_the_viewport_centre_fixed() {
        let camera = Camera { x: 30.0, y: -12.0, zoom: 1.5 };
        let centre = Vec2::new(400.0, 300.0);
        let scaled = scaled_about(camera, centre, 0.985);
        assert!((scaled.zoom - 1.5 * 0.985).abs() < 1e-6);
        // The world point under the centre is the same before and after.
        let world = |c: Camera| ((centre.x - c.x) / c.zoom, (centre.y - c.y) / c.zoom);
        let (before, after) = (world(camera), world(scaled));
        assert!((before.0 - after.0).abs() < 1e-3 && (before.1 - after.1).abs() < 1e-3);
    }

    #[test]
    fn camera_glide_settles_whatever_the_target() {
        for to in [
            Camera { x: 10.0, y: 5.0, zoom: 0.0 },
            Camera { x: 10.0, y: 5.0, zoom: -2.0 },
            Camera { x: 10.0, y: 5.0, zoom: f32::NAN },
            Camera { x: f32::NAN, y: f32::INFINITY, zoom: 1.0 },
        ] {
            let mut canvas = DiagramCanvas::default();
            canvas.camera = Camera { x: 0.0, y: 0.0, zoom: 1.0 };
            canvas.animate_to(to);
            for _ in 0..600 {
                canvas.tick_anim(1.0 / 60.0);
            }
            assert!(!canvas.is_animating(), "glide to {to:?} never settled");
            assert!(canvas.camera.zoom.is_finite() && canvas.camera.zoom > 0.0);
        }
    }
}
