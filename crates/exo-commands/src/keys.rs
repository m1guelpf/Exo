//! Key bindings in Mousetrap's notation.
//!
//! A binding is a string such as `"e"`, `"shift+e"`, `"mod+k"`, `"#"` or `"g i"` (a sequence:
//! `g`, then `i`). `mod` is ⌘ on a Mac and Ctrl elsewhere; `ctralt` is Ctrl on a Mac and Alt
//! elsewhere. Shifted symbols mean Shift plus the US-layout key that types them, so `"#"` is
//! Shift+3 and `"?"` is Shift+/, exactly as Mousetrap binds them.

use egui::{Key, Modifiers, os::OperatingSystem};

/// A key plus the exact set of modifiers held with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Chord {
    pub(crate) key: Key,
    pub(crate) shift: bool,
    pub(crate) ctrl: bool,
    pub(crate) alt: bool,
    pub(crate) meta: bool,
}

impl Chord {
    pub(crate) const ESCAPE: Self = Self::plain(Key::Escape);

    const fn plain(key: Key) -> Self {
        Self {
            key,
            shift: false,
            ctrl: false,
            alt: false,
            meta: false,
        }
    }

    /// The chord a key press is, with shifted symbols folded back onto their US-layout key: egui
    /// reports Shift+/ as `?` (logical key) but Shift+3 as `3` (`#` has no `Key`), so both end up
    /// as base key plus Shift.
    pub(crate) fn from_event(key: Key, modifiers: Modifiers) -> Self {
        let base = unshift(key);
        Self {
            key: base.unwrap_or(key),
            shift: modifiers.shift || base.is_some(),
            ctrl: modifiers.ctrl,
            alt: modifiers.alt,
            meta: modifiers.mac_cmd,
        }
    }

    /// Whether this is Escape or one of the keys that stand in for it: backtick and, on a Mac,
    /// Ctrl+[.
    pub(crate) fn is_escape(self, os: OperatingSystem) -> bool {
        let ctrl_bracket = Self {
            ctrl: true,
            ..Self::plain(Key::OpenBracket)
        };
        self == Self::ESCAPE
            || self == Self::plain(Key::Backtick)
            || (os == OperatingSystem::Mac && self == ctrl_bracket)
    }
}

/// The US-layout key a shifted symbol is typed with (Mousetrap's `_SHIFT_MAP`).
fn unshift(key: Key) -> Option<Key> {
    Some(match key {
        Key::Exclamationmark => Key::Num1,
        Key::Questionmark => Key::Slash,
        Key::Colon => Key::Semicolon,
        Key::Pipe => Key::Backslash,
        Key::Plus => Key::Equals,
        Key::OpenCurlyBracket => Key::OpenBracket,
        Key::CloseCurlyBracket => Key::CloseBracket,
        _ => return None,
    })
}

/// One alternative of a binding: a single chord, or a sequence of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Binding {
    pub(crate) chords: Vec<Chord>,
}

impl Binding {
    /// Parses one Mousetrap binding (`"mod+shift+k"`, `"g i"`) for `os`, or `None` if it isn't
    /// valid notation.
    pub(crate) fn parse(spec: &str, os: OperatingSystem) -> Option<Self> {
        let chords = steps(spec)
            .map(|step| parse_chord(step, os))
            .collect::<Option<Vec<_>>>()?;
        (!chords.is_empty()).then_some(Self { chords })
    }
}

/// A binding's steps: `"g shift+m"` → `g`, `shift+m`.
fn steps(spec: &str) -> impl Iterator<Item = &str> {
    spec.split(' ').filter(|step| !step.is_empty())
}

/// Splits a step into its modifiers and its key: `"mod+shift+k"` → `mod+shift`, `k`. A `+` alone,
/// or after another `+`, is the plus key: `"+"` → `+`; `"mod++"` → `mod`, `+`.
fn split_step(step: &str) -> (&str, &str) {
    match step.strip_suffix('+') {
        Some(rest) if rest.is_empty() || rest.ends_with('+') => (rest.trim_end_matches('+'), "+"),
        _ => step.rsplit_once('+').unwrap_or(("", step)),
    }
}

fn parse_chord(step: &str, os: OperatingSystem) -> Option<Chord> {
    let mac = os == OperatingSystem::Mac;
    let (modifiers, key) = split_step(step);
    let mut chord = key_named(key)?;
    for modifier in modifiers.split('+').filter(|modifier| !modifier.is_empty()) {
        match modifier {
            "shift" => chord.shift = true,
            "ctrl" | "control" => chord.ctrl = true,
            "alt" | "option" => chord.alt = true,
            "meta" | "cmd" | "command" => chord.meta = true,
            "mod" if mac => chord.meta = true,
            "mod" => chord.ctrl = true,
            "ctralt" if mac => chord.ctrl = true,
            "ctralt" => chord.alt = true,
            _ => return None,
        }
    }
    Some(chord)
}

/// A Mousetrap key name as a chord (with Shift for shifted symbols).
fn key_named(name: &str) -> Option<Chord> {
    let shifted = |key| Chord {
        shift: true,
        ..Chord::plain(key)
    };
    let key = match name {
        "backspace" => Key::Backspace,
        "tab" => Key::Tab,
        "enter" | "return" => Key::Enter,
        "esc" | "escape" => Key::Escape,
        "space" => Key::Space,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "end" => Key::End,
        "home" => Key::Home,
        "left" => Key::ArrowLeft,
        "up" => Key::ArrowUp,
        "right" => Key::ArrowRight,
        "down" => Key::ArrowDown,
        "ins" => Key::Insert,
        "del" => Key::Delete,
        "plus" | "+" => return Some(shifted(Key::Equals)),
        ";" => Key::Semicolon,
        "=" => Key::Equals,
        "," => Key::Comma,
        "-" => Key::Minus,
        "." => Key::Period,
        "/" => Key::Slash,
        "`" => Key::Backtick,
        "[" => Key::OpenBracket,
        "\\" => Key::Backslash,
        "]" => Key::CloseBracket,
        "'" => Key::Quote,
        "~" => return Some(shifted(Key::Backtick)),
        "!" => return Some(shifted(Key::Num1)),
        "@" => return Some(shifted(Key::Num2)),
        "#" => return Some(shifted(Key::Num3)),
        "$" => return Some(shifted(Key::Num4)),
        "%" => return Some(shifted(Key::Num5)),
        "^" => return Some(shifted(Key::Num6)),
        "&" => return Some(shifted(Key::Num7)),
        "*" => return Some(shifted(Key::Num8)),
        "(" => return Some(shifted(Key::Num9)),
        ")" => return Some(shifted(Key::Num0)),
        "_" => return Some(shifted(Key::Minus)),
        ":" => return Some(shifted(Key::Semicolon)),
        "\"" => return Some(shifted(Key::Quote)),
        "<" => return Some(shifted(Key::Comma)),
        ">" => return Some(shifted(Key::Period)),
        "?" => return Some(shifted(Key::Slash)),
        "|" => return Some(shifted(Key::Backslash)),
        _ => Key::from_name(name)
            .filter(|key| name.len() == 1 || key.name().eq_ignore_ascii_case(name))?,
    };
    Some(Chord::plain(key))
}

/// The keycap tokens a binding is displayed with (split on `+`, with `then` between a sequence's
/// steps): `"mod+shift+k"` → `mod` `shift` `k`, `"g i"` → `g` `then` `i`.
pub(crate) fn display_tokens(spec: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    for (i, step) in steps(spec).enumerate() {
        if i > 0 {
            tokens.push("then");
        }
        let (modifiers, key) = split_step(step);
        tokens.extend(modifiers.split('+').filter(|modifier| !modifier.is_empty()));
        tokens.push(key);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: OperatingSystem = OperatingSystem::Mac;

    fn chord(spec: &str) -> Chord {
        let binding = Binding::parse(spec, MAC).expect("valid");
        assert_eq!(binding.chords.len(), 1);
        binding.chords[0]
    }

    #[test]
    fn shifted_symbols_bind_like_mousetrap() {
        let event = |key, shift| {
            Chord::from_event(
                key,
                Modifiers {
                    shift,
                    ..Modifiers::NONE
                },
            )
        };
        assert_eq!(chord("#"), event(Key::Num3, true));
        assert_eq!(chord("?"), event(Key::Questionmark, true));
        assert_eq!(chord("shift+/"), event(Key::Questionmark, true));
        assert_eq!(chord("!"), event(Key::Exclamationmark, true));
        assert_eq!(chord("shift+e"), event(Key::E, true));
        assert_ne!(
            chord("e"),
            event(Key::E, true),
            "modifiers must match exactly"
        );
    }

    #[test]
    fn mod_and_ctralt_follow_the_platform() {
        assert!(chord("mod+k").meta && !chord("mod+k").ctrl);
        let windows = Binding::parse("mod+k", OperatingSystem::Windows)
            .expect("valid")
            .chords[0];
        assert!(windows.ctrl && !windows.meta);
        assert!(chord("ctralt+j").ctrl);
        assert!(
            Binding::parse("ctralt+j", OperatingSystem::Windows)
                .expect("valid")
                .chords[0]
                .alt
        );
    }

    #[test]
    fn parses_sequences_and_names() {
        assert_eq!(
            Binding::parse("g shift+m", MAC)
                .expect("valid")
                .chords
                .len(),
            2
        );
        assert_eq!(chord("esc"), Chord::ESCAPE);
        assert_eq!(chord("mod+shift+,").key, Key::Comma);
        assert_eq!(chord("plus"), chord("shift+="));
        assert_eq!(chord("+"), chord("shift+="));
        assert_eq!(
            chord("mod++"),
            Chord {
                meta: true,
                ..chord("+")
            }
        );
        assert!(Binding::parse("mod+nope", MAC).is_none());
        assert!(Binding::parse("mod+", MAC).is_none());
        assert!(Binding::parse("", MAC).is_none());
    }

    #[test]
    fn escape_has_stand_ins() {
        let escape = |key, modifiers, os| Chord::from_event(key, modifiers).is_escape(os);
        assert!(escape(Key::Escape, Modifiers::NONE, MAC));
        assert!(escape(Key::Backtick, Modifiers::NONE, MAC));
        assert!(escape(Key::OpenBracket, Modifiers::CTRL, MAC));
        assert!(!escape(
            Key::OpenBracket,
            Modifiers::CTRL,
            OperatingSystem::Windows
        ));
        assert!(!escape(Key::Escape, Modifiers::SHIFT, MAC));
    }

    #[test]
    fn display_tokens_split_on_plus_and_between_steps() {
        assert_eq!(display_tokens("mod+shift+i"), ["mod", "shift", "i"]);
        assert_eq!(display_tokens("g i"), ["g", "then", "i"]);
        assert_eq!(display_tokens("#"), ["#"]);
        assert_eq!(display_tokens("mod++"), ["mod", "+"]);
        assert_eq!(display_tokens("+"), ["+"]);
    }
}
