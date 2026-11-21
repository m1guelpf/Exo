//! The page a conversation prints from: HTML for the open thread under the print rules
//! (`print.css`), each body in a shadow root of its own (`exo_mail::printable_body`), loading
//! nothing but what the print view's handler answers.

use std::fmt::Write as _;

use egui::Color32;

use super::{Conversation, Header, Message, Row, When};

/// The stylesheet for everything but the bodies.
const PRINT_CSS: &str = include_str!("print.css");

/// The scheme Exo's own resources load from: `exo-print://font/<name>.ttf`.
pub(super) const SCHEME: &str = "exo-print";
/// Where a body's relative URLs resolve: on the remote scheme, on a reserved domain that the
/// handler refuses (as `exo_mail`'s base does on screen).
pub(super) const BASE_URL: &str = "exo-https://exo.invalid/";

/// Only what the print view's handler answers loads: fonts from Exo, a body's remote content
/// through the guarded fetches, its inline attachments, and `data:` images. No scripts at all.
const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; img-src exo-https: exo-http: cid: data:; style-src 'unsafe-inline' exo-https: exo-http:; font-src exo-print: exo-https: exo-http: data:";

/// The legacy icons the print rules show, each in the color its `[fill*=red]` rule prints.
const PAPERCLIP: (&str, &str) = ("legacy/DEPRECATED_paperclip", "#000");
const COLLAPSE: (&str, &str) = ("legacy/DEPRECATED_collapse", "rgba(0,0,0,0.7)");
const PIXEL_BLOCKING: (&str, &str) = ("legacy/DEPRECATED_pixelBlocking", "rgba(0,0,0,0.45)");

/// The page for `conversation`.
pub(super) fn html(conversation: &Conversation) -> String {
    let icon = |(name, color): (&str, &str)| -> String {
        exo_ui::icon_svg(name)
            .map(|svg| svg.replace("\"#fff\"", &format!("\"{color}\"")))
            .unwrap_or_default()
    };
    let icons = Icons {
        paperclip: icon(PAPERCLIP),
        collapse: icon(COLLAPSE),
        pixel_blocking: icon(PIXEL_BLOCKING),
    };
    let page = conversation.page;
    let narrow = if page.narrow() {
        " class=\"isNarrow\""
    } else {
        ""
    };
    let mut html = format!(
        "<!DOCTYPE html><html{narrow} style=\"font-size: {rem}px; width: {width}px\"><head><meta charset=\"utf-8\">\
         <meta http-equiv=\"Content-Security-Policy\" content=\"{CONTENT_SECURITY_POLICY}\">\
         <meta http-equiv=\"x-dns-prefetch-control\" content=\"off\">\
         <title>{title}</title><style>{fonts}{PRINT_CSS}</style></head><body>",
        rem = page.rem(),
        // To the hundredth: a float's last digits mean nothing to a layout.
        width = (page.width() * 100.0).round() / 100.0,
        title = text(&conversation.title),
        fonts = font_faces(),
    );
    header(&mut html, conversation);
    let scrolled = if conversation.scrolled {
        " isScrolled"
    } else {
        ""
    };
    let _ = write!(
        html,
        "<div class=\"ThreadPane-messages{scrolled}\"><div class=\"ThreadPane-table\">"
    );
    for message in &conversation.messages {
        match message {
            Message::Collapsed {
                from,
                snippet,
                files,
                when,
            } => {
                let clip = if *files {
                    format!(
                        "<div class=\"MessagePane-attachmentIcon\">{}</div>",
                        icons.paperclip
                    )
                } else {
                    String::new()
                };
                let _ = write!(
                    html,
                    "<div class=\"MessagePane-collapsed\"><div class=\"MessagePane-from\">{}</div><div class=\"MessagePane-snippet\">{}</div><div class=\"MessagePane-datetime\">{clip}{}</div></div>",
                    text(from),
                    text(snippet),
                    when_html(when)
                );
            }
            Message::Marker {
                text: body,
                link,
                when,
            } => {
                let link = link
                    .as_ref()
                    .map(|link| {
                        format!(
                            "<span class=\"ThreadShareMarker-show-trashed-message\">{}</span>",
                            text(link)
                        )
                    })
                    .unwrap_or_default();
                let when = when
                    .as_ref()
                    .map(|when| {
                        format!(
                            "<span class=\"ThreadShareMarker-datetime\">{}</span>",
                            text(when)
                        )
                    })
                    .unwrap_or_default();
                let _ = write!(
                    html,
                    "<div class=\"ThreadShareMarker\"><span class=\"ThreadShareMarker-body\">{}{link}</span>{when}</div>",
                    text(body)
                );
            }
            Message::Expanded {
                header,
                body,
                files,
            } => {
                html.push_str("<div class=\"MessagePane-expanded\">");
                match header {
                    Header::Compact { participants, when } => {
                        let _ = write!(
                            html,
                            "<div class=\"MessagePane-header\"><div class=\"MessagePane-header-from\">{}</div><div class=\"MessagePane-datetime isSent\">{}</div></div>",
                            text(participants),
                            when_html(when)
                        );
                    }
                    Header::Full {
                        rows,
                        when,
                        chevron,
                        tracking_blocked,
                    } => full_header(&mut html, rows, when, *chevron, *tracking_blocked, &icons),
                }
                html.push_str("<div class=\"MessagePane-iframe-wrapper\">");
                if let Some(body) = body {
                    // A script the print view injects moves each template into its host's shadow
                    // root (`macos::SHADOW_ROOTS`).
                    let _ = write!(
                        html,
                        "<div class=\"Shadowbox\"><template>{body}</template></div>"
                    );
                }
                if !files.is_empty() {
                    html.push_str("<div class=\"AttachmentList\">");
                    for file in files {
                        let badge = if file.kind.is_empty() {
                            String::new()
                        } else {
                            format!(
                                "<div class=\"Attachment-fileType\" style=\"color: {}\"><div class=\"Attachment-fileType-inner\">{}</div></div>",
                                css_color(file.color),
                                text(&file.kind)
                            )
                        };
                        let _ = write!(
                            html,
                            "<div class=\"Attachment\"><div class=\"Attachment-container\"><div class=\"Attachment-name\">{}</div><div class=\"Attachment-bottom-container\">{badge}</div></div></div>",
                            text(&file.name)
                        );
                    }
                    html.push_str("</div>");
                }
                html.push_str("</div></div>");
            }
        }
    }
    html.push_str("</div></div></body></html>");
    html
}

/// The icons, as inline SVG markup.
struct Icons {
    paperclip: String,
    collapse: String,
    pixel_blocking: String,
}

/// The subject and the label chips.
fn header(html: &mut String, conversation: &Conversation) {
    let scrolled = if conversation.scrolled {
        " isScrolled"
    } else {
        ""
    };
    let _ = write!(
        html,
        "<div class=\"ThreadPane-header-container\"><div class=\"ThreadPane-header{scrolled}\"><span class=\"ThreadPane-subject-label\"><span class=\"ThreadPane-subject\">{}</span>",
        text(&conversation.subject)
    );
    if !conversation.labels.is_empty() {
        html.push_str("<div class=\"ThreadPane-header-labels\">");
        for label in &conversation.labels {
            let _ = write!(html, "<div class=\"ThreadLabel\">{}</div>", text(label));
        }
        html.push_str("</div>");
    }
    html.push_str("</span></div></div>");
}

/// Shift-H's header: a row for each of From, To, Cc and Bcc, then the date (or status) and the
/// chevron.
fn full_header(
    html: &mut String,
    rows: &[Row],
    when: &When,
    chevron: bool,
    tracking_blocked: bool,
    icons: &Icons,
) {
    html.push_str(
        "<div class=\"MessagePane-header headerExpanded\"><div class=\"MessagePane-emailHeader\">",
    );
    for row in rows {
        let _ = write!(
            html,
            "<div><span class=\"MessagePane-headerLabel\">{}</span>",
            row.label
        );
        for (at, (name, email)) in row.contacts.iter().enumerate() {
            if at > 0 {
                html.push_str(", ");
            }
            match name {
                Some(name) => {
                    let _ = write!(
                        html,
                        "<span class=\"ContactName-fullName\">{}</span><span class=\"ContactName-email\"> &lt;{}&gt;</span>",
                        text(name),
                        text(email)
                    );
                }
                None => {
                    let _ = write!(
                        html,
                        "<span class=\"ContactName-fullName\">{}</span>",
                        text(email)
                    );
                }
            }
        }
        if row.label == "From" && tracking_blocked && !icons.pixel_blocking.is_empty() {
            let _ = write!(
                html,
                "<span class=\"PixelBlocking\">{}</span>",
                icons.pixel_blocking
            );
        }
        html.push_str("</div>");
    }
    let _ = write!(html, "<div>{}", when_html(when));
    if chevron && !icons.collapse.is_empty() {
        let _ = write!(
            html,
            "<span class=\"MessagePane-headerCollapse\"><span class=\"MessagePane-headerCollapse-icon\">{}</span></span>",
            icons.collapse
        );
    }
    html.push_str("</div></div></div>");
}

/// What goes in a date's place.
fn when_html(when: &When) -> String {
    match when {
        When::Date(date) => format!("<time class=\"DateTime\">{}</time>", text(date)),
        When::Status {
            text: status,
            color,
        } => format!(
            "<span class=\"Status\" style=\"color: {}\">{}</span>",
            css_color(*color),
            text(status)
        ),
    }
}

/// The `@font-face` rules for the families the page and its bodies use, each loading its Exo font
/// from the print view's handler.
fn font_faces() -> String {
    let mut css = String::new();
    for &(family, weight, italic, font) in exo_mail::EMAIL_FACES {
        let style = if italic { "italic" } else { "normal" };
        let _ = write!(
            css,
            "@font-face {{ font-family: \"{family}\"; font-weight: {weight}; font-style: {style}; src: url({SCHEME}://font/{}.ttf); }}",
            font.name
        );
    }
    css
}

/// `color` as CSS.
fn css_color(color: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b())
}

/// `value` escaped for HTML text and double-quoted attributes.
fn text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::print::{File, Page};

    fn conversation(messages: Vec<Message>) -> Conversation {
        Conversation {
            title: "MSA v3 redlines".into(),
            subject: "MSA <v3> & redlines".into(),
            labels: vec!["Clients/Calloway".into()],
            messages,
            page: Page::LETTER,
            ..Conversation::default()
        }
    }

    #[test]
    fn the_page_is_laid_out_for_its_paper() {
        let html = html(&conversation(Vec::new()));
        assert!(
            html.starts_with(
                "<!DOCTYPE html><html class=\"isNarrow\" style=\"font-size: 10px; width: 777.6px\">"
            ),
            "{html}"
        );
        assert!(html.contains("<title>MSA v3 redlines</title>"));
        assert!(
            html.contains(
                "<span class=\"ThreadPane-subject\">MSA &lt;v3&gt; &amp; redlines</span>"
            ),
            "escaped: {html}"
        );
        assert!(html.contains("<div class=\"ThreadLabel\">Clients/Calloway</div>"));
        assert!(html.contains("font-family: \"Exo Sans Email\"; font-weight: 700; font-style: normal; src: url(exo-print://font/ExoSans-Bold.ttf)"));
        assert!(
            html.contains("default-src 'none'"),
            "nothing loads but what the handler answers"
        );
    }

    #[test]
    fn messages_print_as_the_thread_shows_them() {
        let messages = vec![
            Message::Collapsed {
                from: "Ben".into(),
                snippet: "Maya, Attached …".into(),
                files: true,
                when: When::Date("NOV 16".into()),
            },
            Message::Marker {
                text: "This message was moved to Trash. ".into(),
                link: Some("Show message.".into()),
                when: None,
            },
            Message::Expanded {
                header: Header::Full {
                    rows: vec![
                        Row {
                            label: "From",
                            contacts: vec![(
                                Some("Daniel Reyes".into()),
                                "daniel@brightmoor.co".into(),
                            )],
                        },
                        Row {
                            label: "To",
                            contacts: vec![
                                (None, "ben@x.law".into()),
                                (Some("Maya".into()), "maya@b.co".into()),
                            ],
                        },
                    ],
                    when: When::Status {
                        text: "Send failed".into(),
                        color: Color32::from_rgb(0xef, 0x8e, 0x8e),
                    },
                    chevron: false,
                    tracking_blocked: false,
                },
                body: Some("<div class=\"ShadowHTML\">3x</div>".into()),
                files: vec![File {
                    name: "Calloway_MSA_v3_redline".into(),
                    kind: "PDF".into(),
                    color: Color32::from_rgb(0xf7, 0x7b, 0x7b),
                }],
            },
        ];
        let html = html(&conversation(messages));
        let clip = exo_ui::icon_svg(PAPERCLIP.0)
            .expect("compiled in")
            .replace("\"#fff\"", "\"#000\"");
        assert!(html.contains(&format!("<div class=\"MessagePane-from\">Ben</div><div class=\"MessagePane-snippet\">Maya, Attached …</div><div class=\"MessagePane-datetime\"><div class=\"MessagePane-attachmentIcon\">{clip}</div><time class=\"DateTime\">NOV 16</time></div>")), "{html}");
        assert!(html.contains("<div class=\"ThreadShareMarker\"><span class=\"ThreadShareMarker-body\">This message was moved to Trash. <span class=\"ThreadShareMarker-show-trashed-message\">Show message.</span></span></div>"), "{html}");
        assert!(html.contains("<span class=\"ContactName-fullName\">Daniel Reyes</span><span class=\"ContactName-email\"> &lt;daniel@brightmoor.co&gt;</span>"), "{html}");
        assert!(html.contains("<span class=\"ContactName-fullName\">ben@x.law</span>, <span class=\"ContactName-fullName\">Maya</span>"), "{html}");
        assert!(
            html.contains(
                "<span class=\"Status\" style=\"color: #ef8e8e\">Send failed</span></div>"
            ),
            "no chevron on a failed send: {html}"
        );
        assert!(html.contains(
            "<div class=\"Shadowbox\"><template><div class=\"ShadowHTML\">3x</div></template></div>"
        ));
        assert!(html.contains("<div class=\"Attachment-fileType\" style=\"color: #f77b7b\"><div class=\"Attachment-fileType-inner\">PDF</div></div>"));
    }
}
