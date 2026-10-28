//! Transitions: when an element's color changes, it can fade to the new one instead of jumping.
//!
//! An element's `style` always holds where a color is going, and the canvas works out what is shown right now each
//! frame ([`ColorAnim`]). Setting the style needs nothing special: the next frame sees that the target moved and
//! starts a fade from whatever was showing, so changing it again halfway doesn't jump.

use crate::style::{Color, Easing};

/// How a color property changes when its value does. A duration of 0 (the default) means it just changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transition {
    /// How long the change takes, in seconds.
    pub duration: f32,
    pub easing: Easing,
}

impl Default for Transition {
    fn default() -> Transition {
        Transition { duration: 0.0, easing: Easing::Ease }
    }
}

/// CSS's `cubic-bezier(x1, y1, x2, y2)`: solves the curve's x(s) = x for s, then returns y(s).
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let sample = |a: f32, b: f32, s: f32| {
        let inverse = 1.0 - s;
        3.0 * inverse * inverse * s * a + 3.0 * inverse * s * s * b + s * s * s
    };
    let slope = |a: f32, b: f32, s: f32| {
        let inverse = 1.0 - s;
        3.0 * inverse * inverse * a + 6.0 * inverse * s * (b - a) + 3.0 * s * s * (1.0 - b)
    };

    // Newton's method finds s, and a few rounds are plenty for a smooth curve.
    let mut s = x;
    for _ in 0..8 {
        let error = sample(x1, x2, s) - x;
        let derivative = slope(x1, x2, s);
        if error.abs() < 1e-5 || derivative.abs() < 1e-6 {
            break;
        }
        s -= error / derivative;
    }
    sample(y1, y2, s.clamp(0.0, 1.0))
}

/// How far along the change is (0.0 to 1.0) given how much of the time has passed (0.0 to 1.0).
pub fn ease(easing: Easing, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match easing {
        Easing::Linear => t,
        Easing::Ease => cubic_bezier(0.25, 0.1, 0.25, 1.0, t),
        Easing::EaseIn => cubic_bezier(0.42, 0.0, 1.0, 1.0, t),
        Easing::EaseOut => cubic_bezier(0.0, 0.0, 0.58, 1.0, t),
        Easing::EaseInOut => cubic_bezier(0.42, 0.0, 0.58, 1.0, t),
    }
}

/// Mixes two colors, `t` of the way from `a` to `b`. This is done on premultiplied colors, so fading from a
/// transparent color doesn't pass through a smear of whatever color transparent happened to have.
pub fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let premultiplied = |c: Color| [f32::from(c.r) * c.a, f32::from(c.g) * c.a, f32::from(c.b) * c.a, c.a];
    let (a, b) = (premultiplied(a), premultiplied(b));
    let mixed: Vec<f32> = (0..4).map(|i| a[i] + (b[i] - a[i]) * t).collect();
    let alpha = mixed[3];
    if alpha <= 0.0 {
        return Color { r: 0, g: 0, b: 0, a: 0.0 };
    }
    let channel = |v: f32| (v / alpha).round().clamp(0.0, 255.0) as u8;
    Color { r: channel(mixed[0]), g: channel(mixed[1]), b: channel(mixed[2]), a: alpha.clamp(0.0, 1.0) }
}

/// One color property's journey from where it was to where it is going.
#[derive(Clone, Copy, Debug, Default)]
pub struct ColorAnim {
    from: Option<Color>,
    to: Option<Color>,
    /// When it set off, in seconds on the canvas's clock.
    started: f64,
}

impl ColorAnim {
    /// What is showing at `now`, given where the property is going (`target`) and how it is allowed to get there.
    /// The first call shows `target` straight away, since there is nothing to change from. If `target` has moved
    /// since last time, a new change starts from what was showing, however far the last one had got.
    pub fn update(&mut self, target: Color, now: f64, transition: &Transition) -> Color {
        let Some(to) = self.to else {
            *self = ColorAnim { from: Some(target), to: Some(target), started: now };
            return target;
        };
        if to != target {
            let showing = self.value(now, transition);
            *self = ColorAnim { from: Some(if transition.duration > 0.0 { showing } else { target }), to: Some(target), started: now };
        }
        self.value(now, transition)
    }

    /// Forgets everything, so the next `update` shows its target at once.
    pub fn reset(&mut self) {
        *self = ColorAnim::default();
    }

    fn progress(&self, now: f64, transition: &Transition) -> f32 {
        if transition.duration <= 0.0 {
            return 1.0;
        }
        (((now - self.started) / f64::from(transition.duration)) as f32).clamp(0.0, 1.0)
    }

    fn value(&self, now: f64, transition: &Transition) -> Color {
        match (self.from, self.to) {
            (Some(from), Some(to)) => {
                let t = self.progress(now, transition);
                if t >= 1.0 { to } else { lerp_color(from, to, ease(transition.easing, t)) }
            }
            _ => Color::TRANSPARENT,
        }
    }

    /// Whether it hasn't arrived at its target yet.
    pub fn is_moving(&self, now: f64, transition: &Transition) -> bool {
        self.from != self.to && self.progress(now, transition) < 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = Color { r: 255, g: 0, b: 0, a: 1.0 };
    const BLUE: Color = Color { r: 0, g: 0, b: 255, a: 1.0 };
    const FADE: Transition = Transition { duration: 1.0, easing: Easing::Linear };

    #[test]
    fn easing_curves_start_at_0_end_at_1_and_go_forward() {
        for easing in [Easing::Linear, Easing::Ease, Easing::EaseIn, Easing::EaseOut, Easing::EaseInOut] {
            assert!(ease(easing, 0.0).abs() < 1e-3 && (ease(easing, 1.0) - 1.0).abs() < 1e-3, "{easing:?}");
            let samples: Vec<f32> = (0..=20).map(|i| ease(easing, i as f32 / 20.0)).collect();
            assert!(samples.windows(2).all(|w| w[1] >= w[0] - 1e-4), "{easing:?} never goes back: {samples:?}");
        }
        assert_eq!(ease(Easing::Linear, 0.25), 0.25);
        assert!(ease(Easing::EaseOut, 0.3) > 0.3, "ease-out is quick at first");
        assert!(ease(Easing::EaseIn, 0.3) < 0.3, "ease-in is slow at first");
        let in_out = (ease(Easing::EaseInOut, 0.2), ease(Easing::EaseInOut, 0.8));
        assert!(in_out.0 < 0.2 && in_out.1 > 0.8, "ease-in-out is slow at both ends");
    }

    #[test]
    fn time_outside_the_range_is_clamped() {
        assert_eq!((ease(Easing::Ease, -1.0), ease(Easing::Ease, 5.0)), (0.0, 1.0));
    }

    #[test]
    fn colors_mix_halfway() {
        let mid = lerp_color(RED, BLUE, 0.5);
        assert_eq!((mid.r, mid.g, mid.b, mid.a), (128, 0, 128, 1.0));
        assert_eq!(lerp_color(RED, BLUE, 0.0), RED);
        assert_eq!(lerp_color(RED, BLUE, 1.0), BLUE);
    }

    #[test]
    fn fading_from_transparent_keeps_the_target_color() {
        // Black and transparent are what "no color" is stored as. Halfway to opaque white is half-opaque white,
        // not gray, which is what mixing the straight values would give.
        let mid = lerp_color(Color::TRANSPARENT, Color::WHITE, 0.5);
        assert_eq!((mid.r, mid.g, mid.b), (255, 255, 255));
        assert!((mid.a - 0.5).abs() < 1e-3);
    }

    #[test]
    fn the_first_value_shows_at_once() {
        let mut anim = ColorAnim::default();
        assert_eq!(anim.update(RED, 10.0, &FADE), RED);
        assert!(!anim.is_moving(10.0, &FADE));
    }

    #[test]
    fn a_new_target_fades_over_the_duration() {
        let mut anim = ColorAnim::default();
        anim.update(RED, 0.0, &FADE);
        let start = anim.update(BLUE, 1.0, &FADE); // The target moved at t = 1.
        assert_eq!(start, RED);
        assert!(anim.is_moving(1.0, &FADE));

        let half = anim.update(BLUE, 1.5, &FADE);
        assert_eq!((half.r, half.b), (128, 128));
        assert_eq!(anim.update(BLUE, 2.0, &FADE), BLUE, "arrived");
        assert!(!anim.is_moving(2.0, &FADE));
        assert_eq!(anim.update(BLUE, 99.0, &FADE), BLUE);
    }

    #[test]
    fn changing_the_target_mid_fade_starts_from_what_was_showing() {
        let mut anim = ColorAnim::default();
        anim.update(RED, 0.0, &FADE);
        anim.update(BLUE, 0.0, &FADE);
        let showing = anim.update(BLUE, 0.5, &FADE); // Halfway to blue.
        let after = anim.update(RED, 0.5, &FADE); // Sent back the other way at that moment.
        assert_eq!(after, showing, "no jump when it turns around");
    }

    #[test]
    fn with_no_duration_a_change_is_immediate() {
        let none = Transition::default();
        let mut anim = ColorAnim::default();
        anim.update(RED, 0.0, &none);
        assert_eq!(anim.update(BLUE, 0.1, &none), BLUE);
        assert!(!anim.is_moving(0.1, &none));
    }

    #[test]
    fn resetting_makes_the_next_value_immediate() {
        let mut anim = ColorAnim::default();
        anim.update(RED, 0.0, &FADE);
        anim.reset();
        assert_eq!(anim.update(BLUE, 5.0, &FADE), BLUE);
    }
}
