//! HTML in and out: draft bodies read back byte for byte.

use exo_editor::{ContainerKind, Content, Editor, Piece, html};

#[track_caller]
fn round_trip(body: &str) {
    let doc = html::parse(body);
    assert_eq!(html::write(&doc), body);
    assert_eq!(
        html::parse(&html::write(&doc)),
        doc,
        "parsing what was written gives the same document"
    );
}

/// Draft bodies another client saved (its editor's HTML, theme classes stripped).
#[test]
fn drafts_saved_elsewhere_round_trip() {
    round_trip(
        r#"<div class="">Hello&nbsp;<b>bold</b> end<br></div><ul><li>one<br></li><ul><li aria-level="2" style="list-style-type:circle">two</li></ul></ul>"#,
    );
    round_trip(
        r#"<ol><li>a<br></li><ol><li aria-level="2" style="list-style-type:lower-alpha">b<br></li><li aria-level="2" style="list-style-type:lower-alpha">c<br></li></ol><li aria-level="1" style="list-style-type:decimal">d<br></li></ol><div>e</div><div><br></div>"#,
    );
    round_trip(
        r#"<blockquote style="margin:0 0 0 40px;border:none;padding:0px"><div class="">i1<br></div></blockquote><div><div><br></div></div><blockquote style="margin:0 0 0 40px;border:none;padding:0px"><div><div>i2<br></div></div></blockquote><div><div><br></div><div>i3<br></div><div><br></div></div><div><div><br></div></div><div><br></div>"#,
    );
    round_trip(r#"<div class="">alpha beta gamma <i><u>delta<s> strike</s></u></i></div>"#);
    round_trip(
        r#"<div class=""><span class="colour" style="color:rgb(222, 84, 84) !important">red text after</span></div>"#,
    );
    round_trip(
        r#"<div class="">see <a href="http://example.com/docs" target="_blank">docs </a>here</div>"#,
    );
    round_trip(
        r#"<div class="">a&nbsp; b&nbsp; &nbsp;c&nbsp;<br></div><div class="">&nbsp;lead</div>"#,
    );
    round_trip(
        r#"<ul><li>x<br></li><li>y<br></li></ul><ol><li value="1">n</li></ol><div><br></div>"#,
    );
    round_trip(r#"<div class="">top<br></div><div><hr></div><div class="">after</div>"#);
    round_trip(r#"<ul><li>li<br>soft</li></ul>div<br>two<br><div><br></div>"#);
}

/// A revealed reply as another client's editor holds it: text straight inside containers, `<br>`
/// between blocks, nested inline wrappers.
#[test]
fn revealed_quotes_round_trip() {
    round_trip(
        r#"<div class=""><div class=""><div class=""><div class="">hi<br></div></div><br><div class="sh-signature"><div class="gmail_signature"><div style="clear:both" class="">Sent via <a href="https://superhuman.com/products/mail" target="_blank" class="">Superhuman</a><br></div><br></div></div></div><br><div class="sh-quoted-content"><div class=""><div class="gmail_quote">On Tue, Nov 17, 2026 at 8:31 AM, Tom Becker <span dir="ltr" class="">&lt;<a href="mailto:tom@brightmoor.co" target="_blank" class="">tom@brightmoor.co</a>&gt;</span> wrote:<br><blockquote class="gmail_quote" style="margin:0 0 0 .8ex;border-left:1px #ccc solid;padding-left:1ex"><div class="gmail_extra"><div class="gmail_quote"><div dir="ltr" class=""><div class="">Heads up: <span class="sh-date" data-date-isostring="2026-11-17">today</span>'s design crit moves to 3pm, Daniel needed the 1pm slot. Same room.<br></div></div></div></div></blockquote></div></div></div></div>"#,
    );
}

#[test]
fn foreign_markup_round_trips() {
    // A Gmail signature with a table, kept whole; an image; an empty element.
    round_trip(
        r#"<div dir="ltr"><table cellpadding="0"><tbody><tr><td>Jane Doe</td><td>CEO</td></tr></tbody></table></div><div class="gmail_signature"></div>"#,
    );
    round_trip(
        r#"<div>see <img class="Exo-inline-image" src-cid="abc" style="max-width: 100%; height: auto; width:120px"> here</div>"#,
    );
    round_trip(
        r#"<div>a<sub>2</sub> <span class="font" style="font-family:monospace">code</span></div>"#,
    );
}

#[test]
fn the_model_is_what_squire_holds() {
    let doc = html::parse(
        r#"<div>a</div><blockquote style="x"><div>b</div><div>c</div></blockquote><blockquote style="x"><div>d</div></blockquote>"#,
    );
    let quotes: Vec<_> = doc
        .blocks
        .iter()
        .map(|block| block.path.first().map(|c| c.id))
        .collect();
    assert_eq!(quotes[1], quotes[2], "b and c are in one quote");
    assert_ne!(quotes[2], quotes[3], "d is in another");
    assert_eq!(
        doc.blocks[1].path[0].element.container_kind(),
        ContainerKind::Quote
    );
    let doc = html::parse("<div>x<br>y</div>");
    assert!(
        matches!(&doc.blocks[0].content, Content::Inline(runs) if runs.len() == 3 && runs[1].piece == Piece::Break)
    );
}

#[test]
fn whitespace_collapses_like_css() {
    let doc = html::parse("<div>\n  one   two\n</div>\n<div> three </div>");
    assert_eq!(html::write(&doc), "<div>one two</div><div>three</div>");
}

#[test]
fn equivalent_tags_normalize() {
    assert_eq!(
        html::write(&html::parse(
            "<div><strong>a</strong><em>b</em><strike>c</strike></div>"
        )),
        "<div><b>a</b><i>b</i><s>c</s></div>"
    );
}

#[test]
fn loading_a_draft_gives_lines_their_fix_cursor_br() {
    // Loading gives every block without a `<br>` one.
    let editor = Editor::from_html("<div>one</div><div>two<br>three</div>");
    assert_eq!(editor.html(), "<div>one<br></div><div>two<br>three</div>");
}

#[test]
fn pastes_keep_only_the_allowed_styles() {
    let mut editor = Editor::new();
    editor.type_text("x");
    editor.paste_html(r#"<span style="font-size:30px;color:red;font-weight:600">big</span> <span style="font-family:Arial">red</span>"#);
    assert_eq!(
        editor.html(),
        r#"<div class="">x<span style="color:red;font-weight:700">big</span> red</div>"#
    );
}

#[test]
fn pastes_lose_the_app_classes() {
    let mut editor = Editor::new();
    editor.type_text("x");
    editor.paste_html(r#"<span class="exo-date Theme-dark">today</span> <b class="ShadowHTML Exo-quoting-ellipsis note">bold</b>"#);
    // The date's span, left without a class, goes too.
    assert_eq!(
        editor.html(),
        r#"<div class="">xtoday <b class="note">bold</b></div>"#
    );
}

#[test]
fn plain_paste_makes_a_line_per_line() {
    let mut editor = Editor::new();
    editor.type_text("ab");
    editor.set_selection(exo_editor::Selection::caret(exo_editor::Pos::new(0, 1)));
    editor.paste_text("one\ntwo");
    // A paste gives each pasted line its trailing `<br>`; joining the line's start keeps the first
    // one's, and joining the line's end drops the last one's.
    assert_eq!(
        editor.html(),
        r#"<div class="">aone<br></div><div>twob</div>"#
    );
}

#[test]
fn sending_links_bare_urls_as_linkify_does() {
    // `Linkify.text` writes the address as a browser's `URL` prints it: a bare host gets its `/`.
    let editor = Editor::from_html(
        r#"<div>see http://x.com, or Y.com/ü.</div><div><a href="http://y.com">http://y.com</a></div>"#,
    );
    let parts = exo_editor::outgoing::Parts {
        inlined: true,
        ..Default::default()
    };
    assert_eq!(
        exo_editor::outgoing::html(editor.document(), &parts),
        r#"<div><div><html><head></head><body><div>see <a target="_blank" rel="noopener noreferrer" href="http://x.com/">http://x.com</a>, or <a target="_blank" rel="noopener noreferrer" href="http://y.com/%C3%BC">Y.com/ü</a>.<br></div><div><a href="http://y.com">http://y.com</a><br></div></body></html></div><div></div></div>"#
    );
}

#[test]
fn drafts_load_as_dompurify_leaves_them() {
    // Loading sanitizes with DOMPurify's defaults, which don't allow `target`; the
    // inline image's `src-cid` is allowed for it.
    let editor = Editor::from_html(
        r#"<div>see <a href="http://x.com/" target="_blank" onclick="x()">x</a><img src-cid="abc" src="javascript:x()"></div>"#,
    );
    assert_eq!(
        editor.html(),
        r#"<div>see <a href="http://x.com/">x</a><img src-cid="abc"><br></div>"#
    );
}
