//! The ⌘K palette: the main palette and every sub-palette (Label, Move, …), which are the same
//! widget with other rows.
//!
//! A 680px panel whose top sits 199px above the content area's middle, 16px top padding, a 36px
//! header with a hairline, a 32px input row with 24px margins and a 3px purple bar, then rows 60px
//! tall overlapping by 8px in a list at most 244px tall.

use std::{borrow::Cow, sync::Arc};

use egui::{
    Area, Color32, Context, CornerRadius, Event, Galley, Id, Key, Modifiers, Order, Painter, Pos2,
    Rect, Sense, Ui, Vec2,
    os::OperatingSystem,
    text::{CCursor, CCursorRange, LayoutJob, TextWrapping},
};
use exo_ui::{
    BoxShadow, Family, Font, KeycapColors, Keycaps, TextStyle, focus_field, paint_icon, paint_text,
    palette, text_field,
};

use crate::{
    Act, Hint,
    command::Command,
    keys::Chord,
    rank::{Ranked, rank_main, rank_rows},
};

const WIDTH: f32 = 680.0;
const PADDING_TOP: f32 = 16.0;
const GUTTER: f32 = 36.0;
const HEADER_HEIGHT: f32 = 36.0;
const INPUT_MARGIN: f32 = 24.0;
const INPUT_HEIGHT: f32 = 32.0;
const LIST_MAX_HEIGHT: f32 = 244.0;
const ROW_PADDING: f32 = 20.0;
/// A row's icon and the header's: a 20px box with the command's white 18px glyph centred in it.
const ICON: f32 = 20.0;
const GLYPH: f32 = 18.0;
/// An account row's avatar: 24px with `margin: 0 10px 0 -2px`, its 12px bold initial in `#27292d`
/// with `dy: .375em`.
const AVATAR: f32 = 24.0;
const AVATAR_INITIAL: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 16.0);
const AVATAR_INK: Color32 = Color32::from_rgb(0x27, 0x29, 0x2d);
/// Only this many rows render until the user moves through or over the list.
const FIRST_ROWS: usize = 10;

const HEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 14.0, 20.0);
/// `-0.0225em` at 21px.
const INPUT: TextStyle =
    TextStyle::new(Font::new(Family::Mono, 400), 21.0, 28.0).letter_spacing(-0.4725);
/// `-0.0225em` at 16px.
const TITLE_LETTER_SPACING: f32 = -0.36;
const TITLE: TextStyle =
    TextStyle::new(Font::new(Family::Mono, 400), 16.0, 20.0).letter_spacing(TITLE_LETTER_SPACING);
const SIDE: TextStyle =
    TextStyle::new(Font::new(Family::Mono, 400), 14.0, 20.0).letter_spacing(-0.315);

/// The palette's text colour, which the header text takes at 70%.
const HEADER_COLOR: Color32 = Color32::from_rgb(0xfa, 0xfa, 0xfa);
/// The bar left of the input: `inset 3px 0 0 #aeb1dd`.
const INPUT_BAR: Color32 = Color32::from_rgb(0xae, 0xb1, 0xdd);
/// The input's selection colour.
const SELECTION: Color32 = Color32::from_rgba_premultiplied(114, 192, 234, 254);

/// The id the main palette saves its query under.
const MAIN: &str = "COMMAND_PALETTE";

/// A sub-palette: the palette with its own type (`LABEL`, `MOVE`), prompt and rows. Rows are
/// commands, usually title and action only; the palette filters and ranks them with
/// `command-score`.
#[must_use]
pub struct SubPalette<A> {
    id: &'static str,
    prompt: Cow<'static, str>,
    icon: &'static str,
    placeholder: Cow<'static, str>,
    /// What the input starts with, the caret at its end.
    initial: Option<String>,
    hide_input: bool,
    rows: Rows<A>,
    footer: Option<Footer<A>>,
    /// Inert text under the divider when a typed query lists nothing.
    empty: Option<&'static str>,
    /// Inert text under the divider when there are no rows and nothing is typed.
    blank: Option<&'static str>,
    dropdown: Option<Dropdown>,
    /// A later stage of the sub-palette whose row opens it ([`SubPalette::stage`]).
    stage: bool,
}

type Footer<A> = Box<dyn Fn(&str) -> Option<Command<A>>>;
type RowsFor<A> = Box<dyn Fn(&str, usize) -> Vec<Command<A>>>;

/// A sub-palette's rows.
enum Rows<A> {
    /// Fixed rows the palette ranks for the query.
    Ranked(Vec<Command<A>>),
    /// Rows the sub-palette matches and orders itself, for the query as typed and the
    /// dropdown's choice.
    Ordered(RowsFor<A>),
}

/// A choice beside a sub-palette's input, such as Send later's `if no reply` / `regardless`: the
/// rows are made for what it says. A key cycles it (`cycle`, as Send later's ⌘⇧L does), and Tab, or
/// → at the end of the input, opens its menu.
#[derive(Clone, Debug)]
#[must_use]
pub struct Dropdown {
    options: Vec<&'static str>,
    selected: usize,
    cycle: &'static str,
    /// The hint an option picked from the open menu shows, and that option.
    hint: Option<(usize, Hint)>,
}

impl Dropdown {
    /// `options` in the menu's order, `selected` the one it starts on, cycled by `cycle` (a
    /// binding as [`Command::key`] takes it).
    pub fn new(options: Vec<&'static str>, selected: usize, cycle: &'static str) -> Self {
        Self {
            options,
            selected,
            cycle,
            hint: None,
        }
    }

    /// Shows `hint` when `option` is picked from the open menu, by a click or Enter, but not
    /// when the cycle key gets to it.
    pub fn hint_on(mut self, option: usize, hint: Hint) -> Self {
        self.hint = Some((option, hint));
        self
    }
}

impl<A: 'static> SubPalette<A> {
    /// A sub-palette of type `id` (`LABEL`; reopening the same type restores an unfinished query)
    /// with header `prompt` and `icon`.
    pub fn new(
        id: &'static str,
        prompt: impl Into<Cow<'static, str>>,
        icon: &'static str,
        rows: Vec<Command<A>>,
    ) -> Self {
        Self {
            id,
            prompt: prompt.into(),
            icon,
            placeholder: Cow::Borrowed(""),
            initial: None,
            hide_input: false,
            rows: Rows::Ranked(rows),
            footer: None,
            empty: None,
            blank: None,
            dropdown: None,
            stage: false,
        }
    }

    /// A sub-palette whose rows `rows` matches and orders for each query, shown as it returns
    /// them, for pickers with their own ranking rules (Label, Move).
    pub fn ordered(
        id: &'static str,
        prompt: impl Into<Cow<'static, str>>,
        icon: &'static str,
        rows: impl Fn(&str) -> Vec<Command<A>> + 'static,
    ) -> Self {
        Self {
            rows: Rows::Ordered(Box::new(move |query, _| rows(query))),
            ..Self::new(id, prompt, icon, Vec::new())
        }
    }

    /// [`ordered`](Self::ordered), with `dropdown` beside the input: `rows` takes its choice
    /// too, by index.
    pub fn ordered_with(
        id: &'static str,
        prompt: impl Into<Cow<'static, str>>,
        icon: &'static str,
        dropdown: Dropdown,
        rows: impl Fn(&str, usize) -> Vec<Command<A>> + 'static,
    ) -> Self {
        Self {
            rows: Rows::Ordered(Box::new(rows)),
            dropdown: Some(dropdown),
            ..Self::new(id, prompt, icon, Vec::new())
        }
    }

    /// Grey text in the empty input (`Try: 8 am, 3 days, aug 7`).
    pub fn placeholder(mut self, placeholder: impl Into<Cow<'static, str>>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Starts with `query` typed, the caret at its end.
    pub fn initial(mut self, query: impl Into<String>) -> Self {
        self.initial = Some(query.into());
        self
    }

    /// A later stage of the sub-palette whose row opens it, drawn as the same palette moving on
    /// (New Split Inbox's `Edit Subject`): Escape goes back to the main palette if that one's did.
    pub fn stage(mut self) -> Self {
        self.stage = true;
        self
    }

    /// No input: the rows follow the header (Move to Important / Other).
    pub fn hide_input(mut self) -> Self {
        self.hide_input = true;
        self
    }

    /// A row under a divider that depends on the query (`Create label “{query}”`).
    pub fn footer(mut self, footer: impl Fn(&str) -> Option<Command<A>> + 'static) -> Self {
        self.footer = Some(Box::new(footer));
        self
    }

    /// Inert text under the divider when a typed query lists nothing (`No matching date found`).
    pub fn empty_note(mut self, note: &'static str) -> Self {
        self.empty = Some(note);
        self
    }

    /// Inert text under the divider when there are no rows and nothing is typed
    /// (`No links or attachments`).
    pub fn blank_note(mut self, note: &'static str) -> Self {
        self.blank = Some(note);
        self
    }

    /// The titles of the rows listed for `query`, in order, then the footer's (for tests).
    pub fn titles(&self, query: &str) -> Vec<String> {
        let title = |row: &Command<A>| row.title.as_deref().unwrap_or_default().to_owned();
        let choice = self
            .dropdown
            .as_ref()
            .map_or(0, |dropdown| dropdown.selected);
        let mut titles: Vec<String> = match &self.rows {
            Rows::Ranked(rows) => rank_rows(rows, query)
                .into_iter()
                .map(|row| row.title.into_owned())
                .collect(),
            Rows::Ordered(rows) => rows(query, choice).iter().map(title).collect(),
        };
        titles.extend(
            self.footer
                .as_ref()
                .and_then(|footer| footer(query))
                .as_ref()
                .map(title),
        );
        if titles.is_empty() {
            titles.extend(self.note(query).map(str::to_owned));
        }
        titles
    }

    /// The rows listed for `query` with the dropdown on `choice`, as title, side title and
    /// action (for tests).
    pub fn rows(&self, query: &str, choice: usize) -> Vec<(String, Option<String>, A)>
    where
        A: Clone,
    {
        let row = |row: &Command<A>| {
            (
                row.title.as_deref().unwrap_or_default().to_owned(),
                row.side_title.as_deref().map(str::to_owned),
                row.action.clone(),
            )
        };
        match &self.rows {
            Rows::Ranked(rows) => rank_rows(rows, query)
                .iter()
                .map(|ranked| row(ranked.command))
                .collect(),
            Rows::Ordered(rows) => rows(query, choice).iter().map(row).collect(),
        }
    }

    pub(crate) fn into_act(self) -> SubPalette<Act<A>> {
        let footer = self.footer.map(|footer| -> Footer<Act<A>> {
            Box::new(move |query| footer(query).map(|row| row.map(Act::App)))
        });
        SubPalette {
            id: self.id,
            prompt: self.prompt,
            icon: self.icon,
            placeholder: self.placeholder,
            initial: self.initial,
            hide_input: self.hide_input,
            empty: self.empty,
            blank: self.blank,
            dropdown: self.dropdown,
            stage: self.stage,
            rows: match self.rows {
                Rows::Ranked(rows) => {
                    Rows::Ranked(rows.into_iter().map(|row| row.map(Act::App)).collect())
                }
                Rows::Ordered(rows) => Rows::Ordered(Box::new(move |query, choice| {
                    rows(query, choice)
                        .into_iter()
                        .map(|row| row.map(Act::App))
                        .collect()
                })),
            },
            footer,
        }
    }
}

impl<A> SubPalette<A> {
    /// The inert note for no rows listed for `query`.
    fn note(&self, query: &str) -> Option<&'static str> {
        if query.trim().is_empty() {
            self.blank
        } else {
            self.empty
        }
    }
}

/// The palette's state between frames.
pub(crate) struct Palette<A> {
    open: Option<Open<A>>,
    /// The query a palette type was dismissed with, restored (selected) when it reopens.
    saved: Option<(&'static str, String)>,
    /// A row ran this frame; a sub-palette opened in response goes back here on Escape.
    ran_main: bool,
    /// A row of a sub-palette that goes back to the main palette ran this frame.
    ran_from_main: bool,
    /// A hint the dropdown asked for this frame, for the app's hint tooltip.
    hint: Option<Hint>,
    /// The main palette's nudge, its placeholder while nothing is typed.
    nudge: Option<Cow<'static, str>>,
}

struct Open<A> {
    sub: Option<SubPalette<A>>,
    /// Escape from this sub-palette returns to the main palette.
    from_main: bool,
    state: State,
}

/// What the user has done to an open palette.
struct State {
    query: String,
    highlight: usize,
    show_all: bool,
    scroll: f32,
    /// The highlight moved by keyboard, so scroll it into view.
    reveal: bool,
    /// Focus the input (and select or place the caret) on the next frame.
    focus: Option<Selection>,
    /// The dropdown's choice, and its menu's highlight while it's open.
    choice: usize,
    menu: Option<usize>,
    /// When a key last cycled the dropdown, which flashes it for 50 ms.
    flashed: f64,
    /// The option the menu picked this frame.
    picked: Option<usize>,
    /// The main palette's nudge this frame ([`Palette::set_nudge`]).
    nudge: Option<Cow<'static, str>>,
}

#[derive(Clone, Copy)]
enum Selection {
    All,
    End,
}

/// A row that ran, and how.
pub(crate) struct Ran<A> {
    pub(crate) action: A,
    /// The modifiers held as it ran (Enter's, or the click's).
    pub(crate) modifiers: Modifiers,
    /// It ran with ⌘ held and asked to leave the palette open then.
    stays_open: bool,
}

impl<A: Clone> Ran<A> {
    fn new(command: &Command<A>, modifiers: Modifiers) -> Self {
        Self {
            action: command.action.clone(),
            modifiers,
            stays_open: command.stays_open_on_command && modifiers.command,
        }
    }
}

/// What ends a frame of an open palette.
enum Outcome<A> {
    /// A row ran.
    Run(Ran<A>),
    /// Dismissed.
    Close,
    /// Escape in a sub-palette opened from the main palette.
    BackToMain,
}

impl<A: Clone + 'static> Palette<A> {
    /// The open sub-palette's prompt and titles for the query typed.
    pub(crate) fn sub_rows(&self) -> Option<(String, Vec<String>)> {
        let open = self.open.as_ref()?;
        let sub = open.sub.as_ref()?;
        Some((sub.prompt.to_string(), sub.titles(&open.state.query)))
    }
}

impl<A: Clone> Palette<A> {
    pub(crate) const fn new() -> Self {
        Self {
            open: None,
            saved: None,
            ran_main: false,
            ran_from_main: false,
            hint: None,
            nudge: None,
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The hint the dropdown asked for, once.
    pub(crate) fn take_hint(&mut self) -> Option<Hint> {
        self.hint.take()
    }

    /// The open palette's type: `COMMAND_PALETTE` for the main one, else the sub-palette's id.
    pub(crate) fn open_id(&self) -> Option<&'static str> {
        self.open
            .as_ref()
            .map(|open| open.sub.as_ref().map_or(MAIN, |sub| sub.id))
    }

    /// Shows `nudge` in the main palette's empty input, in the nudge's gradient.
    pub(crate) fn set_nudge(&mut self, nudge: Option<Cow<'static, str>>) {
        self.nudge = nudge;
    }

    pub(crate) fn is_main_open(&self) -> bool {
        self.open.as_ref().is_some_and(|open| open.sub.is_none())
    }

    /// Whether the sub-palette of type `id` is open.
    pub(crate) fn is_sub_open(&self, id: &str) -> bool {
        self.open
            .as_ref()
            .and_then(|open| open.sub.as_ref())
            .is_some_and(|sub| sub.id == id)
    }

    /// Opens the main palette, with `initial` typed (caret at the end) or else the query it was
    /// last dismissed with (selected).
    pub(crate) fn open_main(&mut self, initial: Option<&str>) {
        self.open = Some(self.opened(MAIN, None, initial, false));
    }

    pub(crate) fn open_sub(&mut self, sub: SubPalette<A>) {
        let from_main = self.ran_main || self.is_main_open() || (sub.stage && self.ran_from_main);
        let choice = sub
            .dropdown
            .as_ref()
            .map_or(0, |dropdown| dropdown.selected);
        let initial = sub.initial.clone();
        let mut open = self.opened(sub.id, Some(sub), initial.as_deref(), from_main);
        open.state.choice = choice;
        self.open = Some(open);
    }

    fn opened(
        &self,
        id: &'static str,
        sub: Option<SubPalette<A>>,
        initial: Option<&str>,
        from_main: bool,
    ) -> Open<A> {
        let restored = self
            .saved
            .as_ref()
            .filter(|(saved, _)| *saved == id)
            .map(|(_, query)| query.clone());
        let (query, focus) = match (initial, restored) {
            (Some(initial), _) => (initial.to_owned(), Selection::End),
            (None, Some(restored)) => (restored, Selection::All),
            (None, None) => (String::new(), Selection::End),
        };
        Open {
            sub,
            from_main,
            state: State {
                query,
                highlight: 0,
                show_all: false,
                scroll: 0.0,
                reveal: false,
                focus: Some(focus),
                choice: 0,
                menu: None,
                flashed: f64::NEG_INFINITY,
                picked: None,
                nudge: None,
            },
        }
    }

    /// Closes the palette; `save` keeps a non-empty query for the next time this type opens
    /// (dismissing by ⌘K or a click outside does; running a row doesn't).
    pub(crate) fn close(&mut self, save: bool) {
        let Some(open) = self.open.take() else {
            return;
        };
        if save && !open.state.query.is_empty() {
            self.saved = Some((
                open.sub.as_ref().map_or(MAIN, |sub| sub.id),
                open.state.query,
            ));
        }
    }

    /// Handles the palette's keys and draws it over `area` (the window below the tab bar), and
    /// returns the action of the row that ran. `main` is the main palette's candidates, in
    /// registration order.
    pub(crate) fn show(
        &mut self,
        ctx: &Context,
        area: Rect,
        main: &[&Command<A>],
    ) -> Option<Ran<A>> {
        (self.ran_main, self.ran_from_main) = (false, false);
        let open = self.open.as_mut()?;
        let (sub, state) = (open.sub.as_ref(), &mut open.state);
        state.nudge = if sub.is_none() {
            self.nudge.clone()
        } else {
            None
        };
        // The dropdown's keys go first: they change the rows.
        let dropdown = sub.and_then(|sub| sub.dropdown.as_ref());
        if let Some(dropdown) = dropdown {
            dropdown_keys(ctx, state, dropdown);
        }
        let ordered;
        let rows: Vec<Ranked<'_, A>> = match sub.map(|sub| &sub.rows) {
            Some(Rows::Ranked(rows)) => rank_rows(rows, &state.query),
            Some(Rows::Ordered(rows)) => {
                ordered = rows(&state.query, state.choice);
                ordered.iter().map(Ranked::unranked).collect()
            }
            None => rank_main(main, &state.query),
        };
        let footer = match sub {
            Some(sub) => sub
                .footer
                .as_ref()
                .and_then(|footer| footer(&state.query))
                .map(|row| FooterRow::Command(Box::new(row)))
                .or_else(|| {
                    sub.note(&state.query)
                        .filter(|_| rows.is_empty())
                        .map(FooterRow::Text)
                }),
            None => main_footer(&state.query, rows.len()),
        };
        let count = rows.len() + usize::from(footer.as_ref().is_some_and(FooterRow::is_actionable));

        let opens_menu = dropdown.is_some() && !rows.is_empty();
        let outcome = match keyboard(ctx, state, count, opens_menu) {
            Some((Key::Escape, _)) if !state.query.is_empty() => {
                state.query.clear();
                state.highlight = 0;
                None
            }
            Some((Key::Escape, _)) if sub.is_some() && open.from_main => Some(Outcome::BackToMain),
            Some((Key::Escape, _)) => Some(Outcome::Close),
            Some((Key::Enter, modifiers)) => run(&rows, footer.as_ref(), state.highlight)
                .map(|command| Outcome::Run(Ran::new(command, modifiers))),
            _ => None,
        };
        let outcome = match outcome {
            Some(outcome) => outcome,
            None => {
                let outcome = draw(ctx, area, sub, state, &rows, footer.as_ref());
                if let (Some(picked), Some((option, hint))) = (
                    state.picked.take(),
                    dropdown.and_then(|dropdown| dropdown.hint.as_ref()),
                ) {
                    self.hint = (picked == *option).then(|| hint.clone());
                }
                outcome?
            }
        };
        match outcome {
            Outcome::Run(ran) => {
                self.ran_main = self.is_main_open();
                self.ran_from_main = self
                    .open
                    .as_ref()
                    .is_some_and(|open| open.sub.is_some() && open.from_main);
                if !ran.stays_open {
                    // Running anything forgets the saved query.
                    self.saved = None;
                    self.close(false);
                }
                Some(ran)
            }
            Outcome::Close => {
                self.close(true);
                None
            }
            Outcome::BackToMain => {
                self.open = Some(self.opened(MAIN, None, None, false));
                None
            }
        }
    }
}

/// A row under the list, after a 1px divider.
enum FooterRow<A> {
    /// A sub-palette's footer command.
    Command(Box<Command<A>>),
    /// Inert text.
    Text(&'static str),
    /// `Hint: hit` `control` `J` `to move down!`.
    Hint,
}

impl<A> FooterRow<A> {
    fn is_actionable(&self) -> bool {
        matches!(self, Self::Command(_))
    }
}

/// The main palette's footer: the `ctrl+j` hint when the query ends in `j` and there are rows to
/// move through, else a suggestion when nothing matched.
fn main_footer<A>(query: &str, rows: usize) -> Option<FooterRow<A>> {
    if query.to_lowercase().ends_with('j') && rows > 1 {
        Some(FooterRow::Hint)
    } else if rows == 0 && !query.is_empty() {
        Some(FooterRow::Text("Try: “search”, “remind me”, “sent”"))
    } else {
        None
    }
}

/// The highlighted row (the footer's, past the last row).
fn run<'a, A>(
    rows: &[Ranked<'a, A>],
    footer: Option<&'a FooterRow<A>>,
    highlight: usize,
) -> Option<&'a Command<A>> {
    match (rows.get(highlight), footer) {
        (Some(row), _) => Some(row.command),
        (None, Some(FooterRow::Command(command))) => Some(command),
        _ => None,
    }
}

/// The input's id.
const INPUT_ID: &str = "exo_commands::palette_input";

/// A dropdown's keys, before the palette's: its cycle key, and while its menu is open, the
/// menu's: ↑ ↓ K J move, Enter picks, Escape, Tab and ← close it, and nothing reaches the input.
fn dropdown_keys(ctx: &Context, state: &mut State, dropdown: &Dropdown) {
    let os = ctx.os();
    let cycle = crate::keys::Binding::parse(dropdown.cycle, os)
        .and_then(|binding| binding.chords.first().copied());
    let count = dropdown.options.len().max(1);
    let now = ctx.input(|input| input.time);
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            if state.menu.is_some() && matches!(event, Event::Text(_)) {
                return false;
            }
            let Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            else {
                return true;
            };
            let chord = Chord::from_event(*key, *modifiers);
            if Some(chord) == cycle {
                // `cycle`: the next option, the menu's highlight with it, and a flash.
                state.choice = (state.choice + 1) % count;
                state.menu = state.menu.map(|_| state.choice);
                state.flashed = now;
                return false;
            }
            let Some(highlight) = state.menu else {
                return true;
            };
            let plain = !chord.shift && !chord.ctrl && !chord.alt && !chord.meta;
            match chord.key {
                Key::ArrowDown | Key::ArrowUp | Key::J | Key::K if plain => {
                    state.menu = Some((highlight + 1) % count)
                }
                Key::Enter if plain => {
                    state.choice = highlight;
                    state.menu = None;
                    state.picked = Some(highlight);
                }
                Key::Tab | Key::ArrowLeft => state.menu = None,
                _ if chord.is_escape(os) => state.menu = None,
                _ => {}
            }
            false
        });
    });
}

/// Whether the input's caret is at the end of `query`, with nothing selected.
fn caret_at_end(ctx: &Context, query: &str) -> bool {
    let end = query.chars().count();
    egui::text_edit::TextEditState::load(ctx, exo_ui::id(ctx, INPUT_ID))
        .and_then(|state| state.cursor.char_range())
        .is_none_or(|range| range.primary.index.0 == end && range.secondary.index.0 == end)
}

/// Consumes the palette's keys before the input sees them, moving the highlight. Returns Escape or
/// Enter, with the modifiers held, for the caller to act on. With a dropdown that has rows to apply
/// to (`opens_menu`), Tab and → at the input's end open its menu instead.
fn keyboard(
    ctx: &Context,
    state: &mut State,
    count: usize,
    opens_menu: bool,
) -> Option<(Key, Modifiers)> {
    let at_end = opens_menu && caret_at_end(ctx, &state.query);
    let os = ctx.os();
    let mac = os == OperatingSystem::Mac;
    let mut result = None;
    ctx.input_mut(|input| {
        let mut swallow_backtick = false;
        input.events.retain(|event| {
            if swallow_backtick && matches!(event, Event::Text(text) if text == "`") {
                swallow_backtick = false;
                return false;
            }
            let Event::Key {
                key,
                pressed: true,
                repeat,
                modifiers,
                ..
            } = event
            else {
                return true;
            };
            let chord = Chord::from_event(*key, *modifiers);
            let ctralt = if mac {
                chord.ctrl && !chord.alt
            } else {
                chord.alt && !chord.ctrl
            };
            let plain = !chord.shift && !chord.ctrl && !chord.alt && !chord.meta;
            let command = if mac { chord.meta } else { chord.ctrl }
                && !chord.shift
                && !chord.alt
                && !(mac && chord.ctrl);
            let step = |state: &mut State, to: usize| {
                state.highlight = to;
                state.show_all = true;
                state.reveal = true;
            };
            match chord.key {
                Key::ArrowDown | Key::ArrowUp if command && count > 0 => {
                    step(
                        state,
                        if chord.key == Key::ArrowDown {
                            count - 1
                        } else {
                            0
                        },
                    );
                }
                Key::ArrowDown if plain && count > 0 => step(state, (state.highlight + 1) % count),
                Key::ArrowUp if plain && count > 0 => {
                    step(state, (state.highlight + count - 1) % count)
                }
                Key::J | Key::N if ctralt && !chord.shift && !chord.meta && count > 0 => {
                    step(state, (state.highlight + 1) % count)
                }
                Key::K | Key::P if ctralt && !chord.shift && !chord.meta && count > 0 => {
                    step(state, (state.highlight + count - 1) % count)
                }
                Key::Tab | Key::ArrowRight
                    if opens_menu && (chord.key == Key::Tab || (plain && at_end)) =>
                {
                    state.menu = Some(state.choice)
                }
                Key::Enter | Key::Tab if !*repeat => result = Some((Key::Enter, *modifiers)),
                Key::Enter | Key::Tab => {}
                _ if chord.is_escape(os) => {
                    swallow_backtick = chord.key == Key::Backtick;
                    result = Some((Key::Escape, *modifiers));
                }
                _ => return true,
            }
            false
        });
    });
    result
}

/// Draws the palette and handles the mouse.
fn draw<A: Clone>(
    ctx: &Context,
    area: Rect,
    sub: Option<&SubPalette<A>>,
    state: &mut State,
    rows: &[Ranked<'_, A>],
    footer: Option<&FooterRow<A>>,
) -> Option<Outcome<A>> {
    let id = exo_ui::id(ctx, "exo_commands::palette");
    Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(area.min)
        .show(ctx, |ui| {
            // The palette is a transparent layer over the whole content area: a click on it closes
            // the palette, and nothing underneath gets the click.
            let backdrop = ui.allocate_rect(area, Sense::click());
            let layout = Layout::new(
                ui,
                area,
                state,
                rows,
                footer,
                sub.is_some_and(|sub| sub.hide_input),
            );
            let panel = ui.allocate_rect(layout.panel, Sense::click());
            let ran = paint(ui, &layout, sub, state, rows, footer);
            if panel.clicked() || panel.drag_stopped() {
                state.focus.get_or_insert(Selection::End);
            }
            ran.map(Outcome::Run)
                .or_else(|| backdrop.clicked().then_some(Outcome::Close))
        })
        .inner
}

/// Where everything goes this frame.
struct Layout {
    panel: Rect,
    list: Rect,
    /// Each rendered row's top (relative to the list's content) and height.
    rows: Vec<(f32, f32)>,
    content_height: f32,
    footer: Option<Rect>,
    /// How far the panel moved to land on a whole pixel. Row icons (SVGs in the list's
    /// composited layer) stay where layout put them, so they don't move with it.
    snap: f32,
}

impl Layout {
    fn new<A>(
        ui: &Ui,
        area: Rect,
        state: &State,
        rows: &[Ranked<'_, A>],
        footer: Option<&FooterRow<A>>,
        hide_input: bool,
    ) -> Self {
        let left = area.center().x - WIDTH / 2.0;
        // `top: calc(50% - 199px)`. Chromium paints the panel on a whole CSS pixel (measured: 221.5
        // in an 800px window draws at 222), but not the row icons (see `snap`).
        let unsnapped = area.center().y - 199.0;
        let top = unsnapped.round();
        // Without its input the empty input row is 8px under the header (measured).
        let input = if hide_input {
            8.0
        } else {
            INPUT_MARGIN + INPUT_HEIGHT + INPUT_MARGIN
        };
        let list_top = top + PADDING_TOP + HEADER_HEIGHT + input;
        let shown = if state.show_all {
            rows.len()
        } else {
            rows.len().min(FIRST_ROWS)
        };
        let mut y = 0.0;
        let row_rects = rows[..shown]
            .iter()
            .map(|row| {
                // The avatar is taller than a title line; a photo's box pulls itself 2px up
                // (`margin-top: -2px`).
                let avatar = row.command.avatar.as_ref().map_or(0.0, |avatar| {
                    if avatar.photo.is_some() {
                        AVATAR - 2.0
                    } else {
                        AVATAR
                    }
                });
                let content = (title_lines(ui, row) * TITLE.line_height).max(avatar);
                let height = 2.0 * ROW_PADDING + content;
                let rect = (y, height);
                y += height - 8.0;
                rect
            })
            .collect::<Vec<_>>();
        // The last row has `margin-bottom: 8px` instead of −8px.
        let content_height = if shown == 0 { 0.0 } else { y + 16.0 };
        let list_height = content_height.min(LIST_MAX_HEIGHT);
        let list = Rect::from_min_size(Pos2::new(left, list_top), Vec2::new(WIDTH, list_height));
        // The footer: a 1px break, then a row padded `19px 36px 20px` with the last child's 8px
        // bottom margin.
        let footer = footer.map(|_| {
            Rect::from_min_size(Pos2::new(left, list.bottom()), Vec2::new(WIDTH, 1.0 + 59.0))
        });
        let bottom = footer.map_or(list.bottom(), |footer| footer.bottom() + 8.0);
        let panel = Rect::from_min_max(
            Pos2::new(left, top),
            Pos2::new(left + WIDTH, bottom.min(area.bottom())),
        );
        Self {
            panel,
            list,
            rows: row_rects,
            content_height,
            footer,
            snap: top - unsnapped,
        }
    }
}

/// How many lines a row's title takes (it wraps to two at most; one beside a description).
fn title_lines<A>(ui: &Ui, row: &Ranked<'_, A>) -> f32 {
    let galley = title_galley(ui, row, Color32::WHITE, title_width(ui, row));
    galley.rows.len().clamp(1, 2) as f32
}

/// The gap between a title and its description.
const DESCRIPTION_GAP: f32 = 28.0;

fn title_width<A>(ui: &Ui, row: &Ranked<'_, A>) -> f32 {
    let icon = if row.command.icon.is_some() {
        ICON + 13.0
    } else if row.command.avatar.is_some() {
        AVATAR - 2.0 + 10.0
    } else {
        0.0
    };
    // The right-hand side's 12px left margin, there even when it's empty.
    let meta = meta_width(ui, row.command) + 12.0;
    WIDTH - 2.0 * GUTTER - icon - meta
}

fn title_galley<A>(ui: &Ui, row: &Ranked<'_, A>, color: Color32, width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::single_section(row.title.clone().into_owned(), TITLE.format(color));
    // Beside a description the title keeps to one line (`white-space: nowrap`) and gives way
    // only once the description is gone (`flex: 0 1 auto` against its `1 99999999999 auto`).
    // Chromium wraps a title at its spaces only, so one without (a URL) is cut short on its
    // first line, where egui would break it after a `/`.
    let described = row.command.description.is_some();
    let one_line = described || !row.title.contains(char::is_whitespace);
    job.wrap = TextWrapping {
        max_width: if described {
            (width - DESCRIPTION_GAP).max(0.0)
        } else {
            width
        },
        max_rows: if one_line { 1 } else { 2 },
        break_anywhere: one_line,
        overflow_character: Some('…'),
    };
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

/// The right-hand side's width: a tick, then a side title or keycaps.
fn meta_width<A>(ui: &Ui, command: &Command<A>) -> f32 {
    let tick = if command.in_use { 14.0 } else { 0.0 };
    if let Some(side) = &command.side_title {
        return tick
            + 32.0
            + SIDE
                .single_line(ui.painter(), side, Color32::WHITE, None)
                .size()
                .x;
    }
    let keys = command.display_keys();
    if keys.is_empty() {
        return tick;
    }
    let keycaps = Keycaps::new(ui.ctx(), &keys, KeycapColors::base(palette(ui.ctx())));
    // After a tick the first cap's margin is 8px instead of 4px.
    tick + keycaps.width() + if command.in_use { 4.0 } else { 0.0 }
}

/// Paints the palette and handles the mouse over it; returns the clicked row.
fn paint<A: Clone>(
    ui: &mut Ui,
    layout: &Layout,
    sub: Option<&SubPalette<A>>,
    state: &mut State,
    rows: &[Ranked<'_, A>],
    footer: Option<&FooterRow<A>>,
) -> Option<Ran<A>> {
    let ctx = ui.ctx().clone();
    let tokens = palette(&ctx);
    let panel = layout.panel;
    // 4px corners; Carbon's shadows are deeper.
    let radius = CornerRadius::same(4);
    let painter = ui.painter().clone();
    let shadows = match tokens.theme {
        exo_ui::Theme::Snow => [
            BoxShadow::new([0.0, 15.0], 25.0, 0.0, Color32::from_black_alpha(26)),
            BoxShadow::new([0.0, 19.0], 45.0, 0.0, Color32::from_black_alpha(77)),
        ],
        exo_ui::Theme::Carbon => [
            BoxShadow::new([0.0, 15.0], 50.0, 0.0, Color32::from_black_alpha(179)),
            BoxShadow::new([0.0, 19.0], 90.0, 0.0, Color32::BLACK),
        ],
    };
    for shadow in shadows {
        shadow.paint(&painter, panel, radius);
    }
    painter.rect_filled(panel, radius, tokens.background_dark_default);
    let painter = painter.with_clip_rect(panel);

    // Header: icon, prompt, hairline.
    let left = panel.left() + GUTTER;
    let header_top = panel.top() + PADDING_TOP;
    let (icon, prompt) = sub.map_or(("data/icon", "Exo Command"), |sub| {
        (sub.icon, sub.prompt.as_ref())
    });
    // The header's glyph: centred in its box, 1px down, at 70%.
    let glyph = Rect::from_center_size(
        Pos2::new(left + ICON / 2.0, header_top + 1.0 + ICON / 2.0),
        Vec2::splat(GLYPH),
    );
    paint_icon(ui, icon, glyph, Color32::WHITE.gamma_multiply(0.7));
    let prompt = HEADER.single_line(&painter, prompt, HEADER_COLOR.gamma_multiply(0.7), None);
    paint_text(
        &painter,
        Pos2::new(left + 33.0, header_top + 3.0),
        prompt,
        &HEADER,
    );
    let hairline = Rect::from_min_size(
        Pos2::new(left, header_top + HEADER_HEIGHT - 1.0),
        Vec2::new(WIDTH - 2.0 * GUTTER, 1.0),
    );
    painter.rect_filled(hairline, 0.0, Color32::WHITE.gamma_multiply(0.1));

    let mut menu = None;
    if !sub.is_some_and(|sub| sub.hide_input) {
        let input_top = header_top + HEADER_HEIGHT + INPUT_MARGIN;
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(panel.left(), input_top),
                Vec2::new(3.0, INPUT_HEIGHT),
            ),
            0.0,
            INPUT_BAR,
        );
        let nudge = state.nudge.clone();
        let placeholder = nudge.as_deref().map_or_else(
            || sub.map_or("", |sub| sub.placeholder.as_ref()),
            |nudge| nudge,
        );
        // The dropdown takes its width from the right, `margin-right: 36px`, and the input the
        // rest.
        let dropdown = sub.and_then(|sub| sub.dropdown.as_ref()).map(|dropdown| {
            let right = panel.right() - GUTTER;
            let width = dropdown_width(ui, dropdown, state.choice, state.menu.is_some());
            menu = Some((
                dropdown,
                Rect::from_min_size(
                    Pos2::new(right - width, input_top),
                    Vec2::new(width, INPUT_HEIGHT),
                ),
            ));
            width
        });
        let width = WIDTH - 2.0 * GUTTER - dropdown.map_or(0.0, |width| width + GUTTER);
        input(
            ui,
            &painter,
            state,
            Pos2::new(left, input_top + 4.0),
            placeholder,
            nudge.is_some(),
            width,
        );
        if let Some((dropdown, rect)) = menu {
            paint_dropdown_button(ui, &painter, dropdown, state, rect, !rows.is_empty());
        }
    }

    // Rows.
    let pointer_moved = ui.input(|input| input.pointer.delta() != Vec2::ZERO);
    let over_list = ui
        .input(|input| input.pointer.hover_pos())
        .is_some_and(|pos| layout.list.contains(pos));
    let wheel = if over_list {
        ui.input(|input| input.smooth_scroll_delta.y)
    } else {
        0.0
    };
    if over_list && (pointer_moved || wheel != 0.0) {
        state.show_all = true;
    }
    state.scroll =
        (state.scroll - wheel).clamp(0.0, (layout.content_height - layout.list.height()).max(0.0));
    if state.reveal {
        state.reveal = false;
        if let Some(&(top, height)) = layout.rows.get(state.highlight) {
            // Scrolled in at the nearest edge.
            if top < state.scroll {
                state.scroll = top;
            } else if top + height > state.scroll + layout.list.height() {
                state.scroll = top + height - layout.list.height();
            }
        }
    }
    let mut list_ui = ui.new_child(egui::UiBuilder::new());
    list_ui.set_clip_rect(layout.list);
    let mut ran = None;
    for (index, (&(top, height), row)) in layout.rows.iter().zip(rows).enumerate() {
        let rect = Rect::from_min_size(
            Pos2::new(panel.left(), layout.list.top() + top - state.scroll),
            Vec2::new(WIDTH, height),
        );
        let response = ui.interact(
            rect.intersect(layout.list),
            exo_ui::id(ui.ctx(), ("exo_commands::palette_row", index)),
            Sense::click(),
        );
        if (response.hovered() && pointer_moved) || response.is_pointer_button_down_on() {
            state.highlight = index;
        }
        if response.clicked() {
            ran = Some(Ran::new(row.command, ui.input(|input| input.modifiers)));
        }
        let active = index == state.highlight;
        if active {
            list_ui
                .painter()
                .rect_filled(rect, 0.0, Color32::WHITE.gamma_multiply(0.1));
        }
        paint_row(&list_ui, rect, row, active, layout.snap);
    }
    if let (Some(rect), Some(footer)) = (layout.footer, footer) {
        let divider = Rect::from_min_size(
            Pos2::new(rect.left() + GUTTER, rect.top()),
            Vec2::new(WIDTH - 2.0 * GUTTER, 1.0),
        );
        painter.rect_filled(divider, 0.0, Color32::WHITE.gamma_multiply(0.1));
        let active = state.highlight == rows.len() && footer.is_actionable();
        let row_rect = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + 1.0), rect.max);
        if active {
            painter.rect_filled(row_rect, 0.0, Color32::WHITE.gamma_multiply(0.1));
        }
        let color = Color32::WHITE.gamma_multiply(if active { 0.9 } else { 0.5 });
        // Footer rows are padded `19px 36px 20px`.
        let line_top = Pos2::new(rect.left() + GUTTER, row_rect.top() + 19.0 + 1.0);
        match footer {
            FooterRow::Command(command) => {
                let response = ui.interact(
                    row_rect,
                    exo_ui::id(ui.ctx(), "exo_commands::palette_footer"),
                    Sense::click(),
                );
                if response.clicked() {
                    ran = Some(Ran::new(command, ui.input(|input| input.modifiers)));
                }
                // Footer rows are padded 19px on top, rows 20px.
                let shifted = row_rect.translate(Vec2::Y * -1.0);
                paint_row(ui, shifted, &Ranked::unranked(command), active, layout.snap);
            }
            FooterRow::Text(text) => paint_text(
                &painter,
                line_top,
                TITLE.single_line(&painter, text, color, None),
                &TITLE,
            ),
            FooterRow::Hint => paint_hint(ui, &painter, line_top, color),
        }
    }
    // The open menu is over the rows.
    if let (Some((dropdown, rect)), Some(highlight)) = (menu, state.menu) {
        paint_dropdown_menu(ui, dropdown, state, highlight, rect);
    }
    ran
}

/// The dropdown's button and menu items: Exo Mono 14px, `-0.0225em`; the button in a 20px
/// line, the menu's items in 28px ones.
const DROPDOWN: TextStyle =
    TextStyle::new(Font::new(Family::Mono, 400), 14.0, 20.0).letter_spacing(-0.315);
const DROPDOWN_ITEM: TextStyle =
    TextStyle::new(Font::new(Family::Mono, 400), 14.0, 28.0).letter_spacing(-0.315);
/// The dropdown's arrow: `margin-left: 6px` before the 10×6 `DEPRECATED_dropdown`.
const ARROW_GAP: f32 = 6.0;
const ARROW: Vec2 = Vec2::new(10.0, 6.0);
/// A menu item: 32px tall, `padding: 0 16px`.
const MENU_ITEM: f32 = 32.0;
const MENU_PADDING: f32 = 16.0;

/// The dropdown's width: its button's (the option chosen, then the arrow), or with its menu
/// open, the widest item's if wider.
fn dropdown_width(ui: &Ui, dropdown: &Dropdown, choice: usize, open: bool) -> f32 {
    let painter = ui.painter();
    // Chromium's text box takes the last letter's (negative) spacing too.
    let text = |text: &str, style: &TextStyle| {
        style
            .single_line(painter, text, Color32::WHITE, None)
            .size()
            .x
            + style.letter_spacing
    };
    let button = text(
        dropdown.options.get(choice).copied().unwrap_or_default(),
        &DROPDOWN,
    ) + ARROW_GAP
        + ARROW.x;
    let menu = if open {
        dropdown
            .options
            .iter()
            .map(|option| text(option, &DROPDOWN_ITEM) + 2.0 * MENU_PADDING)
            .fold(0.0_f32, f32::max)
    } else {
        0.0
    };
    button.max(menu)
}

/// The dropdown's button in the input row `rect` (its box; the dropdown 10px down, the button 2px
/// more, right-aligned): the choice and the arrow, at half opacity until hovered, open or flashed
/// by its key, underlined while open. A click opens or closes the menu; with no rows it does
/// nothing.
fn paint_dropdown_button(
    ui: &Ui,
    painter: &Painter,
    dropdown: &Dropdown,
    state: &mut State,
    rect: Rect,
    enabled: bool,
) {
    let ctx = ui.ctx();
    let id = exo_ui::id(ctx, "exo_commands::palette_dropdown");
    let label = dropdown
        .options
        .get(state.choice)
        .copied()
        .unwrap_or_default();
    let text_box = DROPDOWN
        .single_line(painter, label, Color32::WHITE, None)
        .size()
        .x
        + DROPDOWN.letter_spacing;
    let width = text_box + ARROW_GAP + ARROW.x;
    let line = Rect::from_min_size(
        Pos2::new(rect.right() - width, rect.top() + 12.0),
        Vec2::new(width, DROPDOWN.line_height),
    );
    let response = ui.interact(
        line,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if enabled && response.clicked() {
        state.menu = match state.menu {
            Some(_) => None,
            None => Some(state.choice),
        };
    }
    let open = state.menu.is_some();
    let flashing = ctx.input(|input| input.time) - state.flashed < 0.05;
    // `opacity: .5`, 1 on hover, open or flashed; `transition: opacity 150ms ease`, none while
    // flashing.
    let lit = enabled && (response.hovered() || open);
    let eased = exo_ui::motion::transition(
        ctx,
        id.with("opacity"),
        lit || flashing,
        0.15,
        exo_ui::motion::CubicBezier::EASE,
    );
    let opacity = if flashing { 1.0 } else { 0.5 + 0.5 * eased };
    if flashing {
        ctx.request_repaint();
    }
    let color = HEADER_COLOR.gamma_multiply(opacity);
    DROPDOWN.paint_line(painter, line.min, label, color, None);
    // The arrow's 7px inline block sits on the baseline, the 10×6 glyph at its top.
    let baseline = line.top() + DROPDOWN.baseline(ctx);
    let arrow = Rect::from_min_size(
        Pos2::new(line.left() + text_box + ARROW_GAP, baseline - 7.0),
        ARROW,
    );
    paint_icon(
        ui,
        "legacy/DEPRECATED_dropdown",
        arrow,
        Color32::WHITE.gamma_multiply(opacity),
    );
    if open {
        // `box-shadow: 0 2px #aeb1dd` under the inline button's box (its glyphs' 18px and the
        // 2px bottom padding, measured: the line's bottom).
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(line.left(), line.bottom()), Vec2::new(width, 2.0)),
            0.0,
            INPUT_BAR,
        );
    }
}

/// The dropdown's open menu under its button (white, `padding: 8px 0`, `margin-top: 8px`), its
/// items right-aligned in the dropdown's box `rect`, the highlighted one on `#e4e4e4`. A click
/// picks one; the pointer moves the highlight.
fn paint_dropdown_menu(
    ui: &Ui,
    dropdown: &Dropdown,
    state: &mut State,
    highlight: usize,
    rect: Rect,
) {
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        Order::Tooltip,
        Id::new("exo_commands::palette_menu"),
    ));
    let top = rect.top() + 10.0 + DROPDOWN.line_height + 8.0;
    let menu = Rect::from_min_size(
        Pos2::new(rect.left(), top),
        Vec2::new(
            rect.width(),
            16.0 + MENU_ITEM * dropdown.options.len() as f32,
        ),
    );
    painter.rect_filled(menu, 0.0, Color32::WHITE);
    let pointer_moved = ui.input(|input| input.pointer.delta() != Vec2::ZERO);
    for (index, option) in dropdown.options.iter().enumerate() {
        let item = Rect::from_min_size(
            Pos2::new(menu.left(), menu.top() + 8.0 + MENU_ITEM * index as f32),
            Vec2::new(menu.width(), MENU_ITEM),
        );
        let response = ui.interact(
            item,
            exo_ui::id(ui.ctx(), ("exo_commands::palette_menu_item", index)),
            Sense::click(),
        );
        if response.hovered() && pointer_moved {
            state.menu = Some(index);
        }
        if response.clicked() {
            state.choice = index;
            state.menu = None;
            state.picked = Some(index);
        }
        let highlighted = index == highlight;
        if highlighted {
            painter.rect_filled(item, 0.0, Color32::from_rgb(0xe4, 0xe4, 0xe4));
        }
        let color = Color32::from_black_alpha(if highlighted { 153 } else { 102 });
        let width = DROPDOWN_ITEM
            .single_line(&painter, option, color, None)
            .size()
            .x;
        // The text 2px down, its 28px line at the item's top.
        DROPDOWN_ITEM.paint_line(
            &painter,
            Pos2::new(item.right() - MENU_PADDING - width, item.top() + 2.0),
            option,
            color,
            None,
        );
    }
}

/// The nudge's gradient: the input's background, `linear-gradient(90deg, #8ea6ff 0%, #ffbcd5 25%,
/// #8ea6ff 75%, #ffbcd5 100%)` four times its `width` wide and sliding from its end to its start
/// every 5s, shows through `galley`'s glyphs (`background-clip: text`), under the placeholder's 30%
/// white.
fn gradient_text(galley: &Arc<Galley>, width: f32, time: f64) -> Arc<Galley> {
    const STOPS: [(f32, [f32; 3]); 4] = [(0.0, BLUE), (0.25, PINK), (0.75, BLUE), (1.0, PINK)];
    const BLUE: [f32; 3] = [142.0, 165.903_85, 255.0];
    const PINK: [f32; 3] = [255.0, 188.0, 212.512_2];
    // `background-position` from 100% to 0%: the image `3 × width` left of the input at first.
    let position = 1.0 - (time % 5.0 / 5.0) as f32;
    let color = |x: f32| {
        let at = ((x + 3.0 * width * position) / (4.0 * width)).clamp(0.0, 1.0);
        let (from, to) = STOPS
            .windows(2)
            .map(|pair| (pair[0], pair[1]))
            .find(|(_, to)| at <= to.0)
            .unwrap_or((STOPS[2], STOPS[3]));
        let t = (at - from.0) / (to.0 - from.0);
        let channel = |i: usize| (from.1[i] + (to.1[i] - from.1[i]) * t) * 0.7 + 255.0 * 0.3;
        Color32::from_rgb(
            channel(0).round() as u8,
            channel(1).round() as u8,
            channel(2).round() as u8,
        )
    };
    let mut tinted = (**galley).clone();
    for placed in &mut tinted.rows {
        let left = placed.pos.x;
        for vertex in &mut Arc::make_mut(&mut placed.row).visuals.mesh.vertices {
            vertex.color = color(left + vertex.pos.x);
        }
    }
    Arc::new(tinted)
}

/// The input: Exo Mono 21px/28px, its line box 4px below the input row's top.
fn input(
    ui: &mut Ui,
    painter: &Painter,
    state: &mut State,
    line_top_left: Pos2,
    placeholder: &str,
    gradient: bool,
    width: f32,
) {
    let ctx = ui.ctx().clone();
    let id = exo_ui::id(&ctx, INPUT_ID);
    if state.query.is_empty() && !placeholder.is_empty() {
        let galley = INPUT.single_line(
            painter,
            placeholder,
            Color32::WHITE.gamma_multiply(0.3),
            None,
        );
        let galley = if gradient {
            ctx.request_repaint();
            gradient_text(&galley, width, ctx.input(|input| input.time))
        } else {
            galley
        };
        paint_text(painter, line_top_left, galley, &INPUT);
    }
    let line = Rect::from_min_size(line_top_left, Vec2::new(width, INPUT.line_height));
    let before = state.query.clone();
    text_field(
        ui,
        id,
        &mut state.query,
        line,
        &INPUT,
        palette(&ctx).text_light_default,
        SELECTION,
    );
    if state.query != before {
        // Every query change puts the highlight back on the first row.
        state.highlight = 0;
        state.scroll = 0.0;
    }
    if let Some(selection) = state.focus.take() {
        let end = CCursor::new(state.query.chars().count());
        let range = match selection {
            Selection::All => CCursorRange::two(CCursor::new(0), end),
            Selection::End => CCursorRange::one(end),
        };
        focus_field(&ctx, id, range);
    }
}

/// Paints one row with `ui`'s painter (clipped to the list).
fn paint_row<A>(ui: &Ui, rect: Rect, row: &Ranked<'_, A>, active: bool, snap: f32) {
    let painter = ui.painter();
    let tokens = palette(ui.ctx());
    let command = row.command;
    let color = Color32::WHITE.gamma_multiply(if active { 0.9 } else { 0.5 });
    let mut x = rect.left() + GUTTER;
    let content_top = rect.top() + ROW_PADDING;
    if let Some(icon) = command.icon {
        // A white glyph at 70%, whole on the active row.
        let icon_color = Color32::WHITE.gamma_multiply(if active { 1.0 } else { 0.7 });
        // `top: 2px`, from where layout put the row (see `Layout::snap`); the glyph at the box's
        // top left (`background-repeat: no-repeat`).
        let icon_top = content_top + 2.0 - snap;
        paint_icon(
            ui,
            icon,
            Rect::from_min_size(Pos2::new(x, icon_top), Vec2::splat(GLYPH)),
            icon_color,
        );
        x += ICON + 13.0;
    } else if let Some(avatar) = &command.avatar {
        let rect = Rect::from_min_size(Pos2::new(x - 2.0, content_top), Vec2::splat(AVATAR));
        match &avatar.photo {
            // The photo's ring: `border-width: 2.5px`, which Chromium floors to 2px, and its
            // `margin-top: -2px` undone by `top: 2px`; it moves with the panel's snap (measured),
            // and like the icons it's 70% opaque off the active row.
            Some(photo) => exo_ui::paint_photo_avatar(
                painter,
                rect,
                photo,
                2.0,
                avatar.fill,
                if active { 1.0 } else { 0.7 },
            ),
            // An inline SVG: it moves with the panel's snap, as the tick does.
            None => exo_ui::paint_initial_avatar(
                painter,
                rect,
                avatar.fill,
                &avatar.initial,
                &AVATAR_INITIAL,
                AVATAR_INK,
                0.375 * 12.0,
            ),
        }
        x += AVATAR - 2.0 + 10.0;
    }
    let title = title_galley(ui, row, color, title_width(ui, row));
    if let Some(description) = &command.description {
        // The description: the rest of the line, ellipsized, 35% white (70% on the active row),
        // `top: 3px` to the title's 2px, which lands its line where the title's is (measured).
        // Chromium counts the title's letter spacing after its last letter too.
        let left = x + title.size().x + TITLE_LETTER_SPACING + DESCRIPTION_GAP;
        let width = x + title_width(ui, row) - left;
        if width > 0.0 {
            let ink = Color32::WHITE.gamma_multiply(if active { 0.7 } else { 0.35 });
            SIDE.paint_line(
                painter,
                Pos2::new(left, content_top + 1.0),
                description,
                ink,
                Some(width),
            );
        }
    }
    paint_text(painter, Pos2::new(x, content_top + 1.0), title, &TITLE);

    // Right side, right-aligned at the gutter.
    let right = rect.right() - GUTTER;
    let mut meta_left = right - meta_width(ui, command);
    if command.in_use {
        // The tick: a 14×11 background SVG, `M1 5.142L4.858 9l7.587-7.587` stroked 2px white,
        // `top: 6px`. Unlike the icons it moves with the panel's snap (measured). Its background
        // lands on the whole px left of a fractional box (measured with keys after it, which make
        // the box start off the pixel).
        let tick = Pos2::new(meta_left.floor(), content_top + 6.0);
        let points =
            [[1.0, 5.142], [4.858, 9.0], [12.445, 1.413]].map(|[x, y]| tick + Vec2::new(x, y));
        painter.add(egui::Shape::line(
            points.to_vec(),
            egui::Stroke::new(2.0, Color32::WHITE),
        ));
        meta_left += 14.0;
    }
    if let Some(side) = &command.side_title {
        SIDE.paint_line(
            painter,
            Pos2::new(meta_left + 32.0, content_top + 1.0),
            side,
            color,
            None,
        );
        return;
    }
    let keys = command.display_keys();
    if !keys.is_empty() {
        // The caps' labels take the row's color.
        let keycaps = Keycaps::new(
            ui.ctx(),
            &keys,
            KeycapColors {
                text: color,
                divider: color,
                ..KeycapColors::base(tokens)
            },
        );
        // After a tick the caps are 4px further right and, the tick's box lowering the line,
        // 1px lower (measured in the Accounts palette).
        let (gap, drop) = if command.in_use {
            (4.0, 1.0)
        } else {
            (0.0, 0.0)
        };
        keycaps.paint(painter, meta_left + gap, content_top + 2.0 + drop);
    }
}

/// `Hint: hit` `control` `J` `to move down!`, with inline keycaps.
fn paint_hint(ui: &Ui, painter: &Painter, line_top_left: Pos2, color: Color32) {
    let ctralt = if ui.ctx().os() == OperatingSystem::Mac {
        "control"
    } else {
        "alt"
    };
    let mut x = line_top_left.x;
    let space = TITLE.single_line(painter, " ", color, None).size().x;
    for (i, piece) in ["Hint: hit", ctralt, "J", "to move down!"]
        .into_iter()
        .enumerate()
    {
        if i > 0 {
            x += space;
        }
        let at = Pos2::new(x, line_top_left.y);
        x += if i == 1 || i == 2 {
            paint_key_hint(painter, at, piece)
        } else {
            TITLE.paint_line(painter, at, piece, color, None)
        };
    }
}

/// An inline keycap in the hint, `hsla(0,0%,100%,.15)`, Exo Sans 14px/24px bold.
fn paint_key_hint(painter: &Painter, line_top_left: Pos2, label: &str) -> f32 {
    const LABEL: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 14.0, 24.0);
    let color = Color32::WHITE.gamma_multiply(0.9);
    let label_width = LABEL.single_line(painter, label, color, None).size().x;
    let width = (label_width + 10.0).max(19.0);
    // `margin-left: -2px; top: 3px`, 18px tall.
    let rect = Rect::from_min_size(line_top_left + Vec2::new(-2.0, 3.0), Vec2::new(width, 18.0));
    // `#555b64` in Carbon.
    painter.rect_filled(
        rect,
        CornerRadius::same(3),
        palette(painter.ctx()).background_key_default,
    );
    // The label `top: -3px` in a 24px line box under the cap's 1px padding.
    LABEL.paint_line(
        painter,
        Pos2::new(rect.center().x - label_width / 2.0, rect.top() + 1.0 - 3.0),
        label,
        color,
        None,
    );
    width - 2.0
}
