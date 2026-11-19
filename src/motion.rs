//! Motion: springs and small eased tweens keyed by `egui::Id`, so any surface can animate a
//! value without owning a timer.
//!
//! Springs ([`spring`], [`hover`], [`press`], [`presence`]) are the primary tool. They carry
//! velocity, so retargeting mid-flight (a hover that reverses, a panel toggled twice) bends
//! the motion instead of restarting it. The older eased tweens remain for surfaces that have
//! not moved over yet.
//!
//! A tween snaps to its first target (nothing animates on first sight unless asked) and,
//! when the target changes, eases from wherever it currently is. Retargeting mid-flight is
//! therefore smooth, which matters for hover and for panels toggled quickly.

// The spring API lands ahead of its callers (hover, press, panels, camera follow in later
// tasks); drop this once they use it.
#![allow(dead_code)]

use egui::{Context, Id};
use std::f32::consts::TAU;

/// Hover, press and other feedback that must feel immediate.
pub const FAST: f32 = 0.12;
/// Popovers, highlights, selection.
pub const BASE: f32 = 0.18;
/// Panels sliding in and out.
pub const PANEL: f32 = 0.26;

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

/// The motion clock in seconds: input time divided by the slow-motion scale. Anything that
/// times an animation itself (rather than through a spring) must read this clock so that
/// `TIDEMAP_SLOWMO` slows it too.
pub fn now(ctx: &Context) -> f64 {
    let time = ctx.input(|input| input.time);
    time / f64::from(time_scale(ctx))
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
    ctx.data_mut(|data| data.insert_temp(id, tween));
    if t < 1.0 {
        mark_active(ctx);
    }
    tween.value
}

const ACTIVE_PASS: &str = "motion-active-pass";

/// Records that something moved in this pass and asks for another frame.
fn mark_active(ctx: &Context) {
    // Read the pass number before taking the data lock: egui's context lock is not
    // re-entrant, so calling back into `ctx` inside `data_mut` would deadlock.
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| data.insert_temp(Id::new(ACTIVE_PASS), pass));
    ctx.request_repaint();
}

/// Whether any tween or spring moved in this pass or the previous one (screenshot mode waits
/// for this).
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

/// Restarts the animation under `id` (for a surface that opens again): the tween and any
/// spring, [`hover`], [`press`] or [`presence`] state keyed by `id` start from scratch.
pub fn restart(ctx: &Context, id: Id) {
    ctx.data_mut(|data| {
        data.remove::<Tween>(id.with("motion-tween"));
        for suffix in SPRING_SUFFIXES {
            data.remove::<Stored>(spring_key(id, suffix));
        }
        data.remove::<PressMemo>(id.with("press-memo"));
    });
}

/// Whether the tween or spring under `id` is still moving.
pub fn moving(ctx: &Context, id: Id) -> bool {
    let time = now(ctx);
    ctx.data(|data| {
        let tween = data
            .get_temp::<Tween>(id.with("motion-tween"))
            .is_some_and(|tween| (time - tween.start) < f64::from(tween.duration) && tween.value != tween.to);
        tween
            || SPRING_SUFFIXES.iter().any(|suffix| {
                data.get_temp::<Stored>(spring_key(id, suffix)).is_some_and(|stored| !stored.at_rest())
            })
    })
}

/// A spring's feel: how quickly it responds and how much it settles without bouncing.
#[derive(Clone, Copy)]
pub struct Spring {
    /// Seconds the spring takes to cover most of the distance to its target.
    pub response: f32,
    /// Damping as a fraction of critical damping: 1.0 settles without overshoot, lower values
    /// bounce a little before resting.
    pub damping: f32,
}

/// Immediate feedback: press and hover arrival.
pub const SNAP: Spring = Spring { response: 0.16, damping: 1.0 };
/// Gentle release, for things that should linger as they leave.
pub const SOFT: Spring = Spring { response: 0.30, damping: 1.0 };
/// Large smooth moves: panels sliding, camera travel.
pub const GLIDE: Spring = Spring { response: 0.38, damping: 1.0 };
/// A lively arrival with a small overshoot, for things that pop in.
pub const POP: Spring = Spring { response: 0.34, damping: 0.74 };

/// Position and velocity of a spring-driven value.
#[derive(Clone, Copy, Default)]
pub struct SpringState {
    /// Current position.
    pub value: f32,
    /// Current speed, in value units per second.
    pub velocity: f32,
}

/// Longest slice of time a single [`advance`] call will integrate, so a stalled frame cannot
/// fling a spring across the screen.
const MAX_DT: f32 = 1.0 / 30.0;
/// Largest integration substep; keeps the stiffest springs stable and accurate.
const MAX_SUBSTEP: f32 = 1.0 / 240.0;
/// Distance (relative to the target's magnitude) under which a slow spring counts as arrived.
const REST_DISTANCE: f32 = 5e-4;

/// Moves `state` toward `target` over `dt` seconds, keeping its velocity so a changed target
/// bends the motion rather than restarting it. A spring that comes close and slow enough
/// snaps exactly onto the target, so values at rest compare equal to it.
pub fn advance(state: &mut SpringState, target: f32, spring: Spring, dt: f32) {
    let sane = state.value.is_finite()
        && state.velocity.is_finite()
        && target.is_finite()
        && spring.response > 0.0
        && spring.damping.is_finite();
    if !sane {
        *state = SpringState { value: target, velocity: 0.0 };
        return;
    }
    let dt = if dt.is_finite() { dt.clamp(0.0, MAX_DT) } else { 0.0 };
    let omega = TAU / spring.response;
    let stiffness = omega * omega;
    let damping = 2.0 * omega * spring.damping;
    let steps = (dt / MAX_SUBSTEP).ceil().max(1.0);
    let h = dt / steps;
    for _ in 0..steps as u32 {
        let accel = stiffness * (target - state.value) - damping * state.velocity;
        state.velocity += accel * h;
        state.value += state.velocity * h;
    }
    let scale = target.abs().max(1.0);
    let close = (state.value - target).abs() < REST_DISTANCE * scale;
    let slow = state.velocity.abs() < REST_DISTANCE * 10.0 * scale;
    if close && slow {
        *state = SpringState { value: target, velocity: 0.0 };
    }
}

/// The closed-form response of a spring released from 0 toward 1 after `elapsed` seconds.
/// Over-damped settings are treated as critically damped.
pub fn step(spring: Spring, elapsed: f32) -> f32 {
    if elapsed <= 0.0 {
        return 0.0;
    }
    if spring.response <= 0.0 {
        return 1.0;
    }
    let omega = TAU / spring.response;
    let zeta = spring.damping;
    if zeta >= 1.0 {
        let x = omega * elapsed;
        1.0 - (1.0 + x) * (-x).exp()
    } else {
        let omega_d = omega * (1.0 - zeta * zeta).sqrt();
        let decay = (-zeta * omega * elapsed).exp();
        let phase = omega_d * elapsed;
        1.0 - decay * (phase.cos() + zeta * omega / omega_d * phase.sin())
    }
}

/// A spring's state between frames.
#[derive(Clone, Copy)]
struct Stored {
    state: SpringState,
    target: f32,
    /// Motion-clock time of the last update.
    last: f64,
}

impl Stored {
    fn at_rest(&self) -> bool {
        self.state.value == self.target && self.state.velocity == 0.0
    }
}

/// Suffixes under which the spring helpers keep state, so `hover(id)` and `spring(id)` never
/// share a spring.
const SPRING_SUFFIXES: [&str; 4] = ["", "hover", "press", "presence"];

fn spring_key(id: Id, suffix: &str) -> Id {
    let id = if suffix.is_empty() { id } else { id.with(suffix) };
    id.with("motion-spring")
}

/// Springs the value stored under `id` toward `target`. The first call returns `target`, so
/// nothing animates on first sight.
pub fn spring(ctx: &Context, id: Id, target: f32, spring: Spring) -> f32 {
    spring_from(ctx, id, target, target, spring)
}

/// Like [`spring`], but the first call starts at `initial` and animates toward `target`.
/// Changing `target` later keeps the current position and momentum. With reduced motion on,
/// the value resolves to `target` immediately.
pub fn spring_from(ctx: &Context, id: Id, initial: f32, target: f32, spring: Spring) -> f32 {
    spring_at(ctx, spring_key(id, ""), initial, target, spring)
}

fn spring_at(ctx: &Context, key: Id, initial: f32, target: f32, spring: Spring) -> f32 {
    // Everything that needs `ctx` is read before the data lock below (it is not re-entrant).
    let time = now(ctx);
    let reduced = reduced(ctx);
    let mut stored = ctx.data(|data| data.get_temp::<Stored>(key)).unwrap_or(Stored {
        state: SpringState { value: initial, velocity: 0.0 },
        target,
        last: time,
    });
    if reduced {
        stored.state = SpringState { value: target, velocity: 0.0 };
    } else {
        advance(&mut stored.state, target, spring, (time - stored.last) as f32);
    }
    stored.target = target;
    stored.last = time;
    ctx.data_mut(|data| data.insert_temp(key, stored));
    if !stored.at_rest() {
        mark_active(ctx);
    }
    stored.state.value
}

/// Hover feedback: 0 to 1 as `on` becomes true (quick), back to 0 as it turns false (slower,
/// so the highlight lingers). The first call returns the target without animating.
pub fn hover(ctx: &Context, id: Id, on: bool) -> f32 {
    let (target, feel) = if on { (1.0, SNAP) } else { (0.0, SOFT) };
    spring_at(ctx, spring_key(id, "hover"), target, target, feel)
}

/// How long a press stays visible after it began, so a click shorter than a frame or two
/// still reads as a press.
const PRESS_HOLD: f64 = 0.08;

/// When the current press began, and whether it was held on the previous call.
#[derive(Clone, Copy)]
struct PressMemo {
    start: f64,
    was_down: bool,
}

/// Press feedback: 0 to 1 while `down`, with a 0.08 s minimum so quick clicks still show,
/// then a bouncy release. The first call starts at 0.
pub fn press(ctx: &Context, id: Id, down: bool) -> f32 {
    let time = now(ctx);
    let memo_key = id.with("press-memo");
    let memo = ctx.data(|data| data.get_temp::<PressMemo>(memo_key));
    let start = match memo {
        Some(memo) if !down || memo.was_down => memo.start,
        _ if down => time,
        _ => f64::NEG_INFINITY,
    };
    ctx.data_mut(|data| data.insert_temp(memo_key, PressMemo { start, was_down: down }));
    let held = down || time - start < PRESS_HOLD;
    let (target, feel) = if held { (1.0, SNAP) } else { (0.0, POP) };
    spring_at(ctx, spring_key(id, "press"), 0.0, target, feel)
}

/// Presence of a surface that opens and closes: springs in from 0 with a small pop when
/// `open`, springs out quickly when closed, and returns `None` once fully closed so the
/// caller can stop drawing it. Reopening mid-exit continues from the current position and
/// momentum. The value is never negative: a closing spring that undershoots 0 reads as 0.
pub fn presence(ctx: &Context, id: Id, open: bool) -> Option<f32> {
    let (target, feel) = if open { (1.0, POP) } else { (0.0, SNAP) };
    let value = spring_at(ctx, spring_key(id, "presence"), 0.0, target, feel).max(0.0);
    (open || value > 0.0).then_some(value)
}

/// Maps elapsed motion time `t` (seconds) to a 0-to-1 opacity ramp over 0.6 s. Opacity is
/// kept separate from position springs so overshoot never makes anything over-opaque.
pub fn fade(t: f32) -> f32 {
    (t / 0.6).clamp(0.0, 1.0)
}

const REDUCED: &str = "motion-reduced";
const TIME_SCALE: &str = "motion-time-scale";

/// Turns reduced motion on or off. When on, springs jump straight to their targets.
pub fn set_reduced(ctx: &Context, on: bool) {
    ctx.data_mut(|data| data.insert_temp(Id::new(REDUCED), on));
}

/// Whether reduced motion is on.
pub fn reduced(ctx: &Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(Id::new(REDUCED))).unwrap_or(false)
}

/// Slows (or speeds) all motion by dividing the motion clock by `scale`, for inspecting
/// animations (`TIDEMAP_SLOWMO`). Values that are not finite and positive fall back to 1.
pub fn set_time_scale(ctx: &Context, scale: f32) {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    ctx.data_mut(|data| data.insert_temp(Id::new(TIME_SCALE), scale));
}

fn time_scale(ctx: &Context) -> f32 {
    ctx.data(|data| data.get_temp::<f32>(Id::new(TIME_SCALE))).unwrap_or(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Context, Id, RawInput};

    fn at(ctx: &Context, time: f64, mut f: impl FnMut(&Context)) {
        let _ = ctx.run(RawInput { time: Some(time), ..Default::default() }, |ctx| f(ctx));
    }

    #[test]
    fn first_frame_moves_less_than_a_fifth() {
        for s in [SNAP, SOFT, GLIDE, POP] { assert!(step(s, 1.0 / 60.0) < 0.2); }
    }

    #[test]
    fn critically_damped_never_overshoots_and_pop_barely_does() {
        let peak = |s| (0..400).map(|i| step(s, i as f32 / 200.0)).fold(0.0_f32, f32::max);
        assert!(peak(GLIDE) <= 1.0 + 1e-4);
        assert!(peak(POP) > 1.0 && peak(POP) < 1.05);
    }

    #[test]
    fn retarget_keeps_position_and_momentum() {
        let ctx = Context::default();
        let id = Id::new("r");
        let mut v = vec![];
        at(&ctx, 0.0, |c| { spring(c, id, 0.0, SOFT); });
        for f in 1..=4 { at(&ctx, f as f64 / 60.0, |c| v.push(spring(c, id, 1.0, SOFT))); }
        at(&ctx, 5.0 / 60.0, |c| v.push(spring(c, id, 0.0, SOFT)));
        assert!(v[4] >= v[3] - 1e-4, "reversed instantly: {v:?}");
        assert!((v[4] - v[3]).abs() < 0.2);
    }

    #[test]
    fn long_gap_stays_bounded() {
        let ctx = Context::default();
        let id = Id::new("g");
        at(&ctx, 0.0, |c| { spring(c, id, 0.0, POP); });
        at(&ctx, 0.016, |c| { spring(c, id, 1.0, POP); });
        let mut late = 0.0;
        at(&ctx, 10.0, |c| late = spring(c, id, 1.0, POP));
        assert!((0.0..=1.05).contains(&late));
    }

    #[test]
    fn springs_rest_exactly_and_busy_clears() {
        let ctx = Context::default();
        let id = Id::new("s");
        at(&ctx, 0.0, |c| { spring(c, id, 0.0, GLIDE); });
        let mut last = 0.0;
        for f in 1..=120 { at(&ctx, f as f64 / 60.0, |c| last = spring(c, id, 1.0, GLIDE)); }
        assert_eq!(last, 1.0);
        let mut busy_now = true;
        at(&ctx, 2.1, |c| { spring(c, id, 1.0, GLIDE); });
        at(&ctx, 2.2, |c| busy_now = busy(c));
        assert!(!busy_now);
    }

    #[test]
    fn hover_leaves_slower_than_it_arrives() {
        let ctx = Context::default();
        let id = Id::new("h");
        let frames_until = |on: bool, done: fn(f32) -> bool, start: u32| {
            let mut f = start;
            loop { f += 1; let mut v = 0.0; at(&ctx, f as f64 / 60.0, |c| v = hover(c, id, on)); if done(v) { return f - start; } }
        };
        at(&ctx, 0.0, |c| { hover(c, id, false); });
        let arrive = frames_until(true, |v| v >= 0.9, 0);
        let leave = frames_until(false, |v| v <= 0.1, 200);
        assert!(leave * 2 > arrive * 3, "arrive {arrive} leave {leave}");
    }

    #[test]
    fn short_click_still_presses() {
        let ctx = Context::default();
        let id = Id::new("p");
        at(&ctx, 0.0, |c| { press(c, id, true); });
        let mut peak = 0.0_f32;
        for f in 1..30 { at(&ctx, f as f64 / 60.0, |c| peak = peak.max(press(c, id, false))); }
        assert!(peak >= 0.5);
    }

    #[test]
    fn presence_reopens_from_current_value() {
        let ctx = Context::default();
        let id = Id::new("o");
        for f in 0..=60 { at(&ctx, f as f64 / 60.0, |c| { presence(c, id, true); }); }
        let mut mid = None;
        for f in 61..=63 { at(&ctx, f as f64 / 60.0, |c| mid = presence(c, id, false)); }
        let mut again = None;
        at(&ctx, 64.0 / 60.0, |c| again = presence(c, id, true));
        let (mid, again) = (mid.unwrap(), again.unwrap());
        assert!(mid > 0.2 && mid < 1.0 && (again - mid).abs() < 0.15);
        let mut gone = Some(1.0);
        for f in 65..=200 { at(&ctx, f as f64 / 60.0, |c| gone = presence(c, id, false)); }
        assert_eq!(gone, None);
    }

    #[test]
    fn presence_is_never_negative() {
        // A guard on the `max(0.0)` in `presence`: whatever momentum a close or a reopen
        // carries, the reported value never dips below 0 (it is a scale and an opacity).
        // Close until nearly gone, then reopen, at several depths.
        for threshold in [0.12, 0.08, 0.05, 0.03, 0.02, 0.01, 0.005] {
            let ctx = Context::default();
            let id = Id::new("neg");
            for f in 0..=60 { at(&ctx, f as f64 / 60.0, |c| { presence(c, id, true); }); }
            let mut f = 61;
            let mut value = 1.0;
            while value > threshold {
                at(&ctx, f as f64 / 120.0 + 0.5, |c| value = presence(c, id, false).unwrap_or(0.0));
                f += 1;
            }
            let mut lowest = f32::MAX;
            for _ in 0..60 {
                at(&ctx, f as f64 / 120.0 + 0.5, |c| lowest = lowest.min(presence(c, id, true).unwrap()));
                f += 1;
            }
            assert!(lowest >= 0.0, "threshold {threshold}: presence went negative: {lowest}");
        }
    }

    #[test]
    fn reduced_motion_resolves_positions_at_once() {
        let ctx = Context::default();
        let id = Id::new("m");
        at(&ctx, 0.0, |c| { set_reduced(c, true); spring(c, id, 0.0, GLIDE); });
        let mut v = 0.0;
        at(&ctx, 0.016, |c| v = spring(c, id, 1.0, GLIDE));
        assert_eq!(v, 1.0);
    }

    #[test]
    fn slow_motion_scales_the_clock() {
        let ctx = Context::default();
        let id = Id::new("t");
        at(&ctx, 0.0, |c| { set_time_scale(c, 10.0); spring(c, id, 0.0, GLIDE); });
        let mut v = 0.0;
        at(&ctx, 0.1, |c| v = spring(c, id, 1.0, GLIDE));
        assert!((v - step(GLIDE, 0.01)).abs() < 0.02);
    }
}
