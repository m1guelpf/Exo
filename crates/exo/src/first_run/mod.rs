//! The first-run flow after the welcome splash: the discovery questions, account setup's loading
//! screen and split step, then Get Me To Zero for an inbox of more than ten conversations, or
//! `Welcome to Exo`. Setup ends the flow.
//!
//! It runs once for an account the first time it opens on this Mac (`do_show_tutorial`, set with
//! the splash), in Snow whatever the theme. Quitting in account setup opens it again at the next
//! launch, in the account's own theme, which is restored first; quitting in the questions doesn't
//! bring them back, and Get Me To Zero opens at the next launch instead.

mod discovery;
mod setup;

use std::sync::mpsc;

use egui::{Key, Rect, Ui, pos2, vec2};
use exo_commands::{Command, Commands, Region};
use exo_store::{Definitions, ListCount, ListId, SplitChanges, SplitInbox};
use exo_ui::{ALERT_TEXT, ALERT_TEXT_INK};

use crate::commands;

/// More conversations than this, or more than the store has, and setup ends in Get Me To Zero
/// (`ca.QOe`).
const GET_ME_TO_ZERO_OVER: usize = 10;
/// The welcome alert at the end of the flow. Its text is a `354px` box, so the first line doesn't
/// wrap at the alert's 300px.
const WELCOME_TITLE: &str = "Welcome to Exo";
const WELCOME_LINES: [&str; 2] = [
    "Get ready to regain focus, be more productive,",
    "and save hours every week!",
];
const GET_STARTED: &str = "Get Started";
/// The title's `<img>` of a rocket, 16px wide (Exo draws the system's emoji).
const ROCKET: &str = "🚀";
const ROCKET_SIZE: egui::Vec2 = vec2(16.0, 16.0);

/// What the flow's keys and buttons do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// `Get Started`, Enter, Space or Escape on `Welcome to Exo`.
    GetStarted,
}

/// A key the flow's screens take for themselves this frame: down or up, with or without Shift.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Press {
    pub key: Key,
    pub pressed: bool,
    pub shift: bool,
}

/// Takes this frame's Tab, Enter, Space, arrows and Escape (with no ⌘, ⌃ or ⌥) before anything
/// else sees them: the flow's screens answer them, on the way down and up. on the way down and up.
/// While `typing` in a field, Space and the arrows stay the field's.
fn take_keys(ctx: &egui::Context, typing: bool) -> Vec<Press> {
    let mut taken = Vec::new();
    ctx.input_mut(|input| {
        let mut space = false;
        input.events.retain(|event| {
            let egui::Event::Key {
                key,
                pressed,
                modifiers,
                ..
            } = *event
            else {
                // A taken Space's text goes with it.
                return !(space && matches!(event, egui::Event::Text(text) if text == " "));
            };
            let ours = matches!(key, Key::Tab | Key::Enter | Key::Escape)
                || (!typing
                    && matches!(
                        key,
                        Key::Space
                            | Key::ArrowUp
                            | Key::ArrowDown
                            | Key::ArrowLeft
                            | Key::ArrowRight
                    ));
            if !ours || modifiers.command || modifiers.ctrl || modifiers.alt {
                return true;
            }
            space |= key == Key::Space && pressed;
            taken.push(Press {
                key,
                pressed,
                shift: modifiers.shift,
            });
            false
        });
    });
    taken
}

/// The flow's state in a mail.
#[derive(Default)]
pub(crate) struct State {
    /// The discovery questions, before account setup.
    discovery: Option<discovery::Discovery>,
    setup: Option<setup::Setup>,
    /// The flow shows in Snow until it ends.
    snow: bool,
    /// The preview is being worked out.
    preview: Option<mpsc::Receiver<Option<exo_store::SplitPreview>>>,
    /// Setup is over and the inbox is being counted.
    count: Option<mpsc::Receiver<Option<ListCount>>>,
    /// `Welcome to Exo` is up.
    welcome: bool,
    /// The interrupted flow's Get Me To Zero has been considered this launch, and the inbox is
    /// being counted for it.
    interrupted_checked: bool,
    interrupted: Option<mpsc::Receiver<Option<ListCount>>>,
}

/// The flow's keys: the welcome's action (its screens take theirs, `take_keys`).
pub(crate) fn register(commands: &mut Commands<commands::Action>, state: &State) {
    let act = commands::Action::FirstRun;
    if state.welcome {
        commands.add(
            Command::shortcut("WELCOME_GET_STARTED", act(Action::GetStarted))
                .keys(["enter", "space", "esc"])
                .regions(&[Region::Alert]),
        );
    }
}

impl crate::Mail {
    /// Starts the first run once the welcome splash has given way (and at every launch after):
    /// account setup again if it was left half done, else the flow if it's due.
    pub(crate) fn maybe_start_first_run(&mut self) {
        if self.account.device_settings().account_setup_pending {
            self.open_account_setup();
        } else if self.account.settings().do_show_tutorial {
            // Marked as started, then on to the questions.
            self.account
                .update_settings(|settings| settings.do_show_tutorial = false);
            self.account
                .update_device_settings(|device| device.ftue_in_progress = true);
            self.open_discovery();
        }
    }

    /// Opens the discovery questions, as the flow does; public for screenshots and tests.
    pub fn open_discovery(&mut self) {
        self.first_run.snow = true;
        self.first_run.discovery = Some(discovery::Discovery::new(&self.ctx, &self.images));
    }

    /// The discovery questions are up. For tests.
    pub fn discovery_open(&self) -> bool {
        self.first_run.discovery.is_some()
    }

    /// Opens account setup, as the flow does; public for screenshots and tests.
    pub fn open_account_setup(&mut self) {
        self.account
            .update_device_settings(|device| device.account_setup_pending = true);
        let (labels, me) = (
            self.account.snapshot().labels.clone(),
            vec![self.account.email().to_owned()],
        );
        let definitions = Definitions {
            labels: &labels,
            me: &me,
        };
        // A Calendar split, off, if the account has none.
        if !self
            .account
            .settings()
            .splits()
            .any(|split| split.kind.as_deref() == Some("calendar"))
            && let Some(calendar) = SplitInbox::defaults(None, definitions)
                .into_iter()
                .find(|split| split.kind.as_deref() == Some("calendar"))
        {
            self.account
                .update_settings(|settings| settings.add_split(calendar));
        }
        let splits: Vec<SplitInbox> = self.account.settings().splits().cloned().collect();
        let candidates = self
            .settings
            .split_library()
            .map(|library| setup::candidates(library, &splits, definitions))
            .unwrap_or_default();
        self.first_run.setup = Some(setup::Setup::new(&self.ctx, &self.images, candidates));
        self.first_run.preview = None;
    }

    /// The flow covers the window, or its alert is up: its keys are the only ones.
    pub fn first_run_covers(&self) -> bool {
        self.first_run.discovery.is_some()
            || self.first_run.setup.is_some()
            || self.first_run.welcome
    }

    /// The flow's region, while it covers the window.
    pub(crate) fn first_run_region(&self) -> Option<Region> {
        if self.first_run.discovery.is_some() {
            Some(Region::FtueDiscoveryQuestions)
        } else if self.first_run.setup.is_some() {
            Some(Region::AccountSetup)
        } else {
            self.first_run.welcome.then_some(Region::Alert)
        }
    }

    /// The theme the flow forces, while it does.
    pub(crate) fn first_run_theme(&self) -> Option<exo_store::Theme> {
        self.first_run.snow.then_some(exo_store::Theme::Snow)
    }

    /// Nothing of the flow is being worked out. For screenshots and tests.
    pub(crate) fn first_run_idle(&self) -> bool {
        self.first_run.preview.is_none()
            && self.first_run.count.is_none()
            && self.first_run.interrupted.is_none()
            && self
                .first_run
                .setup
                .as_ref()
                .is_none_or(setup::Setup::is_idle)
            && self
                .first_run
                .discovery
                .as_ref()
                .is_none_or(discovery::Discovery::is_idle)
    }

    /// The names of account setup's splits and whether each is on, while its split step is up.
    /// For tests.
    pub fn account_setup_splits(&self) -> Option<Vec<(String, bool)>> {
        let splits = self.first_run.setup.as_ref()?.splits()?;
        Some(
            splits
                .rows
                .iter()
                .map(|row| (row.name.clone(), row.enabled))
                .collect(),
        )
    }

    /// Account setup is up. For tests.
    pub fn account_setup_open(&self) -> bool {
        self.first_run.setup.is_some()
    }

    /// `Welcome to Exo` is up. For tests.
    pub fn welcome_alert_open(&self) -> bool {
        self.first_run.welcome
    }

    /// Shows `Welcome to Exo`, as setup ends with ten conversations or fewer. For screenshots.
    pub fn welcome_to_exo(&mut self) {
        self.first_run.welcome = true;
    }

    /// Get Me To Zero is up. For tests.
    pub fn get_me_to_zero_open(&self) -> bool {
        self.bulk_dialogs.is_open()
    }

    /// Carries out `action`.
    pub(crate) fn first_run_run(&mut self, action: Action) {
        match action {
            Action::GetStarted => {
                // The list takes the keyboard on its first conversation.
                self.first_run.welcome = false;
                self.inbox.cursor_to_top();
            }
        }
    }

    /// Once a frame, before anything draws: the keys the flow's screens take, the preview's work,
    /// the inbox count setup ends on, and the interrupted flow's Get Me To Zero.
    pub(crate) fn first_run_frame(&mut self) {
        if let Some(discovery) = &mut self.first_run.discovery {
            discovery.take_keys(&self.ctx);
        } else if self.first_run.setup.is_some() {
            let now = self.ctx.input(|input| input.time);
            for press in take_keys(&self.ctx, false) {
                match self
                    .first_run
                    .setup
                    .as_mut()
                    .and_then(setup::Setup::splits_mut)
                    .and_then(|splits| splits.press(press, now))
                {
                    Some(setup::Event::Toggle(index)) => self.toggle_setup_split(index),
                    Some(setup::Event::Continue) => self.continue_setup(),
                    Some(setup::Event::Loaded) | None => {}
                }
            }
        }
        self.interrupted_first_run();
        // The preview is worked out once the first list is in.
        let wants = matches!(
            self.first_run.setup.as_ref().map(|setup| &setup.step),
            Some(setup::Step::Loading { preview: None, .. })
        );
        if wants && self.first_run.preview.is_none() && !self.snapshot.loading {
            let candidates: Vec<SplitInbox> = self
                .first_run
                .setup
                .iter()
                .flat_map(|setup| {
                    setup
                        .candidates
                        .iter()
                        .map(|candidate| candidate.split.clone())
                })
                .collect();
            let (sender, receiver) = mpsc::channel();
            let ctx = self.ctx.clone();
            self.account.loader().run(
                move |store| async move {
                    store
                        .split_preview(&candidates, 10)
                        .await
                        .inspect_err(|error| log::warn!("account setup's preview: {error}"))
                        .ok()
                },
                move |preview| {
                    let _ = sender.send(preview);
                    ctx.request_repaint();
                },
            );
            self.first_run.preview = Some(receiver);
        }
        if let Some(preview) = self
            .first_run
            .preview
            .as_ref()
            .and_then(|receiver| receiver.try_recv().ok())
        {
            self.first_run.preview = None;
            if let Some(setup::Step::Loading { preview: slot, .. }) =
                self.first_run.setup.as_mut().map(|setup| &mut setup.step)
            {
                *slot = Some(preview.unwrap_or_default());
            }
        }
        if let Some(count) = self
            .first_run
            .count
            .as_ref()
            .and_then(|receiver| receiver.try_recv().ok())
        {
            self.first_run.count = None;
            if ends_in_get_me_to_zero(count) {
                // Get Me To Zero has opened by itself.
                self.account
                    .update_device_settings(|device| device.gmtz_auto_shown = true);
                self.bulk_run(crate::bulk::Action::OpenGetMeToZero);
            } else {
                self.first_run.welcome = true;
            }
        }
    }

    /// Opens Get Me To Zero after an interrupted first run: the first frame of a launch the list is
    /// up with nothing over it, a first-run flow that started and never reached setup's end (it was
    /// quit in the questions), with Get Me To Zero never run nor opened by itself, opens it on an
    /// inbox of more than ten conversations.
    fn interrupted_first_run(&mut self) {
        let free = self.welcome.is_none()
            && !self.first_run_covers()
            && self.thread.is_none()
            && matches!(
                self.commands.region(),
                Region::ThreadList | Region::ThreadListHeader
            );
        if !self.first_run.interrupted_checked && free && !self.account.settings().do_show_tutorial
        {
            self.first_run.interrupted_checked = true;
            let device = self.account.device_settings();
            let started = device.gmtz_started
                || self.account.settings().statistics.gmtz_archived_all_time > 0;
            if device.ftue_in_progress && !started && !device.gmtz_auto_shown {
                let (sender, receiver) = mpsc::channel();
                let ctx = self.ctx.clone();
                self.account.loader().run(
                    |store| async move {
                        store
                            .count(&ListId::Label("INBOX".to_owned()))
                            .await
                            .inspect_err(|error| {
                                log::warn!(
                                    "counting the inbox after an interrupted first run: {error}"
                                )
                            })
                            .ok()
                    },
                    move |count| {
                        let _ = sender.send(count);
                        ctx.request_repaint();
                    },
                );
                self.first_run.interrupted = Some(receiver);
            }
        }
        if let Some(count) = self
            .first_run
            .interrupted
            .as_ref()
            .and_then(|receiver| receiver.try_recv().ok())
        {
            self.first_run.interrupted = None;
            // An inbox that couldn't be counted isn't a backlog.
            if count.is_some() && ends_in_get_me_to_zero(count) && free {
                self.account.update_device_settings(|device| {
                    device.gmtz_auto_shown = true;
                    device.ftue_in_progress = false;
                });
                self.bulk_run(crate::bulk::Action::OpenGetMeToZero);
            }
        }
    }

    /// Draws the discovery questions or account setup over `window` with the tab strip on them,
    /// and the welcome over `page`.
    pub(crate) fn show_first_run(
        &mut self,
        ui: &mut Ui,
        window: Rect,
        page: Rect,
        tab: &crate::chrome::Tab<'_>,
    ) {
        let ctx = ui.ctx().clone();
        if let Some(discovery) = &mut self.first_run.discovery {
            let strip_height = crate::chrome::tab_bar_height(&ctx);
            let answered = egui::Area::new(exo_ui::id(&ctx, "discovery"))
                .order(egui::Order::Foreground)
                .fixed_pos(window.min)
                .show(&ctx, |ui| {
                    ui.set_clip_rect(window);
                    let answered = discovery.show(ui, window, strip_height);
                    let mut strip = ui.new_child(egui::UiBuilder::new().max_rect(window));
                    crate::chrome::show(
                        &mut strip,
                        crate::chrome::Tab {
                            title: tab.title,
                            login_screen: tab.login_screen,
                            account_color: tab.account_color,
                        },
                        crate::chrome::Backdrop::Page,
                    );
                    answered
                });
            if let Some(answered) = answered.inner {
                self.account
                    .update_settings(|settings| settings.discovery_questions = answered.answers);
                if answered.last {
                    // On to account setup.
                    self.first_run.discovery = None;
                    self.open_account_setup();
                }
            }
        }
        if let Some(setup) = &mut self.first_run.setup {
            let zone = self.now.time_zone().clone();
            let strip_height = crate::chrome::tab_bar_height(&ctx);
            let event = egui::Area::new(exo_ui::id(&ctx, "account-setup"))
                .order(egui::Order::Foreground)
                .fixed_pos(window.min)
                .show(&ctx, |ui| {
                    ui.set_clip_rect(window);
                    let event = setup.show(ui, window, strip_height, &zone);
                    let mut strip = ui.new_child(egui::UiBuilder::new().max_rect(window));
                    crate::chrome::show(
                        &mut strip,
                        crate::chrome::Tab {
                            title: tab.title,
                            login_screen: tab.login_screen,
                            account_color: tab.account_color,
                        },
                        crate::chrome::Backdrop::Page,
                    );
                    event
                });
            match event.inner {
                Some(setup::Event::Loaded) => self.setup_loaded(),
                Some(setup::Event::Toggle(index)) => self.toggle_setup_split(index),
                Some(setup::Event::Continue) => self.continue_setup(),
                None => {}
            }
        }
        if self.first_run.welcome {
            self.show_welcome_alert(ui, page);
        }
    }

    /// The loading screen is done: the split step if a split takes some of the inbox, its three on,
    /// else the end of setup.
    fn setup_loaded(&mut self) {
        let now = self.ctx.input(|input| input.time);
        let Some(setup) = &mut self.first_run.setup else {
            return;
        };
        let preview = match &mut setup.step {
            setup::Step::Loading { preview, .. } => preview.take().unwrap_or_default(),
            setup::Step::Splits(_) => return,
        };
        let Some(mut splits) = setup::Splits::new(&setup.candidates, preview, now) else {
            self.finish_setup();
            return;
        };
        let (labels, me) = (
            self.account.snapshot().labels.clone(),
            vec![self.account.email().to_owned()],
        );
        for row in &mut splits.rows {
            let Some(candidate) = setup
                .candidates
                .iter()
                .find(|candidate| candidate.split.id == row.id)
            else {
                continue;
            };
            if !candidate.existing {
                let split = SplitInbox {
                    is_disabled: false,
                    ..candidate.split.clone()
                };
                self.account
                    .update_settings(|settings| settings.add_split(split));
            } else if candidate.split.is_disabled {
                // Turning one on mints it a new id.
                let on = candidate.split.update(
                    &SplitChanges {
                        is_disabled: Some(false),
                        ..SplitChanges::default()
                    },
                    Definitions {
                        labels: &labels,
                        me: &me,
                    },
                );
                row.id.clone_from(&on.id);
                self.account
                    .update_settings(|settings| settings.replace_split(&candidate.split.id, on));
            }
        }
        setup.step = setup::Step::Splits(splits);
    }

    /// A row's toggle: no toast, the split turned on or off where it is.
    fn toggle_setup_split(&mut self, index: usize) {
        let now = self.ctx.input(|input| input.time);
        let Some(id) = self
            .first_run
            .setup
            .as_ref()
            .and_then(setup::Setup::splits)
            .and_then(|splits| splits.rows.get(index))
            .map(|row| row.id.clone())
        else {
            return;
        };
        let Some(split) = self.account.settings().split(&id).cloned() else {
            return;
        };
        let (labels, me) = (
            self.account.snapshot().labels.clone(),
            vec![self.account.email().to_owned()],
        );
        let toggled = split.update(
            &SplitChanges {
                is_disabled: Some(!split.is_disabled),
                ..SplitChanges::default()
            },
            Definitions {
                labels: &labels,
                me: &me,
            },
        );
        let new_id = toggled.id.clone();
        self.account
            .update_settings(|settings| settings.replace_split(&id, toggled));
        if let Some(setup::Step::Splits(splits)) =
            self.first_run.setup.as_mut().map(|setup| &mut setup.step)
        {
            splits.toggled(index, new_id, now);
        }
    }

    /// `Continue with this setup`: every other split setup could have offered is turned off, and
    /// Important and Other on.
    fn continue_setup(&mut self) {
        let Some(splits) = self.first_run.setup.as_ref().and_then(setup::Setup::splits) else {
            return;
        };
        let shown: Vec<String> = splits.rows.iter().map(|row| row.id.clone()).collect();
        // The splits setup could offer: on, with a definition, not VIP or Team (or News without
        // senders).
        let others: Vec<SplitInbox> = self
            .account
            .settings()
            .enabled_splits()
            .filter(|split| {
                !matches!(
                    split.kind.as_deref(),
                    Some("vip" | "team" | "shared" | "news")
                ) && !split.query().trim().is_empty()
                    && !shown.contains(&split.id)
            })
            .cloned()
            .collect();
        let (labels, me) = (
            self.account.snapshot().labels.clone(),
            vec![self.account.email().to_owned()],
        );
        for split in others {
            let off = split.update(
                &SplitChanges {
                    is_disabled: Some(true),
                    ..SplitChanges::default()
                },
                Definitions {
                    labels: &labels,
                    me: &me,
                },
            );
            self.account
                .update_settings(|settings| settings.replace_split(&split.id, off));
        }
        if self.account.settings().unified_inbox {
            // Important and Other on, which shows Important.
            self.account
                .update_settings(exo_store::Settings::toggle_important_other);
            let now = self.ctx.input(|input| input.time);
            self.run(commands::Action::Go(ListId::Important), now);
        }
        self.finish_setup();
    }

    /// Finishes setup: it closes, the theme comes back, the palette's first-run nudge is queued,
    /// and the inbox is counted for what comes next.
    fn finish_setup(&mut self) {
        self.first_run.setup = None;
        self.first_run.snow = false;
        // The flow is no longer in progress.
        self.account.update_device_settings(|device| {
            device.account_setup_pending = false;
            device.ftue_in_progress = false;
        });
        self.command_nudge_first_open();
        let (sender, receiver) = mpsc::channel();
        let ctx = self.ctx.clone();
        self.account.loader().run(
            // Every inbox conversation, the splits' too.
            |store| async move {
                store
                    .count(&ListId::Label("INBOX".to_owned()))
                    .await
                    .inspect_err(|error| log::warn!("counting the inbox after setup: {error}"))
                    .ok()
            },
            move |count| {
                let _ = sender.send(count);
                ctx.request_repaint();
            },
        );
        self.first_run.count = Some(receiver);
    }

    /// `Welcome to Exo`, over `page` dimmed.
    fn show_welcome_alert(&mut self, ui: &mut Ui, page: Rect) {
        // The overlay: the page dimmed to half, nothing behind it clickable.
        ui.painter()
            .rect_filled(page, 0.0, egui::Color32::from_black_alpha(128));
        let _ = ui.interact(
            page,
            ui.id().with("welcome-alert-overlay"),
            egui::Sense::click(),
        );
        let alert = exo_ui::Alert {
            title: WELCOME_TITLE,
            text: "",
            actions: &[GET_STARTED],
            selected: 0,
        };
        let height = WELCOME_LINES.len() as f32 * ALERT_TEXT.line_height;
        let clicked =
            alert.show_body_with_title_emoji(ui, page, ROCKET, ROCKET_SIZE, height, |ui, body| {
                for (index, line) in WELCOME_LINES.iter().enumerate() {
                    ALERT_TEXT.paint_line(
                        ui.painter(),
                        pos2(
                            body.left(),
                            body.top() + index as f32 * ALERT_TEXT.line_height,
                        ),
                        line,
                        ALERT_TEXT_INK,
                        None,
                    );
                }
            });
        if clicked.is_some() {
            self.first_run_run(Action::GetStarted);
        }
    }
}

/// Get Me To Zero for an inbox of more than ten conversations or more than the store has, else the
/// welcome. An inbox that couldn't be counted gets Get Me To Zero, as an unknown size isn't a small
/// one.
fn ends_in_get_me_to_zero(count: Option<ListCount>) -> bool {
    count.is_none_or(|count| !count.exact || count.threads > GET_ME_TO_ZERO_OVER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_inbox_the_store_has_all_of_is_welcomed() {
        let count = |threads, exact| {
            Some(ListCount {
                threads,
                unread: 0,
                exact,
            })
        };
        assert!(!ends_in_get_me_to_zero(count(10, true)));
        assert!(!ends_in_get_me_to_zero(count(0, true)));
        assert!(ends_in_get_me_to_zero(count(11, true)));
        assert!(
            ends_in_get_me_to_zero(count(3, false)),
            "Gmail has more than the store"
        );
        assert!(ends_in_get_me_to_zero(None));
    }
}
