//! Search: the local full-text index, looking a query up in it, and Gmail's search for what
//! isn't stored or isn't matched locally, the two merged.
//!
//! Every value a user types reaches SQLite as a bound parameter, and reaches FTS5 only as a
//! bare alphanumeric term or a quoted string (`crate::query`); the SQL text is built from fixed
//! fragments alone.
//!
//! The index, `search_fts` (made by the migrations, `migrations/0001_baseline.sql`), has one row
//! per thread, its rowid derived from the thread id (`row_id`). `sort` is the thread's latest
//! message's date; `messages` lists the messages whose text `content` holds (those with a body
//! stored) and the revisions of Exo's drafts it holds, so a label change doesn't re-read bodies
//! and a body arriving or a draft's save does.

use std::collections::{BTreeSet, HashMap};
use std::ops::Range;
use std::sync::LazyLock;

use gmail::ListQuery;
use regex::Regex;
use toasty::Executor;
use toasty::stmt::Value;

use crate::db::{AttachmentRow, BodyRow, DraftRow, LabelRow, ListCursor, Mailbox};
use crate::error::Result;
use crate::ids::ListId;
use crate::ids::labels::{INBOX, SH_ALL, SH_IMPORTANT, SH_OTHER, TRASH};
use crate::model::ThreadSummary;
use crate::parse::unescape_html;
use crate::provider;
use crate::query::{FTS_ANCHOR, LabelIndex, Recheck, SearchQuery, tokenize_email};
use crate::quota::Lane;
use crate::split::{Me, important_definition};
use crate::store::{Store, load_views, reindex};
use crate::view::{Router, ThreadView};

/// `snippet()`'s column numbers.
const SUBJECT_COLUMN: i64 = 3;
const CONTENT_COLUMN: i64 = 4;
/// The index's highlight markers and ellipsis: private-use characters no mail contains.
const MARK_START: char = '\u{e550}';
const MARK_END: char = '\u{e551}';
const ELLIPSIS: char = '\u{e553}';
/// What separates the parts of a column (`ARRAY_JOINER`), and drafts' text from the rest
/// (`DRAFT_JOINER`), so a phrase never spans two.
const ARRAY_JOINER: &str = " \u{f8f8} ";
const DRAFT_JOINER: &str = " \u{f8f5} ";
/// A message's text past this many characters isn't indexed.
const MAX_MESSAGE_TEXT: usize = 100_000;
/// A subject snippet's length, in tokens.
const SUBJECT_TOKENS: i64 = 64;
/// Gmail's first page of matches, then every page after it.
const FIRST_GMAIL_PAGE: u32 = 10;
const GMAIL_PAGE: u32 = 100;

/// A thread's row id: a stable 64-bit hash of its id (FNV-1a), so updating a thread's row is a
/// lookup by rowid.
fn row_id(thread: &str) -> i64 {
    let hash = thread
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
    i64::from_ne_bytes(hash.to_ne_bytes())
}

/// Drops `thread` from the index.
pub(crate) async fn unindex(exec: &mut dyn Executor, thread: &str) -> Result<()> {
    toasty::sql::statement("DELETE FROM search_fts WHERE rowid = ?1")
        .bind(row_id(thread))
        .exec(exec)
        .await?;
    Ok(())
}

/// Writes `view`'s row: its subject, every message's text, its people, its attachments' names,
/// its labels and lists (`list_ids`), mailing lists and Message-IDs. A thread that is only a
/// draft written elsewhere isn't searchable: such drafts only show in Drafts.
pub(crate) async fn index(
    exec: &mut dyn Executor,
    view: &ThreadView,
    list_ids: &[&str],
    attachments: &[AttachmentRow],
) -> Result<()> {
    if view.messages.is_empty() {
        return unindex(exec, &view.id).await;
    }
    let row = row_id(&view.id);
    let ids: Vec<&str> = view.messages.iter().map(|m| m.row.id.as_str()).collect();
    // A draft being written is compose's: its record's text stands for its message's.
    let drafts = crate::drafts::editable(exec, &view.id).await?;
    let others: Vec<&str> = ids
        .iter()
        .filter(|id| {
            !drafts
                .iter()
                .any(|draft| draft.message_id.as_deref() == Some(**id))
        })
        .copied()
        .collect();
    let with_body: BTreeSet<String> =
        BodyRow::filter(BodyRow::fields().message_id().in_list(others.as_slice()))
            .select(BodyRow::fields().message_id())
            .exec(exec)
            .await?
            .into_iter()
            .collect();
    let bodied: Vec<&str> = others
        .into_iter()
        .filter(|id| with_body.contains(*id))
        .collect();
    let messages = bodied
        .iter()
        .map(|id| (*id).to_owned())
        .chain(
            drafts
                .iter()
                .map(|draft| format!("{}@{}", draft.id, draft.revision)),
        )
        .collect::<Vec<_>>()
        .join(" ");
    let stored = toasty::sql::query("SELECT messages, content FROM search_fts WHERE rowid = ?1")
        .bind(row)
        .exec(exec)
        .await?;
    let content = match stored.first().map(fields) {
        Some([Value::String(indexed), Value::String(content)]) if *indexed == messages => {
            content.clone()
        }
        _ => content(exec, view, &bodied, &drafts).await?,
    };
    let contacts = |field: fn(&crate::db::MessageRow) -> Vec<&Mailbox>| {
        let entries: BTreeSet<String> = view
            .messages
            .iter()
            .flat_map(|m| field(&m.row))
            .map(|mailbox| {
                let name = mailbox.name.as_deref().unwrap_or_default();
                collapse(&format!(
                    "{} {name} {}",
                    mailbox.email,
                    tokenize_email(&mailbox.email)
                ))
            })
            .collect();
        entries.into_iter().collect::<Vec<_>>().join(ARRAY_JOINER)
    };
    let delivered_to: Vec<Mailbox> = view
        .messages
        .iter()
        .flat_map(|m| {
            m.row
                .headers
                .iter()
                .filter(|header| header.name.eq_ignore_ascii_case("Delivered-To"))
        })
        .map(|header| Mailbox {
            name: None,
            email: header.value.trim().to_owned(),
        })
        .collect();
    let delivered = delivered_to
        .iter()
        .map(|mailbox| format!("{} {}", mailbox.email, tokenize_email(&mailbox.email)))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(ARRAY_JOINER);
    let files: Vec<&str> = attachments
        .iter()
        .filter(|row| !row.inline && ids.contains(&row.message_id.as_str()))
        .map(|row| row.filename.as_str())
        .collect();
    let labels: BTreeSet<&str> = list_ids.iter().copied().chain(view.labels()).collect();
    let mut meta = vec![FTS_ANCHOR];
    if !files.is_empty() {
        meta.push("attachment");
    }
    let joined = |values: Vec<String>| {
        values
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(ARRAY_JOINER)
    };
    let sort = view
        .messages
        .iter()
        .map(|m| m.row.internal_date)
        .max()
        .unwrap_or_default();
    unindex(exec, &view.id).await?;
    toasty::sql::statement(
        r#"INSERT INTO search_fts (rowid, thread_id, sort, messages, subject, content, "from", "to", cc, bcc, replyto, deliveredto, attachments, labels, list, rfc822msgid, meta) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)"#,
    )
    .bind(row)
    .bind(view.id.clone())
    .bind(sort)
    .bind(messages)
    .bind(collapse(&view.subject()))
    .bind(content)
    .bind(contacts(|row| row.sender.0.iter().collect()))
    .bind(contacts(|row| row.to.iter().collect()))
    .bind(contacts(|row| row.cc.iter().collect()))
    .bind(contacts(|row| row.bcc.iter().collect()))
    .bind(contacts(|row| row.reply_to.iter().collect()))
    .bind(delivered)
    .bind(collapse(&files.join(ARRAY_JOINER)))
    .bind(labels.into_iter().collect::<Vec<_>>().join(ARRAY_JOINER))
    .bind(joined(view.messages.iter().filter_map(|m| m.row.list_id.as_deref().map(collapse)).collect()))
    .bind(joined(view.messages.iter().filter_map(|m| m.row.rfc822_id.clone()).collect()))
    .bind(meta.join(" "))
    .exec(exec)
    .await?;
    Ok(())
}

/// The fields of a raw query's row.
fn fields(row: &Value) -> &[Value] {
    match row {
        Value::Record(record) => &record.fields[..],
        _ => &[],
    }
}

/// The text of every message (`content`): received mail first, then drafts' text. Messages
/// `ids` have their bodies stored; `drafts`, being written, are as compose last saved them.
async fn content(
    exec: &mut dyn Executor,
    view: &ThreadView,
    ids: &[&str],
    drafts: &[DraftRow],
) -> Result<String> {
    let bodies: HashMap<String, BodyRow> =
        BodyRow::filter(BodyRow::fields().message_id().in_list(ids))
            .exec(exec)
            .await?
            .into_iter()
            .map(|body| (body.message_id.clone(), body))
            .collect();
    let bodies = &bodies;
    let text = |draft: bool| {
        view.messages
            .iter()
            .filter(move |m| m.is_draft() == draft)
            .filter_map(move |m| bodies.get(&m.row.id))
            .map(body_text)
    };
    let joined = |texts: Vec<String>| {
        texts
            .into_iter()
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(ARRAY_JOINER)
    };
    let drafts = text(true)
        .chain(
            drafts
                .iter()
                .map(|draft| whole_text(Some(&draft.record.0.body), None)),
        )
        .collect();
    Ok(format!(
        "{}{DRAFT_JOINER}{}",
        joined(text(false).collect()),
        joined(drafts)
    ))
}

/// What the index holds of a stored body: the text a reader sees, without the quoted reply and
/// the signature (`crate::text`), or its whole text until that's worked out.
fn body_text(body: &BodyRow) -> String {
    match &body.visible {
        Some(visible) => truncated(collapse(visible)),
        None => whole_text(body.html.as_deref(), body.text.as_deref()),
    }
}

/// A body's whole text: the HTML's without its styles and scripts, or the plain-text part.
pub(crate) fn whole_text(html: Option<&str>, text: Option<&str>) -> String {
    static HIDDEN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?is)<(style|script|head|title)\b.*?</(style|script|head|title)\s*>|<!--.*?-->",
        )
        .expect("valid regex")
    });
    static TAG: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").expect("valid regex"));
    let text = match (html, text) {
        (Some(html), _) => unescape_html(&TAG.replace_all(&HIDDEN.replace_all(html, " "), " ")),
        (None, Some(text)) => text.to_owned(),
        (None, None) => String::new(),
    };
    truncated(collapse(&text))
}

/// `text` up to the [`MAX_MESSAGE_TEXT`] characters the index takes of a message.
fn truncated(text: String) -> String {
    match text.char_indices().nth(MAX_MESSAGE_TEXT) {
        Some((at, _)) => text[..at].to_owned(),
        None => text,
    }
}

/// Whitespace runs to one space, trimmed.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Text with some of it in bold: a search result's subject or snippet with the words that
/// matched highlighted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Highlighted {
    /// The text.
    pub text: String,
    /// Byte ranges of `text` shown in bold, in order and apart.
    pub bold: Vec<Range<usize>>,
}

impl Highlighted {
    /// Text between [`MARK_START`] and [`MARK_END`] in bold.
    fn from_marked(marked: &str) -> Self {
        let mut highlighted = Self::default();
        let mut start = None;
        for c in marked.chars() {
            match c {
                MARK_START => start = Some(highlighted.text.len()),
                MARK_END => {
                    if let Some(start) = start.take() {
                        highlighted.bold.push(start..highlighted.text.len());
                    }
                }
                c => highlighted.text.push(c),
            }
        }
        highlighted
    }

    /// Every whole-word occurrence of `terms` in bold, ignoring case.
    fn terms(text: &str, terms: &[String]) -> Self {
        let patterns = terms.iter().filter_map(|term| {
            Regex::new(&format!(r"(?i)(?-u:\b){}(?-u:\b)", regex::escape(term))).ok()
        });
        Self::bold_matches(text, patterns)
    }

    /// `text` with what `rechecks` match in bold, and nothing else: the index's highlights
    /// redone.
    fn matching(text: &str, rechecks: &[Recheck]) -> Self {
        Self::bold_matches(text, rechecks.iter().map(|recheck| recheck.regex.clone()))
    }

    /// `text` with every match of `patterns` in bold, overlapping ones merged.
    fn bold_matches(text: &str, patterns: impl Iterator<Item = Regex>) -> Self {
        let mut bold: Vec<Range<usize>> = patterns
            .flat_map(|pattern| {
                pattern
                    .find_iter(text)
                    .map(|found| found.range())
                    .collect::<Vec<_>>()
            })
            .collect();
        bold.sort_by_key(|range| range.start);
        let mut merged: Vec<Range<usize>> = Vec::new();
        for range in bold {
            match merged.last_mut() {
                Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
                _ => merged.push(range),
            }
        }
        Self {
            text: text.to_owned(),
            bold: merged,
        }
    }

    fn has_bold(&self) -> bool {
        !self.bold.is_empty()
    }
}

/// One conversation a search found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    /// Its row, dated by the result's sort time.
    pub thread: ThreadSummary,
    /// Its subject with the matched words in bold, when the query has words to match; the
    /// row's subject otherwise.
    pub subject: Option<Highlighted>,
    /// The excerpt that matched, in place of the row's snippet.
    pub snippet: Option<Highlighted>,
}

/// What a search has found so far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchPage {
    /// The conversations, newest first.
    pub results: Vec<SearchResult>,
    /// The local index has more past the limit.
    pub more_local: bool,
    /// The label ids the query asks for, which results' chips leave out.
    pub labels: Vec<String>,
    /// The words results' names show highlighted where a name or address matches.
    pub names: Vec<String>,
}

/// A message Gmail's search found (`messages.list`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailHit {
    thread: String,
    message: String,
}

/// One page of Gmail's matches.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GmailPage {
    /// The matching messages, newest first.
    pub hits: Vec<GmailHit>,
    /// The next page, if Gmail has more.
    pub next: Option<String>,
}

/// A search in progress: the query, when it started, Gmail's matches so far, and each result's
/// sort time, subject and excerpt, which are whatever it was first found with (finding it again
/// with the same sort time changes nothing).
#[derive(Debug, Clone)]
pub struct Search {
    query: SearchQuery,
    started: i64,
    gmail: Vec<GmailHit>,
    next_page: Option<String>,
    gmail_done: bool,
    found: HashMap<String, Found>,
    /// Run ahead of being asked for (instant search): Gmail's first page waits behind what's
    /// on screen.
    preloaded: bool,
}

/// How a result was first found.
#[derive(Debug, Clone)]
struct Found {
    sort: i64,
    subject: Option<Highlighted>,
    snippet: Option<Highlighted>,
}

impl Search {
    /// A search for `raw`, starting now.
    pub fn new(raw: &str) -> Self {
        Self {
            query: SearchQuery::parse(raw),
            started: jiff::Timestamp::now().as_millisecond(),
            gmail: Vec::new(),
            next_page: None,
            gmail_done: false,
            found: HashMap::new(),
            preloaded: false,
        }
    }

    /// A search for `raw` run ahead of being asked for: its first page of Gmail's matches goes in
    /// [`Lane::Show`], after what the user is waiting on.
    pub fn preload(raw: &str) -> Self {
        Self {
            preloaded: true,
            ..Self::new(raw)
        }
    }

    /// The preloaded search was asked for: a first page it hasn't asked Gmail for yet goes as
    /// a search's does.
    pub fn ask(&mut self) {
        self.preloaded = false;
    }

    /// The query.
    pub fn query(&self) -> &SearchQuery {
        &self.query
    }

    /// Gmail has no more matches to give.
    pub fn gmail_done(&self) -> bool {
        self.gmail_done
    }

    /// Where Gmail's next page starts (`None` before the first).
    pub fn next_page(&self) -> Option<&str> {
        self.next_page.as_deref()
    }

    /// How many matches Gmail has given.
    pub fn gmail_hits(&self) -> usize {
        self.gmail.len()
    }

    /// Adds a page of Gmail's matches.
    pub fn add_gmail(&mut self, page: GmailPage) {
        self.gmail_done = page.next.is_none();
        self.next_page = page.next;
        for hit in page.hits {
            if !self.gmail.contains(&hit) {
                self.gmail.push(hit);
            }
        }
    }
}

impl Store {
    /// The conversations `search` has found, newest first: up to `limit` from the local index
    /// (only back as far as All Mail is stored when `online`, since Gmail finds the older ones),
    /// then Gmail's matches it didn't find. `snippet_tokens` is how long an excerpt is, in
    /// words (10, 15, 26 or 35 with the window's width).
    ///
    /// # Errors
    /// On database failures.
    pub async fn search(
        &self,
        search: &mut Search,
        limit: usize,
        online: bool,
        snippet_tokens: i64,
    ) -> Result<SearchPage> {
        let mut db = self.inner.db.clone();
        let router = self.inner.router();
        let labels = label_index(&mut db).await?;
        let me = &router.me.addresses;
        let query = &search.query;
        let fts = query.fts(&labels, me);
        let mut sql = String::from(
            "SELECT thread_id, sort, snippet(search_fts, ?1, char(58704), char(58705), char(58707), ?2), snippet(search_fts, ?3, char(58704), char(58705), char(58707), ?4) FROM search_fts WHERE search_fts MATCH ?5",
        );
        let mut params: Vec<Value> = vec![
            SUBJECT_COLUMN.into(),
            SUBJECT_TOKENS.into(),
            CONTENT_COLUMN.into(),
            snippet_tokens.into(),
        ];
        for (at, (negated, expression)) in fts.matches.iter().enumerate() {
            params.push(expression.clone().into());
            if at > 0 {
                let not = if *negated { "NOT " } else { "" };
                sql.push_str(&format!(
                    " AND rowid {not}IN (SELECT rowid FROM search_fts WHERE search_fts MATCH ?{})",
                    params.len()
                ));
            }
        }
        for (before, date) in &fts.dates {
            params.push((*date).into());
            sql.push_str(&format!(
                " AND sort {} ?{}",
                if *before { "<" } else { ">=" },
                params.len()
            ));
        }
        // Online, local results stop where All Mail's backfill does, unless the query asks for
        // drafts or snippets.
        if online && !query.shows_label("draft") && !query.shows_label("snippets") {
            let all = ListCursor::filter_by_id(SH_ALL)
                .first()
                .exec(&mut db)
                .await?;
            let bound = match all {
                Some(cursor) if cursor.complete => None,
                Some(cursor) => Some(cursor.min_sort.unwrap_or(search.started)),
                None => Some(search.started),
            };
            if let Some(bound) = bound {
                params.push(bound.into());
                sql.push_str(&format!(" AND sort >= ?{}", params.len()));
            }
        }
        params.push(i64::try_from(limit + 1).unwrap_or(i64::MAX).into());
        sql.push_str(&format!(
            " ORDER BY sort DESC, rowid DESC LIMIT ?{}",
            params.len()
        ));
        let mut statement = toasty::sql::query(sql);
        for param in params {
            statement = statement.bind(param);
        }
        let rows = statement.exec(&mut db).await?;
        let mut found: Vec<(String, Option<Highlighted>, Option<Highlighted>)> = Vec::new();
        let highlightable = !query.highlightable_tokens().is_empty();
        for row in &rows {
            let [Value::String(thread), _, subject, snippet] = fields(row) else {
                continue;
            };
            let subject = match subject {
                Value::String(subject) if highlightable => {
                    Some(Highlighted::from_marked(subject)).filter(Highlighted::has_bold)
                }
                _ => None,
            };
            let snippet = match snippet {
                Value::String(snippet) if highlightable => pick_snippet(snippet),
                _ => None,
            };
            found.push((thread.clone(), subject, snippet));
        }
        let more_local = found.len() > limit;
        found.truncate(limit);

        let ids: Vec<String> = found
            .iter()
            .map(|(id, ..)| id.clone())
            .chain(search.gmail.iter().map(|hit| hit.thread.clone()))
            .collect();
        let mut views = load_views(&mut db, &ids).await?;
        let messages: Vec<String> = views
            .values()
            .flat_map(|view| view.all_messages().map(|m| m.row.id.clone()))
            .collect();
        let attachments = AttachmentRow::filter(
            AttachmentRow::fields()
                .message_id()
                .in_list(messages.as_slice()),
        )
        .exec(&mut db)
        .await?;
        let included = query.included_labels(&labels);
        let keeps_trash = query.shows_label("trash");
        let (rechecks, rehighlight) = query.rechecks();
        let list = if query.shows_label("sent") && included.len() == 1 {
            ListId::Sent
        } else {
            ListId::All
        };
        let mut results = Vec::new();
        let mut seen = BTreeSet::new();
        for (id, subject, snippet) in found {
            // A thread new to the index since the search started that Gmail found too was
            // stored by that Gmail search, so it shows as Gmail found it, not as an index hit.
            if !search.found.contains_key(&id) && search.gmail.iter().any(|hit| hit.thread == id) {
                continue;
            }
            let Some(view) = views.get_mut(&id) else {
                continue;
            };
            if query.checks_contacts()
                && query.is_match(view, &attachments, &labels, me) == Some(false)
            {
                continue;
            }
            let mut subject = subject;
            let mut snippet = snippet.unwrap_or_else(|| Highlighted {
                text: latest_snippet(view),
                bold: Vec::new(),
            });
            if !rechecks.is_empty() {
                let searched = subject.as_ref().map_or("", |subject| subject.text.as_str());
                if !recheck(&mut db, view, &rechecks, &snippet.text, searched).await? {
                    continue;
                }
                if rehighlight {
                    snippet = Highlighted::matching(&snippet.text, &rechecks);
                    subject = subject
                        .map(|subject| Highlighted::matching(&subject.text, &rechecks))
                        .filter(Highlighted::has_bold);
                }
            }
            let first = search.found.entry(id.clone()).or_insert_with(|| Found {
                sort: sort_for_search(view, search.started),
                subject,
                snippet: Some(snippet),
            });
            seen.insert(id.clone());
            results.push(SearchResult {
                thread: summary(
                    view,
                    &list,
                    first.sort,
                    &router,
                    &attachments,
                    &included,
                    keeps_trash,
                ),
                subject: first.subject.clone(),
                snippet: first.snippet.clone(),
            });
        }
        let terms = query.highlightable_tokens();
        for hit in &search.gmail {
            if seen.contains(&hit.thread) {
                continue;
            }
            let Some(view) = views.get_mut(&hit.thread) else {
                continue;
            };
            if view.messages.is_empty() || query.excludes(view, &attachments, &labels, me) {
                continue;
            }
            seen.insert(hit.thread.clone());
            let first = search.found.entry(hit.thread.clone()).or_insert_with(|| {
                let message = view
                    .messages
                    .iter()
                    .find(|m| m.row.id == hit.message)
                    .or_else(|| view.messages.first());
                Found {
                    sort: message.map_or_else(|| view.gmail_sort(), |m| m.row.internal_date),
                    subject: Some(Highlighted::terms(&view.subject(), &terms))
                        .filter(Highlighted::has_bold),
                    snippet: message.map(|m| {
                        Highlighted::terms(&collapse(&unescape_html(&m.row.snippet)), &terms)
                    }),
                }
            });
            results.push(SearchResult {
                thread: summary(
                    view,
                    &list,
                    first.sort,
                    &router,
                    &attachments,
                    &included,
                    keeps_trash,
                ),
                subject: first.subject.clone(),
                snippet: first.snippet.clone(),
            });
        }
        results.sort_by(|a, b| {
            b.thread
                .date
                .cmp(&a.thread.date)
                .then_with(|| a.thread.id.cmp(&b.thread.id))
        });
        Ok(SearchPage {
            results,
            more_local,
            labels: included,
            names: query.highlightable_contact_tokens(),
        })
    }

    /// The next page of Gmail's matches for `search`: `messages.list` with the query's Gmail
    /// form, ten at first and a hundred after; conversations not stored yet are fetched for their
    /// rows and stored (and so indexed) on the way. Storing them doesn't tick
    /// [`changes`](Self::changes): read the search again once the page is added to it, so they
    /// show as Gmail found them rather than as index hits.
    ///
    /// # Errors
    /// [`Error::Offline`](crate::Error::Offline) and the like when Gmail can't be reached.
    pub async fn search_gmail(&self, search: &Search) -> Result<GmailPage> {
        let inner = &self.inner;
        inner.bootstrapped().await?;
        let mut db = inner.db.clone();
        let router = inner.router();
        let labels = label_index(&mut db).await?;
        let me = Me {
            addresses: &router.me.addresses,
            name: router.me.name.as_deref(),
        };
        let q = search
            .query
            .to_gmail(&labels, &router.me.addresses, &important_definition(&me));
        let query = ListQuery {
            q: Some(q),
            max_results: Some(if search.next_page.is_some() {
                GMAIL_PAGE
            } else {
                FIRST_GMAIL_PAGE
            }),
            page_token: search.next_page.clone(),
            ..ListQuery::default()
        };
        // The first page is what the user just asked for; later ones fill in as they scroll,
        // and one run ahead may never be asked for.
        let lane = if search.next_page.is_some() || search.preloaded {
            Lane::Show
        } else {
            Lane::Act
        };
        let page = inner
            .api
            .call(lane, crate::sync::ATTEMPTS, |gmail| gmail.messages(&query))
            .await?;
        let hits: Vec<GmailHit> = page
            .messages
            .iter()
            .filter_map(|message| {
                Some(GmailHit {
                    thread: message.thread_id.clone()?,
                    message: message.id.clone()?,
                })
            })
            .collect();
        let threads: Vec<(String, u64)> = hits
            .iter()
            .map(|hit| hit.thread.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|id| (id, 0))
            .collect();
        let fetched = inner.fetch_stale(lane, &threads).await?;
        if !fetched.is_empty() {
            let mut write = inner.begin(&mut db).await?;
            for rows in &fetched {
                if let Some(id) = provider::save_rows(write.exec(), rows).await? {
                    reindex(write.exec(), &id, &router).await?;
                }
            }
            write.commit().await?;
        }
        Ok(GmailPage {
            hits,
            next: page.next_page_token,
        })
    }
}

/// The labels a query can name: the account's and Exo's own lists.
pub(crate) async fn label_index(exec: &mut dyn Executor) -> Result<LabelIndex> {
    let rows = LabelRow::all().exec(exec).await?;
    Ok(LabelIndex::new(
        [],
        rows.iter().map(|row| (row.id.as_str(), row.name.as_str())),
    ))
}

/// Whether a local result passes the check of its phrases and stemmed words: any of them in its
/// excerpt, its subject as the index highlighted it (`searched`) or its subject, or (unless it's
/// a `subject:`) a message's body as the index has it (its visible text, once worked out); a
/// negated one when it's in none.
async fn recheck(
    exec: &mut dyn Executor,
    view: &ThreadView,
    rechecks: &[Recheck],
    excerpt: &str,
    searched: &str,
) -> Result<bool> {
    let subject = view.subject();
    let mut bodies: Option<Vec<String>> = None;
    for recheck in rechecks {
        let regex = &recheck.regex;
        let mut found = regex.is_match(searched) || regex.is_match(&subject);
        if !recheck.subject_only && !found {
            found = regex.is_match(excerpt);
            if !found {
                if bodies.is_none() {
                    let ids: Vec<&str> = view.messages.iter().map(|m| m.row.id.as_str()).collect();
                    let rows =
                        BodyRow::filter(BodyRow::fields().message_id().in_list(ids.as_slice()))
                            .exec(exec)
                            .await?;
                    bodies = Some(
                        rows.into_iter()
                            .filter_map(|body| body.visible.or(body.html).or(body.text))
                            .collect(),
                    );
                }
                found = bodies.iter().flatten().any(|body| regex.is_match(body));
            }
        }
        if found != recheck.negated {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Of the excerpt's pieces (split at ellipses and between messages), the one with the most
/// matches, if any matched.
fn pick_snippet(marked: &str) -> Option<Highlighted> {
    let mut best: Option<Highlighted> = None;
    for piece in marked.split([ELLIPSIS, '\u{f8f8}', '\u{f8f5}']) {
        let piece = Highlighted::from_marked(piece.trim());
        if piece.bold.len() > best.as_ref().map_or(0, |best| best.bold.len()) {
            best = Some(piece);
        }
    }
    best
}

/// A result's sort time: the latest message at or before the search started, drafts included.
fn sort_for_search(view: &ThreadView, started: i64) -> i64 {
    let dates = view.messages.iter().map(|m| m.row.internal_date);
    dates
        .clone()
        .filter(|date| *date <= started)
        .max()
        .or_else(|| dates.max())
        .unwrap_or_default()
}

/// The last message's snippet, whitespace collapsed.
fn latest_snippet(view: &ThreadView) -> String {
    view.messages
        .last()
        .map(|m| collapse(&unescape_html(&m.row.snippet)))
        .unwrap_or_default()
}

/// A result's row. It leaves the results when a triage takes away a label the query asks for, or
/// trashes it unless the query asks for Trash.
fn summary(
    view: &mut ThreadView,
    list: &ListId,
    sort: i64,
    router: &Router,
    attachments: &[AttachmentRow],
    included: &[String],
    keeps_trash: bool,
) -> ThreadSummary {
    let mut row = view.summary(list, sort, router, attachments);
    row.removed_by = view.removed_by_with(router, attachments, None, |lists, labels| {
        (!keeps_trash && labels.contains(TRASH))
            || included.iter().any(|label| match label.as_str() {
                INBOX => !lists
                    .iter()
                    .any(|id| *id == SH_IMPORTANT || *id == SH_OTHER),
                label => !lists.contains(&label) && !labels.contains(label),
            })
    });
    row
}
