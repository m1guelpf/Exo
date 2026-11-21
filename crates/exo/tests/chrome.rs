//! The window's chrome end to end: icon buttons under the pointer.

mod support;

use egui::{CursorIcon, Pos2};
use support::start;

#[test]
fn header_icons_point_and_name_their_shortcut() {
    let mut app = start("header-icons");
    // Search and compose at 1280×800 (`right: calc(2rem - 6px)` and `calc(2rem + 10px)`).
    assert_eq!(
        app.hover(Pos2::new(921.0, 83.0)),
        CursorIcon::PointingHand,
        "search"
    );
    assert_eq!(
        app.hover(Pos2::new(889.0, 83.0)),
        CursorIcon::PointingHand,
        "compose"
    );
    // `Compose` `C` shows once the pointer has rested a 100ms poll, fading in over 200ms: its
    // box 8px under the 32px button (y 67–99), centred on it.
    app.wait(0.5);
    let image = app.harness.render().expect("renders");
    let background = exo_ui::palette(&app.harness.ctx).tooltip_background_default;
    let pixel = |x: u32, y: u32| {
        egui::Color32::from_rgba_unmultiplied(
            image.get_pixel(x, y)[0],
            image.get_pixel(x, y)[1],
            image.get_pixel(x, y)[2],
            255,
        )
    };
    assert_eq!(
        pixel(889, 110),
        background,
        "the tooltip's top padding, under the button"
    );
    assert_ne!(pixel(889, 104), background, "the 8px gap");
}
