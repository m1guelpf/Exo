//! The list's multi-selection: rows picked one by one, or a select-all from a row down (⌘A, ⌘⇧A),
//! which also takes the rows not loaded yet and so is "non-exact" until the list has loaded every
//! row.

use std::collections::HashSet;

use jiff::Timestamp;

use crate::mailbox::ThreadSummary;

/// What is selected.
#[derive(Clone, Debug, Default)]
pub(super) struct Selection {
    /// Rows selected one by one: any row without a cutoff, rows newer than it with one.
    picked: HashSet<String>,
    /// A select-all's cutoff: every row dated at or before it is selected, loaded or not, but the
    /// excluded.
    cutoff: Option<Timestamp>,
    /// Rows at or before the cutoff unselected one by one.
    excluded: HashSet<String>,
    /// The topmost selected row when the cutoff was set, and where it was: the hint names it, and
    /// says the list changed once the cutoff row moves.
    target: Option<(usize, String)>,
}

impl Selection {
    /// Whether `thread` is selected.
    pub(super) fn contains(&self, thread: &ThreadSummary) -> bool {
        if self.picked.contains(&thread.id) {
            return true;
        }
        self.cutoff.is_some_and(|cutoff| thread.received <= cutoff)
            && !self.excluded.contains(&thread.id)
    }

    /// Selects `thread`. Returns whether to hint at ⌘A: four or more rows picked one by one,
    /// without a select-all.
    pub(super) fn insert(&mut self, thread: &ThreadSummary) -> bool {
        self.excluded.remove(&thread.id);
        if self.cutoff.is_none_or(|cutoff| thread.received > cutoff) {
            self.picked.insert(thread.id.clone());
        }
        self.cutoff.is_none() && self.picked.len() >= 4
    }

    /// Unselects `thread`.
    pub(super) fn remove(&mut self, thread: &ThreadSummary) {
        self.picked.remove(&thread.id);
        if self.cutoff.is_some_and(|cutoff| thread.received <= cutoff) {
            self.excluded.insert(thread.id.clone());
        }
    }

    /// ⌘A on `rows[at]`: it and every row dated at or before it, loaded or not, keeping what was
    /// selected above it. Rows above it with its own date that weren't selected stay out.
    pub(super) fn select_from(&mut self, rows: &[&ThreadSummary], at: usize) {
        let Some(from) = rows.get(at) else { return };
        let date = from.received;
        for row in &rows[..at] {
            if row.received == date && !self.contains(row) {
                self.excluded.insert(row.id.clone());
            }
        }
        let cutoff = self.cutoff.map_or(date, |cutoff| cutoff.max(date));
        self.cutoff = Some(cutoff);
        // Rows from here down are the cutoff's now.
        let covered: HashSet<&str> = rows[at..]
            .iter()
            .filter(|row| row.received <= date)
            .map(|row| row.id.as_str())
            .collect();
        self.excluded.retain(|id| !covered.contains(id.as_str()));
        self.picked.retain(|id| !covered.contains(id.as_str()));
        self.target = rows
            .iter()
            .position(|row| self.contains(row))
            .map(|index| (index, rows[index].id.clone()));
    }

    /// Whether `thread` is the row a select-all started from.
    pub(super) fn is_cutoff(&self, thread: &ThreadSummary) -> bool {
        self.cutoff == Some(thread.received)
    }

    /// Drops everything.
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Anything is selected, or a select-all is on.
    pub(super) fn is_active(&self) -> bool {
        !self.picked.is_empty() || self.cutoff.is_some()
    }

    /// The select-all's cutoff, if one is on.
    pub(super) fn cutoff(&self) -> Option<Timestamp> {
        self.cutoff
    }

    /// Forgets rows `listed` no longer has.
    pub(super) fn retain(&mut self, listed: impl Fn(&str) -> bool) {
        self.picked.retain(|id| listed(id));
        self.excluded.retain(|id| listed(id));
    }

    /// How many conversations are selected, out of the list's `total`: with a select-all, the total
    /// less the rows excluded and the unpicked ones above the cutoff. Also whether the rows above
    /// the cutoff changed since it was set.
    pub(super) fn count(&self, rows: &[&ThreadSummary], total: usize) -> (usize, bool) {
        let Some(cutoff) = self.cutoff else {
            return (self.picked.len(), false);
        };
        let above = rows
            .iter()
            .position(|row| row.received <= cutoff)
            .unwrap_or(rows.len());
        let count = total
            .saturating_sub(self.excluded.len() + above)
            .saturating_add(self.picked.len());
        (
            count,
            self.target
                .as_ref()
                .is_none_or(|(index, _)| *index != above),
        )
    }

    /// The select-all hint's copy for `count` of `total` selected, `name` being what the list holds
    /// (`conversations`, `drafts`), when its total is `complete`.
    pub(super) fn message(
        &self,
        rows: &[&ThreadSummary],
        (count, updated): (usize, bool),
        total: usize,
        complete: bool,
        name: &str,
    ) -> String {
        if (count < total && (!self.excluded.is_empty() || !self.picked.is_empty())) || updated {
            let plus = if complete { "" } else { "+" };
            return format!("Selected {}{plus} {name}", crate::format::thousands(count));
        }
        if count == total {
            return format!("Selected all {name}");
        }
        let Some(subject) = self
            .target
            .as_ref()
            .and_then(|(_, id)| rows.iter().find(|row| row.id == *id))
            .map(|row| row.subject.as_str())
        else {
            return format!("Selected all earlier {name}");
        };
        // The subject's first 40 characters, with `…` past them.
        let shown: String = subject.chars().take(40).collect();
        let shown = if subject.chars().count() > 40 {
            format!("{shown}…")
        } else {
            shown
        };
        let shown = if shown.is_empty() || subject == "-" {
            "(no subject)".to_owned()
        } else {
            shown
        };
        format!("Selected \"{shown}\" and all earlier {name}")
    }
}
