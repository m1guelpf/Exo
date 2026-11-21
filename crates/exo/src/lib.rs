#![deny(
    clippy::correctness,
    clippy::suspicious,
    clippy::complexity,
    clippy::perf
)]
//! Exo: a local, Gmail-direct desktop email client.

pub mod account;
pub mod accounts;
pub mod achievements;
mod app;
mod attachment_files;
mod backdrop;
mod body;
mod bulk;
mod calendar;
mod chrome;
pub mod clipboard;
mod clock;
mod command_nudge;
mod commands;
mod compose;
mod contact_pane;
mod contact_picker;
mod copy;
mod deep_links;
pub mod desktop;
pub mod downloads;
mod email_actions;
mod emoji;
mod filter;
pub mod find;
mod first_run;
mod folders;
mod footer_sync;
pub mod format;
mod google_apps;
mod history;
pub mod inbox;
mod link_preview;
pub mod links;
mod lists;
pub mod log_in;
pub mod logging;
pub mod mail_client;
pub mod mailbox;
mod new_split;
pub mod notice;
pub mod notifications;
mod open_links;
pub mod out_of_office;
mod page_menu;
mod photo;
mod pickers;
pub mod power;
pub mod print;
pub mod profile_pictures;
pub mod quick_tips;
mod raw_source;
mod reachability;
pub mod recovery;
mod remind;
mod remote_content;
mod row_menu;
mod rsvp;
mod search;
mod senders;
mod settings;
mod shell;
mod shortcuts_pane;
pub mod sign_in;
mod signed_out;
mod snippets;
mod spelling;
mod splits;
pub mod status_bar;
mod tabs;
pub mod teardown;
pub mod thread;
pub mod time_suggest;
mod touch_bar_tip;
mod unsubscribe;
pub mod update;
mod welcome;
pub mod zoom;

pub use app::{App, LaunchError, Screen, Start};
pub use calendar::places;
pub use clock::Clock;
pub use log_in::LogIn;
pub use signed_out::SignedOut;
pub use teardown::Unopened;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use egui::{Rect, pos2};
use exo_commands::{Commands, Hint, Region, Toast, ToastKind};
use exo_store::{ListId, Mutation};
use exo_ui::motion::{self, CubicBezier};
use jiff::Zoned;

use crate::{
    account::{Account, Event, Snapshot, UndoToken},
    body::Bodies,
    commands::Action,
    contact_pane::ContactAction,
    format::names::Person,
    inbox::{CursorSource, Inbox, View},
    mailbox::{ContactCard, ThreadSummary},
    thread::ThreadView,
};

/// The contact pane follows a row the pointer moved the cursor to once the cursor has stayed on
/// it this long, a trailing debounce on the row the pointer entered.
pub(crate) const SETTLED_HOVER: f64 = 0.1;
/// ⌘C again within this long copies the conversation: the copy hint's 10 s.
const COPY_AGAIN: f64 = 10.0;
/// How high toasts sit at Inbox Zero, unless the streak animation hides the chrome.
const INBOX_ZERO_TOAST_BOTTOM: f32 = 68.0;
/// How high toasts sit over the select-all hint.
const SELECT_ALL_HINT_TOAST_BOTTOM: f32 = 72.0;
/// How high toasts sit over the status bar.
const STATUS_BAR_TOAST_BOTTOM: f32 = 64.0;

/// Where Exo's images are (their `images` folder): `Exo.app`'s
/// `Contents/Resources/assets` (`fastlane build`), else the checkout's `assets`. (Exo's fonts and
/// icons are compiled in.)
pub fn assets_dir() -> PathBuf {
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("Resources/assets")));
    bundled
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets")))
}

/// The window's options, at its place for a first launch; [`desktop::Placement`] moves it to
/// where it was.
pub fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Exo")
            .with_inner_size([1280.0, 800.0])
            .with_position([22.0, 22.0])
            .with_min_inner_size([780.0, 300.0])
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            // Exo hides the native traffic lights and draws its own.
            .with_titlebar_buttons_shown(false),
        ..Default::default()
    }
}

/// The threads an action applies to: in a thread, that thread alone,
/// as long as it's still listed (the list's selection only counts in the list); in the list, the
/// selection, or else the cursor's thread.
fn actionable<'a>(thread: Option<&ThreadView>, inbox: &'a Inbox) -> Vec<&'a ThreadSummary> {
    match thread {
        Some(open) => inbox
            .threads()
            .iter()
            .filter(|row| row.id == open.id())
            .collect(),
        None => inbox.actionable(),
    }
}

/// Which of the palette's copy rows apply: in a thread, its conversation, focused message and
/// subject; in the list, the subject of the one conversation actions apply to.
fn copy_rows(thread: Option<&ThreadView>, actionable: &[&ThreadSummary]) -> commands::CopyRows {
    match thread {
        Some(thread) => commands::CopyRows {
            thread: true,
            message: thread.focused().is_some(),
            subject: thread
                .detail()
                .is_some_and(|detail| !detail.subject.is_empty()),
            attachments: thread
                .detail()
                .and_then(|detail| {
                    detail
                        .messages
                        .iter()
                        .find(|message| Some(message.id.as_str()) == thread.focused())
                })
                .is_some_and(|message| attachment_files::has_copied(&message.attachments)),
        },
        None => commands::CopyRows {
            subject: matches!(actionable, [one] if one.subject != "-"),
            ..commands::CopyRows::default()
        },
    }
}

/// A message's id and its invite's `.ics` bytes.
type IcsFile = (String, Vec<u8>);

/// What an undo puts back: the threads, and where the cursor was and on which list.
struct Undoable {
    token: UndoToken,
    list: ListId,
    threads: Vec<String>,
    cursor: Option<String>,
    /// The action ran in the open thread, which undo reopens.
    in_thread: bool,
    /// It was a move, whose undo says so.
    moved: bool,
}

/// A thread an action may take out of the list without the store having said so beforehand
/// (a move, removing labels): its row slides out when the first snapshot showing the action
/// drops it.
struct Departure {
    thread: String,
    /// The action's [`Account::apply`] number.
    applied: u64,
    list: ListId,
    /// Where the cursor goes from a row that leaves: 1 down, -1 up (`[` and `]`), 0 auto.
    direction: isize,
}

/// An account's mail: the shell, the thread list and the open thread.
pub struct Mail {
    account: Account,
    inbox: Inbox,
    commands: Commands<Action>,
    /// The list on screen.
    list: ListId,
    /// Where back and forward go (⌘[, ⌘], Backspace).
    history: history::History,
    /// The Touch Bar's tip, while it's up.
    touch_bar_tip: Option<touch_bar_tip::Tip>,
    /// The welcome splash, the account's first time on this Mac.
    welcome: Option<welcome::Welcome>,
    /// The first-run flow after it: account setup and what ends it.
    first_run: first_run::State,
    /// `Version Info` or `Update Required`, while it's up.
    notice: Option<notice::Notice>,
    /// The updater, whose check's alerts show here.
    updates: Option<update::Updates>,
    /// The palette's first-run nudge.
    command_nudge: command_nudge::State,
    /// The account's latest snapshot.
    snapshot: Arc<Snapshot>,
    /// The snapshot the list's rows came from; none since the list was switched.
    rows_from: Option<Arc<Snapshot>>,
    /// How many rows the list had when it last asked for more: it asks again once they arrive.
    asked_more_at: Option<usize>,
    /// The newest toast's undo.
    undoable: Option<Undoable>,
    /// Rows that may be on their way out.
    departing: Vec<Departure>,
    /// The Move and Label pickers' alerts and label creation.
    pickers: pickers::Pickers,
    /// Get Me To Zero and Bulk Actions.
    bulk_dialogs: bulk::Dialogs,
    /// Unsubscribe's picker and what it's waiting for.
    unsubscribing: unsubscribe::State,
    /// Out of Office Reply's dialog, alerts and polls.
    out_of_office: out_of_office::OutOfOffice,
    /// Open Links & Attachments' list.
    open_links: open_links::OpenLinks,
    /// ⌘C in the list, while the conversation is read and rendered.
    copy_listed: copy::Listed,
    /// Making splits from the palette, and the New Split dialog.
    new_split: new_split::NewSplit,
    /// The raw source viewer.
    raw_source: raw_source::RawSource,
    /// The address whose conversations the account was last asked for.
    pane_contact: Option<String>,
    /// The thread whose contact the right pane shows.
    pane_thread: Option<String>,
    /// The contact a Mail row opened its thread with, or a draft's chip,
    /// which the pane keeps showing until the focus moves, and what had it then: the thread's
    /// focused message, or in the list the cursor's thread.
    pane_pin: Option<(Person, Option<String>)>,
    /// Whom the contact pane showed this frame, for `Compose to {email}`.
    pane_shown: Option<Person>,
    /// The folder drawer (`LabelSwitcher`).
    folders: folders::Folders,
    /// A row's right-click menu to show next frame, at this point.
    row_menu: Option<egui::Pos2>,
    /// The tab Tab or ⇧Tab has reached while held, whose list shows when it's let go.
    split_highlight: Option<ListId>,
    /// A split just made that's hidden when empty, until its list first empties.
    hidden_split: Option<String>,
    /// The list was at Inbox Zero last frame.
    was_zero: bool,
    /// The day's photo behind Inbox Zero.
    backdrop: backdrop::Backdrop,
    /// The achievements' popups, streak animation and dialog.
    achievements: achievements::State,
    /// Shows a context menu and returns the item chosen ([`desktop::context_menu::show`]); the
    /// account's other pages made from this one share it.
    context_menu: std::rc::Rc<std::cell::RefCell<Box<desktop::context_menu::Show>>>,
    /// A right-click in a message body, whose menu shows next frame.
    message_menu: Option<page_menu::Pending>,
    /// Makes a draft body's spellchecker (macOS's, [`desktop::spelling::engine`]).
    spelling: std::rc::Rc<dyn Fn() -> Option<Box<dyn exo_editor::Spelling>>>,
    /// Whether that spellchecker corrects spelling automatically, autocorrect's default
    /// ([`desktop::spelling::corrects_automatically`]).
    corrects_automatically: fn() -> bool,
    /// The message ⌘C last copied and when: ⌘C on it again copies the conversation.
    copied: Option<(String, f64)>,
    /// The cursor's thread and since when, for the contact pane's settle delay.
    cursor_since: (Option<String>, f64),
    /// The settings menu and dialogs.
    settings: settings::Settings,
    /// The open thread, drawn over the list.
    thread: Option<ThreadView>,
    /// Message bodies, rendered off the UI thread.
    bodies: Bodies,
    /// Invite `.ics` files arriving for the open thread, by message.
    invites: (
        std::sync::mpsc::Sender<IcsFile>,
        std::sync::mpsc::Receiver<IcsFile>,
    ),
    /// Where the invite photos are.
    images: PathBuf,
    link_preview: link_preview::LinkPreview,
    /// Clicked attachments being fetched and previewed.
    attachment_files: attachment_files::AttachmentFiles,
    /// Drafts being written and sends being held.
    compose: compose::Compose,
    /// The Snippets folder, and the ⌘; picker's origin.
    snippets: snippets::Snippets,
    /// The HTML of the message ⌘C last copied, and its fields: pasting it outside a text field
    /// makes a draft of it.
    copied_message: Option<(String, compose::Fields)>,
    /// Search mode, over the list on screen.
    search: Option<search::Search>,
    /// The search results the list's rows came from.
    search_rows_from: Option<Arc<account::SearchResults>>,
    /// The list's cursor when search opened, which it gets back after.
    search_cursor: Option<String>,
    /// The calendar's day view.
    calendar: calendar::Calendar,
    /// The filter narrowing the list.
    filter: Option<exo_store::Filter>,
    /// Each list's last filter, which it opens with again.
    last_filters: std::collections::HashMap<ListId, exo_store::Filter>,
    /// The device's accounts, in the user's order ([`Mail::set_accounts`]).
    accounts: Vec<exo_auth::Account>,
    /// Signing out is waiting for the account to count what Gmail doesn't have yet.
    signing_out: bool,
    /// What this frame asks of the app, returned by [`Mail::frame`].
    request: Option<accounts::Request>,
    /// The footer's sync line.
    footer: footer_sync::FooterSync,
    /// What it says this frame.
    footer_line: Option<String>,
    clock: Clock,
    /// The clock's time this frame.
    now: Zoned,
    ctx: egui::Context,
}

impl Drop for Mail {
    fn drop(&mut self) {
        // Before the account goes, which sends what it holds.
        self.save_drafts();
    }
}

impl Mail {
    /// `account`'s inbox in `ctx` (set up by [`App::new`]) with the images in `assets`, dated
    /// relative to `clock`.
    pub fn new(ctx: &egui::Context, assets: &Path, account: Account, clock: Clock) -> Self {
        let now = clock.now();
        account.set_clock(clock.clone());
        // The account's theme from the start, and for what may replace its mail before a frame.
        settings::apply_theme(ctx, account.settings().theme);
        let snapshot = account.snapshot();
        // The app opens on home: Important while Important and Other are on.
        let home = splits::home(account.settings());
        if home != ListId::Inbox {
            account.show(home.clone());
        }
        let backdrop = backdrop::Backdrop::new(ctx, &now);
        let mut mail = Self {
            bodies: Bodies::new(ctx),
            thread: None,
            link_preview: link_preview::LinkPreview::default(),
            attachment_files: attachment_files::AttachmentFiles::new(ctx, account.email()),
            compose: compose::Compose::default(),
            snippets: snippets::Snippets::default(),
            copied_message: None,
            search: None,
            search_rows_from: None,
            search_cursor: None,
            calendar: calendar::Calendar::default(),
            filter: None,
            last_filters: std::collections::HashMap::new(),
            invites: std::sync::mpsc::channel(),
            images: assets.join("images"),
            now: now.clone(),
            ctx: ctx.clone(),
            account,
            inbox: Inbox::new(now),
            commands: Commands::new(),
            list: home,
            history: history::History::default(),
            touch_bar_tip: None,
            welcome: None,
            first_run: first_run::State::default(),
            notice: None,
            updates: None,
            command_nudge: command_nudge::State::default(),
            snapshot,
            rows_from: None,
            asked_more_at: None,
            undoable: None,
            departing: Vec::new(),
            pickers: pickers::Pickers::default(),
            bulk_dialogs: bulk::Dialogs::default(),
            unsubscribing: unsubscribe::State::default(),
            out_of_office: out_of_office::OutOfOffice::default(),
            open_links: open_links::OpenLinks::default(),
            copy_listed: copy::Listed::default(),
            new_split: new_split::NewSplit::default(),
            raw_source: raw_source::RawSource::default(),
            pane_contact: None,
            pane_thread: None,
            pane_pin: None,
            pane_shown: None,
            folders: folders::Folders::default(),
            row_menu: None,
            split_highlight: None,
            hidden_split: None,
            was_zero: false,
            backdrop,
            achievements: achievements::State::default(),
            context_menu: std::rc::Rc::new(std::cell::RefCell::new(Box::new(
                desktop::context_menu::show,
            ))),
            message_menu: None,
            spelling: std::rc::Rc::new(desktop::spelling::engine),
            corrects_automatically: desktop::spelling::corrects_automatically,
            copied: None,
            cursor_since: (None, 0.0),
            settings: settings::Settings::new(ctx),
            accounts: Vec::new(),
            signing_out: false,
            request: None,
            footer: footer_sync::FooterSync::default(),
            footer_line: None,
            clock,
        };
        mail.set_split_library(splits::load_library());
        mail.resume_welcome();
        if mail.welcome.is_none() {
            mail.maybe_start_first_run();
        }
        // Until the app says otherwise, the device has this one account.
        mail.accounts = vec![exo_auth::Account {
            email: mail.account.email().to_owned(),
            ..exo_auth::Account::default()
        }];
        mail
    }

    /// Another page on the account, for another tab: a view of the account of
    /// its own, at home, set up as this one is (where previews go, the split library, the
    /// spellchecker and context menus, which tests give).
    pub(crate) fn another_page(&self, assets: &Path) -> Box<Self> {
        let mut page = Box::new(Self::new(
            &self.ctx,
            assets,
            self.account.view(),
            self.clock.clone(),
        ));
        page.attachment_files
            .set_cache(self.attachment_files.cache().to_owned());
        page.settings
            .set_split_library(self.settings.split_library().cloned());
        page.spelling = self.spelling.clone();
        page.corrects_automatically = self.corrects_automatically;
        page.context_menu = self.context_menu.clone();
        page.accounts = self.accounts.clone();
        page
    }

    /// Keeps attachment previews in `folder` rather than the temp directory's cache, which every
    /// copy of Exo running shares. Tests give each app its own.
    pub fn set_preview_cache(&mut self, folder: PathBuf) {
        self.attachment_files.set_cache(folder);
    }

    /// Uses `library` as the split library, as this account sees it. Tests and
    /// screenshots give a made-up one in place of the copy in the assets.
    pub fn set_split_library(&mut self, library: Option<exo_store::SplitLibrary>) {
        let library = library.map(|library| {
            library.for_account(
                &splits::team_domain(self.account.email())
                    .into_iter()
                    .collect::<Vec<_>>(),
            )
        });
        self.settings.set_split_library(library);
    }

    /// Checks drafts' spelling with what `make` makes, one for each body, instead of macOS's
    /// spellchecker: tests give a word list, and never change the user's dictionary. Autocorrect
    /// is off unless turned on in Exo, rather than following macOS.
    pub fn set_spelling(&mut self, make: impl Fn() -> Box<dyn exo_editor::Spelling> + 'static) {
        self.spelling = std::rc::Rc::new(move || Some(make()));
        self.corrects_automatically = || false;
    }

    /// Whether autocorrect is on for this account: as turned on or off in Exo, else as macOS
    /// corrects spelling.
    pub fn autocorrect(&self) -> bool {
        self.account
            .settings()
            .autocorrect((self.corrects_automatically)())
    }

    /// Turns autocorrect on or off, as Edit ▸ `Enable Autocorrect` does for every account.
    pub fn set_autocorrect(&mut self, on: bool) {
        self.account
            .update_settings(|settings| settings.typing_assistance.enabled = Some(on));
    }

    /// Shows context menus with `show` instead of AppKit's: tests choose their items.
    pub fn set_context_menu(
        &mut self,
        show: impl FnMut(&[desktop::context_menu::MenuItem], egui::Pos2) -> Option<usize> + 'static,
    ) {
        self.context_menu = std::rc::Rc::new(std::cell::RefCell::new(Box::new(show)));
    }

    /// The right-click menu of the list's rows at `at`, for the conversations it acts on, and
    /// what's chosen in it. Exo's shortcuts are off while it's open; AppKit's menu has the
    /// keyboard then anyway.
    fn show_row_menu(&mut self, at: egui::Pos2, now: f64) {
        let threads = self.inbox.actionable();
        if threads.is_empty() || self.thread.is_some() {
            return;
        }
        let entries = row_menu::entries(&threads, self.inbox.is_exact(), &self.list);
        let items: Vec<desktop::context_menu::MenuItem> =
            entries.iter().map(|entry| entry.item.clone()).collect();
        let chosen = (self.context_menu.borrow_mut())(&items, at)
            .and_then(|index| entries.into_iter().nth(index))
            .and_then(|entry| entry.action);
        if let Some(action) = chosen {
            self.run(action, now);
        }
    }

    /// Shows `Warn Before Quitting` in the settings menu for `warning`; with none, no row.
    pub fn set_quit_warning(&mut self, warning: Option<desktop::QuitWarning>) {
        self.settings.set_quit_warning(warning);
    }

    /// Takes the clock's time for this frame, which everything dated relative to now reads.
    fn set_now(&mut self, now: Zoned) {
        if now.timestamp().as_second() / 60 != self.now.timestamp().as_second() / 60 {
            self.account.check_reminders();
        }
        self.inbox.set_now(now.clone());
        if let Some(thread) = &mut self.thread {
            thread.set_now(now.clone());
        }
        self.now = now;
    }

    /// The account on screen.
    pub fn account(&self) -> &Account {
        &self.account
    }

    /// Stops the account without sending what it holds, for signing out
    /// ([`Account::discard`]).
    pub(crate) fn discard(&mut self) {
        self.account.discard();
    }

    /// Where attachment previews are kept.
    pub(crate) fn preview_cache(&self) -> &Path {
        self.attachment_files.cache()
    }

    /// The account was switched away from, and stays open off screen as it was left: its open
    /// drafts' last keystrokes are saved and go to Gmail now, not at their pace, and it asks for
    /// no more frames.
    pub(crate) fn leave_screen(&mut self) {
        self.save_drafts();
        self.account.write_drafts_now();
        self.account.set_on_screen(false);
    }

    /// The list on screen.
    pub fn list(&self) -> &ListId {
        &self.list
    }

    /// The thread list.
    pub fn inbox(&self) -> &Inbox {
        &self.inbox
    }

    /// The open thread, if any.
    pub fn thread(&self) -> Option<&ThreadView> {
        self.thread.as_ref()
    }

    /// The status bar at the bottom of the window, if one shows: the offline bar, else Out of
    /// Office Reply's.
    pub fn status_bar(&self) -> Option<status_bar::StatusBar> {
        status_bar::StatusBar::offline(&self.snapshot.connection, self.calendar_week().is_some())
            .or_else(|| {
                self.out_of_office()
                    .then(status_bar::StatusBar::out_of_office)
            })
    }

    /// What the footer's sync line says, as last drawn.
    pub fn footer_line(&self) -> Option<&str> {
        self.footer_line.as_deref()
    }

    /// What the Dock badge shows: the list count, from the account's latest counts, which come
    /// in while the window is hidden too.
    pub fn badge(&self) -> Option<String> {
        self.list_count(&self.account.snapshot().counts)
    }

    /// The count the tab title and the Dock badge show: the conversations the store has in the
    /// list on screen, of those in `counts`, which are the inbox views counted (the inbox and
    /// its splits). None in the Snippets folder and the week view, whose badge is the calendar's.
    fn list_count(&self, counts: &splits::Counts) -> Option<String> {
        let snippets = self.snippets.folder.is_open() && self.search.is_none();
        if snippets || self.calendar_week().is_some() {
            return None;
        }
        splits::count_label(counts.get(&self.list)?)
    }

    /// The draft with the keyboard, as its To addresses and subject (for tests).
    pub fn compose_focus(&self) -> Option<(Vec<String>, String)> {
        self.compose.focus_summary()
    }

    /// The draft with the keyboard's body, as its editor writes it (for tests).
    pub fn compose_body(&self) -> Option<String> {
        self.compose.focused().map(|open| open.editor.html())
    }

    /// Where the caret of the body with the keyboard was last drawn (for tests).
    pub fn compose_caret(&self) -> Option<egui::Rect> {
        self.compose.focused().map(|open| open.caret)
    }

    /// How many drafts are open in the open thread (for tests).
    pub fn thread_drafts(&self) -> usize {
        self.thread
            .as_ref()
            .map_or(0, |thread| self.compose.open_in(thread.id()))
    }

    /// The Send later time of the draft with the keyboard (for tests).
    pub fn compose_send_later(&self) -> Option<jiff::Timestamp> {
        self.compose
            .focused()
            .and_then(|open| open.record.send_later.as_ref())
            .map(|later| later.at)
    }

    /// The account, to change (for tests).
    pub fn account_mut(&mut self) -> &mut Account {
        &mut self.account
    }

    /// The reminder of the draft with the keyboard (for tests).
    pub fn compose_reminder(&self) -> Option<exo_store::DraftReminder> {
        self.compose.focused().and_then(|open| open.record.reminder)
    }

    /// The hint tooltip showing, its words and keycaps one space apart. For tests.
    pub fn hint_text(&self) -> Option<String> {
        self.commands.hint_text()
    }

    /// Who called off the scheduled sends of the drafts open, by a reply (for tests).
    pub fn compose_unscheduled_by(&self) -> Vec<String> {
        self.compose.unscheduled_by()
    }

    /// The open sub-palette's prompt and rows, if one is open. For tests.
    pub fn sub_palette_rows(&self) -> Option<(String, Vec<String>)> {
        self.commands.sub_palette_rows()
    }

    /// The toast showing, if any. For tests.
    pub fn toast_text(&self) -> Option<&str> {
        self.commands.toast_text()
    }

    /// The new-message pane is showing.
    pub fn compose_pane_open(&self) -> bool {
        self.compose.pane().is_some()
    }

    /// The folder drawer is open.
    pub fn folders_open(&self) -> bool {
        self.folders.is_open()
    }

    /// Whether the Snippets folder is the list (for tests).
    pub fn snippets_folder_open(&self) -> bool {
        self.snippets.folder.is_open()
    }

    /// The snippet open read-only, if any (for tests).
    pub fn snippet_viewed(&self) -> Option<&str> {
        self.snippets.folder.viewing()
    }

    /// The Snippets folder's highlighted snippet, if any (for tests).
    pub fn snippet_cursor(&self) -> Option<&str> {
        self.snippets.folder.cursor()
    }

    /// Whether a draft is popped out.
    pub fn compose_popped_out(&self) -> bool {
        self.compose.popped().is_some()
    }

    /// The address of whom the contact pane showed last frame.
    pub fn contact_pane_email(&self) -> Option<&str> {
        self.pane_shown.as_ref().map(|person| person.email.as_str())
    }

    /// The settings menu is in the right pane.
    pub fn settings_visible(&self) -> bool {
        self.settings.visible()
    }

    /// A settings dialog is open.
    pub fn settings_dialog_open(&self) -> bool {
        self.settings.dialog_open()
    }

    /// The toast showing, if any (for tests).
    pub fn toast(&self) -> Option<&str> {
        self.commands.toast_text()
    }

    /// Nothing is in flight: the account is idle and every body on screen is laid out and
    /// painted. For screenshots and tests.
    pub fn is_idle(&self) -> bool {
        self.account.is_idle()
            && self.bodies.is_idle()
            && self.snippets.is_idle()
            && self.backdrop.is_loaded()
            && self.welcome.as_ref().is_none_or(welcome::Welcome::is_idle)
            && self.first_run_idle()
    }

    /// The achievement popups' titles, oldest first. For tests.
    pub fn achievement_popups(&self) -> Vec<String> {
        self.achievements.popups()
    }

    /// The Achievements dialog's view, while it's open. For tests.
    pub fn achievements_dialog(&self) -> Option<achievements::View> {
        self.achievements.dialog()
    }

    /// Shows the photo `file` at Inbox Zero, whatever the day, for screenshots. False if there's
    /// no such photo.
    pub fn use_photo(&mut self, file: &str) -> bool {
        backdrop::Backdrop::named(&self.ctx, file, &self.now)
            .map(|backdrop| self.backdrop = backdrop)
            .is_some()
    }

    /// Opens thread `id` (the list cursor's), marking it read without a toast once its messages
    /// are in: the unread ones open expanded.
    fn open_thread(&mut self, id: String) {
        if self.snippet_unsaved() {
            self.ask_leaving_snippet(Box::new(move |mail| mail.open_thread(id)));
            return;
        }
        // A new message being written opens in its pane (its row is only a draft).
        let draft_only = self
            .inbox
            .threads()
            .iter()
            .any(|thread| thread.id == id && thread.draft_only);
        if draft_only && self.open_draft_thread(&id) {
            self.close_thread();
            return;
        }
        self.leave_pane();
        if self.thread.as_ref().is_some_and(|thread| thread.id() != id) {
            self.close_thread();
        }
        let unread = self
            .inbox
            .threads()
            .iter()
            .any(|thread| thread.id == id && thread.unread);
        self.search_opened_thread();
        self.close_snippet_view();
        self.account.open_thread(Some(&id));
        let mut view = ThreadView::new(
            id,
            self.now.clone(),
            self.account.email().to_owned(),
            self.images.clone(),
        );
        if unread {
            view.set_read_on_open();
        }
        self.thread = Some(view);
    }

    /// Back to the list, its cursor where it was. The thread's bodies and their textures go.
    fn close_thread(&mut self) {
        self.pane_pin = None;
        if let Some(id) = self.thread.as_ref().map(|thread| thread.id().to_owned()) {
            self.compose_close_thread(&id);
        }
        if self.thread.take().is_some() {
            self.account.open_thread(None);
            self.bodies
                .set_thread(&[], self.account.email(), &self.account.loader());
        }
    }

    /// The next (1) or previous (-1) conversation from inside a thread:
    /// opens it, or goes back to the list past either end.
    fn next_conversation(&mut self, delta: isize) {
        let before = self.inbox.cursor().map(|thread| thread.id.clone());
        // Where the conversation was, if it has moved up the list since it opened.
        if let Some(anchored) = self.inbox.take_advance_anchor(delta, &[]) {
            self.inbox.point_at(&anchored);
            self.open_thread(anchored);
            return;
        }
        self.inbox.move_cursor(delta);
        match self.inbox.cursor().map(|thread| thread.id.clone()) {
            Some(id) if Some(&id) != before.as_ref() => {
                self.dismiss_command_nudge();
                self.open_thread(id);
            }
            _ => {
                self.close_thread();
                self.command_nudge_left_thread();
            }
        }
    }

    /// Takes the newest snapshot of the list on screen into the list view.
    fn sync_snapshot(&mut self) {
        self.snapshot = self.account.snapshot();
        let current = self.snapshot.list == self.list
            && self.snapshot.filter == self.filter
            && self.search.is_none();
        if current
            && !self
                .rows_from
                .as_ref()
                .is_some_and(|taken| Arc::ptr_eq(taken, &self.snapshot))
        {
            self.inbox
                .set_reading(self.thread.is_some() && self.search.is_none());
            self.dismiss_departed();
            self.inbox.set_threads(self.snapshot.threads.clone());
            self.rows_from = Some(self.snapshot.clone());
        }
        let inbox = lists::is_inbox(&self.list);
        let settings = self.account.settings();
        let mut tabs = if inbox {
            splits::tabs(settings, &self.list, &self.snapshot.counts)
        } else {
            Vec::new()
        };
        splits::describe(&mut tabs, settings, self.settings.split_library());
        self.inbox.set_view(View {
            title: lists::name(&self.list, &self.snapshot.labels, settings.splits()).to_owned(),
            // While Tab is held, the tab it has reached.
            selected: tabs
                .iter()
                .position(|tab| tab.list == *self.split_highlight.as_ref().unwrap_or(&self.list))
                .unwrap_or_default(),
            tabs,
            add_split: splits::shows_add_split(settings, &self.list, self.joined(), &self.now),
            inbox,
            more: current && self.snapshot.more,
            loading: !current || self.snapshot.loading,
            complete: current && self.snapshot.complete,
            archivable: self.list != ListId::Spam,
            filter: self.filter.map(filter::name),
            count: self.snapshot.count.filter(|_| current),
            search: None,
            unsegmented: self.list == ListId::Scheduled,
            // A split (or Other) that shows while empty, until the nudge is done with.
            nudge: !settings.nudge_completed(inbox::HIDE_EMPTY_SPLIT)
                && match &self.list {
                    ListId::Split(id) => settings
                        .split(id)
                        .is_some_and(|split| !split.hides_when_empty()),
                    ListId::Other => !settings.hides_other_when_empty(),
                    _ => false,
                },
            streak: settings.statistics.inbox_zero_streak_week_current,
            hide_chrome: self
                .achievements
                .hides_chrome(self.ctx.input(|input| input.time)),
            settings_open: self.settings.visible(),
        });
        self.search_sync();
    }

    /// Keeps the contact pane on the cursor's thread: at once after a command, and once the
    /// cursor has stayed on a hovered row for [`SETTLED_HOVER`], however the pointer moves
    /// within the row.
    fn follow_cursor(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);
        let cursor = self.inbox.cursor().map(|thread| thread.id.clone());
        if cursor != self.cursor_since.0 {
            self.cursor_since = (cursor.clone(), now);
        }
        if cursor == self.pane_thread {
            return;
        }
        let rested = now - self.cursor_since.1;
        if self.inbox.cursor_source() == CursorSource::Command || rested >= SETTLED_HOVER {
            // Its bodies, if they aren't stored, so opening it shows them at once.
            if let Some(id) = &cursor {
                self.account.prefetch(id);
            }
            self.pane_thread = cursor;
            // The right pane moving to another conversation hides the settings menu.
            self.settings.hide();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(SETTLED_HOVER - rested));
        }
    }

    /// Whom the contact pane shows: for a new message, its last recipient or you; a draft's
    /// selected chip or new recipient, until the focus moves; in a thread, the focused message's
    /// sender (the thread's contact when that's you), unless a Mail row opened it; in the list,
    /// the contact of the thread the cursor rests on.
    fn pane_person(&mut self) -> Option<Person> {
        // A new message: the recipient last added in the compose form, else you.
        if let Some(recipient) = self.compose.pane_recipient() {
            return Some(match recipient {
                Some(address) => Person::new(
                    address.name.clone().unwrap_or_default(),
                    address.email.clone(),
                ),
                None => {
                    let name = self
                        .snapshot
                        .aliases
                        .iter()
                        .find(|alias| alias.primary)
                        .and_then(|alias| alias.name.clone());
                    Person::new(name.unwrap_or_default(), self.account.email())
                }
            });
        }
        // A reply's or a popped-out draft's selected chip, or the recipient it just got, shows
        // until the focus moves (a thread's focused message, else the list's cursor), as a Mail
        // row's contact does; a chip without an address shows you.
        let focused = match &self.thread {
            Some(thread) => thread.focused().map(str::to_owned),
            None => self.pane_thread.clone(),
        };
        if let Some(chip) = self.compose.shown_recipient() {
            let person = if chip.email.is_empty() {
                Person::new(String::new(), self.account.email())
            } else {
                Person::new(chip.name.clone().unwrap_or_default(), chip.email.clone())
            };
            self.pane_pin = Some((person, focused.clone()));
        }
        match &mut self.pane_pin {
            Some((_, pinned)) if pinned.is_none() => *pinned = focused,
            Some((_, pinned)) if *pinned != focused => self.pane_pin = None,
            _ => {}
        }
        if let Some((person, _)) = &self.pane_pin {
            return Some(person.clone());
        }
        let listed = |id: &str| {
            self.inbox
                .threads()
                .iter()
                .find(|thread| thread.id == id)
                .and_then(|thread| thread.contact.clone())
        };
        let Some(thread) = &self.thread else {
            if let Some(person) = self.search_pane_person() {
                return person;
            }
            return listed(self.pane_thread.as_deref()?);
        };
        thread.focused_sender().or_else(|| listed(thread.id()))
    }

    /// The contact card for the pane. The account is told whenever the pane's person changes,
    /// and its next snapshot (a frame or two later) carries their conversations from local
    /// mail; until then the Mail section is empty rather than someone else's.
    fn pane_card(&mut self) -> Option<ContactCard> {
        let person = self.pane_person()?;
        let key = person.email.to_lowercase();
        let is_me = key.eq_ignore_ascii_case(self.account.email());
        if !is_me && self.pane_contact.as_deref() != Some(key.as_str()) {
            self.account.show_contact(&key);
            self.pane_contact = Some(key.clone());
        }
        let recent = match &self.snapshot.contact {
            Some((email, threads)) if *email == key => threads.clone(),
            _ => Vec::new(),
        };
        Some(ContactCard { person, recent })
    }

    /// ⌘C with nothing selected in a thread:
    /// the focused message, or the conversation when it's ⌘C again on the same message while
    /// the first copy's hint could still show.
    fn copy_message_or_conversation(&mut self, now: f64) {
        let Some(focused) = self
            .thread
            .as_ref()
            .and_then(ThreadView::focused)
            .map(str::to_owned)
        else {
            return;
        };
        let again = self
            .copied
            .take()
            .is_some_and(|(id, at)| id == focused && now - at < COPY_AGAIN);
        if again && self.copyable().len() > 1 {
            self.copy_conversation();
        } else {
            self.copy_message();
            self.copied = Some((focused, now));
        }
    }

    /// The open thread's messages a copy includes, with their new parts: drafts aren't, and a
    /// body still being rendered can't be yet.
    fn copyable(&self) -> Vec<(&exo_store::MessageDetail, &str)> {
        let Some(detail) = self.thread.as_ref().and_then(ThreadView::detail) else {
            return Vec::new();
        };
        detail
            .messages
            .iter()
            .filter(|message| !message.is_draft())
            .filter_map(|message| {
                Some((
                    message,
                    self.bodies.facts(&message.id)?.unquoted_html.as_str(),
                ))
            })
            .collect()
    }

    /// Copies the focused message, with the hint to paste it.
    fn copy_message(&mut self) {
        let messages = self.copyable();
        let focused = self.thread.as_ref().and_then(ThreadView::focused);
        let Some((message, html)) = messages
            .iter()
            .find(|(message, _)| Some(message.id.as_str()) == focused)
        else {
            return;
        };
        let contents = copy::message(message, html, &self.now);
        clipboard::write(&self.ctx, &contents.html, &contents.text);
        let copied = (
            contents.html.clone(),
            compose::Fields::of_message(message, html),
        );
        self.commands.hint(copy::message_hint(messages.len() > 1));
        self.copied_message = Some(copied);
    }

    /// Copies the open conversation.
    fn copy_conversation(&mut self) {
        let messages = self.copyable();
        if messages.is_empty() {
            return;
        }
        let contents = copy::conversation(&messages, &self.now);
        clipboard::write(&self.ctx, &contents.html, &contents.text);
        self.commands.hint(copy::conversation_hint());
    }

    /// Copies the focused message's recipients.
    fn copy_recipients(&mut self) {
        let Some(thread) = &self.thread else { return };
        let Some(message) = thread.detail().and_then(|detail| {
            detail
                .messages
                .iter()
                .find(|message| Some(message.id.as_str()) == thread.focused())
        }) else {
            return;
        };
        self.ctx.copy_text(copy::recipients(message));
        self.commands
            .hint(Hint::new().text("Recipients copied to clipboard."));
    }

    /// Copies the thread's subject.
    fn copy_subject(&mut self) {
        let subject = match &self.thread {
            Some(thread) => thread.detail().map(|detail| detail.subject.clone()),
            None => self.inbox.cursor().map(|thread| thread.subject.clone()),
        };
        if let Some(subject) = subject {
            clipboard::write_with_search(&self.ctx, &subject, &format!("subject:\"{subject}\""));
            self.commands
                .hint(Hint::new().text("Subject copied to clipboard."));
        }
    }

    /// Prints the open conversation through the macOS print panel (`Print`, ⌘P, File ▸ Print).
    pub fn print(&mut self) {
        if let Some(thread) = &self.thread {
            print::conversation(
                &self.ctx,
                thread,
                &self.bodies,
                &self.account.loader(),
                self.print_title(),
            );
        }
    }

    /// `Save as PDF`: the save panel, then the open conversation printed into the file chosen.
    fn save_as_pdf(&mut self) {
        if let Some(thread) = &self.thread {
            print::save_as_pdf(
                &self.ctx,
                thread,
                &self.bodies,
                &self.account.loader(),
                self.print_title(),
            );
        }
    }

    /// Prints the open conversation into the PDF at `path` (`Save as PDF` without its panel, for
    /// screenshots and tests); `done` hears whether it was written.
    pub fn print_to_pdf(&mut self, path: std::path::PathBuf, done: print::Done) {
        match &self.thread {
            Some(thread) => print::print(
                &self.ctx,
                thread,
                &self.bodies,
                &self.account.loader(),
                self.print_title(),
                print::Destination::Pdf(path),
                Some(done),
            ),
            None => done(false),
        }
    }

    /// The print job's title, from the open thread's subject.
    fn print_title(&self) -> String {
        let subject = self
            .thread
            .as_ref()
            .and_then(ThreadView::detail)
            .map(|detail| detail.subject.as_str())
            .unwrap_or_default();
        print::title(subject, self.accounts.len(), self.account.email())
    }

    /// The folder drawer's lines for the account's labels.
    fn folder_lines(&self) -> Vec<folders::Line> {
        folders::lines(
            &self.snapshot.labels,
            self.snapshot.muted,
            self.folders.all(),
        )
    }

    /// Carries out what the folder drawer asked for: it closes, and a chosen list shows, with
    /// a hint at its keys when it was clicked.
    fn folder_action(&mut self, action: folders::Action, now: f64) {
        self.folders.close();
        if action == folders::Action::SwitchAccounts {
            return self.accounts_run(accounts::Action::OpenSwitcher, now);
        }
        let folders::Action::Show(place, hint) = action else {
            return;
        };
        match hint {
            // The hint's parts have no edge spaces: each is laid out on its own.
            Some(folders::Hint::GoTo { key, name }) => {
                self.commands.hint(
                    Hint::new()
                        .text("Next time, hit")
                        .keys(["G"])
                        .text("then")
                        .keys([key])
                        .text(format!("to go to {name} instantly!")),
                );
            }
            Some(folders::Hint::Label) => self.commands.hint(
                Hint::new()
                    .text("Next time, hit")
                    .keys(["⌘", "K"])
                    .text("and enter the label to go there faster!"),
            ),
            None => {}
        }
        match place {
            // The drawer's Inbox is home: Important while Important and Other are on.
            folders::Place::List(ListId::Inbox) => {
                self.run(Action::Go(splits::home(self.account.settings())), now)
            }
            folders::Place::List(list) => self.run(Action::Go(list), now),
            folders::Place::Snippets => self.go_to_snippets(),
        }
    }

    /// Carries out `action`.
    fn run(&mut self, action: Action, now: f64) {
        // Going anywhere closes the folder drawer, and leaves the Snippets folder and a snippet
        // open there, once `Save Snippet?` has asked.
        if matches!(action, Action::Go(_) | Action::GoToLabel) {
            if self.snippet_unsaved() {
                self.ask_leaving_snippet(Box::new(move |mail| mail.run(action, now)));
                return;
            }
            self.close_snippet();
            self.close_snippet_view();
            self.folders.close();
            self.snippets.folder.close();
        }
        match action {
            Action::ToggleFolders if self.folders.is_open() => self.folders.close(),
            Action::ToggleFolders => {
                let lines = self.folder_lines();
                let here = if self.snippets.folder.is_open() {
                    folders::Place::Snippets
                } else {
                    folders::Place::List(self.list.clone())
                };
                self.folders.open(&lines, &here);
            }
            Action::Folders(key) => {
                let lines = self.folder_lines();
                if let Some(action) = self.folders.key(key, &lines) {
                    self.folder_action(action, now);
                }
            }
            // ↑ on the first result gives the keyboard back to the search bar.
            Action::Up if self.search.is_some() && self.inbox.cursor_is_first() => {
                self.search_focus_bar(true)
            }
            Action::Up if self.inbox.cursor_is_first() => self.inbox.focus_header(),
            Action::Up => self.inbox.move_cursor(-1),
            Action::Move(delta) => self.inbox.move_cursor(delta),
            Action::Header(commands::HeaderKey::Down) => self.inbox.blur_header(),
            Action::Header(commands::HeaderKey::Move(step)) => {
                if let Some(action) = self.inbox.move_header_focus(step) {
                    self.run_header(action, now);
                }
            }
            Action::Header(key) => {
                if let Some(action) = self.inbox.header_key(key == commands::HeaderKey::Backspace) {
                    self.run_header(action, now);
                }
            }
            Action::Top => self.inbox.cursor_to_top(),
            Action::Bottom => self.inbox.cursor_to_bottom(),
            Action::Page(direction) => self.inbox.page(direction),
            Action::Open => {
                if let Some(id) = self.inbox.cursor().map(|thread| thread.id.clone()) {
                    self.open_thread(id);
                }
            }
            Action::ToggleSelected => self.inbox.toggle_selected(),
            Action::Extend(delta) => self.inbox.extend_selection(delta),
            Action::SelectAll => self.inbox.select_all(),
            Action::SelectAllFromHere => self.inbox.select_all_from_here(),
            Action::DeselectAll => self.inbox.clear_selection(),
            Action::Back { .. } if self.inbox.is_selecting() => self.inbox.clear_selection(),
            // In search, the results give the keyboard back to the bar.
            Action::Back { .. } if self.search.is_some() => self.search_focus_bar(true),
            // A filtered list loses its filter first.
            Action::Back { .. } if self.filter.is_some() => self.filter_set(None),
            // Going to a list doesn't start a back stack, so Escape in one goes home.
            Action::Back { escape: true } => {
                let home = splits::home(self.account.settings());
                if self.list != home {
                    self.run(Action::Go(home), now);
                }
            }
            Action::Back { escape: false } => self.history_back(),
            // Mark Unread from a thread goes back to the list.
            Action::Apply(Mutation::MarkUnread) if self.thread.is_some() => {
                self.triage(Mutation::MarkUnread, now);
                self.close_thread();
            }
            Action::Apply(mutation) => self.triage(mutation, now),
            Action::Pick(action) => self.pick(action, now),
            Action::Undo(token) => self.undo(token, now),
            // Going anywhere leaves search.
            Action::Go(list) if self.search.is_some() => {
                self.search_exit(now);
                self.run(Action::Go(list), now);
            }
            // From a thread, going to the list on screen just goes back to it.
            Action::Go(list) if list == self.list && self.thread.is_some() => self.close_thread(),
            Action::Go(list) if list == self.list => self.inbox.cursor_to_top(),
            Action::Go(list) => {
                self.close_thread();
                self.show(list);
            }
            Action::LeaveThread => self.close_thread(),
            Action::GoToLabel => self.commands.open_palette(Some("go to ")),
            Action::MuteHint => {
                let verb = if self.list == ListId::Muted {
                    "Want to unmute this conversation? Hit"
                } else {
                    "Want to mute this conversation? Hit"
                };
                self.commands
                    .hint(Hint::new().text(verb).keys(["shift", "M"]));
            }
            Action::ThreadBack => {
                if !self
                    .thread
                    .as_mut()
                    .is_some_and(ThreadView::collapse_current)
                {
                    self.close_thread();
                    self.command_nudge_left_thread();
                }
            }
            Action::NextConversation(delta) => self.next_conversation(delta),
            Action::ShowNewMessages => {
                if self
                    .thread
                    .as_mut()
                    .is_some_and(ThreadView::show_new_or_last)
                {
                    self.mark_shown_read();
                }
            }
            Action::ThreadHeader(key) => {
                let clicked = self
                    .thread
                    .as_mut()
                    .and_then(|thread| thread.header_key(key));
                match clicked {
                    Some(thread::Action::ToggleDone) => self.toggle_done(now),
                    Some(thread::Action::Remind) => {
                        self.remind(remind::Action::Open { button: true }, now)
                    }
                    Some(thread::Action::CommandPalette) => self.open_palette_button(),
                    Some(thread::Action::Unsubscribe) => {
                        self.unsubscribe_run(unsubscribe::Action::Launch)
                    }
                    _ => {}
                }
            }
            // Text selected in the thread's header copies as it is; else the bodies'.
            Action::Copy => match self.thread.as_ref().and_then(ThreadView::selected_text) {
                Some(text) => self.ctx.copy_text(text),
                None => self.bodies.copy(),
            },
            // Show images: the focused draft's, else the focused message's.
            Action::ShowImages => {
                if !self.compose_show_images()
                    && let Some(id) = self.thread.as_ref().and_then(ThreadView::focused)
                {
                    self.bodies.show_images(id);
                }
            }
            Action::CopyListed => self.copy_listed(),
            Action::CopyMessage => self.copy_message(),
            Action::CopyConversation => self.copy_conversation(),
            Action::CopyRecipients => self.copy_recipients(),
            Action::CopyAttachments => self.copy_attachments(),
            Action::Preview(action) => self.viewer_run(action, now),
            Action::CopySubject => self.copy_subject(),
            Action::Print => self.print(),
            Action::SaveAsPdf => self.save_as_pdf(),
            Action::Unsubscribe(action) => self.unsubscribe_run(action),
            Action::Compose(action) => {
                // A reply's key ends the palette's nudge.
                if matches!(action, compose::Action::Reply(_)) {
                    self.dismiss_command_nudge();
                }
                self.compose_run(action, now);
            }
            Action::Snippets(action) => self.snippets_run(action, now),
            Action::Search(action) => self.search_run(action, now),
            Action::Filter(action) => self.filter_run(action, now),
            Action::Calendar(action) => self.calendar_run(action),
            Action::Bulk(action) => self.bulk_run(action),
            Action::Remind(action) => self.remind(action, now),
            Action::OutOfOffice(action) => self.out_of_office_run(action),
            // The menu's `Out of Office Reply` is the palette's command.
            Action::Settings(settings::Action::OutOfOffice) => {
                self.out_of_office_run(out_of_office::Action::Open)
            }
            // Get Me To Zero and Bulk Actions are the app's dialogs, not the settings'.
            Action::Settings(settings::Action::Triage(triage)) => self.bulk_run(match triage {
                settings::Triage::GetMeToZero => bulk::Action::OpenGetMeToZero,
                settings::Triage::BulkActions => bulk::Action::OpenBulkActions,
            }),
            Action::OpenLinks(action) => self.open_links_run(action),
            Action::NewSplit(action) => self.new_split_run(action, now),
            Action::RawSource(action) => self.raw_source_run(action),
            Action::GoogleApp(app) => {
                self.open_url(&app.url(self.account.email(), &compose::uuid_v4()))
            }
            Action::Links(action) => self.links_run(action),
            Action::Accounts(action) => self.accounts_run(action, now),
            Action::Rsvp(response) => self.rsvp_command(response),
            Action::Notice(action) => self.notice_run(action),
            Action::FirstRun(action) => self.first_run_run(action),
            Action::Achievements(achievements::Action::Show(view)) => self.show_achievements(view),
            Action::Achievements(achievements::Action::Close) => self.close_achievements(),
            Action::Backdrop(action) => self.backdrop_run(action),
            Action::Settings(settings::Action::Achievements) => {
                self.show_achievements(achievements::View::Main)
            }
            // The settings menu's `Sign Out` row is the palette's command.
            Action::Settings(settings::Action::SignOut) => self.sign_out(),
            Action::Settings(settings::Action::Accounts(event)) => {
                self.account_settings_event(event)
            }
            Action::Settings(settings::Action::AccountPicker(key)) => {
                if let Some(event) = self.settings.account_picker_key(key, &self.accounts) {
                    self.account_settings_event(event);
                }
            }
            Action::Settings(settings::Action::CalendarSettings) => {
                self.settings.hide();
                self.calendar_run(calendar::Action::OpenSettings);
            }
            // `Enable a calendar`: the dialog goes and the week view opens.
            Action::Settings(settings::Action::Notifications(
                settings::NotificationsEvent::EnableCalendar,
            )) => {
                self.run(Action::Settings(settings::Action::CloseDialog), now);
                if self.calendar_week().is_none() {
                    self.calendar_run(calendar::Action::OpenWeek);
                }
            }
            Action::Settings(action) => {
                let escape = action == settings::Action::Escape;
                let outcome = self.settings.run(
                    action,
                    &mut self.account,
                    &mut self.commands,
                    &self.list,
                    &self.now,
                    Action::Settings,
                );
                if let Some(id) = self.settings.take_created_hidden_split() {
                    self.hidden_split = Some(id);
                }
                if let Some(list) = outcome.show {
                    self.run(Action::Go(list), now);
                }
                if escape {
                    self.run(
                        if self.thread.is_some() {
                            Action::ThreadBack
                        } else {
                            Action::Back { escape: true }
                        },
                        now,
                    );
                }
                self.ctx.request_repaint();
            }
            Action::SplitTab(step) => {
                let tabs = splits::tabs(self.account.settings(), &self.list, &self.snapshot.counts);
                if let Some(list) = splits::step(&tabs, &self.list, step) {
                    self.folders.close();
                    self.run(Action::Go(list), now);
                }
            }
            Action::SplitTabHold(step) => {
                // In search, ⇧Tab goes back to the search bar and Tab does nothing.
                if self.search.is_some() {
                    if step < 0 {
                        self.search_focus_bar(true);
                    }
                    return;
                }
                let tabs = splits::tabs(self.account.settings(), &self.list, &self.snapshot.counts);
                // Outside the inbox views every step is to the first tab, however long Tab is held.
                let from = match &self.split_highlight {
                    Some(highlight) if splits::is_inbox_view(&self.list) => highlight.clone(),
                    _ => self.list.clone(),
                };
                self.split_highlight =
                    splits::step(&tabs, &from, step).or(self.split_highlight.take());
                self.folders.close();
                self.sync_snapshot();
            }
            Action::SplitTabRelease => {
                if let Some(list) = self.split_highlight.take() {
                    self.run(Action::Go(list), now);
                }
            }
            thread_action => {
                let bodies = &self.bodies;
                let Some(thread) = self.thread.as_mut() else {
                    return;
                };
                match thread_action {
                    Action::NextMessage(delta) => thread.next_message(delta),
                    Action::Arrow(delta) => thread.arrow(delta, false),
                    Action::ArrowHeld(delta) => thread.arrow(delta, true),
                    Action::ThreadLeft => thread.left(false),
                    Action::ThreadLeftHeld => thread.left(true),
                    Action::ThreadRight => {
                        thread.scroll_sideways(1);
                    }
                    Action::ThreadGutter(key) => thread.gutter_key(key),
                    Action::ExpandMessage => thread.expand_current(),
                    Action::ToggleMessage => thread.toggle_current(bodies),
                    Action::ToggleAllMessages => thread.toggle_all(),
                    Action::ToggleHeader => thread.toggle_header(),
                    Action::PageThread(direction) => thread.page(direction),
                    Action::JumpThread(to_end) => thread.jump(to_end),
                    _ => {}
                }
            }
        }
    }

    /// Opens `url` in the system browser, if it's a web link.
    fn open_url(&self, url: &str) {
        if let Some(url) = format::urls::web(url) {
            self.ctx.open_url(egui::OpenUrl::new_tab(url));
        }
    }

    /// Opens `url` in the system browser without bringing it to the front, if it's a web link:
    /// what ⌘ does to a click on a link.
    fn open_url_in_background(&self, url: &str) {
        let Some(url) = format::urls::web(url) else {
            return;
        };
        if !desktop::open_in_background(&url) {
            self.ctx.open_url(egui::OpenUrl::new_tab(url));
        }
    }

    /// Switches the list view to `list`.
    fn show(&mut self, list: ListId) {
        let filter = filter::for_list(&self.list, &list, self.filter, &self.last_filters);
        if filter != self.filter {
            self.filter = filter;
            self.account.filter(filter);
        }
        self.list = list.clone();
        self.rows_from = None;
        self.asked_more_at = None;
        self.inbox.clear();
        self.account.show(list);
    }

    /// Applies `mutation` to the actionable threads, with the toast for it.
    fn triage(&mut self, mutation: Mutation, now: f64) {
        self.dismiss_command_nudge();
        let ids: Vec<String> = actionable(self.thread.as_ref(), &self.inbox)
            .iter()
            .map(|thread| thread.id.clone())
            .collect();
        self.triage_threads(&ids, mutation, None, 0, now);
    }

    /// Applies `mutation` to the listed threads among `ids`: their rows slide out if it takes
    /// them out of this list, and a toast says `text` (or what `mutation` did), with Undo. A
    /// thread the mutation takes out of the list gives way to the next conversation `direction`
    /// or Auto Advance's way ([`Self::dismiss_and_advance`]). Returns the toast's undo token,
    /// when there was something to apply.
    fn triage_threads(
        &mut self,
        ids: &[String],
        mutation: Mutation,
        text: Option<String>,
        direction: isize,
        now: f64,
    ) -> Option<UndoToken> {
        if self.bulk_instead(ids, &mutation) {
            return None;
        }
        let in_thread = self.thread.is_some();
        let threads: Vec<&ThreadSummary> = self
            .inbox
            .threads()
            .iter()
            .filter(|thread| ids.contains(&thread.id))
            .collect();
        if threads.is_empty() {
            return None;
        }
        let already = match mutation {
            Mutation::MarkDone if threads.iter().all(|thread| thread.done) => {
                Some("Already marked as Done.")
            }
            Mutation::MarkNotDone if threads.iter().all(|thread| !thread.done) => {
                Some("Already marked not Done.")
            }
            _ => None,
        };
        if let Some(text) = already {
            self.commands.toast(Toast::new(text));
            return None;
        }
        let ids: Vec<String> = threads.iter().map(|thread| thread.id.clone()).collect();
        // In Starred, starring and unstarring leave the rows where they are (the account holds
        // them), and marking read or unread doesn't take out an unstarred row it holds.
        let stays = self.search.is_none()
            && self.list == ListId::Starred
            && matches!(
                mutation,
                Mutation::Star | Mutation::Unstar | Mutation::MarkRead | Mutation::MarkUnread
            );
        // Snoozing takes a thread out of the inbox as marking it done does.
        let removes = |thread: &ThreadSummary| {
            thread.removed_by.contains(&mutation)
                || (matches!(mutation, Mutation::Remind { snooze: true, .. })
                    && thread.removed_by.contains(&Mutation::MarkDone))
        };
        let leaving: Vec<String> = threads
            .iter()
            .filter(|thread| !stays && removes(thread))
            .map(|thread| thread.id.clone())
            .collect();
        let cursor = self
            .thread
            .as_ref()
            .map(|thread| thread.id().to_owned())
            .or_else(|| self.inbox.cursor().map(|thread| thread.id.clone()));
        let token = self.account.undo_token();
        let applied = self.account.apply(&ids, mutation.clone(), Some(token));
        if matches!(mutation, Mutation::MarkDone) {
            self.count_done(ids.len(), now);
        }
        // The store predicts the common triage; anything else leaves once its snapshot says so.
        // Search results stay put.
        let unpredicted = ids
            .iter()
            .filter(|id| self.search.is_none() && !stays && !leaving.contains(id));
        self.departing.extend(unpredicted.map(|id| Departure {
            thread: id.clone(),
            applied,
            list: self.list.clone(),
            direction,
        }));
        let text = text.unwrap_or_else(|| commands::toast(&mutation, &self.snapshot.labels));
        self.commands
            .toast(Toast::new(text).undo(Action::Undo(token)));
        self.undoable = Some(Undoable {
            token,
            list: self.list.clone(),
            threads: ids,
            cursor,
            in_thread,
            moved: matches!(mutation, Mutation::Move { .. }),
        });
        if !leaving.is_empty() {
            self.dismiss_and_advance(&leaving, direction, now);
        }
        Some(token)
    }

    /// Slides `leaving` out of the list and moves on. The cursor goes to the next conversation
    /// `direction` (`[`, `]`), else the way Auto Advance goes (`desktopAutoAdvance`) from where
    /// the open conversation was before it moved up the list, or the other way past the end. An
    /// open thread among them gives way to that conversation, or to the list when Auto Advance
    /// is off, there's none, or there's none `direction` (back in the list, the cursor goes down
    /// first).
    pub(crate) fn dismiss_and_advance(&mut self, leaving: &[String], direction: isize, now: f64) {
        let toward = if direction != 0 {
            direction
        } else {
            self.account.settings().desktop_auto_advance.direction()
        };
        let anchored = if direction == 0 && toward != 0 && !self.inbox.is_selecting() {
            self.inbox.take_advance_anchor(toward, leaving)
        } else {
            None
        };
        let went = self
            .inbox
            .dismiss_towards(leaving, if toward == 0 { 1 } else { toward }, now);
        if let Some(anchored) = &anchored {
            self.inbox.point_at(anchored);
        }
        let Some(open) = self
            .thread
            .as_ref()
            .map(|thread| thread.id().to_owned())
            .filter(|open| leaving.contains(open))
        else {
            return;
        };
        match self.inbox.cursor().map(|thread| thread.id.clone()) {
            Some(next) if toward != 0 && next != open && (direction == 0 || went) => {
                self.open_thread(next)
            }
            _ => self.close_thread(),
        }
    }

    /// Slides out the departing rows the latest snapshot no longer lists, before it replaces
    /// the list's rows. A row an action with a direction (`[`, `]`) leaves in place moves the
    /// cursor instead.
    fn dismiss_departed(&mut self) {
        let applied = self.snapshot.applied;
        let (due, waiting): (Vec<Departure>, Vec<Departure>) = std::mem::take(&mut self.departing)
            .into_iter()
            .partition(|departure| departure.applied <= applied);
        self.departing = waiting;
        let now = self.ctx.input(|input| input.time);
        let listed = |id: &str| self.snapshot.threads.iter().any(|thread| thread.id == id);
        let due: Vec<Departure> = due
            .into_iter()
            .filter(|departure| departure.list == self.list)
            .collect();
        let gone: Vec<String> = due
            .iter()
            .filter(|departure| !listed(&departure.thread))
            .map(|departure| departure.thread.clone())
            .collect();
        let direction = due.first().map_or(0, |departure| departure.direction);
        if gone.is_empty() {
            if direction != 0 {
                self.step(direction);
            }
            return;
        }
        self.dismiss_and_advance(&gone, direction, now);
    }

    /// Undoes the mutation behind `token` and takes the user back to its threads.
    fn undo(&mut self, token: UndoToken, now: f64) {
        self.account.undo(token);
        self.picker_undone(token);
        self.bulk_undone(token);
        self.unsubscribe_undone(token);
        let Some(undone) = self.undoable.take_if(|undoable| undoable.token == token) else {
            return;
        };
        if undone.list != self.list {
            self.show(undone.list);
        }
        self.inbox
            .restore(&undone.threads, undone.cursor.clone(), now);
        if undone.moved {
            self.commands.toast(Toast::new("Move undone."));
        }
        if undone.in_thread
            && let Some(id) = undone.cursor
            && self.thread.as_ref().is_none_or(|thread| thread.id() != id)
        {
            self.open_thread(id);
        }
    }

    /// Tells the user about what went wrong in the background.
    fn report_events(&mut self) {
        let events: Vec<Event> = self.account.events();
        for event in events {
            match event {
                Event::Failed { mutation, undo } => {
                    if let Some(text) = commands::failure(&mutation, undo) {
                        self.commands.toast(Toast::new(text).kind(ToastKind::Error));
                    }
                }
                event @ (Event::LabelCreated(_) | Event::LabelNotCreated { .. }) => {
                    let now = self.ctx.input(|input| input.time);
                    self.label_event(event, now);
                }
                Event::Unsent(unsent) => self.sign_out_unsent(unsent),
                Event::Thread { id, detail } => self.unsubscribe_thread_read(id, detail),
                event => self.compose_event(event),
            }
        }
    }

    /// The thread list in the content pane, and what its clicks do.
    fn show_list(
        &mut self,
        ui: &mut egui::Ui,
        content: Rect,
        rem: f32,
        window_width: f32,
        now: f64,
    ) {
        self.inbox.set_focused(!self.folders.is_open());
        let pending = self.pending_rows(&self.account.bulk_actions());
        self.inbox.set_pending(pending);
        self.inbox.set_online(self.snapshot.connection.is_online());
        self.inbox.set_popped_out(self.compose.popped().is_some());
        self.inbox.set_popout_focused(
            self.compose
                .popped()
                .is_some_and(|popped| self.compose.focused().is_some_and(|open| open.id == popped)),
        );
        self.inbox.set_covered(self.popout_card());
        for action in self.inbox.show(ui, content, rem, window_width) {
            match action {
                inbox::Action::Open(id) => self.open_thread(id),
                inbox::Action::ToggleDone => self.toggle_done(now),
                inbox::Action::Remind => self.remind(remind::Action::Open { button: true }, now),
                inbox::Action::CommandPalette => self.open_palette_button(),
                inbox::Action::ShowLabel(name) => self.show_label(&name, now),
                inbox::Action::More if self.asked_more_at != Some(self.inbox.count()) => {
                    self.asked_more_at = Some(self.inbox.count());
                    self.account.more();
                }
                inbox::Action::Header(action) => self.run_header(action, now),
                inbox::Action::SelectHint { deselect } => {
                    let what = if deselect {
                        "to deselect conversations!"
                    } else {
                        "to select conversations!"
                    };
                    self.commands
                        .hint(Hint::new().text("Tip: you can hit").keys(["X"]).text(what));
                }
                inbox::Action::File { file, download } => self.open_result_file(file, !download),
                inbox::Action::Achievements => {
                    self.show_achievements(achievements::View::InboxZero)
                }
                inbox::Action::Nudge(action) => {
                    // Either way it's done with, as of now.
                    let at = self.now.timestamp().to_string();
                    self.account.update_settings(|settings| {
                        settings.complete_nudge(inbox::HIDE_EMPTY_SPLIT, &at)
                    });
                    self.sync_snapshot();
                    // `Try it out`: Split Inbox Settings, asking to hide every empty split.
                    if action == inbox::NudgeAction::Try {
                        self.run(
                            Action::Settings(settings::Action::Open(settings::Dialog::SplitInbox)),
                            now,
                        );
                        self.run(
                            Action::Settings(settings::Action::Split(
                                settings::SplitEvent::OfferHideAll,
                            )),
                            now,
                        );
                    }
                }
                inbox::Action::RowMenu(at) => {
                    // Next frame, once the row has the cursor on screen: the menu waits for the
                    // user.
                    self.row_menu = Some(at);
                    self.ctx.request_repaint();
                }
                inbox::Action::More => {}
            }
        }
        self.search_show(ui, content, rem, window_width, now);
    }

    /// What a click on the list's header (or Enter on what has its keyboard) does.
    fn run_header(&mut self, action: inbox::HeaderAction, now: f64) {
        match action {
            // The folders button, or the selected Inbox tab or view title.
            inbox::HeaderAction::Folders if !self.folders.is_open() => {
                self.run(Action::ToggleFolders, now)
            }
            inbox::HeaderAction::Tab(index) => self.show_tab(index, now),
            inbox::HeaderAction::AddSplit => {
                self.run(Action::Settings(settings::Action::NewSplit), now)
            }
            inbox::HeaderAction::SplitSettings => self.run(
                Action::Settings(settings::Action::Open(settings::Dialog::SplitInbox)),
                now,
            ),
            // The search icon: the `/` hint as search opens.
            inbox::HeaderAction::Search => {
                if self.search.is_none() {
                    self.commands.hint(
                        Hint::new()
                            .text("Next time, hit")
                            .keys(["/"])
                            .text("to search instantly!"),
                    );
                }
                self.run(Action::Search(search::Action::Toggle), now);
            }
            inbox::HeaderAction::Filter => self.run(Action::Filter(filter::Action::Picker), now),
            inbox::HeaderAction::ClearFilter => {
                self.run(Action::Filter(filter::Action::Clear), now)
            }
            // The compose button: out of search, a new message.
            inbox::HeaderAction::Compose => {
                self.commands.hint(
                    Hint::new()
                        .text("Next time, hit")
                        .keys(["C"])
                        .text("to compose instantly!"),
                );
                self.search_exit(now);
                self.compose_new(compose::Fields::default(), None);
            }
            _ => {}
        }
    }

    /// A click on split tab `index`: its list, or for the tab on screen, Important and Other go
    /// home and Inbox opens the folder drawer.
    fn show_tab(&mut self, index: usize, now: f64) {
        let tabs = splits::tabs(self.account.settings(), &self.list, &self.snapshot.counts);
        let Some(tab) = tabs.get(index) else { return };
        let action = match &tab.list {
            ListId::Inbox if self.list == ListId::Inbox => Action::ToggleFolders,
            list @ (ListId::Important | ListId::Other) if *list == self.list => {
                Action::Go(splits::home(self.account.settings()))
            }
            list => Action::Go(list.clone()),
        };
        self.folders.close();
        self.run(action, now);
    }

    /// The open thread in the content pane, over the list, and what its clicks do.
    fn show_thread(
        &mut self,
        ui: &mut egui::Ui,
        content: Rect,
        rem: f32,
        window_width: f32,
        events: &[body::Event],
        now: f64,
    ) {
        let pending = self.thread.as_ref().and_then(|thread| {
            self.pending_rows(&self.account.bulk_actions())
                .remove(thread.id())
        });
        let own_domains = self.own_domains();
        let trash_shown = self.list == ListId::Trash || self.search.is_some();
        let joined = self.joined();
        let Some(thread) = self.thread.as_mut() else {
            return;
        };
        thread.set_status(pending, own_domains);
        thread.set_joined(joined);
        thread.set_trash_shown(trash_shown);
        if let Some(detail) = self
            .snapshot
            .thread
            .clone()
            .filter(|detail| detail.id.as_str() == thread.id())
        {
            self.compose.set_thread_detail(thread, detail);
        }
        if thread.take_read_on_open() {
            self.account
                .apply(&[thread.id().to_owned()], Mutation::MarkRead, None);
        }
        self.bodies.set_images(
            self.account
                .settings()
                .remote_content_blocking
                .unwrap_or_default(),
        );
        if thread.detail().is_some() {
            // A message stored without its body stays a placeholder until it's in.
            let bodied: Vec<_> = thread
                .messages()
                .into_iter()
                .filter(|message| !self.snapshot.bodiless.contains(&message.id))
                .collect();
            self.bodies
                .set_thread(&bodied, self.account.email(), &self.account.loader());
        }
        for (id, attachment) in thread.invite_requests() {
            let (sender, ctx) = (self.invites.0.clone(), self.ctx.clone());
            self.account.loader().attachment(&attachment, move |bytes| {
                if let Ok(bytes) = bytes {
                    let _ = sender.send((id, bytes));
                    ctx.request_repaint();
                }
            });
        }
        while let Ok((id, bytes)) = self.invites.1.try_recv() {
            thread.set_invite(&id, &bytes);
        }
        let chips = thread
            .detail()
            .map(|detail| {
                mailbox::Chips::new(&self.snapshot.labels)
                    .header_chips(&detail.labels, detail.muted)
            })
            .unwrap_or_default();
        thread.set_chips(chips);
        let region = self.commands.region();
        let typing = self
            .compose
            .focused()
            .is_some_and(|open| open.thread.as_deref() == Some(thread.id()));
        let detail = thread.shared_detail();
        let mut inline = detail.as_deref().map(|detail| {
            self.compose.inline(
                detail,
                &self.snapshot.aliases,
                self.account.email(),
                &self.now,
                content.height(),
            )
        });
        thread.set_online(self.snapshot.connection.is_online());
        thread.set_full_names(self.account.settings().show_sender_full_names);
        self.attachment_files
            .update_thread(&self.account.loader(), thread);
        let actions = thread.show(
            ui,
            content,
            rem,
            window_width,
            &mut self.bodies,
            region,
            events,
            inline.as_mut(),
            typing,
        );
        for action in actions {
            match action {
                thread::Action::Back => {
                    self.close_thread();
                    self.command_nudge_left_thread();
                }
                // The previous and next conversation buttons, with their keys' hint.
                thread::Action::Navigate(delta) => {
                    let (key, which) = if delta < 0 {
                        ("K", "previous")
                    } else {
                        ("J", "next")
                    };
                    self.commands.hint(
                        Hint::new()
                            .text("Next time, hit")
                            .keys([key])
                            .text(format!("to instantly open the {which} conversation.")),
                    );
                    self.next_conversation(delta);
                }
                thread::Action::ToggleDone => self.toggle_done(now),
                thread::Action::Remind => self.remind(remind::Action::Open { button: true }, now),
                thread::Action::CommandPalette => self.open_palette_button(),
                thread::Action::ShowLabel(name) => {
                    self.close_thread();
                    self.show_label(&name, now);
                }
                thread::Action::OpenUrl(url)
                    if url
                        .get(..7)
                        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("mailto:")) =>
                {
                    self.compose_mailto(&url, ui.input(|input| input.modifiers.shift))
                }
                // Exo's own links open in a new tab, on the account they name.
                thread::Action::OpenUrl(url) if let Some(link) = links::parse(&url) => {
                    self.request = Some(accounts::Request::Open(link))
                }
                thread::Action::OpenUrl(url) => self.open_url(&url),
                thread::Action::OpenUrlInBackground(url) => self.open_url_in_background(&url),
                thread::Action::ShowNewMessages => self.show_new_messages(),
                thread::Action::ExpandAllHint { expand } => {
                    let then = if expand {
                        "to instantly expand all messages!"
                    } else {
                        "to instantly collapse all messages!"
                    };
                    self.commands
                        .hint(Hint::new().text("Hit").keys(["shift", "O"]).text(then));
                }
                thread::Action::CopyHint { subject } => {
                    let (lead, row) = if subject {
                        ("To copy the subject faster, hit", "→ Copy Subject!")
                    } else {
                        ("To copy recipients faster, hit", "→ Copy Recipients!")
                    };
                    self.commands
                        .hint(Hint::new().text(lead).keys(["⌘", "K"]).text(row));
                }
                // A contact a message shows: the pane shows them until the thread's focus moves.
                thread::Action::ShowContact(person) => self.pane_pin = Some((person, None)),
                thread::Action::ComposeTo(person, pop_out) => {
                    let name = Some(person.name.clone()).filter(|name| !name.is_empty());
                    let fields = compose::Fields {
                        to: vec![exo_store::Address {
                            name,
                            email: person.email.clone(),
                        }],
                        ..compose::Fields::default()
                    };
                    if pop_out {
                        self.compose_new_popped(fields)
                    } else {
                        self.compose_new(fields, None)
                    }
                }
                // The copy's confirmation, cut at 80 characters.
                thread::Action::Copied(text) => {
                    ui.ctx().copy_text(text.clone());
                    let shown: String = if text.chars().count() > 80 {
                        text.chars().take(80).chain(['…']).collect()
                    } else {
                        text
                    };
                    self.commands
                        .hint(Hint::new().text(format!("\"{shown}\" copied to clipboard.")));
                }
                thread::Action::RespondToInvite(invite, response) => {
                    self.calendar_respond_to_invite(invite, response, true)
                }
                thread::Action::ShowDay(date) => self.calendar_show_day(date),
                // Only digits follow `tel:`, so it can go to the app that places calls.
                thread::Action::Dial(number) => ui.ctx().open_url(egui::OpenUrl::new_tab(number)),
                thread::Action::CreateEvent(date) => {
                    let (title, guests) = self
                        .thread
                        .as_ref()
                        .map(ThreadView::event_prefill)
                        .unwrap_or_default();
                    self.calendar_create_for_thread(title, guests, date);
                }
                // A message's Reply, Reply All and Forward buttons, with their keys' hint.
                thread::Action::Compose(compose::Action::Reply(reply)) => {
                    let (key, what) = match reply {
                        exo_store::DraftAction::ReplyAll => ("enter", "to reply all instantly!"),
                        exo_store::DraftAction::Forward => ("F", "to forward instantly!"),
                        _ => ("R", "to reply instantly!"),
                    };
                    self.commands
                        .hint(Hint::new().text("Next time, hit").keys([key]).text(what));
                    self.dismiss_command_nudge();
                    self.compose_run(compose::Action::Reply(reply), now);
                }
                // A message expanded or collapsed by a click, with O's hint.
                thread::Action::QuoteHint { shown } => {
                    let what = if shown {
                        "to instantly expand a message!"
                    } else {
                        "to instantly collapse a message!"
                    };
                    self.commands
                        .hint(Hint::new().text("Next time, hit").keys(["O"]).text(what));
                }
                thread::Action::PopOutHint => self.commands.hint(
                    Hint::new()
                        .text("Tip: hit")
                        .keys(["⌘", "shift", "P"])
                        .text("to pop out your draft!"),
                ),
                thread::Action::Compose(action) => self.compose_run(action, now),
                thread::Action::Unsubscribe => self.unsubscribe_run(unsubscribe::Action::Launch),
                thread::Action::HeaderHint { expanded } => {
                    let then = if expanded {
                        "to instantly expand a message header!"
                    } else {
                        "to instantly collapse a message header!"
                    };
                    self.commands.hint(
                        Hint::new()
                            .text("Next time, hit")
                            .keys(["shift", "H"])
                            .text(then),
                    );
                }
                thread::Action::Download { attachment, open } => {
                    self.attachment_files
                        .open(&self.account.loader(), attachment, open)
                }
                thread::Action::Preview { attachments, start } => {
                    self.preview_attachments(attachments, start)
                }
                thread::Action::DownloadAll { message } => self.download_all(&message),
                thread::Action::DragFile { attachment, at } => self.attachment_files.drag(
                    &self.account.loader(),
                    &attachment,
                    at,
                    ui.ctx().zoom_factor(),
                ),
            }
        }
        self.open_links_hover();
    }

    /// The Show New Messages bar: the messages it held back show, and the conversation is marked
    /// read, without a toast, if they left it unread.
    fn show_new_messages(&mut self) {
        if self
            .thread
            .as_mut()
            .is_some_and(ThreadView::show_new_messages)
        {
            self.mark_shown_read();
        }
    }

    /// Marks the open conversation read, without a toast, if any of its messages is unread.
    fn mark_shown_read(&mut self) {
        let Some(thread) = &self.thread else { return };
        if thread.detail().is_some_and(|detail| {
            detail
                .messages
                .iter()
                .any(exo_store::MessageDetail::is_unread)
        }) {
            let id = thread.id().to_owned();
            self.account
                .apply(std::slice::from_ref(&id), Mutation::MarkRead, None);
        }
    }

    /// A Done button: mark the actionable threads done, or not done when they all are.
    fn toggle_done(&mut self, now: f64) {
        let done = actionable(self.thread.as_ref(), &self.inbox)
            .iter()
            .all(|thread| thread.done);
        let hint = if done {
            Hint::new()
                .text("Next time, hit")
                .keys(["shift", "E"])
                .text("to Mark not Done!")
        } else {
            Hint::new()
                .text("Next time, hit")
                .keys(["E"])
                .text("to Mark Done instantly!")
        };
        self.commands.hint(hint);
        self.triage(
            if done {
                Mutation::MarkNotDone
            } else {
                Mutation::MarkDone
            },
            now,
        );
    }

    /// A ⌘K button: the palette, and the hint at ⌘K.
    fn open_palette_button(&mut self) {
        self.commands.hint(
            Hint::new()
                .text("Next time, hit")
                .keys(["⌘", "K"])
                .text("to instantly open Exo Command!"),
        );
        self.commands.open_palette(None);
    }

    /// A label chip: that label's list.
    fn show_label(&mut self, name: &str, now: f64) {
        let labels = &self.snapshot.labels;
        // An Auto Label's chip shows the Auto Label's name, not its Gmail label's.
        let label = labels.iter().find(|label| label.name == name).or_else(|| {
            labels
                .iter()
                .find(|label| mailbox::chip_name(&label.name).as_deref() == Some(name))
        });
        if let Some(label) = label {
            let list = ListId::Label(label.id.clone());
            self.run(Action::Go(list), now);
        }
    }

    /// Draws one frame and handles its input. Returns what it asks of the app, if anything.
    pub fn frame(&mut self, ui: &mut egui::Ui) -> Option<accounts::Request> {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|input| input.time);
        // What another tab on the account changed of its settings.
        self.account.catch_up();
        self.set_now(self.clock.now());
        if let Some(at) = self.row_menu.take() {
            self.show_row_menu(at, now);
        }
        // The shell's ⌘[ and ⌘], which the page sees too.
        self.history_keys(&ctx);
        self.touch_bar_tip_keys(&ctx);
        self.compose_spelling(&ctx);
        self.calendar_spelling(&ctx);
        self.out_of_office_spelling(&ctx);
        self.field_menus(&ctx);
        self.show_message_menu(&ctx);
        // This frame's keys go to last frame's commands before anything draws, so a key a
        // command takes never reaches a widget.
        let actions = self.commands.dispatch(&ctx);
        // The welcome splash takes the keys while it covers the window, and the first-run flow
        // keeps them to itself.
        if !self.welcome_covers() {
            let covered = self.first_run_covers();
            for action in actions {
                if !covered || matches!(action, Action::FirstRun(_)) {
                    self.run(action, now);
                }
            }
        }
        // ⌘E: what's selected becomes the text to find.
        if crate::find::wants_selection(&ctx) {
            let selected = self
                .thread
                .as_ref()
                .and_then(ThreadView::selected_text)
                .or_else(|| self.bodies.selected_text().map(str::to_owned));
            crate::find::selected(&ctx, selected);
        }
        // Opening the palette closes the folder drawer, and leaves the drafts without the
        // keyboard.
        if self.commands.region() == Region::Command {
            self.folders.close();
        }
        self.compose.palette_open = self.commands.region() == Region::Command;
        settings::apply_theme(
            &ctx,
            self.first_run_theme()
                .unwrap_or(self.account.settings().theme),
        );
        self.first_run_frame();
        self.sync_snapshot();
        self.out_of_office_frame(&ctx);
        self.calendar_frame(&ctx);
        self.compose_sync();
        self.compose_tick(now);
        self.compose_paste_events(&ctx);
        self.compose_images(&ctx);
        self.inbox.settle(now);
        self.report_events();
        self.attachment_files_poll(now);

        let palette = exo_ui::palette(&ctx);
        let window = ui.max_rect();
        ui.painter()
            .rect_filled(window, 0.0, palette.background_base_default);
        // At Inbox Zero the day's photo is behind everything, the tab strip too, and the right
        // pane is hidden. (The list and the rail don't fade.)
        self.backdrop.refresh(&ctx, &self.now);
        let zero = self.inbox.at_inbox_zero();
        if zero {
            self.backdrop
                .paint(ui.painter(), window, self.settings.visible());
            self.paint_behind_app(&ctx, ui.painter(), window, now);
        }
        let popout_wheel = self.take_wheel_over_popout(&ctx);
        // A popped-out draft shrinks the root font and widens the content pane.
        let popped_out = self.compose.popped().is_some();
        let rem = exo_ui::rem(window.width(), popped_out);
        // The status bar takes the window's bottom 32px, and the app shrinks above it.
        let status_bar = self.status_bar();
        // Quick Tips shows only when no other bar does.
        let quick_tips = if status_bar.is_none() {
            self.quick_tips()
        } else {
            None
        };
        let bar_shown = status_bar.is_some() || quick_tips.is_some();
        let main = if bar_shown {
            Rect::from_min_max(
                window.min,
                pos2(window.right(), window.bottom() - exo_ui::MESSAGE_BAR_HEIGHT),
            )
        } else {
            window
        };
        let page = Rect::from_min_max(
            pos2(main.left(), main.top() + chrome::tab_bar_height(&ctx)),
            main.max,
        );
        let mut columns = shell::columns(page, rem, popped_out);
        // At Inbox Zero the list takes the right pane's room.
        if zero {
            columns.content.max.x = columns.right.max.x;
        }
        // A split just made hidden when empty says so when its list first empties.
        self.count_inbox_zero(&ctx, zero, !self.was_zero, now);
        if zero
            && !self.was_zero
            && let Some(id) = self
                .hidden_split
                .take_if(|id| self.list == ListId::Split(id.clone()))
        {
            self.commands
                .toast(Toast::new("This Split Inbox is hidden when empty.").action(
                    "Settings",
                    None,
                    Action::Settings(settings::Action::Split(settings::SplitEvent::Edit(id))),
                ));
        }
        self.was_zero = zero;

        // The rail's Calendar opens the week view, its Mail closes it.
        match shell::rail(ui, columns.rail, zero, self.calendar_week().is_some()) {
            // Each switches only to the other app, with the hint at its key.
            Some(shell::RailAction::Calendar) if self.calendar_week().is_none() => {
                self.run(Action::Calendar(calendar::Action::OpenWeek), now);
                self.commands.hint(
                    Hint::new()
                        .text("Next time, hit")
                        .keys(["2"])
                        .text("to go to Calendar instantly!"),
                );
            }
            Some(shell::RailAction::Mail) if self.calendar_week().is_some() => {
                self.run(Action::Calendar(calendar::Action::CloseWeek), now);
                self.commands.hint(
                    Hint::new()
                        .text("Next time, hit")
                        .keys(["1"])
                        .text("to go to Mail instantly!"),
                );
            }
            Some(shell::RailAction::Calendar | shell::RailAction::Mail) => {}
            None => {}
        }
        let events = self.bodies.poll(&ctx);
        for event in &events {
            match event {
                // A selection copies as plain text, with no hint.
                body::Event::Copied(text) => ctx.copy_text(text.clone()),
                body::Event::NothingSelected => self.copy_message_or_conversation(now),
                body::Event::SelectedMuch => self.commands.hint(
                    Hint::new()
                        .text("Want to copy this message? Deselect the text and just hit")
                        .keys(["⌘", "C"])
                        .text("!"),
                ),
                body::Event::Rendered { thread, facts } => {
                    self.open_links_rendered(thread, facts);
                    self.copy_listed_rendered(thread, facts);
                }
                body::Event::ContextMenu {
                    at, hit, selection, ..
                } => {
                    // Next frame, once the word it selected is on screen.
                    self.message_menu = Some(page_menu::Pending {
                        at: *at,
                        hit: hit.clone(),
                        selection: selection.clone(),
                    });
                    ctx.request_repaint();
                }
                body::Event::Clicked { .. } | body::Event::Anchored { .. } => {}
            }
        }
        if self.compose.pane().is_some() {
            self.show_compose_pane(ui, columns.content, rem, window.width(), now);
        } else if self.thread.is_some() {
            self.show_thread(ui, columns.content, rem, window.width(), &events, now);
        } else if self.snippets.folder.viewing().is_some() && self.search.is_none() {
            self.show_snippet_view(ui, columns.content, rem, window.width(), now);
        } else if self.snippets.folder.is_open() && self.search.is_none() {
            self.show_snippets(ui, columns.content, rem, window.width(), now);
        } else {
            self.show_list(ui, columns.content, rem, window.width(), now);
        }
        self.follow_cursor(&ctx);

        if !zero {
            // The right pane: the accent background behind a 1px border, both painted from the
            // pane's snapped edge.
            let right = exo_ui::pixel_snap(columns.right);
            ui.painter()
                .rect_filled(right, 0.0, palette.background_base_accent);
            ui.painter().vline(
                right.left() + 0.5,
                right.y_range(),
                egui::Stroke::new(1.0, palette.border_base_default),
            );
        }
        // Text inside keeps the unsnapped layout position.
        let pane = Rect::from_min_max(
            pos2(columns.right.left() + 1.0, columns.right.top()),
            columns.right.max,
        );
        // The pane's body, first match wins: the multi-select shortcuts, the Go to…
        // panel while a `g` sequence is pending, then the contact.
        let panel = if self.inbox.is_selecting() {
            Some(shortcuts_pane::Panel::MultiSelect {
                select_all: self.inbox.is_select_all_mode(),
            })
        } else {
            self.commands
                .go_menu_visible()
                .then_some(shortcuts_pane::Panel::GoTo {
                    unified: self.account.settings().unified_inbox,
                })
        };
        match panel {
            // Then the settings menu, which at Inbox Zero sits in the footer instead.
            None if self.settings.visible() => {
                if let Some(action) =
                    self.settings
                        .show_menu(ui, pane, rem, zero, self.account.settings())
                {
                    self.run(Action::Settings(action), now);
                }
            }
            _ if zero => {}
            // The day view comes before the shortcut panels.
            _ if self.calendar_show_pane(ui, pane, rem) => {}
            Some(panel) => shortcuts_pane::show(&ui.painter_at(pane), pane, rem, panel),
            // The Snippets folder's tips, under the highlighted snippet when one is.
            None if self.snippets.folder.is_open()
                && self.thread.is_none()
                && self.search.is_none() =>
            {
                let highlighted = self
                    .snippets
                    .folder
                    .pane(ui.ctx(), now)
                    .map(str::to_owned)
                    .filter(|_| self.compose.pane().is_none());
                let snippet = highlighted.and_then(|id| {
                    self.account
                        .settings()
                        .snippets
                        .iter()
                        .find(|snippet| snippet.id == id)
                });
                snippets::show_pane(&ui.painter_at(pane), pane, rem, snippet, &self.now);
            }
            None => {
                // A new message's pane leaves out no conversation: the draft is the current one.
                let shown = if self.compose.pane().is_some() {
                    None
                } else {
                    self.thread
                        .as_ref()
                        .map(|thread| thread.id().to_owned())
                        .or_else(|| self.pane_thread.clone())
                };
                let card = self.pane_card();
                self.pane_shown = card.as_ref().map(|card| card.person.clone());
                if let Some(card) = card {
                    let is_me = card.person.email.eq_ignore_ascii_case(self.account.email());
                    match contact_pane::show(
                        ui,
                        pane,
                        rem,
                        &card,
                        shown.as_deref(),
                        is_me,
                        self.now.timestamp(),
                    ) {
                        Some(ContactAction::OpenThread(id)) => {
                            self.open_thread(id);
                            self.pane_pin = Some((card.person, None));
                        }
                        Some(ContactAction::OpenDomain(domain)) => {
                            self.open_url(&format!("https://{domain}"))
                        }
                        // The copy button's confirmation.
                        Some(ContactAction::Copied(text)) => self
                            .commands
                            .hint(Hint::new().text(format!("\"{text}\" copied to clipboard."))),
                        Some(ContactAction::Compose(email)) => {
                            self.compose_new(compose::Fields::to(&email), None)
                        }
                        Some(ContactAction::Search(name)) => self.search_enter(Some(&name), now),
                        None => {}
                    }
                } else if self.search.is_some() && self.thread.is_none() {
                    search::show_tips(ui.painter(), pane, rem);
                }
                if self.search.is_some() && !zero && search::tips_overflow(pane, rem) {
                    settings::footer_shadow(ui, pane, rem, true);
                }
            }
        }
        let bulk_actions = self.account.bulk_actions();
        self.count_cleared(&bulk_actions, now);
        self.footer_line = self.footer.line(
            self.snapshot.uploading,
            self.snapshot.connection.is_online(),
            bulk::status(&bulk_actions),
            self.account.sync_progress(),
            now,
        );
        self.footer.paint(ui, pane, rem, self.footer_line.clone());
        if bulk_actions.iter().any(|action| action.active) {
            // The count moves as Gmail answers.
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        // A new event's form covers the footer.
        let composing = matches!(
            self.commands.region(),
            Region::ComposeForm
                | Region::PopOutComposeForm
                | Region::RecipientList
                | Region::TextFormatterLinkInput
                | Region::TextFormatterColor
                | Region::SearchBar
        );
        // The streak animation hides it.
        let footer = if self.calendar_pane_form() || self.achievements.hides_chrome(now) {
            None
        } else {
            shell::footer(
                ui,
                pane,
                rem,
                settings::badge(&self.account),
                self.settings.visible(),
                composing,
                zero,
            )
        };
        match footer {
            Some(shell::FooterAction::Settings) => {
                self.run(Action::Settings(settings::Action::Toggle), now)
            }
            Some(shell::FooterAction::Calendar) => {
                self.run(Action::Calendar(calendar::Action::Footer), now)
            }
            None => {}
        }
        if let Some(wheel) = popout_wheel {
            ctx.input_mut(|input| input.smooth_scroll_delta = wheel);
        }
        self.show_compose_popout(ui, columns.content, rem, window.width(), now);
        self.calendar_show_week(
            ui,
            Rect::from_min_max(pos2(columns.rail.right(), page.top()), page.max),
        );
        self.calendar_show_details(ui, main, page);
        self.calendar_show_form(ui, main);
        self.calendar_show_prompt(ui, page);

        let title = self.tab_title();
        let listed = self
            .accounts
            .iter()
            .find(|account| account.email == self.account.email());
        let account_color = accounts::color_name(self.account.email(), listed);
        // The welcome splash and account setup draw the strip over themselves.
        if self.welcome.is_none() && !self.account_setup_open() {
            let strip = if zero {
                chrome::Backdrop::Photo
            } else {
                chrome::Backdrop::Page
            };
            chrome::show(
                ui,
                chrome::Tab {
                    title: &title,
                    login_screen: false,
                    account_color: Some(account_color),
                },
                strip,
            );
        }
        let hovered = self
            .thread
            .as_ref()
            .and_then(|thread| thread.hovered_link(&self.bodies));
        self.link_preview
            .show(ui.painter(), page, hovered.as_deref(), now);

        // The folder drawer slides out over the list.
        let lines = self.folder_lines();
        let important_other = !self.account.settings().unified_inbox;
        let current = self
            .accounts
            .iter()
            .find(|account| account.email == self.account.email());
        let view = folders::View {
            email: self.account.email(),
            color: current.map_or_else(
                || accounts::default_color(self.account.email()),
                accounts::color,
            ),
            inbox: if important_other {
                self.snapshot.counts.get(&ListId::Important).copied()
            } else {
                self.snapshot.inbox
            },
            important_other,
        };
        if let Some(action) = self.folders.show(ui, page, rem, &lines, &view) {
            self.folder_action(action, now);
        }
        if let Some(bar) = &status_bar {
            let shown = exo_ui::MessageBar {
                title: bar.title,
                description: &bar.runs,
                color: bar.color,
            };
            if let Some(url) = shown.show(ui, window) {
                self.open_url(&url);
            }
        }
        // Its cross turns Quick Tips off, saying how to get them back.
        if let Some(context) = quick_tips
            && (exo_ui::QuickTipsBar {
                parts: context.parts(),
            })
            .show(ui, window)
        {
            self.account
                .update_settings(|settings| settings.quick_tips.show = false);
            self.commands.hint(settings::quick_tips_off_hint());
        }

        self.show_raw_source(&ctx, page);
        self.show_viewer(&ctx, page, now);

        // Last: this frame's commands and the overlays. At Inbox Zero the toast rises to 68px
        // unless the streak animation hides the chrome, moving there over 150ms (`ease`).
        let raised = zero && !self.achievements.hides_chrome(now);
        let toast_zero = motion::transition(
            &ctx,
            exo_ui::id(&ctx, "inbox-zero-toast"),
            raised,
            0.15,
            CubicBezier::EASE,
        );
        // Over the select-all hint, toasts sit at 72px; hints stay where they are.
        let hint_shown = self.thread.is_none() && self.inbox.shows_select_all_hint();
        let toast_bottom = if hint_shown {
            SELECT_ALL_HINT_TOAST_BOTTOM
        } else {
            exo_ui::TOAST_BOTTOM
        };
        // Over the status bar they sit at 64px, over the select-all hint's height but not Inbox
        // Zero's.
        let layered = motion::transition(
            &ctx,
            exo_ui::id(&ctx, "status-bar-toast"),
            bar_shown,
            0.15,
            CubicBezier::EASE,
        );
        let toast_bottom = toast_bottom + (STATUS_BAR_TOAST_BOTTOM - toast_bottom) * layered;
        self.commands
            .set_toast_bottom(toast_bottom + (INBOX_ZERO_TOAST_BOTTOM - toast_bottom) * toast_zero);
        self.selection_hint();
        // The pickers' alert first: signing another account out asks over the Accounts dialog.
        let region = if let Some(region) = self.first_run_region() {
            region
        } else if let Some(region) = self.pickers.region() {
            region
        } else if self.previewing() {
            Region::AttachmentPreview
        } else if self.touch_bar_tip.is_some() || self.notice().is_some() {
            Region::Alert
        } else if let Some(region) = self.out_of_office.region() {
            region
        } else if self.settings.dialog_open() {
            self.settings.dialog_region()
        } else if self.raw_source_open_now() {
            Region::ThreadSourceViewer
        } else if let Some(region) = self.calendar_region() {
            region
        } else if self.bulk_dialogs.is_open()
            || self.new_split_dialog_open()
            || self.achievements.dialog().is_some()
        {
            Region::Dialog
        } else if let Some(region) = self.compose.region() {
            region
        } else if self.folders.is_open() {
            Region::LabelSwitcher
        } else if let Some(thread) = &self.thread {
            thread.region()
        } else if self.snippets.folder.viewing().is_some() && self.search.is_none() {
            Region::ReadOnlySnippet
        } else if self
            .search
            .as_ref()
            .is_some_and(search::Search::bar_focused)
        {
            Region::SearchBar
        } else if self.inbox.header_focused() {
            Region::ThreadListHeader
        } else {
            Region::ThreadList
        };
        self.commands.set_region(region);
        let email_notifications = self
            .account
            .settings()
            .email_notifications(self.account.device_settings());
        settings::register(
            &mut self.commands,
            self.account.settings().theme,
            email_notifications,
            ctx.os(),
            Action::Settings,
        );
        settings::register_triage(
            &mut self.commands,
            self.account.settings(),
            Action::Settings,
        );
        self.settings.autocorrect = self.autocorrect();
        settings::register_typing(
            &mut self.commands,
            self.settings.autocorrect,
            self.account.settings().autocomplete(),
            Action::Settings,
        );
        self.commands
            .set_send_and_done(self.account.settings().send_and_done);
        // A new message's pane has no conversation to act on.
        let in_folder =
            self.snippets.folder.is_open() && self.thread.is_none() && self.search.is_none();
        let actionable = if self.compose.pane().is_some() || in_folder {
            Vec::new()
        } else {
            actionable(self.thread.as_ref(), &self.inbox)
        };
        self.commands.set_multiple_selected(actionable.len() > 1);
        let copy = copy_rows(self.thread.as_ref(), &actionable);
        let (sender, own_domains) = if self.compose.pane().is_some() {
            (None, Vec::new())
        } else {
            (self.actionable_sender(), self.own_domains())
        };
        let account_settings = self.account.settings();
        commands::register(
            &mut self.commands,
            &commands::Context {
                list: &self.list,
                threads: &actionable,
                labels: &self.snapshot.labels,
                copy,
                home: splits::home(account_settings),
                other: splits::other(account_settings),
                unified: account_settings.unified_inbox,
                sender: sender.as_ref(),
                own_domains: &own_domains,
                exact: self.thread.is_some() || self.inbox.is_exact(),
            },
        );
        if let Some(marks_done) = self.rsvp_rows().filter(|_| self.compose.pane().is_none()) {
            rsvp::register(&mut self.commands, marks_done);
        }
        search::register(
            &mut self.commands,
            self.search.as_ref().filter(|_| self.thread.is_none()),
            true,
        );
        filter::register(
            &mut self.commands,
            self.thread.is_none() && self.compose.pane().is_none(),
        );
        let visible_splits = splits::tabs(account_settings, &self.list, &self.snapshot.counts)
            .iter()
            .filter(|tab| !tab.hidden && matches!(tab.list, ListId::Split(_)))
            .count();
        splits::register(&mut self.commands, account_settings, visible_splits);
        if self.folders.is_open() {
            commands::register_folders(&mut self.commands);
        }
        // A draft's images, under `Block all remote images`, until shown.
        let draft_images = self.account.settings().remote_content_blocking
            == Some(exo_store::RemoteContentBlocking::All)
            && self.compose_can_show_images();
        if let Some(thread) = &self.thread {
            commands::register_thread(
                &mut self.commands,
                &commands::ThreadContext {
                    focus_collapsed: thread.focus_is_collapsed(),
                    new_messages_focused: thread.new_messages_focused(),
                    // The messages had the keyboard before the palette took it.
                    show_images: draft_images
                        || region == Region::ThreadMessages
                            && thread
                                .focused()
                                .is_some_and(|id| self.bodies.can_show_images(id)),
                },
            );
        } else if draft_images {
            commands::register_show_images(&mut self.commands);
        }
        let (day_open, week_open) = (
            self.calendar_day().is_some(),
            self.calendar_week().is_some(),
        );
        let details = self.calendar_details();
        let calendar_state = self.calendar_state();
        calendar::register(
            &mut self.commands,
            day_open,
            week_open,
            account_settings.calendar.show_declined_events,
            self.compose.focused().is_some(),
            details,
            calendar_state,
        );
        compose::register_shortcuts(
            &mut self.commands,
            &compose::Shortcuts {
                in_thread: self.thread.is_some(),
                focus_collapsed: self
                    .thread
                    .as_ref()
                    .is_some_and(ThreadView::focus_is_collapsed),
                can_reply: self.compose.pane().is_none()
                    && actionable.len() == 1
                    && !actionable[0].draft_only,
                selecting: self.inbox.is_selecting(),
            },
        );
        let team = compose::team_domains(
            self.account.settings(),
            &self.snapshot.aliases,
            self.account.email(),
        );
        self.compose
            .register(&mut self.commands, &self.snapshot.aliases, &team);
        if self.calendar_pane_form() {
            calendar::register_focus_cycle(&mut self.commands);
        }
        if let Some(shown) = &self.pane_shown {
            compose::register_compose_to(&mut self.commands, shown);
        }
        if let Some(sender) = &sender {
            compose::auto_bcc::register_exclude(&mut self.commands, sender);
        }
        if self.compose.pane().is_none() && !in_folder {
            let (threads, exact) = (
                self.actionable_drafts().1,
                self.thread.is_some() || self.inbox.is_exact(),
            );
            compose::discard::register(&mut self.commands, threads, exact);
        }
        // After the list's: the Snippets folder's keys win there.
        let focused_draft = self.compose.focused();
        snippets::register(
            &mut self.commands,
            &snippets::Context {
                draft_focused: focused_draft.is_some(),
                typing: focused_draft.is_some_and(compose::Open::takes_text),
                snippet: focused_draft.is_some_and(compose::Open::is_snippet),
                message_focused: self
                    .thread
                    .as_ref()
                    .is_some_and(|thread| thread.focused().is_some()),
                folder: in_folder,
                editing: self.compose.pane().is_some(),
                viewing: in_folder
                    && self.compose.pane().is_none()
                    && self.snippets.folder.viewing().is_some(),
                snippets: self.account.settings().snippets.len(),
            },
        );
        self.pickers.register(&mut self.commands);
        accounts::register(&mut self.commands, ctx.os());
        let notice = self.notice();
        notice::register(&mut self.commands, notice, self.updates.is_some());
        first_run::register(&mut self.commands, &self.first_run);
        google_apps::register(
            &mut self.commands,
            self.thread.as_ref().and_then(google_apps::target),
        );
        links::register(
            &mut self.commands,
            self.thread.is_some(),
            actionable.len() == 1,
        );
        let action = self
            .thread
            .as_ref()
            .filter(|_| self.compose.pane().is_none())
            .and_then(|thread| thread.conversation_action(&self.bodies));
        open_links::register(&mut self.commands, &actionable, action.as_ref());
        let (source_thread, source_open) = (self.raw_source_thread(), self.raw_source_open_now());
        raw_source::register(&mut self.commands, source_thread.as_deref(), source_open);
        if self.previewing() {
            attachment_files::viewer::register(&mut self.commands);
        }
        bulk::register(&mut self.commands, &self.bulk_dialogs);
        achievements::register(&mut self.commands, self.achievements.dialog().is_some());
        backdrop::register(
            &mut self.commands,
            self.inbox.at_inbox_zero(),
            self.account.settings().reduce_motion,
        );
        self.register_new_split();
        out_of_office::register(&mut self.commands, &self.out_of_office);
        self.settings
            .register_escape(&mut self.commands, Action::Settings);
        if self.settings.dialog_open() {
            // For VIP's contact picker.
            self.index_address_book();
        }
        let calendars = if self.settings.dialog() == Some(settings::Dialog::Notifications) {
            self.calendar_list()
        } else {
            Vec::new()
        };
        if let Some(action) = self.settings.show_dialog(
            &ctx,
            page,
            &self.account,
            &self.accounts,
            &calendars,
            self.compose.book(),
            &self.now,
        ) {
            self.run(Action::Settings(action), now);
        }
        self.show_bulk_dialog(&ctx, page);
        self.show_new_split_dialog(&ctx, page);
        self.show_out_of_office(ui, page);
        self.compose_blur_on_press(&ctx, columns.content);
        self.show_compose_alert(ui, page);
        self.show_picker_alert(ui, page);
        self.show_touch_bar_tip(ui, page);
        self.show_notice(ui, page);
        self.open_links_frame();
        self.show_file_notifications(&ctx, page, now);
        self.copy_listed_frame();
        self.new_split_frame();
        self.command_nudge_frame(now);
        self.show_achievement_overlays(&ctx, page, self.inbox.at_inbox_zero(), now);
        for action in self.commands.show(&ctx, page) {
            self.run(action, now);
        }
        self.show_first_run(
            ui,
            window,
            page,
            &chrome::Tab {
                title: &title,
                login_screen: false,
                account_color: Some(account_color),
            },
        );
        self.show_welcome(ui, window, &title, account_color);
        self.history_visit();
        self.request.take()
    }
}
