//! Alerts that only tell you something, with one button: ⌘K's `Version Info`, which ⌘C copies,
//! a deep link Exo doesn't know's `Update Required`, and the alerts of ⌘K's `Check for Updates`
//! (also the `upgrade` deep link's), which say how the updater's check goes
//! ([`crate::update`]). Enter, ⌘Enter, Space and Escape close them; nothing else gets through
//! while one is up. Also ⌘K's `Open Source Attribution`, which opens Exo's notices.

use egui::{Color32, Rect, pos2};
use exo_commands::{Command, Commands, Hint, Region};
use exo_ui::{ALERT_TEXT, ALERT_TEXT_INK, Family, Font, TextStyle};

use crate::commands;
use crate::update::{self, Mode};

/// `- Version: …`: Exo's version.
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The copy hint under the text: `rgba(0, 0, 0, .5)` in both themes, on the action row's line.
const SUBTEXT_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 128);
const SUBTEXT_BELOW_TEXT: f32 = 16.0;
/// `Update Required`'s `<strong>`s.
const STRONG: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 14.0, 20.0);
/// `Update Required`'s text, its `<strong>`s marked, a line each as Chromium breaks them at the
/// alert's 300px.
const UPDATE_REQUIRED: [&[(&str, bool)]; 3] = [
    &[
        ("To see this feature, hit ", false),
        ("Cmd+K", true),
        (" (Mac) or ", false),
        ("Ctrl+K", true),
    ],
    &[
        ("(Windows) → ", false),
        ("Check for Updates", true),
        (" to get on the", false),
    ],
    &[("latest version of Exo.", false)],
];
/// Where an alert's keys work.
const ALERT: &[Region] = &[Region::Alert];

/// An alert on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    /// ⌘K's `Version Info`.
    VersionInfo,
    /// A deep link to something Exo doesn't have.
    UpdateRequired,
    /// How ⌘K's `Check for Updates` goes.
    Update(update::Alert),
}

/// Something about these alerts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// ⌘K's `Version Info`.
    VersionInfo,
    /// ⌘K's `Open Source Attribution`: Exo's third-party notices.
    OpenSourceAttribution,
    /// ⌘K's `Check for Updates`, and the `upgrade` deep link.
    CheckForUpdates,
    /// `OK`, Enter, Space or Escape.
    Dismiss,
    /// ⌘C on `Version Info`.
    Copy,
}

impl Notice {
    fn title(self) -> &'static str {
        match self {
            Self::VersionInfo => "Version Info",
            Self::UpdateRequired => "Update Required",
            Self::Update(_) => update::ALERT_TITLE,
        }
    }

    /// Its button.
    fn action(self) -> &'static str {
        match self {
            Self::VersionInfo | Self::UpdateRequired => "OK",
            Self::Update(alert) => alert.action(),
        }
    }

    /// Its text, a line per `\n`.
    pub fn text(self) -> String {
        match self {
            // The version alone.
            Self::VersionInfo => format!("- Version: {VERSION}"),
            Self::UpdateRequired => "To see this feature, hit Cmd+K (Mac) or Ctrl+K (Windows) → Check for Updates to get on the latest version of Exo.".to_owned(),
            Self::Update(alert) => alert.text(),
        }
    }
}

/// `Version Info`, `Open Source Attribution` and, when Exo updates itself, `Check for Updates`;
/// and the open alert's keys.
pub(crate) fn register(
    commands: &mut Commands<commands::Action>,
    open: Option<Notice>,
    updates: bool,
) {
    let act = commands::Action::Notice;
    commands.add(
        Command::new("VERSION", "Version Info", act(Action::VersionInfo))
            .default_score(0.0)
            .icon("data/icon"),
    );
    commands.add(
        Command::new(
            "OPEN_SOURCE_LINK",
            "Open Source Attribution",
            act(Action::OpenSourceAttribution),
        )
        .default_score(0.0)
        .icon("data/open-links-link"),
    );
    if updates {
        commands.add(
            Command::new("UPGRADE", "Check for Updates", act(Action::CheckForUpdates))
                .aliases(["Update Exo", "Upgrade Version"])
                .operative_word("update")
                .default_score(0.9)
                .icon("data/icon"),
        );
    }
    if let Some(open) = open {
        commands.add(
            Command::shortcut("ALERT_OK", act(Action::Dismiss))
                .keys(["enter", "mod+enter", "space", "esc"])
                .regions(ALERT),
        );
        if open == Notice::VersionInfo {
            commands.add(
                Command::shortcut("ALERT_COPY", act(Action::Copy))
                    .key("mod+c")
                    .regions(ALERT),
            );
        }
    }
}

/// Where Exo's third-party notices are: in `Exo.app`'s resources (`fastlane build`), else the
/// checkout's.
fn notices() -> std::path::PathBuf {
    let bundled = std::env::current_exe().ok().and_then(|exe| {
        Some(
            exe.parent()?
                .parent()?
                .join("Resources/THIRD_PARTY_NOTICES.md"),
        )
    });
    bundled.filter(|file| file.is_file()).unwrap_or_else(|| {
        std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../THIRD_PARTY_NOTICES.md"
        ))
    })
}

impl crate::Mail {
    /// Carries out `action`.
    pub(crate) fn notice_run(&mut self, action: Action) {
        match action {
            Action::VersionInfo => self.notice = Some(Notice::VersionInfo),
            // The notices are a file in the app.
            Action::OpenSourceAttribution => self.ctx.open_url(egui::OpenUrl::new_tab(format!(
                "file://{}",
                notices().display()
            ))),
            // The check's alert takes the place of any other.
            Action::CheckForUpdates => {
                self.notice = None;
                if let Some(updates) = &self.updates {
                    updates.check(Mode::Alerts);
                }
            }
            Action::Dismiss => match self.notice() {
                Some(Notice::Update(_)) => {
                    if let Some(updates) = &self.updates {
                        updates.dismiss();
                    }
                }
                _ => self.notice = None,
            },
            Action::Copy => {
                if let Some(notice) = self.notice {
                    self.ctx.copy_text(notice.text());
                    self.commands
                        .hint(Hint::new().text("Alert copied to clipboard."));
                }
            }
        }
    }

    /// The alert on screen, if any: the updater's check's, over any other.
    pub fn notice(&self) -> Option<Notice> {
        self.updates
            .as_ref()
            .and_then(update::Updates::alert)
            .map(Notice::Update)
            .or(self.notice)
    }

    /// Checks for updates with `updates` (`None`: Exo doesn't update itself, and ⌘K has no
    /// `Check for Updates`).
    pub fn set_updates(&mut self, updates: Option<update::Updates>) {
        self.updates = updates;
    }

    /// Draws the alert, if one is up, over `page` dimmed.
    pub(crate) fn show_notice(&mut self, ui: &mut egui::Ui, page: Rect) {
        let Some(notice) = self.notice() else { return };
        // The page dimmed to half, nothing behind it clickable.
        ui.painter()
            .rect_filled(page, 0.0, Color32::from_black_alpha(128));
        let _ = ui.interact(page, ui.id().with("notice-overlay"), egui::Sense::click());
        let text = notice.text();
        let alert = exo_ui::Alert {
            title: notice.title(),
            text: &text,
            actions: &[notice.action()],
            selected: 0,
        };
        let clicked = match notice {
            // The copy hint, on the action row's line at the text's left.
            Notice::VersionInfo => {
                let lines: Vec<&str> = text.split('\n').collect();
                let ctx = ui.ctx().clone();
                alert.show_body(
                    ui,
                    page,
                    lines.len() as f32 * ALERT_TEXT.line_height,
                    |ui, body| {
                        for (index, line) in lines.iter().enumerate() {
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
                        let subtext =
                            format!("Press {} C to copy", exo_ui::keycap_label("mod", ctx.os()));
                        ALERT_TEXT.paint_line(
                            ui.painter(),
                            pos2(body.left(), body.bottom() + SUBTEXT_BELOW_TEXT),
                            &subtext,
                            SUBTEXT_INK,
                            None,
                        );
                    },
                )
            }
            Notice::UpdateRequired => alert.show_body(
                ui,
                page,
                UPDATE_REQUIRED.len() as f32 * ALERT_TEXT.line_height,
                |ui, body| {
                    for (index, runs) in UPDATE_REQUIRED.iter().enumerate() {
                        let mut x = body.left();
                        for (run, strong) in *runs {
                            let style = if *strong { STRONG } else { ALERT_TEXT };
                            x += style.paint_line(
                                ui.painter(),
                                pos2(x, body.top() + index as f32 * ALERT_TEXT.line_height),
                                run,
                                ALERT_TEXT_INK,
                                None,
                            );
                        }
                    }
                },
            ),
            Notice::Update(_) => alert.show(ui, page),
        };
        if clicked.is_some() {
            self.notice_run(Action::Dismiss);
        }
    }
}
