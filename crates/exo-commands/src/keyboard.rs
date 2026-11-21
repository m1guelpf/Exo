//! Turning key presses into commands: sequences, key repeat and the "Go to…" panel timer.
//!
//! The live registrations come in already filtered to the active region, newest last; the newest
//! binding for a key wins.

use egui::{Key, Modifiers, os::OperatingSystem};

use crate::keys::{Binding, Chord};

/// How long a sequence waits for its next key.
const SEQUENCE_TIMEOUT: f64 = 2.4;
/// A held key repeats after `REPEAT_DELAY`, every `REPEAT_INTERVAL`, until it's released or reports
/// no key-down for `REPEAT_FAILSAFE`.
const REPEAT_DELAY: f64 = 0.25;
const REPEAT_INTERVAL: f64 = 0.065;
const REPEAT_FAILSAFE: f64 = 2.0;
/// The Go to… panel shows this long after an unmodified `g`…
const GO_MENU_DELAY: f64 = 0.2;
/// …and hides this long after it showed.
const GO_MENU_DURATION: f64 = 2.4;

/// A key event, from egui's input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum KeyEvent {
    /// With the text the key types, which egui reports next.
    Down {
        key: Key,
        modifiers: Modifiers,
        repeat: bool,
        typed: Option<String>,
    },
    Up {
        key: Key,
    },
}

/// One live registration's keys.
pub(crate) struct Live {
    /// Index of the registration, reported back when it's pressed.
    pub(crate) index: usize,
    pub(crate) bindings: Vec<Binding>,
    pub(crate) repeats: bool,
    /// A backtick presses its `esc` binding ([`crate::Command::backtick_escape`]).
    pub(crate) backtick: bool,
    /// Its keys press it only when they type this ([`crate::Command::when_typing`]).
    pub(crate) typing: Option<&'static str>,
}

/// What a frame's keys did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fire {
    /// The live registration at this index was pressed. If it [repeats](Live::repeats), its key
    /// is now the held key.
    Press(usize),
    /// The held key ticked.
    Repeat,
    /// The held key was let go (or the failsafe gave up on it).
    Release,
}

/// Key state that outlives a frame.
#[derive(Default)]
pub(crate) struct Keyboard {
    /// Chords typed so far of a sequence that's still possible.
    pending: Vec<Chord>,
    pending_at: f64,
    repeating: Option<Repeating>,
    /// When an unmodified `g` started the Go to… panel's timer.
    go_pressed_at: Option<f64>,
}

/// The held key of a repeating registration. Registrations are rebuilt every frame, so this
/// tracks the key, not the registration; whoever saw the press knows what repeats.
struct Repeating {
    /// The key as egui reports it, which is what its key-up and `keys_down` use.
    key: Key,
    next: f64,
    /// The last key down for the held key, OS repeats included (for the failsafe).
    last_down: f64,
}

impl Keyboard {
    /// Advances the clock to `now`: drops a sequence that timed out, and ticks the held key on
    /// its own clock (not the OS's), or lets it go when it's up (`held` says whether a key is
    /// still down) or the failsafe expires.
    pub(crate) fn tick(&mut self, held: impl Fn(Key) -> bool, now: f64) -> Option<Fire> {
        if !self.pending.is_empty() && now - self.pending_at > SEQUENCE_TIMEOUT {
            self.pending.clear();
        }
        let repeating = self.repeating.as_mut()?;
        if !held(repeating.key) || now - repeating.last_down > REPEAT_FAILSAFE {
            self.repeating = None;
            return Some(Fire::Release);
        }
        // Catch up one tick at a time: a slow frame shouldn't flood the list.
        (now >= repeating.next).then(|| {
            repeating.next = (repeating.next + REPEAT_INTERVAL).max(now);
            Fire::Repeat
        })
    }

    /// Feeds one key event through `live` (newest last) at `now`. In a text field (`text_input`)
    /// OS key repeat passes through.
    pub(crate) fn event(
        &mut self,
        event: KeyEvent,
        live: &[Live],
        text_input: bool,
        now: f64,
        os: OperatingSystem,
    ) -> Option<Fire> {
        match event {
            KeyEvent::Down {
                key,
                modifiers,
                repeat,
                typed,
            } => {
                if repeat {
                    if let Some(repeating) = self
                        .repeating
                        .as_mut()
                        .filter(|repeating| repeating.key == key)
                    {
                        repeating.last_down = now;
                        return None;
                    }
                    if !text_input {
                        return None;
                    }
                }
                self.update_go_menu(key, modifiers, live, now);
                // FastFrame's egui reports Shift, Ctrl, Alt and ⌘ as keys of their own. Bindings
                // skip them: the Shift on its way to `!` mustn't end `g`'s sequence and let `!`
                // mark spam. (The Go to… panel still hides, as it does on any key-down.)
                if is_modifier(key) {
                    return None;
                }
                self.press(
                    key,
                    Chord::from_event(key, modifiers),
                    typed.as_deref(),
                    live,
                    now,
                    os,
                )
            }
            KeyEvent::Up { key } => self
                .repeating
                .take_if(|repeating| repeating.key == key)
                .map(|_| Fire::Release),
        }
    }

    fn press(
        &mut self,
        key: Key,
        chord: Chord,
        typed: Option<&str>,
        live: &[Live],
        now: f64,
        os: OperatingSystem,
    ) -> Option<Fire> {
        if !self.pending.is_empty() {
            let mut sequence = self.pending.clone();
            sequence.push(chord);
            // Only the deepest sequence level fires.
            if let Some(found) = newest(live, |binding| binding.chords == sequence) {
                self.pending.clear();
                return Some(Fire::Press(found.index));
            }
            if newest(live, |binding| {
                binding.chords.len() > sequence.len() && binding.chords.starts_with(&sequence)
            })
            .is_some()
            {
                self.pending = sequence;
                self.pending_at = now;
                return None;
            }
            // Any key that doesn't continue a sequence resets them all.
            self.pending.clear();
        }
        if newest(live, |binding| {
            binding.chords.len() > 1 && binding.chords[0] == chord
        })
        .is_some()
        {
            self.pending = vec![chord];
            self.pending_at = now;
        }
        let found = live
            .iter()
            .rev()
            .filter(|entry| entry.typing.is_none_or(|text| typed == Some(text)))
            .find(|entry| {
                entry
                    .bindings
                    .iter()
                    .any(|binding| matches_single(binding, chord, os, entry.backtick))
            })?;
        if found.repeats {
            self.repeating = Some(Repeating {
                key,
                next: now + REPEAT_DELAY,
                last_down: now,
            });
        }
        Some(Fire::Press(found.index))
    }

    /// The Go to… panel's timer: an unmodified `g` where `g` sequences are bound starts the panel's
    /// 200ms timer; any other key, or `g` while it shows, hides it.
    fn update_go_menu(&mut self, key: Key, modifiers: Modifiers, live: &[Live], now: f64) {
        let visible = self.go_menu_visible(now);
        let g = Chord::from_event(Key::G, Modifiers::NONE);
        let starts_go = newest(live, |binding| {
            binding.chords.len() > 1 && binding.chords[0] == g
        })
        .is_some();
        self.go_pressed_at =
            (key == Key::G && modifiers.is_none() && !visible && starts_go).then_some(now);
    }

    /// Whether the Go to… panel shows at `now`.
    pub(crate) fn go_menu_visible(&self, now: f64) -> bool {
        self.go_pressed_at.is_some_and(|at| {
            (at + GO_MENU_DELAY..at + GO_MENU_DELAY + GO_MENU_DURATION).contains(&now)
        })
    }

    /// When something changes next without input (a repeat tick, a timeout), to repaint then; `now`
    /// if a repeat tick is already due.
    pub(crate) fn next_change(&self, now: f64) -> Option<f64> {
        let repeat = self.repeating.as_ref().map(|repeating| repeating.next);
        let sequence = (!self.pending.is_empty()).then_some(self.pending_at + SEQUENCE_TIMEOUT);
        let go_menu = self.go_pressed_at.map(|at| {
            if now < at + GO_MENU_DELAY {
                at + GO_MENU_DELAY
            } else {
                at + GO_MENU_DELAY + GO_MENU_DURATION
            }
        });
        [repeat, sequence, go_menu]
            .into_iter()
            .flatten()
            .filter(|&at| at >= now)
            .reduce(f64::min)
    }
}

/// A modifier key pressed on its own.
fn is_modifier(key: Key) -> bool {
    matches!(
        key,
        Key::ShiftLeft
            | Key::ShiftRight
            | Key::ControlLeft
            | Key::ControlRight
            | Key::AltLeft
            | Key::AltRight
            | Key::SuperLeft
            | Key::SuperRight
    )
}

/// The newest live registration with a binding satisfying `test`.
fn newest(live: &[Live], test: impl Fn(&Binding) -> bool) -> Option<&Live> {
    live.iter()
        .rev()
        .find(|entry| entry.bindings.iter().any(&test))
}

/// Whether a single-chord binding matches `chord`, Escape matching its stand-ins too (a
/// backtick only where `backtick`).
fn matches_single(binding: &Binding, chord: Chord, os: OperatingSystem, backtick: bool) -> bool {
    match binding.chords.as_slice() {
        [only] => {
            *only == chord
                || (*only == Chord::ESCAPE
                    && chord.is_escape(os)
                    && (backtick || chord.key != Key::Backtick))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: OperatingSystem = OperatingSystem::Mac;

    fn live(index: usize, specs: &[&str], repeats: bool) -> Live {
        let bindings = specs
            .iter()
            .map(|spec| Binding::parse(spec, MAC).expect("valid"))
            .collect();
        Live {
            index,
            bindings,
            repeats,
            backtick: true,
            typing: None,
        }
    }

    /// One frame: the clock, then each of `events`.
    fn frame(
        keyboard: &mut Keyboard,
        events: &[KeyEvent],
        live: &[Live],
        held: impl Fn(Key) -> bool,
        text_input: bool,
        now: f64,
    ) -> Vec<Fire> {
        let tick = keyboard.tick(held, now);
        tick.into_iter()
            .chain(
                events
                    .iter()
                    .filter_map(|event| keyboard.event(event.clone(), live, text_input, now, MAC)),
            )
            .collect()
    }

    fn down(key: Key) -> KeyEvent {
        KeyEvent::Down {
            key,
            modifiers: Modifiers::NONE,
            repeat: false,
            typed: None,
        }
    }

    #[test]
    fn newest_registration_wins() {
        let live = [live(0, &["e"], false), live(1, &["e"], false)];
        let fired = frame(
            &mut Keyboard::default(),
            &[down(Key::E)],
            &live,
            |_| false,
            false,
            0.0,
        );
        assert_eq!(fired, [Fire::Press(1)]);
    }

    #[test]
    fn sequences_complete_within_the_window() {
        let live = [live(0, &["g i"], false)];
        let mut keyboard = Keyboard::default();
        assert!(frame(&mut keyboard, &[down(Key::G)], &live, |_| false, false, 0.0).is_empty());
        assert_eq!(
            frame(&mut keyboard, &[down(Key::I)], &live, |_| false, false, 2.3),
            [Fire::Press(0)]
        );
        frame(
            &mut keyboard,
            &[down(Key::G)],
            &live,
            |_| false,
            false,
            10.0,
        );
        assert!(
            frame(
                &mut keyboard,
                &[down(Key::I)],
                &live,
                |_| false,
                false,
                12.5
            )
            .is_empty(),
            "2.4s window"
        );
    }

    #[test]
    fn modifier_keys_neither_continue_nor_end_a_sequence() {
        // `!` is registered last, as the conversation's Mark Spam is.
        let live = [
            live(0, &["g !", "g #", "g shift+m"], false),
            live(1, &["!", "#", "shift+m"], false),
        ];
        let shift = |key| KeyEvent::Down {
            key,
            modifiers: Modifiers::SHIFT,
            repeat: false,
            typed: None,
        };
        for key in [Key::Exclamationmark, Key::Num3, Key::M] {
            let mut keyboard = Keyboard::default();
            let events = [
                down(Key::G),
                KeyEvent::Up { key: Key::G },
                shift(Key::ShiftLeft),
                shift(key),
            ];
            assert_eq!(
                frame(&mut keyboard, &events, &live, |_| false, false, 0.0),
                [Fire::Press(0)],
                "g then {key:?}"
            );
        }
    }

    #[test]
    fn go_menu_shows_after_200ms_and_hides_on_the_next_key() {
        let live = [live(0, &["g i"], false)];
        let mut keyboard = Keyboard::default();
        frame(&mut keyboard, &[down(Key::G)], &live, |_| false, false, 0.0);
        assert!(!keyboard.go_menu_visible(0.1));
        assert!(keyboard.go_menu_visible(0.3));
        assert!(
            keyboard.go_menu_visible(2.5),
            "it outlives the sequence window by 200ms"
        );
        assert!(!keyboard.go_menu_visible(2.7));
        frame(&mut keyboard, &[down(Key::X)], &live, |_| false, false, 0.5);
        assert!(!keyboard.go_menu_visible(0.5));
    }

    #[test]
    fn held_keys_repeat_on_their_own_clock() {
        let live = [live(0, &["j"], true)];
        let mut keyboard = Keyboard::default();
        let held = |key| key == Key::J;
        assert_eq!(
            frame(&mut keyboard, &[down(Key::J)], &live, held, false, 0.0),
            [Fire::Press(0)]
        );
        assert!(
            frame(&mut keyboard, &[], &live, held, false, 0.2).is_empty(),
            "250ms delay"
        );
        assert_eq!(
            frame(&mut keyboard, &[], &live, held, false, 0.26),
            [Fire::Repeat]
        );
        assert!(frame(&mut keyboard, &[], &live, held, false, 0.3).is_empty());
        assert_eq!(
            frame(&mut keyboard, &[], &live, held, false, 0.33),
            [Fire::Repeat]
        );
        let os_repeat = KeyEvent::Down {
            key: Key::J,
            modifiers: Modifiers::NONE,
            repeat: true,
            typed: None,
        };
        assert!(
            frame(&mut keyboard, &[os_repeat], &live, held, false, 0.34).is_empty(),
            "OS repeats only feed the failsafe"
        );
        assert_eq!(
            frame(
                &mut keyboard,
                &[KeyEvent::Up { key: Key::J }],
                &live,
                held,
                false,
                0.35
            ),
            [Fire::Release]
        );
    }

    #[test]
    fn held_shifted_symbols_repeat() {
        let live = [live(0, &["?"], true)];
        let mut keyboard = Keyboard::default();
        let held = |key| key == Key::Questionmark;
        let question = KeyEvent::Down {
            key: Key::Questionmark,
            modifiers: Modifiers::SHIFT,
            repeat: false,
            typed: None,
        };
        assert_eq!(
            frame(&mut keyboard, &[question], &live, held, false, 0.0),
            [Fire::Press(0)]
        );
        assert_eq!(
            frame(&mut keyboard, &[], &live, held, false, 0.26),
            [Fire::Repeat]
        );
    }

    #[test]
    fn a_late_frame_makes_the_next_repeat_due_at_once() {
        let live = [live(0, &["j"], true)];
        let mut keyboard = Keyboard::default();
        let held = |key| key == Key::J;
        frame(&mut keyboard, &[down(Key::J)], &live, held, false, 0.0);
        assert_eq!(
            frame(&mut keyboard, &[], &live, held, false, 0.6),
            [Fire::Repeat]
        );
        assert_eq!(
            keyboard.next_change(0.6),
            Some(0.6),
            "or the repeat stalls until the next event"
        );
    }

    #[test]
    fn os_repeat_is_swallowed_outside_text_fields() {
        let live = [live(0, &["e"], false)];
        let repeat = KeyEvent::Down {
            key: Key::E,
            modifiers: Modifiers::NONE,
            repeat: true,
            typed: None,
        };
        assert!(
            frame(
                &mut Keyboard::default(),
                &[repeat],
                &live,
                |_| true,
                false,
                0.0
            )
            .is_empty()
        );
    }

    #[test]
    fn escape_answers_to_backtick_and_ctrl_bracket() {
        let live = [live(0, &["esc"], false)];
        let ctrl_bracket = KeyEvent::Down {
            key: Key::OpenBracket,
            modifiers: Modifiers::CTRL,
            repeat: false,
            typed: None,
        };
        for event in [down(Key::Escape), down(Key::Backtick), ctrl_bracket] {
            assert_eq!(
                frame(
                    &mut Keyboard::default(),
                    &[event],
                    &live,
                    |_| false,
                    false,
                    0.0
                ),
                [Fire::Press(0)]
            );
        }
    }

    #[test]
    fn a_command_can_leave_backtick_to_be_typed() {
        // Compose's Escape with Backtick as Escape off, under the list's.
        let live = [
            live(0, &["esc"], false),
            Live {
                backtick: false,
                ..live(1, &["esc"], false)
            },
        ];
        assert_eq!(
            frame(
                &mut Keyboard::default(),
                &[down(Key::Escape)],
                &live,
                |_| false,
                true,
                0.0
            ),
            [Fire::Press(1)]
        );
        assert_eq!(
            frame(
                &mut Keyboard::default(),
                &[down(Key::Backtick)],
                &live,
                |_| false,
                true,
                0.0
            ),
            [Fire::Press(0)],
            "the next one down takes it"
        );
        assert!(
            frame(
                &mut Keyboard::default(),
                &[down(Key::Backtick)],
                &live[1..],
                |_| false,
                true,
                0.0
            )
            .is_empty(),
            "nothing takes it: it's typed"
        );
    }

    #[test]
    fn a_binding_for_what_a_key_types_needs_it_typed() {
        let live = [Live {
            typing: Some("#"),
            ..live(0, &["\\", "alt+3"], false)
        }];
        let press = |key, modifiers, typed: Option<&str>| {
            frame(
                &mut Keyboard::default(),
                &[KeyEvent::Down {
                    key,
                    modifiers,
                    repeat: false,
                    typed: typed.map(str::to_owned),
                }],
                &live,
                |_| false,
                false,
                0.0,
            )
        };
        assert_eq!(
            press(Key::Backslash, Modifiers::NONE, Some("#")),
            [Fire::Press(0)],
            "a UK keyboard's # key"
        );
        assert_eq!(
            press(Key::Num3, Modifiers::ALT, Some("#")),
            [Fire::Press(0)],
            "a UK Mac's ⌥3"
        );
        assert!(
            press(Key::Backslash, Modifiers::NONE, Some("\\")).is_empty(),
            "a US keyboard's backslash"
        );
        assert!(
            press(Key::Num3, Modifiers::ALT, Some("£")).is_empty(),
            "a US Mac's ⌥3"
        );
    }
}
