#![deny(
    clippy::correctness,
    clippy::suspicious,
    clippy::complexity,
    clippy::perf
)]
//! Exo's keyboard: key dispatch, the command registry, the ⌘K palette, the shortcuts sheet and
//! toasts with undo.
//!
//! # The model
//!
//! There is no central keymap. Whatever is on screen registers [`Command`]s: a title makes one a
//! palette row, keys make it a shortcut, and usually it's both, so a row and its shortcut can't
//! drift apart. Each command is scoped to [focus regions](Region): it's live only while one of
//! them has the keyboard (for rows: was active when the palette opened). Running a command hands
//! back its action, a value of the app's own type `A`; the app applies it.
//!
//! # Wiring it into the app
//!
//! The app owns one [`Commands`] and calls four things each frame:
//!
//! ```ignore
//! #[derive(Clone)]
//! enum Action { Down, ScanDown, ScanStop, MarkDone, OpenLabels, AddLabel(LabelId), Undo(UndoToken) }
//!
//! struct App { commands: exo_commands::Commands<Action>, inbox: Inbox }
//!
//! fn ui(&mut self, ui: &mut egui::Ui) {
//!     // 1. First, before anything draws: this frame's keys, which go to the commands registered
//!     //    last frame. Keys a command takes never reach a widget.
//!     for action in self.commands.dispatch(ui.ctx()) {
//!         self.apply(action);
//!     }
//!     // 2. Which region has the keyboard: `ThreadList`, `ThreadMessages` once a thread is
//!     //    open, `SearchBar` while searching, `ComposeForm` in a draft, …
//!     self.commands.set_region(self.inbox.region());
//!     // 3. Draw the screens. Each registers its commands while it's on screen, every frame.
//!     self.inbox.ui(ui, &mut self.commands);
//!     // 4. Last: the overlays (palette, shortcuts sheet, toast, hint). `area` is the window
//!     //    below the tab bar.
//!     for action in self.commands.show(ui.ctx(), area) {
//!         self.apply(action);
//!     }
//! }
//! ```
//!
//! # Adding a screen's commands
//!
//! A screen registers its commands as it draws, each with its title, keys and score:
//!
//! ```ignore
//! use exo_commands::{Command, CONVERSATION, LIST_AND_THREAD, Region};
//!
//! // Keyboard only; held, it repeats every 65ms after 250ms.
//! commands.add(Command::shortcut("NEXT", Action::Down).keys(["j", "down", "n", "ctralt+j", "ctralt+n"])
//!     .regions(&[Region::ThreadList]).repeat(Action::ScanDown).on_release(Action::ScanStop));
//! // A palette row and its shortcut, from one definition.
//! commands.add(Command::new("MARK_DONE", "Mark Done", Action::MarkDone).key("e").aliases(["Archive"])
//!     .default_score(11.0).icon("data/done").multiple().regions(CONVERSATION));
//! // A sequence.
//! commands.add(Command::new("inbox", "go to Inbox", Action::GoInbox).key("g i").default_score(1.0)
//!     .scale(0.6).operative_word("Inbox").query_only().regions(LIST_AND_THREAD));
//! ```
//!
//! Register a command only while it applies: `Mark not Done` instead of `Mark Done` when everything
//! is done, nothing for features that don't exist yet. Commands for features Exo hides (AI,
//! sharing) are never registered. When several live commands bind the same key, the one registered
//! last wins.
//!
//! Results go in the toast slot; its Undo also runs on `z` / ⌘Z while it shows:
//!
//! ```ignore
//! commands.toast(Toast::new("Marked as Done.").undo(Action::Undo(token)));
//! ```
//!
//! Sub-palettes (Label, Move, …) open in response to an action, and Escape goes back to the main
//! palette if that's where the action came from:
//!
//! ```ignore
//! Action::OpenLabels => commands.open_sub_palette(SubPalette::new("LABEL", "Add label", "data/label",
//!     labels.iter().map(|label| Command::new(label.id, label.name, Action::AddLabel(label.id))).collect())),
//! ```
//!
//! The crate registers its own: ⌘K (palette), `?` and Escape (shortcuts sheet), `z` / ⌘Z / ⌘⇧Z
//! (the toast's actions) and the palette's `Shortcuts` row. The right pane shows the Go to… panel
//! while [`Commands::go_menu_visible`].

mod command;
mod feedback;
mod keyboard;
mod keys;
mod palette;
mod rank;
mod region;
mod score;
mod shortcuts;

use egui::{Context, Event, Rect, os::OperatingSystem};

pub use command::Command;
pub use exo_ui::ToastKind;
pub use feedback::{Hint, Toast};
pub use palette::{Dropdown, SubPalette};
pub use region::{CONVERSATION, LIST_AND_THREAD, Region, Scope};
pub use score::command_score;

use feedback::{Feedback, ToastKey};
use keyboard::{Fire, KeyEvent, Keyboard, Live};
use keys::Binding;
use palette::Palette;
use shortcuts::Shortcuts;

/// A registered command's action: the app's, or one the overlays handle themselves.
#[derive(Clone, Debug)]
enum Act<A> {
    App(A),
    Builtin(Builtin),
}

#[derive(Clone, Copy, Debug)]
enum Builtin {
    TogglePalette,
    OpenShortcuts,
    ToggleShortcuts,
    CloseShortcuts,
    /// Runs the current toast's action keyed so.
    ToastKey(ToastKey),
}

/// The command registry, key dispatch, and the overlays commands drive: the ⌘K palette, the
/// shortcuts sheet, the toast and the hint tooltip. See the [crate docs](crate) for the wiring.
pub struct Commands<A> {
    region: Region,
    multiple_selected: bool,
    /// This frame's registrations, in order.
    registered: Vec<Command<Act<A>>>,
    /// Last frame's, which this frame's keys go to: keys are dispatched before anything draws.
    previous: Vec<Command<Act<A>>>,
    keyboard: Keyboard,
    /// The repeat and release actions of the command whose key is held, from when it was pressed
    /// (registrations change from frame to frame; a held key keeps doing what it started).
    held: Option<(Act<A>, Option<Act<A>>)>,
    palette: Palette<Act<A>>,
    shortcuts: Shortcuts,
    feedback: Feedback<A>,
    now: f64,
    /// The modifiers held as this frame's palette row ran.
    ran_with: egui::Modifiers,
}

impl<A: Clone + 'static> Default for Commands<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: Clone + 'static> Commands<A> {
    /// An empty registry with the thread list focused.
    pub fn new() -> Self {
        Self {
            region: Region::ThreadList,
            multiple_selected: false,
            registered: Vec::new(),
            previous: Vec::new(),
            keyboard: Keyboard::default(),
            held: None,
            palette: Palette::new(),
            shortcuts: Shortcuts::default(),
            feedback: Feedback::new(),
            now: 0.0,
            ran_with: egui::Modifiers::NONE,
        }
    }

    /// Sets the region the app's focus is in (the "primary" region: the palette and the shortcuts
    /// sheet take the keyboard while open and give it back here).
    pub fn set_region(&mut self, region: Region) {
        self.region = region;
    }

    /// The region that has the keyboard: the palette's or the sheet's while one is open.
    pub fn region(&self) -> Region {
        if self.palette.is_open() {
            Region::Command
        } else if self.shortcuts.is_open() {
            Region::ShortcutsHelp
        } else {
            self.region
        }
    }

    /// Says whether several conversations are selected; the palette then lists only commands
    /// marked [`Command::multiple`].
    pub fn set_multiple_selected(&mut self, multiple: bool) {
        self.multiple_selected = multiple;
    }

    /// Registers `command` for this frame. Call every frame the command's screen is on screen,
    /// between [`Self::dispatch`] and [`Self::show`]; the next frame's keys go to it.
    pub fn add(&mut self, command: Command<A>) {
        self.registered.push(command.map(Act::App));
    }

    /// Opens the main palette, or (with `initial`) opens it with that typed (`g l` opens it on
    /// `go to `).
    pub fn open_palette(&mut self, initial: Option<&str>) {
        self.palette.open_main(initial);
    }

    /// Opens a sub-palette, replacing the main palette if it's open.
    pub fn open_sub_palette(&mut self, sub: SubPalette<A>) {
        self.palette.open_sub(sub.into_act());
    }

    /// The open palette's type: `COMMAND_PALETTE` for the main one, else the sub-palette's
    /// (`SNOOZE`).
    pub fn palette_id(&self) -> Option<&'static str> {
        self.palette.open_id()
    }

    /// Shows `nudge` as the main palette's placeholder (`Try: label, …`), in its sliding gradient,
    /// until it's set to `None`.
    pub fn set_palette_nudge(&mut self, nudge: Option<std::borrow::Cow<'static, str>>) {
        self.palette.set_nudge(nudge);
    }

    /// Whether the sub-palette of type `id` (`OPEN`) is open.
    pub fn is_sub_palette_open(&self, id: &str) -> bool {
        self.palette.is_sub_open(id)
    }

    /// The open sub-palette's prompt and the titles it lists for what's typed, its footer's
    /// last (for tests).
    pub fn sub_palette_rows(&self) -> Option<(String, Vec<String>)> {
        self.palette.sub_rows()
    }

    /// The modifiers held as the palette row [`Self::show`] returned ran, by Enter or a click.
    pub fn ran_with(&self) -> egui::Modifiers {
        self.ran_with
    }

    /// Closes the palette.
    pub fn close_palette(&mut self) {
        self.palette.close(false);
    }

    /// Says whether the account's `Send + Mark Done` is on, which the shortcuts sheet's
    /// `Compose` rows show.
    pub fn set_send_and_done(&mut self, on: bool) {
        self.shortcuts.set_send_and_done(on);
    }

    /// Opens the shortcuts sheet.
    pub fn open_shortcuts(&mut self) {
        self.palette.close(false);
        self.shortcuts.open();
    }

    /// Shows `toast`, replacing the current one.
    pub fn toast(&mut self, toast: Toast<A>) {
        self.feedback.show_toast(toast, self.now);
    }

    /// Hides the current toast.
    pub fn hide_toast(&mut self) {
        self.feedback.hide_toast();
    }

    /// The showing toast's text, if any.
    pub fn toast_text(&self) -> Option<&str> {
        self.feedback.toast_text()
    }

    /// The showing toast's [tag](Toast::tag), if any.
    pub fn toast_tag(&self) -> Option<u64> {
        self.feedback.toast_tag()
    }

    /// The pointer is resting on the toast (its timers are held).
    pub fn toast_hovered(&self) -> bool {
        self.feedback.toast_hovered()
    }

    /// Places the toast `bottom` px above the area's bottom edge: [`exo_ui::TOAST_BOTTOM`]
    /// (32px) normally, 68px at Inbox Zero. The app animates it (`transition: bottom 150ms ease`).
    pub fn set_toast_bottom(&mut self, bottom: f32) {
        self.feedback.toast_bottom = bottom;
    }

    /// How far above the area's bottom edge the toast sits ([`set_toast_bottom`](Self::set_toast_bottom)):
    /// where the notifications stacked over it start.
    pub fn toast_bottom(&self) -> f32 {
        self.feedback.toast_bottom
    }

    /// The showing hint tooltip's words and keycaps, one space apart (`Hit ⌘ shift H to …`).
    pub fn hint_text(&self) -> Option<String> {
        self.feedback.hint_text()
    }

    /// Shows a hint tooltip, replacing the current one.
    pub fn hint(&mut self, hint: Hint) {
        self.feedback.show_hint(hint, self.now);
    }

    /// Whether the "Go to…" panel should be in the right pane: an unmodified `g` was pressed where
    /// go-to sequences are bound, at least 200ms ago and less than 2.6s ago, and nothing since.
    pub fn go_menu_visible(&self) -> bool {
        self.keyboard.go_menu_visible(self.now)
    }

    /// Runs this frame's keys through the commands registered last frame and returns the app
    /// actions they ran, in order. Call first every frame, before anything draws: a key a
    /// command takes is removed from the frame's input with the text it types, so no widget
    /// also gets it (compose's `;` inserts a snippet, not a semicolon).
    ///
    /// # Panics
    ///
    /// In debug builds, if a live command's key binding isn't valid Mousetrap notation (they're
    /// literals, so a typo is a bug the first frame it's live shows); release builds skip it.
    pub fn dispatch(&mut self, ctx: &Context) -> Vec<A> {
        let now = ctx.input(|input| input.time);
        self.now = now;
        let os = ctx.os();
        let (region, primary) = (self.region(), self.region);
        let live: Vec<Live> = self
            .previous
            .iter()
            .enumerate()
            .filter_map(|(index, command)| {
                let live = command.bind_keys
                    && (command.scope.contains(region)
                        || (region == Region::Command
                            && command.within_palette
                            && command.scope.contains(primary)));
                live.then(|| Live {
                    index,
                    bindings: parse_keys(command, os),
                    repeats: command.repeat.is_some(),
                    backtick: command.backtick_escape,
                    typing: command.typing,
                })
            })
            .collect();
        let held = ctx.input(|input| input.keys_down.clone());
        let mut fired: Vec<Fire> = self
            .keyboard
            .tick(|key| held.contains(&key), now)
            .into_iter()
            .collect();
        let keyboard = &mut self.keyboard;
        ctx.input_mut(|input| {
            // egui-winit reports a key, then the text it types; a taken key's text goes with it.
            let typed: Vec<Option<String>> = input
                .events
                .iter()
                .skip(1)
                .map(|event| {
                    if let Event::Text(text) = event {
                        Some(text.clone())
                    } else {
                        None
                    }
                })
                .chain([None])
                .collect();
            let mut at = 0;
            let mut taken = false;
            input.events.retain(|event| {
                let typed = typed[at].clone();
                at += 1;
                if std::mem::take(&mut taken) && matches!(event, Event::Text(_)) {
                    return false;
                }
                let event = match *event {
                    Event::Key {
                        key,
                        pressed: true,
                        repeat,
                        modifiers,
                        ..
                    } => KeyEvent::Down {
                        key,
                        modifiers,
                        repeat,
                        typed,
                    },
                    Event::Key {
                        key,
                        pressed: false,
                        ..
                    } => KeyEvent::Up { key },
                    _ => return true,
                };
                let Some(fire) = keyboard.event(event, &live, region.is_text_input(), now, os)
                else {
                    return true;
                };
                fired.push(fire);
                taken = matches!(fire, Fire::Press(_));
                !taken
            });
        });
        let mut ran = Vec::new();
        for fire in fired {
            match fire {
                Fire::Press(index) => {
                    let command = &self.previous[index];
                    if let Some(repeat) = &command.repeat {
                        self.held = Some((repeat.clone(), command.release.clone()));
                    }
                    ran.push(command.action.clone());
                }
                Fire::Repeat => ran.extend(self.held.as_ref().map(|(repeat, _)| repeat.clone())),
                Fire::Release => ran.extend(self.held.take().and_then(|(_, release)| release)),
            }
        }
        if let Some(at) = self.keyboard.next_change(now) {
            ctx.request_repaint_after_secs((at - now) as f32);
        }
        self.run(ran)
    }

    /// Draws the overlays over `area` (the window below the tab bar) and returns the app
    /// actions run from them (a palette row, a toast's action), in order. Call last every
    /// frame, once every screen has registered its commands: the palette lists them, and the
    /// next frame's keys go to them.
    pub fn show(&mut self, ctx: &Context, area: Rect) -> Vec<A> {
        self.now = ctx.input(|input| input.time);
        self.register_builtins();
        let candidates = palette_candidates(&self.registered, self.region, self.multiple_selected);
        let row = self.palette.show(ctx, area, &candidates);
        self.ran_with = row
            .as_ref()
            .map_or(egui::Modifiers::NONE, |row| row.modifiers);
        let mut ran: Vec<Act<A>> = row.map(|row| row.action).into_iter().collect();
        if let Some(hint) = self.palette.take_hint() {
            self.feedback.show_hint(hint, self.now);
        }
        self.shortcuts.show(ctx, area);
        ran.extend(self.feedback.show(ctx, area, self.now).map(Act::App));
        self.previous = std::mem::take(&mut self.registered);
        self.run(ran)
    }

    /// Carries out the overlays' own actions in `ran` and returns the app's.
    fn run(&mut self, ran: Vec<Act<A>>) -> Vec<A> {
        ran.into_iter()
            .filter_map(|act| match act {
                Act::App(action) => Some(action),
                Act::Builtin(builtin) => self.run_builtin(builtin),
            })
            .collect()
    }

    /// Adds the overlays' own commands: their keys first, so the app's can shadow them, and the
    /// palette's `Shortcuts` row last, after the app's rows.
    fn register_builtins(&mut self) {
        let builtin = |id, builtin| Command::shortcut(id, Act::Builtin(builtin));
        let keys = [
            builtin("TOGGLE_COMMAND_PALETTE", Builtin::TogglePalette)
                .keys(["mod+k", "mod+shift+k"])
                .regions(region::PALETTE_TOGGLE),
            builtin("TOGGLE_SHORTCUTS_HELP", Builtin::ToggleShortcuts)
                .key("?")
                .regions(region::SHORTCUTS_TOGGLE),
            builtin("CLOSE_SHORTCUTS_HELP", Builtin::CloseShortcuts)
                .key("esc")
                .regions(&[Region::ShortcutsHelp]),
        ];
        let toast_keys = self.feedback.toast_keys().into_iter().map(|key| {
            builtin("TOAST_ACTION", Builtin::ToastKey(key))
                .keys(key.bindings().iter().copied())
                .regions(region::TOAST_KEYS)
        });
        self.registered
            .splice(0..0, keys.into_iter().chain(toast_keys));
        self.registered.push(
            Command::new(
                "SHORTCUTS_HELP",
                "Shortcuts",
                Act::Builtin(Builtin::OpenShortcuts),
            )
            .aliases([
                "Keyboard Shortcuts",
                "Short Cuts",
                "Hotkeys",
                "Hot Keys",
                "Help",
            ])
            .default_score(1.5)
            .follow("HELP_CENTER")
            .icon("data/icon"),
        );
    }

    fn run_builtin(&mut self, builtin: Builtin) -> Option<A> {
        match builtin {
            // While a sub-palette is open, ⌘K does nothing.
            Builtin::TogglePalette if self.palette.is_main_open() => self.palette.close(true),
            Builtin::TogglePalette if !self.palette.is_open() => self.palette.open_main(None),
            Builtin::TogglePalette => {}
            Builtin::OpenShortcuts => self.open_shortcuts(),
            Builtin::ToggleShortcuts if self.shortcuts.is_open() => self.shortcuts.close(),
            Builtin::ToggleShortcuts => self.open_shortcuts(),
            Builtin::CloseShortcuts => self.shortcuts.close(),
            Builtin::ToastKey(key) => return self.feedback.press(key),
        }
        None
    }
}

/// `command`'s key bindings for `os`. An invalid one is a typo in a literal: a panic in debug
/// builds, skipped in release builds.
fn parse_keys<A>(command: &Command<A>, os: OperatingSystem) -> Vec<Binding> {
    command
        .keys
        .iter()
        .filter_map(|spec| {
            let binding = Binding::parse(spec, os);
            if binding.is_none() {
                debug_assert!(
                    false,
                    "invalid key binding {spec:?} on command {}",
                    command.id
                );
                log::error!("invalid key binding {spec:?} on command {}", command.id);
            }
            binding
        })
        .collect()
}

/// The main palette's rows: titled commands live in the region the palette was opened from, in
/// registration order, with a later registration of the same id replacing an earlier one in
/// place (so a screen's command overrides a general one).
fn palette_candidates<A>(
    registered: &[Command<A>],
    primary: Region,
    multiple_selected: bool,
) -> Vec<&Command<A>> {
    let mut candidates: Vec<&Command<A>> = Vec::new();
    for command in registered {
        if command.title.is_none()
            || !command.scope.contains(primary)
            || (multiple_selected && !command.multiple)
        {
            continue;
        }
        match candidates
            .iter_mut()
            .find(|existing| existing.id == command.id)
        {
            Some(existing) => *existing = command,
            None => candidates.push(command),
        }
    }
    candidates
}
