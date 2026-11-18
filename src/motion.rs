//! Motion: small eased tweens keyed by `egui::Id`, so any surface can animate a value
//! without owning a timer.
//!
//! A tween snaps to its first target (nothing animates on first sight unless asked) and,
//! when the target changes, eases from wherever it currently is. Retargeting mid-flight is
//! therefore smooth, which matters for hover and for panels toggled quickly.

use egui::{Context, Id};

/// Hover, press and other feedback that must feel immediate.
pub const FAST: f32 = 0.12;
/// Popovers, highlights, selection.
pub const BASE: f32 = 0.18;
/// Panels sliding in and out.
pub const PANEL: f32 = 0.26;
/// Camera moves (fit, focus, zoom steps).
pub const CAMERA: f32 = 0.34;

/// Decelerating curve with a soft landing (close to CSS `cubic-bezier(0.2, 0.8, 0.2, 1)`).
pub fn ease_out(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u * u
}

/// Ease out with a slight overshoot, for things that pop in (key caps, badges).
pub fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c1 = 1.4;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

#[derive(Clone, Copy)]
struct Tween {
    from: f32,
    to: f32,
    start: f64,
    duration: f32,
    value: f32,
    easing: fn(f32) -> f32,
}

fn now(ctx: &Context) -> f64 {
    ctx.input(|input| input.time)
}

/// Eases the value stored under `id` toward `target`. The first call returns `target`.
pub fn tween(ctx: &Context, id: Id, target: f32, duration: f32, easing: fn(f32) -> f32) -> f32 {
    tween_from(ctx, id, target, target, duration, easing)
}

/// Like [`tween`], but the first call starts at `initial` and animates toward `target`.
pub fn tween_from(
    ctx: &Context,
    id: Id,
    initial: f32,
    target: f32,
    duration: f32,
    easing: fn(f32) -> f32,
) -> f32 {
    let id = id.with("motion-tween");
    let time = now(ctx);
    let previous = ctx.data(|data| data.get_temp::<Tween>(id));
    let mut tween = previous.unwrap_or(Tween {
        from: initial,
        to: target,
        start: time,
        duration,
        value: initial,
        easing,
    });
    if (tween.to - target).abs() > f32::EPSILON {
        tween = Tween { from: tween.value, to: target, start: time, duration, value: tween.value, easing };
    }
    let t = if tween.duration <= 0.0 {
        1.0
    } else {
        ((time - tween.start) as f32 / tween.duration).clamp(0.0, 1.0)
    };
    tween.value = if t >= 1.0 {
        tween.to
    } else {
        tween.from + (tween.to - tween.from) * (tween.easing)(t)
    };
    // Read the pass number before taking the data lock: egui's context lock is not
    // re-entrant, so calling back into `ctx` inside `data_mut` would deadlock.
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| {
        data.insert_temp(id, tween);
        if t < 1.0 {
            data.insert_temp(Id::new(ACTIVE_PASS), pass);
        }
    });
    if t < 1.0 {
        ctx.request_repaint();
    }
    tween.value
}

const ACTIVE_PASS: &str = "motion-active-pass";

/// Whether any tween moved in this pass or the previous one (screenshot mode waits for this).
pub fn busy(ctx: &Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|data| data.get_temp::<u64>(Id::new(ACTIVE_PASS)))
        .is_some_and(|active| active + 1 >= pass)
}

/// 0 to 1 as `on` becomes true, back to 0 as it becomes false. Snaps on first sight.
pub fn toggle(ctx: &Context, id: Id, on: bool, duration: f32) -> f32 {
    tween(ctx, id, f32::from(u8::from(on)), duration, ease_out)
}

/// Like [`toggle`], but the first sight of `on == true` animates in from 0.
pub fn appear(ctx: &Context, id: Id, on: bool, duration: f32) -> f32 {
    tween_from(ctx, id, 0.0, f32::from(u8::from(on)), duration, ease_out)
}

/// Restarts the 0-to-1 animation under `id` (for a surface that opens again).
pub fn restart(ctx: &Context, id: Id) {
    let id = id.with("motion-tween");
    ctx.data_mut(|data| data.remove::<Tween>(id));
}

/// Whether the tween under `id` is still moving.
pub fn moving(ctx: &Context, id: Id) -> bool {
    let id = id.with("motion-tween");
    let time = now(ctx);
    ctx.data(|data| data.get_temp::<Tween>(id))
        .is_some_and(|tween| (time - tween.start) < tween.duration as f64 && tween.value != tween.to)
}
