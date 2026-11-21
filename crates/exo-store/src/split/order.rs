//! `splitInboxesV2`: the splits by external id, and one order that places Important, Other and
//! Inbox among them. Every write leaves the order canonical: the shown tabs first, then the
//! splits that are off.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::SplitInbox;
use crate::ids::ListId;

/// Important, Other and Inbox, which the order places among the splits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemSplit {
    /// `SH_IMPORTANT`.
    Important,
    /// `SH_OTHER`.
    Other,
    /// `SH_INBOX`.
    Inbox,
}

impl SystemSplit {
    const ALL: [Self; 3] = [Self::Important, Self::Other, Self::Inbox];

    /// Its id in the order.
    pub fn external_id(self) -> &'static str {
        match self {
            Self::Important => "SH_IMPORTANT",
            Self::Other => "SH_OTHER",
            Self::Inbox => "SH_INBOX",
        }
    }

    fn from_external_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|system| system.external_id() == id)
    }

    /// Its name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Important => "Important",
            Self::Other => "Other",
            Self::Inbox => "Inbox",
        }
    }

    /// The list it shows.
    pub fn list(self) -> ListId {
        match self {
            Self::Important => ListId::Important,
            Self::Other => ListId::Other,
            Self::Inbox => ListId::Inbox,
        }
    }

    /// The library template that describes it.
    pub fn template_id(self) -> &'static str {
        match self {
            Self::Important | Self::Other => "splitTemplate_importantOther",
            Self::Inbox => "splitTemplate_inbox",
        }
    }
}

/// A row of the account's splits: Important, Other or Inbox, or a split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitEntry<'a> {
    /// Important, Other or Inbox.
    System(SystemSplit),
    /// A split.
    Split(&'a SplitInbox),
}

impl<'a> SplitEntry<'a> {
    /// The list it shows.
    pub fn list(&self) -> ListId {
        match self {
            Self::System(system) => system.list(),
            Self::Split(split) => ListId::Split(split.id.clone()),
        }
    }

    /// Its name.
    pub fn name(&self) -> &'a str {
        match *self {
            Self::System(system) => system.name(),
            Self::Split(split) => split.name(),
        }
    }

    /// Its id in the order.
    pub fn external_id(&self) -> &'a str {
        match *self {
            Self::System(system) => system.external_id(),
            Self::Split(split) => split.external_id.as_deref().unwrap_or_default(),
        }
    }

    /// Turned off (Important, Other and Inbox never are; they're left out instead).
    pub fn is_disabled(&self) -> bool {
        matches!(self, Self::Split(split) if split.is_disabled)
    }
}

/// `splitInboxesV2`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SplitInboxesV2 {
    /// `splitInboxes`: each split by its external id.
    pub(crate) split_inboxes: BTreeMap<String, SplitInbox>,
    /// `splitInboxesOrder`: external ids, Important's, Other's and Inbox's among them.
    pub(crate) split_inboxes_order: Vec<String>,
    /// `hideOtherWhenEmpty`: Other's tab hides while it's empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) hide_other_when_empty: Option<bool>,
}

impl SplitInboxesV2 {
    /// Never written: the account's splits aren't set up yet.
    pub(crate) fn is_unset(&self) -> bool {
        self.split_inboxes_order.is_empty()
    }

    /// `splits`, in order, with the order they first get (Inbox, Important, the splits, Other),
    /// made canonical. Splits without an external id get one.
    pub(crate) fn with_splits(splits: &[SplitInbox], unified: bool) -> Self {
        let mut v2 = Self::default();
        let mut order = vec![
            SystemSplit::Inbox.external_id().to_owned(),
            SystemSplit::Important.external_id().to_owned(),
        ];
        for split in splits {
            let mut split = split.clone();
            let id = split
                .external_id
                .get_or_insert_with(super::new_external_id)
                .clone();
            order.push(id.clone());
            v2.split_inboxes.insert(id, split);
        }
        order.push(SystemSplit::Other.external_id().to_owned());
        v2.split_inboxes_order = order;
        v2.canonicalize(unified);
        v2
    }

    /// The order's entries that exist, system ones included.
    pub(crate) fn entries(&self) -> impl Iterator<Item = SplitEntry<'_>> {
        self.split_inboxes_order
            .iter()
            .filter_map(|id| match SystemSplit::from_external_id(id) {
                Some(system) => Some(SplitEntry::System(system)),
                None => self.split_inboxes.get(id).map(SplitEntry::Split),
            })
    }

    /// The split whose list id is `id`.
    pub(crate) fn split(&self, id: &str) -> Option<&SplitInbox> {
        self.split_inboxes.values().find(|split| split.id == id)
    }

    fn split_mut(&mut self, id: &str) -> Option<&mut SplitInbox> {
        self.split_inboxes.values_mut().find(|split| split.id == id)
    }

    /// Drops the order's ids that name nothing, adds the splits it lacks, and makes it canonical
    /// ([`canonical_order`]).
    pub(crate) fn canonicalize(&mut self, unified: bool) {
        let mut order: Vec<String> = self
            .split_inboxes_order
            .iter()
            .filter(|id| {
                SystemSplit::from_external_id(id).is_some() || self.split_inboxes.contains_key(*id)
            })
            .cloned()
            .collect();
        for id in self.split_inboxes.keys() {
            if !order.contains(id) {
                order.push(id.clone());
            }
        }
        self.split_inboxes_order = canonical_order(&order, &self.split_inboxes, unified);
    }

    /// Adds `split`: an enabled split goes where new tabs go, one that's off at the end.
    pub(crate) fn add(&mut self, mut split: SplitInbox, unified: bool) {
        let id = split
            .external_id
            .get_or_insert_with(super::new_external_id)
            .clone();
        let disabled = split.is_disabled;
        self.split_inboxes.insert(id.clone(), split);
        if disabled {
            self.split_inboxes_order.push(id);
        } else {
            let at = insert_index(&self.split_inboxes_order, &self.split_inboxes, unified);
            self.split_inboxes_order
                .insert(at.min(self.split_inboxes_order.len()), id);
        }
        self.canonicalize(unified);
    }

    /// `new` takes the place of the split whose list id is `id` (they share an external id). One
    /// turned on moves to where new tabs go; one turned off goes first among those that are off,
    /// which the canonical order does.
    pub(crate) fn replace(&mut self, id: &str, new: SplitInbox, unified: bool) {
        let Some(external) = self.split(id).and_then(|old| old.external_id.clone()) else {
            return;
        };
        let turned_on = self
            .split_inboxes
            .get(&external)
            .is_some_and(|old| old.is_disabled)
            && !new.is_disabled;
        self.split_inboxes.insert(
            external.clone(),
            SplitInbox {
                external_id: Some(external.clone()),
                ..new
            },
        );
        if turned_on {
            self.split_inboxes_order.retain(|other| *other != external);
            let at = insert_index(&self.split_inboxes_order, &self.split_inboxes, unified);
            self.split_inboxes_order
                .insert(at.min(self.split_inboxes_order.len()), external);
        }
        self.canonicalize(unified);
    }

    /// Changes the split whose list id is `id` in place.
    pub(crate) fn update(&mut self, id: &str, change: impl FnOnce(&mut SplitInbox)) {
        if let Some(split) = self.split_mut(id) {
            change(split);
        }
    }

    /// Deletes the split whose list id is `id` and returns it.
    pub(crate) fn remove(&mut self, id: &str) -> Option<SplitInbox> {
        let external = self.split(id)?.external_id.clone()?;
        self.split_inboxes_order.retain(|other| *other != external);
        self.split_inboxes.remove(&external)
    }

    /// The entry `moved` takes `pushed`'s place (both external ids).
    pub(crate) fn reorder(&mut self, moved: &str, pushed: &str, unified: bool) {
        let order = &mut self.split_inboxes_order;
        let (Some(from), Some(to)) = (
            order.iter().position(|id| id == moved),
            order.iter().position(|id| id == pushed),
        ) else {
            return;
        };
        if from == to {
            return;
        }
        let id = order.remove(from);
        order.insert(to, id);
        self.canonicalize(unified);
    }

    /// The order for Important and Other turning on or off: they wrap the shown splits, or move
    /// after them and Inbox comes first.
    pub(crate) fn toggle_important_other(&mut self, unified: bool) {
        let (enabled, disabled) = split_by_enabled(&self.split_inboxes_order, &self.split_inboxes);
        let id = |system: SystemSplit| system.external_id().to_owned();
        let mut order = Vec::with_capacity(self.split_inboxes_order.len());
        if unified {
            order.push(id(SystemSplit::Inbox));
            order.extend(enabled);
            order.extend([id(SystemSplit::Important), id(SystemSplit::Other)]);
        } else {
            order.push(id(SystemSplit::Important));
            order.extend(enabled);
            order.extend([id(SystemSplit::Other), id(SystemSplit::Inbox)]);
        }
        order.extend(disabled);
        self.split_inboxes_order = order;
        self.canonicalize(unified);
    }
}

/// The order's splits, shown (in order) and off.
fn split_by_enabled(
    order: &[String],
    splits: &BTreeMap<String, SplitInbox>,
) -> (Vec<String>, Vec<String>) {
    order
        .iter()
        .filter(|id| SystemSplit::from_external_id(id).is_none())
        .cloned()
        .partition(|id| !splits.get(id).is_some_and(|split| split.is_disabled))
}

/// The canonical order: the shown splits, Important, Other and Inbox kept where they were
/// among them, then the splits that are off.
fn canonical_order(
    order: &[String],
    splits: &BTreeMap<String, SplitInbox>,
    unified: bool,
) -> Vec<String> {
    let (enabled, disabled) = split_by_enabled(order, splits);
    let index = |id: &str| order.iter().position(|other| other == id);
    // Where `id` goes among the shown splits, if it came before `before`.
    let position = |id: SystemSplit, before: SystemSplit| -> Option<usize> {
        let (at, limit) = (index(id.external_id())?, index(before.external_id())?);
        (at <= limit).then(|| {
            enabled
                .iter()
                .filter(|split| index(split).is_some_and(|i| i < at))
                .count()
        })
    };
    let id = |system: SystemSplit| system.external_id().to_owned();
    let mut out: Vec<String> = enabled.clone();
    if unified {
        out.extend([id(SystemSplit::Important), id(SystemSplit::Other)]);
        out.extend(disabled);
        let at = position(SystemSplit::Inbox, SystemSplit::Important).unwrap_or(0);
        out.insert(at, id(SystemSplit::Inbox));
    } else {
        out.push(id(SystemSplit::Inbox));
        out.extend(disabled);
        let important = position(SystemSplit::Important, SystemSplit::Inbox);
        out.insert(important.unwrap_or(0), id(SystemSplit::Important));
        match position(SystemSplit::Other, SystemSplit::Inbox) {
            None => {
                let inbox = out
                    .iter()
                    .position(|other| *other == id(SystemSplit::Inbox))
                    .unwrap_or(out.len());
                out.insert(inbox, id(SystemSplit::Other));
            }
            Some(other) => {
                let important_first = index(SystemSplit::Important.external_id())
                    < index(SystemSplit::Other.external_id());
                let after_important = important.is_none_or(|important| {
                    important < other || (important == other && important_first)
                });
                out.insert(other + usize::from(after_important), id(SystemSplit::Other));
            }
        }
    }
    out
}

/// Where a split turning on goes in `order`, after the last shown tab, but before Inbox, before
/// Other when it's last, and before Important and Other when they're the last two.
fn insert_index(order: &[String], splits: &BTreeMap<String, SplitInbox>, unified: bool) -> usize {
    let canonical = canonical_order(order, splits, unified);
    let hidden: &[SystemSplit] = if unified {
        &[SystemSplit::Important, SystemSplit::Other]
    } else {
        &[SystemSplit::Inbox]
    };
    let shown: Vec<&String> = canonical
        .iter()
        .filter(|id| match SystemSplit::from_external_id(id) {
            Some(system) => !hidden.contains(&system),
            None => splits.get(*id).is_some_and(|split| !split.is_disabled),
        })
        .collect();
    let Some(&last) = shown.last() else { return 0 };
    let at = |id: &String| {
        canonical
            .iter()
            .position(|other| other == id)
            .unwrap_or(canonical.len())
    };
    let second = shown.len().checked_sub(2).map(|i| shown[i]);
    if unified {
        return if shown.len() != 1 && last == SystemSplit::Inbox.external_id() {
            at(last)
        } else {
            at(last) + 1
        };
    }
    let io = |id: &String| {
        matches!(
            SystemSplit::from_external_id(id),
            Some(SystemSplit::Important | SystemSplit::Other)
        )
    };
    match second {
        Some(second) if io(last) && io(second) => at(second),
        _ if io(last) => at(last),
        _ => at(last) + 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::split::{Definitions, SplitInbox};

    fn split(name: &str, on: bool) -> SplitInbox {
        SplitInbox {
            is_disabled: !on,
            ..SplitInbox::new(
                name,
                "from:example.com",
                None,
                None,
                Definitions {
                    labels: &[],
                    me: &[],
                },
            )
        }
    }

    fn names(v2: &SplitInboxesV2) -> Vec<&str> {
        v2.entries().map(|entry| entry.name()).collect()
    }

    #[test]
    fn new_tabs_go_after_the_last_split_and_keep_important_and_other_together() {
        let mut v2 = SplitInboxesV2::with_splits(&[], false);
        assert_eq!(names(&v2), ["Important", "Other", "Inbox"]);
        // With only Important and Other shown, a new split goes before them.
        v2.add(split("A", true), false);
        assert_eq!(names(&v2), ["A", "Important", "Other", "Inbox"]);
        let first = v2.split_inboxes_order[0].clone();
        v2.reorder(SystemSplit::Important.external_id(), &first, false);
        assert_eq!(names(&v2), ["Important", "A", "Other", "Inbox"]);
        // Other last: before Other.
        v2.add(split("B", true), false);
        v2.add(split("Off", false), false);
        assert_eq!(names(&v2), ["Important", "A", "B", "Other", "Inbox", "Off"]);

        let mut unified = SplitInboxesV2::with_splits(&[split("A", true)], true);
        unified.add(split("B", true), true);
        assert_eq!(names(&unified), ["Inbox", "A", "B", "Important", "Other"]);
    }

    #[test]
    fn turning_a_split_on_or_off_moves_it() {
        let mut v2 = SplitInboxesV2::with_splits(
            &[split("A", true), split("B", true), split("C", false)],
            false,
        );
        let b = v2
            .entries()
            .find(|entry| entry.name() == "B")
            .and_then(|entry| match entry {
                SplitEntry::Split(split) => Some(split.clone()),
                SplitEntry::System(_) => None,
            });
        let b = b.expect("B");
        v2.replace(
            &b.id.clone(),
            SplitInbox {
                is_disabled: true,
                ..b.clone()
            },
            false,
        );
        assert_eq!(names(&v2), ["Important", "A", "Other", "Inbox", "B", "C"]);
        let c = v2
            .split_inboxes
            .values()
            .find(|split| split.name() == "C")
            .cloned()
            .expect("C");
        v2.replace(
            &c.id.clone(),
            SplitInbox {
                is_disabled: false,
                ..c
            },
            false,
        );
        assert_eq!(names(&v2), ["Important", "A", "C", "Other", "Inbox", "B"]);
    }

    #[test]
    fn important_and_other_wrap_the_splits_or_follow_them() {
        let mut v2 = SplitInboxesV2::with_splits(&[split("A", true), split("B", false)], false);
        v2.toggle_important_other(true);
        assert_eq!(names(&v2), ["Inbox", "A", "Important", "Other", "B"]);
        v2.toggle_important_other(false);
        assert_eq!(names(&v2), ["Important", "A", "Other", "Inbox", "B"]);
    }

    #[test]
    fn important_and_other_keep_their_place_among_the_splits() {
        let mut v2 = SplitInboxesV2::with_splits(&[split("A", true), split("B", true)], false);
        let a = v2
            .split_inboxes
            .values()
            .find(|split| split.name() == "A")
            .and_then(|split| split.external_id.clone())
            .expect("A");
        // Other dragged onto A: before it.
        v2.reorder(SystemSplit::Other.external_id(), &a, false);
        assert_eq!(names(&v2), ["Important", "Other", "A", "B", "Inbox"]);
        // Important dragged past B: last.
        let b = v2
            .split_inboxes
            .values()
            .find(|split| split.name() == "B")
            .and_then(|split| split.external_id.clone())
            .expect("B");
        v2.reorder(SystemSplit::Important.external_id(), &b, false);
        assert_eq!(names(&v2), ["Other", "A", "B", "Important", "Inbox"]);
    }
}
