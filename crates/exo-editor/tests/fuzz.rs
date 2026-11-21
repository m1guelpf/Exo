//! Random edits against the headless editor, checking what must always hold: nothing panics,
//! the document stays well formed, its HTML reads back as itself, undo then redo changes
//! nothing, and undoing every step gives back the HTML it started from. Deterministic: the same
//! seeds run every time.

use std::collections::HashMap;

use egui::{Key, Modifiers};
use exo_editor::{Color, Content, Document, Editor, FormatCommand, Piece, Pos, Selection, html};

/// xorshift64*: small, deterministic, good enough to pick edits.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

const TEXT: &[&str] = &[
    "a@b.co ",
    "a",
    "b",
    "word",
    " ",
    "  ",
    "x y",
    "e\u{301}",
    "\u{915}\u{93f}",
    "👍🏽",
    "👨\u{200d}👩\u{200d}👧",
    "🇺🇸",
    "🇯🇵🇫🇷",
    "1\u{fe0f}\u{20e3}",
    "日本語",
    "中",
    "한",
    "\u{1100}\u{1161}",
    ":",
    ":smi",
    ";",
    ";intro",
    "+al",
    "@jo",
    "-",
    "->",
    "--",
    "...",
    ">",
    "<-",
    "=>",
    "* ",
    "1. ",
    "> ",
    "---",
    "\n",
    "<&>\"'",
    "\u{a0}",
    "\t",
    "\u{200b}",
    "\u{feff}",
    "http://example.com",
    "www.x.org",
];

const HTML: &[&str] = &[
    "<b>bold</b> tail",
    "<ul><li>a</li><li>b</li></ul>",
    "<ol><li>one</li><ol><li>two</li><ol><li>three</li></ol></ol></ol>",
    "<blockquote><div>q1</div><blockquote><div>q2</div></blockquote></blockquote>",
    "<table><tr><td>t1</td><td>t2</td></tr></table>",
    "<div><hr></div>",
    "<hr>",
    "<p>p1</p><p>p2</p>",
    "<a href=\"javascript:alert(1)\" onclick=\"x()\">j</a>",
    "<img src=\"x\" onerror=\"alert(1)\">",
    "<span style=\"color:#a\u{e9}\u{e9}a\">c</span>",
    "<div><b>unclosed <i>both",
    "</div></div></blockquote>",
    "",
    "   ",
    "<div>\t tab\r\n</div>",
    "<pre>a\n b</pre>",
    "<div><br></div><div><br></div>",
    "x<br>y",
    "<div>loose<div>nested</div>after</div>",
    "<b><br></b>",
    "<script>alert(1)</script>s",
    "<u>e\u{301}</u>\u{1f600}",
    "<h1>head</h1>",
    "<li>bare item</li>",
    "<dl><dt>t</dt><dd>d</dd></dl>",
];

const PLAIN: &[&str] = &[
    "line1\nline2",
    "http://x.com/a",
    "\r\n",
    "a\tb",
    "",
    "\n\n",
    "one",
    "e\u{301}\u{301}",
    "a\rb",
    "  lead",
];

const LINKS: &[&str] = &[
    "",
    "example.com",
    "javascript:alert(1)",
    "data:text/html,x",
    "chrome://x",
    "a\"b<c>",
    "mailto:a@b.c",
    "  https://x.y  ",
];

const STARTS: &[&str] = &[
    "",
    "<div>hello world</div>",
    "<ul><li>a<br></li><li>b<br></li></ul><div><br></div>",
    "<blockquote style=\"x\"><div>q</div></blockquote><div>after</div>",
    "<div>one</div><div><hr></div><div>two</div><table><tr><td>t</td></tr></table>",
    "<div><div>wrapped<br></div></div><div class=\"gmail_quote\">On X wrote:<br><blockquote class=\"gmail_quote\"><div>quoted</div></blockquote></div>",
];

fn keys() -> Vec<(Key, Modifiers)> {
    let cmd = Modifiers {
        mac_cmd: true,
        command: true,
        ..Modifiers::NONE
    };
    let alt = Modifiers::ALT;
    let shift = Modifiers::SHIFT;
    let cmd_shift = Modifiers { shift: true, ..cmd };
    let alt_shift = Modifiers { shift: true, ..alt };
    let mut out = vec![
        (Key::Enter, Modifiers::NONE),
        (Key::Enter, shift),
        (Key::Backspace, Modifiers::NONE),
        (Key::Backspace, alt),
        (Key::Backspace, cmd),
        (Key::Delete, Modifiers::NONE),
        (Key::Delete, alt),
        (Key::A, cmd),
        (Key::Z, cmd),
        (Key::Z, cmd_shift),
        (Key::Y, cmd),
        (Key::Escape, Modifiers::NONE),
        (Key::Tab, Modifiers::NONE),
        (Key::Home, Modifiers::NONE),
        (Key::End, shift),
    ];
    for key in [
        Key::ArrowLeft,
        Key::ArrowRight,
        Key::ArrowUp,
        Key::ArrowDown,
    ] {
        for modifiers in [Modifiers::NONE, shift, alt, cmd, alt_shift, cmd_shift] {
            out.push((key, modifiers));
        }
    }
    // Backspace and arrows matter most: weight them up.
    for _ in 0..4 {
        out.push((Key::Backspace, Modifiers::NONE));
        out.push((Key::ArrowLeft, Modifiers::NONE));
        out.push((Key::Delete, Modifiers::NONE));
    }
    out
}

/// One random edit; returns what it was, for the failure message.
fn step(editor: &mut Editor, rng: &mut Rng, keys: &[(Key, Modifiers)]) -> String {
    match rng.below(20) {
        0..=5 => {
            let text = *rng.pick(TEXT);
            editor.type_text(text);
            format!("type {text:?}")
        }
        6..=9 => {
            let (key, modifiers) = *rng.pick(keys);
            editor.press(key, modifiers);
            format!("press {key:?} {modifiers:?}")
        }
        10 => {
            let command = *rng.pick(&FormatCommand::ALL);
            if command.applies(editor) {
                command.run(editor);
            }
            format!("command {command:?}")
        }
        11 | 12 => {
            let doc = editor.document();
            let mut pos = || {
                let block = rng.below(doc.blocks.len() + 1);
                let len = doc.blocks.get(block).map_or(0, |block| block.len());
                Pos::new(block, rng.below(len + 2))
            };
            let (anchor, focus) = (pos(), pos());
            let selection = if anchor.offset % 2 == 0 {
                Selection::caret(focus)
            } else {
                Selection::new(anchor, focus)
            };
            editor.set_selection(selection);
            format!("select {selection:?}")
        }
        13 => {
            let markup = *rng.pick(HTML);
            editor.paste_html(markup);
            format!("paste html {markup:?}")
        }
        14 => {
            let text = *rng.pick(PLAIN);
            editor.paste_text(text);
            format!("paste text {text:?}")
        }
        15 => {
            let link = *rng.pick(LINKS);
            editor.set_link(link);
            format!("link {link:?}")
        }
        16 => {
            if let Some(trigger) = editor.trigger().cloned() {
                editor.replace(trigger.start, trigger.end, "😀");
                return format!("resolve trigger {trigger:?}");
            }
            editor.remove_link();
            "remove link".to_owned()
        }
        17 => {
            let color = *rng.pick(&Color::ALL);
            editor.set_color(color);
            format!("color {color:?}")
        }
        18 => {
            let markup = *rng.pick(HTML);
            editor.insert_html(markup);
            format!("insert html {markup:?}")
        }
        _ => {
            editor.set_ghost_text(Some("ghost text".to_owned()));
            editor.accept_ghost_text();
            "accept ghost".to_owned()
        }
    }
}

/// What a document must always be.
fn check_well_formed(doc: &Document, selection: Selection) -> Result<(), String> {
    if doc.blocks.is_empty() {
        return Err("no blocks".into());
    }
    for pos in [selection.anchor, selection.focus] {
        if doc.clamp(pos) != pos {
            return Err(format!("selection {pos:?} outside the document"));
        }
    }
    // Every container is one element: the blocks in it are contiguous, at one depth, with one
    // element and the same containers around it.
    let mut seen: HashMap<u32, (usize, usize)> = HashMap::new();
    for (index, block) in doc.blocks.iter().enumerate() {
        for (depth, container) in block.path.iter().enumerate() {
            if let Some(&(last, at)) = seen.get(&container.id) {
                if last + 1 != index {
                    return Err(format!(
                        "container {} is split: blocks {last} and {index}",
                        container.id
                    ));
                }
                if at != depth || doc.blocks[last].path[..=depth] != block.path[..=depth] {
                    return Err(format!(
                        "container {} differs between blocks {last} and {index}",
                        container.id
                    ));
                }
            }
            seen.insert(container.id, (index, depth));
        }
        let loose = |block: &exo_editor::Block| block.element.tag.is_empty() && block.is_inline();
        if doc.blocks.get(index + 1).is_some_and(|next| {
            loose(block) && loose(next) && next.path == block.path && !block.trailing_break
        }) {
            return Err(format!(
                "loose line {index} runs into the next without a <br>"
            ));
        }
        if let Content::Inline(runs) = &block.content {
            // An empty line holds its placeholder `<br>`, or it has no height in a browser.
            if runs.last().is_none_or(|run| run.piece == Piece::Break) && !block.trailing_break {
                return Err(format!(
                    "block {index} ends in an empty line without its <br>"
                ));
            }
            for run in runs {
                if matches!(&run.piece, Piece::Text(text) if text.is_empty()) {
                    return Err(format!("empty text run in block {index}"));
                }
            }
        }
    }
    Ok(())
}

fn run_seed(seed: u64, steps: usize, trace: bool) {
    let keys = keys();
    let mut rng = Rng(seed);
    let start = *rng.pick(STARTS);
    let mut editor = if start.is_empty() {
        Editor::new()
    } else {
        Editor::from_html(start)
    };
    let initial = editor.html();
    let mut log: Vec<String> = Vec::new();
    for _ in 0..steps {
        let action = step(&mut editor, &mut rng, &keys);
        if trace {
            println!("{action}\n    {} {:?}", editor.html(), editor.selection());
        }
        log.push(action);
        let context = || {
            format!(
                "seed {seed}, start {start:?}, after:\n  {}\nhtml: {}",
                log.join("\n  "),
                editor.html()
            )
        };
        if let Err(error) = check_well_formed(editor.document(), editor.selection()) {
            panic!("{error}\n{}", context());
        }
        let body = editor.html();
        let again = html::write(&html::parse(&body));
        assert_eq!(
            again,
            body,
            "the body doesn't read back as itself\n{}",
            context()
        );
        // Undo then redo comes back to where it was: every change was recorded.
        if editor.can_undo() && rng.below(8) == 0 {
            editor.undo();
            editor.redo();
            assert_eq!(
                editor.html(),
                body,
                "undo then redo doesn't come back (seed {seed}):\n  {}",
                log.join("\n  ")
            );
        }
    }
    let mut undos = 0;
    while editor.can_undo() {
        editor.undo();
        undos += 1;
        assert!(undos <= steps * 4, "undo never ends (seed {seed})");
    }
    assert_eq!(
        editor.html(),
        initial,
        "undoing everything doesn't give back the start (seed {seed}):\n  {}",
        log.join("\n  ")
    );
}

/// 300 seeds of 120 edits by default; `FUZZ_SEEDS` and `FUZZ_STEPS` run more.
#[test]
fn random_edits_keep_the_document_sound() {
    let var = |name: &str, default: u64| {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default)
    };
    let steps = var("FUZZ_STEPS", 120) as usize;
    for seed in 1..=var("FUZZ_SEEDS", 300) {
        run_seed(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15), steps, false);
    }
}

/// Replays one seed, printing every step: `FUZZ_SEED=… cargo test --test fuzz trace -- --ignored --nocapture`.
#[test]
#[ignore = "a debugging aid"]
fn trace() {
    let seed = std::env::var("FUZZ_SEED")
        .expect("FUZZ_SEED")
        .parse()
        .expect("a number");
    let steps = std::env::var("FUZZ_STEPS")
        .ok()
        .and_then(|steps| steps.parse().ok())
        .unwrap_or(120);
    run_seed(seed, steps, true);
}
