//! Exo's links: where a tab is, as `exo://{email}/…`, for the strip's copy button and ⌃/; and
//! back, for the links macOS, another launch or a message hands Exo, which open in the tab for
//! the account they name. Deep links (`deeplink/{name}`, [`crate::deep_links`]) share the
//! scheme.
//!
//! A path is the account (`/{email}`, its home), then a split (`/inbox/{slug}`, Other's
//! `/inbox/other`), a label or folder (`/label/{slug}`), search (`/search/{query}`) or See Your
//! Week (`/calendar/week/{YYYY-MM-DD}`); then the conversation open (`/thread/{id}`), and a list's
//! filter (`?filters=unread`). Any other path, which no Exo link has, opens the account's home.

use std::sync::LazyLock;

use exo_commands::{Command, Commands, Hint};
use exo_store::{Filter, Label, ListId, Settings};
use jiff::{ToSpan, civil::Date};
use regex::Regex;

use crate::{
    commands,
    format::urls::encode_component,
    lists,
    tabs::{Route, View},
};

/// Exo's URL scheme.
pub const SCHEME: &str = "exo:";

/// A link, parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// The account it names, if it names one.
    pub email: Option<String>,
    /// Where it goes.
    pub to: To,
}

/// Where a link goes, before the account's labels and splits say what its slugs name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum To {
    /// A deep link's name (`deeplink`).
    DeepLink(String),
    /// A list, with a conversation open and a filter on if it says so.
    List {
        list: Slug,
        thread: Option<String>,
        filter: Option<Filter>,
    },
    /// Search for `query` (empty: the search bar alone), with a conversation open if it says so.
    Search {
        query: String,
        thread: Option<String>,
    },
    /// See Your Week around a day, or else today.
    Week(Option<Date>),
}

/// The list a path names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Slug {
    /// The account's home.
    Home,
    /// A label or folder, by slug or id.
    Label(String),
    /// A split, by slug.
    Split(String),
}

impl To {
    fn home() -> Self {
        Self::List {
            list: Slug::Home,
            thread: None,
            filter: None,
        }
    }
}

/// `url` as one of Exo's links, if it's on Exo's scheme: `exo:`, then `//` and the path,
/// percent-decoded, and its query.
pub fn parse(url: &str) -> Option<Link> {
    let rest = url
        .get(..SCHEME.len())
        .filter(|scheme| scheme.eq_ignore_ascii_case(SCHEME))
        .map(|_| &url[SCHEME.len()..])?;
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    let rest = rest.split('#').next().unwrap_or_default();
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut segments: Vec<String> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(decode)
        .collect();
    let deep_link = |segments: &[String]| match segments {
        [deeplink, name, ..] if deeplink == "deeplink" => Some(To::DeepLink(name.clone())),
        _ => None,
    };
    if let Some(to) = deep_link(&segments) {
        return Some(Link { email: None, to });
    }
    let email = segments
        .first()
        .filter(|first| EMAIL.is_match(first))
        .cloned();
    if email.is_some() {
        segments.remove(0);
    }
    let to = deep_link(&segments)
        .or_else(|| place(&segments, query))
        .unwrap_or_else(To::home);
    Some(Link { email, to })
}

/// An address in a path's first segment.
static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[^\s<@]+@[a-zA-Z0-9-]+(?:\.[a-zA-Z0-9-]+)+").expect("valid regex")
});

/// What `segments` (after the address) and `params`, the query, name, if Exo reads all of the
/// path.
fn place(segments: &[String], params: &str) -> Option<To> {
    let list = |list: Slug, rest: &[String]| {
        let filter = query_param(params, "filters").and_then(|name| {
            FILTERS
                .iter()
                .find(|(_, slug)| *slug == name)
                .map(|(filter, _)| *filter)
        });
        thread_rest(rest, |thread| To::List {
            list,
            thread,
            filter,
        })
    };
    match segments {
        [inbox, slug, rest @ ..] if inbox == "inbox" && slug == "other" => {
            list(Slug::Label(slug.clone()), rest)
        }
        [inbox, slug, rest @ ..] if inbox == "inbox" => list(Slug::Split(slug.clone()), rest),
        [search, query, rest @ ..] if search == "search" => {
            thread_rest(rest, |thread| To::Search {
                query: query.clone(),
                thread,
            })
        }
        [search] if search == "search" => Some(To::Search {
            query: String::new(),
            thread: None,
        }),
        // `/home` is a label's slug.
        [home, rest @ ..] if home == "home" => list(Slug::Label(home.clone()), rest),
        [label, slug, rest @ ..] if label == "label" => list(Slug::Label(slug.clone()), rest),
        // The day is taken whatever it is, and a conversation after it is left out.
        [calendar, week, rest @ ..] if calendar == "calendar" && week == "week" => {
            let day = rest.first().and_then(|day| week_day(day));
            thread_rest(rest.get(1..).unwrap_or_default(), |_| To::Week(day))
        }
        rest => list(Slug::Home, rest),
    }
}

/// `to` with the conversation `rest` opens (`/thread/{id}`), if that's all `rest` is.
fn thread_rest(rest: &[String], to: impl FnOnce(Option<String>) -> To) -> Option<To> {
    match rest {
        [] => Some(to(None)),
        [thread, id] if thread == "thread" => Some(to(Some(id.clone()))),
        [thread] if thread == "thread" => Some(to(None)),
        _ => None,
    }
}

/// The filters a link carries, by their names in `?filters=`.
const FILTERS: [(Filter, &str); 4] = [
    (Filter::Important, "important"),
    (Filter::Starred, "starred"),
    (Filter::Unread, "unread"),
    (Filter::NoReply, "noreply"),
];

/// The first `name=` value in `query`, decoded, with `+` for a space.
fn query_param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        pair.split_once('=')
            .filter(|(key, _)| *key == name)
            .map(|(_, value)| decode(&value.replace('+', " ")))
    })
}

/// A week link's day: `{year}-{month}-{day}`, a month or day past its end running into the next
/// as JavaScript's `Date` has it.
fn week_day(text: &str) -> Option<Date> {
    static DATE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(\d{4})-(\d{1,2})-(\d{1,2})$").expect("valid regex"));
    let parts = DATE.captures(text)?;
    let number = |at: usize| parts[at].parse::<i64>().ok();
    let year = i16::try_from(number(1)?).ok()?;
    Date::new(year, 1, 1)
        .ok()?
        .checked_add((number(2)? - 1).months())
        .ok()?
        .checked_add((number(3)? - 1).days())
        .ok()
}

/// `segment` with its `%XX` escapes decoded; a malformed one stays.
fn decode(segment: &str) -> String {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |at: usize| bytes.get(at).and_then(|byte| (*byte as char).to_digit(16));
        match (bytes[i], hex(i + 1), hex(i + 2)) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                i += 3;
            }
            (byte, ..) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| segment.to_owned())
}

/// Where `route` is, as a link: the account, its list, search or week, the conversation open and
/// a list's filter. `labels` and `settings` are the account's, which name its labels and splits;
/// one that's gone is the account's home.
pub(crate) fn url(route: &Route, labels: &[Label], settings: &Settings) -> String {
    let mut url = format!("{SCHEME}//{}", route.email);
    let mut filter = None;
    match &route.view {
        View::Week(day) => return format!("{url}/calendar/week/{}", day.strftime("%Y-%m-%d")),
        View::Search(query) => {
            url.push_str("/search/");
            url.push_str(&encode_component(query.as_deref().unwrap_or_default()));
        }
        View::Snippets => url.push_str("/label/snippets"),
        View::List => {
            if let Some(segment) = route
                .list
                .as_ref()
                .and_then(|list| segment(list, labels, settings))
            {
                url.push('/');
                url.push_str(&segment);
            }
            filter = route.filter;
        }
    }
    // An empty search's path ends in `/` (`search/` joined with the empty query), so a
    // conversation after it would read as the query: it's left off.
    if let Some(thread) = route.thread.as_ref().filter(|_| !url.ends_with('/')) {
        url.push_str("/thread/");
        url.push_str(&encode_component(thread));
    }
    if let Some((_, name)) = FILTERS
        .iter()
        .find(|(candidate, _)| Some(*candidate) == filter)
    {
        url.push_str("?filters=");
        url.push_str(name);
    }
    url
}

/// `list`'s part of a path: none for home (Important, or the Inbox), `inbox/{slug}` for Other
/// and a split, `label/{slug}` for everything else.
fn segment(list: &ListId, labels: &[Label], settings: &Settings) -> Option<String> {
    match list {
        ListId::Inbox | ListId::Important => None,
        ListId::Other => Some("inbox/other".to_owned()),
        ListId::Split(id) => settings
            .split(id)
            .map(|split| format!("inbox/{}", encode_component(&split.slug()))),
        ListId::Label(id) => labels
            .iter()
            .find(|label| label.id == *id)
            .map(|label| format!("label/{}", encode_component(&lists::slug(&label.name)))),
        list => system_lists()
            .find(|(candidate, _)| candidate == list)
            .map(|(_, slug)| format!("label/{slug}")),
    }
}

/// Exo's own lists and Gmail's system ones, with their slugs, which the `go to` rows' ids are.
fn system_lists() -> impl Iterator<Item = (ListId, &'static str)> {
    lists::system_go_tos(false)
        .into_iter()
        .chain(lists::late_go_tos())
        .map(|go| (go.list, go.id))
}

/// What a label's slug names in an account with `labels` and `settings`: one of the slugs Exo
/// keeps for its own lists, which no label takes, or else a Gmail label, by its name's slug and
/// then by its id; none for a slug nothing has, which goes home. With
/// Important and Other off, `important` and `other` are home, and with them on, `inbox` is.
fn label_slug(slug: &str, labels: &[Label], settings: &Settings) -> Option<(ListId, View)> {
    let home = crate::splits::home(settings);
    let builtin = match slug {
        "snippets" => return Some((home, View::Snippets)),
        "knowledge-base" => return None,
        "archived" => "done",
        "snoozed" => "reminders",
        slug => slug,
    };
    if let Some((list, _)) = system_lists().find(|(_, candidate)| *candidate == builtin) {
        let home_instead = match list {
            ListId::Important | ListId::Other => settings.unified_inbox,
            ListId::Inbox => !settings.unified_inbox,
            _ => false,
        };
        return Some((if home_instead { home } else { list }, View::List));
    }
    // The last of two labels with one slug, as each replaces the one before.
    let by_slug = || {
        lists::user_labels(labels)
            .filter(|label| lists::slug(&label.name) == slug)
            .last()
    };
    let label = by_slug().or_else(|| lists::user_labels(labels).find(|label| label.id == slug))?;
    Some((ListId::Label(label.id.clone()), View::List))
}

impl crate::Mail {
    /// Where this page is, as a link.
    pub(crate) fn link(&self) -> String {
        url(
            &self.route(),
            &self.snapshot.labels,
            self.account.settings(),
        )
    }

    /// Opens `link` in this account: what a deep link names, or the place its path
    /// does, as going there in a tab would.
    pub fn open_link(&mut self, link: &Link) {
        match &link.to {
            To::DeepLink(name) => self.open_deep_link(name),
            // Over whatever is up: a week link opens the week and does nothing else.
            To::Week(day) => self.calendar_open_week(day.unwrap_or_else(|| self.now.date())),
            To::Search { query, thread } => {
                let view = View::Search(Some(query.clone()).filter(|query| !query.is_empty()));
                let route = Route {
                    view,
                    thread: thread.clone(),
                    ..self.route()
                };
                self.go_to_route(&route);
            }
            To::List {
                list,
                thread,
                filter,
            } => {
                let settings = self.account.settings();
                let home = || (crate::splits::home(settings), View::List);
                let (list, view) = match list {
                    Slug::Home => home(),
                    Slug::Label(slug) => {
                        label_slug(slug, &self.snapshot.labels, settings).unwrap_or_else(home)
                    }
                    Slug::Split(slug) => settings
                        .enabled_splits()
                        .find(|split| split.slug() == *slug)
                        .map_or_else(home, |split| (ListId::Split(split.id.clone()), View::List)),
                };
                // Without one in the link, the filter the list opens with.
                let filter = filter.or_else(|| {
                    crate::filter::for_list(&self.list, &list, self.filter, &self.last_filters)
                });
                let route = Route {
                    email: self.account.email().to_owned(),
                    list: Some(list),
                    thread: thread.clone(),
                    view,
                    filter,
                };
                self.go_to_route(&route);
            }
        }
    }

    /// Carries out a link command: the link on the clipboard (`text/plain` and `text/html`),
    /// and `"{link}" copied to clipboard.`
    pub(crate) fn links_run(&mut self, action: Action) {
        let url = match action {
            Action::CopyLocation => self.link(),
            Action::CopyConversation => {
                let Some(thread) = crate::actionable(self.thread.as_ref(), &self.inbox)
                    .first()
                    .map(|thread| thread.id.clone())
                else {
                    return;
                };
                let route = Route {
                    thread: Some(thread),
                    ..Route::home(self.account.email().to_owned())
                };
                url(&route, &self.snapshot.labels, self.account.settings())
            }
        };
        crate::clipboard::write(&self.ctx, &url, &url);
        self.commands
            .hint(Hint::new().text(format!("\"{url}\" copied to clipboard.")));
    }

    /// The strip's copy button put a tab's link on the clipboard: this page, the tab on screen,
    /// says so.
    pub(crate) fn link_copied(&mut self) {
        self.commands.hint(
            Hint::new()
                .text("Link copied to clipboard. Hit")
                .keys(["ctralt", "/"])
                .text("to do this instantly!"),
        );
    }
}

/// What a link command does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// `Copy Private Link to Current Location`, ⌃/: the page's link.
    CopyLocation,
    /// `Copy Private Link to Conversation`: the one conversation actions apply to.
    CopyConversation,
}

/// Registers the link commands, everywhere and with the palette open
/// too; `thread_view` is whether a conversation is open, which ranks them higher, and
/// `one_conversation` whether actions apply to exactly one.
pub(crate) fn register(
    commands: &mut Commands<commands::Action>,
    thread_view: bool,
    one_conversation: bool,
) {
    let score = if thread_view { 4.31 } else { 1.51 };
    let row = |id: &'static str, title: &'static str, action: Action| {
        Command::new(id, title, commands::Action::Links(action))
            .icon("data/open-links-link")
            .within_palette()
    };
    commands.add(
        row(
            "COPY_PRIVATE_CURRENT_LOCATION_LINK",
            "Copy Private Link to Current Location",
            Action::CopyLocation,
        )
        .aliases(["Copy Private URL", "Copy Location"])
        .key("ctralt+/")
        .default_score(score),
    );
    if one_conversation {
        commands.add(
            row(
                "COPY_PRIVATE_CONVERSATION_LINK",
                "Copy Private Link to Conversation",
                Action::CopyConversation,
            )
            .aliases(["Copy Private Thread URL"])
            .default_score(score + 0.01),
        );
    }
}

#[cfg(test)]
mod tests {
    use exo_store::{Definitions, LabelKind, SplitInbox};

    use super::*;

    fn link(email: Option<&str>, to: To) -> Option<Link> {
        Some(Link {
            email: email.map(str::to_owned),
            to,
        })
    }

    fn list(list: Slug, thread: Option<&str>, filter: Option<Filter>) -> To {
        To::List {
            list,
            thread: thread.map(str::to_owned),
            filter,
        }
    }

    const MAYA: Option<&str> = Some("maya@brightmoor.co");

    #[test]
    fn deep_links_name_a_setting_and_maybe_an_account() {
        assert_eq!(
            parse("exo://deeplink/emoji-settings"),
            link(None, To::DeepLink("emoji-settings".into()))
        );
        assert_eq!(
            parse("exo://maya%40brightmoor.co/deeplink/split-library?role=sales"),
            link(MAYA, To::DeepLink("split-library".into()))
        );
        assert_eq!(
            parse("EXO:///deeplink/reminder-settings/"),
            link(None, To::DeepLink("reminder-settings".into()))
        );
        assert_eq!(parse("mailto:maya@brightmoor.co"), None);
        assert_eq!(
            parse("exo://deeplink"),
            link(None, To::home()),
            "no name: a path Exo can't read"
        );
    }

    #[test]
    fn paths_name_lists_searches_and_the_week() {
        assert_eq!(parse("exo://maya@brightmoor.co"), link(MAYA, To::home()));
        assert_eq!(
            parse("exo://maya@brightmoor.co/inbox/vip/thread/19a1?filters=unread"),
            link(
                MAYA,
                list(
                    Slug::Split("vip".into()),
                    Some("19a1"),
                    Some(Filter::Unread)
                )
            )
        );
        assert_eq!(
            parse("exo://maya@brightmoor.co/inbox/other"),
            link(MAYA, list(Slug::Label("other".into()), None, None))
        );
        assert_eq!(
            parse("exo://maya@brightmoor.co/label/work%2Fprojects?filters=noreply"),
            link(
                MAYA,
                list(
                    Slug::Label("work/projects".into()),
                    None,
                    Some(Filter::NoReply)
                )
            )
        );
        assert_eq!(
            parse("exo://maya@brightmoor.co/thread/19a1?filters=starred#app"),
            link(MAYA, list(Slug::Home, Some("19a1"), Some(Filter::Starred)))
        );
        assert_eq!(
            parse("exo://maya@brightmoor.co/home"),
            link(MAYA, list(Slug::Label("home".into()), None, None))
        );
        assert_eq!(
            parse("exo://label/sent"),
            link(None, list(Slug::Label("sent".into()), None, None)),
            "no address: the account on screen's"
        );
        let search = |query: &str, thread: Option<&str>| To::Search {
            query: query.into(),
            thread: thread.map(str::to_owned),
        };
        assert_eq!(
            parse("exo://maya@brightmoor.co/search/from%3Abea%20budget/thread/19a1?filters=unread"),
            link(MAYA, search("from:bea budget", Some("19a1"))),
            "search has no filter"
        );
        assert_eq!(
            parse("exo://maya@brightmoor.co/search/"),
            link(MAYA, search("", None))
        );
        assert_eq!(
            parse("exo://maya@brightmoor.co/calendar/week/2026-11-20"),
            link(MAYA, To::Week(Date::new(2026, 11, 20).ok()))
        );
        assert_eq!(
            parse("exo://maya@brightmoor.co/calendar/week"),
            link(MAYA, To::Week(None))
        );
        assert_eq!(
            parse("exo://maya@brightmoor.co/calendar/week/soon"),
            link(MAYA, To::Week(None)),
            "taken, whatever it is"
        );
        assert_eq!(
            parse("exo://maya@brightmoor.co/starred/19a1"),
            link(MAYA, To::home()),
            "a path Exo doesn't read"
        );
        assert_eq!(
            parse("exo://maya@brightmoor.co/team-settings/team"),
            link(MAYA, To::home()),
            "teams"
        );
    }

    #[test]
    fn the_weeks_day_runs_over_as_javascripts_does() {
        assert_eq!(week_day("2026-2-30"), Date::new(2026, 3, 2).ok());
        assert_eq!(week_day("2026-13-01"), Date::new(2027, 1, 1).ok());
        assert_eq!(week_day("2026-11-0"), Date::new(2026, 10, 31).ok());
        assert_eq!(week_day("26-11-20"), None);
    }

    fn label(id: &str, name: &str, kind: LabelKind) -> Label {
        Label {
            id: id.into(),
            name: name.into(),
            kind,
            text_color: None,
            background_color: None,
            hidden: false,
        }
    }

    /// An account with Important and Other on and a split called `Big Deals`.
    fn account() -> (Vec<Label>, Settings) {
        let labels = vec![
            label("SENT", "SENT", LabelKind::System),
            label("Label_1", "Work/Projects (2026)", LabelKind::User),
            label("Label_2", "Done", LabelKind::User),
        ];
        let mut settings = Settings::default();
        settings.set_up_splits(|| {
            vec![SplitInbox::new(
                "Big Deals",
                "from:deals@example.com",
                None,
                None,
                Definitions {
                    labels: &[],
                    me: &[],
                },
            )]
        });
        (labels, settings)
    }

    fn route(list: ListId, thread: Option<&str>, view: View, filter: Option<Filter>) -> Route {
        Route {
            email: "maya@brightmoor.co".into(),
            list: Some(list),
            thread: thread.map(str::to_owned),
            view,
            filter,
        }
    }

    #[test]
    fn a_route_is_a_path_on_exos_scheme() {
        let (labels, settings) = account();
        let split = settings.splits().next().expect("a split").id.clone();
        let url = |route: Route| url(&route, &labels, &settings);
        assert_eq!(
            url(route(ListId::Important, None, View::List, None)),
            "exo://maya@brightmoor.co"
        );
        assert_eq!(
            url(route(
                ListId::Inbox,
                Some("19a1"),
                View::List,
                Some(Filter::Unread)
            )),
            "exo://maya@brightmoor.co/thread/19a1?filters=unread"
        );
        assert_eq!(
            url(route(ListId::Other, None, View::List, None)),
            "exo://maya@brightmoor.co/inbox/other"
        );
        assert_eq!(
            url(route(
                ListId::Split(split),
                None,
                View::List,
                Some(Filter::NoReply)
            )),
            "exo://maya@brightmoor.co/inbox/big-deals?filters=noreply"
        );
        assert_eq!(
            url(route(ListId::Done, None, View::List, None)),
            "exo://maya@brightmoor.co/label/done"
        );
        assert_eq!(
            url(route(ListId::Drafts, None, View::List, None)),
            "exo://maya@brightmoor.co/label/draft"
        );
        assert_eq!(
            url(route(
                ListId::Label("Label_1".into()),
                Some("19a1"),
                View::List,
                None
            )),
            "exo://maya@brightmoor.co/label/work%2Fprojects--2026-/thread/19a1"
        );
        assert_eq!(
            url(route(
                ListId::Label("Label_9".into()),
                None,
                View::List,
                None
            )),
            "exo://maya@brightmoor.co",
            "a label that's gone"
        );
        assert_eq!(
            url(route(
                ListId::Sent,
                Some("19a1"),
                View::Search(Some("from:bea budget".into())),
                Some(Filter::Unread)
            )),
            "exo://maya@brightmoor.co/search/from%3Abea%20budget/thread/19a1"
        );
        assert_eq!(
            url(route(ListId::Sent, Some("19a1"), View::Search(None), None)),
            "exo://maya@brightmoor.co/search/"
        );
        assert_eq!(
            url(route(
                ListId::Sent,
                Some("19a1"),
                View::Week(Date::new(2026, 11, 3).expect("a day")),
                None
            )),
            "exo://maya@brightmoor.co/calendar/week/2026-11-03"
        );
        assert_eq!(
            url(route(
                ListId::Sent,
                None,
                View::Snippets,
                Some(Filter::Unread)
            )),
            "exo://maya@brightmoor.co/label/snippets"
        );
    }

    #[test]
    fn slugs_name_exos_lists_then_gmails_labels() {
        let (labels, mut settings) = account();
        let slug = |slug: &str, settings: &Settings| label_slug(slug, &labels, settings);
        assert_eq!(
            slug("archived", &settings),
            Some((ListId::Done, View::List))
        );
        assert_eq!(
            slug("done", &settings),
            Some((ListId::Done, View::List)),
            "Exo's Done, not the label called Done"
        );
        assert_eq!(slug("sent", &settings), Some((ListId::Sent, View::List)));
        assert_eq!(
            slug("snippets", &settings),
            Some((ListId::Important, View::Snippets))
        );
        assert_eq!(
            slug("work/projects--2026-", &settings),
            Some((ListId::Label("Label_1".into()), View::List))
        );
        assert_eq!(
            slug("Label_1", &settings),
            Some((ListId::Label("Label_1".into()), View::List)),
            "by id"
        );
        assert_eq!(
            slug("inbox", &settings),
            Some((ListId::Important, View::List)),
            "home, with Important and Other on"
        );
        assert_eq!(slug("other", &settings), Some((ListId::Other, View::List)));
        assert_eq!(slug("knowledge-base", &settings), None);
        assert_eq!(slug("nothing", &settings), None);
        settings.toggle_important_other();
        assert_eq!(
            slug("important", &settings),
            Some((ListId::Inbox, View::List)),
            "home, with them off"
        );
        assert_eq!(slug("inbox", &settings), Some((ListId::Inbox, View::List)));
    }

    #[test]
    fn what_a_route_says_reads_back() {
        let (labels, settings) = account();
        for (route, to) in [
            (
                route(
                    ListId::Label("Label_1".into()),
                    Some("19a1"),
                    View::List,
                    Some(Filter::Starred),
                ),
                list(
                    Slug::Label("work/projects--2026-".into()),
                    Some("19a1"),
                    Some(Filter::Starred),
                ),
            ),
            (
                route(ListId::Other, None, View::List, None),
                list(Slug::Label("other".into()), None, None),
            ),
            (
                route(
                    ListId::Important,
                    None,
                    View::Search(Some("is:unread a/b".into())),
                    None,
                ),
                To::Search {
                    query: "is:unread a/b".into(),
                    thread: None,
                },
            ),
        ] {
            assert_eq!(parse(&url(&route, &labels, &settings)), link(MAYA, to));
        }
    }
}
