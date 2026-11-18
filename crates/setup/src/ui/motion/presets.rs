//! The named motions. Views use these and never invent their own timings.
//!
//! GPUI has no blur filter and no transforms on non-SVG elements, so
//! opacity and offsets carry every transition.

use std::time::Duration;

use bach_pack::style::layout::{Animation, Arrangement};

use super::easing::{EASE_OUT, EASE_OUT_CSS};
use super::spring::Spring;
use super::value::{Motion, Tween};

/// The short rise of a page entering.
pub const RISE_SPRING: Spring = Spring::new(520.0, 38.0);

/// An entrance or exit made of an opacity fade and an offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transition {
    /// Opacity in and out.
    pub opacity: Tween,
    /// Where the element starts, px below (positive) or above its place.
    pub enter_y: f32,
    /// Where it ends up when leaving.
    pub exit_y: f32,
    /// Where it starts, px right (positive) or left of its place.
    pub enter_x: f32,
    /// How the offset moves.
    pub y: Motion,
    /// How fast the whole thing plays: 2 is twice as fast.
    pub speed: f32,
}

impl Transition {
    fn scaled(&self, elapsed: Duration) -> Duration {
        elapsed.mul_f32(self.speed.max(0.01))
    }

    /// The opacity and the (x, y) offset `elapsed` into the entrance.
    pub fn enter_at(&self, elapsed: Duration) -> (f32, (f32, f32)) {
        let elapsed = self.scaled(elapsed);
        let opacity = self.opacity.progress(elapsed);
        let left = 1.0 - motion_progress(self.y, elapsed);
        (opacity, (self.enter_x * left, self.enter_y * left))
    }

    /// The opacity and the (x, y) offset `elapsed` into the exit.
    pub fn exit_at(&self, elapsed: Duration) -> (f32, (f32, f32)) {
        let elapsed = self.scaled(elapsed);
        let opacity = 1.0 - self.opacity.progress(elapsed);
        (opacity, (0.0, self.exit_y * motion_progress(self.y, elapsed)))
    }

    /// How long until the entrance (or exit) has fully landed.
    pub fn duration(&self) -> Duration {
        let y = match self.y {
            Motion::Spring(spring) => spring.duration(),
            Motion::Tween(tween) => tween.duration,
            Motion::Instant => Duration::ZERO,
        };
        self.opacity.duration.max(y).div_f32(self.speed.max(0.01))
    }
}

/// Unit progress of a motion, keeping spring overshoot.
pub fn motion_progress(motion: Motion, elapsed: Duration) -> f32 {
    match motion {
        Motion::Instant => 1.0,
        Motion::Tween(tween) => tween.progress(elapsed),
        Motion::Spring(spring) => {
            if elapsed >= spring.duration() {
                1.0
            } else {
                1.0 - spring.evaluate(1.0, 0.0, elapsed.as_secs_f32()).0
            }
        }
    }
}

/// A page rising into place as it replaces the last one.
pub const PAGE_ENTER: Transition =
    Transition { opacity: Tween::new(220, EASE_OUT), enter_y: 6.0, exit_y: 6.0, enter_x: 0.0, y: Motion::Spring(RISE_SPRING), speed: 1.0 };

/// How a page comes in under `[layout] animation`, at its speed.
pub fn page_enter(layout: &Arrangement) -> Transition {
    let base = Transition { speed: layout.animation_speed, ..PAGE_ENTER };
    match layout.animation {
        Animation::Rise => base,
        Animation::Fade => Transition { enter_y: 0.0, exit_y: 0.0, ..base },
        Animation::Slide => Transition { enter_y: 0.0, exit_y: 0.0, enter_x: 28.0, ..base },
        Animation::Drop => Transition { enter_y: -14.0, exit_y: -6.0, ..base },
        Animation::None => Transition { opacity: Tween::new(0, EASE_OUT), enter_y: 0.0, exit_y: 0.0, y: Motion::Instant, ..base },
    }
}

/// One sweep of the shimmering status line.
pub const SHIMMER: Duration = Duration::from_millis(720);
/// One turn of the spinner.
pub const SPIN: Duration = Duration::from_secs(1);
/// Hover and press colour fades.
pub const COLOR_FADE: Tween = Tween::new(150, EASE_OUT_CSS);
