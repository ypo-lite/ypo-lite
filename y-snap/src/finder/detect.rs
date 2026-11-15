//! Recognising the finder's automatic triggers: a modifier tapped quickly
//! ([`Taps`]) and a shaken pointer ([`Shakes`]).
//!
//! Pure logic, no Win32: the hooks in [`crate::finder::input`] feed events in
//! and post a message when one of these says "fire". That keeps the part worth
//! testing testable, and the hooks cheap — nothing here allocates once built,
//! takes a lock, or calls the system, as a low-level hook has to stay under
//! `LowLevelHooksTimeout`.
//!
//! The rules and numbers were tuned live with a probe program (since removed)
//! (see `docs/design/mouse-finder-plan.md`). Times are the millisecond ticks
//! of the hook events, compared with wrapping arithmetic; distances are in
//! physical pixels.

use std::f32::consts::PI;

use crate::config::{Shake, ShakeShape, Side, Tap, TapKey};

fn elapsed(now: u32, then: u32) -> u32 {
    now.wrapping_sub(then)
}

// ------------------------------------------------------------------- taps

/// The progress of one `<tap>` trigger.
#[derive(Clone, Copy, Default)]
struct Chain {
    /// When the matching key went down and is still down.
    down_at: Option<u32>,
    /// Something else happened while it was down: this press is not a tap.
    spoiled: bool,
    /// Taps counted so far, and when the last one went down.
    count: u32,
    last_down: u32,
}

/// Counts the quick taps of modifier keys.
///
/// A tap is a press released within `hold`; taps count together while each
/// press follows the previous one within `gap` (down to down). Any other key
/// or a mouse button, pressed in between or while the key is down, starts
/// over — `Ctrl+C` is not a tap — and so does a press held too long, which is
/// what separates a tap from "press, hold, press again". The final tap fires
/// when its key is released, so a held key can never fire.
pub struct Taps {
    triggers: Vec<(u16, Tap)>,
    chains: Vec<Chain>,
}

/// A press this old with no release seen is a lost release, not a held key
/// (a held key repeats, which is ignored, but never goes quiet).
const STALE_MS: u32 = 1000;

impl Taps {
    pub fn new(triggers: Vec<(u16, Tap)>) -> Self {
        let chains = vec![Chain::default(); triggers.len()];
        Self { triggers, chains }
    }

    pub fn is_empty(&self) -> bool {
        self.triggers.is_empty()
    }

    /// A modifier went down or up. `right` is the right-hand key of the pair.
    /// The style to play when this completes a trigger.
    pub fn key(&mut self, key: TapKey, right: bool, down: bool, time: u32) -> Option<u16> {
        let mut fired = None;
        for (index, (style, tap)) in self.triggers.iter().enumerate() {
            let matches = tap.key == key
                && match tap.side {
                    Side::Any => true,
                    Side::Left => !right,
                    Side::Right => right,
                };
            let chain = &mut self.chains[index];
            if !matches {
                // Another modifier, or the other hand: not part of this tap.
                *chain = Chain::default();
                continue;
            }
            if down {
                match chain.down_at {
                    // An auto-repeat of a key still held.
                    Some(at) if elapsed(time, at) <= STALE_MS => {}
                    _ => {
                        chain.down_at = Some(time);
                        chain.spoiled = false;
                    }
                }
                continue;
            }
            let Some(at) = chain.down_at.take() else {
                continue;
            };
            if chain.spoiled || elapsed(time, at) > tap.max_hold_ms {
                *chain = Chain::default();
                continue;
            }
            if chain.count > 0 && elapsed(at, chain.last_down) > tap.max_gap_ms {
                chain.count = 0;
            }
            chain.count += 1;
            chain.last_down = at;
            if chain.count >= tap.count {
                fired = Some(*style);
            }
        }
        if fired.is_some() {
            self.reset();
        }
        fired
    }

    /// Any other key, or a mouse button, went down: nothing counted so far
    /// stands, and a press in progress is spoiled.
    pub fn other(&mut self) {
        for chain in &mut self.chains {
            let pressed = chain.down_at;
            *chain = Chain::default();
            if pressed.is_some() {
                chain.down_at = pressed;
                chain.spoiled = true;
            }
        }
    }

    fn reset(&mut self) {
        self.chains.fill(Chain::default());
    }
}

// ----------------------------------------------------------------- shakes

#[derive(Clone, Copy, Default)]
struct Sample {
    x: i32,
    y: i32,
    time: u32,
}

/// Room for the longest window (1.5 s) at one sample per [`MIN_STEP_MS`].
const CAPACITY: usize = 320;
/// Samples closer in time than this are merged: a fast mouse reports 1000
/// times a second, which adds nothing to a shape drawn in a few hundred ms.
const MIN_STEP_MS: u32 = 5;
/// The path is judged at most this often.
const EVAL_STEP_MS: u32 = 10;
/// The path is decimated to steps of this many pixels, so that jitter does
/// not count as turning.
const STEP_PX: f32 = 20.0;
/// A corner turning more than this is a reversal, not a curve.
const SHARP: f32 = 120.0 * PI / 180.0;
/// Fewer samples than this say nothing.
const MIN_SAMPLES: usize = 5;

/// What the system says at the moment a shake is judged.
#[derive(Clone, Copy, Default)]
pub struct Held {
    pub modifiers: u32,
    /// A mouse button is down: that is a drag, never a shake.
    pub button: bool,
}

/// Watches the pointer for shakes.
pub struct Shakes {
    triggers: Vec<(u16, Shake)>,
    ring: Vec<Sample>,
    /// Index of the oldest sample, and how many there are.
    head: usize,
    len: usize,
    longest_ms: u32,
    last_eval: u32,
    /// After a detection: since when, and for how long, nothing is looked at.
    quiet: Option<(u32, u32)>,
}

impl Shakes {
    pub fn new(triggers: Vec<(u16, Shake)>) -> Self {
        let longest_ms = triggers.iter().map(|(_, s)| s.window_ms).max().unwrap_or(0);
        Self {
            triggers,
            ring: vec![Sample::default(); CAPACITY],
            head: 0,
            len: 0,
            longest_ms,
            last_eval: 0,
            quiet: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.triggers.is_empty()
    }

    fn at(&self, index: usize) -> Sample {
        self.ring[(self.head + index) % CAPACITY]
    }

    fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }

    fn push(&mut self, sample: Sample) {
        if self.len > 0 {
            let last = (self.head + self.len - 1) % CAPACITY;
            if elapsed(sample.time, self.ring[last].time) < MIN_STEP_MS {
                // Same moment: keep where the pointer is now.
                self.ring[last].x = sample.x;
                self.ring[last].y = sample.y;
                return;
            }
        }
        while self.len > 0 && elapsed(sample.time, self.at(0).time) > self.longest_ms {
            self.head = (self.head + 1) % CAPACITY;
            self.len -= 1;
        }
        if self.len == CAPACITY {
            self.head = (self.head + 1) % CAPACITY;
            self.len -= 1;
        }
        let slot = (self.head + self.len) % CAPACITY;
        self.ring[slot] = sample;
        self.len += 1;
    }

    /// The pointer moved to `(x, y)`. `sense` reads the modifiers and buttons,
    /// and is only called when the path is actually judged. The style to play
    /// when this completes a shake.
    pub fn on_move(
        &mut self,
        x: i32,
        y: i32,
        time: u32,
        sense: impl FnOnce() -> Held,
    ) -> Option<u16> {
        if let Some((since, length)) = self.quiet {
            if elapsed(time, since) < length {
                return None;
            }
            self.quiet = None;
        }
        self.push(Sample { x, y, time });
        if elapsed(time, self.last_eval) < EVAL_STEP_MS || self.len < MIN_SAMPLES {
            return None;
        }
        self.last_eval = time;

        let held = sense();
        if held.button {
            self.clear();
            return None;
        }

        for index in 0..self.triggers.len() {
            let (style, shake) = self.triggers[index];
            if shake.modifiers.0 != held.modifiers {
                continue;
            }
            if self.judge(&shake, time) {
                self.clear();
                self.quiet = Some((time, shake.cooldown_ms));
                return Some(style);
            }
        }
        None
    }

    /// Whether the recent path is a shake of a kind `shake` wants.
    fn judge(&self, shake: &Shake, now: u32) -> bool {
        // The samples inside this trigger's window, oldest first.
        let mut window = [Sample::default(); CAPACITY];
        let mut count = 0;
        for i in 0..self.len {
            let sample = self.at(i);
            if elapsed(now, sample.time) <= shake.window_ms {
                window[count] = sample;
                count += 1;
            }
        }
        if count < MIN_SAMPLES {
            return false;
        }
        let window = &window[..count];

        let duration = elapsed(window[count - 1].time, window[0].time).max(1) as f32;
        let path: f32 = window
            .windows(2)
            .map(|p| {
                (((p[1].x - p[0].x) as f32).powi(2) + ((p[1].y - p[0].y) as f32).powi(2)).sqrt()
            })
            .sum();
        let speed = path * 1000.0 / duration;
        let turns = Turns::of(window);

        let circle = shake.has(ShakeShape::Circle)
            && speed >= shake.circle_speed as f32
            && turns.smooth.abs() >= (shake.circle_degrees as f32).to_radians()
            && turns.sharp <= 1;
        if circle {
            return true;
        }
        if speed < shake.speed as f32 {
            return false;
        }
        let segment = shake.segment as i32;
        if shake.has(ShakeShape::Horizontal)
            && reversals(window.iter().map(|s| s.x), segment) >= shake.reversals
            && turns.path_x >= shake.dominance * turns.path_y
        {
            return true;
        }
        shake.has(ShakeShape::Vertical)
            && reversals(window.iter().map(|s| s.y), segment) >= shake.reversals
            && turns.path_y >= shake.dominance * turns.path_x
    }
}

/// What the shape of the path says, beyond axis reversals.
struct Turns {
    /// Signed turning (radians) summed over *smooth* corners only — a circle
    /// is ±2π. A back-and-forth reversal is a sharp corner and is not added:
    /// it would otherwise accumulate noise into a fake "circle".
    smooth: f32,
    /// Corners sharper than [`SHARP`].
    sharp: u32,
    /// Distance travelled along each axis.
    path_x: f32,
    path_y: f32,
}

impl Turns {
    fn of(samples: &[Sample]) -> Self {
        let mut points = [(0.0f32, 0.0f32); CAPACITY];
        let mut count = 0;
        for s in samples {
            let p = (s.x as f32, s.y as f32);
            let far = count == 0 || {
                let last = points[count - 1];
                ((p.0 - last.0).powi(2) + (p.1 - last.1).powi(2)).sqrt() >= STEP_PX
            };
            if far {
                points[count] = p;
                count += 1;
            }
        }

        let mut result = Self {
            smooth: 0.0,
            sharp: 0,
            path_x: 0.0,
            path_y: 0.0,
        };
        let mut previous: Option<f32> = None;
        for pair in points[..count].windows(2) {
            result.path_x += (pair[1].0 - pair[0].0).abs();
            result.path_y += (pair[1].1 - pair[0].1).abs();
            let angle = (pair[1].1 - pair[0].1).atan2(pair[1].0 - pair[0].0);
            if let Some(before) = previous {
                let mut delta = angle - before;
                while delta > PI {
                    delta -= 2.0 * PI;
                }
                while delta < -PI {
                    delta += 2.0 * PI;
                }
                if delta.abs() >= SHARP {
                    result.sharp += 1;
                } else {
                    result.smooth += delta;
                }
            }
            previous = Some(angle);
        }
        result
    }
}

/// Direction reversals along one axis: a reversal counts once the pointer has
/// travelled `segment` pixels back from the last extreme.
fn reversals(values: impl Iterator<Item = i32>, segment: i32) -> u32 {
    let mut direction = 0;
    let mut extreme = None::<i32>;
    let mut count = 0;
    for value in values {
        let Some(e) = extreme else {
            extreme = Some(value);
            continue;
        };
        match direction {
            0 => {
                if value - e >= segment {
                    direction = 1;
                    extreme = Some(value);
                } else if e - value >= segment {
                    direction = -1;
                    extreme = Some(value);
                }
            }
            1 => {
                if value > e {
                    extreme = Some(value);
                } else if e - value >= segment {
                    direction = -1;
                    count += 1;
                    extreme = Some(value);
                }
            }
            _ => {
                if value < e {
                    extreme = Some(value);
                } else if value - e >= segment {
                    direction = 1;
                    count += 1;
                    extreme = Some(value);
                }
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(modifiers: HOT_KEY_MODIFIERS) -> Held {
        Held {
            modifiers: modifiers.0,
            button: false,
        }
    }

    use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, MOD_CONTROL};

    const NONE: HOT_KEY_MODIFIERS = HOT_KEY_MODIFIERS(0);

    fn ctrl_double() -> Taps {
        Taps::new(vec![(7, Tap::new(TapKey::Ctrl))])
    }

    /// A press and a release `hold` ms later.
    fn tap(taps: &mut Taps, key: TapKey, at: u32, hold: u32) -> Option<u16> {
        taps.key(key, false, true, at);
        taps.key(key, false, false, at + hold)
    }

    #[test]
    fn two_quick_taps_fire() {
        let mut taps = ctrl_double();
        assert_eq!(tap(&mut taps, TapKey::Ctrl, 1000, 80), None);
        assert_eq!(tap(&mut taps, TapKey::Ctrl, 1250, 80), Some(7));
        // It starts over afterwards.
        assert_eq!(tap(&mut taps, TapKey::Ctrl, 1600, 80), None);
    }

    #[test]
    fn a_press_held_too_long_is_not_a_tap() {
        let mut taps = ctrl_double();
        // Press, hold, release, press again: what Windows would count.
        assert_eq!(tap(&mut taps, TapKey::Ctrl, 1000, 600), None);
        assert_eq!(tap(&mut taps, TapKey::Ctrl, 1700, 80), None);
        // The second one is where a new count begins.
        assert_eq!(tap(&mut taps, TapKey::Ctrl, 1900, 80), Some(7));
    }

    #[test]
    fn taps_too_far_apart_do_not_add_up() {
        let mut taps = ctrl_double();
        assert_eq!(tap(&mut taps, TapKey::Ctrl, 1000, 80), None);
        assert_eq!(tap(&mut taps, TapKey::Ctrl, 1600, 80), None);
        assert_eq!(tap(&mut taps, TapKey::Ctrl, 1900, 80), Some(7));
    }

    #[test]
    fn another_key_in_between_or_during_starts_over() {
        let mut taps = ctrl_double();
        tap(&mut taps, TapKey::Ctrl, 1000, 80);
        taps.other();
        assert_eq!(tap(&mut taps, TapKey::Ctrl, 1200, 80), None);

        // Ctrl+C: the press is spoiled, and so is the count.
        let mut taps = ctrl_double();
        tap(&mut taps, TapKey::Ctrl, 1000, 80);
        taps.key(TapKey::Ctrl, false, true, 1150);
        taps.other();
        assert_eq!(taps.key(TapKey::Ctrl, false, false, 1220), None);
        assert_eq!(tap(&mut taps, TapKey::Ctrl, 1300, 80), None);
    }

    #[test]
    fn another_modifier_spoils_the_taps() {
        let mut taps = ctrl_double();
        tap(&mut taps, TapKey::Ctrl, 1000, 80);
        // Ctrl+Shift: Shift pressed while Ctrl is down.
        taps.key(TapKey::Ctrl, false, true, 1150);
        taps.key(TapKey::Shift, false, true, 1170);
        taps.key(TapKey::Shift, false, false, 1200);
        assert_eq!(taps.key(TapKey::Ctrl, false, false, 1220), None);
    }

    #[test]
    fn auto_repeat_never_fires() {
        let mut taps = ctrl_double();
        taps.key(TapKey::Ctrl, false, true, 1000);
        for i in 1..20 {
            taps.key(TapKey::Ctrl, false, true, 1000 + 500 + i * 33);
        }
        assert_eq!(taps.key(TapKey::Ctrl, false, false, 2300), None);
    }

    #[test]
    fn a_side_and_a_count_can_be_asked_for() {
        let mut right = Tap::new(TapKey::Shift);
        right.side = Side::Right;
        right.count = 3;
        let mut taps = Taps::new(vec![(1, right)]);
        let mut press = |at: u32, right: bool| {
            taps.key(TapKey::Shift, right, true, at);
            taps.key(TapKey::Shift, right, false, at + 60)
        };
        assert_eq!(press(1000, false), None);
        assert_eq!(press(1200, true), None);
        assert_eq!(press(1400, true), None);
        assert_eq!(press(1600, true), Some(1));
    }

    /// A pointer path sampled every 8 ms starting at `t0`.
    fn path(
        shakes: &mut Shakes,
        modifiers: HOT_KEY_MODIFIERS,
        t0: u32,
        steps: u32,
        at: impl Fn(f32) -> (f32, f32),
    ) -> Option<u16> {
        let mut fired = None;
        for i in 0..steps {
            let t = t0 + i * 8;
            let (x, y) = at((t - t0) as f32 / 1000.0);
            if let Some(style) = shakes.on_move(x as i32, y as i32, t, || held(modifiers)) {
                fired.get_or_insert(style);
            }
        }
        fired
    }

    /// Back and forth: 6 half-periods of 300 px in 0.4 s (4500 px/s).
    fn back_and_forth(t: f32) -> (f32, f32) {
        let phase = (t * 7.5).fract();
        let x = if phase < 0.5 {
            phase * 2.0
        } else {
            2.0 - phase * 2.0
        };
        (500.0 + x * 300.0, 400.0)
    }

    fn circle(t: f32) -> (f32, f32) {
        let angle = t * 15.7;
        (800.0 + 200.0 * angle.cos(), 500.0 + 200.0 * angle.sin())
    }

    #[test]
    fn a_horizontal_shake_fires_and_a_slow_sweep_does_not() {
        let mut shakes = Shakes::new(vec![(3, Shake::new(NONE))]);
        assert_eq!(path(&mut shakes, NONE, 10_000, 90, back_and_forth), Some(3));

        let mut shakes = Shakes::new(vec![(3, Shake::new(NONE))]);
        // Across the screen in a second: fast enough, but one direction.
        let sweep = |t: f32| (t * 1800.0, 300.0 + t * 100.0);
        assert_eq!(path(&mut shakes, NONE, 10_000, 120, sweep), None);
    }

    #[test]
    fn a_circle_fires_only_when_asked_for() {
        let mut only_circle = Shake::new(NONE);
        only_circle.shapes = [false, false, true];
        let mut shakes = Shakes::new(vec![(5, only_circle)]);
        assert_eq!(path(&mut shakes, NONE, 10_000, 120, circle), Some(5));

        let mut only_horizontal = Shake::new(NONE);
        only_horizontal.shapes = [true, false, false];
        let mut shakes = Shakes::new(vec![(5, only_horizontal)]);
        assert_eq!(path(&mut shakes, NONE, 10_000, 120, circle), None);

        let mut only_circle = Shake::new(NONE);
        only_circle.shapes = [false, false, true];
        let mut shakes = Shakes::new(vec![(5, only_circle)]);
        assert_eq!(path(&mut shakes, NONE, 10_000, 90, back_and_forth), None);
    }

    #[test]
    fn a_trigger_wants_exactly_its_modifiers() {
        let mut shakes = Shakes::new(vec![(1, Shake::new(MOD_CONTROL))]);
        assert_eq!(path(&mut shakes, NONE, 10_000, 90, back_and_forth), None);
        let mut shakes = Shakes::new(vec![(1, Shake::new(MOD_CONTROL))]);
        assert_eq!(
            path(&mut shakes, MOD_CONTROL, 10_000, 90, back_and_forth),
            Some(1)
        );
        // Strict without a modifier: Ctrl held means the other trigger's.
        let mut shakes = Shakes::new(vec![(2, Shake::new(NONE))]);
        assert_eq!(
            path(&mut shakes, MOD_CONTROL, 10_000, 90, back_and_forth),
            None
        );
    }

    #[test]
    fn a_button_down_is_a_drag_not_a_shake() {
        let mut shakes = Shakes::new(vec![(3, Shake::new(NONE))]);
        let mut fired = None;
        for i in 0..90u32 {
            let t = 10_000 + i * 8;
            let (x, y) = back_and_forth((t - 10_000) as f32 / 1000.0);
            let down = Held {
                modifiers: 0,
                button: true,
            };
            fired = fired.or(shakes.on_move(x as i32, y as i32, t, || down));
        }
        assert_eq!(fired, None);
    }

    #[test]
    fn after_a_detection_it_is_quiet_for_the_cooldown() {
        let mut shakes = Shakes::new(vec![(3, Shake::new(NONE))]);
        let mut count = 0;
        // Two seconds of continuous shaking: the cooldown is one second.
        for i in 0..250u32 {
            let t = 10_000 + i * 8;
            let (x, y) = back_and_forth((t - 10_000) as f32 / 1000.0);
            if shakes
                .on_move(x as i32, y as i32, t, || held(NONE))
                .is_some()
            {
                count += 1;
            }
        }
        assert!((2..=3).contains(&count), "fired {count} times");
    }

    #[test]
    fn the_buffer_wraps_around_a_long_move() {
        // Minutes of slow movement must not overflow or fire.
        let mut shakes = Shakes::new(vec![(3, Shake::new(NONE))]);
        let slow = |t: f32| ((t * 40.0) % 800.0, 300.0);
        assert_eq!(path(&mut shakes, NONE, 5_000, 20_000, slow), None);
    }
}
