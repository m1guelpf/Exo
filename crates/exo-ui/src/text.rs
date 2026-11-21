//! Text laid out in CSS line boxes, so it lands on the same pixels as in Chromium.
//!
//! egui puts a row's baseline one ascent below the row's top. CSS centers the font's content area
//! in the line box instead, and Chromium rounds the ascent and descent to whole pixels and gives
//! the odd pixel of leading to the bottom. [`TextStyle::baseline`] computes the CSS position and
//! [`paint_text`] moves egui's galley onto it, and paints its colour emoji. egui also hangs each
//! fallback font's glyphs from that font's own ascent, where CSS puts every glyph on the line's
//! baseline: [`align_baselines`] moves them there.

use std::sync::Arc;

use egui::{
    Color32, FontId, Galley, Painter, Pos2, Rect, Response, Sense, TextFormat, Ui, Vec2, Widget,
    epaint::text::{ByteIndex, Glyph, LayoutSection, Row},
    text::{LayoutJob, TextWrapping},
};

use crate::fonts::{Font, VerticalMetrics};

/// A `<wbr>`, as Chromium lays one out: a zero-width space, where a line may break.
pub const WBR: char = '\u{200B}';

/// A text style in CSS terms. Sizes are CSS px, which are egui points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    /// `font-family`, `font-weight` and `font-style`.
    pub font: Font,
    /// `font-size`.
    pub size: f32,
    /// `line-height`.
    pub line_height: f32,
    /// `letter-spacing`.
    pub letter_spacing: f32,
}

impl TextStyle {
    /// A style with no letter spacing.
    pub const fn new(font: Font, size: f32, line_height: f32) -> Self {
        Self {
            font,
            size,
            line_height,
            letter_spacing: 0.0,
        }
    }

    /// The same style with `font-variant-ligatures: none` (see [`Font::without_ligatures`]).
    pub const fn without_ligatures(self) -> Self {
        Self {
            font: self.font.without_ligatures(),
            ..self
        }
    }

    /// The same style with `letter-spacing: px`.
    pub const fn letter_spacing(self, px: f32) -> Self {
        Self {
            letter_spacing: px,
            ..self
        }
    }

    /// The egui format for a run of text in this style.
    pub fn format(&self, color: Color32) -> TextFormat {
        let (face, italics) = self.font.face();
        TextFormat {
            font_id: FontId::new(self.size, self.font.egui_family(face)),
            line_height: Some(self.line_height),
            extra_letter_spacing: self.letter_spacing,
            color,
            italics,
            coords: face.variations(self.size),
            ..Default::default()
        }
    }

    /// Distance from the top of a line box to the baseline, as Chromium computes it.
    pub fn baseline(&self, ctx: &egui::Context) -> f32 {
        css_baseline(
            crate::metrics(ctx, self.font.face().0),
            self.size,
            self.line_height,
        )
    }

    /// Distance from the top of a line box to the bottom of an inline element's content area (the
    /// baseline plus the font's descent), which an absolutely placed `::after` hangs from.
    pub fn content_bottom(&self, ctx: &egui::Context) -> f32 {
        self.baseline(ctx) + (crate::metrics(ctx, self.font.face().0).descent * self.size).round()
    }

    /// Lays `text` out on one line (`white-space: nowrap`), ending in "…" if it's wider than
    /// `max_width` (`text-overflow: ellipsis`). Paint it with [`paint_text`].
    pub fn single_line(
        &self,
        painter: &Painter,
        text: &str,
        color: Color32,
        max_width: Option<f32>,
    ) -> Arc<Galley> {
        layout(painter.ctx(), self.line_job(text, color, max_width))
    }

    /// Lays out and paints one line of `text` (as [`TextStyle::single_line`]) with its line box
    /// at `line_top_left`. Returns the text's width.
    pub fn paint_line(
        &self,
        painter: &Painter,
        line_top_left: Pos2,
        text: &str,
        color: Color32,
        max_width: Option<f32>,
    ) -> f32 {
        paint_job(
            painter,
            line_top_left,
            self.line_job(text, color, max_width),
            self,
        )
    }

    /// [`TextStyle::paint_line`] without kerning (`font-kerning: none`). Returns the width.
    pub fn paint_line_unkerned(
        &self,
        painter: &Painter,
        line_top_left: Pos2,
        text: &str,
        color: Color32,
    ) -> f32 {
        paint_job(
            painter,
            line_top_left,
            unkerned(self.line_job(text, color, None)),
            self,
        )
    }

    /// The width of `text` on one line.
    pub fn width(&self, ctx: &egui::Context, text: &str) -> f32 {
        layout(
            ctx,
            LayoutJob::single_section(text.to_owned(), self.format(Color32::WHITE)),
        )
        .size()
        .x
    }

    /// [`TextStyle::width`] without kerning (`font-kerning: none`).
    pub fn width_unkerned(&self, ctx: &egui::Context, text: &str) -> f32 {
        layout(
            ctx,
            unkerned(LayoutJob::single_section(
                text.to_owned(),
                self.format(Color32::WHITE),
            )),
        )
        .size()
        .x
    }

    /// Breaks `text` into lines no wider than `width` the way Chromium breaks plain text: at
    /// spaces, at each [`WBR`] (a `<wbr>`, which stays at the end of the line it ends) and at
    /// each `\n` (a `<br>`). Paint each with [`TextStyle::paint_line`], one `line_height` below
    /// the last: egui's own wrapping puts continuation rows off the sub-pixel position Chromium
    /// gives them.
    pub fn wrap(&self, ctx: &egui::Context, text: &str, width: f32) -> Vec<String> {
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            let mut line = String::new();
            for (at, word) in paragraph.split(' ').enumerate() {
                // An empty word (two spaces in a row) keeps its space.
                let pieces = std::iter::once(word)
                    .filter(|word| word.is_empty())
                    .chain(word.split_inclusive(WBR));
                for (part, piece) in pieces.enumerate() {
                    let joiner = if at > 0 && part == 0 { " " } else { "" };
                    let candidate = if line.is_empty() {
                        piece.to_owned()
                    } else {
                        format!("{line}{joiner}{piece}")
                    };
                    if !line.is_empty() && self.width(ctx, &candidate) > width {
                        lines.push(std::mem::replace(&mut line, piece.to_owned()));
                    } else {
                        line = candidate;
                    }
                }
            }
            lines.push(line);
        }
        lines
    }

    /// [`TextStyle::wrap`] with `text-wrap: pretty`'s rule against a last line of one word: the
    /// line above gives up its last word when the two fit. A line broken at a `<wbr>` isn't a
    /// word on its own.
    pub fn wrap_pretty(&self, ctx: &egui::Context, text: &str, width: f32) -> Vec<String> {
        let mut wrapped = self.wrap(ctx, text, width);
        if let [.., before, last] = wrapped.as_mut_slice()
            && !last.contains([' ', WBR])
            && !before.ends_with(WBR)
            && let Some((kept, moved)) = before.rsplit_once(' ')
        {
            let joined = format!("{moved} {last}");
            if self.width(ctx, &joined) <= width {
                (*before, *last) = (kept.to_owned(), joined);
            }
        }
        wrapped
    }

    pub(crate) fn line_job(&self, text: &str, color: Color32, max_width: Option<f32>) -> LayoutJob {
        let mut job = LayoutJob::single_section(text.to_owned(), self.format(color));
        job.wrap =
            max_width.map_or_else(TextWrapping::no_max_width, TextWrapping::truncate_at_width);
        job
    }
}

/// `job` as `font-kerning: none` lays it out: each character a section of its own, since egui
/// shapes each section apart and kerns only within one.
pub(crate) fn unkerned(mut job: LayoutJob) -> LayoutJob {
    for section in std::mem::take(&mut job.sections) {
        let start = section.byte_range.start.0;
        let mut leading_space = section.leading_space;
        for (at, ch) in job.text[start..section.byte_range.end.0].char_indices() {
            let from = start + at;
            let byte_range = ByteIndex(from)..ByteIndex(from + ch.len_utf8());
            job.sections.push(LayoutSection {
                leading_space: std::mem::take(&mut leading_space),
                byte_range,
                format: section.format.clone(),
            });
        }
    }
    job
}

/// Lays `job` out with egui, then gives each emoji the room Chromium gives it (egui's is close only
/// at 12px and 16px). Lay text out through this rather than egui's fonts, or [`TextStyle`]'s
/// helpers that use it, so it takes the room it takes in Chromium.
pub fn layout(ctx: &egui::Context, job: LayoutJob) -> Arc<Galley> {
    let mut galley = ctx.fonts_mut(|fonts| fonts.layout_job(job));
    crate::emoji::fit_advances(ctx, &mut galley);
    galley
}

/// Lays out `job` (one line, in `style`'s line box) and paints it at `line_top_left`. Returns
/// the text's width.
///
/// Chromium puts each glyph on the nearest quarter device pixel of its *absolute* position.
/// egui bins glyphs to quarter pixels relative to the galley, then rounds the galley to a whole
/// pixel, which moves text laid out at a fractional x by up to half a pixel. So the layout
/// starts at the origin's sub-pixel phase and the galley is painted at the whole pixel below it.
pub fn paint_job(
    painter: &Painter,
    line_top_left: Pos2,
    mut job: LayoutJob,
    style: &TextStyle,
) -> f32 {
    let pixels_per_point = painter.pixels_per_point();
    let x = line_top_left.x * pixels_per_point;
    let phase = (x - x.floor()) / pixels_per_point;
    if let Some(first) = job.sections.first_mut() {
        first.leading_space += phase;
    }
    if job.wrap.max_width.is_finite() {
        job.wrap.max_width += phase;
    }
    let galley = layout(painter.ctx(), job);
    let width = galley.size().x - phase;
    paint_text(
        painter,
        Pos2::new(x.floor() / pixels_per_point, line_top_left.y),
        galley,
        style,
    );
    width
}

fn css_baseline(metrics: VerticalMetrics, size: f32, line_height: f32) -> f32 {
    let ascent = (metrics.ascent * size).round();
    let descent = (metrics.descent * size).round();
    ((line_height - ascent - descent) / 2.0).floor() + ascent
}

/// Paints `galley` (laid out with `style`) so its first line box starts at `line_top_left`, every
/// glyph on its line's baseline, with any matches of a search in it highlighted ([`crate::find`]).
pub fn paint_text(
    painter: &Painter,
    line_top_left: Pos2,
    mut galley: Arc<Galley>,
    style: &TextStyle,
) {
    let Some(egui_baseline) = own_baseline(&galley, painter.pixels_per_point()) else {
        return;
    };
    align_baselines(&mut galley, egui_baseline);
    let shift = style.baseline(painter.ctx()) - egui_baseline;
    let origin = line_top_left + Vec2::Y * shift;
    crate::find::text(painter, origin, &mut galley, style);
    painter.galley(origin, galley.clone(), Color32::PLACEHOLDER);
    crate::emoji::paint(painter, origin, &galley);
}

/// Where egui put the baseline of `galley`'s text, below its rows' tops: on the first glyph of
/// the first row in its own font, or where that font's would go.
fn own_baseline(galley: &Galley, pixels_per_point: f32) -> Option<f32> {
    let glyphs = &galley.rows.first()?.glyphs;
    let own = glyphs.iter().find(|glyph| {
        glyph.font_face_ascent == glyph.font_ascent && glyph.font_face_height == glyph.font_height
    });
    match own {
        Some(glyph) => Some(glyph.pos.y),
        None => glyphs
            .first()
            .map(|glyph| (glyph.font_ascent * pixels_per_point).round() / pixels_per_point),
    }
}

/// Puts every glyph of `galley` on one baseline, `baseline` below its row's top, as CSS puts
/// every inline box on its line's baseline. egui hangs each font's glyphs from that font's own
/// ascent, so a fallback font's (an emoji, `⌘` from Lucida Grande, Chinese from Hiragino Sans GB)
/// or another face's (an italic run) sit higher or lower than the text around them.
/// [`paint_text`] aligns to the baseline of the text's first glyph in its own font.
pub fn align_baselines(galley: &mut Arc<Galley>, baseline: f32) {
    let aligned = |glyphs: &[Glyph]| {
        glyphs
            .iter()
            .all(|glyph| (glyph.pos.y - baseline).abs() < 0.01)
    };
    if galley.rows.iter().all(|placed| aligned(&placed.glyphs)) {
        return;
    }
    let galley = Arc::make_mut(galley);
    for placed in &mut galley.rows {
        if aligned(&placed.glyphs) {
            continue;
        }
        let row = Arc::make_mut(&mut placed.row);
        for index in 0..row.glyphs.len() {
            let shift = baseline - row.glyphs[index].pos.y;
            shift_glyph(row, index, Vec2::Y * shift);
        }
        row.visuals.mesh_bounds = row.visuals.mesh.calc_bounds();
    }
    update_bounds(galley);
}

/// Moves glyph `index` of `row` by `delta`, with its quad if egui draws one for it.
pub(crate) fn shift_glyph(row: &mut Row, index: usize, delta: Vec2) {
    if delta == Vec2::ZERO {
        return;
    }
    let glyph = &mut row.glyphs[index];
    glyph.pos += delta;
    // A glyph egui draws nothing for has no quad: its `first_vertex` is the next glyph's.
    let first = glyph.first_vertex as usize;
    if glyph.uv_rect.is_nothing()
        || glyph.uv_rect.size.x <= 0.0
        || !row.visuals.glyph_vertex_range.contains(&first)
    {
        return;
    }
    for vertex in &mut row.visuals.mesh.vertices[first..first + 4] {
        vertex.pos += delta;
    }
}

/// Recomputes `galley`'s bounds from its rows', after glyphs in them moved.
pub(crate) fn update_bounds(galley: &mut Galley) {
    galley.rect = galley
        .rows
        .iter()
        .fold(Rect::ZERO, |rect, placed| rect.union(placed.rect()));
    galley.mesh_bounds = galley.rows.iter().fold(Rect::NOTHING, |rect, placed| {
        rect.union(placed.visuals.mesh_bounds.translate(placed.pos.to_vec2()))
    });
}

/// A run of text in one style: wraps to the available width (`white-space: normal`), or with
/// [`Text::truncate`] stays on one line and ends in an ellipsis (`text-overflow: ellipsis`).
#[must_use]
pub struct Text<'a> {
    text: &'a str,
    style: TextStyle,
    color: Color32,
    truncate: bool,
}

impl<'a> Text<'a> {
    /// Black text in `style`.
    pub fn new(text: &'a str, style: TextStyle) -> Self {
        Self {
            text,
            style,
            color: Color32::BLACK,
            truncate: false,
        }
    }

    /// Sets the text color.
    pub fn color(self, color: Color32) -> Self {
        Self { color, ..self }
    }

    /// Keeps the text on one line, ending in "…" if it doesn't fit.
    pub fn truncate(self) -> Self {
        Self {
            truncate: true,
            ..self
        }
    }
}

impl Widget for Text<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let mut job =
            LayoutJob::single_section(self.text.to_owned(), self.style.format(self.color));
        let width = ui.available_width();
        job.wrap = if self.truncate {
            TextWrapping::truncate_at_width(width)
        } else {
            TextWrapping::wrap_at_width(width)
        };
        let galley = layout(ui.ctx(), job);
        let lines = galley.rows.len().max(1) as f32;
        let size = Vec2::new(galley.size().x, lines * self.style.line_height);
        let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
        paint_text(ui.painter(), rect.min, galley, &self.style);
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exo Sans: hhea ascender 0.88em, descender 0.37em.
    const SANS: VerticalMetrics = VerticalMetrics {
        ascent: 0.88,
        descent: 0.37,
    };

    #[test]
    fn unkerned_text_is_as_wide_as_chromium_lays_it_out_without_kerning() {
        let ctx = egui::Context::default();
        crate::install(&ctx).expect("the system fonts load");
        let style = TextStyle::new(Font::new(crate::Family::Sans, 400), 13.0, 28.0);
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            // The `+p` dropdown's names, unkerned (`font-kerning: none`), each cell's text range in
            // Chromium.
            for (name, chromium) in [
                ("Priya Raman", 73.3125),
                ("Nadia Petrova", 82.28125),
                ("Paul Durand", 71.734375),
                ("Carlos Mendes", 87.375),
            ] {
                let width = style.width_unkerned(ui.ctx(), name);
                assert!(
                    (width - chromium).abs() < 0.05,
                    "{name}: {width} against Chromium's {chromium}"
                );
            }
            let address = "pdurand@westbrook-realty.com";
            assert!(
                style.width(ui.ctx(), address) < style.width_unkerned(ui.ctx(), address) - 1.0,
                "kerned, it's narrower"
            );
        });
        output.drop_without_applying_deltas();
    }

    #[test]
    fn baseline_matches_chromium() {
        // Measured against Chromium rendering the same lines in a 30px line box.
        for (size, baseline) in [
            (12.0, 18.0),
            (13.0, 18.0),
            (14.0, 18.0),
            (16.0, 19.0),
            (18.0, 19.0),
            (20.0, 20.0),
        ] {
            assert_eq!(css_baseline(SANS, size, 30.0), baseline, "{size}px");
        }
    }
}
