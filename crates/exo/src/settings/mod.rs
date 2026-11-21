//! Settings: the menu the footer's gear
//! opens in the right pane, the dialogs its rows open, the palette's settings and theme commands,
//! and the theme the account's setting computes to.
//!
//! The menu lists only what works in Exo. Rows that need a server, AI, a team, billing, or a
//! feature Exo doesn't have yet are left out; see [`menu`] for the full list.

mod account_settings;
mod auto_advance;
mod auto_bcc;
mod autocorrect;
mod blocked_senders;
mod downloads;
mod emoji;
mod images;
mod instant_intro;
mod login_item;
mod menu;
mod notifications;
mod reminders;
mod signatures;

pub(crate) use blocked_senders::{
    highlight as block_highlight, paint_footer as paint_dialog_footer,
};
pub(crate) use menu::footer_shadow;
mod split_inbox;
mod split_library;
mod split_preview;
pub(crate) mod split_template;
mod theme;

use egui::{Context, Rect};
use exo_commands::{Command, Commands, LIST_AND_THREAD, Region};
use exo_store::{ListId, RemoteContentBlocking, Theme};

use crate::account::Account;

pub use account_settings::Event as AccountEvent;
pub use auto_advance::Event as AutoAdvanceEvent;
pub use auto_bcc::Event as AutoBccEvent;
pub use downloads::Event as DownloadsEvent;
pub use emoji::Event as EmojiEvent;
pub use instant_intro::Event as InstantIntroEvent;
pub use notifications::Event as NotificationsEvent;
pub use reminders::Event as RemindersEvent;
pub use signatures::Event as SignaturesEvent;
pub use split_inbox::{Back as SplitBack, Event as SplitEvent, Outcome};
pub(crate) use split_inbox::{
    TOO_LONG as SPLIT_TOO_LONG, add as add_split, definitions as split_definitions,
    replace as replace_split, toggle as toggle_split,
};

/// The announcement the gear's dot is for: it shows until Settings is opened on this device.
const NEW_SETTING: &str = "2023-06-13-salespackUI";
/// Where Escape closes a dialog.
const DIALOG: &[Region] = &[Region::Dialog];
/// Where an alert's keys work.
const ALERT: &[Region] = &[Region::Alert];

/// What a settings command, row or button does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Show the menu: the palette's `Settings`.
    Show,
    /// Show or hide the menu: the footer's gear.
    Toggle,
    /// Escape in the list or a thread while the menu shows: hides it, and the app runs the list's
    /// or thread's own Escape too.
    Escape,
    /// Open a settings dialog.
    Open(Dialog),
    /// Close the dialog.
    CloseDialog,
    /// Change the theme.
    SetTheme(Theme),
    /// Change which remote images bodies load.
    SetImages(RemoteContentBlocking),
    /// Open the shortcuts sheet.
    Shortcuts,
    /// Add Split: the split library, or New Split Inbox without one; either
    /// goes back to Split Inbox Settings.
    NewSplit,
    /// `Split Inbox Library`, from the menu or the palette: the same.
    SplitLibrary,
    /// Something in a split inbox dialog.
    Split(SplitEvent),
    /// A Triage row the app opens a dialog of its own for (`crate::bulk`).
    Triage(Triage),
    /// Something in Blocked Senders.
    Blocked(blocked_senders::Event),
    /// The menu's `Sign Out`, which the app carries out.
    SignOut,
    /// The menu's `Achievements`, the app's dialog.
    Achievements,
    /// Something in the Accounts dialog, which the app carries out.
    Accounts(AccountEvent),
    /// A key in the Accounts dialog's color picker, which the app carries out.
    AccountPicker(exo_ui::PickerKey),
    /// The menu's `Open at Login` row, Exo's own: turns the login item on or off.
    OpenAtLogin,
    /// The menu's `Warn Before Quitting` row, Exo's own: turns the quit alert on or off.
    WarnBeforeQuitting,
    /// The menu's `Calendar Accounts`, `Meeting Links` and `Timezones`,
    /// which the app carries out: the menu goes and Calendar Settings opens.
    CalendarSettings,
    /// Something in the Notifications dialog; `Enable a calendar` the app carries out.
    Notifications(NotificationsEvent),
    /// `Enable/Disable Email Notifications` in the palette: the dialog's checkbox, with a hint.
    ToggleEmailNotifications,
    /// Something in the Reminders dialog.
    Reminders(RemindersEvent),
    /// The menu's `Out of Office Reply` row, which the app carries out (`crate::out_of_office`).
    OutOfOffice,
    /// Something in the Auto Advance dialog.
    AutoAdvance(AutoAdvanceEvent),
    /// One of the menu's toggles, from the menu or (`palette`) its `Enable …` / `Disable …`
    /// command.
    Flip { setting: Toggle, palette: bool },
    /// Autocorrect turned `on` or off, from the Autocorrect dialog or (`hint`) the palette.
    SetAutocorrect { on: bool, hint: bool },
    /// Something in the Signatures dialog.
    Signatures(SignaturesEvent),
    /// Something in the Downloads dialog.
    Downloads(DownloadsEvent),
    /// `OK` on `Could not change download folder`, or Escape or Enter on it.
    DismissDownloadsAlert,
    /// Something in the Auto Bcc dialog.
    AutoBcc(AutoBccEvent),
    /// Something in the Instant Intro dialog.
    InstantIntro(InstantIntroEvent),
    /// Something in the Emoji dialog.
    Emoji(EmojiEvent),
}

/// An account setting one of the menu's toggles flips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    /// `Convert Hyphen to Bulleted List` (`convertHyphenToBulletList`).
    HyphenToBulletList,
    /// `Backtick as Escape` (`enableBacktickAsEscape`).
    BacktickAsEscape,
    /// `Send + Mark Done` (`sendAndDone`).
    SendAndDone,
    /// `RSVP + Mark Done` (`rsvpAndDone`).
    RsvpAndDone,
    /// `Show Sender Full Names` (`showSenderFullNames`).
    SenderFullNames,
    /// `Quick Tips` (`quickTips.show`): the Quick Tips bar.
    QuickTips,
    /// `Autocomplete` (`typingAssistance/autocompleteEnabled`).
    Autocomplete,
}

/// The menu's Triage rows that open the app's bulk dialogs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Triage {
    /// `Get Me To Zero`.
    GetMeToZero,
    /// `Bulk Actions`.
    BulkActions,
}

/// A settings dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialog {
    /// `THEME_SETTINGS`.
    Theme,
    /// Split Inbox Settings and New/Edit Split Inbox.
    SplitInbox,
    /// `HIDE_EMPTY_SPLITS_SETTINGS`.
    HideEmptySplits,
    /// `BLOCK_SETTINGS`: Blocked Senders.
    BlockedSenders,
    /// `ACCOUNT_SETTINGS`: Accounts.
    Accounts,
    /// `NOTIFICATION_SETTINGS`: Notifications.
    Notifications,
    /// `PRIVACY_SETTINGS`: Images.
    Images,
    /// `REMINDER_SETTINGS`: Reminders.
    Reminders,
    /// `AUTO_ADVANCE_SETTINGS`: Auto Advance.
    AutoAdvance,
    /// `SIGNATURE_SETTINGS`: Signatures.
    Signatures,
    /// `DOWNLOADS_SETTINGS`: Downloads.
    Downloads,
    /// `AUTO_BCC_SETTINGS`: Auto Bcc.
    AutoBcc,
    /// `INSTANT_INTRO_SETTINGS`: Instant Intro.
    InstantIntro,
    /// `EMOJI_SKIN_TONE_SETTINGS`: Emoji.
    Emoji,
    /// `AUTOCORRECT_SETTINGS`: Autocorrect.
    Autocorrect,
}

/// The settings menu and dialogs on screen.
#[derive(Debug)]
pub struct Settings {
    /// The menu is in the right pane.
    visible: bool,
    /// Where the menu has scrolled; back to the top each time it opens.
    scroll: menu::Scroll,
    /// The dialog showing, or fading out.
    dialog: Option<Dialog>,
    /// The dialog hasn't been closed.
    dialog_open: bool,
    /// The split inbox dialogs' screens and alerts.
    splits: split_inbox::SplitDialogs,
    /// Blocked Senders' focus and alert.
    blocked: blocked_senders::State,
    /// The Accounts dialog's color picker and dragged row.
    accounts: account_settings::State,
    /// `Open at Login`'s login item.
    login_item: login_item::LoginItem,
    /// `Warn Before Quitting`, when the app has it.
    quit_warning: Option<crate::desktop::QuitWarning>,
    /// The Notifications dialog's open list.
    notifications: notifications::State,
    /// The Reminders dialog's inputs.
    reminders: reminders::State,
    /// The Auto Advance dialog's open list.
    auto_advance: auto_advance::State,
    /// The Signatures dialog's alias and keyboard, and the renderer of its signature, made the
    /// first time it shows.
    signatures: signatures::State,
    signature_preview: Option<signatures::Preview>,
    /// The Downloads dialog's list, and whether `Could not change download folder` shows.
    downloads: downloads::State,
    downloads_alert: bool,
    ctx: Context,
    /// The Auto Bcc dialog's inputs.
    auto_bcc: auto_bcc::State,
    /// The Instant Intro dialog's line and alert.
    instant_intro: instant_intro::State,
    /// The Emoji dialog's sliding bar.
    emoji: emoji::State,
    /// Whether autocorrect is on, for the Autocorrect dialog (the app's, which knows macOS's
    /// default).
    pub(crate) autocorrect: bool,
}

impl Settings {
    /// The settings of the app in `ctx`, with the menu hidden.
    pub fn new(ctx: &Context) -> Self {
        Self {
            visible: false,
            scroll: menu::Scroll::default(),
            dialog: None,
            dialog_open: false,
            splits: split_inbox::SplitDialogs::default(),
            blocked: blocked_senders::State::default(),
            accounts: account_settings::State::default(),
            login_item: login_item::LoginItem::new(ctx),
            quit_warning: None,
            notifications: notifications::State::default(),
            reminders: reminders::State::default(),
            auto_advance: auto_advance::State::default(),
            signatures: signatures::State::default(),
            signature_preview: None,
            downloads: downloads::State::default(),
            downloads_alert: false,
            ctx: ctx.clone(),
            auto_bcc: auto_bcc::State::default(),
            instant_intro: instant_intro::State::open(&exo_store::Settings::default()),
            emoji: emoji::State::default(),
            autocorrect: false,
        }
    }

    /// The menu is in the right pane.
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// A dialog has the keyboard.
    pub fn dialog_open(&self) -> bool {
        self.dialog_open
    }

    /// The dialog showing, or fading out.
    pub fn dialog(&self) -> Option<Dialog> {
        self.dialog
    }

    /// The region that has the keyboard while a dialog is open: an alert over it, or the dialog.
    pub fn dialog_region(&self) -> Region {
        if self.splits.alert_open()
            || self.blocked.alert_open()
            || self.downloads_alert
            || self.instant_intro.alert_open()
        {
            Region::Alert
        } else if self.dialog == Some(Dialog::Notifications) && self.notifications.picker_open() {
            Region::CalendarColorPicker
        } else {
            Region::Dialog
        }
    }

    /// Uses `library` (the bundled split library, when there is one) for the split dialogs.
    pub fn set_split_library(&mut self, library: Option<exo_store::SplitLibrary>) {
        self.splits.set_library(library);
    }

    /// The split library, when there is one.
    pub fn split_library(&self) -> Option<&exo_store::SplitLibrary> {
        self.splits.library()
    }

    /// Shows `Warn Before Quitting` for `warning`; with none, no row.
    pub fn set_quit_warning(&mut self, warning: Option<crate::desktop::QuitWarning>) {
        self.quit_warning = warning;
    }

    /// Hides the menu, as the right pane moving to another conversation does.
    pub fn hide(&mut self) {
        self.visible = false;
    }

    fn show(&mut self, account: &mut Account) {
        if !self.visible {
            self.visible = true;
            self.scroll = menu::Scroll::default();
            self.login_item.read();
        }
        account.update_device_settings(|device| {
            device.new_setting_notification = Some(NEW_SETTING.to_owned())
        });
    }

    /// Carries out `action`, with `current` the list on screen, and returns what it asks of the
    /// What `key` does in the Accounts dialog's open color picker, `accounts` its rows: the
    /// color an account changes to, if any.
    /// Split Inbox's sender picker, while a template's settings show it.
    pub(crate) fn sender_picker_mut(
        &mut self,
    ) -> Option<&mut crate::contact_picker::ContactPicker> {
        self.splits.sender_picker_mut()
    }

    pub fn account_picker_key(
        &mut self,
        key: exo_ui::PickerKey,
        accounts: &[exo_auth::Account],
    ) -> Option<AccountEvent> {
        self.accounts.picker_key(key, accounts)
    }

    /// The split just made that's hidden when empty, if one was, once.
    pub fn take_created_hidden_split(&mut self) -> Option<String> {
        self.splits.take_created_hidden()
    }

    /// app. `wrap` makes app actions of settings actions, for the toasts it shows.
    pub fn run<A: Clone + 'static>(
        &mut self,
        action: Action,
        account: &mut Account,
        commands: &mut Commands<A>,
        current: &ListId,
        now: &jiff::Zoned,
        wrap: impl Fn(Action) -> A,
    ) -> Outcome {
        match action {
            Action::Show => self.show(account),
            Action::Toggle if self.visible => self.hide(),
            Action::Toggle => self.show(account),
            Action::Escape => self.hide(),
            Action::Open(dialog) => {
                match dialog {
                    Dialog::Theme | Dialog::Images | Dialog::Autocorrect => {}
                    Dialog::SplitInbox => self.splits.open_list(),
                    Dialog::HideEmptySplits => self.splits.open_hide_empty(),
                    Dialog::BlockedSenders => self.blocked.reset(),
                    Dialog::Accounts => self.accounts.reset(),
                    Dialog::Notifications => self.notifications.reset(),
                    Dialog::AutoAdvance => self.auto_advance = auto_advance::State::default(),
                    Dialog::Reminders => {
                        // Opening the dialog stores the defaults when the account has none; an
                        // `EXTERNAL` default trigger stays as it is.
                        let mut filled = account.settings().reminders.clone();
                        if filled.fill_defaults() {
                            account.update_settings(|settings| settings.reminders = filled);
                        }
                        self.reminders = reminders::State::open(&account.settings().reminders, now);
                    }
                    Dialog::Signatures => {
                        self.signatures = signatures::State::open(&account.snapshot().aliases)
                    }
                    Dialog::Downloads => {
                        self.downloads = downloads::State::default();
                        self.downloads_alert = false;
                    }
                    Dialog::AutoBcc => self.auto_bcc = auto_bcc::State::default(),
                    Dialog::InstantIntro => {
                        self.instant_intro = instant_intro::State::open(account.settings())
                    }
                    Dialog::Emoji => {
                        self.emoji = emoji::State::default();
                        // Opening it with no tone chosen chooses none.
                        if account.settings().emoji_skin_tone.is_none() {
                            account.update_settings(|settings| {
                                settings.emoji_skin_tone = Some(exo_store::EmojiSkinTone::None)
                            });
                        }
                    }
                }
                self.open(dialog);
            }
            Action::NewSplit | Action::SplitLibrary => {
                self.splits.open_list();
                self.splits.open_library();
                self.open(Dialog::SplitInbox);
            }
            Action::CloseDialog => self.dialog_open = false,
            Action::SetTheme(theme) => account.update_settings(|settings| settings.theme = theme),
            // The messages whose images were shown are forgotten as the body sees the change.
            Action::SetImages(images) => {
                account.update_settings(|settings| settings.remote_content_blocking = Some(images))
            }
            Action::Shortcuts => commands.open_shortcuts(),
            // The app carries these out before they get here.
            Action::Triage(_)
            | Action::SignOut
            | Action::Achievements
            | Action::Accounts(_)
            | Action::AccountPicker(_)
            | Action::CalendarSettings
            | Action::OutOfOffice => {}
            Action::OpenAtLogin => self.login_item.toggle(),
            Action::WarnBeforeQuitting => {
                if let Some(warning) = &self.quit_warning {
                    warning.set(!warning.is_on());
                }
            }
            Action::Reminders(event) => {
                if let Some(saved) =
                    self.reminders
                        .handle(event, &account.settings().reminders, now)
                {
                    account.update_settings(|settings| settings.reminders = saved);
                }
            }
            Action::AutoAdvance(event) => {
                if let Some(chosen) = self
                    .auto_advance
                    .handle(event, account.settings().desktop_auto_advance)
                {
                    account.update_settings(|settings| settings.desktop_auto_advance = chosen);
                }
            }
            Action::Signatures(event) => {
                match self.signatures.handle(event, &account.snapshot().aliases) {
                    Some(signatures::Event::ToggleOnReplies) => {
                        let on = account.settings().include_signature_on_replies();
                        account.update_settings(|settings| {
                            settings.include_signature_on_replies = Some(!on)
                        });
                    }
                    // `Gmail settings`, signed in as the account (`authuser`).
                    Some(signatures::Event::OpenGmail) => {
                        let url = format!(
                            "https://mail.google.com/?shxr=0&authuser={}#settings/general",
                            account.email()
                        );
                        self.ctx.open_url(egui::OpenUrl::new_tab(url));
                    }
                    _ => {}
                }
            }
            Action::Downloads(event) => {
                let choices = self.download_choices();
                if let Some(choice) = self.downloads.handle(event, &choices) {
                    self.choose_download_folder(choice);
                }
            }
            Action::DismissDownloadsAlert => self.downloads_alert = false,
            Action::Flip { setting, palette } => {
                let on = toggle(setting, account);
                // From the palette, a hint says what changed.
                let hint = match setting {
                    Toggle::SendAndDone => Some(if on {
                        "Enabled Send + Mark Done"
                    } else {
                        "Disabled Send + Mark Done"
                    }),
                    Toggle::BacktickAsEscape => Some(if on {
                        "Enabled Backtick as Escape"
                    } else {
                        "Disabled Backtick as Escape"
                    }),
                    Toggle::QuickTips if on => Some("Enabled Quick Tips"),
                    Toggle::Autocomplete => Some(if on {
                        "Enabled Autocomplete"
                    } else {
                        "Disabled Autocomplete"
                    }),
                    _ => None,
                };
                if palette && let Some(hint) = hint {
                    commands.hint(exo_commands::Hint::new().text(hint));
                } else if palette && setting == Toggle::QuickTips {
                    commands.hint(quick_tips_off_hint());
                }
            }
            // From the palette, a hint says what changed.
            Action::SetAutocorrect { on, hint } => {
                account.update_settings(|settings| settings.typing_assistance.enabled = Some(on));
                if hint {
                    commands.hint(exo_commands::Hint::new().text(if on {
                        "Enabled Autocorrect"
                    } else {
                        "Disabled Autocorrect"
                    }));
                }
            }
            Action::Emoji(event) => {
                let current = account.settings().emoji_skin_tone.unwrap_or_default();
                let tone = self.emoji.handle(event, current);
                if tone != current {
                    account.update_settings(|settings| settings.emoji_skin_tone = Some(tone));
                }
            }
            Action::InstantIntro(event) => {
                let outcome = self.instant_intro.handle(event);
                if let Some(line) = outcome.save {
                    account.update_settings(|settings| settings.custom_instant_intro = line);
                }
                if outcome.close {
                    self.dialog_open = false;
                }
            }
            Action::AutoBcc(event) => {
                let domain = account
                    .email()
                    .rsplit_once('@')
                    .map_or("", |(_, domain)| domain)
                    .to_owned();
                let saved = self.auto_bcc.handle(event, account.settings(), &domain);
                if saved.auto_bcc.is_some() || saved.exclusions.is_some() {
                    account.update_settings(|settings| {
                        if let Some(list) = saved.auto_bcc {
                            settings.auto_bcc = list;
                        }
                        if let Some(list) = saved.exclusions {
                            settings.auto_bcc_exclusions = list;
                        }
                    });
                }
                if let Some(text) = saved.toast {
                    commands
                        .toast(exo_commands::Toast::new(text).kind(exo_commands::ToastKind::Error));
                }
            }
            Action::Notifications(event) => {
                let lead = account.settings().notifications.calendar_events.lead_time();
                if let Some(event) = self.notifications.handle(event, lead) {
                    notify(event, account);
                }
            }
            Action::ToggleEmailNotifications => {
                let on = !account
                    .settings()
                    .email_notifications(account.device_settings());
                notify(NotificationsEvent::ToggleEmail, account);
                commands.hint(exo_commands::Hint::new().text(if on {
                    "Enabled Notifications"
                } else {
                    "Disabled Notifications"
                }));
            }
            Action::Blocked(event) => {
                let unblock = self.blocked.handle(
                    event,
                    &blocked_senders::senders(&account.settings().blocked_senders),
                );
                if let Some(sender) = unblock {
                    // Off the list (whatever its case), and its filter goes.
                    account.update_settings(|settings| {
                        settings
                            .blocked_senders
                            .retain(|blocked| !blocked.eq_ignore_ascii_case(&sender))
                    });
                    account.unblock(&sender);
                    self.blocked
                        .clamp(blocked_senders::senders(&account.settings().blocked_senders).len());
                }
            }
            Action::Split(event) => {
                let edit = matches!(
                    event,
                    SplitEvent::Edit(_) | SplitEvent::OpenTemplate(..) | SplitEvent::New(_)
                );
                let outcome = self.splits.run(event, account, commands, current, &wrap);
                // A toast's `Split Inbox Settings` opens the dialog on the split.
                if edit && !self.dialog_open {
                    self.open(Dialog::SplitInbox);
                }
                if outcome.close {
                    self.dialog_open = false;
                }
                return outcome;
            }
        }
        Outcome::default()
    }

    /// The Downloads dialog's rows, the folder in use first.
    fn download_choices(&self) -> Vec<downloads::Choice> {
        downloads::choices(
            crate::downloads::chosen(&self.ctx).as_deref(),
            crate::downloads::downloads_folder().as_deref(),
            crate::downloads::desktop_folder().as_deref(),
        )
    }

    /// Downloads go to `choice`'s folder from now on, asking for one for
    /// `Other…`; nothing changes for the folder already in use or a cancelled chooser. A folder
    /// that can't be used says `Could not change download folder`.
    fn choose_download_folder(&mut self, choice: downloads::Choice) {
        let folder = match choice {
            downloads::Choice::Downloads => crate::downloads::downloads_folder(),
            downloads::Choice::Desktop => crate::downloads::desktop_folder(),
            downloads::Choice::Current(_) => None,
            downloads::Choice::Other => {
                crate::downloads::pick(crate::downloads::folder(&self.ctx).as_deref())
            }
        };
        let Some(folder) =
            folder.filter(|folder| crate::downloads::folder(&self.ctx).as_ref() != Some(folder))
        else {
            return;
        };
        if let Err(error) = crate::downloads::choose(&self.ctx, folder) {
            log::error!("changing the download folder: {error}");
            self.downloads_alert = true;
        }
    }

    fn open(&mut self, dialog: Dialog) {
        // The split dialogs share one overlay, so moving between them doesn't fade it again.
        let shared = |dialog| matches!(dialog, Some(Dialog::SplitInbox | Dialog::HideEmptySplits));
        if !(self.dialog_open && shared(self.dialog) && shared(Some(dialog))) {
            self.dialog = Some(dialog);
        }
        self.dialog_open = true;
    }

    /// Registers Escape for hiding the menu while it shows, and the split dialogs' keys while one
    /// is open. Call after the screens register their own Escape, which this shadows: Backspace,
    /// which shares their action, leaves the menu.
    pub fn register_escape<A: Clone + 'static>(
        &self,
        commands: &mut Commands<A>,
        wrap: impl Fn(Action) -> A,
    ) {
        if self.visible {
            commands.add(
                Command::shortcut("HIDE_SETTINGS_MENU", wrap(Action::Escape))
                    .key("esc")
                    .regions(LIST_AND_THREAD),
            );
        }
        if self.dialog_open && self.dialog == Some(Dialog::Accounts) && self.accounts.picker_open()
        {
            // The colour picker's region shortcuts, as every colour picker has them.
            crate::calendar::register_picker_keys(commands, DIALOG, |key| {
                wrap(Action::AccountPicker(key))
            });
        }
        if self.dialog_open && self.dialog == Some(Dialog::Reminders) {
            use reminders::Event;
            let key = |id: &'static str, event: Event, keys: &[&'static str]| {
                Command::shortcut(id, wrap(Action::Reminders(event)))
                    .keys(keys.iter().copied())
                    .regions(DIALOG)
            };
            if self.reminders.menu_open() {
                commands.add(key(
                    "REMINDERS_MENU_DOWN",
                    Event::MenuMove(1),
                    &["down", "j"],
                ));
                commands.add(key("REMINDERS_MENU_UP", Event::MenuMove(-1), &["up", "k"]));
                commands.add(key("REMINDERS_MENU_ESCAPE", Event::MenuClose, &["esc"]));
            }
            commands.add(key("REMINDERS_TAB", Event::Tab(1), &["tab"]));
            commands.add(key("REMINDERS_SHIFT_TAB", Event::Tab(-1), &["shift+tab"]));
            commands.add(key("REMINDERS_ENTER", Event::Enter, &["enter"]));
        }
        if self.dialog_open && self.dialog == Some(Dialog::AutoAdvance) {
            // The dropdown has the keyboard from the start: the arrows
            // open the list or move round it, Enter opens it or takes the row, and with it open,
            // Tab takes the row and Escape closes it.
            use auto_advance::Event;
            let key = |id: &'static str, event: Event, keys: &[&'static str]| {
                Command::shortcut(id, wrap(Action::AutoAdvance(event)))
                    .keys(keys.iter().copied())
                    .regions(DIALOG)
            };
            commands.add(key(
                "AUTO_ADVANCE_DOWN",
                Event::Move(1),
                &["down", "j", "n"],
            ));
            commands.add(key("AUTO_ADVANCE_UP", Event::Move(-1), &["up", "k", "p"]));
            if self.auto_advance.menu_open() {
                commands.add(key("AUTO_ADVANCE_ENTER", Event::Enter, &["enter", "tab"]));
                commands.add(key("AUTO_ADVANCE_ESCAPE", Event::MenuClose, &["esc"]));
            } else {
                commands.add(key("AUTO_ADVANCE_ENTER", Event::ToggleMenu, &["enter"]));
            }
        }
        if self.dialog_open && self.dialog == Some(Dialog::Signatures) {
            use signatures::{Event, Focus};
            let key = |id: &'static str, event: Event, keys: &[&'static str]| {
                Command::shortcut(id, wrap(Action::Signatures(event)))
                    .keys(keys.iter().copied())
                    .regions(DIALOG)
            };
            if self.signatures.menu_open() {
                commands.add(key(
                    "SIGNATURES_MENU_DOWN",
                    Event::MenuMove(1),
                    &["down", "j", "n"],
                ));
                commands.add(key(
                    "SIGNATURES_MENU_UP",
                    Event::MenuMove(-1),
                    &["up", "k", "p"],
                ));
                commands.add(key(
                    "SIGNATURES_MENU_PICK",
                    Event::MenuPick,
                    &["enter", "tab"],
                ));
                commands.add(key("SIGNATURES_MENU_ESCAPE", Event::MenuClose, &["esc"]));
            } else {
                if self.signatures.focus() == Some(Focus::Dropdown) {
                    commands.add(key(
                        "SIGNATURES_MENU_OPEN",
                        Event::ToggleMenu,
                        &["down", "j", "n", "up", "k", "p"],
                    ));
                }
                commands.add(key("SIGNATURES_TAB", Event::Tab(1), &["tab"]));
                commands.add(key("SIGNATURES_SHIFT_TAB", Event::Tab(-1), &["shift+tab"]));
                commands.add(key(
                    "SIGNATURES_ACTIVATE",
                    Event::Activate,
                    &["enter", "space"],
                ));
            }
        }
        if self.dialog_open && self.dialog == Some(Dialog::Downloads) {
            use downloads::Event;
            let key = |id: &'static str, event: Event, keys: &[&'static str]| {
                Command::shortcut(id, wrap(Action::Downloads(event)))
                    .keys(keys.iter().copied())
                    .regions(DIALOG)
            };
            if self.downloads.menu_open() {
                commands.add(key(
                    "DOWNLOADS_MENU_DOWN",
                    Event::MenuMove(1),
                    &["down", "j", "n"],
                ));
                commands.add(key(
                    "DOWNLOADS_MENU_UP",
                    Event::MenuMove(-1),
                    &["up", "k", "p"],
                ));
                commands.add(key(
                    "DOWNLOADS_MENU_PICK",
                    Event::MenuPick,
                    &["enter", "tab"],
                ));
                commands.add(key("DOWNLOADS_MENU_ESCAPE", Event::MenuClose, &["esc"]));
            } else {
                // The dropdown has the keyboard: Enter or Space clicks it, an arrow opens it.
                commands.add(key(
                    "DOWNLOADS_MENU_OPEN",
                    Event::ToggleMenu,
                    &["enter", "space", "down", "j", "n", "up", "k", "p"],
                ));
            }
            if self.downloads_alert {
                commands.add(
                    Command::shortcut("ALERT_OK", wrap(Action::DismissDownloadsAlert))
                        .keys(["enter", "mod+enter", "space", "esc"])
                        .regions(ALERT),
                );
            }
        }
        if self.dialog_open && self.dialog == Some(Dialog::AutoBcc) {
            use auto_bcc::Event;
            let key = |id: &'static str, event: Event, keys: &[&'static str]| {
                Command::shortcut(id, wrap(Action::AutoBcc(event)))
                    .keys(keys.iter().copied())
                    .regions(DIALOG)
            };
            commands.add(key("AUTO_BCC_TAB", Event::Tab(1), &["tab"]));
            commands.add(key("AUTO_BCC_SHIFT_TAB", Event::Tab(-1), &["shift+tab"]));
            // Space types in the inputs; on a link or a trash can it's Enter.
            let activate: &[&str] = if self.auto_bcc.typing() {
                &["enter"]
            } else {
                &["enter", "space"]
            };
            commands.add(key("AUTO_BCC_ENTER", Event::Activate, activate));
        }
        if self.dialog_open && self.dialog == Some(Dialog::Emoji) {
            let step = |id: &'static str, delta: isize, keys: &[&'static str]| {
                Command::shortcut(id, wrap(Action::Emoji(emoji::Event::Step(delta))))
                    .keys(keys.iter().copied())
                    .repeat(wrap(Action::Emoji(emoji::Event::Step(delta))))
                    .regions(DIALOG)
            };
            commands.add(step("EMOJI_SKIN_TONE_NEXT", 1, &["tab", "right"]));
            commands.add(step("EMOJI_SKIN_TONE_PREVIOUS", -1, &["shift+tab", "left"]));
            commands.add(
                Command::shortcut("EMOJI_SKIN_TONE_ENTER", wrap(Action::CloseDialog))
                    .key("enter")
                    .regions(DIALOG),
            );
        }
        if self.dialog_open && self.dialog == Some(Dialog::InstantIntro) {
            use instant_intro::Event;
            let key = |id: &'static str,
                       event: Event,
                       keys: &[&'static str],
                       region: &'static [Region]| {
                Command::shortcut(id, wrap(Action::InstantIntro(event)))
                    .keys(keys.iter().copied())
                    .regions(region)
            };
            commands.add(key("INSTANT_INTRO_TAB", Event::Tab(1), &["tab"], DIALOG));
            commands.add(key(
                "INSTANT_INTRO_SHIFT_TAB",
                Event::Tab(-1),
                &["shift+tab"],
                DIALOG,
            ));
            commands.add(key("INSTANT_INTRO_ENTER", Event::Enter, &["enter"], DIALOG));
            commands.add(key(
                "ALERT_CONFIRM",
                Event::AlertConfirm,
                &["enter", "mod+enter", "space"],
                ALERT,
            ));
            commands.add(key("ALERT_ESCAPE", Event::AlertDismiss, &["esc"], ALERT));
            commands.add(key(
                "ALERT_NEXT",
                Event::AlertCycle(-1),
                &["right", "tab"],
                ALERT,
            ));
            commands.add(key(
                "ALERT_PREVIOUS",
                Event::AlertCycle(1),
                &["left", "shift+tab"],
                ALERT,
            ));
        }
        if self.dialog_open
            && self.dialog == Some(Dialog::Notifications)
            && self.notifications.menu_open()
        {
            commands.add(
                Command::shortcut(
                    "NOTIFY_ME_ESCAPE",
                    wrap(Action::Notifications(NotificationsEvent::Escape)),
                )
                .key("esc")
                .regions(DIALOG),
            );
        }
        if self.dialog_open
            && self.dialog == Some(Dialog::Notifications)
            && self.notifications.picker_open()
        {
            crate::calendar::register_picker_keys(commands, crate::calendar::PICKER, |key| {
                wrap(Action::Notifications(NotificationsEvent::PickerKey(key)))
            });
        }
        if self.dialog_open && self.dialog == Some(Dialog::BlockedSenders) {
            use blocked_senders::Event;
            let blocked = |id: &'static str,
                           event: Event,
                           keys: &[&'static str],
                           region: &'static [Region]| {
                Command::shortcut(id, wrap(Action::Blocked(event)))
                    .keys(keys.iter().copied())
                    .regions(region)
            };
            commands.add(blocked(
                "BLOCKED_DOWN",
                Event::Move(1),
                &["down", "j", "tab", "ctralt+j", "ctralt+n"],
                DIALOG,
            ));
            commands.add(blocked(
                "BLOCKED_UP",
                Event::Move(-1),
                &["up", "k", "shift+tab", "ctralt+k", "ctralt+p"],
                DIALOG,
            ));
            commands.add(blocked(
                "BLOCKED_ENTER",
                Event::Activate,
                &["enter"],
                DIALOG,
            ));
            commands.add(blocked(
                "ALERT_CONFIRM",
                Event::AlertAction(0),
                &["enter", "mod+enter", "space"],
                ALERT,
            ));
            commands.add(blocked(
                "ALERT_ESCAPE",
                Event::AlertDismiss,
                &["esc"],
                ALERT,
            ));
            commands.add(blocked(
                "ALERT_NEXT",
                Event::AlertCycle(-1),
                &["right", "tab"],
                ALERT,
            ));
            commands.add(blocked(
                "ALERT_PREVIOUS",
                Event::AlertCycle(1),
                &["left", "shift+tab"],
                ALERT,
            ));
        }
        if self.dialog_open
            && matches!(
                self.dialog,
                Some(Dialog::SplitInbox | Dialog::HideEmptySplits)
            )
        {
            let split = |id: &'static str,
                         event: SplitEvent,
                         keys: &[&'static str],
                         region: &'static [Region]| {
                Command::shortcut(id, wrap(Action::Split(event)))
                    .keys(keys.iter().copied())
                    .regions(region)
            };
            // The alert's keys: the selected action, dismiss, and
            // moving the selection.
            commands.add(split(
                "ALERT_CONFIRM",
                SplitEvent::AlertConfirm,
                &["enter", "mod+enter", "space"],
                ALERT,
            ));
            commands.add(split(
                "ALERT_ESCAPE",
                SplitEvent::AlertDismiss,
                &["esc"],
                ALERT,
            ));
            commands.add(split(
                "ALERT_NEXT",
                SplitEvent::AlertCycle(-1),
                &["right", "tab"],
                ALERT,
            ));
            commands.add(split(
                "ALERT_PREVIOUS",
                SplitEvent::AlertCycle(1),
                &["left", "shift+tab"],
                ALERT,
            ));
            // Escape runs the dialog's close (with the form's discard check); Enter its primary
            // action.
            commands.add(split(
                "SPLIT_DIALOG_ESCAPE",
                SplitEvent::Close,
                &["esc"],
                DIALOG,
            ));
            if let Some(enter) = self.splits.enter() {
                commands.add(split("SPLIT_DIALOG_ENTER", enter, &["enter"], DIALOG));
            }
            if let Some(remove) = self.splits.row_delete() {
                commands.add(split(
                    "SPLIT_DIALOG_REMOVE_ROW",
                    remove,
                    &["backspace", "del"],
                    DIALOG,
                ));
            }
            // VIP's contact picker, over Enter and Escape while it takes them.
            for (id, key, keys) in self.splits.picker_keys() {
                commands.add(split(
                    id,
                    SplitEvent::Template(split_template::TemplateEvent::PickerKey(key)),
                    keys,
                    DIALOG,
                ));
            }
        }
    }

    /// Draws the menu in `pane` (the right pane inside its border, above its footer) and returns
    /// what was clicked. `zero` is Inbox Zero, where it sits in the footer instead.
    pub fn show_menu(
        &mut self,
        ui: &mut egui::Ui,
        pane: Rect,
        rem: f32,
        zero: bool,
        settings: &exo_store::Settings,
    ) -> Option<Action> {
        let quit_warning = self
            .quit_warning
            .as_ref()
            .map(crate::desktop::QuitWarning::is_on);
        menu::show(
            ui,
            pane,
            rem,
            zero,
            &mut self.scroll,
            settings,
            menu::ExoRows {
                open_at_login: self.login_item.on(),
                warn_before_quitting: quit_warning,
            },
        )
    }

    /// Draws the dialog, if one is showing, over `area` (the window below the tab strip), and
    /// returns what was clicked in it. `accounts` are the device's, for the Accounts dialog;
    /// `calendars` the account's, for Notifications.
    #[expect(
        clippy::too_many_arguments,
        reason = "each dialog's own inputs, from the app"
    )]
    pub fn show_dialog(
        &mut self,
        ctx: &Context,
        area: Rect,
        account: &Account,
        accounts: &[exo_auth::Account],
        calendars: &[crate::calendar::ListedCalendar],
        book: Option<&crate::compose::contacts::Index>,
        now: &jiff::Zoned,
    ) -> Option<Action> {
        let dialog = self.dialog?;
        if dialog == Dialog::AutoAdvance {
            let response = auto_advance::dialog(
                ctx,
                area,
                self.dialog_open,
                &self.auto_advance,
                account.settings().desktop_auto_advance,
            );
            let Some(event) = response.inner else {
                self.dialog = None;
                return None;
            };
            return if response.close {
                Some(Action::CloseDialog)
            } else {
                event.map(Action::AutoAdvance)
            };
        }
        if dialog == Dialog::Signatures {
            let (snapshot, loader) = (account.snapshot(), account.loader());
            let shown = signatures::Shown {
                aliases: &snapshot.aliases,
                on_replies: account.settings().include_signature_on_replies(),
                account: account.email(),
                loader: &loader,
            };
            let preview = self
                .signature_preview
                .get_or_insert_with(|| signatures::Preview::new(ctx));
            let response = signatures::dialog(
                ctx,
                area,
                self.dialog_open,
                &mut self.signatures,
                preview,
                &shown,
            );
            let Some(event) = response.inner else {
                self.dialog = None;
                return None;
            };
            return if response.close {
                Some(Action::CloseDialog)
            } else {
                event.map(Action::Signatures)
            };
        }
        if dialog == Dialog::Downloads {
            let choices = self.download_choices();
            let mut response =
                downloads::dialog(ctx, area, self.dialog_open, &self.downloads, &choices);
            let Some(event) = response.inner.take() else {
                self.dialog = None;
                return None;
            };
            if self.downloads_alert {
                return downloads::alert(ctx, area).then_some(Action::DismissDownloadsAlert);
            }
            return if response.close {
                Some(Action::CloseDialog)
            } else {
                event.map(Action::Downloads)
            };
        }
        if dialog == Dialog::Reminders {
            let response = reminders::dialog(
                ctx,
                area,
                self.dialog_open,
                &mut self.reminders,
                &account.settings().reminders,
                now,
            );
            let Some(event) = response.inner else {
                self.dialog = None;
                return None;
            };
            return if response.close {
                Some(Action::CloseDialog)
            } else {
                event.map(Action::Reminders)
            };
        }
        if dialog == Dialog::Emoji {
            let response = emoji::dialog(
                ctx,
                area,
                self.dialog_open,
                &mut self.emoji,
                account.settings().emoji_skin_tone.unwrap_or_default(),
            );
            let Some(event) = response.inner else {
                self.dialog = None;
                return None;
            };
            return if response.close {
                Some(Action::CloseDialog)
            } else {
                event.map(Action::Emoji)
            };
        }
        if dialog == Dialog::InstantIntro {
            let response =
                instant_intro::dialog(ctx, area, self.dialog_open, &mut self.instant_intro);
            let Some(event) = response.inner else {
                self.dialog = None;
                return None;
            };
            return if response.close {
                Some(Action::CloseDialog)
            } else {
                event.map(Action::InstantIntro)
            };
        }
        if dialog == Dialog::AutoBcc {
            let response = auto_bcc::dialog(
                ctx,
                area,
                self.dialog_open,
                &mut self.auto_bcc,
                account.settings(),
            );
            let Some(event) = response.inner else {
                self.dialog = None;
                return None;
            };
            return if response.close {
                Some(Action::CloseDialog)
            } else {
                event.map(Action::AutoBcc)
            };
        }
        if dialog == Dialog::Notifications {
            let shown = notifications::Shown {
                settings: account.settings(),
                device: account.device_settings(),
                calendars,
            };
            let response =
                notifications::dialog(ctx, area, self.dialog_open, &self.notifications, &shown);
            let Some(event) = response.inner else {
                self.dialog = None;
                return None;
            };
            return if response.close {
                Some(Action::CloseDialog)
            } else {
                event.map(Action::Notifications)
            };
        }
        if dialog == Dialog::Accounts {
            // The keycaps in the page's theme, while the dialog draws its card in Snow's.
            let keycaps = exo_ui::KeycapColors::shortcuts_pane(exo_ui::palette(ctx));
            let response = account_settings::dialog(
                ctx,
                area,
                self.dialog_open,
                &mut self.accounts,
                accounts,
                keycaps,
            );
            let Some(event) = response.inner else {
                self.dialog = None;
                return None;
            };
            return if response.close {
                Some(Action::CloseDialog)
            } else {
                event.map(Action::Accounts)
            };
        }
        if dialog == Dialog::BlockedSenders {
            let senders = blocked_senders::senders(&account.settings().blocked_senders);
            let response =
                blocked_senders::show(ctx, area, self.dialog_open, &self.blocked, &senders);
            let Some(event) = response.inner else {
                self.dialog = None;
                return None;
            };
            return if response.close {
                Some(Action::CloseDialog)
            } else {
                event.map(Action::Blocked)
            };
        }
        if dialog == Dialog::Autocorrect {
            let response = autocorrect::dialog(ctx, area, self.dialog_open, self.autocorrect);
            let Some(action) = response.inner else {
                self.dialog = None;
                return None;
            };
            return if response.close {
                Some(Action::CloseDialog)
            } else {
                action
            };
        }
        if dialog == Dialog::Images {
            let response = images::dialog(
                ctx,
                area,
                self.dialog_open,
                account
                    .settings()
                    .remote_content_blocking
                    .unwrap_or_default(),
            );
            if response.inner.is_none() {
                self.dialog = None;
            }
            if response.close {
                return Some(Action::CloseDialog);
            }
            return response.inner.flatten();
        }
        if dialog != Dialog::Theme {
            let Some((event, close)) = self.splits.show(ctx, area, self.dialog_open, account, book)
            else {
                self.dialog = None;
                return None;
            };
            return if close {
                Some(Action::Split(SplitEvent::Close))
            } else {
                event.map(Action::Split)
            };
        }
        let response = theme::dialog(ctx, area, self.dialog_open, account.settings().theme);
        if response.inner.is_none() {
            self.dialog = None;
        }
        if response.close {
            return Some(Action::CloseDialog);
        }
        response.inner.flatten()
    }
}

/// Flips `setting` in `account`'s settings; returns whether it's now on.
fn toggle(setting: Toggle, account: &mut Account) -> bool {
    let on = !toggled(setting, account.settings());
    account.update_settings(|settings| match setting {
        Toggle::HyphenToBulletList => settings.convert_hyphen_to_bullet_list = on,
        Toggle::BacktickAsEscape => settings.enable_backtick_as_escape = Some(on),
        Toggle::SendAndDone => settings.send_and_done = on,
        Toggle::RsvpAndDone => settings.rsvp_and_done = on,
        Toggle::SenderFullNames => settings.show_sender_full_names = on,
        Toggle::QuickTips => settings.quick_tips.show = on,
        Toggle::Autocomplete => settings.typing_assistance.autocomplete_enabled = Some(on),
    });
    on
}

/// Whether `setting` is on in `settings`.
fn toggled(setting: Toggle, settings: &exo_store::Settings) -> bool {
    match setting {
        Toggle::HyphenToBulletList => settings.convert_hyphen_to_bullet_list,
        Toggle::BacktickAsEscape => settings.backtick_as_escape(),
        Toggle::SendAndDone => settings.send_and_done,
        Toggle::RsvpAndDone => settings.rsvp_and_done,
        Toggle::SenderFullNames => settings.show_sender_full_names,
        Toggle::QuickTips => settings.quick_tips.show,
        Toggle::Autocomplete => settings.autocomplete(),
    }
}

/// Registers the triage settings' commands for this frame: `Auto Advance Settings`, and the
/// `Enable …` / `Disable …` (`Show …` / `Hide …`) rows of the menu's toggles, titled by
/// `settings`.
pub fn register_triage<A: Clone + 'static>(
    commands: &mut Commands<A>,
    settings: &exo_store::Settings,
    wrap: impl Fn(Action) -> A,
) {
    let gear = |id: &'static str, title: &'static str, action: Action, score: f64| {
        Command::new(id, title, wrap(action))
            .default_score(score)
            .icon("data/settings")
    };
    commands.add(gear(
        "AUTO_ADVANCE_SETTINGS",
        "Auto Advance Settings",
        Action::Open(Dialog::AutoAdvance),
        0.95,
    ));
    let toggle = |id: &'static str, setting: Toggle, titles: [&'static str; 2]| {
        gear(
            id,
            titles[usize::from(toggled(setting, settings))],
            Action::Flip {
                setting,
                palette: true,
            },
            0.0,
        )
    };
    let on = |setting| toggled(setting, settings);
    let send = if on(Toggle::SendAndDone) {
        ["Disable Send and Mark Done", "Disable Send & Mark Done"]
    } else {
        ["Enable Send and Mark Done", "Enable Send & Mark Done"]
    };
    commands.add(
        toggle(
            "TOGGLE_SEND_AND_MARK_DONE",
            Toggle::SendAndDone,
            ["Enable Send + Mark Done", "Disable Send + Mark Done"],
        )
        .aliases(send),
    );
    let rsvp = if on(Toggle::RsvpAndDone) {
        ["Disable RSVP and Mark Done", "Disable RSVP & Mark Done"]
    } else {
        ["Enable RSVP and Mark Done", "Enable RSVP & Mark Done"]
    };
    commands.add(
        toggle(
            "TOGGLE_RSVP_AND_MARK_DONE",
            Toggle::RsvpAndDone,
            ["Enable RSVP + Mark Done", "Disable RSVP + Mark Done"],
        )
        .aliases(rsvp),
    );
    commands.add(toggle(
        "TOGGLE_SHOW_SENDERS_FULL_NAME",
        Toggle::SenderFullNames,
        ["Show Sender Full Names", "Hide Sender Full Names"],
    ));
    // The writing toggles (Backtick as Escape's id says `BACKSPACE`).
    commands.add(
        toggle(
            "TOGGLE_BACKSPACE_AS_ESCAPE",
            Toggle::BacktickAsEscape,
            ["Enable Backtick as Escape", "Disable Backtick as Escape"],
        )
        .aliases(["Enable Backtick as Escape", "Disable Backtick as Escape"]),
    );
    let hyphen = if on(Toggle::HyphenToBulletList) {
        "Disable Hyphen to Bullet List"
    } else {
        "Enable Hyphen to Bullet List"
    };
    commands.add(
        toggle(
            "TOGGLE_CONVERT_HYPHEN_TO_BULLET_LIST",
            Toggle::HyphenToBulletList,
            [
                "Enable Convert Hyphen to Bulleted List",
                "Disable Convert Hyphen to Bulleted List",
            ],
        )
        .aliases([hyphen]),
    );
    // Quick Tips, with the palette's own icon.
    commands.add(
        toggle(
            "TOGGLE_QUICK_TIPS",
            Toggle::QuickTips,
            ["Enable Quick Tips", "Disable Quick Tips"],
        )
        .icon("data/icon"),
    );
}

/// How to get the Quick Tips bar back once it's turned off.
pub(crate) fn quick_tips_off_hint() -> exo_commands::Hint {
    exo_commands::Hint::new()
        .text("Want to reenable Quick Tips? Hit")
        .keys(["⌘", "K"])
        .text("→ Quick Tips!")
}

/// Registers the typing commands (`Enable/Disable Autocorrect`, `Enable/Disable Autocomplete`,
/// `Autocorrect Settings`), titled by whether `autocorrect` and `autocomplete` are on.
pub fn register_typing<A: Clone + 'static>(
    commands: &mut Commands<A>,
    autocorrect: bool,
    autocomplete: bool,
    wrap: impl Fn(Action) -> A,
) {
    autocorrect::register(commands, autocorrect, autocomplete, wrap);
}

/// Carries out what the Notifications dialog changed, in `account`'s settings.
fn notify(event: NotificationsEvent, account: &mut Account) {
    match event {
        // Off on this device; on clears the account's setting too.
        NotificationsEvent::ToggleEmail => {
            let on = !account
                .settings()
                .email_notifications(account.device_settings());
            account.update_device_settings(|device| device.disable_notifications = !on);
            if on {
                account.update_settings(|settings| settings.disable_notifications = false);
            }
        }
        NotificationsEvent::ToggleCalendar(id) => {
            let holder = calendar_holder(account, &id);
            account.update_settings(|settings| {
                let on = !settings
                    .calendar_accounts
                    .get(&holder)
                    .and_then(|account| account.calendars.get(&id))
                    .is_some_and(exo_store::CalendarSetting::notifies);
                settings.update_calendar(&holder, &id, |calendar| {
                    calendar
                        .notifications
                        .get_or_insert_default()
                        .enabled_desktop = on
                });
            });
        }
        NotificationsEvent::Lead(seconds) => account.update_settings(|settings| {
            settings.notifications.calendar_events.lead_time_in_seconds = Some(seconds)
        }),
        NotificationsEvent::Recolor(id, color) => {
            let holder = calendar_holder(account, &id);
            account.update_settings(|settings| {
                settings.update_calendar(&holder, &id, |calendar| {
                    calendar.color = Some(color.name().to_owned())
                })
            });
        }
        // The list's and the picker's own, or the app's (`Enable a calendar`).
        NotificationsEvent::ToggleMenu
        | NotificationsEvent::Highlight(_)
        | NotificationsEvent::Swatch(..)
        | NotificationsEvent::PickerKey(_)
        | NotificationsEvent::Escape
        | NotificationsEvent::EnableCalendar => {}
    }
}

/// The calendar account calendar `id` is read through, whose settings it goes by (the mail
/// account for one on no list).
fn calendar_holder(account: &Account, id: &str) -> String {
    let data = account.calendar().data();
    let listed = data
        .calendars
        .iter()
        .flatten()
        .find(|calendar| calendar.entry.id.as_deref() == Some(id));
    listed.map_or_else(
        || account.email().to_owned(),
        |calendar| calendar.account.clone(),
    )
}

/// Whether the gear shows its new-setting dot.
pub fn badge(account: &Account) -> bool {
    account
        .device_settings()
        .new_setting_notification
        .as_deref()
        != Some(NEW_SETTING)
}

/// Makes `ctx` draw in the account's theme, following the system's appearance for Match System.
pub fn apply_theme(ctx: &Context, theme: Theme) {
    exo_ui::set_theme(ctx, computed(theme, ctx.system_theme()));
}

/// The theme `setting` draws with when the system appearance is `system`: Match System is Snow
/// when the system doesn't say.
pub fn computed(setting: Theme, system: Option<egui::Theme>) -> exo_ui::Theme {
    match setting {
        Theme::Snow => exo_ui::Theme::Snow,
        Theme::Carbon => exo_ui::Theme::Carbon,
        Theme::Auto => system.map_or(exo_ui::Theme::Snow, exo_ui::Theme::matching),
    }
}

/// Registers the settings commands for this frame, wrapping their actions with `wrap`.
/// `theme` is the account's setting: each `Set Theme` row shows only when it would change it.
/// Snow and Carbon are titled `Light` and `Dark`, with `Snow` and `Carbon` as aliases.
/// `email_notifications` is whether they're on, which the toggle's title says.
pub fn register<A: Clone + 'static>(
    commands: &mut Commands<A>,
    theme: Theme,
    email_notifications: bool,
    os: egui::os::OperatingSystem,
    wrap: impl Fn(Action) -> A,
) {
    let gear = |id: &'static str, title: &'static str, action: Action, score: f64| {
        Command::new(id, title, wrap(action))
            .default_score(score)
            .icon("data/settings")
    };
    commands.add(gear("SETTINGS", "Settings", Action::Show, 1.0));
    commands.add(
        gear(
            "THEME_SETTINGS",
            "Theme Settings",
            Action::Open(Dialog::Theme),
            0.95,
        )
        .aliases(["Appearance Settings", "Set Theme Style"]),
    );
    commands.add(
        gear(
            "NOTIFICATION_SETTINGS",
            "Notification Settings",
            Action::Open(Dialog::Notifications),
            0.0,
        )
        .follow("SEE_YOUR_WEEK")
        .aliases([
            "Event Notifications",
            "Email Notifications",
            "Calendar Notifications",
        ]),
    );
    commands.add(
        gear(
            "BLOCK_SETTINGS",
            "Blocked Senders",
            Action::Open(Dialog::BlockedSenders),
            0.95,
        )
        .aliases(["Settings"])
        .operative_word("block"),
    );
    commands.add(
        gear(
            "PRIVACY_SETTINGS",
            "Image Settings",
            Action::Open(Dialog::Images),
            0.95,
        )
        .follow("SHOW_IMAGES")
        .follow("INSERT_IMAGE")
        .aliases([
            "Settings",
            "Block Tracking",
            "Prevent Tracking",
            "Pixel Blocking",
            "Block Pixels",
            "Block Images",
        ]),
    );
    commands.add(
        gear(
            "AUTO_BCC_SETTINGS",
            "Auto Bcc Settings",
            Action::Open(Dialog::AutoBcc),
            0.95,
        )
        .scale(0.85)
        .aliases(["CRM", "HubSpot", "Salesforce", "Pipedrive"]),
    );
    commands.add(
        Command::new(
            "EMOJI_SKIN_TONE_SETTINGS",
            "Emoji Settings",
            wrap(Action::Open(Dialog::Emoji)),
        )
        .follow("USE_EMOJI")
        .aliases([
            "Set Default Emoji",
            "Personalize Emoji",
            "Customize Emoji",
            "Skin Tone Settings",
        ])
        .icon("data/emoji"),
    );
    commands.add(
        gear(
            "INSTANT_INTRO_SETTINGS",
            "Instant Intro Settings",
            Action::Open(Dialog::InstantIntro),
            0.0,
        )
        .aliases(["Custom Intro", "Intro Settings", "Move sender to BCC"]),
    );
    commands.add(
        gear(
            "REMINDER_SETTINGS",
            "Reminder Settings",
            Action::Open(Dialog::Reminders),
            1.0,
        )
        .scale(0.1)
        .aliases([
            "Remind Me Settings",
            "Snooze Settings",
            "Remind Settings",
            "Auto Reminders",
        ]),
    );
    // Its `score: 0.95` is a field the palette never reads: no default score.
    commands.add(
        gear(
            "ACCOUNT_SETTINGS",
            "Account Settings",
            Action::Open(Dialog::Accounts),
            0.0,
        )
        .scale(0.85)
        .aliases([
            "Accounts Settings",
            "Settings",
            "Account Colors",
            "Account Colours",
            "Account Shortcuts",
            "Account Order",
            "Reorder Accounts",
            "Manage Accounts",
            "Sign Out",
        ]),
    );

    let toggle = if email_notifications {
        "Disable Email Notifications"
    } else {
        "Enable Email Notifications"
    };
    commands.add(gear(
        "TOGGLE_NOTIFICATIONS",
        toggle,
        Action::ToggleEmailNotifications,
        0.0,
    ));
    commands.add(
        gear(
            "SIGNATURE_SETTINGS",
            "Signatures",
            Action::Open(Dialog::Signatures),
            0.98,
        )
        .aliases(["Settings"]),
    );
    commands.add(
        gear(
            "DOWNLOADS_SETTINGS",
            "Download Settings",
            Action::Open(Dialog::Downloads),
            0.95,
        )
        .aliases([
            "Download Destination",
            "Download Folder",
            "Download Location",
        ])
        .operative_word("download"),
    );

    let set = |id: &'static str, title: String, to: Theme| {
        Command::new(id, title, wrap(Action::SetTheme(to)))
            .default_score(1.5)
            .icon("data/icon")
    };
    if theme != Theme::Snow {
        let mut aliases = vec![
            "Set Theme: Snow",
            "Enable Snow Theme",
            "Enable Light Theme",
            "Enable Light Mode",
        ];
        if theme == Theme::Carbon {
            aliases.extend([
                "Disable Carbon Theme",
                "Disable Dark Theme",
                "Disable Dark Mode",
            ]);
        }
        commands
            .add(set("ENABLE_SNOW_THEME", "Set Theme: Light".into(), Theme::Snow).aliases(aliases));
    }
    if theme != Theme::Carbon {
        commands.add(
            set(
                "ENABLE_CARBON_THEME",
                "Set Theme: Dark".into(),
                Theme::Carbon,
            )
            .aliases([
                "Set Theme: Carbon",
                "Enable Carbon Theme",
                "Enable Dark Theme",
                "Enable Dark Mode",
            ]),
        );
    }
    if theme != Theme::Auto {
        commands.add(
            set(
                "MATCH_OS_THEME",
                format!("Set Theme: Match {}", os_name(os)),
                Theme::Auto,
            )
            .aliases(["Set Theme: Match Device"]),
        );
    }
    commands.add(
        Command::shortcut("CLOSE_DIALOG", wrap(Action::CloseDialog))
            .key("esc")
            .regions(DIALOG),
    );
}

/// How `Set Theme: Match …` names the operating system.
fn os_name(os: egui::os::OperatingSystem) -> &'static str {
    match os {
        egui::os::OperatingSystem::Mac => "macOS",
        egui::os::OperatingSystem::Windows => "Windows",
        _ => "operating system",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_system_follows_the_system_and_defaults_to_snow() {
        assert_eq!(
            computed(Theme::Auto, Some(egui::Theme::Dark)),
            exo_ui::Theme::Carbon
        );
        assert_eq!(
            computed(Theme::Auto, Some(egui::Theme::Light)),
            exo_ui::Theme::Snow
        );
        assert_eq!(computed(Theme::Auto, None), exo_ui::Theme::Snow);
        assert_eq!(
            computed(Theme::Carbon, Some(egui::Theme::Light)),
            exo_ui::Theme::Carbon
        );
        assert_eq!(
            computed(Theme::Snow, Some(egui::Theme::Dark)),
            exo_ui::Theme::Snow
        );
    }
}
