//! A split template's preview card: a white panel over the template's color with its name and
//! icon and three rows of sample conversations, skeleton bars where a row has no text, all in
//! Classic's grays (black at `.4`, skeletons `rgba(0, 0, 0, .12)`, row icons `.3`).

use egui::{Color32, CornerRadius, Painter, Rect, Stroke, StrokeKind, Ui, pos2, vec2};
use exo_store::{PreviewThread, Template};
use exo_ui::{Family, Font, TextStyle, paint_icon, paint_icon_two_tone, palette};

/// The header: 12px/12px, subdued.
const HEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 12.0);
/// A row's value: 8px, nudged down 2px.
const VALUE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 8.0, 8.0);
/// A row's label: 6px/9px 600 in an 8px chip.
const LABEL: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 6.0, 9.0);
/// The skeleton bars' widths where a row has no sender (`:nth-child(1..3)`).
const FROM_SKELETONS: [f32; 3] = [40.0, 60.0, 50.0];
/// The header and the rows: `rgba(0, 0, 0, .4)`.
const TEXT: Color32 = Color32::from_black_alpha(102);
/// A skeleton bar: `rgba(0, 0, 0, .1215686275)`.
const SKELETON: Color32 = Color32::from_black_alpha(31);
/// The rows' icons: `rgba(0, 0, 0, .3)`.
const ROW_ICON: Color32 = Color32::from_black_alpha(77);
/// The icon's height in Classic: 11px.
const ICON_HEIGHT: f32 = 11.0;

/// Where the card is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
    /// A library card: 4px corners, the rows 10px under the header.
    Library,
    /// Template settings' hero: square, the rows 12px under.
    Settings,
}

/// The template's color (`#rrggbbaa`), without its alpha or with it.
fn color(template: &Template, alpha: bool) -> Color32 {
    let color = &template.preview.color;
    let hex = if alpha {
        color.as_str()
    } else {
        color.get(..7).unwrap_or("#000000")
    };
    Color32::from_hex(hex).unwrap_or(Color32::BLACK)
}

/// The icon after the name: a brand's logo at 65% opacity, or Classic's legacy icon
/// for the template, in the template's color unless it's a brand's. The logos Classic draws as
/// inline images keep Origin's.
fn icon(template: &Template) -> Option<(String, bool)> {
    let brand = match template.id.as_str() {
        "splitTemplate_googleDocs" => Some("SocialGoogle"),
        "splitTemplate_msoffice" => Some("AppsMicrosoft"),
        "splitTemplate_otter" => Some("AppsOtter"),
        "splitTemplate_breezyHR" => Some("AppsBreezyhr"),
        "splitTemplate_fireflies" => Some("AppsFireflies"),
        "splitTemplate_docSend" => Some("AppsDocsend"),
        "splitTemplate_signNow" => Some("AppsSignnow"),
        "splitTemplate_signEasy" => Some("AppsSigneasy"),
        "splitTemplate_ashby" => Some("AppsAshby"),
        "splitTemplate_workable" => Some("AppsWorkable"),
        "splitTemplate_zoho" => Some("AppsZoho"),
        "splitTemplate_greenhouse" => Some("AppsGreenhouse"),
        "splitTemplate_github" => Some("AppsGithub"),
        "splitTemplate_pandaDoc" => Some("AppsPandadoc"),
        "splitTemplate_asana" => Some("AppsAsana"),
        "splitTemplate_clickUp" => Some("AppsClickup"),
        "splitTemplate_confluence" => Some("AppsConfluence"),
        "splitTemplate_docuSign" => Some("AppsDocusign"),
        "splitTemplate_figma" => Some("AppsFigma"),
        "splitTemplate_gong" => Some("AppsGong"),
        "splitTemplate_jira" => Some("AppsJira"),
        "splitTemplate_lever" => Some("AppsLever"),
        "splitTemplate_linear" => Some("AppsLinear"),
        "splitTemplate_loom" => Some("AppsLoom"),
        "splitTemplate_monday" => Some("AppsMonday"),
        "splitTemplate_notion" => Some("AppsNotion"),
        "splitTemplate_smartRecruiters" => Some("AppsSmartrecruiters"),
        "splitTemplate_trello" => Some("AppsTrello"),
        "splitTemplate_zoom" => Some("AppsZoom"),
        _ => None,
    };
    if let Some(brand) = brand {
        return Some((format!("origin/{brand}"), true));
    }
    let key = match template.id.as_str() {
        "splitTemplate_aha" => "aha".to_owned(),
        "splitTemplate_dropBoxSign" => "dropboxSign".to_owned(),
        "splitTemplate_workdayRecruiting" => "workdayRecruiting".to_owned(),
        "splitTemplate_chorus" => "chorusAi".to_owned(),
        "splitTemplate_signatureNeeded" => "signature".to_owned(),
        "splitTemplate_googleMeet" => "googleMeet".to_owned(),
        "splitTemplate_shared" => "share".to_owned(),
        "splitTemplate_reminders" => "reminders".to_owned(),
        _ => template.name.to_lowercase(),
    };
    let brand = match key.as_str() {
        "calendar" | "documents" | "finance" | "marketing" | "news" | "notes" | "notifications"
        | "purchases" | "reminders" | "social" | "starred" | "team" | "travel" | "vip"
        | "pitches" | "signature" | "share" => false,
        "salesforce" | "hubspot" | "pipedrive" | "aha" | "chorusAi" | "dropboxSign"
        | "googleMeet" | "workdayRecruiting" => true,
        _ => return None,
    };
    Some((format!("legacy/DEPRECATED_{key}"), brand))
}

/// Paints `template`'s preview filling `rect`, clipped to it.
pub(super) fn paint(ui: &Ui, rect: Rect, template: &Template, place: Place) {
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let tint = color(template, false);
    let radius = match place {
        Place::Library => CornerRadius::same(4),
        // In the settings, square corners.
        Place::Settings => CornerRadius::ZERO,
    };
    // The background is the template's color at its own alpha; the panel is `#fff`.
    painter.rect_filled(rect, radius, color(template, true));
    let content = Rect::from_min_max(rect.min + vec2(12.0, 12.0), rect.max);
    painter.rect_filled(
        content,
        CornerRadius {
            nw: 4,
            se: 4,
            ..CornerRadius::ZERO
        },
        Color32::WHITE,
    );

    // The header: the name, then 4px on its icon.
    let left = content.left() + 20.0;
    let header = content.top() + 14.0;
    let icon = icon(template);
    // With an icon the header's line grows to 14px: the 11px icon sits on the name's baseline,
    // 2px above the name's own 12px line.
    let name_top = if icon.is_some() { header + 2.0 } else { header };
    let name = HEADER.paint_line(&painter, pos2(left, name_top), &template.name, TEXT, None);
    if let Some((icon, brand)) = &icon {
        // 11px tall at its own width (a legacy `<svg>`'s `width`, a logo's aspect), 4px on.
        let size = exo_ui::icon_size(ui.ctx(), icon).unwrap_or(vec2(12.0, 12.0));
        let width = if icon.starts_with("legacy/") {
            size.x
        } else {
            size.x * ICON_HEIGHT / size.y
        };
        let at = exo_ui::pixel_snap(Rect::from_min_size(
            pos2(left + name + 4.0, header),
            vec2(width, ICON_HEIGHT),
        ));
        if *brand {
            // A brand's logo at 65% opacity.
            paint_icon(ui, icon, at, Color32::WHITE.gamma_multiply(0.65));
        } else {
            // The placeholder in the template's color; the rest (some carry a white backing
            // square) keeps its own color.
            paint_icon_two_tone(ui, icon, at, tint, tint);
        }
    }
    let gap = if place == Place::Library { 10.0 } else { 12.0 };
    let mut row = header + if icon.is_some() { 14.0 } else { 12.0 } + gap;
    let rows: Vec<Option<&PreviewThread>> = match &template.preview.threads {
        Some(threads) => threads.iter().map(Option::as_ref).collect(),
        None => vec![None; 3],
    };
    let reminders = template.id == "splitTemplate_reminders";
    for (index, thread) in rows.into_iter().enumerate() {
        paint_row(
            &painter,
            ui,
            thread,
            index,
            left,
            row,
            content.right(),
            tint,
            reminders,
        );
        // A label chip (`margin-top: 1px`) or an event's icon makes the row 9px; `row-gap: 9px`.
        let tall = thread.is_some_and(|thread| {
            thread.label.is_some() || (thread.is_event && thread.subject.is_some())
        });
        row += if tall { 9.0 } else { 8.0 } + 9.0;
    }

    // The border: Classic's white 1px under, then a `rgba(0, 0, 0, .09)` hairline over
    // everything.
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, Color32::WHITE),
        StrokeKind::Inside,
    );
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, Color32::from_black_alpha(23)),
        StrokeKind::Inside,
    );
}

/// A row 8px tall at `top`: the sender (or a skeleton bar) in a column at least 70px wide, 10px
/// gap, then a label chip or shared icon, the subject (or a bar to the edge), an event's icon.
#[expect(
    clippy::too_many_arguments,
    reason = "one row of the card, drawn from its layout"
)]
fn paint_row(
    painter: &Painter,
    ui: &Ui,
    thread: Option<&PreviewThread>,
    index: usize,
    left: f32,
    top: f32,
    right: f32,
    tint: Color32,
    reminders: bool,
) {
    let tokens = palette(ui.ctx());
    let bar = |x: f32, width: f32| Rect::from_min_size(pos2(x, top + 1.0), vec2(width, 8.0));
    // A starred or returned reminder's 6px dot, 4px before the sender.
    let dot = match thread {
        Some(thread) if thread.is_starred => Some(tokens.actions_star),
        _ if reminders => Some(tokens.actions_reminder_default),
        _ => None,
    };
    if let Some(dot) = dot {
        painter.circle_filled(pos2(left - 10.0 + 3.0, top + 2.0 + 3.0), 3.0, dot);
    }
    let from = match thread.and_then(|thread| thread.from.as_deref()) {
        Some(from) => VALUE.paint_line(painter, pos2(left, top + 2.0), from, TEXT, None),
        None => {
            let width = FROM_SKELETONS[index.min(2)];
            painter.rect_filled(bar(left, width), CornerRadius::same(2), SKELETON);
            width
        }
    };
    let mut x = left + from.max(70.0) + 10.0;
    if let Some(label) = thread.and_then(|thread| thread.label.as_deref()) {
        let color = thread
            .and_then(|thread| thread.label_color.as_deref())
            .and_then(|hex| Color32::from_hex(hex.get(..7).unwrap_or(hex)).ok())
            .unwrap_or(tint);
        let width = LABEL.width(ui.ctx(), label) + 4.0;
        let chip = Rect::from_min_size(pos2(x, top + 1.0), vec2(width, 8.0));
        painter.rect_filled(chip, CornerRadius::same(1), color);
        LABEL.paint_line(
            painter,
            pos2(x + 2.0, chip.top()),
            label,
            tokens.text_base_inverse,
            None,
        );
        x = chip.right() + 4.0;
    } else if thread.is_some_and(|thread| thread.is_shared) {
        exo_ui::paint_icon_on(
            painter,
            "legacy/DEPRECATED_share",
            Rect::from_min_size(pos2(x, top + 1.0), vec2(12.0, 12.0)),
            ROW_ICON,
        );
        x += 12.0 + 4.0;
    }
    match thread.and_then(|thread| thread.subject.as_deref()) {
        Some(subject) => {
            let width = VALUE.paint_line(
                painter,
                pos2(x, top + 2.0),
                subject,
                TEXT,
                Some((right - x).max(0.0)),
            );
            if thread.is_some_and(|thread| thread.is_event) {
                // The calendar icon: 8px, `margin: 1px 8px 0 4px`.
                exo_ui::paint_icon_on(
                    painter,
                    "legacy/DEPRECATED_calendar",
                    Rect::from_min_size(pos2(x + width + 4.0, top + 1.0), vec2(8.0, 8.0)),
                    ROW_ICON,
                );
            }
        }
        None => {
            painter.rect_filled(
                bar(x, (right - x).max(0.0)),
                CornerRadius::same(2),
                SKELETON,
            );
        }
    }
}
