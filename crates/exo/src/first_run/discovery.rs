//! The discovery questions, between the welcome splash and account setup. The answers go to the
//! account's settings and nothing reads them. The last `Next` opens account setup.
//!
//! Classic draws them over `tutorial-splash.jpg`. The keyboard: the first question's own focus ring
//! over its options, the others' Tab through their fields in order, and each dropdown's keys.

use std::path::Path;

use egui::{Color32, CornerRadius, Id, Key, Painter, Pos2, Rect, Sense, Ui, Vec2, pos2, vec2};
use exo_store::{Answer, DiscoveryQuestions};
use exo_ui::{BoxShadow, Family, Font, TextStyle, gradient, motion::CubicBezier, paint_text};

use super::Press;
use crate::photo::Photo;

/// The progress bar's steps, for an account that can't name a workspace: 1, two for the first
/// question, three for the second, one for the last. The bar shows one more than the fields
/// answered, and 90% on the last question.
const STEPS: f32 = 7.0;
const LAST_PROGRESS: f32 = 0.9;
/// The progress bar's track: 8px, 2px past either side, `rgba(191,200,210,.3)`; the bar's gradient
/// over its own width, 4px corners, `transition: width 100ms ease-out`.
const BAR_HEIGHT: f32 = 8.0;
const BAR_OVERHANG: f32 = 2.0;
const TRACK: Color32 = Color32::from_rgba_unmultiplied_const(191, 200, 210, 77);
const BAR_TIME: f32 = 0.1;
/// A stage fades in over .3s and out over .3s within its 500 ms delay; a follow-up question fades
/// in over .1s.
const FADE: f32 = 0.3;
const STAGE_DELAY: f64 = 0.5;
const FOLLOW_FADE: f32 = 0.1;

/// The questions' container: 405px wide, padded 16px; the screen is the window's height, centred on
/// the page under the tab strip, so it starts half the strip's height down.
const CONTAINER: f32 = 405.0;
const COLUMN: f32 = 373.0;
/// The title 68px down, 40px tall; the content `margin-top: 20px` and `top: 3px`; a question's
/// heading 32px with `top: 3px`.
const TITLE_TOP: f32 = 68.0;
const CONTENT_TOP: f32 = 63.0;
const QUESTION: f32 = 32.0;
/// The radios 12px down and 12px apart, in 20px rows; a follow-up 44px down (40px on the second
/// question); the instruction 4px down; the tiles 28px down and 12px apart; the `Next` 52px below
/// the last of them.
const RADIOS_TOP: f32 = 12.0;
const RADIO_ROW: f32 = 20.0;
const RADIO_GAP: f32 = 12.0;
const FOLLOW_UP: f32 = 44.0;
const WORK_FOLLOW_UP: f32 = 40.0;
const INSTRUCTION_TOP: f32 = 4.0;
const TILES_TOP: f32 = 28.0;
const TILE_GAP: f32 = 12.0;
const NEXT_TOP: f32 = 52.0;
/// The first question's 72px under its `Next` (the second's 148px): how far the screen scrolls past
/// the `Next`.
const ABOUT_BOTTOM: f32 = 72.0;
const WORK_BOTTOM: f32 = 148.0;

/// The title: 30px/40px semibold, a `#5cc3fa` to `#a87de8` gradient across the column.
const TITLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 30.0, 40.0);
/// The subtitle: 21px/32px, `.2px` apart, `top: 3px`, black at `.9`.
const SUBTITLE: TextStyle =
    TextStyle::new(Font::new(Family::Sans, 400), 21.0, 32.0).letter_spacing(0.2);
const SUBTITLE_NUDGE: f32 = 3.0;
/// The content's 14px/20px at `.6`, `.2px` apart: the instruction and the radios' labels (`top:
/// 3px`).
const BODY: TextStyle =
    TextStyle::new(Font::new(Family::Sans, 400), 14.0, 20.0).letter_spacing(0.2);
const LABEL_NUDGE: f32 = 3.0;
/// A tile's label: 14px/20px, no spacing, black at `.75`, `top: 3px`.
const TILE_LABEL: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 14.0, 20.0);
/// The `Next` button: the column wide, 30px, 2px corners, its text 14px/18px semibold capitals
/// `.2px` apart, 8px down.
const NEXT: TextStyle =
    TextStyle::new(Font::new(Family::Sans, 600), 14.0, 18.0).letter_spacing(0.2);
const NEXT_HEIGHT: f32 = 30.0;
const NEXT_TEXT_TOP: f32 = 8.0;
/// The dropdowns' and fields' 16px/20px.
const FIELD: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 20.0);
/// A dropdown's option here: 16px/20px at `.5`, 14px in, `padding-top: 11px` and `top: 2px`, in
/// 40px rows; the list 436px wide, its scroller 224px tall (200px for the role and the company's
/// size), 8px under its button and 14px left of it (18px on the last question).
const ROW: f32 = 40.0;
const ROW_TEXT: Vec2 = vec2(14.0, 13.0);
const LIST_WIDTH: f32 = 436.0;
const LIST_HEIGHT: f32 = 224.0;
const SHORTER_LIST: f32 = 200.0;
const LIST_GAP: f32 = 8.0;

/// The last question: 190px tall, centred on the page; a 370px column, 70px, then 413px.
const TWO_COLUMN: f32 = 190.0;
const LEFT: f32 = 370.0;
const GAP: f32 = 70.0;
const RIGHT: f32 = 413.0;
/// The last question's title: 42px/48px semibold, `top: 2px`, a `#e39cb0`, `#abcef2`, `#b690f4`
/// gradient across the column; its text 14px/24px at `.55`, 15px under it.
const LEARN: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 42.0, 48.0);
const LEARN_TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 14.0, 24.0);
const LEARN_TEXT_TOP: f32 = 111.0;
/// The question: 21px/32px at `.95`, `top: 3px`; its dropdown 57px down, 30px tall with a 1px rule;
/// `Please specify` 104px down, 38px; the button 50px below.
const HEARD: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 21.0, 32.0);
const HEARD_DROPDOWN: f32 = 57.0;
const HEARD_DROPDOWN_HEIGHT: f32 = 30.0;
const HEARD_INPUT: f32 = 104.0;
const HEARD_INPUT_HEIGHT: f32 = 38.0;
const CTA_TOP: f32 = 50.0;
/// The button: 32px, `padding: 8px 12px 6px`, 14px/18px semibold capitals, 2px corners.
const CTA: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 14.0, 18.0);
const CTA_HEIGHT: f32 = 32.0;
const CTA_PADDING: Vec2 = vec2(12.0, 8.0);

/// `#aeb1dd`: the enabled buttons, a checked tile's border, a focused field's rule.
const LAVENDER: Color32 = Color32::from_rgb(0xae, 0xb1, 0xdd);
/// `0 2px 8px 0 #aeb1dd` on a hovered or focused button.
const GLOW: BoxShadow = BoxShadow::new([0.0, 2.0], 8.0, 0.0, LAVENDER);

const STAGE_TITLES: [&str; 2] = ["Let's talk about email", "Tell us a bit about yourself"];
const DISLIKE: &str = "What do you dislike most about email?";
const MANAGE: &str = "How do you manage your inbox?";
const SELECT_ALL: &str = "(Select all that apply)";
const WORK: &str = "What type of work do you do?";
const ROLE: &str = "What's your role?";
const SIZE: &str = "How big is your company?";
const LEARN_LINES: [&str; 2] = ["One last question", "before we begin"];
const IMPROVE: &str = "Your response helps us improve.";
/// The last question.
const HEARD_QUESTION: &str = "How did you hear about Exo?";
const CHOOSE: &str = "Choose an option";
const DESCRIBE: &str = "Please describe";
const SPECIFY: &str = "Please specify";
const WHICH_ONE: &str = "Which one?";
const NEXT_LABEL: &str = "Next";
/// The fields take 40 characters.
const MAX_INPUT: usize = 40;

/// `eyn`: the first question's options, by id.
const PAIN_POINTS: [(&str, &str); 4] = [
    ("time", "I spend too much time on email"),
    (
        "organization",
        "My inbox is disorganized and I miss important emails",
    ),
    ("followup", "I forget to respond or follow up"),
    ("team", "Getting team input on emails is slow and messy"),
];
/// `eyr`: the tiles, with the legacy icon each shows; `None of the above` has none and takes the
/// others' place.
const WORKFLOWS: [(&str, &str, Option<&str>); 8] = [
    ("labels", "Labels", Some("legacy/DEPRECATED_grayLabel")),
    ("folders", "Folders", Some("legacy/DEPRECATED_grayFolder")),
    ("stars", "Stars", Some("legacy/DEPRECATED_grayStar")),
    ("flags", "Flags", Some("legacy/DEPRECATED_grayFlag")),
    (
        "mark_unread",
        "Mark Unread",
        Some("legacy/DEPRECATED_grayMarkUnread"),
    ),
    ("trash", "Trash", Some("legacy/DEPRECATED_grayTrash")),
    ("archive", "Archive", Some("legacy/DEPRECATED_grayArchive")),
    ("none", "None of the above", None),
];
const NONE_OF_THE_ABOVE: usize = 7;
/// `eyt`, `eya`, `eyi`, `eyo`.
const WORK_TYPES: [(&str, &str); 13] = [
    ("founder", "Founder/CEO"),
    ("sales", "Sales"),
    ("design", "Design"),
    ("product", "Product"),
    ("engineering", "Engineering"),
    ("marketing", "Marketing"),
    ("operations", "Operations"),
    ("finance", "Finance"),
    ("student", "Student"),
    ("human_resources", "Human Resources"),
    ("educator", "Educator"),
    ("customer_service", "Customer Service"),
    ("other", "Other"),
];
const ROLES: [(&str, &str); 6] = [
    ("executive", "Executive"),
    ("director", "Director"),
    ("team_manager", "Team Manager"),
    ("individual_contributor", "Individual Contributor"),
    ("freelancer_contractor", "Freelancer/Contractor"),
    ("not_applicable", "N/A"),
];
const COMPANY_SIZES: [(&str, &str); 5] = [
    ("1", "1"),
    ("2-50", "2-50"),
    ("51-100", "51-100"),
    ("101-250", "101-250"),
    ("250+", "250+"),
];
const HEARD_FROM: [(&str, &str); 10] = [
    ("friend_or_colleague", "Friend or colleague"),
    ("someones_email_signature", "Someone's email signature"),
    ("search_engine", "Search engine"),
    ("social_media", "Social media"),
    ("news_article_or_blog", "News article or blog"),
    ("event_or_conference", "Event or conference"),
    ("podcast", "Podcast"),
    (
        "influencer_or_content_creator",
        "Influencer or content creator",
    ),
    ("ai_search", "AI search"),
    ("other", "Other"),
];
/// The work type that asks `Please describe`.
const OTHER_WORK: usize = 12;

/// Which question is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// `Let's talk about email`: what you dislike about it and how you manage your inbox.
    AboutYourself,
    /// `Tell us a bit about yourself`: your work, your role and your company's size.
    YourWork,
    /// How you heard about Exo.
    HowDidYouHear,
}

/// Where the first question's keys are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Area {
    Radio,
    Checkbox,
    Next,
}

/// A dropdown or a field of the last two questions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Work,
    WorkOther,
    Role,
    Size,
    Heard,
    HeardOther,
}

impl Field {
    /// The dropdown's options.
    fn options(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Work => &WORK_TYPES,
            Self::Role => &ROLES,
            Self::Size => &COMPANY_SIZES,
            Self::Heard => &HEARD_FROM,
            Self::WorkOther | Self::HeardOther => &[],
        }
    }

    fn is_input(self) -> bool {
        matches!(self, Self::WorkOther | Self::HeardOther)
    }
}

/// What has the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    /// A radio's input, which Enter and Space choose; nothing shows it.
    Radio(usize),
    Field(Field),
    Next,
}

/// An open dropdown's list.
#[derive(Clone, Copy, Debug)]
struct Open {
    field: Field,
    highlighted: Option<usize>,
    scroll: f32,
}

/// A question answered with `Next`: the answers so far, to keep, and whether it was the last
/// (account setup is next).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Answered {
    pub answers: DiscoveryQuestions,
    pub last: bool,
}

/// The questions over the window.
pub(crate) struct Discovery {
    id: Id,
    stage: Stage,
    /// When the stage came up, and when its `Next` started it fading out.
    since: f64,
    leaving: Option<f64>,
    /// The first question: the dislike, the tiles in the order checked (a `Set`'s), its keys'
    /// area and option, and when the tiles came in.
    pain: Option<usize>,
    workflow: Vec<usize>,
    area: Area,
    index: Option<usize>,
    tiles_since: f64,
    /// The second: the work, what `Please describe` says, the role, the company's size, and when
    /// each follow-up came in.
    work: Option<usize>,
    work_other: String,
    role: Option<usize>,
    size: Option<usize>,
    role_since: f64,
    size_since: f64,
    /// The last.
    heard: Option<usize>,
    heard_other: String,
    focus: Option<Focus>,
    open: Option<Open>,
    /// The fields answered (`r22`): the bar shows one more step than these.
    answered: Vec<&'static str>,
    /// The bar's width animating from, to, since.
    bar: (f32, f32, f64),
    scroll: f32,
    keys: Vec<Press>,
    answers: DiscoveryQuestions,
    background: Photo,
}

impl std::fmt::Debug for Discovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Discovery")
            .field("stage", &self.stage)
            .field("answers", &self.answers)
            .finish_non_exhaustive()
    }
}

impl Discovery {
    /// The first question, coming up now, over `images`' `tutorial-splash.jpg`.
    pub(crate) fn new(ctx: &egui::Context, images: &Path) -> Self {
        let now = ctx.input(|input| input.time);
        let first = 1.0 / STEPS;
        Self {
            id: exo_ui::id(ctx, ("discovery", now.to_bits())),
            stage: Stage::AboutYourself,
            since: now,
            leaving: None,
            pain: None,
            workflow: Vec::new(),
            area: Area::Radio,
            index: None,
            tiles_since: now,
            work: None,
            work_other: String::new(),
            role: None,
            size: None,
            role_since: now,
            size_since: now,
            heard: None,
            heard_other: String::new(),
            // The region's first stop: the first radio.
            focus: Some(Focus::Radio(0)),
            open: None,
            answered: Vec::new(),
            bar: (first, first, now),
            scroll: 0.0,
            keys: Vec::new(),
            answers: DiscoveryQuestions::default(),
            background: Photo::load(ctx, images.join("tutorial-splash.jpg")),
        }
    }

    /// Its photo is decoded. For screenshots and tests.
    pub(crate) fn is_idle(&self) -> bool {
        self.background.is_loaded()
    }

    /// Takes this frame's keys before anything else sees them; a field's typing keys stay its.
    pub(crate) fn take_keys(&mut self, ctx: &egui::Context) {
        let typing = matches!(self.focus, Some(Focus::Field(field)) if field.is_input());
        self.keys.extend(super::take_keys(ctx, typing));
    }

    /// The share of the bar the stage shows.
    fn progress(&self) -> f32 {
        match self.stage {
            Stage::HowDidYouHear => LAST_PROGRESS,
            _ => (self.answered.len() as f32 + 1.0) / STEPS,
        }
    }

    /// `r92`: a field answered for the first time moves the bar.
    fn answer(&mut self, key: &'static str, now: f64) {
        if self.answered.contains(&key) {
            return;
        }
        let shown = self.bar_width(now);
        self.answered.push(key);
        self.bar = (shown, self.progress(), now);
    }

    fn bar_width(&self, now: f64) -> f32 {
        let (from, to, since) = self.bar;
        from + (to - from)
            * CubicBezier::EASE_OUT.at(((now - since) as f32 / BAR_TIME).clamp(0.0, 1.0))
    }

    /// Whether the stage's `Next` is enabled.
    fn complete(&self) -> bool {
        match self.stage {
            Stage::AboutYourself => self.pain.is_some() && !self.workflow.is_empty(),
            Stage::YourWork => {
                self.work
                    .is_some_and(|work| work != OTHER_WORK || !self.work_other.is_empty())
                    && self.role.is_some()
                    && self.size.is_some()
            }
            Stage::HowDidYouHear => self.heard.is_some(),
        }
    }

    /// Draws the questions over `window`, under a tab strip `strip_height` tall.
    pub(crate) fn show(
        &mut self,
        ui: &mut Ui,
        window: Rect,
        strip_height: f32,
    ) -> Option<Answered> {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|input| input.time);
        let page = Rect::from_min_max(pos2(window.left(), window.top() + strip_height), window.max);
        let painter = ui.painter().with_clip_rect(window);
        // White under the photo, covering, centred.
        painter.rect_filled(page, 0.0, Color32::WHITE);
        self.background
            .paint(&painter.with_clip_rect(page), page, |free| free / 2.0);
        self.paint_bar(&painter, page, now);

        if let Some(left) = self.leaving {
            if now - left >= STAGE_DELAY {
                self.next_stage(now);
            } else {
                // Fading out, the stage goes, its keys and clicks with it.
                ctx.request_repaint();
                self.keys.clear();
                let mut faded = painter.clone();
                faded
                    .set_opacity(1.0 - CubicBezier::EASE.at(((now - left) as f32 / FADE).min(1.0)));
                self.paint_stage(ui, &faded, window, page, now, false);
                return None;
            }
        }
        let shown = CubicBezier::EASE.at(((now - self.since) as f32 / FADE).clamp(0.0, 1.0));
        if shown < 1.0 || now - self.bar.2 < f64::from(BAR_TIME) {
            ctx.request_repaint();
        }
        // Its clicks are its own; one on nothing in particular closes a list (its button's blur).
        if ui
            .interact(window, self.id.with("cover"), Sense::CLICK)
            .clicked()
        {
            self.open = None;
            self.focus = match self.stage {
                Stage::AboutYourself | Stage::YourWork => None,
                // The whole stage is its region, which focuses its first field.
                Stage::HowDidYouHear => Some(Focus::Field(Field::Heard)),
            };
        }
        let mut faded = painter.clone();
        faded.set_opacity(shown);
        let event = self.paint_stage(ui, &faded, window, page, now, true);
        let keys = std::mem::take(&mut self.keys);
        let event = keys.into_iter().fold(event, |event, press| {
            event.or_else(|| self.press(press, now))
        });
        if event.as_ref().is_some_and(|answered| !answered.last) {
            self.leaving = Some(now);
            self.open = None;
        }
        self.sync_text_focus(&ctx);
        event
    }

    /// The next question, fading in.
    fn next_stage(&mut self, now: f64) {
        let (stage, focus) = match self.stage {
            Stage::AboutYourself => (Stage::YourWork, Field::Work),
            Stage::YourWork | Stage::HowDidYouHear => (Stage::HowDidYouHear, Field::Heard),
        };
        let shown = self.bar_width(now);
        self.stage = stage;
        self.since = now;
        self.leaving = None;
        self.scroll = 0.0;
        // The region's first field.
        self.focus = Some(Focus::Field(focus));
        self.bar = (shown, self.progress(), now);
    }

    /// The text field with the keyboard has egui's, and no other does.
    fn sync_text_focus(&self, ctx: &egui::Context) {
        let wanted = match self.focus {
            Some(Focus::Field(field)) if field.is_input() => Some(self.id.with(field_name(field))),
            _ => None,
        };
        ctx.memory_mut(|memory| {
            for field in [Field::WorkOther, Field::HeardOther] {
                let id = self.id.with(field_name(field));
                if Some(id) == wanted {
                    if !memory.has_focus(id) {
                        memory.request_focus(id);
                    }
                    // Tab and the arrows are the questions', not egui's focus moves.
                    memory.set_focus_lock_filter(
                        id,
                        egui::EventFilter {
                            tab: true,
                            horizontal_arrows: true,
                            vertical_arrows: true,
                            escape: true,
                        },
                    );
                } else {
                    memory.surrender_focus(id);
                }
            }
        });
    }

    fn paint_bar(&self, painter: &Painter, page: Rect, now: f64) {
        let track = Rect::from_min_size(
            pos2(page.left() - BAR_OVERHANG, page.top()),
            vec2(page.width() + 2.0 * BAR_OVERHANG, BAR_HEIGHT),
        );
        painter.rect_filled(track, 0.0, TRACK);
        let width = track.width() * self.bar_width(now);
        if width <= 0.0 {
            return;
        }
        let bar = Rect::from_min_size(track.min, vec2(width, BAR_HEIGHT));
        let rgb = |r: u8, g: u8, b: u8| {
            [
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
                1.0,
            ]
        };
        let stops = [
            (0.0, rgb(0x8f, 0xb4, 0xef)),
            (0.5, rgb(0x93, 0x92, 0xef)),
            (1.0, rgb(0xc7, 0x95, 0xcf)),
        ];
        let color_at = |x: f32| gradient::opaque(gradient::ramp(&stops, (x - bar.left()) / width));
        // The rounded ends as discs in their colors, the straight run between them as a field.
        let clipped = painter.with_clip_rect(track.intersect(painter.clip_rect()));
        let radius = BAR_HEIGHT / 2.0;
        for x in [bar.left() + radius, bar.right() - radius] {
            clipped.circle_filled(pos2(x, bar.center().y), radius, color_at(x));
        }
        let run = Rect::from_min_max(
            pos2(bar.left() + radius, bar.top()),
            pos2(
                (bar.right() - radius).max(bar.left() + radius),
                bar.bottom(),
            ),
        );
        gradient::paint_field(&clipped, run, 32, 1, |point| color_at(point.x));
    }

    /// Paints the stage (and answers its pointer when `live`).
    fn paint_stage(
        &mut self,
        ui: &mut Ui,
        painter: &Painter,
        window: Rect,
        page: Rect,
        now: f64,
        live: bool,
    ) -> Option<Answered> {
        match self.stage {
            Stage::AboutYourself => self.about_yourself(ui, painter, window, page, now, live),
            Stage::YourWork => self.your_work(ui, painter, window, page, now, live),
            Stage::HowDidYouHear => self.how_did_you_hear(ui, painter, page, now, live),
        }
    }

    /// The column's left and the title's top, scrolled; and how far down the paint is of the
    /// layout.
    fn column(&self, window: Rect, page: Rect) -> (f32, f32, f32) {
        let left = page.center().x - CONTAINER / 2.0 + (CONTAINER - COLUMN) / 2.0;
        // The screen is the window's height, centred on the page: half the strip above the window,
        // which Chromium paints from a whole pixel (the scroller's).
        let half = (page.top() - window.top()) / 2.0;
        (
            left,
            window.top() + half.round() + TITLE_TOP - self.scroll,
            half.round() - half,
        )
    }

    /// The wheel scrolls the screen when it's taller than the window (`overflow-y: auto`).
    fn scroll_by_wheel(&mut self, ui: &Ui, window: Rect, bottom: f32, live: bool) {
        if live && ui.rect_contains_pointer(window) {
            self.scroll -= ui.input(|input| input.smooth_scroll_delta.y);
        }
        let overflow = (bottom + self.scroll - window.bottom()).max(0.0);
        self.scroll = self.scroll.clamp(0.0, overflow);
    }

    fn paint_title(painter: &Painter, left: f32, top: f32, text: &str) {
        let rgb = |r: u8, g: u8, b: u8| {
            [
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
                1.0,
            ]
        };
        let stops = [(0.0, rgb(0x5c, 0xc3, 0xfa)), (1.0, rgb(0xa8, 0x7d, 0xe8))];
        let galley = TITLE.single_line(painter, text, Color32::WHITE, None);
        paint_text(
            painter,
            pos2(left, top),
            gradient::tint_galley(&galley, COLUMN, &stops),
            &TITLE,
        );
    }

    fn about_yourself(
        &mut self,
        ui: &mut Ui,
        painter: &Painter,
        window: Rect,
        page: Rect,
        now: f64,
        live: bool,
    ) -> Option<Answered> {
        let ctx = ui.ctx().clone();
        let (left, title_top, _) = self.column(window, page);
        Self::paint_title(painter, left, title_top, STAGE_TITLES[0]);
        let content = title_top + CONTENT_TOP;
        SUBTITLE.paint_line(
            painter,
            pos2(left, content + SUBTITLE_NUDGE),
            DISLIKE,
            Color32::from_black_alpha(230),
            None,
        );
        let radios = content + QUESTION + RADIOS_TOP;
        for (index, (_, label)) in PAIN_POINTS.iter().enumerate() {
            let row = radios + index as f32 * (RADIO_ROW + RADIO_GAP);
            let focused = self.area == Area::Radio && self.index == Some(index);
            paint_radio(
                painter,
                pos2(left + 5.0, row + 12.0),
                self.pain == Some(index),
                focused,
            );
            BODY.paint_line(
                painter,
                pos2(left + 20.0, row + LABEL_NUDGE),
                label,
                Color32::from_black_alpha(153),
                None,
            );
            // A radio's row: 6px left of the column, 369px wide.
            let hit = Rect::from_min_size(pos2(left - 6.0, row), vec2(COLUMN - 4.0, RADIO_ROW));
            if live
                && ui
                    .interact(hit, self.id.with(("radio", index)), Sense::CLICK)
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
            {
                self.focus = Some(Focus::Radio(index));
                self.choose_pain(index, now);
            }
        }
        let mut bottom = radios + 4.0 * RADIO_ROW + 3.0 * RADIO_GAP;
        if self.pain.is_some() {
            let block = bottom + FOLLOW_UP;
            let mut follow = painter.clone();
            follow.set_opacity(
                painter.opacity()
                    * CubicBezier::EASE
                        .at(((now - self.tiles_since) as f32 / FOLLOW_FADE).clamp(0.0, 1.0)),
            );
            SUBTITLE.paint_line(
                &follow,
                pos2(left, block + SUBTITLE_NUDGE),
                MANAGE,
                Color32::from_black_alpha(230),
                None,
            );
            let instruction = block + QUESTION + INSTRUCTION_TOP;
            BODY.paint_line(
                &follow,
                pos2(left, instruction),
                SELECT_ALL,
                Color32::from_black_alpha(153),
                None,
            );
            let tiles = tile_rects(&ctx, left, instruction + BODY.line_height + TILES_TOP);
            for (index, rect) in tiles.iter().enumerate() {
                let response = live.then(|| {
                    ui.interact(*rect, self.id.with(("tile", index)), Sense::CLICK)
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                });
                let checked = self.workflow.contains(&index);
                let lit = response.as_ref().is_some_and(egui::Response::hovered)
                    || (self.area == Area::Checkbox && self.index == Some(index));
                paint_tile(ui, &follow, *rect, index, checked, lit);
                if response.is_some_and(|response| response.clicked()) {
                    self.index = None;
                    self.toggle_tile(index, now);
                    // The click lands on the region, which focuses its first stop.
                    self.focus = Some(Focus::Radio(0));
                }
            }
            bottom = tiles.last().map_or(bottom, |rect| rect.bottom());
        }
        let next = Rect::from_min_size(pos2(left, bottom + NEXT_TOP), vec2(COLUMN, NEXT_HEIGHT));
        let event = self.next_button(ui, painter, next, live);
        self.scroll_by_wheel(ui, window, next.bottom() + ABOUT_BOTTOM, live);
        event
    }

    fn your_work(
        &mut self,
        ui: &mut Ui,
        painter: &Painter,
        window: Rect,
        page: Rect,
        now: f64,
        live: bool,
    ) -> Option<Answered> {
        let (left, title_top, snapped) = self.column(window, page);
        Self::paint_title(painter, left, title_top, STAGE_TITLES[1]);
        let content = title_top + CONTENT_TOP;
        SUBTITLE.paint_line(
            painter,
            pos2(left, content + SUBTITLE_NUDGE),
            WORK,
            Color32::from_black_alpha(230),
            None,
        );
        let mut list = None;
        let work = Rect::from_min_size(
            pos2(left - 4.0, content + QUESTION + 16.0),
            vec2(COLUMN, DROPDOWN_HEIGHT),
        );
        list = list.or(self.dropdown(
            ui,
            painter,
            work,
            Field::Work,
            self.work,
            Dropdown::Underlined,
            live,
        ));
        let mut bottom = work.bottom();
        if self.work == Some(OTHER_WORK) {
            let field =
                Rect::from_min_size(pos2(left - 4.0, bottom + 2.0), vec2(COLUMN, INPUT_HEIGHT));
            self.input(
                ui,
                painter,
                field,
                Field::WorkOther,
                DESCRIBE,
                vec2(8.0, 8.0),
                1.5,
                live,
            );
            bottom += INPUT_HEIGHT;
        }
        for (field, question, chosen, since, shown) in [
            (
                Field::Role,
                ROLE,
                self.role,
                self.role_since,
                self.work.is_some(),
            ),
            (
                Field::Size,
                SIZE,
                self.size,
                self.size_since,
                self.role.is_some(),
            ),
        ] {
            if !shown {
                break;
            }
            let block = bottom + WORK_FOLLOW_UP;
            let mut follow = painter.clone();
            follow.set_opacity(
                painter.opacity()
                    * CubicBezier::EASE.at(((now - since) as f32 / FOLLOW_FADE).clamp(0.0, 1.0)),
            );
            SUBTITLE.paint_line(
                &follow,
                pos2(left, block + SUBTITLE_NUDGE),
                question,
                Color32::from_black_alpha(230),
                None,
            );
            let button = Rect::from_min_size(
                pos2(left - 4.0, block + QUESTION + 16.0),
                vec2(COLUMN, DROPDOWN_HEIGHT),
            );
            list = list.or(self.dropdown(
                ui,
                &follow,
                button,
                field,
                chosen,
                Dropdown::Underlined,
                live,
            ));
            bottom = button.bottom();
        }
        let next = Rect::from_min_size(pos2(left, bottom + NEXT_TOP), vec2(COLUMN, NEXT_HEIGHT));
        let event = self.next_button(ui, painter, next, live);
        self.scroll_by_wheel(ui, window, next.bottom() + WORK_BOTTOM, live);
        if let Some(button) = list {
            // The list is tethered to the button as laid out, not as painted.
            self.show_list(
                ui,
                painter,
                button.translate(vec2(-14.0, -snapped)),
                live,
                now,
            );
        }
        event
    }

    fn how_did_you_hear(
        &mut self,
        ui: &mut Ui,
        painter: &Painter,
        page: Rect,
        now: f64,
        live: bool,
    ) -> Option<Answered> {
        let rgb = |r: u8, g: u8, b: u8| {
            [
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
                1.0,
            ]
        };
        // Centred on the page, painted from a whole pixel (its list too).
        let top = (page.top() + (page.height() - TWO_COLUMN) / 2.0).round();
        let left = page.center().x - (LEFT + GAP + RIGHT) / 2.0;
        let stops = [
            (0.0, rgb(0xe3, 0x9c, 0xb0)),
            (0.5, rgb(0xab, 0xce, 0xf2)),
            (1.0, rgb(0xb6, 0x90, 0xf4)),
        ];
        for (index, line) in LEARN_LINES.iter().enumerate() {
            let galley = LEARN.single_line(painter, line, Color32::WHITE, None);
            // The gradient spans the block; each line's glyphs take its color where they sit.
            paint_text(
                painter,
                pos2(left, top + 2.0 + index as f32 * LEARN.line_height),
                gradient::tint_galley(&galley, LEFT, &stops),
                &LEARN,
            );
        }
        LEARN_TEXT.paint_line(
            painter,
            pos2(left, top + LEARN_TEXT_TOP),
            IMPROVE,
            Color32::from_black_alpha(140),
            None,
        );
        let right = left + LEFT + GAP;
        HEARD.paint_line(
            painter,
            pos2(right, top + 3.0),
            HEARD_QUESTION,
            Color32::from_black_alpha(242),
            None,
        );
        let button = Rect::from_min_size(
            pos2(right, top + HEARD_DROPDOWN),
            vec2(RIGHT, HEARD_DROPDOWN_HEIGHT),
        );
        let list = self.dropdown(
            ui,
            painter,
            button,
            Field::Heard,
            self.heard,
            Dropdown::Ruled,
            live,
        );
        let mut cta_top = button.bottom() + CTA_TOP;
        if let Some(placeholder) = self.heard.and_then(heard_placeholder) {
            let field = Rect::from_min_size(
                pos2(right, top + HEARD_INPUT),
                vec2(RIGHT, HEARD_INPUT_HEIGHT),
            );
            self.input(
                ui,
                painter,
                field,
                Field::HeardOther,
                placeholder,
                vec2(0.0, 8.0),
                1.0,
                live,
            );
            cta_top = field.bottom() + CTA_TOP;
        }
        let label = NEXT_LABEL.to_uppercase();
        let width = CTA.width(&ui.ctx().clone(), &label) + 2.0 * CTA_PADDING.x;
        let cta = Rect::from_min_size(
            pos2(right + RIGHT - width, cta_top),
            vec2(width, CTA_HEIGHT),
        );
        let enabled = self.complete();
        let response = live.then(|| ui.interact(cta, self.id.with("cta"), Sense::CLICK));
        let hovered = response.as_ref().is_some_and(egui::Response::hovered);
        if enabled && (hovered || self.focus == Some(Focus::Next)) {
            GLOW.paint(painter, cta, CornerRadius::same(2));
        }
        let (fill, ink) = if enabled {
            (LAVENDER, Color32::WHITE)
        } else {
            (Color32::from_black_alpha(10), Color32::from_black_alpha(33))
        };
        painter.rect_filled(cta, CornerRadius::same(2), fill);
        CTA.paint_line(painter, cta.min + CTA_PADDING, &label, ink, None);
        let mut event = None;
        if enabled && response.is_some_and(|response| response.clicked()) {
            self.focus = Some(Focus::Next);
            event = self.submit();
        }
        if let Some(button) = list {
            self.show_list(ui, painter, button.translate(vec2(-18.0, 0.0)), live, now);
        }
        event
    }

    /// The first two questions' `Next`, enabled once they're answered.
    fn next_button(
        &mut self,
        ui: &Ui,
        painter: &Painter,
        rect: Rect,
        live: bool,
    ) -> Option<Answered> {
        let enabled = self.complete();
        let response = live.then(|| ui.interact(rect, self.id.with("next"), Sense::CLICK));
        let hovered = response.as_ref().is_some_and(egui::Response::hovered);
        if enabled && (hovered || self.focus == Some(Focus::Next)) {
            GLOW.paint(painter, rect, CornerRadius::same(2));
        }
        let (fill, ink) = if enabled {
            (LAVENDER, Color32::WHITE)
        } else {
            (Color32::from_black_alpha(9), Color32::from_black_alpha(33))
        };
        painter.rect_filled(rect, CornerRadius::same(2), fill);
        let label = NEXT_LABEL.to_uppercase();
        let width = NEXT.width(&ui.ctx().clone(), &label);
        NEXT.paint_line(
            painter,
            pos2(rect.center().x - width / 2.0, rect.top() + NEXT_TEXT_TOP),
            &label,
            ink,
            None,
        );
        if enabled && response.is_some_and(|response| response.clicked()) {
            self.focus = Some(Focus::Next);
            return self.submit();
        }
        None
    }

    /// A dropdown's button; returns it when its list is open, to draw over the rest.
    #[expect(
        clippy::too_many_arguments,
        reason = "the button's place, field, value and style"
    )]
    fn dropdown(
        &mut self,
        ui: &Ui,
        painter: &Painter,
        rect: Rect,
        field: Field,
        chosen: Option<usize>,
        style: Dropdown,
        live: bool,
    ) -> Option<Rect> {
        let focused = self.focus == Some(Focus::Field(field));
        // The caret's ink: black, at `.35` in the content, in a box at `.5`.
        let (inset, caret_left, caret_ink, rule) = match style {
            Dropdown::Underlined => (8.0, 32.0, 45, 1.5),
            Dropdown::Ruled => (0.0, 29.0, 128, 1.0),
        };
        let (text, ink) = chosen.map_or((CHOOSE, Color32::from_black_alpha(89)), |index| {
            (field.options()[index].1, Color32::from_black_alpha(230))
        });
        FIELD.paint_line(
            painter,
            pos2(rect.left() + inset, rect.top()),
            text,
            ink,
            None,
        );
        // The dropdown's large caret, painted from a whole pixel.
        let caret = Rect::from_min_size(
            pos2(rect.right() - caret_left, rect.top() + 7.0).round(),
            vec2(12.0, 7.0),
        );
        exo_ui::paint_icon(
            ui,
            "legacy/DEPRECATED_downCaretLarge",
            caret,
            Color32::from_black_alpha(caret_ink).gamma_multiply(painter.opacity()),
        );
        let rule_ink = if focused {
            LAVENDER
        } else {
            Color32::from_black_alpha(26)
        };
        painter.rect_filled(
            Rect::from_min_max(pos2(rect.left(), rect.bottom() - rule), rect.max),
            0.0,
            rule_ink,
        );
        if live
            && ui
                .interact(
                    rect,
                    self.id.with(("dropdown", field_name(field))),
                    Sense::CLICK,
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
        {
            self.focus = Some(Focus::Field(field));
            self.toggle_list(field);
        }
        self.open.filter(|open| open.field == field).map(|_| rect)
    }

    /// A one-line field: its line `padding` in from the box's top left, a `rule` under it.
    #[expect(
        clippy::too_many_arguments,
        reason = "the field's place, placeholder and box"
    )]
    fn input(
        &mut self,
        ui: &mut Ui,
        painter: &Painter,
        rect: Rect,
        field: Field,
        placeholder: &str,
        padding: Vec2,
        rule: f32,
        live: bool,
    ) {
        let focused = self.focus == Some(Focus::Field(field));
        let line = Rect::from_min_size(
            rect.min + padding,
            vec2(rect.width() - 2.0 * padding.x, FIELD.line_height),
        );
        let text = match field {
            Field::WorkOther => &mut self.work_other,
            _ => &mut self.heard_other,
        };
        if text.is_empty() {
            FIELD.paint_line(
                painter,
                line.min,
                placeholder,
                Color32::from_black_alpha(89),
                None,
            );
        }
        let id = self.id.with(field_name(field));
        if live {
            let selection = exo_ui::palette(ui.ctx()).text_selection_default;
            let mut field_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            field_ui.set_opacity(painter.opacity());
            let response = exo_ui::text_field(
                &mut field_ui,
                id,
                text,
                line,
                &FIELD,
                Color32::from_black_alpha(230),
                selection,
            );
            // Leading spaces trimmed, 40 characters at most.
            let trimmed: String = text.trim_start().chars().take(MAX_INPUT).collect();
            if trimmed != *text {
                *text = trimmed;
            }
            if response.clicked() || response.gained_focus() {
                self.focus = Some(Focus::Field(field));
                self.open = None;
            }
        } else {
            FIELD.paint_line(
                painter,
                line.min,
                text,
                Color32::from_black_alpha(230),
                None,
            );
        }
        let rule_ink = if focused {
            LAVENDER
        } else {
            Color32::from_black_alpha(26)
        };
        painter.rect_filled(
            Rect::from_min_max(pos2(rect.left(), rect.bottom() - rule), rect.max),
            0.0,
            rule_ink,
        );
    }

    /// The open list under `button`: shifted by its offset, 8px down.
    fn show_list(&mut self, ui: &Ui, painter: &Painter, button: Rect, live: bool, now: f64) {
        let Some(mut open) = self.open else { return };
        let options = open.field.options();
        let height = if matches!(open.field, Field::Role | Field::Size) {
            SHORTER_LIST
        } else {
            LIST_HEIGHT
        };
        let list = Rect::from_min_size(
            pos2(button.left(), button.bottom() + LIST_GAP),
            vec2(LIST_WIDTH, height),
        );
        let full = options.len() as f32 * ROW;
        if live && ui.rect_contains_pointer(list) {
            open.scroll -= ui.input(|input| input.smooth_scroll_delta.y);
        }
        open.scroll = open.scroll.clamp(0.0, (full - height).max(0.0));
        exo_ui::paint_contact_list_box(painter, list, Color32::WHITE);
        let clipped = painter.with_clip_rect(list.intersect(painter.clip_rect()));
        let mut chosen = None;
        // The pointer highlights a row as it moves over it (`mouseover`), not by resting there.
        let moved = ui.input(|input| input.pointer.delta() != Vec2::ZERO);
        for (index, (_, name)) in options.iter().enumerate() {
            let row = Rect::from_min_size(
                pos2(list.left(), list.top() + index as f32 * ROW - open.scroll),
                vec2(LIST_WIDTH, ROW),
            );
            if !row.intersects(list) {
                continue;
            }
            if live {
                let response = ui
                    .interact(
                        row.intersect(list),
                        self.id.with(("row", index)),
                        Sense::CLICK,
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if response.hovered() && moved {
                    open.highlighted = Some(index);
                }
                if response.clicked() {
                    chosen = Some(index);
                }
            }
            // The highlighted option.
            if open.highlighted == Some(index) {
                clipped.rect_filled(row, 0.0, exo_ui::palette(ui.ctx()).fill_hover_default);
            }
            // Options' names aren't kerned.
            FIELD.paint_line_unkerned(
                &clipped,
                row.min + ROW_TEXT,
                name,
                Color32::from_black_alpha(128),
            );
        }
        self.open = Some(open);
        if let Some(index) = chosen {
            self.select(open.field, index, now);
        }
    }

    /// Opens or closes `field`'s list; it opens on the chosen option.
    fn toggle_list(&mut self, field: Field) {
        if self.open.is_some_and(|open| open.field == field) {
            self.open = None;
            return;
        }
        let highlighted = self.chosen(field);
        let height = if matches!(field, Field::Role | Field::Size) {
            SHORTER_LIST
        } else {
            LIST_HEIGHT
        };
        let scroll =
            highlighted.map_or(0.0, |index| ((index as f32 + 1.0) * ROW - height).max(0.0));
        self.open = Some(Open {
            field,
            highlighted,
            scroll,
        });
    }

    fn chosen(&self, field: Field) -> Option<usize> {
        match field {
            Field::Work => self.work,
            Field::Role => self.role,
            Field::Size => self.size,
            Field::Heard => self.heard,
            Field::WorkOther | Field::HeardOther => None,
        }
    }

    /// An option chosen in `field`'s list, which closes; the field it brings up has the keyboard,
    /// as the last question's `Next` does once it's enabled.
    fn select(&mut self, field: Field, index: usize, now: f64) {
        self.open = None;
        match field {
            Field::Work => {
                if self.work.is_none() {
                    self.role_since = now;
                }
                self.work = Some(index);
                self.answer("workType", now);
                if index == OTHER_WORK {
                    self.focus = Some(Focus::Field(Field::WorkOther));
                }
            }
            Field::Role => {
                if self.role.is_none() {
                    self.size_since = now;
                }
                self.role = Some(index);
                self.answer("role", now);
            }
            Field::Size => {
                self.size = Some(index);
                self.answer("companySize", now);
            }
            Field::Heard => {
                self.heard = Some(index);
                self.focus = Some(if heard_placeholder(index).is_some() {
                    Focus::Field(Field::HeardOther)
                } else {
                    Focus::Next
                });
            }
            Field::WorkOther | Field::HeardOther => {}
        }
    }

    /// `r72`: a dislike chosen; the tiles come in and take the keys, nothing on them yet.
    fn choose_pain(&mut self, index: usize, now: f64) {
        if self.pain.is_none() {
            self.tiles_since = now;
        }
        self.pain = Some(index);
        self.answer("dislikeOptions", now);
        self.area = Area::Checkbox;
        self.index = None;
    }

    /// `ae2`: a tile checked or unchecked; `None of the above` clears the rest, and any other
    /// clears it.
    fn toggle_tile(&mut self, index: usize, now: f64) {
        self.answer("workflow", now);
        if index == NONE_OF_THE_ABOVE {
            let only = self.workflow == [NONE_OF_THE_ABOVE];
            self.workflow.clear();
            if !only {
                self.workflow.push(NONE_OF_THE_ABOVE);
            }
            return;
        }
        self.workflow.retain(|&tile| tile != NONE_OF_THE_ABOVE);
        if let Some(at) = self.workflow.iter().position(|&tile| tile == index) {
            self.workflow.remove(at);
        } else {
            self.workflow.push(index);
        }
    }

    /// `ar2`: the stage's answers kept, and on.
    fn submit(&mut self) -> Option<Answered> {
        if !self.complete() || self.leaving.is_some() {
            return None;
        }
        let answer = |id: &str, other: Option<&str>| Answer {
            answer: id.to_owned(),
            other_input: other.map(str::to_owned),
        };
        match self.stage {
            Stage::AboutYourself => {
                self.answers.pain_point = answer(PAIN_POINTS[self.pain?].0, None);
                let ids: Vec<&str> = self
                    .workflow
                    .iter()
                    .map(|&tile| WORKFLOWS[tile].0)
                    .collect();
                self.answers.workflow = answer(&ids.join(","), None);
            }
            Stage::YourWork => {
                self.answers.your_role = answer(WORK_TYPES[self.work?].0, Some(&self.work_other));
                self.answers.title = answer(ROLES[self.role?].0, None);
                self.answers.company_size = answer(COMPANY_SIZES[self.size?].0, None);
            }
            Stage::HowDidYouHear => {
                self.answers.how_did_you_hear =
                    answer(HEARD_FROM[self.heard?].0, Some(&self.heard_other));
            }
        }
        Some(Answered {
            answers: self.answers.clone(),
            last: self.stage == Stage::HowDidYouHear,
        })
    }

    /// A key taken this frame.
    fn press(&mut self, press: Press, now: f64) -> Option<Answered> {
        match self.stage {
            Stage::AboutYourself => self.about_key(press, now),
            Stage::YourWork | Stage::HowDidYouHear => self.fields_key(press, now),
        }
    }

    /// The first question's keys: Tab and Enter on the radios' inputs as the browser has them, then
    /// its document `keyup` handler (`ac2`).
    fn about_key(&mut self, press: Press, now: f64) -> Option<Answered> {
        let Press {
            key,
            pressed,
            shift,
        } = press;
        if pressed {
            match (key, self.focus) {
                // Tab moves among the radios, the only stops.
                (Key::Tab, focus) => {
                    let from = match focus {
                        Some(Focus::Radio(index)) => index as isize,
                        _ => -1,
                    };
                    let step = if shift { -1 } else { 1 };
                    self.focus = Some(Focus::Radio(
                        (from + step).rem_euclid(PAIN_POINTS.len() as isize) as usize,
                    ));
                }
                (Key::Enter, Some(Focus::Radio(index))) => self.choose_pain(index, now),
                (Key::Enter, Some(Focus::Next)) => return self.submit(),
                _ => {}
            }
            return None;
        }
        let event = match key {
            Key::ArrowDown | Key::ArrowRight => {
                self.step_index(1);
                None
            }
            Key::ArrowUp | Key::ArrowLeft => {
                self.step_index(-1);
                None
            }
            Key::Enter | Key::Space => match (self.area, self.index) {
                (Area::Radio, Some(index)) => {
                    self.choose_pain(index, now);
                    None
                }
                (Area::Checkbox, Some(index)) => {
                    self.toggle_tile(index, now);
                    None
                }
                (Area::Next, _) => self.submit(),
                _ => None,
            },
            Key::Tab => {
                match self.area {
                    Area::Radio if self.pain.is_some() => self.area_to(Area::Checkbox, Some(0)),
                    Area::Checkbox if !self.workflow.is_empty() => {
                        self.area_to(Area::Next, None);
                        self.focus = Some(Focus::Next);
                    }
                    Area::Radio | Area::Checkbox | Area::Next => self.area_to(Area::Radio, Some(0)),
                }
                None
            }
            _ => None,
        };
        // Space lets go of a focused radio's input after the handler: the browser chooses it.
        if key == Key::Space
            && let Some(Focus::Radio(index)) = self.focus
            && event.is_none()
        {
            self.choose_pain(index, now);
        }
        event
    }

    fn area_to(&mut self, area: Area, index: Option<usize>) {
        self.area = area;
        self.index = index;
    }

    /// `aa2`: the arrows move through the area's options, round, from none at -1.
    fn step_index(&mut self, step: isize) {
        let count = match self.area {
            Area::Radio => PAIN_POINTS.len(),
            Area::Checkbox => WORKFLOWS.len(),
            Area::Next => return,
        } as isize;
        let from = self.index.map_or(-1, |index| index as isize);
        self.index = Some((from + step + count).rem_euclid(count) as usize);
    }

    /// The fields' tab order: the dropdowns and fields shown, and an enabled `Next`.
    fn tab_order(&self) -> Vec<Focus> {
        let mut order = Vec::new();
        match self.stage {
            Stage::YourWork => {
                order.push(Focus::Field(Field::Work));
                if self.work == Some(OTHER_WORK) {
                    order.push(Focus::Field(Field::WorkOther));
                }
                if self.work.is_some() {
                    order.push(Focus::Field(Field::Role));
                }
                if self.role.is_some() {
                    order.push(Focus::Field(Field::Size));
                }
            }
            Stage::HowDidYouHear => {
                order.push(Focus::Field(Field::Heard));
                if self.heard.and_then(heard_placeholder).is_some() {
                    order.push(Focus::Field(Field::HeardOther));
                }
            }
            Stage::AboutYourself => {}
        }
        if self.complete() {
            order.push(Focus::Next);
        }
        order
    }

    /// The last two questions' keys: a dropdown's, Tab through the fields, Enter and Space on a
    /// button, and Enter (let go) moving to the next field on the second.
    fn fields_key(&mut self, press: Press, now: f64) -> Option<Answered> {
        let Press {
            key,
            pressed,
            shift,
        } = press;
        let field = match self.focus {
            Some(Focus::Field(field)) => Some(field),
            _ => None,
        };
        let open = self.open.filter(|open| Some(open.field) == field);
        if pressed {
            match key {
                Key::Escape => self.open = None,
                // The highlighted option, and the key goes no further.
                Key::Enter | Key::Tab if open.is_some() => {
                    if let Some(index) = open.and_then(|open| open.highlighted) {
                        self.select(open.map_or(Field::Work, |open| open.field), index, now);
                    }
                }
                Key::Tab => {
                    let order = self.tab_order();
                    let at = self
                        .focus
                        .and_then(|focus| order.iter().position(|&candidate| candidate == focus))
                        .map_or(-1, |at| at as isize);
                    let step = if shift { -1 } else { 1 };
                    if order.len() > 1 || at < 0 {
                        self.focus = order
                            .get((at + step).rem_euclid(order.len().max(1) as isize) as usize)
                            .copied();
                        self.open = None;
                    }
                }
                Key::ArrowDown | Key::ArrowUp if field.is_some_and(|field| !field.is_input()) => {
                    match open {
                        Some(mut open) => {
                            let count = open.field.options().len();
                            let index = match (open.highlighted, key) {
                                (None, Key::ArrowDown) => 0,
                                (None, _) => count - 1,
                                (Some(index), Key::ArrowDown) => (index + 1).min(count - 1),
                                (Some(index), _) => index.saturating_sub(1),
                            };
                            open.highlighted = Some(index);
                            let height = if matches!(open.field, Field::Role | Field::Size) {
                                SHORTER_LIST
                            } else {
                                LIST_HEIGHT
                            };
                            let row = index as f32 * ROW;
                            open.scroll = open.scroll.clamp(row + ROW - height, row);
                            self.open = Some(open);
                        }
                        None => self.toggle_list(field.unwrap_or(Field::Work)),
                    }
                }
                // A button's Enter clicks it as it goes down.
                Key::Enter => match self.focus {
                    Some(Focus::Next) => return self.submit(),
                    Some(Focus::Field(field)) if !field.is_input() => self.toggle_list(field),
                    _ => {}
                },
                _ => {}
            }
            return None;
        }
        match key {
            // And Space as it comes up.
            Key::Space => match self.focus {
                Some(Focus::Next) => return self.submit(),
                Some(Focus::Field(field)) if !field.is_input() => self.toggle_list(field),
                _ => {}
            },
            Key::Enter if self.stage == Stage::YourWork => {
                // The next stop takes the keyboard, closing a list as its button blurs.
                let order = self.tab_order();
                if let Some(next) = self
                    .focus
                    .and_then(|focus| order.iter().position(|&candidate| candidate == focus))
                    .and_then(|at| order.get(at + 1))
                {
                    self.focus = Some(*next);
                    self.open = None;
                }
            }
            _ => {}
        }
        None
    }
}

/// The dropdown's look: the second question's (its text 8px in, the caret 32px from the right, a
/// 1.5px rule) or the last's (flush, 29px, 1px).
#[derive(Clone, Copy, Debug)]
enum Dropdown {
    Underlined,
    Ruled,
}

/// The second question's dropdown, 27.5px with its rule; `Please describe`, 31.5px, 2px down.
const DROPDOWN_HEIGHT: f32 = 27.5;
const INPUT_HEIGHT: f32 = 31.5;

/// The last question's field for `heard`: `Please specify` for Other, `Which one?` for an event,
/// a podcast or a creator.
fn heard_placeholder(heard: usize) -> Option<&'static str> {
    match HEARD_FROM[heard].0 {
        "other" => Some(SPECIFY),
        "influencer_or_content_creator" | "podcast" | "event_or_conference" => Some(WHICH_ONE),
        _ => None,
    }
}

fn field_name(field: Field) -> &'static str {
    match field {
        Field::Work => "work",
        Field::WorkOther => "work-other",
        Field::Role => "role",
        Field::Size => "size",
        Field::Heard => "heard",
        Field::HeardOther => "heard-other",
    }
}

/// A radio: a 12px ring at `.4`, its `#6fbeea` dot when chosen, on a 22px disc at `.05` when it has
/// the keys. Chromium paints it half a pixel down, from a whole row.
fn paint_radio(painter: &Painter, center: Pos2, selected: bool, focused: bool) {
    let corner = pos2(center.x - 6.0, (center.y - 6.0).round());
    if focused {
        painter.circle_filled(corner + vec2(6.0, 6.0), 11.0, Color32::from_black_alpha(13));
    }
    exo_ui::paint_radio(painter, corner, selected);
}

/// The tiles laid out from `top`: wrapping in the column, 12px apart.
fn tile_rects(ctx: &egui::Context, left: f32, top: f32) -> Vec<Rect> {
    let mut rects = Vec::new();
    let (mut x, mut y) = (left, top);
    for (_, label, icon) in WORKFLOWS {
        let icon_width = icon
            .and_then(|icon| exo_ui::icon_size(ctx, icon))
            .map_or(0.0, |size| size.x + TILE_ICON_GAP);
        let width =
            2.0 + TILE_PADDING.0 + icon_width + TILE_LABEL.width(ctx, label) + TILE_PADDING.1;
        if x > left && x + width > left + COLUMN {
            x = left;
            y += TILE_HEIGHT + TILE_GAP;
        }
        rects.push(Rect::from_min_size(pos2(x, y), vec2(width, TILE_HEIGHT)));
        x += width + TILE_GAP;
    }
    rects
}

/// A tile: 32px, `padding: 3px 10px 7px 11px` in a 1px border with 2px corners, `rgba(0,0,0,.12)`
/// (`#aeb1dd` checked); `rgba(174,177,221,.12)` behind it checked, hovered or with the keys. Its
/// icon at `.3` (`.7` checked, `fill-opacity`), its foot 20.8px down (`vertical-align: sub`); its
/// label 7px after.
const TILE_HEIGHT: f32 = 32.0;
const TILE_PADDING: (f32, f32) = (11.0, 10.0);
const TILE_ICON_GAP: f32 = 7.0;
const TILE_ICON_FOOT: f32 = 20.8;

fn paint_tile(ui: &Ui, painter: &Painter, rect: Rect, index: usize, checked: bool, lit: bool) {
    let (_, label, icon) = WORKFLOWS[index];
    if checked || lit {
        painter.rect_filled(
            rect,
            CornerRadius::same(2),
            Color32::from_rgba_unmultiplied(174, 177, 221, 31),
        );
    }
    let border = if checked {
        LAVENDER
    } else {
        Color32::from_black_alpha(31)
    };
    painter.rect_stroke(
        rect.shrink(0.5),
        CornerRadius::same(2),
        egui::Stroke::new(1.0, border),
        egui::StrokeKind::Middle,
    );
    let mut x = rect.left() + 1.0 + TILE_PADDING.0;
    if let Some(icon) = icon
        && let Some(size) = exo_ui::icon_size(ui.ctx(), icon)
    {
        // The icon, painted from a whole pixel.
        let at = Rect::from_min_size(pos2(x, rect.top() + TILE_ICON_FOOT - size.y).round(), size);
        exo_ui::paint_icon(
            ui,
            icon,
            at,
            Color32::from_black_alpha(if checked { 179 } else { 77 })
                .gamma_multiply(painter.opacity()),
        );
        x += size.x + TILE_ICON_GAP;
    }
    TILE_LABEL.paint_line(
        painter,
        pos2(x, rect.top() + 1.0 + 3.0 + LABEL_NUDGE),
        label,
        Color32::from_black_alpha(191),
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn discovery() -> Discovery {
        Discovery::new(&egui::Context::default(), Path::new("/nonexistent"))
    }

    #[test]
    fn none_of_the_above_takes_the_others_place() {
        let mut questions = discovery();
        questions.toggle_tile(2, 0.0);
        questions.toggle_tile(0, 0.0);
        assert_eq!(questions.workflow, [2, 0], "in the order checked");
        questions.toggle_tile(NONE_OF_THE_ABOVE, 0.0);
        assert_eq!(questions.workflow, [NONE_OF_THE_ABOVE]);
        questions.toggle_tile(NONE_OF_THE_ABOVE, 0.0);
        assert!(questions.workflow.is_empty(), "unchecked");
        questions.toggle_tile(NONE_OF_THE_ABOVE, 0.0);
        questions.toggle_tile(4, 0.0);
        assert_eq!(questions.workflow, [4]);
        assert_eq!(questions.answered, ["workflow"], "the bar moves once");
    }

    #[test]
    fn the_first_questions_arrows_go_round_from_none() {
        let mut questions = discovery();
        // `aa2`: from -1, ↑ lands two from the end.
        questions.step_index(-1);
        assert_eq!(questions.index, Some(2));
        questions.step_index(1);
        questions.step_index(1);
        assert_eq!(questions.index, Some(0), "round");
        // A dislike chosen moves the keys to the tiles, none of them yet.
        questions.choose_pain(1, 0.0);
        assert_eq!((questions.area, questions.index), (Area::Checkbox, None));
        questions.step_index(-1);
        assert_eq!(questions.index, Some(6));
    }
}
