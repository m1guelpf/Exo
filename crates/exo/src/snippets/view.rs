//! A snippet opened from the folder: the thread pane titled `Edit Private Snippet`, the
//! snippet's name over a card that holds its recipients, subject, body (rendered as a message's
//! is, by a body renderer of its own) and files (a message's cards), and a footer with `Edit` and
//! Discard.

use egui::{Color32, Rect, Sense, Ui, pos2, vec2};
use exo_commands::Region;
use exo_store::{Contact, MessageDetail, Snippet};
use exo_ui::{Family, Font, IconButton, KeyTooltip, ShortcutTooltip, TextStyle};

use super::Action;
use crate::body::{Bodies, Look};

/// The snippet's name: 14px/20px, weight 600, `top: 3px`.
const NAME: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 14.0, 20.0);
/// The subject: 14px/20px bold, `top: 2px`.
const SUBJECT: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 14.0, 20.0);
/// The footer's `Edit`: 12px/16px bold, `top: 1px`.
const SUBMIT: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 16.0);
/// The room kept above the card for the name, as the editor keeps it for its name field.
const NAME_ROOM: f32 = 36.0;
/// The name's line, above the card's top (`margin: -40px 0 20px`, `top: -1px`, the name's 3px).
const NAME_ABOVE: f32 = 26.0;
/// The view's `padding-top: 16px`; the recipients' header starts 4px higher.
const SPACING_TOP: f32 = 16.0;
const HEADER_TOP: f32 = 12.0;
/// The subject's `padding-top: 12px` under recipients, and its `margin-bottom: 8px`, which the
/// body's `margin-top: 20px` swallows.
const SUBJECT_SPACING: f32 = 12.0;
const BODY_MARGIN_TOP: f32 = 20.0;
/// What the body is taken to be until laid out: one line.
const BODY_PLACEHOLDER: f32 = 24.0;
/// The footer: 16px under the body, a 1px rule, `Edit` 20px down, 36px tall, then the card's
/// 20px.
const FOOTER_MARGIN: f32 = 16.0;
const FOOTER_HEIGHT: f32 = 36.0;
const FOOTER_BOTTOM: f32 = 20.0;
const SUBMIT_DOWN: f32 = 20.0;
/// The Discard icon, centred on a 16px line 2.5px above `Edit`'s.
const ICON_LINE_UP: f32 = 2.5;
const DISCARD: &str = "legacy/DEPRECATED_discard";

/// The view's own body renderer, made when a snippet opens and dropped when the view closes,
/// and what its file cards show.
#[derive(Default)]
pub(crate) struct Preview {
    bodies: Option<Bodies>,
    files: crate::thread::attachments::Files,
}

impl Preview {
    /// Nothing is being laid out or painted.
    pub(crate) fn is_idle(&self) -> bool {
        self.bodies.as_ref().is_none_or(Bodies::is_idle)
    }
}

/// The renderer's id for snippet `id`'s body.
fn body_id(id: &str) -> String {
    format!("snippet:{id}")
}

/// `snippet` as a message, for the body renderer and the recipients' header.
fn message(snippet: &Snippet) -> MessageDetail {
    let contact = |address: &exo_store::Address| Contact {
        name: address.name.clone(),
        email: address.email.clone(),
        is_me: false,
    };
    MessageDetail {
        id: body_id(&snippet.id),
        labels: Vec::new(),
        date: jiff::Timestamp::UNIX_EPOCH,
        from: None,
        to: snippet.to.iter().map(contact).collect(),
        cc: snippet.cc.iter().map(contact).collect(),
        bcc: snippet.bcc.iter().map(contact).collect(),
        reply_to: Vec::new(),
        subject: snippet.subject.clone(),
        snippet: String::new(),
        rfc822_id: None,
        in_reply_to: None,
        references: Vec::new(),
        list_id: None,
        list_unsubscribe: None,
        list_unsubscribe_post: None,
        headers: Vec::new(),
        body: exo_store::Body {
            html: Some(format!(
                "<html><head></head><body>{}</body></html>",
                snippet.body
            )),
            text: None,
        },
        // Its files that aren't inline, as parts of it, by `uuid`.
        attachments: snippet
            .attachments
            .iter()
            .filter(|file| file.content_id.is_none())
            .map(|file| exo_store::Attachment {
                message_id: body_id(&snippet.id),
                attachment_id: None,
                part_id: file.uuid.clone(),
                filename: file.name.clone(),
                mime_type: file.mime_type.clone(),
                size: u32::try_from(file.size).unwrap_or(u32::MAX),
                content_id: None,
                inline: false,
            })
            .collect(),
        failure: None,
        scheduled: None,
        sending: false,
    }
}

/// Where the card's parts sit, from the content's top (the card's plus `message::PADDING_TOP`).
struct Parts {
    /// The recipients' rows (label, people, lines), from the top.
    rows: Vec<(&'static str, Vec<Contact>, f32)>,
    rows_top: f32,
    /// The subject's line (under its padding), from the top.
    subject_top: Option<f32>,
    body_top: f32,
    /// The file cards' list, under the body.
    files_top: f32,
    footer_top: f32,
    /// The content's height, to the card's bottom less its own 4px.
    height: f32,
}

/// The card's layout for `message` with a body `body` tall, `width` wide.
fn parts(ui: &Ui, message: &MessageDetail, body: f32, width: f32) -> Parts {
    // The content starts the card's own padding down; the view's spacing replaces it.
    let padding = crate::thread::CARD_PADDING_TOP;
    let painter = ui.painter();
    let ink = Color32::BLACK;
    let rows: Vec<(&'static str, Vec<Contact>, f32)> = [
        ("To", &message.to),
        ("Cc", &message.cc),
        ("Bcc", &message.bcc),
    ]
    .into_iter()
    .filter(|(_, people)| !people.is_empty())
    .map(|(label, people)| {
        let galley = painter.layout_job(crate::thread::header_job(people, width, ink, ink));
        (label, people.clone(), galley.rows.len().max(1) as f32)
    })
    .collect();
    let has_rows = !rows.is_empty();
    let rows_top = HEADER_TOP - padding;
    let mut top = if has_rows {
        rows_top
            + rows
                .iter()
                .map(|(_, _, lines)| lines * crate::thread::HEADER_ROW)
                .sum::<f32>()
    } else {
        SPACING_TOP - padding
    };
    let subject_top = (!message.subject.is_empty()).then(|| {
        let box_top = top + if has_rows { SUBJECT_SPACING } else { 0.0 };
        top = box_top + SUBJECT.line_height;
        box_top
    });
    // No margin over a body with nothing above it.
    let body_top = if has_rows || subject_top.is_some() {
        top + BODY_MARGIN_TOP
    } else {
        top
    };
    let files_top = body_top + body;
    let files = crate::thread::attachments::height(
        crate::thread::attachments::listed(message).count(),
        width,
    );
    let footer_top = files_top + files + FOOTER_MARGIN;
    let height = footer_top + FOOTER_HEIGHT + FOOTER_BOTTOM - crate::thread::CARD_PADDING_BOTTOM;
    Parts {
        rows,
        rows_top,
        subject_top,
        body_top,
        files_top,
        footer_top,
        height,
    }
}

impl crate::Mail {
    /// Enter or a click on a row: the snippet, read-only.
    pub(crate) fn open_snippet_view(&mut self, id: &str) {
        self.leave_pane();
        self.snippets.folder.view(id);
    }

    /// Back from the view to the folder. Its renderer goes with it: nothing would take what it's
    /// still working on.
    pub(crate) fn close_snippet_view(&mut self) {
        self.snippets.folder.close_view();
        self.snippets.preview = Preview::default();
    }

    /// The snippet being viewed, while it's still kept.
    fn viewed_snippet(&self) -> Option<Snippet> {
        let id = self.snippets.folder.viewing()?;
        self.account
            .settings()
            .snippets
            .iter()
            .find(|snippet| snippet.id == id)
            .cloned()
    }

    /// A card in the view: snippet file `uuid` opens (in the preview, or as a download).
    pub(crate) fn open_snippet_file(&mut self, uuid: &str, preview: bool) {
        if let Some(file) = self.viewed_snippet().and_then(|snippet| {
            snippet
                .attachments
                .into_iter()
                .find(|file| file.uuid == uuid)
        }) {
            self.open_draft_attachment(file, preview);
        }
    }

    /// The view's `Download all`: the snippet's files in one zip named for its subject.
    pub(crate) fn download_snippet_files(&mut self) {
        let Some(snippet) = self.viewed_snippet() else {
            return;
        };
        let folder = self.account.draft_files().to_owned();
        self.attachment_files.download_snippet_files(
            &self.account.loader(),
            &body_id(&snippet.id),
            &snippet.subject,
            &snippet.attachments,
            &folder,
            self.now.datetime(),
        );
    }

    /// Draws the snippet being viewed in `rect` (the content pane).
    pub(crate) fn show_snippet_view(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        rem: f32,
        window_width: f32,
        now: f64,
    ) {
        let Some(snippet) = self.viewed_snippet() else {
            // Discarded meanwhile.
            self.close_snippet_view();
            return;
        };
        let message = message(&snippet);
        let ctx = ui.ctx().clone();
        let popped_out = self.compose.popped().is_some();
        let width = crate::thread::content_width(rect, rem, window_width, popped_out);
        let scale = ctx.pixels_per_point();
        let theme = exo_ui::theme(&ctx);
        let left = crate::thread::content_left(rect, rem, window_width, popped_out);
        let look = Look {
            width,
            scale,
            theme,
            rem,
            // A snippet has no quote to fold.
            show_quoted: true,
            hide_original: false,
            phase: left - left.floor(),
            background: theme.palette().background_elevated_default.to_array(),
        };
        // The body is shown cleaned: whole, no dates, no color tags, so in Snow its text is the
        // pane's black rather than a message's 90%.
        let bodies = self
            .snippets
            .preview
            .bodies
            .get_or_insert_with(|| Bodies::new(&ctx).cleaned_only().unclassified());
        bodies.set_thread(&[&message], self.account.email(), &self.account.loader());
        bodies.poll(&ctx);
        let body = bodies.height(&message.id, look).unwrap_or(BODY_PLACEHOLDER);
        let parts = parts(ui, &message, body, width);
        let pane = crate::thread::PaneCard {
            title: "Edit Private Snippet",
            height: parts.height,
            above: NAME_ROOM,
            pop_out_hint: false,
            read_only: true,
        };
        let focused = self.commands.region() == Region::ReadOnlySnippet;
        let name = crate::compose::snippets::name(&snippet).to_owned();
        let mut asked = Vec::new();
        self.attachment_files
            .update_files(&mut self.snippets.preview.files);
        let preview = &mut self.snippets.preview;
        let (actions, ()) = crate::thread::card_pane(
            ui,
            rect,
            rem,
            window_width,
            popped_out,
            &pane,
            |ui, content, column, card| {
                let palette = exo_ui::palette(ui.ctx());
                let painter = ui.painter().clone();
                let (left, top) = (content.left(), content.top());
                // The name over the card, `rgba(0,0,0,.8)` (white in Carbon).
                NAME.paint_line(
                    &painter,
                    pos2(left, card.top() - NAME_ABOVE),
                    if snippet.name.is_empty() { "" } else { &name },
                    palette.ink(0.8, 0.8),
                    Some(content.width()),
                );
                // The recipients' header: labels and addresses `rgba(0,0,0,.5)`, names `.8`.
                let (label, contact) = (palette.ink(0.5, 0.5), palette.ink(0.8, 0.8));
                let mut row_top = top + parts.rows_top;
                for (row, people, lines) in &parts.rows {
                    crate::thread::HEADER_TEXT.paint_line(
                        &painter,
                        pos2(left, row_top),
                        row,
                        label,
                        None,
                    );
                    let galley = painter.layout_job(crate::thread::header_job(
                        people,
                        content.width(),
                        label,
                        contact,
                    ));
                    exo_ui::paint_text(
                        &painter,
                        pos2(left, row_top),
                        galley,
                        &crate::thread::HEADER_TEXT,
                    );
                    row_top += lines * crate::thread::HEADER_ROW;
                }
                if let Some(subject_top) = parts.subject_top {
                    SUBJECT.paint_line(
                        &painter,
                        pos2(left, top + subject_top + 2.0),
                        &snippet.subject,
                        palette.ink(0.8, 0.8),
                        Some(content.width()),
                    );
                }
                if let Some(bodies) = &mut preview.bodies {
                    bodies.paint(
                        &painter,
                        &message.id,
                        pos2(left, top + parts.body_top),
                        ui.clip_rect(),
                        1.0,
                    );
                }
                match crate::thread::attachments::show(
                    ui,
                    content,
                    top + parts.files_top,
                    &message,
                    &preview.files,
                ) {
                    Some(crate::thread::attachments::Click::Open {
                        attachment,
                        preview,
                    }) => asked.push(Action::OpenFile {
                        uuid: attachment.part_id,
                        preview,
                    }),
                    Some(crate::thread::attachments::Click::DownloadAll { .. }) => {
                        asked.push(Action::DownloadFiles)
                    }
                    // Dragging a file out takes a Gmail part (`attachment_files::drag`).
                    Some(crate::thread::attachments::Click::Drag { .. }) | None => {}
                }
                // The footer: the rule, `Edit` (underlined while the view has the keyboard), Discard.
                let footer = top + parts.footer_top;
                let rule = Rect::from_min_size(pos2(left, footer), vec2(content.width(), 1.0));
                let dark = exo_ui::theme(ui.ctx()) == exo_ui::Theme::Carbon;
                painter.rect_filled(
                    exo_ui::pixel_snap(rule),
                    0.0,
                    if dark {
                        Color32::from_rgb(0x5f, 0x61, 0x65)
                    } else {
                        Color32::from_rgb(0xed, 0xed, 0xed)
                    },
                );
                let pane_top = footer + SUBMIT_DOWN;
                let edit_width = SUBMIT.paint_line(
                    &painter,
                    pos2(left, pane_top + 1.0),
                    "Edit",
                    palette.ink(0.8, 0.8),
                    None,
                );
                // `Edit` takes clicks 10px around its text.
                let edit = Rect::from_min_size(
                    pos2(left - 10.0, pane_top - 10.0),
                    vec2(edit_width + 20.0, FOOTER_HEIGHT),
                );
                if ui
                    .interact(edit, ui.id().with("snippet-view-edit"), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    asked.push(Action::Edit(Some(snippet.id.clone())));
                }
                if focused {
                    // Focused, `Edit` is underlined: a 2px border under its text.
                    painter.rect_filled(
                        exo_ui::pixel_snap(Rect::from_min_max(
                            pos2(left, pane_top + 17.0),
                            pos2(left + edit_width, pane_top + 19.0),
                        )),
                        0.0,
                        palette.border_base_active,
                    );
                }
                let size = exo_ui::icon_size(ui.ctx(), DISCARD).unwrap_or(vec2(11.0, 12.0));
                let line = pane_top - ICON_LINE_UP;
                let icon = Rect::from_min_size(
                    pos2(content.right() - size.x, line + (16.0 - size.y) / 2.0),
                    size,
                );
                let hit =
                    Rect::from_center_size(icon.center(), vec2(icon.width(), 16.0)).expand(8.0);
                let button = IconButton::new(DISCARD)
                    .colors(palette.icon_base_default, palette.icon_base_hover)
                    .show(
                        ui,
                        ui.id().with("snippet-view-discard"),
                        hit,
                        exo_ui::icon_rect(ui.ctx(), DISCARD, icon),
                    );
                let rows = [KeyTooltip {
                    text: "Discard",
                    keys: &["⌘", "shift", ","],
                }];
                ShortcutTooltip {
                    rows: &rows,
                    note: None,
                }
                .show_below(ui, &button, 9.0, false, column);
                if button.clicked() {
                    asked.push(Action::Discard);
                }
            },
        );
        for action in actions {
            match action {
                crate::thread::Action::Back => asked.push(Action::Back),
                crate::thread::Action::Navigate(delta) => asked.push(Action::Move(delta)),
                _ => {}
            }
        }
        for action in asked {
            self.snippets_run(action, now);
        }
    }
}
