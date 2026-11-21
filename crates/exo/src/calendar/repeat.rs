//! Repeat: a new event's presets, the `Custom` dialog, and how a rule reads, under the form's
//! When and the details'.

use egui::{Color32, CornerRadius, Id, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use exo_store::{Frequency, Nth, Recurrence, RecurrenceEnd as End};
use exo_ui::{Family, Font, TextStyle};
use jiff::ToSpan;
use jiff::civil::{Date, Weekday};

/// A preset of the Repeat list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    None,
    Daily,
    Weekly,
    /// `Monthly on the {nth} {Weekday}`: the anchor's week, or the last.
    Monthly(Option<Nth>),
    Annually,
    Weekdays,
    Custom,
}

impl Preset {
    /// Its rule: none for `Does not repeat` and `Custom`.
    pub fn rule(self) -> Option<Recurrence> {
        let frequency = match self {
            Preset::None | Preset::Custom => return None,
            Preset::Daily => Frequency::Daily,
            Preset::Weekly => Frequency::Weekly(Vec::new()),
            Preset::Monthly(nth) => Frequency::MonthlyOnWeekday(nth),
            Preset::Annually => Frequency::Yearly,
            Preset::Weekdays => Frequency::Weekly(vec![
                Weekday::Monday,
                Weekday::Tuesday,
                Weekday::Wednesday,
                Weekday::Thursday,
                Weekday::Friday,
            ]),
        };
        Some(Recurrence {
            frequency,
            interval: 1,
            end: End::Never,
        })
    }
}

/// The weekday's full name: `Tuesday`.
fn weekday_name(date: Date) -> String {
    date.strftime("%A").to_string()
}

/// The day of the month as an ordinal: `1st`, `2nd`, `11th`, `23rd`.
fn ordinal(day: i8) -> String {
    let suffix = match (day % 10, day % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{day}{suffix}")
}

/// A monthly rule's choices: `the 17th`, `the third Tuesday` (unless that's the last week),
/// `the last Tuesday` (in a month's last week).
fn monthly_patterns(anchor: Date) -> Vec<(Option<Option<Nth>>, String)> {
    let nth = Nth::of(anchor);
    let mut patterns = vec![(None, format!("the {}", ordinal(anchor.day())))];
    if nth != Nth::Last {
        patterns.push((
            Some(None),
            format!("the {} {}", nth.word(), weekday_name(anchor)),
        ));
    }
    if anchor
        .checked_add(1.week())
        .is_ok_and(|next| next.month() != anchor.month())
    {
        patterns.push((
            Some(Some(Nth::Last)),
            format!("the last {}", weekday_name(anchor)),
        ));
    }
    patterns
}

/// The presets for a series starting on `anchor`, with their names.
pub fn presets(anchor: Date) -> Vec<(Preset, String)> {
    let mut presets = vec![
        (Preset::None, "Does not repeat".to_owned()),
        (Preset::Daily, "Daily".to_owned()),
        (
            Preset::Weekly,
            format!("Weekly on {}", weekday_name(anchor)),
        ),
    ];
    presets.extend(
        monthly_patterns(anchor)
            .into_iter()
            .filter_map(|(pattern, name)| {
                Some((Preset::Monthly(pattern?), format!("Monthly on {name}")))
            }),
    );
    presets.push((
        Preset::Annually,
        format!("Annually on {}", anchor.strftime("%b %-d")),
    ));
    presets.push((Preset::Weekdays, "Every weekday (Mon to Fri)".to_owned()));
    presets.push((Preset::Custom, "Custom".to_owned()));
    presets
}

/// The Repeat trigger's text: the preset whose rule is `rule`, else `rule` described (a
/// custom one), else `Does not repeat`.
pub fn label(rule: Option<&Recurrence>, anchor: Date) -> String {
    let Some(rule) = rule else {
        return "Does not repeat".to_owned();
    };
    presets(anchor)
        .into_iter()
        .find(|(preset, _)| preset.rule().as_ref() == Some(rule))
        .map_or_else(|| describe(rule, anchor), |(_, name)| name)
}

/// Sun to Sat.
const DAYS: [(Weekday, &str); 7] = [
    (Weekday::Sunday, "Sun"),
    (Weekday::Monday, "Mon"),
    (Weekday::Tuesday, "Tue"),
    (Weekday::Wednesday, "Wed"),
    (Weekday::Thursday, "Thu"),
    (Weekday::Friday, "Fri"),
    (Weekday::Saturday, "Sat"),
];

fn day_label(day: Weekday) -> &'static str {
    DAYS.iter()
        .find(|(known, _)| *known == day)
        .map_or("", |(_, label)| label)
}

/// How a rule's end reads: ` until Feb 17, 2027`, `, 13 times`.
fn describe_end(end: End) -> String {
    match end {
        End::Never => String::new(),
        End::On(day) => format!(" until {}", day.strftime("%b %-d, %Y")),
        End::After(count) => format!(", {count} time{}", if count == 1 { "" } else { "s" }),
    }
}

/// `rule` in words for a series starting on `anchor`: `Weekly on Tue, Thu`, `Every 2 months on
/// the third Tuesday until Feb 17, 2027`.
pub fn describe(rule: &Recurrence, anchor: Date) -> String {
    let (single, unit) = match rule.frequency {
        Frequency::Daily => ("Daily", "day"),
        Frequency::Weekly(_) => ("Weekly", "week"),
        Frequency::MonthlyOnDay | Frequency::MonthlyOnWeekday(_) => ("Monthly", "month"),
        Frequency::Yearly => ("Annually", "year"),
    };
    let head = if rule.interval == 1 {
        single.to_owned()
    } else {
        format!("Every {} {unit}s", rule.interval)
    };
    let tail = match &rule.frequency {
        Frequency::Weekly(days) if !days.is_empty() => format!(
            " on {}",
            days.iter()
                .map(|day| day_label(*day))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Frequency::MonthlyOnWeekday(nth) => format!(
            " on the {} {}",
            nth.unwrap_or_else(|| Nth::of(anchor)).word(),
            weekday_name(anchor)
        ),
        _ => String::new(),
    };
    format!("{head}{tail}{}", describe_end(rule.end))
}

/// `rule`'s frequency with what it leaves to the anchor filled in.
fn comparable(frequency: &Frequency, anchor: Date) -> Frequency {
    match frequency {
        Frequency::Weekly(days) => {
            let mut days = if days.is_empty() {
                vec![anchor.weekday()]
            } else {
                days.clone()
            };
            days.sort_by_key(|day| day.to_sunday_zero_offset());
            days.dedup();
            Frequency::Weekly(days)
        }
        Frequency::MonthlyOnWeekday(nth) => {
            Frequency::MonthlyOnWeekday(Some(nth.unwrap_or_else(|| Nth::of(anchor))))
        }
        other => other.clone(),
    }
}

/// The details' line under When for a series whose instance started on `anchor`: its preset's
/// name and end, else its rule described, else `Repeats` while the rule isn't known.
pub fn details_line(rule: Option<&Recurrence>, anchor: Date) -> String {
    let Some(rule) = rule else {
        return "Repeats".to_owned();
    };
    let pattern = (comparable(&rule.frequency, anchor), rule.interval);
    let preset = presets(anchor).into_iter().find(|(preset, _)| {
        preset.rule().is_some_and(|preset| {
            (comparable(&preset.frequency, anchor), preset.interval) == pattern
        })
    });
    match preset {
        Some((_, name)) => format!("{name}{}", describe_end(rule.end)),
        None => describe(rule, anchor),
    }
}

/// The Custom dialog's frequency: what the Every list and On rows show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
    Day,
    Week,
    /// Monthly: on the anchor's day, or (`Some`) its weekday, with a week of its own when set.
    Month(Option<Option<Nth>>),
    Year,
}

/// How the series ends, as the Ends radios pick it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ends {
    Never,
    On,
    After,
}

/// A list open in the dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Open {
    Frequency,
    Monthly,
    EndDate,
}

/// What the Custom dialog asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CustomAction {
    Done(Recurrence),
    Cancel,
}

/// The Custom dialog's state.
#[derive(Clone, Debug)]
pub struct Custom {
    anchor: Date,
    unit: Unit,
    interval: String,
    days: Vec<Weekday>,
    ends: Ends,
    end_date: Date,
    end_text: String,
    count: String,
    open: Option<(Open, usize)>,
    /// The end date's month while its picker is up.
    month: Date,
}

/// A count typed in the dialog: a whole number from 1 to 999, else 1.
fn count_of(text: &str) -> u32 {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .map_or(1, |value| (value.trunc() as i64).clamp(1, 999) as u32)
}

/// The end date as its input shows it: `February 17, 2027`.
fn end_text(date: Date) -> String {
    date.strftime("%B %-d, %Y").to_string()
}

impl Custom {
    /// The dialog for a series starting on `anchor`, showing `rule` (else weekly on the
    /// anchor's day, ending three months on or after 13 times).
    pub fn new(anchor: Date, rule: Option<&Recurrence>) -> Self {
        let unit = match rule.map(|rule| &rule.frequency) {
            Some(Frequency::Daily) => Unit::Day,
            Some(Frequency::MonthlyOnDay) => Unit::Month(None),
            Some(Frequency::MonthlyOnWeekday(nth)) => Unit::Month(Some(*nth)),
            Some(Frequency::Yearly) => Unit::Year,
            Some(Frequency::Weekly(_)) | None => Unit::Week,
        };
        let days = match rule.map(|rule| &rule.frequency) {
            Some(Frequency::Weekly(days)) if !days.is_empty() => days.clone(),
            _ => vec![anchor.weekday()],
        };
        let (ends, end_date, count) = match rule.map(|rule| rule.end) {
            Some(End::On(day)) if day >= anchor => (Ends::On, day, 13),
            Some(End::After(count)) => (
                Ends::After,
                anchor.checked_add(3.months()).unwrap_or(anchor),
                count.clamp(1, 999),
            ),
            Some(End::On(_)) => (
                Ends::On,
                anchor.checked_add(3.months()).unwrap_or(anchor),
                13,
            ),
            Some(End::Never) | None => (
                Ends::Never,
                anchor.checked_add(3.months()).unwrap_or(anchor),
                13,
            ),
        };
        Self {
            anchor,
            unit,
            interval: rule
                .map_or(1, |rule| rule.interval.clamp(1, 999))
                .to_string(),
            days,
            ends,
            end_date,
            end_text: end_text(end_date),
            count: count.to_string(),
            open: None,
            month: end_date.first_of_month(),
        }
    }

    /// Done is enabled: a weekly rule has a day.
    fn can_finish(&self) -> bool {
        self.unit != Unit::Week || !self.days.is_empty()
    }

    /// The rule the dialog makes, unless it can't.
    pub fn done(&self) -> Option<Recurrence> {
        if !self.can_finish() {
            return None;
        }
        let frequency = match self.unit {
            Unit::Day => Frequency::Daily,
            // In Sun to Sat order.
            Unit::Week => Frequency::Weekly(
                DAYS.iter()
                    .map(|(day, _)| *day)
                    .filter(|day| self.days.contains(day))
                    .collect(),
            ),
            Unit::Month(None) => Frequency::MonthlyOnDay,
            Unit::Month(Some(nth)) => Frequency::MonthlyOnWeekday(nth),
            Unit::Year => Frequency::Yearly,
        };
        let end = match self.ends {
            Ends::Never => End::Never,
            Ends::On => End::On(self.end_date),
            Ends::After => End::After(count_of(&self.count)),
        };
        Some(Recurrence {
            frequency,
            interval: count_of(&self.interval),
            end,
        })
    }

    /// Escape: an open list or the date's picker first. Returns whether something closed.
    pub fn escape(&mut self) -> bool {
        self.open.take().is_some()
    }

    /// The dialog's height for its rows.
    fn height(&self) -> f32 {
        let rows = 1 + usize::from(matches!(self.unit, Unit::Month(_) | Unit::Week));
        TOP + TITLE.line_height
            + FIELDS_TOP
            + rows as f32 * (ROW + FIELDS_GAP)
            + ENDS
            + BODY_BOTTOM
            + FOOTER
    }

    /// Paints the dialog centred over `page`, its backdrop under it, and says what was asked
    /// for. `today` and `week_start` are for the end date's picker and the days' order.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        page: Rect,
        today: Date,
        week_start: Weekday,
    ) -> Option<CustomAction> {
        let palette = exo_ui::palette(ctx);
        let carbon = palette.theme == exo_ui::Theme::Carbon;
        let mut action = None;
        // Over the form and its modal, as the prompts are; what it opens stays on top of it.
        egui::Area::new(exo_ui::id(ctx, "calendar-repeat-backdrop"))
            .order(egui::Order::Tooltip)
            .fixed_pos(page.min)
            .constrain(false)
            .show(ctx, |ui| {
                // The backdrop; a click on it cancels.
                let backdrop = ui.allocate_rect(page, Sense::click());
                ui.painter()
                    .rect_filled(page, 0.0, Color32::from_black_alpha(128));
                if backdrop.clicked() && self.open.is_none() {
                    action = Some(CustomAction::Cancel);
                }
            });
        let size = vec2(WIDTH, self.height());
        let card = exo_ui::pixel_snap(Rect::from_center_size(page.center(), size));
        ctx.move_to_top(egui::LayerId::new(
            egui::Order::Tooltip,
            Id::new("calendar-repeat"),
        ));
        egui::Area::new(exo_ui::id(ctx, "calendar-repeat"))
            .order(egui::Order::Tooltip)
            .fixed_pos(card.min)
            .constrain(false)
            .show(ctx, |ui| {
                ui.allocate_rect(card, Sense::click());
                let painter = ui.painter().clone();
                for (offset, blur, alpha) in [
                    ([0.0, 8.0], 20.0, 26),
                    ([0.0, 15.0], 20.0, 10),
                    ([0.0, 0.0], 10.0, 26),
                ] {
                    exo_ui::BoxShadow::new(offset, blur, 0.0, Color32::from_black_alpha(alpha))
                        .paint(&painter, card, CornerRadius::ZERO);
                }
                painter.rect_filled(
                    card,
                    0.0,
                    if carbon {
                        Color32::from_rgb(0x18, 0x19, 0x1a)
                    } else {
                        Color32::WHITE
                    },
                );
                let ink = palette.ink(0.9, 0.9);
                let left = card.left() + BODY_X;
                TITLE.paint_line(&painter, pos2(left, card.top() + TOP), "Repeat", ink, None);
                let mut y = card.top() + TOP + TITLE.line_height + FIELDS_TOP;
                let mut lists: Vec<(Open, Pos2, Vec<String>, usize)> = Vec::new();

                // Every [n] [unit].
                FIELD.paint_line(&painter, pos2(left, y + TEXT_TOP), "Every", ink, None);
                let number = number_input(
                    ui,
                    exo_ui::id(ui.ctx(), "calendar-repeat-interval"),
                    &mut self.interval,
                    pos2(left + EVERY_WIDTH + 4.0, y + TEXT_TOP),
                    palette,
                    false,
                );
                let unit_name = match self.unit {
                    Unit::Day => "day",
                    Unit::Week => "week",
                    Unit::Month(_) => "month",
                    Unit::Year => "year",
                };
                let frequency = dropdown(
                    ui,
                    exo_ui::id(ui.ctx(), "calendar-repeat-frequency"),
                    pos2(number.rect.right() + 4.0, y + TEXT_TOP),
                    unit_name,
                    carbon,
                );
                if frequency.clicked() {
                    self.toggle(Open::Frequency);
                }
                if let Some((Open::Frequency, highlighted)) = self.open {
                    lists.push((
                        Open::Frequency,
                        pos2(frequency.rect.left() - 12.0, frequency.rect.bottom() + 12.0),
                        ["day", "week", "month", "year"].map(str::to_owned).to_vec(),
                        highlighted,
                    ));
                }
                y += ROW + FIELDS_GAP;

                // Monthly: On [the 17th | the third Tuesday | the last Tuesday].
                if let Unit::Month(pattern) = self.unit {
                    FIELD.paint_line(&painter, pos2(left, y + TEXT_TOP), "On", ink, None);
                    let patterns = monthly_patterns(self.anchor);
                    // `the last …` when the index is the last, or the anchor's week is.
                    let chosen = match pattern {
                        None => None,
                        Some(Some(Nth::Last)) => Some(Some(Nth::Last)),
                        Some(nth) if nth.unwrap_or_else(|| Nth::of(self.anchor)) == Nth::Last => {
                            Some(Some(Nth::Last))
                        }
                        Some(_) => Some(None),
                    };
                    let name = patterns
                        .iter()
                        .find(|(id, _)| *id == chosen)
                        .map_or_else(String::new, |(_, name)| name.clone());
                    let button = dropdown(
                        ui,
                        exo_ui::id(ui.ctx(), "calendar-repeat-monthly"),
                        pos2(left + ROW_LABEL + 8.0, y + TEXT_TOP),
                        &name,
                        carbon,
                    );
                    if button.clicked() {
                        self.toggle(Open::Monthly);
                    }
                    if let Some((Open::Monthly, highlighted)) = self.open {
                        lists.push((
                            Open::Monthly,
                            pos2(button.rect.left() - 20.0, button.rect.bottom() + 12.0),
                            patterns.into_iter().map(|(_, name)| name).collect(),
                            highlighted,
                        ));
                    }
                    y += ROW + FIELDS_GAP;
                }

                // Weekly: On [Sun] … [Sat], from the week's first day.
                if self.unit == Unit::Week {
                    // The row's items centred (`align-items: center`), the prefix nudged `top: 1px`.
                    FIELD.paint_line(&painter, pos2(left, y + CHIP_TEXT_TOP), "On", ink, None);
                    let first = DAYS
                        .iter()
                        .position(|(day, _)| *day == week_start)
                        .unwrap_or(0);
                    for (index, (day, label)) in DAYS.iter().cycle().skip(first).take(7).enumerate()
                    {
                        let chip = Rect::from_min_size(
                            pos2(left + WEEKDAYS_LEFT + index as f32 * (CHIP.x + 6.0), y),
                            CHIP,
                        );
                        let response = ui
                            .interact(
                                chip,
                                exo_ui::id(ui.ctx(), ("calendar-repeat-day", index)),
                                Sense::click(),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                        let selected = self.days.contains(day);
                        // A day's chip, picked or not.
                        let (fill, text, weight) = match (selected, carbon) {
                            (true, false) => (
                                Color32::from_rgb(0xf3, 0xf5, 0xfa),
                                Color32::from_rgb(0x88, 0x92, 0xb9),
                                600,
                            ),
                            (true, true) => (
                                Color32::from_rgb(0xae, 0xb1, 0xdd),
                                Color32::from_rgb(0x27, 0x29, 0x2d),
                                600,
                            ),
                            (false, _) => (Color32::TRANSPARENT, palette.ink(0.6, 0.6), 400),
                        };
                        let fill = if response.hovered() {
                            palette.ink(0.1, 0.1)
                        } else {
                            fill
                        };
                        painter.rect_filled(chip, CornerRadius::same(8), fill);
                        let style = TextStyle::new(
                            Font::new(Family::Sans, weight),
                            FIELD.size,
                            FIELD.line_height,
                        );
                        let width = style.width(ctx, label);
                        style.paint_line(
                            &painter,
                            pos2(chip.center().x - width / 2.0, chip.top() + CHIP_TEXT_TOP),
                            label,
                            text,
                            None,
                        );
                        if response.clicked() {
                            if selected {
                                self.days.retain(|known| known != day)
                            } else {
                                self.days.push(*day)
                            }
                        }
                    }
                    y += ROW + FIELDS_GAP;
                }

                // Ends: ( ) Never / ( ) On [date] / ( ) After [n] times.
                FIELD.paint_line(&painter, pos2(left, y + TEXT_TOP), "Ends", ink, None);
                let radios = left + ROW_LABEL + 8.0;
                for (index, (ends, label)) in [
                    (Ends::Never, "Never"),
                    (Ends::On, "On"),
                    (Ends::After, "After"),
                ]
                .into_iter()
                .enumerate()
                {
                    let line = y + TEXT_TOP + index as f32 * (FIELD.line_height + 8.0);
                    let width = FIELD.width(ctx, label);
                    let hit = Rect::from_min_size(
                        pos2(radios, line),
                        vec2(RADIO + 8.0 + width, FIELD.line_height),
                    );
                    let response = ui
                        .interact(
                            hit,
                            exo_ui::id(ui.ctx(), ("calendar-repeat-ends", index)),
                            Sense::click(),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    paint_radio(&painter, pos2(radios, line), self.ends == ends, carbon);
                    FIELD.paint_line(&painter, pos2(radios + RADIO + 8.0, line), label, ink, None);
                    if response.clicked() {
                        self.ends = ends;
                    }
                    let after = hit.right() + 4.0;
                    match ends {
                        Ends::Never => {}
                        Ends::On => {
                            let inactive = self.ends != Ends::On;
                            let field = date_input(
                                ui,
                                exo_ui::id(ui.ctx(), "calendar-repeat-end-date"),
                                &mut self.end_text,
                                pos2(after, line),
                                palette,
                                inactive,
                            );
                            if field.gained_focus() || field.clicked() {
                                self.open = Some((Open::EndDate, 0));
                                self.month = self.end_date.first_of_month();
                            }
                            if field.lost_focus()
                                || (field.has_focus()
                                    && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                            {
                                if let Some(date) =
                                    exo_ui::parse_date(&self.end_text, self.end_date, today)
                                        .filter(|date| *date >= self.anchor)
                                {
                                    self.end_date = date;
                                    self.ends = Ends::On;
                                }
                                self.end_text = end_text(self.end_date);
                            }
                            if let Some((Open::EndDate, _)) = self.open {
                                ctx.move_to_top(egui::LayerId::new(
                                    egui::Order::Tooltip,
                                    Id::new("calendar-repeat-end-picker"),
                                ));
                                let pick = exo_ui::DatePicker {
                                    id: exo_ui::id(ctx, "calendar-repeat-end-picker"),
                                    month: &mut self.month,
                                    selected: Some(self.end_date),
                                    today,
                                    min: Some(self.anchor),
                                    week_start,
                                }
                                .show(ctx, field.rect.left_bottom() + exo_ui::DATE_PICKER_OFFSET);
                                if let Some(exo_ui::DatePick::Picked(date)) = pick {
                                    self.end_date = date;
                                    self.end_text = end_text(date);
                                    self.ends = Ends::On;
                                    self.open = None;
                                }
                            }
                        }
                        Ends::After => {
                            let inactive = self.ends != Ends::After;
                            let field = number_input(
                                ui,
                                exo_ui::id(ui.ctx(), "calendar-repeat-count"),
                                &mut self.count,
                                pos2(after, line),
                                palette,
                                inactive,
                            );
                            if field.changed() {
                                self.ends = Ends::After;
                            }
                            let times = FIELD.width(ctx, "times");
                            let label = Rect::from_min_size(
                                pos2(field.rect.right() + 4.0, line),
                                vec2(times, FIELD.line_height),
                            );
                            FIELD.paint_line(&painter, label.min, "times", ink, None);
                            if ui
                                .interact(
                                    label,
                                    exo_ui::id(ui.ctx(), "calendar-repeat-times"),
                                    Sense::click(),
                                )
                                .clicked()
                            {
                                self.ends = Ends::After;
                            }
                        }
                    }
                }

                // Cancel, Done, nudged `top: 3px`.
                let footer_top = card.bottom() - FOOTER + 16.0 + 3.0;
                let mut right = card.right() - 20.0;
                // Classic's uppercase dialog buttons; on Carbon's dark card Cancel is white at `.4`
                // and Done keeps its white label.
                for (label, primary) in [("Done", true), ("Cancel", false)] {
                    let kind = if primary {
                        exo_ui::ActionKind::Primary
                    } else {
                        exo_ui::ActionKind::Plain
                    };
                    let button = exo_ui::DialogAction::new(label, kind)
                        .enabled(!primary || self.can_finish())
                        .dark(carbon && !primary);
                    let width = button.width(ctx);
                    let rect = Rect::from_min_size(
                        pos2(right - width, footer_top),
                        vec2(width, exo_ui::DIALOG_ACTION_HEIGHT),
                    );
                    right = rect.left() - 8.0;
                    if button
                        .show(
                            ui,
                            exo_ui::id(ui.ctx(), ("calendar-repeat-action", label)),
                            rect,
                        )
                        .clicked()
                    {
                        action = Some(if primary {
                            self.done().map_or(CustomAction::Cancel, CustomAction::Done)
                        } else {
                            CustomAction::Cancel
                        });
                    }
                }

                for (which, at, items, highlighted) in lists {
                    let labels: Vec<&str> = items.iter().map(String::as_str).collect();
                    let id = exo_ui::id(ctx, ("calendar-repeat-list", which as u8));
                    ctx.move_to_top(egui::LayerId::new(egui::Order::Tooltip, id));
                    // The frequency list (the monthly pattern list): padded `4px 0`, names 12px (20px)
                    // in, at least 35px (128px) wide.
                    let (cell, min) = if which == Open::Frequency {
                        (12.0, 35.0 + 24.0)
                    } else {
                        (20.0, 128.0 + 40.0)
                    };
                    let menu = exo_ui::Menu {
                        items: &labels,
                        highlighted,
                    }
                    .show_compact(ui, id, at, cell, min);
                    if let Some(index) = menu.clicked {
                        self.pick(which, index);
                    } else if let Some(index) = menu.hovered {
                        self.open = Some((which, index));
                    } else if ui.input(|input| input.pointer.any_released())
                        && !ui.rect_contains_pointer(menu.rect)
                    {
                        self.open = None;
                    }
                }
            });
        action
    }

    fn toggle(&mut self, which: Open) {
        self.open = if self.open.is_some_and(|(open, _)| open == which) {
            None
        } else {
            Some((which, 0))
        };
    }

    /// Takes row `index` of list `which`.
    fn pick(&mut self, which: Open, index: usize) {
        self.open = None;
        match which {
            Open::Frequency => {
                self.unit = match index {
                    0 => Unit::Day,
                    1 => Unit::Week,
                    2 => Unit::Month(None),
                    _ => Unit::Year,
                };
            }
            Open::Monthly => {
                if let Some((pattern, _)) = monthly_patterns(self.anchor).get(index) {
                    self.unit = Unit::Month(*pattern);
                }
            }
            Open::EndDate => {}
        }
    }
}

/// The dialog: `width: 550px; padding-top: 20px`, its body `padding: 0 32px 8px`.
const WIDTH: f32 = 550.0;
const TOP: f32 = 20.0;
const BODY_X: f32 = 32.0;
const BODY_BOTTOM: f32 = 8.0;
/// The fields: `margin-top: 24px; gap: 12px`; rows at least 24px.
const FIELDS_TOP: f32 = 24.0;
const FIELDS_GAP: f32 = 12.0;
const ROW: f32 = 24.0;
/// A row's text line: at its top, nudged `top: 1px`; a day's label centred in its 24px.
const TEXT_TOP: f32 = 1.0;
const CHIP_TEXT_TOP: f32 = 5.0;
/// The Ends column: three 16px lines 8px apart.
const ENDS: f32 = 3.0 * 16.0 + 2.0 * 8.0;
/// The footer: `padding: 16px 20px` around 32px buttons.
const FOOTER: f32 = 16.0 + 32.0 + 16.0;
/// `Every`'s box (`flex: 0 0 44px`), a row's label's (`40px`), and the days' (`32px` and
/// `gap: 8px`).
const EVERY_WIDTH: f32 = 44.0;
const ROW_LABEL: f32 = 40.0;
const WEEKDAYS_LEFT: f32 = 32.0 + 8.0;
/// A day's chip: 40×24.
const CHIP: Vec2 = vec2(40.0, 24.0);
const RADIO: f32 = 12.0;
/// The title: 16px/24px; the fields 12px/16px.
const TITLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 24.0);
const FIELD: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);

/// A Classic radio, its ring white at `.4` in Carbon.
fn paint_radio(painter: &egui::Painter, at: Pos2, selected: bool, carbon: bool) {
    if !carbon {
        return exo_ui::paint_radio(painter, at, selected);
    }
    let center = at + Vec2::splat(RADIO / 2.0);
    painter.circle_stroke(
        center,
        5.5,
        Stroke::new(1.0, Color32::from_white_alpha(102)),
    );
    if selected {
        painter.circle_filled(center, 3.0, Color32::from_rgb(0x6f, 0xbe, 0xea));
    }
}

/// A dropdown's button: `value`, then 10px on its caret (a 12px icon, black, Carbon's white at
/// `.6`), 3px down the line and fitted to its 12×6.7 viewBox.
fn dropdown(ui: &egui::Ui, id: Id, at: Pos2, value: &str, carbon: bool) -> egui::Response {
    let palette = exo_ui::palette(ui.ctx());
    let width = FIELD.paint_line(ui.painter(), at, value, palette.ink(0.9, 0.9), None);
    let caret = Rect::from_min_size(
        at + vec2(width + 10.0, 3.0 + (12.0 - 6.7) / 2.0),
        vec2(12.0, 6.7),
    );
    exo_ui::paint_icon(
        ui,
        "legacy/DEPRECATED_downCaret",
        caret,
        if carbon {
            Color32::from_white_alpha(153)
        } else {
            Color32::BLACK
        },
    );
    let rect = Rect::from_min_size(at, vec2(width + 10.0 + 12.0, 20.0));
    ui.interact(rect, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A count's input: a centred number on a hairline, `#aeb1dd` and 2px focused, `.4` while its
/// option isn't picked.
fn number_input(
    ui: &mut egui::Ui,
    id: Id,
    text: &mut String,
    at: Pos2,
    palette: &exo_ui::Palette,
    inactive: bool,
) -> egui::Response {
    let typed = FIELD.width(ui.ctx(), text);
    let width = (typed + 4.0).max(16.0);
    let line = Rect::from_min_size(at, vec2(width, FIELD.line_height));
    // `text-align: center`.
    let field = Rect::from_min_size(
        at + vec2(((width - typed) / 2.0).floor(), 0.0),
        vec2(typed.max(1.0) + 1.0, FIELD.line_height),
    );
    let ink = if inactive {
        palette.ink(0.4, 0.4)
    } else {
        palette.ink(0.8, 0.8)
    };
    let mut response = exo_ui::text_field(
        ui,
        id,
        text,
        field,
        &FIELD,
        ink,
        palette.text_selection_default,
    );
    text.retain(|c| c.is_ascii_digit());
    text.truncate(3);
    hairline(ui, line, response.has_focus());
    response.rect = line;
    response
}

/// The end date's input, sized to its text.
fn date_input(
    ui: &mut egui::Ui,
    id: Id,
    text: &mut String,
    at: Pos2,
    palette: &exo_ui::Palette,
    inactive: bool,
) -> egui::Response {
    let width = FIELD.width(ui.ctx(), text).max(20.0) + 2.0;
    let line = Rect::from_min_size(at, vec2(width, FIELD.line_height));
    let ink = if inactive {
        palette.ink(0.4, 0.4)
    } else {
        palette.ink(0.9, 0.9)
    };
    let response = exo_ui::text_field(
        ui,
        id,
        text,
        line,
        &FIELD,
        ink,
        palette.text_selection_default,
    );
    hairline(ui, line, response.has_focus());
    response
}

/// An input's bottom border under its line: `rgba(0,0,0,.1)` (Carbon's too), 2px `#aeb1dd`
/// focused.
fn hairline(ui: &egui::Ui, line: Rect, focused: bool) {
    let (height, color) = if focused {
        (2.0, Color32::from_rgb(0xae, 0xb1, 0xdd))
    } else {
        (1.0, Color32::from_black_alpha(26))
    };
    ui.painter().rect_filled(
        Rect::from_min_size(line.left_bottom(), vec2(line.width(), height)),
        0.0,
        color,
    );
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    #[test]
    fn presets_follow_the_start() {
        let names: Vec<String> = presets(date(2026, 11, 17))
            .into_iter()
            .map(|(_, name)| name)
            .collect();
        assert_eq!(
            names,
            [
                "Does not repeat",
                "Daily",
                "Weekly on Tuesday",
                "Monthly on the third Tuesday",
                "Annually on Nov 17",
                "Every weekday (Mon to Fri)",
                "Custom"
            ]
        );
        let names: Vec<String> = presets(date(2026, 11, 24))
            .into_iter()
            .map(|(_, name)| name)
            .collect();
        assert_eq!(
            names[3..5],
            [
                "Monthly on the fourth Tuesday".to_owned(),
                "Monthly on the last Tuesday".to_owned()
            ],
            "a month's last week has both"
        );
        let names: Vec<String> = presets(date(2026, 12, 29))
            .into_iter()
            .map(|(_, name)| name)
            .collect();
        assert_eq!(
            names[3], "Monthly on the last Tuesday",
            "the 29th is only the last"
        );
    }

    #[test]
    fn rules_are_described_in_words() {
        let anchor = date(2026, 11, 17);
        let rule = |frequency, interval, end| Recurrence {
            frequency,
            interval,
            end,
        };
        assert_eq!(label(None, anchor), "Does not repeat");
        assert_eq!(
            label(Preset::Weekly.rule().as_ref(), anchor),
            "Weekly on Tuesday"
        );
        let tuesdays = rule(Frequency::Weekly(vec![Weekday::Tuesday]), 1, End::Never);
        assert_eq!(
            label(Some(&tuesdays), anchor),
            "Weekly on Tue",
            "the form compares the rule as made"
        );
        assert_eq!(
            details_line(Some(&tuesdays), anchor),
            "Weekly on Tuesday",
            "the details as it repeats"
        );
        assert_eq!(
            details_line(Some(&rule(Frequency::Daily, 1, End::After(1))), anchor),
            "Daily, 1 time"
        );
        assert_eq!(
            details_line(
                Some(&rule(
                    Frequency::MonthlyOnWeekday(None),
                    2,
                    End::On(date(2027, 2, 17))
                )),
                anchor
            ),
            "Every 2 months on the third Tuesday until Feb 17, 2027"
        );
        assert_eq!(
            details_line(
                Some(&rule(
                    Frequency::Weekly(vec![Weekday::Tuesday, Weekday::Thursday]),
                    1,
                    End::Never
                )),
                anchor
            ),
            "Weekly on Tue, Thu"
        );
        assert_eq!(details_line(None, anchor), "Repeats");
        assert_eq!(ordinal(22), "22nd");
        assert_eq!(ordinal(13), "13th");
    }

    #[test]
    fn the_custom_dialog_makes_its_rule() {
        let anchor = date(2026, 11, 17);
        let mut custom = Custom::new(anchor, None);
        assert_eq!(
            custom.done(),
            Some(Recurrence {
                frequency: Frequency::Weekly(vec![Weekday::Tuesday]),
                interval: 1,
                end: End::Never
            })
        );
        custom.days = vec![Weekday::Friday, Weekday::Monday];
        custom.interval = "2".into();
        custom.ends = Ends::After;
        assert_eq!(
            custom
                .done()
                .map(|rule| rule.rule(
                    &"2026-11-17T15:00[America/New_York]".parse().expect("time"),
                    false
                ))
                .as_deref(),
            Some("RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,FR;WKST=SU;COUNT=13")
        );
        custom.days.clear();
        assert_eq!(custom.done(), None, "a week with no day");
        custom.pick(Open::Monthly, 1);
        custom.interval = "1200".into();
        assert_eq!(
            custom.done().map(|rule| (rule.frequency, rule.interval)),
            Some((Frequency::MonthlyOnWeekday(None), 999))
        );
    }
}
