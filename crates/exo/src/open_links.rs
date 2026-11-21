//! Open Links & Attachments (`⌘O`, the palette's `Open Link`): a sub-palette of a conversation's
//! one-click actions, files and links, and the palette's row for its last action (`OPEN_ACTION`).
//! In a thread it lists the focused message's; in the list, every message's of the highlighted
//! conversation, read (its bodies fetched, if it's stored for its rows alone) and rendered on
//! demand. A link opens in the browser; a file previews, or downloads when it can't be previewed,
//! the way a click on its card does. With ⌘ held, a link opens in the background and the list stays
//! open; with ⌘ or ⇧, a file downloads.
//!
//! Hovering a link in the focused message, as hover intent has it, hints at `⌘O`.

use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        mpsc::{Receiver, Sender, channel},
    },
};

use exo_commands::{Command, Commands, Hint, Region, SubPalette, command_score};
use exo_mail::Link;
use exo_store::{Attachment, MessageDetail, ThreadDetail, ThreadId};

use crate::{Mail, commands, mailbox::ThreadSummary};

/// The sub-palette's type.
const PALETTE: &str = "OPEN";
/// Where `⌘O` works.
const REGIONS: &[Region] = &[
    Region::ThreadListHeader,
    Region::ThreadList,
    Region::ActionPane,
    Region::ThreadMessagesHeader,
    Region::ThreadMessages,
    Region::ThreadMessageCalendarInvite,
];
/// Hover intent's poll (`interval: 250`).
const INTENT_INTERVAL: f64 = 0.25;
/// A link that says this, to this, is the `Sent via Superhuman` signature's, and links saying
/// these are its invitations: the list leaves them out.
const SIGNATURE_LINKS: &[(&str, &str)] = &[
    ("Superhuman", "https://sprh.mn/?vip"),
    ("Superhuman iOS", "https://sprh.mn/?vip"),
    ("Superhuman Android", "https://sprh.mn/?vip"),
];
const INVITATION_TEXTS: &[&str] = &["Go to Superhuman", "Take me to Superhuman"];

/// What Open Links & Attachments' commands ask for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// `⌘O` and the palette's `Open Link`: opens the list, or closes it when it's open.
    Launch,
    /// A link's row, or an action's.
    Link(String),
    /// The palette's row for the conversation's last one-click action: it opens in the
    /// background.
    OneClick(String),
    /// A file's row.
    Attachment(Box<Attachment>),
}

/// What the list shows: the actions, the files, then the links, in the messages' order.
#[derive(Debug, Default)]
struct Found {
    actions: Vec<exo_mail::Action>,
    attachments: Vec<Attachment>,
    links: Vec<Link>,
    /// A conversation being read has actions that show.
    priority: bool,
}

/// A conversation read for the list: its id and what the store has of it.
type Read = (String, Option<Arc<ThreadDetail>>);

/// Open Links & Attachments' state between frames.
pub(crate) struct OpenLinks {
    /// Shared with the open sub-palette's rows, which fill in as the messages arrive.
    found: Rc<RefCell<Found>>,
    /// What the list is for.
    target: Option<Target>,
    /// The link under the pointer, for hover intent: its message and `href`.
    hover: exo_ui::HoverIntent<(String, String)>,
    /// Conversations the store read for the list.
    read: (Sender<Read>, Receiver<Read>),
}

impl Default for OpenLinks {
    fn default() -> Self {
        Self {
            found: Rc::default(),
            target: None,
            hover: exo_ui::HoverIntent::new(INTENT_INTERVAL),
            read: channel(),
        }
    }
}

/// Whose files and links the list shows.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    /// The open thread's message.
    Message(String),
    /// The list's highlighted conversation, being read and rendered.
    Conversation(String),
}

/// Registers `⌘O` for the conversations actions apply to, and the palette's `Open Link` for one.
/// Neither is for drafts being written.
///
/// With one conversation, `action` is its last one-click action, which gets a row of its own
/// titled by what it says. Only the open conversation is rendered, so that's where it is.
pub(crate) fn register(
    commands: &mut Commands<commands::Action>,
    threads: &[&ThreadSummary],
    action: Option<&exo_mail::Action>,
) {
    if threads.is_empty() || threads.iter().all(|thread| thread.draft_only) {
        return;
    }
    let launch = commands::Action::OpenLinks(Action::Launch);
    commands.add(
        Command::shortcut("OPEN_KEY", launch.clone())
            .key("mod+o")
            .regions(REGIONS)
            .within_palette(),
    );
    if threads.len() == 1 {
        commands.add(
            Command::new("OPEN", "Open Link", launch)
                .aliases(["Open Attachment", "Open File"])
                .key_code(["⌘", "O"])
                .default_score(4.4)
                .icon("data/open-links"),
        );
        if let Some(action) = action {
            let row = Command::new(
                "OPEN_ACTION",
                action.text.clone(),
                commands::Action::OpenLinks(Action::OneClick(action.url.clone())),
            );
            commands.add(
                row.aliases(["Open action", "Quick open"])
                    .key_code(["⌘", "O"])
                    .default_score(4.401)
                    .icon("data/open-links"),
            );
        }
    }
}

/// The files a message lists here: every one but its inline images, invites included.
fn attached(message: &MessageDetail) -> impl Iterator<Item = &Attachment> {
    message
        .attachments
        .iter()
        .filter(|attachment| !attachment.inline)
}

/// A conversation's files, one per name: where the name first appears, the latest message's.
fn conversation_files(detail: &ThreadDetail) -> Vec<Attachment> {
    let mut files: Vec<Attachment> = Vec::new();
    for attachment in detail
        .messages
        .iter()
        .filter(|message| !message.is_draft())
        .flat_map(attached)
    {
        match files
            .iter_mut()
            .find(|file| file.filename == attachment.filename)
        {
            Some(file) => *file = attachment.clone(),
            None => files.push(attachment.clone()),
        }
    }
    files
}

/// Whether the list shows `link`: a web link that says something, and isn't the signature's or
/// an invitation ([`SIGNATURE_LINKS`]).
fn can_show(text: &str, url: &str) -> bool {
    let web = ["http://", "https://"].iter().any(|scheme| {
        url.strip_prefix(scheme)
            .is_some_and(|rest| !rest.is_empty())
    });
    !text.is_empty()
        && web
        && !SIGNATURE_LINKS
            .iter()
            .any(|(said, prefix)| *said == text && url.starts_with(prefix))
        && !INVITATION_TEXTS.iter().any(|said| text.contains(said))
}

/// A URL as the list shows it: no scheme, `www.` or final `/`.
fn display_url(url: &str) -> String {
    let url = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);
    let url = url.strip_prefix("www.").unwrap_or(url);
    url.strip_suffix('/').unwrap_or(url).to_owned()
}

/// Whether a link's text says nothing but "click here" and the like: with the first of each of
/// those words gone (`click(ing)?` and `visit(ing)?` with their `ing`), nothing is left.
fn is_ambiguous(title: &str) -> bool {
    let mut rest = title.to_lowercase();
    for (word, suffix) in [
        ("click", "ing"),
        ("at", ""),
        ("right", ""),
        ("here", ""),
        ("visit", "ing"),
        ("this", ""),
        ("link", ""),
        ("url", ""),
    ] {
        if let Some(at) = rest.find(word) {
            let end = at + word.len();
            let end = if rest[end..].starts_with(suffix) {
                end + suffix.len()
            } else {
                end
            };
            rest.replace_range(at..end, "");
        }
    }
    rest.trim().is_empty()
}

/// An action's or a link's row.
#[derive(Clone, Debug, PartialEq, Eq)]
struct LinkRow {
    title: String,
    description: Option<String>,
    url: String,
    /// A one-click action, and whether it's schema.org's.
    action: bool,
    declared: bool,
}

/// The actions' and links' rows, in order and without repeats: each titled by what it says, or
/// where it goes when that says nothing, with where it goes beside it otherwise.
fn link_rows(actions: &[exo_mail::Action], links: &[Link]) -> Vec<LinkRow> {
    let actions = actions.iter().map(|action| {
        (
            action.text.as_str(),
            action.url.as_str(),
            true,
            action.source.is_some(),
        )
    });
    let links = links
        .iter()
        .map(|link| (link.text.as_str(), link.url.as_str(), false, false));
    let mut rows: Vec<LinkRow> = Vec::new();
    for (text, url, action, declared) in actions
        .chain(links)
        .filter(|(text, url, ..)| can_show(text, url))
    {
        let mut title = display_url(text);
        if is_ambiguous(&title) {
            title = display_url(url);
        }
        let shown = display_url(url);
        let description = (shown.to_lowercase() != title.to_lowercase()).then_some(shown);
        let key = format!("{url}{title}").to_lowercase();
        if !rows
            .iter()
            .any(|row| format!("{}{}", row.url, row.title).to_lowercase() == key)
        {
            rows.push(LinkRow {
                title,
                description,
                url: url.to_owned(),
                action,
                declared,
            });
        }
    }
    rows
}

/// The list's rows for `query`, without case: actions, files, then links, those matching best
/// first. The first schema.org action, or Google Calendar's `More details` link (as `Open in Google
/// Calendar`), goes on top.
fn rows(found: &Found, query: &str) -> Vec<Command<commands::Action>> {
    let mut links = link_rows(&found.actions, &found.links);
    let top = links.iter().position(|row| {
        row.declared
            || (row.title.to_lowercase().starts_with("more details")
                && row.url.contains("https://www.google.com/calendar/event"))
    });
    if let Some(top) = top.filter(|top| !links[*top].declared) {
        links[top].title = "Open in Google Calendar".to_owned();
    }
    let link_row = |at: usize, row: &LinkRow| {
        let icon = if row.action {
            "data/open-links"
        } else {
            "data/open-links-link"
        };
        let command = Command::new(
            format!("LINK_{at}"),
            row.title.clone(),
            commands::Action::OpenLinks(Action::Link(row.url.clone())),
        )
        .icon(icon)
        .stays_open_on_command();
        match &row.description {
            Some(description) => command.description(description.clone()),
            None => command,
        }
    };
    let files = found
        .attachments
        .iter()
        .enumerate()
        .map(|(at, attachment)| {
            (
                false,
                Command::new(
                    format!("ATTACHMENT_{at}"),
                    attachment.filename.clone(),
                    commands::Action::OpenLinks(Action::Attachment(Box::new(attachment.clone()))),
                )
                .icon("data/add-attachments"),
            )
        });
    let actions = links
        .iter()
        .enumerate()
        .filter(|(_, row)| row.action)
        .map(|(at, row)| (Some(at) == top, link_row(at, row)));
    let others = links
        .iter()
        .enumerate()
        .filter(|(_, row)| !row.action)
        .map(|(at, row)| (Some(at) == top, link_row(at, row)));
    let mut ordered: Vec<(bool, Command<commands::Action>)> =
        actions.chain(files).chain(others).collect();
    if let Some(at) = ordered.iter().position(|(top, _)| *top) {
        let promoted = ordered.remove(at);
        ordered.insert(0, promoted);
    }
    let all: Vec<Command<commands::Action>> = ordered.into_iter().map(|(_, row)| row).collect();
    if query.is_empty() {
        return all;
    }
    let query = query.to_lowercase();
    let mut scored: Vec<(f64, Command<commands::Action>)> = all
        .into_iter()
        .filter_map(|row| {
            // `title.slice(0, 200)`, in UTF-16 units.
            let units: Vec<u16> = row
                .title()
                .unwrap_or_default()
                .to_lowercase()
                .encode_utf16()
                .take(200)
                .collect();
            let score = command_score(&String::from_utf16_lossy(&units), &query);
            (score > 0.0).then_some((score, row))
        })
        .collect();
    scored.sort_by(|(a, _), (b, _)| b.total_cmp(a));
    scored.into_iter().map(|(_, row)| row).collect()
}

impl Mail {
    /// Carries out an Open Links & Attachments command.
    pub(crate) fn open_links_run(&mut self, action: Action) {
        match action {
            // `⌘O` in the list closes it.
            Action::Launch if self.commands.is_sub_palette_open(PALETTE) => {
                self.commands.close_palette()
            }
            Action::Launch => self.open_links_launch(),
            Action::Link(url) if self.commands.ran_with().command => {
                self.open_url_in_background(&url)
            }
            Action::Link(url) => self.open_url(&url),
            Action::OneClick(url) => self.open_url_in_background(&url),
            Action::Attachment(attachment) => {
                let held = self.commands.ran_with();
                let preview = !held.command && !held.shift;
                // An image or a message opens in Exo's viewer over the list's others;
                // anything else in Quick Look, or downloads.
                if preview && crate::attachment_files::is_previewable(&attachment) {
                    let files: Vec<Attachment> = self
                        .open_links
                        .found
                        .borrow()
                        .attachments
                        .iter()
                        .filter(|file| crate::attachment_files::is_previewable(file))
                        .cloned()
                        .collect();
                    let start = files
                        .iter()
                        .position(|file| {
                            file.message_id == attachment.message_id
                                && file.part_id == attachment.part_id
                        })
                        .unwrap_or(0);
                    self.preview_attachments(files, start);
                } else {
                    self.attachment_files
                        .open(&self.account.loader(), *attachment, preview);
                }
            }
        }
    }

    /// Opens the list on the open thread's focused message, or the list's highlighted
    /// conversation, which it fills in once that's read.
    fn open_links_launch(&mut self) {
        let target = match &self.thread {
            Some(thread) => Target::Message(thread.focused().unwrap_or_default().to_owned()),
            None => {
                let Some(thread) = self.inbox.cursor().filter(|thread| !thread.draft_only) else {
                    return;
                };
                self.open_links_read(thread.id.clone());
                Target::Conversation(thread.id.clone())
            }
        };
        *self.open_links.found.borrow_mut() = Found::default();
        self.open_links.target = Some(target);
        self.open_links_fill();
        let found = self.open_links.found.clone();
        let palette = SubPalette::ordered(PALETTE, "Open", "data/open-links", move |query| {
            rows(&found.borrow(), query)
        })
        .blank_note("No links or attachments")
        .empty_note("No matching links or attachments");
        self.commands.open_sub_palette(palette);
    }

    /// Whether Open Links & Attachments' list is open. For tests.
    pub fn open_links_shown(&self) -> bool {
        self.commands.is_sub_palette_open(PALETTE)
    }

    /// Reads conversation `id` for the list, fetching the bodies Gmail has and the store doesn't.
    fn open_links_read(&self, id: String) {
        let (sender, ctx) = (self.open_links.read.0.clone(), self.ctx.clone());
        self.account.loader().run(
            move |store| async move {
                let thread = ThreadId::new(id.clone());
                if let Err(error) = store.load_bodies(&thread).await {
                    log::info!("fetching the bodies of {id} for its links: {error}");
                }
                let detail = store
                    .thread(&thread)
                    .await
                    .inspect_err(|error| log::error!("reading thread {id}: {error}"))
                    .ok()
                    .flatten();
                (id, detail.map(Arc::new))
            },
            move |read| {
                let _ = sender.send(read);
                ctx.request_repaint();
            },
        );
    }

    /// Keeps the open list in step with what it's for: the focused message, whose render's
    /// links may arrive after the list opened, or the conversation being read. Call every frame.
    pub(crate) fn open_links_frame(&mut self) {
        while let Ok((id, detail)) = self.open_links.read.1.try_recv() {
            self.open_links_thread_read(&id, detail.as_ref());
        }
        if self.commands.is_sub_palette_open(PALETTE) {
            self.open_links_fill();
        }
    }

    /// Fills the list in from what the open thread's focused message has now.
    fn open_links_fill(&mut self) {
        let Some(Target::Message(id)) = &self.open_links.target else {
            return;
        };
        let Some(message) = self.thread.as_ref().and_then(|thread| {
            thread
                .messages()
                .into_iter()
                .find(|message| &message.id == id)
                .cloned()
        }) else {
            return;
        };
        let links = self
            .bodies
            .facts(id)
            .map(|facts| facts.links.clone())
            .unwrap_or_default();
        let actions = self
            .thread
            .as_ref()
            .map(|thread| thread.focused_actions(&self.bodies))
            .unwrap_or_default();
        let mut found = self.open_links.found.borrow_mut();
        found.attachments = attached(&message).cloned().collect();
        found.links = links;
        found.actions = actions;
    }

    /// The store read conversation `id`: if the list is for it, its files go in and its messages
    /// are rendered for their links.
    fn open_links_thread_read(&mut self, id: &str, detail: Option<&Arc<ThreadDetail>>) {
        let Some(detail) =
            detail.filter(|_| self.open_links.target == Some(Target::Conversation(id.to_owned())))
        else {
            return;
        };
        let mut found = self.open_links.found.borrow_mut();
        found.attachments = conversation_files(detail);
        found.priority = crate::email_actions::have_priority(detail, &self.own_domains());
        drop(found);
        let messages: Vec<&MessageDetail> = detail
            .messages
            .iter()
            .filter(|message| !message.is_draft())
            .collect();
        self.bodies.render(id, &messages, self.account.email());
    }

    /// Conversation `id`'s messages, rendered: the list shows their links and actions if it's
    /// for it (the actions each once, if they show at all).
    pub(crate) fn open_links_rendered(&mut self, id: &str, facts: &[(String, crate::body::Facts)]) {
        if self.open_links.target == Some(Target::Conversation(id.to_owned())) {
            let mut found = self.open_links.found.borrow_mut();
            found.links = facts
                .iter()
                .flat_map(|(_, facts)| facts.links.iter().cloned())
                .collect();
            found.actions = Vec::new();
            let priority = found.priority;
            for action in facts
                .iter()
                .flat_map(|(_, facts)| facts.actions.iter().cloned())
                .filter(|_| priority)
            {
                if !found
                    .actions
                    .iter()
                    .any(|seen| seen.text == action.text && seen.url == action.url)
                {
                    found.actions.push(action);
                }
            }
        }
    }

    /// Hints at `⌘O` once the pointer settles on a link in the focused message (hover intent),
    /// unless the message is marketing mail or the list wouldn't show the link.
    pub(crate) fn open_links_hover(&mut self) {
        let hovered = self.thread.as_ref().and_then(|thread| {
            let message = thread.hovered_body()?;
            let href = thread.hovered_link(&self.bodies)?;
            (thread.focused() == Some(message)).then(|| (message.to_owned(), href))
        });
        let Some((message, href)) = self.open_links.hover.update(&self.ctx, hovered).cloned()
        else {
            return;
        };
        let Some(facts) = self.bodies.facts(&message) else {
            return;
        };
        let listed = facts
            .links
            .iter()
            .any(|link| link.url == href && can_show(&link.text, &link.url));
        if listed && !facts.unquoted_marketing {
            self.commands.hint(
                Hint::new()
                    .text("Hit")
                    .keys(["⌘", "O"])
                    .text("to open links instantly!")
                    .duration(2.0),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(text: &str, url: &str) -> Link {
        Link {
            text: text.into(),
            url: url.into(),
            ..Link::default()
        }
    }

    #[test]
    fn a_link_is_titled_by_its_text_with_its_address_beside_it() {
        let rows = link_rows(
            &[],
            &[link("Read the agenda", "https://www.brightmoor.co/agenda/")],
        );
        assert_eq!(
            rows,
            vec![LinkRow {
                title: "Read the agenda".into(),
                description: Some("brightmoor.co/agenda".into()),
                url: "https://www.brightmoor.co/agenda/".into(),
                action: false,
                declared: false
            }]
        );
    }

    #[test]
    fn a_link_that_says_where_it_goes_has_nothing_beside_it() {
        let rows = link_rows(&[], &[link("www.brightmoor.co", "https://brightmoor.co/")]);
        assert_eq!(
            (rows[0].title.as_str(), rows[0].description.as_deref()),
            ("brightmoor.co", None)
        );
    }

    #[test]
    fn click_here_is_titled_by_where_it_goes() {
        for text in [
            "Click here",
            "click here!",
            "CLICK HERE",
            "Visit this link",
            "here",
            "Clicking right here",
        ] {
            let rows = link_rows(&[], &[link(text, "https://brightmoor.co/q4")]);
            let shown = if text == "click here!" {
                "click here!"
            } else {
                "brightmoor.co/q4"
            };
            assert_eq!(rows[0].title, shown, "{text}");
        }
        assert!(!is_ambiguous("Read this"));
    }

    #[test]
    fn only_web_links_that_say_something_are_listed_once() {
        let rows = link_rows(
            &[],
            &[
                link("", "https://brightmoor.co/"),
                link("Mail us", "mailto:hi@brightmoor.co"),
                link("Bare", "https://"),
                link("Agenda", "https://brightmoor.co/agenda"),
                link("agenda", "https://brightmoor.co/agenda"),
                link("Superhuman", "https://sprh.mn/?vip=maya"),
                link("Go to Superhuman now", "https://superhuman.com/"),
            ],
        );
        assert_eq!(
            rows.iter()
                .map(|row| row.title.as_str())
                .collect::<Vec<_>>(),
            ["Agenda"]
        );
    }

    #[test]
    fn files_come_before_links_and_a_query_ranks_by_title() {
        let file = Attachment {
            filename: "Q4 board deck.pdf".into(),
            ..attachment()
        };
        let found = Found {
            attachments: vec![file],
            links: vec![
                link("Quarterly numbers", "https://brightmoor.co/q4"),
                link("Agenda", "https://brightmoor.co/agenda"),
            ],
            ..Found::default()
        };
        let titles = |query: &str| {
            rows(&found, query)
                .iter()
                .map(|row| row.title().unwrap_or_default().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            titles(""),
            ["Q4 board deck.pdf", "Quarterly numbers", "Agenda"]
        );
        assert_eq!(titles("agen"), ["Agenda"]);
        assert_eq!(titles("AGEN"), ["Agenda"], "without case");
        assert!(titles("zzz").is_empty());
    }

    #[test]
    fn actions_come_first_and_a_declared_one_or_calendars_details_on_top() {
        let file = Attachment {
            filename: "Receipt.pdf".into(),
            ..attachment()
        };
        let action = |text: &str, url: &str, source| exo_mail::Action {
            text: text.into(),
            url: url.into(),
            source,
        };
        let found = Found {
            actions: vec![
                action("Track Order", "https://shop.example/track", None),
                action(
                    "View order",
                    "https://shop.example/o/1",
                    Some(exo_mail::ActionSource::LdJson),
                ),
            ],
            attachments: vec![file],
            links: vec![
                link("Track Order", "https://shop.example/track"),
                link("Help", "https://shop.example/help"),
            ],
            priority: true,
        };
        let titles = |found: &Found| {
            rows(found, "")
                .iter()
                .map(|row| row.title().unwrap_or_default().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            titles(&found),
            ["View order", "Track Order", "Receipt.pdf", "Help"],
            "the link an action repeats shows once"
        );
        let calendar = Found {
            links: vec![
                link("Agenda", "https://brightmoor.co/a"),
                link(
                    "More details »",
                    "https://www.google.com/calendar/event?eid=1",
                ),
            ],
            ..Found::default()
        };
        assert_eq!(titles(&calendar), ["Open in Google Calendar", "Agenda"]);
    }

    #[test]
    fn a_conversation_lists_each_file_name_once_from_its_latest_message() {
        let message = |id: &str, files: &[(&str, &str)]| MessageDetail {
            id: id.into(),
            labels: vec!["INBOX".into()],
            date: jiff::Timestamp::UNIX_EPOCH,
            from: None,
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: Vec::new(),
            subject: "Board".into(),
            snippet: String::new(),
            rfc822_id: None,
            in_reply_to: None,
            references: Vec::new(),
            list_id: None,
            list_unsubscribe: None,
            list_unsubscribe_post: None,
            headers: Vec::new(),
            body: exo_store::Body::default(),
            attachments: files
                .iter()
                .map(|(name, part)| Attachment {
                    filename: (*name).into(),
                    part_id: (*part).into(),
                    ..attachment()
                })
                .collect(),
            failure: None,
            scheduled: None,
            sending: false,
        };
        let messages = vec![
            message("a", &[("deck.pdf", "1"), ("notes.txt", "2")]),
            message("b", &[("deck.pdf", "3")]),
        ];
        let detail = ThreadDetail {
            id: exo_store::ThreadId::new("t".to_owned()),
            subject: "Board".into(),
            labels: Vec::new(),
            muted: false,
            reminder: None,
            messages,
        };
        let files = conversation_files(&detail);
        assert_eq!(
            files
                .iter()
                .map(|file| (file.filename.as_str(), file.part_id.as_str()))
                .collect::<Vec<_>>(),
            [("deck.pdf", "3"), ("notes.txt", "2")]
        );
    }

    fn attachment() -> Attachment {
        Attachment {
            message_id: "m".into(),
            attachment_id: None,
            part_id: "1".into(),
            filename: String::new(),
            mime_type: "application/pdf".into(),
            size: 1,
            content_id: None,
            inline: false,
        }
    }
}
