//! The Achievements dialog: a banner with the category's ribbon over its statistic cards, the main
//! view's cards opening their category, and a back button out of one.
//!
//! The ribbons' art is the system's emoji.

use egui::{
    Area, Color32, Context, CornerRadius, CursorIcon, Mesh, Order, Painter, Pos2, Rect, Sense, Ui,
    Vec2, pos2, vec2,
};
use exo_store::{Achievements, Statistics};
use exo_ui::{BoxShadow, Family, Font, TextStyle, Theme, gradient, motion, paint_icon};
use jiff::civil::Date;

use super::{Card, View};

/// The dialog: 460px wide, 16px corners.
const WIDTH: f32 = 460.0;
const RADIUS: u8 = 16;
/// The banner: 144px tall.
const BANNER: f32 = 144.0;
/// The ribbon's emoji, sized to the art's height.
const RIBBON: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 66.0, 80.0);
/// A card's header: 21px/32px.
const HEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 21.0, 32.0);
/// The cards' subheader: 12px/20px, `top: 3px`, with bold runs.
const SUBHEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
const SUBHEADER_BOLD: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 20.0);
/// A card's value: 32px/36px.
const VALUE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 32.0, 36.0);
/// A card's unit: 14px/20px, `top: 3px`.
const UNIT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 14.0, 20.0);
/// A card's tag: 10px/12px semibold.
const TAG: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 10.0, 12.0);
/// The backdrop: `rgba(0,0,0,.5)`, fading over 200ms.
const BACKDROP: Color32 = Color32::from_black_alpha(128);

/// What a frame of the dialog asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked {
    /// A click outside: close it.
    Close,
    /// A card in the main view, or the back button (to [`View::Main`]).
    Show(View),
}

/// What the dialog shows: the counts on `today`.
pub struct Shown<'a> {
    pub view: View,
    pub statistics: &'a Statistics,
    pub seen: &'a Achievements,
    pub today: Date,
}

/// Paints the dialog over `area` (the page below the tab strip) while `open`, fading in and
/// out as the dialog overlay does.
pub fn show(ctx: &Context, area: Rect, open: bool, shown: &Shown<'_>) -> Option<Asked> {
    let id = exo_ui::id(ctx, "achievements-dialog");
    let opacity = motion::transition(ctx, id.with("fade"), open, 0.2, motion::CubicBezier::EASE);
    if opacity <= 0.0 {
        return None;
    }
    Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(area.min)
        .show(ctx, |ui| {
            ui.set_opacity(opacity);
            let backdrop = ui.allocate_rect(area, Sense::click());
            ui.painter().rect_filled(area, 0.0, BACKDROP);
            // Escape is the dialog's command (`achievements::register`).
            let asked = card(ui, area, shown);
            asked.or((open && backdrop.clicked()).then_some(Asked::Close))
        })
        .inner
}

/// A card's colors: Snow's or Carbon's.
fn pick(theme: Theme, snow: Color32, carbon: Color32) -> Color32 {
    if theme == Theme::Snow { snow } else { carbon }
}

fn card(ui: &mut Ui, area: Rect, shown: &Shown<'_>) -> Option<Asked> {
    let ctx = ui.ctx().clone();
    let theme = exo_ui::theme(&ctx);
    let main = shown.view == View::Main;
    let rows: Vec<Row> = shown
        .view
        .cards()
        .iter()
        .map(|&card| Row::new(card, shown))
        .collect();
    let numbers = if main { 120.0 } else { 50.0 };
    let text_width = WIDTH - 64.0 - 24.0 - numbers;
    let heights: Vec<f32> = rows
        .iter()
        .map(|row| row.height(&ctx, text_width))
        .collect();
    // The Inbox Zero and Send views' cards have 16px under them; the dialog's foot is `32px 0 16px`
    // under the main view's cards and in Carbon, else `22px 0 24px`.
    let after_cards = if matches!(shown.view, View::InboxZero | View::Send) {
        16.0
    } else {
        0.0
    };
    let wordmark = if main || theme == Theme::Carbon {
        32.0 + 16.0
    } else {
        22.0 + 24.0
    };
    let height = BANNER + heights.iter().sum::<f32>() + after_cards + wordmark;
    let modal = exo_ui::pixel_snap(Rect::from_center_size(
        area.center(),
        vec2(WIDTH, height.min(area.height())),
    ));
    ui.allocate_rect(modal, Sense::click());
    let painter = ui.painter().with_clip_rect(modal);
    // `box-shadow: 0 15px 75px 0 rgba(0,0,0,.18)`; `#fafcff`, `#1f1e1d` in Carbon.
    BoxShadow::new([0.0, 15.0], 75.0, 0.0, Color32::from_black_alpha(46)).paint(
        ui.painter(),
        modal,
        CornerRadius::same(RADIUS),
    );
    let background = pick(
        theme,
        Color32::from_rgb(0xfa, 0xfc, 0xff),
        Color32::from_rgb(0x1f, 0x1e, 0x1d),
    );
    ui.painter()
        .rect_filled(modal, CornerRadius::same(RADIUS), background);
    paint_banner(
        ui.painter(),
        Rect::from_min_size(modal.min, vec2(WIDTH, BANNER)),
        theme,
    );

    // The ribbon: centered, its middle 60% down the banner.
    let ribbon = match shown.view {
        View::Main => "💎",
        View::InboxZero => "🔥",
        View::MarkDone => "✅",
        View::Send => "💌",
    };
    let ribbon_width = RIBBON.width(&ctx, ribbon);
    RIBBON.paint_line(
        &painter,
        pos2(
            modal.center().x - ribbon_width / 2.0,
            modal.top() + 0.6 * BANNER - RIBBON.line_height / 2.0,
        ),
        ribbon,
        Color32::WHITE,
        None,
    );

    let mut asked = None;
    if !main && back_button(ui, &painter, modal.min + vec2(20.0, 20.0), theme) {
        asked = Some(Asked::Show(View::Main));
    }
    let mut top = modal.top() + BANNER;
    for (index, (row, height)) in rows.iter().zip(&heights).enumerate() {
        let rect = Rect::from_min_size(pos2(modal.left() + 32.0, top), vec2(WIDTH - 64.0, *height));
        let last = index + 1 == rows.len();
        if !last {
            // `border-bottom: 1px solid rgba(0,0,0,.05)`, `#474543` in Carbon.
            painter.hline(
                rect.x_range(),
                rect.bottom() - 0.5,
                egui::Stroke::new(
                    1.0,
                    pick(
                        theme,
                        Color32::from_black_alpha(13),
                        Color32::from_rgb(0x47, 0x45, 0x43),
                    ),
                ),
            );
        }
        let accent = accent(shown.view, index, theme);
        row.paint(&painter, rect, text_width, numbers, accent, theme);
        if main {
            let response = ui
                .interact(
                    rect,
                    exo_ui::id(ui.ctx(), ("achievements-card", index)),
                    Sense::click(),
                )
                .on_hover_cursor(CursorIcon::PointingHand);
            if response.clicked() {
                asked = Some(Asked::Show(match row.card {
                    Card::InboxZero => View::InboxZero,
                    Card::MarkedDone => View::MarkDone,
                    _ => View::Send,
                }));
            }
        }
        top += height;
    }
    asked
}

/// The value's color for the card at `index` of `view`: the main view's first, second and
/// third in Inbox Zero's, Mark Done's and Send's; a category's all in its own.
fn accent(view: View, index: usize, theme: Theme) -> Color32 {
    let orange = pick(
        theme,
        Color32::from_rgb(0xc2, 0x50, 0x00),
        Color32::from_rgb(0xff, 0x9d, 0x56),
    );
    let green = pick(
        theme,
        Color32::from_rgb(0x14, 0x80, 0x72),
        Color32::from_rgb(0x3a, 0xc7, 0xa1),
    );
    let purple = pick(
        theme,
        Color32::from_rgb(0x71, 0x4c, 0xb6),
        Color32::from_rgb(0xbe, 0xa1, 0xf5),
    );
    match (view, index) {
        (View::Main, 1) | (View::MarkDone, _) => green,
        (View::Main, 2) | (View::Send, _) => purple,
        _ => orange,
    }
}

/// The back button: a 40px circle, white (`#1f1e1d` in Carbon), its 14px arrow 13px in and 12.5px
/// down. Returns whether it was clicked.
fn back_button(ui: &Ui, painter: &Painter, at: Pos2, theme: Theme) -> bool {
    let rect = Rect::from_min_size(at, Vec2::splat(40.0));
    let response = ui
        .interact(
            rect,
            exo_ui::id(ui.ctx(), "achievements-back"),
            Sense::click(),
        )
        .on_hover_cursor(CursorIcon::PointingHand);
    let round = CornerRadius::same(20);
    for (offset, blur, alpha) in [
        ([0.0, 0.0], 6.0, 26),
        ([0.0, 2.0], 6.0, 38),
        ([0.0, 0.0], 0.5, 26),
    ] {
        BoxShadow::new(offset, blur, 0.0, Color32::from_black_alpha(alpha))
            .paint(painter, rect, round);
    }
    painter.rect_filled(
        rect,
        round,
        pick(theme, Color32::WHITE, Color32::from_rgb(0x1f, 0x1e, 0x1d)),
    );
    // `[fill*=red] { fill: #000 }`, `#e8e5df` in Carbon.
    let icon = Rect::from_min_size(at + vec2(13.0, 12.5), Vec2::splat(14.0));
    let ink = pick(theme, Color32::BLACK, Color32::from_rgb(0xe8, 0xe5, 0xdf));
    paint_icon(
        ui,
        "legacy/DEPRECATED_backArrow",
        exo_ui::icon_rect(ui.ctx(), "legacy/DEPRECATED_backArrow", icon),
        ink,
    );
    response.clicked()
}

/// The banner's background: a 115° gradient under a radial veil at its bottom middle and a fade
/// into the dialog over its last 96px.
fn paint_banner(painter: &Painter, rect: Rect, theme: Theme) {
    let rgba = |r: u8, g: u8, b: u8, a: f32| {
        [
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
            a,
        ]
    };
    let snow = theme == Theme::Snow;
    let stops: &[(f32, [f32; 4])] = if snow {
        &[
            (0.0, rgba(0xee, 0xe6, 0xff, 1.0)),
            (0.48, rgba(0xf8, 0xf5, 0xfb, 1.0)),
            (1.0, rgba(0xed, 0xf4, 0xff, 1.0)),
        ]
    } else {
        &[
            (0.0, rgba(0x35, 0x24, 0x51, 1.0)),
            (1.0, rgba(0x1b, 0x30, 0x47, 1.0)),
        ]
    };
    let angle = 115_f32.to_radians();
    let direction = vec2(angle.sin(), -angle.cos());
    let length = rect.width() * angle.sin().abs() + rect.height() * angle.cos().abs();
    let fade = if snow {
        rgba(0xfa, 0xfc, 0xff, 1.0)
    } else {
        rgba(0x1f, 0x1e, 0x1d, 1.0)
    };
    let color_at = |point: Pos2| {
        let t = (point - rect.center()).dot(direction) / length + 0.5;
        let mut color = gradient::ramp(stops, t);
        // `radial-gradient(80.67px 51.3px at 50% 100%, #fcfaf7 30%, rgba(252,250,247,.2) 72%)`;
        // Carbon's `ellipse 270px 180px`, `#1f1e1d 35%, rgba(31,30,29,.88) 75%, transparent`.
        let (radii, veil): (Vec2, &[(f32, [f32; 4])]) = if snow {
            (
                vec2(80.67, 51.3),
                &[
                    (0.3, rgba(0xfc, 0xfa, 0xf7, 1.0)),
                    (0.72, rgba(0xfc, 0xfa, 0xf7, 0.2)),
                ],
            )
        } else {
            (
                vec2(270.0, 180.0),
                &[
                    (0.35, rgba(0x1f, 0x1e, 0x1d, 1.0)),
                    (0.75, rgba(0x1f, 0x1e, 0x1d, 0.88)),
                    (1.0, rgba(0x1f, 0x1e, 0x1d, 0.0)),
                ],
            )
        };
        let offset = point - rect.center_bottom();
        let distance = vec2(offset.x / radii.x, offset.y / radii.y).length();
        color = gradient::over(gradient::ramp(veil, distance), color);
        // `linear-gradient(to bottom, transparent calc(100% - 96px), #fafcff)`.
        let faded = ((point.y - (rect.bottom() - 96.0)) / 96.0).clamp(0.0, 1.0);
        color = gradient::over([fade[0], fade[1], fade[2], faded], color);
        Color32::from_rgb(
            (color[0] * 255.0).round() as u8,
            (color[1] * 255.0).round() as u8,
            (color[2] * 255.0).round() as u8,
        )
    };
    // A grid fine enough that interpolating between its vertices doesn't show; its top rows a
    // pixel apart, narrowed to the dialog's rounded corners.
    let radius = f32::from(RADIUS);
    let columns = 46;
    let mut ys: Vec<f32> = (0..RADIUS).map(f32::from).collect();
    ys.extend((0..=16).map(|i| radius + (rect.height() - radius) * i as f32 / 16.0));
    let mut mesh = Mesh::default();
    for &y in &ys {
        let from_corner = (radius - y).max(0.0);
        let inset = radius - (radius * radius - from_corner * from_corner).sqrt();
        for column in 0..=columns {
            let x =
                rect.left() + inset + (rect.width() - 2.0 * inset) * column as f32 / columns as f32;
            let point = pos2(x, rect.top() + y);
            mesh.colored_vertex(point, color_at(point));
        }
    }
    let index = |row: u32, column: u32| row * (columns + 1) + column;
    for row in 0..ys.len() as u32 - 1 {
        for column in 0..columns {
            mesh.add_triangle(
                index(row, column),
                index(row, column + 1),
                index(row + 1, column),
            );
            mesh.add_triangle(
                index(row, column + 1),
                index(row + 1, column + 1),
                index(row + 1, column),
            );
        }
    }
    painter.add(mesh);
}

/// One statistic card's content.
struct Row {
    card: Card,
    header: &'static str,
    /// The subheader's runs, bold or not.
    subheader: Vec<(String, bool)>,
    value: u64,
    unit: String,
    new: bool,
}

impl Row {
    fn new(card: Card, shown: &Shown<'_>) -> Self {
        let statistics = shown.statistics;
        let plain = |text: &str| vec![(text.to_owned(), false)];
        let bold = |before: &str, strong: String, after: &str| {
            vec![
                (before.to_owned(), false),
                (strong, true),
                (after.to_owned(), false),
            ]
        };
        let plural = |word: &str, count: u64| {
            if count == 1 {
                word.to_owned()
            } else {
                format!("{word}s")
            }
        };
        let left = |value: u64| super::remaining(card, value);
        let n = super::grouped;
        let (header, subheader, value, unit) = match card {
            Card::InboxZero => {
                let days = u64::from(statistics.inbox_zero_days_all_time);
                let subheader = if days == 0 {
                    plain("Hit Inbox Zero once to reach the next level.")
                } else {
                    bold(
                        "Hit Inbox Zero another ",
                        format!("{} {}", n(left(days)), plural("day", left(days))),
                        " to reach the next level.",
                    )
                };
                ("Inbox Zero", subheader, days, "Day")
            }
            Card::CurrentStreak => {
                let streak = u64::from(statistics.inbox_zero_streak_week_current);
                let subheader = match (streak, left(streak)) {
                    (0, _) => plain("Hit Inbox Zero to start a streak!"),
                    (_, 1) => plain("Hit Inbox Zero next week to reach the next level."),
                    (_, more) => bold(
                        "Hit Inbox Zero ",
                        format!("{} weeks in a row", n(streak + more)),
                        " to reach your next level.",
                    ),
                };
                ("Current Streak", subheader, streak, "Week")
            }
            Card::LongestStreak => {
                let longest = u64::from(statistics.inbox_zero_streak_week_max);
                let subheader = if longest == 0 {
                    plain("Hit Inbox Zero to start a streak!")
                } else {
                    bold(
                        "Your longest Inbox Zero streak is ",
                        format!("{} {}", n(longest), plural("week", longest)),
                        "!",
                    )
                };
                ("Longest Streak", subheader, longest, "Week")
            }
            Card::MarkedDone => {
                let done = statistics.archived_all_time;
                (
                    "Marked Done",
                    bold(
                        "Mark ",
                        format!("{} {}", n(left(done)), plural("conversation", left(done))),
                        " done to reach the next level.",
                    ),
                    done,
                    "Conversation",
                )
            }
            Card::GetMeToZero => {
                let cleared = statistics.gmtz_archived_all_time;
                (
                    "Get Me To Zero",
                    bold(
                        "Clear ",
                        format!(
                            "{} {}",
                            n(left(cleared)),
                            plural("conversation", left(cleared))
                        ),
                        " with Get Me To Zero to reach the next level.",
                    ),
                    cleared,
                    "Conversation",
                )
            }
            Card::SentMessages => {
                let sends = statistics.sends_all_time;
                let count = if sends == 0 {
                    "1 message".to_owned()
                } else {
                    format!("{} {}", n(left(sends)), plural("message", left(sends)))
                };
                (
                    "Sent Messages",
                    bold("Send ", count, " to reach the next level."),
                    sends,
                    "Message",
                )
            }
            Card::SendsLastWeek => {
                let sends = super::sends_last_week(statistics, shown.today);
                (
                    "Sends Last Week",
                    bold(
                        "You sent ",
                        format!("{} {}", n(sends), plural("message", sends)),
                        " last week.",
                    ),
                    sends,
                    "Message",
                )
            }
            Card::WeeklySendRecord => {
                let record = statistics.sends_week_max;
                let subheader = if record == 0 {
                    plain("Send your first message to get started!")
                } else {
                    bold(
                        "Your record is ",
                        format!("{} sent messages", n(record)),
                        " in one week!",
                    )
                };
                ("Weekly Send Record", subheader, record, "Message")
            }
        };
        Self {
            card,
            header,
            subheader,
            value,
            unit: plural(unit, value),
            new: super::is_new(card, shown.seen),
        }
    }

    /// The subheader's lines in `width` (`text-wrap: pretty`).
    fn lines(&self, ctx: &Context, width: f32) -> Vec<String> {
        let text: String = self.subheader.iter().map(|(run, _)| run.as_str()).collect();
        SUBHEADER.wrap_pretty(ctx, &text, width)
    }

    /// `padding: 16px 0`, less the separator's pixel; the content `margin-top: 4px`, under a
    /// 32px header.
    fn height(&self, ctx: &Context, width: f32) -> f32 {
        16.0 + 4.0
            + HEADER.line_height
            + self.lines(ctx, width).len() as f32 * SUBHEADER.line_height
            + 16.0
    }

    fn paint(
        &self,
        painter: &Painter,
        rect: Rect,
        width: f32,
        numbers: f32,
        accent: Color32,
        theme: Theme,
    ) {
        let ctx = painter.ctx();
        let top = rect.top() + 16.0 + 4.0;
        let header_width = HEADER.paint_line(
            painter,
            pos2(rect.left(), top),
            self.header,
            pick(theme, Color32::from_rgb(0x14, 0x14, 0x13), Color32::WHITE),
            None,
        );
        if self.new {
            // The tag: `NEW!` on `#fa9da9`, `margin-left: 8px; top: 8px`, 15px tall.
            let label = "NEW!";
            let tag = Rect::from_min_size(
                pos2(rect.left() + header_width + 8.0, top + 8.0),
                vec2(4.0 + TAG.width(ctx, label) + 4.0, 15.0),
            );
            painter.rect_filled(
                tag,
                CornerRadius::same(4),
                Color32::from_rgb(0xfa, 0x9d, 0xa9),
            );
            TAG.paint_line(
                painter,
                pos2(tag.left() + 4.0, tag.top() + 3.0),
                label,
                pick(theme, Color32::WHITE, Color32::from_black_alpha(191)),
                None,
            );
        }
        // `rgba(0,0,0,.6)`, `#bfbcb6` in Carbon.
        let ink = pick(
            theme,
            Color32::from_black_alpha(153),
            Color32::from_rgb(0xbf, 0xbc, 0xb6),
        );
        let mut y = top + HEADER.line_height + 3.0;
        let mut runs = self
            .subheader
            .iter()
            .flat_map(|(run, bold)| run.chars().map(move |c| (c, *bold)))
            .peekable();
        for line in self.lines(ctx, width) {
            let mut x = rect.left();
            // Each line's characters in the subheader's order, past the spaces it broke at.
            while runs
                .peek()
                .is_some_and(|(c, _)| c.is_whitespace() && !line.starts_with(' '))
            {
                runs.next();
            }
            let mut segment = String::new();
            let mut segment_bold = false;
            for expected in line.chars() {
                let Some((c, bold)) = runs.next() else { break };
                debug_assert_eq!(c, expected);
                if bold != segment_bold && !segment.is_empty() {
                    x += style(segment_bold).paint_line(painter, pos2(x, y), &segment, ink, None);
                    segment.clear();
                }
                segment_bold = bold;
                segment.push(c);
            }
            style(segment_bold).paint_line(painter, pos2(x, y), &segment, ink, None);
            y += SUBHEADER.line_height;
        }
        // The number, centered in its column (`text-align: center`): 120px in the main view,
        // at least 50px in a category's.
        let column = Rect::from_min_size(
            pos2(rect.right() - numbers, rect.top() + 16.0),
            vec2(numbers, VALUE.line_height + UNIT.line_height),
        );
        let value = super::abbreviated(self.value);
        let value_width = VALUE.width(ctx, &value);
        let unit_width = UNIT.width(ctx, &self.unit);
        let column = if numbers < 120.0 {
            column.with_min_x(column.right() - numbers.max(value_width).max(unit_width))
        } else {
            column
        };
        VALUE.paint_line(
            painter,
            pos2(column.center().x - value_width / 2.0, column.top()),
            &value,
            accent,
            None,
        );
        UNIT.paint_line(
            painter,
            pos2(
                column.center().x - unit_width / 2.0,
                column.top() + VALUE.line_height + 3.0,
            ),
            &self.unit,
            accent,
            None,
        );
    }
}

fn style(bold: bool) -> &'static TextStyle {
    if bold { &SUBHEADER_BOLD } else { &SUBHEADER }
}
