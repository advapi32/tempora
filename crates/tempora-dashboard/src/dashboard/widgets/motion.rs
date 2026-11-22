//! Motion: eased tweens for chart values, hover fades and page entrances.
//!
//! Every animation settles within half a second and then stops asking for frames, so an idle
//! window stays idle. All of them, egui's own included, turn off with Windows' "Animation
//! effects" setting (see [`set_enabled`]).

use eframe::egui::{Context, Id, Theme, emath::easing};

/// A chart's marks grow in this long the first time it is drawn.
pub(in crate::dashboard) const ENTER: f32 = 0.45;
/// Marks move to new values (another week, a filter) in this long.
pub(in crate::dashboard) const CHANGE: f32 = 0.3;
/// Hover washes and dimming fade in this long.
pub(in crate::dashboard) const HOVER: f32 = 0.14;
/// A page fades in this long.
pub(in crate::dashboard) const PAGE: f32 = 0.18;
/// Each further bar starts growing this much after the one before it.
pub(in crate::dashboard) const STAGGER: f32 = 0.018;
/// The longest stagger, so a 90-day chart doesn't take seconds to fill in.
const STAGGER_MAX: f32 = 0.2;
/// egui's default `animation_time`, restored when motion is turned back on.
const EGUI_ANIMATION: f32 = 1.0 / 12.0;

/// Whether anything animates. Off, every tween and fade returns its target at once.
pub(in crate::dashboard) fn enabled(ctx: &Context) -> bool {
    ctx.global_style().animation_time > 0.0
}

/// Turns motion on or off for the dashboard and for egui's own widgets, in both themes.
pub(in crate::dashboard) fn set_enabled(ctx: &Context, enabled: bool) {
    let time = if enabled { EGUI_ANIMATION } else { 0.0 };
    for theme in [Theme::Light, Theme::Dark] {
        ctx.style_mut_of(theme, |style| style.animation_time = time);
    }
}

/// How a tweened value moves.
#[derive(Clone, Copy, Debug)]
pub(in crate::dashboard) struct Ease {
    /// Where the value starts the first frame it is drawn, or `None` to start at its target.
    pub from: Option<f32>,
    /// Wait before the entrance starts.
    pub delay: f32,
    /// Length of the entrance.
    pub enter: f32,
    /// Length of a later move to a new target.
    pub change: f32,
}

impl Ease {
    /// Grows from zero on entrance and follows later changes.
    pub const GROW: Self = Self {
        from: Some(0.0),
        delay: 0.0,
        enter: ENTER,
        change: CHANGE,
    };

    /// Starts at its target and only animates later changes.
    pub const FOLLOW: Self = Self {
        from: None,
        delay: 0.0,
        enter: 0.0,
        change: CHANGE,
    };

    /// The entrance of the `index`th mark in a row, a little after the one before it.
    pub fn staggered(self, index: usize) -> Self {
        Self {
            delay: (index as f32 * STAGGER).min(STAGGER_MAX),
            ..self
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Tween {
    from: f32,
    to: f32,
    start: f64,
    duration: f32,
    /// The frame this value was last drawn in.
    frame: u64,
}

impl Tween {
    fn at(&self, now: f64) -> f32 {
        let t = if self.duration <= 0.0 {
            1.0
        } else {
            ((now - self.start) as f32 / self.duration).clamp(0.0, 1.0)
        };
        if t >= 1.0 {
            self.to
        } else {
            self.from + (self.to - self.from) * easing::cubic_out(t)
        }
    }

    fn done(&self, now: f64) -> bool {
        now >= self.start + f64::from(self.duration)
    }
}

/// Eases the value under `id` toward `target`. A value that wasn't drawn in the previous frame
/// is new: it enters from `ease.from`. Later targets are reached from wherever the value is.
pub(in crate::dashboard) fn tween(ctx: &Context, id: Id, target: f32, ease: Ease) -> f32 {
    if !enabled(ctx) {
        return target;
    }
    let now = ctx.input(|input| input.time);
    let frame = ctx.cumulative_frame_nr();
    let previous = ctx.data(|data| data.get_temp::<Tween>(id));
    let mut state = match previous {
        Some(state) if state.frame + 1 >= frame => state,
        _ => Tween {
            from: ease.from.unwrap_or(target),
            to: target,
            start: now + f64::from(ease.delay),
            duration: ease.enter,
            frame,
        },
    };
    if state.to != target {
        state = Tween {
            from: state.at(now),
            to: target,
            start: now,
            duration: ease.change,
            frame,
        };
    }
    state.frame = frame;
    ctx.data_mut(|data| data.insert_temp(id, state));
    if !state.done(now) {
        ctx.request_repaint();
    }
    state.at(now)
}

/// 0 to 1 as `on` turns true, eased out; back to 0 as it turns false. Starts settled.
pub(in crate::dashboard) fn fade(ctx: &Context, id: Id, on: bool, time: f32) -> f32 {
    let time = if enabled(ctx) { time } else { 0.0 };
    ctx.animate_bool_with_time_and_easing(id, on, time, easing::cubic_out)
}

pub(in crate::dashboard) fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::RawInput;

    /// Runs one frame at `time` seconds and returns what `draw` read.
    fn at<T>(ctx: &Context, time: f64, draw: impl FnOnce(&Context) -> T) -> T {
        let mut value = None;
        let mut draw = Some(draw);
        ctx.run_ui(
            RawInput {
                time: Some(time),
                ..RawInput::default()
            },
            |ui| value = draw.take().map(|draw| draw(ui.ctx())),
        )
        .textures_delta
        .clear();
        value.expect("drawn")
    }

    #[test]
    fn a_new_value_enters_then_follows_from_where_it_is() {
        let ctx = Context::default();
        let id = Id::new("bar");
        let value = |time, target| at(&ctx, time, |ctx| tween(ctx, id, target, Ease::GROW));
        assert_eq!(value(0.0, 1.0), 0.0, "enters from zero");
        let partway = value(0.2, 1.0);
        assert!(0.0 < partway && partway < 1.0, "{partway}");
        assert_eq!(value(1.0, 1.0), 1.0, "settles on the target");
        assert_eq!(
            value(1.0, 0.5),
            1.0,
            "a new target starts from the current value"
        );
        let moving = value(1.1, 0.5);
        assert!(0.5 < moving && moving < 1.0, "{moving}");
        assert_eq!(value(2.0, 0.5), 0.5);
    }

    #[test]
    fn a_value_missing_for_a_frame_enters_again() {
        let ctx = Context::default();
        let id = Id::new("bar");
        assert_eq!(at(&ctx, 0.0, |ctx| tween(ctx, id, 1.0, Ease::GROW)), 0.0);
        assert_eq!(at(&ctx, 1.0, |ctx| tween(ctx, id, 1.0, Ease::GROW)), 1.0);
        at(&ctx, 1.1, |_| ());
        assert_eq!(
            at(&ctx, 1.2, |ctx| tween(ctx, id, 1.0, Ease::GROW)),
            0.0,
            "shown again after another page"
        );
    }

    #[test]
    fn follow_starts_settled_and_staggered_entrances_wait() {
        let ctx = Context::default();
        assert_eq!(
            at(&ctx, 0.0, |ctx| tween(ctx, Id::new("a"), 7.0, Ease::FOLLOW)),
            7.0
        );
        let late = Ease::GROW.staggered(5);
        assert_eq!(
            at(&ctx, 0.0, |ctx| tween(ctx, Id::new("b"), 1.0, late)),
            0.0
        );
        assert_eq!(
            at(&ctx, late.delay as f64 - 0.01, |ctx| tween(
                ctx,
                Id::new("b"),
                1.0,
                late
            )),
            0.0,
            "still waiting its turn"
        );
        assert!(Ease::GROW.staggered(1_000).delay <= STAGGER_MAX);
    }

    #[test]
    fn turned_off_everything_is_at_its_target() {
        let ctx = Context::default();
        set_enabled(&ctx, false);
        let id = Id::new("bar");
        assert_eq!(at(&ctx, 0.0, |ctx| tween(ctx, id, 1.0, Ease::GROW)), 1.0);
        assert_eq!(at(&ctx, 0.0, |ctx| fade(ctx, id, true, HOVER)), 1.0);
        set_enabled(&ctx, true);
        assert!(enabled(&ctx));
    }
}
