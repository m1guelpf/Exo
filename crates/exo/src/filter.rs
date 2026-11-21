//! List filters: Unread, Starred, Important and No reply, one at a time, toggled with
//! ⇧U/⇧S/⇧I/⇧R or the Filter Conversations picker, shown as a tab in the header. In search
//! they're `is:` tokens in the query instead.

use std::collections::HashMap;

use exo_commands::{Command, Commands, Region, SubPalette};
use exo_store::{Filter, ListId};

use crate::{commands, lists};

/// What a filter command does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Turn this filter on, or off when it's on.
    Toggle(Filter),
    /// Turn the filter off.
    Clear,
    /// Open the picker (`Filter Conversations`).
    Picker,
}

/// Where the filter keys work: the list and thread regions and the folder drawer.
const KEYS: &[Region] = &[
    Region::ThreadListHeader,
    Region::ThreadList,
    Region::LabelSwitcher,
    Region::ActionPane,
    Region::ThreadMessagesHeader,
    Region::ThreadMessages,
    Region::ThreadMessageCalendarInvite,
    Region::ReadOnlySnippet,
];

/// The filters in the picker's order, with their names and keys.
const FILTERS: [(Filter, &str, &str, &str); 4] = [
    (Filter::Unread, "Unread", "shift+u", "U"),
    (Filter::Starred, "Starred", "shift+s", "S"),
    (Filter::Important, "Important", "shift+i", "I"),
    (Filter::NoReply, "No reply", "shift+r", "R"),
];

/// The filter's tab and picker name.
pub(crate) fn name(filter: Filter) -> &'static str {
    FILTERS
        .iter()
        .find(|(candidate, ..)| *candidate == filter)
        .map_or("", |(_, name, ..)| name)
}

/// The token search opens with under this filter (none for No reply).
pub(crate) fn search_suffix(filter: Filter) -> Option<&'static str> {
    match filter {
        Filter::Unread => Some("is:unread"),
        Filter::Starred => Some("is:starred"),
        Filter::Important => Some("is:important"),
        Filter::NoReply => None,
    }
}

/// The search with the filter's `is:` token removed if it has it, else added.
pub(crate) fn toggle_in_search(raw: &str, filter: Filter) -> String {
    let token = match filter {
        Filter::Important => "is:important",
        Filter::Unread => "is:unread",
        Filter::Starred => "is:starred",
        Filter::NoReply => "is:sh_no_reply",
    };
    match raw.find(token) {
        Some(at) => {
            let rest = raw[at + token.len()..].trim_start();
            format!("{}{rest}", &raw[..at]).trim().to_owned()
        }
        None => [raw.trim(), token].join(" ").trim().to_owned(),
    }
}

/// Whether `filter` applies to `list`: Important doesn't filter Important or Other, and Starred
/// doesn't filter Starred (unless it's what's on).
fn applies(list: &ListId, filter: Filter, current: Option<Filter>) -> bool {
    !(filter == Filter::Important && matches!(list, ListId::Important | ListId::Other))
        && !(filter == Filter::Starred
            && *list == ListId::Starred
            && current != Some(Filter::Starred))
}

/// The filter a list opens with: the current one between inbox tabs, else the one it last had.
pub(crate) fn for_list(
    from: &ListId,
    to: &ListId,
    current: Option<Filter>,
    last: &HashMap<ListId, Filter>,
) -> Option<Filter> {
    if lists::is_inbox(from) && lists::is_inbox(to) {
        current
    } else {
        last.get(to).copied()
    }
}

/// Registers the filter keys and the palette's Filter Conversations for the list on screen;
/// `filterable` is false in a thread.
pub(crate) fn register(commands: &mut Commands<commands::Action>, filterable: bool) {
    if !filterable {
        return;
    }
    let act = |action: Action| commands::Action::Filter(action);
    for (filter, _, key, _) in FILTERS {
        commands.add(
            Command::shortcut(key, act(Action::Toggle(filter)))
                .key(key)
                .regions(KEYS),
        );
    }
    commands.add(
        Command::new("FILTER", "Filter Conversations", act(Action::Picker))
            .icon("data/filter")
            .default_score(1.57)
            .regions(KEYS),
    );
}

/// The Filter Conversations picker, without an input: the filters this list takes, the one on
/// marked, and Clear filter when one is.
pub(crate) fn picker(list: &ListId, current: Option<Filter>) -> SubPalette<commands::Action> {
    let rows = FILTERS
        .iter()
        .filter(|(filter, ..)| applies(list, *filter, current))
        .map(|(filter, title, key, letter)| {
            let row = Command::new(
                *key,
                *title,
                commands::Action::Filter(Action::Toggle(*filter)),
            )
            .key_code(["shift", letter]);
            if current == Some(*filter) {
                row.in_use()
            } else {
                row
            }
        })
        .collect();
    let palette =
        SubPalette::new("FILTER", "Filter Conversations", "data/filter", rows).hide_input();
    if current.is_some() {
        palette.footer(|_| {
            Some(Command::new(
                "CLEAR_FILTER",
                "Clear filter",
                commands::Action::Filter(Action::Clear),
            ))
        })
    } else {
        palette
    }
}

impl crate::Mail {
    /// Carries out a filter command.
    pub(crate) fn filter_run(&mut self, action: Action, now: f64) {
        match action {
            Action::Picker => {
                let picker = picker(&self.list, self.filter);
                self.commands.open_sub_palette(picker);
            }
            Action::Clear => self.filter_set(None),
            Action::Toggle(filter)
                if self.thread.is_none() && applies(&self.list, filter, self.filter) =>
            {
                self.folders.close();
                if let Some(raw) = self
                    .search
                    .as_ref()
                    .map(|search| search.query().unwrap_or_default().to_owned())
                {
                    let query = toggle_in_search(&raw, filter);
                    self.search_enter(Some(&query), now);
                    return;
                }
                self.filter_set(if self.filter == Some(filter) {
                    None
                } else {
                    Some(filter)
                });
            }
            Action::Toggle(_) => {}
        }
    }

    /// Narrows the list with `filter` (or shows all of it) and reloads it.
    pub(crate) fn filter_set(&mut self, filter: Option<Filter>) {
        if filter == self.filter {
            return;
        }
        self.filter = filter;
        match filter {
            Some(filter) => self.last_filters.insert(self.list.clone(), filter),
            None => self.last_filters.remove(&self.list),
        };
        self.account.filter(filter);
        self.rows_from = None;
        self.asked_more_at = None;
        self.inbox.clear();
    }

    /// The search bar's text with the filter's token, for search opening over the list.
    pub(crate) fn filter_search_suffix(&self) -> Option<&'static str> {
        self.filter.and_then(search_suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_toggles_the_filters_token() {
        assert_eq!(
            toggle_in_search("budget", Filter::Unread),
            "budget is:unread"
        );
        assert_eq!(
            toggle_in_search("budget is:unread", Filter::Unread),
            "budget"
        );
        assert_eq!(
            toggle_in_search("is:unread budget", Filter::Unread),
            "budget"
        );
        assert_eq!(toggle_in_search("", Filter::Important), "is:important");
        assert_eq!(toggle_in_search("x", Filter::NoReply), "x is:sh_no_reply");
    }

    #[test]
    fn filters_apply_where_they_narrow_and_carry_between_lists() {
        assert!(!applies(&ListId::Important, Filter::Important, None));
        assert!(!applies(&ListId::Starred, Filter::Starred, None));
        assert!(applies(
            &ListId::Starred,
            Filter::Starred,
            Some(Filter::Starred)
        ));
        assert!(applies(&ListId::Inbox, Filter::Important, None));
        let last = HashMap::from([(ListId::Sent, Filter::Starred)]);
        assert_eq!(
            for_list(
                &ListId::Important,
                &ListId::Other,
                Some(Filter::Unread),
                &last
            ),
            Some(Filter::Unread)
        );
        assert_eq!(
            for_list(&ListId::Inbox, &ListId::Sent, Some(Filter::Unread), &last),
            Some(Filter::Starred)
        );
        assert_eq!(
            for_list(&ListId::Sent, &ListId::Done, Some(Filter::Starred), &last),
            None
        );
    }
}
