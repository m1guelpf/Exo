//! The thread list view: the header with the view's tab, and the list of conversations with its
//! sliding cursor, date separators, hover actions, selection, the archive animation and the empty
//! states.
//!
//! Keyboard shortcuts aren't bound here; the operations they trigger are methods
//! ([`Inbox::move_cursor`], [`Inbox::dismiss`], …) for the app's commands to call.

mod header;
mod row;
pub(crate) use row::{REMIND_ICON, REMIND_SET_ICON};
mod nudge;
mod selection;
mod zero;

use std::collections::HashMap;

use egui::{Pos2, Rect, Sense, Ui, Vec2, pos2, vec2};
use exo_ui::{
    Family, Font, HighlightState, HighlightTarget, ListHighlight, TextStyle,
    motion::{self, CubicBezier},
};
use jiff::Zoned;

use crate::{
    format::dates::{self, Segment},
    mailbox::ThreadSummary,
};

pub use header::{Focus as HeaderFocus, HeaderAction};
pub use nudge::{HIDE_EMPTY_SPLIT, NudgeAction};

/// A row's height. A search result with files is taller, to show them under its row.
const ROW_HEIGHT: f32 = 36.0;
const ROW_WITH_FILES: f32 = 68.0;
/// A date separator's height.
const SEPARATOR_HEIGHT: f32 = 48.0;
/// The list's top padding.
const LIST_PADDING_TOP: f32 = 2.0;
/// The list's side padding.
const VIEW_PADDING_X: f32 = 6.0;
/// A dismissed row slides out over 200ms and is gone at the removal timer's first tick, 550ms
/// after it started.
const LEAVE_SLIDE: f32 = 0.2;
const LEAVE_CURVE: CubicBezier = CubicBezier::new(0.525, 0.0825, 0.215, 1.18);
const LEAVE_HIDDEN: f64 = 0.4;
const LEAVE_REMOVED: f64 = 0.55;
/// A row brought back mid-slide (undo) slides back in over 400ms on the same curve.
const RETURN_SLIDE: f32 = 0.4;
/// The list asks for more rows when its end is this close below the visible area (react-list's
/// `threshold`).
const LOAD_AHEAD: f32 = 2000.0;
/// The status line: 12px/16px, 10px above and below, nudged down 1px.
const STATUS: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
const STATUS_HEIGHT: f32 = 10.0 + 16.0 + 10.0;
/// The status lines under the rows.
const EMPTY: &str = "No conversations found here.";
const EMPTY_OFFLINE: &str = "This folder is not available offline.";
const EARLIER_OFFLINE: &str = "Earlier conversations are not available offline.";
/// The status line over the rows.
pub(crate) const NOTHING_FOUND: &str = "No conversations found.";
pub(crate) const PARTIAL_OFFLINE: &str = "Search results may be partial when offline.";
/// The loading row's padding: 8px above Classic's 15px spinner and 12px below it.
const SPINNER_TOP: f32 = 8.0;
const SPINNER: f32 = 15.0;
const SPINNER_HEIGHT: f32 = SPINNER_TOP + SPINNER + 12.0;
/// The select-all hint's height. While it's open the list leaves 60px under its rows for it.
const HINT_HEIGHT: f32 = 56.0;
const HINT_ROOM: f32 = 60.0;
/// The select-all hint's text: 12px/15px.
const HINT_TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 15.0);

/// The middle column of a list `width` px wide, which other lists' rows (the Snippets folder's) cut
/// their text to as the thread list's do.
pub(crate) fn center_width(width: f32, rem: f32, compact: bool) -> f64 {
    row::Geometry::new(width, rem, compact).center_width()
}

/// What the app should do in response to the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open this thread.
    Open(String),
    /// A Done button: mark the actionable threads done, or not done when they all are.
    ToggleDone,
    /// A clock button: Remind me for the actionable threads.
    Remind,
    /// Open the command palette.
    CommandPalette,
    /// Show the list for the label with this name (a chip was clicked).
    ShowLabel(String),
    /// Something in the header.
    Header(HeaderAction),
    /// The list scrolled close to the end of its loaded rows.
    More,
    /// A row was right-clicked at this point: the menu for the actionable conversations, which
    /// the right-click made the clicked row, or the selection it's in.
    RowMenu(Pos2),
    /// The hide-empty nudge's close button or `Try it out`.
    Nudge(NudgeAction),
    /// A search result's file card: preview the file, or save it when `download`.
    File {
        file: exo_store::Attachment,
        download: bool,
    },
    /// A row's checkmark was clicked: suggest X (`SELECT`, or `DESELECT` for a selected row).
    SelectHint { deselect: bool },
    /// Inbox Zero's button: the Inbox Zero achievements.
    Achievements,
}

/// The header's height ([`header::HEIGHT`]), for lists drawn under it elsewhere.
pub(crate) const HEADER_HEIGHT: f32 = header::HEIGHT;

/// The thread list's header titled `title` with the folders button, compose and search, over
/// a list that isn't the thread list (the Snippets folder's).
pub(crate) fn show_header(
    ui: &Ui,
    rect: Rect,
    rem: f32,
    compact: bool,
    title: &str,
) -> Option<HeaderAction> {
    let view = View {
        title: title.to_owned(),
        ..View::default()
    };
    header::show(
        ui,
        rect,
        rem,
        compact,
        &view,
        &header::Mode::Normal,
        false,
        false,
        None,
    )
}

/// What the list shows besides its rows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    /// The header's title: `Inbox`, `Done`, a label's name.
    pub title: String,
    /// An inbox view: split tabs, Add Split and the split gear in the header, Inbox Zero when
    /// empty.
    pub inbox: bool,
    /// An inbox view's tabs.
    pub tabs: Vec<crate::splits::Tab>,
    /// The tab on screen, an index into [`View::tabs`].
    pub selected: usize,
    /// The header offers Add Split.
    pub add_split: bool,
    /// More rows exist past the loaded ones.
    pub more: bool,
    /// The list is still being fetched, so empty doesn't mean empty yet.
    pub loading: bool,
    /// Every thread of the list is stored: offline, what isn't can't load.
    pub complete: bool,
    /// Its threads can be marked done (not in Spam).
    pub archivable: bool,
    /// The filter narrowing it, by the name its tab shows (`Unread`): never Inbox Zero.
    pub filter: Option<&'static str>,
    /// How many threads it has locally, all of them when exact (a select-all's total).
    pub count: Option<exo_store::ListCount>,
    /// Search results: the search header (the app's) over the top, and no date separators.
    pub search: Option<SearchView>,
    /// No date separators either: Scheduled.
    pub unsegmented: bool,
    /// At Inbox Zero, the card offering to hide empty splits shows.
    pub nudge: bool,
    /// Weeks in a row the inbox has been emptied (`inboxZeroStreakWeekCurrent`), which Inbox
    /// Zero's button counts.
    pub streak: u32,
    /// The streak animation has the window: the header and Inbox Zero's button are hidden.
    pub hide_chrome: bool,
    /// The settings menu is open: at Inbox Zero, the header's search and compose are hidden to give
    /// it the room.
    pub settings_open: bool,
}

/// How the list looks as search results.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchView {
    /// The search header's height, which takes the normal header's place.
    pub header: f32,
    /// The status line at the top of the results.
    pub status: Option<&'static str>,
}

/// What the list shows under its rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bottom {
    /// More may load: the spinner.
    Spinner,
    /// It's all loaded that can be, and this says why the list ends where it does.
    Status(&'static str),
    /// Nothing.
    None,
}

impl Bottom {
    fn height(self) -> f32 {
        match self {
            Self::Spinner => SPINNER_HEIGHT,
            Self::Status(_) => STATUS_HEIGHT,
            Self::None => 0.0,
        }
    }
}

/// A line of the list: a date separator, or the thread at this index of [`Inbox::threads`].
enum Item {
    Separator(String),
    Thread(usize),
}

/// A hint tooltip the selection asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionHint {
    /// Four or more rows picked one by one: ⌘A selects everything below (`SELECT_ALL`).
    SelectAll,
    /// ⌘A again on the row a select-all started from: Escape undoes it.
    ChangedYourMind,
}

/// How the cursor got where it is, which decides how the rest of the screen follows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorSource {
    /// The keyboard or the app moved it: scroll it into view, update the contact pane now.
    Command,
    /// The pointer moved onto a row: the contact pane follows once the mouse settles.
    Pointer,
}

/// The thread list's state.
pub struct Inbox {
    threads: Vec<ThreadSummary>,
    items: Vec<Item>,
    view: View,
    now: Zoned,
    cursor: Option<String>,
    cursor_source: CursorSource,
    /// Put the cursor here once the thread is listed (an undone thread coming back).
    pending_cursor: Option<String>,
    /// Scroll the cursor into view on the next frame, aligned to the top if set.
    reveal: Option<Reveal>,
    selected: selection::Selection,
    anchor: Option<String>,
    /// A hint the selection asks for, until the app takes it.
    hint: Option<SelectionHint>,
    /// What rows a bulk action waits or runs for show instead of their date (`Pending: Mark
    /// Done`).
    pending: HashMap<String, String>,
    /// Gmail can be reached (`network.online`): offline, rows with a send waiting say so.
    online: bool,
    /// A draft is popped out: the status line goes over the rows.
    popped_out: bool,
    /// The popped-out draft has the keyboard: the highlight goes.
    popout_focused: bool,
    /// The popped-out draft's card, over the rows: the pointer on it isn't on them.
    covered: Option<Rect>,
    /// Dismissed threads and when they started leaving (egui time).
    leaving: Vec<(ThreadSummary, f64)>,
    /// Threads sliding back in, from how far out (0–1 of the row's width) and since when.
    returning: Vec<(ThreadSummary, f32, f64)>,
    scroll: f32,
    /// The list's visible height, from the last frame, for paging.
    viewport: f32,
    /// Which row the pointer gives the cursor.
    hover: RowHover,
    /// The list has the keyboard: not while the folder drawer is open.
    focused: bool,
    /// The focus bar shows on the cursor's row: not while the search bar has the keyboard.
    bar: bool,
    /// The cursor's conversation is open in the thread pane, outside search.
    reading: bool,
    /// The highlight moves to the cursor without sliding next frame.
    jump: bool,
    /// Search results' file strips paged along: the first card shown and how far it's scrolled.
    file_pages: HashMap<String, (usize, f32)>,
    /// What in the header has the keyboard, when it has it.
    header_focus: Option<HeaderFocus>,
    /// Where Auto Advance goes from the open conversation once it has moved up the list while open:
    /// the conversations that were around it.
    advance_anchor: Option<AdvanceAnchor>,
}

/// The rows around the open conversation before it moved.
#[derive(Clone, Debug, PartialEq, Eq)]
struct AdvanceAnchor {
    thread: String,
    above: Option<String>,
    below: Option<String>,
}

/// The pointer's track: where it was last and which way it's heading.
#[derive(Clone, Copy, Debug, Default)]
struct Heading {
    /// The last position and when it was seen.
    last: Option<(Pos2, f64)>,
    /// The direction of travel in degrees (`atan(dy / dx)`), smoothed over quick moves.
    angle: Option<f32>,
}

/// Moves this close together are smoothed: a third of the new angle, two of the old.
const HEADING_SMOOTHING: f64 = 0.035;
/// A row crossed towards the right edge gets the cursor if the pointer stays on it this long.
const SELECT_AFTER: f64 = 0.1;

impl Heading {
    /// Records the pointer at `at` and says whether it's travelling towards the right edge of
    /// `area` (within the angles from where it was to the area's right corners), so the row
    /// under it shouldn't take the cursor yet. The corners are measured from the window's origin
    /// with the area's size.
    fn towards_right_edge(&mut self, at: Pos2, now: f64, area: Rect) -> bool {
        let Some((from, then)) = self.last.replace((at, now)) else {
            return false;
        };
        let elapsed = now - then;
        if elapsed <= 0.0 {
            return false;
        }
        let velocity = (at - from) / elapsed as f32;
        if velocity.length() < 1.0 {
            return false;
        }
        let raw = (velocity.y / velocity.x).atan().to_degrees();
        let angle = match self.angle {
            Some(previous) if elapsed < HEADING_SMOOTHING => raw / 3.0 + previous * 2.0 / 3.0,
            _ => raw,
        };
        self.angle = Some(angle);
        if velocity.x < 0.0 {
            return false;
        }
        let top = ((0.0 - from.y) / (area.width() - from.x))
            .atan()
            .to_degrees();
        let bottom = ((area.height() - from.y) / (area.width() - from.x))
            .atan()
            .to_degrees();
        top < angle && angle < bottom
    }
}

/// Which row a hover gives the cursor: the row under a pointer that really moved (not a list
/// scrolled under it), at once, unless the pointer is heading for the list's right edge, when only
/// once it has stayed on the row for [`SELECT_AFTER`]. The Snippets folder's rows follow it too.
#[derive(Clone, Debug, Default)]
pub(crate) struct RowHover {
    last_pointer: Option<Pos2>,
    /// Where the pointer is heading, to tell hovering a row from crossing the list on the way
    /// to the right pane.
    heading: Heading,
    /// A row crossed on the way to the right edge: it gets the cursor if the pointer is still
    /// on it at this time.
    deferred: Option<(String, f64)>,
}

impl RowHover {
    /// With the pointer over row `row` this frame (`None`: over none) of a list `area`, whose
    /// cursor is on `cursor`: the row that takes the cursor now, if any.
    pub(crate) fn update(
        &mut self,
        ui: &Ui,
        row: Option<String>,
        cursor: Option<&str>,
        now: f64,
        area: Rect,
    ) -> Option<String> {
        let pointer = ui.input(|input| input.pointer.hover_pos());
        let moved = pointer.is_some() && pointer != self.last_pointer;
        self.last_pointer = pointer;
        let mut takes = None;
        match (row, pointer) {
            (Some(id), Some(at)) => {
                if moved {
                    if self.heading.towards_right_edge(at, now, area) {
                        if cursor != Some(id.as_str())
                            && self
                                .deferred
                                .as_ref()
                                .is_none_or(|(deferred, _)| *deferred != id)
                        {
                            self.deferred = Some((id.clone(), now + SELECT_AFTER));
                        }
                    } else {
                        self.deferred = None;
                        takes = Some(id.clone());
                    }
                }
                // Due whether or not the pointer keeps moving on the row.
                if self
                    .deferred
                    .take_if(|(deferred, due)| *deferred == id && now >= *due)
                    .is_some()
                {
                    takes = Some(id);
                }
            }
            // Leaving the row drops it.
            _ => self.deferred = None,
        }
        if let Some((_, due)) = &self.deferred {
            ui.ctx()
                .request_repaint_after_secs((*due - now).max(0.0) as f32);
        }
        takes
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reveal {
    /// Scrolled into view only when it's off screen, and then centred.
    IfNeeded,
    /// Aligned to the top of the list.
    Top,
    /// Aligned to the bottom of the list.
    Bottom,
}

impl Inbox {
    /// An empty list, dated relative to `now`.
    pub fn new(now: Zoned) -> Self {
        Self {
            threads: Vec::new(),
            items: Vec::new(),
            view: View::default(),
            now,
            cursor: None,
            cursor_source: CursorSource::Command,
            pending_cursor: None,
            reveal: None,
            selected: selection::Selection::default(),
            anchor: None,
            hint: None,
            pending: HashMap::new(),
            online: true,
            popped_out: false,
            popout_focused: false,
            covered: None,
            leaving: Vec::new(),
            returning: Vec::new(),
            scroll: 0.0,
            viewport: 0.0,
            hover: RowHover::default(),
            focused: true,
            bar: true,
            reading: false,
            advance_anchor: None,
            jump: true,
            file_pages: HashMap::new(),
            header_focus: None,
        }
    }

    /// Says whether the cursor's conversation is open in the thread pane (not in search),
    /// which is when the list remembers where it was if it moves up. What it remembers stays
    /// while the cursor stays on the conversation.
    pub fn set_reading(&mut self, reading: bool) {
        self.reading = reading;
    }

    /// The conversation Auto Advance goes to `direction` from the open one, if it moved up the
    /// list while open: the one that was there before it moved, while it's still listed and
    /// not among `leaving`. It's used once.
    pub fn take_advance_anchor(&mut self, direction: isize, leaving: &[String]) -> Option<String> {
        let anchor = self
            .advance_anchor
            .take_if(|anchor| Some(&anchor.thread) == self.cursor.as_ref())?;
        let id = if direction > 0 {
            anchor.below.clone()
        } else {
            anchor.above.clone()
        };
        let usable = id.filter(|id| {
            self.thread(id).is_some() && !self.is_leaving(id) && !leaving.contains(id)
        });
        if usable.is_none() {
            self.advance_anchor = Some(anchor);
        }
        usable
    }

    /// Puts the cursor on the listed conversation `id`.
    pub fn point_at(&mut self, id: &str) {
        if self.thread(id).is_some() {
            self.set_cursor(Some(id.to_owned()), CursorSource::Command);
        }
    }

    fn segment(&mut self) {
        // Search results and Scheduled aren't grouped by date.
        if self.view.search.is_some() || self.view.unsegmented {
            self.items = (0..self.threads.len()).map(Item::Thread).collect();
            return;
        }
        let items = self
            .threads
            .iter()
            .enumerate()
            .map(|(at, thread)| (thread.received, at))
            .collect();
        self.items = dates::segment(items, &self.now)
            .into_iter()
            .map(|segment| match segment {
                Segment::Separator(label) => Item::Separator(label),
                Segment::Item(at) => Item::Thread(at),
            })
            .collect();
    }

    /// Dates the rows relative to `now`; the date separators move when the day does.
    pub fn set_now(&mut self, now: Zoned) {
        let moved = now.date() != self.now.date() || now.time_zone() != self.now.time_zone();
        self.now = now;
        if moved {
            self.segment();
        }
    }

    /// Says whether the list has the keyboard. Without it (the folder drawer has it) the
    /// cursor's row looks like any other.
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// Gives the header the keyboard (↑ on the first row), on its first tab: Important or Inbox, or
    /// the view's own. Not while selecting, when the header has the selection's actions.
    pub fn focus_header(&mut self) {
        if self.is_selecting() {
            return;
        }
        let items = header::focusables(&self.view);
        let home = self.view.tabs.iter().position(|tab| {
            matches!(
                tab.list,
                exo_store::ListId::Important | exo_store::ListId::Inbox
            ) && !tab.hidden
        });
        self.header_focus = home.map(HeaderFocus::Tab).or_else(|| {
            items
                .iter()
                .copied()
                .find(|item| matches!(item, HeaderFocus::Tab(_) | HeaderFocus::Title))
        });
    }

    /// The header has the keyboard.
    pub fn header_focused(&self) -> bool {
        self.header_focus.is_some()
    }

    /// What the list shows besides its rows.
    pub fn view(&self) -> &View {
        &self.view
    }

    /// What in the header has the keyboard.
    pub fn header_focus(&self) -> Option<HeaderFocus> {
        self.header_focus
    }

    /// The list has the keyboard back (↓, J, Escape in the header; the pointer on a row).
    pub fn blur_header(&mut self) {
        self.header_focus = None;
    }

    /// Moves the header's focus `step` items along, no further than either end; ← on the first item
    /// opens the folder drawer.
    pub fn move_header_focus(&mut self, step: isize) -> Option<HeaderAction> {
        let items = header::focusables(&self.view);
        let at = self
            .header_focus
            .and_then(|focus| items.iter().position(|item| *item == focus))?;
        if at == 0 && step < 0 {
            // The drawer takes the keyboard.
            self.header_focus = None;
            return Some(HeaderAction::Folders);
        }
        self.header_focus = items
            .get(at.saturating_add_signed(step).min(items.len() - 1))
            .copied();
        None
    }

    /// What Enter (a click on it) or Backspace (on the filter tab: it goes) does to what has the
    /// header's keyboard.
    pub fn header_key(&mut self, backspace: bool) -> Option<HeaderAction> {
        let focus = self.header_focus?;
        if backspace {
            return (focus == HeaderFocus::Filter).then_some(HeaderAction::ClearFilter);
        }
        self.header_focus = None;
        Some(focus.action())
    }

    /// Says whether the cursor's row shows the focus bar (the search bar has the keyboard).
    pub fn set_bar(&mut self, bar: bool) {
        self.bar = bar;
    }

    /// Sets what the header and empty states show.
    pub fn set_view(&mut self, view: View) {
        // A focused tab that's gone (or a selection taking the header over) gives the keyboard
        // back to the list.
        if self
            .header_focus
            .is_some_and(|focus| !header::focusables(&view).contains(&focus))
            || self.is_selecting()
        {
            self.header_focus = None;
        }
        let regroup = view.search.is_some() != self.view.search.is_some()
            || view.unsegmented != self.view.unsegmented;
        self.view = view;
        if regroup {
            self.segment();
        }
    }

    /// The cursor is on the first row the cursor can land on (or there's none).
    pub fn cursor_is_first(&self) -> bool {
        self.landable().first().copied() == self.cursor.as_deref() || self.cursor.is_none()
    }

    /// Switches to another list: no rows until its threads arrive, the cursor on its first row,
    /// scrolled to the top.
    pub fn clear(&mut self) {
        self.threads.clear();
        self.items.clear();
        self.cursor = None;
        self.pending_cursor = None;
        self.leaving.clear();
        self.returning.clear();
        self.clear_selection();
        self.scroll = 0.0;
        // Another list: the highlight doesn't slide from the last one's row, and its file strips
        // start at their first card.
        self.jump = true;
        self.file_pages.clear();
    }

    /// The conversations in the list, leaving rows included.
    pub fn count(&self) -> usize {
        self.threads.len()
    }

    /// The listed threads, in order.
    pub fn threads(&self) -> &[ThreadSummary] {
        &self.threads
    }

    /// The thread under the cursor.
    pub fn cursor(&self) -> Option<&ThreadSummary> {
        let id = self.cursor.as_deref()?;
        self.thread(id)
    }

    fn thread(&self, id: &str) -> Option<&ThreadSummary> {
        self.threads.iter().find(|thread| thread.id == id)
    }

    fn item_thread(&self, item: &Item) -> Option<&ThreadSummary> {
        match *item {
            Item::Thread(at) => self.threads.get(at),
            Item::Separator(_) => None,
        }
    }

    /// How the cursor last moved.
    pub fn cursor_source(&self) -> CursorSource {
        self.cursor_source
    }

    /// Threads the cursor can land on, in order: not leaving.
    fn landable(&self) -> Vec<&str> {
        self.threads
            .iter()
            .map(|thread| thread.id.as_str())
            .filter(|id| !self.is_leaving(id))
            .collect()
    }

    fn is_leaving(&self, id: &str) -> bool {
        self.leaving.iter().any(|(leaving, _)| leaving.id == id)
    }

    fn set_cursor(&mut self, id: Option<String>, source: CursorSource) {
        if id != self.cursor {
            self.reveal = (source == CursorSource::Command).then_some(Reveal::IfNeeded);
            // A hover moves the highlight at once, without sliding, and gives the list the
            // keyboard.
            self.jump |= source == CursorSource::Pointer;
            if source == CursorSource::Pointer {
                self.header_focus = None;
            }
        }
        self.cursor = id;
        self.cursor_source = source;
    }

    /// Moves the cursor `delta` rows (J/K, ↓/↑), stopping at either end. From no cursor, any
    /// move lands on the first row.
    pub fn move_cursor(&mut self, delta: isize) {
        let landable = self.landable();
        let next = match self
            .cursor
            .as_deref()
            .and_then(|id| landable.iter().position(|candidate| *candidate == id))
        {
            Some(at) => landable.get(
                at.saturating_add_signed(delta)
                    .min(landable.len().saturating_sub(1)),
            ),
            None => landable.first(),
        };
        let next = next.map(|id| (*id).to_owned());
        self.set_cursor(next, CursorSource::Command);
    }

    /// Takes the cursor off the list (the search bar took the keyboard).
    pub fn clear_cursor(&mut self) {
        self.set_cursor(None, CursorSource::Command);
    }

    /// Jumps the cursor to the first row (⌘↑, Home, `g g`), scrolled to the top.
    pub fn cursor_to_top(&mut self) {
        let first = self.landable().first().map(|id| (*id).to_owned());
        self.set_cursor(first, CursorSource::Command);
        self.reveal = Some(Reveal::Top);
    }

    /// Jumps the cursor to the last row (⌘↓, End, ⇧G).
    pub fn cursor_to_bottom(&mut self) {
        let last = self.landable().last().map(|id| (*id).to_owned());
        self.set_cursor(last, CursorSource::Command);
        self.reveal = Some(Reveal::Bottom);
    }

    /// Page down (Space) or up (⇧Space): the first row past the visible area's bottom (or top)
    /// edge becomes the cursor and is scrolled to the other edge.
    pub fn page(&mut self, direction: isize) {
        let (top, bottom) = (self.scroll, self.scroll + self.viewport);
        let rows: Vec<(&str, f32, f32)> = self
            .row_tops()
            .filter(|(id, _, _)| !self.is_leaving(id))
            .collect();
        let target = if direction > 0 {
            rows.iter()
                .find(|(_, row_top, height)| *row_top + height > bottom)
                .or(rows.last())
        } else {
            rows.iter()
                .rev()
                .find(|(_, row_top, _)| *row_top < top)
                .or(rows.first())
        };
        let Some((id, _, _)) = target else { return };
        let id = (*id).to_owned();
        self.set_cursor(Some(id), CursorSource::Command);
        self.reveal = Some(if direction > 0 {
            Reveal::Top
        } else {
            Reveal::Bottom
        });
    }

    /// Each item's top, below the list's top padding.
    fn item_tops(&self) -> impl Iterator<Item = f32> {
        self.items.iter().scan(self.status_height(), |top, item| {
            let at = *top;
            *top += self.item_height(item);
            Some(at)
        })
    }

    /// Each thread row's id, top (in content coordinates) and height.
    fn row_tops(&self) -> impl Iterator<Item = (&str, f32, f32)> {
        self.items
            .iter()
            .zip(self.item_tops())
            .filter_map(|(item, top)| {
                self.item_thread(item).map(|thread| {
                    (
                        thread.id.as_str(),
                        LIST_PADDING_TOP + top,
                        self.row_height(thread),
                    )
                })
            })
    }

    /// A row's height: taller for a search result with files.
    fn row_height(&self, thread: &ThreadSummary) -> f32 {
        if self.view.search.is_some() && !thread.files.is_empty() {
            ROW_WITH_FILES
        } else {
            ROW_HEIGHT
        }
    }

    fn item_height(&self, item: &Item) -> f32 {
        match item {
            Item::Separator(_) => SEPARATOR_HEIGHT,
            Item::Thread(at) => self
                .threads
                .get(*at)
                .map_or(ROW_HEIGHT, |thread| self.row_height(thread)),
        }
    }

    /// Selects or unselects the cursor's thread (X).
    pub fn toggle_selected(&mut self) {
        if let Some(id) = self.cursor.clone() {
            self.toggle(&id);
        }
    }

    fn toggle(&mut self, id: &str) {
        let Some(thread) = self.thread(id).cloned() else {
            return;
        };
        if self.selected.contains(&thread) {
            self.selected.remove(&thread);
        } else {
            self.select(&thread);
        }
        self.anchor = Some(id.to_owned());
    }

    /// Selects `thread`, asking for the ⌘A hint when it's the fourth picked or more.
    fn select(&mut self, thread: &ThreadSummary) {
        if self.selected.insert(thread) {
            self.hint = Some(SelectionHint::SelectAll);
        }
    }

    /// Selects the thread `id`, if listed.
    fn select_id(&mut self, id: &str) {
        if let Some(thread) = self.thread(id).cloned() {
            self.select(&thread);
        }
    }

    /// Extends the selection while moving the cursor `delta` rows (⇧J/⇧K), or shrinks it when
    /// moving back into it.
    pub fn extend_selection(&mut self, delta: isize) {
        let Some(from) = self.cursor.clone() else {
            return self.move_cursor(delta);
        };
        self.move_cursor(delta);
        let Some(to) = self.cursor.clone().filter(|to| *to != from) else {
            return;
        };
        if self.is_selected(&from) && self.is_selected(&to) {
            if let Some(thread) = self.thread(&from).cloned() {
                self.selected.remove(&thread);
            }
        } else {
            self.select_id(&from);
            self.select_id(&to);
        }
        self.anchor = Some(to);
    }

    /// Selects every thread between the selection anchor and `id` (shift-click).
    fn select_range(&mut self, id: &str) {
        let ids = self.landable();
        let to = ids.iter().position(|candidate| *candidate == id);
        let from = self
            .anchor
            .as_deref()
            .and_then(|anchor| ids.iter().position(|candidate| *candidate == anchor))
            .or(to);
        if let (Some(from), Some(to)) = (from, to) {
            let range: Vec<String> = ids[from.min(to)..=from.max(to)]
                .iter()
                .map(|id| (*id).to_owned())
                .collect();
            for id in range {
                self.select_id(&id);
            }
        }
    }

    /// Clears the selection (Esc in multi-select, ⌘⌥A).
    pub fn clear_selection(&mut self) {
        self.selected.clear();
        self.anchor = None;
    }

    /// Whether any conversation is selected, or a select-all is on.
    pub fn is_selecting(&self) -> bool {
        self.selected.is_active()
    }

    fn is_selected(&self, id: &str) -> bool {
        self.thread(id)
            .is_some_and(|thread| self.selected.contains(thread))
    }

    /// The loaded rows selected, in list order: not rows on their way out.
    fn selected_rows(&self) -> Vec<&ThreadSummary> {
        self.threads
            .iter()
            .filter(|thread| !self.is_leaving(&thread.id) && self.selected.contains(thread))
            .collect()
    }

    /// The selected threads that are loaded, in list order.
    pub fn selection(&self) -> Vec<String> {
        self.selected_rows()
            .into_iter()
            .map(|thread| thread.id.clone())
            .collect()
    }

    /// The loaded rows not selected while selecting, which a select-all leaves out.
    pub fn unselected(&self) -> Vec<String> {
        self.threads
            .iter()
            .filter(|thread| !self.is_leaving(&thread.id) && !self.selected.contains(thread))
            .map(|thread| thread.id.clone())
            .collect()
    }

    /// The threads an action applies to: the selection, or else the cursor's thread.
    pub fn actionable(&self) -> Vec<&ThreadSummary> {
        if self.is_selecting() {
            self.selected_rows()
        } else {
            self.cursor().into_iter().collect()
        }
    }

    /// The select-all's cutoff: the date at or before which every row is selected, loaded or not.
    pub fn cutoff(&self) -> Option<jiff::Timestamp> {
        self.selected.cutoff()
    }

    /// The selection is exactly the loaded rows selected: no select-all, or the list has no more
    /// rows to load.
    pub fn is_exact(&self) -> bool {
        self.selected.cutoff().is_none() || !self.view.more
    }

    /// Too much is selected to act on row by row: over 50 rows, or a select-all that reaches rows
    /// not loaded. The shortcuts panel leaves out what only works on loaded rows.
    pub fn is_select_all_mode(&self) -> bool {
        self.selected_rows().len() > 50 || !self.is_exact()
    }

    /// Selects everything (⌘⇧A), or clears the selection when it already is.
    pub fn select_all(&mut self) {
        let (count, total) = self.selection_count();
        if count > 0 && count == total {
            return self.clear_selection();
        }
        self.select_from(0);
    }

    /// Selects the cursor's thread and every one dated at or before it, loaded or not (⌘A); doing
    /// it again from the same row hints that Escape undoes it.
    pub fn select_all_from_here(&mut self) {
        let rows = self.landable_rows();
        let Some(at) = self
            .cursor
            .as_deref()
            .and_then(|id| rows.iter().position(|row| row.id == id))
        else {
            return;
        };
        if self.selected.is_cutoff(rows[at]) {
            self.hint = Some(SelectionHint::ChangedYourMind);
        }
        self.select_from(at);
    }

    fn select_from(&mut self, at: usize) {
        let rows: Vec<ThreadSummary> = self.landable_rows().into_iter().cloned().collect();
        let rows: Vec<&ThreadSummary> = rows.iter().collect();
        self.selected.select_from(&rows, at);
    }

    /// The rows the cursor can land on.
    fn landable_rows(&self) -> Vec<&ThreadSummary> {
        self.threads
            .iter()
            .filter(|thread| !self.is_leaving(&thread.id))
            .collect()
    }

    /// The list's total for a select-all, and whether it's every thread there is: the local
    /// count, or the loaded rows when that's all of them or there's no count (filtered, search).
    fn total(&self) -> (usize, bool) {
        let loaded = self.landable_rows().len();
        match self.view.count {
            Some(count) if self.view.more && self.view.filter.is_none() => {
                (count.threads.max(loaded), count.exact)
            }
            _ => (loaded, !self.view.more),
        }
    }

    /// How many conversations are selected and the list's total.
    pub fn selection_count(&self) -> (usize, usize) {
        let (total, _) = self.total();
        let (count, _) = self.selected.count(&self.landable_rows(), total);
        (count, total)
    }

    /// The select-all hint's copy, while it shows: a select-all of more than one conversation, with
    /// `view` being where (`Important`, `this search`).
    pub fn select_all_hint(&self, view: &str) -> Option<String> {
        self.selected.cutoff()?;
        let rows = self.landable_rows();
        let (total, complete) = self.total();
        let counted = self.selected.count(&rows, total);
        if counted.0 <= 1 {
            return None;
        }
        // Drafts holds drafts, and says nothing about where.
        let drafts = self.view.title == "Drafts" && self.view.search.is_none();
        let name = if drafts { "drafts" } else { "conversations" };
        let message = self.selected.message(&rows, counted, total, complete, name);
        Some(if drafts {
            format!("{message}.")
        } else {
            format!("{message} in {view}.")
        })
    }

    /// The hint the selection last asked for.
    pub fn take_hint(&mut self) -> Option<SelectionHint> {
        self.hint.take()
    }

    /// Whether the select-all hint shows: a select-all of more than one conversation.
    pub fn shows_select_all_hint(&self) -> bool {
        self.selected.cutoff().is_some() && self.selection_count().0 > 1
    }

    /// Sets what rows a bulk action waits or runs for show instead of their date.
    pub fn set_pending(&mut self, pending: HashMap<String, String>) {
        self.pending = pending;
    }

    /// Whether Gmail can be reached, as the offline bar says.
    pub fn set_online(&mut self, online: bool) {
        self.online = online;
    }

    /// Whether a draft is popped out.
    pub fn set_popped_out(&mut self, popped_out: bool) {
        self.popped_out = popped_out;
    }

    /// Whether the popped-out draft has the keyboard.
    pub fn set_popout_focused(&mut self, focused: bool) {
        self.popout_focused = focused;
    }

    /// Where the popped-out draft's card covers the list, if it does.
    pub fn set_covered(&mut self, card: Option<Rect>) {
        self.covered = card;
    }

    /// The status line over the rows: a search's, or any list's while a draft is popped out, which
    /// then says `No conversations found.` once it's all here and empty, or offline, with more than
    /// is stored, `Search results may be partial when offline.`, whatever the list.
    fn top_status(&self) -> Option<&'static str> {
        match &self.view.search {
            Some(search) => search.status,
            None if !self.popped_out || self.at_inbox_zero() => None,
            None if !self.online && !self.view.complete => Some(PARTIAL_OFFLINE),
            None => (self.view.complete && !self.view.loading && self.threads.is_empty())
                .then_some(NOTHING_FOUND),
        }
    }

    /// Whether every conversation is selected (the header's checkmark: the count is the total).
    fn all_selected(&self) -> bool {
        let (count, total) = self.selection_count();
        count > 0 && count == total
    }

    /// Slides `ids` out of the list (their action takes them out of this view), clears the
    /// selection, and moves the cursor to the next row down (the default *Auto advance*), or up
    /// from the bottom.
    pub fn dismiss(&mut self, ids: &[String], now: f64) {
        self.dismiss_towards(ids, 1, now);
    }

    /// [`Self::dismiss`], the cursor going up first when `direction` is negative. Returns
    /// whether the cursor moved `direction`, not the other way for want of a row.
    pub fn dismiss_towards(&mut self, ids: &[String], direction: isize, now: f64) -> bool {
        let landable = self.landable();
        let cursor_at = self
            .cursor
            .as_deref()
            .and_then(|id| landable.iter().position(|candidate| *candidate == id));
        let cursor_leaves = self.cursor.as_ref().is_some_and(|id| ids.contains(id));
        let mut went = false;
        let next = cursor_at.filter(|_| cursor_leaves).and_then(|at| {
            let below = landable[at..]
                .iter()
                .find(|id| !ids.iter().any(|leaving| leaving == *id));
            let above = landable[..at]
                .iter()
                .rev()
                .find(|id| !ids.iter().any(|leaving| leaving == *id));
            let (ahead, behind) = if direction < 0 {
                (above, below)
            } else {
                (below, above)
            };
            went = ahead.is_some();
            ahead.or(behind).map(|id| (*id).to_owned())
        });
        for id in ids {
            if let Some(thread) = self.thread(id).filter(|_| !self.is_leaving(id)).cloned() {
                self.leaving.push((thread, now));
            }
        }
        self.clear_selection();
        if cursor_leaves {
            self.set_cursor(next, CursorSource::Command);
        }
        went
    }

    /// Undo: rows still sliding out slide back in from where they are, and the cursor goes back
    /// to `cursor` (now, or once its thread is listed again).
    pub fn restore(&mut self, ids: &[String], cursor: Option<String>, now: f64) {
        for id in ids {
            if let Some(at) = self
                .leaving
                .iter()
                .position(|(leaving, _)| leaving.id == *id)
            {
                let (thread, started) = self.leaving.remove(at);
                let out = LEAVE_CURVE.at(((now - started) as f32 / LEAVE_SLIDE).min(1.0));
                self.returning.push((thread, out, now));
            }
        }
        match cursor {
            Some(id) if self.thread(&id).is_some() => {
                self.set_cursor(Some(id), CursorSource::Command)
            }
            Some(id) => self.pending_cursor = Some(id),
            None => {}
        }
    }

    /// Replaces the list's threads (newest first), keeping the cursor and selection on threads
    /// that are still there. Rows still sliding out or back in stay in place until they're done.
    pub fn set_threads(&mut self, mut threads: Vec<ThreadSummary>) {
        let animating = self
            .leaving
            .iter()
            .map(|(thread, _)| thread)
            .chain(self.returning.iter().map(|(thread, _, _)| thread));
        let missing: Vec<ThreadSummary> = animating
            .filter(|thread| !threads.iter().any(|listed| listed.id == thread.id))
            .cloned()
            .collect();
        for thread in missing {
            // Where it sorts: newest first, the higher id first on the same date.
            let at = threads
                .iter()
                .position(|listed| (listed.received, &listed.id) < (thread.received, &thread.id))
                .unwrap_or(threads.len());
            threads.insert(at, thread);
        }
        self.capture_advance_anchor(&threads);
        let has = |id: &str| threads.iter().any(|thread| thread.id == id);
        if let Some(id) = self.pending_cursor.take_if(|id| has(id)) {
            self.set_cursor(Some(id), CursorSource::Command);
        }
        if !self.cursor.as_deref().is_some_and(has) {
            // Gone from the list: the row that was below it, else the one above, so J and K go on
            // from where it was; the first row on a list that's new.
            let at = self
                .cursor
                .as_deref()
                .and_then(|id| self.threads.iter().position(|thread| thread.id == id));
            let neighbour = at.and_then(|at| {
                [at + 1, at.wrapping_sub(1)]
                    .into_iter()
                    .filter_map(|near| self.threads.get(near))
                    .find(|thread| has(&thread.id))
            });
            self.cursor = match neighbour {
                Some(thread) => Some(thread.id.clone()),
                None => {
                    self.jump = true;
                    threads.first().map(|thread| thread.id.clone())
                }
            };
        }
        self.selected.retain(has);
        self.threads = threads;
        self.segment();
    }

    /// Remembers the rows around the open conversation when `threads`, the list's next rows,
    /// move it up past them: the row above it now below it, or the row below it no longer next to
    /// it. Once per open conversation.
    fn capture_advance_anchor(&mut self, threads: &[ThreadSummary]) {
        if self
            .advance_anchor
            .as_ref()
            .is_some_and(|anchor| Some(&anchor.thread) != self.cursor.as_ref())
        {
            self.advance_anchor = None;
        }
        let Some(id) = self
            .cursor
            .clone()
            .filter(|_| self.reading && self.advance_anchor.is_none())
        else {
            return;
        };
        let rank =
            |rows: &[ThreadSummary], id: &str| rows.iter().position(|thread| thread.id == id);
        let Some(old) = rank(&self.threads, &id) else {
            return;
        };
        let above = old
            .checked_sub(1)
            .and_then(|at| self.threads.get(at))
            .map(|thread| thread.id.clone());
        let below = self.threads.get(old + 1).map(|thread| thread.id.clone());
        let Some(new) = rank(threads, &id) else {
            return;
        };
        let below_moved = below
            .as_deref()
            .and_then(|below| rank(threads, below))
            .is_some_and(|at| at > new + 1);
        let above_passed = above
            .as_deref()
            .and_then(|above| rank(threads, above))
            .is_some_and(|at| at > new);
        if below_moved || above_passed {
            self.advance_anchor = Some(AdvanceAnchor {
                thread: id,
                above,
                below,
            });
        }
    }

    /// Drops rows whose slide-out finished, and slide-ins that finished. Call once a frame,
    /// before asking about the rows.
    pub fn settle(&mut self, now: f64) {
        self.returning
            .retain(|(_, _, started)| now - started < f64::from(RETURN_SLIDE));
        let departed: Vec<String> = self
            .leaving
            .iter()
            .filter(|(_, started)| now - started >= LEAVE_REMOVED)
            .map(|(thread, _)| thread.id.clone())
            .collect();
        if departed.is_empty() {
            return;
        }
        self.leaving
            .retain(|(thread, _)| !departed.contains(&thread.id));
        self.threads.retain(|thread| !departed.contains(&thread.id));
        self.segment();
    }

    /// Whether the list is empty for good: loaded, nothing left to fetch.
    fn is_empty(&self) -> bool {
        self.threads.is_empty() && !self.view.loading && !self.view.more
    }

    /// Inbox Zero: an inbox view that is empty for good. The header and list give way to the
    /// backdrop, and the app hides the right pane.
    pub fn at_inbox_zero(&self) -> bool {
        self.view.inbox && self.view.filter.is_none() && self.is_empty()
    }

    /// Under the rows: the spinner until the list has loaded all it can (which offline is what's
    /// stored), then the status line (search results have theirs at the top).
    fn bottom(&self) -> Bottom {
        let offline_end = !self.online && !self.view.complete;
        if self.view.loading || (self.view.more && !offline_end) {
            return Bottom::Spinner;
        }
        // The status line is over the rows instead.
        if self.view.search.is_some() || self.popped_out {
            return Bottom::None;
        }
        match (self.threads.is_empty(), offline_end) {
            (true, true) => Bottom::Status(EMPTY_OFFLINE),
            (true, false) if self.at_inbox_zero() => Bottom::None,
            (true, false) => Bottom::Status(EMPTY),
            (false, true) => Bottom::Status(EARLIER_OFFLINE),
            (false, false) => Bottom::None,
        }
    }

    /// Paints the view in `rect` (the content pane) and handles the mouse.
    pub fn show(&mut self, ui: &mut Ui, rect: Rect, rem: f32, window_width: f32) -> Vec<Action> {
        let now = ui.input(|input| input.time);
        let palette = exo_ui::palette(ui.ctx());
        let zero = self.at_inbox_zero();
        if !zero {
            ui.painter()
                .rect_filled(rect, 0.0, palette.background_base_default);
        }
        let view = rect.shrink2(vec2(VIEW_PADDING_X, 0.0));
        let search = self.view.search.clone();
        let compact = window_width <= 1152.0;
        // The inbox views' tabs wrap onto more lines (`flex-wrap`), making the header taller.
        let header_height = match &search {
            Some(search) => search.header,
            None if self.view.inbox => {
                let probe = Rect::from_min_size(view.min, vec2(view.width(), header::HEIGHT));
                header::height_shown(
                    ui,
                    header::layout(ui, probe, rem, compact, &self.view).lines,
                )
            }
            None => header::HEIGHT,
        };
        let header_rect = Rect::from_min_size(view.min, vec2(view.width(), header_height));
        let list_rect = Rect::from_min_max(pos2(view.left(), header_rect.bottom()), view.max);
        let geometry = row::Geometry::new(list_rect.width(), rem, compact);

        let mut actions = self.list(ui, list_rect, &geometry, now);
        if let Some(status) = self.top_status() {
            STATUS.paint_line(
                &ui.painter_at(list_rect),
                pos2(
                    list_rect.left() + geometry.padding_left(),
                    list_rect.top() + LIST_PADDING_TOP + 10.0 + 1.0 - self.scroll,
                ),
                status,
                palette.text_base_subdued,
                None,
            );
        }
        self.paint_bottom(ui, list_rect, &geometry, now);
        let context = if search.is_some() {
            "this search"
        } else {
            self.view.title.as_str()
        };
        if let Some(text) = self.select_all_hint(context) {
            select_all_hint(ui, rect, rem, &text);
        }
        if search.is_some() {
            // The search header is the app's; the scroll shadow under it is the list's.
            exo_ui::scroll_shadow(
                ui.painter(),
                ui.id().with("thread-list-header").with("shadow"),
                header_rect.left_bottom(),
                header_rect.width(),
                self.scroll > 0.0,
            );
            if !self.leaving.is_empty() || !self.returning.is_empty() {
                ui.ctx().request_repaint();
            }
            return actions;
        }
        let mode = if !self.is_selecting() {
            header::Mode::Normal
        } else {
            let done = self
                .threads
                .iter()
                .filter(|thread| self.selected.contains(thread))
                .all(|thread| thread.done);
            let selected: Vec<&ThreadSummary> = self
                .threads
                .iter()
                .filter(|thread| self.selected.contains(thread))
                .collect();
            // Remind me unless every selected conversation is drafts only: set when they all have a
            // reminder, with the reminder's status for one conversation only.
            let remind = selected
                .iter()
                .any(|thread| !thread.draft_only)
                .then(|| header::Remind {
                    set: selected.iter().all(|thread| thread.has_reminder()),
                    status: match selected.as_slice() {
                        [thread] => thread
                            .reminder
                            .as_ref()
                            .map(|reminder| crate::remind::status(reminder, &self.now)),
                        _ => None,
                    },
                });
            header::Mode::Selecting {
                all: self.all_selected(),
                done,
                remind,
            }
        };
        // The streak animation hides the header at once.
        let header = if self.view.hide_chrome {
            None
        } else {
            header::show(
                ui,
                header_rect,
                rem,
                compact,
                &self.view,
                &mode,
                self.scroll > 0.0,
                zero,
                self.header_focus,
            )
        };
        match header {
            Some(HeaderAction::ToggleSelectAll) if self.all_selected() => self.clear_selection(),
            Some(HeaderAction::ToggleSelectAll) => self.select_all(),
            Some(HeaderAction::Background) => self.clear_selection(),
            Some(HeaderAction::Done) => actions.push(Action::ToggleDone),
            Some(HeaderAction::Remind) => actions.push(Action::Remind),
            Some(HeaderAction::Command) => actions.push(Action::CommandPalette),
            Some(other) => actions.push(Action::Header(other)),
            None => {}
        }
        if zero {
            if zero::show(
                ui,
                rect,
                rem,
                compact,
                self.view.streak,
                self.view.hide_chrome,
            ) {
                actions.push(Action::Achievements);
            }
            if self.view.nudge
                && let Some(action) = nudge::show(ui, list_rect)
            {
                actions.push(Action::Nudge(action));
            }
        }
        if !self.leaving.is_empty() || !self.returning.is_empty() {
            ui.ctx().request_repaint();
        }
        actions
    }

    fn content_height(&self) -> f32 {
        let hint = if self.shows_select_all_hint() {
            HINT_ROOM
        } else {
            0.0
        };
        self.rows_end() + self.bottom().height() + hint
    }

    /// Where the rows end, in content coordinates.
    fn rows_end(&self) -> f32 {
        LIST_PADDING_TOP
            + self.status_height()
            + self
                .items
                .iter()
                .map(|item| self.item_height(item))
                .sum::<f32>()
    }

    /// Paints [`Self::bottom`] under the rows in `list`.
    fn paint_bottom(&self, ui: &Ui, list: Rect, geometry: &row::Geometry, now: f64) {
        let top = list.top() + self.rows_end() - self.scroll;
        let painter = ui.painter_at(list);
        match self.bottom() {
            Bottom::Spinner => {
                let spinner = Rect::from_center_size(
                    pos2(list.center().x, top + SPINNER_TOP + SPINNER / 2.0),
                    Vec2::splat(SPINNER),
                );
                // It turns only while it's on screen.
                if spinner.intersects(list) {
                    // The spinner: black at .25, white in Carbon.
                    exo_ui::paint_spinner(
                        &painter,
                        spinner,
                        now,
                        exo_ui::palette(ui.ctx()).ink(0.25, 0.25),
                    );
                }
            }
            // `top: 1px` inside its 10px padding.
            Bottom::Status(text) => {
                STATUS.paint_line(
                    &painter,
                    pos2(list.left() + geometry.padding_left(), top + 10.0 + 1.0),
                    text,
                    exo_ui::palette(ui.ctx()).text_base_subdued,
                    None,
                );
            }
            Bottom::None => {}
        }
    }

    /// Room the status line over the rows takes (10px, a 16px line, 10px), when there are rows
    /// under it.
    fn status_height(&self) -> f32 {
        let shown = self.top_status().is_some() && !self.items.is_empty();
        if shown { 36.0 } else { 0.0 }
    }

    /// Whether the cursor's row is wholly in view.
    fn cursor_in_view(&self) -> bool {
        let Some(id) = self.cursor.as_deref() else {
            return true;
        };
        self.row_tops()
            .find(|(row, _, _)| *row == id)
            .is_some_and(|(_, top, height)| {
                top >= self.scroll && top + height <= self.scroll + self.viewport
            })
    }

    /// Scrolls the cursor's row into view as `reveal` asks.
    fn reveal_cursor(&mut self, reveal: Reveal) {
        let Some(id) = self.cursor.as_deref() else {
            return;
        };
        let Some((_, top, height)) = self.row_tops().find(|(row, _, _)| *row == id) else {
            return;
        };
        let bottom = top + height;
        self.scroll = match reveal {
            Reveal::IfNeeded if top < self.scroll || bottom > self.scroll + self.viewport => {
                top + height / 2.0 - self.viewport / 2.0
            }
            Reveal::IfNeeded => self.scroll,
            Reveal::Top => top - LIST_PADDING_TOP,
            Reveal::Bottom => bottom - self.viewport,
        };
    }

    fn list(&mut self, ui: &mut Ui, rect: Rect, geometry: &row::Geometry, now: f64) -> Vec<Action> {
        self.viewport = rect.height();
        let max_scroll = (self.content_height() - rect.height()).max(0.0);
        let hovered = ui.rect_contains_pointer(rect);
        if hovered {
            self.scroll -= ui.input(|input| input.smooth_scroll_delta.y);
        }
        if let Some(reveal) = self.reveal.take() {
            // A move to a row off screen jumps there (checked before it scrolls the row in).
            self.jump |= !self.cursor_in_view();
            self.reveal_cursor(reveal);
        }
        // Find scrolls to a match in a row out of view, and searches the rows react-list renders
        // (`threshold: 2000`), which paint for it out of view and out of the pointer's reach.
        let find = ui.id().with("thread-list-find");
        if let Some(delta) = exo_ui::find::reveal(ui.ctx(), find) {
            self.scroll += delta.y;
        }
        self.scroll = self.scroll.clamp(0.0, max_scroll);
        let origin = pos2(rect.left(), rect.top() + LIST_PADDING_TOP - self.scroll);
        let painter = ui.painter_at(rect);
        let _find = exo_ui::find::Scope::begin(ui.ctx(), find, rect, self.scroll);
        let reach = if exo_ui::find::searching(ui.ctx()) {
            rect.expand2(vec2(0.0, LOAD_AHEAD))
        } else {
            rect
        };
        let mut out_of_view = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        out_of_view.set_clip_rect(Rect::NOTHING);

        let tops: Vec<f32> = self.item_tops().collect();
        let cursor_item = self.cursor.as_deref().and_then(|id| {
            self.items
                .iter()
                .position(|item| self.item_thread(item).is_some_and(|thread| thread.id == id))
        });
        let cursor_target = cursor_item.map(|i| HighlightTarget {
            top: tops[i],
            height: self.item_height(&self.items[i]),
        });
        let cursor_selected = self
            .cursor
            .as_deref()
            .is_some_and(|id| self.is_selected(id));
        // With the folder drawer open, the highlight fades out over 100ms and the cursor's row
        // shows its date. So with the header's keyboard; with the pop-out's, only the highlight
        // goes.
        let keyboard = self.focused && self.header_focus.is_none();
        let away = motion::transition_in(
            ui.ctx(),
            ui.id().with("thread-list-away"),
            !keyboard || self.popout_focused,
            0.1,
            CubicBezier::EASE,
        );
        let mut highlight_painter = painter.clone();
        highlight_painter.multiply_opacity(1.0 - away);
        let highlight_bar = ListHighlight::new(ui.id().with("thread-list-highlight")).paint(
            &highlight_painter,
            origin,
            rect.width(),
            cursor_target,
            HighlightState {
                focused: keyboard && self.bar,
                selected: cursor_selected,
                jump: std::mem::take(&mut self.jump),
                joined_above: cursor_item.is_some_and(|i| self.neighbour_selected(i, -1)),
                joined_below: cursor_item.is_some_and(|i| self.neighbour_selected(i, 1)),
            },
        );

        let mut events = Vec::new();
        let mut hovered_row = None;
        // On the pop-out's card the pointer is the card's, not a row's.
        let covered = |at: Pos2| self.covered.is_some_and(|card| card.contains(at));
        let on_rows = !ui
            .input(|input| input.pointer.hover_pos())
            .is_some_and(covered);
        let right_click = ui
            .input(|input| input.pointer.button_clicked(egui::PointerButton::Secondary))
            .then(|| ui.input(|input| input.pointer.interact_pos()))
            .flatten()
            .filter(|at| !covered(*at));
        let mut menu = None;
        for (i, &top) in tops.iter().enumerate() {
            let item_rect = Rect::from_min_size(
                origin + vec2(0.0, top),
                vec2(rect.width(), self.item_height(&self.items[i])),
            );
            if !item_rect.intersects(reach) {
                continue;
            }
            match &self.items[i] {
                Item::Separator(label) => row::separator(&painter, item_rect, geometry, label),
                &Item::Thread(at) => {
                    let ui = if item_rect.intersects(rect) {
                        &mut *ui
                    } else {
                        &mut out_of_view
                    };
                    let thread = &self.threads[at];
                    let elapsed = self
                        .leaving
                        .iter()
                        .find(|(leaving, _)| leaving.id == thread.id)
                        .map(|(_, started)| now - started);
                    if elapsed.is_some_and(|elapsed| elapsed >= LEAVE_HIDDEN) {
                        continue;
                    }
                    // Leaving slides the row its full width to the left; returning slides it back.
                    let returning = self
                        .returning
                        .iter()
                        .find(|(returning, _, _)| returning.id == thread.id)
                        .map(|(_, out, started)| {
                            out * (1.0
                                - LEAVE_CURVE.at(((now - started) as f32 / RETURN_SLIDE).min(1.0)))
                        });
                    let out = elapsed
                        .map(|elapsed| LEAVE_CURVE.at((elapsed as f32 / LEAVE_SLIDE).min(1.0)))
                        .or(returning)
                        .unwrap_or(0.0);
                    let shift = -item_rect.width() * out;
                    let state = row::State {
                        cursor: keyboard && self.cursor.as_deref() == Some(thread.id.as_str()),
                        selected: self.selected.contains(thread),
                        joined_above: self.neighbour_joins(i, -1),
                        joined_below: self.neighbour_joins(i, 1),
                        archivable: self.view.archivable,
                        pending: self.pending.get(&thread.id).map(String::as_str),
                        offline: !self.online,
                        strip: (self.row_height(thread) == ROW_WITH_FILES).then(|| {
                            self.file_pages
                                .get(&thread.id)
                                .map_or(0.0, |(_, scrolled)| *scrolled)
                        }),
                    };
                    let response = ui.interact(
                        item_rect.intersect(rect),
                        ui.id().with(("thread-row", &thread.id)),
                        Sense::click(),
                    );
                    let hit = row::paint(
                        ui,
                        &painter,
                        item_rect.translate(vec2(shift, 0.0)),
                        geometry,
                        thread,
                        &state,
                        &self.now,
                    );
                    if elapsed.is_none() {
                        // Anywhere on the row, as CSS `:hover` reaches it through its children:
                        // the row's own response isn't hovered while a chip, the checkmark or a
                        // hover action on top of it is.
                        if on_rows && ui.rect_contains_pointer(item_rect.intersect(rect)) {
                            hovered_row = Some(thread.id.clone());
                        }
                        if let Some(at) =
                            right_click.filter(|at| item_rect.intersect(rect).contains(*at))
                        {
                            menu = Some((thread.id.clone(), at));
                        }
                        if hit.is_some() || response.clicked() {
                            events.push((thread.id.clone(), hit));
                        }
                    }
                }
            }
        }
        highlight_bar.paint(&highlight_painter);
        if let Some(id) = self
            .hover
            .update(ui, hovered_row, self.cursor.as_deref(), now, rect)
        {
            self.set_cursor(Some(id), CursorSource::Pointer);
        }

        let modifiers = ui.input(|input| input.modifiers);
        let mut actions = Vec::new();
        if let Some((id, at)) = menu {
            // The menu acts on the selection when the row is in it, else on the row alone, the
            // selection let go; the row takes the cursor as a hover does.
            if !self.is_selected(&id) {
                self.clear_selection();
            }
            self.set_cursor(Some(id), CursorSource::Pointer);
            actions.push(Action::RowMenu(at));
        }
        if self.view.more && self.content_height() - (self.scroll + rect.height()) < LOAD_AHEAD {
            actions.push(Action::More);
        }
        for (id, hit) in events {
            match hit {
                Some(row::Hit::Done) => actions.push(Action::ToggleDone),
                Some(row::Hit::Remind) => actions.push(Action::Remind),
                Some(row::Hit::Command) => actions.push(Action::CommandPalette),
                Some(row::Hit::Status) if modifiers.shift => self.select_range(&id),
                Some(row::Hit::Status) if modifiers.command || self.is_selecting() => {
                    self.toggle(&id)
                }
                Some(row::Hit::Status) => actions.push(Action::Open(id)),
                Some(row::Hit::File { index, download }) => {
                    if let Some(file) = self.thread(&id).and_then(|thread| thread.files.get(index))
                    {
                        actions.push(Action::File {
                            file: file.clone(),
                            download,
                        });
                    }
                }
                Some(row::Hit::Page(direction)) => self.page_files(&id, direction, geometry),
                Some(row::Hit::Label(label)) if !self.is_selecting() => {
                    actions.push(Action::ShowLabel(label))
                }
                Some(row::Hit::Label(_) | row::Hit::Checkmark) if modifiers.shift => {
                    self.select_range(&id)
                }
                // The checkmark asks for the select hint, or the deselect one.
                Some(row::Hit::Checkmark) => {
                    actions.push(Action::SelectHint {
                        deselect: self.is_selected(&id),
                    });
                    self.toggle(&id);
                }
                Some(row::Hit::Label(_)) => self.toggle(&id),
                None if modifiers.shift => self.select_range(&id),
                None if modifiers.command || self.is_selecting() => self.toggle(&id),
                None => actions.push(Action::Open(id)),
            }
        }
        actions
    }

    /// Pages a search result's file strip `direction`: by as many cards as fit (less the subject
    /// column's lead on the first page), no further than its end.
    fn page_files(&mut self, id: &str, direction: isize, geometry: &row::Geometry) {
        let Some(files) = self.thread(id).map(|thread| thread.files.len()) else {
            return;
        };
        let (outer, inner) = geometry.strip_widths(files);
        let (index, scrolled) = self.file_pages.get(id).copied().unwrap_or_default();
        let room = if scrolled == 0.0 {
            outer - 0.225 * outer
        } else {
            outer
        };
        let page = (room / row::CARD_STEP).floor().max(1.0) as usize;
        let max = (inner - outer).max(0.0);
        let next = if direction > 0 {
            let to = (index + page) as f32 * row::CARD_STEP;
            if to > max {
                ((max / row::CARD_STEP).ceil() as usize, max)
            } else {
                (index + page, to)
            }
        } else {
            index
                .checked_sub(page)
                .map_or((0, 0.0), |to| (to, to as f32 * row::CARD_STEP))
        };
        self.file_pages.insert(id.to_owned(), next);
    }

    /// Whether the item `offset` away from item `i` is a selected thread.
    fn neighbour_selected(&self, i: usize, offset: isize) -> bool {
        self.neighbour(i, offset)
            .is_some_and(|thread| self.selected.contains(thread))
    }

    /// Whether a selected row and the item `offset` away from it join into one block: it is
    /// selected too, or it's the cursor.
    fn neighbour_joins(&self, i: usize, offset: isize) -> bool {
        self.neighbour(i, offset).is_some_and(|thread| {
            self.selected.contains(thread) || self.cursor.as_deref() == Some(thread.id.as_str())
        })
    }

    fn neighbour(&self, i: usize, offset: isize) -> Option<&ThreadSummary> {
        i.checked_add_signed(offset)
            .and_then(|j| self.items.get(j))
            .and_then(|item| self.item_thread(item))
    }
}

/// The select-all's pill over the bottom of the list view `view`, saying `text`. It takes the
/// clicks under it.
fn select_all_hint(ui: &mut Ui, view: Rect, rem: f32, text: &str) {
    let palette = exo_ui::palette(ui.ctx());
    // `padding: 0 2rem 20px calc(2.5rem - 19px)` around a 36px box with a 1px border.
    let pill = Rect::from_min_max(
        pos2(view.left() + 2.5 * rem - 19.0, view.bottom() - HINT_HEIGHT),
        pos2(view.right() - 2.0 * rem, view.bottom() - HINT_HEIGHT + 36.0),
    );
    let _ = ui.interact(pill, ui.id().with("select-all-hint"), Sense::click());
    let painter = ui.painter();
    // The background reaches under the border (`background-clip: border-box`).
    // The pill: `#fff` with text at `rgba(0, 0, 0, .7)`, in Carbon `#464b53` with
    // `hsla(0, 0%, 100%, .85)`.
    let background = match palette.theme {
        exo_ui::Theme::Snow => egui::Color32::WHITE,
        exo_ui::Theme::Carbon => egui::Color32::from_rgb(0x46, 0x4b, 0x53),
    };
    painter.rect_filled(pill, 4.0, background);
    painter.rect_stroke(
        pill,
        4.0,
        egui::Stroke::new(1.0, egui::Color32::from_black_alpha(20)),
        egui::StrokeKind::Inside,
    );
    // `padding: 11px calc(1rem + 18px) 10px`, cut with an ellipsis.
    let left = pill.left() + 1.0 + rem + 18.0;
    HINT_TEXT.paint_line(
        painter,
        pos2(left, pill.top() + 1.0 + 11.0),
        text,
        palette.ink(0.7, 0.85),
        Some(pill.right() - 1.0 - rem - 18.0 - left),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mailbox::Attachments;

    fn thread(i: usize, now: &Zoned) -> ThreadSummary {
        ThreadSummary {
            id: format!("t{i}"),
            participants: Vec::new(),
            drafts: 0,
            draft_only: false,
            subject: format!("Subject {i}"),
            snippet: String::new(),
            labels: Vec::new(),
            label_ids: Vec::new(),
            recipients: false,
            received: now.timestamp() - jiff::SignedDuration::from_mins(i as i64),
            unread: false,
            starred: false,
            done: false,
            muted: false,
            attachment: Attachments::None,
            contact: None,
            removed_by: Vec::new(),
            messages: 1,
            search: None,
            failure: None,
            sending: false,
            scheduled: None,
            reminder: None,
            files: Vec::new(),
        }
    }

    fn now() -> Zoned {
        "2026-11-17T09:00:00[America/New_York]"
            .parse()
            .expect("valid time")
    }

    fn inbox(count: usize) -> Inbox {
        let mut inbox = Inbox::new(now());
        inbox.set_threads((0..count).map(|i| thread(i, &now())).collect());
        inbox
    }

    fn cursor(inbox: &Inbox) -> Option<&str> {
        inbox.cursor().map(|thread| thread.id.as_str())
    }

    fn ids(inbox: &Inbox) -> Vec<String> {
        inbox
            .actionable()
            .into_iter()
            .map(|thread| thread.id.clone())
            .collect()
    }

    #[test]
    fn a_new_day_moves_the_date_separators() {
        let separators = |inbox: &Inbox| {
            inbox
                .items
                .iter()
                .filter_map(|item| {
                    if let Item::Separator(label) = item {
                        Some(label.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        let mut inbox = Inbox::new(now());
        // Three conversations today and three yesterday (days with fewer merge).
        inbox.set_threads(
            [0, 1, 2, 1440, 1441, 1442]
                .into_iter()
                .map(|minutes| thread(minutes, &now()))
                .collect(),
        );
        assert_eq!(separators(&inbox), ["Yesterday"]);
        inbox.set_now(
            now()
                .checked_add(jiff::SignedDuration::from_hours(24))
                .expect("in range"),
        );
        assert_eq!(
            separators(&inbox),
            ["November 16th"],
            "today's mail is yesterday's by now"
        );
    }

    #[test]
    fn the_list_ends_in_a_spinner_until_its_loaded_then_says_why_it_ends() {
        let view = |more, complete| View {
            title: "Starred".into(),
            more,
            complete,
            ..View::default()
        };
        let mut list = inbox(3);
        list.set_view(view(true, false));
        assert_eq!(list.bottom(), Bottom::Spinner, "more to load");
        list.set_online(false);
        assert_eq!(
            list.bottom(),
            Bottom::Status(EARLIER_OFFLINE),
            "offline, Gmail has the rest"
        );
        list.set_view(view(false, true));
        assert_eq!(list.bottom(), Bottom::None, "everything is here");
        let mut empty = Inbox::new(now());
        empty.set_view(view(true, false));
        empty.set_online(false);
        assert_eq!(empty.bottom(), Bottom::Status(EMPTY_OFFLINE));
        empty.set_online(true);
        empty.set_view(view(false, true));
        assert_eq!(empty.bottom(), Bottom::Status(EMPTY));
        empty.set_view(View {
            inbox: true,
            ..view(false, true)
        });
        assert_eq!(empty.bottom(), Bottom::None, "Inbox Zero says nothing");
        empty.set_view(View {
            loading: true,
            ..view(false, false)
        });
        assert_eq!(empty.bottom(), Bottom::Spinner, "still fetching");
    }

    #[test]
    fn with_a_draft_popped_out_the_status_line_goes_over_the_rows() {
        let view = |more, complete| View {
            title: "Muted".into(),
            more,
            complete,
            ..View::default()
        };
        let mut empty = Inbox::new(now());
        empty.set_popped_out(true);
        empty.set_view(view(false, true));
        assert_eq!(
            (empty.top_status(), empty.bottom()),
            (Some(NOTHING_FOUND), Bottom::None),
            "all here and empty"
        );
        let mut list = inbox(3);
        list.set_popped_out(true);
        list.set_view(view(true, false));
        assert_eq!(list.top_status(), None, "online, more to load");
        list.set_online(false);
        assert_eq!(
            (list.top_status(), list.bottom()),
            (Some(PARTIAL_OFFLINE), Bottom::None),
            "offline, Gmail has the rest"
        );
        list.set_popped_out(false);
        assert_eq!(
            (list.top_status(), list.bottom()),
            (None, Bottom::Status(EARLIER_OFFLINE)),
            "back under the rows"
        );
    }

    #[test]
    fn a_cursor_whose_row_goes_moves_to_the_row_below_it() {
        let mut inbox = inbox(4);
        inbox.move_cursor(1);
        inbox.set_threads([0, 2, 3].into_iter().map(|i| thread(i, &now())).collect());
        assert_eq!(cursor(&inbox), Some("t2"), "the row that was below");
        inbox.move_cursor(1);
        inbox.set_threads([0, 2].into_iter().map(|i| thread(i, &now())).collect());
        assert_eq!(cursor(&inbox), Some("t2"), "the last row's goes up");
        inbox.move_cursor(1);
        inbox.move_cursor(1);
        assert_eq!(cursor(&inbox), Some("t2"), "J goes on from there");
    }

    #[test]
    fn a_search_results_files_page_by_what_fits_and_stop_at_the_end() {
        let file = |name: &str| exo_store::Attachment {
            message_id: "m".into(),
            attachment_id: None,
            part_id: "1".into(),
            filename: name.into(),
            mime_type: "application/pdf".into(),
            size: 1,
            content_id: None,
            inline: false,
        };
        let mut inbox = inbox(2);
        let mut threads = inbox.threads().to_vec();
        threads[0].files = (0..6).map(|i| file(&format!("f{i}.pdf"))).collect();
        inbox.set_view(View {
            search: Some(SearchView::default()),
            ..View::default()
        });
        inbox.set_threads(threads);
        assert_eq!(inbox.row_height(&inbox.threads()[0]), ROW_WITH_FILES);
        assert_eq!(
            inbox.row_height(&inbox.threads()[1]),
            ROW_HEIGHT,
            "no files"
        );
        // A 1280px window's list: 835.55px of strip, three cards past the subject column's lead.
        let geometry = row::Geometry::new(903.8, 13.5, false);
        inbox.page_files("t0", 1, &geometry);
        let (index, scrolled) = inbox.file_pages["t0"];
        let max = 0.225 * 835.55 + 6.0 * row::CARD_STEP - 8.0 - 835.55;
        assert!(
            (scrolled - max).abs() < 0.01 && index == 3,
            "the end, at most: {index} {scrolled}"
        );
        inbox.page_files("t0", -1, &geometry);
        assert_eq!(inbox.file_pages["t0"], (0, 0.0), "back to the start");
    }

    #[test]
    fn crossing_towards_the_right_pane_isnt_a_hover() {
        let area = Rect::from_min_size(pos2(46.0, 117.0), vec2(873.0, 683.0));
        let heading = |to: Pos2| {
            let mut heading = Heading::default();
            assert!(
                !heading.towards_right_edge(pos2(300.0, 300.0), 0.0, area),
                "first sight"
            );
            heading.towards_right_edge(to, 0.016, area)
        };
        assert!(heading(pos2(340.0, 302.0)), "right, slightly down");
        assert!(!heading(pos2(301.0, 330.0)), "down the list");
        assert!(!heading(pos2(260.0, 302.0)), "left");
        assert!(!heading(pos2(300.0, 300.0)), "still");
    }

    #[test]
    fn the_cursor_stops_at_either_end() {
        let mut inbox = inbox(3);
        assert_eq!(cursor(&inbox), Some("t0"));
        inbox.move_cursor(-1);
        assert_eq!(cursor(&inbox), Some("t0"));
        inbox.move_cursor(5);
        assert_eq!(cursor(&inbox), Some("t2"));
        inbox.cursor_to_top();
        assert_eq!(cursor(&inbox), Some("t0"));
    }

    #[test]
    fn an_open_conversation_that_moves_up_advances_from_where_it_was() {
        let mut moved = inbox(5);
        moved.move_cursor(2);
        moved.set_reading(true);
        // A new message takes t2 to the top while it's open.
        moved.set_threads(
            [2, 0, 1, 3, 4]
                .into_iter()
                .map(|i| thread(i, &now()))
                .collect(),
        );
        assert_eq!(
            moved.take_advance_anchor(1, &["t3".into()]),
            None,
            "not to a conversation leaving with it"
        );
        assert_eq!(
            moved.take_advance_anchor(-1, &[]),
            Some("t1".into()),
            "up: the one that was above"
        );
        assert_eq!(moved.take_advance_anchor(1, &[]), None, "used once");
        // Only while reading, and not for a conversation that kept its place.
        let mut listed = inbox(5);
        listed.move_cursor(2);
        listed.set_threads(
            [2, 0, 1, 3, 4]
                .into_iter()
                .map(|i| thread(i, &now()))
                .collect(),
        );
        assert_eq!(listed.take_advance_anchor(1, &[]), None);
        let mut kept = inbox(5);
        kept.move_cursor(2);
        kept.set_reading(true);
        kept.set_threads((0..5).map(|i| thread(i, &now())).collect());
        assert_eq!(kept.take_advance_anchor(1, &[]), None);
    }

    #[test]
    fn dismissing_advances_down_then_up() {
        let mut inbox = inbox(3);
        inbox.dismiss(&ids(&inbox), 0.0);
        assert_eq!(cursor(&inbox), Some("t1"));
        inbox.cursor_to_bottom();
        inbox.dismiss(&ids(&inbox), 0.0);
        assert_eq!(cursor(&inbox), Some("t1"), "nothing below, so up");
        // Gone from the list once the slide-out ends.
        inbox.settle(1.0);
        assert_eq!(inbox.count(), 1);
    }

    #[test]
    fn actions_take_the_selection_first() {
        let mut inbox = inbox(4);
        inbox.toggle_selected();
        inbox.move_cursor(2);
        inbox.toggle_selected();
        assert_eq!(ids(&inbox), ["t0", "t2"]);
        inbox.dismiss(&ids(&inbox), 0.0);
        assert!(inbox.selection().is_empty());
        assert_eq!(cursor(&inbox), Some("t3"));
    }

    #[test]
    fn shift_selects_a_range_from_the_anchor() {
        let mut inbox = inbox(5);
        inbox.move_cursor(1);
        inbox.toggle_selected();
        inbox.select_range("t3");
        assert_eq!(inbox.selection(), ["t1", "t2", "t3"]);
    }

    #[test]
    fn shift_j_extends_and_shift_k_shrinks() {
        let mut inbox = inbox(5);
        inbox.extend_selection(1);
        inbox.extend_selection(1);
        assert_eq!(inbox.selection(), ["t0", "t1", "t2"]);
        inbox.extend_selection(-1);
        assert_eq!(inbox.selection(), ["t0", "t1"]);
        assert_eq!(cursor(&inbox), Some("t1"));
    }

    #[test]
    fn select_all_from_here_takes_the_rest() {
        let mut inbox = inbox(4);
        inbox.move_cursor(2);
        inbox.select_all_from_here();
        assert_eq!(inbox.selection(), ["t2", "t3"]);
    }

    #[test]
    fn selections_leave_out_rows_on_their_way_out() {
        let mut inbox = inbox(4);
        inbox.dismiss(&["t0".into(), "t2".into()], 0.0);
        inbox.select_all();
        assert_eq!(inbox.selection(), ["t1", "t3"], "⌘⇧A");
        inbox.clear_selection();
        inbox.select_all_from_here();
        assert_eq!(inbox.selection(), ["t1", "t3"], "⌘A");
        inbox.clear_selection();
        inbox.toggle("t1");
        inbox.select_range("t3");
        assert_eq!(inbox.selection(), ["t1", "t3"], "shift-click");
    }

    #[test]
    fn a_select_all_takes_rows_that_load_later_but_not_the_ones_unselected() {
        let mut inbox = inbox(4);
        inbox.set_view(View {
            more: true,
            ..View::default()
        });
        inbox.move_cursor(1);
        inbox.select_all_from_here();
        assert_eq!(inbox.selection(), ["t1", "t2", "t3"]);
        assert!(
            !inbox.is_exact(),
            "rows past the loaded ones are selected too"
        );
        inbox.move_cursor(1);
        inbox.toggle_selected();
        assert_eq!(inbox.selection(), ["t1", "t3"]);
        assert_eq!(inbox.unselected(), ["t0", "t2"]);
        // More rows load: they're older than the cutoff, so selected.
        inbox.set_threads((0..6).map(|i| thread(i, &now())).collect());
        assert_eq!(inbox.selection(), ["t1", "t3", "t4", "t5"]);
        inbox.set_view(View {
            more: false,
            ..View::default()
        });
        assert!(inbox.is_exact(), "every row is loaded");
    }

    #[test]
    fn the_select_all_hint_counts_what_is_selected() {
        let mut inbox = inbox(4);
        let count = |threads, exact| View {
            title: "Important".into(),
            more: true,
            count: Some(exo_store::ListCount {
                threads,
                unread: 0,
                exact,
            }),
            ..View::default()
        };
        inbox.set_view(count(1240, false));
        inbox.select_all();
        assert_eq!(
            inbox.select_all_hint("Important").as_deref(),
            Some("Selected all conversations in Important.")
        );
        inbox.toggle("t2");
        assert_eq!(
            inbox.select_all_hint("Important").as_deref(),
            Some("Selected 1,239+ conversations in Important.")
        );
        inbox.clear_selection();
        inbox.move_cursor(1);
        inbox.select_all_from_here();
        assert_eq!(
            inbox.select_all_hint("Important").as_deref(),
            Some("Selected \"Subject 1\" and all earlier conversations in Important.")
        );
        assert_eq!(inbox.take_hint(), None);
        inbox.select_all_from_here();
        assert_eq!(
            inbox.take_hint(),
            Some(SelectionHint::ChangedYourMind),
            "⌘A again on the same row"
        );
        inbox.clear_selection();
        assert_eq!(inbox.select_all_hint("Important"), None);
    }

    #[test]
    fn select_all_clears_when_everything_is() {
        let mut inbox = inbox(3);
        inbox.select_all();
        assert_eq!(inbox.selection(), ["t0", "t1", "t2"]);
        inbox.select_all();
        assert!(!inbox.is_selecting());
    }

    #[test]
    fn the_fourth_pick_hints_at_select_all() {
        let mut inbox = inbox(5);
        for _ in 0..3 {
            inbox.toggle_selected();
            inbox.move_cursor(1);
        }
        assert_eq!(inbox.take_hint(), None);
        inbox.toggle_selected();
        assert_eq!(inbox.take_hint(), Some(SelectionHint::SelectAll));
    }

    #[test]
    fn a_leaving_row_outlives_the_list_that_dropped_it() {
        let mut inbox = inbox(3);
        inbox.dismiss(&["t1".into()], 0.0);
        inbox.set_threads(vec![thread(0, &now()), thread(2, &now())]);
        assert_eq!(inbox.count(), 3, "still sliding out");
        assert_eq!(inbox.threads()[1].id, "t1", "in its place");
        inbox.settle(1.0);
        assert_eq!(inbox.count(), 2);
    }

    #[test]
    fn undo_brings_a_leaving_row_back_and_the_cursor_with_it() {
        let mut inbox = inbox(2);
        inbox.dismiss(&["t0".into()], 0.0);
        assert_eq!(cursor(&inbox), Some("t1"));
        inbox.restore(&["t0".into()], Some("t0".into()), 0.1);
        inbox.settle(1.0);
        assert_eq!(inbox.count(), 2);
        assert_eq!(cursor(&inbox), Some("t0"));
    }

    #[test]
    fn an_undone_thread_gets_the_cursor_when_it_comes_back() {
        let mut inbox = inbox(2);
        inbox.dismiss(&["t0".into()], 0.0);
        inbox.set_threads(vec![thread(1, &now())]);
        inbox.settle(1.0);
        inbox.restore(&["t0".into()], Some("t0".into()), 2.0);
        assert_eq!(cursor(&inbox), Some("t1"));
        inbox.set_threads(vec![thread(0, &now()), thread(1, &now())]);
        assert_eq!(cursor(&inbox), Some("t0"));
    }
}
