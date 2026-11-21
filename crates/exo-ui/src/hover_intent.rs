//! Hover intent: a target counts as hovered once the pointer, polled every `interval` from when it
//! came onto the target, moved less than 7px (across plus down) between two polls. A pointer
//! sweeping across a target never settles on it.

use egui::{Context, Pos2};

/// The default poll, 100ms, and how far the pointer may move between polls, 7px.
const INTERVAL: f64 = 0.1;
const SENSITIVITY: f32 = 7.0;

/// The pointer settling on one of a kind of target (`K` tells them apart).
#[derive(Clone, Debug)]
pub struct HoverIntent<K> {
    interval: f64,
    /// What the pointer is over, where it was at the last poll and when, and whether it settled.
    over: Option<(K, Pos2, f64, bool)>,
}

impl<K> Default for HoverIntent<K> {
    fn default() -> Self {
        Self::new(INTERVAL)
    }
}

impl<K> HoverIntent<K> {
    /// Polls every `interval` seconds rather than every 100ms.
    pub fn new(interval: f64) -> Self {
        Self {
            interval,
            over: None,
        }
    }
}

impl<K: PartialEq> HoverIntent<K> {
    /// Follows the pointer over `target` (`None` over nothing) this frame. Returns the target
    /// on the frame the pointer settles on it.
    pub fn update(&mut self, ctx: &Context, target: Option<K>) -> Option<&K> {
        let (Some(target), Some(pointer)) = (target, ctx.pointer_latest_pos()) else {
            self.over = None;
            return None;
        };
        let now = ctx.input(|input| input.time);
        let interval = self.interval;
        if self.over.as_ref().is_none_or(|(over, ..)| *over != target) {
            self.over = Some((target, pointer, now, false));
            ctx.request_repaint_after_secs(interval as f32);
            return None;
        }
        let (over, polled_at, polled, settled) = self.over.as_mut()?;
        if *settled {
            return None;
        }
        if now - *polled < interval {
            ctx.request_repaint_after_secs((interval - (now - *polled)) as f32);
            return None;
        }
        if (pointer.x - polled_at.x).abs() + (pointer.y - polled_at.y).abs() >= SENSITIVITY {
            (*polled_at, *polled) = (pointer, now);
            ctx.request_repaint_after_secs(interval as f32);
            return None;
        }
        *settled = true;
        Some(over)
    }
}
