//! The Accounts dialog: the device's accounts in their order, each with its
//! avatar, its address, its ctrl+N (with two or more accounts) and a color swatch that opens the
//! color picker; hovered, `Sign out` and (with two or more) a drag handle that reorders them.
//! Under them, `Sign in with your email provider to add an account.`
//!
//! Metrics are fixed offsets from the card rather than the rows' flex layout (negative margins,
//! `line-height: 0` wrappers) rebuilt.

use egui::{Color32, Context, CornerRadius, Rect, Sense, Ui, pos2, vec2};
use exo_ui::{Dialog, DialogResponse, Family, Font, KeycapColors, Keycaps, TextStyle, motion};

use crate::accounts::{self, COLORS};

/// What the dialog asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// `Sign out` on this account's row.
    SignOut(String),
    /// A row dragged to another place.
    Move {
        /// The account.
        email: String,
        /// Its new place.
        to: usize,
    },
    /// A color picked for this account.
    Color {
        /// The account.
        email: String,
        /// The color's name.
        color: &'static str,
    },
    /// The footer's link: sign in to another account.
    Add,
}

/// The color picker's account and a dragged row, between frames.
#[derive(Debug, Default)]
pub struct State {
    /// The row whose swatch opened the color picker, and the color it opened on.
    picker: Option<usize>,
    start: usize,
    /// The row being dragged, and how far.
    drag: Option<(usize, f32)>,
}

impl State {
    /// The color picker is open: Escape closes it, not the dialog.
    pub fn picker_open(&self) -> bool {
        self.picker.is_some()
    }

    /// What `key` does to the open color picker of `accounts`' rows: the color its account
    /// changes to at once, if any. Escape (back to the color it
    /// opened on), Enter and Space close it.
    pub fn picker_key(
        &mut self,
        key: exo_ui::PickerKey,
        accounts: &[exo_auth::Account],
    ) -> Option<Event> {
        let account = accounts.get(self.picker?)?;
        let current = color_index(account);
        let to = match exo_ui::picker_key(key, current, self.start, COLORS.len(), PER_ROW) {
            exo_ui::PickerMove::Preview(to) => to,
            exo_ui::PickerMove::Close(to) => {
                self.picker = None;
                to
            }
        };
        (to != current).then(|| Event::Color {
            email: account.email.clone(),
            color: COLORS[to].0,
        })
    }

    /// The dialog opened: no picker, nothing dragged.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Rows are 32px, 44px apart (`margin-bottom: 12px`), the first 41px below
/// the card's top; the avatar 4px into it, the address's 20px line and the caps 6px.
const FIRST_ROW: f32 = 61.0;
const ROW_STEP: f32 = 44.0;
const ROW_HEIGHT: f32 = 32.0;
const AVATAR: f32 = 24.0;
const AVATAR_TOP: f32 = 4.0;
const AVATAR_GAP: f32 = 10.0;
const LINE_TOP: f32 = 6.0;
const KEYS_TOP: f32 = 6.0;
/// The row reaches 13px left of the body (`margin-left: calc(-1*calc(10px + 3px))`) and 9px
/// right of it.
const ROW_LEFT: f32 = 13.0;
const ROW_RIGHT: f32 = 9.0;
/// The swatch: a 14px square with 2px corners, `top: -1px`, its right edge 1px past the body,
/// 12px after the caps.
const SWATCH: f32 = 14.0;
const SWATCH_TOP: f32 = 8.0;
const SWATCH_GAP: f32 = 12.0;
/// The drag handle: the 12×10 icon padded 10px, 4px before the caps, `top: -1px`.
const HANDLE: egui::Vec2 = egui::Vec2::new(12.0, 10.0);
const HANDLE_TOP: f32 = 10.0;
const HANDLE_BEFORE_KEYS: f32 = 4.0 + 10.0;
/// `Sign out`: padded 10px, 7px before the handle's box (or the swatch's).
const SIGN_OUT_BEFORE: f32 = 7.0 + 10.0;
/// `Sign out` and the handle fade in over a hovered row (`transition: opacity 150ms ease`).
const FADE: f32 = 0.15;
/// The footer's line: 71px below the card's top and a row step lower per account (the list's
/// margins, the footer's 20px padding and its `top: 7px`).
const FOOTER_TOP: f32 = 71.0;
/// The body: the rows, the footer's 40px and 4px of the list's margins, and 4px more padding
/// at the bottom than the dialog shell's (24px).
const BODY_EXTRA: f32 = 48.0;
/// A row's text: 12px/20px; the address in `rgba(0, 0, 0, .85)`.
const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
const NAME_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 217);
/// `Sign out` and the handle's fill: black at .3, .4 hovered.
const SUBDUED: [u8; 2] = [77, 102];
/// The footer: `rgba(0, 0, 0, .6)`, its link `#54acdc`.
const FOOTER_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 153);
const LINK_INK: Color32 = Color32::from_rgb(0x54, 0xac, 0xdc);
/// The 24px avatar's initial: 12px with `dy: .375em`, in black.
const INITIAL: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
/// The color picker ([`exo_ui::ColorPicker`]) under the swatch (a 10px offset less the tooltip's
/// 8px padding), two rows of six.
const PICKER_BELOW: f32 = -2.0;
/// The tooltip's transparent 8px padding over the box, which covers the rows under it.
const PICKER_PLACEMENT: f32 = 8.0;
const PER_ROW: usize = 6;

const FOOTER_LINK: &str = "Sign in with your email provider";
const FOOTER_REST: &str = "to add an account.";
const SIGN_OUT: &str = "Sign out";

/// Draws the dialog over `area`: `accounts` in their order, `keycaps` in the app's theme (the
/// dialog draws in Snow's). Returns what was clicked or dragged.
pub fn dialog(
    ctx: &Context,
    area: Rect,
    open: bool,
    state: &mut State,
    accounts: &[exo_auth::Account],
    keycaps: KeycapColors,
) -> DialogResponse<Option<Event>> {
    let body_height = ROW_STEP * accounts.len() as f32 + BODY_EXTRA;
    let mut swatch = None;
    let mut response =
        Dialog::new("accounts", "Accounts", body_height).show(ctx, area, open, |ui, body| {
            list(ui, body, state, accounts, keycaps, &mut swatch)
        });
    // The picker is a tooltip of the page's, outside the dialog's Snow-drawn card.
    if let (Some(index), Some(swatch), Some(inner)) =
        (state.picker, swatch, response.inner.as_mut())
        && let Some(account) = accounts.get(index)
    {
        match picker(ctx, swatch, account) {
            // It stays open: a click on a color doesn't close it.
            exo_ui::Picked::Color(index) => {
                *inner = Some(Event::Color {
                    email: account.email.clone(),
                    color: COLORS[index].0,
                })
            }
            exo_ui::Picked::Outside => state.picker = None,
            exo_ui::Picked::Nothing => {}
        }
    }
    response
}

/// The rows and the footer; `picker_swatch` gets the swatch the color picker opened from.
fn list(
    ui: &mut Ui,
    body: Rect,
    state: &mut State,
    accounts: &[exo_auth::Account],
    keycaps: KeycapColors,
    picker_swatch: &mut Option<Rect>,
) -> Option<Event> {
    let ctx = ui.ctx().clone();
    // The body starts under the header; the metrics are from the card's top and right.
    let (card_top, right) = (body.top() - 40.0, body.right() + 1.0);
    let columns = Columns {
        body,
        right,
        several: accounts.len() > 1,
        keycaps,
    };
    let mut event = None;
    if let Some((_, offset)) = &mut state.drag {
        *offset += ui.input(|input| input.pointer.delta().y);
    }
    // The open picker's tooltip covers what's under it, rows included.
    let covered = state.picker.map(|index| {
        let swatch = swatch_rect(right, card_top + FIRST_ROW + ROW_STEP * index as f32);
        let picker = Rect::from_min_size(
            picker_at(swatch, account_picker_size()).round(),
            account_picker_size(),
        );
        picker.with_min_y(picker.min.y - PICKER_PLACEMENT)
    });
    let last = accounts.len().saturating_sub(1);
    let dragging = state.drag.map(|(from, offset)| {
        (
            from,
            offset,
            ((from as f32 * ROW_STEP + offset) / ROW_STEP)
                .round()
                .clamp(0.0, last as f32) as usize,
        )
    });
    for (index, account) in accounts.iter().enumerate() {
        // Others make way for the dragged row (`animation: 150`).
        let slot = match dragging {
            Some((from, _, to)) if index != from && from < index && index <= to => index - 1,
            Some((from, _, to)) if index != from && to <= index && index < from => index + 1,
            _ => index,
        };
        let shift = motion::transition(
            &ctx,
            exo_ui::id(&ctx, ("account-row-shift", &account.email)),
            slot != index,
            0.15,
            motion::CubicBezier::EASE,
        ) * (slot as f32 - index as f32);
        if dragging.is_some_and(|(from, ..)| from == index) {
            continue;
        }
        let top = card_top + FIRST_ROW + ROW_STEP * (index as f32 + shift);
        let pointer_free =
            dragging.is_none() && !covered.is_some_and(|covered| ui.rect_contains_pointer(covered));
        if let Some(clicked) = row(ui, &columns, top, index, account, pointer_free, state) {
            event = Some(clicked);
        }
    }
    if let Some((from, offset, to)) = dragging {
        // The dragged copy: white with a 3px `#aeb1dd` bar on its left.
        let top = card_top + FIRST_ROW + ROW_STEP * from as f32 + offset;
        let rect = Rect::from_min_max(
            pos2(body.left() - ROW_LEFT, top),
            pos2(body.right() + ROW_RIGHT, top + ROW_HEIGHT),
        );
        let painter = ui.painter();
        exo_ui::BoxShadow::new([1.0, 1.0], 4.0, 0.0, Color32::from_black_alpha(51)).paint(
            painter,
            rect,
            CornerRadius::same(4),
        );
        painter.rect_filled(rect, CornerRadius::same(4), Color32::WHITE);
        painter.rect_filled(
            Rect::from_min_size(rect.min, vec2(3.0, ROW_HEIGHT)),
            CornerRadius {
                nw: 4,
                sw: 4,
                ne: 0,
                se: 0,
            },
            Color32::from_rgb(0xae, 0xb1, 0xdd),
        );
        paint_identity(ui, body.left(), top, &accounts[from]);
        if ui.input(|input| input.pointer.any_released()) {
            state.drag = None;
            if to != from {
                event = Some(Event::Move {
                    email: accounts[from].email.clone(),
                    to,
                });
            }
        }
    }

    let footer = card_top + FOOTER_TOP + ROW_STEP * accounts.len() as f32;
    let painter = ui.painter();
    let link = TEXT.width(&ctx, FOOTER_LINK);
    TEXT.paint_line(
        painter,
        pos2(body.left(), footer),
        FOOTER_LINK,
        LINK_INK,
        None,
    );
    let rest = body.left() + link + TEXT.width(&ctx, "a b") - TEXT.width(&ctx, "ab");
    TEXT.paint_line(painter, pos2(rest, footer), FOOTER_REST, FOOTER_INK, None);
    let hit = Rect::from_min_size(pos2(body.left(), footer), vec2(link, TEXT.line_height));
    if ui
        .interact(hit, exo_ui::id(ui.ctx(), "accounts-add"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        event = Some(Event::Add);
    }

    *picker_swatch = state
        .picker
        .map(|index| swatch_rect(right, card_top + FIRST_ROW + ROW_STEP * index as f32));
    event
}

/// What every row lays out against.
struct Columns {
    body: Rect,
    /// The swatches' right edge.
    right: f32,
    /// More than one account: rows get the caps and the drag handle.
    several: bool,
    keycaps: KeycapColors,
}

/// The `index`th account's row at `top`; `pointer_free` when nothing covers or drags over it.
fn row(
    ui: &mut Ui,
    columns: &Columns,
    top: f32,
    index: usize,
    account: &exo_auth::Account,
    pointer_free: bool,
    state: &mut State,
) -> Option<Event> {
    let ctx = ui.ctx().clone();
    let Columns {
        body,
        right,
        several,
        keycaps,
    } = *columns;
    let rect = Rect::from_min_max(
        pos2(body.left() - ROW_LEFT, top),
        pos2(body.right() + ROW_RIGHT, top + ROW_HEIGHT),
    );
    let hovered = pointer_free && ui.rect_contains_pointer(rect);
    let shown = motion::transition(
        &ctx,
        exo_ui::id(&ctx, ("account-row-hover", &account.email)),
        hovered,
        FADE,
        motion::CubicBezier::EASE,
    );
    paint_identity(ui, body.left(), top, account);

    let swatch = swatch_rect(right, top);
    ui.painter()
        .rect_filled(swatch, CornerRadius::same(2), accounts::color(account));
    let swatch_response = ui
        .interact(
            swatch,
            exo_ui::id(ui.ctx(), ("account-swatch", &account.email)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if swatch_response.clicked() {
        state.picker = Some(index);
        state.start = color_index(account);
    }

    // `Sign out` ends 7px (and its padding) before the handle's box, or the swatch's margin.
    let mut sign_out_right = swatch.left() - SWATCH_GAP - SIGN_OUT_BEFORE;
    if several {
        let caps = Keycaps::new(
            &ctx,
            &["ctralt".to_owned(), (index + 1).to_string()],
            keycaps,
        );
        let keys_left = swatch.left() - SWATCH_GAP - caps.width();
        caps.paint(ui.painter(), keys_left, top + KEYS_TOP);
        let handle = Rect::from_min_size(
            pos2(keys_left - HANDLE_BEFORE_KEYS - HANDLE.x, top + HANDLE_TOP),
            HANDLE,
        );
        sign_out_right = handle.left() - SIGN_OUT_BEFORE;
        let response = ui
            .interact(
                handle.expand(10.0),
                exo_ui::id(ui.ctx(), ("account-handle", &account.email)),
                Sense::drag(),
            )
            .on_hover_cursor(egui::CursorIcon::Grab);
        let name = "legacy/DEPRECATED_dragHandle";
        let ink = Color32::from_black_alpha(SUBDUED[usize::from(response.hovered())]);
        exo_ui::paint_icon(
            ui,
            name,
            exo_ui::icon_rect(&ctx, name, handle),
            ink.gamma_multiply(shown),
        );
        if response.drag_started() {
            state.drag = Some((index, response.drag_delta().y));
        }
    }
    let width = TEXT.width(&ctx, SIGN_OUT);
    let text = Rect::from_min_size(
        pos2(sign_out_right - width, top + LINE_TOP),
        vec2(width, TEXT.line_height),
    );
    let response = ui.interact(
        text.expand2(vec2(10.0, 0.0)),
        exo_ui::id(&ctx, ("account-sign-out", &account.email)),
        Sense::click(),
    );
    let response = if hovered {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    };
    let ink = Color32::from_black_alpha(SUBDUED[usize::from(response.hovered())]);
    TEXT.paint_line(
        ui.painter(),
        text.min,
        SIGN_OUT,
        ink.gamma_multiply(shown),
        None,
    );
    (response.clicked() && hovered).then(|| Event::SignOut(account.email.clone()))
}

/// The avatar and the address of a row at `top`.
fn paint_identity(ui: &Ui, left: f32, top: f32, account: &exo_auth::Account) {
    let avatar = Rect::from_min_size(pos2(left, top + AVATAR_TOP), vec2(AVATAR, AVATAR));
    match crate::profile_pictures::photo(ui.ctx(), &account.email) {
        // The avatar's ring in the account's color: `border-width: 2.5px`, which Chromium floors
        // to 2px.
        Some(photo) => exo_ui::paint_photo_avatar(
            ui.painter(),
            avatar,
            &photo,
            2.0,
            accounts::color(account),
            1.0,
        ),
        None => exo_ui::paint_initial_avatar(
            ui.painter(),
            avatar,
            accounts::color(account),
            &accounts::initial(&account.email),
            &INITIAL,
            Color32::BLACK,
            0.375 * 12.0,
        ),
    }
    TEXT.paint_line(
        ui.painter(),
        pos2(left + AVATAR + AVATAR_GAP, top + LINE_TOP),
        &account.email,
        NAME_INK,
        None,
    );
}

fn swatch_rect(right: f32, row_top: f32) -> Rect {
    Rect::from_min_size(
        pos2(right - SWATCH, row_top + SWATCH_TOP),
        vec2(SWATCH, SWATCH),
    )
}

/// The color picker under `swatch`.
fn picker_at(swatch: Rect, size: egui::Vec2) -> egui::Pos2 {
    pos2(
        swatch.center().x - size.x / 2.0,
        swatch.bottom() + PICKER_BELOW,
    )
}

/// Where `account`'s color is in [`COLORS`].
fn color_index(account: &exo_auth::Account) -> usize {
    let name = accounts::color_name(&account.email, Some(account));
    COLORS
        .iter()
        .position(|(known, _)| *known == name)
        .unwrap_or_default()
}

/// The account colors' picker, `chosen` on its white circle.
fn picker_spec(chosen: Option<usize>) -> exo_ui::ColorPicker<'static> {
    const SWATCHES: [Color32; COLORS.len()] = {
        let mut swatches = [Color32::WHITE; COLORS.len()];
        let mut index = 0;
        while index < COLORS.len() {
            swatches[index] = COLORS[index].1;
            index += 1;
        }
        swatches
    };
    exo_ui::ColorPicker {
        colors: &SWATCHES,
        per_row: PER_ROW,
        chosen,
        padding_bottom: 6.0,
    }
}

/// The color picker under `swatch`, `account`'s color chosen.
fn picker(ctx: &Context, swatch: Rect, account: &exo_auth::Account) -> exo_ui::Picked {
    let picker = picker_spec(Some(color_index(account)));
    picker.show(
        ctx,
        exo_ui::id(ctx, "account-color-picker"),
        picker_at(swatch, picker.size()),
        swatch,
    )
}

/// The picker's size.
fn account_picker_size() -> egui::Vec2 {
    picker_spec(None).size()
}
