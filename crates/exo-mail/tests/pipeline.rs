//! The render pipeline against the exact markup expected for gmail-mock's fixtures.

use exo_mail::{Message, Render, html_from_payload, render, text_to_html};

const ZARA: &str = r#"<div dir="ltr"><div>Thanks both for making time for Aiko's onsite. Could you drop your scorecards in by tomorrow noon? She has another offer with a deadline on Friday, so we need to move quickly if it's a yes.</div></div>"#;

fn reply(new: &str, attribution: &str, previous: &str) -> String {
    format!(
        r#"{new}<br><div class="gmail_quote gmail_quote_container"><div dir="ltr" class="gmail_attr">{attribution}<br></div><blockquote class="gmail_quote" style="margin:0px 0px 0px 0.8ex;border-left:1px solid rgb(204,204,204);padding-left:1ex">{previous}</blockquote></div>"#
    )
}

fn message(html: String, name: &str, email: &str) -> Message {
    Message {
        html,
        from_email: email.into(),
        from_name: name.into(),
        ..Message::default()
    }
}

/// The hiring-debrief thread: Zara, Tom quoting Zara, Maya quoting both.
fn debrief() -> Vec<Render> {
    let tom_html = reply(
        r#"<div dir="ltr"><div>Strong yes from me. Her portfolio walkthrough was the best we've seen this cycle, and the whiteboard exercise showed she can simplify a messy flow without losing the edge cases.</div></div>"#,
        "On Mon, Nov 16, 2026 at 4:30 PM Zara Ahmed &lt;zara@brightmoor.co&gt; wrote:",
        ZARA,
    );
    let maya_html = reply(
        r#"<div dir="ltr"><div>Yes from me too. Some concern about how much she's worked directly with engineering, but she asked the right questions about constraints. Scorecard is in.</div></div>"#,
        "On Mon, Nov 16, 2026 at 5:54 PM Tom Becker &lt;tom@brightmoor.co&gt; wrote:",
        &tom_html,
    );
    let messages = [
        message(ZARA.into(), "Zara Ahmed", "zara@brightmoor.co"),
        message(tom_html, "Tom Becker", "tom@brightmoor.co"),
        message(maya_html, "Maya Okafor", "maya@brightmoor.co"),
    ];
    let mut renders: Vec<Render> = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        let replying_to = index.checked_sub(1).map(|p| (&messages[p], &renders[p]));
        renders.push(render(message, replying_to, &[]));
    }
    renders
}

#[test]
fn splits_a_gmail_reply_at_its_quote() {
    let maya = &debrief()[2];
    assert_eq!(
        maya.unquoted_html,
        r#"<div dir="ltr" class="exo-color-black exo-color exo-flush-top"><div class="exo-color-black exo-color">Yes from me too. Some concern about how much she's worked directly with engineering, but she asked the right questions about constraints. Scorecard is in.</div></div>"#
    );
    // Dates are marked without a `data-date-isostring`.
    assert_eq!(
        maya.quoted_html,
        r#"<div class="gmail_quote gmail_quote_container exo-color-black exo-color sh-quoted-content"><div dir="ltr" class="gmail_attr exo-color-black exo-color">On Mon, Nov 16, 2026 at 5:54 PM Tom Becker &lt;<a target="_blank" rel="noopener noreferrer" href="mailto:tom@brightmoor.co" class="exo-color-blue exo-color">tom@<wbr>brightmoor.<wbr>co</a>&gt; wrote:<br></div><blockquote class="gmail_quote exo-color-black exo-color" style="margin:0px 0px 0px 0.8ex;border-left:1px solid rgb(204,204,204);padding-left:1ex"><div dir="ltr" class="exo-color-black exo-color"><div class="exo-color-black exo-color">Strong yes from me. Her portfolio walkthrough was the best we've seen this cycle, and the whiteboard exercise showed she can simplify a messy flow without losing the edge cases.</div></div><br><div class="gmail_quote gmail_quote_container exo-color-black exo-color"><div dir="ltr" class="gmail_attr exo-color-black exo-color">On Mon, Nov 16, 2026 at 4:30 PM Zara Ahmed &lt;<a target="_blank" rel="noopener noreferrer" href="mailto:zara@brightmoor.co" class="exo-color-blue exo-color">zara@<wbr>brightmoor.<wbr>co</a>&gt; wrote:<br></div><blockquote class="gmail_quote exo-color-black exo-color" style="margin:0px 0px 0px 0.8ex;border-left:1px solid rgb(204,204,204);padding-left:1ex"><div dir="ltr" class="exo-color-black exo-color"><div class="exo-color-black exo-color">Thanks both for making time for Aiko's onsite. Could you drop your scorecards in by <span class="exo-date exo-color-black exo-color">tomorrow</span> noon? She has another offer with a deadline <span class="exo-date exo-color-black exo-color">on Friday</span>, so we need to move quickly if it's a yes.</div></div></blockquote></div></blockquote></div>"#
    );
    assert!(maya.has_quoted_content);
    assert!(!maya.unquoted_is_marketing && !maya.quoted_is_marketing);
    assert_eq!(
        maya.visible_text,
        "Yes from me too. Some concern about how much she's worked directly with engineering, but she asked the right questions about constraints. Scorecard is in."
    );
}

#[test]
fn marks_dates_and_tags_colors() {
    let html = r#"<div dir="ltr"><div>Heads up: today's design crit moves to 3pm, Daniel needed the 1pm slot. Same room.</div></div>"#;
    let rendered = render(
        &message(html.into(), "Tom Becker", "tom@brightmoor.co"),
        None,
        &[],
    );
    assert_eq!(
        rendered.unquoted_html,
        r#"<div dir="ltr" class="exo-color-black exo-color exo-flush-top"><div class="exo-color-black exo-color">Heads up: <span class="exo-date exo-color-black exo-color">today</span>'s design crit moves to 3pm, Daniel needed the 1pm slot. Same room.</div></div>"#
    );
    assert!(!rendered.has_quoted_content);
}

#[test]
fn hides_a_trailing_signature() {
    let html = r#"<div dir="ltr">Sounds good, see you then.<br clear="all"><br><div><div dir="ltr" class="gmail_signature">Ann Lee<div>Head of Design, Brightmoor</div></div></div></div>"#;
    let rendered = render(
        &message(html.into(), "Ann Lee", "ann@brightmoor.co"),
        None,
        &[],
    );
    assert!(rendered.has_quoted_content);
    assert!(
        rendered.quoted_html.contains("sh-signature"),
        "{}",
        rendered.quoted_html
    );
    assert!(rendered.quoted_html.contains("Head of Design"));
    assert!(!rendered.unquoted_html.contains("Head of Design"));
    assert_eq!(rendered.visible_text, "Sounds good, see you then.");
}

#[test]
fn keeps_marketing_mail_as_designed() {
    let html = r#"<html><head><style>.cta{color:#fff}@media (prefers-color-scheme: dark){body{background:#000}}</style></head>
<body style="margin:0;background:#f3f4f6"><table width="100%"><tr><td align="center" style="padding:24px">
<h1 style="font-size:26px">Big news</h1><p>Read all about it.</p>
<a href="https://news.example/read" style="display:inline-block;padding:12px 22px;background:#0f766e" class="cta">Read</a>
<p style="font-size:12px">You're receiving this because you subscribed. <a href="https://news.example/u/123">Unsubscribe</a></p>
<img src="https://t.yesware.com/t/abc/open.gif" width="1" height="1">
</td></tr></table></body></html>"#;
    let rendered = render(
        &message(html.into(), "News", "hello@news.example"),
        None,
        &[],
    );
    assert!(rendered.unquoted_is_marketing);
    assert_eq!(
        rendered.read_status_urls,
        ["https://t.yesware.com/t/abc/open.gif"]
    );
    assert_eq!(
        rendered.unsubscribe_link.as_ref().map(|l| l.url.as_str()),
        Some("https://news.example/u/123")
    );
    assert_eq!(rendered.links.len(), 2);
    assert!(
        !rendered.unquoted_html.contains("prefers-color-scheme"),
        "dark-mode CSS is dropped"
    );
    assert!(rendered.body_attributes.has_background);
    // Dark text on the light gray would vanish in dark mode, so its color is kept aside.
    assert_eq!(
        rendered.body_attributes.style.as_deref(),
        Some("margin: 0; background: #f3f4f6; --exo-original-color: rgb(0, 0, 0);")
    );
}

#[test]
fn renders_plain_text() {
    let html = text_to_html("Hi,\n\nSee https://example.com/doc.\n\n> earlier\n> text");
    let rendered = render(&message(html, "Ann", "ann@x.co"), None, &[]);
    assert!(rendered.unquoted_html.contains(
        r#"<a target="_blank" rel="noopener noreferrer" href="https://example.com/doc""#
    ));
    assert!(rendered.unquoted_html.contains("<blockquote"));
}

#[test]
fn picks_the_html_alternative() {
    use gmail::{MessagePart, MessagePartBody};
    let part = |mime: &str, data: &str| MessagePart {
        mime_type: Some(mime.into()),
        body: Some(MessagePartBody {
            data: Some(data.as_bytes().to_vec()),
            ..MessagePartBody::default()
        }),
        ..MessagePart::default()
    };
    let payload = MessagePart {
        mime_type: Some("multipart/alternative".into()),
        parts: vec![
            part("text/plain", "plain"),
            part("text/html", "<b>rich</b>"),
        ],
        ..MessagePart::default()
    };
    assert_eq!(html_from_payload(&payload), "<b>rich</b>");
    assert_eq!(
        html_from_payload(&part("text/plain", "a <b>")),
        "<p>a &lt;b&gt;</p>"
    );
}
