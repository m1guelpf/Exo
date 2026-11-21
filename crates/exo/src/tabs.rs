//! The window's tabs. A tab is an account and a route, what its link says ([`crate::links`]): the
//! list and its filter, the open conversation, and search, the week view or the Snippets folder
//! if one is up. The screen on screen is the active tab's; each other tab keeps its own page as
//! it was left ([`Parked`]), or until it first shows, its route, which a page goes to then
//! ([`Mail::go_to_route`](crate::Mail::go_to_route)).
//!
//! ⌘T or `+` opens a tab on the active tab's account, last in the strip; ⌘W or × closes one,
//! showing its right neighbour or else its left one, and ⌘⇧T opens the one closed last again;
//! ⌘1…⌘9 show the first to ninth, ⌘⌥→ or ⌃Tab the next and ⌘⌥← or ⌃⇧Tab the previous, wrapping
//! round. A window holds at most 20: one more closes the active tab's neighbour. The tabs are kept
//! in `tabs.json` and come back at the next launch.

use std::path::{Path, PathBuf};

use egui::{Event, Key, Modifiers};
use exo_store::{Filter, ListId};
use jiff::civil::Date;

use crate::{Mail, Screen, account::Account, chrome::Pill, history::Place};

/// The most tabs a window holds.
pub(crate) const MAX_TABS: usize = 20;
/// The file in the state folder.
const FILE_NAME: &str = "tabs.json";

/// Where a tab is: an account and a route, as its link says it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Route {
    /// The account.
    pub email: String,
    /// The list; none for the account's home (`/{email}`).
    pub list: Option<ListId>,
    /// The conversation open, if one is.
    #[serde(default)]
    pub thread: Option<String>,
    /// What's up over the list.
    #[serde(default)]
    pub view: View,
    /// The list's filter, if it has one on.
    #[serde(default)]
    pub filter: Option<Filter>,
}

/// What's up over a tab's list.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum View {
    /// Nothing: the list.
    #[default]
    List,
    /// Search, with the query it ran, if it ran one (`/search/{query}`, `/search`).
    Search(Option<String>),
    /// The week view around a day (`/calendar/week/{day}`).
    Week(Date),
    /// The Snippets folder.
    Snippets,
}

impl Route {
    /// `email`'s home, as a new tab opens it (`/{email}`).
    pub(crate) fn home(email: String) -> Self {
        Self {
            email,
            list: None,
            thread: None,
            view: View::List,
            filter: None,
        }
    }
}

/// Where a tab not on screen is.
pub(crate) enum Parked {
    /// Its page, as it was left.
    Page(Box<Mail>),
    /// An account's mail at a route, whose page is made when the tab shows (a tab restored or
    /// opened again).
    Route(Route),
    /// The sign-in screen, made when it shows (a new tab with no account, a restored one).
    SignIn,
    /// A screen of its own, as it was left: "Almost there…", "Sorry!", a sign-in under way.
    Screen(Box<Screen>),
}

impl Parked {
    /// The account it shows, if it's an account's mail.
    fn email(&self) -> Option<&str> {
        match self {
            Self::Page(page) => Some(page.account().email()),
            Self::Route(route) => Some(&route.email),
            Self::SignIn | Self::Screen(_) => None,
        }
    }

    /// Where it is, if it's an account's mail.
    fn route(&self) -> Option<Route> {
        match self {
            Self::Page(page) => Some(page.route()),
            Self::Route(route) => Some(route.clone()),
            Self::SignIn | Self::Screen(_) => None,
        }
    }
}

/// A closed tab, for ⌘⇧T.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Closed {
    Route(Route),
    SignIn,
}

impl Closed {
    /// What ⌘⇧T brings back of `parked`: its route, or the sign-in screen.
    fn of(parked: &Parked) -> Self {
        parked.route().map_or(Self::SignIn, Self::Route)
    }
}

impl From<Closed> for Parked {
    fn from(closed: Closed) -> Self {
        match closed {
            Closed::Route(route) => Self::Route(route),
            Closed::SignIn => Self::SignIn,
        }
    }
}

/// One tab.
struct Tab {
    /// What its pill showed last.
    pill: Pill,
    /// Where it is; none for the tab on screen, which is wherever the screen is.
    parked: Option<Parked>,
}

/// The window's tabs, in the strip's order.
pub(crate) struct Tabs {
    tabs: Vec<Tab>,
    active: usize,
    /// The tab closed last, which ⌘⇧T opens again, once.
    closed: Option<Closed>,
}

impl Default for Tabs {
    /// The one tab a window opens with, on screen.
    fn default() -> Self {
        Self {
            tabs: vec![Tab {
                pill: Pill {
                    title: "Exo".to_owned(),
                    ..Pill::default()
                },
                parked: None,
            }],
            active: 0,
            closed: None,
        }
    }
}

impl Tabs {
    /// How many tabs there are.
    pub(crate) fn len(&self) -> usize {
        self.tabs.len()
    }

    /// The tab on screen's place.
    pub(crate) fn active(&self) -> usize {
        self.active
    }

    /// Whether a tab not on screen shows `email`'s mail.
    pub(crate) fn shows(&self, email: &str) -> bool {
        self.parked()
            .any(|(_, parked)| parked.email() == Some(email))
    }

    /// The first tab not on screen showing `email`'s mail.
    pub(crate) fn find(&self, email: &str) -> Option<usize> {
        self.parked()
            .find(|(_, parked)| parked.email() == Some(email))
            .map(|(index, _)| index)
    }

    /// The tabs not on screen, with their places.
    pub(crate) fn parked(&self) -> impl Iterator<Item = (usize, &Parked)> {
        self.tabs
            .iter()
            .enumerate()
            .filter_map(|(index, tab)| Some((index, tab.parked.as_ref()?)))
    }

    /// The pages of the tabs not on screen.
    pub(crate) fn pages(&self) -> impl Iterator<Item = &Mail> {
        self.parked().filter_map(|(_, parked)| match parked {
            Parked::Page(page) => Some(&**page),
            _ => None,
        })
    }

    /// The pages of the tabs not on screen, to change.
    pub(crate) fn pages_mut(&mut self) -> impl Iterator<Item = &mut Mail> {
        self.tabs
            .iter_mut()
            .filter_map(|tab| match &mut tab.parked {
                Some(Parked::Page(page)) => Some(&mut **page),
                _ => None,
            })
    }

    /// Where tab `index`, not on screen, is, if it's an account's mail.
    pub(crate) fn route(&self, index: usize) -> Option<Route> {
        self.tabs.get(index)?.parked.as_ref()?.route()
    }

    /// Tab `index`'s pill, to change.
    pub(crate) fn pill_mut(&mut self, index: usize) -> Option<&mut Pill> {
        self.tabs.get_mut(index).map(|tab| &mut tab.pill)
    }

    /// Takes the pages on `email`'s account out of the tabs not on screen, which keep their
    /// routes, for the account to close.
    pub(crate) fn unpage(&mut self, email: &str) -> Vec<Box<Mail>> {
        let mut pages = Vec::new();
        for tab in &mut self.tabs {
            if let Some(Parked::Page(page)) = &tab.parked
                && page.account().email() == email
            {
                let route = Parked::Route(page.route());
                if let Some(Parked::Page(page)) = tab.parked.replace(route) {
                    pages.push(page);
                }
            }
        }
        pages
    }

    /// The pills, for the strip.
    pub(crate) fn pills(&self) -> Vec<Pill> {
        self.tabs.iter().map(|tab| tab.pill.clone()).collect()
    }

    /// What the tab on screen's pill showed last.
    pub(crate) fn active_pill(&self) -> &Pill {
        &self.tabs[self.active].pill
    }

    /// The tab on screen showed `pill`.
    pub(crate) fn shown(&mut self, pill: Pill) {
        self.tabs[self.active].pill = pill;
    }

    /// Makes tab `to` the one on screen, if it's another tab: returns where it is, for the caller
    /// to show, and the place of the tab that was on screen, for [`park`](Self::park).
    pub(crate) fn select(&mut self, to: usize) -> Option<(usize, Parked)> {
        let parked = self.tabs.get_mut(to)?.parked.take()?;
        Some((std::mem::replace(&mut self.active, to), parked))
    }

    /// Tab `index`, no longer on screen, is at `parked`.
    pub(crate) fn park(&mut self, index: usize, parked: Parked) {
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.parked = Some(parked);
        }
    }

    /// The tab after the one on screen, wrapping round.
    pub(crate) fn next(&self) -> usize {
        (self.active + 1) % self.tabs.len()
    }

    /// The tab before the one on screen, wrapping round.
    pub(crate) fn previous(&self) -> usize {
        (self.active + self.tabs.len() - 1) % self.tabs.len()
    }

    /// The tab that shows when the one on screen closes: its right neighbour, else its left one.
    pub(crate) fn neighbour(&self) -> Option<usize> {
        if self.active + 1 < self.tabs.len() {
            Some(self.active + 1)
        } else {
            self.active.checked_sub(1)
        }
    }

    /// Adds a tab at `parked`, last, with `pill` until it shows; at the limit, the active tab's
    /// neighbour closes first. Returns its place.
    pub(crate) fn push(&mut self, pill: Pill, parked: Parked) -> usize {
        if self.tabs.len() >= MAX_TABS {
            // The one right of the active tab, or left of it when the active tab is last.
            let index = if self.active + 1 < self.tabs.len() {
                self.active + 1
            } else {
                self.active - 1
            };
            self.close(index);
        }
        self.tabs.push(Tab {
            pill,
            parked: Some(parked),
        });
        self.tabs.len() - 1
    }

    /// Removes tab `index`, which isn't on screen, remembering it for ⌘⇧T. Returns where it
    /// was, for its page to go.
    pub(crate) fn close(&mut self, index: usize) -> Option<Parked> {
        if index == self.active || index >= self.tabs.len() {
            return None;
        }
        let tab = self.tabs.remove(index);
        if index < self.active {
            self.active -= 1;
        }
        let parked = tab.parked?;
        self.closed = Some(Closed::of(&parked));
        Some(parked)
    }

    /// Removes the tab on screen, once another has taken its place (`select`), remembering it as
    /// `closed`.
    pub(crate) fn close_shown(&mut self, index: usize, closed: Closed) {
        if index != self.active && index < self.tabs.len() {
            self.tabs.remove(index);
            if index < self.active {
                self.active -= 1;
            }
            self.closed = Some(closed);
        }
    }

    /// The tab closed last, to open again; forgotten once taken.
    pub(crate) fn take_closed(&mut self) -> Option<Closed> {
        self.closed.take()
    }

    /// `saved`, restored: at most [`MAX_TABS`], centred on the active one where they can be, the
    /// first marked active winning, else the first. The active one comes back with its place
    /// taken out, for the caller to show. None for no tabs.
    fn restored(saved: Saved) -> Option<(Self, Parked)> {
        let active = saved.tabs.iter().position(|tab| tab.active).unwrap_or(0);
        let start = active.saturating_sub(MAX_TABS / 2);
        let end = saved.tabs.len().min(start + MAX_TABS);
        let start = start.min(end.saturating_sub(MAX_TABS));
        let mut tabs: Vec<Tab> = saved.tabs[start..end]
            .iter()
            .map(|tab| {
                let parked = tab.route.clone().map_or(Parked::SignIn, Parked::Route);
                Tab {
                    pill: Pill {
                        title: "Exo".to_owned(),
                        login_screen: tab.route.is_none(),
                        ..Pill::default()
                    },
                    parked: Some(parked),
                }
            })
            .collect();
        let active = active - start;
        let parked = tabs.get_mut(active)?.parked.take()?;
        Some((
            Self {
                tabs,
                active,
                closed: None,
            },
            parked,
        ))
    }

    /// What `tabs.json` keeps, `on_screen` the route of the tab on screen (none on the sign-in
    /// screen or an alert).
    fn saved(&self, on_screen: Option<&Route>) -> Saved {
        let tabs = self.tabs.iter().enumerate().map(|(index, tab)| {
            let route = match &tab.parked {
                None => on_screen.cloned(),
                Some(parked) => parked.route(),
            };
            SavedTab {
                route,
                active: index == self.active,
            }
        });
        Saved {
            tabs: tabs.collect(),
        }
    }
}

/// What a tab key asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    /// ⌘T.
    New,
    /// ⌘W, with more than one tab (with one, the window closes, [`crate::desktop`]).
    Close,
    /// ⌘⇧T.
    Reopen,
    /// ⌘1…⌘9: that tab, from 0.
    Select(usize),
    /// ⌘⌥→, ⌃Tab, ⌘⇧].
    Next,
    /// ⌘⌥←, ⌃⇧Tab, ⌘⇧[.
    Previous,
}

/// What `event` asks of the tabs, if it's a tab key (a shortcut or a menu item's accelerator);
/// `several` says whether there's more than one tab, for ⌘W.
fn command(event: &Event, several: bool) -> Option<Command> {
    let Event::Key {
        key,
        pressed: true,
        modifiers,
        ..
    } = event
    else {
        return None;
    };
    let command = Modifiers::COMMAND;
    let exact = |pattern: Modifiers| modifiers.matches_exact(pattern);
    let digit = [
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
        Key::Num8,
        Key::Num9,
    ]
    .iter()
    .position(|digit| digit == key);
    match key {
        Key::T if exact(command) => Some(Command::New),
        Key::T if exact(command | Modifiers::SHIFT) => Some(Command::Reopen),
        Key::W if several && exact(command) => Some(Command::Close),
        _ if digit.is_some() && exact(command) => digit.map(Command::Select),
        Key::ArrowRight if exact(command | Modifiers::ALT) => Some(Command::Next),
        Key::ArrowLeft if exact(command | Modifiers::ALT) => Some(Command::Previous),
        Key::Tab if exact(Modifiers::CTRL) => Some(Command::Next),
        Key::Tab if exact(Modifiers::CTRL | Modifiers::SHIFT) => Some(Command::Previous),
        // ⇧ makes `[` a `{` on most layouts.
        Key::CloseBracket | Key::CloseCurlyBracket if exact(command | Modifiers::SHIFT) => {
            Some(Command::Next)
        }
        Key::OpenBracket | Key::OpenCurlyBracket if exact(command | Modifiers::SHIFT) => {
            Some(Command::Previous)
        }
        _ => None,
    }
}

/// Takes this frame's tab keys out of `ctx`'s input, whatever has the keyboard; `several` says
/// whether there's more than one tab.
pub(crate) fn keys(ctx: &egui::Context, several: bool) -> Vec<Command> {
    ctx.input_mut(|input| {
        let mut commands = Vec::new();
        input.events.retain(|event| match command(event, several) {
            Some(command) => {
                commands.push(command);
                false
            }
            None => true,
        });
        commands
    })
}

/// What `tabs.json` holds.
#[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Saved {
    tabs: Vec<SavedTab>,
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct SavedTab {
    /// Its route; none on the sign-in screen.
    #[serde(default)]
    route: Option<Route>,
    #[serde(default)]
    active: bool,
}

/// Where the tabs are kept, and what was written there last.
pub(crate) struct File {
    path: PathBuf,
    /// Writes off the UI thread.
    writer: fastframe_file::Writer,
    written: Vec<u8>,
}

impl File {
    /// The tabs kept in `dir`, the state folder, and the file to keep them in from now on. None
    /// to restore when there's no file or it can't be read.
    pub(crate) fn load(dir: &Path) -> (Self, Option<(Tabs, Parked)>) {
        let path = dir.join(FILE_NAME);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("reading {}: {error}", path.display());
                }
                Vec::new()
            }
        };
        let saved = (!bytes.is_empty())
            .then(|| {
                serde_json::from_slice::<Saved>(&bytes)
                    .inspect_err(|error| log::warn!("ignoring {}: {error}", path.display()))
                    .ok()
            })
            .flatten();
        let restored = saved.and_then(Tabs::restored);
        (
            Self {
                path,
                writer: fastframe_file::Writer::new(),
                written: bytes,
            },
            restored,
        )
    }

    /// Writes `tabs` when they've changed since the last write, `on_screen` the route of the tab
    /// on screen.
    pub(crate) fn save(&mut self, tabs: &Tabs, on_screen: Option<&Route>) {
        let Ok(contents) = serde_json::to_vec(&tabs.saved(on_screen)) else {
            return;
        };
        if contents != self.written {
            self.writer.write(
                &self.path,
                contents.clone(),
                fastframe_file::Access::Default,
            );
            self.written = contents;
        }
    }
}

impl Drop for File {
    fn drop(&mut self) {
        // Saved as Exo quits.
        self.writer.flush(fastframe_file::FLUSH_TIMEOUT);
    }
}

/// A tab's title: `{name} ({count}) - {who}`, or `{name} - {who}` with no count.
pub(crate) fn title(name: &str, count: Option<&str>, who: &str) -> String {
    match count {
        Some(count) => format!("{name} ({count}) - {who}"),
        None => format!("{name} - {who}"),
    }
}

/// The title of a tab not on screen at `route` in `account`, from the account's latest labels
/// and counts, as its page would keep it; `who` is what it ends with. None for the week view,
/// which keeps the title it had.
pub(crate) fn parked_title(account: &Account, route: &Route, who: &str) -> Option<String> {
    let name = match &route.view {
        View::Week(_) => return None,
        View::Search(Some(query)) => return Some(title(&format!("Search: {query}"), None, who)),
        View::Search(None) => return Some(title("Search", None, who)),
        View::Snippets => return Some(title("Snippets", None, who)),
        View::List => route
            .list
            .clone()
            .unwrap_or_else(|| crate::splits::home(account.settings())),
    };
    let snapshot = account.snapshot();
    let count = snapshot
        .counts
        .get(&name)
        .and_then(crate::splits::count_label);
    Some(title(
        crate::lists::name(&name, &snapshot.labels, account.settings().splits()),
        count.as_deref(),
        who,
    ))
}

impl crate::Mail {
    /// Where the mail is, as a tab's route.
    pub(crate) fn route(&self) -> Route {
        let view = if let Some(day) = self.calendar_week() {
            View::Week(day)
        } else if self.search.is_some() {
            View::Search(self.search_query().map(str::to_owned))
        } else if self.snippets.folder.is_open() {
            View::Snippets
        } else {
            View::List
        };
        Route {
            email: self.account.email().to_owned(),
            list: Some(self.list.clone()),
            thread: self.thread.as_ref().map(|thread| thread.id().to_owned()),
            view,
            filter: self.filter,
        }
    }

    /// Goes to `route`, a tab's shown again or a link's: out of what it doesn't have up (search,
    /// the week, the Snippets folder, a new message's pane, whose draft is saved as going to a
    /// list saves it), to its list with its filter and its conversation open or none, then into
    /// what it has up again. A snippet with unsaved changes stays open.
    pub(crate) fn go_to_route(&mut self, route: &Route) {
        let now = self.ctx.input(|input| input.time);
        let home = crate::splits::home(self.account.settings());
        let list = route.list.clone().unwrap_or(home);
        if self.calendar_week().is_some() && !matches!(route.view, View::Week(_)) {
            self.run(
                crate::commands::Action::Calendar(crate::calendar::Action::CloseWeek),
                now,
            );
        }
        let search = match &route.view {
            View::Search(query) => Some(query.as_deref()),
            View::List | View::Week(_) | View::Snippets => None,
        };
        if self.search.is_some() && search.is_none_or(|query| query != self.search_query()) {
            self.search_exit(now);
        }
        if self.snippets.folder.is_open() && route.view != View::Snippets {
            self.close_snippet_view();
            self.snippets.folder.close();
        }
        if !self.snippet_unsaved() {
            self.leave_pane();
        }
        if let Some(query) = search.filter(|_| self.search.is_none()) {
            if list != self.list {
                self.close_thread();
                self.show(list.clone());
            }
            self.search_again(query, now);
        }
        if self.search.is_some() {
            match &route.thread {
                Some(id) if self.thread.as_ref().is_none_or(|open| open.id() != id) => {
                    self.open_thread(id.clone())
                }
                Some(_) => {}
                None => self.close_thread(),
            }
        } else {
            self.go_to(Place {
                list,
                thread: route.thread.clone(),
            });
            if route.view == View::List && route.filter != self.filter {
                self.filter_set(route.filter);
            }
        }
        match route.view {
            View::Week(day) if self.calendar_week() != Some(day) => self.calendar_open_week(day),
            View::Snippets if !self.snippets.folder.is_open() => self.run(
                crate::commands::Action::Snippets(crate::snippets::Action::Go),
                now,
            ),
            View::List | View::Search(_) | View::Week(_) | View::Snippets => {}
        }
    }

    /// What the tab says: with several accounts it names the account; with one, Exo. Searching,
    /// the search names it, without a count; See Your Week names the week (`📆 {week} - {who}`),
    /// with no count.
    pub(crate) fn tab_title(&self) -> String {
        let name = match (&self.search, self.search_query()) {
            (Some(_), Some(query)) => format!("Search: {query}"),
            (Some(_), None) => "Search".to_owned(),
            (None, _) if self.snippets.folder.is_open() => "Snippets".to_owned(),
            (None, _) => crate::lists::name(
                &self.list,
                &self.snapshot.labels,
                self.account.settings().splits(),
            )
            .to_owned(),
        };
        let who = if self.accounts.len() > 1 {
            self.account.email()
        } else {
            "Exo"
        };
        let count = if self.search.is_some() {
            None
        } else {
            self.list_count(&self.snapshot.counts)
        };
        match self.calendar_week() {
            Some(day) => format!(
                "📆 {} - {who}",
                crate::calendar::week_title(self.calendar_week_start(day))
            ),
            None => title(&name, count.as_deref(), who),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(email: &str) -> Parked {
        Parked::Route(Route::home(email.to_owned()))
    }

    fn pill(title: &str) -> Pill {
        Pill {
            title: title.to_owned(),
            ..Pill::default()
        }
    }

    /// Tabs whose titles are `titles`, the first on screen.
    fn tabs(titles: &[&str]) -> Tabs {
        let mut tabs = Tabs::default();
        tabs.shown(pill(titles[0]));
        for title in &titles[1..] {
            tabs.push(pill(title), route(title));
        }
        tabs
    }

    fn titles(tabs: &Tabs) -> Vec<String> {
        tabs.pills().into_iter().map(|pill| pill.title).collect()
    }

    /// Shows tab `to`, the one leaving at `leaving`.
    fn select(tabs: &mut Tabs, to: usize, leaving: &str) -> Option<Parked> {
        let (from, parked) = tabs.select(to)?;
        tabs.park(from, route(leaving));
        Some(parked)
    }

    #[test]
    fn next_and_previous_wrap_round() {
        let mut tabs = tabs(&["a", "b", "c"]);
        assert_eq!((tabs.next(), tabs.previous()), (1, 2));
        select(&mut tabs, 2, "a");
        assert_eq!((tabs.next(), tabs.previous()), (0, 1));
        assert!(tabs.select(2).is_none(), "already on screen");
        assert!(tabs.select(3).is_none(), "no such tab");
    }

    #[test]
    fn the_twenty_first_tab_closes_the_active_ones_neighbour() {
        let names: Vec<String> = (0..20).map(|n| n.to_string()).collect();
        let mut tabs = tabs(&names.iter().map(String::as_str).collect::<Vec<_>>());
        select(&mut tabs, 5, "0");
        tabs.push(pill("new"), route("new"));
        assert_eq!(tabs.len(), MAX_TABS);
        assert!(
            !titles(&tabs).contains(&"6".to_owned()),
            "the one right of the active tab"
        );
        assert_eq!(
            tabs.take_closed(),
            Some(Closed::Route(Route::home("6".to_owned())))
        );
        // The active tab last: the one left of it.
        select(&mut tabs, 19, "5");
        tabs.push(pill("newer"), route("newer"));
        assert_eq!(
            titles(&tabs)[18..],
            ["new".to_owned(), "newer".to_owned()],
            "19 went"
        );
        assert_eq!(tabs.active(), 18);
    }

    #[test]
    fn closing_remembers_one_tab() {
        let mut tabs = tabs(&["a", "b", "c"]);
        select(&mut tabs, 2, "a");
        tabs.close(0);
        assert_eq!(
            (titles(&tabs), tabs.active()),
            (vec!["b".to_owned(), "c".to_owned()], 1),
            "the active tab stays"
        );
        assert_eq!(
            tabs.take_closed(),
            Some(Closed::Route(Route::home("a".to_owned())))
        );
        assert_eq!(tabs.take_closed(), None, "one level");
        assert_eq!(tabs.neighbour(), Some(0), "the last one's left");
        let (from, _) = tabs.select(0).expect("b");
        tabs.close_shown(from, Closed::SignIn);
        assert_eq!((titles(&tabs), tabs.active()), (vec!["b".to_owned()], 0));
        assert_eq!(tabs.neighbour(), None);
    }

    #[test]
    fn restoring_keeps_twenty_round_the_active_tab() {
        let saved = |active: usize, count: usize| Saved {
            tabs: (0..count)
                .map(|n| SavedTab {
                    route: Some(Route::home(n.to_string())),
                    active: n == active,
                })
                .collect(),
        };
        let (tabs, parked) = Tabs::restored(saved(30, 40)).expect("tabs");
        assert_eq!((tabs.len(), tabs.active()), (MAX_TABS, 10), "20..40");
        assert!(matches!(parked, Parked::Route(route) if route.email == "30"));
        let (tabs, _) = Tabs::restored(saved(38, 40)).expect("tabs");
        assert_eq!(tabs.active(), 18, "the last twenty");
        let (tabs, _) = Tabs::restored(saved(99, 3)).expect("tabs");
        assert_eq!(tabs.active(), 0, "none active: the first");
        assert!(Tabs::restored(Saved::default()).is_none());
    }

    #[test]
    fn what_is_saved_comes_back() {
        let mut tabs = tabs(&["a", "b"]);
        tabs.push(pill("Exo"), Parked::SignIn);
        let week = Route {
            view: View::Week(Date::new(2026, 11, 20).expect("a day")),
            ..Route::home("c".into())
        };
        tabs.push(pill("c"), Parked::Route(week.clone()));
        let on_screen = Route {
            email: "a".into(),
            list: Some(ListId::Label("Label_1".into())),
            thread: Some("t".into()),
            view: View::Search(Some("from:bea".into())),
            filter: Some(Filter::Unread),
        };
        let saved = tabs.saved(Some(&on_screen));
        let json = serde_json::to_vec(&saved).expect("serializes");
        let back: Saved = serde_json::from_slice(&json).expect("reads");
        assert_eq!(back, saved);
        let (restored, parked) = Tabs::restored(back).expect("tabs");
        assert_eq!(restored.len(), 4);
        assert_eq!(restored.route(3), Some(week));
        assert!(matches!(parked, Parked::Route(route) if route == on_screen));
        assert!(restored.find("b") == Some(1) && restored.shows("b") && !restored.shows("a"));
    }

    #[test]
    fn the_shells_keys() {
        let press = |key, modifiers| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let command = Modifiers {
            mac_cmd: true,
            command: true,
            ..Modifiers::NONE
        };
        assert_eq!(command_of(press(Key::T, command), true), Some(Command::New));
        assert_eq!(
            command_of(press(Key::T, command | Modifiers::SHIFT), true),
            Some(Command::Reopen)
        );
        assert_eq!(
            command_of(press(Key::W, command), true),
            Some(Command::Close)
        );
        assert_eq!(
            command_of(press(Key::W, command), false),
            None,
            "the window's, with one tab"
        );
        assert_eq!(
            command_of(press(Key::Num1, command), true),
            Some(Command::Select(0))
        );
        assert_eq!(
            command_of(press(Key::Num9, command), true),
            Some(Command::Select(8))
        );
        assert_eq!(
            command_of(press(Key::Num1, Modifiers::CTRL), true),
            None,
            "⌃1 switches accounts"
        );
        assert_eq!(
            command_of(press(Key::ArrowRight, command | Modifiers::ALT), true),
            Some(Command::Next)
        );
        assert_eq!(
            command_of(press(Key::ArrowLeft, command), true),
            None,
            "⌘← is back"
        );
        assert_eq!(
            command_of(press(Key::Tab, Modifiers::CTRL), true),
            Some(Command::Next)
        );
        assert_eq!(
            command_of(press(Key::Tab, Modifiers::CTRL | Modifiers::SHIFT), true),
            Some(Command::Previous)
        );
        assert_eq!(
            command_of(press(Key::Tab, Modifiers::NONE), true),
            None,
            "the next split"
        );
        assert_eq!(
            command_of(
                press(Key::OpenCurlyBracket, command | Modifiers::SHIFT),
                true
            ),
            Some(Command::Previous)
        );
        assert_eq!(
            command_of(press(Key::CloseBracket, command | Modifiers::SHIFT), true),
            Some(Command::Next)
        );
        assert_eq!(
            command_of(press(Key::OpenBracket, command), true),
            None,
            "⌘[ is back"
        );
    }

    fn command_of(event: Event, several: bool) -> Option<Command> {
        command(&event, several)
    }
}
