//! Toasts with undo, and hint tooltips.
//!
//! There is one toast at a time: a new one replaces the current one. It stays 8s, fades out over
//! 2s (`ease-in`) and is gone; hovering cancels both timers and brings a fading toast back over
//! 0.2s, and leaving restarts the 8s. Its actions are bound to their keys (`z` / ⌘Z for Undo) only
//! while it exists: undo is per toast, there is no history. Hints are a separate single slot shown
//! for 10s that fades out over 500ms.

use std::borrow::Cow;

use egui::{Context, Rect};
use exo_ui::{
    HintTooltip, TOAST_BOTTOM, ToastActionView, ToastKind, ToastView, TooltipPart,
    motion::CubicBezier,
};

/// A toast shows for 8s, then fades out over 2s.
const VISIBLE: f64 = 8.0;
const FADE: f64 = 2.0;
/// A hovered fading toast comes back over 0.2s.
const RESTORE: f64 = 0.2;
/// How long a hint shows.
const HINT_DURATION: f64 = 10.0;
/// A hint fades in over 500ms and out over 500ms ease-in-out.
const HINT_FADE: f64 = 0.5;
/// How far above the area's bottom a hint sits.
const HINT_BOTTOM: f32 = 18.0;

/// A toast: what an action did, with a way to undo it.
#[derive(Clone, Debug)]
#[must_use]
pub struct Toast<A> {
    text: Cow<'static, str>,
    kind: ToastKind,
    actions: Vec<ToastAction<A>>,
    auto_hide: bool,
    /// The lines between actions show without hovering.
    divided: bool,
    /// Correlates the toasts of one flow: a send's toast is updated only while it's still the one
    /// showing.
    tag: Option<u64>,
}

/// An action on a toast.
#[derive(Clone, Debug)]
struct ToastAction<A> {
    name: Cow<'static, str>,
    key: Option<ToastKey>,
    action: A,
    keep_on_action: bool,
    hidden: bool,
    tooltip: Option<Cow<'static, str>>,
}

/// The keys that run a toast's actions while it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToastKey {
    /// `z` and ⌘Z, which both run whichever action is keyed with either.
    Undo,
    /// ⌘⇧Z.
    InstantSend,
}

impl ToastKey {
    const ALL: [Self; 2] = [Self::Undo, Self::InstantSend];

    fn parse(key: &str) -> Option<Self> {
        match key {
            "z" | "mod+z" => Some(Self::Undo),
            "mod+shift+z" => Some(Self::InstantSend),
            _ => None,
        }
    }

    /// The bindings that run it.
    pub(crate) fn bindings(self) -> &'static [&'static str] {
        match self {
            Self::Undo => &["z", "mod+z"],
            Self::InstantSend => &["mod+shift+z"],
        }
    }

    /// The keycaps its action's tooltip shows.
    fn keycaps(self) -> &'static [&'static str] {
        match self {
            Self::Undo => &["z"],
            Self::InstantSend => &["mod", "shift", "z"],
        }
    }

    /// The hint shown after its action is clicked instead.
    fn mouse_hint(self) -> Hint {
        let hint = Hint::new().text("Next time, hit");
        match self {
            Self::Undo => hint.keys(["Z"]).text("to undo instantly!"),
            Self::InstantSend => hint.keys(["⌘", "shift", "Z"]).text("to Instant Send!"),
        }
    }
}

impl<A> Toast<A> {
    /// A toast reading `text` (`Marked as Done.`).
    pub fn new(text: impl Into<Cow<'static, str>>) -> Self {
        Self {
            text: text.into(),
            kind: ToastKind::Info,
            actions: Vec::new(),
            auto_hide: true,
            divided: false,
            tag: None,
        }
    }

    /// Shows the lines between its actions without hovering.
    pub fn divided(mut self) -> Self {
        self.divided = true;
        self
    }

    /// Adds `Undo`, run by `z` and ⌘Z.
    pub fn undo(self, undo: A) -> Self {
        self.action("Undo", Some("z"), undo)
    }

    /// Adds an action labelled `name`, optionally run by `key` (`"z"`, `"mod+z"` or
    /// `"mod+shift+z"`) while the toast shows.
    pub fn action(
        mut self,
        name: impl Into<Cow<'static, str>>,
        key: Option<&'static str>,
        action: A,
    ) -> Self {
        let parsed = key.and_then(ToastKey::parse);
        debug_assert_eq!(
            parsed.is_some(),
            key.is_some(),
            "a toast action's key is z, mod+z or mod+shift+z"
        );
        self.actions.push(ToastAction {
            name: name.into(),
            key: parsed,
            action,
            keep_on_action: false,
            hidden: false,
            tooltip: None,
        });
        self
    }

    /// Shows the last action only while the toast is hovered (Instant Send).
    pub fn hidden(mut self) -> Self {
        if let Some(action) = self.actions.last_mut() {
            action.hidden = true;
        }
        self
    }

    /// Titles the last action's key tooltip, instead of its name.
    pub fn tooltip(mut self, title: impl Into<Cow<'static, str>>) -> Self {
        if let Some(action) = self.actions.last_mut() {
            action.tooltip = Some(title.into());
        }
        self
    }

    /// Tags the toast as part of one flow, for [`crate::Commands::toast_tag`].
    pub fn tag(mut self, tag: u64) -> Self {
        self.tag = Some(tag);
        self
    }

    /// Keeps the toast up after its last action runs (Instant Send).
    pub fn keep_on_action(mut self) -> Self {
        if let Some(action) = self.actions.last_mut() {
            action.keep_on_action = true;
        }
        self
    }

    /// Sets the accent.
    pub fn kind(mut self, kind: ToastKind) -> Self {
        self.kind = kind;
        self
    }

    /// Keeps the toast until it's replaced or dismissed (as for `Sending…`).
    pub fn sticky(mut self) -> Self {
        self.auto_hide = false;
        self
    }
}

/// A hint tooltip's line: words and keycaps.
#[derive(Clone, Debug, PartialEq)]
#[must_use]
pub struct Hint {
    parts: Vec<HintPart>,
    duration: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum HintPart {
    Text(Cow<'static, str>),
    Keys(Vec<Cow<'static, str>>),
}

impl Hint {
    /// An empty hint, shown for 10s.
    pub fn new() -> Self {
        Self {
            parts: Vec::new(),
            duration: HINT_DURATION,
        }
    }

    /// Appends words (`Next time, hit`).
    pub fn text(mut self, text: impl Into<Cow<'static, str>>) -> Self {
        self.parts.push(HintPart::Text(text.into()));
        self
    }

    /// Appends keycaps, one per token (`["mod", "k"]`).
    pub fn keys<I: IntoIterator<Item = S>, S: Into<Cow<'static, str>>>(
        mut self,
        tokens: I,
    ) -> Self {
        self.parts
            .push(HintPart::Keys(tokens.into_iter().map(Into::into).collect()));
        self
    }

    /// Shows the hint for `seconds` instead of 10 (2s for the Open Links hints).
    pub fn duration(mut self, seconds: f64) -> Self {
        self.duration = seconds;
        self
    }
}

impl Default for Hint {
    fn default() -> Self {
        Self::new()
    }
}

/// The toast and hint slots.
pub(crate) struct Feedback<A> {
    toast: Option<Shown<A>>,
    hint: Option<ShownHint>,
    /// Toast ids keep each toast's animations its own.
    serial: u64,
    /// How far above the area's bottom the toast sits.
    pub(crate) toast_bottom: f32,
}

struct Shown<A> {
    toast: Toast<A>,
    serial: u64,
    timer: Timer,
}

/// Where a toast's timers are.
#[derive(Clone, Copy)]
enum Timer {
    /// Shown for 8s from `since`, then fading out for 2s (if it auto-hides).
    Running { since: f64 },
    /// Hovered, so held: coming back from opacity `from` over 0.2s since `at`.
    Paused { at: f64, from: f32 },
}

struct ShownHint {
    hint: Hint,
    shown_at: f64,
    /// When it started fading out (time up or clicked).
    hiding_at: Option<f64>,
}

impl<A: Clone> Feedback<A> {
    pub(crate) const fn new() -> Self {
        Self {
            toast: None,
            hint: None,
            serial: 0,
            toast_bottom: TOAST_BOTTOM,
        }
    }

    pub(crate) fn show_toast(&mut self, toast: Toast<A>, now: f64) {
        self.serial += 1;
        self.toast = Some(Shown {
            toast,
            serial: self.serial,
            timer: Timer::Running { since: now },
        });
    }

    pub(crate) fn hide_toast(&mut self) {
        self.toast = None;
    }

    /// The showing toast's text.
    pub(crate) fn toast_text(&self) -> Option<&str> {
        self.toast.as_ref().map(|shown| shown.toast.text.as_ref())
    }

    /// The showing toast's tag.
    pub(crate) fn toast_tag(&self) -> Option<u64> {
        self.toast.as_ref().and_then(|shown| shown.toast.tag)
    }

    /// The pointer is over the showing toast.
    pub(crate) fn toast_hovered(&self) -> bool {
        self.toast
            .as_ref()
            .is_some_and(|shown| matches!(shown.timer, Timer::Paused { .. }))
    }

    /// The showing hint's words and keycaps, one space apart.
    pub(crate) fn hint_text(&self) -> Option<String> {
        let shown = self
            .hint
            .as_ref()
            .filter(|shown| shown.hiding_at.is_none())?;
        let parts: Vec<String> = shown
            .hint
            .parts
            .iter()
            .map(|part| match part {
                HintPart::Text(text) => text.to_string(),
                HintPart::Keys(keys) => keys.join(" "),
            })
            .collect();
        Some(parts.join(" "))
    }

    pub(crate) fn show_hint(&mut self, hint: Hint, now: f64) {
        self.hint = Some(ShownHint {
            hint,
            shown_at: now,
            hiding_at: None,
        });
    }

    /// The keys the current toast's actions take.
    pub(crate) fn toast_keys(&self) -> Vec<ToastKey> {
        let actions = self
            .toast
            .as_ref()
            .map_or(&[][..], |shown| &shown.toast.actions);
        ToastKey::ALL
            .into_iter()
            .filter(|&key| actions.iter().any(|action| action.key == Some(key)))
            .collect()
    }

    /// Runs the current toast's first action keyed `key`.
    pub(crate) fn press(&mut self, key: ToastKey) -> Option<A> {
        let index = self
            .toast
            .as_ref()?
            .toast
            .actions
            .iter()
            .position(|action| action.key == Some(key))?;
        self.run(index)
    }

    fn run(&mut self, index: usize) -> Option<A> {
        let shown = self.toast.as_ref()?;
        let action = shown.toast.actions.get(index)?;
        let result = action.action.clone();
        if !action.keep_on_action {
            self.toast = None;
        }
        Some(result)
    }

    /// Draws the toast and hint in `area` and returns the action a click ran.
    pub(crate) fn show(&mut self, ctx: &Context, area: Rect, now: f64) -> Option<A> {
        let ran = self.show_toast_frame(ctx, area, now);
        self.show_hint_frame(ctx, area, now);
        ran
    }

    fn show_toast_frame(&mut self, ctx: &Context, area: Rect, now: f64) -> Option<A> {
        let shown = self.toast.as_mut()?;
        if shown.expired(now) {
            self.toast = None;
            return None;
        }
        let opacity = shown.opacity(now);
        let views: Vec<_> = shown
            .toast
            .actions
            .iter()
            .map(|action| ToastActionView {
                name: &action.name,
                keys: action.key.map_or(&[], ToastKey::keycaps),
                tooltip: action.tooltip.as_deref(),
                hidden: action.hidden,
                disabled: false,
            })
            .collect();
        let view = ToastView {
            text: &shown.toast.text,
            kind: shown.toast.kind,
            actions: &views,
            divided: shown.toast.divided,
            percentage: None,
            clickable: false,
        };
        let response = view.show(
            ctx,
            exo_ui::id(ctx, ("exo_commands::toast", shown.serial)),
            area,
            self.toast_bottom,
            opacity,
        );
        match (response.hovered, shown.timer) {
            (true, Timer::Running { .. }) => {
                shown.timer = Timer::Paused {
                    at: now,
                    from: opacity,
                }
            }
            (false, Timer::Paused { .. }) => shown.timer = Timer::Running { since: now },
            _ => {}
        }
        if let Some(at) = shown.next_change(now) {
            ctx.request_repaint_after_secs((at - now) as f32);
        }
        if response.closed {
            self.toast = None;
            return None;
        }
        let index = response.clicked?;
        let key = shown.toast.actions.get(index).and_then(|action| action.key);
        let ran = self.run(index);
        if let Some(key) = key {
            self.show_hint(key.mouse_hint(), now);
        }
        ran
    }

    fn show_hint_frame(&mut self, ctx: &Context, area: Rect, now: f64) {
        let Some(shown) = &mut self.hint else {
            return;
        };
        let hiding_at = shown
            .hiding_at
            .unwrap_or(shown.shown_at + shown.hint.duration);
        if now >= hiding_at + HINT_FADE {
            self.hint = None;
            return;
        }
        let opacity = if now < hiding_at {
            CubicBezier::EASE.at(((now - shown.shown_at) / HINT_FADE).min(1.0) as f32)
        } else {
            1.0 - CubicBezier::EASE_IN_OUT.at(((now - hiding_at) / HINT_FADE) as f32)
        };
        let keys: Vec<Vec<&str>> = shown
            .hint
            .parts
            .iter()
            .map(|part| match part {
                HintPart::Keys(tokens) => tokens.iter().map(AsRef::as_ref).collect(),
                HintPart::Text(_) => Vec::new(),
            })
            .collect();
        let parts: Vec<TooltipPart<'_>> = shown
            .hint
            .parts
            .iter()
            .zip(&keys)
            .map(|(part, keys)| match part {
                HintPart::Text(text) => TooltipPart::Text(text),
                HintPart::Keys(_) => TooltipPart::Keys(keys),
            })
            .collect();
        let response = HintTooltip { parts: &parts }.show(
            ctx,
            exo_ui::id(ctx, "exo_commands::hint"),
            area,
            HINT_BOTTOM,
            opacity,
        );
        if response.clicked() && now < hiding_at {
            shown.hiding_at = Some(now);
        }
        let animating = now < shown.shown_at + HINT_FADE || now >= hiding_at;
        ctx.request_repaint_after_secs(if animating {
            0.0
        } else {
            (hiding_at - now) as f32
        });
    }
}

impl<A> Shown<A> {
    /// Whether it has faded out.
    fn expired(&self, now: f64) -> bool {
        matches!(self.timer, Timer::Running { since } if self.toast.auto_hide && now - since >= VISIBLE + FADE)
    }

    /// The card's opacity: 1, fading out over 2s ease-in, or coming back over 0.2s ease-in.
    fn opacity(&self, now: f64) -> f32 {
        match self.timer {
            Timer::Running { since } if self.toast.auto_hide => {
                1.0 - CubicBezier::EASE_IN.at(((now - since - VISIBLE) / FADE) as f32)
            }
            Timer::Running { .. } => 1.0,
            Timer::Paused { at, from } => {
                from + (1.0 - from) * CubicBezier::EASE_IN.at(((now - at) / RESTORE) as f32)
            }
        }
    }

    /// When the card changes next on its own: the fade's start, or now while it animates.
    fn next_change(&self, now: f64) -> Option<f64> {
        match self.timer {
            Timer::Running { since } if self.toast.auto_hide => Some((since + VISIBLE).max(now)),
            Timer::Running { .. } => None,
            Timer::Paused { at, from } => (from < 1.0 && now < at + RESTORE).then_some(now),
        }
    }
}
