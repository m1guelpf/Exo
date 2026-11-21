//! Split inbox tabs: which tabs an inbox view's header shows, where `Tab`, `g i` and `g o` go,
//! and the palette's split commands.
//!
//! The tabs are the enabled splits and Important and Other (or Inbox, when they're off) in the
//! account's one order.

use std::collections::HashMap;

use exo_commands::{Command, Commands, Region};
use exo_store::{ListCount, ListId, Settings, SplitEntry, SplitLibrary, SystemSplit};

use crate::commands::Action;
use crate::settings;

/// The domain the account's Team split and template cover: its address's, unless that's a
/// webmail one.
pub(crate) fn team_domain(email: &str) -> Option<String> {
    email
        .rsplit_once('@')
        .map(|(_, domain)| domain.to_lowercase())
        .filter(|domain| !crate::format::names::is_popular_domain(domain))
}

/// The split library Exo ships: the templates a split can start from.
const BUNDLED_LIBRARY: &str = include_str!("../../../assets/split-library.json");

/// The split library, Exo's own.
pub fn load_library() -> Option<SplitLibrary> {
    BUNDLED_LIBRARY
        .parse()
        .inspect_err(|error| log::error!("the bundled split library doesn't parse: {error}"))
        .ok()
}

/// A tab of the inbox header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    /// The list it shows.
    pub list: ListId,
    /// Its name.
    pub name: String,
    /// The list's conversations, when known.
    pub count: Option<ListCount>,
    /// A `•` follows it: the first of exactly two shown tabs.
    pub bullet: bool,
    /// It hides while it's empty, and it is: it collapses out of the row.
    pub hidden: bool,
    /// What its tooltip says it is ([`describe`]).
    pub description: Option<String>,
}

impl Tab {
    /// Its tooltip's key row.
    pub fn key(&self) -> (&'static str, &'static [&'static str]) {
        match self.list {
            ListId::Important => ("Go to Important", &["G", "then", "i"]),
            ListId::Other => ("Go to Other", &["G", "then", "O"]),
            _ => ("Next Split Inbox", &["tab"]),
        }
    }
}

/// Gives `tabs` what their tooltips say they are: Important's and Other's own words; a split's
/// template's description from `library` (by its template, else its kind, `splitTemplate_{kind}`;
/// the Inbox's is `splitTemplate_inbox`); else the split's query.
pub fn describe(tabs: &mut [Tab], settings: &Settings, library: Option<&SplitLibrary>) {
    for tab in tabs {
        let split = match &tab.list {
            ListId::Split(id) => settings
                .enabled_splits()
                .into_iter()
                .find(|split| split.id == *id),
            _ => None,
        };
        let template = match (&tab.list, split) {
            (ListId::Inbox, _) => Some("splitTemplate_inbox".to_owned()),
            (_, Some(split)) => split.template_id.clone().or_else(|| {
                split
                    .kind
                    .as_ref()
                    .map(|kind| format!("splitTemplate_{kind}"))
            }),
            _ => None,
        };
        let described = template
            .and_then(|id| library?.templates.get(&id))
            .map(|template| template.description.clone())
            .filter(|text| !text.is_empty());
        tab.description = match tab.list {
            ListId::Important => Some("Direct, personal, and high priority messages.".to_owned()),
            ListId::Other => Some(
                "Split conversations from marketing, social networks, and automatic updates."
                    .to_owned(),
            ),
            _ => described
                .or_else(|| split.map(|split| split.query().to_owned()))
                .filter(|text| !text.is_empty()),
        };
    }
}

/// Every split's list count by list, as the account last read them.
pub type Counts = HashMap<ListId, ListCount>;

/// List counts show below this (`INBOX_COUNTS`).
pub(crate) const INBOX_COUNTS: usize = 1000;

/// How the tab title and the Dock badge show `count`: its conversations, with `+` while the
/// store doesn't have them all yet. None for an empty list and from [`INBOX_COUNTS`].
pub(crate) fn count_label(count: &ListCount) -> Option<String> {
    (1..INBOX_COUNTS)
        .contains(&count.threads)
        .then(|| format!("{}{}", count.threads, if count.exact { "" } else { "+" }))
}

/// Where `g i` goes: the Inbox, or Important when Important and Other are on.
pub fn home(settings: &Settings) -> ListId {
    if settings.unified_inbox {
        ListId::Inbox
    } else {
        ListId::Important
    }
}

/// Where `g o` goes: Other, or the Inbox when Important and Other are off.
pub fn other(settings: &Settings) -> ListId {
    if settings.unified_inbox {
        ListId::Inbox
    } else {
        ListId::Other
    }
}

/// An inbox view: the header shows the tabs, Add Split and the split gear.
pub fn is_inbox_view(list: &ListId) -> bool {
    matches!(
        list,
        ListId::Inbox | ListId::Important | ListId::Other | ListId::Split(_)
    )
}

/// The tabs: splits (and Other) that hide when empty are [hidden](Tab::hidden) while they're
/// known to be empty, unless they're on screen.
pub fn tabs(settings: &Settings, current: &ListId, counts: &Counts) -> Vec<Tab> {
    let hides = |entry: &SplitEntry<'_>| match entry {
        SplitEntry::Split(split) => split.hides_when_empty(),
        SplitEntry::System(system) => {
            *system == SystemSplit::Other && settings.hides_other_when_empty()
        }
    };
    let mut tabs: Vec<Tab> = settings
        .split_entries()
        .into_iter()
        .filter(|entry| !entry.is_disabled())
        .map(|entry| {
            let list = entry.list();
            let count = counts.get(&list).copied();
            let hidden =
                hides(&entry) && *current != list && count.is_some_and(|count| count.threads == 0);
            Tab {
                count,
                name: entry.name().to_owned(),
                list,
                bullet: false,
                hidden,
                description: None,
            }
        })
        .collect();
    let mut shown: Vec<&mut Tab> = tabs.iter_mut().filter(|tab| !tab.hidden).collect();
    if let [first, _] = shown.as_mut_slice() {
        first.bullet = true;
    }
    tabs
}

/// The list `Tab` (`step` 1) or ⇧Tab (-1) goes to from `current`: the next or previous tab,
/// wrapping around; from outside the inbox, the first tab.
pub fn step(tabs: &[Tab], current: &ListId, step: isize) -> Option<ListId> {
    let tabs: Vec<&Tab> = tabs.iter().filter(|tab| !tab.hidden).collect();
    let len = isize::try_from(tabs.len()).ok().filter(|len| *len > 0)?;
    let (at, step) = match tabs.iter().position(|tab| tab.list == *current) {
        Some(at) if is_inbox_view(current) => (isize::try_from(at).ok()?, step),
        _ => (0, 0),
    };
    let next = usize::try_from((at + step).rem_euclid(len)).ok()?;
    tabs.get(next).map(|tab| tab.list.clone())
}

/// Whether the header shows Add Split: in an inbox view, until a split is first turned on, and
/// not once the account `joined` (first signed in to Exo) over a week before `now`; with no join
/// date, it shows.
pub fn shows_add_split(
    settings: &Settings,
    current: &ListId,
    joined: Option<jiff::Timestamp>,
    now: &jiff::Zoned,
) -> bool {
    let week_on = joined.and_then(|joined| {
        joined
            .to_zoned(now.time_zone().clone())
            .checked_add(jiff::Span::new().days(7))
            .ok()
    });
    let member_longer = week_on.is_some_and(|week_on| *now > week_on);
    is_inbox_view(current)
        && !member_longer
        && settings.splits_engagement.first_split_enabled_at.is_none()
}

/// Where Tab and ⇧Tab switch splits: the list and its header.
const TAB_REGIONS: &[Region] = &[Region::ThreadListHeader, Region::ThreadList];
/// Where the palette's split rows are offered: the list and thread regions.
pub(crate) const PALETTE: &[Region] = &[
    Region::ThreadListHeader,
    Region::ThreadList,
    Region::ActionPane,
    Region::ThreadMessagesHeader,
    Region::ThreadMessages,
    Region::ThreadMessageCalendarInvite,
    Region::ReadOnlySnippet,
];

/// Registers the split commands for this frame: Tab and ⇧Tab, `Next Split Inbox` while there's
/// more than one tab, `go to {split} Inbox` for each enabled split once something is typed, and
/// the split settings.
pub fn register(commands: &mut Commands<Action>, settings: &Settings, visible_splits: usize) {
    // Held, the highlight moves along the tabs; the list follows when the key goes up.
    let hold = |id: &'static str, step: isize, key: &'static str| {
        Command::shortcut(id, Action::SplitTabHold(step))
            .key(key)
            .repeat(Action::SplitTabHold(step))
            .on_release(Action::SplitTabRelease)
            .regions(TAB_REGIONS)
    };
    commands.add(hold("NEXT_SPLIT_TAB", 1, "tab"));
    commands.add(hold("PREVIOUS_SPLIT_TAB", -1, "shift+tab"));
    if !settings.unified_inbox || visible_splits > 0 {
        commands.add(
            Command::new("NEXT_SPLIT", "Next Split Inbox", Action::SplitTab(1))
                .aliases(["Switch Split Inbox"])
                .key_code(["tab"])
                .default_score(1.0)
                .scale(0.8)
                .icon("data/go-to")
                .regions(TAB_REGIONS),
        );
    }
    for split in settings.enabled_splits() {
        commands.add(
            Command::new(
                split.id.clone(),
                format!("go to {} Inbox", split.name()),
                Action::Go(ListId::Split(split.id.clone())),
            )
            .operative_word(split.name().to_owned())
            .default_score(1.0)
            .icon("data/split-inbox")
            .query_only()
            .regions(PALETTE),
        );
    }
    // `VIP Split Senders` and `News Split Senders`: their templates (`template:splitTemplate_…`).
    for (id, title, template) in [
        (
            "VIP_SPLIT_SENDERS",
            "VIP Split Senders",
            "splitTemplate_vip",
        ),
        (
            "NEWS_SPLIT_SENDERS",
            "News Split Senders",
            "splitTemplate_news",
        ),
    ] {
        let open = settings::Action::Split(settings::SplitEvent::OpenTemplate(
            template.to_owned(),
            settings::SplitBack::List,
        ));
        commands.add(
            Command::new(id, title, Action::Settings(open))
                .default_score(0.8)
                .scale(0.5)
                .icon("data/settings")
                .query_only()
                .regions(PALETTE),
        );
    }
    let gear = |id: &'static str, title: &'static str, dialog: settings::Dialog| {
        Command::new(id, title, Action::Settings(settings::Action::Open(dialog)))
            .icon("data/settings")
            .scale(0.4)
            .regions(PALETTE)
    };
    commands.add(
        gear(
            "SPLIT_INBOX_SETTINGS",
            "Split Inbox Settings",
            settings::Dialog::SplitInbox,
        )
        .aliases(["Settings"])
        .operative_word("split")
        .default_score(0.5),
    );
    commands.add(
        gear(
            "HIDE_EMPTY_SPLITS_SETTINGS",
            "Hide Empty Split Inboxes",
            settings::Dialog::HideEmptySplits,
        )
        .aliases(["Hide Empty Splits Settings"])
        .default_score(0.0),
    );
    commands.add(
        Command::new(
            "SPLIT_INBOX_LIBRARY",
            "Split Inbox Library",
            Action::Settings(settings::Action::SplitLibrary),
        )
        .aliases(["Split Inbox Templates"])
        .icon("data/settings")
        .scale(0.4)
        .default_score(0.0)
        .regions(PALETTE),
    );
}

#[cfg(test)]
mod tests {
    use exo_store::{Definitions, SplitInbox};

    use super::*;

    fn split(name: &str, hide: bool) -> SplitInbox {
        let mut split = SplitInbox::new(
            name,
            "from:example.com",
            None,
            None,
            Definitions {
                labels: &[],
                me: &[],
            },
        );
        split.hide_split_when_empty = Some(hide);
        split
    }

    #[test]
    fn counts_show_with_a_plus_until_complete_and_below_a_thousand() {
        let label = |threads, exact| {
            count_label(&ListCount {
                threads,
                unread: 0,
                exact,
            })
        };
        assert_eq!(label(0, true), None);
        assert_eq!(label(12, true).as_deref(), Some("12"));
        assert_eq!(label(12, false).as_deref(), Some("12+"));
        assert_eq!(label(999, false).as_deref(), Some("999+"));
        assert_eq!(label(1000, true), None);
    }

    #[test]
    fn add_split_shows_in_the_inbox_for_an_accounts_first_week() {
        let settings = Settings::default();
        let now: jiff::Zoned = "2026-11-19T10:00[America/New_York]"
            .parse()
            .expect("a time");
        let joined = |at: &str| Some(at.parse::<jiff::Zoned>().expect("a time").timestamp());
        assert!(
            shows_add_split(&settings, &ListId::Important, None, &now),
            "no join date"
        );
        assert!(
            shows_add_split(
                &settings,
                &ListId::Important,
                joined("2026-11-12T10:00[America/New_York]"),
                &now
            ),
            "a week to the minute"
        );
        assert!(
            !shows_add_split(
                &settings,
                &ListId::Important,
                joined("2026-11-12T09:59[America/New_York]"),
                &now
            ),
            "over a week"
        );
        assert!(
            !shows_add_split(
                &settings,
                &ListId::Done,
                joined("2026-11-18T10:00[America/New_York]"),
                &now
            ),
            "not an inbox view"
        );
    }

    fn names(tabs: &[Tab]) -> Vec<(&str, bool)> {
        tabs.iter()
            .filter(|tab| !tab.hidden)
            .map(|tab| (tab.name.as_str(), tab.bullet))
            .collect()
    }

    #[test]
    fn tabs_follow_the_accounts_order() {
        let mut settings = Settings::default();
        assert_eq!(
            names(&tabs(&settings, &ListId::Important, &Counts::new())),
            [("Important", true), ("Other", false)]
        );
        let (news, receipts) = (split("News", false), split("Receipts", true));
        settings.add_split(news);
        settings.add_split(receipts.clone());
        let empty = Counts::from([(
            ListId::Split(receipts.id.clone()),
            ListCount {
                threads: 0,
                unread: 0,
                exact: true,
            },
        )]);
        // With only Important and Other shown, a new split goes before them.
        assert_eq!(
            names(&tabs(&settings, &ListId::Important, &empty)),
            [("News", false), ("Important", false), ("Other", false)]
        );
        let showing = ListId::Split(receipts.id.clone());
        assert_eq!(
            names(&tabs(&settings, &showing, &empty)).len(),
            4,
            "the split on screen stays"
        );
        settings.toggle_important_other();
        assert_eq!(
            names(&tabs(&settings, &ListId::Inbox, &empty)),
            [("Inbox", true), ("News", false)]
        );
        settings.toggle_important_other();
        settings.set_hide_other_when_empty(true);
        let empty_other = Counts::from([(
            ListId::Other,
            ListCount {
                threads: 0,
                unread: 0,
                exact: true,
            },
        )]);
        assert_eq!(
            names(&tabs(&settings, &ListId::Important, &empty_other)),
            [("Important", false), ("News", false), ("Receipts", false)]
        );
    }

    #[test]
    fn tabs_say_what_they_are_and_hidden_ones_stay_in_place() {
        let mut settings = Settings::default();
        let receipts = split("Receipts", true);
        settings.add_split(split("News", false));
        settings.add_split(receipts.clone());
        let empty = Counts::from([(
            ListId::Split(receipts.id.clone()),
            ListCount {
                threads: 0,
                unread: 0,
                exact: true,
            },
        )]);
        let mut tabs = tabs(&settings, &ListId::Important, &empty);
        describe(&mut tabs, &settings, None);
        let shown: Vec<(&str, bool, Option<&str>)> = tabs
            .iter()
            .map(|tab| (tab.name.as_str(), tab.hidden, tab.description.as_deref()))
            .collect();
        assert_eq!(
            shown,
            [
                ("News", false, Some("from:example.com")),
                ("Receipts", true, Some("from:example.com")),
                (
                    "Important",
                    false,
                    Some("Direct, personal, and high priority messages.")
                ),
                (
                    "Other",
                    false,
                    Some(
                        "Split conversations from marketing, social networks, and automatic updates."
                    )
                ),
            ]
        );
        assert_eq!(tabs[2].key(), ("Go to Important", &["G", "then", "i"][..]));
        assert_eq!(tabs[0].key(), ("Next Split Inbox", &["tab"][..]));
        assert_eq!(
            step(&tabs, &tabs[0].list, 1),
            Some(ListId::Important),
            "Tab skips the hidden tab"
        );
    }

    #[test]
    fn tab_wraps_around_and_starts_from_the_first_tab_elsewhere() {
        let mut settings = Settings::default();
        settings.add_split(split("News", false));
        let tabs = tabs(&settings, &ListId::Important, &Counts::new());
        assert_eq!(step(&tabs, &ListId::Important, 1), Some(ListId::Other));
        assert_eq!(
            step(&tabs, &ListId::Important, -1),
            Some(tabs[0].list.clone())
        );
        assert_eq!(step(&tabs, &ListId::Other, 1), Some(tabs[0].list.clone()));
        assert_eq!(step(&tabs, &ListId::Starred, 1), Some(tabs[0].list.clone()));
    }
}
