//! The dialog: the toggle, the first and last day, the subject, the message and who gets replies,
//! over Cancel and Save.
//!
//! It has a light look of its own, in Snow and Carbon alike: Exo Sans Alt in the light theme's
//! text colors, a purple switch and Save. The date inputs and the message keep Classic's look.

use egui::{Color32, Context, CornerRadius, Id, Mesh, Pos2, Rect, Sense, Ui, pos2, vec2};
use exo_editor::{ActiveDate, BodyText, Editor, EditorStyle};
use exo_store::{OutOfOfficeDays, OutOfOfficeResponder};
use exo_ui::{ActionKind, DateInput, DialogAction, Family, Font, SliderColors, TextStyle};
use gmail::VacationSettings;
use jiff::{SignedDuration, Timestamp, Zoned, civil::Date};

use super::date_input::{self, DateEvent};

/// The light look's text colors: default, subdued and accent.
const TEXT_DEFAULT: Color32 = Color32::from_rgb(0x14, 0x14, 0x13);
const TEXT_SUBDUED: Color32 = Color32::from_rgb(0x73, 0x71, 0x6d);
const TEXT_ACCENT: Color32 = Color32::from_rgb(0x79, 0x76, 0x72);
/// The header: Exo Sans Alt 16px/20px.
const HEADER: TextStyle = TextStyle::new(Font::new(Family::SansAlt, 400), 16.0, 20.0);
/// A label: 600 12px/20px, `letter-spacing: .06px` (Classic's), `top: 3px`.
const LABEL: TextStyle =
    TextStyle::new(Font::new(Family::SansAlt, 600), 12.0, 20.0).letter_spacing(0.06);
/// The text, the subject and the checkboxes' labels: 400 12px/20px, `top: 3px`.
const TEXT: TextStyle = TextStyle::new(Font::new(Family::SansAlt, 400), 12.0, 20.0);
/// The toggle's switch: the light look's neutral background off and its activated accent on, under
/// Classic's filter.
const SWITCH: SliderColors = SliderColors {
    off: Color32::from_rgb(0x73, 0x71, 0x6d),
    on: Color32::from_rgb(0x71, 0x4c, 0xb6),
};
/// A checked box: the activated accent with a white tick; unchecked, the text colour at a quarter.
const CHECKBOX: [Color32; 3] = [
    Color32::from_rgb(0x71, 0x4c, 0xb6),
    Color32::WHITE,
    TEXT_DEFAULT,
];
/// `hr`'s and the message's `border-bottom: 1px solid rgba(0, 0, 0, .1)`.
const RULE: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 26);
/// A loading skeleton: `rgba(0, 0, 0, .08)`, 16px tall, 4px corners, a white shine crossing it
/// every 2s.
const SKELETON: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 20);
const SKELETON_HEIGHT: f32 = 16.0;

/// The body's rows, from the body's top (below the header): the toggle's line box, the switch,
/// the rule, the dates' line, the subject's label and input, the message's label, the editor
/// (the Squire 1px up) and its rule, the recipients' lines, and the footer's buttons.
const TOGGLE_TOP: f32 = 23.0;
const SWITCH_TOP: f32 = 24.0;
const SWITCH_GAP: f32 = 12.0;
const HR_TOP: f32 = 56.0;
const DATES_TOP: f32 = 79.0;
/// The date inputs' line box: 2px under the text's (`top: -1px` on the picker, `top: 1px` and
/// a 16px line in the input).
const DATE_INPUT_TOP: f32 = 81.0;
/// A date picker's `padding: 0 4px`; the two spans 16px apart.
const DATE_PADDING: f32 = 4.0;
const DATES_GAP: f32 = 16.0;
const SUBJECT_LABEL_TOP: f32 = 119.0;
const SUBJECT_TOP: f32 = 151.0;
const MESSAGE_LABEL_TOP: f32 = 195.0;
const EDITOR_TOP: f32 = 223.0;
/// The message editor's 164px height, less the stable gutter of its scrollbar (11px, 12px in
/// Carbon).
const EDITOR_HEIGHT: f32 = 164.0;
const MESSAGE_RULE_TOP: f32 = 395.0;
const RECIPIENTS_TOP: f32 = 419.0;
const CONTACTS_TOP: f32 = 447.0;
const DOMAIN_TOP: f32 = 475.0;
/// A checkbox's 14px and the 10px after it.
const CHECKBOX_TEXT_LEFT: f32 = 24.0;
const ACTIONS_TOP: f32 = 507.0;
/// The actions' `margin-right: -13px`, 8px between the secondary one and the next.
const ACTIONS_RIGHT: f32 = 13.0;
const ACTIONS_GAP: f32 = 8.0;
/// Down to the footer's `margin-bottom: 7px`.
const BODY_HEIGHT: f32 = 539.0;
/// While loading, skeletons stand in for the dates (20px under the rule) and the subject (12px
/// down), and the message's label sits 4px lower (`padding-top: 28px`): everything from the
/// subject's label down ends up 1px lower.
const LOADING_DATES: f32 = 77.0;
const LOADING_SUBJECT: f32 = 149.0;
const LOADING_SHIFT: f32 = 1.0;

/// The subject takes 250 characters.
const SUBJECT_MAX: usize = 250;

/// The form's fields (the dialog's state).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form {
    /// `Enable Out of Office Reply`.
    pub enabled: bool,
    /// The first day's start (a local midnight once picked).
    pub start: Timestamp,
    /// The last day's end, its last second (`23:59:59`), if there's a last day.
    pub end: Option<Timestamp>,
    /// `Subject`.
    pub subject: String,
    /// `Message`, as the editor writes it.
    pub body: String,
    /// `People in my contacts`.
    pub contacts: bool,
    /// `People at {domain}`.
    pub domain: bool,
}

impl Form {
    /// What `copy` fills the form with at `now`: its days when they're still to come or the reply
    /// is on, else today and no last day.
    pub fn of(copy: Option<&OutOfOfficeResponder>, now: &Zoned) -> Self {
        let today = midnight(now.date(), now);
        let copy = copy.cloned().unwrap_or_default();
        let kept = copy
            .start_time
            .and_then(|ms| Timestamp::from_millisecond(ms).ok())
            .filter(|start| *start >= today || copy.is_enabled());
        Self {
            enabled: copy.is_enabled(),
            start: kept.unwrap_or(today),
            end: kept
                .and(copy.end_time)
                .and_then(|ms| Timestamp::from_millisecond(ms).ok())
                .map(|end| end - SignedDuration::from_secs(1)),
            subject: copy.google.response_subject,
            body: Editor::from_html(&copy.google.response_body_html).html(),
            contacts: copy.google.restrict_to_contacts,
            domain: copy.google.restrict_to_domain,
        }
    }

    /// What Save sends Gmail: the days as Gmail's `startTime` and `endTime`, a second past the last
    /// day's last second.
    pub fn vacation(&self) -> VacationSettings {
        VacationSettings {
            enable_auto_reply: Some(self.enabled),
            response_subject: Some(self.subject.clone()),
            response_body_html: Some(self.body.clone()),
            response_body_plain_text: None,
            restrict_to_contacts: Some(self.contacts),
            restrict_to_domain: Some(self.domain),
            start_time: Some(self.start.as_millisecond()),
            end_time: self
                .end
                .map(|end| (end + SignedDuration::from_secs(1)).as_millisecond()),
        }
    }
}

/// A focusable control, in tab order: the numbered ones first (subject 4 to Cancel 9), then the
/// switch and the date inputs in the page's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    /// `Subject` (4).
    Subject,
    /// `Message` (5).
    Message,
    /// `People in my contacts` (6).
    Contacts,
    /// `People at {domain}` (7).
    Domain,
    /// Save (8).
    Save,
    /// Cancel (9).
    Cancel,
    /// The toggle's switch.
    Switch,
    /// `First day:`.
    Start,
    /// `Last day:`.
    End,
}

const TAB_ORDER: [Control; 9] = [
    Control::Subject,
    Control::Message,
    Control::Contacts,
    Control::Domain,
    Control::Save,
    Control::Cancel,
    Control::Switch,
    Control::Start,
    Control::End,
];

/// What happened in the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Cancel, the ×, the backdrop or Escape.
    Close,
    /// Save.
    Save,
    /// Tab (1) or ⇧Tab (-1).
    Tab(isize),
    /// Enter on the focused control.
    Enter,
    /// Space on the focused control.
    Space,
    /// ↓ in a date input: its month opens.
    Down,
    /// A file was pasted in the message.
    PastedFile,
}

/// The dialog on screen, or fading out.
pub struct Dialog {
    /// It hasn't been closed.
    pub open: bool,
    /// Gmail hasn't answered yet.
    pub loading: bool,
    /// Save is waiting on Gmail.
    pub submitting: bool,
    /// The form.
    pub form: Form,
    /// What the copy filled it with, for the discard check; the toggle as the copy had it.
    initial: Form,
    /// The days the nudge opened it with.
    days: Option<OutOfOfficeDays>,
    editor: Editor,
    /// The subject's spelling, as a field's, autocorrect or not.
    pub subject_spelling: exo_editor::FieldSpelling,
    /// The dates' spelling, which is checked too.
    pub start_spelling: exo_editor::FieldSpelling,
    pub end_spelling: exo_editor::FieldSpelling,
    /// A right-click in the message, for the app's menu.
    pub message_menu: Option<exo_editor::ContextMenu>,
    /// A word the message's autocorrect pill learned, for the app's hint.
    pub message_learned: Option<String>,
    /// The detected date the message's caret is on.
    date: Option<ActiveDate>,
    /// A date the caret came onto, or whose text changed under it, for the app to show its day.
    day: Option<String>,
    start: DateInput,
    end: DateInput,
    focus: Option<Control>,
    /// `People at {domain}`, unless the account is on `gmail.com`.
    domain: Option<String>,
    scroll: f32,
}

impl Dialog {
    /// The dialog for `email`'s account, loading; `days` from the nudge.
    pub fn new(email: &str, days: Option<OutOfOfficeDays>, now: &Zoned) -> Self {
        let form = Form::of(None, now);
        let domain = email
            .rsplit_once('@')
            .map(|(_, domain)| domain.to_owned())
            .filter(|domain| !domain.eq_ignore_ascii_case("gmail.com"));
        let mut editor = Editor::new();
        editor.set_body_text(BodyText::SMALL);
        Self {
            open: true,
            loading: true,
            submitting: false,
            initial: form.clone(),
            start: DateInput::showing(
                Some(form.start.to_zoned(now.time_zone().clone()).date()),
                now.date(),
            ),
            end: DateInput::default(),
            form,
            days,
            editor,
            subject_spelling: exo_editor::FieldSpelling::default(),
            start_spelling: exo_editor::FieldSpelling::default(),
            end_spelling: exo_editor::FieldSpelling::default(),
            message_menu: None,
            message_learned: None,
            date: None,
            day: None,
            focus: None,
            domain,
            scroll: 0.0,
        }
    }

    /// Gmail answered and `copy` is up to date: the form takes it, and the nudge's days with the
    /// reply on.
    pub fn loaded(&mut self, copy: Option<&OutOfOfficeResponder>, now: &Zoned) {
        self.loading = false;
        self.initial = Form::of(copy, now);
        self.form = self.initial.clone();
        if let Some(days) = &self.days {
            self.form.start = days.start;
            self.form.end = Some(days.end);
            self.form.enabled = true;
        }
        self.editor.set_html(&self.form.body);
        self.show_dates(now);
    }

    /// The form differs from what it was filled with.
    pub fn changed(&self) -> bool {
        !self.loading && self.form != self.initial
    }

    /// Save can't be pressed: it's waiting, the reply is off and was when it opened, the subject or
    /// message is empty, or a typed date is wrong.
    pub fn save_disabled(&self) -> bool {
        self.loading
            || self.submitting
            || (!self.form.enabled && !self.initial.enabled)
            || self.form.subject.is_empty()
            || self.editor.is_empty()
            || self.start.error
            || self.end.error
    }

    /// What Save sends.
    pub fn vacation(&self) -> VacationSettings {
        self.form.vacation()
    }

    /// The control with the keyboard.
    pub fn focus(&self) -> Option<Control> {
        self.focus
    }

    /// Whether the message's editor has the keyboard and something of its own for Escape.
    pub fn editor_wants_escape(&self) -> bool {
        self.focus == Some(Control::Message) && self.editor.wants_escape()
    }

    /// Does `choice` from the menu of a right-click `menu` in the first date (`start`) or the
    /// last.
    pub(crate) fn date_choice(
        &mut self,
        ctx: &egui::Context,
        start: bool,
        menu: &exo_editor::ContextMenu,
        choice: crate::page_menu::Choice,
    ) -> crate::spelling::Done {
        let (spelling, input) = if start {
            (&mut self.start_spelling, &mut self.start)
        } else {
            (&mut self.end_spelling, &mut self.end)
        };
        crate::spelling::field_choice(ctx, spelling, &mut input.text, menu, choice)
    }

    /// Whether the message's caret is on a detected date, which ⌘⇧– and ⌘⇧= move.
    pub fn on_date(&self) -> bool {
        self.focus == Some(Control::Message) && self.date.is_some()
    }

    /// The date the message's caret came onto since the last call, to show its day.
    pub fn take_day(&mut self) -> Option<String> {
        self.day.take()
    }

    /// Moves the date the message's caret is on by `days` from its day `today`, as compose does.
    pub fn shift_date(&mut self, days: i64, today: &Zoned) {
        if let Some(date) = self.editor.active_date()
            && let Some(text) = crate::thread::dates::shift(&date.text, days, today)
        {
            self.editor.shift_date(&date, &text);
            self.form.body = self.editor.html();
        }
    }

    /// The editor, for its formatting commands.
    pub fn editor_mut(&mut self) -> &mut Editor {
        &mut self.editor
    }

    /// The editor, to ask which formatting commands apply.
    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    /// Flips the reply on or off.
    pub fn toggle(&mut self) {
        if !self.loading {
            self.form.enabled = !self.form.enabled;
            if !self.form.enabled {
                self.focus = self
                    .focus
                    .filter(|control| matches!(control, Control::Switch | Control::Cancel));
            }
        }
    }

    /// Moves the keyboard `step` controls along the tab order, skipping disabled ones.
    pub fn tab(&mut self, step: isize, now: &Zoned) {
        // Tab in a date input takes what's typed. The first day's keeps the key (only the last
        // day's lets Tab move on), and taking a first day moves to the last.
        let from = self.focus;
        if self.commit(now) && from == Some(Control::Start) {
            return;
        }
        let available: Vec<Control> = TAB_ORDER
            .into_iter()
            .filter(|control| self.focusable(*control))
            .collect();
        if available.is_empty() {
            return;
        }
        let at = self
            .focus
            .and_then(|focus| available.iter().position(|control| *control == focus));
        let len = available.len() as isize;
        let next = match at {
            Some(at) => (at as isize + step).rem_euclid(len),
            None if step > 0 => 0,
            None => len - 1,
        };
        self.set_focus(Some(available[next as usize]), now);
    }

    fn focusable(&self, control: Control) -> bool {
        match control {
            Control::Switch => !self.loading,
            Control::Cancel => true,
            Control::Save => !self.save_disabled(),
            Control::Domain => self.form.enabled && self.domain.is_some(),
            _ => self.form.enabled && !self.loading,
        }
    }

    fn set_focus(&mut self, focus: Option<Control>, now: &Zoned) {
        if focus == self.focus {
            return;
        }
        self.focus = focus;
        // A date input's month opens when it gets the keyboard and closes when it loses it.
        let today = now.date();
        for (control, input) in [
            (Control::Start, &mut self.start),
            (Control::End, &mut self.end),
        ] {
            input.month =
                (focus == Some(control)).then(|| input.parsed.unwrap_or(today).first_of_month());
        }
    }

    /// Enter on the focused control.
    pub fn enter(&mut self, now: &Zoned) -> Option<Event> {
        match self.focus? {
            Control::Switch => self.toggle(),
            Control::Contacts => self.form.contacts = !self.form.contacts,
            Control::Domain => self.form.domain = !self.form.domain,
            Control::Save => return Some(Event::Save),
            Control::Cancel => return Some(Event::Close),
            Control::Start | Control::End => {
                self.commit(now);
            }
            Control::Subject | Control::Message => {}
        }
        None
    }

    /// Space on the focused control: what Enter does, but in the date inputs (where it's typed).
    pub fn space(&mut self, now: &Zoned) -> Option<Event> {
        match self.focus? {
            Control::Start | Control::End | Control::Subject | Control::Message => None,
            _ => self.enter(now),
        }
    }

    /// ↓ in a date input opens its month.
    pub fn down(&mut self, now: &Zoned) {
        let today = now.date();
        let input = match self.focus {
            Some(Control::Start) => &mut self.start,
            Some(Control::End) => &mut self.end,
            _ => return,
        };
        if input.month.is_none() {
            input.month = Some(input.parsed.unwrap_or(today).first_of_month());
        }
    }

    /// Takes what's typed in the focused date input, when it's a date (Enter, Tab). Returns
    /// whether it did.
    fn commit(&mut self, now: &Zoned) -> bool {
        let (input, start) = match self.focus {
            Some(Control::Start) => (&self.start, true),
            Some(Control::End) => (&self.end, false),
            _ => return false,
        };
        let Some(date) = input.parsed.filter(|_| !input.error) else {
            return false;
        };
        if start {
            self.pick_start(date, now);
        } else {
            // The month closes; the input keeps the keyboard.
            self.pick_end(Some(date), now);
            self.end.month = None;
        }
        true
    }

    /// Picks the first day: it moves to `date`; a last day before it moves along to keep the time
    /// away as long; the last day's input gets the keyboard.
    fn pick_start(&mut self, date: Date, now: &Zoned) {
        let start = midnight(date, now);
        if let Some(end) = self.form.end.filter(|end| start > *end) {
            self.form.end = Some(start + end.duration_since(self.form.start));
        }
        self.form.start = start;
        self.start.error = false;
        self.show_dates(now);
        self.set_focus(Some(Control::End), now);
    }

    /// Picks the last day: `date`'s last second, or none; the input keeps the keyboard, its month
    /// closed.
    fn pick_end(&mut self, date: Option<Date>, now: &Zoned) {
        self.form.end = date
            .and_then(|date| {
                date.at(23, 59, 59, 0)
                    .to_zoned(now.time_zone().clone())
                    .ok()
            })
            .map(|end| end.timestamp());
        self.end.error = false;
        self.show_dates(now);
    }

    /// The date inputs show the form's days again.
    fn show_dates(&mut self, now: &Zoned) {
        let tz = now.time_zone().clone();
        let today = now.date();
        let start = self.form.start.to_zoned(tz.clone()).date();
        let end = self.form.end.map(|end| end.to_zoned(tz).date());
        let (start_month, end_month) = (
            self.start.month.map(|_| start.first_of_month()),
            self.end.month.and(end.map(Date::first_of_month)),
        );
        self.start = DateInput {
            month: start_month,
            ..DateInput::showing(Some(start), today)
        };
        self.end = DateInput {
            month: end_month,
            ..DateInput::showing(end, today)
        };
    }

    /// Draws the dialog over `area` and returns what happened.
    pub fn show(&mut self, ctx: &Context, area: Rect, now: &Zoned) -> Option<Option<Event>> {
        let carbon = exo_ui::theme(ctx) == exo_ui::Theme::Carbon;
        let height = if self.loading {
            BODY_HEIGHT + LOADING_SHIFT
        } else {
            BODY_HEIGHT
        };
        let response = exo_ui::Dialog::new("out-of-office-reply", "Out of Office Reply", height)
            .header_style(HEADER, TEXT_DEFAULT)
            .show(ctx, area, self.open, |ui, body| {
                self.body(ui, body, now, carbon)
            });
        let inner = response.inner?;
        Some(if response.close {
            Some(Event::Close)
        } else {
            inner
        })
    }

    fn body(&mut self, ui: &mut Ui, body: Rect, now: &Zoned, carbon: bool) -> Option<Event> {
        let ctx = ui.ctx().clone();
        let id = exo_ui::id(&ctx, "out-of-office-reply");
        let today = now.date();
        let enabled = self.form.enabled;
        let disabled = !enabled;
        let (left, top, width) = (body.left(), body.top(), body.width());
        let painter = ui.painter().clone();
        let mut event = None;
        let subdued_if = |off: bool| if off { TEXT_SUBDUED } else { TEXT_DEFAULT };
        let shift = if self.loading { LOADING_SHIFT } else { 0.0 };
        // A click that no control takes gives the keyboard back to the form.
        let clicked = ui.input(|input| input.pointer.any_click())
            && ctx
                .pointer_interact_pos()
                .is_some_and(|at| body.contains(at));
        let focus_before = self.focus;
        let mut claimed = false;

        // The toggle: its label greys out only while loading.
        let label_width = LABEL.paint_line(
            &painter,
            pos2(left, top + TOGGLE_TOP),
            "Enable Out of Office Reply",
            subdued_if(self.loading),
            None,
        );
        // The switch's filtered layer lands on a whole CSS pixel (632.66 paints at 633).
        let switch = Rect::from_min_size(
            pos2((left + label_width + SWITCH_GAP).round(), top + SWITCH_TOP),
            exo_ui::SLIDER_SIZE,
        );
        if self.focus == Some(Control::Switch) {
            // Focused: Classic's `box-shadow: 0 0 0 3px rgba(0,0,0,.1)`.
            painter.rect_filled(
                switch.expand(3.0),
                CornerRadius::same(11),
                Color32::from_black_alpha(26),
            );
        }
        exo_ui::paint_slider(
            &painter,
            switch,
            enabled,
            self.loading,
            id.with("switch"),
            SWITCH,
        );
        if !self.loading
            && ui
                .interact(switch, id.with("switch-hit"), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
        {
            self.toggle();
            self.set_focus(Some(Control::Switch), now);
            claimed = true;
        }
        painter.rect_filled(
            Rect::from_min_size(pos2(left, top + HR_TOP), vec2(width, 1.0)),
            0.0,
            RULE,
        );

        // The first and last day.
        if self.loading {
            skeleton(
                ui,
                Rect::from_min_size(
                    pos2(left, top + LOADING_DATES),
                    vec2(width, SKELETON_HEIGHT),
                ),
            );
        } else {
            let first_width = TEXT.paint_line(
                &painter,
                pos2(left, top + DATES_TOP),
                "First day:",
                subdued_if(disabled),
                None,
            );
            let mut start_field = date_input::Field {
                id: id.with("start"),
                input: &mut self.start,
                focused: self.focus == Some(Control::Start),
                disabled,
                placeholder: "",
                min: today,
                today,
                spelling: &mut self.start_spelling,
            };
            let start_left = left + first_width + DATE_PADDING;
            let start_width = start_field.width(&ctx);
            let start_event = start_field.show(ui, pos2(start_left, top + DATE_INPUT_TOP));
            let last_left = start_left + start_width + DATE_PADDING + DATES_GAP;
            let last_width = TEXT.paint_line(
                &painter,
                pos2(last_left, top + DATES_TOP),
                "Last day:",
                subdued_if(disabled),
                None,
            );
            let start_date = self.form.start.to_zoned(now.time_zone().clone()).date();
            let mut end_field = date_input::Field {
                id: id.with("end"),
                input: &mut self.end,
                focused: self.focus == Some(Control::End),
                disabled,
                placeholder: "Select date",
                min: start_date,
                today,
                spelling: &mut self.end_spelling,
            };
            let end_event = end_field.show(
                ui,
                pos2(last_left + last_width + DATE_PADDING, top + DATE_INPUT_TOP),
            );
            claimed |= start_event.is_some() || end_event.is_some();
            self.date_event(Control::Start, start_event, now);
            self.date_event(Control::End, end_event, now);
        }

        // The subject.
        LABEL.paint_line(
            &painter,
            pos2(left, top + SUBJECT_LABEL_TOP + shift),
            "Subject",
            subdued_if(disabled),
            None,
        );
        if self.loading {
            skeleton(
                ui,
                Rect::from_min_size(
                    pos2(left, top + LOADING_SUBJECT),
                    vec2(width, SKELETON_HEIGHT),
                ),
            );
        } else {
            let line =
                Rect::from_min_size(pos2(left, top + SUBJECT_TOP), vec2(width, TEXT.line_height));
            // `input[type=text]`: white (`#f5f5f5` in Carbon) behind it.
            painter.rect_filled(
                line,
                0.0,
                if carbon {
                    Color32::from_rgb(0xf5, 0xf5, 0xf5)
                } else {
                    Color32::WHITE
                },
            );
            if disabled {
                // Disabled: the accent text color at `.4`.
                let ink = TEXT_ACCENT.gamma_multiply(0.4);
                let text = if self.form.subject.is_empty() {
                    "Enter subject"
                } else {
                    &self.form.subject
                };
                TEXT.paint_line(&painter, line.min, text, ink, Some(width));
            } else {
                if self.form.subject.is_empty() {
                    TEXT.paint_line(&painter, line.min, "Enter subject", TEXT_SUBDUED, None);
                }
                let field_id = id.with("subject");
                let field = self.subject_spelling.show(
                    ui,
                    field_id,
                    &mut self.form.subject,
                    line,
                    &TEXT,
                    TEXT_DEFAULT,
                    exo_ui::palette(&ctx).text_selection_default,
                );
                if self.form.subject.chars().count() > SUBJECT_MAX {
                    self.form.subject = self.form.subject.chars().take(SUBJECT_MAX).collect();
                }
                if field.clicked() || field.gained_focus() {
                    self.set_focus(Some(Control::Subject), now);
                    claimed = true;
                }
                hold_focus(ui, self.focus == Some(Control::Subject), field_id);
            }
        }

        // The message.
        LABEL.paint_line(
            &painter,
            pos2(left, top + MESSAGE_LABEL_TOP + shift),
            "Message",
            subdued_if(disabled),
            None,
        );
        let editor_area = Rect::from_min_size(
            pos2(left, top + EDITOR_TOP + 1.0 + shift),
            vec2(width, EDITOR_HEIGHT),
        );
        if self.loading {
            // Three skeleton lines, the last 40% wide, 4px apart.
            for (index, share) in [1.0, 1.0, 0.4].into_iter().enumerate() {
                skeleton(
                    ui,
                    Rect::from_min_size(
                        editor_area.min + vec2(0.0, index as f32 * (SKELETON_HEIGHT + 4.0)),
                        vec2(width * share, SKELETON_HEIGHT),
                    ),
                );
            }
        } else {
            let (pasted, focused) = self.message(ui, editor_area, carbon, disabled, now);
            claimed |= focused;
            event = pasted.or(event);
        }
        painter.rect_filled(
            Rect::from_min_size(pos2(left, top + MESSAGE_RULE_TOP + shift), vec2(width, 1.0)),
            0.0,
            RULE,
        );

        // Who gets replies.
        TEXT.paint_line(
            &painter,
            pos2(left, top + RECIPIENTS_TOP + shift),
            "Only send automatic replies to:",
            subdued_if(disabled),
            None,
        );
        let domain = self.domain.clone();
        let rows: Vec<(Control, String, f32, f32, bool)> = std::iter::once((
            Control::Contacts,
            "People in my contacts".to_owned(),
            CONTACTS_TOP,
            -1.0,
            self.form.contacts,
        ))
        .chain(domain.map(|domain| {
            (
                Control::Domain,
                format!("People at {domain}"),
                DOMAIN_TOP,
                0.0,
                self.form.domain,
            )
        }))
        .collect();
        for (control, label, row_top, nudge, checked) in rows {
            let line_top = top + row_top + shift;
            let text_width = TEXT.paint_line(
                &painter,
                pos2(left + CHECKBOX_TEXT_LEFT, line_top),
                &label,
                subdued_if(disabled),
                None,
            );
            // A checkbox's `margin-top: 1px` (the contacts' `top: -1px`).
            let mark = pos2(left, line_top + 1.0 + nudge);
            if self.focus == Some(control) {
                // Focused: Classic's `outline: 5px solid rgba(0,0,0,.05)`, 3px corners.
                let ring = Rect::from_min_size(mark, vec2(14.0, 14.0)).expand(5.0);
                painter.rect_stroke(
                    ring.shrink(2.5),
                    CornerRadius::same(5),
                    egui::Stroke::new(5.0, Color32::from_black_alpha(13)),
                    egui::StrokeKind::Middle,
                );
            }
            let inks = CHECKBOX.map(|ink| {
                if disabled {
                    ink.gamma_multiply(0.5)
                } else {
                    ink
                }
            });
            exo_ui::paint_checkbox_inks(&painter, mark, checked, inks);
            let row = Rect::from_min_size(
                pos2(left, line_top),
                vec2(CHECKBOX_TEXT_LEFT + text_width, TEXT.line_height),
            );
            if enabled
                && ui
                    .interact(row, id.with(("recipients", control as u8)), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
            {
                match control {
                    Control::Contacts => self.form.contacts = !self.form.contacts,
                    _ => self.form.domain = !self.form.domain,
                }
                self.set_focus(Some(control), now);
                claimed = true;
            }
        }

        // Cancel and Save.
        let save = DialogAction::new("Save", ActionKind::Primary)
            .origin_light()
            .enabled(!self.save_disabled())
            .focused(self.focus == Some(Control::Save));
        let cancel = DialogAction::new("Cancel", ActionKind::Plain)
            .origin_light()
            .focused(self.focus == Some(Control::Cancel));
        let right = body.right() + ACTIONS_RIGHT;
        let actions_top = top + ACTIONS_TOP + shift;
        let save_rect = Rect::from_min_max(
            pos2(right - save.width(&ctx), actions_top),
            pos2(right, actions_top + exo_ui::DIALOG_ACTION_HEIGHT),
        );
        let cancel_right = save_rect.left() - ACTIONS_GAP;
        let cancel_rect = Rect::from_min_max(
            pos2(cancel_right - cancel.width(&ctx), save_rect.top()),
            pos2(cancel_right, save_rect.bottom()),
        );
        if cancel.show(ui, id.with("cancel"), cancel_rect).clicked() {
            event = Some(Event::Close);
            claimed = true;
        }
        if save.show(ui, id.with("save"), save_rect).clicked() {
            event = Some(Event::Save);
            claimed = true;
        }
        if clicked && !claimed && self.focus == focus_before {
            self.set_focus(None, now);
        }
        event
    }

    /// Draws the message's editor in `area`, scrolling past its 164px (the Squire 1px up).
    /// Returns a file pasted in it, and whether it took the keyboard.
    fn message(
        &mut self,
        ui: &mut Ui,
        area: Rect,
        carbon: bool,
        disabled: bool,
        now: &Zoned,
    ) -> (Option<Event>, bool) {
        // `scrollbar-gutter: stable`: the text stops short of the scrollbar's track.
        let gutter = if carbon { 12.0 } else { 11.0 };
        let text = Rect::from_min_size(
            area.min - vec2(0.0, 1.0),
            vec2(area.width() - gutter, area.height()),
        );
        let pasted_file = !disabled
            && self.focus == Some(Control::Message)
            && ui.input(|input| {
                input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Paste(_)))
            })
            && !crate::clipboard::read_files(ui.ctx()).is_empty();
        if pasted_file {
            ui.input_mut(|input| {
                input
                    .events
                    .retain(|event| !matches!(event, egui::Event::Paste(_)))
            });
        }
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(text)
                .id_salt("out-of-office-message"),
        );
        child.set_clip_rect(area.intersect(ui.clip_rect()));
        // The message at .4 while off.
        if disabled {
            child.set_opacity(0.4);
        }
        let output = egui::ScrollArea::vertical()
            .id_salt("out-of-office-message-scroll")
            .max_height(area.height())
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(&mut child, |ui| {
                let style = EditorStyle {
                    placeholder: Some("Enter message".into()),
                    min_height: area.height(),
                };
                let edited = self.editor.ui(ui, &style);
                self.message_menu = edited.context_menu.or(self.message_menu.take());
                self.message_learned = edited.learned.or(self.message_learned.take());
                if edited.date != self.date {
                    self.day = edited
                        .date
                        .as_ref()
                        .map(|date| date.text.clone())
                        .or(self.day.take());
                    self.date = edited.date;
                }
                (
                    edited.response.id,
                    edited.response.clicked() || edited.response.gained_focus(),
                    edited.changed,
                )
            });
        let (editor_id, focused, changed) = output.inner;
        self.scroll = output.state.offset.y;
        if changed {
            self.form.body = self.editor.html();
        }
        if disabled {
            // Clicks on a disabled message are swallowed.
            ui.interact(
                area,
                exo_ui::id(ui.ctx(), "out-of-office-message-off"),
                Sense::click_and_drag(),
            );
            ui.memory_mut(|memory| memory.surrender_focus(editor_id));
        } else {
            if focused {
                self.set_focus(Some(Control::Message), now);
            }
            hold_focus(ui, self.focus == Some(Control::Message), editor_id);
        }
        // The thumb: 7px of `rgba(0, 0, 0, .5)` in its 11px track, while the message overflows.
        let content = output.content_size.y;
        if content > area.height() {
            let track =
                Rect::from_min_max(pos2(area.right() - gutter, area.top()), area.max).shrink(2.0);
            let length = track.height() * area.height() / content;
            let offset = (track.height() - length) * self.scroll / (content - area.height());
            let thumb = Rect::from_min_size(
                pos2(track.left(), track.top() + offset),
                vec2(track.width(), length),
            );
            ui.painter()
                .rect_filled(thumb, CornerRadius::same(4), Color32::from_black_alpha(128));
        }
        (pasted_file.then_some(Event::PastedFile), focused)
    }

    /// Applies what a date input reported.
    fn date_event(&mut self, control: Control, event: Option<DateEvent>, now: &Zoned) {
        let Some(event) = event else { return };
        let today = now.date();
        match event {
            DateEvent::Focused => {
                self.set_focus(Some(control), now);
                let input = if control == Control::Start {
                    &mut self.start
                } else {
                    &mut self.end
                };
                input
                    .month
                    .get_or_insert(input.parsed.unwrap_or(today).first_of_month());
            }
            DateEvent::MonthChanged => self.set_focus(Some(control), now),
            DateEvent::Picked(date) => match control {
                Control::Start => self.pick_start(date, now),
                _ => {
                    self.pick_end(Some(date), now);
                    self.end.month = None;
                }
            },
            DateEvent::Cleared => {
                let tz = now.time_zone().clone();
                let start = self.form.start.to_zoned(tz.clone()).date();
                if control == Control::Start {
                    self.start.typed(start, today, Some(today), false);
                } else {
                    let reference = self.form.end.map_or(today, |end| end.to_zoned(tz).date());
                    if self.end.typed(reference, today, Some(start), true) {
                        self.form.end = None;
                    }
                }
                let input = if control == Control::Start {
                    &mut self.start
                } else {
                    &mut self.end
                };
                input
                    .month
                    .get_or_insert(input.parsed.unwrap_or(today).first_of_month());
            }
        }
    }
}

/// Gives egui's keyboard focus to `id` while it's `wanted`, and takes it away when not.
pub(super) fn hold_focus(ui: &Ui, wanted: bool, id: Id) {
    let has = ui.memory(|memory| memory.has_focus(id));
    if wanted && !has {
        ui.memory_mut(|memory| memory.request_focus(id));
    } else if !wanted && has {
        ui.memory_mut(|memory| memory.surrender_focus(id));
    }
}

/// `date`'s local midnight in `now`'s zone.
fn midnight(date: Date, now: &Zoned) -> Timestamp {
    date.to_zoned(now.time_zone().clone())
        .map_or_else(|_| now.timestamp(), |zoned| zoned.timestamp())
}

/// A loading skeleton in `rect`, its shine where the 2s animation has it.
fn skeleton(ui: &Ui, rect: Rect) {
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(4), SKELETON);
    // `background-position` from `left -100px` to `right -100px` over 2s, `ease`, repeating.
    let time = ui.input(|input| input.time);
    let progress = exo_ui::motion::CubicBezier::EASE.at((time % 2.0) as f32 / 2.0);
    let band = 100.0;
    let x = rect.left() - band + (rect.width() + band) * progress;
    let stops = [(0.0, 0.0), (0.2, 0.2), (0.6, 0.5), (1.0, 0.0)];
    let mut mesh = Mesh::default();
    for (index, (at, alpha)) in stops.iter().enumerate() {
        let column = x + band * at;
        let color = Color32::from_white_alpha((alpha * 255.0) as u8);
        mesh.colored_vertex(
            Pos2::new(column.clamp(rect.left(), rect.right()), rect.top()),
            color,
        );
        mesh.colored_vertex(
            Pos2::new(column.clamp(rect.left(), rect.right()), rect.bottom()),
            color,
        );
        if index > 0 {
            let base = (index as u32 - 1) * 2;
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base + 1, base + 2, base + 3);
        }
    }
    painter.with_clip_rect(rect).add(egui::Shape::mesh(mesh));
    ui.ctx().request_repaint();
}
