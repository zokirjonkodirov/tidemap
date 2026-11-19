//! Motion: springs keyed by `egui::Id`, so any surface can animate a value without owning a
//! timer.
//!
//! Springs ([`spring`], [`hover`], [`press`], [`presence`]) carry velocity, so retargeting
//! mid-flight (a hover that reverses, a panel toggled twice) bends the motion instead of
//! restarting it. A spring snaps to its first target: nothing animates on first sight unless
//! asked. With reduced motion on ([`set_reduced`]) springs resolve at once and only
//! [`presence`] still fades, so surfaces do not blink in and out.

use egui::{Context, Id};
use std::f32::consts::TAU;

/// Decelerating curve with a soft landing (close to CSS `cubic-bezier(0.2, 0.8, 0.2, 1)`),
/// for effects timed by hand rather than by a spring.
pub fn ease_out(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u * u
}

/// The motion clock in seconds: input time divided by the slow-motion scale. Anything that
/// times an animation itself (rather than through a spring) must read this clock so that
/// `TIDEMAP_SLOWMO` slows it too.
pub fn now(ctx: &Context) -> f64 {
    let time = ctx.input(|input| input.time);
    time / f64::from(time_scale(ctx))
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

/// Marks this pass as animating for something that runs its own clock (a snapshot reveal)
/// rather than a spring: [`busy`] reports true and another frame is requested.
pub fn keep_busy(ctx: &Context) {
    mark_active(ctx);
}

/// Whether any spring moved in this pass or the previous one (screenshot mode waits
/// for this).
pub fn busy(ctx: &Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|data| data.get_temp::<u64>(Id::new(ACTIVE_PASS)))
        .is_some_and(|active| active + 1 >= pass)
}

/// Restarts the animation under `id` (for a surface that opens again): any [`spring`],
/// [`hover`], [`press`] or [`presence`] state keyed by `id` starts from scratch.
pub fn restart(ctx: &Context, id: Id) {
    ctx.data_mut(|data| {
        for suffix in SPRING_SUFFIXES {
            data.remove::<Stored>(spring_key(id, suffix));
        }
        data.remove::<PressMemo>(id.with("press-memo"));
    });
}

/// Whether a spring under `id` is still moving.
pub fn moving(ctx: &Context, id: Id) -> bool {
    ctx.data(|data| {
        SPRING_SUFFIXES
            .iter()
            .any(|suffix| data.get_temp::<Stored>(spring_key(id, suffix)).is_some_and(|stored| !stored.at_rest()))
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
///
/// With reduced motion on, the value ramps linearly to its target over [`REDUCED_FADE`]
/// seconds instead, so the surface still fades but neither pops nor overshoots.
pub fn presence(ctx: &Context, id: Id, open: bool) -> Option<f32> {
    let key = spring_key(id, "presence");
    let value = if reduced(ctx) {
        ramp_at(ctx, key, f32::from(u8::from(open)))
    } else {
        let (target, feel) = if open { (1.0, POP) } else { (0.0, SNAP) };
        spring_at(ctx, key, 0.0, target, feel).max(0.0)
    };
    (open || value > 0.0).then_some(value)
}

/// Seconds a [`presence`] takes to fade fully in or out under reduced motion.
pub const REDUCED_FADE: f32 = 0.15;

/// Moves the value stored under `key` toward `target` at a constant rate, taking
/// [`REDUCED_FADE`] seconds to cross 0 to 1. Starts at 0.
fn ramp_at(ctx: &Context, key: Id, target: f32) -> f32 {
    let time = now(ctx);
    let mut stored = ctx.data(|data| data.get_temp::<Stored>(key)).unwrap_or(Stored {
        state: SpringState::default(),
        target,
        last: time,
    });
    let reach = ((time - stored.last).max(0.0) as f32 / REDUCED_FADE).min(1.0);
    let gap = target - stored.state.value;
    stored.state = SpringState { value: stored.state.value + gap.clamp(-reach, reach), velocity: 0.0 };
    stored.target = target;
    stored.last = time;
    ctx.data_mut(|data| data.insert_temp(key, stored));
    if !stored.at_rest() {
        mark_active(ctx);
    }
    stored.state.value
}

/// Maps elapsed motion time `t` (seconds) to a 0-to-1 opacity ramp over 0.6 s. Opacity is
/// kept separate from position springs so overshoot never makes anything over-opaque.
pub fn fade(t: f32) -> f32 {
    (t / 0.6).clamp(0.0, 1.0)
}

const REDUCED: &str = "motion-reduced";
const TIME_SCALE: &str = "motion-time-scale";

/// Turns reduced motion on or off. When on, springs jump straight to their targets (and
/// [`presence`] fades briefly).
pub fn set_reduced(ctx: &Context, on: bool) {
    ctx.data_mut(|data| data.insert_temp(Id::new(REDUCED), on));
}

/// Whether reduced motion is on.
pub fn reduced(ctx: &Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(Id::new(REDUCED))).unwrap_or(false)
}

/// Whether the operating system asks for less animation. Windows reports it through
/// "Show animations in Windows"; other platforms report false.
#[cfg(windows)]
pub fn system_reduced() -> bool {
    use windows_sys::{
        Win32::UI::WindowsAndMessaging::{SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW},
        core::BOOL,
    };
    let mut animations: BOOL = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL through `pvParam`, which points at
    // a live, writable local for the whole call; `uiParam` and `fWinIni` are unused (0).
    let read = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut animations).cast(), 0) };
    read != 0 && animations == 0
}

/// Whether the operating system asks for less animation (always false on this platform).
#[cfg(not(windows))]
pub fn system_reduced() -> bool {
    false
}

/// What the last system read said, and whether the window was focused when it was taken.
#[derive(Clone, Copy)]
struct SystemRead {
    reduced: bool,
    focused: bool,
}

/// The system's reduced-motion flag without asking the OS every frame: `read` runs on the
/// first call and whenever the window regains focus (the moment a user returns from changing
/// the setting); every other frame returns the remembered answer.
pub fn system_preference(ctx: &Context, focused: bool, read: impl FnOnce() -> bool) -> bool {
    let key = Id::new("motion-system-read");
    let last = ctx.data(|data| data.get_temp::<SystemRead>(key));
    let reduced = match last {
        Some(last) if !focused || last.focused => last.reduced,
        _ => read(),
    };
    ctx.data_mut(|data| data.insert_temp(key, SystemRead { reduced, focused }));
    reduced
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

    #[test]
    fn system_flag_is_read_at_start_and_on_focus_gain_only() {
        let ctx = Context::default();
        let reads = std::cell::Cell::new(0);
        let ask = |focused: bool, answer: bool| {
            let mut out = false;
            at(&ctx, 0.0, |c| out = system_preference(c, focused, || { reads.set(reads.get() + 1); answer }));
            out
        };
        assert!(ask(true, true));
        assert!(ask(true, false), "still focused: remembered");
        assert!(ask(false, false), "lost focus: remembered");
        assert!(!ask(true, false), "regained focus: read again");
        assert_eq!(reads.get(), 2);
    }

    #[test]
    fn busy_is_true_while_a_spring_moves() {
        let ctx = Context::default();
        let id = Id::new("b");
        at(&ctx, 0.0, |c| { spring(c, id, 0.0, GLIDE); });
        let mut busy_now = false;
        at(&ctx, 0.016, |c| { spring(c, id, 1.0, GLIDE); busy_now = busy(c); });
        assert!(busy_now);
    }

    #[test]
    fn restart_clears_spring_state_and_moving_reports_motion() {
        let ctx = Context::default();
        let id = Id::new("rs");
        at(&ctx, 0.0, |c| { spring(c, id, 0.0, GLIDE); });
        at(&ctx, 0.016, |c| { spring(c, id, 1.0, GLIDE); });
        let mut was_moving = false;
        at(&ctx, 0.02, |c| was_moving = moving(c, id));
        assert!(was_moving);
        let mut v = 0.0;
        at(&ctx, 0.03, |c| { restart(c, id); v = spring(c, id, 1.0, GLIDE); });
        assert_eq!(v, 1.0, "restart must forget the old position");
        let mut still = true;
        at(&ctx, 0.04, |c| still = moving(c, id));
        assert!(!still);
    }

    #[test]
    fn presence_under_reduced_ramps_linearly_over_a_short_fade() {
        let ctx = Context::default();
        let id = Id::new("pr");
        at(&ctx, 0.0, |c| { set_reduced(c, true); presence(c, id, true); });
        let mut early = None;
        at(&ctx, 0.016, |c| early = presence(c, id, true));
        assert!(early.unwrap() < 0.5, "must not jump in one frame: {early:?}");
        let mut mid = None;
        at(&ctx, 0.075, |c| mid = presence(c, id, true));
        assert!((mid.unwrap() - 0.5).abs() < 0.1, "linear halfway: {mid:?}");
        let mut done = None;
        at(&ctx, 0.17, |c| done = presence(c, id, true));
        assert_eq!(done, Some(1.0));
        let mut closing = Some(1.0);
        at(&ctx, 0.2, |c| closing = presence(c, id, false));
        assert!(closing.unwrap() > 0.0);
        at(&ctx, 0.4, |c| closing = presence(c, id, false));
        assert_eq!(closing, None);
    }
}
