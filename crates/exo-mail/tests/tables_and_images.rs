//! Table and image layouts from real marketing mail that Blitz used to get wrong, reduced to
//! their essentials. Expected sizes are Chromium's.

use std::{io::Cursor, sync::Arc};

use base64::Engine as _;
use exo_mail::{BodyView, Done, Fetch, Fonts, Message, Region, ViewOptions, render};

fn view(html: &str) -> BodyView {
    let fonts = Fonts::load();
    let message = Message {
        html: html.into(),
        from_email: "news@example.com".into(),
        ..Message::default()
    };
    BodyView::new(
        &render(&message, None, &[]),
        ViewOptions::default(),
        &fonts,
        Some(Arc::new(Offline)),
    )
}

/// Loads nothing: the bodies here only use `data:` images, which the view decodes itself.
struct Offline;

impl Fetch for Offline {
    fn fetch(&self, _url: &str, _done: Done) {}
}

/// A PNG of `pixels` (RGBA rows, top to bottom) as a `data:` URL.
fn png(width: u32, height: u32, pixels: Vec<u8>) -> String {
    let image = image::RgbaImage::from_raw(width, height, pixels).expect("width × height pixels");
    let mut bytes = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .expect("PNG encodes");
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// A footer whose cells say `height: 1px`: each row grows to fit its content instead of the
/// rows' text being drawn on top of each other.
#[test]
fn table_cells_grow_to_fit_content_taller_than_their_height() {
    let cell = |padding: &str, content: u32| {
        format!(
            "<tr><td style=\"padding:{padding};height:1px\"><div style=\"height:{content}px\"></div></td></tr>"
        )
    };
    let mut view = view(&format!(
        "<table cellpadding=\"0\" cellspacing=\"0\" width=\"600\">{}{}{}</table>",
        cell("17px 0 9px", 32),
        cell("0 0 9px", 16),
        cell("0 0 56px", 16),
    ));
    assert_eq!(view.layout(640.0, 2.0), 58.0 + 25.0 + 72.0);
}

/// An image cell that asks for more width than the message has (`<td width="1280">`): the
/// column shrinks to the table's width and the image fits it, rather than the table overflowing
/// and the image being cut off.
#[test]
fn wide_cells_shrink_to_fit_the_table() {
    let image = png(1100, 619, [80, 120, 200, 255].repeat(1100 * 619));
    let mut view = view(&format!(
        "<table width=\"100%\" cellpadding=\"0\" cellspacing=\"0\"><tr><td width=\"1280\">\
         <img src=\"{image}\" width=\"550\" height=\"309\" \
         style=\"display:block;height:auto;width:auto !important;max-width:100% !important\">\
         </td></tr></table>"
    ));
    let height = view.layout(640.0, 2.0);
    assert_eq!(view.content_width(), 640.0);
    assert!((height - 640.0 * 619.0 / 1100.0).abs() < 1.0, "{height}");
}

/// A scaled-up image's edges don't blend in its opposite edge (a comic whose black bottom border
/// showed as a grey line along its top).
#[test]
fn image_edges_dont_wrap_around() {
    let white_over_black = png(1, 2, [[255, 255, 255, 255], [0, 0, 0, 255]].concat());
    let mut view = view(&format!(
        "<img src=\"{white_over_black}\" style=\"display:block;width:10px;height:20px\">"
    ));
    view.layout(100.0, 2.0);
    let image = view.paint(
        Region {
            top: 0.0,
            height: 20.0,
            left: 0.0,
            width: 100.0,
            phase: [0.0; 2],
        },
        [255; 4],
    );
    let top_row = &image.rgba[10 * 4..][..4];
    assert_eq!(top_row, [255, 255, 255, 255]);
}

/// A receipt whose content sits in a `<td style="display: block">` (Parque Warner's): the cell
/// is wrapped in an anonymous cell, as in Chromium, instead of the whole message vanishing.
/// Consecutive block cells share one anonymous cell, so they stack.
#[test]
fn block_cells_are_wrapped_in_an_anonymous_cell() {
    let mut view = view(
        "<table cellpadding=\"0\" cellspacing=\"0\" width=\"100%\"><tr>\
         <td style=\"display:block;padding:20px\"><div style=\"height:100px\"></div></td>\
         </tr><tr>\
         <td style=\"display:block\"><div style=\"height:30px\"></div></td>\
         <td style=\"display:block\"><div style=\"height:50px\"></div></td>\
         </tr></table>",
    );
    assert_eq!(view.layout(640.0, 2.0), 140.0 + 30.0 + 50.0);
}

/// A `<table style="display: block">` (Kickstarter's): its rows go in an anonymous table, so
/// they stack instead of being laid out side by side.
#[test]
fn rows_of_a_block_table_stay_rows() {
    let mut view = view(
        "<table cellpadding=\"0\" cellspacing=\"0\" width=\"100%\" style=\"display:block\">\
         <tr><td style=\"display:block\"><div style=\"height:30px\"></div></td></tr>\
         <tr><td style=\"display:block\"><div style=\"height:50px\"></div></td></tr>\
         </table>",
    );
    assert_eq!(view.layout(640.0, 2.0), 30.0 + 50.0);
}

/// Images sized by their height alone keep it when `min-width` widens them (Kickstarter's
/// 1px spacer was 324px tall) and aren't given `height: auto` by the restyle (its 39px step
/// icons were 120px tall).
#[test]
fn images_keep_a_specified_height() {
    let image = png(200, 120, [80, 120, 200, 255].repeat(200 * 120));
    let mut spacer = view(&format!(
        "<img src=\"{image}\" height=\"1\" style=\"display:block;min-width:540px;height:1px\">"
    ));
    assert_eq!(spacer.layout(640.0, 2.0), 1.0);
    let mut icon = view(&format!(
        "<img src=\"{image}\" style=\"display:block;min-width:39px;height:39px;max-width:100%\">"
    ));
    assert_eq!(icon.layout(640.0, 2.0), 39.0);
}

/// The first device row with dark ink in `view`'s top `height` CSS px, in CSS px.
fn first_ink(view: &mut BodyView, height: f32) -> Option<f32> {
    let image = view.paint(
        Region {
            top: 0.0,
            height,
            left: 0.0,
            width: 300.0,
            phase: [0.0; 2],
        },
        [255; 4],
    );
    let dark =
        |y: u32| (0..image.width).any(|x| image.rgba[((y * image.width + x) * 4) as usize] < 100);
    (0..image.height).find(|&y| dark(y)).map(|y| y as f32 / 2.0)
}

/// A cell with its own `height` keeps its content in the middle, as `vertical-align: middle`
/// does in Chromium (`body-tables.eml`): its text was laid out at the top whenever the body was
/// laid out again, since the cell is measured again outside the layout cache.
#[test]
fn a_cell_with_a_height_centres_its_content() {
    let mut view = view(
        "<table width=\"500\" cellpadding=\"0\" cellspacing=\"0\" style=\"font-family: Arial; font-size: 14px\">\
         <tr><td height=\"60\">Centred</td></tr></table>",
    );
    view.layout(640.0, 2.0);
    view.layout(600.0, 2.0);
    view.layout(640.0, 2.0);
    // The 24px line box is 18px down; the text's ink starts a few px into it.
    let ink = first_ink(&mut view, 60.0).expect("text");
    assert!((20.0..26.0).contains(&ink), "{ink}");
}

/// A line that ends in a `<br>` is as tall as the others in a table cell (three 24px lines, a 72px
/// row); its line break took the cell's `vertical-align: middle` again, which made it 25px.
#[test]
fn a_cells_line_breaks_keep_their_lines_height() {
    let mut view = view(
        "<table cellpadding=\"0\" cellspacing=\"0\" style=\"font-family: Arial; font-size: 14px\">\
         <tr><td>One<br>two<br>three</td></tr></table>",
    );
    assert_eq!(view.layout(640.0, 2.0), 72.0);
}

/// A table without cells has no border spacing, as in Chromium (`body-tables2.eml`): it is only
/// as big as its borders.
#[test]
fn tables_without_cells_have_no_border_spacing() {
    let height = |tables: &str| {
        view(&format!("<div>Above</div>{tables}<div>Below</div>")).layout(640.0, 2.0)
    };
    let bare = height("");
    assert_eq!(
        height("<table cellspacing=\"10\"></table><table cellspacing=\"10\"><tr></tr></table>"),
        bare
    );
    assert_eq!(
        height("<table cellspacing=\"10\" border=\"3\"></table>"),
        bare + 6.0
    );
}

/// A table's `border` attribute gives it an `outset` border that wide and its cells 1px `inset`
/// ones (an empty value is 1px), beveled as Chromium bevels black: #a8a8a8 on the table's top
/// edge, #545454 on a cell's (`body-table-borders.eml`).
#[test]
fn a_tables_border_attribute_borders_it_and_its_cells() {
    let table = |border: &str| {
        view(&format!(
            "<table border=\"{border}\" cellspacing=\"0\" cellpadding=\"0\"><tr><td><div style=\"width:20px;height:20px\"></div></td></tr></table>"
        ))
    };
    assert_eq!(table("0").layout(640.0, 2.0), 20.0);
    assert_eq!(table("").layout(640.0, 2.0), 24.0);
    let mut view = table("4");
    assert_eq!(view.layout(640.0, 2.0), 30.0);
    let image = view.paint(
        Region {
            top: 0.0,
            height: 30.0,
            left: 0.0,
            width: 30.0,
            phase: [0.0; 2],
        },
        [255; 4],
    );
    let gray = |y: u32| image.rgba[((y * image.width + 30) * 4) as usize];
    assert!(
        (160..=185).contains(&gray(1)),
        "the table's top edge: {}",
        gray(1)
    );
    assert!(
        (80..=110).contains(&gray(9)),
        "the cell's top edge: {}",
        gray(9)
    );
}
