//! The maths behind a calm scroll: where a scrollbar's thumb sits, and how
//! the content glides to where the wheel sent it.
//!
//! Plain numbers, no GPUI, so the behaviour is tested without a window.
//! Nothing here runs unless something is moving: the glide asks for another
//! frame only until it has arrived, so a still page costs no CPU.

/// The gap above and below the thumb's travel.
pub const INSET: f32 = 6.;
/// A thumb never shrinks below this, so it stays easy to grab.
const MIN_THUMB: f32 = 24.;
/// How long the glide takes to cover about two thirds of what is left.
/// Short enough to feel direct, long enough to read as smooth.
const GLIDE_SECONDS: f32 = 0.075;
/// Closer than this to the goal, the glide is done (half a pixel).
const ARRIVED: f32 = 0.5;

/// The scrollbar's thumb, in pixels from the top of the card.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thumb {
    pub top: f32,
    pub height: f32,
    /// How far the thumb can travel from the top to the bottom.
    pub travel: f32,
    /// How far the content can scroll.
    pub hidden: f32,
}

impl Thumb {
    /// The thumb for a card `visible` tall whose content hides `hidden`
    /// more, scrolled down by `scrolled` (0 at the top). None when
    /// everything fits, so no bar is drawn.
    pub fn new(visible: f32, hidden: f32, scrolled: f32) -> Option<Self> {
        if hidden < 1. || visible < 1. {
            return None;
        }
        let height = (visible * visible / (visible + hidden)).max(MIN_THUMB).min(visible);
        let travel = (visible - INSET * 2. - height).max(0.);
        let scrolled = scrolled.clamp(0., hidden);
        Some(Self { top: INSET + travel * (scrolled / hidden), height, travel, hidden })
    }

    /// How far down the content is scrolled when the thumb's top is at
    /// `top` (dragging the thumb).
    pub fn scrolled_at(&self, top: f32) -> f32 {
        if self.travel <= 0. {
            return 0.;
        }
        ((top - INSET) / self.travel).clamp(0., 1.) * self.hidden
    }
}

/// The scroll position as the eye sees it, easing toward the one the wheel
/// asked for. Offsets follow GPUI: 0 at the top, negative further down.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Glide {
    pub current: f32,
    pub goal: f32,
}

impl Glide {
    /// Moves the goal by `delta` (a wheel notch), kept inside the content.
    pub fn push(&mut self, delta: f32, hidden: f32) {
        self.goal = (self.goal + delta).clamp(-hidden.max(0.), 0.);
    }

    /// Goes straight there, with no glide (dragging the thumb, Home, End).
    pub fn jump(&mut self, offset: f32, hidden: f32) {
        self.goal = offset.clamp(-hidden.max(0.), 0.);
        self.current = self.goal;
    }

    /// Takes the position the page really has (the content grew or the
    /// window resized), so the glide never fights the layout.
    pub fn follow(&mut self, actual: f32, hidden: f32) {
        let low = -hidden.max(0.);
        self.current = self.current.clamp(low, 0.);
        self.goal = self.goal.clamp(low, 0.);
        if (actual - self.current).abs() > 1. && self.idle() {
            self.current = actual.clamp(low, 0.);
            self.goal = self.current;
        }
    }

    pub fn idle(&self) -> bool {
        (self.goal - self.current).abs() < ARRIVED
    }

    /// Advances by `seconds` of wall time; true while still moving. The easing
    /// uses the time, not the frame count, so a slow frame doesn't slow the
    /// glide and a fast display doesn't speed it up.
    pub fn step(&mut self, seconds: f32) -> bool {
        if self.idle() {
            self.current = self.goal;
            return false;
        }
        let share = 1. - (-seconds.max(0.) / GLIDE_SECONDS).exp();
        self.current += (self.goal - self.current) * share;
        if self.idle() {
            self.current = self.goal;
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_thumb_when_everything_fits() {
        assert_eq!(Thumb::new(300., 0., 0.), None);
        assert_eq!(Thumb::new(0., 100., 0.), None);
    }

    #[test]
    fn the_thumb_is_proportional_and_never_tiny() {
        let half = Thumb::new(300., 300., 0.).unwrap();
        assert!((half.height - 150.).abs() < 0.01);
        let long = Thumb::new(300., 100_000., 0.).unwrap();
        assert_eq!(long.height, MIN_THUMB);
    }

    #[test]
    fn the_thumb_travels_from_top_to_bottom() {
        let top = Thumb::new(300., 600., 0.).unwrap();
        let bottom = Thumb::new(300., 600., 600.).unwrap();
        assert_eq!(top.top, INSET);
        assert!((bottom.top + bottom.height - (300. - INSET)).abs() < 0.01);
        // Past the end is held at the end.
        assert_eq!(Thumb::new(300., 600., 9_000.).unwrap().top, bottom.top);
    }

    #[test]
    fn dragging_the_thumb_maps_back_to_a_scroll_position() {
        let thumb = Thumb::new(300., 600., 0.).unwrap();
        assert_eq!(thumb.scrolled_at(-50.), 0.);
        assert_eq!(thumb.scrolled_at(10_000.), 600.);
        let middle = thumb.scrolled_at(INSET + thumb.travel / 2.);
        assert!((middle - 300.).abs() < 0.01);
    }

    #[test]
    fn the_glide_arrives_without_overshooting() {
        let mut glide = Glide::default();
        glide.push(-120., 1_000.);
        let mut last = glide.current;
        let mut frames = 0;
        while glide.step(1. / 60.) {
            assert!(glide.current <= last, "moves one way only");
            assert!(glide.current >= glide.goal, "never overshoots");
            last = glide.current;
            frames += 1;
            assert!(frames < 120, "settles within two seconds");
        }
        assert_eq!(glide.current, -120.);
    }

    #[test]
    fn the_glide_takes_the_same_time_at_any_frame_rate() {
        let run = |dt: f32| {
            let mut glide = Glide::default();
            glide.push(-200., 1_000.);
            for _ in 0..(0.1 / dt) as usize {
                glide.step(dt);
            }
            glide.current
        };
        assert!((run(1. / 60.) - run(1. / 240.)).abs() < 2.);
    }

    #[test]
    fn pushing_stays_inside_the_content() {
        let mut glide = Glide::default();
        glide.push(50., 100.);
        assert_eq!(glide.goal, 0.);
        glide.push(-500., 100.);
        assert_eq!(glide.goal, -100.);
        glide.jump(-30., 100.);
        assert_eq!((glide.current, glide.goal), (-30., -30.));
    }
}
