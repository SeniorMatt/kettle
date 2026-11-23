//! The input state machine: decides which of the game's input events the overlay takes, and turns them into
//! egui events. No system calls here; the SDL and Xlib hooks feed it and act on its verdicts, so it is
//! tested without a display.
//!
//! Rules:
//! * Shift+Tab toggles (the Tab press is never seen by the game, nor its release).
//! * While open, every key, button, motion and wheel event is swallowed and given to egui. Escape closes.
//! * A release of something the game saw pressed (before the overlay opened) still reaches the game, so no key
//!   stays stuck down. A release of something we swallowed is swallowed too.

use egui::{Event, Key, Modifiers, MouseWheelUnit, PointerButton, Pos2, Vec2};
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyKind {
    Shift,
    Ctrl,
    Alt,
    Tab,
    Escape,
    /// Takes a screenshot (like Steam's default).
    F12,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    Pass,
    Swallow,
}

/// The result of one key event: what to do with it, and the new open state when it toggled.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeyStep {
    pub verdict: Verdict,
    pub toggled: Option<bool>,
}

#[derive(Default)]
pub struct Input {
    open: bool,
    /// Kettle's setting is off: nothing opens and every event passes.
    pub disabled: bool,
    shift: HashSet<u32>,
    ctrl: HashSet<u32>,
    alt: HashSet<u32>,
    swallowed_keys: HashSet<u32>,
    swallowed_buttons: HashSet<u8>,
    events: Vec<Event>,
    pos: Pos2,
    /// F12 was pressed: a screenshot is wanted (taken by the renderer at the next frame).
    shot: bool,
}

fn button_of(b: u8) -> Option<PointerButton> {
    Some(match b {
        1 => PointerButton::Primary,
        2 => PointerButton::Middle,
        3 => PointerButton::Secondary,
        4 => PointerButton::Extra1,
        5 => PointerButton::Extra2,
        _ => return None,
    })
}

impl Input {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn modifiers(&self) -> Modifiers {
        let ctrl = !self.ctrl.is_empty();
        Modifiers { alt: !self.alt.is_empty(), ctrl, shift: !self.shift.is_empty(), mac_cmd: false, command: ctrl }
    }

    /// Open or close (also used for a toggle that came from Kettle's menu).
    pub fn set_open(&mut self, open: bool) {
        if self.open == open {
            return;
        }
        self.open = open;
        self.events.clear();
        if open {
            self.events.push(Event::PointerMoved(self.pos));
        } else {
            self.events.push(Event::PointerGone);
        }
    }

    pub fn set_pos(&mut self, pos: Pos2) {
        self.pos = pos;
        if self.open {
            self.events.push(Event::PointerMoved(pos));
        }
    }

    pub fn pos(&self) -> Pos2 {
        self.pos
    }

    /// Was a screenshot asked for with the key since the last call?
    pub fn take_shot(&mut self) -> bool {
        std::mem::take(&mut self.shot)
    }

    /// The events collected since the last call (for one egui frame).
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// A key press or release. `native` identifies the physical key (any stable number per backend).
    pub fn key(&mut self, native: u32, kind: KeyKind, key: Option<Key>, down: bool, repeat: bool) -> KeyStep {
        let set = match kind {
            KeyKind::Shift => Some(&mut self.shift),
            KeyKind::Ctrl => Some(&mut self.ctrl),
            KeyKind::Alt => Some(&mut self.alt),
            _ => None,
        };
        if let Some(set) = set {
            if down {
                set.insert(native);
            } else {
                set.remove(&native);
            }
        }
        let swallowed_before = self.swallowed_keys.contains(&native);
        let step = |verdict, toggled| KeyStep { verdict, toggled };
        if self.disabled && !self.open && !swallowed_before {
            return step(Verdict::Pass, None);
        }
        if !down {
            if swallowed_before {
                self.swallowed_keys.remove(&native);
                if self.open {
                    self.push_key(key, false, false);
                }
                return step(Verdict::Swallow, None);
            }
            return step(Verdict::Pass, None);
        }
        // Shift+Tab toggles, open or closed. Escape closes.
        if kind == KeyKind::Tab && !self.shift.is_empty() && !repeat {
            self.swallowed_keys.insert(native);
            self.set_open(!self.open);
            return step(Verdict::Swallow, Some(self.open));
        }
        if self.open && kind == KeyKind::Escape && !repeat {
            self.swallowed_keys.insert(native);
            self.set_open(false);
            return step(Verdict::Swallow, Some(false));
        }
        if swallowed_before {
            return step(Verdict::Swallow, None); // auto-repeat of a swallowed key
        }
        if kind == KeyKind::F12 {
            self.swallowed_keys.insert(native);
            self.shot = true;
            return step(Verdict::Swallow, None);
        }
        if self.open {
            self.swallowed_keys.insert(native);
            self.push_key(key, true, repeat);
            return step(Verdict::Swallow, None);
        }
        step(Verdict::Pass, None)
    }

    fn push_key(&mut self, key: Option<Key>, pressed: bool, repeat: bool) {
        if let Some(key) = key {
            let modifiers = self.modifiers();
            self.events.push(Event::Key { key, physical_key: Some(key), pressed, repeat, modifiers });
        }
    }

    /// Text typed (UTF-8). Control characters are dropped.
    pub fn text(&mut self, s: &str) -> Verdict {
        if !self.open {
            return Verdict::Pass;
        }
        let t: String = s.chars().filter(|c| !c.is_control()).collect();
        if !t.is_empty() && self.ctrl.is_empty() {
            self.events.push(Event::Text(t));
        }
        Verdict::Swallow
    }

    /// The pointer moved to window position `pos`.
    pub fn motion(&mut self, pos: Pos2) -> Verdict {
        self.pos = pos;
        if !self.open {
            return Verdict::Pass;
        }
        self.events.push(Event::PointerMoved(pos));
        Verdict::Swallow
    }

    /// A mouse button: 1 left, 2 middle, 3 right, 4 and 5 the side buttons.
    pub fn button(&mut self, button: u8, down: bool, pos: Pos2) -> Verdict {
        self.pos = pos;
        if !down {
            if self.swallowed_buttons.remove(&button) {
                if self.open
                    && let Some(b) = button_of(button)
                {
                    self.events.push(Event::PointerButton {
                        pos,
                        button: b,
                        pressed: false,
                        modifiers: self.modifiers(),
                    });
                }
                return Verdict::Swallow;
            }
            return Verdict::Pass;
        }
        if !self.open {
            return Verdict::Pass;
        }
        self.swallowed_buttons.insert(button);
        if let Some(b) = button_of(button) {
            self.events.push(Event::PointerMoved(pos));
            self.events.push(Event::PointerButton { pos, button: b, pressed: true, modifiers: self.modifiers() });
        }
        Verdict::Swallow
    }

    /// The wheel, in lines (positive y = away from the user = up).
    pub fn wheel(&mut self, dx: f32, dy: f32) -> Verdict {
        if !self.open {
            return Verdict::Pass;
        }
        self.events.push(Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: Vec2::new(dx, dy),
            phase: egui::TouchPhase::Move,
            modifiers: self.modifiers(),
        });
        Verdict::Swallow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LSHIFT: u32 = 1;
    const TAB: u32 = 2;
    const A: u32 = 3;
    const ESC: u32 = 4;

    fn shift_tab(i: &mut Input) -> KeyStep {
        i.key(LSHIFT, KeyKind::Shift, None, true, false);
        let s = i.key(TAB, KeyKind::Tab, Some(Key::Tab), true, false);
        i.key(TAB, KeyKind::Tab, Some(Key::Tab), false, false);
        i.key(LSHIFT, KeyKind::Shift, None, false, false);
        s
    }

    #[test]
    fn shift_tab_opens_and_closes_and_the_game_never_sees_tab() {
        let mut i = Input::default();
        assert_eq!(
            i.key(TAB, KeyKind::Tab, Some(Key::Tab), true, false).verdict,
            Verdict::Pass,
            "plain Tab is the game's"
        );
        i.key(TAB, KeyKind::Tab, Some(Key::Tab), false, false);
        let s = shift_tab(&mut i);
        assert_eq!(s, KeyStep { verdict: Verdict::Swallow, toggled: Some(true) });
        assert!(i.is_open());
        let s = shift_tab(&mut i);
        assert_eq!(s, KeyStep { verdict: Verdict::Swallow, toggled: Some(false) });
        assert!(!i.is_open());
    }

    #[test]
    fn tab_release_is_swallowed_but_shift_release_passes() {
        let mut i = Input::default();
        i.key(LSHIFT, KeyKind::Shift, None, true, false);
        i.key(TAB, KeyKind::Tab, Some(Key::Tab), true, false);
        assert_eq!(i.key(TAB, KeyKind::Tab, Some(Key::Tab), false, false).verdict, Verdict::Swallow);
        assert_eq!(i.key(LSHIFT, KeyKind::Shift, None, false, false).verdict, Verdict::Pass);
    }

    #[test]
    fn open_swallows_everything_and_feeds_egui() {
        let mut i = Input::default();
        shift_tab(&mut i);
        i.take_events();
        assert_eq!(i.key(A, KeyKind::Other, Some(Key::A), true, false).verdict, Verdict::Swallow);
        assert_eq!(i.text("a"), Verdict::Swallow);
        assert_eq!(i.motion(Pos2::new(5.0, 6.0)), Verdict::Swallow);
        assert_eq!(i.button(1, true, Pos2::new(5.0, 6.0)), Verdict::Swallow);
        assert_eq!(i.button(1, false, Pos2::new(5.0, 6.0)), Verdict::Swallow);
        assert_eq!(i.wheel(0.0, 1.0), Verdict::Swallow);
        let ev = i.take_events();
        assert!(ev.iter().any(|e| matches!(e, Event::Text(t) if t == "a")));
        assert!(ev.iter().any(|e| matches!(e, Event::PointerButton { pressed: true, .. })));
        assert!(ev.iter().any(|e| matches!(e, Event::PointerButton { pressed: false, .. })));
    }

    #[test]
    fn closed_passes_everything() {
        let mut i = Input::default();
        assert_eq!(i.key(A, KeyKind::Other, Some(Key::A), true, false).verdict, Verdict::Pass);
        assert_eq!(i.text("a"), Verdict::Pass);
        assert_eq!(i.motion(Pos2::new(1.0, 1.0)), Verdict::Pass);
        assert_eq!(i.button(1, true, Pos2::ZERO), Verdict::Pass);
        assert_eq!(i.button(1, false, Pos2::ZERO), Verdict::Pass);
        assert_eq!(i.wheel(0.0, 1.0), Verdict::Pass);
        assert!(i.take_events().is_empty());
    }

    #[test]
    fn releases_of_keys_the_game_saw_pressed_still_reach_it() {
        let mut i = Input::default();
        assert_eq!(i.key(A, KeyKind::Other, Some(Key::A), true, false).verdict, Verdict::Pass);
        assert_eq!(i.button(1, true, Pos2::ZERO), Verdict::Pass);
        shift_tab(&mut i);
        assert_eq!(i.key(A, KeyKind::Other, Some(Key::A), false, false).verdict, Verdict::Pass);
        assert_eq!(i.button(1, false, Pos2::ZERO), Verdict::Pass);
    }

    #[test]
    fn things_pressed_while_open_are_released_while_swallowed_even_after_close() {
        let mut i = Input::default();
        shift_tab(&mut i);
        i.key(A, KeyKind::Other, Some(Key::A), true, false);
        i.button(3, true, Pos2::ZERO);
        shift_tab(&mut i);
        assert!(!i.is_open());
        assert_eq!(i.key(A, KeyKind::Other, Some(Key::A), false, false).verdict, Verdict::Swallow);
        assert_eq!(i.button(3, false, Pos2::ZERO), Verdict::Swallow);
        assert_eq!(i.key(A, KeyKind::Other, Some(Key::A), true, false).verdict, Verdict::Pass, "back to normal");
    }

    #[test]
    fn escape_closes_only_when_open() {
        let mut i = Input::default();
        assert_eq!(i.key(ESC, KeyKind::Escape, Some(Key::Escape), true, false).verdict, Verdict::Pass);
        i.key(ESC, KeyKind::Escape, Some(Key::Escape), false, false);
        shift_tab(&mut i);
        let s = i.key(ESC, KeyKind::Escape, Some(Key::Escape), true, false);
        assert_eq!(s.toggled, Some(false));
        assert!(!i.is_open());
    }

    #[test]
    fn repeat_of_swallowed_tab_stays_swallowed() {
        let mut i = Input::default();
        i.key(LSHIFT, KeyKind::Shift, None, true, false);
        i.key(TAB, KeyKind::Tab, Some(Key::Tab), true, false);
        let s = i.key(TAB, KeyKind::Tab, Some(Key::Tab), true, true);
        assert_eq!((s.verdict, s.toggled), (Verdict::Swallow, None));
        assert!(i.is_open());
    }

    #[test]
    fn modifiers_follow_the_keys() {
        let mut i = Input::default();
        i.key(10, KeyKind::Ctrl, None, true, false);
        assert!(i.modifiers().ctrl && i.modifiers().command);
        i.key(10, KeyKind::Ctrl, None, false, false);
        assert!(!i.modifiers().ctrl);
    }

    #[test]
    fn f12_takes_a_screenshot_and_the_game_does_not_see_it() {
        let mut i = Input::default();
        let s = i.key(70, KeyKind::F12, None, true, false);
        assert_eq!(s.verdict, Verdict::Swallow);
        assert!(i.take_shot());
        assert!(!i.take_shot(), "once per press");
        // Its release is swallowed too, and a held key does not take more.
        i.key(70, KeyKind::F12, None, true, true);
        assert!(!i.take_shot());
        assert_eq!(i.key(70, KeyKind::F12, None, false, false).verdict, Verdict::Swallow);
        // Overlay switched off in Kettle: F12 is the game's.
        i.disabled = true;
        assert_eq!(i.key(70, KeyKind::F12, None, true, false).verdict, Verdict::Pass);
        assert!(!i.take_shot());
    }
}
