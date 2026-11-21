//! Editing edge cases: block boundaries, graphemes, undo, and what pasted or typed input may
//! put in the draft.

use egui::{Key, Modifiers};
use exo_editor::{Editor, Pos, Selection};

fn at(editor: &mut Editor, block: usize, offset: usize) {
    editor.set_selection(Selection::caret(Pos::new(block, offset)));
}

fn select(editor: &mut Editor, from: (usize, usize), to: (usize, usize)) {
    editor.set_selection(Selection::new(
        Pos::new(from.0, from.1),
        Pos::new(to.0, to.1),
    ));
}

#[test]
fn a_line_emptied_by_a_delete_keeps_its_br() {
    // Without it the line has no height in the sent mail.
    let mut editor = Editor::from_html("<div>a</div><div>bc</div><div>d</div>");
    select(&mut editor, (1, 0), (1, 2));
    editor.press(Key::Backspace, Modifiers::NONE);
    assert_eq!(
        editor.html(),
        "<div>a<br></div><div><br></div><div>d<br></div>"
    );

    let mut editor = Editor::from_html("<div>a</div><div>bc</div><div>d</div>");
    at(&mut editor, 0, 1);
    editor.press(Key::Backspace, Modifiers::NONE);
    assert_eq!(
        editor.html(),
        "<div><br></div><div>bc<br></div><div>d<br></div>"
    );

    let mut editor = Editor::new();
    editor.type_text("ab");
    editor.press(Key::Enter, Modifiers::NONE);
    editor.type_text("cd");
    select(&mut editor, (0, 0), (1, 2));
    editor.press(Key::Delete, Modifiers::NONE);
    assert_eq!(editor.html(), r#"<div class=""><br></div>"#);
}

#[test]
fn pasted_blank_lines_stay_lines() {
    let mut editor = Editor::new();
    editor.paste_text("a\n\nb");
    assert_eq!(
        editor.html(),
        r#"<div class="">a<br></div><div><br></div><div>b<br></div>"#
    );
}

#[test]
fn pasted_text_loses_one_trailing_newline() {
    // A plain-text paste strips it, so a copied line pastes into the line, not after it.
    let mut editor = Editor::from_html("<div>ab</div>");
    at(&mut editor, 0, 1);
    editor.paste_text("x\r\n");
    assert_eq!(editor.html(), "<div>axb<br></div>");
}

#[test]
fn pasted_tabs_show_as_the_browser_shows_them() {
    let mut editor = Editor::new();
    editor.paste_text("a\tb\u{0}c");
    assert_eq!(editor.html(), r#"<div class="">a bc<br></div>"#);
    editor.replace(Pos::new(0, 0), Pos::new(0, 1), "x\ny\u{feff}");
    assert_eq!(editor.html(), r#"<div class="">x y bc<br></div>"#);
}

#[test]
fn undo_takes_back_a_link_inserted_at_the_caret() {
    let mut editor = Editor::new();
    editor.type_text("see ");
    let before = editor.html();
    editor.set_link("example.com");
    assert_eq!(
        editor.html(),
        r#"<div class="">see <a href="http://example.com" target="_blank">example.com</a></div>"#
    );
    editor.undo();
    assert_eq!(editor.html(), before);
}

#[test]
fn delete_at_the_end_of_a_rule_keeps_the_caret_in_the_document() {
    let mut editor = Editor::from_html("<div>a</div><div><hr></div><div>b</div>");
    at(&mut editor, 1, 1);
    editor.press(Key::Delete, Modifiers::NONE);
    assert_eq!(editor.selection(), Selection::caret(Pos::new(1, 0)));
    editor.type_text("x");
    assert_eq!(editor.html(), "<div>a<br></div><div>xb<br></div>");
}

#[test]
fn a_link_pasted_into_a_link_replaces_it() {
    // HTML can't nest links: the draft would read back differently.
    let mut editor = Editor::from_html(r#"<div><a href="http://a.com">abc</a></div>"#);
    at(&mut editor, 0, 1);
    editor.paste_html(r#"<a href="http://b.com">B</a>"#);
    assert_eq!(
        editor.html(),
        r#"<div><a href="http://a.com">a</a><a href="http://b.com">B</a><a href="http://a.com">bc</a><br></div>"#
    );
}

#[test]
fn pasted_markup_cant_run() {
    let mut editor = Editor::new();
    editor.paste_html(concat!(
        r#"<a href=" java&#x09;script:alert(1)" onclick="x()">a</a><img src="data:image/png;base64,AA" onerror="x()"><iframe src="x"></iframe>"#,
        r#"<table><tr><td onmouseover="x()"><script>x()</script><a href="vbscript:x">t</a><svg><style>*{}</style></svg></td></tr></table>"#,
    ));
    // The `data:` image becomes one of the draft's inline attachments.
    let uploads = editor.take_uploads();
    assert_eq!(
        uploads
            .iter()
            .map(|upload| (upload.mime_type.as_str(), upload.bytes.as_slice()))
            .collect::<Vec<_>>(),
        [("image/png", &[0u8][..])]
    );
    let cid = &uploads[0].cid;
    assert_eq!(
        editor.html(),
        format!(
            r#"<div class=""><a>a</a><img src="" src-cid="{cid}" class="Exo-inline-image" style="max-width:100%; height: auto"></div><table><tbody><tr><td><a>t</a></td></tr></tbody></table><div class=""><br></div>"#
        )
    );
}

#[test]
fn a_draft_cant_run_either() {
    let editor =
        Editor::from_html(r#"<div><a href="javascript:x()">a</a><object data="x"></object></div>"#);
    assert_eq!(editor.html(), "<div><a>a</a><br></div>");
}

#[test]
fn hostile_nesting_doesnt_overflow_the_stack() {
    let mut editor = Editor::new();
    editor.paste_html(&("<b>".repeat(50_000) + "x"));
    editor.paste_html(&("<table><tr><td>".to_owned() + &"<span>".repeat(50_000) + "y"));
    let editor = Editor::from_html(&("<blockquote>".repeat(1_000) + "z"));
    assert!(editor.html().contains('z'));
}

#[test]
fn arrows_and_delete_take_graphemes_whole() {
    let mut editor = Editor::from_html("<div>🇺🇸e\u{301}\u{1100}\u{1161}</div>");
    let right = |editor: &mut Editor| {
        editor.press(Key::ArrowRight, Modifiers::NONE);
        editor.selection().focus.offset
    };
    assert_eq!(
        [right(&mut editor), right(&mut editor), right(&mut editor)],
        [2, 4, 6]
    );
    editor.press(Key::ArrowLeft, Modifiers::NONE);
    assert_eq!(editor.selection().focus.offset, 4);
    at(&mut editor, 0, 2);
    editor.press(Key::Delete, Modifiers::NONE);
    assert_eq!(editor.document().blocks[0].text(), "🇺🇸\u{1100}\u{1161}");
}

#[test]
fn backspace_takes_emoji_whole_and_an_accent_alone() {
    let mut editor = Editor::from_html("<div>a🇺🇸e\u{301}</div>");
    at(&mut editor, 0, 5);
    editor.press(Key::Backspace, Modifiers::NONE);
    assert_eq!(editor.document().blocks[0].text(), "a🇺🇸e");
    editor.press(Key::Backspace, Modifiers::NONE);
    editor.press(Key::Backspace, Modifiers::NONE);
    assert_eq!(editor.document().blocks[0].text(), "a");
}

#[test]
fn word_moves_and_selections_dont_split_graphemes() {
    let mut editor = Editor::from_html("<div>cafe\u{301} 👨\u{200d}👩\u{200d}👧 x</div>");
    editor.press(Key::ArrowRight, Modifiers::ALT);
    assert_eq!(editor.selection().focus.offset, 5, "past the accent");
    // A position inside a grapheme snaps to its nearer end.
    at(&mut editor, 0, 4);
    assert_eq!(editor.selection().focus.offset, 3);
    at(&mut editor, 0, 10);
    assert_eq!(editor.selection().focus.offset, 11);
}

#[test]
fn the_link_input_ignores_what_the_sanitizer_strips() {
    let mut editor = Editor::new();
    editor.type_text("x");
    editor.set_selection(Selection::new(Pos::new(0, 0), Pos::new(0, 1)));
    editor.set_link("data:text/html,<script>alert(1)</script>");
    editor.set_link("javascript:alert(1)");
    assert_eq!(
        editor.html(),
        r#"<div class=""><a href="http://javascript:alert(1)" target="_blank">x</a></div>"#
    );
}

#[test]
fn pastes_are_autolinked_as_squire_does() {
    // A paste links the URLs in the pasted fragment (a bare host gets its `/`); for HTML (not
    // plain text) a text node that is only an address stays as it is.
    let mut editor = Editor::new();
    editor.paste_text("see http://x.com or a@b.co");
    assert_eq!(
        editor.html(),
        r#"<div class="">see <a href="http://x.com/" target="_blank">http://x.com</a> or <a href="mailto:a@b.co" target="_blank">a@b.co</a><br></div>"#
    );
    let mut editor = Editor::new();
    editor.paste_html("<b>a@b.co</b> at http://x.com");
    assert_eq!(
        editor.html(),
        r#"<div class=""><b>a@b.co</b> at <a href="http://x.com/" target="_blank">http://x.com</a><br></div>"#
    );
}

#[test]
fn space_links_the_url_before_it() {
    let mut editor = Editor::new();
    editor.type_text("go http://x.com now");
    assert_eq!(
        editor.html(),
        r#"<div class="">go <a href="http://x.com/" target="_blank">http://x.com</a> now</div>"#
    );
    // As in Squire, a line that is only an address isn't linked by Space.
    let mut editor = Editor::new();
    editor.type_text("a@b.co ");
    assert_eq!(editor.html(), r#"<div class="">a@b.co&nbsp;</div>"#);
}

#[test]
fn a_selection_between_two_rules_deletes_neither() {
    let mut editor = Editor::from_html("<div><hr></div><div><hr></div><div>x</div>");
    select(&mut editor, (0, 1), (1, 0));
    editor.press(Key::Backspace, Modifiers::NONE);
    assert_eq!(
        editor.html(),
        "<div><hr></div><div><hr></div><div>x<br></div>"
    );
}

#[test]
fn tab_then_shift_tab_over_items_gives_back_the_list() {
    // The selection ends at the start of the third item, which Tab leaves out; it
    // must stay there, so Shift+Tab takes the same two items back out.
    let mut editor =
        Editor::from_html("<ul><li>a<br></li><li>b<br></li><li>c<br></li></ul><div><br></div>");
    select(&mut editor, (0, 1), (2, 0));
    assert!(editor.indent_list());
    assert_eq!(
        editor.selection(),
        Selection::new(Pos::new(0, 1), Pos::new(2, 0))
    );
    assert!(editor.outdent_list());
    // Outdented items take their level's style, as in the `numbered_list_nesting_and_leaving`
    // capture.
    let item = r#"<li aria-level="1" style="list-style-type:disc">"#;
    assert_eq!(
        editor.html(),
        format!("<ul>{item}a<br></li>{item}b<br></li><li>c<br></li></ul><div><br></div>")
    );
}

#[test]
fn enter_after_dashes_in_a_longer_line_splits_it() {
    // Enter reads the line's whole text, not what's before the caret: `---abc` isn't a rule.
    let mut editor = Editor::from_html("<div>---abc</div>");
    at(&mut editor, 0, 3);
    editor.press(Key::Enter, Modifiers::NONE);
    assert_eq!(editor.html(), "<div>---<br></div><div>abc<br></div>");
}

#[test]
fn dashes_in_a_quote_make_a_rule_in_it() {
    // The rule check comes before the quote's break-out.
    let mut editor = Editor::from_html(
        r#"<blockquote style="x"><div>q</div><div>---</div></blockquote><div><br></div>"#,
    );
    at(&mut editor, 1, 3);
    editor.press(Key::Enter, Modifiers::NONE);
    assert_eq!(
        editor.html(),
        r#"<blockquote style="x"><div>q<br></div><div><hr></div><div><br></div></blockquote><div><br></div>"#
    );
    assert_eq!(editor.selection(), Selection::caret(Pos::new(2, 0)));
}

#[test]
fn writing_doesnt_recurse_per_container() {
    // Editing has no depth cap (each paste of a quote into itself nests one more), so writing
    // the body mustn't use the stack per level.
    let path = (0..50_000)
        .map(|id| exo_editor::Container {
            id,
            element: exo_editor::Element::new("blockquote"),
        })
        .collect();
    let doc = exo_editor::Document::new(vec![exo_editor::Block::div(path, Vec::new())]);
    assert_eq!(
        exo_editor::html::write(&doc)
            .matches("<blockquote>")
            .count(),
        50_000
    );
}

#[test]
fn a_mention_opens_once_a_name_follows_its_trigger() {
    // `+` alone matches nothing; the name typed after it opens the contacts (the trigger was
    // armed when it was typed).
    let mut editor = Editor::new();
    editor.type_text("Thanks +");
    assert!(editor.trigger().is_none());
    editor.type_text("pri");
    let trigger = editor.trigger().expect("open");
    assert_eq!(
        (trigger.kind, trigger.query.as_str(), trigger.start),
        (exo_editor::TriggerKind::Contact('+'), "pri", Pos::new(0, 7))
    );
    let (start, end) = (trigger.start, trigger.end);
    editor.replace(start, end, "+Priya");
    assert!(editor.trigger().is_none());
    editor.type_text(" and me");
    assert!(
        editor.trigger().is_none(),
        "picking ends it until the next trigger character"
    );
}

#[test]
fn modify_html_is_one_undo_step_and_keeps_the_caret_from_the_end() {
    let mut editor = Editor::new();
    editor.modify_html("<div class=\"Superhuman-Intro-Snippet\">Thanks <span class=\"Superhuman-Intro-Name\">Sam</span>!<div><br></div></div><div><br></div>");
    assert_eq!(editor.document().blocks[0].text(), "Thanks Sam!");
    assert_eq!(
        editor.selection().focus.block,
        editor.document().blocks.len() - 1,
        "the caret stays on the line it was on, now the last"
    );
    editor.type_text("Hi");
    editor.press(Key::Z, Modifiers::COMMAND);
    editor.press(Key::Z, Modifiers::COMMAND);
    assert!(editor.is_empty(), "back to the empty body");
}

#[test]
fn a_number_that_follows_a_list_cut_short_continues_it() {
    // Typed: `1. a`, `b`, out of the list, `para`, then `3. c`, `d`.
    let mut editor = Editor::new();
    editor.type_text("1. a\nb\n\npara\n3. c\nd");
    assert_eq!(
        editor.html(),
        r#"<ol><li value="1">a<br></li><li>b<br></li></ol><div>para<br></div><ol><li value="3">c<br></li><li>d</li></ol><div><br></div>"#,
        "`3.` continues the list the paragraph cut short"
    );
    let mut skipped = Editor::new();
    skipped.type_text("1. a\n\npara\n3. c");
    assert!(
        skipped.html().contains("<div>3. c"),
        "`3.` after a one-item list isn't its next number: {}",
        skipped.html()
    );
}

#[test]
fn down_or_right_at_the_very_end_goes_past_it() {
    // What follows the caret shows nothing and has one line break at most.
    let mut editor = Editor::from_html("<div>Thanks</div><div><br></div>");
    at(&mut editor, 0, 6);
    assert_eq!(
        editor.press(Key::ArrowDown, Modifiers::NONE),
        None,
        "an empty line follows"
    );
    at(&mut editor, 1, 0);
    assert_eq!(
        editor.press(Key::ArrowDown, Modifiers::NONE),
        Some(exo_editor::KeyEffect::PastEnd)
    );
    assert_eq!(
        editor.press(Key::ArrowRight, Modifiers::NONE),
        Some(exo_editor::KeyEffect::PastEnd)
    );
    assert_eq!(
        editor.press(Key::ArrowDown, Modifiers::SHIFT),
        None,
        "extending isn't moving on"
    );
    at(&mut editor, 0, 3);
    assert_eq!(editor.press(Key::ArrowRight, Modifiers::NONE), None);
}

#[test]
fn a_fence_and_enter_make_a_code_block() {
    // The fence's own line is emptied, not left holding a space.
    let mut editor = Editor::from_html("<div><br></div>");
    editor.type_text("Code:\n```\nlet x = 1\ny");
    assert_eq!(
        editor.html(),
        "<div>Code:<br></div><pre style=\"margin-left: 20px;\"><div>let x = 1\ny<br></div></pre><div><br></div>"
    );
    // A second Enter on the empty line leaves the block for a plain line.
    editor.type_text("\n\nafter");
    assert_eq!(
        editor.html(),
        "<div>Code:<br></div><pre style=\"margin-left: 20px;\"><div>let x = 1\ny<br></div></pre><div>after</div><div><br></div>"
    );
    // A code block reads back with its white space.
    let round_trip = Editor::from_html(&editor.html());
    assert!(round_trip.html().starts_with("<div>Code:<br></div><pre style=\"margin-left: 20px;\"><div>let x = 1\ny<br></div></pre>"), "{}", round_trip.html());
    let mut spaced = Editor::from_html("<pre>  two\n    four</pre>");
    assert_eq!(spaced.html(), "<pre>  two\n    four</pre>");
    at(&mut spaced, 0, 5);
    spaced.enter(true);
    assert_eq!(
        spaced.html(),
        "<pre>  two\n\n    four</pre>",
        "Shift+Enter is a line break, even on an empty line"
    );
}

#[test]
fn an_empty_line_in_the_middle_of_code_splits_it() {
    let mut editor = Editor::from_html("<pre style=\"margin-left: 20px;\"><div>a\n\nb</div></pre>");
    at(&mut editor, 0, 2);
    editor.enter(false);
    assert_eq!(
        editor.html(),
        "<pre style=\"margin-left: 20px;\"><div>a<br></div></pre><div><br></div><pre style=\"margin-left: 20px;\"><div>b</div></pre>"
    );
    assert_eq!(editor.selection(), Selection::caret(Pos::new(1, 0)));
    editor.undo();
    assert_eq!(
        editor.html(),
        "<pre style=\"margin-left: 20px;\"><div>a\n\nb</div></pre>"
    );
    // Not in a list, and not when the line holds more than the fence.
    let mut list = Editor::new();
    list.type_text("* ```\n");
    assert!(!list.html().contains("<pre"), "{}", list.html());
    let mut more = Editor::new();
    more.type_text("```js\n");
    assert!(!more.html().contains("<pre"), "{}", more.html());
}
