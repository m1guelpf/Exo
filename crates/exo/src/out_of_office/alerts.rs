//! Out of Office Reply's alerts: offline, Gmail not answering, a save that didn't take, changes to
//! discard, a pasted image, and the nudge to set a reply for out-of-office days on the calendar.

use egui::{Color32, Rect, Sense, Ui, pos2, text::LayoutJob, vec2};
use exo_store::OutOfOfficeDays;
use exo_ui::{
    ALERT_PARAGRAPH_GAP, ALERT_TEXT, ALERT_TEXT_INK, ALERT_TEXT_WIDTH, Family, Font, TextStyle,
};
use jiff::tz::TimeZone;

/// `<b>` in the alert's text.
const BOLD: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 14.0, 20.0);
/// A link in an alert's text: `#54acdc`.
const LINK: Color32 = Color32::from_rgb(0x54, 0xac, 0xdc);
/// The opt-out row: `margin: 20px 0 24px`, `rgba(0, 0, 0, .55)`; its label 8px after the 14px box,
/// 1px up; the row 21px tall.
const OPT_OUT_ABOVE: f32 = 20.0;
const OPT_OUT_HEIGHT: f32 = 21.0;
/// Its `margin-bottom: 24px` swallows the text's own 8px.
const OPT_OUT_BELOW: f32 = 24.0 - ALERT_PARAGRAPH_GAP;
const OPT_OUT_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 140);
const OPT_OUT_TEXT_LEFT: f32 = 22.0;
/// `Don't show this again`.
const OPT_OUT: &str = "Don't show this again";

/// An alert showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Alert {
    /// The dialog was asked for offline.
    Offline,
    /// Gmail couldn't be read when the dialog opened: it closes the dialog too.
    SyncFailed,
    /// Gmail didn't take the save.
    SaveFailed,
    /// Closing a changed form.
    Discard,
    /// A file was pasted in the message; `email` for the Gmail settings link.
    PastedFile {
        /// The account's address.
        email: String,
    },
    /// Out-of-office days are coming up, and whether `Don't show this again` is checked.
    Nudge {
        /// The days.
        days: OutOfOfficeDays,
        /// The box is checked.
        opt_out: bool,
        /// They come from an event just made in Exo, which the alert doesn't date.
        created: bool,
    },
}

/// One of an alert's text runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Run {
    Text,
    Bold,
    Link,
}

impl Alert {
    /// The title.
    pub fn title(&self) -> &'static str {
        match self {
            Self::Offline => "You are offline",
            Self::SyncFailed | Self::PastedFile { .. } => "Sorry!",
            Self::SaveFailed => "Changes not saved",
            Self::Discard => "Discard changes?",
            Self::Nudge { .. } => "Set Out of Office Reply?",
        }
    }

    /// The actions, right to left (the sync failure's is lowercase).
    pub fn actions(&self) -> &'static [&'static str] {
        match self {
            Self::Offline | Self::SaveFailed => &["OK"],
            Self::SyncFailed => &["ok"],
            Self::PastedFile { .. } => &["Ok"],
            Self::Discard => &["Discard", "Cancel"],
            Self::Nudge { .. } => &["Set reply", "No thanks"],
        }
    }

    /// The text as plain text, `<p>`s on lines of their own. For tests.
    pub fn text(&self, tz: &TimeZone) -> String {
        self.paragraphs(tz)
            .iter()
            .map(|runs| {
                runs.iter()
                    .map(|(text, _)| text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn paragraphs(&self, tz: &TimeZone) -> Vec<Vec<(String, Run)>> {
        let text = |text: &str| vec![(text.to_owned(), Run::Text)];
        match self {
            Self::Offline => vec![text(
                "You cannot change your Out of Office Reply settings when offline. Please check your internet connection and try again.",
            )],
            Self::SyncFailed => vec![text(
                "We were unable to sync your Out of Office Reply settings with your email provider. Please check your connection and try again.",
            )],
            Self::SaveFailed => vec![text(
                "Sorry! We were unable to sync your Out of Office Reply settings with your email provider. Please try again later.",
            )],
            Self::Discard => vec![text("Your settings will not be saved.")],
            Self::PastedFile { .. } => vec![vec![
                ("To add an image, go to ".into(), Run::Text),
                ("Gmail settings".into(), Run::Link),
                (" and update your Vacation Responder.".into(), Run::Text),
            ]],
            Self::Nudge { created: true, .. } => vec![text(
                "Would you like to send automatic out of office replies while you are away?",
            )],
            Self::Nudge { days, .. } => {
                // The weekday, month and day, in English: `Monday, June 2`.
                let date = |at: jiff::Timestamp| {
                    at.to_zoned(tz.clone()).strftime("%A, %B %-d").to_string()
                };
                let (start, end) = (
                    days.start.to_zoned(tz.clone()).date(),
                    days.end.to_zoned(tz.clone()).date(),
                );
                let when = if start == end {
                    vec![
                        (
                            "It looks like you may be out of office on ".into(),
                            Run::Text,
                        ),
                        (format!("{}.", date(days.start)), Run::Bold),
                    ]
                } else {
                    vec![
                        (
                            "It looks like you may be out of office from ".into(),
                            Run::Text,
                        ),
                        (date(days.start), Run::Bold),
                        (" to ".into(), Run::Text),
                        (format!("{}.", date(days.end)), Run::Bold),
                    ]
                };
                vec![
                    when,
                    text(
                        "Would you like to send automatic out of office replies while you are away?",
                    ),
                ]
            }
        }
    }

    /// The Gmail settings link of a pasted file's alert.
    pub fn link(&self) -> Option<String> {
        match self {
            Self::PastedFile { email } => Some(format!(
                "https://mail.google.com/?shxr=0&authuser={email}#settings/general"
            )),
            _ => None,
        }
    }

    /// Paints the alert with `selected` filled over `page`. Returns the action clicked, and
    /// whether the link was.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        page: Rect,
        selected: usize,
        tz: &TimeZone,
    ) -> (Option<usize>, bool) {
        let ctx = ui.ctx().clone();
        let paragraphs: Vec<Vec<Vec<(String, Run)>>> = self
            .paragraphs(tz)
            .iter()
            .map(|runs| wrap(&ctx, runs, ALERT_TEXT_WIDTH))
            .collect();
        let lines: usize = paragraphs.iter().map(Vec::len).sum();
        let mut height = lines as f32 * ALERT_TEXT.line_height
            + paragraphs.len().saturating_sub(1) as f32 * ALERT_PARAGRAPH_GAP;
        let nudge = matches!(self, Self::Nudge { .. });
        if nudge {
            height += OPT_OUT_ABOVE + OPT_OUT_HEIGHT + OPT_OUT_BELOW;
        }
        let title = self.title();
        let actions = self.actions();
        let mut link_clicked = false;
        let mut toggled = false;
        let alert = exo_ui::Alert {
            title,
            text: "",
            actions,
            selected,
        };
        let clicked = alert.show_body(ui, page, height, |ui, text| {
            let mut y = text.top();
            for (index, lines) in paragraphs.iter().enumerate() {
                for line in lines {
                    let mut job = LayoutJob::default();
                    let mut x = text.left();
                    for (part, run) in line {
                        let style = if *run == Run::Bold { BOLD } else { ALERT_TEXT };
                        let ink = if *run == Run::Link {
                            LINK
                        } else {
                            ALERT_TEXT_INK
                        };
                        job.append(part, 0.0, style.format(ink));
                        let width = style.width(&ctx, part);
                        if *run == Run::Link {
                            let hit = Rect::from_min_size(
                                pos2(x, y),
                                vec2(width, ALERT_TEXT.line_height),
                            );
                            link_clicked |= ui
                                .interact(
                                    hit,
                                    ui.id().with(("out-of-office-alert-link", index)),
                                    Sense::click(),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked();
                        }
                        x += width;
                    }
                    exo_ui::paint_job(ui.painter(), pos2(text.left(), y), job, &ALERT_TEXT);
                    y += ALERT_TEXT.line_height;
                }
                y += ALERT_PARAGRAPH_GAP;
            }
            if let Self::Nudge { opt_out, .. } = self {
                let row = Rect::from_min_size(
                    pos2(text.left(), y - ALERT_PARAGRAPH_GAP + OPT_OUT_ABOVE),
                    vec2(text.width(), OPT_OUT_HEIGHT),
                );
                exo_ui::paint_checkbox(
                    ui.painter(),
                    row.min,
                    *opt_out,
                    exo_ui::Theme::Snow.palette(),
                );
                let label_width = ALERT_TEXT.paint_line(
                    ui.painter(),
                    pos2(row.left() + OPT_OUT_TEXT_LEFT, row.top() - 1.0),
                    OPT_OUT,
                    OPT_OUT_INK,
                    None,
                );
                let hit = Rect::from_min_size(
                    row.min,
                    vec2(OPT_OUT_TEXT_LEFT + label_width, OPT_OUT_HEIGHT - 4.0),
                );
                toggled = ui
                    .interact(hit, ui.id().with("out-of-office-opt-out"), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked();
            }
        });
        if toggled && let Self::Nudge { opt_out, .. } = self {
            *opt_out = !*opt_out;
        }
        (clicked, link_clicked)
    }
}

/// Breaks `runs` into lines no wider than `width`, at spaces, as the alert's text wraps
/// (`text-wrap: pretty`: a last line of one word takes the word before it when it fits).
fn wrap(ctx: &egui::Context, runs: &[(String, Run)], width: f32) -> Vec<Vec<(String, Run)>> {
    let style = |run: Run| if run == Run::Bold { BOLD } else { ALERT_TEXT };
    // Words, each with the space after it, in their run's style.
    let mut words: Vec<(String, Run)> = Vec::new();
    for (text, run) in runs {
        let mut word = String::new();
        for ch in text.chars() {
            word.push(ch);
            if ch == ' ' {
                words.push((std::mem::take(&mut word), *run));
            }
        }
        if !word.is_empty() {
            words.push((word, *run));
        }
    }
    let measure = |line: &[(String, Run)]| -> f32 {
        let last = line.len().saturating_sub(1);
        line.iter()
            .enumerate()
            .map(|(index, (word, run))| {
                style(*run).width(ctx, if index == last { word.trim_end() } else { word })
            })
            .sum()
    };
    let mut lines: Vec<Vec<(String, Run)>> = vec![Vec::new()];
    for word in words {
        let line = lines.last_mut().expect("a line");
        line.push(word);
        if line.len() > 1 && measure(line) > width {
            let word = line.pop().expect("just pushed");
            lines.push(vec![word]);
        }
    }
    if let [.., before, last] = lines.as_mut_slice()
        && last.len() == 1
        && before.len() > 1
    {
        let joined = vec![before[before.len() - 1].clone(), last[0].clone()];
        if measure(&joined) <= width {
            before.pop();
            *last = joined;
        }
    }
    // Merge each line's words back into runs.
    lines
        .into_iter()
        .map(|line| {
            let mut merged: Vec<(String, Run)> = Vec::new();
            let count = line.len();
            for (index, (word, run)) in line.into_iter().enumerate() {
                let word = if index + 1 == count {
                    word.trim_end().to_owned()
                } else {
                    word
                };
                match merged.last_mut() {
                    Some((text, last)) if *last == run => text.push_str(&word),
                    _ => merged.push((word, run)),
                }
            }
            merged
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;

    use super::*;

    fn days(start: &str, end: &str) -> OutOfOfficeDays {
        OutOfOfficeDays {
            start: start.parse::<Timestamp>().expect("a time"),
            end: end.parse::<Timestamp>().expect("a time"),
            ids: vec!["ooo".into()],
        }
    }

    #[test]
    fn the_nudge_names_the_days() {
        let tz = TimeZone::get("America/New_York").expect("a zone");
        let one = Alert::Nudge {
            days: days("2026-11-19T05:00:00Z", "2026-11-20T04:59:59.999Z"),
            opt_out: false,
            created: false,
        };
        assert_eq!(
            one.text(&tz),
            "It looks like you may be out of office on Thursday, November 19.\nWould you like to send automatic out of office replies while you are away?"
        );
        let two = Alert::Nudge {
            days: days("2026-11-19T05:00:00Z", "2026-11-21T04:59:59.999Z"),
            opt_out: false,
            created: false,
        };
        let created = Alert::Nudge {
            days: days("2026-11-19T05:00:00Z", "2026-11-21T04:59:59.999Z"),
            opt_out: false,
            created: true,
        };
        assert_eq!(
            created.text(&tz),
            "Would you like to send automatic out of office replies while you are away?"
        );
        assert_eq!(
            two.text(&tz).lines().next(),
            Some(
                "It looks like you may be out of office from Thursday, November 19 to Friday, November 20."
            )
        );
        assert_eq!(two.actions(), ["Set reply", "No thanks"]);
    }

    #[test]
    fn a_pasted_file_links_to_gmails_settings() {
        let alert = Alert::PastedFile {
            email: "maya@brightmoor.co".into(),
        };
        assert_eq!(
            alert.text(&TimeZone::UTC),
            "To add an image, go to Gmail settings and update your Vacation Responder."
        );
        assert_eq!(
            alert.link().as_deref(),
            Some("https://mail.google.com/?shxr=0&authuser=maya@brightmoor.co#settings/general")
        );
    }
}
