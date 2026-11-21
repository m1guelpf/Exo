//! Search: the search bar that takes the header's place, its suggestions and history, and what
//! running a search does. The results are an account's search (`crate::account`), shown by the
//! thread list.
//!
//! Ask AI is hidden as for an account without AI: no `Ask AI` row in the suggestions or the
//! history, and ⇧Enter searches like Enter.

mod app;
mod autocomplete;
mod bar;
mod tips;

use std::{
    collections::BTreeMap,
    sync::{Arc, LazyLock},
};

use exo_commands::{Command, Commands, Region};
use exo_store::{
    Label, ListId, ScoredContact, SearchHistoryItem, SearchHistoryKind, SearchQuery, SplitInbox,
};
use regex::Regex;

use crate::{commands, compose::contacts, format::names::Person};

use autocomplete::Suggestion;
pub(crate) use bar::{header_height, show_bar, show_empty};
pub(crate) use tips::{overflow as tips_overflow, show as show_tips};

use autocomplete::{Kind, LabelChoice, OPERATORS};

/// Where the bar's keys live.
const SEARCH_BAR: &[Region] = &[Region::SearchBar];
/// How long the bar rests on a search before it's run ahead, in seconds.
const INSTANT_DELAY: f64 = 0.3;

/// What the bar asks of the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    /// Show this search's results.
    Run(String),
    /// Run this search ahead, for Enter to show at once.
    Preload(String),
    /// Stop showing results.
    Stop,
    /// Give the results the keyboard once there are some, on this thread if it's listed, else on
    /// the first (waiting for the list, and doing nothing if it's empty).
    FocusList(Option<String>),
    /// Show this person in the contact pane, or nobody.
    Contact(Option<Person>),
    /// Remember this search.
    Remember(String),
    /// Leave search.
    Exit,
}

/// A key the bar handles itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    Enter,
    Tab,
    /// → with the caret at the end.
    Right,
    Up,
    Down,
    Escape,
}

/// What the search commands do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// `/`, `⌘/`, `&`: search, or select the bar's text when already searching.
    Toggle,
    /// A key in the bar.
    Key(Key),
}

/// The address book as the bar suggests people from it: as read, and indexed in two, the
/// addresses `Remove Suggestion` took out (`excluded`) in their own.
struct Book {
    contacts: Arc<Vec<ScoredContact>>,
    excluded: BTreeMap<String, String>,
    kept: contacts::Index,
    removed: contacts::Index,
}

/// The search on screen: the bar's text and suggestions, and the search whose results show.
pub(crate) struct Search {
    /// The bar's text (`value`).
    value: String,
    /// What's drawn under the text: the text, a completion and lozenges (`underlay`).
    underlay: Option<String>,
    /// Where the lozenge around the word being typed starts.
    lozenge_start: Option<usize>,
    /// `None` when closed (the history shows), empty when nothing matched.
    suggestions: Option<Vec<Suggestion>>,
    /// The suggestion picked with keys or the mouse.
    selected: Option<usize>,
    selected_by_mouse: bool,
    /// The history row picked with keys or the mouse.
    history_selected: Option<usize>,
    /// Tab took a suggestion since the last other key.
    used_autocomplete: bool,
    /// The search whose results show.
    query: Option<String>,
    /// The search to run ahead once the bar has rested on it (instant search), and when that
    /// is once [`poll_instant`](Self::poll_instant) has seen it.
    instant: Option<(String, Option<f64>)>,
    /// The address book the bar suggests people from (`account.contacts`), indexed.
    book: Option<Book>,
    /// The address book was asked for.
    pub book_asked: bool,
    /// The thread the cursor was on when the bar took the keyboard.
    last_focused: Option<String>,
    /// The bar has the keyboard; otherwise the results do.
    focused: bool,
    /// Select all of the bar's text on the next frame.
    select_all: bool,
    /// The results take the keyboard once listed, on this thread if it's among them.
    focus_pending: Option<Option<String>>,
    /// The caret is at the end of the text, with nothing selected.
    caret_at_end: bool,
    /// The text the field last showed: when the bar's text changes otherwise (a suggestion
    /// taken), the caret goes to its end, as a textarea's does.
    field_text: String,
    labels: Vec<LabelChoice>,
    /// Whom the contact pane shows, as a suggestion or history row named them; `Some(None)` is
    /// nobody. `None` once the results' cursor decides.
    pub contact: Option<Option<Person>>,
}

impl Search {
    /// Search mode, the bar holding `initial`.
    pub fn new(initial: &str, labels: &[Label]) -> Self {
        let value = normalize_quotes(initial);
        Self {
            underlay: Some(value.clone()),
            value,
            lozenge_start: None,
            suggestions: None,
            selected: None,
            selected_by_mouse: false,
            history_selected: None,
            used_autocomplete: false,
            query: None,
            instant: None,
            book: None,
            book_asked: false,
            last_focused: None,
            focused: true,
            select_all: false,
            focus_pending: None,
            caret_at_end: true,
            field_text: String::new(),
            labels: autocomplete::label_choices(labels),
            contact: None,
        }
    }

    /// The search whose results show.
    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }

    /// The bar's text.
    pub fn text(&self) -> &str {
        &self.value
    }

    /// The bar has the keyboard.
    pub fn bar_focused(&self) -> bool {
        self.focused
    }

    /// The suggestion list is open.
    fn list_open(&self) -> bool {
        self.suggestions
            .as_ref()
            .is_some_and(|suggestions| !suggestions.is_empty())
    }

    /// The history shows (no suggestions and no results).
    pub fn history_open(&self) -> bool {
        self.suggestions.is_none() && self.query.is_none()
    }

    /// Keeps the labels the bar suggests current.
    pub fn set_labels(&mut self, labels: &[Label]) {
        self.labels = autocomplete::label_choices(labels);
    }

    /// The bar takes the keyboard, remembering the results' cursor.
    pub fn focus_bar(&mut self, cursor: Option<String>) {
        self.focused = true;
        self.focus_pending = None;
        self.last_focused = cursor;
    }

    /// The results take the keyboard as soon as there are some, on `prefer` if it's among them.
    pub fn focus_list_when_listed(&mut self, prefer: Option<String>) {
        self.focus_pending = Some(prefer);
    }

    /// The results arrived: `Some(cursor)` when they take the keyboard now, on that thread (or
    /// the first). An empty finished search leaves it with the bar.
    pub fn results_listed(&mut self, ids: &[&str], finished: bool) -> Option<Option<String>> {
        let prefer = self.focus_pending.as_ref()?;
        if ids.is_empty() {
            if finished {
                self.focus_pending = None;
            }
            return None;
        }
        let cursor = prefer
            .clone()
            .filter(|prefer| ids.contains(&prefer.as_str()));
        self.focus_pending = None;
        self.focused = false;
        self.contact = None;
        Some(cursor)
    }

    /// `/` again: the bar takes the keyboard with its text selected (`select`).
    pub fn select(&mut self, cursor: Option<String>) {
        self.focus_bar(cursor);
        self.select_all = true;
    }

    /// The text changed to `value`.
    pub fn edit(&mut self, value: &str) -> Vec<Effect> {
        self.last_focused = None;
        self.selected = None;
        self.history_selected = None;
        self.used_autocomplete = false;
        let value = value.replace(['\n', '\r'], " ");
        let mut effects = Vec::new();
        if value.is_empty() {
            effects.push(Effect::Contact(None));
        }
        let old = self.underlay.as_deref().map(str::to_lowercase);
        effects.extend(self.set_value(&value));
        if !old
            .as_deref()
            .is_some_and(|old| old.starts_with(&self.value.to_lowercase()))
        {
            let mut underlay = self.value.clone();
            if let Some(tail) = old.as_deref().and_then(|old| old.get(self.value.len()..)) {
                underlay.push_str(tail);
            }
            self.underlay = Some(underlay);
        }
        // The suggestions come after the underlay has changed.
        effects.extend(self.update_autocomplete());
        effects
    }

    /// Sets the bar's text: smart quotes straightened; an empty query, or one ending in an
    /// operator, stops showing results.
    fn set_value(&mut self, value: &str) -> Vec<Effect> {
        self.value = normalize_quotes(value);
        let trimmed = self.value.trim();
        if trimmed.is_empty() || ENDS_WITH_OPERATOR.is_match(trimmed) {
            self.instant = None;
            if self.query.take().is_some() {
                return vec![Effect::Stop];
            }
        }
        Vec::new()
    }

    /// Enqueues an instant search: `value` runs ahead once the bar has rested on it for
    /// [`INSTANT_DELAY`], unless something else is enqueued or searched first.
    fn enqueue_instant(&mut self, value: &str) {
        let value = value.trim();
        if !value.is_empty() && !ENDS_WITH_OPERATOR.is_match(value) {
            self.instant = Some((value.to_owned(), None));
        }
    }

    /// The instant search due by `now` (`Effect::Preload`, if it isn't what shows); otherwise
    /// how long until one is, to look again then.
    pub fn poll_instant(&mut self, now: f64) -> (Vec<Effect>, Option<f64>) {
        let Some((_, due)) = &mut self.instant else {
            return (Vec::new(), None);
        };
        let due = *due.get_or_insert(now + INSTANT_DELAY);
        if now < due {
            return (Vec::new(), Some(due - now));
        }
        let Some((value, _)) = self.instant.take() else {
            return (Vec::new(), None);
        };
        (self.preload(&value), None)
    }

    /// Runs `value` ahead, unless it's the search showing.
    fn preload(&self, value: &str) -> Vec<Effect> {
        let shown = self
            .query
            .as_deref()
            .is_some_and(|shown| shown.trim() == value.trim());
        if SearchQuery::parse(value).is_empty() || shown {
            Vec::new()
        } else {
            vec![Effect::Preload(value.to_owned())]
        }
    }

    /// Updates the suggestions for the text, with people from the address book when its last word
    /// could be someone.
    fn update_autocomplete(&mut self) -> Vec<Effect> {
        let people: Vec<Vec<(String, Option<String>)>> =
            if autocomplete::wants_contacts(&self.value) {
                autocomplete::contact_words(&self.value)
                    .iter()
                    .map(|word| self.people(word))
                    .collect()
            } else {
                Vec::new()
            };
        let results = autocomplete::suggest(&self.value, &self.labels, &people);
        self.on_results(results)
    }

    /// For one word, the address book's best five people, as `(email, name)`, those
    /// `Remove Suggestion` took out after the rest.
    fn people(&self, word: &str) -> Vec<(String, Option<String>)> {
        if word.trim().is_empty() || word.contains('"') {
            return Vec::new();
        }
        let Some(book) = &self.book else {
            return Vec::new();
        };
        let top = |index: &contacts::Index| {
            index
                .top(word, autocomplete::LIMIT)
                .into_iter()
                .map(|person| (person.email.clone(), person.name.clone()))
                .collect::<Vec<_>>()
        };
        top(&book.kept)
            .into_iter()
            .chain(top(&book.removed))
            .take(autocomplete::LIMIT)
            .collect()
    }

    /// Indexes the account's address book once it's read (or read again), and again when
    /// `Remove Suggestion` takes an address out or puts one back (`excluded`); suggestions
    /// waiting on people are worked out again.
    pub fn set_book(
        &mut self,
        book: &Arc<Vec<ScoredContact>>,
        excluded: &BTreeMap<String, String>,
    ) -> Vec<Effect> {
        if self.book.as_ref().is_some_and(|indexed| {
            Arc::ptr_eq(&indexed.contacts, book) && indexed.excluded == *excluded
        }) {
            return Vec::new();
        }
        self.book = Some(Book {
            contacts: book.clone(),
            excluded: excluded.clone(),
            kept: contacts::Index::book_without(book, excluded),
            removed: contacts::Index::excluded(book, excluded),
        });
        if autocomplete::wants_contacts(&self.value) && self.focused && self.query.is_none() {
            self.update_autocomplete()
        } else {
            Vec::new()
        }
    }

    /// Takes in the suggestions for the text: with the caret at the end (or the bar unfocused) the
    /// first one completes it; with none, the word being typed is lozenged and the text waits to
    /// run ahead.
    fn on_results(&mut self, mut results: Vec<Suggestion>) -> Vec<Effect> {
        if !results.is_empty() && (self.caret_at_end || !self.focused) {
            for suggestion in &mut results {
                suggestion.start = self.start_point(suggestion);
            }
            let first = results[0].clone();
            self.suggestions = Some(results);
            return self.change_suggested_input(&first);
        }
        let tokens = autocomplete::increasing_substrings(&self.value, false);
        let last = tokens.last().cloned().unwrap_or_default();
        let mut current = last.clone();
        if let [.., before, _] = tokens.as_slice() {
            let head = before.rfind(last.as_str()).map_or("", |at| &before[..at]);
            if !OPERATOR.is_match(&last) && !head.is_empty() && ENDS_WITH_OPERATOR.is_match(head) {
                current = before.clone();
            }
        }
        self.suggestions = if self.value.is_empty() {
            None
        } else {
            Some(Vec::new())
        };
        self.underlay = Some(self.value.clone());
        self.lozenge_start = self.value.rfind(current.as_str());
        let value = self.value.clone();
        self.enqueue_instant(&value);
        Vec::new()
    }

    /// The bar shows what `suggestion` would complete, lozenged, and the contact pane shows the
    /// person it names.
    fn change_suggested_input(&mut self, suggestion: &Suggestion) -> Vec<Effect> {
        let underlay = self.underlay_for(suggestion);
        let mut start = suggestion.start;
        if let Some(at) = start {
            let head = underlay.get(..at).unwrap_or_default();
            if let Some(operator) = ENDS_WITH_OPERATOR.find(head) {
                start = Some(at - operator.as_str().len());
            }
        }
        self.underlay = Some(underlay);
        self.lozenge_start = start;
        let composite = self.composite(suggestion);
        self.enqueue_instant(&composite);
        match &suggestion.kind {
            Kind::Contact { name, email, .. } => vec![Effect::Contact(Some(Person::new(
                name.clone().unwrap_or_default(),
                email.clone(),
            )))],
            _ => Vec::new(),
        }
    }

    /// The underlay: the text and the rest of the suggestion when it completes the last word, and
    /// ` ⇥ address` for someone picked by name.
    fn underlay_for(&self, suggestion: &Suggestion) -> String {
        let mut underlay = self.value.clone();
        let completion = self.auto_completion(suggestion);
        if let Some(rest) = &completion {
            underlay.push_str(rest);
        }
        if let Kind::Contact { email, by_name, .. } = &suggestion.kind
            && (completion.is_none() || *by_name)
        {
            underlay.push_str(&format!(" ⇥ {email}"));
        }
        underlay
    }

    /// What the suggestion adds after what's typed, if it starts with it.
    fn auto_completion(&self, suggestion: &Suggestion) -> Option<String> {
        let replacement = match suggestion.kind {
            Kind::Label | Kind::Category => suggestion.completion.as_deref()?,
            _ => suggestion.replacement.as_str(),
        };
        let original = suggestion.original.to_lowercase();
        if !replacement.to_lowercase().starts_with(&original) {
            return None;
        }
        let rest = replacement.get(suggestion.original.len()..)?;
        let end = self.value.to_lowercase().rfind(&original)? + original.len();
        if self.value.len() == end {
            Some(rest.to_owned())
        } else if self.value.len() == end + 1 && self.value.ends_with(' ') && rest.starts_with(' ')
        {
            Some(rest[1..].to_owned())
        } else {
            None
        }
    }

    /// Where the word a suggestion completes starts.
    fn start_point(&self, suggestion: &Suggestion) -> Option<usize> {
        let mut original = suggestion.original.to_lowercase();
        if let Some(operator) = OPERATOR.find(&original) {
            original = operator.as_str().to_owned();
        }
        let value = self.value.to_lowercase();
        value
            .rfind(&original)
            .or_else(|| format!("in:{value}").rfind(&original))
    }

    /// The text with the suggestion in place of the word it completes.
    fn composite(&self, suggestion: &Suggestion) -> String {
        static SPACED_COLON: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"\s*:").expect("valid regex"));
        static TRAILING_COLON: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r":\s*$").expect("valid regex"));
        let inserted = match &suggestion.kind {
            Kind::Contact { email, .. } => email.clone(),
            _ => SPACED_COLON
                .replace(suggestion.replacement.trim_start(), ":")
                .into_owned(),
        };
        let head = self
            .value
            .get(..suggestion.start.unwrap_or_default().min(self.value.len()))
            .unwrap_or_default();
        format!("{}{inserted}", TRAILING_COLON.replace(head, ":"))
    }

    /// Takes a suggestion (or the text as typed). An operator keeps the bar open for its value;
    /// anything else searches.
    fn on_added(&mut self, suggestion: Option<Suggestion>, focus_list: bool) -> Vec<Effect> {
        static BANG: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(?-u:\B)!\w+").expect("valid regex"));
        let mut value = match &suggestion {
            Some(suggestion) => ensure_trailing_space(&self.composite(suggestion)),
            None => self.value.clone(),
        };
        if !value.contains(['"', '\'']) {
            value = BANG
                .replace_all(&value, |found: &regex::Captures<'_>| {
                    format!("\"{}\"", &found[0])
                })
                .into_owned();
        }
        self.suggestions = if self.value.is_empty() {
            None
        } else {
            Some(Vec::new())
        };
        self.underlay = Some(value.clone());
        if suggestion
            .as_ref()
            .is_some_and(|suggestion| suggestion.kind == Kind::Operator)
        {
            let mut effects = self.set_value(&value);
            effects.extend(self.update_autocomplete());
            return effects;
        }
        self.set_value_and_search(&value, focus_list)
    }

    /// Sets the bar's text to `value` and searches it.
    fn set_value_and_search(&mut self, value: &str, focus_list: bool) -> Vec<Effect> {
        let mut effects = Vec::new();
        if focus_list
            && self.used_autocomplete
            && history_kind(value) == Some(SearchHistoryKind::Contact)
        {
            effects.push(Effect::Remember(value.to_owned()));
        }
        self.lozenge_start = Some(value.trim().len());
        effects.extend(self.set_value(value));
        effects.extend(self.start_search(focus_list));
        effects
    }

    /// Starts the search: without `focus_list` (Tab), the search only runs ahead; with it, the
    /// results show and take the keyboard, back on the thread they had if the search didn't change.
    /// Either way the instant search waiting is dropped.
    fn start_search(&mut self, focus_list: bool) -> Vec<Effect> {
        self.instant = None;
        if !focus_list {
            return self.preload(&self.value);
        }
        if SearchQuery::parse(&self.value).is_empty() {
            return Vec::new();
        }
        let same = self
            .query
            .as_deref()
            .is_some_and(|shown| shown.trim() == self.value.trim());
        let mut effects = Vec::new();
        if !same {
            // The new results decide whom the contact pane shows.
            self.query = Some(self.value.clone());
            self.contact = None;
            effects.push(Effect::Run(self.value.clone()));
        }
        effects.push(Effect::FocusList(
            self.last_focused.clone().filter(|_| same),
        ));
        effects
    }

    /// A key the bar handles; `history` is the account's recent searches.
    pub fn key(&mut self, key: Key, history: &[SearchHistoryItem]) -> Vec<Effect> {
        match key {
            Key::Escape if self.list_open() => {
                self.suggestions = None;
                self.underlay = Some(self.value.clone());
                Vec::new()
            }
            Key::Escape => vec![Effect::Exit],
            Key::Tab | Key::Right => self.select_suggestion(),
            Key::Enter => {
                let selected = self
                    .selected
                    .and_then(|at| self.suggestions.as_ref()?.get(at).cloned());
                self.on_added(selected, true)
            }
            Key::Up | Key::Down => {
                self.used_autocomplete = false;
                let step = if key == Key::Up { -1 } else { 1 };
                if self.list_open() {
                    self.step_suggestions(step)
                } else if self.history_open() {
                    self.step_history(step, history)
                } else if key == Key::Down {
                    vec![Effect::FocusList(None)]
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// Takes a suggestion (Tab, → at the end): the picked or first suggestion, or with none open,
    /// the search as typed.
    fn select_suggestion(&mut self) -> Vec<Effect> {
        if !self.list_open() {
            return self.on_added(None, true);
        }
        self.used_autocomplete = true;
        let suggestions = self.suggestions.as_deref().unwrap_or_default();
        let picked = self
            .selected
            .and_then(|at| suggestions.get(at))
            .or(suggestions.first())
            .cloned();
        self.on_added(picked, false)
    }

    /// Steps through the suggestions: Down past the last leaves the list unpicked.
    fn step_suggestions(&mut self, step: isize) -> Vec<Effect> {
        let count = self.suggestions.as_ref().map_or(0, Vec::len);
        let current = self.selected.map_or(-1, |at| at as isize);
        let next = current + step;
        if step < 0 {
            let next = if next < -1 { count as isize - 1 } else { next };
            return self.highlight_suggestion(usize::try_from(next).ok(), false);
        }
        if (next as usize) < count {
            return self.highlight_suggestion(Some(next as usize), false);
        }
        let mut effects = self.highlight_suggestion(Some(0), false);
        effects.extend(self.highlight_suggestion(None, false));
        effects
    }

    /// Picks a suggestion, which the bar then shows completed.
    fn highlight_suggestion(&mut self, at: Option<usize>, by_mouse: bool) -> Vec<Effect> {
        self.selected = at;
        self.selected_by_mouse = by_mouse;
        match at.and_then(|at| self.suggestions.as_ref()?.get(at).cloned()) {
            Some(suggestion) => self.change_suggested_input(&suggestion),
            None => Vec::new(),
        }
    }

    /// The pointer moved onto suggestion `at` (only when the mouse moved).
    pub fn hover_suggestion(&mut self, at: Option<usize>) {
        self.selected = at;
        self.selected_by_mouse = at.is_some();
    }

    /// A suggestion was clicked.
    pub fn click_suggestion(&mut self, at: usize) -> Vec<Effect> {
        let picked = self
            .suggestions
            .as_ref()
            .and_then(|suggestions| suggestions.get(at).cloned());
        self.on_added(picked, true)
    }

    /// Steps through the history: past either end the bar empties.
    fn step_history(&mut self, step: isize, history: &[SearchHistoryItem]) -> Vec<Effect> {
        let count = history.len() as isize;
        let current = self.history_selected.map_or(-1, |at| at as isize);
        let mut next = current + step;
        if step < 0 && next < -1 {
            next = count - 1;
        } else if step > 0 && next >= count {
            next = -1;
        }
        self.history_selected = usize::try_from(next).ok();
        let item = self
            .history_selected
            .and_then(|at| history_rows(history).get(at).map(|item| (*item).clone()));
        let mut effects = Vec::new();
        if let Some(item) = &item
            && item.kind == SearchHistoryKind::Contact
        {
            effects.push(Effect::Contact(Some(Person::new(
                String::new(),
                item.value.clone(),
            ))));
        }
        let value = item.map(|item| item.value).unwrap_or_default();
        self.underlay = Some(value.clone());
        effects.extend(self.set_value(&value));
        effects
    }

    /// The pointer moved onto history row `at` (in display order).
    pub fn hover_history(&mut self, at: Option<usize>) {
        self.history_selected = at;
    }

    /// A history row was clicked.
    pub fn click_history(&mut self, at: usize, history: &[SearchHistoryItem]) -> Vec<Effect> {
        let Some(item) = history_rows(history).get(at).map(|item| (*item).clone()) else {
            return Vec::new();
        };
        self.underlay = Some(item.value.clone());
        self.set_value_and_search(&item.value, true)
    }
}

/// The history in display order: people first, then searches, newest first.
pub(crate) fn history_rows(history: &[SearchHistoryItem]) -> Vec<&SearchHistoryItem> {
    let mut rows: Vec<&SearchHistoryItem> = history.iter().collect();
    let order = |kind: SearchHistoryKind| match kind {
        SearchHistoryKind::Contact => 0,
        SearchHistoryKind::Query => 1,
    };
    rows.sort_by(|a, b| {
        order(a.kind)
            .cmp(&order(b.kind))
            .then_with(|| b.timestamp.cmp(&a.timestamp))
    });
    rows
}

/// How a search is remembered: an address alone is a person, a name and address with a name an
/// address can't carry isn't remembered, and anything else is a query.
pub(crate) fn history_kind(value: &str) -> Option<SearchHistoryKind> {
    static ADDRESSED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#""?((?:[^\\"]|\\.)*)"?\s*<\s*([^<>]+@[^@<>]+)\s*>"#).expect("valid regex")
    });
    static VALID: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)^[a-z0-9!#$%&'*+/=?^_`{|}~-]+(?:\.[a-z0-9!#$%&'*+/=?^_`{|}~-]+)*@(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+[a-z0-9](?:[a-z0-9-]*[a-z0-9])$").expect("valid regex")
    });
    static WORD: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^[a-z0-9$%&'*+.\-/=?^_`{}|~ ]*$").expect("valid regex"));
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    // `Name <address>`, or an address alone.
    let (email, name) = match value
        .find('@')
        .filter(|at| *at > 0)
        .and(ADDRESSED.captures(value))
    {
        Some(captures) => (captures[2].trim().to_owned(), captures[1].trim().to_owned()),
        None if value.find('@').is_some_and(|at| at > 0) => (value.to_owned(), String::new()),
        None => (String::new(), value.to_owned()),
    };
    let name = if name == email { String::new() } else { name };
    match (VALID.is_match(&email), WORD.is_match(&name)) {
        (true, false) => None,
        (true, true) => Some(SearchHistoryKind::Contact),
        (false, _) => Some(SearchHistoryKind::Query),
    }
}

/// Curly quotes typed or pasted become straight ones.
fn normalize_quotes(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' | '\u{201a}' => '\'',
            '\u{201c}' | '\u{201d}' | '\u{201e}' => '"',
            c => c,
        })
        .collect()
}

/// An operator anywhere, and one at the end, before any spaces.
static OPERATOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(OPERATORS).expect("valid regex"));
static ENDS_WITH_OPERATOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"{OPERATORS}\s*$")).expect("valid regex"));

/// A completed search gets a space after it, unless it ends in an operator.
fn ensure_trailing_space(value: &str) -> String {
    if value.is_empty() || value.ends_with(' ') || ENDS_WITH_OPERATOR.is_match(value) {
        value.to_owned()
    } else {
        format!("{value} ")
    }
}

/// The bar's text when search opens over `list`: the view's own search, then the active filter's
/// token.
pub(crate) fn initial_query<'a>(
    list: &ListId,
    labels: &[Label],
    splits: impl IntoIterator<Item = &'a SplitInbox>,
    filter: Option<&str>,
) -> String {
    let query = match list {
        ListId::Inbox | ListId::Important | ListId::Other => String::new(),
        ListId::Done => "-in:inbox".into(),
        ListId::Starred => "is:starred".into(),
        ListId::Split(id) => splits
            .into_iter()
            .find(|split| split.id == *id)
            .map(|split| {
                split
                    .matcher
                    .gmail_query
                    .trim_start_matches("-in:DRAFT -in:CHAT ")
                    .to_owned()
            })
            .unwrap_or_default(),
        ListId::Sent => "in:sent".into(),
        ListId::Drafts => "in:draft".into(),
        ListId::Spam => "in:spam".into(),
        ListId::Trash => "in:trash".into(),
        ListId::Muted => "in:muted".into(),
        ListId::Scheduled => "in:scheduled".into(),
        ListId::Reminders => "in:reminders".into(),
        ListId::All => "in:all".into(),
        ListId::Label(id) => labels
            .iter()
            .find(|label| label.id == *id)
            .map(|label| format!("in:{}", exo_store::label_slug(&label.name)))
            .unwrap_or_default(),
    };
    let mut query = if query.is_empty() || query.ends_with(' ') {
        query
    } else {
        format!("{query} ")
    };
    if let Some(filter) = filter {
        query.push_str(filter);
        query.push(' ');
    }
    query
}

/// Registers search's keys for this frame: `/` to search, and while the bar has the keyboard,
/// its keys. → only completes with the caret at the end.
pub(crate) fn register(
    commands: &mut Commands<commands::Action>,
    search: Option<&Search>,
    can_search: bool,
) {
    const TOGGLE: &[Region] = &[
        Region::ThreadListHeader,
        Region::ThreadList,
        Region::ActionPane,
        Region::ThreadMessagesHeader,
        Region::ThreadMessages,
        Region::ThreadMessageCalendarInvite,
        Region::ReadOnlySnippet,
        Region::LabelSwitcher,
    ];
    let act = |action: Action| commands::Action::Search(action);
    if can_search {
        commands.add(
            Command::new("SEARCH", "Search", act(Action::Toggle))
                .keys(["/", "mod+/", "&"])
                .key_code(["/"])
                .aliases(["Look Up"])
                .default_score(1.6)
                .regions(TOGGLE),
        );
    }
    let Some(search) = search.filter(|search| search.focused) else {
        return;
    };
    let key = |id: &'static str, key: Key, keys: &[&'static str]| {
        Command::shortcut(id, act(Action::Key(key)))
            .keys(keys.iter().copied())
            .regions(SEARCH_BAR)
    };
    commands.add(key("SEARCH_ENTER", Key::Enter, &["enter", "shift+enter"]));
    commands.add(key("SEARCH_TAB", Key::Tab, &["tab"]));
    commands.add(key("SEARCH_UP", Key::Up, &["up"]));
    commands.add(key("SEARCH_DOWN", Key::Down, &["down"]));
    commands.add(key("SEARCH_ESCAPE", Key::Escape, &["esc"]));
    if search.caret_at_end {
        commands.add(key("SEARCH_RIGHT", Key::Right, &["right"]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search() -> Search {
        let system = |id: &str| Label {
            id: id.into(),
            name: id.into(),
            kind: exo_store::LabelKind::System,
            text_color: None,
            background_color: None,
            hidden: false,
        };
        let mut search = Search::new(
            "",
            &["INBOX", "SENT", "STARRED", "SPAM", "TRASH", "UNREAD"].map(system),
        );
        let person = |name: &str, email: &str| ScoredContact {
            email: email.into(),
            name: Some(name.into()),
            score: 1.0,
        };
        search.set_book(
            &Arc::new(vec![
                person("Freight Signals", "newsletter@freightsignals.com"),
                person("Ana Ruiz", "ana@letters.anaruiz.com"),
            ]),
            &BTreeMap::new(),
        );
        search
    }

    #[test]
    fn people_remove_suggestion_took_out_come_after_the_rest() {
        let mut search = search();
        let person = |name: &str, email: &str, score| ScoredContact {
            email: email.into(),
            name: Some(name.into()),
            score,
        };
        let book = Arc::new(vec![
            person("Ana Ruiz", "ana@example.com", 9.0),
            person("Ana Lee", "lee@example.com", 1.0),
        ]);
        search.set_book(&book, &BTreeMap::new());
        let emails = |search: &Search| {
            search
                .people("ana")
                .into_iter()
                .map(|(email, _)| email)
                .collect::<Vec<_>>()
        };
        assert_eq!(emails(&search), ["ana@example.com", "lee@example.com"]);
        search.set_book(
            &book,
            &BTreeMap::from([(
                "ana@example.com".to_owned(),
                "2026-11-19T10:00:00Z".to_owned(),
            )]),
        );
        assert_eq!(
            emails(&search),
            ["lee@example.com", "ana@example.com"],
            "still suggested, last"
        );
    }

    #[test]
    fn typing_an_operator_suggests_it_and_tab_keeps_the_bar_open() {
        let mut search = search();
        assert!(
            search.edit("fr").is_empty(),
            "the first suggestion is an operator: no contact"
        );
        assert_eq!(
            search.suggestions.as_ref().map(Vec::len),
            Some(2),
            "and Freight Signals"
        );
        assert_eq!(search.underlay.as_deref(), Some("from:"));
        assert_eq!(search.lozenge_start, Some(0));
        assert!(search.list_open());
        let effects = search.key(Key::Tab, &[]);
        assert_eq!(search.value, "from:");
        assert_eq!(effects, [], "an operator waits for its value");
        assert_eq!(search.suggestions, Some(Vec::new()));
    }

    #[test]
    fn a_contact_completes_its_address_and_enter_searches() {
        let mut search = search();
        let effects = search.edit("an");
        assert_eq!(
            effects,
            [Effect::Contact(Some(Person::new(
                "Ana Ruiz",
                "ana@letters.anaruiz.com"
            )))]
        );
        assert_eq!(search.underlay.as_deref(), Some("ana@letters.anaruiz.com"));
        // Enter without picking a suggestion searches what's typed.
        let effects = search.key(Key::Enter, &[]);
        assert_eq!(effects, [Effect::Run("an".into()), Effect::FocusList(None)]);
        assert_eq!(search.query(), Some("an"));
    }

    #[test]
    fn tab_then_enter_remembers_the_person() {
        let mut search = search();
        search.edit("an");
        assert_eq!(
            search.key(Key::Tab, &[]),
            [Effect::Preload("ana@letters.anaruiz.com ".into())],
            "Tab runs the search ahead, without showing it"
        );
        assert_eq!(search.value, "ana@letters.anaruiz.com ");
        let effects = search.key(Key::Enter, &[]);
        assert_eq!(
            effects,
            [
                Effect::Remember("ana@letters.anaruiz.com ".into()),
                Effect::Run("ana@letters.anaruiz.com ".into()),
                Effect::FocusList(None)
            ]
        );
    }

    #[test]
    fn a_pause_runs_the_highlighted_suggestion_ahead() {
        let mut search = search();
        search.edit("an");
        assert!(
            search
                .poll_instant(10.0)
                .1
                .is_some_and(|wait| (wait - INSTANT_DELAY).abs() < 1e-9),
            "timed from the frame that saw it"
        );
        search.edit("ana");
        assert_eq!(
            search.poll_instant(10.2).0,
            [],
            "typing again starts the wait over"
        );
        assert_eq!(search.poll_instant(10.4).0, []);
        assert_eq!(
            search.poll_instant(10.5).0,
            [Effect::Preload("ana@letters.anaruiz.com".into())],
            "the suggestion as it would search"
        );
        assert_eq!(search.poll_instant(11.0), (vec![], None), "once");

        // Nothing to suggest: what's typed.
        search.edit("zzqx");
        search.poll_instant(20.0);
        assert_eq!(
            search.poll_instant(20.3).0,
            [Effect::Preload("zzqx".into())]
        );
        // Enter drops what's waiting.
        search.edit("budget");
        search.poll_instant(30.0);
        assert_eq!(
            search.key(Key::Enter, &[]),
            [Effect::Run("budget".into()), Effect::FocusList(None)]
        );
        assert_eq!(search.poll_instant(31.0), (vec![], None));
        // The search showing isn't run ahead again.
        search.edit("budget ");
        search.poll_instant(40.0);
        assert_eq!(search.poll_instant(40.3).0, []);
        // An operator waiting for its value runs nothing.
        search.edit("budget from:");
        assert_eq!(search.poll_instant(50.0), (vec![], None));
    }

    #[test]
    fn escape_closes_suggestions_then_leaves() {
        let mut search = search();
        search.edit("in:se");
        assert!(search.list_open());
        assert!(search.key(Key::Escape, &[]).is_empty());
        assert!(!search.list_open());
        assert_eq!(search.key(Key::Escape, &[]), [Effect::Exit]);
    }

    #[test]
    fn arrows_pick_suggestions_and_wrap_to_none() {
        let mut search = search();
        search.edit("in:s");
        let count = search.suggestions.as_ref().map_or(0, Vec::len);
        assert!(count > 1);
        search.key(Key::Down, &[]);
        assert_eq!(search.selected, Some(0));
        search.key(Key::Up, &[]);
        assert_eq!(search.selected, None);
        search.key(Key::Up, &[]);
        assert_eq!(search.selected, Some(count - 1));
        let enter = search.key(Key::Enter, &[]);
        assert!(
            matches!(enter.as_slice(), [Effect::Run(_), Effect::FocusList(None)]),
            "{enter:?}"
        );
    }

    #[test]
    fn clearing_the_bar_stops_the_search() {
        let mut search = search();
        search.edit("zzqx");
        search.key(Key::Enter, &[]);
        assert_eq!(search.edit(""), [Effect::Contact(None), Effect::Stop]);
        assert!(search.history_open());
    }

    #[test]
    fn history_rows_cycle_into_the_bar() {
        let item = |value: &str, kind, timestamp: &str| SearchHistoryItem {
            value: value.into(),
            kind,
            timestamp: timestamp.into(),
        };
        let history = [
            item("budget", SearchHistoryKind::Query, "2026-11-16T10:00:00Z"),
            item(
                "ana@letters.anaruiz.com",
                SearchHistoryKind::Contact,
                "2026-11-10T10:00:00Z",
            ),
            item("pottery", SearchHistoryKind::Query, "2026-11-17T10:00:00Z"),
        ];
        let rows: Vec<&str> = history_rows(&history)
            .iter()
            .map(|item| item.value.as_str())
            .collect();
        assert_eq!(rows, ["ana@letters.anaruiz.com", "pottery", "budget"]);
        let mut search = search();
        let effects = search.key(Key::Down, &history);
        assert_eq!(
            effects,
            [Effect::Contact(Some(Person::new(
                "",
                "ana@letters.anaruiz.com"
            )))]
        );
        assert_eq!(search.value, "ana@letters.anaruiz.com");
        search.key(Key::Up, &history);
        assert_eq!(search.value, "", "past the top the bar empties");
    }

    #[test]
    fn remembered_kinds_follow_parse_contact() {
        assert_eq!(
            history_kind("ana@letters.anaruiz.com "),
            Some(SearchHistoryKind::Contact)
        );
        assert_eq!(
            history_kind("from:ana@x.com"),
            Some(SearchHistoryKind::Query)
        );
        assert_eq!(history_kind("budget"), Some(SearchHistoryKind::Query));
        assert_eq!(history_kind("  "), None);
    }

    #[test]
    fn search_opens_with_the_view_and_filter_in_the_bar() {
        assert_eq!(initial_query(&ListId::Inbox, &[], &[], None), "");
        assert_eq!(initial_query(&ListId::Done, &[], &[], None), "-in:inbox ");
        assert_eq!(
            initial_query(&ListId::Starred, &[], &[], None),
            "is:starred "
        );
        assert_eq!(
            initial_query(&ListId::Sent, &[], &[], Some("is:unread")),
            "in:sent is:unread "
        );
        assert_eq!(
            initial_query(&ListId::Inbox, &[], &[], Some("is:unread")),
            "is:unread "
        );
    }
}
