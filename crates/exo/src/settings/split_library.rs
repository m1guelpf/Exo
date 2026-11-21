//! The Split Inbox Library: a sidebar of categories and roles, a search, `Build your own`, and a
//! grid of template cards per section. Picking a category scrolls to it (and scrolling highlights
//! the section on top); picking a role shows only its templates.

use egui::{Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use exo_store::{SplitLibrary, Template};
use exo_ui::{
    ActionKind, DIALOG_ACTION_HEIGHT, DialogAction, Family, Font, TextStyle, motion, paint_icon,
    palette, text_field,
};

use super::split_preview::{self, Place};

/// Sidebar headers and items: 12px/16px 600, `top: 1px`, in `padding: 8px 12px`.
const SIDEBAR: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 12.0, 16.0);
/// The title: 16px/20px, `top: 2px`.
const TITLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 20.0);
/// Section names and `Build your own`: 14px/16px.
const HEADING: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 14.0, 16.0);
/// Body text: 12px/20px, `top: 3px`.
const BODY: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
/// A card's name: 12px/16px 600, `top: 1px`.
const NAME: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 12.0, 16.0);
/// The no-results line: 12px/16px, `top: 1px`.
const SMALL: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
/// The search field: 14px/16px.
const SEARCH: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 14.0, 16.0);
/// The sidebar: 180px wide, 28px in, then a 16px gap.
const SIDEBAR_LEFT: f32 = 28.0;
const SIDEBAR_WIDTH: f32 = 180.0;
/// The header's and the content's `padding-right: 40px`.
const CONTENT_RIGHT: f32 = 40.0;
/// The search row's height, which it opens to over 300ms.
const SEARCH_HEIGHT: f32 = 52.0;
const SEARCH_CURVE: motion::CubicBezier = motion::CubicBezier::new(0.4, 0.0, 0.2, 1.0);
/// A section scrolled within this far of the top is the current one (the scroll spy).
const SPY: f32 = 50.0;
/// A card's preview height and gaps around its text.
const PREVIEW_HEIGHT: f32 = 104.0;
const GRID_GAP: f32 = 12.0;
/// Classic's corners on the sidebar's selection, `Build your own` and hovered cards.
const RADIUS: u8 = 4;

/// The selected sidebar item and a hovered card: `#f2f4fa`, `#dee3f2` in Carbon.
fn highlight(ctx: &egui::Context) -> Color32 {
    match exo_ui::app_theme(ctx) {
        exo_ui::Theme::Snow => Color32::from_rgb(0xf2, 0xf4, 0xfa),
        exo_ui::Theme::Carbon => Color32::from_rgb(0xde, 0xe3, 0xf2),
    }
}

/// What the user did in the library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryEvent {
    /// A template's card: its settings.
    Open(String),
    /// `Build Your Own` (or the no-results link): New Split Inbox.
    BuildYourOwn,
}

/// The library's scroll, sidebar selection and search, kept while a template opened from it
/// shows.
#[derive(Debug, Default)]
pub(super) struct Library {
    scroll: f32,
    /// Where a category click is scrolling to, smoothly.
    target: Option<f32>,
    /// The highlighted category, `None` while a role or a search shows.
    category: Option<usize>,
    /// The role whose templates show alone.
    role: Option<usize>,
    search_open: bool,
    search: String,
    /// The search field wants the keyboard (it takes it as it opens).
    focus_search: bool,
    scrollbar: exo_ui::Scrollbar,
}

impl Library {
    /// Searching: the search shows results in place of the sections.
    fn searching(&self) -> bool {
        !self.search.trim().is_empty() && self.role.is_none() && self.category.is_none()
    }
}

/// A section as it shows: its name and its templates.
struct Shown<'a> {
    name: Option<&'a str>,
    templates: Vec<&'a Template>,
}

/// The grid's columns at the window's `width` (`repeat(3, 1fr)`, 2 at ≤1147px, 1 at ≤719px).
fn columns(width: f32) -> usize {
    if width <= 719.0 {
        1
    } else if width <= 1147.0 {
        2
    } else {
        3
    }
}

/// A card's height for `template` at `width`: its padding, preview, name and description.
fn card_height(ctx: &egui::Context, template: &Template, width: f32) -> f32 {
    let lines = BODY
        .wrap(ctx, &template.description, width - 24.0)
        .len()
        .max(1);
    8.0 + PREVIEW_HEIGHT + 12.0 + 16.0 + 20.0 * lines as f32 + 12.0
}

/// The rows of a grid of `templates`: each row's top offset and height.
fn grid_rows(
    ctx: &egui::Context,
    templates: &[&Template],
    columns: usize,
    width: f32,
) -> Vec<(f32, f32)> {
    let mut top = 0.0;
    templates
        .chunks(columns)
        .map(|row| {
            let height = row
                .iter()
                .map(|template| card_height(ctx, template, width))
                .fold(0.0, f32::max);
            let at = (top, height);
            top += height + GRID_GAP;
            at
        })
        .collect()
}

/// A section's height: `margin-top: 8px`, its 32px name, the grid and `margin-bottom: 32px`.
fn section_height(rows: &[(f32, f32)], named: bool) -> f32 {
    let grid = rows.last().map_or(0.0, |(top, height)| top + height);
    8.0 + if named { 32.0 } else { 0.0 } + grid + 32.0
}

/// Draws the library filling the dialog's `card` and returns what was picked. `in_use` says
/// whether an enabled split uses a template (its card shows a check).
pub(super) fn show(
    ui: &mut Ui,
    card: Rect,
    library: &SplitLibrary,
    state: &mut Library,
    in_use: &dyn Fn(&Template) -> bool,
) -> Option<LibraryEvent> {
    let ctx = ui.ctx().clone();
    let tokens = palette(&ctx);
    let painter = ui.painter().clone();
    let mut event = None;
    let categories = library.categories();
    let roles = library.roles();
    let template = |id: &String| library.templates.get(id);

    // The sidebar: `Category`, its items, 16px, `Roles` and its items.
    let sidebar_left = card.left() + SIDEBAR_LEFT;
    let mut y = card.top() + 36.0;
    let mut clicked_category = None;
    let mut clicked_role = None;
    let groups: [(&str, &[exo_store::LibrarySection<'_>], Option<usize>); 2] = [
        ("Category", &categories, state.category),
        ("Roles", &roles, state.role),
    ];
    for (group, (header, sections, selected)) in groups.into_iter().enumerate() {
        if sections.is_empty() {
            continue;
        }
        // The sidebar's headers: `rgba(0, 0, 0, .3)`.
        SIDEBAR.paint_line(
            &painter,
            pos2(sidebar_left + 12.0, y + 9.0),
            header,
            tokens.ink(0.3, 0.3),
            None,
        );
        y += 32.0;
        for (index, section) in sections.iter().enumerate() {
            let rect = Rect::from_min_size(pos2(sidebar_left, y), vec2(SIDEBAR_WIDTH, 32.0));
            // Items at `rgba(0, 0, 0, .55)`, the selected one's at `.9`.
            let ink = if selected == Some(index) {
                painter.rect_filled(rect, CornerRadius::same(RADIUS), highlight(&ctx));
                tokens.text_base_default
            } else {
                tokens.ink(0.55, 0.55)
            };
            let name = section
                .section
                .sidebar_name
                .as_deref()
                .unwrap_or(&section.section.name);
            SIDEBAR.paint_line(
                &painter,
                pos2(rect.left() + 12.0, y + 9.0),
                name,
                ink,
                Some(SIDEBAR_WIDTH - 24.0),
            );
            if ui
                .interact(
                    rect,
                    exo_ui::id(ui.ctx(), ("split-library-sidebar", group, index)),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                if group == 0 {
                    clicked_category = Some(index);
                } else {
                    clicked_role = Some(index);
                }
            }
            y += 32.0;
        }
        y += 16.0;
    }

    // The header: the title and the search button, then the search field sliding open.
    let left = card.left() + SIDEBAR_LEFT + SIDEBAR_WIDTH + 16.0;
    let right = card.right() - CONTENT_RIGHT;
    TITLE.paint_line(
        &painter,
        pos2(left + 16.0, card.top() + 32.0 + 8.0 + 2.0),
        "Split Inbox Library",
        tokens.text_base_default,
        None,
    );
    // The search button: 76px in, 8px of padding around Classic's 14px icon, at
    // `rgba(0, 0, 0, .4)`.
    let search_button = Rect::from_min_size(
        pos2(card.right() - 76.0 - 30.0, card.top() + 35.0),
        vec2(30.0, 30.0),
    );
    let search_response = ui
        .interact(
            search_button,
            exo_ui::id(ui.ctx(), "split-library-search-button"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    paint_icon(
        ui,
        "legacy/DEPRECATED_search",
        search_button.shrink(8.0),
        tokens.ink(0.4, 0.4),
    );
    if search_response.clicked() {
        state.search_open = !state.search_open;
        state.focus_search = state.search_open;
        // Closing it goes back to the sections, and the text goes.
        if !state.search_open && state.role.is_none() {
            state.search.clear();
            state.category.get_or_insert(0);
        }
    }
    let open = motion::transition(
        &ctx,
        exo_ui::id(&ctx, "split-library-search"),
        state.search_open,
        0.3,
        SEARCH_CURVE,
    );
    let search_top = card.top() + 32.0 + 36.0 + 16.0;
    let search_height = SEARCH_HEIGHT * open;
    if open > 0.0 {
        let clip = Rect::from_min_max(
            pos2(left, search_top),
            pos2(right, search_top + search_height),
        );
        let mut field_ui = ui.new_child(egui::UiBuilder::new().max_rect(clip));
        field_ui.set_clip_rect(clip.intersect(ui.clip_rect()));
        let line = Rect::from_min_max(
            pos2(left + 16.0, search_top + 12.0),
            pos2(right - 12.0 - 16.0, search_top + 12.0 + 16.0),
        );
        if state.search.is_empty() {
            // The placeholder: `rgba(0, 0, 0, .4)`.
            SEARCH.paint_line(
                field_ui.painter(),
                line.min,
                "Search",
                tokens.ink(0.4, 0.4),
                None,
            );
        }
        let before = state.search.clone();
        let id = exo_ui::id(&ctx, "split-library-search-field");
        let response = text_field(
            &mut field_ui,
            id,
            &mut state.search,
            line,
            &SEARCH,
            tokens.text_base_default,
            tokens.text_selection_default,
        );
        if std::mem::take(&mut state.focus_search) {
            response.request_focus();
        }
        // The rule: 1px `rgba(0, 0, 0, .09)`, focused or not in Classic.
        let rule = Rect::from_min_size(
            pos2(left + 16.0, line.bottom() + 8.0),
            vec2(right - left - 28.0, 1.0),
        );
        field_ui
            .painter()
            .rect_filled(rule, 0.0, tokens.ink(0.09, 0.09));
        // The clear button (12×13, black at `opacity: .4` in Classic): clears, or
        // closes the search when it's empty.
        let clear = Rect::from_min_size(
            pos2(right - 12.0 - 12.0, search_top + 11.0),
            vec2(12.0, 13.0),
        );
        exo_ui::paint_close(field_ui.painter(), clear.min, tokens.ink(0.4, 0.4));
        if field_ui
            .interact(
                clear.expand(10.0),
                exo_ui::id(field_ui.ctx(), "split-library-search-clear"),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            if state.search.trim().is_empty() {
                state.search_open = false;
                state.search.clear();
                state.category.get_or_insert(0);
            } else {
                state.search.clear();
                state.focus_search = true;
                state.category = Some(0);
            }
        }
        if state.search != before {
            // Typing searches everything: no role, no category.
            state.role = None;
            state.category = None;
            state.scroll = 0.0;
            state.target = None;
            if state.search.trim().is_empty() {
                state.category = Some(0);
            }
        }
    }

    // `Build your own`.
    let build = Rect::from_min_max(
        pos2(left + 8.0, search_top + search_height),
        pos2(right - 8.0, search_top + search_height + 76.0),
    );
    // `Build your own`: `1px solid rgba(0, 0, 0, .09)`; its subheader
    // `rgba(0, 0, 0, .55)`.
    painter.rect_stroke(
        build,
        CornerRadius::same(RADIUS),
        Stroke::new(1.0, tokens.ink(0.09, 0.09)),
        StrokeKind::Inside,
    );
    HEADING.paint_line(
        &painter,
        pos2(build.left() + 17.0, build.top() + 19.0 + 1.0),
        "Build your own",
        tokens.text_base_default,
        None,
    );
    BODY.paint_line(
        &painter,
        pos2(build.left() + 17.0, build.top() + 19.0 + 16.0 + 1.0 + 3.0),
        "Organize your inbox exactly how you want.",
        tokens.ink(0.55, 0.55),
        None,
    );
    let button = DialogAction::new("Build Your Own", ActionKind::Primary);
    let width = button.width(&ctx);
    let button_rect = Rect::from_min_size(
        pos2(build.right() - 17.0 - width, build.top() + 19.0 + 3.0),
        vec2(width, DIALOG_ACTION_HEIGHT),
    );
    if button
        .show(ui, exo_ui::id(ui.ctx(), "split-library-build"), button_rect)
        .clicked()
    {
        event = Some(LibraryEvent::BuildYourOwn);
    }

    // The sections, or the search's results, scrolling under the header.
    let viewport = Rect::from_min_max(
        pos2(left, build.bottom() + 8.0),
        pos2(card.right(), card.bottom()),
    );
    let width = right - left;
    let columns = columns(ctx.content_rect().width());
    let column = (width - GRID_GAP * (columns - 1) as f32) / columns as f32;
    let shown: Vec<Shown<'_>> = if state.searching() {
        vec![Shown {
            name: None,
            templates: library.search(&state.search),
        }]
    } else if let Some(role) = state.role.and_then(|role| roles.get(role)) {
        vec![Shown {
            name: Some(&role.section.name),
            templates: role.templates.iter().filter_map(template).collect(),
        }]
    } else {
        categories
            .iter()
            .map(|section| Shown {
                name: Some(&section.section.name),
                templates: section.templates.iter().filter_map(template).collect(),
            })
            .collect()
    };
    let mut tops = Vec::with_capacity(shown.len());
    let mut total = 0.0;
    for (index, section) in shown.iter().enumerate() {
        tops.push(total);
        let rows = grid_rows(&ctx, &section.templates, columns, column);
        let mut height = section_height(&rows, section.name.is_some());
        // The last category stretches so it can scroll to the top (`min-height: calc(100vh -
        // 240px)`, no bottom margin).
        if state.role.is_none() && !state.searching() && index + 1 == shown.len() {
            height = (height - 32.0).max(ctx.content_rect().height() - 240.0);
        }
        total += height;
    }
    let max_scroll = (total - viewport.height()).max(0.0);

    if let Some(index) = clicked_category {
        state.role = None;
        state.search.clear();
        state.category = Some(index);
        // To the section's border box, under its `margin-top: 8px` (`offsetTop`).
        state.target = tops
            .get(index)
            .copied()
            .map(|top| (top + 8.0).min(max_scroll));
    }
    if let Some(index) = clicked_role {
        state.role = Some(index);
        state.search.clear();
        state.category = None;
        state.scroll = 0.0;
        state.target = None;
    }
    if ui.rect_contains_pointer(viewport) {
        let delta = ui.input(|input| input.smooth_scroll_delta.y);
        if delta != 0.0 {
            state.target = None;
            state.scroll -= delta;
        }
    }
    if let Some(target) = state.target {
        // A smooth scroll: most of the way each frame, done within a pixel.
        let step = (target - state.scroll) * 0.25;
        state.scroll = if step.abs() < 0.5 {
            target
        } else {
            state.scroll + step
        };
        if state.scroll == target {
            state.target = None;
        } else {
            ctx.request_repaint();
        }
    }
    state.scroll = state.scroll.clamp(0.0, max_scroll);
    // The scroll spy: the last category whose top is within 50px of the viewport's.
    if state.target.is_none() && state.role.is_none() && !state.searching() {
        state.category = Some(
            tops.iter()
                .rposition(|top| *top + 8.0 - state.scroll <= SPY)
                .unwrap_or(0),
        );
    }

    state.scrollbar.show(
        ui,
        exo_ui::id(ui.ctx(), "split-library-scrollbar"),
        viewport,
        state.scroll,
        total,
    );
    let mut content = ui.new_child(egui::UiBuilder::new().max_rect(viewport));
    content.set_clip_rect(viewport.intersect(ui.clip_rect()));
    for (section, top) in shown.iter().zip(&tops) {
        let mut y = viewport.top() + top - state.scroll + 8.0;
        if let Some(name) = section.name {
            HEADING.paint_line(
                content.painter(),
                pos2(left + 16.0, y + 8.0),
                name,
                tokens.text_base_default,
                None,
            );
            y += 32.0;
        }
        let rows = grid_rows(&ctx, &section.templates, columns, column);
        for (row, (row_top, row_height)) in section.templates.chunks(columns).zip(&rows) {
            for (index, template) in row.iter().enumerate() {
                // Grid items stretch to their row's tallest.
                let rect = Rect::from_min_size(
                    pos2(left + (column + GRID_GAP) * index as f32, y + row_top),
                    vec2(column, *row_height),
                );
                if rect.bottom() < viewport.top() || rect.top() > viewport.bottom() {
                    continue;
                }
                let response = content
                    .interact(
                        rect.intersect(viewport),
                        exo_ui::id(content.ctx(), ("split-library-card", &template.id)),
                        Sense::click(),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if response.hovered() {
                    content.painter().rect_filled(
                        rect,
                        CornerRadius::same(RADIUS),
                        highlight(&ctx),
                    );
                }
                paint_card(&content, rect, template, in_use(template));
                if response.clicked() {
                    event = Some(LibraryEvent::Open(template.id.clone()));
                }
            }
        }
        if state.searching() && section.templates.is_empty() {
            // `No Split Inbox templates found. But don't let that stop you — build your own!`,
            // centered 24px down.
            let lead = "No Split Inbox templates found. But don't let that stop you —";
            let lead_width = SMALL.width(&ctx, lead);
            let space = SMALL.width(&ctx, "a b") - SMALL.width(&ctx, "ab");
            let link_width = SMALL.width(&ctx, "build your own");
            let total = lead_width + space + link_width + SMALL.width(&ctx, "!");
            let x = left + (width - total) / 2.0;
            let line = y + 24.0 + 1.0;
            // No results: `rgba(0, 0, 0, .4)`.
            let ink = tokens.ink(0.4, 0.4);
            SMALL.paint_line(content.painter(), pos2(x, line), lead, ink, None);
            let link =
                Rect::from_min_size(pos2(x + lead_width + space, line), vec2(link_width, 16.0));
            SMALL.paint_line(
                content.painter(),
                link.min,
                "build your own",
                tokens.link_text_primary,
                None,
            );
            SMALL.paint_line(content.painter(), pos2(link.right(), line), "!", ink, None);
            if content
                .interact(
                    link,
                    exo_ui::id(content.ctx(), "split-library-build-link"),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                event = Some(LibraryEvent::BuildYourOwn);
            }
        }
    }
    event
}

/// A template's card in `rect`: its preview, its name (with a check while a split uses it) and
/// its description.
fn paint_card(ui: &Ui, rect: Rect, template: &Template, in_use: bool) {
    let tokens = palette(ui.ctx());
    let painter = ui.painter();
    let preview = Rect::from_min_size(
        rect.min + vec2(8.0, 8.0),
        vec2(rect.width() - 16.0, PREVIEW_HEIGHT),
    );
    split_preview::paint(ui, preview, template, Place::Library);
    let header = preview.bottom() + 12.0;
    let name = NAME.paint_line(
        painter,
        pos2(rect.left() + 16.0, header + 1.0),
        &template.name,
        tokens.text_base_default,
        Some(rect.width() - 40.0),
    );
    if in_use {
        // The checkmark: `rgba(0, 0, 0, .5)`.
        exo_ui::paint_icon_on(
            painter,
            "legacy/DEPRECATED_checkmarkInCircle",
            exo_ui::pixel_snap(Rect::from_min_size(
                pos2(rect.left() + 16.0 + name + 4.0, header),
                vec2(16.0, 16.0),
            )),
            tokens.ink(0.5, 0.5),
        );
    }
    // The description: `rgba(0, 0, 0, .55)`.
    for (index, line) in BODY
        .wrap(ui.ctx(), &template.description, rect.width() - 24.0)
        .iter()
        .enumerate()
    {
        BODY.paint_line(
            painter,
            pos2(
                rect.left() + 16.0,
                header + 16.0 + 20.0 * index as f32 + 3.0,
            ),
            line,
            tokens.ink(0.55, 0.55),
            None,
        );
    }
}
