//! Account setup: the loading screen that "tailors your inbox", then `Your ideal Split Inbox
//! setup`, the last step.
//!
//! Its look is Classic's, the same in Carbon: a lavender page (`#f0edf5`,
//! `checklist-background.png`) under the tab strip, a white left pane and, on the right, a preview
//! window of the inbox the splits make.

use std::path::Path;

use egui::{Color32, CornerRadius, Id, Key, Painter, Pos2, Rect, Sense, Ui, Vec2, pos2, vec2};
use exo_store::{PreviewEmail, PreviewList, SplitInbox, SplitLibrary, SplitPreview, Template};
use exo_ui::{BoxShadow, Family, Font, TextStyle, gradient, motion::CubicBezier, paint_text};
use jiff::tz::TimeZone;

use super::Press;
use crate::photo::Photo;

/// The first 500 ms show the page and nothing on it.
const BLANK: f64 = 0.5;
/// The loading messages, each shown 2 s, fading out over the last 150 ms and in again.
const MESSAGES: [&str; 4] = [
    "Identifying important topics and contacts",
    "Understanding how you stay organized",
    "Identifying your busiest senders",
    "Getting your new inbox ready",
];
const MESSAGE_TIME: f64 = 2.0;
const MESSAGE_FADE: f64 = 0.15;
/// The last message shows for 2 s before the loading can end, and it ends regardless 20 s in.
const LOADING_TIME: f64 = MESSAGES.len() as f64 * MESSAGE_TIME;
const LOADING_TIMEOUT: f64 = 20.0;
/// The splits the step shows: the three that take most of the inbox.
const SHOWN: usize = 3;
/// The preview lists ten threads.
const PREVIEW_THREADS: usize = 10;

/// The page: `#f0edf5` under `checklist-background.png`, `cover`, centered; in Carbon plain
/// `#f0edf5`.
const PAGE: Color32 = Color32::from_rgb(0xf0, 0xed, 0xf5);
/// The left pane: half the window, at least 500px, padded 42px; its column 64px down,
/// `calc(50vw - 84px)` wide within 416–514px and `calc(100vh - 134px)` tall, against the pane's
/// right.
const PANE_MIN: f32 = 500.0;
const PANE_PADDING: f32 = 42.0;
const COLUMN_TOP: f32 = 64.0;
const COLUMN_WIDTHS: (f32, f32) = (416.0, 514.0);
const COLUMN_SHORTER: f32 = 134.0;
/// The content's `padding-bottom: 25px`, and Classic's 36px sides.
const CONTENT_BOTTOM: f32 = 25.0;
const SIDES: f32 = 36.0;
/// `h1`: 21px/32px, `.2px` apart, ligatures off, `top: 3px`, `margin-bottom: 2px`.
const HEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 21.0, 32.0)
    .letter_spacing(0.2)
    .without_ligatures();
const HEADER_NUDGE: f32 = 3.0;
const HEADER_MARGIN: f32 = 2.0;
/// The section's subcopy: Classic's 14px/20px at `.8`, `top: 3.5px`, which Chromium draws a whole
/// pixel down.
const SUBCOPY: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 14.0, 20.0);
const SUBCOPY_NUDGE: f32 = 4.0;
/// The section's content 37px down; the rows 18px apart in Classic.
const SECTION_TOP: f32 = 37.0;
const ROW_GAP: f32 = 18.0;
/// A split's row: `padding: 14px 20px`, a 1px `#dedbd5` border with 12px corners; 12px between its
/// text and its tag and toggle, 8px between those, 10px between the name and the description.
const ROW_PADDING: Vec2 = vec2(20.0, 14.0);
const ROW_BORDER: Color32 = Color32::from_rgb(0xde, 0xdb, 0xd5);
const ROW_RADIUS: u8 = 12;
const ROW_INNER_GAP: f32 = 12.0;
const ACTIONS_GAP: f32 = 8.0;
const NAME_GAP: f32 = 10.0;
/// A row's name: 14px/20px semibold at `.85`, `top: 3px`.
const NAME: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 14.0, 20.0);
const NAME_NUDGE: f32 = 3.0;
/// A row's description: 14px/16px at `.6`, `top: 1px`, `text-wrap: pretty`.
const DESCRIPTION: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 14.0, 16.0);
const DESCRIPTION_NUDGE: f32 = 1.0;
/// A row's tag: 10px/12px semibold, `padding: 2px 4px 0`, 4px corners; black at `.55` on `.08`, or
/// the category's violet.
const TAG: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 10.0, 12.0);
const TAG_PADDING: Vec2 = vec2(4.0, 2.0);
const CATEGORY_INK: Color32 = Color32::from_rgb(83, 49, 146);
const CATEGORY_FILL: Color32 = Color32::from_rgb(232, 224, 255);
/// A split's switch: a 36×20 pill, `rgba(0,0,0,.1)` off and `rgba(0,0,0,.85)` on; a 12px white
/// thumb 4px in, sliding 16px; `.2s ease`.
const TOGGLE: Vec2 = vec2(36.0, 20.0);
const THUMB: f32 = 12.0;
const THUMB_INSET: f32 = 4.0;
const THUMB_TRAVEL: f32 = 16.0;
const TOGGLE_TIME: f32 = 0.2;
const THUMB_SHADOW: BoxShadow = BoxShadow::new(
    [0.0, 1.0],
    3.0,
    0.0,
    Color32::from_rgba_premultiplied(0, 0, 0, 51),
);
/// The list's footer: 12px/16px at `.35`, 8px below the rows, `top: 1px`.
const FOOTER: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
const FOOTER_MARGIN: f32 = 8.0;
/// The button's row: `padding-top: 17px`; the button 32px tall, `top: 1.5px`, centered, `#aeb1dd`
/// with white 14px/18px semibold capitals `.2px` apart, `padding: 8px 12px 6px`, 2px corners,
/// `0 2px 8px #aeb1dd` hovered.
const BUTTONS_TOP: f32 = 17.0;
const BUTTON_HEIGHT: f32 = 32.0;
const BUTTON_NUDGE: f32 = 1.5;
const BUTTON: TextStyle =
    TextStyle::new(Font::new(Family::Sans, 600), 14.0, 18.0).letter_spacing(0.2);
const BUTTON_PADDING: Vec2 = vec2(12.0, 8.0);
const BUTTON_FILL: Color32 = Color32::from_rgb(0xae, 0xb1, 0xdd);
const BUTTON_GLOW: BoxShadow = BoxShadow::new([0.0, 2.0], 8.0, 0.0, BUTTON_FILL);
/// The step fades in over `.3s`.
const FADE_IN: f32 = 0.3;
/// `split-row-enter` and `split-tab-enter`: `.35s ease`, 150 ms apart; `email-preview-row-enter`
/// the same, all at once.
const ENTER: f32 = 0.35;
const STAGGER: f32 = 0.15;
const ROW_ENTER: f32 = 8.0;
const TAB_ENTER: f32 = 10.0;

/// The preview window: 120px into the pane and 128px past its right and 80px past its bottom,
/// white, its top left corner 16px, `padding: 20px 24px 0`.
const WINDOW_INSET: f32 = 120.0;
const WINDOW_PAST: Vec2 = vec2(128.0, 80.0);
const WINDOW_RADIUS: u8 = 16;
const WINDOW_PADDING: Vec2 = vec2(24.0, 20.0);
const WINDOW_SHADOWS: [BoxShadow; 2] = [
    BoxShadow::new(
        [0.0, 0.0],
        2.0,
        0.0,
        Color32::from_rgba_premultiplied(2, 2, 2, 31),
    ),
    BoxShadow::new(
        [0.0, 6.0],
        24.0,
        0.0,
        Color32::from_rgba_premultiplied(2, 2, 2, 31),
    ),
];
/// The preview's tabs: 48px with `padding: 8px 0 4px`; the 16×12 menu icon, 12px before the tabs;
/// each tab `padding: 8px 12px`, its count 4px after its name.
const TABS_HEIGHT: f32 = 48.0;
const TABS_PADDING_TOP: f32 = 8.0;
const TAB_HEIGHT: f32 = 36.0;
const MENU: Vec2 = vec2(16.0, 12.0);
const MENU_GAP: f32 = 12.0;
const TAB_PADDING: Vec2 = vec2(12.0, 8.0);
const COUNT_GAP: f32 = 4.0;
/// A preview tab's name: 14px/20px, `top: 3px`, `.4` (`.8` active); its count 12px/20px
/// `all-petite-caps`, `.25` (`.35` active). Exo's font has no petite capitals, so Chromium
/// synthesizes them as it does small capitals: every character at 70% of the size, on the line's
/// baseline.
const TAB_NAME: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 14.0, 20.0);
const TAB_COUNT_LINE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
const TAB_COUNT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0 * 0.7, 20.0);
const TAB_NUDGE: f32 = 3.0;
/// A tab hiding: `max-width: 300px` to 0 over `.3s`, opacity over `.2s`, the tab sliding 10px left
/// over `.25s`.
const TAB_MAX: f32 = 300.0;
/// The focus ring of a split's switch and a preview tab.
const FOCUS_RING: Color32 = Color32::from_rgb(0x54, 0xac, 0xdc);
/// The preview's threads 12px down; rows `padding: 12px 0` round a 16px line.
const THREADS_TOP: f32 = 12.0;
const THREAD_PADDING: f32 = 12.0;
/// `-from`: 12px/16px semibold at `.85`, 35% of the row with `padding-right: 8px`; `-subject`
/// 12px/16px at `.5`; both `top: 1px`. `-date`: 10px/12px at `.4`, `padding-left: 8px`, on the
/// row's baseline.
const FROM: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 12.0, 16.0);
const SUBJECT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
const DATE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 10.0, 12.0);
const FROM_SHARE: f32 = 0.35;
const CELL_GAP: f32 = 8.0;
/// The preview's empty state: 12px/20px at `.5`, `top: 3px`, centered, `padding: 32px 16px` within
/// 360px.
const EMPTY: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
const EMPTY_PADDING: Vec2 = vec2(16.0, 32.0);
const EMPTY_WIDTH: f32 = 360.0;

/// The loading screen (Classic): white under three soft ellipses, its column centered with 8px
/// gaps; the title a pink-to-violet gradient (`background-clip: text`), 30px/40px semibold; the
/// subtitle 16px/20px medium at `.9`, `top: 3px`; an 88px ring 80px above and below; the message
/// 16px/20px at `.6`, `top: 2px`; the footer 12px/16px `#686868` 24px over the tab strip's height
/// from the bottom.
const LOADING_TITLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 30.0, 40.0);
const LOADING_SUBTITLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 500), 16.0, 20.0);
const LOADING_MESSAGE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 20.0);
const LOADING_FOOTER: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
const LOADING_GAP: f32 = 8.0;
const SPINNER: f32 = 88.0;
const SPINNER_RING: f32 = 6.0;
const SPINNER_MARGIN: f32 = 80.0;
/// `gradient-loading-spin 1.2s linear infinite`.
const SPINNER_TURN: f64 = 1.2;
const SPINNER_INK: Color32 = Color32::from_rgb(0xb6, 0x90, 0xf4);
const LOADING_FOOTER_INK: Color32 = Color32::from_rgb(0x68, 0x68, 0x68);
const LOADING_FOOTER_BOTTOM: f32 = 24.0;

const TITLE: &str = "Your ideal Split Inbox setup";
const SUBTITLE: &str = "Splits automatically sort incoming messages, so you can stay organized and focus on what matters most.";
const IMPORTANT: (&str, &str) = (
    "Important",
    "Only high-priority emails that need your attention",
);
const OTHER: (&str, &str) = ("Other", "Everything that does not fit a split");
const DEFAULT_TAG: &str = "Default";
const LIBRARY_FOOTER: &str =
    "You can update this anytime, with 50+ options in the Split Inbox Library";
const CONTINUE: &str = "Continue with this setup";
const EMPTY_IMPORTANT: &str = "No emails in your important Split Inbox. Direct, personal, and high priority messages will show up here.";
const EMPTY_OTHER: &str = "No emails in your Other Split Inbox. Messages from marketing, social networks, and automatic updates will show up here.";
const LOADING_HEADING: &str = "Tailoring your inbox to how you work";
/// The loading screen's subtitle.
const LOADING_SUBHEADING: &str = "Checking your recent email activity to customize Exo";
const LOADING_PRIVATE: &str = "Analysis happens only on your device. Your data stays private.";

/// A split the library could set up, before Exo knows how much of the inbox it would take.
#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    /// What the step calls it.
    pub name: String,
    /// As it would be turned on.
    pub split: SplitInbox,
    /// The account's own split it is, when it has one (its Calendar).
    pub existing: bool,
}

/// The library's templates the step can offer: each one this account can use with a definition of
/// its own (no Auto Labels) that isn't a built-in kind, and Calendar; the account's own split for
/// one when it has it (its default Calendar, or what an earlier setup turned on).
pub(crate) fn candidates(
    library: &SplitLibrary,
    splits: &[SplitInbox],
    definitions: exo_store::Definitions<'_>,
) -> Vec<Candidate> {
    let built_in = |template: &Template| {
        template
            .kind
            .as_deref()
            .is_some_and(|kind| kind != "calendar")
    };
    library
        .templates
        .values()
        .filter(|template| !built_in(template))
        .filter_map(|template| {
            if let Some(split) = template.split_in(splits) {
                return Some(Candidate {
                    name: split.name().to_owned(),
                    split: split.clone(),
                    existing: true,
                });
            }
            let senders = template
                .properties
                .senders
                .as_ref()
                .map(|senders| senders.value.clone())
                .unwrap_or_default();
            let query = template.query_with(&senders);
            if query.trim().is_empty() {
                return None;
            }
            let mut split = SplitInbox::new(
                &template.name,
                &query,
                template.kind.as_deref(),
                Some(&template.id),
                definitions,
            );
            split.leave_threads_in_important_other =
                template.properties.show_in_important_other.unwrap_or(false);
            Some(Candidate {
                name: template.name.clone(),
                split,
                existing: false,
            })
        })
        .collect()
}

/// What the step says of a split, by its kind or name.
fn description(name: &str, kind: Option<&str>) -> Option<&'static str> {
    match (kind, name) {
        (Some("calendar"), _) => Some("Your calendar updates and reminders"),
        (Some("news"), _) => Some("Emails from subscriptions"),
        (_, "Asana") => Some("Stay on top of Asana tasks and projects"),
        (_, "GitHub") => Some("Stay on top of PRs and comments from GitHub"),
        (_, "Google Docs") => Some("Stay on top of Google Docs and comments"),
        (_, "Basecamp") => Some("Stay on top of Basecamp to-dos and projects"),
        (_, "ClickUp") => Some("Stay on top of ClickUp notifications"),
        (_, "Jira") => Some("Jira updates and issue activity"),
        (_, "Linear") => Some("Stay on top of Linear tasks and projects"),
        (_, "Monday") => Some("Stay on top of Monday notifications"),
        (_, "Notion") => Some("Stay on top of Notion pages and comments"),
        (_, "OneDrive") => Some("Stay on top of OneDrive notifications"),
        (_, "Sharepoint") => Some("Stay on top of Sharepoint notifications"),
        (_, "Trello") => Some("Stay on top of Trello tasks and projects"),
        _ => None,
    }
}

/// The tag a split's row shows, by its kind or name.
fn category_tag(name: &str, kind: Option<&str>) -> Option<&'static str> {
    const NOTIFICATIONS: [&str; 12] = [
        "Asana",
        "Basecamp",
        "ClickUp",
        "GitHub",
        "Google Docs",
        "Jira",
        "Linear",
        "Monday",
        "Notion",
        "OneDrive",
        "Sharepoint",
        "Trello",
    ];
    match kind {
        Some("calendar") => Some("Top category"),
        Some("news") => Some("Common tag"),
        _ => NOTIFICATIONS
            .contains(&name)
            .then_some("Frequent notifications"),
    }
}

/// One of the shown splits.
#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub name: String,
    kind: Option<String>,
    /// Its split's list id now (toggling one mints another).
    pub id: String,
    pub enabled: bool,
    /// What it takes of the inbox.
    list: PreviewList,
    /// Times it was turned back on: its tab enters again each time.
    reentered: u32,
    /// When it was last turned on or off.
    toggled: Option<f64>,
}

/// A preview tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    Important,
    Split(usize),
    Other,
}

/// The split step's state.
#[derive(Debug)]
pub(crate) struct Splits {
    since: f64,
    pub rows: Vec<Row>,
    important: PreviewList,
    other: PreviewList,
    tab: Tab,
    /// When the tab on screen was picked: its rows enter then.
    tab_since: f64,
    scroll: f32,
    /// What has the keyboard, and whether it shows (`:focus-visible`, once a key is pressed).
    focus: Option<Focus>,
    focus_visible: bool,
}

/// What has the keyboard in the split step: `Continue` from the start, else what was clicked or
/// tabbed to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    /// The content, which a click on a row or the page focuses.
    Content,
    /// A row's switch, clicked.
    Toggle(usize),
    /// The button's own region.
    ButtonRegion,
    Continue,
    /// The preview's tab (only the active one is in the tab order).
    Tab(Tab),
    /// The preview's list, a stop of its own.
    Tabpanel,
}

impl Splits {
    /// The step over `preview`'s ranking of `candidates`: the three that take most of the inbox,
    /// with Important's tab up unless it's empty and something else isn't. None when no candidate
    /// takes anything (the step is skipped).
    pub(crate) fn new(candidates: &[Candidate], preview: SplitPreview, now: f64) -> Option<Self> {
        let mut ranked: Vec<(&Candidate, PreviewList)> = candidates
            .iter()
            .zip(preview.splits)
            .filter(|(_, list)| list.count > 0)
            .collect();
        ranked.sort_by_key(|(_, list)| std::cmp::Reverse(list.count));
        ranked.truncate(SHOWN);
        if ranked.is_empty() {
            return None;
        }
        let rows = ranked
            .into_iter()
            .map(|(candidate, list)| Row {
                name: candidate.name.clone(),
                kind: candidate.split.kind.clone(),
                id: candidate.split.id.clone(),
                enabled: true,
                list,
                reentered: 0,
                toggled: None,
            })
            .collect();
        let mut splits = Self {
            since: now,
            rows,
            important: preview.important,
            other: preview.other,
            tab: Tab::Important,
            tab_since: now,
            scroll: 0.0,
            focus: Some(Focus::Continue),
            focus_visible: false,
        };
        if splits.count(Tab::Important) == 0 {
            splits.tab = splits
                .rows
                .iter()
                .position(|row| row.list.count > 0)
                .map_or(
                    if splits.other.count > 0 {
                        Tab::Other
                    } else {
                        Tab::Important
                    },
                    Tab::Split,
                );
        }
        Some(splits)
    }

    /// A tab's count: a split turned off gives its conversations back to Important (only News would
    /// go to Other, and News isn't offered).
    fn count(&self, tab: Tab) -> usize {
        match tab {
            Tab::Important => {
                self.important.count
                    + self
                        .rows
                        .iter()
                        .filter(|row| !row.enabled)
                        .map(|row| row.list.count)
                        .sum::<usize>()
            }
            Tab::Other => self.other.count,
            Tab::Split(index) => self.rows[index].list.count,
        }
    }

    /// The tab's first ten conversations: Important's own, then those of the splits turned off.
    fn threads(&self, tab: Tab) -> Vec<&PreviewEmail> {
        let own = match tab {
            Tab::Important => &self.important,
            Tab::Other => &self.other,
            Tab::Split(index) => &self.rows[index].list,
        };
        let given = self
            .rows
            .iter()
            .filter(|row| tab == Tab::Important && !row.enabled)
            .flat_map(|row| row.list.threads.iter());
        own.threads
            .iter()
            .chain(given)
            .take(PREVIEW_THREADS)
            .collect()
    }

    /// A row was turned on or off, `id` its split's new list id: a tab turned off that was up
    /// gives way to Important.
    pub(crate) fn toggled(&mut self, index: usize, id: String, now: f64) {
        let row = &mut self.rows[index];
        row.enabled = !row.enabled;
        row.id = id;
        row.toggled = Some(now);
        if row.enabled {
            row.reentered += 1;
        } else if self.tab == Tab::Split(index) {
            self.pick(Tab::Important, now);
        }
    }

    fn pick(&mut self, tab: Tab, now: f64) {
        if self.tab != tab {
            self.tab = tab;
            self.tab_since = now;
        }
    }

    /// Focus given by a click, which shows no ring.
    fn click_focus(&mut self, focus: Focus) {
        self.focus = Some(focus);
        self.focus_visible = false;
    }

    /// A key on the step: Tab moves the focus, and Enter (down) or Space (up) on a focused button
    /// clicks it.
    pub(crate) fn press(&mut self, press: Press, now: f64) -> Option<Event> {
        let Press {
            key,
            pressed,
            shift,
        } = press;
        if self.focus.is_some() {
            self.focus_visible = true;
        }
        match (key, pressed) {
            (Key::Tab, true) => {
                self.focus = tab_from(self.focus, self.tab, shift);
                None
            }
            (Key::Enter, true) | (Key::Space, false) => match self.focus? {
                Focus::Continue => Some(Event::Continue),
                Focus::Toggle(index) => Some(Event::Toggle(index)),
                Focus::Tab(tab) => {
                    self.pick(tab, now);
                    None
                }
                Focus::Content | Focus::ButtonRegion | Focus::Tabpanel => None,
            },
            _ => None,
        }
    }
}

/// Where Tab (⇧Tab back) takes the keyboard from `focus`: round account setup's own stops (the
/// content, the button's region, the active tab, the list), from the one holding it; the button's
/// region has only the button, so it keeps it there.
fn tab_from(focus: Option<Focus>, active: Tab, back: bool) -> Option<Focus> {
    let ring = [
        Focus::Content,
        Focus::ButtonRegion,
        Focus::Tab(active),
        Focus::Tabpanel,
    ];
    let at: isize = match focus {
        Some(Focus::Continue | Focus::ButtonRegion) => return focus,
        Some(Focus::Content | Focus::Toggle(_)) => 0,
        Some(Focus::Tab(_)) => 2,
        Some(Focus::Tabpanel) => 3,
        None => -1,
    };
    let step = if back { -1 } else { 1 };
    Some(ring[(at + step).rem_euclid(ring.len() as isize) as usize])
}

/// Where account setup is.
#[derive(Debug)]
pub(crate) enum Step {
    /// Tailoring: the preview is being worked out (`None` until it is).
    Loading {
        since: Option<f64>,
        preview: Option<SplitPreview>,
    },
    /// `Your ideal Split Inbox setup`.
    Splits(Splits),
}

/// What happened on screen this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    /// The loading screen is done: the step decides what's next.
    Loaded,
    /// A row's toggle (or the row) was clicked.
    Toggle(usize),
    /// `Continue with this setup`.
    Continue,
}

/// Account setup over the window.
pub(crate) struct Setup {
    id: Id,
    opened: f64,
    pub step: Step,
    pub candidates: Vec<Candidate>,
    background: Photo,
}

impl std::fmt::Debug for Setup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Setup")
            .field("step", &self.step)
            .finish_non_exhaustive()
    }
}

impl Setup {
    /// Account setup opening now, its page's image from `images`.
    pub(crate) fn new(ctx: &egui::Context, images: &Path, candidates: Vec<Candidate>) -> Self {
        let now = ctx.input(|input| input.time);
        Self {
            id: exo_ui::id(ctx, ("account-setup", now.to_bits())),
            opened: now,
            step: Step::Loading {
                since: None,
                preview: None,
            },
            candidates,
            background: Photo::load(ctx, images.join("checklist-background.png")),
        }
    }

    /// Its page's image is decoded. For screenshots and tests.
    pub(crate) fn is_idle(&self) -> bool {
        self.background.is_loaded()
    }

    /// The split step, if it's up.
    pub(crate) fn splits(&self) -> Option<&Splits> {
        match &self.step {
            Step::Splits(splits) => Some(splits),
            Step::Loading { .. } => None,
        }
    }

    /// The split step, if it's up, to change.
    pub(crate) fn splits_mut(&mut self) -> Option<&mut Splits> {
        match &mut self.step {
            Step::Splits(splits) => Some(splits),
            Step::Loading { .. } => None,
        }
    }

    /// Draws it over `window`, under a tab strip `strip_height` tall, and says what was done.
    pub(crate) fn show(
        &mut self,
        ui: &mut Ui,
        window: Rect,
        strip_height: f32,
        zone: &TimeZone,
    ) -> Option<Event> {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|input| input.time);
        // Under the tab strip, over its bottom pixel, a window tall.
        let page = Rect::from_min_size(
            pos2(window.left(), window.top() + strip_height - 1.0),
            window.size(),
        );
        let painter = ui.painter().with_clip_rect(window);
        // Its clicks are its own; one on the page focuses the content.
        let cover = ui.interact(window, self.id.with("cover"), Sense::CLICK | Sense::DRAG);
        if cover.clicked()
            && let Step::Splits(splits) = &mut self.step
        {
            splits.click_focus(Focus::Content);
        }
        painter.rect_filled(page, 0.0, PAGE);
        if exo_ui::theme(&ctx) == exo_ui::Theme::Snow {
            self.background.paint(&painter, page, |free| free / 2.0);
        }
        if now - self.opened < BLANK {
            ctx.request_repaint();
            return None;
        }
        match &mut self.step {
            Step::Loading { since, preview } => {
                let since = *since.get_or_insert(now);
                paint_loading(&painter, page, strip_height, now - since);
                ctx.request_repaint();
                let elapsed = now - since;
                (elapsed >= LOADING_TIME && preview.is_some() || elapsed >= LOADING_TIMEOUT)
                    .then_some(Event::Loaded)
            }
            Step::Splits(splits) => show_splits(ui, &painter, page, splits, zone, self.id, now),
        }
    }
}

/// The loading screen over `page`, `elapsed` seconds in.
fn paint_loading(painter: &Painter, page: Rect, strip_height: f32, elapsed: f64) {
    let ctx = painter.ctx();
    // The loading screen's background (Classic, `top: 0`): three `radial-gradient` ellipses, each
    // its color fading out by 70% of the way, over white; the first on top.
    let rgba = |r: u8, g: u8, b: u8, a: f32| {
        [
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
            a,
        ]
    };
    let ellipses = [
        (
            vec2(0.60, 0.55),
            vec2(0.08, 0.95),
            rgba(252, 200, 215, 0.85),
        ),
        (
            vec2(0.55, 0.65),
            vec2(0.78, 0.18),
            rgba(196, 224, 245, 0.75),
        ),
        (
            vec2(0.45, 0.55),
            vec2(1.00, 0.28),
            rgba(220, 200, 245, 0.70),
        ),
    ];
    gradient::paint_field(painter, page, 128, 80, |point| {
        let mut color = [1.0; 4];
        for (radii, center, ink) in ellipses.iter().rev() {
            let at = gradient::radial(
                point,
                page.min + *center * page.size(),
                *radii * page.size(),
            );
            let clear = [ink[0], ink[1], ink[2], 0.0];
            color = gradient::over(gradient::ramp(&[(0.0, *ink), (0.7, clear)], at), color);
        }
        gradient::opaque(color)
    });
    let column = LOADING_TITLE.line_height
        + LOADING_GAP
        + LOADING_SUBTITLE.line_height
        + LOADING_GAP
        + SPINNER_MARGIN * 2.0
        + SPINNER
        + LOADING_GAP
        + LOADING_MESSAGE.line_height;
    let top = page.center().y - column / 2.0;
    let centered =
        |style: &TextStyle, text: &str| pos2(page.center().x - style.width(ctx, text) / 2.0, 0.0);
    // `-webkit-linear-gradient(left, #e39cb0, #abcef2, #b690f4)` across the title.
    let title = LOADING_TITLE.single_line(painter, LOADING_HEADING, Color32::WHITE, None);
    let width = title.size().x;
    let stops: [(f32, [f32; 4]); 3] = [
        (0.0, rgba(0xe3, 0x9c, 0xb0, 1.0)),
        (0.5, rgba(0xab, 0xce, 0xf2, 1.0)),
        (1.0, rgba(0xb6, 0x90, 0xf4, 1.0)),
    ];
    paint_text(
        painter,
        pos2(page.center().x - width / 2.0, top),
        gradient::tint_galley(&title, width, &stops),
        &LOADING_TITLE,
    );
    let subtitle_top = top + LOADING_TITLE.line_height + LOADING_GAP;
    LOADING_SUBTITLE.paint_line(
        painter,
        centered(&LOADING_SUBTITLE, LOADING_SUBHEADING) + vec2(0.0, subtitle_top + 3.0),
        LOADING_SUBHEADING,
        Color32::from_black_alpha(230),
        None,
    );
    let spinner = Rect::from_center_size(
        pos2(
            page.center().x,
            subtitle_top
                + LOADING_SUBTITLE.line_height
                + LOADING_GAP
                + SPINNER_MARGIN
                + SPINNER / 2.0,
        ),
        Vec2::splat(SPINNER),
    );
    paint_ring(painter, spinner, (elapsed / SPINNER_TURN).fract() as f32);
    // The message, changing every 2 s, out over 150 ms before it does and in after.
    let index = ((elapsed / MESSAGE_TIME) as usize).min(MESSAGES.len() - 1);
    let into = elapsed - index as f64 * MESSAGE_TIME;
    let fade = |t: f64| CubicBezier::EASE.at((t / MESSAGE_FADE).clamp(0.0, 1.0) as f32);
    let opacity = if index < MESSAGES.len() - 1 && into >= MESSAGE_TIME - MESSAGE_FADE {
        1.0 - fade(into - (MESSAGE_TIME - MESSAGE_FADE))
    } else if index > 0 {
        fade(into)
    } else {
        1.0
    };
    let message = MESSAGES[index];
    let message_top = spinner.bottom() + SPINNER_MARGIN + LOADING_GAP + 2.0;
    LOADING_MESSAGE.paint_line(
        painter,
        centered(&LOADING_MESSAGE, message) + vec2(0.0, message_top),
        message,
        Color32::from_black_alpha(153).gamma_multiply(opacity),
        None,
    );
    // The footer sits 24px over the tab strip's height from the bottom.
    let footer_top =
        page.bottom() - LOADING_FOOTER_BOTTOM - strip_height - LOADING_FOOTER.line_height;
    LOADING_FOOTER.paint_line(
        painter,
        centered(&LOADING_FOOTER, LOADING_PRIVATE) + vec2(0.0, footer_top),
        LOADING_PRIVATE,
        LOADING_FOOTER_INK,
        None,
    );
}

/// The loading ring: `conic-gradient(#b690f4 0deg 270deg, transparent 270deg)` masked to its outer
/// 6px, `turn` of the way round.
fn paint_ring(painter: &Painter, rect: Rect, turn: f32) {
    let radius = rect.width() / 2.0 - SPINNER_RING / 2.0;
    let start = turn * std::f32::consts::TAU;
    let points: Vec<Pos2> = (0..=90)
        .map(|step| {
            // Clockwise from 12 o'clock.
            let angle = start + 0.75 * std::f32::consts::TAU * step as f32 / 90.0;
            rect.center() + radius * vec2(angle.sin(), -angle.cos())
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(SPINNER_RING, SPINNER_INK),
    ));
}

/// How far into a keyframe animation `duration` long, starting `delay` after `since`, `now` is
/// (eased).
fn entered(now: f64, since: f64, delay: f32, duration: f32) -> f32 {
    CubicBezier::EASE.at((((now - since) as f32 - delay) / duration).clamp(0.0, 1.0))
}

/// The left pane's column at this window size.
fn column(page: Rect) -> Rect {
    let pane = Rect::from_min_size(
        page.min,
        vec2((page.width() / 2.0).max(PANE_MIN), page.height()),
    );
    let width = (page.width() / 2.0 - 2.0 * PANE_PADDING).clamp(COLUMN_WIDTHS.0, COLUMN_WIDTHS.1);
    Rect::from_min_size(
        pos2(pane.right() - PANE_PADDING - width, page.top() + COLUMN_TOP),
        vec2(width, page.height() - COLUMN_SHORTER),
    )
}

fn show_splits(
    ui: &mut Ui,
    painter: &Painter,
    page: Rect,
    splits: &mut Splits,
    zone: &TimeZone,
    id: Id,
    now: f64,
) -> Option<Event> {
    let ctx = ui.ctx().clone();
    let shown = entered(now, splits.since, 0.0, FADE_IN);
    if shown < 1.0
        || (now - splits.since) < f64::from(STAGGER * (splits.rows.len() + 2) as f32 + ENTER)
        || (now - splits.tab_since) < f64::from(ENTER)
    {
        ctx.request_repaint();
    }
    let mut painter = painter.clone();
    painter.set_opacity(shown);
    let pane = Rect::from_min_size(
        page.min,
        vec2((page.width() / 2.0).max(PANE_MIN), page.height()),
    );
    painter.rect_filled(pane, 0.0, Color32::WHITE);
    let column = column(page);
    let buttons = Rect::from_min_max(
        pos2(column.left(), column.bottom() - BUTTONS_TOP - BUTTON_HEIGHT),
        column.max,
    );
    let content = Rect::from_min_max(column.min, pos2(column.right(), buttons.top()));
    let mut event = None;
    let mut clicked = None;

    // The content scrolls under the buttons.
    let response = ui.interact(content, id.with("content"), Sense::hover());
    if response.hovered() {
        splits.scroll -= ui.input(|input| input.smooth_scroll_delta.y);
    }
    let text = content.shrink2(vec2(SIDES, 0.0));
    let subcopy = SUBCOPY.wrap(&ctx, SUBTITLE, text.width());
    let rows = rows_layout(&ctx, splits, text.width());
    let list_height: f32 =
        rows.iter().map(|row| row.height).sum::<f32>() + ROW_GAP * (rows.len() - 1) as f32;
    let full = HEADER.line_height
        + HEADER_MARGIN
        + subcopy.len() as f32 * SUBCOPY.line_height
        + SECTION_TOP
        + list_height
        + ROW_GAP
        + FOOTER_MARGIN
        + FOOTER.line_height
        + CONTENT_BOTTOM;
    let scroll = splits.scroll.clamp(0.0, (full - content.height()).max(0.0));
    let clipped = painter.with_clip_rect(content.intersect(painter.clip_rect()));
    let top = content.top() - scroll;
    HEADER.paint_line(
        &clipped,
        pos2(text.left(), top + HEADER_NUDGE),
        TITLE,
        Color32::from_black_alpha(230),
        None,
    );
    let mut y = top + HEADER.line_height + HEADER_MARGIN;
    for line in &subcopy {
        SUBCOPY.paint_line(
            &clipped,
            pos2(text.left(), y + SUBCOPY_NUDGE),
            line,
            Color32::from_black_alpha(204),
            None,
        );
        y += SUBCOPY.line_height;
    }
    y += SECTION_TOP;
    for (index, row) in rows.iter().enumerate() {
        let rect = Rect::from_min_size(pos2(text.left(), y), vec2(text.width(), row.height));
        let enter = entered(now, splits.since, STAGGER * index as f32, ENTER);
        let lift = ROW_ENTER * (1.0 - enter);
        let mut row_painter = clipped.clone();
        row_painter.set_opacity(clipped.opacity() * enter);
        let ring = row.split.is_some_and(|split| {
            splits.focus_visible && splits.focus == Some(Focus::Toggle(split))
        });
        paint_row(
            &row_painter,
            rect.translate(vec2(0.0, -lift)),
            row,
            &ctx,
            now,
            ring,
        );
        // The row toggles as its switch does, but focuses the content where the switch takes the
        // keyboard itself.
        if let Some(split) = row.split {
            if ui
                .interact(
                    rect.intersect(content),
                    id.with(("row", split)),
                    Sense::CLICK,
                )
                .clicked()
            {
                clicked = Some(Focus::Content);
                event = Some(Event::Toggle(split));
            }
            if ui
                .interact(
                    toggle_rect(rect).intersect(content),
                    id.with(("toggle", split)),
                    Sense::CLICK,
                )
                .clicked()
            {
                clicked = Some(Focus::Toggle(split));
                event = Some(Event::Toggle(split));
            }
        }
        y += row.height + ROW_GAP;
    }
    FOOTER.paint_line(
        &clipped,
        pos2(text.left(), y + FOOTER_MARGIN + 1.0),
        LIBRARY_FOOTER,
        Color32::from_black_alpha(89),
        None,
    );
    if let Some(focus) = clicked {
        splits.click_focus(focus);
    }

    // `Continue with this setup`, focused.
    let label = CONTINUE.to_uppercase();
    let width = BUTTON.width(&ctx, &label) + 2.0 * BUTTON_PADDING.x;
    let button = exo_ui::pixel_snap(Rect::from_min_size(
        pos2(
            buttons.center().x - width / 2.0,
            buttons.top() + BUTTONS_TOP + BUTTON_NUDGE,
        ),
        vec2(width, BUTTON_HEIGHT),
    ));
    // The button's region, then the button over it.
    if ui
        .interact(
            Rect::from_min_max(pos2(column.left(), buttons.top()), column.max),
            id.with("buttons"),
            Sense::CLICK,
        )
        .clicked()
    {
        splits.click_focus(Focus::ButtonRegion);
    }
    let response = ui.interact(button, id.with("continue"), Sense::CLICK);
    if response.hovered() {
        BUTTON_GLOW.paint(&painter, button, CornerRadius::same(2));
    }
    painter.rect_filled(button, CornerRadius::same(2), BUTTON_FILL);
    BUTTON.paint_line(
        &painter,
        pos2(
            button.left() + BUTTON_PADDING.x,
            button.top() + BUTTON_PADDING.y,
        ),
        &label,
        Color32::WHITE,
        None,
    );
    if response.clicked() {
        splits.click_focus(Focus::Continue);
        event = Some(Event::Continue);
    }

    splits.scroll = scroll;
    let right = Rect::from_min_max(pos2(pane.right(), page.top()), page.max);
    match show_preview(
        ui,
        &painter.with_clip_rect(right.intersect(painter.clip_rect())),
        right,
        splits,
        zone,
        id,
        now,
    ) {
        Some(Focus::Tab(tab)) => {
            splits.pick(tab, now);
            splits.click_focus(Focus::Tab(tab));
        }
        Some(focus) => splits.click_focus(focus),
        None => {}
    }
    event
}

/// A row of the list as laid out at `width`.
struct RowLayout<'a> {
    name: &'a str,
    description_lines: Vec<String>,
    tag: Option<(&'static str, bool)>,
    /// Which shown split it is; none for Important and Other.
    split: Option<usize>,
    enabled: bool,
    toggled: Option<f64>,
    height: f32,
}

fn rows_layout<'a>(ctx: &egui::Context, splits: &'a Splits, width: f32) -> Vec<RowLayout<'a>> {
    let inner = width - 2.0 * (ROW_PADDING.x + 1.0);
    let mut rows = Vec::new();
    let mut push = |name: &'a str,
                    description: Option<&str>,
                    tag: Option<(&'static str, bool)>,
                    split: Option<usize>,
                    enabled: bool,
                    toggled: Option<f64>| {
        // The text takes what the tag and toggle leave it, and no more than it needs.
        let tag_width = tag.map_or(0.0, |(tag, _)| TAG.width(ctx, tag) + 2.0 * TAG_PADDING.x);
        let actions = tag_width
            + if split.is_some() {
                ACTIONS_GAP * f32::from(tag.is_some()) + TOGGLE.x
            } else {
                0.0
            };
        let available = inner - ROW_INNER_GAP - actions;
        let description_lines = description
            .map(|text| DESCRIPTION.wrap_pretty(ctx, text, available))
            .unwrap_or_default();
        let text_height = NAME.line_height
            + if description_lines.is_empty() {
                0.0
            } else {
                NAME_GAP + description_lines.len() as f32 * DESCRIPTION.line_height
            };
        rows.push(RowLayout {
            name,
            description_lines,
            tag,
            split,
            enabled,
            toggled,
            height: 2.0 + 2.0 * ROW_PADDING.y + text_height,
        });
    };
    push(
        IMPORTANT.0,
        Some(IMPORTANT.1),
        Some((DEFAULT_TAG, false)),
        None,
        true,
        None,
    );
    for (index, row) in splits.rows.iter().enumerate() {
        let kind = row.kind.as_deref();
        push(
            &row.name,
            description(&row.name, kind),
            category_tag(&row.name, kind).map(|tag| (tag, true)),
            Some(index),
            row.enabled,
            row.toggled,
        );
    }
    push(
        OTHER.0,
        Some(OTHER.1),
        Some((DEFAULT_TAG, false)),
        None,
        true,
        None,
    );
    rows
}

/// Paints a row in `rect`.
fn paint_row(
    painter: &Painter,
    rect: Rect,
    row: &RowLayout<'_>,
    ctx: &egui::Context,
    now: f64,
    ring: bool,
) {
    painter.rect_stroke(
        rect.shrink(0.5),
        CornerRadius::same(ROW_RADIUS),
        egui::Stroke::new(1.0, ROW_BORDER),
        egui::StrokeKind::Middle,
    );
    let inner = rect.shrink2(ROW_PADDING + Vec2::splat(1.0));
    let mut right = inner.right();
    if row.split.is_some() {
        let toggle = toggle_rect(rect);
        if ring {
            paint_focus_ring(painter, toggle, 10);
        }
        // `.2s ease` from where it was.
        let on = match row.toggled {
            Some(at) => {
                let progress =
                    CubicBezier::EASE.at(((now - at) as f32 / TOGGLE_TIME).clamp(0.0, 1.0));
                if (now - at) < f64::from(TOGGLE_TIME) {
                    ctx.request_repaint();
                }
                if row.enabled {
                    progress
                } else {
                    1.0 - progress
                }
            }
            None => f32::from(u8::from(row.enabled)),
        };
        let off_fill = Color32::from_black_alpha(26);
        let on_fill = Color32::from_black_alpha(217);
        painter.rect_filled(
            toggle,
            CornerRadius::same(10),
            exo_ui::motion::mix(off_fill, on_fill, on),
        );
        let thumb = Rect::from_min_size(
            pos2(
                toggle.left() + THUMB_INSET + THUMB_TRAVEL * on,
                toggle.center().y - THUMB / 2.0,
            ),
            Vec2::splat(THUMB),
        );
        THUMB_SHADOW.paint(painter, thumb, CornerRadius::same(6));
        painter.circle_filled(thumb.center(), THUMB / 2.0, Color32::WHITE);
        right = toggle.left() - ACTIONS_GAP;
    }
    if let Some((tag, category)) = row.tag {
        let width = TAG.width(ctx, tag) + 2.0 * TAG_PADDING.x;
        let height = TAG.line_height + TAG_PADDING.y;
        let rect = Rect::from_min_size(
            pos2(right - width, inner.center().y - height / 2.0),
            vec2(width, height),
        );
        let (fill, ink) = if category {
            (CATEGORY_FILL, CATEGORY_INK)
        } else {
            (
                Color32::from_black_alpha(20),
                Color32::from_black_alpha(140),
            )
        };
        painter.rect_filled(rect, CornerRadius::same(4), fill);
        TAG.paint_line(
            painter,
            pos2(rect.left() + TAG_PADDING.x, rect.top() + TAG_PADDING.y),
            tag,
            ink,
            None,
        );
        right = rect.left();
    }
    let text_width = right - ROW_INNER_GAP - inner.left();
    NAME.paint_line(
        painter,
        pos2(inner.left(), inner.top() + NAME_NUDGE),
        row.name,
        Color32::from_black_alpha(217),
        Some(text_width),
    );
    let mut y = inner.top() + NAME.line_height + NAME_GAP + DESCRIPTION_NUDGE;
    for line in &row.description_lines {
        DESCRIPTION.paint_line(
            painter,
            pos2(inner.left(), y),
            line,
            Color32::from_black_alpha(153),
            None,
        );
        y += DESCRIPTION.line_height;
    }
}

/// A row's switch: at the right of its padding, centred.
fn toggle_rect(row: Rect) -> Rect {
    let inner = row.shrink2(ROW_PADDING + Vec2::splat(1.0));
    Rect::from_min_size(
        pos2(inner.right() - TOGGLE.x, inner.center().y - TOGGLE.y / 2.0),
        TOGGLE,
    )
}

/// `:focus-visible { outline: 2px solid #54acdc; outline-offset: 2px }` round `rect`, following
/// its `radius`.
fn paint_focus_ring(painter: &Painter, rect: Rect, radius: u8) {
    painter.rect_stroke(
        rect.expand(3.0),
        CornerRadius::same(radius + 3),
        egui::Stroke::new(2.0, FOCUS_RING),
        egui::StrokeKind::Middle,
    );
}

/// The preview window in the right pane; returns what a click focused (a tab, which it picks,
/// or the list).
fn show_preview(
    ui: &mut Ui,
    painter: &Painter,
    pane: Rect,
    splits: &Splits,
    zone: &TimeZone,
    id: Id,
    now: f64,
) -> Option<Focus> {
    let ctx = ui.ctx().clone();
    let window = Rect::from_min_max(pane.min + Vec2::splat(WINDOW_INSET), pane.max + WINDOW_PAST);
    let corners = CornerRadius {
        nw: WINDOW_RADIUS,
        ..CornerRadius::ZERO
    };
    for shadow in WINDOW_SHADOWS {
        shadow.paint(painter, window, corners);
    }
    painter.rect_filled(window, corners, Color32::WHITE);
    let inner = Rect::from_min_max(
        window.min + WINDOW_PADDING,
        pos2(window.right() - WINDOW_PADDING.x, window.bottom()),
    );
    let tabs_top = inner.top() + TABS_PADDING_TOP;
    let menu = Rect::from_min_size(
        pos2(inner.left(), tabs_top + TAB_HEIGHT / 2.0 - MENU.y / 2.0),
        MENU,
    );
    // The hamburger, black at .6.
    exo_ui::paint_icon(
        ui,
        "legacy/DEPRECATED_hamburgerMenu",
        menu,
        Color32::from_black_alpha(153),
    );
    let mut x = menu.right() + MENU_GAP;
    let mut clicked = None;
    let tabs: Vec<Tab> = std::iter::once(Tab::Important)
        .chain((0..splits.rows.len()).map(Tab::Split))
        .chain(std::iter::once(Tab::Other))
        .collect();
    for (index, tab) in tabs.iter().enumerate() {
        let name = match tab {
            Tab::Important => IMPORTANT.0,
            Tab::Other => OTHER.0,
            Tab::Split(split) => splits.rows[*split].name.as_str(),
        };
        let count = splits.count(*tab).to_string();
        let active = splits.tab == *tab;
        let natural = 2.0 * TAB_PADDING.x
            + TAB_NAME.width(&ctx, name)
            + COUNT_GAP
            + TAB_COUNT.width(&ctx, &count);
        // Entering (staggered at first, at once when turned back on), and collapsing when off.
        let (mut opacity, mut slide, mut width) = (1.0, 0.0, natural);
        let row = match tab {
            Tab::Split(split) => Some(&splits.rows[*split]),
            _ => None,
        };
        let entering = match row {
            Some(row) if row.reentered > 0 => row.toggled.map(|at| entered(now, at, 0.0, ENTER)),
            _ => Some(entered(now, splits.since, STAGGER * index as f32, ENTER)),
        };
        if let Some(row) = row.filter(|row| !row.enabled) {
            let since = row.toggled.unwrap_or(splits.since);
            let collapse = |duration: f32| {
                CubicBezier::EASE.at(((now - since) as f32 / duration).clamp(0.0, 1.0))
            };
            width = natural.min(TAB_MAX * (1.0 - collapse(0.3)));
            opacity = 1.0 - collapse(0.2);
            slide = -TAB_ENTER * collapse(0.25);
            if now - since < 0.3 {
                ctx.request_repaint();
            }
        } else if let Some(enter) = entering {
            opacity = enter;
            slide = -TAB_ENTER * (1.0 - enter);
            if enter < 1.0 {
                ctx.request_repaint();
            }
        }
        let wrapper = Rect::from_min_size(pos2(x, tabs_top), vec2(width, TAB_HEIGHT));
        if width > 0.0 && opacity > 0.0 {
            let mut tab_painter = painter.with_clip_rect(wrapper.intersect(painter.clip_rect()));
            tab_painter.set_opacity(painter.opacity() * opacity);
            let left = wrapper.left() + TAB_PADDING.x + slide;
            let (name_ink, count_ink) = if active {
                (
                    Color32::from_black_alpha(204),
                    Color32::from_black_alpha(89),
                )
            } else {
                (
                    Color32::from_black_alpha(102),
                    Color32::from_black_alpha(64),
                )
            };
            let name_width = TAB_NAME.paint_line(
                &tab_painter,
                pos2(left, wrapper.top() + TAB_PADDING.y + TAB_NUDGE),
                name,
                name_ink,
                None,
            );
            let count_top =
                wrapper.top() + TAB_PADDING.y + TAB_NUDGE + TAB_COUNT_LINE.baseline(&ctx)
                    - TAB_COUNT.baseline(&ctx);
            TAB_COUNT.paint_line(
                &tab_painter,
                pos2(left + name_width + COUNT_GAP, count_top),
                &count,
                count_ink,
                None,
            );
            if row.is_none_or(|row| row.enabled)
                && ui
                    .interact(
                        wrapper.intersect(pane),
                        id.with(("tab", index)),
                        Sense::CLICK,
                    )
                    .clicked()
            {
                clicked = Some(Focus::Tab(*tab));
            }
            if active && splits.focus_visible && splits.focus == Some(Focus::Tab(*tab)) {
                paint_focus_ring(
                    painter,
                    Rect::from_min_size(
                        pos2(left - TAB_PADDING.x, wrapper.top()),
                        vec2(natural, TAB_HEIGHT),
                    ),
                    10,
                );
            }
        }
        x += width;
    }

    let list_top = inner.top() + TABS_HEIGHT + THREADS_TOP;
    if ui
        .interact(
            Rect::from_min_max(pos2(window.left(), list_top), window.max).intersect(pane),
            id.with("tabpanel"),
            Sense::CLICK,
        )
        .clicked()
    {
        clicked = Some(Focus::Tabpanel);
    }
    let threads = splits.threads(splits.tab);
    if threads.is_empty() && matches!(splits.tab, Tab::Important | Tab::Other) {
        let text = if splits.tab == Tab::Important {
            EMPTY_IMPORTANT
        } else {
            EMPTY_OTHER
        };
        let width = EMPTY_WIDTH.min(inner.width()) - 2.0 * EMPTY_PADDING.x;
        let mut y = list_top + EMPTY_PADDING.y + 3.0;
        for line in EMPTY.wrap(&ctx, text, width) {
            let left = inner.center().x - EMPTY.width(&ctx, &line) / 2.0;
            EMPTY.paint_line(
                painter,
                pos2(left, y),
                &line,
                Color32::from_black_alpha(128),
                None,
            );
            y += EMPTY.line_height;
        }
        return clicked;
    }
    let enter = entered(now, splits.tab_since, 0.0, ENTER);
    let mut rows_painter = painter.clone();
    rows_painter.set_opacity(painter.opacity() * enter);
    let lift = ROW_ENTER * (1.0 - enter);
    let from_width = inner.width() * FROM_SHARE;
    let row_height = 2.0 * THREAD_PADDING + FROM.line_height;
    for (index, thread) in threads.iter().enumerate() {
        let top = list_top + index as f32 * row_height - lift + THREAD_PADDING;
        let date = format_date(thread, zone);
        let date_width = DATE.width(&ctx, &date);
        FROM.paint_line(
            &rows_painter,
            pos2(inner.left(), top + 1.0),
            &thread.sender,
            Color32::from_black_alpha(217),
            Some(from_width - CELL_GAP),
        );
        let subject_left = inner.left() + from_width;
        let subject_width = inner.right() - date_width - CELL_GAP - subject_left;
        SUBJECT.paint_line(
            &rows_painter,
            pos2(subject_left, top + 1.0),
            &thread.subject,
            Color32::from_black_alpha(128),
            Some(subject_width),
        );
        // On the row's baseline.
        let baseline = top + 1.0 + FROM.baseline(&ctx);
        DATE.paint_line(
            &rows_painter,
            pos2(inner.right() - date_width, baseline - DATE.baseline(&ctx)),
            &date,
            Color32::from_black_alpha(102),
            None,
        );
    }
    clicked
}

/// The day and the month's short name in capitals (`17 NOV`).
fn format_date(thread: &PreviewEmail, zone: &TimeZone) -> String {
    let date = thread.date.to_zoned(zone.clone());
    format!(
        "{} {}",
        date.day(),
        date.strftime("%b").to_string().to_uppercase()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_goes_round_setups_own_tabindexes_and_stops_at_the_button() {
        let active = Tab::Split(1);
        assert_eq!(
            tab_from(Some(Focus::Toggle(0)), active, false),
            Some(Focus::ButtonRegion),
            "a switch is in the content"
        );
        assert_eq!(
            tab_from(Some(Focus::Content), active, true),
            Some(Focus::Tabpanel),
            "round"
        );
        assert_eq!(
            tab_from(Some(Focus::Tabpanel), active, true),
            Some(Focus::Tab(active))
        );
        assert_eq!(
            tab_from(Some(Focus::Tab(active)), active, false),
            Some(Focus::Tabpanel)
        );
        assert_eq!(tab_from(None, active, false), Some(Focus::Content));
        for stuck in [Focus::Continue, Focus::ButtonRegion] {
            assert_eq!(
                tab_from(Some(stuck), active, false),
                Some(stuck),
                "its region has only the button"
            );
        }
    }
}
