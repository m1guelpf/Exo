//! Quick Tips: with the setting on, the status bar, when no other bar is up, teaches the keys of
//! whatever has the keyboard, and its cross turns it off. It hides at Inbox Zero and while
//! sharing availability.

use exo_commands::Region;
use exo_store::ListId;
use exo_ui::TipPart::{self, Dot, Keys, Keyword, Text};

/// What a tip line is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    /// The inbox, Important, Other or a split.
    Inbox,
    /// The Snippets folder.
    Snippets,
    /// Any other list.
    List,
    /// Search results.
    SearchResults,
    /// The search bar.
    SearchBar,
    /// A conversation.
    Thread,
    /// A draft: a new message or a reply.
    Draft,
    /// A snippet being written.
    Snippet,
    /// The week view.
    Week,
    /// A new event.
    CreateEvent,
    /// The command palette.
    Palette,
    /// The palette over the week view.
    PaletteInWeek,
    /// The palette over a new event.
    PaletteInEvent,
    /// The palette picking a time: Remind me and Send later.
    PaletteTime,
    /// An empty bar, without its cross.
    None,
}

impl Context {
    /// The bar's line.
    pub fn parts(self) -> &'static [TipPart<'static>] {
        match self {
            Self::Inbox => &[
                Text("Hit"),
                Keys(&["E"]),
                Text("to Mark Done"),
                Dot,
                Keys(&["H"]),
                Text("to set a reminder"),
                Dot,
                Keys(&["C"]),
                Text("to compose"),
                Dot,
                Keys(&["/"]),
                Text("to search"),
                Dot,
                Keys(&["⌘", "K"]),
                Text("for Exo Command"),
            ],
            Self::Snippets => &[
                Text("Hit"),
                Keys(&["J"]),
                Text("to go down"),
                Dot,
                Keys(&["K"]),
                Text("to go up"),
                Dot,
                Keys(&["C"]),
                Text("to create a Snippet"),
                Dot,
                Keys(&["/"]),
                Text("to search"),
            ],
            Self::List => &[
                Text("Hit"),
                Keys(&["J"]),
                Text("to go down"),
                Dot,
                Keys(&["K"]),
                Text("to go up"),
                Dot,
                Keys(&["/"]),
                Text("to search"),
            ],
            Self::SearchResults => &[
                Text("Hit"),
                Keys(&["J"]),
                Text("to go down"),
                Dot,
                Keys(&["K"]),
                Text("to go up"),
                Dot,
                Keys(&["/"]),
                Text("to search"),
                Dot,
                Keys(&["enter"]),
                Text("to open a thread"),
            ],
            Self::SearchBar => &[
                Text("Hit"),
                Keys(&["tab"]),
                Text("to complete"),
                Dot,
                Keys(&["enter"]),
                Text("to search"),
            ],
            Self::Thread => &[
                Text("Hit"),
                Keys(&["E"]),
                Text("to Mark Done"),
                Dot,
                Keys(&["H"]),
                Text("to set a reminder"),
                Dot,
                Keys(&["Enter"]),
                Text("to reply all"),
                Dot,
                Keys(&["⌘", "K"]),
                Text("for Exo Command"),
            ],
            Self::Draft => &[
                Text("Hit"),
                Keys(&["⌘", "enter"]),
                Text("to send"),
                Dot,
                Keys(&["⌘", "shift", ","]),
                Text("to discard"),
            ],
            Self::Snippet => &[
                Text("Hit"),
                Keys(&["⌘", "enter"]),
                Text("to save"),
                Dot,
                Keys(&["⌘", "shift", ","]),
                Text("to discard"),
            ],
            Self::Week => &[
                Text("Hit"),
                Keys(&["="]),
                Text("for next week"),
                Dot,
                Keys(&["-"]),
                Text("for previous week"),
                Dot,
                Keys(&["T"]),
                Text("for today"),
                Dot,
                Keys(&["B"]),
                Text("to create an event"),
            ],
            Self::CreateEvent => &[
                Text("Hit"),
                Keys(&["⌘", "enter"]),
                Text("to save event"),
                Dot,
                Keys(&["⌘", "shift", ","]),
                Text("to cancel event"),
                Dot,
                Keys(&["⌘", "K"]),
                Text("to add Zoom links, descriptions, and more"),
            ],
            Self::Palette => &[
                Text("Try:"),
                Keyword("search"),
                Dot,
                Keyword("remind me"),
                Dot,
                Keyword("sent"),
            ],
            Self::PaletteInWeek => &[
                Text("Try:"),
                Keyword("create event"),
                Dot,
                Keyword("calendar settings"),
                Dot,
                Keyword("search"),
            ],
            Self::PaletteInEvent => &[
                Text("Try:"),
                Keyword("zoom link"),
                Dot,
                Keyword("location"),
                Dot,
                Keyword("description"),
            ],
            Self::PaletteTime => &[
                Text("Try:"),
                Keyword("8 am"),
                Dot,
                Keyword("3 days"),
                Dot,
                Keyword("aug 7"),
            ],
            Self::None => &[],
        }
    }
}

impl crate::Mail {
    /// What the Quick Tips bar is about, while it shows: the setting's on, and it isn't Inbox
    /// Zero or Share Availability. Only when no other bar shows does it.
    pub fn quick_tips(&self) -> Option<Context> {
        let calendar = self.calendar_region();
        let sharing = matches!(
            calendar,
            Some(
                Region::ShareAvailability
                    | Region::ShareAvailabilitySidebarAttendees
                    | Region::ShareAvailabilitySidebarTimezoneInput
            )
        );
        if !self.account.settings().quick_tips.show || self.inbox.at_inbox_zero() || sharing {
            return None;
        }
        let draft = self.compose.focused();
        Some(match self.commands.region() {
            Region::Command => match self.commands.palette_id() {
                Some("COMMAND_PALETTE") if calendar == Some(Region::CalendarCreate) => {
                    Context::PaletteInEvent
                }
                Some("COMMAND_PALETTE") if self.calendar_week().is_some() => Context::PaletteInWeek,
                Some("COMMAND_PALETTE") => Context::Palette,
                Some("SNOOZE" | "SCHEDULE") => Context::PaletteTime,
                _ => Context::None,
            },
            Region::ThreadList if self.search.is_some() => Context::SearchResults,
            Region::ThreadList if self.snippets.folder.is_open() => Context::Snippets,
            Region::ThreadList
                if matches!(
                    self.list,
                    ListId::Inbox | ListId::Important | ListId::Other | ListId::Split(_)
                ) =>
            {
                Context::Inbox
            }
            Region::ThreadList => Context::List,
            Region::ComposeForm | Region::PopOutComposeForm | Region::RecipientList
                if draft.is_some_and(crate::compose::Open::is_snippet) =>
            {
                Context::Snippet
            }
            Region::ComposeForm | Region::PopOutComposeForm | Region::RecipientList => {
                Context::Draft
            }
            Region::ThreadMessages | Region::ThreadMessageCalendarInvite | Region::ActionPane => {
                Context::Thread
            }
            Region::SearchBar => Context::SearchBar,
            Region::SeeYourWeek
            | Region::SeeYourWeekSidebar
            | Region::SeeYourWeekSidebarMeetWithInput => Context::Week,
            Region::CalendarCreate => Context::CreateEvent,
            _ => Context::None,
        })
    }
}
