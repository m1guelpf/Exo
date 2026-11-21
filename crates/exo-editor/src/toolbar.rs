//! The floating formatting toolbar: shown under a non-empty selection, with the link input and the
//! colour picker as its sub-modes; and the link popover under a caret in a link.

use egui::{Area, Color32, CornerRadius, Key, Order, Rect, Sense, Ui, Vec2, pos2, vec2};
use exo_ui::{Family, Font, TextStyle};

use crate::{
    editor::{Color, Editor, Format, LinkAction},
    model::{Document, Selection},
};

/// A button: 26×26, 3px corners, 7px apart, 7px padding around the row.
const BUTTON: f32 = 26.0;
const GAP: f32 = 7.0;
const PADDING: f32 = 7.0;
/// The link input: 200px wide, Exo Sans 12px/24px, `padding: 3px 8px 0`; Chromium's placeholder
/// colour (`#757575`, in both themes).
const LINK_WIDTH: f32 = 200.0;
const LINK_TEXT: TextStyle =
    TextStyle::new(Font::new(Family::Sans, 400), 12.0, 24.0).without_ligatures();
const LINK_PLACEHOLDER: Color32 = Color32::from_rgb(0x75, 0x75, 0x75);
/// Keytips while ⌘ is held, and Classic's B, I, U and S faces: 16px/24px Exo Sans, 2px down.
const KEYTIP: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 24.0);
/// Colour swatches: 32px circles 3px apart, a 26px dot inside.
const SWATCH: f32 = 32.0;
const SWATCH_GAP: f32 = 3.0;
/// The link popover, a tooltip: Exo Sans 12px/20px bold, padding `6px 7px 3px`, its parts 7px
/// apart.
const POPOVER_TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 20.0);
const POPOVER_PADDING: [f32; 3] = [6.0, 7.0, 3.0];
/// The popover's remove button: `margin-left: 2px; padding: 1px 7px 0 6px` around the 11×12 discard
/// icon, `opacity: .4`, `.6` hovered (150ms ease), `1` pressed.
const REMOVE_MARGIN: f32 = 2.0;
const REMOVE_SIZE: Vec2 = Vec2::new(25.0, 20.0);
const REMOVE_ICON: Rect = Rect::from_min_max(pos2(6.0, 2.0), pos2(17.0, 14.0));
/// The remove button's keytip, while ⌘ is held: a 16px square `margin: 0 5px 0 6px`, 3px corners,
/// `opacity: .6` (`1` hovered).
const KEYTIP_MARGIN: f32 = 6.0;
const KEYTIP_SIZE: f32 = 16.0;

/// A button's color on the toolbar's tooltip card: white on Snow's dark card, black on Carbon's
/// light one.
fn ink(palette: &exo_ui::Palette) -> Color32 {
    match palette.theme {
        exo_ui::Theme::Snow => palette.text_light_default,
        exo_ui::Theme::Carbon => Color32::BLACK,
    }
}

/// What a format button shows.
#[derive(Clone, Copy)]
enum Face {
    /// A letter in the keytip style.
    Letter(&'static str),
    /// An icon at its size.
    Icon(&'static str, Vec2),
    /// The colour button's three dots.
    Colors,
}

/// What a toolbar click did, so the app can show its "Next time, hit …" hint (clicks only;
/// shortcuts show none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolbarAction {
    /// A format button.
    Format(Format),
    /// The link button opened the link input.
    LinkSet,
    /// The link button, or the link popover's, removed the link.
    LinkRemoved,
    /// A bare URL was pasted into the link input.
    LinkPasted,
    /// The colour button opened the colour picker.
    Color(Color),
    /// The numbered-list button.
    Numbers,
    /// The bulleted-list button.
    Bullets,
    /// The quote button.
    Quote,
}

impl ToolbarAction {
    /// Its hint's copy, with `{keys}` where the key caps go.
    pub const fn hint(self) -> Option<(&'static str, &'static [&'static str], &'static str)> {
        match self {
            Self::Color(_) => Some(("Next time, hit", &["⌘", "O"], "to instantly change color!")),
            Self::Numbers => Some((
                "Next time, hit",
                &["⌘", "shift", "7"],
                "to instantly toggle a numbered list!",
            )),
            Self::Bullets => Some((
                "Next time, hit",
                &["⌘", "shift", "8"],
                "to instantly toggle a bullet list!",
            )),
            Self::Quote => Some((
                "Next time, hit",
                &["⌘", "shift", "9"],
                "to instantly toggle a quote!",
            )),
            Self::LinkSet => Some(("Next time, hit", &["⌘", "K"], "to instantly set a link!")),
            Self::LinkRemoved => {
                Some(("Next time, hit", &["⌘", "K"], "to instantly remove a link!"))
            }
            Self::LinkPasted => Some((
                "Next time, just select text and hit",
                &["⌘", "V"],
                "to instantly set a link!",
            )),
            Self::Format(_) => None,
        }
    }
}

/// The toolbar's sub-mode.
#[derive(Clone, Debug, Default)]
pub(crate) enum Mode {
    #[default]
    Buttons,
    /// The link input, with what's typed; `fresh` until it has been focused.
    Link { input: String, fresh: bool },
    /// The colour picker on swatch `index`; `original` is the state to go back to on Esc.
    Color {
        index: usize,
        original: Box<(Document, Selection)>,
    },
}

/// What the toolbar keeps between frames.
#[derive(Default)]
pub(crate) struct State {
    pub(crate) mode: Mode,
    /// Whether it showed last frame.
    pub(crate) visible: bool,
    /// Whether the link popover showed last frame.
    pub(crate) popover: bool,
    /// Where Escape closed the link popover: it stays closed until the caret moves or the text
    /// changes.
    pub(crate) popover_closed: Option<(Selection, u64)>,
    /// Where the toolbar and the popover were drawn, whose clicks keep the editor's focus.
    pub(crate) rects: Vec<Rect>,
    /// The link input's spelling (its `<input>` has Chromium's spellcheck on).
    pub(crate) link_spelling: crate::FieldSpelling,
}

/// A click in the link popover.
pub(crate) enum PopoverClick {
    /// On the URL: open it.
    Open(String),
    /// On the remove button.
    Remove,
}

impl State {
    /// Closes any sub-mode (a mouse-down in the body).
    pub(crate) fn reset(&mut self) {
        self.mode = Mode::Buttons;
    }

    /// Whether a sub-mode is open.
    pub(crate) fn is_open(&self) -> bool {
        !matches!(self.mode, Mode::Buttons)
    }
}

impl Editor {
    /// The link input's spellchecker, for the app to give it one.
    pub fn link_spelling_mut(&mut self) -> &mut crate::FieldSpelling {
        &mut self.view.toolbar.link_spelling
    }

    /// The link input's spelling and text while it shows, for what its right-click menu chose.
    pub fn link_field(&mut self) -> Option<(&mut crate::FieldSpelling, &mut String)> {
        let toolbar = &mut self.view.toolbar;
        match &mut toolbar.mode {
            Mode::Link { input, .. } => Some((&mut toolbar.link_spelling, input)),
            _ => None,
        }
    }
    /// ⌘K: removes the link the selection is in, or opens the link input.
    pub fn begin_link(&mut self) {
        match self.link_action() {
            LinkAction::None => {}
            LinkAction::Remove => self.remove_link(),
            LinkAction::Edit(href) => {
                self.view.toolbar.mode = Mode::Link {
                    input: href,
                    fresh: true,
                }
            }
        }
    }

    /// ⌘O: opens the colour picker on the selection's colour.
    pub fn begin_color(&mut self) {
        let index = Color::ALL
            .iter()
            .position(|color| *color == self.color())
            .unwrap_or(0);
        self.save_undo();
        self.view.toolbar.mode = Mode::Color {
            index,
            original: Box::new((self.doc.clone(), self.selection)),
        };
    }

    /// Whether the link input or the colour picker is open.
    pub fn formatter_open(&self) -> bool {
        self.view.toolbar.is_open()
    }

    /// Draws the toolbar under the selection (whose bounds are `selection`) when it should
    /// show, and handles its clicks and keys.
    pub(crate) fn show_toolbar(
        &mut self,
        ui: &Ui,
        selection: Rect,
        focused: bool,
    ) -> Option<ToolbarAction> {
        let has_text = !self.selected_text().trim().is_empty();
        let show = self.view.toolbar.is_open()
            || (focused
                && !self.selection.is_collapsed()
                && has_text
                && self.view.preedit.is_none());
        self.view.toolbar.visible = show;
        if !show || !selection.is_positive() {
            return None;
        }
        let ctx = ui.ctx().clone();
        let id = exo_ui::id(&ctx, "exo_editor::toolbar");
        let opacity = ctx.animate_bool_with_time(id.with("fade"), true, 0.2);
        let width = match &self.view.toolbar.mode {
            Mode::Buttons => PADDING * 2.0 + BUTTON * 9.0 + GAP * 8.0,
            Mode::Link { .. } => PADDING * 2.0 + LINK_WIDTH,
            Mode::Color { .. } => 7.0 * 2.0 + SWATCH * 6.0 + SWATCH_GAP * 5.0,
        };
        let height = match &self.view.toolbar.mode {
            Mode::Color { .. } => 6.0 * 2.0 + SWATCH,
            _ => PADDING * 2.0 + BUTTON,
        };
        // Centred on the selection, 3px below it, kept 8px inside the editor's area.
        let bounds = ui.clip_rect();
        let x = (selection.center().x - width / 2.0).clamp(
            bounds.min.x + 8.0,
            (bounds.max.x - 8.0 - width).max(bounds.min.x + 8.0),
        );
        let top_left = pos2(x.round(), (selection.max.y + 3.0).round());
        let mut action = None;
        let command = ui.input(|input| input.modifiers.mac_cmd || input.modifiers.command);
        self.view
            .toolbar
            .rects
            .push(Rect::from_min_size(top_left, vec2(width, height)));
        Area::new(id)
            .order(Order::Foreground)
            .fixed_pos(top_left)
            .show(&ctx, |ui| {
                ui.set_opacity(opacity);
                let rect = Rect::from_min_size(top_left, vec2(width, height));
                let palette = exo_ui::palette(ui.ctx());
                ui.painter().rect_filled(
                    rect,
                    CornerRadius::same(4),
                    palette.tooltip_background_default,
                );
                // Clicks on the toolbar's padding don't reach the editor under it.
                ui.allocate_exact_size(rect.size(), Sense::click());
                action = match self.view.toolbar.mode.clone() {
                    Mode::Buttons => self.buttons(ui, rect, command),
                    Mode::Link { input, fresh } => self.link_input(ui, rect, input, fresh),
                    Mode::Color { index, original } => {
                        self.color_picker(ui, rect, index, original);
                        None
                    }
                };
            });
        action
    }

    fn buttons(&mut self, ui: &Ui, rect: Rect, command: bool) -> Option<ToolbarAction> {
        // Classic's faces: letters for the text styles, legacy icons at their own sizes.
        let buttons: [(Face, &str, Option<&str>, ToolbarAction); 9] = [
            (
                Face::Letter("B"),
                "B",
                None,
                ToolbarAction::Format(Format::Bold),
            ),
            (
                Face::Letter("I"),
                "I",
                None,
                ToolbarAction::Format(Format::Italic),
            ),
            (
                Face::Letter("U"),
                "U",
                None,
                ToolbarAction::Format(Format::Underline),
            ),
            (
                Face::Letter("S"),
                "X",
                Some("⇧"),
                ToolbarAction::Format(Format::Strikethrough),
            ),
            (
                Face::Icon("legacy/DEPRECATED_link", vec2(12.0, 12.0)),
                "K",
                None,
                ToolbarAction::LinkSet,
            ),
            (Face::Colors, "O", None, ToolbarAction::Color(Color::None)),
            (
                Face::Icon("legacy/DEPRECATED_numberedList", vec2(19.0, 20.0)),
                "7",
                Some("⇧"),
                ToolbarAction::Numbers,
            ),
            (
                Face::Icon("legacy/DEPRECATED_bulletList", vec2(18.0, 14.0)),
                "8",
                Some("⇧"),
                ToolbarAction::Bullets,
            ),
            (
                Face::Icon("legacy/DEPRECATED_blockQuote", vec2(15.0, 10.0)),
                "9",
                Some("⇧"),
                ToolbarAction::Quote,
            ),
        ];
        let palette = exo_ui::palette(ui.ctx());
        let mut clicked = None;
        for (index, (face, key, shift, action)) in buttons.into_iter().enumerate() {
            let button = Rect::from_min_size(
                rect.min + vec2(PADDING + index as f32 * (BUTTON + GAP), PADDING),
                Vec2::splat(BUTTON),
            );
            let response = ui.interact(button, ui.id().with(("toolbar", index)), Sense::click());
            let selected = match action {
                ToolbarAction::Format(format) => self.has_format(format.tag()),
                ToolbarAction::LinkSet => matches!(self.link_action(), LinkAction::Remove),
                _ => false,
            };
            // The whole button (background too) is at `opacity: .6` until hovered or selected.
            let opacity = if response.hovered() || selected {
                1.0
            } else {
                0.6
            };
            // `hsla(0,0%,100%,.15)` in every theme.
            ui.painter().rect_filled(
                button,
                CornerRadius::same(3),
                Color32::from_rgba_unmultiplied(255, 255, 255, 38).gamma_multiply(opacity),
            );
            let content = ink(palette).gamma_multiply(opacity);
            let letter = |text: &str| {
                let galley = KEYTIP.single_line(ui.painter(), text, content, None);
                let left = button.center().x - galley.size().x / 2.0;
                let width = galley.size().x;
                exo_ui::paint_text(
                    ui.painter(),
                    pos2(left, button.min.y + 2.0),
                    galley,
                    &KEYTIP,
                );
                (left, width)
            };
            if command {
                letter(&shift.map_or_else(|| key.to_owned(), |shift| format!("{shift}{key}")));
            } else if let Face::Letter(text) = face {
                let (left, width) = letter(text);
                if text == "S" {
                    // `span::after`: a 1px `currentcolor` line, `bottom: 13px` in the letter's box.
                    let bottom = button.min.y + 2.0 + KEYTIP.content_bottom(ui.ctx()) - 13.0;
                    ui.painter().rect_filled(
                        Rect::from_min_max(pos2(left, bottom - 1.0), pos2(left + width, bottom)),
                        0.0,
                        content,
                    );
                }
            } else if let Face::Icon(icon, size) = face {
                exo_ui::paint_icon(
                    ui,
                    icon,
                    exo_ui::pixel_snap(Rect::from_center_size(button.center(), size)),
                    content,
                );
            } else {
                // The colour button: three overlapping dots in a 12px square, green above,
                // red and blue below.
                // The button's opacity applies to it as a group, so the dots don't show through
                // each other: each is its colour mixed into the toolbar.
                let under = palette.tooltip_background_default;
                for (center, [r, g, b]) in [
                    (vec2(0.0, -2.2), [0x5c, 0xba, 0x87]),
                    (vec2(-2.5, 2.2), [0xde, 0x54, 0x54]),
                    (vec2(2.5, 2.2), [0x4a, 0x90, 0xe2]),
                ] {
                    let color = exo_ui::motion::mix(under, Color32::from_rgb(r, g, b), opacity);
                    ui.painter()
                        .circle_filled(button.center() + center, 3.3, color);
                }
            }
            if response.clicked() {
                clicked = Some(action);
            }
        }
        let action = clicked?;
        match action {
            ToolbarAction::Format(format) => self.toggle_format(format),
            ToolbarAction::LinkSet => {
                if matches!(self.link_action(), LinkAction::Remove) {
                    self.remove_link();
                    return Some(ToolbarAction::LinkRemoved);
                }
                self.begin_link();
            }
            ToolbarAction::Color(_) => self.begin_color(),
            ToolbarAction::Numbers => self.toggle_list(true),
            ToolbarAction::Bullets => self.toggle_list(false),
            ToolbarAction::Quote => self.toggle_quote(),
            ToolbarAction::LinkRemoved | ToolbarAction::LinkPasted => {}
        }
        Some(action)
    }

    fn link_input(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        mut input: String,
        fresh: bool,
    ) -> Option<ToolbarAction> {
        let id = ui.id().with("link_input");
        let line = Rect::from_min_size(
            rect.min + vec2(PADDING + 8.0, PADDING + 3.0),
            vec2(LINK_WIDTH - 16.0, 24.0),
        );
        let palette = exo_ui::palette(ui.ctx());
        let color = ink(palette);
        if input.is_empty() {
            LINK_TEXT.paint_line(
                ui.painter(),
                line.min,
                "Paste or type a link",
                LINK_PLACEHOLDER,
                Some(line.width()),
            );
        }
        // A paste that is just a link earns the hint.
        let pasted = ui.memory(|memory| memory.has_focus(id))
            && ui.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Paste(text) if exo_mail::match_link(text).is_some_and(|(range, _)| range.len() == text.len()))));
        let response = self.view.toolbar.link_spelling.show(
            ui,
            id,
            &mut input,
            line,
            &LINK_TEXT,
            color,
            palette.text_selection_default,
        );
        if fresh {
            response.request_focus();
        }
        let (enter, tab, escape) = ui.input(|input| {
            (
                input.key_pressed(Key::Enter),
                input.key_pressed(Key::Tab),
                input.key_pressed(Key::Escape),
            )
        });
        if escape {
            self.view.toolbar.reset();
            self.collapse_selection();
            return None;
        }
        if enter || tab {
            self.view.toolbar.reset();
            self.set_link(&input);
            return None;
        }
        self.view.toolbar.mode = Mode::Link {
            input,
            fresh: false,
        };
        pasted.then_some(ToolbarAction::LinkPasted)
    }

    fn color_picker(
        &mut self,
        ui: &Ui,
        rect: Rect,
        mut index: usize,
        original: Box<(Document, Selection)>,
    ) {
        let count = Color::ALL.len();
        let (left, right, confirm, escape) = ui.input(|input| {
            let back = input.key_pressed(Key::ArrowLeft)
                || (input.key_pressed(Key::Tab) && input.modifiers.shift)
                || (input.key_pressed(Key::O) && input.modifiers.shift);
            let forward = input.key_pressed(Key::ArrowRight)
                || (input.key_pressed(Key::Tab) && !input.modifiers.shift)
                || (input.key_pressed(Key::O) && !input.modifiers.shift && input.modifiers.command);
            (
                back,
                forward,
                input.key_pressed(Key::Enter) || input.key_pressed(Key::Space),
                input.key_pressed(Key::Escape),
            )
        });
        let mut picked = None;
        for (swatch, color) in Color::ALL.into_iter().enumerate() {
            let outer = Rect::from_min_size(
                rect.min + vec2(7.0 + swatch as f32 * (SWATCH + SWATCH_GAP), 6.0),
                Vec2::splat(SWATCH),
            );
            let response = ui.interact(outer, ui.id().with(("swatch", swatch)), Sense::click());
            if swatch == index {
                ui.painter().circle_filled(
                    outer.center(),
                    SWATCH / 2.0,
                    exo_ui::palette(ui.ctx()).text_light_subdued,
                );
            } else if response.hovered() {
                ui.painter().circle_stroke(
                    outer.center(),
                    SWATCH / 2.0,
                    egui::Stroke::new(0.6, Color32::from_rgb(0x90, 0x93, 0x97)),
                );
            }
            let [r, g, b] = color.rgb();
            let fill = if color == Color::None && exo_ui::theme(ui.ctx()) == exo_ui::Theme::Carbon {
                Color32::WHITE
            } else {
                Color32::from_rgb(r, g, b)
            };
            ui.painter().circle_filled(
                pos2(outer.center().x, outer.min.y + SWATCH * 0.09375 + 13.0),
                13.0,
                fill,
            );
            if response.clicked() {
                picked = Some(swatch);
            }
        }
        if escape {
            let (doc, selection) = *original;
            self.view.toolbar.reset();
            self.restore(doc, selection);
            return;
        }
        if let Some(swatch) = picked {
            self.view.toolbar.reset();
            self.apply_color(Color::ALL[swatch]);
            return;
        }
        if left || right {
            index = if left {
                (index + count - 1) % count
            } else {
                (index + 1) % count
            };
            self.apply_color(Color::ALL[index]);
        }
        if confirm {
            self.view.toolbar.reset();
            return;
        }
        self.view.toolbar.mode = Mode::Color { index, original };
    }

    /// The link the caret is in, when it shows the link popover: a link that isn't a mention, the
    /// popover not closed here with Escape.
    pub(crate) fn popover_link(&self) -> Option<(usize, std::ops::Range<usize>, String)> {
        if self.view.toolbar.popover_closed == Some((self.selection, self.revision)) {
            return None;
        }
        let (block, range, link) = self.link_around_caret()?;
        if link.attr("data-mention").is_some() {
            return None;
        }
        Some((block, range, link.attr("href")?.to_owned()))
    }

    /// Draws the link popover under the link (whose box is `anchor`) for an editor whose left is
    /// at `left` (the formatting toolbar in a tooltip): the URL without its scheme,
    /// `www.` and trailing `/`, then the remove button, or its keytip `K` while ⌘ is held.
    pub(crate) fn link_popover(
        &mut self,
        ui: &Ui,
        anchor: Rect,
        left: f32,
        href: &str,
    ) -> Option<PopoverClick> {
        let ctx = ui.ctx().clone();
        let id = exo_ui::id(&ctx, "exo_editor::link_popover");
        let shown = href
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        let shown = shown.strip_prefix("www.").unwrap_or(shown);
        let shown = shown.strip_suffix('/').unwrap_or(shown);
        let bounds = ui.clip_rect();
        let color = exo_ui::tooltip_text_color(&ctx);
        let mut job =
            egui::text::LayoutJob::single_section(shown.to_owned(), POPOVER_TEXT.format(color));
        // `word-break: break-word` in a box no wider than the editor.
        job.wrap = egui::text::TextWrapping::wrap_at_width(
            (bounds.width()
                - 16.0
                - 2.0 * POPOVER_PADDING[1]
                - GAP
                - REMOVE_MARGIN
                - REMOVE_SIZE.x)
                .max(40.0),
        );
        let galley = exo_ui::layout(&ctx, job);
        let [top, side, bottom] = POPOVER_PADDING;
        let size = vec2(
            side + galley.size().x + GAP + REMOVE_MARGIN + REMOVE_SIZE.x + side,
            top + galley.rows.len().max(1) as f32 * POPOVER_TEXT.line_height + bottom,
        );
        // Centred under the link on a whole pixel of the tooltip layer (`Math.ceil`), which
        // starts where the editor does to the fraction; 3px below it; kept 8px inside the area.
        let x = (anchor.center().x - size.x / 2.0).ceil() + left.fract();
        let x = x.clamp(
            bounds.min.x + 8.0,
            (bounds.max.x - 8.0 - size.x).max(bounds.min.x + 8.0),
        );
        let rect = Rect::from_min_size(pos2(x, anchor.max.y + 3.0), size);
        let command = ui.input(|input| input.modifiers.mac_cmd || input.modifiers.command);
        let mut click = None;
        self.view.toolbar.rects.push(rect);
        Area::new(id)
            .order(Order::Foreground)
            .fixed_pos(rect.min)
            .show(&ctx, |ui| {
                // Clicks on the popover don't reach the editor under it, nor take its focus (which
                // would close the popover).
                ui.allocate_exact_size(rect.size(), Sense::CLICK);
                exo_ui::paint_tooltip_box(ui.painter(), rect);
                let text = rect.min + vec2(side, top);
                let link = ui
                    .interact(
                        Rect::from_min_size(
                            text,
                            galley.size().max(vec2(0.0, POPOVER_TEXT.line_height)),
                        ),
                        id.with("url"),
                        Sense::CLICK,
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                link.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Link, true, shown));
                // Underlined while hovered.
                let underline = if link.hovered() {
                    egui::Stroke::new(1.0, color)
                } else {
                    egui::Stroke::NONE
                };
                let mut galley = galley.clone();
                if underline != egui::Stroke::NONE {
                    let mut job = (*galley.job).clone();
                    for section in &mut job.sections {
                        section.format.underline = underline;
                    }
                    galley = exo_ui::layout(&ctx, job);
                }
                let right = text.x + galley.size().x;
                exo_ui::paint_text(ui.painter(), text, galley, &POPOVER_TEXT);
                if link.clicked() {
                    click = Some(PopoverClick::Open(href.to_owned()));
                }
                let palette = exo_ui::palette(&ctx);
                if command {
                    let keytip = Rect::from_min_size(
                        pos2(right + GAP + KEYTIP_MARGIN, text.y),
                        Vec2::splat(KEYTIP_SIZE),
                    );
                    let response = ui
                        .interact(keytip, id.with("keytip"), Sense::CLICK)
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    let opacity = ctx.animate_value_with_time(
                        id.with("keytip_opacity"),
                        if response.hovered() { 1.0 } else { 0.6 },
                        0.15,
                    );
                    let background = match palette.theme {
                        exo_ui::Theme::Snow => Color32::from_white_alpha(38),
                        exo_ui::Theme::Carbon => Color32::from_rgb(0x55, 0x5b, 0x64),
                    };
                    ui.painter().rect_filled(
                        keytip,
                        CornerRadius::same(3),
                        background.gamma_multiply(opacity),
                    );
                    let galley = POPOVER_TEXT.single_line(
                        ui.painter(),
                        "K",
                        Color32::WHITE.gamma_multiply(opacity),
                        None,
                    );
                    let width = galley.size().x;
                    exo_ui::paint_text(
                        ui.painter(),
                        pos2(keytip.center().x - width / 2.0, keytip.min.y),
                        galley,
                        &POPOVER_TEXT,
                    );
                    if response.clicked() {
                        click = Some(PopoverClick::Remove);
                    }
                } else {
                    let button =
                        Rect::from_min_size(pos2(right + GAP + REMOVE_MARGIN, text.y), REMOVE_SIZE);
                    let response = ui
                        .interact(button, id.with("remove"), Sense::CLICK)
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Remove link")
                    });
                    // `:active` is at once; hovering eases.
                    let opacity = if response.is_pointer_button_down_on() {
                        1.0
                    } else {
                        ctx.animate_value_with_time(
                            id.with("remove_opacity"),
                            if response.hovered() { 0.6 } else { 0.4 },
                            0.15,
                        )
                    };
                    exo_ui::paint_icon(
                        ui,
                        "legacy/DEPRECATED_discard",
                        REMOVE_ICON.translate(button.min.to_vec2()),
                        ink(palette).gamma_multiply(opacity),
                    );
                    if response.clicked() {
                        click = Some(PopoverClick::Remove);
                    }
                }
            });
        if matches!(click, Some(PopoverClick::Remove)) {
            self.remove_link();
        }
        click
    }
}

/// Where a toolbar anchors: a point under the selection.
pub(crate) fn anchor(rects: &[Rect]) -> Rect {
    rects
        .iter()
        .copied()
        .fold(Rect::NOTHING, |acc, rect| acc.union(rect))
}
