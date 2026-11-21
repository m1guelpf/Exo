#![deny(
    clippy::correctness,
    clippy::suspicious,
    clippy::complexity,
    clippy::perf
)]
//! Exo's offline-first data layer: one SQLite file per account caching Gmail, the sync engine
//! that keeps it current, and the outbox that sends the user's triage actions.
//!
//! ```no_run
//! # async fn demo(dir: &std::path::Path) -> exo_store::Result<()> {
//! use exo_store::{ListId, Mutation, Store};
//!
//! let store = Store::open(
//!     dir,
//!     "maya@brightmoor.co",
//!     gmail::Endpoints::google(),
//!     gmail::StaticToken("ya29...".into()),
//! )
//! .await?;
//! while store.sync().await?.backfilling {}
//! let inbox = store.list(&ListId::Inbox, None, 50).await?;
//! if let Some(first) = inbox.threads.first() {
//!     let undo = store.apply(&[first.id.clone()], Mutation::MarkDone).await?;
//!     store.flush().await?;
//!     store.undo(undo).await?;
//! }
//! # Ok(()) }
//! ```

mod address_book;
mod api;
mod bulk;
mod calendar;
mod connection;
mod db;
mod drafts;
mod error;
mod evict;
mod ids;
mod migrations;
mod model;
mod mutation;
mod onboarding;
mod out_of_office;
mod outbox;
mod parse;
mod parts;
mod provider;
mod query;
mod quota;
mod reminders;
mod search;
mod settings;
mod source;
mod split;
mod statistics;
mod store;
mod sync;
mod text;
mod unsubscribes;
mod view;

pub use address_book::ScoredContact;
pub use bulk::{BulkTarget, ListCutoff, Progress, Relabelled, Scope};
pub use calendar::outbox::{CalendarChange, CalendarOutcome, Cut, QueuedCalendarChange};
pub use calendar::recurrence::{End as RecurrenceEnd, Frequency, Nth, Recurrence};
pub use calendar::{AccountCalendar, StoredEvent};
pub use connection::{Connection, Debounce, Outage};
pub use drafts::{
    Address, AttachmentSource, Draft, DraftAction, DraftAttachment, DraftFailure, DraftRecord,
    DraftReminder, DraftReport, SendFailure, SendLater, SentDraft, StoredDraft, Upload, Uploads,
};
pub use error::{Error, Failing, Result, Unusable};
pub use evict::{Evicted, Eviction};
pub use ids::{Filter, ListId, ThreadId};
pub use model::{
    Alias, Attachment, Attachments, Body, Contact, Cursor, Header, Label, LabelKind, ListCount,
    MessageDetail, RecentThread, ReminderStatus, ThreadDetail, ThreadPage, ThreadSummary,
};
pub use mutation::Mutation;
pub use onboarding::{Answer, DiscoveryQuestions, PreviewEmail, PreviewList, SplitPreview};
pub use out_of_office::{
    DetectedEvent, EventTime, GoogleResponder, OUT_OF_OFFICE_LOOKAHEAD, OutOfOfficeDays,
    OutOfOfficeEventDetection, OutOfOfficeResponder,
};
pub use outbox::{FailedMutation, FlushReport, Undo, Unsent};
pub use parts::PartsCache;
pub use query::{LastToken, SearchQuery, TokenKind};
pub use quota::Quota;
pub use search::{GmailPage, Highlighted, Search, SearchPage, SearchResult};
pub use settings::{
    AdditionalTimezone, AutoAdvance, CalendarAccount, CalendarEventNotifications,
    CalendarNotifications, CalendarSetting, CalendarSettings, DeviceSettings, Document,
    EmojiSkinTone, MeetingLinks, Notifications, Nudge, PreviousDropdown, PreviousReminder,
    PreviousSchedule, PreviousSuggestion, QuickTips, ReminderSettings, RemoteContentBlocking,
    SearchHistoryItem, SearchHistoryKind, Settings, Snippet, SnippetAnalytics, SnippetSend,
    SplitsEngagement, Theme, TriggerCondition, TypingAssistance,
};
pub use split::{
    AutoLabels, CALENDAR_QUERY, Definitions, Invalidity, LabelIndex, LabelRef, LibrarySection,
    Matcher, Me, Preview, PreviewThread, Properties, QueryProperty, SPLIT_ID_PREFIX, Section,
    Senders, SplitChanges, SplitEntry, SplitInbox, SplitLibrary, SplitQuery, SystemSplit, Template,
    clean_whitespace, gmail_search, is_calendar_name, is_reserved_name, label_slug, name_error,
    query_too_long, slugify_name,
};
pub use statistics::{Achievements, Statistics};
pub use store::{Options, Store};
pub use sync::{Arrival, RECENT_THREADS, SyncProgress, SyncReport};
pub use text::{IndexText, TextSource};
pub use unsubscribes::PendingUnsubscribe;
