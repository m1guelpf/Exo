//! Focus regions: which part of the UI owns the keyboard.
//!
//! There is no central keymap. Every binding is registered for a set of regions and only fires
//! while one of them is the active region. Typing in an input is safe because input regions only
//! bind chords, Escape, Tab and arrows.

/// A focus region, for the surfaces Exo has or will have. Exo has no AI, team or CRM surfaces, so
/// there are no regions for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[expect(
    missing_docs,
    reason = "each variant is named for the surface that has the keyboard"
)]
pub enum Region {
    ThreadList,
    ThreadListHeader,
    ActionPane,
    LabelSwitcher,
    SearchBar,
    ThreadMessages,
    ThreadMessagesHeader,
    ThreadMessagesNewMessages,
    ThreadMessageCalendarInvite,
    ReadOnlySnippet,
    ThreadSourceViewer,
    AttachmentPreview,
    ComposeForm,
    PopOutComposeForm,
    RecipientList,
    TextFormatterLinkInput,
    TextFormatterColor,
    /// The ⌘K palette.
    Command,
    /// The shortcuts sheet.
    ShortcutsHelp,
    Dialog,
    Alert,
    AccountSetup,
    /// The first-run flow's discovery questions.
    FtueDiscoveryQuestions,
    Dropdown,
    SeeYourWeek,
    SeeYourWeekSidebar,
    SeeYourWeekSidebarMeetWithInput,
    /// A calendar's colour picker: the week sidebar's `…`, the Notifications dialog's swatches.
    CalendarColorPicker,
    CalendarCreate,
    ShareAvailability,
    ShareAvailabilitySidebarAttendees,
    ShareAvailabilitySidebarTimezoneInput,
}

impl Region {
    /// Overlays take the keyboard temporarily and give it back to the primary region.
    pub const fn is_overlay(self) -> bool {
        matches!(
            self,
            Self::Command | Self::ShortcutsHelp | Self::Dialog | Self::Alert | Self::Dropdown
        )
    }

    /// Regions whose focus is a text field. There, held keys repeat (the field needs them) instead
    /// of being swallowed.
    pub const fn is_text_input(self) -> bool {
        matches!(
            self,
            Self::SearchBar
                | Self::ComposeForm
                | Self::PopOutComposeForm
                | Self::RecipientList
                | Self::TextFormatterLinkInput
                | Self::Command
                | Self::ShortcutsHelp
                | Self::LabelSwitcher
                | Self::SeeYourWeekSidebarMeetWithInput
                | Self::CalendarCreate
                | Self::ShareAvailabilitySidebarAttendees
                | Self::ShareAvailabilitySidebarTimezoneInput
        )
    }
}

/// Where a registration is live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Everywhere.
    Global,
    /// While one of these regions is active.
    In(&'static [Region]),
}

impl Scope {
    /// Whether this scope includes `region`.
    pub fn contains(self, region: Region) -> bool {
        match self {
            Self::Global => true,
            Self::In(regions) => regions.contains(&region),
        }
    }
}

/// The list and thread regions where single-letter shortcuts live.
pub const LIST_AND_THREAD: &[Region] = &[
    Region::ThreadListHeader,
    Region::ThreadList,
    Region::ActionPane,
    Region::ThreadMessagesHeader,
    Region::ThreadMessages,
    Region::ThreadMessageCalendarInvite,
    Region::ReadOnlySnippet,
];

/// Where conversation actions (`e`, `s`, `#`, …) work: the list and an open thread.
pub const CONVERSATION: &[Region] = &[
    Region::ThreadList,
    Region::ThreadMessagesHeader,
    Region::ThreadMessages,
    Region::ThreadMessageCalendarInvite,
    Region::ThreadMessagesNewMessages,
    Region::ActionPane,
];

/// Where ⌘K opens the palette: everywhere but the shortcuts sheet, dialogs, alerts and dropdowns.
pub(crate) const PALETTE_TOGGLE: &[Region] = &[
    Region::ThreadListHeader,
    Region::ThreadList,
    Region::ActionPane,
    Region::ThreadMessagesHeader,
    Region::ThreadMessages,
    Region::ThreadMessageCalendarInvite,
    Region::ReadOnlySnippet,
    Region::SearchBar,
    Region::ComposeForm,
    Region::PopOutComposeForm,
    Region::RecipientList,
    Region::TextFormatterLinkInput,
    Region::Command,
    Region::LabelSwitcher,
    Region::SeeYourWeek,
    Region::SeeYourWeekSidebar,
    Region::SeeYourWeekSidebarMeetWithInput,
    Region::CalendarCreate,
];

/// Where `?` toggles the shortcuts sheet.
pub(crate) const SHORTCUTS_TOGGLE: &[Region] = &[
    Region::ThreadListHeader,
    Region::ThreadList,
    Region::ThreadMessagesHeader,
    Region::ThreadMessages,
    Region::ThreadMessageCalendarInvite,
    Region::ShortcutsHelp,
];

/// Where `z` / ⌘Z run the toast's Undo: list and thread regions, read-only snippets and the week
/// view.
pub(crate) const TOAST_KEYS: &[Region] = &[
    Region::ThreadListHeader,
    Region::ThreadList,
    Region::ActionPane,
    Region::ThreadMessagesHeader,
    Region::ThreadMessages,
    Region::ThreadMessageCalendarInvite,
    Region::ReadOnlySnippet,
    Region::SeeYourWeek,
    Region::SeeYourWeekSidebar,
];
