//! Search in the app: entering and leaving it, what the bar's effects do, and the results in the
//! thread list.

use std::sync::Arc;

use egui::{Rect, Ui, pos2, vec2};
use exo_store::SearchHistoryItem;

use super::{Action, Effect, Search, header_height, history_kind, initial_query};
use crate::{
    account::snippet_tokens,
    format::names::Person,
    inbox::{SearchView, View},
};

impl crate::Mail {
    /// The search bar is open.
    pub fn is_searching(&self) -> bool {
        self.search.is_some()
    }

    /// The search whose results show.
    pub fn search_query(&self) -> Option<&str> {
        self.search.as_ref()?.query()
    }

    /// The search bar's text.
    pub fn search_text(&self) -> Option<&str> {
        Some(self.search.as_ref()?.text())
    }

    /// The search bar has the keyboard.
    pub fn search_bar_focused(&self) -> bool {
        self.search.as_ref().is_some_and(Search::bar_focused)
    }

    /// The list's filter.
    pub fn filter(&self) -> Option<exo_store::Filter> {
        self.filter
    }

    /// Carries out a search command.
    pub(crate) fn search_run(&mut self, action: Action, now: f64) {
        match action {
            Action::Toggle => self.search_toggle(now),
            Action::Key(key) => {
                let history = self.search_history();
                let Some(search) = self.search.as_mut() else {
                    return;
                };
                let effects = search.key(key, &history);
                self.search_effects(effects, now);
            }
        }
    }

    /// `/`: search from the view on screen, or select the bar's text when already searching.
    fn search_toggle(&mut self, now: f64) {
        if self.search.is_some() {
            self.close_thread();
            self.search_focus_bar(true);
            return;
        }
        self.search_enter(None, now);
    }

    /// Enters search mode with the view's search in the bar, or with `run`, that search run at once
    /// (a contact's name clicked, keeping the contact).
    pub(crate) fn search_enter(&mut self, run: Option<&str>, now: f64) {
        self.close_thread();
        self.folders.close();
        if self.search.is_none() {
            self.search_cursor = self.inbox.cursor().map(|thread| thread.id.clone());
            // The list behind lets go of its sticky rows.
            self.account.unstick();
        }
        let settings = self.account.settings();
        let initial = match run {
            Some(query) => format!("{query} "),
            None => initial_query(
                &self.list,
                &self.snapshot.labels,
                settings.splits(),
                self.filter_search_suffix(),
            ),
        };
        self.inbox.clear();
        self.search_rows_from = None;
        let mut search = Search::new(&initial, &self.snapshot.labels);
        if run.is_none() {
            search.contact = Some(None);
        }
        let effects = if run.is_some() {
            search.key(super::Key::Enter, &[])
        } else {
            Vec::new()
        };
        self.search = Some(search);
        self.search_effects(effects, now);
    }

    /// Search mode with `query` run as it was, or with nothing run (a tab shown again).
    pub(crate) fn search_again(&mut self, query: Option<&str>, now: f64) {
        self.search_enter(None, now);
        let Some(query) = query else { return };
        let Some(search) = self.search.as_mut() else {
            return;
        };
        let mut effects = search.edit(query);
        effects.extend(search.key(super::Key::Enter, &[]));
        self.search_effects(effects, now);
    }

    /// Leaves search: back to the list, its cursor where it was.
    pub(crate) fn search_exit(&mut self, now: f64) {
        if self.search.take().is_none() {
            return;
        }
        self.account.search(None, 0);
        self.search_rows_from = None;
        self.rows_from = None;
        self.inbox.clear();
        let cursor = self.search_cursor.take();
        self.inbox.restore(&[], cursor, now);
    }

    /// The bar takes the keyboard back from the results: the results lose their cursor, remembered
    /// for coming back to the same search, and the contact pane keeps whom it showed. Given it by a
    /// key (`/`, Escape, ↑ on the first row), the bar's text is selected, so what's typed next
    /// replaces it; a click leaves the caret where it lands.
    pub(crate) fn search_focus_bar(&mut self, select: bool) {
        let cursor = self
            .inbox
            .cursor()
            .map(|thread| (thread.id.clone(), thread.contact.clone()));
        let Some(search) = &mut self.search else {
            return;
        };
        let (id, contact) = cursor.unzip();
        if let Some(contact) = contact
            && search.contact.is_none()
        {
            search.contact = Some(contact);
        }
        if select {
            search.select(id)
        } else {
            search.focus_bar(id)
        }
        self.inbox.clear_cursor();
    }

    /// The account's recent searches.
    fn search_history(&self) -> Vec<SearchHistoryItem> {
        self.account.settings().search_history.clone()
    }

    /// Does what the bar asked.
    fn search_effects(&mut self, effects: Vec<Effect>, now: f64) {
        for effect in effects {
            match effect {
                Effect::Run(query) => {
                    self.account.search(
                        Some(&query),
                        snippet_tokens(self.ctx.content_rect().width()),
                    );
                    self.search_rows_from = None;
                    self.inbox.clear();
                }
                Effect::Preload(query) => self
                    .account
                    .preload_search(&query, snippet_tokens(self.ctx.content_rect().width())),
                Effect::Stop => {
                    self.account.search(None, 0);
                    self.search_rows_from = None;
                    self.inbox.clear();
                }
                Effect::FocusList(prefer) => {
                    if let Some(search) = &mut self.search {
                        search.focus_list_when_listed(prefer);
                    }
                    self.search_take_focus(now);
                }
                Effect::Contact(person) => {
                    if let Some(search) = &mut self.search {
                        search.contact = Some(person);
                    }
                }
                Effect::Remember(value) => self.search_remember(&value),
                Effect::Exit => self.search_exit(now),
            }
        }
    }

    /// Remembers `value` in the search history.
    pub(crate) fn search_remember(&mut self, value: &str) {
        if let Some(kind) = history_kind(value) {
            let now = jiff::Timestamp::now();
            self.account
                .update_settings(|settings| settings.remember_search(value, kind, now));
        }
    }

    /// Opening a result remembers its search.
    pub(crate) fn search_opened_thread(&mut self) {
        if let Some(query) = self
            .search
            .as_ref()
            .and_then(|search| search.query())
            .map(str::to_owned)
        {
            self.search_remember(&query);
        }
    }

    /// Takes the account's answers into search: people for the suggestions, and the results
    /// into the list. Call after the list's own snapshot is taken.
    pub(crate) fn search_sync(&mut self) {
        let snapshot = self.snapshot.clone();
        let history = self.search_history();
        let now = self.ctx.input(|input| input.time);
        let Some(search) = self.search.as_mut() else {
            return;
        };
        search.set_labels(&snapshot.labels);
        match &snapshot.address_book {
            Some(book) => {
                let effects = search.set_book(book, &self.account.settings().excluded_contacts);
                self.search_effects(effects, now);
            }
            None if !search.book_asked => {
                search.book_asked = true;
                self.account.load_address_book();
            }
            None => {}
        }
        let query = self
            .search
            .as_ref()
            .and_then(|search| search.query().map(str::to_owned));
        let results = snapshot
            .search
            .as_ref()
            .filter(|results| query.as_deref() == Some(results.query.as_str()));
        if let Some(results) = results
            && !self
                .search_rows_from
                .as_ref()
                .is_some_and(|taken| Arc::ptr_eq(taken, results))
        {
            self.inbox.set_threads(results.threads.clone());
            self.search_rows_from = Some(results.clone());
            // While the bar has the keyboard the results have no cursor, however they change;
            // focusing the list below gives them one.
            if self.search.as_ref().is_some_and(Search::bar_focused) {
                self.inbox.clear_cursor();
            }
        }
        // The list takes the keyboard once there are results (each frame, as the list may change
        // under it).
        self.search_take_focus(now);
        let Some(search) = self.search.as_ref() else {
            return;
        };
        let status = results.and_then(|results| {
            if results.offline {
                Some(crate::inbox::PARTIAL_OFFLINE)
            } else {
                (results.finished && results.threads.is_empty())
                    .then_some(crate::inbox::NOTHING_FOUND)
            }
        });
        self.inbox.set_view(View {
            title: String::new(),
            more: results.is_some_and(|results| results.more),
            loading: query.is_some() && results.is_none_or(|results| !results.finished),
            archivable: true,
            search: Some(SearchView {
                header: header_height(search, &history),
                status,
            }),
            ..View::default()
        });
        self.inbox.set_bar(!search.bar_focused());
    }

    /// Gives the results the keyboard if the bar is waiting for them to.
    fn search_take_focus(&mut self, now: f64) {
        let finished = self
            .search_rows_from
            .as_ref()
            .is_some_and(|results| results.finished);
        let ids: Vec<String> = self
            .inbox
            .threads()
            .iter()
            .map(|thread| thread.id.clone())
            .collect();
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        let Some(cursor) = self
            .search
            .as_mut()
            .and_then(|search| search.results_listed(&ids, finished))
        else {
            return;
        };
        match cursor {
            Some(id) => self.inbox.restore(&[], Some(id), now),
            None => self.inbox.cursor_to_top(),
        }
    }

    /// The search header over the list view, and what its clicks do; with no search to show,
    /// the hint where the list would be.
    pub(crate) fn search_show(
        &mut self,
        ui: &mut Ui,
        content: Rect,
        rem: f32,
        window_width: f32,
        now: f64,
    ) {
        let history = self.search_history();
        let Some(search) = self.search.as_mut() else {
            return;
        };
        let compact = window_width <= 1152.0;
        // The list's side padding.
        let view = content.shrink2(vec2(6.0, 0.0));
        if search.query().is_none() && history.is_empty() {
            let list = Rect::from_min_max(
                pos2(view.left(), view.top() + header_height(search, &history)),
                view.max,
            );
            super::show_empty(ui.painter(), list, rem, compact);
        }
        let response = super::show_bar(ui, view, rem, compact, search, &history);
        if response.focus {
            self.search_focus_bar(false);
        }
        self.search_effects(response.effects, now);
        if response.close {
            self.search_exit(now);
        }
        // Instant search, timed from the frame that typed or highlighted what it runs.
        let Some(search) = self.search.as_mut() else {
            return;
        };
        let (effects, wait) = search.poll_instant(now);
        if let Some(wait) = wait {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
        self.search_effects(effects, now);
    }

    /// Whom the right pane shows while searching outside a thread: someone a suggestion named,
    /// else the results' cursor's contact. `None` with nobody to show (the pane shows Tips).
    pub(crate) fn search_pane_person(&self) -> Option<Option<Person>> {
        let search = self.search.as_ref()?;
        if let Some(person) = &search.contact {
            return Some(person.clone());
        }
        let id = self
            .pane_thread
            .as_deref()
            .filter(|_| search.query().is_some());
        Some(
            id.and_then(|id| self.inbox.threads().iter().find(|thread| thread.id == id))
                .and_then(|thread| thread.contact.clone()),
        )
    }
}
