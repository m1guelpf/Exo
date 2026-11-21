//! The settings menu: a `Settings` title over a scrolling list of sections, in the right pane
//! above its footer. Rows have no hover style; a click on a row's label runs it.
//!
//! The menu's sections and rows, in order, and what Exo does with each (rows that can't work are
//! left out):
//!
//! | Section | Row | In Exo |
//! | --- | --- | --- |
//! | `General` | Autocomplete, Autocorrect | **built**, with macOS's spellchecker |
//! | | Auto Archive, Auto Drafts, Auto Labels, Knowledge Base, Write with AI, Personalization | hidden: AI |
//! | | Auto Label Library | hidden: AI |
//! | | Connectors, MCP | hidden: needs a server |
//! | | Reminders | **built** |
//! | | Split Inbox, Split Inbox Library | **built** |
//! | `Exo for Sales` | HubSpot, Pipedrive, Salesforce, Recent Opens | hidden: needs a server |
//! | `Learn Exo` | Shortcuts, Quick Tips | **built** |
//! | | Guides, Attend Live Webinar | hidden: no guides or webinars |
//! | | Tutorial, Getting Started Checklist | hidden: no tutorial |
//! | `My Account` | Theme | **built** |
//! | | Add Accounts | **built** (the Accounts dialog) |
//! | | Achievements | **built** (the app's dialog, `crate::achievements`) |
//! | | Edit Profile, Add Payment, Billing | hidden: needs a server |
//! | `My Team` | every row | hidden: teams |
//! | `Calendar` | Calendar Accounts, Meeting Links, Timezones | **built** (Calendar Settings, `crate::calendar`) |
//! | | Scheduling | hidden: not built |
//! | | Notifications | **built** |
//! | `Triage` | Get Me To Zero, Bulk Actions | **built** (the app's dialogs, `crate::bulk`) |
//! | | Hide Empty Split Inboxes | **built** |
//! | `Writing` | Emoji, Signatures, Convert Hyphen to Bulleted List | **built** |
//! | `Workflow` | Auto Advance, Auto Bcc, Blocked Senders, Downloads, Images, Instant Intro | **built** |
//! | | Out of Office Reply | **built** (the app's dialog, `crate::out_of_office`) |
//! | | Redirect to Exo | hidden: web only |
//! | | Read Statuses | hidden: needs a server |
//! | `Advanced` | Backtick as Escape, Send + Mark Done, RSVP + Mark Done, Show Sender Full Names | **built** |
//! | | Hide Comment Bar | hidden: teams |
//! | | Open at Login | **Exo's own** toggle, shown where Exo can be a login item: in `Exo.app` |
//! | | Warn Before Quitting | **Exo's own** toggle: the quit alert (`crate::desktop::QuitWarning`), always shown |
//! | — | Sign Out | **built** (the app signs out, `crate::accounts`) |
//!
//! A section with no rows isn't drawn.

use egui::{Color32, Rect, Sense, Ui, pos2, vec2};
use exo_ui::{Family, Font, LinesTooltip, TextStyle, TooltipPart, motion, palette};

use super::{Action, Dialog, Toggle, Triage};

/// The title's padding, `36px 2rem 16px`, around a 20px line.
const TITLE_TOP: f32 = 36.0;
const LIST_TOP: f32 = 72.0;
/// The right pane's footer: `padding: 22px 2rem 12px` around 28px buttons.
const FOOTER_HEIGHT: f32 = 62.0;
/// The title: 16px/20px.
const TITLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 20.0);
/// A section's header: 12px/16px bold, `top: 1px`, `margin: 20px 0 8px`.
const HEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 16.0);
const HEADER_ABOVE: f32 = 20.0;
const HEADER_BELOW: f32 = 8.0;
/// A row's label: 12px/16px, `top: 1px`; rows `padding: 0 0 8px`, the last one 0.
const LABEL: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
const LINE: f32 = 16.0;
const ROW_GAP: f32 = 8.0;
/// A toggle's tooltip: its box 14px left of the row (a 6px offset and the tooltip's 8px
/// padding), at most 289px wide.
const TOOLTIP_GAP: f32 = 14.0;
const TOOLTIP_WIDTH: f32 = 289.0 - 16.0 - 14.0;
/// `Backtick as Escape`'s tooltip, wider than the others, its keys separate
/// caps with spaces between them.
const BACKTICK_TOOLTIP: LinesTooltip<'static> = {
    use TooltipPart::{Keys, Text};
    LinesTooltip {
        paragraphs: &[
            &[&[
                Text("Use "),
                Keys(&["`"]),
                Text(" instead of "),
                Keys(&["esc"]),
            ]],
            &[
                &[Text("• Best used on keyboards with a Touch Bar ")],
                &[
                    Text("• You can still type "),
                    Keys(&["`"]),
                    Text(" by hitting "),
                    Keys(&["option"]),
                    Text(" "),
                    Keys(&["shift"]),
                    Text(" "),
                    Keys(&["`"]),
                ],
            ],
        ],
    }
};
const NUDGE: f32 = 1.0;
/// The sign-out row, the list's last: a 20px top margin (the last section's 4px bottom margin
/// collapses into it) and 8px of padding above the 16px label, 4px of padding and a 20px margin
/// below it.
const SIGN_OUT_ABOVE: f32 = 28.0;
const SIGN_OUT_BELOW: f32 = 24.0;
const SIGN_OUT: &str = "Sign Out";
/// Exo's own rows, toggles in the `Advanced` section, where its other app-wide toggles go.
const ADVANCED: &str = "Advanced";
const OPEN_AT_LOGIN: &str = "Open at Login";
const WARN_BEFORE_QUITTING: &str = "Warn Before Quitting";
/// The `Advanced` section's rows of Exo's own: each slider's state, `None` to leave the row out.
#[derive(Debug, Clone, Copy)]
pub struct ExoRows {
    /// `Open at Login`, shown where Exo can be a login item.
    pub open_at_login: Option<bool>,
    /// `Warn Before Quitting`.
    pub warn_before_quitting: Option<bool>,
}

/// Where the menu is scrolled to.
#[derive(Debug)]
pub struct Scroll {
    offset: f32,
    /// The line over the footer: on when the menu opens, then on
    /// after each scroll unless it reached the end.
    shadow: bool,
}

impl Default for Scroll {
    /// The top, as the menu opens.
    fn default() -> Self {
        Self {
            offset: 0.0,
            shadow: true,
        }
    }
}

/// A section of the menu.
struct Section {
    header: &'static str,
    rows: &'static [(&'static str, Action)],
}

/// A row as drawn: its title, what it does, and for a toggle whether its
/// slider is on.
type Row = (&'static str, Action, Option<bool>);

/// The sections Exo shows, in order (see the module docs).
const SECTIONS: &[Section] = &[
    Section {
        header: "General",
        rows: &[
            (
                "Autocomplete",
                Action::Flip {
                    setting: Toggle::Autocomplete,
                    palette: false,
                },
            ),
            ("Autocorrect", Action::Open(Dialog::Autocorrect)),
            ("Reminders", Action::Open(Dialog::Reminders)),
            ("Split Inbox", Action::Open(Dialog::SplitInbox)),
            ("Split Inbox Library", Action::SplitLibrary),
        ],
    },
    Section {
        header: "Learn Exo",
        rows: &[
            ("Shortcuts", Action::Shortcuts),
            (
                "Quick Tips",
                Action::Flip {
                    setting: Toggle::QuickTips,
                    palette: false,
                },
            ),
        ],
    },
    Section {
        header: "My Account",
        rows: &[
            ("Add Accounts", Action::Open(Dialog::Accounts)),
            ("Theme", Action::Open(Dialog::Theme)),
            ("Achievements", Action::Achievements),
        ],
    },
    Section {
        header: "Calendar",
        rows: &[
            ("Calendar Accounts", Action::CalendarSettings),
            ("Meeting Links", Action::CalendarSettings),
            ("Timezones", Action::CalendarSettings),
            ("Notifications", Action::Open(Dialog::Notifications)),
        ],
    },
    Section {
        header: "Triage",
        rows: &[
            ("Get Me To Zero", Action::Triage(Triage::GetMeToZero)),
            ("Bulk Actions", Action::Triage(Triage::BulkActions)),
            (
                "Hide Empty Split Inboxes",
                Action::Open(Dialog::HideEmptySplits),
            ),
        ],
    },
    Section {
        header: "Writing",
        rows: &[
            ("Emoji", Action::Open(Dialog::Emoji)),
            ("Signatures", Action::Open(Dialog::Signatures)),
            (
                "Convert Hyphen to Bulleted List",
                Action::Flip {
                    setting: Toggle::HyphenToBulletList,
                    palette: false,
                },
            ),
        ],
    },
    Section {
        header: "Workflow",
        rows: &[
            ("Auto Advance", Action::Open(Dialog::AutoAdvance)),
            ("Auto Bcc", Action::Open(Dialog::AutoBcc)),
            ("Blocked Senders", Action::Open(Dialog::BlockedSenders)),
            ("Downloads", Action::Open(Dialog::Downloads)),
            ("Images", Action::Open(Dialog::Images)),
            ("Instant Intro", Action::Open(Dialog::InstantIntro)),
            ("Out of Office Reply", Action::OutOfOffice),
        ],
    },
];

/// Draws the menu in `pane` (the right pane's inside) above its footer, scrolled to `scroll`,
/// and returns the row clicked. At Inbox Zero (`zero`) the right pane is gone and the menu sits
/// in the footer, with no line over the footer, in white over the blurred photo.
/// `settings` are the account's, for the toggles; `exos` are Exo's own rows.
pub fn show(
    ui: &mut Ui,
    pane: Rect,
    rem: f32,
    zero: bool,
    scroll: &mut Scroll,
    settings: &exo_store::Settings,
    exos: ExoRows,
) -> Option<Action> {
    let ctx = ui.ctx().clone();
    let tokens = palette(&ctx);
    // The title and section headers: black at .8, white at .85 in Carbon; rows at .6. At Inbox
    // Zero, white at .7, the title's too, and the section headers at .85.
    let white = |alpha: f32| Color32::from_white_alpha((alpha * 255.0).round() as u8);
    let (heading, strong, subdued) = if zero {
        (white(0.7), white(0.85), white(0.7))
    } else {
        (
            tokens.ink(0.8, 0.85),
            tokens.ink(0.8, 0.85),
            tokens.ink(0.6, 0.6),
        )
    };
    let left = pane.left() + 2.0 * rem;
    let width = pane.width() - 4.0 * rem;
    TITLE.paint_line(
        ui.painter(),
        pos2(left, pane.top() + TITLE_TOP),
        "Settings",
        heading,
        None,
    );

    let list = Rect::from_min_max(
        pos2(pane.left(), pane.top() + LIST_TOP),
        pos2(pane.right(), pane.bottom() - FOOTER_HEIGHT),
    );
    // A toggle's slider shows its setting.
    let slider = |action: &Action| match action {
        Action::Flip { setting, .. } => Some(super::toggled(*setting, settings)),
        _ => None,
    };
    let mut sections: Vec<(&str, Vec<Row>)> = SECTIONS
        .iter()
        .map(|section| {
            (
                section.header,
                section
                    .rows
                    .iter()
                    .map(|(title, action)| (*title, action.clone(), slider(action)))
                    .collect(),
            )
        })
        .collect();
    let toggle = |title, setting, on| {
        (
            title,
            Action::Flip {
                setting,
                palette: false,
            },
            Some(on),
        )
    };
    let mut advanced = vec![
        toggle(
            "Backtick as Escape",
            Toggle::BacktickAsEscape,
            settings.backtick_as_escape(),
        ),
        toggle(
            "Send + Mark Done",
            Toggle::SendAndDone,
            settings.send_and_done,
        ),
        toggle(
            "RSVP + Mark Done",
            Toggle::RsvpAndDone,
            settings.rsvp_and_done,
        ),
        toggle(
            "Show Sender Full Names",
            Toggle::SenderFullNames,
            settings.show_sender_full_names,
        ),
    ];
    let exos = [
        (OPEN_AT_LOGIN, Action::OpenAtLogin, exos.open_at_login),
        (
            WARN_BEFORE_QUITTING,
            Action::WarnBeforeQuitting,
            exos.warn_before_quitting,
        ),
    ];
    advanced.extend(exos.into_iter().filter(|(_, _, on)| on.is_some()));
    sections.push((ADVANCED, advanced));
    sections.retain(|(_, rows)| !rows.is_empty());
    let height: f32 = sections
        .iter()
        .map(|(_, rows)| {
            HEADER_ABOVE + LINE + HEADER_BELOW + rows.len() as f32 * (LINE + ROW_GAP) - ROW_GAP
        })
        .sum::<f32>()
        + SIGN_OUT_ABOVE
        + LINE
        + SIGN_OUT_BELOW;
    let wheel = if ui.rect_contains_pointer(list) {
        ui.input(|input| input.smooth_scroll_delta.y)
    } else {
        0.0
    };
    if wheel != 0.0 {
        scroll.offset = (scroll.offset - wheel).clamp(0.0, (height - list.height()).max(0.0));
        scroll.shadow = scroll.offset + list.height() < height;
    }

    let mut list_ui = ui.new_child(egui::UiBuilder::new().max_rect(list));
    list_ui.set_clip_rect(list.intersect(ui.clip_rect()));
    let painter = list_ui.painter().clone();
    let mut clicked = None;
    let mut y = list.top() - scroll.offset;
    for (header, rows) in sections {
        y += HEADER_ABOVE;
        HEADER.paint_line(&painter, pos2(left, y + NUDGE), header, strong, None);
        y += LINE + HEADER_BELOW;
        let count = rows.len();
        for (index, (title, action, toggle)) in rows.into_iter().enumerate() {
            // The label spans the row (`flex: 1 0 auto`); a button's click is on the
            // label, a toggle's on the whole row, its bottom padding too.
            let label = Rect::from_min_size(pos2(left, y + NUDGE), vec2(width, LINE));
            let gap = if toggle.is_some() && index + 1 < count {
                ROW_GAP
            } else {
                0.0
            };
            let target = if toggle.is_some() {
                Rect::from_min_size(pos2(left, y), vec2(width, LINE + gap))
            } else {
                label
            };
            let response = list_ui
                .interact(
                    target,
                    exo_ui::id(list_ui.ctx(), ("settings-row", title)),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if let Some(text) = tooltip(&action) {
                exo_ui::TextTooltip {
                    text,
                    max_width: TOOLTIP_WIDTH,
                    learn_more: false,
                }
                .show_left(&list_ui, &response, TOOLTIP_GAP);
            } else if matches!(
                action,
                Action::Flip {
                    setting: Toggle::BacktickAsEscape,
                    ..
                }
            ) {
                BACKTICK_TOOLTIP.show_left(&list_ui, &response, TOOLTIP_GAP);
            }
            if response.clicked() {
                clicked = Some(action);
            }
            LABEL.paint_line(&painter, label.min, title, subdued, None);
            if let Some(on) = toggle {
                // The slider: 16px high, right-aligned at the row's top.
                let slider = Rect::from_min_size(
                    pos2(left + width - exo_ui::SLIDER_SIZE.x, y),
                    exo_ui::SLIDER_SIZE,
                );
                exo_ui::paint_slider(
                    &painter,
                    slider,
                    on,
                    false,
                    exo_ui::id(&ctx, ("settings-toggle", title)),
                    exo_ui::SliderColors::CLASSIC,
                );
            }
            y += LINE + ROW_GAP;
        }
        y -= ROW_GAP;
    }
    // `Sign Out` spans its row like a label, in the menu's colour.
    let label = Rect::from_min_size(pos2(left, y + SIGN_OUT_ABOVE + NUDGE), vec2(width, LINE));
    if list_ui
        .interact(
            label,
            exo_ui::id(list_ui.ctx(), "settings-sign-out"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        clicked = Some(Action::SignOut);
    }
    LABEL.paint_line(&painter, label.min, SIGN_OUT, subdued, None);

    footer_shadow(ui, pane, rem, scroll.shadow && !zero);
    clicked
}

/// A toggle's tooltip of text alone.
fn tooltip(action: &Action) -> Option<&'static str> {
    match action {
        Action::Flip {
            setting: Toggle::HyphenToBulletList,
            ..
        } => Some(
            "Typing a hyphen followed by a space at the start of a line starts a bulleted list.",
        ),
        Action::Flip {
            setting: Toggle::SendAndDone,
            ..
        } => Some("Lets you Send + Mark Done simultaneously."),
        Action::Flip {
            setting: Toggle::RsvpAndDone,
            ..
        } => Some("Lets you RSVP to event invites + Mark Done simultaneously."),
        Action::Flip {
            setting: Toggle::SenderFullNames,
            ..
        } => Some("See senders' full names when reading messages."),
        Action::Flip {
            setting: Toggle::QuickTips,
            ..
        } => Some("Show the Quick Tips bar and learn common shortcuts to work twice as fast."),
        _ => None,
    }
}

/// The 1px line over the footer (`box-shadow: 0 -1px 0 0 rgba(0,0,0,.05)`, white at .08 in
/// Carbon), inset `2rem - 12px` from the pane's sides and fading over 150ms `ease-in-out`.
pub(crate) fn footer_shadow(ui: &Ui, pane: Rect, rem: f32, on: bool) {
    let shown = motion::transition(
        ui.ctx(),
        exo_ui::id(ui.ctx(), "settings-footer-shadow"),
        on,
        0.15,
        motion::CubicBezier::EASE_IN_OUT,
    );
    let color = match exo_ui::theme(ui.ctx()) {
        exo_ui::Theme::Snow => Color32::from_black_alpha(13),
        exo_ui::Theme::Carbon => Color32::from_white_alpha(20),
    };
    let inset = 2.0 * rem - 12.0;
    let top = pane.bottom() - FOOTER_HEIGHT;
    let line = Rect::from_min_max(
        pos2(pane.left() + inset, top - 1.0),
        pos2(pane.right() - inset, top),
    );
    ui.painter()
        .rect_filled(line, 0.0, color.gamma_multiply(shown));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The menu at 400×1300 (all of it, unscrolled), with `Open at Login` off and `Warn Before
    /// Quitting` as `warn` has it, after a click at `at`; the action clicked.
    fn click_with(at: egui::Pos2, warn: Option<bool>) -> Option<Action> {
        let ctx = egui::Context::default();
        exo_ui::install(&ctx).expect("the system fonts load");
        let pane = Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 1300.0));
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut clicked = None;
        for event in [egui::Event::PointerMoved(at), button(true), button(false)] {
            let input = egui::RawInput {
                screen_rect: Some(pane),
                events: vec![event],
                ..Default::default()
            };
            let output = ctx.run_ui(input, |ui| {
                if let Some(action) = show(
                    ui,
                    pane,
                    16.0,
                    false,
                    &mut Scroll::default(),
                    &exo_store::Settings::default(),
                    ExoRows {
                        open_at_login: Some(false),
                        warn_before_quitting: warn,
                    },
                ) {
                    clicked = Some(action);
                }
            });
            output.drop_without_applying_deltas();
        }
        clicked
    }

    /// [`click_with`], without `Warn Before Quitting`.
    fn click(at: egui::Pos2) -> Option<Action> {
        click_with(at, None)
    }

    #[test]
    fn warn_before_quitting_follows_open_at_login() {
        // `Open at Login` at 1104, then `Warn Before Quitting` at 1128, a toggle's row apart.
        assert_eq!(
            click_with(pos2(32.0 + 5.0, 1112.0), Some(true)),
            Some(Action::OpenAtLogin)
        );
        assert_eq!(
            click_with(pos2(32.0 + 5.0, 1136.0), Some(true)),
            Some(Action::WarnBeforeQuitting),
            "the label"
        );
        assert_eq!(
            click_with(pos2(400.0 - 32.0 - 10.0, 1136.0), Some(true)),
            Some(Action::WarnBeforeQuitting),
            "the slider"
        );
        assert_eq!(
            click_with(pos2(32.0 + 5.0, 1188.0), Some(true)),
            Some(Action::SignOut),
            "Sign Out moved down a row"
        );
    }

    #[test]
    fn the_calendar_rows_open_calendar_settings_and_notifications() {
        // `Calendar` after `My Account`, whose last row is Achievements: its header at 440, its
        // rows from 464, 24px apart.
        assert_eq!(
            click(pos2(32.0 + 5.0, 472.0)),
            Some(Action::CalendarSettings),
            "Calendar Accounts"
        );
        assert_eq!(
            click(pos2(32.0 + 5.0, 544.0)),
            Some(Action::Open(Dialog::Notifications))
        );
    }

    #[test]
    fn quick_tips_follows_shortcuts() {
        // `Learn Exo` after `General`: its header at 248, its rows from 272.
        assert_eq!(click(pos2(32.0 + 5.0, 280.0)), Some(Action::Shortcuts));
        assert_eq!(
            click(pos2(400.0 - 32.0 - 10.0, 304.0)),
            Some(Action::Flip {
                setting: Toggle::QuickTips,
                palette: false
            }),
            "the slider"
        );
    }

    #[test]
    fn autocomplete_and_autocorrect_lead_general() {
        assert_eq!(
            click(pos2(400.0 - 32.0 - 10.0, 124.0)),
            Some(Action::Flip {
                setting: Toggle::Autocomplete,
                palette: false
            }),
            "the slider"
        );
        assert_eq!(
            click(pos2(32.0 + 5.0, 148.0)),
            Some(Action::Open(Dialog::Autocorrect))
        );
        assert_eq!(
            click(pos2(32.0 + 5.0, 172.0)),
            Some(Action::Open(Dialog::Reminders))
        );
        assert_eq!(
            click(pos2(32.0 + 5.0, 196.0)),
            Some(Action::Open(Dialog::SplitInbox))
        );
    }

    #[test]
    fn writing_follows_triage_and_its_toggle_flips_from_anywhere_on_its_row() {
        // `Writing` after `Triage`: its header at 688, its rows from 712, Emoji first.
        let flip = Some(Action::Flip {
            setting: Toggle::HyphenToBulletList,
            palette: false,
        });
        assert_eq!(
            click(pos2(32.0 + 5.0, 720.0)),
            Some(Action::Open(Dialog::Emoji))
        );
        assert_eq!(
            click(pos2(32.0 + 5.0, 744.0)),
            Some(Action::Open(Dialog::Signatures))
        );
        assert_eq!(click(pos2(32.0 + 5.0, 768.0)), flip, "the label");
        assert_eq!(click(pos2(400.0 - 32.0 - 10.0, 768.0)), flip, "the slider");
    }

    #[test]
    fn workflow_starts_with_auto_advance() {
        // `Workflow` after `Writing`: its header at 788, its rows from 812.
        assert_eq!(
            click(pos2(32.0 + 5.0, 820.0)),
            Some(Action::Open(Dialog::AutoAdvance))
        );
        assert_eq!(
            click(pos2(32.0 + 5.0, 844.0)),
            Some(Action::Open(Dialog::AutoBcc)),
            "Auto Bcc, after Auto Advance"
        );
        assert_eq!(
            click(pos2(32.0 + 5.0, 868.0)),
            Some(Action::Open(Dialog::BlockedSenders))
        );
        assert_eq!(
            click(pos2(32.0 + 5.0, 892.0)),
            Some(Action::Open(Dialog::Downloads)),
            "Downloads, after Blocked Senders"
        );
        assert_eq!(
            click(pos2(32.0 + 5.0, 916.0)),
            Some(Action::Open(Dialog::Images))
        );
        assert_eq!(
            click(pos2(32.0 + 5.0, 940.0)),
            Some(Action::Open(Dialog::InstantIntro)),
            "Instant Intro, after Images"
        );
        assert_eq!(
            click(pos2(32.0 + 5.0, 964.0)),
            Some(Action::OutOfOffice),
            "Out of Office Reply, after Instant Intro"
        );
    }

    #[test]
    fn advanced_toggles_flip_from_anywhere_on_their_rows_and_open_at_login_comes_last() {
        // `Advanced` after `Workflow`: its header at 984, its rows from 1008, Backtick as Escape
        // first.
        let flip = |setting| {
            Some(Action::Flip {
                setting,
                palette: false,
            })
        };
        assert_eq!(
            click(pos2(32.0 + 5.0, 1016.0)),
            flip(Toggle::BacktickAsEscape)
        );
        assert_eq!(click(pos2(32.0 + 5.0, 1040.0)), flip(Toggle::SendAndDone));
        assert_eq!(
            click(pos2(400.0 - 32.0 - 10.0, 1064.0)),
            flip(Toggle::RsvpAndDone),
            "the slider"
        );
        assert_eq!(
            click(pos2(32.0 + 5.0, 1088.0)),
            flip(Toggle::SenderFullNames)
        );
        assert_eq!(
            click(pos2(32.0 + 5.0, 1112.0)),
            Some(Action::OpenAtLogin),
            "the label"
        );
        assert_eq!(
            click(pos2(400.0 - 32.0 - 10.0, 1112.0)),
            Some(Action::OpenAtLogin),
            "the slider"
        );
        assert_eq!(
            click(pos2(32.0 + 5.0, 1164.0)),
            Some(Action::SignOut),
            "Sign Out after Advanced"
        );
    }
}
