//! The week view: over everything but the rail, a top bar (Today, the chevrons, the month), the
//! week's grid, and a sidebar with the mini month, Meet, and each calendar account's calendars.

use std::collections::BTreeSet;

use egui::{Color32, Id, Order, Rect, Sense, Stroke, Ui, UiBuilder, Vec2, pos2, vec2};
use exo_ui::{CalendarColor, Family, Font, IconButton, KeyTooltip, TextStyle};
use jiff::{ToSpan, civil::Date};

use super::{
    grid::{Grid, GridAction, GridView, Variant},
    meet::{Meet, MeetAction},
    month::{self, MonthAction},
    share::{Share, ShareAction},
};

/// The sidebar: 288px, overlapping the calendar by its 1px border.
const SIDEBAR: f32 = 288.0;
/// The top bar: `margin-top: 24px; margin-left: 58px`, 32px tall.
const TOP_BAR_TOP: f32 = 24.0;
const TOP_BAR_LEFT: f32 = 58.0;
const TOP_BAR_HEIGHT: f32 = 32.0;
/// Today and Create: small secondary buttons 30px tall, 1px down the bar, a 1px inset ring,
/// radius 8px, their 12px/18px text 6px down between 10px sides; Create's 16px plus 8px in, 7px
/// down and 4px before its text.
const BUTTON: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 18.0);
const BUTTON_TOP: f32 = 1.0;
const BUTTON_HEIGHT: f32 = 30.0;
const BUTTON_PADDING_X: f32 = 10.0;
const BUTTON_TEXT_TOP: f32 = 6.0;
const PLUS: f32 = 16.0;
const PLUS_LEFT: f32 = 8.0;
const PLUS_TOP: f32 = 7.0;
const PLUS_GAP: f32 = 4.0;
/// Create's `margin-right: 24px`.
const CREATE_RIGHT: f32 = 24.0;
/// 32px after Today and after the chevrons; the chevrons (8×15) 24px apart, 8px down, with 4px
/// of hit slop around each.
const GAP: f32 = 32.0;
const CHEVRON: f32 = 16.0;
const CHEVRON_GAP: f32 = 24.0;
const CHEVRON_TOP: f32 = 8.0;
/// The month: 21px/32px nudged `top: 3px`.
const DATE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 21.0, 32.0);
const DATE_TOP: f32 = 3.0;
/// The sidebar's content: `padding: 32px 18px 28px`, sections 28px apart; the mini month sits
/// 14px out to the left.
const SIDEBAR_PADDING_TOP: f32 = 32.0;
const SIDEBAR_PADDING_X: f32 = 18.0;
const SECTION_GAP: f32 = 28.0;
const MONTH_LEFT: f32 = -14.0;
/// Section headers: a 16px icon, then a 12px/16px weight-600 `h2` 24px in, nudged `top: 1px`.
const HEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 12.0, 16.0);
const HEADER_TEXT_LEFT: f32 = 24.0;
/// The account header 36px below the section's, its address 12px bold; calendars from 28px
/// below that, every 28px: a 14px checkbox and the name 22px in, 12px/16px. The next account's
/// header comes 36px after the last calendar's row (`margin-top: 20px`).
const ACCOUNT_TOP: f32 = 36.0;
const ACCOUNT_GAP: f32 = 36.0;
/// The Calendars header's 12px `+` at its right, `#bfc8d2` (Carbon `#555b64`) like its calendar
/// icon, both `#929dac` while the header is hovered.
const ADD: f32 = 12.0;
/// A signed-out account's warning: a 14px circle at the header's right in `#e5675c`, its
/// `Signed out` tooltip 14px left of it.
const DEAUTH: f32 = 14.0;
const DEAUTH_INK: Color32 = Color32::from_rgb(0xe5, 0x67, 0x5c);
/// The headers' menus: their top 8px under the header, their right 16px past its right; `Sign in
/// with Google`'s 165px wide at least, the signed-out menu's 94px plus its padding.
const MENU_BELOW: f32 = 8.0;
const MENU_RIGHT: f32 = 16.0;
const ADD_MENU_WIDTH: f32 = 165.0;
const SIGNED_OUT_MENU_WIDTH: f32 = 126.0;
/// Loading (`margin-top: 4px`): `Loading calendars` (12px/16px nudged `top: 1px`, black or white
/// at `.5`), then 8px after it a 12px spinner (`#bfbfbf`, both themes) 4px down the row; the
/// next account's header 56px under this one's.
const LOADING_TOP: f32 = 16.0 + 4.0;
const LOADING_GAP: f32 = 8.0;
const LOADING_SPINNER: f32 = 12.0;
const LOADING_SPINNER_TOP: f32 = 4.0;
const LOADING_INK: Color32 = Color32::from_rgb(0xbf, 0xbf, 0xbf);
const LOADING_NEXT: f32 = 56.0;
/// The Calendars header's menu: `Sign in with Google` alone, as Exo is Gmail-only.
const ADD_ITEMS: [&str; 1] = ["Sign in with Google"];
const SIGNED_OUT_ITEMS: [&str; 2] = ["Sign in again", "Remove account"];
const ACCOUNT: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 16.0);
const ROW_PITCH: f32 = 28.0;
const ROW_NAME_LEFT: f32 = 22.0;
const ROW: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
/// The footer's 28px gear, 12px from the bottom, `padding: 0 18px` with `margin-right: -6px`.
const FOOTER_BUTTON: f32 = 28.0;
const FOOTER_BOTTOM: f32 = 12.0;
/// The footer bar, 58px with its 28px buttons, under the content, which scrolls above it with
/// `padding-bottom: 28px`; while it can, a 1px line tops the footer 18px in from either side
/// (`box-shadow: 0 -1px 0 0`, black at `.05`, Carbon white at `.08`).
const FOOTER: f32 = 58.0;
const SIDEBAR_PADDING_BOTTOM: f32 = 28.0;

/// A calendar account in the sidebar's list.
pub struct ListedAccount {
    pub email: String,
    pub state: AccountState,
}

/// What a calendar account's place in the list shows.
pub enum AccountState {
    /// Its calendars.
    Calendars(Vec<ListedCalendar>),
    /// `Loading calendars`: it just signed in.
    Loading,
    /// Google signed it out: its header, a warning, and a menu.
    SignedOut,
}

/// A calendar in the sidebar's list.
pub struct ListedCalendar {
    /// The calendar account it's listed under.
    pub account: String,
    pub id: String,
    pub name: String,
    pub color: CalendarColor,
    pub shown: bool,
}

/// A calendar row's `…` button in Classic: 12px.
const MENU_BUTTON: f32 = 12.0;

/// What the week view shows besides the grid.
pub struct Page<'a> {
    pub grid: Grid<'a>,
    /// The week's first day, which names its months whatever days show.
    pub week_start: Date,
    /// Weeks start on Monday.
    pub monday: bool,
    /// The mail account.
    pub account: &'a str,
    /// The calendar accounts and their calendars, in the list's order.
    pub accounts: &'a [ListedAccount],
    /// The week's events are being fetched and none are in yet.
    pub loading: bool,
    /// Who Meet suggests.
    pub contacts: Option<&'a crate::compose::contacts::Index>,
    /// Meet's guests' markers, by address.
    pub markers: &'a std::collections::BTreeMap<String, super::guests::Marker>,
    /// Meet's `Hit enter to instantly find times!` may show.
    pub find_time_hint: bool,
    /// The calendar accounts whose calendars are folded away.
    pub folded: &'a BTreeSet<String>,
}

/// What the user did in the week view.
#[derive(Clone, Debug, PartialEq)]
pub enum WeekAction {
    /// Today, or the chevrons: this week, or one week back (-1) or on (1).
    Today,
    Step(i64),
    /// Create: a new event in the week.
    Create,
    /// A day in the mini month.
    Select(Date),
    /// A calendar's checkbox.
    ToggleCalendar(String),
    /// A colour picked for calendar `id` from its `…`.
    Recolor(String, CalendarColor),
    /// The Calendars header's `Sign in with Google`.
    AddAccount,
    /// A signed-out account's `Sign in again`.
    SignInAgain(String),
    /// A signed-out account's `Remove account`.
    RemoveAccount(String),
    /// An account's header or caret: its calendars folded away, or back.
    Fold(String),
    /// The footer's gear.
    Settings,
    Grid(GridAction),
    Meet(MeetAction),
    Share(ShareAction),
}

/// The week view's state.
pub struct Week {
    /// The day the week is around.
    pub active: Date,
    grid: GridView,
    /// The mini month's month.
    month: Date,
    /// How far the sidebar's content is scrolled, how tall it was last drawn, and its scrollbar.
    scroll: f32,
    content: f32,
    scrollbar: exo_ui::Scrollbar,
    /// Meet with and Find Time.
    pub meet: Meet,
    /// Share Availability, when the week is picking free times.
    pub share: Option<Share>,
    /// The calendar whose `…` opened its colour picker, and the colour it had then (Escape puts
    /// it back).
    pub picker: Option<(String, CalendarColor)>,
    /// The Calendars header's menu, or a signed-out account's, while it's open, and its
    /// highlighted row.
    pub account_menu: Option<(AccountMenu, usize)>,
}

/// A menu of the calendar account list's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountMenu {
    /// The Calendars header's, from its `+`.
    Add,
    /// Signed-out account `email`'s.
    SignedOut(String),
}

impl Week {
    /// The week view around `active`.
    pub fn new(active: Date) -> Self {
        Self {
            active,
            grid: GridView::default(),
            month: active.first_of_month(),
            scroll: 0.0,
            content: 0.0,
            scrollbar: exo_ui::Scrollbar::default(),
            meet: Meet::default(),
            share: None,
            picker: None,
            account_menu: None,
        }
    }

    /// Where the form's draft was drawn last.
    pub fn draft_anchor(&self) -> Option<Rect> {
        self.grid.draft_anchor
    }

    /// Whether something is being dragged in the grid.
    pub fn dragging(&self) -> bool {
        self.grid.dragging()
    }

    /// Drops the grid's drag.
    pub fn cancel_drag(&mut self) {
        self.grid.cancel_drag();
    }

    /// Opens event `key`'s details once the grid draws it, scrolled so its end at `end_hour` is
    /// in view if the grid hasn't scrolled yet.
    pub fn select(&mut self, key: String, end_hour: i8) {
        self.grid.reveal(end_hour);
        self.grid.select(key);
    }

    /// Moves to `date`'s week, the mini month with it.
    pub fn go_to(&mut self, date: Date) {
        self.active = date;
        self.month = date.first_of_month();
    }

    /// Paints the page over `area` (the window right of the rail, under the tab strip) on its
    /// own layer, which takes the pointer from what's under it.
    pub fn show(&mut self, ui: &Ui, area: Rect, page: &Page<'_>, compose: bool) -> Vec<WeekAction> {
        let mut actions = Vec::new();
        egui::Area::new(exo_ui::id(ui.ctx(), "calendar-week"))
            .order(Order::Middle)
            .fixed_pos(area.min)
            .constrain(false)
            .show(ui.ctx(), |ui| {
                // The layer is the page's size, so the pointer anywhere on it is over it (the
                // sidebar's wheel).
                ui.expand_to_include_rect(area);
                let mut ui = ui.new_child(UiBuilder::new().max_rect(area));
                let ui = &mut ui;
                ui.set_clip_rect(area);
                // Swallow the pointer everywhere on the page.
                let _ = ui.interact(
                    area,
                    exo_ui::id(ui.ctx(), "calendar-week-page"),
                    Sense::click(),
                );
                let palette = exo_ui::palette(ui.ctx());
                ui.painter()
                    .rect_filled(area, 0.0, palette.background_base_default);
                let calendar =
                    Rect::from_min_max(area.min, pos2(area.right() - SIDEBAR + 1.0, area.bottom()));
                self.top_bar(ui, calendar, page, compose, &mut actions);
                let grid_rect = Rect::from_min_max(
                    pos2(
                        calendar.left(),
                        calendar.top() + TOP_BAR_TOP + TOP_BAR_HEIGHT,
                    ),
                    calendar.max,
                );
                actions.extend(
                    self.grid
                        .show(ui, grid_rect, 0.0, &page.grid, Variant::Week)
                        .into_iter()
                        .map(WeekAction::Grid),
                );
                let sidebar =
                    Rect::from_min_max(pos2(area.right() - SIDEBAR, area.top()), area.max);
                match &mut self.share {
                    Some(share) => {
                        let first = page
                            .accounts
                            .iter()
                            .find_map(|account| match &account.state {
                                AccountState::Calendars(calendars) => calendars.first(),
                                _ => None,
                            });
                        let color = first.map_or(CalendarColor::BLUE, |calendar| calendar.color);
                        actions.extend(
                            share
                                .show(ui, sidebar, page.grid.now, color, page.contacts)
                                .into_iter()
                                .map(WeekAction::Share),
                        );
                    }
                    None => self.sidebar(ui, sidebar, page, &mut actions),
                }
                if page.loading {
                    paint_spinner(ui);
                }
            });
        actions
    }

    /// The top bar: Today, the chevrons, and the week's month.
    fn top_bar(
        &self,
        ui: &Ui,
        calendar: Rect,
        page: &Page<'_>,
        compose: bool,
        actions: &mut Vec<WeekAction>,
    ) {
        let palette = exo_ui::palette(ui.ctx());
        let painter = ui.painter();
        let top = calendar.top() + TOP_BAR_TOP;
        let mut x = calendar.left() + TOP_BAR_LEFT;

        let response = origin_button(
            ui,
            exo_ui::id(ui.ctx(), "calendar-week-today"),
            pos2(x, top + BUTTON_TOP),
            "Today",
            false,
        );
        let button = response.rect;
        let keys: &[&str] = if compose {
            &["⌘", "shift", "0"]
        } else {
            &["t"]
        };
        KeyTooltip {
            text: "Today",
            keys,
        }
        .show_below(ui, &response, 8.0);
        if response.clicked() {
            actions.push(WeekAction::Today);
        }
        x = button.right() + GAP;

        let chevrons = [
            (
                -1,
                "legacy/DEPRECATED_leftChevronLarge",
                "Previous Week",
                "–",
            ),
            (1, "legacy/DEPRECATED_rightChevronLarge", "Next Week", "="),
        ];
        for (step, icon, text, key) in chevrons {
            let size = exo_ui::icon_size(ui.ctx(), icon).unwrap_or(Vec2::splat(CHEVRON));
            let icon_rect = Rect::from_min_size(pos2(x, top + CHEVRON_TOP), size);
            // `::before { inset: -4px }`: the hit area reaches 4px past the icon.
            let response = IconButton::new(icon).show(
                ui,
                exo_ui::id(ui.ctx(), ("calendar-week-chevron", step)),
                icon_rect.expand(4.0),
                icon_rect,
            );
            let keys: &[&str] = if compose {
                &["⌘", "shift", key]
            } else {
                &[key]
            };
            KeyTooltip { text, keys }.show_below(ui, &response, 8.0);
            if response.clicked() {
                actions.push(WeekAction::Step(step));
            }
            x = icon_rect.right() + if step < 0 { CHEVRON_GAP } else { GAP };
        }
        DATE.paint_line(
            painter,
            pos2(x, top + DATE_TOP),
            &date_display(page.week_start),
            palette.text_base_default,
            None,
        );

        let width = origin_button_width(ui.ctx(), "Create", true);
        let create = origin_button(
            ui,
            exo_ui::id(ui.ctx(), "calendar-week-create"),
            pos2(calendar.right() - CREATE_RIGHT - width, top + BUTTON_TOP),
            "Create",
            true,
        );
        KeyTooltip {
            text: "Create Event",
            keys: &["b"],
        }
        .show_below(ui, &create, 8.0);
        if create.clicked() {
            actions.push(WeekAction::Create);
        }
    }
}

/// A small secondary button's width: its text between 10px sides, after the plus.
fn origin_button_width(ctx: &egui::Context, text: &str, plus: bool) -> f32 {
    BUTTON.width(ctx, text)
        + 2.0 * BUTTON_PADDING_X
        + if plus {
            PLUS_LEFT + PLUS + PLUS_GAP - BUTTON_PADDING_X
        } else {
            0.0
        }
}

/// A small secondary button at `at`: Classic's black at `.7` in a `#dedbd5` ring, `#f2f0eb`
/// hovered with the text `#141413`; Carbon's white at `.7` in a `#474543` ring, filled with it
/// hovered.
fn origin_button(ui: &Ui, id: Id, at: egui::Pos2, text: &str, plus: bool) -> egui::Response {
    let ctx = ui.ctx();
    let palette = exo_ui::palette(ctx);
    let carbon = palette.theme == exo_ui::Theme::Carbon;
    let button = Rect::from_min_size(
        at,
        vec2(origin_button_width(ctx, text, plus), BUTTON_HEIGHT),
    );
    let response = ui
        .interact(button, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let ring = if carbon {
        egui::Color32::from_rgb(0x47, 0x45, 0x43)
    } else {
        egui::Color32::from_rgb(0xde, 0xdb, 0xd5)
    };
    let ink = match (response.hovered(), carbon) {
        (true, false) => egui::Color32::from_rgb(0x14, 0x14, 0x13),
        _ => palette.ink(0.7, 0.7),
    };
    let painter = ui.painter();
    let snapped = exo_ui::pixel_snap(button);
    if response.hovered() {
        painter.rect_filled(
            snapped,
            8.0,
            if carbon {
                ring
            } else {
                egui::Color32::from_rgb(0xf2, 0xf0, 0xeb)
            },
        );
    }
    painter.rect_stroke(
        snapped,
        8.0,
        Stroke::new(1.0, ring),
        egui::StrokeKind::Inside,
    );
    let mut x = button.left() + BUTTON_PADDING_X;
    if plus {
        let icon = Rect::from_min_size(
            pos2(button.left() + PLUS_LEFT, button.top() + PLUS_TOP),
            Vec2::splat(PLUS),
        );
        exo_ui::paint_icon(ui, "origin/Plus", icon, ink);
        x = icon.right() + PLUS_GAP;
    }
    BUTTON.paint_line(
        painter,
        pos2(x, button.top() + BUTTON_TEXT_TOP),
        text,
        ink,
        None,
    );
    response
}

impl Week {
    /// The sidebar: the mini month, Meet and the calendar accounts' calendars, scrolling once
    /// they outgrow it, over the footer and its gear.
    fn sidebar(
        &mut self,
        ui: &mut Ui,
        sidebar: Rect,
        page: &Page<'_>,
        actions: &mut Vec<WeekAction>,
    ) {
        let palette = exo_ui::palette(ui.ctx());
        let painter = ui.painter();
        painter.rect_filled(sidebar, 0.0, palette.background_base_accent);
        // `border-left: 1px solid rgba(0,0,0,.09)`, a Classic literal both themes keep.
        painter.vline(
            sidebar.left() + 0.5,
            sidebar.y_range(),
            Stroke::new(1.0, Color32::from_black_alpha(23)),
        );
        let footer = sidebar.bottom() - FOOTER;
        let viewport = Rect::from_min_max(
            pos2(sidebar.left() + 1.0, sidebar.top()),
            pos2(sidebar.right(), footer),
        );
        let overflow = (self.content - viewport.height()).max(0.0);
        if ui.rect_contains_pointer(viewport) {
            self.scroll -= ui.input(|input| input.smooth_scroll_delta.y);
        }
        self.scroll = self.scroll.clamp(0.0, overflow);
        let mut content = ui.new_child(UiBuilder::new().max_rect(viewport));
        content.set_clip_rect(viewport.intersect(ui.clip_rect()));
        let bottom = self.sidebar_content(&mut content, viewport, page, actions);
        self.content = bottom + self.scroll - viewport.top();
        self.scrollbar.show(
            ui,
            exo_ui::id(ui.ctx(), "calendar-week-sidebar-scrollbar"),
            viewport,
            self.scroll,
            self.content,
        );
        if overflow > 0.0 {
            let line = palette.ink(0.05, 0.08);
            ui.painter().hline(
                viewport.left() + SIDEBAR_PADDING_X..=sidebar.right() - SIDEBAR_PADDING_X,
                footer - 0.5,
                Stroke::new(1.0, line),
            );
        }

        let gear = Rect::from_min_size(
            pos2(
                sidebar.right() - SIDEBAR_PADDING_X + 6.0 - FOOTER_BUTTON,
                sidebar.bottom() - FOOTER_BOTTOM - FOOTER_BUTTON,
            ),
            Vec2::splat(FOOTER_BUTTON),
        );
        let icon = "legacy/DEPRECATED_settings";
        if IconButton::new(icon)
            .show(
                ui,
                exo_ui::id(ui.ctx(), "calendar-week-gear"),
                gear,
                exo_ui::icon_rect(ui.ctx(), icon, gear),
            )
            .clicked()
        {
            actions.push(WeekAction::Settings);
        }
    }

    /// The sidebar's scrolling content in `viewport`: the mini month, Meet, then each calendar
    /// account and its calendars. Returns the bottom of its padding.
    fn sidebar_content(
        &mut self,
        ui: &mut Ui,
        viewport: Rect,
        page: &Page<'_>,
        actions: &mut Vec<WeekAction>,
    ) -> f32 {
        let palette = exo_ui::palette(ui.ctx());
        let left = viewport.left() + SIDEBAR_PADDING_X;
        let mut top = viewport.top() + SIDEBAR_PADDING_TOP - self.scroll;

        match month::show(
            ui,
            pos2(left + MONTH_LEFT, top),
            self.month,
            self.active,
            page.grid.now.date(),
            page.monday,
        ) {
            Some(MonthAction::Month(step)) => self.month = month::shift(self.month, step),
            Some(MonthAction::Select(day)) => actions.push(WeekAction::Select(day)),
            None => {}
        }
        top += month::HEIGHT + SECTION_GAP;

        // Meet with.
        let right = viewport.right() - SIDEBAR_PADDING_X;
        let column = Rect::from_min_max(pos2(left, top), pos2(right, top));
        actions.extend(
            self.meet
                .show(
                    ui,
                    column,
                    page.contacts,
                    page.account,
                    page.markers,
                    page.find_time_hint,
                )
                .into_iter()
                .map(WeekAction::Meet),
        );
        top += self.meet.height() + SECTION_GAP;
        let painter = ui.painter().clone();
        let painter = &painter;

        // The calendar accounts: the header and its `+`, then each account.
        let header = Rect::from_min_max(pos2(left, top), pos2(right, top + 16.0));
        let header_hit = ui
            .interact(
                header,
                exo_ui::id(ui.ctx(), "calendar-week-accounts-header"),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        // The header's icons: `#bfc8d2`, Carbon `#555b64`; hovered `#929dac`.
        let disc = match (header_hit.hovered(), exo_ui::theme(ui.ctx())) {
            (true, _) => Color32::from_rgb(0x92, 0x9d, 0xac),
            (false, exo_ui::Theme::Snow) => Color32::from_rgb(0xbf, 0xc8, 0xd2),
            (false, exo_ui::Theme::Carbon) => Color32::from_rgb(0x55, 0x5b, 0x64),
        };
        exo_ui::paint_icon(
            ui,
            "legacy/DEPRECATED_calendarHeader",
            Rect::from_min_size(pos2(left, top), Vec2::splat(16.0)),
            disc,
        );
        // The section's header: black (Carbon white) at `.8`.
        HEADER.paint_line(
            painter,
            pos2(left + HEADER_TEXT_LEFT, top + 1.0),
            "Calendars",
            palette.ink(0.8, 0.8),
            None,
        );
        exo_ui::paint_icon(
            ui,
            "legacy/DEPRECATED_plus",
            Rect::from_min_size(pos2(right - ADD, top), Vec2::splat(ADD)),
            disc,
        );
        if header_hit.clicked() {
            self.toggle_account_menu(AccountMenu::Add);
        }
        if self
            .account_menu
            .as_ref()
            .is_some_and(|(menu, _)| *menu == AccountMenu::Add)
        {
            self.account_menu_list(ui, header, header_hit.hovered(), actions);
        }
        // The accounts' headers: black (Carbon white) at `.8`.
        let title_ink = palette.ink(0.8, 0.8);
        let mut header_top = top + ACCOUNT_TOP;
        // The bottom of what's drawn last.
        let mut end = header.bottom();
        for account in page.accounts {
            top = header_top;
            end = top + 16.0;
            let email = account.email.as_str();
            let row = Rect::from_min_max(pos2(left, top), pos2(right, top + 16.0));
            let hit = ui
                .interact(
                    row,
                    exo_ui::id(ui.ctx(), ("calendar-week-account", email)),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let calendars = match &account.state {
                AccountState::SignedOut => {
                    ACCOUNT.paint_line(
                        painter,
                        pos2(left, top + 1.0),
                        email,
                        title_ink,
                        Some(right - left - DEAUTH - 8.0),
                    );
                    let icon = Rect::from_min_size(pos2(right - DEAUTH, top), Vec2::splat(DEAUTH));
                    exo_ui::paint_icon(ui, "legacy/DEPRECATED_warningCircle", icon, DEAUTH_INK);
                    let warning = ui.interact(
                        Rect::from_min_size(icon.min, vec2(DEAUTH, 16.0)),
                        exo_ui::id(ui.ctx(), ("calendar-week-account-signed-out", email)),
                        Sense::click(),
                    );
                    KeyTooltip {
                        text: "Signed out",
                        keys: &[],
                    }
                    .show_at(ui, &warning, exo_ui::Placement::Left(14.0));
                    let menu = AccountMenu::SignedOut(email.to_owned());
                    if hit.clicked() || warning.clicked() {
                        self.toggle_account_menu(menu.clone());
                    }
                    if self
                        .account_menu
                        .as_ref()
                        .is_some_and(|(open, _)| *open == menu)
                    {
                        self.account_menu_list(
                            ui,
                            row,
                            hit.hovered() || warning.hovered(),
                            actions,
                        );
                    }
                    header_top = top + ACCOUNT_GAP;
                    continue;
                }
                AccountState::Loading => None,
                AccountState::Calendars(calendars) => Some(calendars),
            };
            let folded = page.folded.contains(email);
            ACCOUNT.paint_line(
                painter,
                pos2(left, top + 1.0),
                email,
                title_ink,
                Some(right - left - 24.0),
            );
            // The caret's 12px box at the header's right.
            let fold = Rect::from_min_size(pos2(right - 14.0, top), Vec2::splat(16.0));
            // The caret: the 14×8 glyph in a box 12px wide (`width: 12px`, its `height="8"` kept),
            // which it fits inside, 1px under the box's centre; turned over while folded
            // (`rotate(180deg)`, then 2px down), its ink where the upright one's is. A box as tall
            // as the fitted caret would round to 6px and shrink it.
            let icon = "legacy/DEPRECATED_dropdownLarge";
            let glyph = Rect::from_center_size(fold.center(), vec2(12.0, 8.0))
                .translate(vec2(0.0, if folded { 2.0 } else { 1.0 }));
            // Its stroke: black (Carbon white) at `.3`, `.5` hovered.
            let caret = IconButton::new(icon)
                .colors(palette.ink(0.3, 0.3), palette.ink(0.5, 0.5))
                .upside_down(folded)
                .show(
                    ui,
                    exo_ui::id(ui.ctx(), ("calendar-week-fold", email)),
                    fold,
                    glyph,
                );
            if hit.clicked() || caret.clicked() {
                actions.push(WeekAction::Fold(email.to_owned()));
            }
            if folded {
                header_top = top + ACCOUNT_GAP;
                continue;
            }
            let Some(calendars) = calendars else {
                // Loading: its text, then the spinner.
                let row = top + LOADING_TOP;
                let width = ROW.paint_line(
                    painter,
                    pos2(left, row + 1.0),
                    "Loading calendars",
                    palette.ink(0.5, 0.5),
                    None,
                );
                let spinner = Rect::from_min_size(
                    pos2(left + width + LOADING_GAP, row + LOADING_SPINNER_TOP),
                    Vec2::splat(LOADING_SPINNER),
                );
                exo_ui::paint_spinner(painter, spinner, ui.input(|input| input.time), LOADING_INK);
                end = row + ROW.line_height;
                header_top = top + LOADING_NEXT;
                continue;
            };
            for calendar in calendars {
                top += ROW_PITCH;
                let row = Rect::from_min_max(pos2(left, top), pos2(right, top + ROW.line_height));
                let response = ui
                    .interact(
                        Rect::from_min_max(
                            row.min,
                            pos2(
                                left + ROW_NAME_LEFT + ROW.width(ui.ctx(), &calendar.name),
                                row.bottom(),
                            ),
                        ),
                        exo_ui::id(ui.ctx(), ("calendar-week-row", &calendar.id)),
                        Sense::click(),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                let theme = exo_ui::theme(ui.ctx());
                // The tick is cut out of the swatch: the sidebar shows through, in Carbon too.
                exo_ui::paint_tinted_checkbox(
                    painter,
                    pos2(left, top + 1.0),
                    calendar.shown,
                    calendar.color.swatch(theme),
                    palette.background_base_accent,
                    palette,
                );
                // The name: `top: 1px`, black (Carbon white) at `.8`.
                ROW.paint_line(
                    painter,
                    pos2(left + ROW_NAME_LEFT, top + 1.0),
                    &calendar.name,
                    palette.ink(0.8, 0.8),
                    Some(right - left - ROW_NAME_LEFT - 24.0),
                );
                if response.clicked() {
                    actions.push(WeekAction::ToggleCalendar(calendar.id.clone()));
                }
                self.calendar_menu(ui, row, calendar, actions);
                end = row.bottom();
            }
            header_top = top + ACCOUNT_GAP;
        }
        end + SIDEBAR_PADDING_BOTTOM
    }
}

impl Week {
    /// Opens `menu`, or closes it when it's the one open.
    fn toggle_account_menu(&mut self, menu: AccountMenu) {
        self.account_menu = if self
            .account_menu
            .as_ref()
            .is_some_and(|(open, _)| *open == menu)
        {
            None
        } else {
            Some((menu, 0))
        };
    }

    /// The open account menu, under `header` (which opens and closes it, `on_trigger` while the
    /// pointer is on it): a pick, a hover, or a click elsewhere, which closes it.
    fn account_menu_list(
        &mut self,
        ui: &Ui,
        header: Rect,
        on_trigger: bool,
        actions: &mut Vec<WeekAction>,
    ) {
        let Some((menu, highlighted)) = self.account_menu.clone() else {
            return;
        };
        let (items, min_width): (&[&str], f32) = match menu {
            AccountMenu::Add => (&ADD_ITEMS, ADD_MENU_WIDTH),
            AccountMenu::SignedOut(_) => (&SIGNED_OUT_ITEMS, SIGNED_OUT_MENU_WIDTH),
        };
        let list = exo_ui::Menu { items, highlighted };
        let width = list.size(ui.ctx()).x.max(min_width);
        let shown = list.show_at_least(
            ui,
            exo_ui::id(ui.ctx(), "calendar-week-account-menu"),
            pos2(
                header.right() + MENU_RIGHT - width,
                header.bottom() + MENU_BELOW,
            ),
            min_width,
        );
        if let Some(picked) = shown.clicked {
            self.account_menu = None;
            actions.push(match (menu, picked) {
                (AccountMenu::Add, _) => WeekAction::AddAccount,
                (AccountMenu::SignedOut(email), 0) => WeekAction::SignInAgain(email),
                (AccountMenu::SignedOut(email), _) => WeekAction::RemoveAccount(email),
            });
        } else if let Some(hovered) = shown.hovered {
            self.account_menu = Some((menu, hovered));
        } else if ui.input(|input| input.pointer.any_released())
            && !on_trigger
            && !ui.rect_contains_pointer(shown.rect)
        {
            self.account_menu = None;
        }
    }

    /// A calendar row's `…` (12px at the row's right, 2px down, shown while the row is hovered or
    /// its picker is open, its dots at `.3`, `.5` hovered, a 6px reach around it) and its colour
    /// picker: five rows of the 25 colours, tethered by its top right 16px right of the button's
    /// bottom right, 8px under it.
    fn calendar_menu(
        &mut self,
        ui: &mut Ui,
        row: Rect,
        calendar: &ListedCalendar,
        actions: &mut Vec<WeekAction>,
    ) {
        let open = self
            .picker
            .as_ref()
            .is_some_and(|(id, _)| *id == calendar.id);
        let button = Rect::from_min_size(
            pos2(row.right() - MENU_BUTTON, row.top() + 2.0),
            Vec2::splat(MENU_BUTTON),
        );
        let reach = ui
            .interact(
                button.expand(6.0),
                exo_ui::id(ui.ctx(), ("calendar-week-row-menu", &calendar.id)),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if open || ui.rect_contains_pointer(row.expand2(vec2(0.0, 6.0))) || reach.hovered() {
            let palette = exo_ui::palette(ui.ctx());
            let ink = if reach.hovered() {
                palette.ink(0.5, 0.5)
            } else {
                palette.ink(0.3, 0.3)
            };
            exo_ui::paint_icon(
                ui,
                "legacy/DEPRECATED_ellipses",
                exo_ui::icon_rect(ui.ctx(), "legacy/DEPRECATED_ellipses", button),
                ink,
            );
        }
        if reach.clicked() {
            self.picker = if open {
                None
            } else {
                Some((calendar.id.clone(), calendar.color))
            };
        }
        if !open || reach.clicked() {
            return;
        }
        let colors: Vec<CalendarColor> = CalendarColor::picker().collect();
        let bases: Vec<Color32> = colors.iter().map(|color| color.base()).collect();
        let picker = exo_ui::ColorPicker {
            colors: &bases,
            per_row: super::PICKER_ROW,
            chosen: colors.iter().position(|color| *color == calendar.color),
            padding_bottom: 3.0,
        };
        let size = picker.size();
        let at = pos2(button.right() + 16.0 - size.x, button.bottom() + 8.0);
        match picker.show(
            ui.ctx(),
            exo_ui::id(ui.ctx(), "calendar-color-picker"),
            at,
            button.expand(6.0),
        ) {
            exo_ui::Picked::Color(index) => {
                self.picker = None;
                actions.push(WeekAction::Recolor(calendar.id.clone(), colors[index]));
            }
            exo_ui::Picked::Outside => self.picker = None,
            exo_ui::Picked::Nothing => {}
        }
    }
}

/// The page's loading spinner: a 48px white disc with its top left at the window's centre (`top:
/// 50%; left: 50%`, no translate), a soft shadow, and a 16px spinner in its middle.
fn paint_spinner(ui: &Ui) {
    const SHADOWS: [exo_ui::BoxShadow; 3] = [
        exo_ui::BoxShadow::new(
            [0.0, -1.0],
            8.0,
            0.0,
            Color32::from_rgba_premultiplied(0, 0, 0, 10),
        ),
        exo_ui::BoxShadow::new(
            [0.0, 2.0],
            8.0,
            0.0,
            Color32::from_rgba_premultiplied(0, 0, 0, 13),
        ),
        exo_ui::BoxShadow::new(
            [0.0, 4.0],
            12.0,
            0.0,
            Color32::from_rgba_premultiplied(0, 0, 0, 20),
        ),
    ];
    let palette = exo_ui::palette(ui.ctx());
    let disc = Rect::from_min_size(ui.ctx().content_rect().center(), Vec2::splat(48.0));
    for shadow in SHADOWS {
        shadow.paint(ui.painter(), disc, egui::CornerRadius::same(24));
    }
    // `background: #fff`, a literal in every theme.
    ui.painter()
        .circle_filled(disc.center(), 24.0, Color32::WHITE);
    let turn = (ui.input(|input| input.time) % 1.0) as f32 * std::f32::consts::TAU;
    let points: Vec<egui::Pos2> = (0..=36)
        .map(|step| {
            let angle = turn + step as f32 / 36.0 * 1.5 * std::f32::consts::PI;
            disc.center() + 7.0 * vec2(angle.cos(), angle.sin())
        })
        .collect();
    ui.painter().add(egui::Shape::line(
        points,
        Stroke::new(2.0, palette.icon_base_default),
    ));
    ui.ctx().request_repaint();
}

/// The top bar's month: `November 2026`, `Nov – Dec 2026`, or `Dec 2026 – Jan 2027` for the 7
/// days from `start`.
pub fn date_display(start: Date) -> String {
    let end = start + 6.days();
    if start.year() != end.year() {
        format!(
            "{} {} – {} {}",
            start.strftime("%b"),
            start.year(),
            end.strftime("%b"),
            end.year()
        )
    } else if start.month() != end.month() {
        format!(
            "{} – {} {}",
            start.strftime("%b"),
            end.strftime("%b"),
            start.year()
        )
    } else {
        start.strftime("%B %Y").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn the_week_names_its_months() {
        assert_eq!(date_display(date(2026, 11, 15)), "November 2026");
        assert_eq!(date_display(date(2026, 11, 29)), "Nov – Dec 2026");
        assert_eq!(date_display(date(2026, 12, 27)), "Dec 2026 – Jan 2027");
    }
}
