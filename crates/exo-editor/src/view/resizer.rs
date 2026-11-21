//! The image resizer: over a focused image, a handle at each corner and, on an image wider than
//! 130px, `Small` and `Best Fit`, in a tooltip box exactly over the image.
//!
//! The handles are 8px squares (their 1px border included) centred on the image's corners, the
//! buttons 31px tall in the UA's 13.333px Arial, 8px inside the image's top right corner and 10px
//! apart.

use egui::{Color32, CornerRadius, CursorIcon, Id, Pos2, Rect, Sense, Ui, pos2, vec2};
use exo_ui::{BoxShadow, Family, Font, TextStyle};

use crate::{Editor, Fit, image::MIN_DRAG_WIDTH};

/// The resize handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle {
    /// Inert, white.
    TopLeft,
    /// Drags the width (`ew-resize`).
    TopRight,
    /// Drags the height, the width following (`ns-resize`).
    BottomLeft,
    /// Drags the width (`nwse-resize`).
    BottomRight,
}

impl Handle {
    const ALL: [Self; 4] = [
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
    ];

    fn cursor(self) -> CursorIcon {
        match self {
            Self::TopLeft => CursorIcon::Default,
            Self::TopRight => CursorIcon::ResizeHorizontal,
            Self::BottomLeft => CursorIcon::ResizeVertical,
            Self::BottomRight => CursorIcon::ResizeNwSe,
        }
    }

    fn top(self) -> bool {
        matches!(self, Self::TopLeft | Self::TopRight)
    }
}

/// A handle being dragged, and the image's box when the drag started.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Drag {
    handle: Handle,
    before: Rect,
}

/// `width: 8px; height: 8px`, the border inside (`box-sizing: border-box`), at `-4px`.
const HANDLE: f32 = 8.0;
const HANDLE_FILL: Color32 = Color32::from_rgb(0x88, 0x92, 0xb9);
const HANDLE_BORDER: Color32 = Color32::from_rgb(0xec, 0xec, 0xec);
/// `box-shadow: 0 1px 1px 0 rgba(0, 0, 0, .5)`.
const HANDLE_SHADOW: BoxShadow = BoxShadow::new(
    [0.0, 1.0],
    1.0,
    0.0,
    Color32::from_rgba_premultiplied(0, 0, 0, 128),
);
/// The tooltip's shadow, around the image.
const SHADOWS: [BoxShadow; 2] = [
    BoxShadow::new(
        [0.0, 2.0],
        4.0,
        0.0,
        Color32::from_rgba_premultiplied(0, 0, 0, 26),
    ),
    BoxShadow::new(
        [0.0, 2.0],
        12.0,
        0.0,
        Color32::from_rgba_premultiplied(0, 0, 0, 13),
    ),
];
/// The buttons' text: the UA's `13.333px Arial`, bold, `line-height: normal`.
const BUTTON_TEXT: TextStyle = TextStyle::new(Font::new(Family::Arial, 700), 40.0 / 3.0, 15.0);
/// The buttons: `padding: 8px; margin-left: 10px; margin-bottom: 5px`, and the buttons' row
/// `margin: 8px`.
const BUTTON_PADDING: f32 = 8.0;
const BUTTON_GAP: f32 = 10.0;
const BUTTON_BELOW: f32 = 5.0;
const BUTTONS_INSET: f32 = 8.0;
const BUTTON_FILL: Color32 = Color32::from_rgb(0x0c, 0x08, 0x07);
/// Buttons show on an image wider than this.
const BUTTONS_FROM: f32 = 130.0;

/// The resizer's parts over an image.
struct Parts {
    handles: [(Handle, Rect); 4],
    buttons: Vec<(Fit, &'static str, Rect)>,
    /// The image's top is scrolled out of view: no buttons, no top handles, no shadow.
    hide_top: bool,
    /// Its bottom is: no bottom handles, no shadow.
    hide_bottom: bool,
}

fn parts(ui: &Ui, image: Rect) -> Parts {
    let corner = |handle: Handle| {
        let x = if matches!(handle, Handle::TopLeft | Handle::BottomLeft) {
            image.left()
        } else {
            image.right()
        };
        let y = if handle.top() {
            image.top()
        } else {
            image.bottom()
        };
        Rect::from_min_size(
            pos2(x - HANDLE / 2.0, y - HANDLE / 2.0),
            vec2(HANDLE, HANDLE),
        )
    };
    let mut buttons = Vec::new();
    if image.width() > BUTTONS_FROM {
        // A wrapping flex row, its lines packed to the right (`justify-content: flex-end`).
        let labels = [(Fit::Small, "Small"), (Fit::BestFit, "Best Fit")];
        let widths: Vec<f32> = labels
            .iter()
            .map(|(_, label)| super::layout_unit(advance(ui, label)) + 2.0 * BUTTON_PADDING)
            .collect();
        let available = image.width() - 2.0 * BUTTONS_INSET;
        let height = BUTTON_TEXT.line_height + 2.0 * BUTTON_PADDING;
        let mut lines: Vec<Vec<usize>> = vec![Vec::new()];
        let mut used = 0.0;
        for (index, width) in widths.iter().enumerate() {
            let line = lines.last_mut().expect("a line");
            if !line.is_empty() && used + BUTTON_GAP + width > available {
                lines.push(vec![index]);
                used = BUTTON_GAP + width;
            } else {
                line.push(index);
                used += BUTTON_GAP + width;
            }
        }
        let right = image.right() - BUTTONS_INSET;
        for (row, line) in lines.iter().enumerate() {
            let total: f32 = line.iter().map(|index| BUTTON_GAP + widths[*index]).sum();
            let mut x = right - total;
            let y = image.top() + BUTTONS_INSET + row as f32 * (height + BUTTON_BELOW);
            for index in line {
                x += BUTTON_GAP;
                let (fit, label) = labels[*index];
                buttons.push((
                    fit,
                    label,
                    Rect::from_min_size(pos2(x, y), vec2(widths[*index], height)),
                ));
                x += widths[*index];
            }
        }
    }
    let clip = ui.clip_rect();
    Parts {
        handles: Handle::ALL.map(|handle| (handle, corner(handle))),
        buttons,
        hide_top: image.top() < clip.top(),
        hide_bottom: image.bottom() > clip.bottom(),
    }
}

/// The advance of a button's label (a galley's size is rounded up to whole points).
fn advance(ui: &Ui, label: &str) -> f32 {
    let galley = exo_ui::layout(
        ui.ctx(),
        egui::text::LayoutJob::single_section(label.to_owned(), BUTTON_TEXT.format(Color32::WHITE)),
    );
    galley
        .rows
        .iter()
        .flat_map(|row| &row.glyphs)
        .map(|glyph| glyph.advance_width)
        .sum()
}

/// The tooltip box's shadow around the focused image, painted under it.
pub(super) fn paint_shadow(ui: &Ui, image: Rect) {
    let parts = parts(ui, image);
    if parts.hide_top || parts.hide_bottom {
        return;
    }
    for shadow in SHADOWS {
        shadow.paint_outside(ui.painter(), exo_ui::pixel_snap(image));
    }
}

impl Editor {
    /// The resizer over the focused image of an editor at `origin`: its handles drag the width
    /// (an undo step before and after the drag), its buttons set Small or Best Fit.
    pub(super) fn resizer(&mut self, ui: &Ui, origin: Pos2, id: Id) {
        let Some(at) = self.focused_image() else {
            self.view.resize = None;
            return;
        };
        let Some(image) = self.image_rect(origin, at) else {
            return;
        };
        let parts = parts(ui, image);
        let painter = ui.painter();
        for (handle, rect) in parts.handles {
            if (handle.top() && parts.hide_top) || (!handle.top() && parts.hide_bottom) {
                continue;
            }
            let response = ui
                .interact(
                    rect,
                    id.with(("resize", handle as u8)),
                    if handle == Handle::TopLeft {
                        Sense::click()
                    } else {
                        Sense::drag()
                    },
                )
                .on_hover_cursor(handle.cursor());
            if handle != Handle::TopLeft {
                if response.drag_started() {
                    self.begin_resize();
                    self.view.resize = Some(Drag {
                        handle,
                        before: image,
                    });
                }
                if response.dragged()
                    && let (Some(drag), Some(pointer)) =
                        (self.view.resize, response.interact_pointer_pos())
                {
                    let width = match drag.handle {
                        // `inverse`: the drop of the bottom edge, in width at the image's ratio.
                        Handle::BottomLeft => {
                            drag.before.width()
                                + (pointer.y - drag.before.height() - image.top())
                                    * (drag.before.width() / drag.before.height())
                        }
                        _ => pointer.x - image.left(),
                    };
                    if width > MIN_DRAG_WIDTH {
                        self.resize_focused(width);
                    }
                }
                if response.drag_stopped() && self.view.resize.take().is_some() {
                    self.end_resize();
                }
            }
            let rect = exo_ui::pixel_snap(rect);
            HANDLE_SHADOW.paint(painter, rect, CornerRadius::ZERO);
            painter.rect_filled(rect, CornerRadius::ZERO, HANDLE_BORDER);
            painter.rect_filled(
                rect.shrink(1.0),
                CornerRadius::ZERO,
                if handle == Handle::TopLeft {
                    Color32::WHITE
                } else {
                    HANDLE_FILL
                },
            );
        }
        if parts.hide_top {
            return;
        }
        for (fit, label, rect) in parts.buttons {
            if ui
                .interact(rect, id.with(("fit", label)), Sense::click())
                .clicked()
            {
                self.fit_image(fit);
            }
            // The background snaps to whole pixels; the text keeps its place.
            painter.rect_filled(exo_ui::pixel_snap(rect), CornerRadius::same(4), BUTTON_FILL);
            BUTTON_TEXT.paint_line(
                painter,
                rect.min + vec2(BUTTON_PADDING, BUTTON_PADDING),
                label,
                Color32::WHITE,
                None,
            );
        }
    }
}
