//! Desktop notifications: new mail and calendar reminders, posted to macOS, and their clicks
//! handed back to the app; and an update to Exo ready to install.
//!
//! Exo raises new mail from its own sync ([`mail`]), and calendar reminders ([`reminders`]).
//! Either way notifications go through [`Notifier::post`]: macOS asks for permission the first
//! time Exo has one to show, and a click comes back through the [`Launcher`], which brings the
//! window back and opens what the notification was about ([`Click`]). The updater's goes through
//! [`Notifier::post_update`], and a click on it installs the update.

pub mod mail;
pub mod reminders;

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, PoisonError},
};

use jiff::civil::Date;

use crate::desktop::Launcher;

/// The button of a calendar reminder whose event has a call.
pub const JOIN_MEETING: &str = "Join meeting";
/// The button's id, for the OS.
const JOIN: &str = "join";
/// The key of the notification of an update ready to install: a newer one replaces it.
const UPDATE: &str = "UPDATE";
/// Its buttons' ids, for the OS.
const INSTALL: &str = "install";
const LATER: &str = "later";
/// How many notifications keep listening for their clicks: older ones stay in Notification
/// Center, but a click on one does nothing (fastframe-notifications' advice, as ZapFast does).
const LISTENING: usize = 32;

/// A notification to post.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// Its key: `EMAIL-{message}` or `CALENDAR-{event}`. A notice whose key is showing
    /// replaces it.
    pub key: String,
    /// The title.
    pub title: String,
    /// The line macOS shows under the title.
    pub subtitle: Option<String>,
    /// The body.
    pub body: String,
    /// The `Join meeting` button, and what it does.
    pub button: Option<Click>,
    /// What clicking the notification does.
    pub click: Click,
}

impl Notice {
    /// The OS's notification: its title, subtitle, body and key, and the button.
    fn notification(&self) -> fastframe_notifications::Notification {
        let notification = fastframe_notifications::Notification::new(&self.title)
            .subtitle(self.subtitle.as_deref().unwrap_or_default())
            .body(&self.body)
            .key(&self.key);
        if self.button.is_some() {
            notification.action(JOIN, JOIN_MEETING)
        } else {
            notification
        }
    }
}

/// What a click on a notification opens, in the account it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Click {
    /// The account's address.
    pub account: String,
    /// What opens there.
    pub open: Open,
}

/// What a click opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Open {
    /// The conversation, on the message: from home's list, or more of it if it's open already.
    Thread {
        /// The thread.
        thread: String,
        /// The message the notification was about.
        message: String,
    },
    /// The week view on the event's day with its details up; with `url`, the call opens too
    /// (`Join meeting`).
    Event {
        /// The event's key on the calendar ([`crate::calendar`]'s `Event::key`).
        key: String,
        /// Its day.
        day: Date,
        /// The hour it ends at, to scroll the grid to.
        end_hour: i8,
        /// The conference link to open.
        url: Option<String>,
    },
}

/// Posts notifications for the accounts, and hands their clicks to the [`Launcher`]. One per
/// process; cheap to clone, and usable from any thread.
#[derive(Clone)]
pub struct Notifier(Arc<Inner>);

struct Inner {
    /// The OS's notifications; none in a test's notifier, which only records.
    client: Option<fastframe_notifications::Client>,
    /// What a test's notifier was given.
    posted: Mutex<Vec<Notice>>,
    launcher: Launcher,
    /// Notifications still listening for their clicks, newest last.
    listening: Mutex<VecDeque<(String, fastframe_notifications::Handle)>>,
    /// One permission prompt at a time.
    asking: tokio::sync::Mutex<()>,
    /// Said once that macOS won't show Exo's notifications.
    refused: std::sync::atomic::AtomicBool,
}

impl Notifier {
    /// Posts as the app bundle `bundle_id` (macOS only shows a bundle's notifications), handing
    /// clicks to `launcher`.
    ///
    /// # Errors
    /// When the notification service can't start.
    pub fn new(
        bundle_id: &str,
        launcher: Launcher,
    ) -> Result<Self, fastframe_notifications::Error> {
        let client = fastframe_notifications::Client::new(fastframe_notifications::App::new(
            bundle_id, "Exo",
        ))?;
        Ok(Self::with(Some(client), launcher))
    }

    fn with(client: Option<fastframe_notifications::Client>, launcher: Launcher) -> Self {
        Self(Arc::new(Inner {
            client,
            posted: Mutex::new(Vec::new()),
            launcher,
            listening: Mutex::new(VecDeque::new()),
            asking: tokio::sync::Mutex::new(()),
            refused: std::sync::atomic::AtomicBool::new(false),
        }))
    }

    /// A notifier that only keeps what it's given, for tests.
    pub fn recording(launcher: Launcher) -> Self {
        Self::with(None, launcher)
    }

    /// What a [`recording`](Self::recording) notifier was given.
    pub fn posted(&self) -> Vec<Notice> {
        self.0
            .posted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Shows `notices` in order, asking macOS for permission first if it never has; without
    /// permission none shows. They go on a task of their own, so a permission prompt waiting for
    /// its answer holds up nothing else: call from a Tokio runtime (an account's).
    pub fn post(&self, notices: Vec<Notice>) {
        let Some(client) = self.0.client.clone().filter(|_| !notices.is_empty()) else {
            self.0
                .posted
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(notices);
            return;
        };
        let notifier = self.clone();
        tokio::spawn(async move {
            if notifier.allowed(&client).await {
                for notice in notices {
                    notifier.show(&client, notice);
                }
            }
        });
    }

    /// Shows `notice`, listening for its clicks.
    fn show(&self, client: &fastframe_notifications::Client, notice: Notice) {
        let (launcher, withdraw, key) =
            (self.0.launcher.clone(), client.clone(), notice.key.clone());
        let (click, button) = (notice.click.clone(), notice.button.clone());
        // A click opens what it's about and clears the notification.
        let shown = client.show_with(notice.notification(), move |event| {
            let clicked = match event {
                fastframe_notifications::Event::Activated => Some(click.clone()),
                fastframe_notifications::Event::Action(id) if id == JOIN => button.clone(),
                _ => None,
            };
            if let Some(clicked) = clicked {
                launcher.clicked(clicked);
                let _ = withdraw.withdraw(key.clone());
            }
        });
        match shown {
            Ok(handle) => self.listen(notice.key, handle),
            Err(error) => log::warn!("showing a notification: {error}"),
        }
    }

    /// Says the update to `version` is downloaded: `Update ready!`, with `Install and Restart`,
    /// which a click on the notification does too, and `Not now`. Asks macOS for permission
    /// first if it never has, and waits for its answer: call off the UI thread and outside a
    /// Tokio runtime (the updater's thread).
    pub fn post_update(&self, version: &str) {
        let Some(client) = self.0.client.clone() else {
            return;
        };
        if !futures::executor::block_on(self.allowed(&client)) {
            return;
        }
        let launcher = self.0.launcher.clone();
        let shown = client.show_with(update_notification(version), move |event| match event {
            fastframe_notifications::Event::Activated => launcher.install_update(),
            fastframe_notifications::Event::Action(id) if id == INSTALL => {
                launcher.install_update();
            }
            _ => {}
        });
        match shown {
            Ok(handle) => self.listen(UPDATE.to_owned(), handle),
            Err(error) => log::warn!("showing a notification: {error}"),
        }
    }

    /// Keeps listening to the clicks on the notification `key` through `handle`, in place of an
    /// earlier one with that key.
    fn listen(&self, key: String, handle: fastframe_notifications::Handle) {
        let mut listening = self
            .0
            .listening
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        listening.retain(|(listened, _)| *listened != key);
        listening.push_back((key, handle));
        if listening.len() > LISTENING {
            listening.pop_front();
        }
    }

    /// Whether macOS shows Exo's notifications, after asking when it never has.
    async fn allowed(&self, client: &fastframe_notifications::Client) -> bool {
        use fastframe_notifications::Permission;
        let _asking = self.0.asking.lock().await;
        let permission = match client.permission().await {
            Ok(Permission::NotDetermined) => client.request_permission().await,
            permission => permission,
        };
        match permission {
            Ok(Permission::Granted | Permission::Unknown) => true,
            refused => {
                if !self
                    .0
                    .refused
                    .swap(true, std::sync::atomic::Ordering::Relaxed)
                {
                    log::info!("notifications won't show: {refused:?}");
                }
                false
            }
        }
    }
}

impl crate::Mail {
    /// Opens what a clicked notification is about: any dialog goes, then the conversation opens
    /// from home's list on the message, marked read, or the week shows the event.
    pub fn open_notification(&mut self, open: Open) {
        let now = self.ctx.input(|input| input.time);
        if self.settings.dialog_open() {
            self.run(
                crate::commands::Action::Settings(crate::settings::Action::CloseDialog),
                now,
            );
        }
        match open {
            Open::Thread { thread, message } => {
                if self.thread.as_ref().is_none_or(|open| open.id() != thread) {
                    self.calendar_run(crate::calendar::Action::CloseWeek);
                    self.run(
                        crate::commands::Action::Go(crate::splits::home(self.account.settings())),
                        now,
                    );
                    self.open_thread(thread.clone());
                    // Home's rows may not be in yet to say it's unread; on a read thread this
                    // changes nothing.
                    self.account.apply(
                        std::slice::from_ref(&thread),
                        exo_store::Mutation::MarkRead,
                        None,
                    );
                }
                if let Some(view) = &mut self.thread {
                    view.focus_message(&message);
                }
            }
            Open::Event {
                key,
                day,
                end_hour,
                url,
            } => {
                self.calendar_open_event(key, day, end_hour);
                if let Some(url) = url {
                    self.open_url(&url);
                }
            }
        }
    }
}

/// The notification of the update to `version`, ready to install: what the dialog asking to
/// install it says.
fn update_notification(version: &str) -> fastframe_notifications::Notification {
    let ready = crate::update::Dialog::Ready(version.to_owned());
    fastframe_notifications::Notification::new(ready.title())
        .body(ready.body())
        .key(UPDATE)
        .action(INSTALL, crate::update::INSTALL_AND_RESTART)
        .action(LATER, crate::update::NOT_NOW)
}

/// The bundle id Exo runs as, when it's an app bundle (`Exo.app`); a bare binary has none, and
/// posts no notifications.
pub fn bundle_id() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let bundle = objc2_foundation::NSBundle::mainBundle();
        // A bare binary's main bundle is its folder, which has no Info.plist and no id.
        bundle.bundleIdentifier().map(|id| id.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(subtitle: Option<&str>) -> Notice {
        Notice {
            key: "EMAIL-m1".into(),
            title: "Priya Raman".into(),
            subtitle: subtitle.map(str::to_owned),
            body: "Launch checklist".into(),
            button: None,
            click: Click {
                account: "maya@brightmoor.co".into(),
                open: Open::Thread {
                    thread: "t1".into(),
                    message: "m1".into(),
                },
            },
        }
    }

    // fastframe-notifications keeps a notification's fields to itself; its `Debug` shows them.
    #[test]
    fn the_subtitle_stays_out_of_the_body() {
        let shown = format!(
            "{:?}",
            notice(Some("Can you look at the checklist before 3?")).notification()
        );
        assert!(
            shown.contains(
                r#"subtitle: "Can you look at the checklist before 3?", body: "Launch checklist""#
            ),
            "{shown}"
        );
        let shown = format!("{:?}", notice(None).notification());
        assert!(
            shown.contains(r#"subtitle: "", body: "Launch checklist""#),
            "{shown}"
        );
    }

    #[test]
    fn an_update_ready_installs_from_the_notification() {
        let shown = format!("{:?}", update_notification("0.2.0"));
        for part in [
            r#"title: "Update ready!""#,
            r#"body: "Restart to start using Exo 0.2.0.""#,
            r#""install", "Install and Restart""#,
            r#""later", "Not now""#,
        ] {
            assert!(shown.contains(part), "{part} in {shown}");
        }
    }
}
