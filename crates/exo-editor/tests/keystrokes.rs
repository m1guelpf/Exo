//! Keystroke scripts typed into a reply, each checked against the exact body the reply sends.
//!
//! `expected` is the sent body between `<body>` and `</body>`. Its first line starts with an empty
//! `class=""`, as a sent draft's does.

use egui::{Key, Modifiers};
use exo_editor::{Color, Editor, FormatCommand, outgoing};

/// One step of a script, its keys named as Playwright names them.
enum Step {
    Type(&'static str),
    Press(&'static str),
}

use Step::{Press, Type};

fn run(steps: &[Step]) -> Editor {
    let mut editor = Editor::new();
    let mut link: Option<String> = None;
    for step in steps {
        match step {
            Type(text) if link.is_some() => link.as_mut().expect("checked").push_str(text),
            Type(text) => editor.type_text(text),
            Press("Enter") if link.is_some() => editor.set_link(&link.take().expect("checked")),
            Press(name) => {
                if let Some(command) = command(name) {
                    if command == FormatCommand::Hyperlink {
                        link = Some(String::new());
                    }
                    if command.applies(&editor) {
                        command.run(&mut editor);
                        continue;
                    }
                }
                let (key, modifiers) = key(name);
                editor.press(key, modifiers);
            }
        }
    }
    editor
}

/// The formatting keys go through commands in the app (exo-commands dispatches them).
fn command(name: &str) -> Option<FormatCommand> {
    FormatCommand::ALL.into_iter().find(|command| {
        command.keys().iter().any(|binding| {
            let playwright = binding
                .replace("mod+", "Meta+")
                .replace("shift+", "Shift+")
                .replace(']', "BracketRight")
                .replace('[', "BracketLeft")
                .replace("tab", "Tab");
            let playwright = playwright
                .rsplit_once('+')
                .map_or_else(|| playwright.clone(), |(mods, key)| format!("{mods}+{key}"));
            playwright.eq_ignore_ascii_case(name)
        })
    })
}

fn key(name: &str) -> (Key, Modifiers) {
    let mut parts: Vec<&str> = name.split('+').collect();
    let key = parts.pop().expect("a key");
    let mut modifiers = Modifiers::NONE;
    for modifier in parts {
        match modifier {
            "Shift" => modifiers.shift = true,
            "Alt" => modifiers.alt = true,
            "Meta" => {
                modifiers.mac_cmd = true;
                modifiers.command = true;
            }
            other => panic!("modifier {other}"),
        }
    }
    (
        Key::from_name(key).unwrap_or_else(|| panic!("key {key}")),
        modifiers,
    )
}

fn body(editor: &Editor) -> String {
    let html = outgoing::html(
        editor.document(),
        &outgoing::Parts {
            inlined: true,
            ..Default::default()
        },
    );
    html.split("<body>")
        .nth(1)
        .and_then(|rest| rest.split("</body>").next())
        .expect("a body")
        .to_owned()
}

#[track_caller]
fn check(steps: &[Step], expected: &str) {
    assert_eq!(body(&run(steps)), expected);
}

#[test]
fn bold_then_a_nested_list() {
    // `Hello <b>`, not `Hello&nbsp;<b>`: the space typed at the line's end doesn't stay a
    // no-break space once ⌘B moves typing into a new text node; it's written as the finished line
    // needs it.
    check(
        &[
            Type("Hello "),
            Press("Meta+b"),
            Type("bold"),
            Press("Meta+b"),
            Type(" end"),
            Press("Enter"),
            Press("Meta+Shift+8"),
            Type("one"),
            Press("Enter"),
            Press("Tab"),
            Type("two"),
        ],
        r#"<div class="">Hello <b>bold</b> end<br></div><ul><li>one<br></li><ul><li aria-level="2" style="list-style-type:circle">two</li></ul></ul>"#,
    );
}

#[test]
fn numbered_list_nesting_and_leaving() {
    check(
        &[
            Type("a"),
            Press("Meta+Shift+7"),
            Press("Enter"),
            Type("b"),
            Press("Tab"),
            Press("Enter"),
            Type("c"),
            Press("Enter"),
            Press("Shift+Tab"),
            Type("d"),
            Press("Enter"),
            Press("Enter"),
            Type("e"),
        ],
        r#"<ol><li>a<br></li><ol><li aria-level="2" style="list-style-type:lower-alpha">b<br></li><li aria-level="2" style="list-style-type:lower-alpha">c<br></li></ol><li aria-level="1" style="list-style-type:decimal">d<br></li></ol><div>e</div><div><br></div>"#,
    );
}

const QUOTE: &str = "background-image: -webkit-linear-gradient(left, rgb(237, 237, 237) 75%, rgb(237, 237, 237) 75%);background-size: 1px 1px;background-position: left;background-repeat: repeat-y;padding: 0 0 0 20px;";
const INDENT: &str = "margin:0 0 0 40px;border:none;padding:0px";

#[test]
fn enter_in_a_quote_breaks_out_into_three_lines() {
    check(
        &[
            Type("q1"),
            Press("Meta+Shift+9"),
            Press("Enter"),
            Type("q2"),
        ],
        &format!(
            r#"<blockquote style="{QUOTE}"><div class="">q1<br></div></blockquote><div><div><br></div><div>q2</div><div><br></div></div><div><br></div>"#
        ),
    );
}

#[test]
fn enter_mid_quote_splits_it() {
    check(
        &[
            Type("abcdef"),
            Press("Meta+Shift+9"),
            Press("ArrowLeft"),
            Press("ArrowLeft"),
            Press("ArrowLeft"),
            Press("Enter"),
            Type("X"),
        ],
        &format!(
            r#"<blockquote style="{QUOTE}"><div class="">abc<br></div></blockquote><div><div><br></div><div>X</div><div><br></div></div><blockquote style="{QUOTE}"><div class="">def</div></blockquote><div><br></div>"#
        ),
    );
}

#[test]
fn increase_level_in_a_quote_wraps_it() {
    check(
        &[
            Type("q1"),
            Press("Meta+Shift+9"),
            Press("Meta+BracketRight"),
            Type(" deeper"),
        ],
        &format!(
            r#"<blockquote style="{QUOTE}"><blockquote style="{QUOTE}"><div class="">q1 deeper</div></blockquote></blockquote><div><br></div>"#
        ),
    );
}

#[test]
fn indents() {
    // A ⌘[ that does nothing leaves the line as typed (`i3`, no `<br>` added).
    check(
        &[
            Type("i1"),
            Press("Meta+BracketRight"),
            Press("Enter"),
            Type("i2"),
            Press("Meta+BracketRight"),
            Press("Enter"),
            Type("i3"),
            Press("Meta+BracketLeft"),
        ],
        &format!(
            r#"<blockquote style="{INDENT}"><div class="">i1<br></div></blockquote><div><div><br></div></div><blockquote style="{INDENT}"><div><div>i2<br></div></div></blockquote><div><div><br></div><div>i3</div><div><br></div></div><div><div><br></div></div><div><br></div>"#
        ),
    );
}

#[test]
fn italic_underline_then_strike_typed_inside() {
    check(
        &[
            Type("alpha beta gamma delta"),
            Press("Shift+ArrowLeft"),
            Press("Shift+ArrowLeft"),
            Press("Shift+ArrowLeft"),
            Press("Shift+ArrowLeft"),
            Press("Shift+ArrowLeft"),
            Press("Meta+i"),
            Press("Meta+u"),
            Press("End"),
            Press("Meta+Shift+x"),
            Type(" strike"),
        ],
        r#"<div class="">alpha beta gamma <i><u>delta<s> strike</s></u></i></div>"#,
    );
}

#[test]
fn colour_continues_when_typing_after_it() {
    let mut editor = run(&[Type("red text"), Press("Shift+Home")]);
    editor.set_color(Color::LavaRed);
    editor.press(Key::End, Modifiers::NONE);
    editor.type_text(" after");
    assert_eq!(
        body(&editor),
        r#"<div class=""><span class="colour" style="color:rgb(222, 84, 84) !important">red text after</span></div>"#
    );
}

#[test]
fn no_color_is_a_dark_gray_span() {
    let mut editor = run(&[Type("plain"), Press("Shift+Home")]);
    editor.set_color(Color::None);
    editor.press(Key::End, Modifiers::NONE);
    editor.type_text(" z");
    assert_eq!(
        body(&editor),
        r#"<div class=""><span class="colour" style="color:rgb(34, 34, 34)">plain z</span></div>"#
    );
}

#[test]
fn link_from_the_input_gets_a_scheme() {
    check(
        &[
            Type("see docs here"),
            Press("Alt+ArrowLeft"),
            Press("Alt+Shift+ArrowLeft"),
            Press("Meta+k"),
            Type("example.com/docs"),
            Press("Enter"),
        ],
        r#"<div class="">see <a href="http://example.com/docs" target="_blank">docs </a>here</div>"#,
    );
}

#[test]
fn spaces_are_written_as_chromium_leaves_them() {
    check(
        &[Type("a  b   c "), Press("Enter"), Type(" lead")],
        r#"<div class="">a&nbsp; b&nbsp; &nbsp;c&nbsp;<br></div><div class="">&nbsp;lead</div>"#,
    );
    check(
        &[
            Type("a    b"),
            Press("Enter"),
            Type("  lead2"),
            Press("Enter"),
            Type("trail2  "),
        ],
        r#"<div class="">a&nbsp; &nbsp; b<br></div><div class="">&nbsp; lead2<br></div><div class="">trail2&nbsp;&nbsp;</div>"#,
    );
}

#[test]
fn markdown_lists() {
    check(
        &[
            Type("* x"),
            Press("Enter"),
            Type("y"),
            Press("Enter"),
            Press("Enter"),
            Type("1. n"),
        ],
        r#"<ul><li>x<br></li><li>y<br></li></ul><ol><li value="1">n</li></ol><div><br></div>"#,
    );
}

#[test]
fn backspace_joins_list_items() {
    check(
        &[
            Type("* a"),
            Press("Enter"),
            Type("b"),
            Press("Home"),
            Press("Backspace"),
            Press("End"),
            Press("Enter"),
            Type("c"),
            Press("Home"),
            Press("Backspace"),
            Press("Backspace"),
        ],
        r#"<ul><li>ac</li></ul><div><br></div>"#,
    );
}

#[test]
fn three_dashes_make_a_rule() {
    check(
        &[
            Type("top"),
            Press("Enter"),
            Type("---"),
            Press("Enter"),
            Type("after"),
        ],
        r#"<div class="">top<br></div><div><hr></div><div class="">after</div>"#,
    );
}

#[test]
fn unicode_replacements() {
    check(
        &[Type("a -> b -- c... d")],
        r#"<div class="">a → b — c… d</div>"#,
    );
}

#[test]
fn backspace_right_after_a_replacement_undoes_it() {
    let editor = run(&[Type("a ->"), Press("Backspace")]);
    assert_eq!(body(&editor), r#"<div class="">a -&gt;</div>"#);
}

/// The reply's quote, as it's built from the message.
fn quote() -> String {
    let sent: jiff::Zoned = "2026-11-17T08:31:00-05:00[America/New_York]"
        .parse()
        .expect("a date");
    let message = r#"<div dir="ltr" class="exo-color-black exo-color"><div class="exo-color-black exo-color">Heads up: <span class="exo-date exo-color-black exo-color" data-date-isostring="2026-11-17">today</span>'s design crit moves to 3pm, Daniel needed the 1pm slot. Same room.</div></div>"#;
    outgoing::reply_quote(
        &sent,
        "Tom Becker",
        "tom@brightmoor.co",
        "exo-color-black exo-color",
        "",
        "",
        message,
    )
}

#[test]
fn reply_sent_without_opening_the_quote() {
    // No read statuses and no "Sent via" line: the pixel slot is empty and the signature block
    // holds nothing.
    let editor = run(&[
        Type("Hello "),
        Press("Meta+b"),
        Type("bold"),
        Press("Meta+b"),
        Type(" end"),
        Press("Enter"),
        Press("Meta+Shift+8"),
        Type("one"),
        Press("Enter"),
        Press("Tab"),
        Type("two"),
    ]);
    let quote = quote();
    let parts = outgoing::Parts {
        quote: Some(&quote),
        include_signature_content: true,
        ..Default::default()
    };
    assert_eq!(
        outgoing::html(editor.document(), &parts),
        r#"<div><div><div><html><head></head><body><div class="">Hello <b>bold</b> end<br></div><ul><li>one<br></li><ul><li aria-level="2" style="list-style-type:circle">two</li></ul></ul></body></html></div><div></div><br/><div class="gmail_signature"></div></div><br/><div><html><head></head><body><div class="gmail_quote">On Tue, Nov 17, 2026 at 8:31 AM, Tom Becker <span dir="ltr">&lt;<a href="mailto:tom@brightmoor.co" target="_blank">tom@brightmoor.co</a>&gt;</span> wrote:<br><blockquote class="gmail_quote" style="margin:0 0 0 .8ex;border-left:1px #ccc solid;padding-left:1ex"><div class="gmail_extra"><div class="gmail_quote exo-color-black exo-color"><div dir="ltr" class="exo-color-black exo-color"><div class="exo-color-black exo-color">Heads up: <span class="exo-date exo-color-black exo-color" data-date-isostring="2026-11-17">today</span>'s design crit moves to 3pm, Daniel needed the 1pm slot. Same room.</div></div></div></div></blockquote></div></body></html></div><br/></div>"#
    );
}

#[test]
fn reply_sent_after_opening_the_quote() {
    let mut editor = run(&[Type("hi")]);
    let quote = quote();
    let signature = outgoing::signature(None, true, Some("draft002d77693774c3f1"));
    editor.reveal_quoted(Some(&signature), Some(&quote));
    let parts = outgoing::Parts {
        inlined: true,
        ..Default::default()
    };
    // An empty signature holds just its `<br>`.
    assert_eq!(
        outgoing::html(editor.document(), &parts),
        r#"<div><div><html><head></head><body><div class=""><div class=""><div class=""><div class="">hi<br></div></div><br><div class="sh-signature"><div class="gmail_signature"><br></div></div></div><br><div class="sh-quoted-content"><div class=""><div class="gmail_quote">On Tue, Nov 17, 2026 at 8:31 AM, Tom Becker <span dir="ltr" class="">&lt;<a href="mailto:tom@brightmoor.co" target="_blank" class="">tom@brightmoor.co</a>&gt;</span> wrote:<br><blockquote class="gmail_quote" style="margin:0 0 0 .8ex;border-left:1px #ccc solid;padding-left:1ex"><div class="gmail_extra"><div class="gmail_quote"><div dir="ltr" class=""><div class="">Heads up: <span class="exo-date" data-date-isostring="2026-11-17">today</span>'s design crit moves to 3pm, Daniel needed the 1pm slot. Same room.<br></div></div></div></div></blockquote></div></div></div></div></body></html></div><div></div></div>"#
    );
}

#[test]
fn a_draft_begun_elsewhere_goes_without_its_sh_colour_tags() {
    // Its quote, appended as the other client saved it to Gmail, is in the body Exo loads.
    let draft = exo_editor::html::parse(
        r#"<div class="">ok<br></div><div class="gmail_quote sh-color-black sh-color"><div class="sh-color-red sh-color sh-modified-inline" style="color:#c00">red <span class="sh-date sh-color-black sh-color">today</span></div></div>"#,
    );
    let parts = outgoing::Parts {
        inlined: true,
        ..Default::default()
    };
    assert_eq!(
        outgoing::html(&draft, &parts),
        r#"<div><div><html><head></head><body><div class="">ok<br></div><div class="gmail_quote"><div class="" style="color:#c00">red <span class="sh-date">today</span></div></div></body></html></div><div></div></div>"#
    );
}

#[test]
fn changing_from_swaps_an_untouched_signature() {
    let id = "draft002d77693774c3f1";
    let maya = outgoing::signature(Some("<b>Maya</b> Okafor"), true, Some(id));
    let hello = outgoing::signature(Some("Hello from Maya"), true, Some(id));
    let mut editor = run(&[Type("hi")]);
    editor.reveal_quoted(Some(&maya), Some(&quote()));
    assert!(
        editor.switch_signature(id, &maya, &hello),
        "an untouched signature follows From"
    );
    let html = editor.html();
    assert!(
        html.contains("Hello from Maya") && !html.contains("Okafor") && html.contains("Heads up"),
        "{html}"
    );
    assert!(
        html.starts_with(r#"<div class=""><div class=""><div class=""><div class="">hi"#),
        "the body stays: {html}"
    );
    // Edited, it stays as the user left it.
    let mut edited = Editor::from_html(&html.replace("Hello from Maya", "Cheers"));
    assert!(!edited.switch_signature(id, &hello, &maya));
    assert!(edited.html().contains("Cheers"));
}

#[test]
fn undo_steps_at_boundaries() {
    let mut editor = Editor::new();
    editor.type_text("abc def");
    editor.undo();
    assert_eq!(
        body(&editor),
        r#"<div class="">abc</div>"#,
        "the space is a boundary: ' def' undoes in one step"
    );
    editor.undo();
    assert_eq!(body(&editor), r#"<div class=""><br></div>"#);
    editor.redo();
    editor.redo();
    assert_eq!(body(&editor), r#"<div class="">abc def</div>"#);
}

#[test]
fn undo_hands_back_marks_tied_to_a_change() {
    let mut editor = Editor::new();
    editor.type_text("cc ");
    editor.save_undo();
    editor.replace(
        exo_editor::Pos::new(0, 3),
        exo_editor::Pos::new(0, 3),
        "+Ali",
    );
    editor.mark_undo(7);
    assert_eq!(editor.undo(), vec![7]);
    assert_eq!(editor.redo(), vec![7]);
}
