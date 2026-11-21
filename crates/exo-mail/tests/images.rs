//! The Images setting in bodies: what a view asks the fetch hook for with remote content blocked
//! and not, the empty boxes blocked images become (sized as Chromium sizes broken ones), read
//! receipts, and Outlook's `Image removed by sender.` boxes.

use std::sync::{Arc, Mutex};

use exo_mail::{
    BodyView, Done, Fetch, Fonts, Message, Region, Render, Theme, ViewOptions, load_read_receipts,
    render,
};

/// Records what it's asked for and loads nothing.
#[derive(Default)]
struct Record(Mutex<Vec<String>>);

impl Fetch for Record {
    fn fetch(&self, url: &str, _done: Done) {
        self.0
            .lock()
            .expect("the record's lock")
            .push(url.to_owned());
    }
}

impl Record {
    fn urls(&self) -> Vec<String> {
        let mut urls = self.0.lock().expect("the record's lock").clone();
        urls.sort();
        urls
    }
}

fn fonts() -> Fonts {
    Fonts::load()
}

fn rendered(html: &str) -> Render {
    let message = Message {
        html: html.into(),
        from_email: "liam.oconnell@fastmail.com".into(),
        ..Message::default()
    };
    render(&message, None, &[])
}

/// Lays `html` out 400px wide at 2×, blocking remote content or not, and paints it on white;
/// returns the URLs asked for and the pixels.
fn show(html: &str, theme: Theme, block: bool) -> (Vec<String>, exo_mail::Image) {
    let record = Arc::new(Record::default());
    let options = ViewOptions {
        theme,
        block_remote_content: block,
        ..ViewOptions::default()
    };
    let mut view = BodyView::new(&rendered(html), options, &fonts(), Some(record.clone()));
    let height = view.layout(400.0, 2.0);
    let region = Region {
        top: 0.0,
        height,
        left: 0.0,
        width: 400.0,
        phase: [0.0; 2],
    };
    let image = view.paint(region, [255; 4]);
    (record.urls(), image)
}

/// The box of pixels that aren't white, in CSS px (`[left, top, width, height]`), and the
/// colors at its edge and in its middle.
fn ink_box(image: &exo_mail::Image) -> ([f32; 4], [u8; 3], [u8; 3]) {
    let pixel = |x: u32, y: u32| {
        let at = ((y * image.width + x) * 4) as usize;
        [image.rgba[at], image.rgba[at + 1], image.rgba[at + 2]]
    };
    let (mut min, mut max) = ([u32::MAX; 2], [0; 2]);
    for y in 0..image.height {
        for x in 0..image.width {
            if pixel(x, y) != [255; 3] {
                min = [min[0].min(x), min[1].min(y)];
                max = [max[0].max(x), max[1].max(y)];
            }
        }
    }
    assert!(min[0] <= max[0], "something was painted");
    let bounds = [min[0], min[1], max[0] - min[0] + 1, max[1] - min[1] + 1].map(|v| v as f32 / 2.0);
    let middle = pixel((min[0] + max[0]) / 2, (min[1] + max[1]) / 2);
    (bounds, pixel(min[0], (min[1] + max[1]) / 2), middle)
}

const REMOTE: &str = "<style>@import url(https://x.example/a.css);\
    @font-face { font-family: Brand; src: url(https://x.example/brand.woff) }\
    p { font-family: Brand }</style>\
    <p>Hi</p><img src=\"https://x.example/photo.png\" width=\"40\" height=\"40\">\
    <img src=\"cid:map@x\" width=\"20\" height=\"20\">\
    <img src=\"https://x.example/a.png\" srcset=\"https://x.example/a2.png 2x\" width=\"10\" height=\"10\">\
    <table background=\"https://x.example/table.png\"><tr>\
    <td style=\"background-image:url(https://x.example/cell.png)\">See below.</td></tr></table>";

#[test]
fn blocked_bodies_ask_only_for_inline_attachments() {
    let (blocked, _) = show(REMOTE, Theme::Light, true);
    assert_eq!(
        blocked,
        ["cid:map@x"],
        "no image, srcset, background, stylesheet or font"
    );
    let (loaded, _) = show(REMOTE, Theme::Light, false);
    assert!(
        loaded.len() > 4
            && loaded
                .iter()
                .all(|url| url.starts_with("https://x.example/") || url.starts_with("cid:")),
        "{loaded:?}"
    );
    assert!(rendered(REMOTE).has_remote_images);
    assert!(!rendered("<p>Hi</p><img src=\"cid:map@x\">").has_remote_images);
}

#[test]
fn blocked_images_are_empty_boxes_sized_like_broken_ones() {
    // White, with a 1px `rgba(0, 0, 0, .2)` border, on the white page.
    let boxed = |html: &str| ink_box(&show(html, Theme::Light, true).1);
    let (bounds, edge, middle) =
        boxed("<div><img src=\"https://x.example/a.png\" width=\"120\" height=\"60\"></div>");
    assert_eq!(
        [bounds[2], bounds[3]],
        [120.0, 60.0],
        "its dimensions, and their ratio under `height: auto`"
    );
    assert_eq!((edge, middle), ([204; 3], [255; 3]));
    // Alt text, or a missing dimension, makes a 16px square (Chromium's alt-text fallback).
    for html in [
        "<div><img src=\"https://x.example/a.png\" width=\"240\" height=\"160\" alt=\"Ridge\"></div>",
        "<div><img src=\"https://x.example/a.png\" alt=\"Dunes\"></div>",
        "<div><img src=\"https://x.example/a.png\" width=\"300\"></div>",
        "<div><img src=\"https://x.example/a.png\" width=\"50%\" height=\"80\"></div>",
    ] {
        let (bounds, ..) = boxed(html);
        assert_eq!([bounds[2], bounds[3]], [16.0, 16.0], "{html}");
    }
    // A block-level image keeps its size, alt text or not.
    let (bounds, ..) = boxed(
        "<img src=\"https://x.example/a.png\" width=\"160\" height=\"48\" alt=\"Badge\" style=\"display:block\">",
    );
    assert_eq!([bounds[2], bounds[3]], [160.0, 48.0]);
    // Wider than the column: `max-width: 100%`, keeping the ratio.
    let (bounds, ..) =
        boxed("<div><img src=\"https://x.example/a.png\" width=\"800\" height=\"200\"></div>");
    assert_eq!([bounds[2], bounds[3]], [400.0, 100.0]);
}

#[test]
fn carbon_marketing_mail_keeps_a_white_box_with_a_white_border() {
    // Classic Carbon's marketing mail (`.Theme-carbon:not(.Theme-dark).isMarketing`) whitens the
    // base rule's border; its personal mail keeps it.
    let marketing = "<table width=\"100%\"><tr><td><img src=\"https://x.example/a.png\" width=\"120\" height=\"60\"></td></tr></table>\
                     <p align=\"center\">Our winter sale ends on Sunday.</p>";
    assert!(rendered(marketing).unquoted_is_marketing);
    let border = |theme| {
        let (_, image) = show(marketing, theme, true);
        // The box's rows, above the link.
        image.rgba[..(image.width * 2 * 50) as usize * 4]
            .chunks(4)
            .any(|pixel| pixel[..3] == [204; 3])
    };
    assert!(border(Theme::Light), "Snow's grey border");
    assert!(!border(Theme::Dark), "Carbon's white one");
}

/// Carbon's personal mail keeps the box white on its dark page: the restyle that drops
/// near-black elements' backgrounds doesn't reach it (`remote-images-personal.eml`).
#[test]
fn carbon_personal_mail_keeps_the_white_box() {
    let html = "<div>The trail map <img src=\"https://x.example/a.png\" width=\"120\" height=\"60\"> is attached.</div>";
    assert!(!rendered(html).unquoted_is_marketing);
    let options = ViewOptions {
        theme: Theme::Dark,
        block_remote_content: true,
        ..ViewOptions::default()
    };
    let mut view = BodyView::new(&rendered(html), options, &fonts(), None);
    let height = view.layout(400.0, 2.0);
    let region = Region {
        top: 0.0,
        height,
        left: 0.0,
        width: 400.0,
        phase: [0.0; 2],
    };
    let image = view.paint(region, [40, 42, 46, 255]);
    let white = image
        .rgba
        .chunks(4)
        .filter(|pixel| pixel[..3] == [255; 3])
        .count();
    assert!(white > 2 * 110 * 2 * 50, "{white} white pixels");
}

#[test]
fn read_receipts_load_only_when_asked_and_never_from_off_limits_hosts() {
    let html = "<p>Hi</p><img src=\"https://t.yesware.com/t/1/open.gif\" width=\"1\" height=\"1\">\
                <img src=\"https://r.superhuman.com/x.gif\">";
    let render = rendered(html);
    assert_eq!(render.read_status_urls.len(), 2);
    let (shown, _) = show(html, Theme::Light, false);
    assert_eq!(
        shown,
        Vec::<String>::new(),
        "the body itself never loads them"
    );
    let record = Record::default();
    load_read_receipts(&render, &record);
    assert_eq!(record.urls(), ["https://t.yesware.com/t/1/open.gif"]);
}

#[test]
fn image_removed_by_sender_boxes_are_hidden() {
    for alt in [
        "Image removed by sender.",
        "Das Bild wurde vom Absender entfernt",
    ] {
        let html = format!(
            "<p>Before <span style=\"border:1px solid #f00\"> <img alt=\"{alt}\"> </span>after</p>"
        );
        let (_, image) = show(&html, Theme::Light, false);
        assert!(
            !image
                .rgba
                .chunks(4)
                .any(|pixel| pixel[0] > 200 && pixel[1] < 60),
            "{alt}: no red border"
        );
    }
    let (_, image) = show(
        "<p>Before <span style=\"border:1px solid #f00\"><img alt=\"A photo\"></span>after</p>",
        Theme::Light,
        false,
    );
    assert!(
        image
            .rgba
            .chunks(4)
            .any(|pixel| pixel[0] > 200 && pixel[1] < 60),
        "other placeholders stay"
    );
}
