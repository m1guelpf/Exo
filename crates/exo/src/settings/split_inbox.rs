//! The split inbox dialogs: Split Inbox Settings (its Active and Inactive tabs), New and Edit
//! Split Inbox, Hide Empty Split Inboxes, their alerts, and the actions behind them (creating,
//! replacing, turning on and off, reordering and deleting splits, and turning Important and Other
//! on and off).
//!
//! With the split library (`split_library`, `split_template`), Add Split Inbox opens the library
//! and a split from a template opens the template's settings. Without it, Add Split Inbox opens
//! New Split Inbox and a built-in split opens Edit Split Inbox. Important, Other and Inbox, whose
//! settings only the library describes, turn Important and Other off (after a confirmation) or on.

use std::sync::Arc;

use egui::{Color32, Context, CornerRadius, Rect, Sense, Ui, pos2, vec2};
use exo_commands::{Commands, Toast};
use exo_store::{
    Definitions, ListId, Me, Settings, SplitChanges, SplitEntry, SplitInbox, SplitLibrary,
    SplitQuery, SystemSplit, Template, clean_whitespace, name_error, query_too_long,
};
use exo_ui::{
    ActionKind, Alert, CHECKBOX_SIZE, DIALOG_ACTION_HEIGHT, DIALOG_WIDTH, Dialog, DialogAction,
    DialogInput, Family, Font, PARAGRAPH, TextStyle, motion, paint_checkbox, paint_icon, palette,
};

use super::split_library::{self, Library, LibraryEvent};
use super::split_template::{Kind, TemplateAction, TemplateEvent, TemplateForm};
use crate::account::Account;
use crate::splits;
use crate::{compose::contacts, contact_picker::PickerKey};

/// The dialogs' content width: the dialog's 550px less `padding: 20px 32px`.
const CONTENT_WIDTH: f32 = DIALOG_WIDTH - 64.0;
/// Body text: 12px/20px, nudged down 3px.
const BODY: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
const BODY_BOLD: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 12.0, 20.0);
const NUDGE: f32 = 3.0;
/// The Hide Empty link, checkbox labels and Hide Empty's rows: 12px/16px, nudged down 1px.
const SMALL: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
/// A row: `padding: 4px 32px 8px` around a title and subheader line.
const ROW_HEIGHT: f32 = 52.0;
/// A row's gear, `padding: 6px` around 14px, 7px past the content (`margin-right: -7px`), and its
/// drag handle, 6px around 12×10.
const GEAR_SIZE: f32 = 26.0;
const HANDLE_SIZE: [f32; 2] = [24.0, 22.0];
/// A Hide Empty Split Inboxes row: `padding: 7px 64px` around a 16px label, with a 27px checkbox.
const HIDE_ROW_HEIGHT: f32 = 40.0;

/// Split Inbox Settings' tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ListTab {
    /// The splits shown as tabs, Important and Other (or Inbox) among them.
    #[default]
    Active,
    /// The splits that are off.
    Inactive,
}

/// Which split screen the dialog shows.
#[derive(Debug, Default)]
enum Screen {
    /// Split Inbox Settings (`basic`).
    #[default]
    List,
    /// New or Edit Split Inbox.
    Form(Form),
    /// Hide Empty Split Inboxes.
    HideEmpty,
    /// The Split Inbox Library.
    Library(Library),
    /// A template's settings, for its split or a new one.
    Template(TemplateForm, Back),
}

/// Where a form or a template's settings go back to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Back {
    /// Split Inbox Settings (`basic`).
    #[default]
    List,
    /// The library, as it was.
    Library,
    /// Nowhere: the dialog closes (`newFromCommand`).
    Close,
}

/// The New/Edit Split Inbox form's state.
#[derive(Debug, Default)]
struct Form {
    /// The split being edited, by id; `None` for a new one.
    editing: Option<String>,
    /// Where closing goes.
    back: Back,
    name: String,
    query: String,
    leave: bool,
    hide: bool,
    /// The name's inline error.
    name_error: Option<&'static str>,
    /// The query is marked wrong since its alert.
    query_error: bool,
    /// The field to focus on the next frame.
    focus: Option<Field>,
    /// Edit Split Inbox just opened: the name has the keyboard with nothing selected, the caret
    /// after the name.
    opening: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Name,
    Query,
}

/// An alert over the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
enum AlertKind {
    /// A definition that can't define a split (`Sorry!`).
    Invalid(String),
    /// `Discard changes?`.
    Discard,
    /// `Delete Split`, for the split being edited.
    Delete,
    /// `This makes your Split Inbox queries too long.`
    TooLong,
    /// `Almost there…`: turn Important and Other off.
    TurnOffImportantOther,
    /// `Almost there…`: a template's name is a split's already.
    NameTaken,
    /// `Delete Split`, from a template's settings.
    DeleteTemplateSplit,
    /// `Hide empty Split Inboxes for all Splits?`, from the nudge's `Try it out`.
    HideAll,
}

impl AlertKind {
    fn title(&self) -> &'static str {
        match self {
            Self::Invalid(_) | Self::TooLong => "Sorry!",
            Self::Discard => "Discard changes?",
            Self::Delete | Self::DeleteTemplateSplit => "Delete Split",
            Self::TurnOffImportantOther | Self::NameTaken => "Almost there…",
            Self::HideAll => "Hide empty Split Inboxes for all Splits?",
        }
    }

    fn actions(&self) -> &'static [&'static str] {
        match self {
            Self::Invalid(_) => &["OK"],
            Self::TooLong => &["Ok"],
            Self::Discard => &["Discard", "Cancel"],
            Self::Delete | Self::DeleteTemplateSplit => &["Delete", "Cancel"],
            Self::TurnOffImportantOther => &["Confirm", "Cancel"],
            Self::NameTaken => &["OK"],
            Self::HideAll => &["Turn on for all splits", "Cancel"],
        }
    }
}

/// What the user did in a split dialog, or a key for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// `Add Split Inbox`, or Enter on the list.
    AddSplit,
    /// The `Active` or `Inactive` tab.
    SelectTab(ListTab),
    /// A row of the list (or its gear): the settings of the split, or of Important and Other.
    OpenRow(ListId),
    /// A toast's `Split Inbox Settings`: edit split `id`.
    Edit(String),
    /// An Active row dragged from one index to another.
    Reorder(usize, usize),
    /// Edit Split Inbox's `Turn On Split Inbox` or `Turn Off Split Inbox`.
    Toggle,
    /// The `Hide Empty Split Inboxes` link.
    ShowHideEmpty,
    /// Save or Create, or Enter in the form.
    Save,
    /// `Delete Split Inbox`.
    Delete,
    /// Cancel, the back arrow, Escape or the backdrop: close, asking first if anything changed.
    Close,
    /// The form's first checkbox.
    ToggleLeave,
    /// The form's second checkbox.
    ToggleHide,
    /// Hide Empty's row for a split, or Other.
    ToggleHideEmpty(ListId),
    /// Hide Empty's `Select all` / `Unselect all`.
    HideAll,
    /// The hide-empty nudge's `Try it out`: asks to hide every empty split.
    OfferHideAll,
    /// `Split Inboxes updated.`'s Undo: these show while empty again.
    UndoHideAll(Vec<ListId>),
    /// Something in the library.
    Library(LibraryEvent),
    /// Something in a template's settings.
    Template(TemplateEvent),
    /// Enter in a template's settings.
    TemplateEnter,
    /// Template `id`'s settings (`template:<id>`), from a toast's `Split Inbox Settings` or the
    /// palette's `VIP Split Senders` and `News Split Senders`, going back where it says; without
    /// the library, New Split Inbox.
    OpenTemplate(String, Back),
    /// New Split Inbox, going `back` when closed: the palette's `New Split Inbox from scratch`.
    New(Back),
    /// The alert's action, by index.
    AlertAction(usize),
    /// Enter or Space in the alert.
    AlertConfirm,
    /// Escape or a click outside the alert.
    AlertDismiss,
    /// The arrows or Tab in the alert: move the selection.
    AlertCycle(isize),
}

/// What a split action asks of the app.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Show this list.
    pub show: Option<ListId>,
    /// Close the dialog.
    pub close: bool,
}

/// The split dialogs' state.
#[derive(Debug, Default)]
pub struct SplitDialogs {
    screen: Screen,
    /// The list's tab, kept while the dialog moves between its screens.
    tab: ListTab,
    /// A click on the list took the keyboard from Add Split Inbox, which has it each time the
    /// list shows.
    list_blurred: bool,
    alert: Option<(AlertKind, usize)>,
    /// The list's scroll offset, and its scrollbar.
    scroll: f32,
    scrollbar: exo_ui::Scrollbar,
    /// A row being dragged: its index and how far the pointer has moved it.
    drag: Option<(usize, f32)>,
    /// The split library, when there is one.
    library: Option<Arc<SplitLibrary>>,
    /// The library as a template opened from it left it.
    library_state: Option<Library>,
    /// A split just made that's hidden when empty, which says so when it first empties.
    created_hidden: Option<String>,
}

impl SplitDialogs {
    /// An alert is up: it has the keyboard.
    pub fn alert_open(&self) -> bool {
        self.alert.is_some()
    }

    /// The split just made that's hidden when empty, if one was, once.
    pub fn take_created_hidden(&mut self) -> Option<String> {
        self.created_hidden.take()
    }

    /// Uses `library` for the library and templates' settings.
    pub fn set_library(&mut self, library: Option<SplitLibrary>) {
        self.library = library.map(Arc::new);
    }

    /// The split library, when Exo has a copy.
    pub fn library(&self) -> Option<&SplitLibrary> {
        self.library.as_deref()
    }

    /// The dialogs as they start, keeping the library.
    fn reset(&mut self) {
        *self = Self {
            library: self.library.take(),
            created_hidden: self.created_hidden.take(),
            ..Self::default()
        };
    }

    /// Opens Split Inbox Settings on its Active tab.
    pub fn open_list(&mut self) {
        self.reset();
    }

    /// Opens New Split Inbox; closing it goes `back`.
    pub fn open_new(&mut self, back: Back) {
        self.alert = None;
        self.screen = Screen::Form(Form {
            back,
            focus: Some(Field::Name),
            ..Form::default()
        });
    }

    /// Opens Hide Empty Split Inboxes.
    pub fn open_hide_empty(&mut self) {
        self.reset();
        self.screen = Screen::HideEmpty;
    }

    /// Opens the library, or New Split Inbox without one; either goes back
    /// to Split Inbox Settings.
    pub fn open_library(&mut self) {
        self.alert = None;
        if self.library.is_some() {
            self.screen = Screen::Library(self.library_state.take().unwrap_or_default());
        } else {
            self.open_new(Back::List);
        }
    }

    /// Opens the settings for template `id` (or a built-in kind's), going `back`; false when
    /// there's no such template.
    fn open_template(&mut self, settings: &Settings, id: &str, back: Back) -> bool {
        let Some(template) = self
            .library
            .as_ref()
            .and_then(|library| library.template(id))
        else {
            return false;
        };
        self.alert = None;
        self.screen = Screen::Template(TemplateForm::new(template, settings), back);
        true
    }

    /// The settings for `split`: its template's, else Edit Split Inbox;
    /// both go back to the list.
    fn open_split(&mut self, settings: &Settings, id: &str) {
        let template = settings
            .split(id)
            .and_then(|split| {
                self.library
                    .as_ref()
                    .and_then(|library| library.template_for(split))
            })
            .map(|template| template.id.clone());
        if !template.is_some_and(|template| self.open_template(settings, &template, Back::List)) {
            self.open_edit(settings, id);
        }
    }

    /// Opens Edit Split Inbox for split `id` (`custom:<slug>`), back to the list on close.
    fn open_edit(&mut self, settings: &Settings, id: &str) {
        let Some(split) = settings.split(id) else {
            return;
        };
        self.alert = None;
        self.screen = Screen::Form(Form {
            editing: Some(split.id.clone()),
            back: Back::List,
            name: split.name().to_owned(),
            query: split.query().trim().to_owned(),
            leave: split.leave_threads_in_important_other,
            hide: split.hides_when_empty(),
            focus: Some(Field::Name),
            opening: true,
            ..Form::default()
        });
    }

    /// Carries out `event`. `current` is the list on screen; `wrap` makes app actions of
    /// settings actions, for toasts.
    pub fn run<A: Clone + 'static>(
        &mut self,
        event: Event,
        account: &mut Account,
        commands: &mut Commands<A>,
        current: &ListId,
        wrap: &impl Fn(super::Action) -> A,
    ) -> Outcome {
        let mut outcome = Outcome::default();
        match event {
            Event::AddSplit => self.open_library(),
            Event::New(back) => self.open_new(back),
            Event::SelectTab(tab) => self.tab = tab,
            Event::OpenRow(list) => {
                let settings = account.settings().clone();
                let template = if list == ListId::Inbox {
                    "splitTemplate_inbox"
                } else {
                    "splitTemplate_importantOther"
                };
                match list {
                    ListId::Split(id) => self.open_split(&settings, &id),
                    _ if self.open_template(&settings, template, Back::List) => {}
                    ListId::Important | ListId::Other if !settings.unified_inbox => {
                        self.alert = Some((AlertKind::TurnOffImportantOther, 0))
                    }
                    _ => outcome.show = Some(toggle_important_other(account, commands, wrap)),
                }
            }
            Event::Edit(id) => {
                let settings = account.settings().clone();
                self.open_split(&settings, &id);
            }
            // Without the template (no library), New Split Inbox.
            Event::OpenTemplate(id, back) => {
                let settings = account.settings().clone();
                if !self.open_template(&settings, &id, back) {
                    self.reset();
                    self.open_new(back);
                }
            }
            Event::Library(LibraryEvent::Open(id)) => {
                if let Screen::Library(state) = std::mem::take(&mut self.screen) {
                    self.library_state = Some(state);
                }
                let settings = account.settings().clone();
                self.open_template(&settings, &id, Back::Library);
            }
            Event::Library(LibraryEvent::BuildYourOwn) => {
                if let Screen::Library(state) = std::mem::take(&mut self.screen) {
                    self.library_state = Some(state);
                }
                self.open_new(Back::Library);
            }
            Event::Template(event) => {
                outcome = self.template_event(event, account, commands, current, wrap)
            }
            Event::TemplateEnter => {
                let enter = match (&self.screen, &self.library) {
                    (Screen::Template(form, _), Some(library)) => library
                        .templates
                        .get(&form.template)
                        .and_then(|template| form.enter(template)),
                    _ => None,
                };
                if let Some(event) = enter {
                    outcome = self.template_event(event, account, commands, current, wrap);
                }
            }
            Event::Reorder(from, to) => {
                let rows: Vec<String> = active_rows(account.settings())
                    .iter()
                    .map(|entry| entry.external_id().to_owned())
                    .collect();
                if let (Some(moved), Some(pushed)) = (rows.get(from), rows.get(to)) {
                    account.update_settings(|settings| settings.reorder_splits(moved, pushed));
                }
            }
            Event::Toggle => outcome = self.toggle_edited(account, commands, wrap, current),
            Event::ShowHideEmpty => self.open_hide_empty(),
            Event::Save => outcome = self.save(account, commands, wrap, current),
            Event::Delete => {
                if matches!(
                    self.screen,
                    Screen::Form(Form {
                        editing: Some(_),
                        ..
                    })
                ) {
                    self.alert = Some((AlertKind::Delete, 0));
                }
            }
            Event::Close => outcome = self.close(account),
            Event::ToggleLeave => {
                if let Screen::Form(form) = &mut self.screen {
                    form.leave = !form.leave;
                }
            }
            Event::ToggleHide => {
                if let Screen::Form(form) = &mut self.screen {
                    form.hide = !form.hide;
                }
            }
            Event::ToggleHideEmpty(list) => account.update_settings(|settings| {
                let hide = !hides_when_empty(settings, &list);
                set_hides_when_empty(settings, &list, hide);
            }),
            Event::HideAll => account.update_settings(|settings| {
                let rows: Vec<ListId> = hide_empty_rows(settings)
                    .iter()
                    .map(SplitEntry::list)
                    .collect();
                let hide = !rows.iter().all(|list| hides_when_empty(settings, list));
                for list in rows {
                    set_hides_when_empty(settings, &list, hide);
                }
            }),
            Event::OfferHideAll => self.alert = Some((AlertKind::HideAll, 0)),
            Event::UndoHideAll(lists) => account.update_settings(|settings| {
                for list in &lists {
                    set_hides_when_empty(settings, list, false);
                }
            }),
            Event::AlertConfirm => {
                if let Some((_, selected)) = self.alert {
                    outcome = self.run(
                        Event::AlertAction(selected),
                        account,
                        commands,
                        current,
                        wrap,
                    );
                }
            }
            Event::AlertAction(index) => {
                outcome = self.alert_action(index, account, commands, wrap)
            }
            Event::AlertDismiss => self.alert = None,
            Event::AlertCycle(step) => {
                if let Some((kind, selected)) = &mut self.alert {
                    let count = isize::try_from(kind.actions().len()).unwrap_or(1);
                    *selected = usize::try_from(
                        (isize::try_from(*selected).unwrap_or(0) + step).rem_euclid(count),
                    )
                    .unwrap_or(0);
                }
            }
        }
        outcome
    }

    /// Validates the form's name and definition, showing what's wrong, and
    /// returns them cleaned.
    fn validate(&mut self, account: &Account) -> Option<(String, String)> {
        let Screen::Form(form) = &mut self.screen else {
            return None;
        };
        let settings = account.settings();
        let others: Vec<SplitInbox> = settings
            .splits()
            .filter(|split| Some(&split.id) != form.editing.as_ref())
            .cloned()
            .collect();
        let name = clean_whitespace(&form.name);
        let query = clean_whitespace(&form.query);
        // A built-in split keeps its own name (Calendar), unchecked: a template's settings don't
        // check names either.
        let builtin_name = form
            .editing
            .as_ref()
            .and_then(|id| settings.split(id))
            .is_some_and(|split| split.is_default() && split.has_same_name(&name));
        form.name_error = if builtin_name {
            None
        } else {
            name_error(&name, &others)
        };
        if form.name_error.is_some() {
            form.focus = Some(Field::Name);
        }
        let (labels, _) = definitions(account);
        let index = exo_store::LabelIndex::new(
            [],
            labels
                .iter()
                .map(|label| (label.id.as_str(), label.name.as_str())),
        );
        if let Some(invalid) = SplitQuery::parse(&query).invalidity(&index) {
            self.alert = Some((AlertKind::Invalid(invalid.text), 0));
            return None;
        }
        (form.name_error.is_none() && !name.is_empty() && !query.is_empty())
            .then_some((name, query))
    }

    /// Validates and saves the form.
    fn save<A: Clone + 'static>(
        &mut self,
        account: &mut Account,
        commands: &mut Commands<A>,
        wrap: &impl Fn(super::Action) -> A,
        current: &ListId,
    ) -> Outcome {
        let Some((name, query)) = self.validate(account) else {
            return Outcome::default();
        };
        let Screen::Form(form) = &self.screen else {
            return Outcome::default();
        };
        let settings = account.settings().clone();
        let (labels, me) = definitions(account);
        let definitions = Definitions {
            labels: &labels,
            me: &me,
        };
        match form.editing.clone() {
            Some(id) => {
                let Some(old) = settings.split(&id) else {
                    return Outcome::default();
                };
                let new = old.update(&form.changes(old, name, query), definitions);
                if !replace(account, commands, wrap, old, &new, true) {
                    self.alert = Some((AlertKind::TooLong, 0));
                    return Outcome::default();
                }
                self.screen = Screen::List;
                // A redefined split has a new id: keep showing it.
                let show = (*current == ListId::Split(id)).then(|| ListId::Split(new.id.clone()));
                Outcome { show, close: false }
            }
            None => {
                let mut split = SplitInbox::new(&name, &query, None, None, definitions);
                split.leave_threads_in_important_other = form.leave;
                split.hide_split_when_empty = Some(form.hide);
                if !add(account, commands, wrap, &split) {
                    self.alert = Some((AlertKind::TooLong, 0));
                    return Outcome::default();
                }
                self.created_hidden = form.hide.then(|| split.id.clone());
                let tab = self.tab;
                self.reset();
                self.tab = tab;
                Outcome {
                    show: Some(ListId::Split(split.id)),
                    close: true,
                }
            }
        }
    }

    /// Edit Split Inbox's `Turn On Split Inbox` / `Turn Off Split Inbox`:
    /// toggles the split with the form's changes, then closes the dialog when it turned on, or
    /// goes back to the list.
    fn toggle_edited<A: Clone + 'static>(
        &mut self,
        account: &mut Account,
        commands: &mut Commands<A>,
        wrap: &impl Fn(super::Action) -> A,
        current: &ListId,
    ) -> Outcome {
        let Screen::Form(Form {
            editing: Some(id), ..
        }) = &self.screen
        else {
            return Outcome::default();
        };
        let Some(split) = account.settings().split(id).cloned() else {
            return Outcome::default();
        };
        let changes = if matches!(&self.screen, Screen::Form(form) if form.has_changes(account.settings()))
        {
            let Some((name, query)) = self.validate(account) else {
                return Outcome::default();
            };
            let Screen::Form(form) = &self.screen else {
                return Outcome::default();
            };
            form.changes(&split, name, query)
        } else {
            SplitChanges::default()
        };
        let show = toggle(account, commands, wrap, &split, changes, current);
        if split.is_disabled {
            let tab = self.tab;
            self.reset();
            self.tab = tab;
            Outcome { show, close: true }
        } else {
            self.screen = Screen::List;
            Outcome { show, close: false }
        }
    }

    /// Carries out something done in a template's settings.
    fn template_event<A: Clone + 'static>(
        &mut self,
        event: TemplateEvent,
        account: &mut Account,
        commands: &mut Commands<A>,
        current: &ListId,
        wrap: &impl Fn(super::Action) -> A,
    ) -> Outcome {
        let Some(library) = self.library.clone() else {
            return Outcome::default();
        };
        let Screen::Template(form, _) = &mut self.screen else {
            return Outcome::default();
        };
        let Some(template) = library.templates.get(&form.template) else {
            return Outcome::default();
        };
        match event {
            TemplateEvent::Add => {
                form.add(template);
                form.focus = true;
            }
            TemplateEvent::Pick(sender) => form.pick(sender),
            TemplateEvent::PickerKey(key) => {
                if let Some(sender) = form.picker.key(key) {
                    form.pick(sender);
                }
            }
            TemplateEvent::Remove(index) => {
                if index < form.senders.len() {
                    form.senders.remove(index);
                }
                form.focus = true;
            }
            TemplateEvent::ToggleLeave => form.leave = !form.leave,
            TemplateEvent::ToggleHide => {
                form.hide = !form.hide;
                // Important • Other's applies at once.
                if form.kind == Kind::ImportantOther {
                    let hide = form.hide;
                    account.update_settings(|settings| settings.set_hide_other_when_empty(hide));
                }
            }
            TemplateEvent::Action(TemplateAction::Reset) => form.reset(template),
            TemplateEvent::Action(TemplateAction::Delete) => {
                self.alert = Some((AlertKind::DeleteTemplateSplit, 0))
            }
            TemplateEvent::Action(TemplateAction::Toggle) => {
                return self.template_save(template, true, account, commands, current, wrap);
            }
            TemplateEvent::Action(TemplateAction::Primary) => {
                return self.template_primary(template, account, commands, current, wrap);
            }
        }
        Outcome::default()
    }

    /// The primary button: saves the template's split, turns it on, or creates it.
    fn template_primary<A: Clone + 'static>(
        &mut self,
        template: &Template,
        account: &mut Account,
        commands: &mut Commands<A>,
        current: &ListId,
        wrap: &impl Fn(super::Action) -> A,
    ) -> Outcome {
        let Screen::Template(form, _) = &self.screen else {
            return Outcome::default();
        };
        if let Some(split) = form
            .split
            .as_ref()
            .and_then(|id| account.settings().split(id))
        {
            let toggle = split.is_disabled;
            return self.template_save(template, toggle, account, commands, current, wrap);
        }
        if account
            .settings()
            .splits()
            .any(|split| split.has_same_name(&template.name))
        {
            self.alert = Some((AlertKind::NameTaken, 0));
            return Outcome::default();
        }
        // Creates the split with the template's name, definition, kind and id.
        let (labels, me) = definitions(account);
        let mut split = SplitInbox::new(
            &template.name,
            &template.query_with(&form.senders),
            template.kind.as_deref(),
            Some(&template.id),
            Definitions {
                labels: &labels,
                me: &me,
            },
        );
        split.leave_threads_in_important_other = form.leave;
        split.hide_split_when_empty = Some(form.hide);
        if too_long(
            account.settings(),
            None,
            &split,
            &Me {
                addresses: &me,
                name: None,
            },
        ) {
            self.alert = Some((AlertKind::TooLong, 0));
            return Outcome::default();
        }
        account.update_settings(|settings| {
            settings.add_split(split.clone());
            settings.engage_splits();
        });
        let settings = Event::OpenTemplate(template.id.clone(), Back::List);
        commands.toast(
            Toast::new(format!("Split Inbox turned on: {}", split.name())).action(
                "Split Inbox Settings",
                None,
                wrap(super::Action::Split(settings)),
            ),
        );
        self.created_hidden = form.hide.then(|| split.id.clone());
        self.reset();
        Outcome {
            show: Some(ListId::Split(split.id)),
            close: true,
        }
    }

    /// Saves the template's split, turning it on or off when `toggle`; Important • Other
    /// and Inbox turn Important and Other on or off.
    fn template_save<A: Clone + 'static>(
        &mut self,
        template: &Template,
        toggle: bool,
        account: &mut Account,
        commands: &mut Commands<A>,
        current: &ListId,
        wrap: &impl Fn(super::Action) -> A,
    ) -> Outcome {
        let Screen::Template(form, back) = &self.screen else {
            return Outcome::default();
        };
        let back = *back;
        match form.kind {
            Kind::ImportantOther if form.was_enabled => {
                self.alert = Some((AlertKind::TurnOffImportantOther, 0));
                return Outcome::default();
            }
            Kind::ImportantOther | Kind::Inbox => {
                let show = Some(toggle_important_other(account, commands, wrap));
                self.reset();
                return Outcome { show, close: true };
            }
            Kind::Split => {}
        }
        let Some(old) = form
            .split
            .as_ref()
            .and_then(|id| account.settings().split(id))
            .cloned()
        else {
            return Outcome::default();
        };
        let editable = template
            .properties
            .senders
            .as_ref()
            .is_some_and(|senders| senders.is_editable);
        let mut changes = SplitChanges {
            leave_threads_in_important_other: Some(form.leave),
            hide_split_when_empty: Some(form.hide),
            ..SplitChanges::default()
        };
        if editable {
            changes.query = Some(template.query_with(&form.senders));
        }
        let changed = form.has_changes(template);
        if toggle && !changed {
            // Turns it on or off, which closes the dialog when the split turns on.
            let show = self::toggle(
                account,
                commands,
                wrap,
                &old,
                SplitChanges::default(),
                current,
            );
            let outcome = if old.is_disabled {
                self.reset();
                Outcome {
                    show: None,
                    close: true,
                }
            } else {
                self.leave_form(back)
            };
            return Outcome { show, ..outcome };
        }
        if toggle {
            changes.is_disabled = Some(!old.is_disabled);
        }
        let (labels, me) = definitions(account);
        let mut new = old.update(
            &changes,
            Definitions {
                labels: &labels,
                me: &me,
            },
        );
        if new.kind != template.kind {
            new.kind.clone_from(&template.kind);
        }
        if too_long(
            account.settings(),
            Some(&old),
            &new,
            &Me {
                addresses: &me,
                name: None,
            },
        ) {
            self.alert = Some((AlertKind::TooLong, 0));
            return Outcome::default();
        }
        account.update_settings(|settings| {
            settings.replace_split(&old.id, new.clone());
            if !new.is_disabled {
                settings.engage_splits();
            }
        });
        // The toast for the template's changes.
        let text = if new.is_disabled != old.is_disabled {
            format!(
                "Split Inbox turned {}: {}",
                if new.is_disabled { "off" } else { "on" },
                new.name()
            )
        } else if new.is_default() {
            format!("{} Split updated.", new.name())
        } else {
            format!("Split '{}' updated.", new.name())
        };
        commands.toast(Toast::new(text).action(
            "Split Inbox Settings",
            None,
            wrap(super::Action::Split(Event::Edit(new.id.clone()))),
        ));
        let show = (*current == ListId::Split(old.id.clone())).then(|| {
            if new.is_disabled {
                splits::home(account.settings())
            } else {
                ListId::Split(new.id.clone())
            }
        });
        Outcome {
            show,
            ..self.leave_form(back)
        }
    }

    /// Cancel, back, Escape or the backdrop, after the form's discard check.
    fn close(&mut self, account: &Account) -> Outcome {
        match &self.screen {
            Screen::Form(form) if form.has_changes(account.settings()) => {
                self.alert = Some((AlertKind::Discard, 0));
                Outcome::default()
            }
            Screen::Form(form) => self.leave_form(form.back),
            Screen::Template(form, back) => {
                let back = *back;
                let changed = self
                    .library
                    .as_ref()
                    .and_then(|library| library.templates.get(&form.template))
                    .is_some_and(|template| form.has_changes(template));
                if changed {
                    self.alert = Some((AlertKind::Discard, 0));
                    Outcome::default()
                } else {
                    self.leave_form(back)
                }
            }
            Screen::Library(_) => self.leave_form(Back::List),
            Screen::List | Screen::HideEmpty => Outcome {
                show: None,
                close: true,
            },
        }
    }

    /// Leaves the form or template for where it goes back to.
    fn leave_form(&mut self, back: Back) -> Outcome {
        self.alert = None;
        match back {
            Back::List => {
                self.screen = Screen::List;
                Outcome::default()
            }
            Back::Library => {
                self.screen = Screen::List;
                self.open_library();
                Outcome::default()
            }
            Back::Close => {
                self.screen = Screen::List;
                Outcome {
                    show: None,
                    close: true,
                }
            }
        }
    }

    fn alert_action<A: Clone + 'static>(
        &mut self,
        index: usize,
        account: &mut Account,
        commands: &mut Commands<A>,
        wrap: &impl Fn(super::Action) -> A,
    ) -> Outcome {
        let Some((kind, _)) = self.alert.take() else {
            return Outcome::default();
        };
        match (kind, index) {
            (AlertKind::Invalid(_), _) => {
                if let Screen::Form(form) = &mut self.screen {
                    form.query_error = true;
                    if form.name_error.is_none() {
                        form.focus = Some(Field::Query);
                    }
                }
                Outcome::default()
            }
            (AlertKind::Discard, 0) => {
                let back = match &self.screen {
                    Screen::Form(form) => form.back,
                    Screen::Template(_, back) => *back,
                    _ => Back::List,
                };
                self.leave_form(back)
            }
            (AlertKind::DeleteTemplateSplit, 0) => {
                let Screen::Template(form, back) = &self.screen else {
                    return Outcome::default();
                };
                let back = *back;
                let Some(split) = form
                    .split
                    .as_ref()
                    .and_then(|id| account.settings().split(id))
                    .cloned()
                else {
                    return Outcome::default();
                };
                delete(account, commands, wrap, &split);
                let outcome = self.leave_form(back);
                Outcome {
                    show: Some(splits::home(account.settings())),
                    ..outcome
                }
            }
            (AlertKind::TurnOffImportantOther, 0) => {
                let show = Some(toggle_important_other(account, commands, wrap));
                // From the template's settings, it goes back where it came from.
                let back = match &self.screen {
                    Screen::Template(_, back) => self.leave_form(*back),
                    _ => Outcome::default(),
                };
                Outcome { show, ..back }
            }
            (AlertKind::HideAll, 0) => {
                // Hide all when empty: every active split but Important (or
                // Inbox), and Other, that doesn't hide yet; Undo shows those again.
                let settings = account.settings();
                let mut changed: Vec<ListId> = hide_empty_rows(settings)
                    .iter()
                    .map(SplitEntry::list)
                    .filter(|list| !hides_when_empty(settings, list))
                    .collect();
                if !settings.hides_other_when_empty() && !changed.contains(&ListId::Other) {
                    changed.push(ListId::Other);
                }
                account.update_settings(|settings| {
                    for list in &changed {
                        set_hides_when_empty(settings, list, true);
                    }
                });
                let undo = wrap(super::Action::Split(Event::UndoHideAll(changed)));
                commands.toast(
                    Toast::new("Split Inboxes updated.")
                        .divided()
                        .undo(undo)
                        .action(
                            "Settings",
                            None,
                            wrap(super::Action::Open(super::Dialog::HideEmptySplits)),
                        ),
                );
                Outcome::default()
            }
            (AlertKind::Delete, 0) => {
                let Screen::Form(Form {
                    editing: Some(id), ..
                }) = &self.screen
                else {
                    return Outcome::default();
                };
                let id = id.clone();
                let Some(split) = account.settings().split(&id).cloned() else {
                    return Outcome::default();
                };
                delete(account, commands, wrap, &split);
                self.screen = Screen::List;
                Outcome {
                    show: Some(splits::home(account.settings())),
                    close: false,
                }
            }
            _ => Outcome::default(),
        }
    }

    /// What Backspace and Delete do: remove the template's sender row with the keyboard.
    pub fn row_delete(&self) -> Option<Event> {
        match &self.screen {
            Screen::Template(form, _) => form
                .focused_row()
                .map(|row| Event::Template(TemplateEvent::Remove(row))),
            _ => None,
        }
    }

    /// VIP's contact picker, while its screen shows.
    pub(crate) fn sender_picker_mut(
        &mut self,
    ) -> Option<&mut crate::contact_picker::ContactPicker> {
        match &mut self.screen {
            Screen::Template(form, _) => Some(&mut form.picker),
            _ => None,
        }
    }

    /// The keys VIP's contact picker takes while it has the keyboard, over the dialog's.
    pub fn picker_keys(&self) -> Vec<(&'static str, PickerKey, &'static [&'static str])> {
        match &self.screen {
            Screen::Template(form, _) if form.picker.is_focused() => form.picker.keys(),
            _ => Vec::new(),
        }
    }

    /// What Enter does on the current screen: Add Split Inbox on the list,
    /// Save in the form.
    pub fn enter(&self) -> Option<Event> {
        match self.screen {
            Screen::List => Some(Event::AddSplit),
            Screen::Form(_) => Some(Event::Save),
            Screen::Template(..) => Some(Event::TemplateEnter),
            Screen::HideEmpty | Screen::Library(_) => None,
        }
    }

    /// Draws the dialog for the current screen over `area` while `open` (and while it fades out
    /// after), and its alert. Returns what the user did and whether the dialog's close control or
    /// backdrop was clicked; `None` once it has faded out.
    pub fn show(
        &mut self,
        ctx: &Context,
        area: Rect,
        open: bool,
        account: &Account,
        book: Option<&contacts::Index>,
    ) -> Option<(Option<Event>, bool)> {
        let settings = account.settings();
        let library = self.library.clone();
        let template = match (&self.screen, &library) {
            (Screen::Template(form, _), Some(library)) => library.templates.get(&form.template),
            _ => None,
        };
        let (header, back, body_height) = match &self.screen {
            Screen::List => (
                "Split Inbox Settings",
                false,
                list_height(ctx, settings, self.tab),
            ),
            Screen::Form(form) if form.editing.is_some() => {
                ("Edit Split Inbox", true, form_height(form))
            }
            Screen::Form(form) => ("New Split Inbox", true, form_height(form)),
            Screen::HideEmpty => (
                "Hide Empty Split Inboxes",
                false,
                hide_empty_height(settings),
            ),
            // Their own headers: the library has none, a template's is over its hero.
            Screen::Library(_) => ("", true, 0.0),
            Screen::Template(form, _) => (
                "",
                true,
                template.map_or(0.0, |template| form.body_height(ctx, template, settings)),
            ),
        };
        if !matches!(self.screen, Screen::List) {
            self.list_blurred = false;
        }
        let mut dialog = Dialog::new("split-inbox", header, body_height).top();
        if back {
            dialog = dialog.back();
        }
        if matches!(self.screen, Screen::Library(_)) {
            // The library: up to 1080px wide, `margin: 36px 48px`.
            dialog = dialog.fill(1080.0, vec2(48.0, 36.0));
        }
        // An alert's backdrop, a layer above, takes the pointer from the dialog under it.
        let alert_open = self.alert.is_some();
        let response = dialog.show(ctx, area, open, |ui, body| {
            match &mut self.screen {
                Screen::List => list(
                    ui,
                    body,
                    settings,
                    self.tab,
                    &mut self.list_blurred,
                    (&mut self.scroll, &mut self.scrollbar),
                    &mut self.drag,
                ),
                Screen::Form(form) => form.show(ui, body, settings),
                Screen::HideEmpty => hide_empty(ui, body, settings),
                Screen::Library(state) => {
                    let library = library.as_deref()?;
                    split_library::show(ui, body, library, state, &|template| {
                        in_use(settings, template)
                    })
                    .map(Event::Library)
                }
                Screen::Template(form, _) => {
                    let template = template?;
                    // The card around the body: `padding: 20px 32px` and the header's line.
                    let card = Rect::from_min_max(
                        body.min - vec2(32.0, 40.0),
                        body.max + vec2(32.0, 20.0),
                    );
                    let event = form
                        .show(ui, card, body, template, settings, book)
                        .map(Event::Template);
                    paint_template_header(ui, body, template);
                    event
                }
            }
        });
        let Some(mut event) = response.inner else {
            self.reset();
            return None;
        };
        let mut close = response.close && !alert_open;
        if let Some((kind, selected)) = &self.alert {
            // The alert takes the keyboard from the form's fields.
            ctx.memory_mut(|memory| {
                if let Some(focused) = memory.focused() {
                    memory.surrender_focus(focused);
                }
            });
            let text = match kind {
                AlertKind::Invalid(text) => text.clone(),
                AlertKind::Discard => "Your changes will not be saved.".to_owned(),
                AlertKind::TooLong => TOO_LONG.to_owned(),
                AlertKind::Delete => {
                    let name = match &self.screen {
                        Screen::Form(form) => form.editing.as_ref().and_then(|id| settings.split(id)).map_or("", SplitInbox::name),
                        _ => "",
                    };
                    format!("Would you like to delete the split {name}?{PARAGRAPH}Please note: you cannot undo this action.")
                }
                AlertKind::TurnOffImportantOther => format!("Conversations from Important and Other will appear in Inbox instead.{PARAGRAPH}You can always turn Important and Other back on from the Split Inbox Library."),
                AlertKind::NameTaken => "A Split Inbox with this name already exists. To create this one, please rename or remove the existing Split Inbox.".to_owned(),
                AlertKind::DeleteTemplateSplit => {
                    let name = match &self.screen {
                        Screen::Template(form, _) => form.split.as_ref().and_then(|id| settings.split(id)).map_or("", SplitInbox::name),
                        _ => "",
                    };
                    format!("Would you like to delete the split {name}?{PARAGRAPH}You can always turn this split back on from the Split Inbox Library.")
                }
                AlertKind::HideAll => {
                    let home = if settings.unified_inbox { "Inbox" } else { "Important" };
                    // A string with an empty line, not paragraphs.
                    format!("This will hide empty Split Inboxes for all active splits except {home}.\n\nYou can update this anytime from the Split Inbox Settings.")
                }
            };
            let alert = Alert {
                title: kind.title(),
                text: &text,
                actions: kind.actions(),
                selected: *selected,
            };
            let clicked = egui::Area::new(exo_ui::id(ctx, "split-inbox-alert"))
                .order(egui::Order::Tooltip)
                .fixed_pos(area.min)
                .show(ctx, |ui| {
                    // The alert's backdrop, over the dialog's.
                    let backdrop = ui.allocate_rect(area, Sense::click());
                    ui.painter()
                        .rect_filled(area, 0.0, Color32::from_black_alpha(128));
                    let clicked = alert.show(ui, area);
                    match clicked {
                        Some(index) => Some(Event::AlertAction(index)),
                        None if backdrop.clicked() => Some(Event::AlertDismiss),
                        None => None,
                    }
                });
            event = clicked.inner.or(event);
            close = false;
        }
        Some((event, close))
    }
}

/// An enabled split (or Important and Other, or Inbox) uses `template`: its card shows a check.
fn in_use(settings: &Settings, template: &Template) -> bool {
    settings
        .split_entries()
        .iter()
        .filter(|entry| !entry.is_disabled())
        .any(|entry| match entry {
            SplitEntry::System(system) => system.template_id() == template.id,
            SplitEntry::Split(split) => {
                split.template_id.as_deref() == Some(template.id.as_str())
                    || (template.kind.is_some() && split.kind == template.kind)
            }
        })
}

/// A template's header over its hero: its name (`rgba(0, 0, 0, .85)`), or `Important • Other`
/// with its dot at `.4`.
fn paint_template_header(ui: &Ui, body: Rect, template: &Template) {
    const HEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 20.0);
    let tokens = palette(ui.ctx());
    let painter = ui.painter();
    let ink = tokens.ink(0.85, 0.85);
    let at = pos2(body.left(), body.top() - 20.0 + 2.0);
    if template.id == "splitTemplate_importantOther" {
        let x = at.x + HEADER.paint_line(painter, at, "Important ", ink, None);
        let x = x + HEADER.paint_line(painter, pos2(x, at.y), "•", tokens.ink(0.4, 0.4), None);
        HEADER.paint_line(painter, pos2(x, at.y), " Other", ink, None);
    } else {
        HEADER.paint_line(painter, at, &template.name, ink, Some(body.width() - 40.0));
    }
}

/// The account's labels and addresses, for defining splits.
pub(crate) fn definitions(account: &Account) -> (Vec<exo_store::Label>, Vec<String>) {
    (
        account.snapshot().labels.clone(),
        vec![account.email().to_owned()],
    )
}

/// The alert's text for a split that makes the queries too long (title `Sorry!`), its second
/// paragraph indented.
pub(crate) const TOO_LONG: &str = "This makes your Split Inbox queries too long.\n\n       Please remove some splits or reduce the number of contacts in each split.";

/// Adds `split`, saying `Split '{name}'
/// created.` with `Split Inbox Settings`; false, adding nothing, when that would make a split's
/// Gmail search too long.
pub(crate) fn add<A: Clone + 'static>(
    account: &mut Account,
    commands: &mut Commands<A>,
    wrap: &impl Fn(super::Action) -> A,
    split: &SplitInbox,
) -> bool {
    let (_, me) = definitions(account);
    if too_long(
        account.settings(),
        None,
        split,
        &Me {
            addresses: &me,
            name: None,
        },
    ) {
        return false;
    }
    account.update_settings(|settings| {
        settings.add_split(split.clone());
        settings.engage_splits();
    });
    commands.toast(
        Toast::new(format!("Split '{}' created.", split.name())).action(
            "Split Inbox Settings",
            None,
            wrap(super::Action::Split(Event::Edit(split.id.clone()))),
        ),
    );
    true
}

/// Replaces `old` with `new`, saying (when `notify`) `Split '{name}'
/// updated.` (`{name} Split updated.` for a built-in one) with `Split Inbox Settings`; false,
/// changing nothing, when that would make a split's Gmail search too long.
pub(crate) fn replace<A: Clone + 'static>(
    account: &mut Account,
    commands: &mut Commands<A>,
    wrap: &impl Fn(super::Action) -> A,
    old: &SplitInbox,
    new: &SplitInbox,
    notify: bool,
) -> bool {
    let (_, me) = definitions(account);
    if too_long(
        account.settings(),
        Some(old),
        new,
        &Me {
            addresses: &me,
            name: None,
        },
    ) {
        return false;
    }
    account.update_settings(|settings| settings.replace_split(&old.id, new.clone()));
    if !notify {
        return true;
    }
    let text = if new.is_default() {
        format!("{} Split updated.", new.name())
    } else {
        format!("Split '{}' updated.", new.name())
    };
    commands.toast(Toast::new(text).action(
        "Split Inbox Settings",
        None,
        wrap(super::Action::Split(Event::Edit(new.id.clone()))),
    ));
    true
}

/// Whether `new` (replacing `old`, or added) makes a split's Gmail search too long.
fn too_long(settings: &Settings, old: Option<&SplitInbox>, new: &SplitInbox, me: &Me<'_>) -> bool {
    let mut splits: Vec<SplitInbox> = settings
        .splits()
        .filter(|split| old.is_none_or(|old| old.id != split.id))
        .cloned()
        .collect();
    splits.push(new.clone());
    query_too_long(&splits, new, me)
}

/// Deletes `split` and says so; a split from a template offers its
/// settings again, back to the library.
fn delete<A: Clone + 'static>(
    account: &mut Account,
    commands: &mut Commands<A>,
    wrap: &impl Fn(super::Action) -> A,
    split: &SplitInbox,
) {
    account.update_settings(|settings| {
        settings.remove_split(&split.id);
    });
    let mut toast = Toast::new(format!("Split Inbox deleted: {}", split.name()));
    if let Some(template) = &split.template_id {
        toast = toast.action(
            "Split Inbox Settings",
            None,
            wrap(super::Action::Split(Event::OpenTemplate(
                template.clone(),
                Back::Library,
            ))),
        );
    }
    commands.toast(toast);
}

/// Turns `split` on or off with `changes`, shows it once on (or goes
/// home if it was on screen as it turned off), and says so.
pub(crate) fn toggle<A: Clone + 'static>(
    account: &mut Account,
    commands: &mut Commands<A>,
    wrap: &impl Fn(super::Action) -> A,
    split: &SplitInbox,
    changes: SplitChanges,
    current: &ListId,
) -> Option<ListId> {
    let (labels, me) = definitions(account);
    let toggled = split.update(
        &SplitChanges {
            is_disabled: Some(!split.is_disabled),
            ..changes
        },
        Definitions {
            labels: &labels,
            me: &me,
        },
    );
    account.update_settings(|settings| {
        settings.replace_split(&split.id, toggled.clone());
        if !toggled.is_disabled {
            settings.engage_splits();
        }
    });
    let state = if toggled.is_disabled { "off" } else { "on" };
    commands.toast(
        Toast::new(format!("Split Inbox turned {state}: {}", toggled.name())).action(
            "Split Inbox Settings",
            None,
            wrap(super::Action::Split(Event::Edit(toggled.id.clone()))),
        ),
    );
    if toggled.is_disabled {
        (*current == ListId::Split(split.id.clone())).then(|| splits::home(account.settings()))
    } else {
        Some(ListId::Split(toggled.id))
    }
}

/// Turns Important and Other on or off; the inbox shows its new home.
fn toggle_important_other<A: Clone + 'static>(
    account: &mut Account,
    commands: &mut Commands<A>,
    wrap: &impl Fn(super::Action) -> A,
) -> ListId {
    account.update_settings(|settings| {
        settings.toggle_important_other();
        if !settings.unified_inbox {
            settings.engage_splits();
        }
    });
    let state = if account.settings().unified_inbox {
        "off"
    } else {
        "on"
    };
    // The Important • Other template's settings (Split Inbox Settings without the library).
    let settings = Event::OpenTemplate("splitTemplate_importantOther".to_owned(), Back::List);
    commands.toast(
        Toast::new(format!("Split Inboxes turned {state}: Important • Other")).action(
            "Split Inbox Settings",
            None,
            wrap(super::Action::Split(settings)),
        ),
    );
    splits::home(account.settings())
}

impl Form {
    /// What saving the form changes in `old`, given its cleaned name
    /// and definition.
    fn changes(&self, old: &SplitInbox, name: String, query: String) -> SplitChanges {
        SplitChanges {
            name: Some(name),
            query: (clean_whitespace(old.query()) != query).then_some(query),
            leave_threads_in_important_other: Some(self.leave),
            hide_split_when_empty: Some(self.hide),
            ..SplitChanges::default()
        }
    }

    /// Whether the form differs from what's saved (a new one: whether anything's typed).
    fn has_changes(&self, settings: &Settings) -> bool {
        let (name, query) = (clean_whitespace(&self.name), clean_whitespace(&self.query));
        match self.editing.as_ref().and_then(|id| settings.split(id)) {
            None if self.editing.is_some() => false,
            None => !name.is_empty() || !query.is_empty(),
            Some(split) => {
                name != clean_whitespace(split.name())
                    || query != clean_whitespace(split.query())
                    || self.leave != split.leave_threads_in_important_other
                    || self.hide != split.hides_when_empty()
            }
        }
    }

    /// Whether the name or the definition is missing, which disables Save.
    fn incomplete(&self) -> bool {
        clean_whitespace(&self.name).is_empty() || clean_whitespace(&self.query).is_empty()
    }

    /// Draws the form in `body` and returns what was clicked.
    fn show(&mut self, ui: &mut Ui, body: Rect, settings: &Settings) -> Option<Event> {
        let ctx = ui.ctx().clone();
        let tokens = palette(&ctx);
        let left = body.left();
        let mut event = None;
        let input = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
        let opening = std::mem::take(&mut self.opening);
        let mut y = body.top();
        let field = |ui: &mut Ui,
                     y: &mut f32,
                     label: &str,
                     which: Field,
                     focus: &mut Option<Field>,
                     text: &mut String,
                     error: bool,
                     placeholder: &str| {
            // The subheader: `padding-top: 24px`, 12px/20px 600 at `rgba(0, 0, 0, .85)`,
            // `margin-bottom: 12px`.
            BODY_BOLD.paint_line(
                ui.painter(),
                pos2(left, *y + 24.0 + NUDGE),
                label,
                tokens.ink(0.85, 0.85),
                None,
            );
            *y += 24.0 + 20.0 + 12.0;
            let line = Rect::from_min_size(pos2(left, *y + NUDGE), vec2(CONTENT_WIDTH, 20.0));
            let id = exo_ui::id(&ctx, ("split-inbox-field", label));
            let before = text.clone();
            let response = DialogInput::new(input, placeholder)
                .error(error)
                .show(ui, id, text, line);
            if focus.take_if(|wanted| *wanted == which).is_some() {
                // Focus selects the name (`select()`), and puts the caret after the query.
                let count = text.chars().count();
                let from = if which == Field::Name && !opening {
                    0
                } else {
                    count
                };
                exo_ui::focus_field(
                    ui.ctx(),
                    id,
                    egui::text::CCursorRange::two(
                        egui::text::CCursor::new(from),
                        egui::text::CCursor::new(count),
                    ),
                );
            }
            *y += 20.0;
            (response, *text != before)
        };
        let (_, name_changed) = field(
            ui,
            &mut y,
            "Name",
            Field::Name,
            &mut self.focus,
            &mut self.name,
            self.name_error.is_some(),
            "Enter a Split Inbox name",
        );
        if name_changed {
            self.name_error = None;
        }
        if let Some(error) = self.name_error {
            // The error: `margin-top: 16px`, 12px/20px critical.
            BODY.paint_line(
                ui.painter(),
                pos2(left, y + 16.0 + NUDGE),
                error,
                tokens.text_critical_default,
                None,
            );
            y += 36.0;
        }
        let (_, query_changed) = field(
            ui,
            &mut y,
            "Definition",
            Field::Query,
            &mut self.focus,
            &mut self.query,
            self.query_error,
            "Enter a search query that every conversation in this Split Inbox must match",
        );
        if query_changed {
            self.query_error = false;
        }

        // The help: `margin-top: 16px`, its paragraph and 20px-indented list at
        // `rgba(0, 0, 0, .4)`.
        y += 16.0;
        let painter = ui.painter().clone();
        let help = tokens.ink(0.4, 0.4);
        BODY.paint_line(&painter, pos2(left, y + NUDGE), "Write the search query based on the sender, recipients, subject line, or label. For example:", help, None);
        for (i, example) in [
            "from:alice@example.com OR to:feedback@example.com",
            "subject:\"survey results\"",
            "label:\"Team Updates\"",
        ]
        .into_iter()
        .enumerate()
        {
            let line_top = y + 20.0 * (i as f32 + 1.0) + NUDGE;
            // Chromium's `disc` marker at 12px: 4px across, 8px in, mid-line.
            painter.circle_filled(pos2(left + 8.0, line_top + 10.0), 2.0, help);
            BODY.paint_line(&painter, pos2(left + 20.0, line_top), example, help, None);
        }
        y += 80.0;

        // The checkboxes: `margin-top: 16px`, each `margin-top: 8px` and 16px tall.
        y += 16.0;
        let leave_label = if settings.unified_inbox {
            "Also show conversations that match this Split Inbox in Inbox"
        } else {
            "Also show conversations that match this Split Inbox in Important or Other"
        };
        for (label, checked, toggle) in [
            (leave_label, self.leave, Event::ToggleLeave),
            (
                "Hide this Split Inbox when empty",
                self.hide,
                Event::ToggleHide,
            ),
        ] {
            y += 8.0;
            let mark = pos2(left, y + 1.0);
            let label_x = left + 14.0 + 12.0;
            let width = SMALL.width(&ctx, label);
            let hit = Rect::from_min_max(mark, pos2(label_x + width, y + 17.0));
            if ui
                .interact(
                    hit,
                    exo_ui::id(ui.ctx(), ("split-inbox-check", label)),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                event = Some(toggle);
            }
            paint_checkbox(&painter, mark, checked, tokens);
            // The checkbox's label: `rgba(0, 0, 0, .55)`.
            SMALL.paint_line(
                &painter,
                pos2(label_x, y + 1.0),
                label,
                tokens.ink(0.55, 0.55),
                None,
            );
            y += 16.0;
        }

        // The buttons: `margin-top: 44px`, right-aligned, nudged down 3px; the tertiary one at the
        // left edge, 12px out. A split that's on turns off rather than being deleted, and one
        // that's off turns on rather than being saved.
        let top = y + 44.0 + NUDGE;
        let disabled = self
            .editing
            .as_ref()
            .and_then(|id| settings.split(id))
            .map(|split| split.is_disabled);
        let (primary, on_primary) = match disabled {
            None => ("Create", Event::Save),
            Some(true) => ("Turn On Split Inbox", Event::Toggle),
            Some(false) => ("Save", Event::Save),
        };
        let primary = DialogAction::new(primary, ActionKind::Primary).enabled(!self.incomplete());
        let cancel = DialogAction::new("Cancel", ActionKind::Plain);
        let mut right = body.right() + 13.0;
        for (id, action, clicked) in [
            ("primary", primary, on_primary),
            ("cancel", cancel, Event::Close),
        ] {
            let width = action.width(&ctx);
            let rect =
                Rect::from_min_size(pos2(right - width, top), vec2(width, DIALOG_ACTION_HEIGHT));
            if action
                .show(ui, exo_ui::id(ui.ctx(), ("split-inbox-action", id)), rect)
                .clicked()
            {
                event = Some(clicked);
            }
            right = rect.left() - 8.0;
        }
        if let Some(disabled) = disabled {
            let (label, clicked) = if disabled {
                ("Delete Split Inbox", Event::Delete)
            } else {
                ("Turn Off Split Inbox", Event::Toggle)
            };
            let tertiary = DialogAction::new(label, ActionKind::Plain);
            let rect = Rect::from_min_size(
                pos2(left - 12.0, top),
                vec2(tertiary.width(&ctx), DIALOG_ACTION_HEIGHT),
            );
            if tertiary
                .show(
                    ui,
                    exo_ui::id(ui.ctx(), ("split-inbox-action", "tertiary")),
                    rect,
                )
                .clicked()
            {
                event = Some(clicked);
            }
        }
        event
    }
}

/// The form's body height: its fields, help, checkboxes and buttons.
fn form_height(form: &Form) -> f32 {
    let error = if form.name_error.is_some() { 36.0 } else { 0.0 };
    // Two fields (56 + 20), the help (16 + 80), the checkboxes (16 + 48), the buttons (44 + 28 + 7).
    2.0 * 76.0 + error + 96.0 + 64.0 + 79.0
}

/// The Active tab's rows: the splits shown as tabs, with Important and Other (or Inbox), in
/// their order.
fn active_rows(settings: &Settings) -> Vec<SplitEntry<'_>> {
    settings
        .split_entries()
        .into_iter()
        .filter(|entry| !entry.is_disabled())
        .collect()
}

/// A row of the list: an entry, or the Inactive tab's `Important • Other` while they're off.
#[derive(Clone, Copy)]
enum Row<'a> {
    Entry(SplitEntry<'a>),
    ImportantOtherOff,
}

impl Row<'_> {
    fn list(&self) -> ListId {
        match self {
            Self::Entry(entry) => entry.list(),
            Self::ImportantOtherOff => ListId::Other,
        }
    }

    fn id(&self) -> String {
        match self {
            Self::Entry(entry) => entry.external_id().to_owned(),
            Self::ImportantOtherOff => "important-other-off".to_owned(),
        }
    }
}

/// The rows of `tab`: Active's entries, or the splits that are off, after a row for Important and
/// Other while they're off.
fn rows(settings: &Settings, tab: ListTab) -> Vec<Row<'_>> {
    match tab {
        ListTab::Active => active_rows(settings).into_iter().map(Row::Entry).collect(),
        ListTab::Inactive => settings
            .unified_inbox
            .then_some(Row::ImportantOtherOff)
            .into_iter()
            .chain(
                settings
                    .splits()
                    .filter(|split| split.is_disabled)
                    .map(|split| Row::Entry(SplitEntry::Split(split))),
            )
            .collect(),
    }
}

/// The intro and Add Split Inbox (20px under the header), the
/// tabs and their 20px gap: where the scrolling part starts, from the body's top.
fn list_head(ctx: &Context) -> f32 {
    20.0 + intro_height(ctx) + 20.0 + TABS_HEIGHT
}

/// The tabs: a 4px-down 16px label, its 7px padding and 1px underline (Classic), and the
/// container's `padding-bottom: 3px` and 1px border.
const TABS_HEIGHT: f32 = 28.0;
/// Around the rows: the scrolling part's `padding: 12px 0`, and on the Active tab the sortable
/// list's `padding: 4px 0 8px`.
fn rows_padding(tab: ListTab) -> [f32; 2] {
    match tab {
        ListTab::Active => [16.0, 20.0],
        ListTab::Inactive => [12.0, 12.0],
    }
}

/// The scrolling part's natural height.
fn content_height(settings: &Settings, tab: ListTab) -> f32 {
    let [top, bottom] = rows_padding(tab);
    let rows = rows(settings, tab).len();
    // The empty Inactive tab's notice: `margin: 4px 32px 8px` around a line.
    let rows = if rows == 0 {
        32.0
    } else {
        ROW_HEIGHT * rows as f32
    };
    top + rows + bottom
}

/// The list's body height.
fn list_height(ctx: &Context, settings: &Settings, tab: ListTab) -> f32 {
    list_head(ctx) + content_height(settings, tab)
}

const INTRO: &str = "Customize your splits, create new ones, or explore the library to find your ideal workflow and focus on what matters most.";
/// `Add Split Inbox`: `width: 116px` inside `padding: 8px 12px 4px`.
const ADD_WIDTH: f32 = 116.0 + 24.0;

fn intro_lines(ctx: &Context) -> Vec<String> {
    // The header row is a flex row: the text, a 20px gap, the button 13px past the edge.
    BODY.wrap(ctx, INTRO, CONTENT_WIDTH - 20.0 - ADD_WIDTH + 13.0)
}

/// The header row: the intro, or the 32px button when that's taller.
fn intro_height(ctx: &Context) -> f32 {
    (intro_lines(ctx).len() as f32 * 20.0).max(DIALOG_ACTION_HEIGHT)
}

/// Split Inbox Settings: the intro and Add Split Inbox, the Active and Inactive tabs with the Hide
/// Empty link, and the tab's rows.
fn list(
    ui: &mut Ui,
    body: Rect,
    settings: &Settings,
    tab: ListTab,
    blurred: &mut bool,
    (scroll, scrollbar): (&mut f32, &mut exo_ui::Scrollbar),
    drag: &mut Option<(usize, f32)>,
) -> Option<Event> {
    let ctx = ui.ctx().clone();
    *blurred |= ui.input(|input| input.pointer.any_pressed());
    let tokens = palette(&ctx);
    let painter = ui.painter().clone();
    let (left, right) = (body.left(), body.right());
    let mut event = None;

    let row_top = body.top() + 20.0;
    for (i, line) in intro_lines(&ctx).iter().enumerate() {
        BODY.paint_line(
            &painter,
            pos2(left, row_top + NUDGE + 20.0 * i as f32),
            line,
            tokens.text_base_default,
            None,
        );
    }
    let add = Rect::from_min_size(
        pos2(right + 13.0 - ADD_WIDTH, row_top + 4.0),
        vec2(ADD_WIDTH, DIALOG_ACTION_HEIGHT),
    );
    // It has the keyboard when the list shows; Enter anywhere runs it.
    if DialogAction::new("Add Split Inbox", ActionKind::Primary)
        .focused(!*blurred)
        .show(ui, exo_ui::id(ui.ctx(), "split-inbox-add"), add)
        .clicked()
    {
        event = Some(Event::AddSplit);
    }

    // The tabs, 4px down in their row: labels at `rgba(0, 0, 0, .4)`, `.8` focused. Only Carbon
    // underlines the focused tab, over the row's border.
    let tabs_top = row_top + intro_height(&ctx) + 20.0;
    divider(
        &painter,
        left,
        right,
        tabs_top + TABS_HEIGHT - 1.0,
        tokens.border_base_default,
    );
    let underline = exo_ui::app_theme(&ctx) == exo_ui::Theme::Carbon;
    let mut x = left;
    for (label, which) in [("Active", ListTab::Active), ("Inactive", ListTab::Inactive)] {
        let width = SMALL.width(&ctx, label);
        let rect = Rect::from_min_size(pos2(x, tabs_top + 4.0), vec2(width, 24.0));
        let focused = which == tab;
        SMALL.paint_line(
            &painter,
            pos2(x, rect.top() + 1.0),
            label,
            if focused {
                tokens.ink(0.8, 0.8)
            } else {
                tokens.ink(0.4, 0.4)
            },
            None,
        );
        if focused && underline {
            painter.rect_filled(
                Rect::from_min_max(pos2(rect.left(), rect.bottom() - 1.0), rect.max),
                0.0,
                tokens.ink(0.8, 0.8),
            );
        }
        if ui
            .interact(
                rect,
                exo_ui::id(ui.ctx(), ("split-inbox-tab", label)),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            event = Some(Event::SelectTab(which));
        }
        x = rect.right() + 24.0;
    }
    // `Hide Empty Split Inboxes`, right-aligned and centered on the tabs' line, `top: 1px`.
    let label = "Hide Empty Split Inboxes";
    let width = SMALL.width(&ctx, label);
    let link = Rect::from_min_size(pos2(right - width, tabs_top + 5.0), vec2(width, 16.0));
    SMALL.paint_line(&painter, link.min, label, tokens.link_text_primary, None);
    if ui
        .interact(
            link,
            exo_ui::id(ui.ctx(), "split-inbox-hide-empty"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        event = Some(Event::ShowHideEmpty);
    }

    // The scrolling part, full-bleed, down to the body's bottom.
    let rows = rows(settings, tab);
    if let Some((_, offset)) = drag {
        *offset += ui.input(|input| input.pointer.delta().y);
    }
    let area_top = tabs_top + TABS_HEIGHT;
    let natural = content_height(settings, tab);
    let area = Rect::from_min_max(
        pos2(left - 32.0, area_top),
        pos2(right + 32.0, (area_top + natural).min(body.bottom())),
    );
    if ui.rect_contains_pointer(area) {
        *scroll = (*scroll - ui.input(|input| input.smooth_scroll_delta.y))
            .clamp(0.0, (natural - area.height()).max(0.0));
    }
    let mut area_ui = ui.new_child(egui::UiBuilder::new().max_rect(area));
    area_ui.set_clip_rect(area.intersect(ui.clip_rect()));
    let area_painter = area_ui.painter().clone();
    let rows_top = area_top + rows_padding(tab)[0] - *scroll;

    if rows.is_empty() {
        let notice = match tab {
            ListTab::Active => "When you add a Split, it will appear here.",
            ListTab::Inactive => "When you turn off a Split, it will appear here.",
        };
        // The notice: `rgba(0, 0, 0, .4)`.
        BODY.paint_line(
            &area_painter,
            pos2(left, rows_top + 4.0 + NUDGE),
            notice,
            tokens.ink(0.4, 0.4),
            None,
        );
    }
    let sortable = tab == ListTab::Active;
    let last = rows.len().max(1) - 1;
    let dragging = drag.filter(|_| sortable).map(|(from, offset)| {
        (
            from,
            offset,
            ((from as f32 * ROW_HEIGHT + offset) / ROW_HEIGHT)
                .round()
                .clamp(0.0, last as f32) as usize,
        )
    });
    for (index, row) in rows.iter().enumerate() {
        // Others make way for the dragged row (`animation: 150`).
        let slot = match dragging {
            Some((from, _, to)) if index != from && from < index && index <= to => index - 1,
            Some((from, _, to)) if index != from && to <= index && index < from => index + 1,
            _ => index,
        };
        let shift = motion::transition(
            &ctx,
            exo_ui::id(&ctx, ("split-row-shift", row.id())),
            slot != index,
            0.15,
            motion::CubicBezier::EASE,
        ) * (slot as f32 - index as f32);
        let top = rows_top + ROW_HEIGHT * (index as f32 + shift);
        if dragging.is_some_and(|(from, ..)| from == index) {
            continue;
        }
        if let Some(clicked) = split_row(
            &mut area_ui,
            row,
            index,
            top,
            left,
            right,
            sortable,
            dragging.is_some(),
            drag,
        ) {
            event = Some(clicked);
        }
    }
    if let Some((from, offset, to)) = dragging {
        let top = rows_top + ROW_HEIGHT * from as f32 + offset;
        // The dragged copy: `#fff` with a 3px bar on its left.
        let rect = Rect::from_min_size(pos2(left - 32.0, top), vec2(DIALOG_WIDTH, ROW_HEIGHT));
        exo_ui::BoxShadow::new([1.0, 1.0], 4.0, 0.0, Color32::from_black_alpha(51)).paint(
            &area_painter,
            rect,
            CornerRadius::same(4),
        );
        area_painter.rect_filled(rect, CornerRadius::same(4), Color32::WHITE);
        area_painter.rect_filled(
            Rect::from_min_size(rect.min, vec2(3.0, ROW_HEIGHT)),
            CornerRadius {
                nw: 4,
                sw: 4,
                ne: 0,
                se: 0,
            },
            tokens.border_base_active,
        );
        paint_row_text(
            &area_painter,
            &rows[from],
            top,
            left,
            CONTENT_WIDTH - 49.0,
            tokens,
        );
        if ui.input(|input| input.pointer.any_released()) {
            *drag = None;
            if to != from {
                event = Some(Event::Reorder(from, to));
            }
        }
    }
    // A list taller than the dialog scrolls, with a scrollbar.
    scrollbar.show(
        &area_ui,
        exo_ui::id(area_ui.ctx(), "split-inbox-scrollbar"),
        area,
        *scroll,
        natural,
    );
    event
}

/// A 1px rule across the content at `y`.
fn divider(painter: &egui::Painter, left: f32, right: f32, y: f32, color: Color32) {
    painter.rect_filled(
        Rect::from_min_max(pos2(left, y), pos2(right, y + 1.0)),
        0.0,
        color,
    );
}

/// A row's subheader.
fn subheader(row: &Row<'_>) -> String {
    let split = match row {
        Row::ImportantOtherOff | Row::Entry(SplitEntry::System(SystemSplit::Other)) => {
            return "Split conversations from marketing, social networks, and automatic updates"
                .into();
        }
        Row::Entry(SplitEntry::System(SystemSplit::Important)) => {
            return "Direct, personal, and high priority messages".into();
        }
        Row::Entry(SplitEntry::System(SystemSplit::Inbox)) => {
            return "Your latest conversations".into();
        }
        Row::Entry(SplitEntry::Split(split)) => split,
    };
    let query = split.query();
    let items = || {
        query
            .split(" OR ")
            .map(|item| item.trim_start_matches("from:"))
            .collect::<Vec<_>>()
    };
    match split.kind.as_deref() {
        Some("team") => {
            let stripped = query
                .trim_end_matches(" to:me")
                .trim_start_matches('(')
                .trim_end_matches(')');
            let items: Vec<&str> = stripped
                .split(" OR ")
                .map(|item| item.trim_start_matches("from:"))
                .filter(|item| !item.is_empty())
                .collect();
            match items.len() {
                0 => "Be responsive to your colleagues".into(),
                1 => query.to_owned(),
                _ => format!("{} to:me", items.join(", ")),
            }
        }
        Some("vip") if query.is_empty() => "Be responsive to specific senders".into(),
        Some("news") if query.is_empty() => "News, articles, and newsletters".into(),
        Some("vip" | "news") => items().join(", "),
        Some("calendar") => "Never miss an invitation again".into(),
        Some("shared") => "View all shared conversations".into(),
        _ if split.template_id.as_deref() == Some("splitTemplate_reminders") => {
            "Reminders that are ready to follow up".into()
        }
        _ => {
            let words = query
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|word| word.eq_ignore_ascii_case("and") || word.eq_ignore_ascii_case("or"));
            if words {
                format!("({query})")
            } else {
                query.to_owned()
            }
        }
    }
}

/// Paints a row's title and subheader, `width` wide.
fn paint_row_text(
    painter: &egui::Painter,
    row: &Row<'_>,
    top: f32,
    left: f32,
    width: f32,
    tokens: &exo_ui::Palette,
) {
    let name = match row {
        Row::Entry(entry) => entry.name(),
        Row::ImportantOtherOff => "Important • Other",
    };
    BODY_BOLD.paint_line(
        painter,
        pos2(left, top + 4.0 + NUDGE),
        name,
        tokens.text_base_default,
        Some(width),
    );
    // An empty subheader is a non-breaking space, so the row keeps its height.
    BODY.paint_line(
        painter,
        pos2(left, top + 24.0 + NUDGE),
        &subheader(row),
        tokens.text_base_subdued,
        Some(width),
    );
}

/// A row: filled while hovered, its title and
/// subheader, and while hovered its drag handle (Active only) and gear. A click anywhere opens
/// its settings.
#[expect(
    clippy::too_many_arguments,
    reason = "one row of the list, drawn from the list's layout"
)]
fn split_row(
    ui: &mut Ui,
    row: &Row<'_>,
    index: usize,
    top: f32,
    left: f32,
    right: f32,
    sortable: bool,
    dragging: bool,
    drag: &mut Option<(usize, f32)>,
) -> Option<Event> {
    let ctx = ui.ctx().clone();
    let tokens = palette(&ctx);
    let painter = ui.painter().clone();
    let id = row.id();
    let rect = Rect::from_min_size(pos2(left - 32.0, top), vec2(DIALOG_WIDTH, ROW_HEIGHT));
    let gear = Rect::from_min_size(
        pos2(right + 7.0 - GEAR_SIZE, top + 13.0),
        vec2(GEAR_SIZE, GEAR_SIZE),
    );
    let handle = Rect::from_min_size(
        pos2(gear.left() - HANDLE_SIZE[0], top + 15.0),
        HANDLE_SIZE.into(),
    );
    let handle_response = sortable.then(|| {
        ui.interact(
            handle,
            exo_ui::id(ui.ctx(), ("split-row-handle", &id)),
            Sense::drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Grab)
    });
    let gear_response = ui
        .interact(
            gear,
            exo_ui::id(ui.ctx(), ("split-row-gear", &id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let response = ui
        .interact(
            rect,
            exo_ui::id(ui.ctx(), ("split-row", &id)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let hovered = !dragging && ui.rect_contains_pointer(rect);
    if hovered {
        // Hovered: `#f2f4fa`, `#dfe3f1` in Carbon.
        let fill = match exo_ui::app_theme(&ctx) {
            exo_ui::Theme::Snow => tokens.fill_active_default,
            exo_ui::Theme::Carbon => Color32::from_rgb(0xdf, 0xe3, 0xf1),
        };
        painter.rect_filled(rect, 0.0, fill);
    }
    let actions_width = if sortable {
        GEAR_SIZE + HANDLE_SIZE[0] - 7.0
    } else {
        GEAR_SIZE - 7.0
    };
    paint_row_text(
        &painter,
        row,
        top,
        left,
        CONTENT_WIDTH - actions_width,
        tokens,
    );

    // The handle and gear fade in while the row is hovered (`transition: opacity 150ms ease`).
    let shown = motion::transition(
        &ctx,
        exo_ui::id(&ctx, ("split-row-actions", &id)),
        hovered,
        0.15,
        motion::CubicBezier::EASE,
    );
    if shown > 0.0 {
        let actions = handle_response
            .as_ref()
            .map(|response| (handle, "legacy/DEPRECATED_dragHandle", response))
            .into_iter()
            .chain([(gear, "legacy/DEPRECATED_settings", &gear_response)]);
        for (rect, icon, response) in actions {
            let color = if response.hovered() {
                tokens.icon_base_hover
            } else {
                tokens.icon_base_default
            };
            paint_icon(
                ui,
                icon,
                exo_ui::icon_rect(&ctx, icon, rect),
                color.gamma_multiply(shown),
            );
        }
    }
    if let Some(handle) = &handle_response
        && handle.drag_started()
    {
        *drag = Some((index, handle.drag_delta().y));
        return None;
    }
    let on_handle = handle_response.is_some_and(|handle| handle.hovered());
    (gear_response.clicked() || (response.clicked() && !on_handle))
        .then(|| Event::OpenRow(row.list()))
}

/// Hide Empty's rows: the enabled splits and Other, but not Important or Inbox.
fn hide_empty_rows(settings: &Settings) -> Vec<SplitEntry<'_>> {
    active_rows(settings)
        .into_iter()
        .filter(|entry| {
            !matches!(
                entry,
                SplitEntry::System(SystemSplit::Important | SystemSplit::Inbox)
            )
        })
        .collect()
}

/// Makes the row for `list` hide when empty, or not.
fn set_hides_when_empty(settings: &mut Settings, list: &ListId, hide: bool) {
    match list {
        ListId::Split(id) => {
            settings.update_split(id, |split| split.hide_split_when_empty = Some(hide))
        }
        _ => settings.set_hide_other_when_empty(hide),
    }
}

/// Whether the row for `list` hides when empty.
fn hides_when_empty(settings: &Settings, list: &ListId) -> bool {
    match list {
        ListId::Split(id) => settings.split(id).is_some_and(SplitInbox::hides_when_empty),
        _ => settings.hides_other_when_empty(),
    }
}

fn hide_empty_height(settings: &Settings) -> f32 {
    let rows = hide_empty_rows(settings).len();
    if rows == 0 {
        // The empty notice: `padding: 8px 0` around a 20px line.
        20.0 + 36.0
    } else {
        // The subheader row (20 + 8 below), the rows, the list's -20px bottom margin.
        20.0 + 28.0 + HIDE_ROW_HEIGHT * rows as f32 - 20.0
    }
}

/// Hide Empty Split Inboxes: a checkbox row per enabled split and Other, applied as it's clicked.
fn hide_empty(ui: &mut Ui, body: Rect, settings: &Settings) -> Option<Event> {
    let ctx = ui.ctx().clone();
    let tokens = palette(&ctx);
    let painter = ui.painter().clone();
    let (left, right) = (body.left(), body.right());
    let top = body.top() + 20.0;
    let rows = hide_empty_rows(settings);
    if rows.is_empty() {
        // The empty notice: `rgba(0, 0, 0, .55)`.
        BODY.paint_line(
            &painter,
            pos2(left, top + 8.0 + NUDGE),
            "No active Split Inboxes to hide when empty.",
            tokens.ink(0.55, 0.55),
            None,
        );
        return None;
    }
    let mut event = None;
    BODY.paint_line(
        &painter,
        pos2(left, top + NUDGE),
        "Choose which Split Inboxes are hidden when they have no messages.",
        tokens.text_base_default,
        None,
    );
    let all = rows
        .iter()
        .all(|entry| hides_when_empty(settings, &entry.list()));
    let select = if all { "Unselect all" } else { "Select all" };
    let width = BODY.width(&ctx, select);
    let select_rect = Rect::from_min_size(pos2(right - width, top + NUDGE), vec2(width, 20.0));
    BODY.paint_line(
        &painter,
        select_rect.min,
        select,
        tokens.link_text_primary,
        None,
    );
    if ui
        .interact(
            select_rect,
            exo_ui::id(ui.ctx(), "hide-empty-select-all"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        event = Some(Event::HideAll);
    }
    for (i, entry) in rows.iter().enumerate() {
        let row_top = top + 28.0 + HIDE_ROW_HEIGHT * i as f32;
        // Full-bleed rows, filled while hovered.
        let row = Rect::from_min_size(
            pos2(left - 32.0, row_top),
            vec2(DIALOG_WIDTH, HIDE_ROW_HEIGHT),
        );
        let response = ui
            .interact(
                row,
                exo_ui::id(ui.ctx(), ("hide-empty-row", entry.external_id())),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.hovered() {
            // Hovered: `hsl(220, 45%, 97%)`, `#e8e9f0` in Carbon.
            let fill = match exo_ui::app_theme(&ctx) {
                exo_ui::Theme::Snow => tokens.fill_hover_default,
                exo_ui::Theme::Carbon => Color32::from_rgb(0xe8, 0xe9, 0xf0),
            };
            painter.rect_filled(row, 0.0, fill);
        }
        if response.clicked() {
            event = Some(Event::ToggleHideEmpty(entry.list()));
        }
        SMALL.paint_line(
            &painter,
            pos2(left, row_top + 7.0 + 6.0),
            entry.name(),
            tokens.text_base_default,
            None,
        );
        paint_checkbox(
            &painter,
            pos2(right + 7.0 - CHECKBOX_SIZE - 6.0, row_top + 13.0),
            hides_when_empty(settings, &entry.list()),
            tokens,
        );
    }
    event
}
