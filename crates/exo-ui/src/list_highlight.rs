//! The list cursor: one background and focus bar that slide to the cursor's row instead of each row
//! restyling.

use egui::{Color32, Context, CornerRadius, Id, Painter, Pos2, Rect, vec2};

use crate::motion::{self, CubicBezier};

/// `transform 256ms cubic-bezier(0.19, 1, 0.22, 1)`.
const SLIDE: f32 = 0.256;
const SLIDE_CURVE: CubicBezier = CubicBezier::new(0.19, 1.0, 0.22, 1.0);
/// The focus bar's `opacity 160ms` on the same curve.
const BAR_FADE: f32 = 0.16;
/// The focus bar's width.
const BAR_WIDTH: f32 = 3.0;
/// The highlight's corners.
const RADIUS: u8 = 2;

/// Where the highlight sits, in the list's content coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HighlightTarget {
    /// The row's top, from the top of the list's content.
    pub top: f32,
    /// The row's height (`scaleY(height / 36)`).
    pub height: f32,
}

/// How the highlight looks this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HighlightState {
    /// The list has keyboard focus, which shows the focus bar.
    pub focused: bool,
    /// The cursor row is selected: no background, and a blue bar.
    pub selected: bool,
    /// Move without animating (list changes, resizes, jumps off-screen).
    pub jump: bool,
    /// A selected row touches it above or below, so the corners there are square and the two
    /// read as one block.
    pub joined_above: bool,
    pub joined_below: bool,
}

#[derive(Clone, Copy)]
struct Slide {
    from: HighlightTarget,
    to: HighlightTarget,
    started: f64,
}

/// The cursor highlight of one list, identified by `id`.
pub struct ListHighlight {
    id: Id,
}

impl ListHighlight {
    /// The highlight with state stored under `id`.
    pub fn new(id: Id) -> Self {
        Self { id }
    }

    /// Paints the highlight's background on `target` in a list whose content starts at `origin`
    /// and is `width` wide, under the rows; `None` hides it (no cursor). Returns its focus bar,
    /// which goes over the rows, so a selected row's own fill doesn't hide it.
    pub fn paint(
        &self,
        painter: &Painter,
        origin: Pos2,
        width: f32,
        target: Option<HighlightTarget>,
        state: HighlightState,
    ) -> HighlightBar {
        let ctx = painter.ctx();
        let Some(target) = target else {
            ctx.data_mut(|data| data.remove::<Slide>(self.id));
            return HighlightBar(None);
        };
        let (current, animating) = self.slide(ctx, target, state.jump);
        let palette = crate::palette(ctx);
        let rect = crate::pixel_snap(Rect::from_min_size(
            origin + vec2(0.0, current.top),
            vec2(width, current.height),
        ));

        // On a selected row the background fades out (`filter: opacity(0)`), over the slide's
        // timing.
        let background = motion::transition(
            ctx,
            self.id.with("background"),
            !state.selected,
            SLIDE,
            SLIDE_CURVE,
        );
        let (top, bottom) = (
            if state.joined_above { 0 } else { RADIUS },
            if state.joined_below { 0 } else { RADIUS },
        );
        let corners = CornerRadius {
            nw: top,
            ne: top,
            sw: bottom,
            se: bottom,
        };
        painter.rect_filled(
            rect,
            corners,
            palette.background_brand_subdued.gamma_multiply(background),
        );

        let bar = motion::transition(
            ctx,
            self.id.with("bar"),
            state.focused || state.selected,
            BAR_FADE,
            SLIDE_CURVE,
        );
        let bar_color = if state.selected {
            SELECTED_BAR
        } else {
            palette.border_base_active
        };
        let bar_rect = Rect::from_min_size(rect.min, vec2(BAR_WIDTH, rect.height()));
        // The bar is clipped by the component's rounded corners, so only its left side rounds.
        let radius = CornerRadius {
            nw: top,
            sw: bottom,
            ne: 0,
            se: 0,
        };
        if animating {
            ctx.request_repaint();
        }
        HighlightBar(Some((bar_rect, radius, bar_color.gamma_multiply(bar))))
    }

    /// Advances the slide towards `target` and returns where the highlight is now.
    fn slide(&self, ctx: &Context, target: HighlightTarget, jump: bool) -> (HighlightTarget, bool) {
        let now = ctx.input(|input| input.time);
        let previous = ctx.data(|data| data.get_temp::<Slide>(self.id));
        let position = |slide: &Slide| {
            let t = ((now - slide.started) as f32 / SLIDE).clamp(0.0, 1.0);
            let eased = SLIDE_CURVE.at(t);
            let lerp = |a: f32, b: f32| a + (b - a) * eased;
            (
                HighlightTarget {
                    top: lerp(slide.from.top, slide.to.top),
                    height: lerp(slide.from.height, slide.to.height),
                },
                t < 1.0,
            )
        };
        let slide = match previous {
            Some(slide) if slide.to == target => slide,
            // A CSS transition retargeted mid-way starts from wherever it is.
            Some(slide) if !jump => Slide {
                from: position(&slide).0,
                to: target,
                started: now,
            },
            _ => Slide {
                from: target,
                to: target,
                started: now,
            },
        };
        ctx.data_mut(|data| data.insert_temp(self.id, slide));
        position(&slide)
    }
}

/// A highlight's focus bar, painted over the list's rows with [`HighlightBar::paint`].
#[must_use = "the bar goes over the rows: paint it after them"]
pub struct HighlightBar(Option<(Rect, CornerRadius, Color32)>);

impl HighlightBar {
    /// Paints the bar.
    pub fn paint(self, painter: &Painter) {
        if let Some((rect, radius, color)) = self.0 {
            painter.rect_filled(rect, radius, color);
        }
    }
}

/// The bar on a selected row.
const SELECTED_BAR: Color32 = Color32::from_rgb(0x23, 0x7a, 0xa9);
