//! The mouse inputs a binding can use, and the pure state machine that decides
//! what the hook does with each event. No Win32: modifiers come in as `MOD_*`
//! flags and the events as arguments, so every rule here is unit tested. What
//! cannot be pure (reading the modifiers, the hook itself, posting) is
//! `src/mouse.rs` in the application.
//!
//! # Wheel and buttons
//!
//! A wheel notch is an event: it is either claimed and swallowed or passed.
//! A button is a session (`Idle` / `Pressed`): the press is swallowed, the
//! matching release is consumed, and the action fires **on the release**, as
//! VulCan does. The binding is chosen at the press, with the modifiers held
//! then, so letting go of `Alt` before the button still fires it. A release
//! with no claimed press is never eaten.

use crate::model::{Action, Target};

/// Something the mouse can do that a binding listens for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// The vertical wheel: up raises, down lowers (see [`MouseBinding::invert`]).
    Wheel,
    /// The wheel tilted sideways: right raises, left lowers.
    WheelTilt,
    Left,
    Right,
    Middle,
    X1,
    X2,
}

impl Input {
    /// How many buttons there are: the size of the press table.
    pub const BUTTONS: usize = 5;

    /// The button's slot in the press table, `None` for a wheel.
    pub fn button_index(self) -> Option<usize> {
        match self {
            Self::Left => Some(0),
            Self::Right => Some(1),
            Self::Middle => Some(2),
            Self::X1 => Some(3),
            Self::X2 => Some(4),
            Self::Wheel | Self::WheelTilt => None,
        }
    }

    pub fn is_wheel(self) -> bool {
        matches!(self, Self::Wheel | Self::WheelTilt)
    }

    /// The name of a button in the configuration file.
    pub fn button_name(self) -> Option<&'static str> {
        match self {
            Self::Left => Some("left"),
            Self::Right => Some("right"),
            Self::Middle => Some("middle"),
            Self::X1 => Some("x1"),
            Self::X2 => Some("x2"),
            Self::Wheel | Self::WheelTilt => None,
        }
    }

    /// The button a configuration name designates, in any case.
    pub fn from_button_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "middle" => Some(Self::Middle),
            "x1" => Some(Self::X1),
            "x2" => Some(Self::X2),
            _ => None,
        }
    }

    /// Whether a binding may use this input with no modifier. The wheel and
    /// the three main buttons would take over ordinary scrolling and clicking
    /// everywhere, so they never may; the side buttons are the exception.
    pub fn may_be_bare(self) -> bool {
        matches!(self, Self::X1 | Self::X2)
    }
}

/// One `<wheel>` or `<button>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseBinding {
    pub input: Input,
    /// `MOD_*` flags, without `MOD_NOREPEAT`; matched exactly.
    pub modifiers: u32,
    pub target: Target,
    /// What a button does. `None` for a wheel, whose action is the volume
    /// change the wheel itself stands for.
    pub action: Option<Action>,
    /// A wheel only: down raises and up lowers.
    pub invert: bool,
}

/// What the hook does with an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Not ours: let it through.
    Pass,
    /// Ours, nothing to do yet (a button press, whose action waits for the
    /// release) or nothing to do at all.
    Swallow,
    /// Ours: swallow it and run the binding at this index.
    Fire(usize),
}

/// The decisions of the mouse hook.
#[derive(Debug, Clone)]
pub struct Machine {
    /// Copied at install, only ever read by the hook.
    bindings: Vec<MouseBinding>,
    /// Per button: the binding its claimed press selected.
    pressed: [Option<usize>; Input::BUTTONS],
}

impl Machine {
    pub fn new(bindings: &[MouseBinding]) -> Self {
        Self {
            bindings: bindings.to_vec(),
            pressed: [None; Input::BUTTONS],
        }
    }

    pub fn binding(&self, index: usize) -> Option<&MouseBinding> {
        self.bindings.get(index)
    }

    /// Whether any binding listens to `input`: the hook asks this first, so an
    /// event nobody wants costs one slice scan and no modifier read.
    pub fn uses(&self, input: Input) -> bool {
        self.bindings.iter().any(|binding| binding.input == input)
    }

    fn find(&self, input: Input, modifiers: u32) -> Option<usize> {
        self.bindings
            .iter()
            .position(|binding| binding.input == input && binding.modifiers == modifiers)
    }

    /// A wheel event, vertical or tilted. Claimed only with exactly the bound
    /// modifiers; the handler gets the delta separately (see the hook's
    /// accumulator), so a claimed notch is a `Fire` here.
    pub fn wheel(&self, input: Input, modifiers: u32) -> Verdict {
        match self.find(input, modifiers) {
            Some(index) if input.is_wheel() => Verdict::Fire(index),
            _ => Verdict::Pass,
        }
    }

    /// A button went down.
    pub fn button_down(&mut self, input: Input, modifiers: u32) -> Verdict {
        let Some(slot) = input.button_index() else {
            return Verdict::Pass;
        };
        // A release lost to a dropped hook must not eat the next real one.
        self.pressed[slot] = None;

        match self.find(input, modifiers) {
            Some(index) => {
                self.pressed[slot] = Some(index);
                Verdict::Swallow
            }
            None => Verdict::Pass,
        }
    }

    /// A button came up. The press it answers decides, whatever the modifiers
    /// are now: a claimed press fires, an unclaimed one passes.
    pub fn button_up(&mut self, input: Input) -> Verdict {
        let Some(slot) = input.button_index() else {
            return Verdict::Pass;
        };
        match self.pressed[slot].take() {
            Some(index) => Verdict::Fire(index),
            None => Verdict::Pass,
        }
    }

    /// A fire that found nothing to act on, passed on after all (Phase 5's
    /// applications). Nothing to undo for a button today: its press is long
    /// swallowed. Kept so the hook has one place to say it.
    pub fn forget(&mut self, input: Input) {
        if let Some(slot) = input.button_index() {
            self.pressed[slot] = None;
        }
    }
}

/// The signed notch count of a wheel delta: 120 units is one notch, a
/// touchpad's smaller deltas are fractions. `invert` flips the direction.
pub fn notches(delta: i32, invert: bool) -> f32 {
    let notches = delta as f32 / 120.0;
    if invert { -notches } else { notches }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALT: u32 = 0x1;
    const SHIFT: u32 = 0x4;

    fn wheel(modifiers: u32) -> MouseBinding {
        MouseBinding {
            input: Input::Wheel,
            modifiers,
            target: Target::Master,
            action: None,
            invert: false,
        }
    }

    fn button(input: Input, modifiers: u32, action: Action) -> MouseBinding {
        MouseBinding {
            input,
            modifiers,
            target: Target::Master,
            action: Some(action),
            invert: false,
        }
    }

    fn machine() -> Machine {
        Machine::new(&[
            wheel(ALT),
            wheel(ALT | SHIFT),
            button(Input::Middle, ALT, Action::Mute),
            button(Input::X1, 0, Action::Mute),
        ])
    }

    #[test]
    fn a_wheel_needs_exactly_the_bound_modifiers() {
        let m = machine();
        assert_eq!(m.wheel(Input::Wheel, ALT), Verdict::Fire(0));
        assert_eq!(m.wheel(Input::Wheel, ALT | SHIFT), Verdict::Fire(1));
        assert_eq!(m.wheel(Input::Wheel, SHIFT), Verdict::Pass);
        assert_eq!(m.wheel(Input::Wheel, ALT | 0x2), Verdict::Pass);
    }

    #[test]
    fn the_bare_wheel_is_never_claimed() {
        assert_eq!(machine().wheel(Input::Wheel, 0), Verdict::Pass);
    }

    #[test]
    fn a_wheel_binding_does_not_answer_the_tilt() {
        assert_eq!(machine().wheel(Input::WheelTilt, ALT), Verdict::Pass);
    }

    #[test]
    fn a_button_swallows_the_press_and_fires_on_the_release() {
        let mut m = machine();
        assert_eq!(m.button_down(Input::Middle, ALT), Verdict::Swallow);
        assert_eq!(m.button_up(Input::Middle), Verdict::Fire(2));
        // The session is over: a second release is a stray one.
        assert_eq!(m.button_up(Input::Middle), Verdict::Pass);
    }

    #[test]
    fn the_release_fires_after_the_modifier_is_let_go() {
        let mut m = machine();
        m.button_down(Input::Middle, ALT);
        // `button_up` takes no modifiers at all.
        assert_eq!(m.button_up(Input::Middle), Verdict::Fire(2));
    }

    #[test]
    fn wrong_modifiers_pass_both_the_press_and_the_release() {
        let mut m = machine();
        assert_eq!(m.button_down(Input::Middle, 0), Verdict::Pass);
        assert_eq!(m.button_up(Input::Middle), Verdict::Pass);
        assert_eq!(m.button_down(Input::Middle, ALT | SHIFT), Verdict::Pass);
        assert_eq!(m.button_up(Input::Middle), Verdict::Pass);
    }

    #[test]
    fn a_stray_release_is_never_eaten() {
        let mut m = machine();
        assert_eq!(m.button_up(Input::Middle), Verdict::Pass);
        assert_eq!(m.button_up(Input::Left), Verdict::Pass);
    }

    #[test]
    fn a_press_clears_a_stale_claim() {
        let mut m = machine();
        m.button_down(Input::Middle, ALT);
        // The release never arrived; this press is unmatched.
        assert_eq!(m.button_down(Input::Middle, 0), Verdict::Pass);
        assert_eq!(m.button_up(Input::Middle), Verdict::Pass);
    }

    #[test]
    fn buttons_are_independent_sessions() {
        let mut m = machine();
        assert_eq!(m.button_down(Input::Middle, ALT), Verdict::Swallow);
        assert_eq!(m.button_down(Input::X1, 0), Verdict::Swallow);
        assert_eq!(m.button_up(Input::X1), Verdict::Fire(3));
        assert_eq!(m.button_up(Input::Middle), Verdict::Fire(2));
    }

    #[test]
    fn forget_lets_the_release_through() {
        let mut m = machine();
        m.button_down(Input::Middle, ALT);
        m.forget(Input::Middle);
        assert_eq!(m.button_up(Input::Middle), Verdict::Pass);
    }

    #[test]
    fn a_wheel_is_not_a_button() {
        let mut m = machine();
        assert_eq!(m.button_down(Input::Wheel, ALT), Verdict::Pass);
        assert_eq!(m.button_up(Input::Wheel), Verdict::Pass);
        assert_eq!(m.wheel(Input::Middle, ALT), Verdict::Pass);
    }

    #[test]
    fn unused_inputs_are_cheap_to_rule_out() {
        let m = machine();
        assert!(m.uses(Input::Wheel));
        assert!(!m.uses(Input::Right));
    }

    #[test]
    fn a_notch_is_one_and_a_touchpad_delta_a_fraction() {
        assert_eq!(notches(120, false), 1.0);
        assert_eq!(notches(-120, false), -1.0);
        assert_eq!(notches(30, false), 0.25);
        assert_eq!(notches(240, true), -2.0);
    }

    #[test]
    fn only_the_side_buttons_may_be_bare() {
        assert!(Input::X1.may_be_bare());
        assert!(Input::X2.may_be_bare());
        for input in [Input::Left, Input::Right, Input::Middle, Input::Wheel] {
            assert!(!input.may_be_bare());
        }
    }

    #[test]
    fn button_names_round_trip() {
        for input in [Input::Left, Input::Right, Input::Middle, Input::X1, Input::X2] {
            let name = input.button_name().unwrap();
            assert_eq!(Input::from_button_name(&name.to_uppercase()), Some(input));
        }
        assert_eq!(Input::from_button_name("wheel"), None);
    }
}
