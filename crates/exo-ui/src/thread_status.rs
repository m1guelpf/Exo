//! A conversation's unread dot and star, as rows and the thread header show it hanging left of the
//! text, and their animations as they come and go: the star's sprite, the dot's pop, and the star's
//! 4px slide out of the dot's way.

use egui::{Color32, Context, Id, Painter, Pos2, Rect, Vec2, pos2, vec2};

use crate::motion::CubicBezier;

/// The star's sprite (`icons/sprites/star.svg`, 1530×23): 102 frames 15px apart, from the faintest
/// dot through a burst of rays to the resting dot. The 15px box shows the top 15px of a frame.
const SPRITE: &str = "sprites/star";
const SPRITE_ROOT: &str = r#"width="1530" height="23""#;
const FRAME: f32 = 15.0;
/// The last frame, where a star rests.
const RESTING: usize = 101;
/// A star comes or goes in 101 steps over 704ms (`transition: transform 704ms steps(101)`), and
/// one that went is taken away at 725ms.
const STEPS_FOR: f64 = 0.704;
const REMOVED_AT: f64 = 0.725;

/// How a conversation's star shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Star {
    /// No star.
    None,
    /// At rest.
    Resting,
    /// This frame of the sprite, as it comes or goes.
    Frame(usize),
}

impl Star {
    /// The star of a conversation that is starred or not, shown without animating.
    pub fn resting(starred: bool) -> Self {
        if starred { Self::Resting } else { Self::None }
    }

    /// The star `elapsed` seconds after the conversation was starred (the sprite runs forward to
    /// rest) or unstarred (it runs back to its first frame, then goes).
    pub fn changed(starred: bool, elapsed: f64) -> Self {
        // `steps(101)` jumps at the end of each step: the last frame shows once the time is up.
        let step = ((elapsed / STEPS_FOR * RESTING as f64).floor().max(0.0) as usize).min(RESTING);
        match (starred, step) {
            (true, RESTING) => Self::Resting,
            (true, step) => Self::Frame(step),
            (false, _) if elapsed >= REMOVED_AT => Self::None,
            (false, step) => Self::Frame(RESTING - step),
        }
    }
}

/// The star of the conversation shown under `id`, which is now `starred`: when that changed while
/// it showed, the sprite plays; when it wasn't shown the frame before, it rests: a star that
/// appears with its row doesn't animate.
pub fn animated_star(ctx: &Context, id: Id, starred: bool) -> Star {
    #[derive(Clone, Copy)]
    struct Seen {
        starred: bool,
        /// When it last changed while shown.
        changed: Option<f64>,
        pass: u64,
    }
    let (now, pass) = (ctx.input(|input| input.time), ctx.cumulative_pass_nr());
    let seen = ctx
        .data(|data| data.get_temp::<Seen>(id))
        .filter(|seen| seen.pass + 1 >= pass);
    let changed = match seen {
        Some(seen) if seen.starred != starred => Some(now),
        Some(seen) => seen.changed,
        None => None,
    };
    ctx.data_mut(|data| {
        data.insert_temp(
            id,
            Seen {
                starred,
                changed,
                pass,
            },
        )
    });
    let Some(changed) = changed else {
        return Star::resting(starred);
    };
    let star = Star::changed(starred, now - changed);
    if matches!(star, Star::Frame(_)) {
        ctx.request_repaint();
    }
    star
}

/// The dot pops in over 300ms with an overshoot, shrinks away over 300ms on another curve, and the
/// star slides between its two places over 300ms on the first one.
const DOT_FOR: f64 = 0.3;
const POP: CubicBezier = CubicBezier::new(0.38, 0.87, 0.64, 1.65);
const SHRINK: CubicBezier = CubicBezier::new(0.0, -0.87, 0.74, 0.05);
/// `transform-origin: 11px 7.5px` in the dot's 15px box.
const DOT_ORIGIN: Vec2 = Vec2::new(11.0, 7.5);
/// A star beside an unread dot moves left (`transform: translateX(-4px)`).
const STAR_SHIFT: f32 = -4.0;

/// A conversation's status as painted, mid-animation or at rest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Status {
    /// The dot's kind: one that's shrinking away keeps its color.
    pub unread: Unread,
    /// The dot's scale (1 at rest).
    pub dot: f32,
    /// The star.
    pub star: Star,
    /// How far left of its unread-less place the star is: 0, or 4px beside an unread dot.
    pub star_shift: f32,
}

impl Status {
    /// The status at rest.
    pub fn resting(unread: Unread, starred: bool) -> Self {
        Self {
            unread,
            dot: 1.0,
            star: Star::resting(starred),
            star_shift: if unread == Unread::No {
                0.0
            } else {
                STAR_SHIFT
            },
        }
    }

    /// The boxes it shows (the dot's and the star's 15px boxes, hanging off `origin`), which is
    /// where its tooltip's hover is (the status is a 0×0 box over them): `None` with neither.
    pub fn rect(&self, origin: Pos2) -> Option<Rect> {
        let dot = (self.unread != Unread::No)
            .then(|| Rect::from_min_size(origin + vec2(-20.0, 0.0), Vec2::splat(FRAME)));
        let star = (self.star != Star::None).then(|| {
            Rect::from_min_size(
                origin + vec2(-17.0 + self.star_shift, 0.0),
                Vec2::splat(FRAME),
            )
        });
        match (dot, star) {
            (Some(dot), Some(star)) => Some(dot.union(star)),
            (dot, star) => dot.or(star),
        }
    }
}

/// The status of the conversation shown under `id`, which is now `unread` and `starred`: what
/// changed while it showed animates; shown afresh, it rests.
pub fn animated_status(ctx: &Context, id: Id, unread: Unread, starred: bool) -> Status {
    #[derive(Clone, Copy)]
    struct Seen {
        unread: Unread,
        /// When the dot last came or went while shown, and its kind then.
        changed: Option<(f64, Unread)>,
        pass: u64,
    }
    let star = animated_star(ctx, id.with("star"), starred);
    let (now, pass) = (ctx.input(|input| input.time), ctx.cumulative_pass_nr());
    let seen = ctx
        .data(|data| data.get_temp::<Seen>(id))
        .filter(|seen| seen.pass + 1 >= pass);
    let changed = match seen {
        Some(seen) if (seen.unread == Unread::No) != (unread == Unread::No) => Some((
            now,
            if unread == Unread::No {
                seen.unread
            } else {
                unread
            },
        )),
        Some(seen) => seen.changed,
        None => None,
    };
    ctx.data_mut(|data| {
        data.insert_temp(
            id,
            Seen {
                unread,
                changed,
                pass,
            },
        )
    });
    let mut status = Status {
        star,
        ..Status::resting(unread, starred)
    };
    let Some((at, kind)) = changed else {
        return status;
    };
    let progress = ((now - at) / DOT_FOR).min(1.0) as f32;
    if progress < 1.0 {
        ctx.request_repaint();
    }
    // The star slides along with the dot, from where it was.
    let (from, to) = if unread == Unread::No {
        (STAR_SHIFT, 0.0)
    } else {
        (0.0, STAR_SHIFT)
    };
    status.star_shift = from + (to - from) * POP.at(progress);
    if unread == Unread::No {
        // Shrinking away, then gone (at 725ms; it's at scale 0 by then).
        if progress < 1.0 {
            status.unread = kind;
            status.dot = 1.0 - SHRINK.at(progress);
        }
    } else {
        status.dot = POP.at(progress);
    }
    status
}

/// The unread dot's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unread {
    /// Read: no dot.
    No,
    /// Unread: blue.
    Yes,
    /// Back from a reminder and not read since: pink.
    Reminder,
}

/// Paints the unread dot and the star whose boxes hang off `origin` (the status's position): 6.6px
/// dots in 15px boxes. The star's sprite rests on a frame that is the same dot in gold, 3.5px
/// further left in its box (measured), painted over the unread dot.
pub fn paint_thread_status(painter: &Painter, origin: Pos2, unread: Unread, star: Star) {
    paint_status(
        painter,
        origin,
        &Status {
            star,
            ..Status::resting(unread, star != Star::None)
        },
    );
}

/// Paints `status` hanging off `origin`, as [`paint_thread_status`] does, scaling the dot about
/// its transform origin and sliding the star.
pub fn paint_status(painter: &Painter, origin: Pos2, status: &Status) {
    // The box snaps to whole px; the image is drawn at its corner.
    let frame = |box_left: f32| {
        crate::pixel_snap(Rect::from_min_size(
            origin + vec2(box_left, 0.0),
            Vec2::splat(FRAME),
        ))
    };
    let dot = |frame: Rect, center_x: f32, scale: f32, color: Color32| {
        let center = frame.min + DOT_ORIGIN + (vec2(center_x, 7.35) - DOT_ORIGIN) * scale;
        painter.circle_filled(center, 3.31 * scale, color);
    };
    // A returned reminder's dot is `#ce9ece`.
    match status.unread {
        Unread::No => {}
        Unread::Yes => dot(
            frame(-20.0),
            10.84,
            status.dot,
            Color32::from_rgb(0x72, 0xc0, 0xea),
        ),
        Unread::Reminder => dot(
            frame(-20.0),
            10.84,
            status.dot,
            Color32::from_rgb(0xce, 0x9e, 0xce),
        ),
    }
    // A transform moves the snapped box by whatever it is mid-slide.
    let star_frame = frame(-17.0).translate(vec2(status.star_shift, 0.0));
    match status.star {
        Star::None => {}
        Star::Resting => dot(star_frame, 7.34, 1.0, Color32::from_rgb(0xff, 0xcb, 0x33)),
        Star::Frame(at) => paint_sprite_frame(painter, star_frame, at),
    }
}

/// Paints the status of a conversation with a failed draft hanging off `origin`: the unread dot in
/// red (`rgba(239, 142, 142, 1)`) and no star.
pub fn paint_failed_status(painter: &Painter, origin: Pos2) {
    let frame = crate::pixel_snap(Rect::from_min_size(
        origin + vec2(-20.0, 0.0),
        Vec2::splat(FRAME),
    ));
    painter.circle_filled(
        frame.min + vec2(10.84, 7.35),
        3.31,
        Color32::from_rgb(0xef, 0x8e, 0x8e),
    );
}

/// Paints frame `at` of the star's sprite in `rect`, its 15px box. Each frame is an image of its
/// own: the whole strip is wider than a texture may be.
fn paint_sprite_frame(painter: &Painter, rect: Rect, at: usize) {
    let ctx = painter.ctx();
    let left = at as f32 * FRAME;
    let edit = |svg: String| {
        svg.replacen(
            SPRITE_ROOT,
            &format!(r#"width="{FRAME}" height="{FRAME}" viewBox="{left} 0 {FRAME} {FRAME}""#),
            1,
        )
    };
    let Some(uri) = crate::icons::edited_icon(ctx, SPRITE, &format!("frame{at}"), edit) else {
        return;
    };
    let side = (FRAME * painter.pixels_per_point()).round() as u32;
    let hint = egui::SizeHint::Size {
        width: side,
        height: side,
        maintain_aspect_ratio: false,
    };
    let Ok(egui::load::TexturePoll::Ready { texture }) =
        egui::ImageSource::Uri(uri.into()).load(ctx, egui::TextureOptions::default(), hint)
    else {
        return;
    };
    painter.image(
        texture.id,
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sprite_runs_forward_to_star_and_back_to_unstar() {
        assert_eq!(Star::changed(true, 0.0), Star::Frame(0));
        assert_eq!(Star::changed(true, 0.352), Star::Frame(50));
        assert_eq!(Star::changed(true, 0.703), Star::Frame(100));
        assert_eq!(Star::changed(true, 0.704), Star::Resting);
        assert_eq!(Star::changed(false, 0.0), Star::Frame(101));
        assert_eq!(Star::changed(false, 0.352), Star::Frame(51));
        assert_eq!(
            Star::changed(false, 0.71),
            Star::Frame(0),
            "the first frame shows until it's taken away"
        );
        assert_eq!(Star::changed(false, 0.725), Star::None);
    }

    #[test]
    fn the_dot_pops_in_shrinks_away_and_moves_the_star() {
        let ctx = Context::default();
        let id = crate::id(&ctx, "status");
        let frame = |unread: Unread, time: f64| {
            let input = egui::RawInput {
                time: Some(time),
                ..Default::default()
            };
            let mut status = Status::resting(Unread::No, false);
            ctx.run_ui(input, |ui| {
                status = animated_status(ui.ctx(), id, unread, true)
            })
            .drop_without_applying_deltas();
            status
        };
        assert_eq!(
            frame(Unread::No, 0.0),
            Status::resting(Unread::No, true),
            "shown read: at rest"
        );
        let popping = frame(Unread::Yes, 1.0);
        assert_eq!(
            (popping.unread, popping.dot, popping.star_shift),
            (Unread::Yes, 0.0, 0.0),
            "marked unread: the dot starts at nothing"
        );
        let overshoot = frame(Unread::Yes, 1.18);
        assert!(
            overshoot.dot > 1.0 && overshoot.star_shift < STAR_SHIFT,
            "it overshoots: {overshoot:?}"
        );
        assert_eq!(frame(Unread::Yes, 1.3), Status::resting(Unread::Yes, true));
        let shrinking = frame(Unread::No, 2.0);
        assert_eq!(
            (shrinking.unread, shrinking.dot),
            (Unread::Yes, 1.0),
            "read again: the dot shrinks from full size"
        );
        assert!(frame(Unread::No, 2.25).dot < 0.5);
        assert_eq!(
            frame(Unread::No, 2.3),
            Status::resting(Unread::No, true),
            "and goes"
        );
    }

    #[test]
    fn a_star_animates_only_when_it_changes_while_shown() {
        let ctx = Context::default();
        let id = crate::id(&ctx, "star");
        let frame = |starred: bool, time: f64| {
            let input = egui::RawInput {
                time: Some(time),
                ..Default::default()
            };
            let mut star = Star::None;
            ctx.run_ui(input, |ui| star = animated_star(ui.ctx(), id, starred))
                .drop_without_applying_deltas();
            star
        };
        assert_eq!(frame(true, 0.0), Star::Resting, "shown starred");
        assert_eq!(
            frame(false, 1.0),
            Star::Frame(RESTING),
            "unstarred while shown"
        );
        assert_eq!(frame(false, 1.352), Star::Frame(51));
        assert_eq!(frame(false, 1.8), Star::None);
        assert_eq!(frame(true, 2.0), Star::Frame(0), "starred again");
        // Not shown for a frame (scrolled off, not the cursor): shown again, it rests.
        ctx.run_ui(
            egui::RawInput {
                time: Some(2.1),
                ..Default::default()
            },
            |_| {},
        )
        .drop_without_applying_deltas();
        assert_eq!(frame(false, 2.2), Star::None);
    }
}
