//! The compose form: the header (collapsed `Draft to …`, or the To/Cc/Bcc/From rows of chips),
//! the subject, the body editor and its `…`, and the action pane.
//!
//! Heights follow the CSS flow: the header box pulled up 12px and 4px into its neighbours, the
//! scroll area 4px below it, the body wrapper's top margin collapsing into the push-down box,
//! and the form at least 162px (a reply) or 178px (a new message) with its footer at the bottom.

use egui::{Color32, CornerRadius, Id, Rangef, Rect, Sense, Ui, UiBuilder, Vec2, pos2, vec2};
use exo_commands::{Command, Commands, Region};
use exo_editor::{EditorStyle, FormatCommand};
use exo_store::{Address, Alias};
use exo_ui::{
    ComposeKeysTooltip, Family, Font, IconButton, KeyTooltip, ShortcutTooltip, TextStyle, Theme,
};

use super::{
    Action, contacts,
    draft::{Field, List, Open, SubjectReplacement},
    recipients, signature,
};
use crate::{
    commands,
    format::{
        friendly::{self, ComposeFormat},
        names::{self, NameFormat, Person},
    },
};

/// Where a form is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Surface {
    /// Under the message it answers.
    Inline,
    /// The new-message pane.
    Pane,
    /// Popped out beside the thread or list.
    Popout,
}

/// What the form needs besides its draft.
pub(super) struct Context<'a> {
    pub aliases: &'a [Alias],
    /// The account's address, for "Me".
    pub me: &'a str,
    pub surface: Surface,
    /// The form has the keyboard.
    pub focused: bool,
    /// The address book, once read.
    pub book: Option<&'a contacts::Index>,
    /// The open thread's participants, for a reply.
    pub thread_people: Option<&'a contacts::Index>,
    /// The thread's scroll area's horizontal extent, which the tooltips under the form's
    /// controls stay inside.
    pub pin: Rangef,
    /// What the drop zone covers while files are dragged over the window: the message card the
    /// form is in, or the pop-out's column; the form itself when `None`.
    pub drop: Option<Rect>,
    /// Now, for `Send: {date}`.
    pub now: &'a jiff::Zoned,
    /// The reminder set on the draft's thread, if any: when, and whether it's `regardless`,
    /// which the Remind button shows when the draft has none of its own.
    pub thread_reminder: Option<(jiff::Timestamp, bool)>,
    /// The account's `Send + Mark Done` (`sendAndDone`).
    pub send_and_done: bool,
    /// The form's `max-height`: past it, the body scrolls between the header and the footer.
    /// `f32::INFINITY` for none.
    pub max_height: f32,
    /// Share Availability's `You can now hit ⌘ shift A…` may show, on a reply.
    pub availability_tip: bool,
}

impl Context<'_> {
    /// Where the controls' tooltips go: under them, or above them in the pop-out.
    fn place(&self) -> Place {
        if self.surface == Surface::Popout {
            Place::Above
        } else {
            Place::Below(self.pin)
        }
    }
}

/// Where a control's tooltip goes.
#[derive(Clone, Copy)]
enum Place {
    /// Under it, kept inside this extent.
    Below(Rangef),
    /// Above it.
    Above,
}

/// What happened in the form this frame.
pub(super) struct Response {
    pub actions: Vec<Action>,
    /// A press landed inside the form.
    pub clicked_inside: bool,
}

/// The rule over a reply's form: a 1px top border and 17px under it.
pub(super) const SEPARATOR: f32 = 18.0;
/// The form's `max-height: calc(100vh - 120px)`, a reply's `100vh - 116px`, of the page under
/// the tab strip and over a status bar.
pub(super) const MAX_BELOW_PAGE: f32 = 120.0;
pub(super) const REPLY_MAX_BELOW_PAGE: f32 = 116.0;

const FIELD: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 14.0, 20.0);
/// The collapsed header: 14px bold in a 24px line.
const COLLAPSED: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 14.0, 24.0);
/// The action pane's labels: 12px bold, 16px line.
const SUBMIT: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 16.0);
/// Send's done mark (Classic): ` +` at 10px (its inline block drops the leading space), 3px
/// after the label, then `padding-right: 17px` holding the check, a 14×11 background 1px down
/// at the right; `#8892b9` in both themes.
const DONE_PLUS: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 10.0, 16.0);
const DONE_GAP: f32 = 3.0;
const DONE_PADDING: f32 = 17.0;
const DONE_CHECK: Vec2 = Vec2::new(14.0, 11.0);
const DONE_INK: Color32 = Color32::from_rgb(0x88, 0x92, 0xb9);
/// The `…` toggle: 14px in a 16px line, in `sans-serif` (`span::before`), whose taller line box
/// puts it 2px lower than Exo Sans's would.
const QUOTING: TextStyle = TextStyle::new(Font::new(Family::SansSerif, 400), 14.0, 16.0);

/// The header: `margin: -12px -8px -4px; padding: 4px 8px; min-height: 40px; max-height:
/// 300px; overflow-y: auto` (border-box): past 300px its rows scroll.
const HEADER_PULL_UP: f32 = 12.0;
const HEADER_PULL_DOWN: f32 = 4.0;
const HEADER_PADDING: f32 = 4.0;
const HEADER_SIDE: f32 = 8.0;
const HEADER_MIN: f32 = 40.0;
const HEADER_MAX: f32 = 300.0;
/// The collapsed header's `margin: 2px 0 8px` around its 24px line.
const COLLAPSED_BLOCK: f32 = 2.0 + 24.0 + 8.0;
/// A recipient row: 28px lines (27px chips and their 1px margin), `margin-bottom: 4px`,
/// `top: -1px`, labels in a 50px column.
const LINE: f32 = 28.0;
const ROW_GAP: f32 = 4.0;
const LABEL_WIDTH: f32 = 50.0;
/// A chip's border box: `padding: 4px 16px 1px 6px`, a 1px border; `margin: 0 1px 1px 0`.
const CHIP_HEIGHT: f32 = 27.0;
const CHIP_EXTRA: f32 = 6.0 + 16.0 + 2.0;
const CHIP_TEXT_X: f32 = 7.0;
const CHIP_TEXT_Y: f32 = 5.0;
/// The input after the chips: the autosize field is its text's width (whole px) and 2px, in
/// `padding-left: 6px` and a 1px border; it wraps as a box 26px wider than the field.
const INPUT_BORDER_BOX: f32 = 2.0 + 6.0 + 2.0;
const INPUT_SCROLL: f32 = 2.0 + 26.0;
/// The subject: `padding-top: 14px`, a 20px line, `top: 2px`; in a reply, `margin-bottom: 8px`.
const SUBJECT_HEIGHT: f32 = 34.0;
const SUBJECT_REPLY_MARGIN: f32 = 8.0;
/// The scroll area: `margin-top: 4px; margin-left: -12px`, its scrollbar's track at
/// `right: -14px`.
const SCROLL_TOP: f32 = 4.0;
const SCROLL_LEFT: f32 = 12.0;
const SCROLLBAR_RIGHT: f32 = 14.0;
/// The box that pushes the footer down: its `margin-bottom`.
const PUSH_DOWN_GAP: f32 = 8.0;
/// The `…` toggle's box: 16px tall.
const QUOTING_HEIGHT: f32 = 16.0;
/// The attachment strip: `margin: 4px 0 4px 4px` in the scroll area (`margin-left: -12px`): 8px
/// wider than the form, its cards from 4px left of it. Its top margin collapses into the
/// push-down's 8px.
const STRIP_WIDER: f32 = 8.0;
/// The footer icons: Classic's `gap: 30px`.
const ICON_GAP: f32 = 30.0;
/// The footer: `<hr>` (new message) with `margin: 16px 0 -1px`, then the action pane's 20px top
/// margin, 16px line and 16px bottom margin.
const FOOTER_REPLY: f32 = 52.0;
const FOOTER_PANE: f32 = 68.0;
/// The header buttons: Classic's `padding: 4px 8px 7px` around the 12px action, 28×23 boxes
/// floated right and moved 8px into the padding (`right: -8px`), `top: 1px`, `3px` with the
/// header expanded.
const BUTTON: f32 = 28.0;
const BUTTON_HEIGHT: f32 = 23.0;
/// A suggestion's name and address: 13px in 28px lines, padded `4px 16px 0`; the name column
/// at least 165px, padding included; the list padded 11px, 8px under the input (offset `2 10`,
/// Classic's `margin: 8px 0 0`); `font-kerning: none`.
const SUGGESTION: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 13.0, 28.0);
const SUGGESTION_ROW: f32 = 32.0;
const SUGGESTION_PADDING: f32 = 11.0;
const SUGGESTION_CELL_PADDING: f32 = 16.0;
const SUGGESTION_NAME_MIN: f32 = 165.0;

/// The form's vertical metrics for one surface.
struct Metrics {
    padding_top: f32,
    min_height: f32,
    wrapper_margin: f32,
    wrapper_min: f32,
    push_min: f32,
    footer: f32,
}

impl Metrics {
    fn of(surface: Surface, open: &Open) -> Self {
        match surface {
            // A snippet's body wrapper: `margin-top: 6px`.
            Surface::Pane if open.is_snippet() => Self {
                padding_top: 2.0,
                min_height: 178.0,
                wrapper_margin: 6.0,
                wrapper_min: 48.0,
                push_min: 196.0,
                footer: FOOTER_PANE,
            },
            Surface::Pane => Self {
                padding_top: 2.0,
                min_height: 178.0,
                wrapper_margin: 20.0,
                wrapper_min: 48.0,
                push_min: 196.0,
                footer: FOOTER_PANE,
            },
            // Not the new-message pane: a reply keeps the reply's tighter body.
            Surface::Popout if !open.is_new_email() => Self {
                padding_top: 0.0,
                min_height: 162.0,
                wrapper_margin: if open.state.header_expanded && open.state.cc_bcc_shown {
                    12.0
                } else {
                    0.0
                },
                wrapper_min: 32.0,
                push_min: 196.0,
                footer: FOOTER_PANE,
            },
            Surface::Popout => Self {
                padding_top: 0.0,
                min_height: 178.0,
                wrapper_margin: 20.0,
                wrapper_min: 48.0,
                push_min: 196.0,
                footer: FOOTER_PANE,
            },
            // A reply with Cc, Bcc and From showing: the body wrapper's `margin-top: 12px`.
            Surface::Inline => Self {
                padding_top: 0.0,
                min_height: 162.0,
                wrapper_margin: if open.state.header_expanded && open.state.cc_bcc_shown {
                    12.0
                } else {
                    0.0
                },
                wrapper_min: 32.0,
                push_min: 32.0,
                footer: FOOTER_REPLY,
            },
        }
    }
}

/// One recipient row as laid out: where its chips and input go.
struct Row {
    /// `None` for the From row.
    list: Option<List>,
    top: f32,
    lines: usize,
    chips: Vec<(usize, Rect, String)>,
    /// Where the input is: after the chips, in place of the chip being edited, or nowhere while
    /// chips are selected.
    input: Option<Rect>,
}

/// Where everything goes, relative to the form's top left.
struct Layout {
    header_top: f32,
    /// The header's box, and how tall its content is, padding included (more when it scrolls).
    header_height: f32,
    header_content: f32,
    rows: Vec<Row>,
    buttons: Vec<Button>,
    subject_top: Option<f32>,
    body_top: f32,
    body_height: f32,
    quote_top: Option<f32>,
    /// The attachment strip's top and its cards' size, when there are cards.
    attachments: Option<(f32, Vec2, usize)>,
    footer_top: f32,
    height: f32,
    /// The scroll area, when the form is at its max height.
    scrolled: Option<Scrolled>,
}

/// The scroll area with less room than its content: its top and bottom, relative to the
/// form, and how tall the body, `…` and files in it are. The body, `…` and files are laid out
/// unscrolled.
#[derive(Clone, Copy)]
struct Scrolled {
    top: f32,
    bottom: f32,
    content: f32,
}

/// One of the form's scroll areas, the body's while the form is at its max height or the header's
/// past 300px: how far it has scrolled, where the body's caret last was, and its scrollbar.
#[derive(Default)]
pub(crate) struct BodyScroll {
    offset: f32,
    /// The scroll area on screen as last drawn, its track included, and how far it scrolls.
    area: Option<(Rect, f32)>,
    /// Relative to the editor's top left (the body's).
    caret: Option<Rect>,
    /// The caret moved, or the body changed (the header's: its input took the keyboard or was
    /// typed into): it comes into view.
    reveal: bool,
    bar: exo_ui::Scrollbar,
}

/// A header button.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Button {
    /// The caret that shows Cc, Bcc and From.
    Expand,
    /// Pop out, or (popped out) pop in.
    Pop,
    /// Close the pop-out.
    Close,
}

impl Button {
    /// Its box: Close's `padding-left: 7px` makes it a pixel narrower.
    fn width(self) -> f32 {
        if self == Self::Close {
            BUTTON - 1.0
        } else {
            BUTTON
        }
    }

    /// The field it is when it has the keyboard.
    fn field(self) -> Field {
        match self {
            Self::Expand => Field::ExpandRecipients,
            Self::Pop => Field::PopAction,
            Self::Close => Field::PopClose,
        }
    }
}

/// The header buttons the form shows, in order; a snippet doesn't pop out.
fn buttons(open: &Open, surface: Surface) -> Vec<Button> {
    let mut buttons = if open.state.header_expanded && !open.state.cc_bcc_shown {
        vec![Button::Expand]
    } else {
        Vec::new()
    };
    if open.is_snippet() {
        return buttons;
    }
    buttons.push(Button::Pop);
    if surface == Surface::Popout {
        buttons.push(Button::Close);
    }
    buttons
}

/// A control's tooltip box starts this far under its 32px box (measured: Send, Attach,
/// Discard), and a header button's this far under its own.
const CONTROL_TOOLTIP_GAP: f32 = 9.0;
/// Above it, in the pop-out, the box's bottom is this far below its 32px box's top (measured:
/// Send, `{ }`, Discard).
const CONTROL_TOOLTIP_ABOVE: f32 = -7.0;
const HEADER_TOOLTIP_GAP: f32 = 8.0;

fn layout(ui: &Ui, open: &mut Open, width: f32, left: f32, cx: &Context<'_>) -> Layout {
    let metrics = Metrics::of(cx.surface, open);
    let ctx = ui.ctx();
    let header_top = metrics.padding_top - HEADER_PULL_UP;
    let snippet_pad = if open.is_snippet() && open.state.header_expanded {
        4.0
    } else {
        0.0
    };
    let content_top = header_top + HEADER_PADDING + snippet_pad;
    let buttons = buttons(open, cx.surface);
    // The buttons float right (`top: 3px` expanded and `right: -8px` are relative: they leave
    // its place). A list is a flex container, so the float narrows all of the one beside it,
    // not just its lines beside it.
    let float_bottom = content_top + BUTTON_HEIGHT;
    let float_width: f32 = buttons.iter().map(|button| button.width()).sum();
    let mut rows = Vec::new();
    let content_height = if open.state.header_expanded {
        let mut y = content_top - 1.0;
        let mut lists: Vec<Option<List>> = vec![Some(List::To)];
        if open.state.cc_bcc_shown {
            lists.extend([Some(List::Cc), Some(List::Bcc)]);
            if active_aliases(cx.aliases) > 1 {
                lists.push(None);
            }
        }
        for list in lists {
            let row = lay_out_row(
                ctx,
                open,
                list,
                y,
                if y < float_bottom {
                    width - float_width
                } else {
                    width
                },
            );
            y += row.lines as f32 * LINE + ROW_GAP;
            rows.push(row);
        }
        y + 1.0 - content_top
    } else if open.is_snippet() {
        // No `Draft to …` for a snippet.
        0.0
    } else {
        COLLAPSED_BLOCK
    };
    // A snippet's header has no `min-height`; expanded, `padding-top: 8px`.
    let header_content = if open.is_snippet() {
        content_height + 2.0 * HEADER_PADDING + snippet_pad
    } else {
        (content_height + 2.0 * HEADER_PADDING).max(HEADER_MIN)
    };
    let header_height = header_content.min(HEADER_MAX);
    let mut y = header_top + header_height - HEADER_PULL_DOWN;
    // A snippet's subject shows with its recipients.
    let subject_top = if open.is_snippet() && !open.state.header_expanded {
        None
    } else if open.is_new_email() {
        let top = y;
        y += SUBJECT_HEIGHT;
        Some(top)
    } else if open.state.header_expanded && open.state.cc_bcc_shown {
        let top = y;
        y += SUBJECT_HEIGHT + SUBJECT_REPLY_MARGIN;
        Some(top)
    } else {
        None
    };
    y += SCROLL_TOP;
    let scroll_top = y;
    let body_top = y + metrics.wrapper_margin;
    let editor = open.editor.height(ui, width, left);
    open.body_height = editor;
    let body_height = editor.max(metrics.wrapper_min);
    let quote_top = open
        .has_hidden_quote(cx.aliases)
        .then_some(body_top + body_height);
    let quote_height = if quote_top.is_some() {
        QUOTING_HEIGHT
    } else {
        0.0
    };
    y = body_top + metrics.push_min.max(body_height + quote_height) + PUSH_DOWN_GAP;
    let cards = listed(open).count();
    let attachments = (cards > 0).then(|| {
        let (card, per_row) = exo_ui::attachment_grid(width + STRIP_WIDER);
        let top = y;
        let strip = cards.div_ceil(per_row) as f32 * (card.y + exo_ui::CARD_MARGIN_BOTTOM);
        // The strip's bottom margin snaps it to the 4px grid.
        y = top + strip + 12.0 - strip % 4.0;
        (top, card, per_row)
    });
    let natural = (y + metrics.footer).max(metrics.min_height);
    // Taller than its `max-height` (a `min-height` wins over it), the scroll area takes what's
    // left between the header and the footer.
    let height = natural.min(cx.max_height).max(metrics.min_height);
    let scrolled = (height < natural).then_some(Scrolled {
        top: scroll_top,
        bottom: height - metrics.footer,
        content: y - scroll_top,
    });
    Layout {
        header_top,
        header_height,
        header_content,
        rows,
        buttons,
        subject_top,
        body_top,
        body_height,
        quote_top,
        attachments,
        footer_top: height - metrics.footer,
        height,
        scrolled,
    }
}

/// Lays out one row's chips and input, wrapping like `flex-wrap`.
fn lay_out_row(
    ctx: &egui::Context,
    open: &Open,
    list: Option<List>,
    top: f32,
    line_width: f32,
) -> Row {
    let mut chips = Vec::new();
    let mut line = 0;
    let mut x = LABEL_WIDTH;
    let line_top = |line: usize| top + line as f32 * LINE;
    let (addresses, text, editing, selecting): (Vec<Address>, &str, Option<usize>, bool) =
        match list {
            Some(list) => {
                let input = open.input(list);
                (
                    open.list(list).clone(),
                    input.text.as_str(),
                    input.editing,
                    !input.selected.is_empty(),
                )
            }
            None => (Vec::new(), "", None, false),
        };
    let text_width = FIELD.width(ctx, text).round();
    let input_width = text_width + INPUT_SCROLL;
    let input_size = vec2(text_width + INPUT_BORDER_BOX, CHIP_HEIGHT);
    let mut input = None;
    for (index, address) in addresses.iter().enumerate() {
        // The chip being edited is the input, as wide as its text.
        let label = chip_label(address);
        let chip_width = if editing == Some(index) {
            input_width
        } else {
            FIELD.width(ctx, &label) + CHIP_EXTRA
        };
        if x + chip_width > line_width && x > LABEL_WIDTH {
            line += 1;
            x = LABEL_WIDTH;
        }
        if editing == Some(index) {
            input = Some(Rect::from_min_size(pos2(x, line_top(line)), input_size));
            x += chip_width;
            continue;
        }
        chips.push((
            index,
            Rect::from_min_size(pos2(x, line_top(line)), vec2(chip_width, CHIP_HEIGHT)),
            label,
        ));
        x += chip_width + 1.0;
    }
    if list.is_some() && editing.is_none() && !selecting {
        if x + input_width > line_width && x > LABEL_WIDTH {
            line += 1;
            x = LABEL_WIDTH;
        }
        // An empty list's input, nothing typed, sits 1px higher (`top: -1px`).
        let nudge = if addresses.is_empty() && text.is_empty() {
            -1.0
        } else {
            0.0
        };
        input = Some(Rect::from_min_size(
            pos2(x, line_top(line) + nudge),
            input_size,
        ));
    }
    Row {
        list,
        top,
        lines: line + 1,
        chips,
        input,
    }
}

/// A chip's text: the name, else the lower-cased address, else what was typed.
fn chip_label(address: &Address) -> String {
    match &address.name {
        Some(name) if !name.is_empty() => name.clone(),
        _ if address.email.is_empty() => String::new(),
        _ => address.email.to_lowercase(),
    }
}

fn active_aliases(aliases: &[Alias]) -> usize {
    aliases.iter().filter(|alias| alias.active).count()
}

/// The form's height at `width`.
pub(super) fn height(ui: &Ui, open: &mut Open, width: f32, cx: &Context<'_>) -> f32 {
    layout(ui, open, width, 0.0, cx).height
}

/// The inline form's separator: `border-top: 1px solid #ededed` (dark `#5f6165`), literals.
pub(super) fn separator(ui: &Ui, left: f32, top: f32, width: f32) {
    let color = if exo_ui::theme(ui.ctx()) == Theme::Carbon {
        Color32::from_rgb(0x5f, 0x61, 0x65)
    } else {
        Color32::from_rgb(0xed, 0xed, 0xed)
    };
    ui.painter().rect_filled(
        exo_ui::pixel_snap(Rect::from_min_size(pos2(left, top), vec2(width, 1.0))),
        0.0,
        color,
    );
}

/// Colours the form takes as Classic literals rather than from the palette.
struct Ink {
    dark: bool,
}

impl Ink {
    /// `rgba(0,0,0,a)`, or white at `a` in dark themes.
    fn text(&self, alpha: f32) -> Color32 {
        let value = (alpha * 255.0).round() as u8;
        if self.dark {
            Color32::from_white_alpha(value)
        } else {
            Color32::from_black_alpha(value)
        }
    }
}

/// Draws the form at `rect`'s top left, `rect.width()` wide, and handles its input.
pub(super) fn show(ui: &mut Ui, open: &mut Open, rect: Rect, cx: &Context<'_>) -> Response {
    let ctx = ui.ctx().clone();
    let now = ctx.input(|input| input.time);
    let width = rect.width();
    let origin = rect.min;
    let layout = layout(ui, open, width, origin.x, cx);
    let form = Rect::from_min_size(origin, vec2(width, layout.height));
    let palette = exo_ui::palette(&ctx);
    let ink = Ink {
        dark: exo_ui::theme(&ctx) == Theme::Carbon,
    };
    let mut response = Response {
        actions: Vec::new(),
        clicked_inside: false,
    };
    let pressed_at = ctx.input(|input| {
        input
            .pointer
            .any_pressed()
            .then(|| input.pointer.press_origin())
            .flatten()
    });
    response.clicked_inside =
        pressed_at.is_some_and(|at| form.contains(at) && ui.clip_rect().contains(at));
    let base = exo_ui::id(&ctx, ("compose", open.id.clone()));
    // On a field nothing is typed into (a button, From, a reply's subject, the `…`), no text
    // widget keeps the keyboard: keys are the field's commands.
    if cx.focused && !open.takes_text() {
        ctx.memory_mut(|memory| {
            if let Some(focused) = memory.focused() {
                memory.surrender_focus(focused);
            }
        });
    }

    // A snippet's name, over the pane's header (`position: absolute; padding-top: 16px;
    // margin-top: -44px`, its line 38px above the form).
    if let Some(snippet) = &mut open.snippet {
        let spelling = &mut open.name_spelling;
        let id = base.with("snippet-name");
        let line = Rect::from_min_size(pos2(origin.x, origin.y - 38.0), vec2(width + 27.0, 20.0));
        if snippet.name.is_empty() {
            FIELD.paint_line(
                ui.painter(),
                line.min,
                "Enter description",
                ink.text(0.5),
                None,
            );
        }
        let before = snippet.name.clone();
        let field = spelling.show(
            ui,
            id,
            &mut snippet.name,
            line,
            &FIELD,
            ink.text(0.8),
            palette.text_selection_default,
        );
        let changed = snippet.name != before;
        if field.gained_focus() || field.clicked() {
            response.actions.push(Action::Focus(Field::SnippetName));
        }
        focus_if(ui, cx, open.state.field == Field::SnippetName, id);
        if changed {
            open.touch(now);
        }
    }

    // Header: past its max height its rows (and buttons) scroll within its box.
    let header_box = Rect::from_min_size(
        pos2(origin.x - HEADER_SIDE, origin.y + layout.header_top),
        vec2(width + 2.0 * HEADER_SIDE, layout.header_height),
    );
    // What comes into view is the input's caret: its text's line, down to its content area's
    // bottom (measured: 23px into the input).
    let caret_bottom = CHIP_TEXT_Y + FIELD.content_bottom(&ctx);
    let input = layout
        .rows
        .iter()
        .find(|row| {
            row.list
                .is_some_and(|list| cx.focused && open.state.field == list.field())
        })
        .and_then(|row| row.input)
        .map(|input| {
            Rect::from_x_y_ranges(
                input.x_range(),
                input.top() + CHIP_TEXT_Y..=input.top() + caret_bottom,
            )
        });
    let header_scroll = scroll_header(ui, open, &layout, header_box, input);
    let mut header_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(header_box)
            .id_salt(("compose-header", &open.id)),
    );
    if header_scroll.is_some() {
        header_ui.set_clip_rect(header_box.intersect(ui.clip_rect()));
    }
    let header_origin = origin - vec2(0.0, header_scroll.unwrap_or(0.0));
    let content_top = header_origin.y + layout.header_top + HEADER_PADDING;
    if open.state.header_expanded {
        for row in &layout.rows {
            show_row(
                &mut header_ui,
                open,
                row,
                header_origin,
                base,
                cx,
                &ink,
                &mut response,
                now,
            );
        }
        // A drag lasts while the button's down; let go off a list, nothing moves.
        if ctx.input(|input| input.pointer.primary_down()) {
            if let Some(drag) = &open.chip_drag {
                paint_drag_image(ui, drag, &ink);
            }
        } else {
            open.chip_drag = None;
        }
    } else if !open.is_snippet() {
        let line_top = content_top + 2.0;
        // Clicking it (or Enter on it) expands the header, To focused.
        let header = collapsed_header(
            ui,
            open,
            pos2(origin.x, line_top),
            width - BUTTON,
            cx,
            &ink,
            true,
            base.with("collapsed"),
        );
        if header.clicked() {
            response.actions.push(Action::Focus(Field::To));
        }
    }
    let mut right = origin.x + width + 8.0;
    for button in layout.buttons.iter().rev() {
        let button_width = button.width();
        right -= button_width;
        // `top: 3px`, or `1px` on a collapsed header, whose line starts 2px lower.
        let hit = Rect::from_min_size(
            pos2(right, content_top + 3.0),
            vec2(button_width, BUTTON_HEIGHT),
        );
        let focused = cx.focused && open.state.field == button.field();
        if let Some(action) = header_button(
            &header_ui,
            *button,
            hit,
            cx,
            cx.surface == Surface::Popout,
            focused,
            base,
        ) {
            response.actions.push(action);
        }
    }
    if let Some(offset) = header_scroll {
        open.header_scroll.bar.show(
            ui,
            base.with("header-scrollbar"),
            header_box,
            offset,
            layout.header_content,
        );
    }

    // Subject.
    if let Some(top) = layout.subject_top {
        let line = Rect::from_min_size(
            pos2(origin.x, origin.y + top + 2.0 + 14.0),
            vec2(width - 27.0, 20.0),
        );
        if open.is_new_email() {
            let id = base.with("subject");
            if open.record.subject.is_empty() {
                FIELD.paint_line(ui.painter(), line.min, "Subject", ink.text(0.5), None);
            }
            let focused_here = cx.focused && open.state.field == Field::Subject;
            if focused_here {
                undo_subject_replacement(ui, open, id);
            }
            let typed = super::subject::typed_trigger(ui);
            let before = open.record.subject.clone();
            let field = open.subject_spelling.show(
                ui,
                id,
                &mut open.record.subject,
                line,
                &FIELD,
                ink.text(0.8),
                palette.text_selection_default,
            );
            // Text dragged from a body or another app goes in where it's let go, as an `<input>`
            // takes a drop.
            if exo_ui::text_drag::field(
                ui,
                id,
                &mut open.record.subject,
                line,
                &FIELD,
                ink.text(0.8),
            )
            .is_some()
            {
                response.actions.push(Action::Focus(Field::Subject));
            }
            if open.record.subject != before {
                replace_in_subject(ui, open, id, &before);
                open.touch(now);
            }
            if focused_here {
                super::subject::update(ui, open, id, line, &FIELD, typed);
            } else {
                open.subject_assistant.trigger = None;
            }
            if field.gained_focus() || field.clicked() {
                response.actions.push(Action::Focus(Field::Subject));
            }
            focus_if(ui, cx, open.state.field == Field::Subject, id);
            if focused_here && std::mem::take(&mut open.select_subject) {
                let all = egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(open.record.subject.chars().count()),
                );
                exo_ui::focus_field(ui.ctx(), id, all);
            }
        } else {
            // A reply's subject reads only: clicking it, or Enter on it, starts a new thread.
            // Focused, it's underlined (`box-shadow: 0 2px`).
            FIELD.paint_line(
                ui.painter(),
                line.min,
                &open.record.subject,
                ink.text(0.8),
                Some(line.width()),
            );
            let subject = Rect::from_min_size(
                pos2(origin.x, origin.y + top + 2.0),
                vec2(width, SUBJECT_HEIGHT),
            );
            if cx.focused && open.state.field == Field::Subject {
                underline(
                    ui,
                    subject.left(),
                    subject.right(),
                    subject.bottom(),
                    palette.border_base_active,
                );
            }
            if ui
                .interact(subject, base.with("subject"), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                response.actions.push(Action::EditSubject);
            }
        }
    }

    // Body, at its scroll, which the `…` and the files share.
    let viewport = match layout.scrolled {
        Some(scrolled) => Some(scroll_body(ui, open, &layout, scrolled, origin, width)),
        // Room enough: nothing to scroll (`scrollTop` goes back to 0).
        None => {
            open.body_scroll = BodyScroll::default();
            None
        }
    };
    let scroll = if viewport.is_some() {
        open.body_scroll.offset
    } else {
        0.0
    };
    let mut scrolled_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(form)
            .id_salt(("compose-scrolled", &open.id)),
    );
    if let Some(viewport) = viewport {
        scrolled_ui.set_clip_rect(viewport.intersect(ui.clip_rect()));
    }
    let body = Rect::from_min_size(
        pos2(origin.x, origin.y + layout.body_top - scroll),
        vec2(width, layout.body_height),
    );
    let placeholder = placeholder(open, cx);
    let mut body_ui = scrolled_ui.new_child(
        UiBuilder::new()
            .max_rect(body)
            .id_salt(("compose-body", &open.id)),
    );
    if cx.focused
        && open.state.field == Field::Body
        && ui.input(|input| {
            input.key_pressed(egui::Key::Backspace)
                || input.modifiers.command && input.key_pressed(egui::Key::Z)
        })
    {
        super::assistant::edit_mention(open, cx.book);
    }
    let edited = open.editor.ui(
        &mut body_ui,
        &EditorStyle {
            placeholder,
            min_height: layout.body_height,
        },
    );
    if edited.changed {
        open.touch(now);
    }
    if edited.response.clicked() || edited.response.gained_focus() || edited.dropped {
        response.actions.push(Action::Focus(Field::Body));
    }
    if edited.escaped {
        response.actions.push(Action::Escape);
    }
    if edited.past_end && layout.quote_top.is_some() {
        response.actions.push(Action::Focus(Field::QuotingStart));
    }
    response.actions.extend(edited.toolbar.map(Action::Toolbar));
    response.actions.extend(edited.learned.map(Action::Learned));
    response
        .actions
        .extend(edited.open_link.map(Action::OpenLink));
    // A date the caret comes onto, or whose text changes under it, shows its day.
    if edited.date != open.date {
        response.actions.extend(
            edited
                .date
                .as_ref()
                .map(|date| Action::DateActive(date.text.clone())),
        );
        open.date = edited.date.clone();
    }
    open.caret = edited.caret;
    if viewport.is_some() {
        follow_caret(
            ui,
            open,
            edited.response.has_focus().then_some(edited.caret),
            edited.changed,
            body.min,
        );
    }
    if let Some(menu) = edited.context_menu {
        // Next frame, once the word it selected is on screen: the menu waits for the user.
        open.context_menu = Some(menu);
        response.actions.push(Action::Focus(Field::Body));
        ctx.request_repaint();
    }
    focus_if(ui, cx, open.state.field == Field::Body, edited.response.id);
    // The assistant is the subject's while it has the keyboard, else the body's.
    let in_subject = open.state.field == Field::Subject;
    let assistant_trigger = if in_subject {
        open.subject_assistant.trigger.clone()
    } else {
        edited.trigger.clone()
    };
    // The `;` picker (the assistant's snippet mode) under the `;`.
    let trigger = assistant_trigger
        .clone()
        .filter(|trigger| trigger.kind == exo_editor::TriggerKind::Snippet);
    if trigger != open.snippet_trigger {
        // Its rows follow next frame.
        ctx.request_repaint();
    }
    open.snippet_trigger = trigger;
    // The `:` emoji and `+`/`@`/`-` contact dropdowns; a mention's recipients follow its undo.
    super::assistant::note_trigger(open, assistant_trigger.as_ref());
    super::assistant::undo_marks(open, &edited.undone, &edited.redone);
    if cx.focused
        && let Some(picked) = super::assistant::show(ui, open, cx.pin, base.with("assistant"))
    {
        response
            .actions
            .push(Action::Assistant(super::assistant::Key::Pick(Some(picked))));
    }
    if cx.focused
        && let Some(trigger) = &open.snippet_trigger
        && (!open.snippet_rows.is_empty() || open.snippet_hint)
        && open.date.is_none()
        && let Some(picked) = snippet_picker(
            ui,
            open,
            trigger.anchor,
            in_subject,
            width,
            base.with("snippet-picker"),
        )
    {
        response.actions.push(Action::SnippetPick(Some(picked)));
    }
    // Share Availability's menu where the caret was when it opened, its row while what's typed
    // since matches it; a press elsewhere closes it.
    if cx.focused
        && let Some(menu) = &mut open.availability_menu
    {
        if !super::availability::follow(&open.editor, menu, edited.changed) {
            response.actions.push(Action::AvailabilityMenu(false));
        } else if menu.shows_row() {
            let anchor = *menu.anchor.get_or_insert(edited.caret);
            let (picked, card) =
                super::availability::show(ui, anchor, base.with("availability-menu"));
            if picked {
                response.actions.push(Action::AvailabilityPick);
            } else if ctx.input(|input| input.pointer.any_pressed())
                && !ctx
                    .pointer_interact_pos()
                    .is_some_and(|at| card.contains(at))
            {
                response.actions.push(Action::AvailabilityDismiss);
            }
        }
    }

    // The `…`: reveals the quote and signature.
    if let Some(top) = layout.quote_top.map(|top| top - scroll) {
        let ui = &scrolled_ui;
        let id = base.with("quoting");
        let hit = Rect::from_min_size(pos2(origin.x - 4.0, origin.y + top), vec2(24.0, 24.0));
        let hovered_or_focused = cx.focused && open.state.field == Field::QuotingStart;
        let toggle = ui
            .interact(hit, id, Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let shift = exo_ui::motion::transition(
            &ctx,
            id.with("hover"),
            toggle.hovered() || hovered_or_focused,
            0.1,
            exo_ui::motion::CubicBezier::EASE,
        );
        if hovered_or_focused {
            // The focus ring: a 22×12 pill, `0 0 6px 1px #aeb1dd`.
            let ring = Rect::from_min_size(
                pos2(
                    origin.x + 4.0 * shift - 4.0 + 4.0,
                    origin.y + top + 6.0 - 4.0,
                ),
                vec2(22.0, 12.0),
            );
            exo_ui::BoxShadow::new([0.0, 0.0], 6.0, 1.0, Color32::from_rgb(0xae, 0xb1, 0xdd))
                .paint(ui.painter(), ring, CornerRadius::same(2));
        }
        let color = ink.text(0.75).gamma_multiply(0.4);
        QUOTING.paint_line(
            ui.painter(),
            pos2(origin.x + 4.0 * shift, origin.y + top),
            "…",
            color,
            None,
        );
        if toggle.is_pointer_button_down_on() && toggle.drag_started() || toggle.clicked() {
            response.actions.push(Action::ClickRevealQuote);
        }
    }

    if let Some((top, card, per_row)) = layout
        .attachments
        .map(|(top, card, per_row)| (top - scroll, card, per_row))
    {
        let ui = &scrolled_ui;
        let cards: Vec<(String, String, String, Option<f32>)> = listed(open)
            .map(|attachment| {
                (
                    attachment.uuid.clone(),
                    attachment.name.clone(),
                    attachment.mime_type.clone(),
                    uploading(open, attachment),
                )
            })
            .collect();
        for (at, (uuid, name, mime_type, upload)) in cards.into_iter().enumerate() {
            let (row, column) = (at / per_row, at % per_row);
            let min = pos2(
                origin.x - STRIP_WIDER
                    + exo_ui::CARD_MARGIN_X
                    + column as f32 * (card.x + 2.0 * exo_ui::CARD_MARGIN_X),
                origin.y + top + row as f32 * (card.y + exo_ui::CARD_MARGIN_BOTTOM),
            );
            let upload = upload.map(|progress| Uploading {
                progress,
                online: open.online,
            });
            match attachment_card(
                ui,
                Rect::from_min_size(min, card),
                &name,
                &mime_type,
                upload,
                base.with(("attachment", &uuid)),
            ) {
                Some(CardClick::Remove) => response.actions.push(Action::RemoveAttachment(uuid)),
                Some(CardClick::Open { preview }) => response
                    .actions
                    .push(Action::OpenAttachment { uuid, preview }),
                None => {}
            }
        }
    }

    if let Some((viewport, content)) =
        viewport.zip(layout.scrolled.map(|scrolled| scrolled.content))
    {
        open.body_scroll.bar.show(
            ui,
            base.with("scrollbar"),
            viewport.with_max_x(viewport.right() + SCROLLBAR_RIGHT),
            scroll,
            content,
        );
    }

    // Footer: the rule (new message), Send, and the icons.
    let mut pane_top = origin.y + layout.footer_top + 20.0;
    if matches!(cx.surface, Surface::Pane | Surface::Popout) {
        let rule = Rect::from_min_size(
            pos2(origin.x, origin.y + layout.footer_top + 16.0),
            vec2(width, 1.0),
        );
        let color = if ink.dark {
            Color32::from_rgb(0x5f, 0x61, 0x65)
        } else {
            Color32::from_rgb(0xed, 0xed, 0xed)
        };
        ui.painter()
            .rect_filled(exo_ui::pixel_snap(rule), 0.0, color);
        pane_top += 16.0;
    }
    let focused_on = |field: Field| cx.focused && open.state.field == field;
    // A snippet's is `Done`; a draft with a Send later time (not one a reply called off)
    // `Schedule`.
    let scheduled = open
        .record
        .send_later
        .as_ref()
        .filter(|_| open.unscheduled_by.is_none())
        .map(|later| later.at);
    let label = if open.is_snippet() {
        "Done"
    } else if scheduled.is_some() {
        "Schedule"
    } else {
        "Send"
    };
    // Outside the new-message pane, `Send +✓` while a send would mark done: with the setting on,
    // or while ⌘⇧ are held.
    let held = cx.focused && ctx.input(|input| input.modifiers.mac_cmd && input.modifiers.shift);
    let done_icon = (cx.send_and_done || held) && cx.surface != Surface::Pane;
    let label_width = SUBMIT.width(&ctx, label);
    let send_width = if done_icon {
        label_width + DONE_GAP + DONE_PLUS.width(&ctx, "+") + DONE_PADDING
    } else {
        label_width
    };
    // Send's clickable: `padding: 10px; margin: -10px`.
    let send_hit = Rect::from_min_size(
        pos2(origin.x - 10.0, pane_top - 10.0),
        vec2(send_width + 20.0, 36.0),
    );
    let send = ui
        .interact(send_hit, base.with("send"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    SUBMIT.paint_line(
        ui.painter(),
        pos2(origin.x, pane_top + 1.0),
        label,
        ink.text(0.8),
        None,
    );
    if done_icon {
        DONE_PLUS.paint_line(
            ui.painter(),
            pos2(origin.x + label_width + DONE_GAP, pane_top + 1.0),
            "+",
            ink.text(0.8),
            None,
        );
        // The check goes on the device pixel nearest its box, half a pixel lower than its 1px
        // offset says (measured).
        let pixels = ctx.pixels_per_point();
        let left = ((origin.x + send_width - DONE_CHECK.x) * pixels).round() / pixels;
        paint_done_check(ui.painter(), pos2(left, pane_top + 2.0 + 0.5));
    }
    if focused_on(Field::Send) {
        // Focused, the label's `border-bottom: 2px solid …` under its 16px line.
        underline(
            ui,
            origin.x,
            origin.x + send_width,
            pane_top + 17.0,
            palette.border_base_active,
        );
    }
    // `Send ⌘ enter`, and for a reply `Send + Mark Done ⌘ shift enter`, or with Send + Mark
    // Done on only `Send + Mark Done ⌘ enter`; a snippet's `Done ⌘ enter`.
    let reply = matches!(
        open.record.action,
        exo_store::DraftAction::Reply | exo_store::DraftAction::ReplyAll
    );
    let send_rows = match (reply, cx.send_and_done) {
        (true, true) => vec![KeyTooltip {
            text: "Send + Mark Done",
            keys: &["⌘", "enter"],
        }],
        (true, false) => vec![
            KeyTooltip {
                text: label,
                keys: &["⌘", "enter"],
            },
            KeyTooltip {
                text: "Send + Mark Done",
                keys: &["⌘", "shift", "enter"],
            },
        ],
        (false, _) => vec![KeyTooltip {
            text: label,
            keys: &["⌘", "enter"],
        }],
    };
    match (scheduled, cx.place()) {
        // `Hit ⌘ enter to Schedule`, open while the form has the keyboard, its keys breathing
        // from .3 to 1 (`ease-in-out .4s infinite alternate`).
        (Some(_), Place::Below(within)) if cx.focused => {
            let phase = (ctx.input(|input| input.time) / 0.4) % 2.0;
            let t = if phase < 1.0 { phase } else { 2.0 - phase } as f32;
            let breath = 0.3 + 0.7 * exo_ui::motion::CubicBezier::EASE_IN_OUT.at(t);
            ctx.request_repaint();
            ComposeKeysTooltip {
                before: "Hit",
                keys: &["⌘", "enter"],
                after: " to Schedule",
            }
            .show_open(
                ui,
                base.with("schedule-tip"),
                send_hit,
                CONTROL_TOOLTIP_GAP,
                within,
                breath,
            );
        }
        (Some(_), _) => {}
        (None, place) => tooltip(
            ui,
            place,
            &send,
            focused_on(Field::Send),
            &send_rows,
            None,
            CONTROL_TOOLTIP_GAP,
        ),
    }
    if send.clicked() {
        let mark_done = !open.is_snippet()
            && ctx.input(|input| input.modifiers.command && input.modifiers.shift);
        response.actions.push(Action::ClickSend { mark_done });
    }
    if !open.is_snippet() {
        let gap = action_gap(open, width, cx, done_icon);
        let after = send_later(
            ui,
            open,
            &mut response,
            pos2(origin.x + send_width + gap, pane_top),
            width,
            cx,
            &ink,
            base,
        );
        remind_me(
            ui,
            open,
            &mut response,
            pos2(after + gap, pane_top),
            width,
            cx,
            &ink,
            base,
        );
    }
    // A collapsed snippet's `Add To, Cc, Bcc, Subject…` (`ect` only above 1150px), in Remind me's
    // place: it shows the recipients and subject.
    if open.is_snippet() && !open.state.header_expanded {
        let text = if ctx.content_rect().width() > 1150.0 {
            "Add To, Cc, Bcc, Subject…"
        } else {
            "Add To, Cc, Bcc, Subj…"
        };
        let left = origin.x + send_width + 18.0;
        let add = ui
            .interact(
                Rect::from_min_size(
                    pos2(left - 10.0, pane_top - 10.0),
                    vec2(SUBMIT.width(&ctx, text) + 20.0, 36.0),
                ),
                base.with("snippet-header"),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        SUBMIT.paint_line(
            ui.painter(),
            pos2(left, pane_top + 1.0),
            text,
            ink.text(0.35),
            None,
        );
        if add.clicked() {
            response.actions.push(Action::Focus(Field::To));
        }
    }
    // The footer icons, at their own sizes from the right, centered on a 16px line 2.5px above
    // the pane's, their clickable boxes pulled in around them.
    let line = pane_top - 2.5;
    let mut right = origin.x + width;
    let size_of = |name: &str| exo_ui::icon_size(&ctx, name).unwrap_or(vec2(16.0, 16.0));
    let mut place = |size: Vec2| {
        let rect = Rect::from_min_size(pos2(right - size.x, line + (16.0 - size.y) / 2.0), size);
        right -= size.x + ICON_GAP;
        rect
    };
    let discard = place(size_of("legacy/DEPRECATED_discard"));
    let attach = place(size_of("legacy/DEPRECATED_paperclip"));
    // Then, not on a snippet, `{ }` and Share Availability's calendar
    // (`DEPRECATED_roundedCalendar`).
    if !open.is_snippet() {
        let name = "legacy/DEPRECATED_snippet";
        let snippet = place(size_of(name));
        let hit = icon_hit(snippet);
        let button = IconButton::new(name)
            .colors(palette.icon_base_default, palette.icon_base_hover)
            .show(
                ui,
                base.with(name),
                hit,
                exo_ui::icon_rect(ui.ctx(), name, snippet),
            );
        let row = KeyTooltip {
            text: "Use Snippet Inline",
            keys: &[";"],
        };
        tooltip(
            ui,
            cx.place(),
            &button,
            false,
            std::slice::from_ref(&row),
            None,
            CONTROL_TOOLTIP_GAP,
        );
        if button.clicked() {
            response.actions.push(Action::SnippetInline);
        }
        let name = "legacy/DEPRECATED_roundedCalendar";
        // Its 14px square drawn 12px tall (`height: 12px`).
        let calendar = place(vec2(14.0, 12.0));
        let hit = icon_hit(calendar);
        if open.online {
            // On a reply, `You can now hit ⌘ shift A…` stays up in place of its tooltip while no
            // other icon is hovered, the icon lit and underlined.
            let other_hovered = [icon_hit(snippet), attach_hit(attach), icon_hit(discard)]
                .into_iter()
                .any(|hit| ui.rect_contains_pointer(hit));
            let persistent = cx.availability_tip && reply && !other_hovered;
            let button = IconButton::new(name)
                .colors(palette.icon_base_default, palette.icon_base_hover)
                .lit(persistent)
                .show(ui, base.with(name), hit, exo_ui::pixel_snap(calendar));
            if persistent {
                // Classic's 2px border under its 5px padding.
                underline(
                    ui,
                    calendar.left(),
                    calendar.right(),
                    calendar.bottom() + 5.0,
                    palette.border_base_active,
                );
                match availability_tip(ui, cx, hit, base.with("availability-tip")) {
                    Some(exo_ui::DismissableClick::Close) => {
                        response.actions.push(Action::DismissAvailabilityTip)
                    }
                    // A click on the tip opens the menu, as one on the icon does.
                    Some(exo_ui::DismissableClick::Body) => {
                        response.actions.push(Action::AvailabilityMenu(true))
                    }
                    None => {}
                }
            } else {
                let row = KeyTooltip {
                    text: "Share Availability",
                    keys: &["⌘", "shift", "A"],
                };
                tooltip(
                    ui,
                    cx.place(),
                    &button,
                    false,
                    std::slice::from_ref(&row),
                    Some("Insert a booking link into your message."),
                    CONTROL_TOOLTIP_GAP,
                );
            }
            if button.clicked() {
                response.actions.push(Action::AvailabilityMenu(true));
            }
        } else {
            // Offline it's `disabled`: its clickable takes no pointer (`pointer-events: none`), so
            // it neither lights nor clicks, and its tooltip says why.
            let anchor = ui.interact(hit, base.with(name), Sense::hover());
            exo_ui::paint_icon(
                ui,
                name,
                exo_ui::pixel_snap(calendar),
                palette.icon_base_default,
            );
            let row = KeyTooltip {
                text: "Share Availability is not available while offline.",
                keys: &[],
            };
            tooltip(
                ui,
                cx.place(),
                &anchor,
                false,
                std::slice::from_ref(&row),
                None,
                CONTROL_TOOLTIP_GAP,
            );
        }
    }
    let icons = [
        (
            (
                "legacy/DEPRECATED_paperclip",
                attach,
                attach_hit(attach),
                Field::Attach,
            ),
            KeyTooltip {
                text: "Attach",
                keys: &["⌘", "shift", "U"],
            },
            Some("(or drag 'n' drop files)"),
            Action::Attach,
        ),
        (
            (
                "legacy/DEPRECATED_discard",
                discard,
                icon_hit(discard),
                Field::Discard,
            ),
            KeyTooltip {
                text: "Discard",
                keys: &["⌘", "shift", ","],
            },
            None,
            Action::Discard,
        ),
    ];
    for ((name, icon, hit, field), row, note, action) in icons {
        let focused = focused_on(field);
        let rest = if focused {
            palette.icon_base_hover
        } else {
            palette.icon_base_default
        };
        let button = IconButton::new(name)
            .colors(rest, palette.icon_base_hover)
            .show(
                ui,
                base.with(name),
                hit,
                exo_ui::icon_rect(ui.ctx(), name, icon),
            );
        if focused {
            // Focused: `padding-bottom: 2px` and a `box-shadow: 0 2px …` under it.
            underline(
                ui,
                icon.left(),
                icon.right(),
                line + 18.0,
                palette.border_base_active,
            );
        }
        tooltip(
            ui,
            cx.place(),
            &button,
            focused,
            std::slice::from_ref(&row),
            note,
            CONTROL_TOOLTIP_GAP,
        );
        if button.clicked() {
            response.actions.push(action);
        }
    }
    if ctx.input(|input| !input.raw.hovered_files.is_empty()) {
        drop_zone(ui, cx.drop.unwrap_or(form), ink.dark);
    }
    response
}

/// A footer icon's clickable, which its tooltip hangs under: its 16px line and 8px around.
fn icon_hit(icon: Rect) -> Rect {
    Rect::from_center_size(icon.center(), vec2(icon.width(), 16.0)).expand(8.0)
}

/// Attach's: Classic's inline `padding: 10px`, 10px either side of the icon and 28px tall on its
/// line (measured), so its tooltip sits 2px higher than its neighbours'.
fn attach_hit(icon: Rect) -> Rect {
    Rect::from_center_size(icon.center(), vec2(icon.width() + 20.0, 28.0))
}

/// The scroll area on screen, the form's top left at `origin`: the wheel over it (or its track,
/// right of it) scrolls it, and a caret that moved last frame comes into view (now that the
/// layout has what moved it), within its range and to device pixels, as Chromium scrolls.
fn scroll_body(
    ui: &Ui,
    open: &mut Open,
    layout: &Layout,
    scrolled: Scrolled,
    origin: egui::Pos2,
    width: f32,
) -> Rect {
    let viewport = Rect::from_min_max(
        pos2(origin.x - SCROLL_LEFT, origin.y + scrolled.top),
        pos2(origin.x + width, origin.y + scrolled.bottom),
    );
    let editor_bottom = layout.body_top + open.body_height;
    let scroll = &mut open.body_scroll;
    if ui.rect_contains_pointer(viewport.with_max_x(viewport.right() + SCROLLBAR_RIGHT)) {
        scroll.offset -= ui.input(|input| input.smooth_scroll_delta.y);
    }
    if let Some(caret) = scroll.caret.filter(|_| std::mem::take(&mut scroll.reveal)) {
        scroll.offset += reveal(
            caret.translate(vec2(0.0, layout.body_top)),
            editor_bottom,
            scrolled,
            scroll.offset,
        );
    }
    let pixels = ui.pixels_per_point();
    let most = (scrolled.content - viewport.height()).max(0.0);
    scroll.offset = ((scroll.offset * pixels).round() / pixels).clamp(0.0, most);
    scroll.area = Some((
        viewport.with_max_x(viewport.right() + SCROLLBAR_RIGHT),
        most,
    ));
    viewport
}

/// The header's rows past its max height, its box `area` on screen: the wheel over it scrolls
/// them, and the focused recipient input's caret line (`input`, relative to the form) comes into
/// view at its nearest edge as the input takes the keyboard and as it's typed into, as Chromium
/// reveals a field's caret; a comma or Enter making a chip (no text typed) moves it unrevealed.
/// Returns how far they've scrolled, when they can.
fn scroll_header(
    ui: &Ui,
    open: &mut Open,
    layout: &Layout,
    area: Rect,
    input: Option<Rect>,
) -> Option<f32> {
    let most = layout.header_content - layout.header_height;
    let scroll = &mut open.header_scroll;
    if most <= 0.0 {
        *scroll = BodyScroll::default();
        return None;
    }
    if ui.rect_contains_pointer(area) {
        scroll.offset -= ui.input(|input| input.smooth_scroll_delta.y);
    }
    if let Some(shown) = input.filter(|_| std::mem::take(&mut scroll.reveal)) {
        let (top, bottom) = (
            layout.header_top + scroll.offset,
            layout.header_top + layout.header_height + scroll.offset,
        );
        if shown.top() < top {
            scroll.offset -= top - shown.top();
        } else if shown.bottom() > bottom {
            scroll.offset += shown.bottom() - bottom;
        }
    }
    let pixels = ui.pixels_per_point();
    scroll.offset = ((scroll.offset * pixels).round() / pixels).clamp(0.0, most);
    scroll.area = Some((area, most));
    Some(scroll.offset)
}

impl BodyScroll {
    /// Whether the wheel at `pointer`, moving the content `delta` down, scrolls this area
    /// rather than what it's in: it's over the area as last drawn, which can still scroll that
    /// way (Chromium's scroll chaining: at its end, the wheel goes on to the thread).
    pub(crate) fn takes_wheel(&self, pointer: egui::Pos2, delta: f32) -> bool {
        self.area.is_some_and(|(area, most)| {
            area.contains(pointer)
                && if delta > 0.0 {
                    self.offset > 0.0
                } else {
                    delta < 0.0 && self.offset < most
                }
        })
    }
}

/// How far to scroll to show `caret` (relative to the form, unscrolled) in the scroll area
/// scrolled to `offset`: Chromium's `CenterIfNeeded` (`RevealSelection`), a caret partly out
/// of view coming in at its nearest edge, one out of view to the middle. On the body's last
/// line what's after it (the `…`, the files) comes into view with it, so typing at the end
/// keeps the area scrolled to its bottom.
fn reveal(caret: Rect, editor_bottom: f32, scrolled: Scrolled, offset: f32) -> f32 {
    let last_line = caret.bottom() + caret.height() > editor_bottom;
    let shown = if last_line {
        caret.with_max_y(scrolled.top + scrolled.content)
    } else {
        caret
    };
    let shown = if shown.height() > scrolled.bottom - scrolled.top {
        caret
    } else {
        shown
    };
    let (top, bottom) = (scrolled.top + offset, scrolled.bottom + offset);
    if shown.top() >= top && shown.bottom() <= bottom {
        0.0
    } else if shown.bottom() <= top || shown.top() >= bottom {
        shown.center().y - (top + bottom) / 2.0
    } else if shown.top() < top {
        shown.top() - top
    } else {
        shown.bottom() - bottom
    }
}

/// Notes where the caret is in the body (`body` is the editor's top left on screen), while
/// the editor has the keyboard: one that moved, or an edit, comes into view next frame
/// ([`reveal`]). The wheel leaves it out of view.
fn follow_caret(ui: &Ui, open: &mut Open, caret: Option<Rect>, changed: bool, body: egui::Pos2) {
    let caret = caret.map(|caret| caret.translate(-body.to_vec2()));
    let scroll = &mut open.body_scroll;
    if caret.is_some() && (changed || scroll.caret != caret) {
        scroll.reveal = true;
        ui.ctx().request_repaint();
    }
    scroll.caret = caret;
}

/// From one action's text to the next (Send, Send later, Remind me): the clickable's `padding:
/// 10px; margin: -10px` and its `margin-right`, then the next one's padding. The margin is
/// 22px, 12px in an action pane up to 480px wide, or a tighter spacing's (`!important`): popped
/// out, 3px with a reminder and no send time while a send marks done, else 5px; elsewhere 5px
/// while a send marks done with either date set. `marks_done` is the done icon's condition.
fn action_gap(open: &Open, width: f32, cx: &Context<'_>, marks_done: bool) -> f32 {
    let reminder = shown_reminder(open, cx).is_some();
    let send_at = open.record.send_later.is_some();
    let margin = if cx.surface == Surface::Popout {
        if reminder && !send_at && marks_done {
            3.0
        } else {
            5.0
        }
    } else if marks_done && (reminder || send_at) {
        5.0
    } else if width <= 480.0 {
        12.0
    } else {
        22.0
    };
    10.0 + margin - 10.0 + 10.0
}
/// `Unscheduled: {name} replied`'s `#ce9ece`, Snow and Carbon alike.
const UNSCHEDULED_INK: Color32 = Color32::from_rgb(0xce, 0x9e, 0xce);

/// The reminder the Remind button shows: the draft's, else its thread's.
fn shown_reminder(open: &Open, cx: &Context<'_>) -> Option<(jiff::Timestamp, bool)> {
    open.record
        .reminder
        .map(|reminder| (reminder.at, reminder.keep_on_reply))
        .or(cx.thread_reminder)
}

/// The format of the action pane's dates, `Send:` and `Remind:` alike, for a pane `width` wide:
/// narrower with both a send time and a reminder, or popped out.
fn date_format(open: &Open, width: f32, cx: &Context<'_>) -> ComposeFormat {
    use ComposeFormat::{Compact, ExtraCompact, Friendly, MostCompact};
    let reminder = shown_reminder(open, cx).map(|(at, _)| at);
    let send_at = open.record.send_later.as_ref().map(|later| later.at);
    if cx.surface == Surface::Popout {
        return if send_at.is_none() && reminder.is_some() {
            ExtraCompact
        } else {
            MostCompact
        };
    }
    let both = reminder.is_some() && send_at.is_some();
    let now = cx.now;
    let day = |at: jiff::Timestamp| at.to_zoned(now.time_zone().clone()).date();
    let tomorrow = now.date().tomorrow().unwrap_or(now.date());
    let after_tomorrow = |at: Option<jiff::Timestamp>| at.is_some_and(|at| day(at) > tomorrow);
    // The draft's own reminder 2 to 6 days out: before `1 week from now`'s midnight.
    let week = now
        .date()
        .checked_add(jiff::ToSpan::days(7))
        .unwrap_or(now.date());
    let soon = open
        .record
        .reminder
        .is_some_and(|reminder| day(reminder.at) > tomorrow && day(reminder.at) < week);
    if both && after_tomorrow(reminder) && after_tomorrow(send_at) {
        return if width < 641.0 {
            MostCompact
        } else {
            ExtraCompact
        };
    }
    if width < if both { 641.0 } else { 480.0 } {
        MostCompact
    } else if !both && width < 600.0 {
        ExtraCompact
    } else if both || width < 620.0 || soon {
        Compact
    } else {
        Friendly
    }
}

/// The action pane's Send later at `at`: `Send later` in the lighter ink, the draft's time
/// (`Send: {date}`, in the format the pane's `width` allows), or `Unscheduled: {name} replied`.
/// With no time set, its tooltip says what it does. A click opens the picker. Returns where its
/// text ends.
#[expect(
    clippy::too_many_arguments,
    reason = "one control of the form, drawn from its state"
)]
fn send_later(
    ui: &Ui,
    open: &Open,
    response: &mut Response,
    at: egui::Pos2,
    width: f32,
    cx: &Context<'_>,
    ink: &Ink,
    base: Id,
) -> f32 {
    let ctx = ui.ctx();
    let (text, color) = match (&open.unscheduled_by, &open.record.send_later) {
        (Some(name), _) if name.is_empty() => ("Unscheduled: replied".to_owned(), UNSCHEDULED_INK),
        (Some(name), _) => (format!("Unscheduled: {name} replied"), UNSCHEDULED_INK),
        (None, Some(later)) => (
            format!(
                "Send: {}",
                friendly::compose_form(later.at, cx.now, date_format(open, width, cx))
            ),
            ink.text(0.6),
        ),
        (None, None) => ("Send later".to_owned(), ink.text(0.35)),
    };
    let text_width = SUBMIT.width(ctx, &text);
    let hit = Rect::from_min_size(
        pos2(at.x - 10.0, at.y - 10.0),
        vec2(text_width + 20.0, 36.0),
    );
    let button = ui
        .interact(hit, base.with("send-later"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    SUBMIT.paint_line(ui.painter(), pos2(at.x, at.y + 1.0), &text, color, None);
    let focused = cx.focused && open.state.field == Field::SendLater;
    if focused {
        underline(
            ui,
            at.x,
            at.x + text_width,
            at.y + 17.0,
            exo_ui::palette(ctx).border_base_active,
        );
    }
    if open.record.send_later.is_none() && open.unscheduled_by.is_none() {
        let row = KeyTooltip {
            text: "Send later",
            keys: &["⌘", "shift", "L"],
        };
        tooltip(
            ui,
            cx.place(),
            &button,
            focused,
            std::slice::from_ref(&row),
            Some("Send at the perfect moment"),
            CONTROL_TOOLTIP_GAP,
        );
    }
    if button.clicked() {
        response.actions.push(Action::SendLater);
    }
    at.x + text_width
}

/// Below this action pane width the button says only `Remind`.
const REMIND_ME_MIN_WIDTH: f32 = 430.0;
/// Below this window width a single date's tooltip says the reminder's date.
const REMIND_TOOLTIP_COMPACT: f32 = 1150.0;

/// The action pane's Remind button at `at`: `Remind me` (` me` only in a wide enough pane) in
/// the lighter ink, or with a reminder (the draft's, else its thread's) `Remind: {date}`, and
/// for one a reply cancels ` if no reply` (or `…`) as the pane's `width` allows.
/// Only the draft's own takes the darker `rgba(0,0,0,.6)`. A click opens the picker.
#[expect(
    clippy::too_many_arguments,
    reason = "one control of the form, drawn from its state"
)]
fn remind_me(
    ui: &Ui,
    open: &Open,
    response: &mut Response,
    at: egui::Pos2,
    width: f32,
    cx: &Context<'_>,
    ink: &Ink,
    base: Id,
) {
    let ctx = ui.ctx();
    let both = shown_reminder(open, cx).is_some() && open.record.send_later.is_some();
    let text = match shown_reminder(open, cx) {
        None if width <= REMIND_ME_MIN_WIDTH => "Remind".to_owned(),
        None => "Remind me".to_owned(),
        Some((when, keep_on_reply)) => {
            let suffix = if keep_on_reply || width < if both { 641.0 } else { 480.0 } {
                ""
            } else if both || width < 600.0 || cx.surface == Surface::Popout {
                "…"
            } else {
                " if no reply"
            };
            format!(
                "Remind: {}{suffix}",
                friendly::compose_form(when, cx.now, date_format(open, width, cx))
            )
        }
    };
    let color = ink.text(if open.record.reminder.is_some() {
        0.6
    } else {
        0.35
    });
    let text_width = SUBMIT.width(ctx, &text);
    let hit = Rect::from_min_size(
        pos2(at.x - 10.0, at.y - 10.0),
        vec2(text_width + 20.0, 36.0),
    );
    let button = ui
        .interact(hit, base.with("remind-me"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    // An automatic reminder just put on flourishes from the label's ink and keeps its last frame.
    match super::reminder::flourish_progress(open, ctx.input(|input| input.time)) {
        Some(progress) => {
            super::reminder::paint_flourish(
                ui.painter(),
                pos2(at.x, at.y + 1.0),
                &text,
                SUBMIT,
                progress,
                ink.text(0.6),
            );
            if progress < 1.0 {
                ctx.request_repaint();
            }
        }
        None => {
            SUBMIT.paint_line(ui.painter(), pos2(at.x, at.y + 1.0), &text, color, None);
        }
    }
    let focused = cx.focused && open.state.field == Field::RemindMe;
    if focused {
        underline(
            ui,
            at.x,
            at.x + text_width,
            at.y + 17.0,
            exo_ui::palette(ctx).border_base_active,
        );
    }
    // `Remind me ⌘ shift H`, then what it's for, or in a compact layout the draft's reminder.
    let note = match open.record.reminder {
        Some(reminder)
            if both
                || ctx.content_rect().width() < REMIND_TOOLTIP_COMPACT
                || cx.surface == Surface::Popout =>
        {
            format!(
                "{}{}",
                friendly::exact_friendly(reminder.at, cx.now),
                if reminder.keep_on_reply {
                    ""
                } else {
                    " if no reply"
                }
            )
        }
        _ => "Follow up on time, every time".to_owned(),
    };
    let row = KeyTooltip {
        text: "Remind me",
        keys: &["⌘", "shift", "H"],
    };
    tooltip(
        ui,
        cx.place(),
        &button,
        focused,
        std::slice::from_ref(&row),
        Some(note.as_str()),
        CONTROL_TOOLTIP_GAP,
    );
    if button.clicked() {
        response.actions.push(Action::RemindMe);
    }
}

/// A draft's listed attachments: its files, not its inline images.
pub(super) fn listed(open: &Open) -> impl Iterator<Item = &exo_store::DraftAttachment> {
    open.record
        .attachments
        .iter()
        .filter(|attachment| attachment.content_id.is_none())
}

/// How far `attachment` has gone up to Gmail (0–100) while the draft's copy there lacks it.
/// A snippet's files are Exo's alone and never go up.
pub(super) fn uploading(open: &Open, attachment: &exo_store::DraftAttachment) -> Option<f32> {
    open.upload
        .progress(attachment)
        .filter(|_| !open.is_snippet())
}

/// A card's file on its way to Gmail: how far it has come, and whether Gmail is in reach.
#[derive(Clone, Copy)]
struct Uploading {
    progress: f32,
    online: bool,
}

/// What a click on a draft's card asks for.
enum CardClick {
    /// The cross: the file comes off the draft.
    Remove,
    /// The card: the file opens, previewed unless ⌘ or ⇧ was held.
    Open { preview: bool },
}

/// One of a draft's cards: on hover, a 50% black overlay and the remove cross (12px, 9px from
/// the top and 8px from the right); while its file goes up, its progress bar (online) and,
/// hovered, `Processing…` or `Will upload when online` under it. Returns what a click on it
/// asks for.
fn attachment_card(
    ui: &Ui,
    card: Rect,
    name: &str,
    mime_type: &str,
    upload: Option<Uploading>,
    id: Id,
) -> Option<CardClick> {
    let response = ui
        .interact(card, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let ctx = ui.ctx();
    let hover = exo_ui::motion::transition(
        ctx,
        id.with("hover"),
        response.hovered(),
        0.25,
        exo_ui::motion::CubicBezier::EASE,
    );
    exo_ui::paint_attachment_card(ui, card, name, mime_type, hover);
    // The overlay: `opacity: 0` → `.5`, `.125s linear`.
    let overlay = exo_ui::motion::transition(
        ctx,
        id.with("overlay"),
        response.hovered(),
        0.125,
        exo_ui::motion::CubicBezier::LINEAR,
    );
    if overlay > 0.0 {
        ui.painter().rect_filled(
            exo_ui::pixel_snap(card),
            CornerRadius::same(4),
            Color32::from_black_alpha((0.5 * overlay * 255.0).round() as u8),
        );
    }
    if let Some(Uploading { progress, online }) = upload {
        // The progress bar, over the overlay, while online: from 10%, in whole percents.
        if online {
            exo_ui::paint_attachment_progress(ui, card, name, progress.floor().max(10.0));
        }
        // Its tooltip, centred under the card.
        let text = if online {
            "Processing…"
        } else {
            "Will upload when online"
        };
        KeyTooltip { text, keys: &[] }.show_below(ui, &response, 8.0);
    }
    // The remove cross: a white 9×8 cross scaled to 12px wide, `left center` in its 12px box.
    let cross = Rect::from_min_size(
        pos2(card.right() - 8.0 - 12.0, card.top() + 9.0),
        vec2(12.0, 12.0),
    );
    let on_cross = response.hover_pos().is_some_and(|at| cross.contains(at));
    // `opacity: 0` → `1` on the card → `.7` on the cross, `150ms ease`.
    let shown = exo_ui::motion::transition(
        ctx,
        id.with("cross"),
        response.hovered(),
        0.15,
        exo_ui::motion::CubicBezier::EASE,
    );
    let dimmed = exo_ui::motion::transition(
        ctx,
        id.with("cross-hover"),
        on_cross,
        0.15,
        exo_ui::motion::CubicBezier::EASE,
    );
    let opacity = shown * (1.0 - 0.3 * dimmed);
    if opacity > 0.0 {
        let glyph = Rect::from_center_size(cross.center(), vec2(12.0, 12.0 * 8.0 / 9.0));
        exo_ui::paint_icon(
            ui,
            "css/isInvalid-cross",
            glyph,
            Color32::WHITE.gamma_multiply(opacity),
        );
    }
    if !response.clicked() {
        return None;
    }
    if response
        .interact_pointer_pos()
        .is_some_and(|at| cross.contains(at))
    {
        return Some(CardClick::Remove);
    }
    let modifiers = ui.input(|input| input.modifiers);
    Some(CardClick::Open {
        preview: !(modifiers.command || modifiers.shift),
    })
}

/// The overlay every form shows over `area` while files are dragged over the window: 80%
/// opaque, a 2px dashed border, the page-with-a-plus icon in the middle and `Drop files here`
/// under it.
fn drop_zone(ui: &Ui, area: Rect, dark: bool) {
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        Id::new("compose-drop-zone"),
    ));
    let rect = exo_ui::pixel_snap(area);
    // `hsla(0,0%,100%,.98)` / `#27292d` at `opacity: .8`.
    let fill = if dark {
        Color32::from_rgb(0x27, 0x29, 0x2d)
    } else {
        Color32::from_white_alpha(250)
    };
    painter.rect_filled(rect, CornerRadius::same(2), fill.gamma_multiply(0.8));
    // `2px dashed rgba(0,0,0,.1)`; dark `hsla(0,0%,100%,.85)`.
    let stroke = egui::Stroke::new(
        2.0,
        if dark {
            Color32::from_white_alpha(217)
        } else {
            Color32::from_black_alpha(26)
        }
        .gamma_multiply(0.8),
    );
    let inner = rect.shrink(1.0);
    let corners = [
        inner.left_top(),
        inner.right_top(),
        inner.right_bottom(),
        inner.left_bottom(),
        inner.left_top(),
    ];
    painter.extend(egui::Shape::dashed_line(&corners, stroke, 6.0, 6.0));
    // The background's 22×26 icon, `center center no-repeat` (its own `#7F7F7F`).
    let icon = exo_ui::pixel_snap(Rect::from_center_size(rect.center(), vec2(22.0, 26.0)));
    exo_ui::paint_icon_on(
        &painter,
        "css/FileDropZone",
        icon,
        Color32::WHITE.gamma_multiply(0.8),
    );
    // `::after { content: "Drop files here" }`: 12px/12px bold, `top: 50%; margin-top: 1.5rem`.
    let label = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 12.0);
    let text = "Drop files here";
    let width = label.width(ui.ctx(), text);
    let color = Color32::from_black_alpha(204).gamma_multiply(0.8);
    label.paint_line(
        &painter,
        pos2(rect.center().x - width / 2.0, rect.center().y + 24.0),
        text,
        color,
        None,
    );
}

/// One header button in its box, with its tooltip; returns what a click asks for.
fn header_button(
    ui: &Ui,
    button: Button,
    hit: Rect,
    cx: &Context<'_>,
    popped_out: bool,
    focused: bool,
    base: Id,
) -> Option<Action> {
    let palette = exo_ui::palette(ui.ctx());
    let many_aliases = active_aliases(cx.aliases) > 1;
    let expand_rows = [
        KeyTooltip {
            text: "Cc",
            keys: &["⌘", "shift", "C"],
        },
        KeyTooltip {
            text: "Bcc",
            keys: &["⌘", "shift", "B"],
        },
        KeyTooltip {
            text: "From",
            keys: &["⌘", "shift", "F"],
        },
    ];
    let (icon, rows, action): (&str, &[KeyTooltip<'_>], Action) = match button {
        Button::Expand => (
            "legacy/DEPRECATED_downCaret",
            if many_aliases {
                &expand_rows[..]
            } else {
                &expand_rows[..2]
            },
            Action::Focus(Field::Cc),
        ),
        Button::Pop if popped_out => (
            "legacy/DEPRECATED_popin",
            &[KeyTooltip {
                text: "Pop In",
                keys: &["⌘", "shift", "P"],
            }],
            Action::ClickPop,
        ),
        Button::Pop => (
            "legacy/DEPRECATED_popout",
            &[
                KeyTooltip {
                    text: "Pop Out",
                    keys: &["⌘", "shift", "P"],
                },
                KeyTooltip {
                    text: "Pop Out & Search",
                    keys: &["⌘", "/"],
                },
            ],
            Action::ClickPop,
        ),
        Button::Close => (
            "legacy/DEPRECATED_popoutclose",
            &[KeyTooltip {
                text: "Close",
                keys: &["esc"],
            }],
            Action::ClickClosePopout,
        ),
    };
    // The icon centered in a 12px box; its focus box is 12×17.
    // The action sits 8px from the box's right and 4px from its top.
    let action_box = Rect::from_min_size(
        pos2(hit.right() - 8.0 - 12.0, hit.top() + 4.0),
        vec2(12.0, 12.0),
    );
    let icon_rect = exo_ui::icon_rect(ui.ctx(), icon, action_box);
    let rest = if focused {
        palette.icon_base_hover
    } else {
        palette.icon_base_default
    };
    let clicked = IconButton::new(icon)
        .colors(rest, palette.icon_base_hover)
        .show(ui, base.with(icon), hit, icon_rect);
    if focused {
        // Focused, a `box-shadow: 0 2px …` under its 17px box.
        underline(
            ui,
            action_box.left(),
            action_box.right(),
            action_box.top() + 17.0,
            palette.border_base_active,
        );
    }
    // Header buttons' tooltips are pinned to the window, not the scroll container.
    let place = match cx.place() {
        Place::Below(_) => Place::Below(ui.ctx().content_rect().x_range()),
        above => above,
    };
    tooltip(ui, place, &clicked, focused, rows, None, HEADER_TOOLTIP_GAP);
    clicked.clicked().then_some(action)
}

/// The done mark's check (the path `M1 5.142L4.858 9l7.587-7.587`, stroked 2px) with its 14×11
/// box at `top_left`.
fn paint_done_check(painter: &egui::Painter, top_left: egui::Pos2) {
    let points =
        [vec2(1.0, 5.142), vec2(4.858, 9.0), vec2(12.445, 1.413)].map(|point| top_left + point);
    painter.add(egui::Shape::line(
        points.to_vec(),
        egui::Stroke::new(2.0, DONE_INK),
    ));
}

/// A 2px focus line in `color` across `left..right` from `top` (`box-shadow: 0 2px`, or a
/// `border-bottom: 2px`).
fn underline(ui: &Ui, left: f32, right: f32, top: f32, color: Color32) {
    ui.painter().rect_filled(
        exo_ui::pixel_snap(Rect::from_min_max(pos2(left, top), pos2(right, top + 2.0))),
        0.0,
        color,
    );
}

/// The tooltip of a control in the form, while it's hovered or `focused`: under it, or above it
/// in the pop-out.
fn tooltip(
    ui: &Ui,
    place: Place,
    anchor: &egui::Response,
    focused: bool,
    rows: &[KeyTooltip<'_>],
    note: Option<&str>,
    gap: f32,
) {
    let tip = ShortcutTooltip { rows, note };
    match place {
        Place::Below(within) => tip.show_below(ui, anchor, gap, focused, within),
        Place::Above => tip.show_above(ui, anchor, CONTROL_TOOLTIP_ABOVE, focused),
    }
}

/// Share Availability's standing tip (`You can now hit ⌘ shift A…`) from the calendar icon's
/// `hit`, where its tooltip goes, pinned: 256px wide.
fn availability_tip(
    ui: &Ui,
    cx: &Context<'_>,
    hit: Rect,
    id: Id,
) -> Option<exo_ui::DismissableClick> {
    let content = [
        exo_ui::TooltipPart::Text("You can now hit"),
        exo_ui::TooltipPart::Keys(&["⌘", "shift", "A"]),
        exo_ui::TooltipPart::Text("to insert a booking link into your message."),
    ];
    let tip = exo_ui::DismissableTooltip {
        title: "Share Availability",
        content: &content,
        width: 256.0,
    };
    match cx.place() {
        Place::Below(within) => tip.show(
            ui,
            id,
            hit,
            exo_ui::Placement::Below {
                gap: CONTROL_TOOLTIP_GAP,
                shift: 0.0,
            },
            within,
        ),
        Place::Above => tip.show(
            ui,
            id,
            hit,
            exo_ui::Placement::Above(CONTROL_TOOLTIP_ABOVE),
            Rangef::EVERYTHING,
        ),
    }
}

/// `Draft to …` at `at`, at most `max_width` wide, and the tooltip listing the field shortcuts
/// when it's hovered (without Subject on the pop-out's placeholder, `subject`). Only its text
/// takes the pointer (`pointer-events: none` around it).
#[expect(
    clippy::too_many_arguments,
    reason = "one line of the form, with its tooltip's context"
)]
fn collapsed_header(
    ui: &Ui,
    open: &Open,
    at: egui::Pos2,
    max_width: f32,
    cx: &Context<'_>,
    ink: &Ink,
    subject: bool,
    id: Id,
) -> egui::Response {
    let palette = exo_ui::palette(ui.ctx());
    let text = collapsed_text(open, cx.me);
    let painter = ui.painter();
    let draft_width = COLLAPSED.paint_line(painter, at, "Draft", palette.actions_draft, None);
    let rest_width = COLLAPSED.paint_line(
        painter,
        pos2(at.x + draft_width, at.y),
        &format!(" to {text}"),
        ink.text(0.8),
        Some(max_width - draft_width),
    );
    let hit = Rect::from_min_size(at, vec2(draft_width + rest_width, COLLAPSED.line_height));
    let response = ui
        .interact(hit, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let rows = [
        KeyTooltip {
            text: "To",
            keys: &["⌘", "shift", "O"],
        },
        KeyTooltip {
            text: "Cc",
            keys: &["⌘", "shift", "C"],
        },
        KeyTooltip {
            text: "Bcc",
            keys: &["⌘", "shift", "B"],
        },
        KeyTooltip {
            text: "From",
            keys: &["⌘", "shift", "F"],
        },
        KeyTooltip {
            text: "Subject",
            keys: &["⌘", "shift", "S"],
        },
    ];
    let rows: Vec<KeyTooltip<'_>> = rows
        .into_iter()
        .filter(|row| {
            (row.text != "From" || active_aliases(cx.aliases) > 1)
                && (row.text != "Subject" || subject)
        })
        .collect();
    // Measured: the box's top 3px under the line.
    tooltip(ui, cx.place(), &response, false, &rows, None, 3.0);
    response
}

/// What stays under the message while its draft is popped out, after the separator: the
/// collapsed `Draft to …` (`margin: -2px 0 12px`) and the pop-in button. The card ends 38px
/// below the separator (measured), its own 4px padding included.
pub(super) const PLACEHOLDER: f32 = 34.0;

/// Draws the popped-out draft's placeholder from `top` (the separator's bottom) across
/// `left..left + width`; returns what it asked for.
pub(super) fn placeholder_header(
    ui: &Ui,
    open: &Open,
    left: f32,
    top: f32,
    width: f32,
    cx: &Context<'_>,
) -> Vec<Action> {
    let ink = Ink {
        dark: exo_ui::theme(ui.ctx()) == Theme::Carbon,
    };
    let id = exo_ui::id(ui.ctx(), ("compose-placeholder", &open.id));
    let _ = collapsed_header(
        ui,
        open,
        pos2(left, top - 2.0),
        width - BUTTON,
        cx,
        &ink,
        false,
        id.with("header"),
    );
    // Its icon 3px below the separator (measured).
    let hit = Rect::from_min_size(
        pos2(left + width + 8.0 - BUTTON, top - 1.0),
        vec2(BUTTON, BUTTON_HEIGHT),
    );
    header_button(ui, Button::Pop, hit, cx, true, false, id)
        .into_iter()
        .collect()
}

/// Gives the widget `id` the keyboard when the form has it and `wanted` says it's this one's.
fn focus_if(ui: &Ui, cx: &Context<'_>, wanted: bool, id: Id) {
    if cx.focused && wanted && !ui.memory(|memory| memory.has_focus(id)) {
        ui.memory_mut(|memory| memory.request_focus(id));
    }
}

/// The body's placeholder: `Say hello`, or `Write a note` for a note to self; none in an inline
/// reply.
fn placeholder(open: &Open, cx: &Context<'_>) -> Option<String> {
    if open.is_snippet() {
        return Some("Enter content".into());
    }
    let visible = cx.surface == Surface::Pane
        || (cx.surface == Surface::Popout && open.is_new_email())
        || open.record.action == exo_store::DraftAction::Forward;
    if !visible {
        return None;
    }
    let note = open.record.to.len() == 1
        && open.record.to[0].name.as_deref() == Some("Me")
        && recipients::same_address(&open.record.to[0].email, cx.me)
        && open.record.cc.is_empty()
        && open.record.bcc.is_empty();
    Some(if note { "Write a note" } else { "Say hello" }.into())
}

/// `{recipients}` of `Draft to {recipients}`: one person by full name, several by first name
/// (you left out), joined with `, ` and ` & `, then `. Bcc: …`; at most 15.
fn collapsed_text(open: &Open, me: &str) -> String {
    let is_me = |address: &Address| recipients::same_address(&address.email, me);
    let person = |address: &Address| {
        Person::new(
            address.name.clone().unwrap_or_default(),
            address.email.clone(),
        )
    };
    let main: Vec<&Address> = open.record.to.iter().chain(&open.record.cc).collect();
    let join = |names: Vec<String>| match names.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} & {last}", rest.join(", ")),
    };
    let names = |list: &[&Address]| -> Vec<String> {
        if list.len() == 1 {
            return vec![names::display_name(
                &person(list[0]),
                is_me(list[0]),
                NameFormat::Full,
                false,
            )];
        }
        list.iter()
            .filter(|address| !is_me(address))
            .map(|address| names::display_name(&person(address), false, NameFormat::First, false))
            .collect()
    };
    let mut text = join(names(&main).into_iter().take(15).collect());
    let bcc: Vec<&Address> = open.record.bcc.iter().collect();
    if !bcc.is_empty() {
        text.push_str(&format!(". Bcc: {}", join(names(&bcc))));
    }
    text
}

/// Draws one recipient row (or From) and handles its clicks and typing.
#[expect(
    clippy::too_many_arguments,
    reason = "one row of the form, drawn from its layout"
)]
fn show_row(
    ui: &mut Ui,
    open: &mut Open,
    row: &Row,
    origin: egui::Pos2,
    base: Id,
    cx: &Context<'_>,
    ink: &Ink,
    response: &mut Response,
    now: f64,
) {
    let ctx = ui.ctx().clone();
    let palette = exo_ui::palette(&ctx);
    let label = match row.list {
        Some(list) => list.label(),
        None => "From",
    };
    let top = origin.y + row.top;
    let label_hit = Rect::from_min_size(pos2(origin.x, top), vec2(LABEL_WIDTH, CHIP_HEIGHT));
    // The label's transparent 1px border puts its text 1px in.
    FIELD.paint_line(
        ui.painter(),
        pos2(origin.x + 1.0, top + CHIP_TEXT_Y),
        label,
        ink.text(0.8),
        None,
    );
    let field = row.list.map_or(Field::From, List::field);
    let focused_here = cx.focused && open.state.field == field;
    // From's label focuses it only when it hasn't the keyboard.
    if ui
        .interact(label_hit, base.with(("label", label)), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
        && !(row.list.is_none() && focused_here)
    {
        response.actions.push(Action::Focus(field));
    }
    let Some(list) = row.list else {
        from_row(
            ui,
            open,
            pos2(origin.x, top),
            cx,
            ink,
            base.with("from"),
            response,
        );
        return;
    };
    let selected = open.input(list).selected.clone();
    let caret_at_end = focused_here && selected.is_empty() && open.input(list).editing.is_none();
    let count = row.chips.len();
    let mut remove = None;
    for (index, rect, text) in &row.chips {
        let chip = rect.translate(origin.to_vec2());
        let address = &open.list(list)[*index];
        let invalid = !recipients::sendable(address);
        let id = base.with(("chip", list as usize, *index));
        let hit = ui
            .interact(chip, id, Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::Move);
        let hovered = hit.hovered();
        let is_selected = focused_here && selected.contains(index);
        // A hovered, selected or invalid chip's colours (literals).
        let (background, border) = if invalid {
            (
                Some(Color32::from_rgb(0xef, 0x8e, 0x8e)),
                Some(if is_selected {
                    Color32::from_rgb(126, 30, 30)
                } else {
                    Color32::from_rgb(0xe2, 0x86, 0x86)
                }),
            )
        } else if hovered || is_selected {
            let (fill, edge) = if ink.dark {
                (
                    Color32::from_rgb(0x55, 0x5b, 0x64),
                    Color32::from_rgb(0x63, 0x69, 0x73),
                )
            } else {
                (
                    Color32::from_rgb(0xfa, 0xfa, 0xfa),
                    Color32::from_rgb(0xf3, 0xf3, 0xf3),
                )
            };
            (
                Some(fill),
                Some(if is_selected {
                    Color32::from_rgb(0xae, 0xb1, 0xdd)
                } else {
                    edge
                }),
            )
        } else {
            (None, None)
        };
        let painter = ui.painter_at(chip);
        let snapped = exo_ui::pixel_snap(chip);
        if let Some(fill) = background {
            painter.rect_filled(snapped, CornerRadius::same(2), fill);
        }
        if let Some(edge) = border {
            painter.rect_stroke(
                snapped.shrink(0.5),
                CornerRadius::same(2),
                egui::Stroke::new(1.0, edge),
                egui::StrokeKind::Middle,
            );
        }
        let color = if invalid {
            Color32::WHITE
        } else {
            ink.text(0.8)
        };
        FIELD.paint_line(
            &painter,
            pos2(chip.left() + CHIP_TEXT_X, chip.top() + CHIP_TEXT_Y),
            text,
            color,
            Some(chip.width() - CHIP_EXTRA + 1.0),
        );
        // The tail: `right: -14px`, translated -6px (-14px when shown); the ✕ is 9px, the `•`
        // 7px back over it.
        let shown = hovered || is_selected || invalid;
        let tail_left = chip.right() - 1.0 + 14.0 - 30.0 - if shown { 14.0 } else { 6.0 };
        if shown {
            let fill = background.unwrap_or(Color32::TRANSPARENT);
            exo_ui::paint_fade_in(
                &painter,
                Rect::from_min_max(
                    pos2(tail_left, chip.top() + 1.0),
                    pos2(chip.right() - 1.0, chip.bottom() - 1.0),
                ),
                fill,
            );
            let cross = Rect::from_min_size(
                pos2(tail_left + 14.0, chip.top() + CHIP_TEXT_Y + 6.0),
                vec2(9.0, 8.0),
            );
            let cross_color = if invalid {
                Color32::WHITE
            } else {
                Color32::from_rgb(0xb9, 0xb9, 0xb9)
            };
            exo_ui::paint_icon(ui, "legacy/close", cross, cross_color);
            let tail = Rect::from_min_size(pos2(tail_left, chip.top()), vec2(30.0, chip.height()));
            if ui
                .interact(tail.intersect(chip), id.with("remove"), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                remove = Some(*index);
            }
        }
        let last = *index + 1 == count;
        let separator_hidden = shown || (last && !caret_at_end);
        if !separator_hidden {
            let separator = if invalid {
                Color32::from_white_alpha(51)
            } else {
                ink.text(0.2)
            };
            FIELD.paint_line(
                ui.painter(),
                pos2(tail_left + 14.0 + 9.0 - 7.0, chip.top() + CHIP_TEXT_Y),
                "•",
                separator,
                None,
            );
        }
        // Mouse-down selects; a double click edits it in place.
        if hit.is_pointer_button_down_on()
            && ctx.input(|input| input.pointer.primary_pressed())
            && remove.is_none()
        {
            let modifiers = ctx.input(|input| input.modifiers);
            super::chips::press(open, list, *index, modifiers, cx.book, now);
            response.actions.push(Action::Focus(field));
        }
        if hit.double_clicked() {
            super::chips::start_edit(open, list, *index);
            response.actions.push(Action::Focus(field));
            return;
        }
        // Chips can be dragged.
        if hit.drag_started() {
            let grab = ctx
                .input(|input| input.pointer.press_origin())
                .map_or(Vec2::ZERO, |at| at - chip.min);
            super::chips::start_drag(
                open,
                list,
                *index,
                (text.clone(), chip.size(), grab),
                cx.book,
                now,
            );
        }
    }
    drop_target(ui, open, row, list, origin, base, response);
    if let Some(index) = remove {
        open.list_mut(list).remove(index);
        let input = open.input_mut(list);
        input.selected.clear();
        if input.editing.is_some_and(|editing| editing >= index) {
            input.editing = input.editing.and_then(|editing| editing.checked_sub(1));
        }
        open.touch(now);
    }
    // The input, after the chips or in place of the one being edited; none while chips are
    // selected, when nothing typed goes anywhere but a paste replaces them.
    let Some(input) = row.input.map(|input| input.translate(origin.to_vec2())) else {
        if focused_here {
            super::chips::paste_over(ui, open, list, cx.book, now);
        }
        return;
    };
    let line = Rect::from_min_size(
        pos2(input.left() + 7.0, input.top() + CHIP_TEXT_Y),
        vec2(input.width() - 8.0, 20.0),
    );
    let id = base.with(("input", list as usize));
    let before = open.input(list).text.clone();
    let mut text = before.clone();
    let field_response = exo_ui::text_field(
        ui,
        id,
        &mut text,
        line,
        &FIELD,
        ink.text(0.8),
        palette.text_selection_default,
    );
    if field_response.gained_focus() {
        open.header_scroll.reveal = true;
    }
    let range = egui::text_edit::TextEditState::load(ui.ctx(), id)
        .and_then(|state| state.cursor.char_range());
    open.input_mut(list).caret = range
        .filter(|range| range.primary == range.secondary)
        .map(|range| range.primary.index.0);
    if text != before {
        open.input_mut(list).text = text;
        open.header_scroll.reveal = true;
        open.input_mut(list).selected.clear();
        open.input_mut(list).just_excluded = None;
        if before.is_empty() && open.input(list).text.contains([',', ';', '\n']) {
            // A pasted list of addresses becomes chips.
            commit_input(open, now, cx.book);
        }
        open.touch(now);
    }
    // Suggestions follow the text while the field has the keyboard.
    if focused_here {
        let state = open.input_mut(list);
        if state.text != state.query {
            state.query.clone_from(&state.text);
            state.closed = false;
            state.highlight = 0;
            let found = match cx.book {
                Some(book) => contacts::suggest_sourced(
                    book,
                    cx.thread_people.unwrap_or(&contacts::Index::default()),
                    &state.text,
                ),
                None => Vec::new(),
            };
            (state.suggestions, state.from_thread) = found.into_iter().unzip();
        }
        if open.input(list).suggesting() {
            let existing: Vec<String> = open
                .list(list)
                .iter()
                .map(|address| address.email.to_lowercase())
                .collect();
            match suggestions(
                ui,
                input,
                open.input_mut(list),
                &existing,
                base.with(("suggestions", list as usize)),
            ) {
                exo_ui::ContactListResponse {
                    removed: Some(index),
                    ..
                } => response.actions.push(Action::RemoveSuggestion(index)),
                exo_ui::ContactListResponse {
                    pressed: Some(picked),
                    ..
                } => response.actions.push(Action::PickSuggestion(picked)),
                _ => {}
            }
        } else {
            // The list is gone; the next one is sized afresh.
            open.input_mut(list).list_width = 0.0;
        }
    } else if !open.input(list).suggestions.is_empty() {
        let input = open.input_mut(list);
        input.suggestions.clear();
        input.query.clear();
    }
    // Clicking the row's empty space puts the caret at the end.
    let row_rect = Rect::from_min_size(
        pos2(origin.x + LABEL_WIDTH, top),
        vec2(1e4, row.lines as f32 * LINE),
    )
    .intersect(ui.clip_rect());
    let row_click = ui.interact(row_rect, base.with(("row", list as usize)), Sense::click());
    if field_response.gained_focus() || field_response.clicked() || row_click.clicked() {
        open.input_mut(list).selected.clear();
        response.actions.push(Action::Focus(field));
    }
    focus_if(ui, cx, open.state.field == field, id);
    if focused_here && std::mem::take(&mut open.input_mut(list).select_text) {
        let all = egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(open.input(list).text.chars().count()),
        );
        exo_ui::focus_field(ui.ctx(), id, all);
    }
}

/// The drop cursor: 2px wide, `1.5em` tall, `#6fbeea`, fading in and out over 150ms.
const DROP_CURSOR: Vec2 = Vec2::new(2.0, 21.0);
const DROP_CURSOR_COLOR: Color32 = Color32::from_rgb(0x6f, 0xbe, 0xea);

/// A recipient list as a drop target while chips are dragged: over it, the drop cursor shows
/// where they'd go, and letting go there drops them.
fn drop_target(
    ui: &Ui,
    open: &Open,
    row: &Row,
    list: List,
    origin: egui::Pos2,
    base: Id,
    response: &mut Response,
) {
    let ctx = ui.ctx();
    let area = Rect::from_min_size(
        pos2(origin.x, origin.y + row.top),
        vec2(1e4, row.lines as f32 * LINE),
    )
    .intersect(ui.clip_rect());
    let id = base.with(("drop-cursor", list as usize));
    let place = open
        .chip_drag
        .as_ref()
        .and(ctx.input(|input| input.pointer.hover_pos()))
        .filter(|at| area.contains(*at))
        .map(|at| {
            let chips: Vec<Rect> = row
                .chips
                .iter()
                .map(|(_, rect, _)| rect.translate(origin.to_vec2()))
                .collect();
            drop_place(&chips, area, at)
        });
    if let Some((_, at)) = place {
        ctx.data_mut(|data| data.insert_temp(id, at));
    }
    let shown = exo_ui::motion::transition(
        ctx,
        id,
        place.is_some(),
        0.15,
        exo_ui::motion::CubicBezier::EASE_IN,
    );
    if shown > 0.0
        && let Some(at) = ctx.data(|data| data.get_temp::<egui::Pos2>(id))
    {
        ui.painter().rect_filled(
            Rect::from_min_size(at, DROP_CURSOR),
            0.0,
            DROP_CURSOR_COLOR.gamma_multiply(shown),
        );
    }
    if let (Some((at, _)), Some(drag)) = (place, &open.chip_drag)
        && ctx.input(|input| input.pointer.primary_released())
    {
        response.actions.push(Action::DropChips {
            from: drag.list,
            chips: drag.chips.clone(),
            into: list,
            at,
        });
    }
}

/// For the pointer at `at` over a list at `list` with chips at `chips` (in order), the place a
/// drop goes and the cursor's top left. The row is the lowest whose top is above the pointer;
/// the chip is the first on it ending right of the pointer, else its last; the cursor goes a
/// pixel before it, or after it past its middle. An empty list's is at the label's width.
fn drop_place(chips: &[Rect], list: Rect, at: egui::Pos2) -> (usize, egui::Pos2) {
    let row = chips
        .iter()
        .rev()
        .map(|chip| chip.top())
        .find(|top| at.y > *top);
    let on_row: Vec<(usize, Rect)> = chips
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, chip)| Some(chip.top()) == row)
        .collect();
    let fallback = (
        0,
        Rect::from_min_size(pos2(list.left() + LABEL_WIDTH, list.top()), vec2(2.0, 0.0)),
    );
    let (rank, chip) = on_row
        .iter()
        .find(|(_, chip)| chip.right() > at.x)
        .or(on_row.last())
        .copied()
        .unwrap_or(fallback);
    let (x, rank) = if chips.is_empty() {
        (chip.left() - 1.0, 0)
    } else if at.x < chip.left() + chip.width() / 2.0 {
        (chip.left() - 1.0, rank)
    } else {
        (chip.right() + 1.0, rank + 1)
    };
    (
        rank,
        pos2(
            list.left() + (x - list.left()).floor(),
            list.top() + (chip.top() - list.top() + 2.0).floor(),
        ),
    )
}

/// The image that follows the pointer while chips are dragged, at 75%: the grabbed chip,
/// selected, kept where it was grabbed; several, a stack of three chips reading `Multiple
/// recipients` with its top left at the pointer.
fn paint_drag_image(ui: &Ui, drag: &super::chips::Drag, ink: &Ink) {
    let ctx = ui.ctx();
    let Some(pointer) = ctx.input(|input| input.pointer.hover_pos()) else {
        return;
    };
    let mut painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        Id::new("compose-chip-drag"),
    ));
    painter.set_opacity(0.75);
    // A selected chip's colours (Classic literals).
    let (fill, edge) = if ink.dark {
        (
            Color32::from_rgb(0x55, 0x5b, 0x64),
            Color32::from_rgb(0xae, 0xb1, 0xdd),
        )
    } else {
        (
            Color32::from_rgb(0xfa, 0xfa, 0xfa),
            Color32::from_rgb(0xae, 0xb1, 0xdd),
        )
    };
    let chip = |rect: Rect, edge: Color32| {
        let snapped = exo_ui::pixel_snap(rect);
        painter.rect_filled(snapped, CornerRadius::same(2), fill);
        painter.rect_stroke(
            snapped.shrink(0.5),
            CornerRadius::same(2),
            egui::Stroke::new(1.0, edge),
            egui::StrokeKind::Middle,
        );
    };
    let (label, size, grab) = &drag.grabbed;
    if drag.chips.len() > 1 {
        // Three 135×28 chips 3px apart, back to front, edged at half strength.
        let faint = edge.gamma_multiply(0.5);
        for step in [6.0, 3.0, 0.0] {
            chip(
                Rect::from_min_size(pointer + vec2(step, step), vec2(135.0, 28.0)),
                faint,
            );
        }
        FIELD.paint_line(
            &painter,
            pointer + vec2(7.0, CHIP_TEXT_Y),
            "Multiple recipients",
            ink.text(0.7),
            None,
        );
        return;
    }
    let rect = Rect::from_min_size(pointer - *grab, *size);
    chip(rect, edge);
    FIELD.paint_line(
        &painter,
        pos2(rect.left() + CHIP_TEXT_X, rect.top() + CHIP_TEXT_Y),
        label,
        ink.text(0.8),
        Some(rect.width() - CHIP_EXTRA + 1.0),
    );
    // The shown tail and its ✕, as [`show_row`] draws them.
    exo_ui::paint_fade_in(
        &painter,
        Rect::from_min_max(
            pos2(rect.right() - 31.0, rect.top() + 1.0),
            pos2(rect.right() - 1.0, rect.bottom() - 1.0),
        ),
        fill,
    );
    let cross = Rect::from_min_size(
        pos2(rect.right() - 17.0, rect.top() + CHIP_TEXT_Y + 6.0),
        vec2(9.0, 8.0),
    );
    exo_ui::paint_icon_on(
        &painter,
        "legacy/close",
        cross,
        Color32::from_rgb(0xb9, 0xb9, 0xb9),
    );
}

/// The suggestions under the input `input` (on screen), as `ContactSelectorList` draws them:
/// a name column and an address column, the typed text dark in each, the highlighted row
/// filled, with `Remove Suggestion` on it while hovered unless it's the thread's. Says which row
/// the pointer picked (mouse-down picks) or removed.
fn suggestions(
    ui: &Ui,
    input: Rect,
    state: &mut super::draft::Input,
    existing: &[String],
    id: Id,
) -> exo_ui::ContactListResponse {
    let ctx = ui.ctx().clone();
    let names: Vec<String> = state
        .suggestions
        .iter()
        .map(|address| {
            address
                .name
                .clone()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| address.email.clone())
        })
        .collect();
    let name_text = names
        .iter()
        .map(|name| SUGGESTION.width_unkerned(&ctx, name))
        .fold(0.0, f32::max);
    let email_text = state
        .suggestions
        .iter()
        .map(|address| SUGGESTION.width_unkerned(&ctx, &address.email))
        .fold(0.0, f32::max);
    let (name_width, email_width) =
        suggestion_columns(name_text, email_text, &mut state.list_width);
    let rows: Vec<exo_ui::ContactRow<'_>> = state
        .suggestions
        .iter()
        .zip(&names)
        .map(|(address, name)| exo_ui::ContactRow {
            name,
            email: &address.email,
            disabled: existing.contains(&address.email.to_lowercase()),
            avatar: None,
        })
        .collect();
    let pad = SUGGESTION_CELL_PADDING;
    let style = exo_ui::ContactListStyle {
        text: SUGGESTION,
        row: SUGGESTION_ROW,
        padding: SUGGESTION_PADDING,
        columns: exo_ui::ContactColumns {
            avatar: None,
            name: (pad, name_width - 2.0 * pad),
            email: (name_width + pad, email_width - 2.0 * pad),
        },
        force_snow: false,
    };
    let query = state.text.trim().trim_start_matches("to:").trim();
    let removable: Vec<bool> = state.from_thread.iter().map(|thread| !thread).collect();
    let list = exo_ui::ContactSelectorList::new(&rows, query, Some(state.highlight), style)
        .removable(&removable);
    list.show(
        &ctx,
        id,
        pos2(input.left() - 10.0, input.bottom() + 6.0),
        state.list_width,
        egui::Order::Foreground,
    )
}

/// The suggestion list's columns for its widest name and address (`*_text`), given the width
/// the list has had since it opened (`seen`, updated). Each cell is its text padded 16px a side;
/// the name's `min-width: 165px` counts the padding (`box-sizing: border-box`). The list keeps
/// the widest rows it has shown since it opened, and what's over is shared between the columns
/// in proportion to their widths (measured: typing `pri` leaves Priya's row as wide as `p`'s
/// list was, 386px).
fn suggestion_columns(name_text: f32, email_text: f32, seen: &mut f32) -> (f32, f32) {
    let name = (name_text + 2.0 * SUGGESTION_CELL_PADDING).max(SUGGESTION_NAME_MIN);
    let email = email_text + 2.0 * SUGGESTION_CELL_PADDING;
    *seen = seen.max(name + email);
    let grow = *seen / (name + email);
    (name * grow, email * grow)
}

/// The `;` picker's box: the base background, 4px corners and the low elevation's shadow,
/// `0 1px 4px 0 #1414130f`. Returns it pixel-snapped.
fn paint_list_card(painter: &egui::Painter, list: Rect) -> Rect {
    let palette = exo_ui::palette(painter.ctx());
    let snapped = exo_ui::pixel_snap(list);
    exo_ui::BoxShadow::new(
        [0.0, 1.0],
        4.0,
        0.0,
        Color32::from_rgba_unmultiplied(0x14, 0x14, 0x13, 0x0f),
    )
    .paint(painter, snapped, CornerRadius::same(4));
    painter.rect_filled(
        snapped,
        CornerRadius::same(4),
        palette.background_base_default,
    );
    snapped
}

/// The From list's addresses: 13px in 28px rows, padded 18px.
const FROM_ITEM: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 13.0, 28.0);
const FROM_ITEM_PADDING: f32 = 18.0;

/// The From row's dropdown, from the row's top left `at`: the alias's address, with a caret
/// while hovered or focused; focused, its list of aliases under it when open.
fn from_row(
    ui: &Ui,
    open: &mut Open,
    at: egui::Pos2,
    cx: &Context<'_>,
    ink: &Ink,
    id: Id,
    response: &mut Response,
) {
    let ctx = ui.ctx().clone();
    let palette = exo_ui::palette(&ctx);
    let focused = cx.focused && open.state.field == Field::From;
    if !focused {
        // Blur closes the list.
        open.from_menu = None;
    }
    // The button: `rgba(0,0,0,.75)`, white in Carbon.
    let color = ink.text(0.75);
    // A snippet without its own From shows `use default` on the button.
    let email = match &open.record.from {
        Some(from) => from.email.clone(),
        None if open.is_snippet() => "use default".to_owned(),
        None => String::new(),
    };
    // `padding-left: 6px`; the button `top: 3px` and `padding-top: 2px`.
    let text = pos2(at.x + LABEL_WIDTH + 6.0, at.y + CHIP_TEXT_Y);
    let text_width = FIELD.paint_line(ui.painter(), text, &email, color, None);
    // The caret (Snow/Carbon): 10px wide, 10px after the address, centred on its line, at half
    // opacity while hovered or focused (`opacity` over 150ms).
    let caret = Rect::from_center_size(
        pos2(
            text.x + text_width + 10.0 + 5.0,
            text.y + FIELD.line_height / 2.0,
        ),
        vec2(10.0, 10.0 * 6.7 / 12.0),
    );
    let button = ui
        .interact(
            Rect::from_min_max(pos2(text.x, at.y + 3.0), pos2(caret.right(), at.y + 25.0)),
            id,
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let shown = exo_ui::motion::transition(
        &ctx,
        id.with("caret"),
        button.hovered() || focused,
        0.15,
        exo_ui::motion::CubicBezier::EASE,
    );
    if shown > 0.0 {
        exo_ui::paint_icon(
            ui,
            "legacy/DEPRECATED_downCaret",
            caret,
            ink.text(1.0).gamma_multiply(0.5 * shown),
        );
    }
    if button.clicked() {
        if !focused {
            response.actions.push(Action::Focus(Field::From));
        }
        response.actions.push(Action::FromToggle);
    }
    let Some(highlight) = open.from_menu else {
        return;
    };
    // The list, tethered under the button: 18px left of the address, `margin: 7px -1px`.
    let choices: Vec<String> = from_choices(open, cx.aliases)
        .into_iter()
        .map(choice_label)
        .collect();
    let width = choices
        .iter()
        .map(|email| FROM_ITEM.width(&ctx, email))
        .fold(0.0, f32::max)
        + 2.0 * FROM_ITEM_PADDING;
    let list = Rect::from_min_size(
        pos2(text.x - 18.0, at.y + 25.0 + 8.0),
        vec2(
            width,
            choices.len() as f32 * FROM_ITEM.line_height + 2.0 * SUGGESTION_PADDING,
        ),
    );
    let pointer = ctx.input(|input| input.pointer.hover_pos());
    let colors = exo_ui::ContactListColors::new(palette, false);
    egui::Area::new(id.with("list"))
        .order(egui::Order::Foreground)
        .fixed_pos(list.min)
        .show(&ctx, |area| {
            let painter = area.painter().clone();
            let _ = exo_ui::paint_contact_list_box(&painter, list, colors.background);
            let _ = area.allocate_rect(list, Sense::click());
            for (index, email) in choices.iter().enumerate() {
                let row = Rect::from_min_size(
                    pos2(
                        list.left(),
                        list.top() + SUGGESTION_PADDING + index as f32 * FROM_ITEM.line_height,
                    ),
                    vec2(width, FROM_ITEM.line_height),
                );
                let hovered = pointer.is_some_and(|at| row.contains(at));
                if hovered && index != highlight {
                    response
                        .actions
                        .push(Action::FromMove(index as isize - highlight as isize));
                }
                if index == highlight {
                    painter.rect_filled(exo_ui::pixel_snap(row), 0.0, colors.selected);
                }
                FROM_ITEM.paint_line(
                    &painter,
                    pos2(row.left() + FROM_ITEM_PADDING, row.top()),
                    email,
                    palette.ink(0.7, 0.7),
                    None,
                );
                if hovered && area.input(|input| input.pointer.primary_clicked()) {
                    response.actions.push(Action::FromChoose(Some(index)));
                }
            }
        });
}

/// The address a snippet's From list sorts `"use default"` (the draft's own From) by among the
/// aliases. It is never sent or saved: a snippet saves its default From as `None`.
const USE_DEFAULT: &str = "default.snippet.address@exo.invalid";

/// The From list's items: the active aliases, and for a snippet `"use default"` (`None`), the
/// selected first and the rest by address.
pub(super) fn from_choices<'a>(open: &Open, aliases: &'a [Alias]) -> Vec<Option<&'a Alias>> {
    let selected = open
        .record
        .from
        .as_ref()
        .map_or(USE_DEFAULT, |from| from.email.as_str());
    let mut choices: Vec<Option<&Alias>> = aliases
        .iter()
        .filter(|alias| alias.active)
        .map(Some)
        .collect();
    if open.is_snippet() {
        choices.push(None);
    }
    let email = |choice: &Option<&Alias>| {
        choice
            .map_or(USE_DEFAULT, |alias| alias.email.as_str())
            .to_owned()
    };
    choices.sort_by_key(|choice| {
        (
            !recipients::same_address(&email(choice), selected),
            email(choice),
        )
    });
    choices
}

/// A From list item's text: the alias's address, or `"use default"` with its quotes.
fn choice_label(choice: Option<&Alias>) -> String {
    choice.map_or_else(|| "\"use default\"".to_owned(), |alias| alias.email.clone())
}

/// The From list's keys and clicks: `FromToggle` opens or closes it, `FromMove` moves its
/// highlight (opening it if it's closed), `FromChoose` takes the highlighted alias (or the one
/// given) and closes it, `FromClose` closes it.
pub(super) fn from_menu(open: &mut Open, aliases: &[Alias], action: &Action, now: f64) {
    let choices = from_choices(open, aliases);
    let count = choices.len() as isize;
    if count == 0 {
        return;
    }
    match (action, open.from_menu) {
        (Action::FromToggle, Some(_)) | (Action::FromClose, _) => open.from_menu = None,
        (Action::FromToggle | Action::FromMove(_), None) => open.from_menu = Some(0),
        (Action::FromMove(delta), Some(at)) => {
            open.from_menu = Some((at as isize + delta).rem_euclid(count) as usize)
        }
        (Action::FromChoose(picked), Some(at)) => {
            let choice = choices[picked.unwrap_or(at).min(count as usize - 1)].cloned();
            open.from_menu = None;
            match choice {
                Some(alias) => choose_from(open, aliases, &alias, now),
                None if open.record.from.is_some() => {
                    open.record.from = None;
                    open.touch(now);
                }
                None => {}
            }
        }
        _ => {}
    }
}

/// The draft is from `alias` now, and a signature already in the body follows it.
pub(super) fn choose_from(open: &mut Open, aliases: &[Alias], alias: &Alias, now: f64) {
    if open
        .record
        .from
        .as_ref()
        .is_some_and(|from| from.email == alias.email)
    {
        return;
    }
    let old = open.record.from.replace(Address {
        name: alias.name.clone(),
        email: alias.email.clone(),
    });
    if open.record.quoted_inlined {
        signature::switch(open, aliases, old.as_ref());
    }
    open.touch(now);
}

/// Backspace right after a Unicode replacement in the subject brings back what was typed; any
/// other key forgets the replacement.
fn undo_subject_replacement(ui: &Ui, open: &mut Open, id: Id) {
    let Some(replacement) = open.subject_replacement.clone() else {
        return;
    };
    let backspace = ui.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Key { key: egui::Key::Backspace, pressed: true, modifiers, .. } if modifiers.is_none())));
    let other_key = ui.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Key { key, pressed: true, .. } if *key != egui::Key::Backspace)));
    if other_key {
        open.subject_replacement = None;
        return;
    }
    if !backspace {
        return;
    }
    let caret = egui::text_edit::TextEditState::load(ui.ctx(), id)
        .and_then(|state| state.cursor.char_range());
    open.subject_replacement = None;
    if caret.is_none_or(|range| {
        !range.is_empty() || usize::from(range.primary.index) != replacement.caret
    }) {
        return;
    }
    ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace));
    open.record.subject = replacement.typed;
    let at = egui::text::CCursor::new(replacement.typed_caret);
    exo_ui::focus_field(ui.ctx(), id, egui::text::CCursorRange::one(at));
}

/// When the last edit typed one of `>-—=~<.` and the text before the caret ends in a pattern
/// (`->`, `...`), it becomes its character (`→`, `…`), as in the body.
fn replace_in_subject(ui: &Ui, open: &mut Open, id: Id, before: &str) {
    let Some(caret) = egui::text_edit::TextEditState::load(ui.ctx(), id)
        .and_then(|state| state.cursor.char_range())
        .filter(|range| range.is_empty())
        .map(|range| usize::from(range.primary.index))
    else {
        return;
    };
    let chars: Vec<char> = open.record.subject.chars().collect();
    if chars.len() != before.chars().count() + 1 || caret == 0 || caret > chars.len() {
        return;
    }
    let Some((len, replacement)) =
        exo_editor::unicode_replacement(chars[caret - 1], &chars[..caret])
    else {
        return;
    };
    let start = caret - len;
    let replaced: String = chars[..start]
        .iter()
        .copied()
        .chain(replacement.chars())
        .chain(chars[caret..].iter().copied())
        .collect();
    let after = start + replacement.chars().count();
    open.subject_replacement = Some(SubjectReplacement {
        caret: after,
        typed: std::mem::replace(&mut open.record.subject, replaced),
        typed_caret: caret,
    });
    exo_ui::focus_field(
        ui.ctx(),
        id,
        egui::text::CCursorRange::one(egui::text::CCursor::new(after)),
    );
    ui.ctx().request_repaint();
}

/// A `;` picker row's text: 13px in 28px lines.
const SNIPPET_ROW: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 13.0, 28.0);

/// The `;` picker under the `;` at `anchor`: up to five rows of a snippet's name, its text and
/// `Me`, the highlighted one filled; or, with no snippets at all, `To create a Snippet, hit ⌘ K
/// → "Snippets"`. At least 400px wide (`min-width`), 14px left of the `;`, padded 7px, with no
/// border (Classic's `border: 0; margin: 0 0 0 2px`). The subject's (`in_subject`) hangs from
/// the line's bottom 2px right of the `;` (Classic's margin wins over its `-13px`), no wider
/// than `min(100vw - 460px, 600px)`. Returns the row pressed (mouse-down applies it).
fn snippet_picker(
    ui: &Ui,
    open: &mut Open,
    anchor: Rect,
    in_subject: bool,
    form_width: f32,
    id: Id,
) -> Option<usize> {
    let ctx = ui.ctx().clone();
    let palette = exo_ui::palette(&ctx);
    const PAD: f32 = 17.0;
    const ROW: f32 = 32.0;
    let rows = open.snippet_rows.clone();
    let me = SNIPPET_ROW.width(&ctx, "Me");
    let widest = rows
        .iter()
        .map(|row| {
            SNIPPET_ROW
                .width(&ctx, &row.name)
                .min(275.0f32.max(form_width / 2.0))
                + 2.0 * PAD
                + SNIPPET_ROW.width(&ctx, &row.preview).min(300.0)
                + me
                + PAD
        })
        .fold(0.0, f32::max);
    let count = if rows.is_empty() { 1 } else { rows.len() };
    let list = if in_subject {
        let most = (ctx.content_rect().width() - 460.0).min(600.0);
        Rect::from_min_size(
            pos2(anchor.left() + 2.0, anchor.bottom()),
            vec2(
                widest.clamp(400.0, most.max(400.0)),
                count as f32 * ROW + 14.0,
            ),
        )
    } else {
        let width = widest.clamp(400.0, form_width.max(400.0));
        Rect::from_min_size(
            pos2(anchor.left() - 14.0, anchor.bottom() - 1.0),
            vec2(width - 2.0, count as f32 * ROW + 14.0),
        )
    };
    // The highlighted row's fill, the name's colour and the muted text's: literals.
    let (selected, name_color, muted) = match palette.theme {
        // `hsl(220, 45%, 97%)`.
        Theme::Snow => (
            Color32::from_rgb(0xf4, 0xf6, 0xfb),
            palette.ink(0.85, 0.85),
            palette.ink(0.45, 0.45),
        ),
        Theme::Carbon => (
            Color32::from_rgb(0x46, 0x4b, 0x53),
            Color32::WHITE,
            palette.ink(0.45, 0.45),
        ),
    };
    let pointer = ctx.input(|input| {
        (
            input.pointer.hover_pos(),
            input.pointer.primary_pressed(),
            input.pointer.is_moving(),
        )
    });
    let mut picked = None;
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(list.min)
        .show(&ctx, |area| {
            let painter = area.painter().clone();
            let _ = paint_list_card(&painter, list);
            let _ = area.allocate_rect(list, Sense::click());
            let top = list.top() + 7.0;
            if rows.is_empty() {
                // The hint, its keys in inline caps.
                let line = pos2(list.left() + PAD, top + 4.0);
                let lead =
                    SNIPPET_ROW.paint_line(&painter, line, "To create a Snippet, hit", muted, None);
                let caps = exo_ui::Keycaps::new(
                    &ctx,
                    &["⌘", "K"],
                    exo_ui::KeycapColors::shortcuts_pane(palette),
                );
                caps.paint(&painter, line.x + lead, top + 8.0);
                SNIPPET_ROW.paint_line(
                    &painter,
                    pos2(line.x + lead + caps.width() + 4.0, line.y),
                    "→ \"Snippets\"",
                    muted,
                    None,
                );
                return;
            }
            for (index, row) in rows.iter().enumerate() {
                let rect = Rect::from_min_size(
                    pos2(list.left(), top + index as f32 * ROW),
                    vec2(list.width(), ROW),
                );
                if index == open.snippet_pick {
                    painter.rect_filled(exo_ui::pixel_snap(rect), 0.0, selected);
                }
                let line = rect.top() + 4.0;
                let name_width = SNIPPET_ROW.paint_line(
                    &painter,
                    pos2(rect.left() + PAD, line),
                    &row.name,
                    name_color,
                    Some(275.0f32.max(rect.width() / 2.0)),
                );
                let me_left = rect.right() - PAD - me;
                let preview_left = rect.left() + PAD + name_width + PAD;
                SNIPPET_ROW.paint_line(
                    &painter,
                    pos2(preview_left, line),
                    &row.preview,
                    muted,
                    Some((me_left - PAD - preview_left).clamp(0.0, 300.0)),
                );
                SNIPPET_ROW.paint_line(&painter, pos2(me_left, line), "Me", muted, None);
                match pointer {
                    (Some(at), pressed, moving) if rect.contains(at) => {
                        if moving && index != open.snippet_pick {
                            open.snippet_pick = index;
                        }
                        if pressed {
                            picked = Some(index);
                        }
                    }
                    _ => {}
                }
            }
        });
    picked
}

/// The `;` picker's keys while it has rows and isn't hidden by a date, then the `:` and contact
/// dropdown's, in the body or the subject.
fn picker_keys(commands: &mut Commands<commands::Action>, open: &Open, backtick: bool) {
    const FIELDS: &[Region] = &[Region::ComposeForm];
    let act = commands::Action::Compose;
    if open.snippet_trigger.is_some() && !open.snippet_rows.is_empty() && open.date.is_none() {
        commands.add(
            Command::shortcut("SNIPPET_DOWN", act(Action::SnippetMove(1)))
                .key("down")
                .regions(FIELDS),
        );
        commands.add(
            Command::shortcut("SNIPPET_UP", act(Action::SnippetMove(-1)))
                .key("up")
                .regions(FIELDS),
        );
        commands.add(
            Command::shortcut("SNIPPET_PICK", act(Action::SnippetPick(None)))
                .keys(["enter", "tab"])
                .regions(FIELDS),
        );
        commands.add(
            Command::shortcut("SNIPPET_CLOSE", act(Action::SnippetClose))
                .key("esc")
                .backtick_escape(backtick)
                .regions(FIELDS),
        );
    }
    super::assistant::register(commands, open, backtick);
}

/// Commits the focused field's typed text as chips (a pasted list splits into several, with
/// what isn't an address left typed).
pub(super) fn commit_input(open: &mut Open, now: f64, book: Option<&contacts::Index>) {
    let Some(list) = open.state.field.list() else {
        return;
    };
    // The input moves on unrevealed: committing sets its value, typing nothing.
    open.header_scroll.reveal = false;
    if super::chips::finish_in_place(open, list, book, now) {
        return;
    }
    // The highlighted suggestion, when they show.
    if open.input(list).suggesting() {
        let input = open.input_mut(list);
        let picked = input.suggestions[input.highlight.min(input.suggestions.len() - 1)].clone();
        input.text.clear();
        input.suggestions.clear();
        input.query.clear();
        input.selected.clear();
        if !open
            .list(list)
            .iter()
            .any(|address| address.email.eq_ignore_ascii_case(&picked.email))
        {
            open.list_mut(list).push(picked);
        }
        open.touch(now);
        return;
    }
    let text = std::mem::take(&mut open.input_mut(list).text);
    let parts = if text.contains([',', ';', '\n']) {
        recipients::split_pasted(&text)
    } else {
        vec![text.trim().to_owned()]
    };
    let mut leftover = String::new();
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        let mut address = recipients::parse(part);
        // Parts without a name take the address book's.
        if address.name.is_none() && !address.email.is_empty() {
            address.name = book.and_then(|book| book.name_for(&address.email));
        }
        let last = index + 1 == parts.len();
        if last && parts.len() > 1 && !recipients::sendable(&address) {
            leftover = part.clone();
            continue;
        }
        if open.recipients().count() >= recipients::MAX_RECIPIENTS {
            break;
        }
        open.list_mut(list).push(address);
    }
    open.input_mut(list).text = leftover;
    open.input_mut(list).selected.clear();
    open.touch(now);
}

/// Commits every list's typed text (as Send does).
pub(super) fn commit_all(open: &mut Open, now: f64, book: Option<&contacts::Index>) {
    let field = open.state.field;
    for list in List::ALL {
        if !open.input(list).text.trim().is_empty() {
            open.state.field = list.field();
            commit_input(open, now, book);
        }
    }
    open.state.field = field;
}

/// Backspace in an empty recipient input: the chip before the caret goes, or the one being
/// edited, and the one before that is selected.
pub(super) fn backspace(open: &mut Open, now: f64) {
    let Some(list) = open.state.field.list() else {
        return;
    };
    let (index, in_place) = match open.input_mut(list).editing.take() {
        Some(index) => (index, true),
        None => match open.list(list).len().checked_sub(1) {
            Some(last) => (last, false),
            None => return,
        },
    };
    if index < open.list(list).len() {
        open.list_mut(list).remove(index);
    }
    let count = open.list(list).len();
    let input = open.input_mut(list);
    input.text.clear();
    input.selected = if in_place && count > 0 {
        vec![index.saturating_sub(1)]
    } else {
        Vec::new()
    };
    input.anchor = input.selected.first().copied();
    open.touch(now);
}

/// ← / → with nothing typed, or at the edge of what's typed: moves the chip selection; past the
/// last chip the caret returns to the input. An edit finishes instead: a chip edited in place
/// stays selected (the one before it when it was emptied); text typed at the end becomes a
/// chip, selected after ←, with the caret after it after →.
pub(super) fn move_selection(
    open: &mut Open,
    delta: isize,
    book: Option<&contacts::Index>,
    now: f64,
) {
    let Some(list) = open.state.field.list() else {
        return;
    };
    if let Some(index) = open.input(list).editing {
        let emptied = open.input(list).text.trim().is_empty();
        super::chips::finish_in_place(open, list, book, now);
        if emptied {
            let count = open.list(list).len();
            let input = open.input_mut(list);
            input.selected = if count > 0 {
                vec![index.saturating_sub(1).min(count - 1)]
            } else {
                Vec::new()
            };
            input.anchor = input.selected.first().copied();
        }
        return;
    }
    if !open.input(list).text.trim().is_empty() {
        let before = open.list(list).len();
        commit_input(open, now, book);
        let count = open.list(list).len();
        let input = open.input_mut(list);
        input.selected = if delta < 0 && count > before {
            vec![count - 1]
        } else {
            Vec::new()
        };
        input.anchor = input.selected.first().copied();
        return;
    }
    let count = open.list(list).len();
    let input = open.input_mut(list);
    // From the selection's first chip going left, its last going right.
    let current = if delta < 0 {
        input.selected.first()
    } else {
        input.selected.last()
    }
    .copied()
    .unwrap_or(count);
    let next = current.saturating_add_signed(delta).min(count);
    input.selected = if next == count {
        Vec::new()
    } else {
        vec![next]
    };
    input.anchor = input.selected.first().copied();
}

/// The field Tab (1) or ⇧Tab (-1) moves to, in the form's tab order, which wraps around: the
/// header's fields, the subject (a reply's too, read-only, when Cc and Bcc show), the body,
/// Send, the icons, then the header buttons, the last first. Send later and Remind me follow
/// Send (but not on a snippet).
pub(super) fn next_field(open: &Open, aliases: &[Alias], surface: Surface, delta: isize) -> Field {
    let mut order = Vec::new();
    // A snippet's name comes first; it has no collapsed header.
    if open.is_snippet() {
        order.push(Field::SnippetName);
    }
    if open.state.header_expanded {
        order.push(Field::To);
        if open.state.cc_bcc_shown {
            order.extend([Field::Cc, Field::Bcc]);
            if active_aliases(aliases) > 1 {
                order.push(Field::From);
            }
        }
    } else if !open.is_snippet() {
        order.push(Field::CollapsedHeader);
    }
    let subject = if open.is_snippet() {
        open.state.header_expanded
    } else {
        open.is_new_email() || open.state.header_expanded && open.state.cc_bcc_shown
    };
    if subject {
        order.push(Field::Subject);
    }
    order.extend([Field::Body, Field::Send]);
    if !open.is_snippet() {
        order.extend([Field::SendLater, Field::RemindMe]);
    }
    order.extend([Field::Attach, Field::Discard]);
    order.extend(
        buttons(open, surface)
            .iter()
            .rev()
            .map(|button| button.field()),
    );
    // The `…` isn't in the tab order: from it, Tab goes on from the body.
    let current = if open.state.field == Field::QuotingStart {
        Field::Body
    } else {
        open.state.field
    };
    let at = order
        .iter()
        .position(|field| *field == current)
        .unwrap_or(0);
    order[(at as isize + delta).rem_euclid(order.len() as isize) as usize]
}

/// The `…`: puts the signature and quote into the body.
pub(super) fn reveal_quote(open: &mut Open, aliases: &[Alias], now: f64) {
    if open.record.quoted_inlined {
        return;
    }
    let quote = open.record.quoted_content.clone();
    open.editor
        .reveal_quoted(Some(&signature::revealed(open, aliases)), quote.as_deref());
    open.record.quoted_inlined = true;
    open.state.field = Field::Body;
    open.touch(now);
}

/// Registers the focused draft's commands. With `Send + Mark Done` on (`send_and_done`), `Send &
/// Mark Done` takes ⌘ Enter too and `Send` goes, except in the new-message pane (`in_pane`),
/// which never marks done. `backtick` is Backtick as Escape: without it, compose's Escape leaves
/// a backtick to be typed.
pub(super) fn register(
    commands: &mut Commands<commands::Action>,
    open: &Open,
    aliases: &[Alias],
    popped_out: bool,
    send_and_done: bool,
    in_pane: bool,
    backtick: bool,
) {
    const FORM: &[Region] = &[Region::ComposeForm, Region::RecipientList];
    const LISTS: &[Region] = &[Region::RecipientList];
    const BODY_AND_FIELDS: &[Region] = &[Region::ComposeForm];
    let act = commands::Action::Compose;
    // A snippet's `Send` is `Save Snippet`; it has no Send & Mark Done and doesn't pop out.
    let snippet = open.is_snippet();
    let send_title = if snippet { "Save Snippet" } else { "Send" };
    let send_keys: &[&'static str] = if send_and_done {
        &["mod+enter", "mod+shift+enter"]
    } else {
        &["mod+enter"]
    };
    if !(send_and_done && !in_pane && !snippet) {
        commands.add(
            Command::new("SEND", send_title, act(Action::Send { mark_done: false }))
                .keys(send_keys.iter().copied())
                .key_code(["⌘", "enter"])
                .aliases(["Ship It"])
                .default_score(100.0)
                .regions(FORM),
        );
    }
    if !snippet && !(send_and_done && in_pane) {
        let (keys, key_code, score): (&[&'static str], &[&'static str], f64) = if send_and_done {
            (send_keys, &["⌘", "enter"], 101.0)
        } else {
            (&["mod+shift+enter"], &["⌘", "shift", "enter"], 92.0)
        };
        commands.add(
            Command::new(
                "SEND_AND_MARK_DONE",
                "Send & Mark Done",
                act(Action::Send { mark_done: true }),
            )
            .keys(keys.iter().copied())
            .key_code(key_code.iter().copied())
            .aliases(["Send and Mark Done", "Send + Mark Done"])
            .default_score(score)
            .regions(FORM),
        );
    }
    if !snippet {
        // `Remind Me` (its Snooze id), and `Remove Reminder` once the draft has one.
        commands.add(
            Command::new("SNOOZE", "Remind Me", act(Action::RemindMe))
                .key("mod+shift+h")
                .key_code(["⌘", "shift", "H"])
                .aliases(["Add Reminder", "Snooze", "Boomerang"])
                .default_score(98.0)
                .icon("data/snooze")
                .regions(FORM),
        );
        if open.record.reminder.is_some() {
            commands.add(
                Command::new(
                    "REMOVE_REMINDER",
                    "Remove Reminder",
                    commands::Action::Remind(crate::remind::Action::RemoveFromDraft {
                        draft: open.id.clone(),
                        manual: false,
                    }),
                )
                .default_score(95.0)
                .icon("data/snooze")
                .regions(FORM),
            );
        }
        commands.add(
            Command::new("SEND_LATER", "Send Later", act(Action::SendLater))
                .key("mod+shift+l")
                .key_code(["⌘", "shift", "L"])
                .aliases(["Schedule", "Delayed Send", "Delay Send"])
                .default_score(99.0)
                .icon("data/send")
                .regions(FORM),
        );
    }
    commands.add(
        Command::new("DISCARD_DRAFTS", "Discard Draft", act(Action::Discard))
            .keys(["mod+shift+,", "mod+shift+."])
            .key_code(["⌘", "shift", ","])
            .aliases(["Delete Draft", "Trash Draft", "Abandon Draft"])
            .default_score(95.0)
            .scale(1.1)
            .regions(FORM),
    );
    let (title, aliases_of_pop) = if popped_out {
        ("Pop In Draft", ["Pop-in", "Popin"])
    } else {
        ("Pop Out Draft", ["Pop-out", "Popout"])
    };
    if !snippet {
        commands.add(
            Command::new("POP_OUT_DRAFT", title, act(Action::Pop))
                .key("mod+shift+p")
                .key_code(["⌘", "shift", "P"])
                .aliases(aliases_of_pop)
                .regions(FORM),
        );
        let title = if popped_out {
            "Search"
        } else {
            "Pop Out Draft & Search"
        };
        commands.add(
            Command::new("SEARCH", title, act(Action::PopAndSearch))
                .key("mod+/")
                .key_code(["⌘", "/"])
                .default_score(1.6)
                .icon("data/search")
                .regions(FORM),
        );
    }
    if !snippet {
        commands.add(
            crate::calendar::share_availability_command(act(Action::AvailabilityMenu(true)))
                .key("mod+shift+a")
                .key_code(["⌘", "shift", "A"])
                .regions(FORM),
        );
    }
    commands.add(
        Command::new("ATTACH", "Attach", act(Action::Attach))
            .key("mod+shift+u")
            .key_code(["⌘", "shift", "U"])
            .aliases(["Add Attachment"])
            .default_score(96.0)
            .regions(FORM),
    );
    commands.add(
        Command::new("INSERT_IMAGE", "Insert Image", act(Action::InsertImage))
            .aliases(["Attach Image"])
            .default_score(90.0)
            .icon("data/insert-image")
            .regions(FORM),
    );
    let change = |id: &'static str,
                  title: &'static str,
                  field: Field,
                  key: &'static str,
                  code: &'static str| {
        Command::new(id, title, act(Action::Focus(field)))
            .key(key)
            .key_code(["⌘", "shift", code])
            .default_score(94.0)
            .regions(FORM)
    };
    commands.add(change(
        "CHANGE_TO",
        "Change To",
        Field::To,
        "mod+shift+o",
        "O",
    ));
    commands
        .add(change("CHANGE_CC", "Change CC", Field::Cc, "mod+shift+c", "C").aliases(["Add CC"]));
    commands.add(
        change("CHANGE_BCC", "Change BCC", Field::Bcc, "mod+shift+b", "B").aliases(["Add BCC"]),
    );
    if active_aliases(aliases) > 1 && !snippet {
        commands.add(
            change(
                "CHANGE_FROM",
                "Change From",
                Field::From,
                "mod+shift+f",
                "F",
            )
            .aliases(["Change Alias", "Change Aliases"]),
        );
    }
    if open.is_new_email() {
        commands.add(
            change(
                "CHANGE_SUBJECT",
                "Change Subject",
                Field::Subject,
                "mod+shift+s",
                "S",
            )
            .aliases(["Edit Subject"]),
        );
    } else {
        // A reply's: the palette's row makes it a new message, ⌘⇧S only focuses its subject.
        commands.add(
            Command::new("CHANGE_SUBJECT", "Change Subject", act(Action::EditSubject))
                .key("mod+shift+s")
                .key_code(["⌘", "shift", "S"])
                .display_only()
                .aliases(["Edit Subject"])
                .default_score(94.0)
                .regions(FORM),
        );
        commands.add(
            Command::shortcut("CHANGE_SUBJECT_KEY", act(Action::Focus(Field::Subject)))
                .key("mod+shift+s")
                .regions(FORM),
        );
    }
    let tabs = |commands: &mut Commands<commands::Action>| {
        commands.add(
            Command::shortcut("TAB", act(Action::Tab(1)))
                .key("tab")
                .regions(FORM),
        );
        commands.add(
            Command::shortcut("SHIFT_TAB", act(Action::Tab(-1)))
                .key("shift+tab")
                .regions(FORM),
        );
    };
    // Compose's own Escape ignores a backtick unless Backtick as Escape is on.
    let escape = |commands: &mut Commands<commands::Action>| {
        commands.add(
            Command::shortcut("ESCAPE", act(Action::Escape))
                .key("esc")
                .backtick_escape(backtick)
                .regions(FORM),
        )
    };
    // ⌘O is swallowed in compose (the editor's colour picker has it).
    match open.state.field {
        Field::SnippetName => {
            // Enter goes on to To with the recipients showing, else the body.
            let next = if open.state.header_expanded {
                Field::To
            } else {
                Field::Body
            };
            commands.add(
                Command::shortcut("SNIPPET_NAME_ENTER", act(Action::Focus(next)))
                    .key("enter")
                    .regions(BODY_AND_FIELDS),
            );
            escape(commands);
            tabs(commands);
        }
        Field::Body => {
            // Escape the editor has no use for leaves the form (egui would take it as a blur). The
            // editor keeps a backtick to type it unless Backtick as Escape is on, and then types
            // one for ⌥⇧`.
            if !open.editor.wants_escape() {
                commands.add(
                    Command::shortcut("ESCAPE", act(Action::Escape))
                        .key("esc")
                        .backtick_escape(backtick)
                        .regions(BODY_AND_FIELDS),
                );
            }
            if backtick {
                commands.add(
                    Command::shortcut("INSERT_BACKTICK", act(Action::TypeBacktick))
                        .key("alt+shift+`")
                        .regions(BODY_AND_FIELDS),
                );
            }
            // Only on a date do they move it (they go through otherwise).
            if open.date.is_some() {
                commands.add(
                    Command::shortcut("PREVIOUS_DAY", act(Action::ShiftDate(-1)))
                        .key("mod+shift+-")
                        .regions(BODY_AND_FIELDS),
                );
                commands.add(
                    Command::shortcut("NEXT_DAY", act(Action::ShiftDate(1)))
                        .key("mod+shift+=")
                        .regions(BODY_AND_FIELDS),
                );
            }
            // Squire's keymap; Tab and ⇧Tab outside a list move focus (to Send, or back).
            for command in FormatCommand::ALL
                .into_iter()
                .filter(|command| command.applies(&open.editor))
            {
                commands.add(
                    Command::shortcut(command.id(), act(Action::Format(command)))
                        .keys(command.keys().iter().copied())
                        .regions(BODY_AND_FIELDS),
                );
            }
            // Nor while the editor takes it (ghost text, a suggestion chosen in autocorrect's pill).
            if !open.editor.in_list() && !open.editor.wants_tab() {
                tabs(commands);
            }
            // Share Availability's menu.
            if let Some(menu) = &open.availability_menu {
                // Its one row stays highlighted.
                commands.add(
                    Command::shortcut("AVAILABILITY_MOVE", act(Action::AvailabilityMenu(true)))
                        .keys(["up", "down"])
                        .regions(BODY_AND_FIELDS),
                );
                // Enter picks only while a row shows; with none it types a line.
                if menu.shows_row() {
                    commands.add(
                        Command::shortcut("AVAILABILITY_PICK", act(Action::AvailabilityPick))
                            .keys(["enter", "tab"])
                            .regions(BODY_AND_FIELDS),
                    );
                }
                commands.add(
                    Command::shortcut("AVAILABILITY_CLOSE", act(Action::AvailabilityMenu(false)))
                        .key("esc")
                        .backtick_escape(backtick)
                        .regions(BODY_AND_FIELDS),
                );
                // ← or Backspace at the query's start and → at its end close it there.
                let caret = open.editor.selection().focus;
                if caret == menu.start {
                    commands.add(
                        Command::shortcut(
                            "AVAILABILITY_LEAVE",
                            act(Action::AvailabilityMenu(false)),
                        )
                        .keys(["left", "backspace"])
                        .regions(BODY_AND_FIELDS),
                    );
                } else if caret == menu.end() {
                    commands.add(
                        Command::shortcut(
                            "AVAILABILITY_LEAVE",
                            act(Action::AvailabilityMenu(false)),
                        )
                        .key("right")
                        .regions(BODY_AND_FIELDS),
                    );
                }
            }
            picker_keys(commands, open, backtick);
        }
        Field::QuotingStart => {
            commands.add(
                Command::shortcut("REVEAL_QUOTE", act(Action::RevealQuote))
                    .keys(["space", "o", "enter"])
                    .regions(BODY_AND_FIELDS),
            );
            commands.add(
                Command::shortcut("DELETE_QUOTE", act(Action::DeleteQuote))
                    .key("backspace")
                    .regions(BODY_AND_FIELDS),
            );
            commands.add(
                Command::shortcut("QUOTE_BACK", act(Action::Focus(Field::Body)))
                    .keys(["up", "left", "down"])
                    .regions(BODY_AND_FIELDS),
            );
            commands.add(
                Command::shortcut("ESCAPE", act(Action::Focus(Field::Body)))
                    .key("esc")
                    .regions(BODY_AND_FIELDS),
            );
            tabs(commands);
        }
        Field::CollapsedHeader => {
            commands.add(
                Command::shortcut("EXPAND_HEADER", act(Action::Focus(Field::To)))
                    .key("enter")
                    .regions(BODY_AND_FIELDS),
            );
            escape(commands);
            tabs(commands);
        }
        Field::Subject => {
            // A new message's subject goes on to the body; a reply's starts a new message.
            let enter = if open.is_new_email() {
                Action::Focus(Field::Body)
            } else {
                Action::EditSubject
            };
            commands.add(
                Command::shortcut("SUBJECT_ENTER", act(enter))
                    .key("enter")
                    .regions(BODY_AND_FIELDS),
            );
            escape(commands);
            tabs(commands);
            // A new message's `:` and `;` pickers, over those.
            picker_keys(commands, open, backtick);
        }
        Field::From if open.from_menu.is_some() => {
            // The list's keys; Enter or Tab takes the highlighted alias, Escape only closes it.
            commands.add(
                Command::shortcut("FROM_DOWN", act(Action::FromMove(1)))
                    .keys(["down", "j", "n"])
                    .regions(FORM),
            );
            commands.add(
                Command::shortcut("FROM_UP", act(Action::FromMove(-1)))
                    .keys(["up", "k", "p"])
                    .regions(FORM),
            );
            commands.add(
                Command::shortcut("FROM_CHOOSE", act(Action::FromChoose(None)))
                    .keys(["enter", "tab", "shift+tab"])
                    .regions(FORM),
            );
            commands.add(
                Command::shortcut("ESCAPE", act(Action::FromClose))
                    .key("esc")
                    .regions(FORM),
            );
        }
        Field::From => {
            // ↓ or ↑ opens the list; Enter clicks the button, which does too.
            commands.add(
                Command::shortcut("FROM_OPEN", act(Action::FromToggle))
                    .keys(["down", "j", "n", "up", "k", "p", "enter"])
                    .regions(FORM),
            );
            escape(commands);
            tabs(commands);
        }
        Field::Send
        | Field::SendLater
        | Field::RemindMe
        | Field::Attach
        | Field::Discard
        | Field::ExpandRecipients
        | Field::PopAction
        | Field::PopClose => {
            // Enter on a focused control clicks it; a keyboard Send shows no hint.
            let click = match open.state.field {
                Field::Send => Action::Send { mark_done: false },
                Field::SendLater => Action::SendLater,
                Field::RemindMe => Action::RemindMe,
                Field::Attach => Action::Attach,
                Field::Discard => Action::Discard,
                Field::ExpandRecipients => Action::Focus(Field::Cc),
                Field::PopAction => Action::Pop,
                _ => Action::ClosePopout,
            };
            commands.add(
                Command::shortcut("CLICK", act(click))
                    .key("enter")
                    .regions(FORM),
            );
            escape(commands);
            tabs(commands);
        }
        Field::To | Field::Cc | Field::Bcc => {
            let list = open.state.field.list().expect("a recipient field");
            let input = open.input(list);
            // The recipient field's own Escape takes a backtick whatever the setting.
            commands.add(
                Command::shortcut("ESCAPE", act(Action::Escape))
                    .key("esc")
                    .regions(FORM),
            );
            tabs(commands);
            commands.add(
                Command::shortcut("COMMIT", act(Action::Commit))
                    .key("enter")
                    .regions(LISTS),
            );
            if input.suggesting() {
                // With suggestions: ↑/↓ (and ⌃J/N, ⌃K/P) move the highlight, Tab takes it and
                // stays, Escape closes them.
                commands.add(
                    Command::shortcut("SUGGESTION_DOWN", act(Action::MoveSuggestion(1)))
                        .keys(["down", "ctralt+j", "ctralt+n"])
                        .regions(LISTS),
                );
                commands.add(
                    Command::shortcut("SUGGESTION_UP", act(Action::MoveSuggestion(-1)))
                        .keys(["up", "ctralt+k", "ctralt+p"])
                        .regions(LISTS),
                );
                commands.add(
                    Command::shortcut("SUGGESTION_TAB", act(Action::Commit))
                        .key("tab")
                        .regions(LISTS),
                );
                commands.add(
                    Command::shortcut("SUGGESTION_ESCAPE", act(Action::CloseSuggestions))
                        .key("esc")
                        .regions(LISTS),
                );
            }
            if input.text.contains('@') {
                commands.add(
                    Command::shortcut("COMMIT_PUNCTUATION", act(Action::Commit))
                        .keys([",", ";"])
                        .regions(LISTS),
                );
            }
            if input.text.is_empty() {
                commands.add(
                    Command::shortcut("CHIP_BACKSPACE", act(Action::Backspace))
                        .keys(["backspace", "del"])
                        .regions(LISTS),
                );
                commands.add(
                    Command::shortcut("CHIP_LEFT", act(Action::MoveSelection(-1)))
                        .key("left")
                        .regions(LISTS),
                );
                if !input.selected.is_empty() || input.editing.is_some() {
                    commands.add(
                        Command::shortcut("CHIP_RIGHT", act(Action::MoveSelection(1)))
                            .key("right")
                            .regions(LISTS),
                    );
                }
            } else if input.caret == Some(0) {
                // At the typed text's edge, ← and → finish it.
                commands.add(
                    Command::shortcut("CHIP_LEFT", act(Action::MoveSelection(-1)))
                        .key("left")
                        .regions(LISTS),
                );
            } else if input.caret == Some(input.text.chars().count()) {
                commands.add(
                    Command::shortcut("CHIP_RIGHT", act(Action::MoveSelection(1)))
                        .key("right")
                        .regions(LISTS),
                );
            }
            super::chips::register(commands, open, list);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exo_store::{DraftAction, DraftRecord};

    fn reply(cc: &[&str]) -> Open {
        let address = |email: &&str| Address {
            name: None,
            email: (*email).into(),
        };
        let record = DraftRecord {
            action: DraftAction::ReplyAll,
            from: Some(Address {
                name: None,
                email: "maya@brightmoor.co".into(),
            }),
            to: vec![Address {
                name: None,
                email: "tom@brightmoor.co".into(),
            }],
            cc: cc.iter().map(address).collect(),
            bcc: Vec::new(),
            subject: "Re: Lunch".into(),
            body: String::new(),
            quoted_content: None,
            quoted_inlined: false,
            in_reply_to: None,
            in_reply_to_rfc822: None,
            references: Vec::new(),
            rfc822_id: "x@y".into(),
            fingerprint: String::new(),
            attachments: Vec::new(),
            send_later: None,
            reminder: None,
        };
        Open::new("draft00aaaaaaaaaaaaaa".into(), Some("t".into()), record)
    }

    fn aliases(count: usize) -> Vec<Alias> {
        [
            "maya@brightmoor.co",
            "hello@mayaokafor.com",
            "a@brightmoor.co",
        ][..count]
            .iter()
            .map(|email| Alias {
                email: (*email).into(),
                name: None,
                signature: None,
                default: false,
                primary: false,
                active: true,
            })
            .collect()
    }

    fn walk(
        open: &mut Open,
        aliases: &[Alias],
        surface: Surface,
        delta: isize,
        steps: usize,
    ) -> Vec<Field> {
        (0..steps)
            .map(|_| {
                open.state.field = next_field(open, aliases, surface, delta);
                open.state.field
            })
            .collect()
    }

    #[test]
    fn the_input_takes_the_next_line_unless_its_scroll_box_fits() {
        let ctx = egui::Context::default();
        exo_ui::install(&ctx).expect("the system fonts load");
        let open = reply(&[]);
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            // Tom's chip and its margin, then the input's box, 26px wider than its 2px field:
            // 28px, nothing typed.
            let line =
                LABEL_WIDTH + FIELD.width(ui.ctx(), "tom@brightmoor.co") + CHIP_EXTRA + 1.0 + 28.0;
            assert_eq!(
                lay_out_row(ui.ctx(), &open, Some(List::To), 0.0, line).lines,
                1
            );
            let wrapped = lay_out_row(ui.ctx(), &open, Some(List::To), 0.0, line - 1.0);
            assert_eq!(
                wrapped.lines, 2,
                "with less left, the input goes on a line of its own"
            );
            assert_eq!(
                wrapped.input.map(|input| (input.left(), input.width())),
                Some((LABEL_WIDTH, 10.0)),
                "its 2px field padded 6px in a 1px border"
            );
        });
        output.drop_without_applying_deltas();
    }

    #[test]
    fn tab_goes_through_send_and_the_icons_and_wraps() {
        let mut collapsed = reply(&[]);
        assert_eq!(collapsed.state.field, Field::Body);
        let order = walk(&mut collapsed, &aliases(1), Surface::Inline, 1, 8);
        assert_eq!(
            order,
            [
                Field::Send,
                Field::SendLater,
                Field::RemindMe,
                Field::Attach,
                Field::Discard,
                Field::PopAction,
                Field::CollapsedHeader,
                Field::Body
            ]
        );
        assert_eq!(
            walk(&mut collapsed, &aliases(1), Surface::Inline, -1, 2),
            [Field::CollapsedHeader, Field::PopAction]
        );

        let mut expanded = reply(&["priya@brightmoor.co"]);
        expanded.state.expand(Field::Discard);
        let order = walk(&mut expanded, &aliases(2), Surface::Popout, 1, 8);
        assert_eq!(
            order,
            [
                Field::PopClose,
                Field::PopAction,
                Field::To,
                Field::Cc,
                Field::Bcc,
                Field::From,
                Field::Subject,
                Field::Body
            ],
            "a reply's subject is in the order once Cc shows"
        );
    }

    #[test]
    fn the_action_panes_dates_narrow_with_a_reminder_and_a_send_time() {
        let now: jiff::Zoned = "2026-11-17T09:00[America/New_York]".parse().expect("now");
        let at = |text: &str| text.parse::<jiff::Zoned>().expect("a time").timestamp();
        let cx = |surface| Context {
            aliases: &[],
            me: "",
            surface,
            focused: false,
            book: None,
            thread_people: None,
            pin: Rangef::EVERYTHING,
            drop: None,
            now: &now,
            thread_reminder: None,
            send_and_done: false,
            max_height: f32::INFINITY,
            availability_tip: false,
        };
        let mut open = reply(&[]);
        open.record.reminder = Some(exo_store::DraftReminder {
            at: at("2026-11-19T08:00[America/New_York]"),
            keep_on_reply: false,
            automatic: false,
        });
        assert_eq!(
            date_format(&open, 640.0, &cx(Surface::Inline)),
            ComposeFormat::Compact,
            "a reminder 2 days out (`Thu Nov 19, 8:00 AM`)"
        );
        open.record.reminder = Some(exo_store::DraftReminder {
            at: at("2026-12-01T08:00[America/New_York]"),
            keep_on_reply: false,
            automatic: false,
        });
        assert_eq!(
            date_format(&open, 640.0, &cx(Surface::Inline)),
            ComposeFormat::Friendly
        );
        assert_eq!(
            date_format(&open, 590.0, &cx(Surface::Inline)),
            ComposeFormat::ExtraCompact
        );
        assert_eq!(
            date_format(&open, 640.0, &cx(Surface::Popout)),
            ComposeFormat::ExtraCompact,
            "popped out, a reminder alone"
        );
        open.record.send_later = Some(exo_store::SendLater {
            at: at("2026-11-18T08:00[America/New_York]"),
            abort_on_reply: false,
            current_message_ids: Vec::new(),
        });
        assert_eq!(
            date_format(&open, 640.0, &cx(Surface::Inline)),
            ComposeFormat::MostCompact,
            "both, under 641px"
        );
        assert_eq!(
            date_format(&open, 700.0, &cx(Surface::Inline)),
            ComposeFormat::Compact
        );
        open.record.send_later = Some(exo_store::SendLater {
            at: at("2026-11-25T08:00[America/New_York]"),
            abort_on_reply: false,
            current_message_ids: Vec::new(),
        });
        assert_eq!(
            date_format(&open, 700.0, &cx(Surface::Inline)),
            ComposeFormat::ExtraCompact,
            "both after tomorrow"
        );
    }

    #[test]
    fn the_actions_close_up_popped_out_and_while_a_send_marks_done_with_a_date() {
        let now: jiff::Zoned = "2026-11-17T09:00[America/New_York]".parse().expect("now");
        let cx = |surface| Context {
            aliases: &[],
            me: "",
            surface,
            focused: false,
            book: None,
            thread_people: None,
            pin: Rangef::EVERYTHING,
            drop: None,
            now: &now,
            thread_reminder: None,
            send_and_done: false,
            max_height: f32::INFINITY,
            availability_tip: false,
        };
        let mut open = reply(&[]);
        assert_eq!(
            action_gap(&open, 640.0, &cx(Surface::Inline), false),
            32.0,
            "`margin-right: 22px` between 10px paddings"
        );
        assert_eq!(
            action_gap(&open, 480.0, &cx(Surface::Inline), false),
            22.0,
            "12px in a pane up to 480px"
        );
        assert_eq!(
            action_gap(&open, 500.0, &cx(Surface::Popout), false),
            15.0,
            "popped out, 5px"
        );
        open.record.reminder = Some(exo_store::DraftReminder {
            at: now.timestamp(),
            keep_on_reply: false,
            automatic: false,
        });
        assert_eq!(
            action_gap(&open, 500.0, &cx(Surface::Popout), true),
            13.0,
            "popped out, 3px with a reminder, no send time, marking done"
        );
        assert_eq!(
            action_gap(&open, 640.0, &cx(Surface::Inline), true),
            15.0,
            "a date set while a send marks done"
        );
        assert_eq!(action_gap(&open, 640.0, &cx(Surface::Inline), false), 32.0);
    }

    #[test]
    fn a_moved_caret_scrolls_in_as_chromium_reveals_it() {
        let scrolled = Scrolled {
            top: 0.0,
            bottom: 200.0,
            content: 400.0,
        };
        let caret = |top: f32| Rect::from_min_size(pos2(0.0, top), vec2(1.0, 18.0));
        assert_eq!(reveal(caret(40.0), 500.0, scrolled, 0.0), 0.0, "in view");
        assert_eq!(
            reveal(caret(190.0), 500.0, scrolled, 0.0),
            8.0,
            "partly below: to the edge"
        );
        assert_eq!(
            reveal(caret(300.0), 500.0, scrolled, 0.0),
            209.0,
            "out of view: to the middle"
        );
        assert_eq!(
            reveal(caret(280.0), 300.0, scrolled, 150.0),
            50.0,
            "on the last line, with what's after it"
        );
    }

    #[test]
    fn the_suggestion_list_keeps_the_widest_width_it_has_had_since_it_opened() {
        // Text widths (px): `p` shows Paul Durand (71.9) at pdurand@westbrook-realty.com (188.6);
        // `pri` then shows Priya Raman (73.5) at priya@brightmoor.co (122.5).
        let mut seen = 0.0;
        let (name, email) = suggestion_columns(71.9, 188.6, &mut seen);
        assert_eq!(
            (name, (email * 10.0).round() / 10.0),
            (165.0, 220.6),
            "the name column's 165px includes its padding"
        );
        let (name, email) = suggestion_columns(73.5, 122.5, &mut seen);
        assert!(
            (name - 199.3).abs() < 0.5 && (email - 186.7).abs() < 0.5,
            "measured 199.33 / 186.67, got {name} / {email}"
        );
    }

    #[test]
    fn the_from_list_puts_the_selected_alias_first() {
        let mut open = reply(&[]);
        open.record.from = Some(Address {
            name: None,
            email: "hello@mayaokafor.com".into(),
        });
        let aliases = aliases(3);
        let choices: Vec<String> = from_choices(&open, &aliases)
            .into_iter()
            .map(choice_label)
            .collect();
        assert_eq!(
            choices,
            [
                "hello@mayaokafor.com",
                "a@brightmoor.co",
                "maya@brightmoor.co"
            ]
        );
        from_menu(&mut open, &aliases, &Action::FromToggle, 0.0);
        from_menu(&mut open, &aliases, &Action::FromMove(-1), 0.0);
        assert_eq!(open.from_menu, Some(2), "↑ wraps");
        from_menu(&mut open, &aliases, &Action::FromChoose(None), 0.0);
        assert_eq!(
            (open.from_menu, open.record.from.map(|from| from.email)),
            (None, Some("maya@brightmoor.co".to_owned()))
        );
    }
}
