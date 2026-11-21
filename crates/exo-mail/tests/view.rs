//! Laying out, painting, hit-testing and selecting a body.

use std::sync::{Arc, Mutex};

use exo_mail::{BodyView, Fonts, Granularity, Hit, Message, Region, Render, ViewOptions, render};

fn fonts() -> Fonts {
    Fonts::load()
}

fn rendered(html: &str) -> Render {
    let message = Message {
        html: html.into(),
        from_email: "ann@example.com".into(),
        from_name: "Ann Lee".into(),
        ..Message::default()
    };
    render(&message, None, &[])
}

#[test]
fn lays_personal_mail_out_on_the_24px_grid() {
    let body = rendered("<div>One line.</div><div>Another line.</div>");
    let mut view = BodyView::new(&body, ViewOptions::default(), &fonts(), None);
    assert_eq!(view.layout(640.0, 2.0), 48.0);
    // Narrower, the same text wraps onto more lines.
    let long = rendered(&"word ".repeat(40));
    let mut view = BodyView::new(&long, ViewOptions::default(), &fonts(), None);
    let wide = view.layout(640.0, 2.0);
    let narrow = view.layout(200.0, 2.0);
    assert!(
        narrow > wide && wide % 24.0 == 0.0 && narrow % 24.0 == 0.0,
        "{wide} → {narrow}"
    );
}

#[test]
fn hits_links_and_the_quote_toggle() {
    let body = rendered(
        "<div>Read <a href=\"https://example.com/a\">the doc</a> first.</div>\
         <div class=\"gmail_signature\">Ann Lee</div>",
    );
    assert!(body.has_quoted_content, "the signature hides behind the …");
    let mut view = BodyView::new(&body, ViewOptions::default(), &fonts(), None);
    view.layout(640.0, 2.0);
    assert_eq!(
        view.hit(60.0, 12.0),
        Some(Hit::Link {
            href: "https://example.com/a".into()
        })
    );
    assert_eq!(view.hit(5.0, 12.0), Some(Hit::Text));
    assert_eq!(view.hit(8.0, 32.0), Some(Hit::QuoteToggle));
}

#[test]
fn tells_text_link_boxes_and_nothing_apart() {
    let link = |href: &str| Some(Hit::Link { href: href.into() });
    let body = rendered(
        "<div>Short line.</div>\
         <p><a href=\"https://example.com/button\" style=\"display:inline-block;padding:12px 24px\">Go</a></p>\
         <table><tr><td style=\"padding:12px 24px\"><a href=\"https://example.com/cell\">Pay</a></td></tr></table>",
    );
    let mut view = BodyView::new(&body, ViewOptions::default(), &fonts(), None);
    view.layout(640.0, 2.0);
    // The I-beam over text, the arrow past its end (Chromium's `cursor: auto`).
    assert_eq!(view.hit(10.0, 12.0), Some(Hit::Text));
    assert_eq!(view.hit(300.0, 12.0), None);
    // A padded link is a link all over its box; a padded cell around a link isn't.
    assert_eq!(
        view.hit(4.0, 30.0),
        link("https://example.com/button"),
        "in the button's padding"
    );
    assert_eq!(
        view.hit(30.0, 48.0),
        link("https://example.com/button"),
        "on its text"
    );
    let cell = 24.0 + 24.0 + 48.0 + 12.0;
    assert_eq!(
        view.hit(30.0, cell),
        link("https://example.com/cell"),
        "on the link in the cell"
    );
    assert_eq!(view.hit(4.0, cell), None, "in the cell's padding");
    // An `&nbsp;` spacer cell is text only on its line (centred in the row), not down the cell.
    let spacer = rendered(
        "<table><tr><td width=\"24\">&nbsp;</td><td><div style=\"height:100px\">Tall</div></td></tr></table>",
    );
    let mut view = BodyView::new(&spacer, ViewOptions::default(), &fonts(), None);
    view.layout(640.0, 2.0);
    assert_eq!(view.hit(4.0, 50.0), Some(Hit::Text));
    assert_eq!(view.hit(4.0, 5.0), None);
    assert_eq!(view.hit(4.0, 95.0), None);
}

#[test]
fn selects_text() {
    let body = rendered("<div>Select these words please.</div>");
    let mut view = BodyView::new(&body, ViewOptions::default(), &fonts(), None);
    view.layout(640.0, 2.0);
    assert!(view.select_from(1.0, 12.0, Granularity::Character));
    assert!(view.select_to(400.0, 12.0));
    assert_eq!(
        view.selected_text().as_deref(),
        Some("Select these words please.")
    );
    view.clear_selection();
    assert_eq!(view.selected_text(), None);
}

/// A double click selects a word between Unicode word boundaries, as Chromium's ICU does (an
/// apostrophe or a decimal point joins, a hyphen or a comma doesn't, a space is a word of its own),
/// and a triple click the paragraph with its line break. Both start at the caret the click puts
/// down, so the right half of a word's last letter takes the space after it. On this body that's
/// `Don't`, `mail`, `,`, `3.14`, the space after `mail`, and `"Don't e-mail Bob, 3.14 now.\n"`.
#[test]
fn double_and_triple_clicks_select_words_and_paragraphs() {
    let body =
        rendered("<div>Don't e-mail Bob, 3.14 now.<br>Second line.</div><div>Next block</div>");
    let mut view = BodyView::new(&body, ViewOptions::default(), &fonts(), None);
    view.layout(640.0, 2.0);
    let mut words: Vec<String> = Vec::new();
    for x in 0..300 {
        assert!(view.select_from(x as f32, 12.0, Granularity::Word));
        let word = view.selected_text().unwrap_or_default();
        if words.last() != Some(&word) {
            words.push(word);
        }
    }
    let expected = [
        "Don't", " ", "e", "-", "mail", " ", "Bob", ",", " ", "3.14", " ", "now", ".", "\n",
    ];
    assert_eq!(words, expected);

    assert!(view.select_from(40.0, 12.0, Granularity::Paragraph));
    assert_eq!(
        view.selected_text().as_deref(),
        Some("Don't e-mail Bob, 3.14 now.\n")
    );
    assert!(view.select_from(40.0, 36.0, Granularity::Paragraph));
    assert_eq!(view.selected_text().as_deref(), Some("Second line.\n"));
    assert!(view.select_from(40.0, 60.0, Granularity::Paragraph));
    assert_eq!(view.selected_text().as_deref(), Some("Next block"));
    // Past the end of the last block there's nothing to take.
    assert!(view.select_from(400.0, 60.0, Granularity::Word));
    assert_eq!(view.selected_text(), None);
}

#[test]
fn paints_tall_bodies_in_bands() {
    // Marketing mail keeps its colors; 6000 CSS px is three bands at 2×.
    let body = rendered("<div align=\"center\" style=\"height:6000px;background:#00ff00\">x</div>");
    assert!(body.unquoted_is_marketing);
    let mut view = BodyView::new(&body, ViewOptions::default(), &fonts(), None);
    assert_eq!(view.layout(100.0, 2.0), 6000.0);
    let image = view.paint(
        Region {
            top: 0.0,
            height: 6000.0,
            left: 0.0,
            width: view.width(),
            phase: [0.0; 2],
        },
        [255; 4],
    );
    assert_eq!((image.width, image.height), (200, 12000));
    let pixel = |x: usize, y: usize| &image.rgba[(y * image.width as usize + x) * 4..][..4];
    for y in [0, 8191, 8192, 11999] {
        assert_eq!(pixel(150, y), [0, 255, 0, 255], "row {y}");
    }
}

/// Holds requests until the test answers them, as a network would.
#[derive(Default)]
struct Deferred(Mutex<Vec<(String, exo_mail::Done)>>);

impl exo_mail::Fetch for Deferred {
    fn fetch(&self, url: &str, done: exo_mail::Done) {
        self.0.lock().unwrap().push((url.to_owned(), done));
    }
}

#[test]
fn loads_images_through_the_fetch_hook() {
    let body = rendered(
        "<div><img src=\"cid:logo@x\" width=\"20\" height=\"20\"> \
         <img src=\"https://t.yesware.com/t/1/open.gif\" width=\"1\" height=\"1\"></div>",
    );
    assert_eq!(
        body.read_status_urls,
        ["https://t.yesware.com/t/1/open.gif"]
    );
    let fetch = Arc::new(Deferred::default());
    let mut view = BodyView::new(&body, ViewOptions::default(), &fonts(), Some(fetch.clone()));
    view.layout(100.0, 2.0);
    assert!(!view.poll(), "nothing has arrived yet");

    // The tracking pixel became a blank `data:` GIF, which never reaches the hook.
    let (url, done) = fetch.0.lock().unwrap().pop().expect("one request");
    assert_eq!(url, "cid:logo@x");
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    std::thread::spawn(move || done.finish(png)).join().unwrap();
    assert!(view.poll());
    let image = view.paint(
        Region {
            top: 0.0,
            height: 24.0,
            left: 0.0,
            width: view.width(),
            phase: [0.0; 2],
        },
        [255; 4],
    );
    let pixel = &image.rgba[(20 * image.width as usize + 10) * 4..][..4];
    assert_eq!(pixel, [255, 0, 0, 255]);
}

#[test]
fn shrinks_slightly_wide_mail_to_fit() {
    // 700px of table in a 640px column: it's zoomed to 640/700.
    let body = rendered(
        "<table width=\"700\" align=\"center\" style=\"height:70px\"><tr><td>x</td></tr></table>",
    );
    let mut view = BodyView::new(&body, ViewOptions::default(), &fonts(), None);
    let height = view.layout(640.0, 2.0);
    assert!((height - 64.0).abs() < 0.5, "{height}");
    assert!((view.content_width() - 640.0).abs() < 0.5);
    // Much wider bodies keep their size and scroll.
    let wide = rendered("<table width=\"1000\" style=\"height:70px\"><tr><td>x</td></tr></table>");
    let mut view = BodyView::new(&wide, ViewOptions::default(), &fonts(), None);
    view.layout(640.0, 2.0);
    assert!(view.content_width() >= 1000.0);
}

#[test]
fn fetches_only_web_and_attachment_urls() {
    let body = rendered(
        "<div style=\"background:url(file:///etc/hosts)\">x</div>\
         <img srcset=\"file:///etc/passwd 1x\"><img src=\"cid:logo@x\">",
    );
    let fetch = Arc::new(Deferred::default());
    let mut view = BodyView::new(&body, ViewOptions::default(), &fonts(), Some(fetch.clone()));
    view.layout(100.0, 2.0);
    let urls: Vec<String> = fetch
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|(url, _)| url.clone())
        .collect();
    assert_eq!(urls, ["cid:logo@x"]);
}

#[test]
fn too_wide_mail_scrolls_sideways_with_a_scrollbar_below_it() {
    // A sale mail's 1200px table in a 640px column: wider than what gets shrunk.
    let wide = rendered(
        "<table width=\"1200\" cellpadding=\"0\" cellspacing=\"0\" style=\"height:70px\"><tr>\
         <td width=\"1000\">x</td><td width=\"200\" style=\"background:#0000ff\"></td></tr></table>",
    );
    let mut view = BodyView::new(&wide, ViewOptions::default(), &fonts(), None);
    let height = view.layout(640.0, 2.0);
    assert!(view.scrolls());
    assert_eq!(view.content_width(), 1200.0);
    assert_eq!(height, 70.0 + exo_mail::SCROLLBAR_HEIGHT);
    // What scrolls into view is painted, not clipped at the column's edge.
    let image = view.paint(
        Region {
            top: 0.0,
            height: 70.0,
            left: 1000.0,
            width: 200.0,
            phase: [0.0; 2],
        },
        [255; 4],
    );
    assert_eq!(image.width, 400);
    let pixel = &image.rgba[(70 * image.width as usize + 200) * 4..][..4];
    assert_eq!(pixel, [0, 0, 255, 255]);

    let narrow = rendered("<table width=\"600\" style=\"height:70px\"><tr><td>x</td></tr></table>");
    let mut view = BodyView::new(&narrow, ViewOptions::default(), &fonts(), None);
    assert_eq!(view.layout(640.0, 2.0), 70.0, "no scrollbar when it fits");
    assert!(!view.scrolls());
}

/// A GIF of solid 2×2 frames, each shown `delay_ms`, looping forever or, without a loop count,
/// playing once.
fn gif(colors: &[[u8; 4]], delay_ms: u32, looping: bool) -> String {
    use base64::Engine as _;
    use image::{Delay, Frame, RgbaImage, codecs::gif::GifEncoder};
    let mut bytes = Vec::new();
    {
        let mut encoder = GifEncoder::new(&mut bytes);
        if looping {
            encoder
                .set_repeat(image::codecs::gif::Repeat::Infinite)
                .unwrap();
        }
        let frames = colors.iter().map(|color| {
            Frame::from_parts(
                RgbaImage::from_pixel(2, 2, image::Rgba(*color)),
                0,
                0,
                Delay::from_numer_denom_ms(delay_ms, 1),
            )
        });
        encoder.encode_frames(frames).unwrap();
    }
    format!(
        "data:image/gif;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// The color at the middle of `view`'s 20×20 image at the top left.
fn image_color(view: &mut BodyView) -> [u8; 4] {
    let image = view.paint(
        Region {
            top: 0.0,
            height: 20.0,
            left: 0.0,
            width: 20.0,
            phase: [0.0; 2],
        },
        [255; 4],
    );
    image.rgba[(20 * image.width as usize + 20) * 4..][..4]
        .try_into()
        .unwrap()
}

#[test]
fn plays_animated_gifs_frame_by_frame() {
    use std::time::Duration;
    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    let ms = Duration::from_millis;
    let body = rendered(&format!(
        "<img src=\"{}\" style=\"display:block;width:20px;height:20px\">",
        gif(&[RED, BLUE], 50, true)
    ));
    let mut view = BodyView::new(
        &body,
        ViewOptions::default(),
        &fonts(),
        Some(Arc::new(Deferred::default())),
    );
    view.layout(100.0, 2.0);
    assert_eq!(view.animations(), [[0.0, 0.0, 20.0, 20.0]]);
    assert_eq!(image_color(&mut view), RED);
    // The clock starts when the image first shows; nothing moves off screen.
    let start = view.animate(ms(1000), 0.0..100.0);
    assert!(start.changed.is_empty());
    assert_eq!(start.next, Some(ms(1050)));
    assert!(
        view.animate(ms(1100), 200.0..300.0).changed.is_empty(),
        "off screen"
    );
    assert!(view.animate(ms(1049), 0.0..100.0).changed.is_empty());
    let second = view.animate(ms(1050), 0.0..100.0);
    assert_eq!(second.changed, [[0.0, 0.0, 20.0, 20.0]]);
    assert_eq!(image_color(&mut view), BLUE);
    assert!(
        !view.animate(ms(1100), 0.0..100.0).changed.is_empty(),
        "it loops"
    );
    assert_eq!(image_color(&mut view), RED);

    // Without a loop count a GIF plays once, and frames asking for 10 ms show for 100 ms.
    let once = rendered(&format!(
        "<img src=\"{}\" style=\"display:block;width:20px;height:20px\">",
        gif(&[RED, BLUE], 10, false)
    ));
    let mut view = BodyView::new(
        &once,
        ViewOptions::default(),
        &fonts(),
        Some(Arc::new(Deferred::default())),
    );
    view.layout(100.0, 2.0);
    assert_eq!(view.animate(ms(0), 0.0..100.0).next, Some(ms(100)));
    let last = view.animate(ms(100), 0.0..100.0);
    assert_eq!(image_color(&mut view), BLUE);
    assert_eq!(last.next, Some(ms(200)));
    let over = view.animate(ms(200), 0.0..100.0);
    assert!(over.changed.is_empty() && over.next.is_none(), "{over:?}");
    assert_eq!(image_color(&mut view), BLUE);
}

#[test]
fn finds_where_an_in_message_link_points() {
    // Three 24px lines, then an empty `<a name>` at the start of the fourth, and an `id` below.
    let body = rendered(
        "<div>Jump to <a href=\"#totals\">the totals</a>.</div><div>One.</div><div>Two.</div>\
         <div><a name=\"totals\"></a>Totals</div><div id=\"notes\">Notes</div>",
    );
    let mut view = BodyView::new(&body, ViewOptions::default(), &fonts(), None);
    view.layout(640.0, 2.0);
    assert_eq!(
        view.hit(80.0, 12.0),
        Some(Hit::Link {
            href: "#totals".into()
        })
    );
    let totals = view.anchor_top("totals").expect("the empty anchor");
    assert!(
        (75.0..77.0).contains(&totals),
        "the fourth line's text, 3px into its 24px line: {totals}"
    );
    let notes = view.anchor_top("notes").expect("the id");
    assert!((96.0..98.0).contains(&notes), "{notes}");
    assert_eq!(view.anchor_top("nowhere"), None);
}

#[test]
fn bounds_the_selection() {
    let body = rendered("<div>One line of text.</div><div>And another one under it.</div>");
    let mut view = BodyView::new(&body, ViewOptions::default(), &fonts(), None);
    view.layout(640.0, 2.0);
    assert_eq!(view.selection_bounds(), None);
    view.select_from(5.0, 12.0, Granularity::Character);
    view.select_to(40.0, 36.0);
    let [left, top, width, height] = view.selection_bounds().expect("selected");
    // The text's content area on both lines, inside their 24px line boxes.
    assert!(
        left < 5.0 && (2.0..6.0).contains(&top) && (36.0..46.0).contains(&height) && width > 40.0,
        "{left} {top} {width} {height}"
    );
}

/// Drags across a body of blocks, paragraphs, a list and a table, in window coordinates with the
/// body's origin at (364.625, 161): a drag after a double or triple click extends by words or
/// paragraphs, keeping the one clicked, and the text has a newline between blocks, a blank line
/// after a paragraph, and tabs between table cells.
#[test]
fn drags_extend_by_the_clicks_granularity_and_copy_blocks_as_lines() {
    let body = rendered(
        "<div>First block with several words to select by dragging across them.</div>\
         <div>Second block, also a line of its own.</div>\
         <p>A paragraph element with its margins.</p><p>Another paragraph element after it.</p>\
         <ul><li>One item</li><li>Two items</li></ul>\
         <table><tr><td>Cell A</td><td>Cell B</td></tr><tr><td>Cell C</td><td>Cell D</td></tr></table>\
         <div>Line one<br>Line two after a break</div><div>The end.</div>",
    );
    let mut view = BodyView::new(&body, ViewOptions::default(), &fonts(), None);
    view.layout(640.0, 2.0);
    let mut drag = |granularity, from: (f32, f32), to: (f32, f32)| {
        let local = |(x, y): (f32, f32)| (x - 364.625, y - 161.0);
        let (from, to) = (local(from), local(to));
        view.select_from(from.0, from.1, granularity);
        view.select_to(to.0, to.1);
        view.selected_text().unwrap_or_default()
    };
    use Granularity::{Character, Paragraph, Word};
    let words = "several words to select by dragging across them.\nSecond block,";
    assert_eq!(drag(Word, (500.0, 172.0), (450.0, 196.0)), words);
    assert_eq!(drag(Word, (450.0, 196.0), (500.0, 172.0)), words);
    assert_eq!(drag(Word, (500.0, 172.0), (520.0, 172.0)), "several words");
    assert_eq!(
        drag(Paragraph, (400.0, 220.0), (420.0, 300.0)),
        "A paragraph element with its margins.\n\nAnother paragraph element after it.\n\nOne item\n"
    );
    assert_eq!(
        drag(Paragraph, (400.0, 220.0), (400.0, 220.0)),
        "A paragraph element with its margins.\n\n"
    );
    assert_eq!(
        drag(Paragraph, (400.0, 446.0), (400.0, 446.0)),
        "Line two after a break\n"
    );
    assert_eq!(
        drag(Character, (370.0, 172.0), (440.0, 446.0)),
        "irst block with several words to select by dragging across them.\nSecond block, also a line of its own.\n\
         A paragraph element with its margins.\n\nAnother paragraph element after it.\n\nOne item\nTwo items\n\
         Cell A\tCell B\nCell C\tCell D\nLine one\nLine two aft"
    );
    assert_eq!(
        drag(Character, (370.0, 367.0), (440.0, 395.0)),
        "Cell A\tCell B\nCell C\tCell "
    );
}

/// Link text breaks at the `<wbr>`s the pipeline puts after `.`, `@` and `/`, as Chromium lays them
/// out (a zero width space): this body's first line shows as `Questions go to atencionalcliente@` /
/// `parquewarner.com at any time.` The break isn't text: copying and find-in-page don't see it.
#[test]
fn link_text_breaks_at_word_break_opportunities() {
    let body = rendered(
        "<div style=\"width: 300px\">Questions go to <a href=\"mailto:atencionalcliente@parquewarner.com\">\
         atencionalcliente@parquewarner.com</a> at any time.</div>",
    );
    let mut view = BodyView::new(&body, ViewOptions::default(), &fonts(), None);
    assert_eq!(view.layout(640.0, 2.0), 48.0, "two lines");
    assert!(view.select_from(0.0, 12.0, Granularity::Character));
    assert!(view.select_to(299.0, 36.0));
    assert_eq!(
        view.selected_text().as_deref(),
        Some("Questions go to atencionalcliente@parquewarner.com at any time.")
    );
    let found = view.find(|text| {
        text.match_indices("parquewarner.com")
            .map(|(at, word)| at..at + word.len())
            .collect()
    });
    assert_eq!(found.len(), 1);
}

/// Paints the body's first `height` CSS px, 640px wide at 2×, on white.
fn paint_top(view: &mut BodyView, height: f32) -> exo_mail::Image {
    view.paint(
        Region {
            top: 0.0,
            height,
            left: 0.0,
            width: 640.0,
            phase: [0.0; 2],
        },
        [255; 4],
    )
}

/// The rightmost device pixel of row `y` that `matches`, if any.
fn rightmost(image: &exo_mail::Image, y: u32, matches: impl Fn(&[u8]) -> bool) -> Option<u32> {
    let row = &image.rgba[(y * image.width * 4) as usize..][..(image.width * 4) as usize];
    row.chunks(4).rposition(matches).map(|x| x as u32)
}

/// Chromium paints a selected line break as a box a space wide after the line's text where the
/// selection runs on past it into the next block, and not where the selection ends
/// (`body-newline.eml`). The selection is Classic's: macOS's highlight in Snow, rgba(106, 134,
/// 173, .996) in Carbon.
#[test]
fn a_selection_paints_the_line_breaks_it_runs_past() {
    let select = |theme| {
        let body = rendered("<div>First block</div><div>Second block</div>");
        let options = ViewOptions {
            theme,
            ..ViewOptions::default()
        };
        let mut view = BodyView::new(&body, options, &fonts(), None);
        view.layout(640.0, 2.0);
        assert!(view.select_from(0.0, 12.0, Granularity::Character));
        assert!(view.select_to(600.0, 36.0));
        paint_top(&mut view, 48.0)
    };
    let image = select(exo_mail::Theme::Light);
    let highlight = |pixel: &[u8]| pixel[..3] == [0xb4, 0xd5, 0xfe];
    let ink = |pixel: &[u8]| pixel[0] < 128;
    let (first, second) = ((2..46).step_by(4), (50..94).step_by(4));
    let end = |rows: std::iter::StepBy<std::ops::Range<u32>>, matches: &dyn Fn(&[u8]) -> bool| {
        rows.filter_map(|y| rightmost(&image, y, matches))
            .max()
            .expect("painted")
    };
    // A space is 3.6px at 14px: 7 device pixels past the first block's text (whose last glyph's
    // side bearing adds one or two), none past the end.
    let past = |rows: std::iter::StepBy<std::ops::Range<u32>>| {
        end(rows.clone(), &highlight) - end(rows, &ink)
    };
    let (first, second) = (past(first), past(second));
    assert!(
        (7..=11).contains(&first) && second <= 3,
        "{first}, {second}"
    );
    let image = select(exo_mail::Theme::Dark);
    let carbon = |pixel: &[u8]| {
        pixel[..3]
            .iter()
            .zip([106, 134, 173])
            .all(|(&c, e)| c.abs_diff(e) <= 2)
    };
    assert!(image.rgba.chunks(4).any(carbon), "Carbon's selection");
}

/// Underlines skip the ink of the glyphs that cross them, as Chromium's `text-decoration-skip-ink:
/// auto` does (`body-skip-ink.eml`): under a run of `g`s, the line's row is cut around each
/// descender.
#[test]
fn underlines_skip_descenders() {
    let underline = |text: &str| {
        let mut view = BodyView::new(
            &rendered(&format!("<div><u>{text}</u></div>")),
            ViewOptions::default(),
            &fonts(),
            None,
        );
        view.layout(640.0, 2.0);
        let image = paint_top(&mut view, 24.0);
        let dark = |y: u32| {
            let row = &image.rgba[(y * image.width * 4) as usize..][..(image.width * 4) as usize];
            row.chunks(4).filter(|pixel| pixel[0] < 128).count()
        };
        (0..image.height).map(dark).collect::<Vec<_>>()
    };
    let plain = underline("xxxxxxxx");
    let (row, &full) = plain
        .iter()
        .enumerate()
        .max_by_key(|(_, count)| **count)
        .expect("rows");
    let skipped = underline("gggggggg")[row];
    assert!(
        skipped > 0 && skipped < full - full / 5,
        "{skipped} of {full} pixels in the underline's row"
    );
}
