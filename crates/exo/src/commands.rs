//! The thread list's commands, with their keys, regions, palette titles, aliases and scores,
//! registered every frame for what's on screen, and the toast copy for what they do.
//!
//! Only what works end to end is registered: no AI. Compose, search and the list filters
//! register their own (`compose::register_shortcuts`, `search::register`,
//! `filter::register`).

use exo_commands::{CONVERSATION, Command, Commands, Region};
use exo_store::{Label, ListId, Mutation};

use crate::{
    account::UndoToken,
    lists::{self, GoTo},
    mailbox::ThreadSummary,
};

/// Where list navigation and selection keys live.
const THREAD_LIST: &[Region] = &[Region::ThreadList];
/// Where `g` sequences work: the list and thread regions and the label switcher.
const GO_TO: &[Region] = &[
    Region::ThreadListHeader,
    Region::ThreadList,
    Region::ActionPane,
    Region::ThreadMessagesHeader,
    Region::ThreadMessages,
    Region::ThreadMessageCalendarInvite,
    Region::ReadOnlySnippet,
    Region::LabelSwitcher,
];

/// Where the thread view's message keys live.
const THREAD_MESSAGES: &[Region] = &[Region::ThreadMessages];
/// Where the arrows, N and P, `g g`, ⇧G, Enter and ⌘C work: the messages and the Show New
/// Messages bar.
const MESSAGES_AND_NEW: &[Region] = &[Region::ThreadMessages, Region::ThreadMessagesNewMessages];
/// Where ⇧N works.
const THREAD_PANE: &[Region] = &[
    Region::ThreadMessagesHeader,
    Region::ActionPane,
    Region::ThreadMessages,
    Region::ThreadMessageCalendarInvite,
    Region::ThreadMessagesNewMessages,
];
/// Where Escape leaves the thread.
const THREAD_BACK: &[Region] = &[
    Region::ThreadMessages,
    Region::ActionPane,
    Region::ReadOnlySnippet,
];
/// Where J/K move between conversations without leaving the thread (`ActionPane`).
const NEXT_CONVERSATION: &[Region] = &[
    Region::ActionPane,
    Region::ThreadMessagesHeader,
    Region::ThreadMessages,
    Region::ThreadMessageCalendarInvite,
    Region::ReadOnlySnippet,
];

/// A key in the list's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderKey {
    /// ↓, J, Escape: back to the list.
    Down,
    /// ← and →: the previous or next item.
    Move(isize),
    /// Enter: what a click does.
    Enter,
    /// Backspace: the filter tab goes.
    Backspace,
}

/// What a command does, for the app to carry out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Move the cursor this many rows.
    Move(isize),
    /// ↑ in the list: up a row, or from the first one into the header (the search bar while
    /// searching).
    Up,
    /// Something in the list's header with the keyboard.
    Header(HeaderKey),
    /// Jump to the first row.
    Top,
    /// Jump to the last row.
    Bottom,
    /// Page down (1) or up (-1).
    Page(isize),
    /// Open the cursor's thread.
    Open,
    /// Select or unselect the cursor's thread.
    ToggleSelected,
    /// Extend the selection while moving this many rows.
    Extend(isize),
    /// Select every thread.
    SelectAll,
    /// Select the cursor's thread and those below it.
    SelectAllFromHere,
    /// Clear the selection.
    DeselectAll,
    /// Escape or Backspace: clear the selection, or go back. Escape goes back to where the back
    /// stack began, which for a list is the home list; Backspace
    /// goes back to the previous list.
    Back {
        /// Escape rather than Backspace.
        escape: bool,
    },
    /// Apply a mutation to the actionable threads.
    Apply(Mutation),
    /// Something the Move and Label pickers do (`crate::pickers`).
    Pick(crate::pickers::Action),
    /// Undo the mutation behind this toast.
    Undo(UndoToken),
    /// Show a list.
    Go(ListId),
    /// Open the palette on `go to ` (`g l`).
    GoToLabel,
    /// Hint that Mute is ⇧M (`m`).
    MuteHint,
    /// Escape in a thread: collapse the focused message, or go back to the list.
    ThreadBack,
    /// `g u`: back to the list from a thread.
    LeaveThread,
    /// J / K in a thread: the next (1) or previous (-1) conversation.
    NextConversation(isize),
    /// N / P: the next (1) or previous (-1) message.
    NextMessage(isize),
    /// ↓ / ↑: scroll within the focused message, then move on.
    Arrow(isize),
    /// ↓ / ↑ held: as [`Self::Arrow`], without leaving the messages.
    ArrowHeld(isize),
    /// ← in the messages: scrolls a widened column left, else the back gutter takes the keyboard.
    ThreadLeft,
    /// ← held: scrolls a widened column left, never leaving the messages.
    ThreadLeftHeld,
    /// → in the messages, pressed or held: scrolls a widened column right.
    ThreadRight,
    /// ⇧N: show the new messages, or else focus the last message; Enter on the bar.
    ShowNewMessages,
    /// A key in the thread's header.
    ThreadHeader(crate::thread::HeaderKey),
    /// → or Tab in the back gutter.
    ThreadGutter(crate::thread::GutterKey),
    /// Enter on a collapsed message: expand it.
    ExpandMessage,
    /// O: cycle the focused message through collapsed, expanded and quoted text shown.
    ToggleMessage,
    /// ⇧O: expand or collapse every message.
    ToggleAllMessages,
    /// ⇧H: show or hide the focused message's full header.
    ToggleHeader,
    /// Space / ⇧Space in a thread.
    PageThread(isize),
    /// The first (false) or last (true) message.
    JumpThread(bool),
    /// ⌘C: copy the selected text.
    Copy,
    /// ⌘C in the list: copy the cursor's conversation.
    CopyListed,
    /// Show the focused message's blocked remote images (`Show Images`).
    ShowImages,
    /// Something Unsubscribe does (`⌘U`, its picker).
    Unsubscribe(crate::unsubscribe::Action),
    /// Copy the focused message (`Copy Message Contents`).
    CopyMessage,
    /// Copy the open conversation (`Copy Conversation Contents`).
    CopyConversation,
    /// Copy the focused message's recipients (`Copy Recipients`).
    CopyRecipients,
    /// Copy the focused message's files, for pasting into a draft (`Copy Attachments`).
    CopyAttachments,
    /// Something the attachment viewer's keys do.
    Preview(crate::attachment_files::viewer::Action),
    /// Copy the thread's subject (`Copy Subject`).
    CopySubject,
    /// Print the open conversation (`Print`, ⌘P, File ▸ Print): the macOS print panel.
    Print,
    /// Save the open conversation as a PDF (`Save as PDF`).
    SaveAsPdf,
    /// ← in the list: open or close the folder drawer.
    ToggleFolders,
    /// A key in the folder drawer.
    Folders(crate::folders::Key),
    /// A settings command, row or button.
    Settings(crate::settings::Action),
    /// Tab / ⇧Tab: the next (1) or previous (-1) split tab.
    SplitTab(isize),
    /// Tab or ⇧Tab held: the tab highlight moves that way, when pressed and on each repeat,
    /// without the list changing.
    SplitTabHold(isize),
    /// …let go: the highlighted tab's list shows.
    SplitTabRelease,
    /// Something compose does.
    Compose(crate::compose::Action),
    /// Something about snippets.
    Snippets(crate::snippets::Action),
    /// Something search does.
    Search(crate::search::Action),
    /// Something a list filter does.
    Filter(crate::filter::Action),
    /// Something the calendar does.
    Calendar(crate::calendar::Action),
    /// Something Get Me To Zero or Bulk Actions does.
    Bulk(crate::bulk::Action),
    /// Something Remind me does.
    Remind(crate::remind::Action),
    /// Something Out of Office Reply does.
    OutOfOffice(crate::out_of_office::Action),
    /// Open a Google app as the account (the palette's Google apps rows).
    GoogleApp(crate::google_apps::App),
    /// Copy a link to where the page is, or to its conversation (`crate::links`).
    Links(crate::links::Action),
    /// Something Open Links & Attachments does (`⌘O`, its list).
    OpenLinks(crate::open_links::Action),
    /// Making a split from the palette (`New Split Inbox`, `Add to Split Inbox`, …).
    NewSplit(crate::new_split::Action),
    /// Something the raw source viewer does (`|`).
    RawSource(crate::raw_source::Action),
    /// Something about the device's accounts: switching, adding, signing out.
    Accounts(crate::accounts::Action),
    /// A palette RSVP row: answer the open conversation's invitation (`crate::rsvp`).
    Rsvp(crate::thread::invite::Response),
    /// `Version Info`, `Open Source Attribution`, and their alert's keys (`crate::notice`).
    Notice(crate::notice::Action),
    /// Account setup's button and `Welcome to Exo`'s action (`crate::first_run`).
    FirstRun(crate::first_run::Action),
    /// Something the Achievements dialog does.
    Achievements(crate::achievements::Action),
    /// Something about Inbox Zero's photo.
    Backdrop(crate::backdrop::Action),
}

/// The header's and the back gutter's keys (`ThreadMessagesHeader`, `ActionPane`).
fn header_keys(commands: &mut Commands<Action>) {
    use crate::thread::{GutterKey, HeaderKey};
    const HEADER: &[Region] = &[Region::ThreadMessagesHeader];
    const GUTTER: &[Region] = &[Region::ActionPane];
    let header = |id: &'static str, key: HeaderKey| {
        Command::shortcut(id, Action::ThreadHeader(key)).regions(HEADER)
    };
    commands.add(header("HEADER_ESCAPE", HeaderKey::Escape).key("esc"));
    commands.add(header("HEADER_UP", HeaderKey::Up).keys(["up", "p", "ctralt+k", "ctralt+p"]));
    commands
        .add(header("HEADER_DOWN", HeaderKey::Down).keys(["down", "n", "ctralt+j", "ctralt+n"]));
    commands.add(header("HEADER_LEFT", HeaderKey::Step(-1)).key("left"));
    commands.add(header("HEADER_RIGHT", HeaderKey::Step(1)).key("right"));
    commands.add(header("HEADER_ENTER", HeaderKey::Enter).key("enter"));
    // Enter goes back as the button does; ↑ and ↓ move between conversations.
    commands.add(
        Command::shortcut("ACTION_PANE_ENTER", Action::LeaveThread)
            .key("enter")
            .regions(GUTTER),
    );
    commands.add(
        Command::shortcut("ACTION_PANE_RIGHT", Action::ThreadGutter(GutterKey::Right))
            .keys(["right", "tab"])
            .regions(GUTTER),
    );
    commands.add(
        Command::shortcut("ACTION_PANE_UP", Action::NextConversation(-1))
            .keys(["up", "ctralt+k", "ctralt+p"])
            .regions(GUTTER),
    );
    commands.add(
        Command::shortcut("ACTION_PANE_DOWN", Action::NextConversation(1))
            .keys(["down", "ctralt+j", "ctralt+n"])
            .regions(GUTTER),
    );
}

/// What the thread view's commands depend on this frame.
pub struct ThreadContext {
    /// The focused message is collapsed, so Enter expands it.
    pub focus_collapsed: bool,
    /// The Show New Messages bar has the keyboard, so Enter shows them.
    pub new_messages_focused: bool,
    /// The focused message's remote images are blocked and it has some (`SHOW_IMAGES`).
    pub show_images: bool,
}

/// Registers the open thread's commands for this frame. Reply, forward and compose are compose's
/// (`compose::register_shortcuts`).
pub fn register_thread(commands: &mut Commands<Action>, cx: &ThreadContext) {
    commands.add(
        Command::shortcut("BACK", Action::ThreadBack)
            .keys(["esc", "backspace"])
            .regions(THREAD_BACK),
    );
    commands.add(
        Command::shortcut("GO_TO_LIST", Action::LeaveThread)
            .key("g u")
            .regions(GO_TO),
    );
    commands.add(
        Command::shortcut("NEXT_CONVERSATION", Action::NextConversation(1))
            .key("j")
            .regions(NEXT_CONVERSATION),
    );
    commands.add(
        Command::shortcut("PREVIOUS_CONVERSATION", Action::NextConversation(-1))
            .key("k")
            .regions(NEXT_CONVERSATION),
    );
    commands.add(
        Command::shortcut("NEXT_MESSAGE", Action::NextMessage(1))
            .keys(["n", "ctralt+n"])
            .repeat(Action::NextMessage(1))
            .regions(MESSAGES_AND_NEW),
    );
    commands.add(
        Command::shortcut("PREVIOUS_MESSAGE", Action::NextMessage(-1))
            .keys(["p", "ctralt+p"])
            .repeat(Action::NextMessage(-1))
            .regions(MESSAGES_AND_NEW),
    );
    commands.add(
        Command::shortcut("FOCUS_DOWN", Action::Arrow(1))
            .keys(["down", "ctralt+j"])
            .repeat(Action::ArrowHeld(1))
            .regions(MESSAGES_AND_NEW),
    );
    commands.add(
        Command::shortcut("FOCUS_UP", Action::Arrow(-1))
            .keys(["up", "ctralt+k"])
            .repeat(Action::ArrowHeld(-1))
            .regions(MESSAGES_AND_NEW),
    );
    commands.add(
        Command::shortcut("FOCUS_LEFT", Action::ThreadLeft)
            .key("left")
            .repeat(Action::ThreadLeftHeld)
            .regions(MESSAGES_AND_NEW),
    );
    commands.add(
        Command::shortcut("FOCUS_RIGHT", Action::ThreadRight)
            .key("right")
            .repeat(Action::ThreadRight)
            .regions(MESSAGES_AND_NEW),
    );
    if cx.focus_collapsed {
        commands.add(
            Command::shortcut("EXPAND_MESSAGE", Action::ExpandMessage)
                .key("enter")
                .regions(THREAD_MESSAGES),
        );
    }
    commands.add(
        Command::new(
            "EXPAND_COLLAPSE_HEADER",
            "Expand / collapse Message header",
            Action::ToggleHeader,
        )
        .key("shift+h")
        .key_code(["shift", "H"])
        .regions(THREAD_MESSAGES),
    );
    commands.add(
        Command::new(
            "EXPAND_COLLAPSE_CURRENT",
            "Expand / collapse current Message",
            Action::ToggleMessage,
        )
        .key("o")
        .key_code(["O"])
        .regions(THREAD_MESSAGES),
    );
    commands.add(
        Command::new(
            "EXPAND_COLLAPSE_ALL",
            "Expand / collapse all Messages",
            Action::ToggleAllMessages,
        )
        .key("shift+o")
        .key_code(["shift", "O"])
        .regions(THREAD_MESSAGES),
    );
    commands.add(
        Command::shortcut("PAGE_DOWN", Action::PageThread(1))
            .keys(["space", "pagedown"])
            .repeat(Action::PageThread(1))
            .regions(THREAD_MESSAGES),
    );
    commands.add(
        Command::shortcut("PAGE_UP", Action::PageThread(-1))
            .keys(["shift+space", "pageup"])
            .repeat(Action::PageThread(-1))
            .regions(THREAD_MESSAGES),
    );
    commands.add(
        Command::shortcut("JUMP_TO_TOP", Action::JumpThread(false))
            .keys(["mod+up", "home", "shift+p"])
            .regions(THREAD_MESSAGES),
    );
    commands.add(
        Command::shortcut("JUMP_TO_TOP_SEQUENCE", Action::JumpThread(false))
            .key("g g")
            .regions(MESSAGES_AND_NEW),
    );
    commands.add(
        Command::shortcut("JUMP_TO_BOTTOM", Action::JumpThread(true))
            .keys(["mod+down", "end"])
            .regions(THREAD_MESSAGES),
    );
    commands.add(
        Command::shortcut("JUMP_TO_BOTTOM_SHIFT", Action::JumpThread(true))
            .key("shift+g")
            .regions(MESSAGES_AND_NEW),
    );
    commands.add(
        Command::shortcut("SHOW_NEW_MESSAGES", Action::ShowNewMessages)
            .key("shift+n")
            .regions(THREAD_PANE),
    );
    if cx.new_messages_focused {
        commands.add(
            Command::shortcut("SHOW_NEW_MESSAGES_ENTER", Action::ShowNewMessages)
                .key("enter")
                .regions(&[Region::ThreadMessagesNewMessages]),
        );
    }
    commands.add(
        Command::shortcut("COPY", Action::Copy)
            .key("mod+c")
            .regions(MESSAGES_AND_NEW),
    );
    header_keys(commands);
    // Print and Save as PDF, in the thread view. ⌘P belongs in the File menu; until Exo's menu
    // has Print, the key is the row's.
    commands.add(
        Command::new("PRINT", "Print", Action::Print)
            .key("mod+p")
            .key_code(["⌘", "P"])
            .default_score(1.45)
            .icon("data/print"),
    );
    commands.add(
        Command::new("PRINT_TO_PDF", "Save as PDF", Action::SaveAsPdf)
            .aliases(["Print to PDF", "Save to PDF", "Export to PDF"])
            .default_score(1.45)
            .icon("data/download"),
    );
    if cx.show_images {
        register_show_images(commands);
    }
}

/// `Show Images`: the focused message's remote images, or the focused draft's, which the Images
/// setting blocks.
pub fn register_show_images(commands: &mut Commands<Action>) {
    commands.add(
        Command::new("SHOW_IMAGES", "Show Images", Action::ShowImages)
            .aliases(["Load Images", "See Images", "View Images"])
            .operative_word("show images")
            .follow("INSERT_IMAGE")
            .icon("data/insert-image"),
    );
}

/// What the commands depend on this frame.
pub struct Context<'a> {
    /// The list on screen.
    pub list: &'a ListId,
    /// The threads actions apply to: the selection, or the cursor's.
    pub threads: &'a [&'a ThreadSummary],
    /// The account's labels.
    pub labels: &'a [Label],
    /// Which copy rows apply.
    pub copy: CopyRows,
    /// Where `g i` goes: home.
    pub home: ListId,
    /// Where `g o` goes: Other, or the Inbox.
    pub other: ListId,
    /// Important and Other are off (`unifiedInbox`).
    pub unified: bool,
    /// The one actionable conversation's sender, if it isn't you.
    pub sender: Option<&'a crate::pickers::Sender>,
    /// Your addresses' domains.
    pub own_domains: &'a [String],
    /// The selection is exactly the loaded rows: Remove all Labels is
    /// only offered then.
    pub exact: bool,
}

/// The palette's copy rows that apply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CopyRows {
    /// A thread is open: Copy Conversation Contents.
    pub thread: bool,
    /// A message in it has focus: Copy Message Contents and Copy Recipients.
    pub message: bool,
    /// The thread has a subject: Copy Subject, in the list only once something is typed.
    pub subject: bool,
    /// The focused message has files besides its inline images: Copy Attachments.
    pub attachments: bool,
}

/// Registers the list's commands for this frame.
pub fn register(commands: &mut Commands<Action>, cx: &Context<'_>) {
    navigation(commands);
    if !cx.threads.is_empty() {
        conversation(commands, cx);
    }
    selection(commands);
    go_to(commands, cx);
}

fn navigation(commands: &mut Commands<Action>) {
    commands.add(
        Command::shortcut("NEXT", Action::Move(1))
            .keys(["j", "down", "n", "ctralt+j", "ctralt+n"])
            .repeat(Action::Move(1))
            .regions(THREAD_LIST),
    );
    // K and P stay in the list (`contained`); ↑ leaves it from the first row.
    commands.add(
        Command::shortcut("PREVIOUS", Action::Move(-1))
            .keys(["k", "p", "ctralt+k", "ctralt+p"])
            .repeat(Action::Move(-1))
            .regions(THREAD_LIST),
    );
    commands.add(
        Command::shortcut("PREVIOUS_UP", Action::Up)
            .key("up")
            .repeat(Action::Up)
            .regions(THREAD_LIST),
    );
    const HEADER: &[Region] = &[Region::ThreadListHeader];
    let header = |id: &'static str, key: HeaderKey| {
        Command::shortcut(id, Action::Header(key)).regions(HEADER)
    };
    commands.add(header("HEADER_DOWN", HeaderKey::Down).keys(["down", "j", "ctralt+j", "esc"]));
    commands.add(header("HEADER_ENTER", HeaderKey::Enter).key("enter"));
    commands.add(header("HEADER_BACKSPACE", HeaderKey::Backspace).key("backspace"));
    commands.add(
        header("HEADER_LEFT", HeaderKey::Move(-1))
            .key("left")
            .repeat(Action::Header(HeaderKey::Move(-1))),
    );
    commands.add(
        header("HEADER_RIGHT", HeaderKey::Move(1))
            .key("right")
            .repeat(Action::Header(HeaderKey::Move(1))),
    );
    commands.add(
        Command::shortcut("OPEN", Action::Open)
            .keys(["enter", "o"])
            .regions(THREAD_LIST),
    );
    commands.add(
        Command::shortcut("COPY_LISTED", Action::CopyListed)
            .key("mod+c")
            .regions(THREAD_LIST),
    );
    commands.add(
        Command::shortcut("JUMP_TO_TOP", Action::Top)
            .keys(["mod+up", "home", "g g"])
            .regions(THREAD_LIST),
    );
    commands.add(
        Command::shortcut("JUMP_TO_BOTTOM", Action::Bottom)
            .keys(["mod+down", "end", "shift+g"])
            .regions(THREAD_LIST),
    );
    commands.add(
        Command::shortcut("PAGE_DOWN", Action::Page(1))
            .keys(["space", "pagedown"])
            .repeat(Action::Page(1))
            .regions(THREAD_LIST),
    );
    commands.add(
        Command::shortcut("PAGE_UP", Action::Page(-1))
            .keys(["shift+space", "pageup"])
            .repeat(Action::Page(-1))
            .regions(THREAD_LIST),
    );
    commands.add(
        Command::shortcut("BACK", Action::Back { escape: true })
            .key("esc")
            .regions(THREAD_LIST),
    );
    commands.add(
        Command::shortcut("BACK_BACKSPACE", Action::Back { escape: false })
            .key("backspace")
            .regions(THREAD_LIST),
    );
    commands.add(
        Command::shortcut("TOGGLE_LABEL_SWITCHER", Action::ToggleFolders)
            .key("left")
            .regions(THREAD_LIST),
    );
}

/// Registers the folder drawer's keys for this frame, while it's open.
pub fn register_folders(commands: &mut Commands<Action>) {
    use crate::folders::Key;
    const LABEL_SWITCHER: &[Region] = &[Region::LabelSwitcher];
    let key = |id: &'static str, key: Key| {
        Command::shortcut(id, Action::Folders(key)).regions(LABEL_SWITCHER)
    };
    // Held, ↑ stops at the first folder like K does.
    commands.add(
        key("UP", Key::Up)
            .key("up")
            .repeat(Action::Folders(Key::UpWithin)),
    );
    commands.add(
        key("UP_WITHIN", Key::UpWithin)
            .keys(["k", "ctralt+k", "p", "ctralt+p"])
            .repeat(Action::Folders(Key::UpWithin)),
    );
    commands.add(
        key("DOWN", Key::Down)
            .keys(["down", "j", "ctralt+j", "n", "ctralt+n"])
            .repeat(Action::Folders(Key::Down)),
    );
    commands.add(key("OPEN", Key::Enter).key("enter"));
    commands.add(key("CLOSE", Key::Close).keys(["esc", "mod+right"]));
    commands.add(key("RIGHT", Key::Right).key("right"));
    commands.add(key("BOTTOM", Key::Bottom).keys(["mod+down", "end", "shift+j"]));
    commands.add(key("TOP", Key::Top).keys(["mod+up", "home", "shift+k"]));
    commands.add(
        key("PAGE_DOWN", Key::PageDown)
            .keys(["pagedown", "space"])
            .repeat(Action::Folders(Key::PageDown)),
    );
    commands.add(
        key("PAGE_UP", Key::PageUp)
            .keys(["pageup", "shift+space"])
            .repeat(Action::Folders(Key::PageUp)),
    );
}

fn selection(commands: &mut Commands<Action>) {
    commands.add(
        Command::shortcut("SELECT", Action::ToggleSelected)
            .key("x")
            .regions(THREAD_LIST),
    );
    commands.add(
        Command::shortcut("EXTEND_DOWN", Action::Extend(1))
            .keys(["shift+j", "shift+down"])
            .repeat(Action::Extend(1))
            .regions(THREAD_LIST),
    );
    commands.add(
        Command::shortcut("EXTEND_UP", Action::Extend(-1))
            .keys(["shift+k", "shift+up"])
            .repeat(Action::Extend(-1))
            .regions(THREAD_LIST),
    );
    commands.add(
        Command::shortcut("DESELECT_ALL", Action::DeselectAll)
            .key("mod+alt+a")
            .regions(THREAD_LIST),
    );
    commands.add(
        Command::new(
            "SELECT_ALL_FROM_HERE",
            "Select All From Here",
            Action::SelectAllFromHere,
        )
        .key("mod+a")
        .key_code(["⌘", "A"])
        .icon("data/select-all")
        .multiple()
        .regions(THREAD_LIST),
    );
    commands.add(
        Command::new("SELECT_ALL", "Select All", Action::SelectAll)
            .key("mod+shift+a")
            .key_code(["⌘", "shift", "A"])
            .follow("SELECT_ALL_FROM_HERE")
            .icon("data/select-all")
            .multiple()
            .regions(THREAD_LIST),
    );
}

/// The actions on the actionable threads, in the palette's order (provider defaults, label
/// commands, then Mark Done).
fn conversation(commands: &mut Commands<Action>, cx: &Context<'_>) {
    let all = |test: fn(&ThreadSummary) -> bool| cx.threads.iter().all(|thread| test(thread));
    let row =
        |id: &'static str, title: &'static str, action: Action, icon: &'static str, score: f64| {
            Command::new(id, title, action)
                .icon(icon)
                .default_score(score)
                .multiple()
                .regions(CONVERSATION)
        };

    let starred = all(|thread| thread.starred);
    commands.add(
        if starred {
            row(
                "STAR",
                "Unstar",
                Action::Apply(Mutation::Unstar),
                "data/star",
                9.0,
            )
            .aliases(["Remove Flag"])
        } else {
            row(
                "STAR",
                "Star",
                Action::Apply(Mutation::Star),
                "data/star",
                9.0,
            )
            .aliases(["Flag"])
        }
        .key("s"),
    );

    let trashed = all(|thread| thread.has_label("TRASH"));
    let trash_action = Action::Apply(if trashed {
        Mutation::Untrash
    } else {
        Mutation::Trash
    });
    let trash = if trashed {
        row(
            "TRASH",
            "Remove from Trash",
            trash_action.clone(),
            "data/discard",
            6.0,
        )
    } else {
        row("TRASH", "Trash", trash_action.clone(), "data/discard", 6.0).aliases(["Delete"])
    };
    commands.add(trash.keys(["#", "del"]));
    // On UK keyboards ⇧3 types `£`; `#` is the key by Enter, or ⌥3 on a Mac.
    commands.add(
        Command::shortcut("TRASH_UK", trash_action)
            .keys(["\\", "alt+3"])
            .when_typing("#")
            .regions(CONVERSATION),
    );

    // `!` opens the Mark Spam picker, which offers blocking, for a single sender, and reports
    // spam directly otherwise (`crate::pickers`).
    let spam = if all(|thread| thread.has_label("SPAM")) || *cx.list == ListId::Spam {
        row(
            "MARK_SPAM",
            "Mark not Spam",
            Action::Apply(Mutation::NotSpam),
            "data/spam",
            5.0,
        )
        .aliases(["Mark not Junk"])
    } else {
        row(
            "MARK_SPAM",
            "Mark Spam",
            Action::Pick(crate::pickers::Action::MarkSpam),
            "data/spam",
            5.0,
        )
        .aliases(["Mark Junk"])
    };
    commands.add(spam.key("!"));

    let read = if all(|thread| thread.unread) {
        row(
            "MARK_UNREAD",
            "Mark Read",
            Action::Apply(Mutation::MarkRead),
            "data/mark-unread",
            4.0,
        )
        .aliases(["Mark as Read", "Toggle Read"])
    } else {
        row(
            "MARK_UNREAD",
            "Mark Unread",
            Action::Apply(Mutation::MarkUnread),
            "data/mark-unread",
            4.5,
        )
        .aliases(["Mark as Unread", "Toggle Read"])
    };
    commands.add(read.key("u"));

    let muted = all(|thread| thread.muted);
    commands.add(
        if muted {
            row(
                "MUTE",
                "Unmute",
                Action::Apply(Mutation::Unmute),
                "data/mute",
                4.0,
            )
            .aliases(["Stop Ignoring", "Follow"])
        } else {
            row(
                "MUTE",
                "Mute",
                Action::Apply(Mutation::Mute),
                "data/mute",
                4.0,
            )
            .aliases(["Ignore", "Unfollow"])
        }
        .key("shift+m"),
    );
    commands.add(
        Command::shortcut("MUTE_INTENT", Action::MuteHint)
            .key("m")
            .regions(CONVERSATION),
    );
    unsubscribe(commands, cx);
    block(commands, cx);
    important_or_other(commands, cx);
    copy(commands, cx.copy);
    labels(commands, cx.exact);
    reminders(commands, cx);

    // `e` marks done and `⇧E` not done anywhere but Spam; the palette row, which toggles, isn't
    // offered for a draft not in a conversation yet, which stays in the inbox.
    if *cx.list == ListId::Spam {
        return;
    }
    commands.add(
        Command::shortcut("MARK_DONE_KEY", Action::Apply(Mutation::MarkDone))
            .key("e")
            .regions(CONVERSATION),
    );
    commands.add(
        Command::shortcut("MARK_NOT_DONE_KEY", Action::Apply(Mutation::MarkNotDone))
            .key("shift+e")
            .regions(CONVERSATION),
    );
    if !cx.threads.iter().any(|thread| thread.draft_only) {
        let row = if all(|thread| thread.done) {
            row(
                "MARK_DONE",
                "Mark not Done",
                Action::Apply(Mutation::MarkNotDone),
                "data/done",
                11.0,
            )
            .key_code(["shift", "E"])
            .aliases(["Unarchive", "Move to Inbox"])
        } else {
            row(
                "MARK_DONE",
                "Mark Done",
                Action::Apply(Mutation::MarkDone),
                "data/done",
                11.0,
            )
            .key_code(["E"])
            .aliases(["Archive"])
        };
        commands.add(row);
    }
}

/// `Unsubscribe` (`⌘U`) for one conversation, in the list and in a thread: a Gmail account always
/// has it, and with nothing to unsubscribe from it offers to block the sender.
fn unsubscribe(commands: &mut Commands<Action>, cx: &Context<'_>) {
    if let [thread] = cx.threads
        && !thread.draft_only
        && cx.exact
    {
        let action = Action::Unsubscribe(crate::unsubscribe::Action::Launch);
        commands.add(
            Command::new("UNSUBSCRIBE", "Unsubscribe", action)
                .key("mod+u")
                .key_code(["⌘", "U"])
                .icon("data/unsubscribe")
                .regions(CONVERSATION),
        );
    }
}

/// `Block Sender: {email}` and, unless it's a webmail domain or yours, `Block Domain: {domain}`
/// for one conversation with a sender.
fn block(commands: &mut Commands<Action>, cx: &Context<'_>) {
    let Some(sender) = cx.sender else { return };
    let row = |id: &'static str, title: String, from: &str, domain: bool| {
        let action = Action::Pick(crate::pickers::Action::Block {
            from: from.to_owned(),
            domain,
            spam: false,
        });
        Command::new(id, title, action)
            .icon("data/unsubscribe")
            .operative_word("block")
            .default_score(0.98)
            .regions(CONVERSATION)
    };
    commands.add(row(
        "BLOCK_SENDER",
        format!("Block Sender: {}", sender.email),
        &sender.email,
        false,
    ));
    if let Some(domain) = sender.domain.as_ref().filter(|domain| {
        !crate::format::names::is_popular_domain(domain) && !cx.own_domains.contains(domain)
    }) {
        commands.add(row(
            "BLOCK_DOMAIN",
            format!("Block Domain: {domain}"),
            domain,
            true,
        ));
    }
}

/// `Move to Important` and `Move to Other` for one conversation while Important and Other are
/// on, each but where the list on screen is that one.
fn important_or_other(commands: &mut Commands<Action>, cx: &Context<'_>) {
    let [thread] = cx.threads else { return };
    if cx.unified || thread.draft_only {
        return;
    }
    let row = |id: &'static str, title: &'static str, other: bool, alias: &'static str| {
        let action = Action::Pick(crate::pickers::Action::OpenImportantOrOther(other));
        Command::new(id, title, action)
            .icon("data/move")
            .aliases([alias])
            .operative_word("move")
            .follow("MOVE")
            .regions(CONVERSATION)
    };
    if *cx.list != ListId::Important {
        commands.add(row(
            "MOVE_TO_IMPORTANT",
            "Move to Important",
            false,
            "Mark as Important",
        ));
    }
    if *cx.list != ListId::Other {
        commands.add(row(
            "MOVE_TO_OTHER",
            "Move to Other",
            true,
            "Mark as not Important",
        ));
    }
}

/// Move, then the label commands, and the keys that take threads out of
/// the list on screen. Without an exact selection, `⇧Y` only says why it can't.
fn labels(commands: &mut Commands<Action>, exact: bool) {
    use crate::pickers::Action::{OpenLabel, OpenMove, RemoveAll, RemoveFromList};
    let row = |id: &'static str, title: &'static str, action, icon: &'static str, score: f64| {
        Command::new(id, title, Action::Pick(action))
            .icon(icon)
            .default_score(score)
            .multiple()
            .regions(CONVERSATION)
    };
    commands.add(
        row("MOVE", "Move", OpenMove, "data/move", 8.0)
            .key("v")
            .key_code(["V"])
            .aliases(["Folder", "File"])
            .operative_word("move"),
    );
    commands.add(if exact {
        row(
            "REMOVE_ALL_LABELS",
            "Remove all Labels",
            RemoveAll,
            "data/label",
            6.9,
        )
        .key("shift+y")
        .key_code(["shift", "Y"])
        .aliases([
            "Remove all Labels and Mark Done",
            "Remove all Labels and Archive",
        ])
    } else {
        Command::shortcut("REMOVE_ALL_LABELS", Action::Pick(RemoveAll))
            .key("shift+y")
            .regions(CONVERSATION)
    });
    commands.add(
        row("LABEL", "Label", OpenLabel, "data/label", 7.0)
            .key("l")
            .key_code(["L"])
            .aliases([
                "Add Label",
                "Remove Label",
                "Create Label",
                "New Label",
                "Tag",
                "Create Tag",
                "Remove Tag",
            ])
            .follow("CREATE_EVENT"),
    );
    commands.add(
        row(
            "REMOVE_FROM_LABEL",
            "Remove from Label",
            RemoveFromList(0),
            "data/label",
            7.0,
        )
        .key("y")
        .key_code(["Y"])
        .aliases(["Remove from List"]),
    );
    // `]` is -1 and `[` is 1: the previous and the next conversation.
    commands.add(
        Command::shortcut(
            "REMOVE_FROM_LABEL_PREVIOUS",
            Action::Pick(RemoveFromList(-1)),
        )
        .key("]")
        .regions(CONVERSATION),
    );
    commands.add(
        Command::shortcut("REMOVE_FROM_LABEL_NEXT", Action::Pick(RemoveFromList(1)))
            .key("[")
            .regions(CONVERSATION),
    );
}

/// `Remind Me` and, for threads with one, `Remove Reminder(s)`: not for drafts being written, and
/// only for an exact selection, where `H` says why it can't.
fn reminders(commands: &mut Commands<Action>, cx: &Context<'_>) {
    use crate::remind::Action::{Open, Remove};
    // ⌘⇧H is compose's own in the reply pane.
    const SHIFT_H: &[Region] = &[
        Region::ThreadList,
        Region::ThreadMessagesHeader,
        Region::ThreadMessages,
        Region::ThreadMessageCalendarInvite,
        Region::ThreadMessagesNewMessages,
    ];
    if cx.threads.iter().any(|thread| thread.draft_only) {
        return;
    }
    let open = Action::Remind(Open { button: false });
    commands.add(
        Command::shortcut("SNOOZE_SHIFT", open.clone())
            .key("mod+shift+h")
            .regions(SHIFT_H),
    );
    if !cx.exact {
        commands.add(
            Command::shortcut("SNOOZE", open)
                .key("h")
                .regions(CONVERSATION),
        );
        return;
    }
    let row = |id: &'static str, title: &'static str, action: Action, score: f64| {
        Command::new(id, title, action)
            .icon("data/snooze")
            .default_score(score)
            .scale(1.01)
            .multiple()
            .regions(CONVERSATION)
    };
    commands.add(
        row("SNOOZE", "Remind Me", open, 10.0)
            .key("h")
            .key_code(["H"])
            .aliases(["Snooze", "Boomerang", "Add Reminder"]),
    );
    let reminded = cx
        .threads
        .iter()
        .filter(|thread| thread.has_reminder())
        .count();
    if reminded > 0 {
        let title = if reminded > 1 {
            "Remove Reminders"
        } else {
            "Remove Reminder"
        };
        commands.add(row(
            "REMOVE_REMINDER",
            title,
            Action::Remind(Remove {
                threads: None,
                to_inbox: true,
            }),
            6.5,
        ));
    }
}

/// The copy rows, for one conversation at a time.
fn copy(commands: &mut Commands<Action>, rows: CopyRows) {
    let row = |id: &'static str, title: &'static str, action: Action, score: f64| {
        Command::new(id, title, action)
            .icon("data/copy")
            .default_score(score)
            .regions(CONVERSATION)
    };
    if rows.message {
        // Both copy the message; pasting it (⌘V) makes the new draft.
        commands.add(
            row(
                "COPY_MESSAGE_TO_DRAFT",
                "Copy Message to new Draft",
                Action::CopyMessage,
                4.3,
            )
            .key_code(["⌘", "C"]),
        );
        commands.add(
            row(
                "COPY_MESSAGE",
                "Copy Message Contents",
                Action::CopyMessage,
                4.2,
            )
            .key_code(["⌘", "C"]),
        );
    }
    if rows.thread {
        commands.add(
            row(
                "COPY_CONVERSATION",
                "Copy Conversation Contents",
                Action::CopyConversation,
                4.1,
            )
            .key_code(["⌘", "C"]),
        );
    }
    if rows.message {
        commands.add(row(
            "COPY_RECIPIENTS",
            "Copy Recipients",
            Action::CopyRecipients,
            4.05,
        ));
    }
    if rows.attachments {
        commands.add(row(
            "COPY_ATTACHMENTS",
            "Copy Attachments",
            Action::CopyAttachments,
            4.005,
        ));
    }
    if rows.subject {
        let subject = row("COPY_SUBJECT", "Copy Subject", Action::CopySubject, 4.01);
        commands.add(if rows.thread {
            subject
        } else {
            subject.query_only()
        });
    }
}

/// `go to …` rows for every view and label, and the `g` sequences for system views.
fn go_to(commands: &mut Commands<Action>, cx: &Context<'_>) {
    let highlighted = !cx.threads.is_empty();
    let go = |go: &GoTo| {
        let mut command = Command::new(
            go.id,
            format!("go to {}", go.name),
            Action::Go(go.list.clone()),
        )
        .keys(go.keys.iter().copied())
        .key_code(go.key_code.iter().copied())
        .operative_word(go.operative_word)
        .default_score(1.0)
        .scale(0.6)
        .icon("data/go-to")
        .multiple()
        .regions(GO_TO);
        if let Some(follow) = go.follow {
            command = command.follow(follow);
        }
        // With a conversation highlighted, an empty palette lists its actions instead.
        if highlighted {
            command.query_only()
        } else {
            command
        }
    };
    for system in lists::system_go_tos(cx.unified) {
        commands.add(go(&system));
    }
    for label in lists::listed_labels(cx.labels) {
        let command = Command::new(
            lists::slug(&label.name),
            format!("go to {}", label.name),
            Action::Go(ListId::Label(label.id.clone())),
        )
        .operative_word(label.name.clone())
        .default_score(1.0)
        .scale(0.5)
        .icon("data/go-to")
        .multiple()
        .regions(GO_TO);
        commands.add(if highlighted {
            command.query_only()
        } else {
            command
        });
    }
    for late in lists::late_go_tos() {
        commands.add(go(&late));
    }
    // `g i` goes home and `g o` to Other, the Inbox for either with the unified inbox.
    commands.add(
        Command::shortcut("GO_HOME", Action::Go(cx.home.clone()))
            .key("g i")
            .regions(GO_TO),
    );
    commands.add(
        Command::shortcut("GO_TO_OTHER", Action::Go(cx.other.clone()))
            .key("g o")
            .regions(GO_TO),
    );
    commands.add(
        Command::shortcut("GO_TO_LABEL", Action::GoToLabel)
            .keys(["g l", "g f"])
            .regions(GO_TO),
    );
}

/// What the toast says after `mutation`.
pub fn toast(mutation: &Mutation, labels: &[Label]) -> String {
    let label = |id: &str| lists::name(&ListId::Label(id.to_owned()), labels, &[]);
    match mutation {
        Mutation::MarkDone => "Marked as Done.".into(),
        Mutation::MarkNotDone => "Marked not Done.".into(),
        Mutation::Star => "Starred.".into(),
        Mutation::Unstar => "Unstarred.".into(),
        Mutation::MarkRead => "Marked as Read.".into(),
        Mutation::MarkUnread => "Marked as Unread.".into(),
        Mutation::Trash => "Moved to Trash.".into(),
        Mutation::Untrash => "Moved to Inbox.".into(),
        Mutation::Spam => "Marked as Spam.".into(),
        Mutation::NotSpam => "Marked as Not Spam.".into(),
        Mutation::Mute => "Muted.".into(),
        Mutation::Unmute => "Unmuted.".into(),
        Mutation::AddLabel(id) => format!("Added to label {}.", label(id)),
        Mutation::RemoveLabel(id) => format!("Removed from label {}.", label(id)),
        Mutation::RemoveLabels(_) => "Removed all labels.".into(),
        Mutation::Move { from, to } => crate::pickers::move_toast(from.as_deref(), to, labels),
        // Remind me's toasts say when, which only the picker knows (`remind::toast`).
        Mutation::CancelReminder { .. } => "Reminder removed.".into(),
        Mutation::Remind { .. } | Mutation::ReturnFromReminder => String::new(),
    }
}

/// What the error toast says when `mutation` (or its undo) failed, for those with copy of their
/// own.
pub fn failure(mutation: &Mutation, undo: bool) -> Option<&'static str> {
    Some(match (mutation, undo) {
        (Mutation::MarkDone, false) => "Sorry! Failed to mark as Done.",
        (Mutation::MarkNotDone, false) => "Sorry! Failed to mark not Done.",
        (Mutation::Trash, false) => "Sorry! Failed to move to Trash.",
        (Mutation::Untrash, false) => "Sorry! Failed to move to Inbox.",
        (Mutation::Star, false) => "Sorry! Failed to star.",
        (Mutation::Unstar, false) => "Sorry! Failed to unstar.",
        (Mutation::Star, true) => "Sorry! Failed to undo star.",
        (Mutation::Unstar, true) => "Sorry! Failed to undo unstar.",
        (Mutation::Mute, false) => "Sorry! Failed to mute the conversation.",
        (Mutation::Unmute, false) => "Sorry! Failed to Unmute the conversation.",
        (Mutation::Spam | Mutation::NotSpam, false) => "Sorry! Failed to mark as Spam.",
        (Mutation::AddLabel(_), false) => "Sorry! Failed to add label.",
        (Mutation::RemoveLabel(_), false) => "Sorry! Failed to remove from label.",
        (Mutation::AddLabel(_), true) => "Sorry! Failed to undo add label.",
        (Mutation::RemoveLabel(_), true) => "Sorry! Failed to undo remove from label.",
        (Mutation::RemoveLabels(_), false) => "Sorry! Failed to remove labels.",
        (Mutation::RemoveLabels(_), true) => "Sorry! Failed to undo remove labels.",
        (Mutation::Move { .. }, _) => "Sorry! Failed to move conversation.",
        (
            Mutation::MarkRead
            | Mutation::MarkUnread
            | Mutation::Mute
            | Mutation::Unmute
            | Mutation::Spam
            | Mutation::NotSpam,
            true,
        ) => "Sorry! Failed to undo.",
        _ => return None,
    })
}
