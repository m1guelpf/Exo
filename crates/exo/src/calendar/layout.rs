//! Where a day's timed events go in its column: events that overlap nest as a forest, each child
//! indented from its parent, and each block's top and height follow its times at 44px an hour.

use jiff::{SignedDuration, Timestamp, Zoned};

/// An hour's height in the grid.
pub const HOUR_HEIGHT: f32 = 44.0;
/// The shortest an event is drawn: 15 minutes.
const MIN_DURATION: SignedDuration = SignedDuration::from_mins(15);
/// How far apart a child's start must be from its parent's for tight indentation.
const HALF_HOUR: SignedDuration = SignedDuration::from_mins(30);
/// An hour.
const HOUR: SignedDuration = SignedDuration::from_hours(1);

/// A block's place in its day column.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    /// Which of the events laid out.
    pub event: usize,
    /// `top` and `height`, px from the column's top.
    pub top: f32,
    pub height: f32,
    /// `left`, a percentage of the column.
    pub left: f32,
    /// `width: calc({width}% - 2px)`.
    pub width: f32,
    /// The time goes on the title's line.
    pub time_on_same_line: bool,
}

/// An event at least 15 minutes long, and its place in the forest.
#[derive(Debug)]
struct Node {
    event: usize,
    start: Timestamp,
    end: Timestamp,
    parent: Option<usize>,
    children: Option<Vec<usize>>,
    next: Option<usize>,
    depth: usize,
    tight: bool,
    x_offset: f32,
    unoverlapped_width: f32,
    width: f32,
}

/// Lays out `events` (start, end) in the column of the day starting at `day_start` (local
/// midnight; `day_start`'s zone gives each top its wall-clock time).
pub fn day_layout(events: &[(Timestamp, Timestamp)], day_start: &Zoned) -> Vec<Placed> {
    let mut nodes: Vec<Node> = events
        .iter()
        .enumerate()
        .map(|(event, &(start, end))| Node {
            event,
            start,
            end: end.max(start + MIN_DURATION),
            parent: None,
            children: None,
            next: None,
            depth: 1,
            tight: false,
            x_offset: 0.0,
            unoverlapped_width: 0.0,
            width: 0.0,
        })
        .collect();
    let order = make_tree(&mut nodes);
    let order = promote_longer_children(&mut nodes, order);
    compute_x_offsets(&mut nodes, &order);
    order
        .iter()
        .map(|&node| place(&nodes, node, day_start))
        .collect()
}

/// The events by start, then longer first.
fn sort_by_time(nodes: &[Node]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by(|&a, &b| {
        nodes[a]
            .start
            .cmp(&nodes[b].start)
            .then(nodes[b].end.cmp(&nodes[a].end))
    });
    order
}

/// The render order: chains of events that follow each other without overlapping.
fn sort_by_render(nodes: &mut [Node], mut rest: Vec<usize>) -> Vec<usize> {
    let mut out = Vec::with_capacity(rest.len());
    while !rest.is_empty() {
        out.push(rest.remove(0));
        let mut at = 0;
        while at < rest.len() {
            let (last, candidate) = (out[out.len() - 1], rest[at]);
            if nodes[last].end <= nodes[candidate].start {
                rest.remove(at);
                out.push(candidate);
                nodes[last].next = Some(candidate);
            } else {
                at += 1;
            }
        }
    }
    out
}

/// The forest: in render order, each event adopts the later events that start inside it after
/// its previous child ends.
fn make_tree(nodes: &mut [Node]) -> Vec<usize> {
    let mut by_time = sort_by_time(nodes);
    let render = sort_by_render(nodes, by_time.clone());
    for &node in &render {
        let mut after = nodes[node].start;
        let mut at = 0;
        while at < by_time.len() && nodes[by_time[at]].start < nodes[node].end {
            let candidate = by_time[at];
            if candidate == node {
                by_time.remove(at);
            } else if after <= nodes[candidate].start {
                by_time.remove(at);
                nodes[candidate].parent = Some(node);
                nodes[node]
                    .children
                    .get_or_insert_with(Vec::new)
                    .push(candidate);
                after = nodes[candidate].end;
            } else {
                at += 1;
            }
        }
    }
    render
}

/// A root whose only child is a longer leaf swaps with it.
fn promote_longer_children(nodes: &mut [Node], mut order: Vec<usize>) -> Vec<usize> {
    let duration = |node: &Node| node.end.duration_since(node.start);
    for at in 0..order.len() {
        let root = order[at];
        let child = match &nodes[root].children {
            Some(children) if nodes[root].parent.is_none() && children.len() == 1 => children[0],
            _ => continue,
        };
        if nodes[child]
            .children
            .as_ref()
            .is_some_and(|children| !children.is_empty())
            || duration(&nodes[child]) <= duration(&nodes[root])
        {
            continue;
        }
        let Some(child_at) = order.iter().position(|&node| node == child) else {
            continue;
        };
        nodes[child].parent = None;
        nodes[child].children = Some(vec![root]);
        nodes[root].parent = Some(child);
        nodes[root].children = None;
        order[at] = child;
        order[child_at] = root;
    }
    order
}

/// Each event's depth, indentation, offset and widths.
fn compute_x_offsets(nodes: &mut [Node], order: &[usize]) {
    for &node in order.iter().rev() {
        nodes[node].depth = depth(nodes, node);
    }
    for &node in order {
        if nodes[node].parent.is_some() {
            nodes[node].tight = tight_indentation(nodes, node);
        }
        nodes[node].x_offset = x_offset(nodes, node);
        nodes[node].unoverlapped_width = unoverlapped_width(nodes, node);
        nodes[node].width = overlapped_width(nodes, node);
    }
}

fn depth(nodes: &[Node], node: usize) -> usize {
    nodes[node].children.as_ref().map_or(1, |children| {
        children
            .iter()
            .map(|&child| depth(nodes, child))
            .max()
            .unwrap_or(0)
            + 1
    })
}

/// Tight indentation: a child that starts half an hour or more after its parent tucks in under
/// it, unless something follows an ancestor too soon.
fn tight_indentation(nodes: &[Node], node: usize) -> bool {
    let Some(parent) = nodes[node].parent else {
        return false;
    };
    let start = nodes[node].start;
    if nodes[parent].start.duration_since(start).abs() < HALF_HOUR {
        return false;
    }
    let mut ancestor = Some(parent);
    while let Some(at) = ancestor {
        if let Some(next) = nodes[at].next
            && (nodes[next].start < nodes[node].end
                || nodes[next].start.duration_since(start).abs() < HALF_HOUR)
        {
            return false;
        }
        ancestor = nodes[at].parent;
    }
    true
}

fn unoverlapped_width(nodes: &[Node], node: usize) -> f32 {
    let Some(parent) = nodes[node].parent else {
        return 100.0 / nodes[node].depth as f32;
    };
    let parent_width = nodes[parent].unoverlapped_width;
    let mut width = parent_width;
    if nodes[node].tight {
        width += parent_width - 5.0;
    }
    width.min((100.0 - (nodes[parent].x_offset + 5.0)) / nodes[node].depth as f32)
}

fn overlapped_width(nodes: &[Node], node: usize) -> f32 {
    let (mut width, x) = (nodes[node].unoverlapped_width, nodes[node].x_offset);
    if nodes[node].parent.is_none() || nodes[node].children.is_some() {
        width *= 1.7;
    } else {
        width = width.max(100.0 - x);
    }
    width.clamp(100.0 / 3.0, 100.0).min(100.0 - x)
}

fn x_offset(nodes: &[Node], node: usize) -> f32 {
    let Some(parent) = nodes[node].parent else {
        return 0.0;
    };
    let parent_width = nodes[parent].unoverlapped_width;
    let mut x = nodes[parent].x_offset + parent_width;
    if nodes[node].tight {
        x -= parent_width - 5.0;
    }
    x.clamp(0.0, 90.0)
}

/// `node`'s place in the column of the day starting at `day_start`.
fn place(nodes: &[Node], node: usize, day_start: &Zoned) -> Placed {
    let at = &nodes[node];
    let visible = at.start.max(day_start.timestamp());
    let top = top_for(&visible.to_zoned(day_start.time_zone().clone()));
    let mut duration = at
        .end
        .duration_since(at.start)
        .min(at.end.duration_since(visible));
    duration = duration.max(MIN_DURATION);
    let height =
        event_height(duration.as_secs_f64() as f32 / 60.0).min(24.0 * HOUR_HEIGHT - 2.0 - top);
    let child_nearby = at
        .children
        .as_ref()
        .and_then(|children| children.first())
        .is_some_and(|&child| {
            let gap = nodes[child].start.duration_since(at.start).abs();
            gap >= HALF_HOUR && gap < HOUR && (nodes[child].x_offset - at.x_offset).abs() < 6.0
        });
    Placed {
        event: at.event,
        top,
        height,
        left: at.x_offset,
        width: if at.width == 0.0 { 100.0 } else { at.width },
        time_on_same_line: duration < SignedDuration::from_mins(45) || child_nearby,
    }
}

/// A wall-clock time's px from midnight.
pub fn top_for(time: &Zoned) -> f32 {
    ((60 * i32::from(time.hour()) + i32::from(time.minute())) as f32 * (HOUR_HEIGHT / 60.0)).round()
}

/// A block's height for a duration in minutes.
pub fn event_height(minutes: f32) -> f32 {
    ((minutes * (HOUR_HEIGHT / 60.0)).floor() - 1.0).max(11.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day() -> Zoned {
        "2026-11-17T00:00:00[America/New_York]"
            .parse()
            .expect("valid")
    }

    fn at(hour: i8, minute: i8) -> Timestamp {
        day()
            .date()
            .at(hour, minute, 0, 0)
            .to_zoned(day().time_zone().clone())
            .expect("exists")
            .timestamp()
    }

    #[test]
    fn a_lone_event_fills_the_column() {
        let placed = day_layout(&[(at(11, 0), at(11, 30)), (at(15, 0), at(16, 0))], &day());
        // 11:00–11:30 at 484px, 21 tall; 3–4 pm at 660px, 43 tall.
        assert_eq!(
            (
                placed[0].top,
                placed[0].height,
                placed[0].left,
                placed[0].width,
                placed[0].time_on_same_line
            ),
            (484.0, 21.0, 0.0, 100.0, true)
        );
        assert_eq!(
            (placed[1].top, placed[1].height, placed[1].time_on_same_line),
            (660.0, 43.0, false)
        );
    }

    #[test]
    fn overlapping_events_nest() {
        // 9–11 and 9:15–10:30: the second is the first's child, indented by its width.
        let placed = day_layout(&[(at(9, 0), at(11, 0)), (at(9, 15), at(10, 30))], &day());
        let by_event = |event| {
            placed
                .iter()
                .find(|placed| placed.event == event)
                .expect("placed")
        };
        let (parent, child) = (by_event(0), by_event(1));
        assert_eq!((parent.left, parent.width), (0.0, 85.0), "50% × 1.7");
        assert_eq!((child.left, child.width), (50.0, 50.0));
        assert!(child.top > parent.top);
        // Starting half an hour in, the child tucks under its parent.
        let placed = day_layout(&[(at(9, 0), at(11, 0)), (at(9, 30), at(10, 30))], &day());
        let child = placed
            .iter()
            .find(|placed| placed.event == 1)
            .expect("placed");
        assert_eq!((child.left, child.width), (5.0, 95.0));
    }

    #[test]
    fn a_short_root_with_a_longer_child_swaps() {
        // 9–9:30 holds 9:15–12; the longer child becomes the root.
        let placed = day_layout(&[(at(9, 0), at(9, 30)), (at(9, 15), at(12, 0))], &day());
        let long = placed
            .iter()
            .find(|placed| placed.event == 1)
            .expect("placed");
        assert_eq!(long.left, 0.0);
    }

    #[test]
    fn short_events_get_a_minimum_height_and_events_clip_to_the_day() {
        let placed = day_layout(
            &[
                (at(10, 0), at(10, 5)),
                (at(23, 0), at(23, 0) + SignedDuration::from_hours(3)),
            ],
            &day(),
        );
        assert_eq!(placed[0].height, 10.0_f32.max(11.0));
        assert_eq!(
            placed[1].height,
            24.0 * HOUR_HEIGHT - 2.0 - placed[1].top,
            "capped at the day's end"
        );
    }
}
