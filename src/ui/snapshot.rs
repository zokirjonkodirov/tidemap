//! Snapshot crossfade: the previous frame is screenshotted and animated away on top of the
//! new one.
//!
//! A change that would otherwise be a hard cut (the theme, the page and diagram views) is made
//! in three steps. [`request`] asks the backend for a screenshot of what is on screen now;
//! [`take`] receives it a frame or two later, keeps it as a texture and hands the [`Purpose`]
//! back so the app makes the change in that very frame; [`paint`] draws the old frame over the
//! new one and animates it away: the theme sweeps out in a growing circle from where it was
//! asked for, the page scales and fades.

use egui::{
    Color32, Context, Event, Id, LayerId, Mesh, Order, Pos2, Rect, Shape, TextureHandle, TextureId, TextureOptions,
    UserData, ViewportCommand, epaint::Vertex, pos2, vec2,
};
use tidemap_model::Theme;

use crate::motion;

/// What a snapshot is for. Both are toggles: what they do is decided when they are applied,
/// so several made in a row compose (two theme toggles end where they began).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Purpose {
    /// The appearance flips between light and dark, spreading outward from `from`.
    Theme { from: Pos2 },
    /// The page and diagram views swap; the old view fades inside `clip` (the canvas area).
    Page { clip: Rect },
}

/// The theme after one toggle. `system_dark` is whether the system appearance is dark, which
/// decides what a toggle from "match system" means; an explicit theme just flips.
pub fn toggled_theme(theme: Theme, system_dark: bool) -> Theme {
    match theme {
        Theme::Dark => Theme::Light,
        Theme::Light => Theme::Dark,
        Theme::System if system_dark => Theme::Light,
        Theme::System => Theme::Dark,
    }
}

/// What travels with a screenshot request and comes back with it: which request it answers.
#[derive(Clone, Copy)]
struct Ticket {
    id: u64,
    purpose: Purpose,
}

/// The request waiting for its screenshot, and the ones made since, in order.
#[derive(Clone)]
struct Pending {
    ticket: Ticket,
    queued: Vec<Purpose>,
    /// Pass number and motion-clock time of the request.
    pass: u64,
    at: f64,
}

/// A reveal in progress. Dropping it frees the texture.
#[derive(Clone)]
struct Reveal {
    purpose: Purpose,
    start: f64,
    texture: TextureHandle,
}

const PENDING: &str = "snapshot-pending";
const REVEAL: &str = "snapshot-reveal";
const NEXT_ID: &str = "snapshot-next-id";
/// Segments of the circle in a theme reveal.
const SEGMENTS: usize = 96;
/// Length of the plain fade used instead of the sweep and scale under reduced motion.
const REDUCED_FADE: f32 = 0.15;
/// A page snapshot ends at this scale (about its centre) while it fades.
const PAGE_END_SCALE: f32 = 0.98;
/// Passes and motion-clock seconds after which a screenshot that has not arrived is given up on.
const OVERDUE_PASSES: u64 = 3;
const OVERDUE_SECONDS: f64 = 0.15;

/// Whether a screenshot requested at (`requested_pass`, `requested_at`) is overdue at
/// (`now_pass`, `now`): three passes or 150 ms have gone by. The change is then made without
/// animation, so a toggle is never left unapplied.
fn overdue(requested_pass: u64, requested_at: f64, now_pass: u64, now: f64) -> bool {
    now_pass >= requested_pass + OVERDUE_PASSES || now - requested_at >= OVERDUE_SECONDS
}

/// Asks for a screenshot of the current frame, for `purpose`. A request made while an earlier
/// one is still waiting is queued behind it: all are applied, in order, when that screenshot
/// arrives, so no toggle is ever lost.
pub fn request(ctx: &Context, purpose: Purpose) {
    let key = Id::new(PENDING);
    motion::keep_busy(ctx);
    if let Some(mut waiting) = ctx.data(|data| data.get_temp::<Pending>(key)) {
        waiting.queued.push(purpose);
        ctx.data_mut(|data| data.insert_temp(key, waiting));
        return;
    }
    let (pass, at) = (ctx.cumulative_pass_nr(), motion::now(ctx));
    let id = ctx.data_mut(|data| {
        let id = data.get_temp::<u64>(Id::new(NEXT_ID)).unwrap_or(0) + 1;
        data.insert_temp(Id::new(NEXT_ID), id);
        id
    });
    let ticket = Ticket { id, purpose };
    ctx.data_mut(|data| data.insert_temp(key, Pending { ticket, queued: Vec::new(), pass, at }));
    ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::new(ticket)));
}

/// Takes our screenshot when it arrives: keeps it as the texture of a new reveal (replacing any
/// running one, whose half-revealed frame the screenshot already contains) and returns the
/// purposes to apply, oldest first, in this same frame. When the screenshot is overdue they are
/// returned without a reveal. While it is awaited, frames keep coming, so one or the other
/// always resolves without input. Screenshots that are not ours (screenshot mode's own) or not
/// the current request's (a late one after the fallback) are ignored.
pub fn take(ctx: &Context) -> Vec<Purpose> {
    let Some(waiting) = ctx.data(|data| data.get_temp::<Pending>(Id::new(PENDING))) else { return Vec::new() };
    let arrived = ctx.input(|input| {
        input.events.iter().find_map(|event| match event {
            Event::Screenshot { user_data, image, .. } => user_data
                .data
                .as_ref()
                .and_then(|data| data.downcast_ref::<Ticket>())
                .filter(|ticket| ticket.id == waiting.ticket.id)
                .map(|_| image.clone()),
            _ => None,
        })
    });
    let first = waiting.ticket.purpose;
    let reveal = arrived.map(|image| Reveal {
        purpose: first,
        start: motion::now(ctx),
        texture: ctx.load_texture("snapshot", (*image).clone(), TextureOptions::LINEAR),
    });
    if reveal.is_none() && !overdue(waiting.pass, waiting.at, ctx.cumulative_pass_nr(), motion::now(ctx)) {
        motion::keep_busy(ctx);
        return Vec::new();
    }
    ctx.data_mut(|data| {
        data.remove::<Pending>(Id::new(PENDING));
        if let Some(reveal) = reveal {
            data.insert_temp(Id::new(REVEAL), reveal);
        }
    });
    ctx.request_repaint();
    std::iter::once(first).chain(waiting.queued).collect()
}

/// Progress of a reveal `age` seconds old, 0 to 1, and whether it is over. Reduced motion is a
/// plain 150 ms ramp; otherwise the glide spring.
fn progress(reduced: bool, age: f32) -> (f32, bool) {
    if reduced {
        let t = (age / REDUCED_FADE).clamp(0.0, 1.0);
        (t, t >= 1.0)
    } else {
        let p = motion::step(motion::GLIDE, age);
        (p, p >= 0.999)
    }
}

/// Paints the old frame over the new one while a reveal runs, on a top layer that takes no
/// input, and frees the texture when it ends. Call it last in the frame.
pub fn paint(ctx: &Context) {
    let Some(reveal) = ctx.data(|data| data.get_temp::<Reveal>(Id::new(REVEAL))) else { return };
    let reduced = motion::reduced(ctx);
    let age = (motion::now(ctx) - reveal.start) as f32;
    let (p, done) = progress(reduced, age);
    if done {
        ctx.data_mut(|data| data.remove::<Reveal>(Id::new(REVEAL)));
        return;
    }
    motion::keep_busy(ctx);
    let screen = ctx.content_rect();
    let painter = ctx.layer_painter(LayerId::new(Order::Debug, Id::new("snapshot")));
    let texture = reveal.texture.id();
    match reveal.purpose {
        Purpose::Theme { .. } if reduced => {
            let mut mesh = Mesh::with_texture(texture);
            mesh.add_rect_with_uv(screen, unit_uv(), Color32::from_white_alpha(alpha(1.0 - p)));
            painter.add(Shape::mesh(mesh));
        }
        Purpose::Theme { from, .. } => {
            let radius = p * farthest_corner(from, screen);
            painter.add(Shape::mesh(ring_mesh(from, radius, screen, SEGMENTS, texture)));
        }
        Purpose::Page { clip } => {
            let scale = if reduced { 1.0 } else { egui::lerp(1.0..=PAGE_END_SCALE, p) };
            let opacity = if reduced { 1.0 - p } else { 1.0 - motion::fade(p) };
            let shown = Rect::from_center_size(clip.center(), clip.size() * scale);
            let uv = Rect::from_min_max(
                pos2((clip.min.x - screen.min.x) / screen.width(), (clip.min.y - screen.min.y) / screen.height()),
                pos2((clip.max.x - screen.min.x) / screen.width(), (clip.max.y - screen.min.y) / screen.height()),
            );
            let mut mesh = Mesh::with_texture(texture);
            mesh.add_rect_with_uv(shown, uv, Color32::from_white_alpha(alpha(opacity)));
            painter.with_clip_rect(clip).add(Shape::mesh(mesh));
        }
    }
}

/// The whole texture.
fn unit_uv() -> Rect {
    Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0))
}

/// An opacity from 0 to 1 as a byte.
fn alpha(opacity: f32) -> u8 {
    (opacity.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Distance from `center` to the farthest corner of `screen`.
fn farthest_corner(center: Pos2, screen: Rect) -> f32 {
    [screen.left_top(), screen.right_top(), screen.left_bottom(), screen.right_bottom()]
        .into_iter()
        .map(|corner| (corner - center).length())
        .fold(0.0, f32::max)
}

/// The part of the old frame outside a circle: quads between the circle of `radius` around
/// `center` and one that reaches past the farthest corner of `screen`, each vertex textured with
/// `(pos - screen.min) / screen.size()`. Empty once the circle covers the screen.
pub fn ring_mesh(center: Pos2, radius: f32, screen: Rect, segments: usize, texture: TextureId) -> Mesh {
    let mut mesh = Mesh::with_texture(texture);
    let reach = farthest_corner(center, screen);
    if radius >= reach || segments < 3 {
        return mesh;
    }
    // The outer polygon is inscribed in its circle, so widen it to contain the circle's corner.
    let outer = reach / (std::f32::consts::PI / segments as f32).cos();
    let uv = |pos: Pos2| pos2((pos.x - screen.min.x) / screen.width(), (pos.y - screen.min.y) / screen.height());
    let vertex = |pos: Pos2| Vertex { pos, uv: uv(pos), color: Color32::WHITE };
    for i in 0..segments {
        let angle = std::f32::consts::TAU * i as f32 / segments as f32;
        let direction = vec2(angle.cos(), angle.sin());
        mesh.vertices.push(vertex(center + direction * radius));
        mesh.vertices.push(vertex(center + direction * outer));
    }
    for i in 0..segments {
        let (inner, out) = (2 * i as u32, 2 * i as u32 + 1);
        let next = 2 * ((i + 1) % segments) as u32;
        mesh.indices.extend_from_slice(&[inner, out, next + 1, inner, next + 1, next]);
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_is_empty_once_radius_covers_screen() {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 940.0));
        let corner = egui::vec2(1440.0, 940.0).length();
        assert!(ring_mesh(egui::Pos2::ZERO, corner + 1.0, screen, 96, egui::TextureId::default()).indices.is_empty());
        assert_eq!(ring_mesh(egui::Pos2::ZERO, 0.0, screen, 96, egui::TextureId::default()).indices.len(), 96 * 6);
    }

    #[test]
    fn queued_theme_toggles_compose() {
        let system_dark = true;
        let fold = |start: Theme, toggles: usize| (0..toggles).fold(start, |theme, _| toggled_theme(theme, system_dark));
        assert_eq!(fold(Theme::Light, 1), Theme::Dark);
        assert_eq!(fold(Theme::Light, 2), Theme::Light);
        assert_eq!(fold(Theme::Dark, 3), Theme::Light);
        // From "match system" (dark here) one toggle is light; the next is dark again.
        assert_eq!(fold(Theme::System, 1), Theme::Light);
        assert_eq!(fold(Theme::System, 2), Theme::Dark);
    }

    #[test]
    fn missing_screenshot_is_overdue_after_three_passes_or_150_ms() {
        assert!(!overdue(10, 1.0, 12, 1.05));
        assert!(overdue(10, 1.0, 13, 1.05));
        assert!(overdue(10, 1.0, 11, 1.16));
        assert!(!overdue(10, 1.0, 10, 1.0));
    }
}
