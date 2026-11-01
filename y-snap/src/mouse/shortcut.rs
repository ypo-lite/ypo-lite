//! Mouse shortcuts: a modifier plus a mouse trigger fires one action, the way
//! a hotkey does. See `docs/mouse-shortcuts-plan.md`.
//!
//! A shortcut is one event, not a session like [`super::gesture`]: fire on
//! the press, swallow the release. The decisions live here as a pure state
//! machine — modifiers and time come in as arguments — so they are testable
//! without Windows. What cannot be pure (reading the modifiers, resolving the
//! window, posting the message) is [`super::HookState::on_press`].
//!
//! # Wheel tilt has no release
//!
//! Windows sends a held tilt as repeating `WM_MOUSEHWHEEL` events and never
//! says when it ends, so "held" is inferred from time: an event within
//! [`TILT_GAP_MS`] of the previous one is the same hold, swallowed without
//! firing again. Probe A measured the first repeat at about 400 ms and the
//! rest every ~100 ms, hence 500.

use windows::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS;

use crate::config::{MouseShortcut, MouseTrigger};

/// Two tilt events closer than this are one hold. Above the ~406 ms first
/// repeat Probe A measured; two deliberate tilts less than half a second
/// apart therefore count as one.
pub const TILT_GAP_MS: u32 = 500;

/// What the hook should do with an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// Not ours: let it through.
    Pass,
    /// Ours, but nothing to fire (a held tilt, or a gesture in progress).
    Swallow,
    /// Fire the binding at this index.
    Fire(usize),
}

pub struct Shortcuts {
    /// Copied at install, only ever read in the hook.
    bindings: Vec<MouseShortcut>,
    /// Press claimed, release still to swallow.
    held: [bool; MouseTrigger::ALL.len()],
    /// `time` of the last tilt event seen: left, right.
    tilt: [Option<u32>; 2],
}

impl Shortcuts {
    pub fn new(bindings: &[MouseShortcut]) -> Self {
        Self {
            bindings: bindings.to_vec(),
            held: [false; MouseTrigger::ALL.len()],
            tilt: [None; 2],
        }
    }

    /// Whether any binding uses `trigger`. The hook asks this first so an
    /// ordinary click costs one slice scan and not a modifier read.
    pub fn uses(&self, trigger: MouseTrigger) -> bool {
        self.bindings.iter().any(|b| b.trigger == trigger)
    }

    pub fn binding(&self, index: usize) -> Option<&MouseShortcut> {
        self.bindings.get(index)
    }

    fn find(&self, trigger: MouseTrigger, modifiers: HOT_KEY_MODIFIERS) -> Option<usize> {
        self.bindings
            .iter()
            .position(|b| b.trigger == trigger && b.modifiers == modifiers)
    }

    /// A button went down. `gesture_active` is a drag or resize in progress,
    /// which owns the pointer: a matching press is "not now", not a fire.
    pub fn press(
        &mut self,
        trigger: MouseTrigger,
        modifiers: HOT_KEY_MODIFIERS,
        gesture_active: bool,
    ) -> Press {
        // A release lost to a dropped hook must not swallow the next real one.
        self.held[trigger.index()] = false;

        let Some(index) = self.find(trigger, modifiers) else {
            return Press::Pass;
        };
        self.held[trigger.index()] = true;
        if gesture_active {
            Press::Swallow
        } else {
            Press::Fire(index)
        }
    }

    /// A button came up. True when its press was claimed, whatever the
    /// modifiers are now.
    pub fn release(&mut self, trigger: MouseTrigger) -> bool {
        std::mem::take(&mut self.held[trigger.index()])
    }

    /// The wheel tilted; `time` is the event's own millisecond timestamp,
    /// which wraps.
    pub fn tilt(
        &mut self,
        trigger: MouseTrigger,
        modifiers: HOT_KEY_MODIFIERS,
        time: u32,
        gesture_active: bool,
    ) -> Press {
        let Some(side) = tilt_side(trigger) else {
            return Press::Pass;
        };
        let Some(index) = self.find(trigger, modifiers) else {
            return Press::Pass;
        };

        let same_hold = self.tilt[side].is_some_and(|last| time.wrapping_sub(last) < TILT_GAP_MS);
        // Refreshed even when swallowed, so the gap restarts and a long hold
        // never fires twice.
        self.tilt[side] = Some(time);

        if same_hold || gesture_active {
            Press::Swallow
        } else {
            Press::Fire(index)
        }
    }

    /// A fire that found nothing to act on was passed on after all: forget
    /// that it was claimed, so its release (or tilt repeats) are not eaten.
    pub fn unclaim(&mut self, trigger: MouseTrigger) {
        self.held[trigger.index()] = false;
        if let Some(side) = tilt_side(trigger) {
            self.tilt[side] = None;
        }
    }
}

fn tilt_side(trigger: MouseTrigger) -> Option<usize> {
    match trigger {
        MouseTrigger::WheelLeft => Some(0),
        MouseTrigger::WheelRight => Some(1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_SHIFT, MOD_WIN};

    use super::*;
    use crate::action::{Action, Target};

    const NONE: HOT_KEY_MODIFIERS = HOT_KEY_MODIFIERS(0);

    fn bind(trigger: MouseTrigger, modifiers: HOT_KEY_MODIFIERS) -> MouseShortcut {
        MouseShortcut {
            trigger,
            modifiers,
            action: Action::Close,
            target: Target::Pointed,
            desktop: 0,
        }
    }

    fn shortcuts() -> Shortcuts {
        Shortcuts::new(&[
            bind(MouseTrigger::Middle, MOD_WIN),
            bind(MouseTrigger::WheelRight, NONE),
            bind(MouseTrigger::WheelLeft, NONE),
            bind(MouseTrigger::Left, MOD_SHIFT),
        ])
    }

    #[test]
    fn fires_on_press_and_swallows_the_release() {
        let mut s = shortcuts();
        assert_eq!(
            s.press(MouseTrigger::Middle, MOD_WIN, false),
            Press::Fire(0)
        );
        assert!(s.release(MouseTrigger::Middle));
        assert!(!s.release(MouseTrigger::Middle));
    }

    #[test]
    fn the_release_is_swallowed_after_the_modifiers_are_let_go() {
        let mut s = shortcuts();
        s.press(MouseTrigger::Middle, MOD_WIN, false);
        // `release` takes no modifiers at all.
        assert!(s.release(MouseTrigger::Middle));
    }

    #[test]
    fn wrong_modifiers_pass() {
        let mut s = shortcuts();
        assert_eq!(s.press(MouseTrigger::Middle, NONE, false), Press::Pass);
        assert_eq!(
            s.press(
                MouseTrigger::Middle,
                HOT_KEY_MODIFIERS(MOD_WIN.0 | MOD_SHIFT.0),
                false
            ),
            Press::Pass
        );
        assert!(!s.release(MouseTrigger::Middle));
    }

    #[test]
    fn a_press_clears_a_stale_claim() {
        let mut s = shortcuts();
        s.press(MouseTrigger::Middle, MOD_WIN, false);
        // The release never arrived; the next press is unmatched.
        assert_eq!(s.press(MouseTrigger::Middle, MOD_ALT, false), Press::Pass);
        assert!(!s.release(MouseTrigger::Middle));
    }

    #[test]
    fn a_press_during_a_gesture_is_swallowed_not_fired() {
        let mut s = shortcuts();
        assert_eq!(s.press(MouseTrigger::Left, MOD_SHIFT, true), Press::Swallow);
        assert!(s.release(MouseTrigger::Left));
    }

    #[test]
    fn unclaim_lets_the_release_through() {
        let mut s = shortcuts();
        s.press(MouseTrigger::Middle, MOD_WIN, false);
        s.unclaim(MouseTrigger::Middle);
        assert!(!s.release(MouseTrigger::Middle));
    }

    #[test]
    fn a_held_tilt_fires_once() {
        let mut s = shortcuts();
        let right = MouseTrigger::WheelRight;
        assert_eq!(s.tilt(right, NONE, 1_000, false), Press::Fire(1));
        // First repeat ~400 ms later, then every ~100 ms for three seconds.
        assert_eq!(s.tilt(right, NONE, 1_400, false), Press::Swallow);
        for t in (1_500..4_000).step_by(100) {
            assert_eq!(s.tilt(right, NONE, t, false), Press::Swallow);
        }
    }

    #[test]
    fn a_tilt_fires_again_after_the_gap() {
        let mut s = shortcuts();
        let right = MouseTrigger::WheelRight;
        s.tilt(right, NONE, 1_000, false);
        assert_eq!(
            s.tilt(right, NONE, 1_000 + TILT_GAP_MS, false),
            Press::Fire(1)
        );
    }

    #[test]
    fn the_two_directions_are_independent() {
        let mut s = shortcuts();
        assert_eq!(
            s.tilt(MouseTrigger::WheelRight, NONE, 1_000, false),
            Press::Fire(1)
        );
        assert_eq!(
            s.tilt(MouseTrigger::WheelLeft, NONE, 1_050, false),
            Press::Fire(2)
        );
    }

    #[test]
    fn tilt_time_wraps() {
        let mut s = shortcuts();
        let right = MouseTrigger::WheelRight;
        assert_eq!(s.tilt(right, NONE, u32::MAX - 50, false), Press::Fire(1));
        // 100 ms later, across the wrap: still the same hold.
        assert_eq!(s.tilt(right, NONE, 49, false), Press::Swallow);
    }

    #[test]
    fn a_tilt_with_other_modifiers_passes_and_leaves_the_hold_alone() {
        let mut s = shortcuts();
        let right = MouseTrigger::WheelRight;
        assert_eq!(s.tilt(right, MOD_ALT, 1_000, false), Press::Pass);
        assert_eq!(s.tilt(right, NONE, 1_100, false), Press::Fire(1));
    }

    #[test]
    fn a_tilt_during_a_gesture_is_swallowed() {
        let mut s = shortcuts();
        assert_eq!(
            s.tilt(MouseTrigger::WheelRight, NONE, 1_000, true),
            Press::Swallow
        );
    }

    #[test]
    fn unclaim_forgets_a_tilt() {
        let mut s = shortcuts();
        let right = MouseTrigger::WheelRight;
        s.tilt(right, NONE, 1_000, false);
        s.unclaim(right);
        assert_eq!(s.tilt(right, NONE, 1_100, false), Press::Fire(1));
    }

    #[test]
    fn unused_triggers_are_cheap_to_rule_out() {
        let s = shortcuts();
        assert!(s.uses(MouseTrigger::Middle));
        assert!(!s.uses(MouseTrigger::X1));
    }
}
