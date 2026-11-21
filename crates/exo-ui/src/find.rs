//! Find in page (⌘F): the text on screen, searched the way Chromium's find-in-page searches a page.
//!
//! While a search is on ([`search`]), each run of text [`crate::paint_text`] paints is searched
//! as it's painted, and its matches are highlighted under it as Chromium highlights them: yellow,
//! the current match orange, the text in them black. Screens register nothing for that. What
//! one paints outside the page (the shell's tab strip) it paints inside [`unsearchable`]; what it
//! paints some other way (a message body) adds its matches at that point with [`add_matches`], or
//! says with [`pending`] that they're still to come.
//!
//! A screen that scrolls also paints what's out of view, as a page keeps it in its DOM, while
//! [`searching`], inside a [`Scope`], and scrolls by what [`reveal`] says when a search moves to a
//! match out of view, as Chromium scrolls it into view.
//!
//! Matches count in the order they're painted, which is the page's order, except where a screen
//! paints out of it: a [`Slot`] keeps a place for what's painted later. Each frame's matches are
//! settled at the start of the next ([`State::roll`]), so the count and the current match lag
//! the screen by a frame.

use std::{ops::Range, sync::Arc};

use egui::{
    Color32, Context, Galley, Id, Painter, Pos2, Rangef, Rect, Vec2, mutex::Mutex, pos2, vec2,
};

use crate::TextStyle;

/// A match's highlight (Chromium's `PlatformTextSearchHighlightColor`, inactive).
pub const MATCH: Color32 = Color32::from_rgb(255, 255, 0);
/// The current match's highlight (active).
pub const CURRENT: Color32 = Color32::from_rgb(255, 150, 50);
/// The text in a highlight (`PlatformTextSearchColor`).
pub const MATCH_TEXT: Color32 = Color32::BLACK;

/// Searches for `query` from now on, or stops searching (`None`, or empty). A new query starts
/// from the current match, keeping it if it still matches there (Chromium's
/// `kStartInSelection`).
pub fn search(ctx: &Context, query: Option<&str>) {
    let raw = query.filter(|query| !query.is_empty());
    let query = raw.map(fold_query);
    let changed = with(ctx, |state| {
        if state.query == query {
            return false;
        }
        if query.is_none() {
            state.stop();
        } else {
            state.query = query;
            state.raw = raw.map(str::to_owned);
            state.request = Some(Request::Restart);
            // What's been painted this frame was searched for the old query.
            state.stale = true;
        }
        true
    });
    if changed {
        ctx.request_repaint();
    }
}

/// Moves to the next match (or the previous), wrapping around, and scrolls it into view; with
/// none current, to the first (or the last).
pub fn step(ctx: &Context, forward: bool) {
    with(ctx, |state| {
        if state.query.is_some() {
            state.request = Some(Request::Step(forward));
        }
    });
    ctx.request_repaint();
}

/// Whether a search is on: screens that scroll paint what's out of view.
pub fn searching(ctx: &Context) -> bool {
    with(ctx, |state| state.query.is_some())
}

/// What's searched for, if anything: for text Exo doesn't paint itself to find with [`matches`].
pub fn query(ctx: &Context) -> Option<String> {
    with(ctx, |state| state.raw.clone())
}

/// The current match's place among the matches (from 1; 0 with none) and their number, once
/// the search has looked at a frame: what Chromium's `found-in-page` reports.
pub fn count(ctx: &Context) -> Option<(usize, usize)> {
    with(ctx, |state| state.query.as_ref().and(state.count))
}

/// Runs `paint` without searching what it paints: text outside the page (the tab strip).
pub fn unsearchable<R>(ctx: &Context, paint: impl FnOnce() -> R) -> R {
    with(ctx, |state| state.unsearchable += 1);
    let painted = paint();
    with(ctx, |state| {
        state.unsearchable = state.unsearchable.saturating_sub(1)
    });
    painted
}

/// A place in the page's order, kept for what's painted later (a header painted over what's
/// under it): see [`in_slot`].
#[derive(Clone, Copy, Debug)]
#[must_use]
pub struct Slot(u32);

/// Keeps the place in the page's order at this point of the frame.
pub fn slot(ctx: &Context) -> Slot {
    with(ctx, |state| {
        let slot = state.next_section + 1;
        state.next_section += 2;
        state.section = state.next_section;
        Slot(slot)
    })
}

/// Runs `paint` with what it paints counted at `slot`.
pub fn in_slot<R>(ctx: &Context, slot: Slot, paint: impl FnOnce() -> R) -> R {
    let outside = with(ctx, |state| std::mem::replace(&mut state.section, slot.0));
    let painted = paint();
    with(ctx, |state| state.section = outside);
    painted
}

/// Adds matches Exo doesn't paint as text (in a message body, which `source` names) at this
/// point in the page's order: `boxes`, on screen, in their own order. Returns which of them is
/// the current match.
pub fn add_matches(ctx: &Context, source: Id, boxes: &[Rect]) -> Option<usize> {
    with(ctx, |state| {
        if state.query.is_none() || state.unsearchable > 0 {
            return None;
        }
        let order = state.next_order();
        let scope = state.scopes.last().copied();
        let mut current = None;
        for (index, rect) in boxes.iter().enumerate() {
            let key = Key {
                scope: scope.map(|scope| scope.id),
                source: source.value(),
                index,
            };
            if state.current_key == Some(key) {
                current = Some(index);
            }
            state.found.push(Found {
                key,
                rect: *rect,
                scope,
                order: (order.0, order.1, index),
            });
        }
        current
    })
}

/// Says that what's painted at this point has matches still to come (a message body still being
/// searched): a new search waits for them before choosing its first match, so it starts where
/// the page's first match is rather than at whichever arrives first.
pub fn pending(ctx: &Context) {
    with(ctx, |state| {
        state.pending |= state.query.is_some() && state.unsearchable == 0
    });
}

/// How far (right and down, in points) to scroll `scope` before painting it, to bring the
/// current match into view the way Chromium does (`ScrollAlignment::CenterIfNeeded` both ways):
/// along each axis, not at all if it's in view, to the nearest edge if it's partly out, centred
/// if it's out. `Some` once, when a search has just moved to a match in it.
pub fn reveal(ctx: &Context, scope: Id) -> Option<Vec2> {
    let reveal = with(ctx, |state| {
        state.reveal.take_if(|reveal| reveal.scope == scope)
    })?;
    let into_view = |rect: Rangef, viewport: Rangef| {
        if rect.min >= viewport.min && rect.max <= viewport.max {
            0.0
        } else if rect.max <= viewport.min || rect.min >= viewport.max {
            rect.center() - viewport.center()
        } else if rect.min < viewport.min {
            rect.min - viewport.min
        } else {
            rect.max - viewport.max
        }
    };
    let (rect, viewport) = (reveal.rect, reveal.viewport);
    let delta = vec2(
        into_view(rect.x_range(), viewport.x_range()),
        into_view(rect.y_range(), viewport.y_range()),
    );
    (delta != Vec2::ZERO).then_some(delta)
}

/// What a scrolling screen paints, while it paints it: matches in it are its to scroll to, and
/// order by where they are in it. Closes when dropped.
#[must_use]
pub struct Scope {
    ctx: Context,
    open: bool,
}

impl Scope {
    /// Opens scope `id`, whose `viewport` on screen shows its content scrolled `offset` points
    /// down.
    pub fn begin(ctx: &Context, id: Id, viewport: Rect, offset: f32) -> Self {
        let open = with(ctx, |state| {
            let open = state.query.is_some();
            if open {
                state.scopes.push(ScopeFrame {
                    id,
                    viewport,
                    offset,
                });
            }
            open
        });
        Self {
            ctx: ctx.clone(),
            open,
        }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        if self.open {
            with(&self.ctx, |state| state.scopes.pop());
        }
    }
}

/// The byte ranges of `text` that match `query`, left to right without overlapping, compared
/// as Chromium's find compares text: ignoring case and diacritics, a no-break space as a space.
pub fn matches(text: &str, query: &str) -> Vec<Range<usize>> {
    let query = fold_query(query);
    found_in(text, &query)
        .into_iter()
        .map(|range| range.bytes)
        .collect()
}

/// A match of a folded query in a text.
struct Matched {
    bytes: Range<usize>,
    chars: Range<usize>,
}

fn fold_query(query: &str) -> Vec<char> {
    fold(query).into_iter().map(|folded| folded.ch).collect()
}

fn found_in(text: &str, query: &[char]) -> Vec<Matched> {
    if query.is_empty() {
        return Vec::new();
    }
    let folded = fold(text);
    let mut found = Vec::new();
    let mut at = 0;
    while at + query.len() <= folded.len() {
        if folded[at..at + query.len()]
            .iter()
            .map(|folded| folded.ch)
            .eq(query.iter().copied())
        {
            let (first, last) = (&folded[at], &folded[at + query.len() - 1]);
            found.push(Matched {
                bytes: first.bytes.start..last.bytes.end,
                chars: first.char..last.char + 1,
            });
            at += query.len();
        } else {
            at += 1;
        }
    }
    // A character that folds to several can't be matched in part.
    found.dedup_by(|later, earlier| later.chars.start < earlier.chars.end);
    found
}

/// One character of folded text, and the character of the text it comes from.
struct Folded {
    ch: char,
    /// That character's index and bytes.
    char: usize,
    bytes: Range<usize>,
}

/// `text` without case or diacritics (each character decomposed, its combining marks dropped,
/// lowercased), and a no-break space as a space.
fn fold(text: &str) -> Vec<Folded> {
    let mut folded = Vec::with_capacity(text.len());
    for (index, (at, ch)) in text.char_indices().enumerate() {
        let bytes = at..at + ch.len_utf8();
        let ch = if ch == '\u{a0}' { ' ' } else { ch };
        unicode_normalization::char::decompose_canonical(ch, |part| {
            if !unicode_normalization::char::is_combining_mark(part) {
                folded.extend(part.to_lowercase().map(|ch| Folded {
                    ch,
                    char: index,
                    bytes: bytes.clone(),
                }));
            }
        });
    }
    folded
}

/// Searches a run of text [`crate::paint_text`] is about to paint at `origin`, and paints its
/// matches' highlights under it, the text in them black.
pub(crate) fn text(painter: &Painter, origin: Pos2, galley: &mut Arc<Galley>, style: &TextStyle) {
    let ctx = painter.ctx();
    if !painter.is_visible()
        || galley
            .job
            .sections
            .iter()
            .all(|section| section.format.color.a() == 0)
    {
        return;
    }
    let Some((query, order, scope)) = with(ctx, |state| {
        let query = state.query.clone().filter(|_| state.unsearchable == 0)?;
        Some((query, state.next_order(), state.scopes.last().copied()))
    }) else {
        return;
    };
    let found = found_in(galley.text(), &query);
    if found.is_empty() {
        return;
    }
    let metrics = crate::metrics(ctx, style.font.face().0);
    let ascent = (metrics.ascent * style.size).round();
    let descent = (metrics.descent * style.size).round();
    let ppp = painter.pixels_per_point();
    let snap = |value: f32| (value * ppp).round() / ppp;
    // Where the run is in what scrolls, which doesn't change as it scrolls.
    let content = origin + egui::vec2(0.0, scope.map_or(0.0, |scope| scope.offset));
    let source = Id::new((
        galley.text(),
        content.x.round() as i32,
        content.y.round() as i32,
    ))
    .value();
    let pieces: Vec<Vec<(usize, Range<usize>, Rect)>> = found
        .iter()
        .map(|matched| pieces(galley, &matched.chars, origin, ascent, descent, &snap))
        .collect();
    let current = with(ctx, |state| {
        let mut current = None;
        for (index, pieces) in pieces.iter().enumerate() {
            let key = Key {
                scope: scope.map(|scope| scope.id),
                source,
                index,
            };
            if state.current_key == Some(key) {
                current = Some(index);
            }
            let rect = pieces
                .iter()
                .map(|(_, _, rect)| *rect)
                .reduce(Rect::union)
                .unwrap_or_else(|| Rect::from_min_size(origin, egui::Vec2::ZERO));
            state.found.push(Found {
                key,
                rect,
                scope,
                order: (order.0, order.1, index),
            });
        }
        current
    });
    let galley = Arc::make_mut(galley);
    for (index, pieces) in pieces.iter().enumerate() {
        let color = if current == Some(index) {
            CURRENT
        } else {
            MATCH
        };
        for (row, glyphs, rect) in pieces {
            if rect.width() > 0.0 {
                painter.rect_filled(*rect, 0.0, color);
            }
            recolor(&mut galley.rows[*row], glyphs.clone(), MATCH_TEXT);
        }
    }
}

/// The highlight of characters `chars` of `galley` (painted at `origin`): a box per row, over
/// the font's content area (`ascent` above the baseline, `descent` below, as Chromium bounds a
/// text fragment), with the row and the glyphs in it. A match in text cut off by an ellipsis
/// is a box as wide as nothing where it was cut.
fn pieces(
    galley: &Galley,
    chars: &Range<usize>,
    origin: Pos2,
    ascent: f32,
    descent: f32,
    snap: &impl Fn(f32) -> f32,
) -> Vec<(usize, Range<usize>, Rect)> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let last = galley.rows.len().saturating_sub(1);
    for (index, placed) in galley.rows.iter().enumerate() {
        // An elided galley's last glyph is the ellipsis, which stands for no character.
        let shown = placed.glyphs.len()
            - usize::from(galley.elided && index == last && !placed.glyphs.is_empty());
        let (from, to) = (chars.start.max(start), chars.end.min(start + shown));
        let baseline = |glyph: usize| placed.glyphs.get(glyph).map_or(0.0, |glyph| glyph.pos.y);
        if from < to {
            let x = |column: usize| {
                snap(
                    origin.x
                        + placed.pos.x
                        + placed.x_offset(egui::text::CharIndex(column - start)),
                )
            };
            let y = origin.y + placed.pos.y + baseline(from - start);
            let rect = Rect::from_min_max(
                pos2(x(from), snap(y - ascent)),
                pos2(x(to), snap(y + descent)),
            );
            pieces.push((index, from - start..to - start, rect));
        } else if index == last
            && galley.elided
            && chars.start >= start + shown
            && pieces.is_empty()
        {
            let x = origin.x + placed.pos.x + placed.x_offset(egui::text::CharIndex(shown));
            let y = origin.y + placed.pos.y + baseline(0);
            pieces.push((
                index,
                0..0,
                Rect::from_min_max(pos2(x, y - ascent), pos2(x, y + descent)),
            ));
        }
        start += placed.char_count_including_newline().0;
    }
    pieces
}

/// Paints glyphs `glyphs` of `placed` in `color`.
fn recolor(placed: &mut egui::epaint::text::PlacedRow, glyphs: Range<usize>, color: Color32) {
    if glyphs.is_empty() {
        return;
    }
    let row = Arc::make_mut(&mut placed.row);
    for index in glyphs {
        let glyph = &row.glyphs[index];
        // A glyph egui draws nothing for has no quad: its `first_vertex` is the next glyph's.
        let first = glyph.first_vertex as usize;
        if glyph.uv_rect.is_nothing()
            || glyph.uv_rect.size.x <= 0.0
            || !row.visuals.glyph_vertex_range.contains(&first)
        {
            continue;
        }
        for vertex in &mut row.visuals.mesh.vertices[first..first + 4] {
            vertex.color = color;
        }
    }
}

/// What identifies a match from one frame to the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    scope: Option<Id>,
    /// The run of text (its text and where it is) or the body it's in.
    source: u64,
    /// Which of its matches.
    index: usize,
}

/// A match painted this frame.
#[derive(Clone, Copy, Debug)]
struct Found {
    key: Key,
    rect: Rect,
    scope: Option<ScopeFrame>,
    /// Where it is in the page: its section ([`Slot`]), its run, its place in the run.
    order: (u32, u32, usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ScopeFrame {
    id: Id,
    viewport: Rect,
    offset: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Request {
    /// A new query: the first match from the current one on.
    Restart,
    Step(bool),
}

/// A scroll to make, for the current match.
#[derive(Clone, Copy, Debug)]
struct Reveal {
    scope: Id,
    rect: Rect,
    viewport: Rect,
}

#[derive(Default)]
struct State {
    /// The search, folded, and as it was asked.
    query: Option<Vec<char>>,
    raw: Option<String>,
    /// The frame `found` is from.
    pass: u64,
    found: Vec<Found>,
    /// This frame's are for an older query.
    stale: bool,
    /// Some of this frame's are still to come ([`pending`]).
    pending: bool,
    /// The last frame's matches, in order, and which is current.
    settled: Vec<Found>,
    current: Option<usize>,
    current_key: Option<Key>,
    count: Option<(usize, usize)>,
    request: Option<Request>,
    reveal: Option<Reveal>,
    scopes: Vec<ScopeFrame>,
    unsearchable: usize,
    /// The section text is painted in, the last one handed out, and the runs searched.
    section: u32,
    next_section: u32,
    runs: u32,
}

impl State {
    /// Stops searching, forgetting the matches.
    fn stop(&mut self) {
        (self.query, self.raw) = (None, None);
        self.found.clear();
        self.settled.clear();
        (self.current, self.current_key, self.count) = (None, None, None);
        (self.request, self.reveal, self.stale, self.pending) = (None, None, false, false);
    }

    /// Where the next run is in the page's order.
    fn next_order(&mut self) -> (u32, u32) {
        self.runs += 1;
        (self.section, self.runs)
    }

    /// Starts frame `pass`, settling the last frame's matches.
    fn roll(&mut self, pass: u64) -> bool {
        if pass == self.pass {
            return false;
        }
        self.pass = pass;
        let found = std::mem::take(&mut self.found);
        let stale = std::mem::take(&mut self.stale);
        let pending = std::mem::take(&mut self.pending);
        (self.section, self.next_section, self.runs) = (0, 0, 0);
        self.scopes.clear();
        self.unsearchable = 0;
        if self.query.is_none() {
            return false;
        }
        if stale {
            // This frame searches for the new query; the next one settles it.
            return true;
        }
        if pending && self.request == Some(Request::Restart) {
            // A new query waits for all of the page's matches (what's still to come repaints).
            return false;
        }
        let before = (self.current_key, self.count);
        self.settle(found);
        before != (self.current_key, self.count)
    }

    fn settle(&mut self, mut found: Vec<Found>) {
        found.sort_by_key(|found| found.order);
        let total = found.len();
        let kept = self
            .current_key
            .and_then(|key| found.iter().position(|found| found.key == key));
        let held = kept.or_else(|| {
            self.current
                .map(|current| current.min(total.saturating_sub(1)))
        });
        let request = self.request.take();
        let current = if total == 0 {
            None
        } else {
            match request {
                Some(Request::Step(forward)) => Some(match held {
                    Some(at) if forward => (at + 1) % total,
                    Some(at) => (at + total - 1) % total,
                    None if forward => 0,
                    None => total - 1,
                }),
                Some(Request::Restart) => {
                    let from = self
                        .current
                        .and_then(|current| self.settled.get(current))
                        .map(|was| was.order);
                    Some(
                        from.and_then(|from| found.iter().position(|found| found.order >= from))
                            .unwrap_or(0),
                    )
                }
                None => held,
            }
        };
        self.current = current;
        self.current_key = current.map(|at| found[at].key);
        self.count = Some((current.map_or(0, |at| at + 1), total));
        if request.is_some()
            && let Some(at) = current
            && let Some(scope) = found[at].scope
        {
            self.reveal = Some(Reveal {
                scope: scope.id,
                rect: found[at].rect,
                viewport: scope.viewport,
            });
        }
        self.settled = found;
    }
}

/// Runs `f` on the search's state, starting a new frame's first.
fn with<R>(ctx: &Context, f: impl FnOnce(&mut State) -> R) -> R {
    let id = crate::id(ctx, "exo_ui::find");
    let state = ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Arc<Mutex<State>>>(id)
            .clone()
    });
    let mut state = state.lock();
    if state.roll(ctx.cumulative_pass_nr()) {
        ctx.request_repaint();
    }
    f(&mut state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_ignore_case_and_diacritics() {
        assert_eq!(
            matches("Café au lait, CAFE noir", "cafe"),
            vec![0..5, 15..19]
        );
        assert_eq!(matches("Résumé", "resume"), vec![0..8]);
        assert_eq!(matches("naïve", "NAÏVE"), vec![0..6]);
        assert_eq!(
            matches("a\u{a0}b", "a b"),
            vec![0..4],
            "a no-break space is a space"
        );
    }

    #[test]
    fn matches_dont_overlap() {
        assert_eq!(matches("aaaa", "aa"), vec![0..2, 2..4]);
        assert_eq!(matches("abc", ""), Vec::<Range<usize>>::new());
        assert_eq!(matches("abc", "abcd"), Vec::<Range<usize>>::new());
    }

    fn found(order: u32, source: u64) -> Found {
        let key = Key {
            scope: None,
            source,
            index: 0,
        };
        Found {
            key,
            rect: Rect::from_min_size(pos2(0.0, order as f32 * 20.0), egui::vec2(10.0, 10.0)),
            scope: None,
            order: (0, order, 0),
        }
    }

    fn settled(state: &mut State, found: Vec<Found>) -> Option<(usize, usize)> {
        state.settle(found);
        state.count
    }

    #[test]
    fn a_search_starts_at_the_first_match_and_steps_around() {
        let mut state = State {
            query: Some(vec!['a']),
            request: Some(Request::Restart),
            ..State::default()
        };
        let page = || vec![found(1, 1), found(2, 2), found(3, 3)];
        assert_eq!(settled(&mut state, page()), Some((1, 3)));
        state.request = Some(Request::Step(true));
        assert_eq!(settled(&mut state, page()), Some((2, 3)));
        state.request = Some(Request::Step(false));
        assert_eq!(settled(&mut state, page()), Some((1, 3)));
        state.request = Some(Request::Step(false));
        assert_eq!(
            settled(&mut state, page()),
            Some((3, 3)),
            "back from the first wraps to the last"
        );
        state.request = Some(Request::Step(true));
        assert_eq!(
            settled(&mut state, page()),
            Some((1, 3)),
            "on from the last wraps to the first"
        );
    }

    #[test]
    fn the_current_match_follows_its_text_as_others_come_and_go() {
        let mut state = State {
            query: Some(vec!['a']),
            request: Some(Request::Step(true)),
            ..State::default()
        };
        settled(&mut state, vec![found(1, 1), found(2, 2)]);
        state.request = Some(Request::Step(true));
        assert_eq!(
            settled(&mut state, vec![found(1, 1), found(2, 2)]),
            Some((2, 2))
        );
        // A match painted before it appears (a row scrolled within reach).
        assert_eq!(
            settled(&mut state, vec![found(0, 9), found(1, 1), found(2, 2)]),
            Some((3, 3))
        );
        assert!(state.reveal.is_none(), "nothing moved the search");
    }

    #[test]
    fn a_new_query_keeps_to_the_current_match_or_moves_on_from_it() {
        let mut state = State {
            query: Some(vec!['a']),
            request: Some(Request::Restart),
            ..State::default()
        };
        settled(&mut state, vec![found(1, 1), found(2, 2), found(3, 3)]);
        state.request = Some(Request::Step(true));
        assert_eq!(
            settled(&mut state, vec![found(1, 1), found(2, 2), found(3, 3)]),
            Some((2, 3))
        );
        // The longer query no longer matches the second run, but does the first and third.
        state.request = Some(Request::Restart);
        assert_eq!(
            settled(&mut state, vec![found(1, 11), found(3, 13)]),
            Some((2, 2))
        );
        state.request = Some(Request::Restart);
        assert_eq!(
            settled(&mut state, vec![found(1, 21)]),
            Some((1, 1)),
            "past the last, the first"
        );
        state.request = Some(Request::Restart);
        assert_eq!(settled(&mut state, vec![]), Some((0, 0)));
    }

    #[test]
    fn a_new_query_waits_for_matches_still_to_come() {
        let mut state = State {
            query: Some(vec!['a']),
            request: Some(Request::Restart),
            ..State::default()
        };
        // A body above the second run is still being searched.
        (state.found, state.pending) = (vec![found(2, 2)], true);
        state.roll(1);
        assert_eq!(
            (state.count, state.request),
            (None, Some(Request::Restart)),
            "not settled yet"
        );
        state.found = vec![found(1, 1), found(2, 2)];
        state.roll(2);
        assert_eq!(state.count, Some((1, 2)), "from the page's first match");
    }

    #[test]
    fn painted_text_is_searched_and_highlighted_under_it() {
        use egui::{Shape, epaint::TextShape};
        let ctx = Context::default();
        crate::install(&ctx).expect("the system fonts load");
        let style = TextStyle::new(crate::Font::new(crate::Family::Sans, 400), 14.0, 20.0);
        let frame = |ctx: &Context| {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                style.paint_line(
                    ui.painter(),
                    pos2(10.0, 10.0),
                    "One agenda, two agendas",
                    Color32::WHITE,
                    None,
                );
                unsearchable(ui.ctx(), || {
                    style.paint_line(
                        ui.painter(),
                        pos2(10.0, 40.0),
                        "agenda",
                        Color32::WHITE,
                        None,
                    )
                });
            });
            let shapes = std::mem::take(&mut output.shapes);
            output.drop_without_applying_deltas();
            shapes
                .into_iter()
                .map(|clipped| clipped.shape)
                .collect::<Vec<Shape>>()
        };
        search(&ctx, Some("AGENDA"));
        // A frame searched for the new query, one settled it; the next paints it.
        frame(&ctx);
        frame(&ctx);
        let shapes = frame(&ctx);
        assert_eq!(
            count(&ctx),
            Some((1, 2)),
            "the unsearchable run isn't counted"
        );
        let fills: Vec<Color32> = shapes
            .iter()
            .filter_map(|shape| {
                if let Shape::Rect(rect) = shape {
                    Some(rect.fill)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(fills, [CURRENT, MATCH], "the first is current");
        let text: Vec<&TextShape> = shapes
            .iter()
            .filter_map(|shape| {
                if let Shape::Text(text) = shape {
                    Some(text)
                } else {
                    None
                }
            })
            .collect();
        let colors: Vec<Color32> = text[0].galley.rows[0]
            .visuals
            .mesh
            .vertices
            .iter()
            .map(|vertex| vertex.color)
            .collect();
        assert!(
            colors.contains(&MATCH_TEXT) && colors.contains(&Color32::WHITE),
            "the matches' glyphs are black, the rest as painted"
        );
        step(&ctx, true);
        frame(&ctx);
        let shapes = frame(&ctx);
        let fills: Vec<Color32> = shapes
            .iter()
            .filter_map(|shape| {
                if let Shape::Rect(rect) = shape {
                    Some(rect.fill)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            (fills.as_slice(), count(&ctx)),
            ([MATCH, CURRENT].as_slice(), Some((2, 2)))
        );
        search(&ctx, None);
        assert!(
            frame(&ctx)
                .iter()
                .all(|shape| !matches!(shape, Shape::Rect(_))),
            "no search, no highlights"
        );
    }

    #[test]
    fn reveal_scrolls_as_chromium_scrolls_a_match_into_view() {
        let ctx = Context::default();
        let viewport = Rect::from_min_max(pos2(0.0, 100.0), pos2(100.0, 500.0));
        let scope = Id::new("list");
        let reveal = |left: f32, top: f32| {
            with(&ctx, |state| {
                state.reveal = Some(Reveal {
                    scope,
                    rect: Rect::from_min_size(pos2(left, top), vec2(10.0, 20.0)),
                    viewport,
                })
            });
            super::reveal(&ctx, scope)
        };
        assert_eq!(reveal(0.0, 200.0), None, "in view");
        assert_eq!(
            reveal(0.0, 490.0),
            Some(vec2(0.0, 10.0)),
            "partly below: to the bottom edge"
        );
        assert_eq!(
            reveal(0.0, 90.0),
            Some(vec2(0.0, -10.0)),
            "partly above: to the top edge"
        );
        assert_eq!(
            reveal(0.0, 1000.0),
            Some(vec2(0.0, 1010.0 - 300.0)),
            "out of view: centred"
        );
        assert_eq!(
            reveal(95.0, 200.0),
            Some(vec2(5.0, 0.0)),
            "partly right: to the right edge"
        );
        assert_eq!(
            reveal(400.0, 1000.0),
            Some(vec2(405.0 - 50.0, 1010.0 - 300.0)),
            "out of view both ways"
        );
        assert_eq!(super::reveal(&ctx, scope), None, "once");
    }
}
