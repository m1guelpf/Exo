//! A split template's settings: the template's description over its preview, its definition
//! (senders the user adds, or the template's own, and what its query does), the split's
//! checkboxes, and a footer that turns the split on or off, saves or deletes it. Important • Other
//! and Inbox have templates too, which turn Important and Other on or off.

use egui::{Color32, CornerRadius, Rect, Sense, Ui, pos2, vec2};
use exo_store::{Settings, SplitInbox, Template};
use exo_ui::{
    ActionKind, CHECKBOX_SIZE, DIALOG_ACTION_HEIGHT, DialogAction, DialogInput, Family, Font,
    TextStyle, paint_checkbox, palette,
};

use super::split_preview::{self, Place};
use crate::{
    compose::contacts,
    contact_picker::{ContactPicker, PickerKey, is_valid_domain},
};

const BODY: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
const BODY_BOLD: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 12.0, 20.0);
const SMALL: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
/// The description: 14px/24px, `max-width: 276px`.
const DESCRIPTION: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 14.0, 24.0);
const DESCRIPTION_WIDTH: f32 = 276.0;
const NUDGE: f32 = 3.0;
/// The content width: the dialog's 550px less `padding: 20px 32px`.
const CONTENT_WIDTH: f32 = 550.0 - 64.0;
/// The hero behind the header: 152px tall.
const HERO_HEIGHT: f32 = 152.0;
/// The preview: 234×104, flush with the card's right edge.
const PREVIEW: [f32; 2] = [234.0, 104.0];
/// The sticky footer: its rule, `padding: 16px 32px 20px` around the buttons, and the 7px under.
const FOOTER_HEIGHT: f32 = 72.0;
/// A sender row: 32px, 4px apart.
const SENDER_ROW: f32 = 32.0;

/// The hero and sender rows: `#f2f4fa`, `#dee3f2` in Carbon.
fn highlight(ctx: &egui::Context) -> Color32 {
    match exo_ui::app_theme(ctx) {
        exo_ui::Theme::Snow => Color32::from_rgb(0xf2, 0xf4, 0xfa),
        exo_ui::Theme::Carbon => Color32::from_rgb(0xde, 0xe3, 0xf2),
    }
}

/// Which kind of template it is, for what its screen shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// `splitTemplate_importantOther`: turns Important and Other on or off.
    ImportantOther,
    /// `splitTemplate_inbox`: the one Inbox, which can switch to Important and Other.
    Inbox,
    /// A split's template.
    Split,
}

/// What the footer's buttons do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateAction {
    /// The primary: `Turn on Split Inbox` or `Save`.
    Primary,
    /// `Turn off Split Inbox`, `Turn off/on Important • Other`, `Enable Important • Other`.
    Toggle,
    /// `Delete Split Inbox`.
    Delete,
    /// `Reset`: the template's senders again.
    Reset,
}

/// A footer button: its label, look, action and whether it can be pressed.
type Button = (&'static str, ActionKind, TemplateAction, bool);

/// What the user did on the screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemplateEvent {
    /// A footer button.
    Action(TemplateAction),
    /// The `+` beside the definition, or Enter in it with text.
    Add,
    /// A sender picked in VIP's contact picker.
    Pick(String),
    /// A key in VIP's contact picker.
    PickerKey(PickerKey),
    /// A sender row's trash.
    Remove(usize),
    /// The first checkbox.
    ToggleLeave,
    /// The second checkbox.
    ToggleHide,
}

/// The screen's state.
#[derive(Debug)]
pub(super) struct TemplateForm {
    /// The template, by id.
    pub template: String,
    pub kind: Kind,
    /// The split the template set up, by list id, as the screen opened.
    pub split: Option<String>,
    /// The split (or Important and Other) was on as the screen opened.
    pub was_enabled: bool,
    /// The senders, as edited, and as they opened.
    pub senders: Vec<String>,
    initial_senders: Vec<String>,
    /// The definition field's text and its error.
    pub input: String,
    pub error: Option<&'static str>,
    pub leave: bool,
    initial_leave: bool,
    pub hide: bool,
    initial_hide: bool,
    /// The definition field wants the keyboard.
    pub focus: bool,
    /// VIP's definition: a contact picker (`ContactPicker`) rather than the field.
    pub picker: ContactPicker,
    /// The sender row with the keyboard (`tabIndex: 0` on an editable list's rows).
    row_focus: Option<usize>,
    scroll: f32,
}

/// A built-in split's senders from its definition.
pub(crate) fn query_items(kind: Option<&str>, query: &str) -> Vec<String> {
    if query.is_empty() {
        return Vec::new();
    }
    let senders = match kind {
        Some("team") => {
            let inner = query.strip_suffix(" to:me").unwrap_or(query);
            inner
                .strip_prefix('(')
                .and_then(|inner| inner.strip_suffix(')'))
                .unwrap_or(inner)
                .to_owned()
        }
        None | Some("vip" | "news") => query.to_owned(),
        Some(_) => return Vec::new(),
    };
    senders
        .replace("from:", "")
        .split(" OR ")
        .map(str::to_owned)
        .collect()
}

fn unique(senders: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for sender in senders {
        if !out.contains(&sender) {
            out.push(sender);
        }
    }
    out
}

impl TemplateForm {
    /// The screen for `template` in an account with `settings`.
    pub(super) fn new(template: &Template, settings: &Settings) -> Self {
        let kind = match template.id.as_str() {
            "splitTemplate_importantOther" => Kind::ImportantOther,
            "splitTemplate_inbox" => Kind::Inbox,
            _ => Kind::Split,
        };
        let split = template.split_in(settings.splits());
        let was_enabled = match kind {
            Kind::ImportantOther => !settings.unified_inbox,
            _ => split.is_some_and(|split| !split.is_disabled),
        };
        let senders = template.properties.senders.as_ref();
        let editable = senders.is_some_and(|senders| senders.is_editable);
        let template_senders = senders
            .map(|senders| senders.value.clone())
            .unwrap_or_default();
        let items = split
            .map(|split| query_items(template.kind.as_deref(), split.query()))
            .unwrap_or_default();
        let documents = template.id == "splitTemplate_documents";
        let initial_senders = if split.is_some() && documents && {
            let (mut a, mut b) = (items.clone(), template_senders.clone());
            a.sort();
            b.sort();
            a != b
        } {
            unique(items)
        } else if editable {
            unique(items.into_iter().chain(template_senders))
        } else {
            unique(template_senders)
        };
        let leave = split
            .map(|split| split.leave_threads_in_important_other)
            .or(template.properties.show_in_important_other)
            .unwrap_or(false);
        let hide = match kind {
            Kind::ImportantOther => settings.hides_other_when_empty(),
            _ => split.is_none_or(SplitInbox::hides_when_empty),
        };
        Self {
            template: template.id.clone(),
            kind,
            split: split.map(|split| split.id.clone()),
            was_enabled,
            senders: initial_senders.clone(),
            initial_senders,
            input: String::new(),
            error: None,
            leave,
            initial_leave: leave,
            hide,
            initial_hide: hide,
            focus: editable,
            picker: ContactPicker::default(),
            row_focus: None,
            scroll: 0.0,
        }
    }

    /// Whether there's anything to save.
    pub(super) fn has_changes(&self, template: &Template) -> bool {
        let editable = template
            .properties
            .senders
            .as_ref()
            .is_some_and(|senders| senders.is_editable);
        let same = self.senders.len() == self.initial_senders.len()
            && self
                .senders
                .iter()
                .all(|sender| self.initial_senders.contains(sender));
        !self.input.is_empty()
            || self.leave != self.initial_leave
            || (self.kind != Kind::ImportantOther && self.hide != self.initial_hide)
            || (editable && !same)
    }

    /// Adds the definition field's sender, or says why it can't. Returns whether it did.
    pub(super) fn add(&mut self, template: &Template) -> bool {
        let sender = self.input.trim().to_owned();
        if sender.is_empty() {
            return false;
        }
        let domains_only = template
            .properties
            .senders
            .as_ref()
            .is_some_and(|senders| senders.domains_only);
        let valid = is_valid_domain(&sender)
            || (!domains_only && crate::compose::recipients::is_valid(&sender));
        if !valid {
            self.error = Some(if domains_only {
                "Please enter a valid domain."
            } else {
                "Please enter a valid email address or domain."
            });
            return false;
        }
        if !self.senders.contains(&sender) {
            self.senders.insert(0, sender);
        }
        self.input.clear();
        true
    }

    /// Picks `sender` from the suggestions: it goes first, unless it's there already.
    pub(super) fn pick(&mut self, sender: String) {
        if !self.senders.contains(&sender) {
            self.senders.insert(0, sender);
        }
    }

    /// `Reset`: the template's senders again.
    pub(super) fn reset(&mut self, template: &Template) {
        self.senders = template
            .properties
            .senders
            .as_ref()
            .map(|senders| senders.value.clone())
            .unwrap_or_default();
        self.focus = true;
    }

    /// The primary button can't be pressed: VIP, News or Documents without senders.
    fn primary_disabled(&self, template: &Template) -> bool {
        self.senders.is_empty()
            && (matches!(template.kind.as_deref(), Some("vip" | "news"))
                || (template.id == "splitTemplate_documents" && self.error.is_some()))
    }

    /// The content's height under the header: the hero, the definition, the senders and the
    /// checkboxes, and `padding-bottom: 20px`.
    fn content_height(&self, ctx: &egui::Context, template: &Template, settings: &Settings) -> f32 {
        let mut height = 8.0 + PREVIEW[1];
        let editable = template
            .properties
            .senders
            .as_ref()
            .is_some_and(|senders| senders.is_editable);
        if self.kind != Kind::Inbox {
            height += 44.0 + 16.0;
        }
        if editable {
            height += 20.0;
        }
        if let Some(description) = query_description(template) {
            height += if self.kind == Kind::Inbox { 16.0 } else { 0.0 }
                + 20.0 * BODY.wrap_pretty(ctx, description, CONTENT_WIDTH).len() as f32;
        }
        if self.error.is_some() {
            height += 16.0 + 20.0;
        }
        let senders = self.shown_senders(template).len();
        if senders > 0 {
            height += self.list_margin(template)
                + SENDER_ROW * senders as f32
                + 4.0 * (senders - 1) as f32;
        }
        let boxes = self.checkboxes(template, settings).len();
        if boxes > 0 {
            height += 16.0 + 24.0 * boxes as f32;
        }
        // The content's `padding-bottom: 20px`, none for Inbox.
        height + if self.kind == Kind::Inbox { 0.0 } else { 20.0 }
    }

    /// The sender list's `margin-top: 12px`, which collapses into the subheader's 16px when
    /// nothing comes between them.
    fn list_margin(&self, template: &Template) -> f32 {
        let editable = template
            .properties
            .senders
            .as_ref()
            .is_some_and(|senders| senders.is_editable);
        if editable || query_description(template).is_some() || self.error.is_some() {
            12.0
        } else {
            0.0
        }
    }

    /// The sticky footer's height and the gap above it: 20px (the dialog's padding) over a
    /// footer with its rule; Inbox's has no rule and sits 3px under its content (measured).
    fn footer(&self) -> (f32, f32) {
        if self.kind == Kind::Inbox {
            (FOOTER_HEIGHT - 1.0, 3.0)
        } else {
            (FOOTER_HEIGHT, 20.0)
        }
    }

    /// The dialog body's height: the content and the sticky footer, less the dialog's own 20px
    /// bottom padding, which the footer replaces.
    pub(super) fn body_height(
        &self,
        ctx: &egui::Context,
        template: &Template,
        settings: &Settings,
    ) -> f32 {
        let (footer, gap) = self.footer();
        self.content_height(ctx, template, settings) + gap + footer - 20.0
    }

    /// The senders the list shows: the edited ones, or the template's.
    fn shown_senders<'a>(&'a self, template: &'a Template) -> &'a [String] {
        let editable = template
            .properties
            .senders
            .as_ref()
            .is_some_and(|senders| senders.is_editable);
        if editable {
            &self.senders
        } else {
            template
                .properties
                .senders
                .as_ref()
                .map_or(&[], |senders| senders.value.as_slice())
        }
    }

    /// The checkboxes, label and state: leave in Important and Other (not for Calendar,
    /// Reminders, Inbox or Important • Other), and hide when empty (not for Inbox).
    fn checkboxes(
        &self,
        template: &Template,
        settings: &Settings,
    ) -> Vec<(&'static str, bool, TemplateEvent)> {
        let mut boxes = Vec::new();
        let leave = template.kind.as_deref() != Some("calendar")
            && template.id != "splitTemplate_reminders"
            && self.kind == Kind::Split;
        if leave {
            let label = if settings.unified_inbox {
                "Also show conversations that match this Split Inbox in Inbox"
            } else {
                "Also show conversations that match this Split Inbox in Important or Other"
            };
            boxes.push((label, self.leave, TemplateEvent::ToggleLeave));
        }
        match self.kind {
            Kind::Inbox => {}
            Kind::ImportantOther => boxes.push((
                "Hide Other when empty",
                self.hide,
                TemplateEvent::ToggleHide,
            )),
            Kind::Split => boxes.push((
                "Hide this Split Inbox when empty",
                self.hide,
                TemplateEvent::ToggleHide,
            )),
        }
        boxes
    }

    /// The footer's buttons (label, kind, action, enabled): the tertiary one at the left, and
    /// the rest at the right, left to right.
    fn buttons(&self, template: &Template) -> (Option<Button>, Vec<Button>) {
        let tertiary = |label| Some((label, ActionKind::Plain, TemplateAction::Toggle, true));
        match self.kind {
            Kind::Inbox => (tertiary("Enable Important • Other"), Vec::new()),
            Kind::ImportantOther if self.was_enabled => {
                (tertiary("Turn off Important • Other"), Vec::new())
            }
            Kind::ImportantOther => (
                None,
                vec![(
                    "Turn on Important • Other",
                    ActionKind::Primary,
                    TemplateAction::Toggle,
                    true,
                )],
            ),
            Kind::Split => {
                let left = if self.was_enabled {
                    tertiary("Turn off Split Inbox")
                } else {
                    self.split.as_ref().map(|_| {
                        (
                            "Delete Split Inbox",
                            ActionKind::Plain,
                            TemplateAction::Delete,
                            true,
                        )
                    })
                };
                let mut right = Vec::new();
                if self.was_enabled
                    && (template.id == "splitTemplate_documents"
                        || template.kind.as_deref() == Some("team"))
                {
                    right.push(("Reset", ActionKind::Plain, TemplateAction::Reset, true));
                }
                let primary = if self.was_enabled {
                    "Save"
                } else {
                    "Turn on Split Inbox"
                };
                right.push((
                    primary,
                    ActionKind::Primary,
                    TemplateAction::Primary,
                    !self.primary_disabled(template),
                ));
                (left, right)
            }
        }
    }

    /// What Enter does: adds what's typed in the definition, or else presses the primary button.
    /// In VIP's picker it does only what the picker does with it.
    pub(super) fn enter(&self, template: &Template) -> Option<TemplateEvent> {
        let editable = template
            .properties
            .senders
            .as_ref()
            .is_some_and(|senders| senders.is_editable);
        if let Some(row) = self.row_focus {
            Some(TemplateEvent::Remove(row))
        } else if is_vip(template) && self.picker.is_focused() {
            None
        } else if editable && !self.input.trim().is_empty() {
            Some(TemplateEvent::Add)
        } else if editable && self.senders.is_empty() {
            None
        } else {
            let (left, right) = self.buttons(template);
            right
                .last()
                .or(left.as_ref())
                .filter(|(.., enabled)| *enabled)
                .map(|(_, _, action, _)| TemplateEvent::Action(*action))
        }
    }

    /// The sender row with the keyboard: Enter, Backspace and Delete remove it, and the
    /// definition takes the keyboard back.
    pub(super) fn focused_row(&self) -> Option<usize> {
        self.row_focus
    }

    /// Draws the screen's body (`body`, under the dialog's header, over `card`) and returns what
    /// the user did.
    pub(super) fn show(
        &mut self,
        ui: &mut Ui,
        card: Rect,
        body: Rect,
        template: &Template,
        settings: &Settings,
        book: Option<&contacts::Index>,
    ) -> Option<TemplateEvent> {
        let ctx = ui.ctx().clone();
        let tokens = palette(&ctx);
        let (left, right) = (body.left(), body.right());
        let mut event = None;
        let (footer, gap) = self.footer();
        let footer_top = card.bottom() - footer;

        // The content scrolls between the header and the footer.
        let viewport = Rect::from_min_max(
            pos2(card.left(), body.top()),
            pos2(card.right(), footer_top),
        );
        let natural = self.content_height(&ctx, template, settings);
        let room = (footer_top - gap - body.top()).max(0.0);
        if ui.rect_contains_pointer(viewport) {
            self.scroll -= ui.input(|input| input.smooth_scroll_delta.y);
        }
        self.scroll = self.scroll.clamp(0.0, (natural - room).max(0.0));

        // The hero, behind the dialog's header from the card's top.
        let hero = Rect::from_min_size(
            pos2(card.left(), card.top() - self.scroll),
            vec2(card.width(), HERO_HEIGHT),
        );
        let mut hero_painter = ui.painter().clone();
        hero_painter.set_clip_rect(card.intersect(ui.clip_rect()));
        // At `opacity: 50%`, rounded as the dialog is (`--dialog-border-radius`, 4px).
        hero_painter.rect_filled(
            hero,
            CornerRadius {
                nw: 4,
                ne: 4,
                ..CornerRadius::ZERO
            },
            highlight(&ctx).gamma_multiply(0.5),
        );

        let mut content = ui.new_child(egui::UiBuilder::new().max_rect(viewport));
        content.set_clip_rect(viewport.intersect(ui.clip_rect()));
        let painter = content.painter().clone();
        let mut y = body.top() + 8.0 - self.scroll;
        for (index, line) in DESCRIPTION
            .wrap_pretty(&ctx, &template.description, DESCRIPTION_WIDTH)
            .iter()
            .enumerate()
        {
            DESCRIPTION.paint_line(
                &painter,
                pos2(left, y + 20.0 + 24.0 * index as f32),
                line,
                tokens.ink(0.85, 0.85),
                None,
            );
        }
        let preview = Rect::from_min_size(pos2(card.right() - PREVIEW[0], y), PREVIEW.into());
        split_preview::paint(&content, preview, template, Place::Settings);
        y += PREVIEW[1];

        if self.kind != Kind::Inbox {
            // The subheader: `padding-top: 24px`, 12px/20px 600 at `rgba(0, 0, 0, .85)`,
            // `margin-bottom: 16px`.
            BODY_BOLD.paint_line(
                &painter,
                pos2(left, y + 24.0 + NUDGE),
                "Definition",
                tokens.ink(0.85, 0.85),
                None,
            );
            y += 44.0 + 16.0;
        }
        let senders = template.properties.senders.as_ref();
        if let Some(senders) = senders.filter(|senders| senders.is_editable && is_vip(template)) {
            // `ContactPicker` with the template's placeholder (or its own).
            let placeholder = senders
                .placeholder
                .as_deref()
                .unwrap_or("Enter name, email, or domain");
            let line = Rect::from_min_size(pos2(left, y + NUDGE), vec2(CONTENT_WIDTH, 20.0));
            if std::mem::take(&mut self.focus) {
                self.picker.request_focus();
            }
            if let Some(sender) = self.picker.show(
                &mut content,
                exo_ui::id(&ctx, "split-template-picker"),
                DialogInput::new(BODY, placeholder),
                line,
                book,
            ) {
                event = Some(TemplateEvent::Pick(sender));
            }
            y += 20.0;
        } else if let Some(senders) = senders.filter(|senders| senders.is_editable) {
            let placeholder = senders.placeholder.as_deref().unwrap_or(
                "Enter a search query that every conversation in this Split Inbox must match",
            );
            let line = Rect::from_min_size(pos2(left, y + NUDGE), vec2(CONTENT_WIDTH, 20.0));
            let id = exo_ui::id(&ctx, "split-template-definition");
            let before = self.input.clone();
            let response = DialogInput::new(BODY, placeholder)
                .error(self.error.is_some())
                .show(&mut content, id, &mut self.input, line);
            if std::mem::take(&mut self.focus) {
                response.request_focus();
            }
            if self.input != before {
                self.error = None;
            }
            if !self.input.trim().is_empty() {
                // The add button: a 16px `+` at the field's right.
                let add = Rect::from_min_size(pos2(right - 16.0, y + 2.0), vec2(16.0, 16.0));
                exo_ui::paint_icon_on(
                    &painter,
                    "legacy/DEPRECATED_plusCircle",
                    exo_ui::pixel_snap(add),
                    tokens.background_brand_default,
                );
                if content
                    .interact(
                        add,
                        exo_ui::id(content.ctx(), "split-template-add"),
                        Sense::click(),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    event = Some(TemplateEvent::Add);
                }
            }
            y += 20.0;
        }
        if let Some(description) = query_description(template) {
            if self.kind == Kind::Inbox {
                y += 16.0;
            }
            for line in BODY.wrap_pretty(&ctx, description, CONTENT_WIDTH) {
                BODY.paint_line(
                    &painter,
                    pos2(left, y + NUDGE),
                    &line,
                    tokens.text_base_default,
                    None,
                );
                y += 20.0;
            }
        }
        if let Some(error) = self.error {
            BODY.paint_line(
                &painter,
                pos2(left, y + 16.0 + NUDGE),
                error,
                tokens.text_critical_default,
                None,
            );
            y += 36.0;
        }
        let editable = senders.is_some_and(|senders| senders.is_editable);
        let shown = self.shown_senders(template).to_vec();
        if !shown.is_empty() {
            y += self.list_margin(template);
            let from = SMALL.width(&ctx, "From");
            self.row_focus = None;
            for (index, sender) in shown.iter().enumerate() {
                let row = Rect::from_min_size(
                    pos2(left - 8.0, y),
                    vec2(CONTENT_WIDTH + 16.0, SENDER_ROW),
                );
                painter.rect_filled(row, CornerRadius::same(4), highlight(&ctx));
                // An editable list's rows take the keyboard, in order after the field: the
                // focused one is ringed (`box-shadow: 0 0 0 2px #bec1e4`) and shows its trash.
                let focused = editable
                    && content
                        .interact(
                            row,
                            exo_ui::id(content.ctx(), ("split-template-row", index)),
                            Sense::click(),
                        )
                        .has_focus();
                if focused {
                    painter.rect_stroke(
                        row,
                        CornerRadius::same(4),
                        egui::Stroke::new(2.0, Color32::from_rgb(0xbe, 0xc1, 0xe4)),
                        egui::StrokeKind::Outside,
                    );
                    self.row_focus = Some(index);
                }
                // `From`: `rgba(0, 0, 0, .55)`.
                SMALL.paint_line(
                    &painter,
                    pos2(left, y + 8.0 + 1.0),
                    "From",
                    tokens.ink(0.55, 0.55),
                    None,
                );
                BODY.paint_line(
                    &painter,
                    pos2(left + from + 16.0, y + 6.0 + 1.0),
                    sender,
                    tokens.text_base_default,
                    Some(CONTENT_WIDTH - from - 16.0 - 30.0),
                );
                if editable && (focused || content.rect_contains_pointer(row)) {
                    // The trash, `padding: 8px` at the row's end, while the row is hovered.
                    let trash = Rect::from_min_size(
                        pos2(row.right() - 8.0 - 14.0, y + 9.0),
                        vec2(14.0, 14.0),
                    );
                    let response = content
                        .interact(
                            trash.expand(8.0),
                            exo_ui::id(content.ctx(), ("split-template-remove", index)),
                            Sense::click(),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    // The trash: `rgba(0, 0, 0, .4)`, `.5` hovered.
                    let color = if response.hovered() {
                        tokens.ink(0.5, 0.5)
                    } else {
                        tokens.ink(0.4, 0.4)
                    };
                    let name = "legacy/DEPRECATED_trash";
                    exo_ui::paint_icon_on(
                        &painter,
                        name,
                        exo_ui::icon_rect(ui.ctx(), name, trash),
                        color,
                    );
                    if response.clicked() {
                        event = Some(TemplateEvent::Remove(index));
                    }
                }
                y += SENDER_ROW + 4.0;
            }
            y -= 4.0;
        }
        let boxes = self.checkboxes(template, settings);
        if !boxes.is_empty() {
            y += 16.0;
            for (label, checked, toggle) in boxes {
                y += 8.0;
                let mark = pos2(left, y + 1.0);
                let label_x = left + CHECKBOX_SIZE - 1.0 + 12.0;
                let hit =
                    Rect::from_min_max(mark, pos2(label_x + SMALL.width(&ctx, label), y + 17.0));
                if content
                    .interact(
                        hit,
                        exo_ui::id(content.ctx(), ("split-template-check", label)),
                        Sense::click(),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    event = Some(toggle);
                }
                paint_checkbox(&painter, mark, checked, tokens);
                // The checkbox's label: `rgba(0, 0, 0, .55)`.
                SMALL.paint_line(
                    &painter,
                    pos2(label_x, y + 1.0),
                    label,
                    tokens.ink(0.55, 0.55),
                    None,
                );
                y += 16.0;
            }
        }

        // The sticky footer: a rule (none for Inbox), the tertiary button at the left, the rest
        // at the right.
        if self.kind != Kind::Inbox {
            let painter = ui.painter();
            painter.rect_filled(
                Rect::from_min_size(pos2(card.left(), footer_top), vec2(card.width(), 1.0)),
                0.0,
                tokens.border_base_default,
            );
        }
        // The buttons: under the rule, `padding-top: 16px`, `top: 1px`.
        let top = footer_top + if self.kind == Kind::Inbox { 0.0 } else { 1.0 } + 16.0 + 1.0;
        let (tertiary, buttons) = self.buttons(template);
        let mut placed: Vec<(Button, Rect)> = Vec::new();
        if let Some(button @ (label, kind, ..)) = tertiary {
            let width = DialogAction::new(label, kind).width(&ctx);
            placed.push((
                button,
                Rect::from_min_size(pos2(left - 12.0, top), vec2(width, DIALOG_ACTION_HEIGHT)),
            ));
        }
        let mut right_edge = right + 13.0;
        for button @ (label, kind, ..) in buttons.into_iter().rev() {
            let width = DialogAction::new(label, kind).width(&ctx);
            let rect = Rect::from_min_size(
                pos2(right_edge - width, top),
                vec2(width, DIALOG_ACTION_HEIGHT),
            );
            right_edge = rect.left() - 8.0;
            placed.push((button, rect));
        }
        for ((label, kind, action, enabled), rect) in placed {
            // The primary button disabled: `rgba(0, 0, 0, .035)`.
            if DialogAction::new(label, kind)
                .enabled(enabled)
                .disabled_fill(tokens.ink(0.035, 0.035))
                .show(
                    ui,
                    exo_ui::id(ui.ctx(), ("split-template-action", label)),
                    rect,
                )
                .clicked()
            {
                event = Some(TemplateEvent::Action(action));
            }
        }
        event
    }
}

/// VIP's template (`"vip" === type`), whose definition is a contact picker.
fn is_vip(template: &Template) -> bool {
    template.kind.as_deref() == Some("vip")
}

/// What the template's query does, shown under the definition.
fn query_description(template: &Template) -> Option<&str> {
    template
        .properties
        .query
        .as_ref()
        .and_then(|query| query.description.as_deref())
        .filter(|description| !description.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_splits_give_back_their_senders() {
        assert_eq!(
            query_items(Some("team"), "(from:a.co OR from:b.co) to:me"),
            ["a.co", "b.co"]
        );
        assert_eq!(query_items(Some("team"), "from:a.co to:me"), ["a.co"]);
        assert_eq!(
            query_items(Some("vip"), "from:ana@a.co OR from:b.co"),
            ["ana@a.co", "b.co"]
        );
        assert_eq!(
            query_items(None, "from:bot@tool.example"),
            ["bot@tool.example"]
        );
        assert!(query_items(Some("calendar"), "filename:ics").is_empty());
        assert!(query_items(Some("vip"), "").is_empty());
    }
}
