//! Personal snippets: the Snippets folder (`G` `;`), their commands, and the ⌘; picker that uses
//! one. They're kept in the account's settings; compose edits and applies them
//! ([`crate::compose::snippets`]). Team snippets are left out.

mod analytics;
mod folder;
mod pane;
mod view;

use std::collections::BTreeMap;

use exo_commands::{Command, Commands, Hint, Region, SubPalette};
use exo_store::Snippet;

use crate::{
    commands,
    compose::{self, Field},
};

use folder::Folder;
pub(crate) use pane::show as show_pane;

/// The snippets' state in the app: the folder, and where ⌘; was pressed while its palette has
/// the keyboard.
#[derive(Default)]
pub(crate) struct Snippets {
    pub folder: Folder,
    origin: Option<Origin>,
    replies: analytics::Replies,
    preview: view::Preview,
    /// Where leaving a snippet with unsaved changes goes once `Save Snippet?` lets it.
    pub leaving: Option<compose::Leaving>,
}

impl Snippets {
    /// The read-only view's body is laid out and painted.
    pub(crate) fn is_idle(&self) -> bool {
        self.preview.is_idle()
    }
}

/// What the snippet commands and the folder ask for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// `G` `;`: the Snippets folder.
    Go,
    /// `Create Snippet`: its prompt (`Private Snippet`).
    Create,
    /// A new private snippet named this (`Private Snippet`, `New Private Snippet`, `C` in the
    /// folder, `Create Snippet “{query}”`).
    New(String),
    /// `Create Snippet from Draft`.
    FromDraft,
    /// `Create Snippet from Message`.
    FromMessage,
    /// ⌘; (`Use Snippet`): the picker.
    Pick,
    /// A snippet picked there.
    Use(String),
    /// `;` outside the body (`Use Snippet Inline`): `;` typed in the body to open its picker.
    Inline,
    /// J / K in the folder, or in the view, where the next snippet opens.
    Move(isize),
    /// A column's header: the section sorts by it.
    Sort(Column),
    /// Enter or a click on a row: open it read-only (the cursor's, without one).
    Open(Option<String>),
    /// `Edit Snippet` or the view's `Edit`: the editor, on this snippet, the one viewed, or the
    /// cursor's.
    Edit(Option<String>),
    /// Escape or the back gutter in the view: the folder again.
    Back,
    /// `Discard Snippet`: the one viewed, or the cursor's.
    Discard,
    /// A file's card in the view: the file opens, previewed unless asked not to.
    OpenFile { uuid: String, preview: bool },
    /// The view's `Download all`: its files zipped.
    DownloadFiles,
}

/// Where ⌘; was pressed, which decides where the snippet goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    /// A draft's form: into that draft.
    Draft,
    /// A thread: a Reply All with it.
    Thread,
    /// Anywhere else: a new message with it.
    Elsewhere,
}

/// The snippets as the folder orders them: by `sort`'s column and its tie-breaks, or by when each
/// was last used then last saved, newest first.
pub(crate) fn ordered<'a>(
    snippets: &'a [Snippet],
    last_used: &BTreeMap<String, String>,
    sort: Option<Sort>,
) -> Vec<&'a Snippet> {
    let (keys, ascending) = sort.map_or((&[Key::Mru, Key::Chronological][..], false), |sort| {
        (sort.column.keys(), sort.ascending)
    });
    let mut ordered: Vec<&Snippet> = snippets.iter().collect();
    ordered.sort_by(|a, b| {
        let order = keys
            .iter()
            .map(|key| key.value(a, last_used).total_cmp(&key.value(b, last_used)))
            .find(|order| order.is_ne())
            .unwrap_or(std::cmp::Ordering::Equal);
        if ascending { order } else { order.reverse() }
    });
    ordered
}

/// A column the folder sorts by, clicking its header.
/// Author sorts only the team section's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Column {
    Sends,
    Opened,
    Replied,
}

/// The section's order: a column, either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sort {
    pub column: Column,
    pub ascending: bool,
}

/// What a snippet sorts by.
#[derive(Clone, Copy)]
enum Key {
    Sends,
    /// Opens per send: none, without read statuses.
    Opens,
    /// Replies per send.
    Replies,
    /// When it was last used.
    Mru,
    /// When it was last saved.
    Chronological,
}

impl Column {
    /// The column's key, then its tie-breaks.
    fn keys(self) -> &'static [Key] {
        match self {
            Self::Sends => &[
                Key::Sends,
                Key::Opens,
                Key::Replies,
                Key::Mru,
                Key::Chronological,
            ],
            Self::Opened => &[
                Key::Opens,
                Key::Replies,
                Key::Sends,
                Key::Mru,
                Key::Chronological,
            ],
            Self::Replied => &[Key::Replies, Key::Sends, Key::Mru, Key::Chronological],
        }
    }
}

impl Key {
    fn value(self, snippet: &Snippet, last_used: &BTreeMap<String, String>) -> f64 {
        let millis = |at: Option<&String>| {
            at.and_then(|at| at.parse::<jiff::Timestamp>().ok())
                .map_or(0.0, |at| at.as_millisecond() as f64)
        };
        let analytics = &snippet.analytics;
        match self {
            Self::Sends => f64::from(analytics.sends),
            Self::Opens => 0.0,
            Self::Replies => f64::from(analytics.replies) / f64::from(analytics.sends.max(1)),
            Self::Mru => millis(last_used.get(&snippet.id)),
            Self::Chronological => millis(Some(&snippet.date)),
        }
    }
}

/// Registers the snippet commands.
pub(crate) fn register(commands: &mut Commands<commands::Action>, cx: &Context) {
    let act = commands::Action::Snippets;
    const COMPOSE: &[Region] = &[Region::ComposeForm, Region::RecipientList];
    // `Use Snippet`'s score: 93 with a draft focused (+4 in compose), 4.8 in the folder, 1.55
    // elsewhere.
    let score = if cx.draft_focused {
        97.0
    } else if cx.folder {
        4.8
    } else {
        1.55
    };
    commands.add(
        Command::new("USE_SNIPPET", "Use Snippet", act(Action::Pick))
            .key("mod+;")
            .key_code(["⌘", ";"])
            .aliases([
                "Insert Snippet",
                "Use Template",
                "Insert Template",
                "Insert Canned Response",
            ])
            .default_score(score)
            .icon("data/snippets"),
    );
    if cx.draft_focused && !cx.typing && !cx.snippet {
        commands.add(
            Command::new(
                "USE_SNIPPET_INLINE",
                "Use Snippet Inline",
                act(Action::Inline),
            )
            .key(";")
            .key_code([";"])
            .aliases(["Insert Snippet Inline", "Use Template Inline"])
            .icon("data/snippets")
            .regions(COMPOSE),
        );
    }
    commands.add(
        Command::new("CREATE_SNIPPET", "Create Snippet", act(Action::Create))
            .aliases([
                "Create Template",
                "Create Canned Response",
                "New Snippet",
                "New Template",
                "New Canned Response",
            ])
            .icon("data/snippets"),
    );
    if cx.draft_focused && !cx.snippet {
        commands.add(
            Command::new(
                "CREATE_SNIPPET_FROM_DRAFT",
                "Create Snippet from Draft",
                act(Action::FromDraft),
            )
            .aliases([
                "Create Template from Draft",
                "Create Canned Response from Draft",
                "Save Snippet from Draft",
                "Save Template from Draft",
                "Save Canned Response from Draft",
            ])
            .icon("data/snippets")
            .regions(COMPOSE),
        );
    }
    if cx.message_focused {
        commands.add(
            Command::new(
                "CREATE_SNIPPET_FROM_MESSAGE",
                "Create Snippet from Message",
                act(Action::FromMessage),
            )
            .aliases([
                "Create Template from Message",
                "Create Canned Response from Message",
            ])
            .icon("data/snippets")
            .regions(&[Region::ThreadMessages]),
        );
    }
    commands.add(
        Command::shortcut("GO_TO_SNIPPETS", act(Action::Go))
            .key("g ;")
            .regions(exo_commands::LIST_AND_THREAD),
    );
    if cx.viewing {
        folder::register_view(commands);
    } else if cx.folder && !cx.editing {
        folder::register(commands, cx.snippets);
    }
}

/// What registering the commands needs to know.
pub(crate) struct Context {
    /// A draft has the keyboard.
    pub draft_focused: bool,
    /// …in a field that takes text (the body, the subject, a recipient's).
    pub typing: bool,
    /// …and it's a snippet being edited.
    pub snippet: bool,
    /// A message of an open thread has the keyboard.
    pub message_focused: bool,
    /// The Snippets folder is the list.
    pub folder: bool,
    /// A snippet is open in the pane.
    pub editing: bool,
    /// A snippet is open read-only.
    pub viewing: bool,
    /// How many snippets there are.
    pub snippets: usize,
}

impl crate::Mail {
    /// Carries out a snippet action.
    pub(crate) fn snippets_run(&mut self, action: Action, now: f64) {
        match action {
            Action::Go => self.go_to_snippets(),
            Action::Create => {
                // `Create Snippet`'s prompt, with no `Team Snippet` row: Exo has no team snippets.
                let row = Command::new(
                    "PRIVATE_SNIPPET",
                    "Private Snippet",
                    commands::Action::Snippets(Action::New(String::new())),
                )
                .default_score(10.0);
                self.commands.open_sub_palette(SubPalette::new(
                    "CreateSnippet",
                    "Create Snippet",
                    "data/snippets",
                    vec![row],
                ));
            }
            Action::New(name) => self.create_snippet(name),
            Action::FromDraft => self.snippet_from_draft(),
            Action::FromMessage => self.snippet_from_message(),
            Action::Pick => self.pick_snippet(),
            Action::Use(id) => self.use_snippet(&id, now),
            Action::Inline => self.snippet_inline(now),
            Action::Move(delta) if self.snippets.folder.viewing().is_some() => {
                self.snippet_step(delta)
            }
            Action::Move(delta) => {
                let ids = self.snippet_ids();
                self.snippets.folder.move_cursor(&ids, delta);
            }
            Action::Open(id) => {
                if let Some(id) = id.or_else(|| self.snippets.folder.cursor().map(str::to_owned)) {
                    self.open_snippet_view(&id);
                }
            }
            Action::Back => self.close_snippet_view(),
            Action::Sort(column) => self.snippets.folder.sort_by(column),
            Action::Edit(id) => {
                let folder = &self.snippets.folder;
                if let Some(id) =
                    id.or_else(|| folder.viewing().or(folder.cursor()).map(str::to_owned))
                {
                    self.edit_snippet(&id);
                }
            }
            Action::Discard => self.discard_folder_snippet(),
            Action::OpenFile { uuid, preview } => self.open_snippet_file(&uuid, preview),
            Action::DownloadFiles => self.download_snippet_files(),
        }
    }

    /// `G` `;`: the folder, in the list's place, in its default order; the thread or new message
    /// closes.
    pub(crate) fn go_to_snippets(&mut self) {
        if self.snippet_unsaved() {
            self.ask_leaving_snippet(Box::new(Self::go_to_snippets));
            return;
        }
        self.leave_pane();
        self.close_thread();
        self.close_snippet_view();
        self.snippets.folder.open();
        self.snippets.folder.unsort();
        self.check_snippet_replies();
    }

    /// The view moves to the next (or previous) snippet from the cursor's.
    pub(crate) fn snippet_step(&mut self, delta: isize) {
        let ids = self.snippet_ids();
        self.snippets.folder.move_cursor(&ids, delta);
        if let Some(cursor) = self.snippets.folder.cursor().map(str::to_owned) {
            self.open_snippet_view(&cursor);
        }
    }

    /// The folder's rows, in its order.
    pub(crate) fn snippet_ids(&self) -> Vec<String> {
        let settings = self.account.settings();
        ordered(
            &settings.snippets,
            &settings.snippets_last_used,
            self.snippets.folder.sort(),
        )
        .into_iter()
        .map(|snippet| snippet.id.clone())
        .collect()
    }

    /// ⌘; opens the `Use Snippet` palette, or with no snippets the dialog that offers to create
    /// one.
    fn pick_snippet(&mut self) {
        let origin = if self.compose.focused().is_some() {
            Origin::Draft
        } else if self.thread.is_some() {
            Origin::Thread
        } else {
            Origin::Elsewhere
        };
        self.snippets.origin = Some(origin);
        let settings = self.account.settings();
        if settings.snippets.is_empty() {
            self.ask_create_snippet();
            return;
        }
        // Rows: the name (`—` when blank), the body's text (`–`), and `Me`; ranked by when each
        // was last used, plus 10 for one's own.
        let rows: Vec<Command<commands::Action>> = settings
            .snippets
            .iter()
            .map(|snippet| {
                let used = settings
                    .snippets_last_used
                    .get(&snippet.id)
                    .and_then(|at| at.parse::<jiff::Timestamp>().ok())
                    .map_or(0.0, |at| at.as_millisecond() as f64);
                let name = if snippet.name.trim().is_empty() {
                    "—".to_owned()
                } else {
                    snippet.name.clone()
                };
                let preview = compose::snippets::preview(snippet);
                Command::new(
                    format!("SNIPPET_{}", snippet.id),
                    name,
                    commands::Action::Snippets(Action::Use(snippet.id.clone())),
                )
                .description(if preview.is_empty() {
                    "–".to_owned()
                } else {
                    preview
                })
                .side_title("Me")
                .default_score(used + 10.0)
            })
            .collect();
        let picker =
            SubPalette::new("Snippets", "Use Snippet", "data/snippets", rows).footer(|query| {
                (!query.is_empty()).then(|| {
                    Command::new(
                        "CREATE_SNIPPET_NAMED",
                        format!("Create Snippet “{query}”"),
                        commands::Action::Snippets(Action::New(query.to_owned())),
                    )
                })
            });
        self.commands.open_sub_palette(picker);
    }

    /// The picked snippet goes where ⌘; was pressed.
    fn use_snippet(&mut self, id: &str, now: f64) {
        let Some(snippet) = self
            .account
            .settings()
            .snippets
            .iter()
            .find(|snippet| snippet.id == id)
            .cloned()
        else {
            return;
        };
        match self.snippets.origin.take().unwrap_or(Origin::Elsewhere) {
            Origin::Draft if self.compose.focused().is_some() => {
                let in_list = self
                    .compose
                    .focused()
                    .is_some_and(|open| open.state.field.list().is_some());
                self.compose_apply_snippet(&snippet, in_list, now);
                self.commands.hint(
                    Hint::new()
                        .text("Next time, just type")
                        .keys([";"])
                        .text("to use a Snippet inline!"),
                );
            }
            Origin::Thread if self.thread.is_some() => {
                self.compose_run(compose::Action::ReplyAllWith(Field::Body), now);
                if self.compose.focused().is_some() {
                    self.compose_apply_snippet(&snippet, false, now);
                }
            }
            _ => {
                let fields = compose::Fields {
                    from: snippet.from.clone(),
                    to: snippet.to.clone(),
                    cc: snippet.cc.clone(),
                    bcc: snippet.bcc.clone(),
                    subject: String::new(),
                    body: String::new(),
                };
                self.compose_new(fields, Some(Field::Body));
                // Recipients came with the fields; the rest (subject, body, variables, files)
                // is applying it.
                let only = exo_store::Snippet {
                    to: Vec::new(),
                    cc: Vec::new(),
                    bcc: Vec::new(),
                    ..snippet
                };
                self.compose_apply_snippet(&only, false, now);
            }
        }
        self.account.update_settings(|settings| {
            settings
                .snippets_last_used
                .insert(id.to_owned(), jiff::Timestamp::now().to_string());
        });
    }

    /// `Discard Snippet` on the snippet viewed, back in the folder, or the folder's highlighted one.
    fn discard_folder_snippet(&mut self) {
        let folder = &self.snippets.folder;
        let Some(id) = folder.viewing().or(folder.cursor()).map(str::to_owned) else {
            return;
        };
        self.close_snippet_view();
        self.discard_saved_snippet(&id);
    }
}
