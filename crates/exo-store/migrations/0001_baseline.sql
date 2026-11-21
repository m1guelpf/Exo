-- Schema 9, the baseline: every table, index and the search index as Exo made them before
-- migrations (toasty's `push_schema` for the models in `db.rs`, then `search_fts`). Files from
-- then are brought here and recorded as having run this (`migrations::bridge`).
CREATE TABLE "settings" (
    "key" TEXT NOT NULL,
    "value" TEXT NOT NULL,
    PRIMARY KEY ("key")
);
-- #[toasty::breakpoint]
CREATE TABLE "kv" (
    "key" TEXT NOT NULL,
    "value" TEXT NOT NULL,
    PRIMARY KEY ("key")
);
-- #[toasty::breakpoint]
CREATE TABLE "events" (
    "calendar_id" TEXT NOT NULL,
    "id" TEXT NOT NULL,
    "start" BIGINT NOT NULL,
    "end" BIGINT NOT NULL,
    "all_day" BOOLEAN NOT NULL,
    "event" TEXT NOT NULL,
    PRIMARY KEY ("calendar_id", "id")
);
-- #[toasty::breakpoint]
CREATE INDEX "index_events_by_calendar_id_and_start" ON "events" ("calendar_id", "start");
-- #[toasty::breakpoint]
CREATE TABLE "calendars" (
    "id" TEXT NOT NULL,
    "position" INTEGER NOT NULL,
    "entry" TEXT NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE TABLE "messages" (
    "id" TEXT NOT NULL,
    "thread_id" TEXT NOT NULL,
    "history_id" INTEGER NOT NULL,
    "internal_date" BIGINT NOT NULL,
    "label_ids" TEXT NOT NULL,
    "subject" TEXT NOT NULL,
    "snippet" TEXT NOT NULL,
    "sender" TEXT NOT NULL,
    "sender_raw" TEXT NOT NULL,
    "to" TEXT NOT NULL,
    "cc" TEXT NOT NULL,
    "bcc" TEXT NOT NULL,
    "reply_to" TEXT NOT NULL,
    "rfc822_id" TEXT,
    "in_reply_to" TEXT,
    "references" TEXT NOT NULL,
    "list_id" TEXT,
    "list_unsubscribe" TEXT,
    "list_unsubscribe_post" TEXT,
    "headers" TEXT NOT NULL,
    "size" INTEGER NOT NULL,
    "has_attachments" BOOLEAN NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE INDEX "index_messages_by_thread_id" ON "messages" ("thread_id");
-- #[toasty::breakpoint]
CREATE TABLE "drafts" (
    "id" TEXT NOT NULL,
    "thread_id" TEXT,
    "gmail_id" TEXT,
    "message_id" TEXT,
    "record" TEXT NOT NULL,
    "raw" TEXT NOT NULL,
    "revision" INTEGER NOT NULL,
    "pushed" INTEGER NOT NULL,
    "discarded" BOOLEAN NOT NULL,
    "send_at" TEXT,
    "failures" INTEGER NOT NULL,
    "not_before" TEXT,
    "failure" TEXT,
    "failed_at" TEXT,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE TABLE "outbox" (
    "id" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    "queue" TEXT NOT NULL,
    "replace_key" TEXT,
    "mutation" TEXT NOT NULL,
    "undo" BOOLEAN NOT NULL,
    "steps" TEXT NOT NULL,
    "attempts" INTEGER NOT NULL,
    "failures" INTEGER NOT NULL,
    "not_before" TEXT,
    "started_at" TEXT
);
-- #[toasty::breakpoint]
CREATE INDEX "index_outbox_by_queue" ON "outbox" ("queue");
-- #[toasty::breakpoint]
CREATE TABLE "lists" (
    "id" TEXT NOT NULL,
    "page_token" TEXT,
    "min_sort" BIGINT,
    "complete" BOOLEAN NOT NULL,
    "seen" INTEGER NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE TABLE "participants" (
    "email" TEXT NOT NULL,
    "thread_id" TEXT NOT NULL,
    "sort" BIGINT NOT NULL,
    PRIMARY KEY ("email", "thread_id")
);
-- #[toasty::breakpoint]
CREATE INDEX "index_participants_by_email_and_sort" ON "participants" ("email", "sort");
-- #[toasty::breakpoint]
CREATE INDEX "index_participants_by_thread_id" ON "participants" ("thread_id");
-- #[toasty::breakpoint]
CREATE TABLE "list_ids" (
    "list_id" TEXT NOT NULL,
    "thread_id" TEXT NOT NULL,
    "sort" BIGINT NOT NULL,
    PRIMARY KEY ("list_id", "thread_id")
);
-- #[toasty::breakpoint]
CREATE INDEX "index_list_ids_by_list_id_and_sort" ON "list_ids" ("list_id", "sort");
-- #[toasty::breakpoint]
CREATE INDEX "index_list_ids_by_thread_id" ON "list_ids" ("thread_id");
-- #[toasty::breakpoint]
CREATE TABLE "labels" (
    "id" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "system" BOOLEAN NOT NULL,
    "hidden" BOOLEAN NOT NULL,
    "text_color" TEXT,
    "background_color" TEXT,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE TABLE "bodies" (
    "message_id" TEXT NOT NULL,
    "html" TEXT,
    "text" TEXT,
    PRIMARY KEY ("message_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "attachments" (
    "message_id" TEXT NOT NULL,
    "part_id" TEXT NOT NULL,
    "attachment_id" TEXT,
    "filename" TEXT NOT NULL,
    "mime_type" TEXT NOT NULL,
    "size" INTEGER NOT NULL,
    "content_id" TEXT,
    "inline" BOOLEAN NOT NULL,
    PRIMARY KEY ("message_id", "part_id")
);
-- #[toasty::breakpoint]
CREATE TABLE "threads" (
    "id" TEXT NOT NULL,
    "history_id" INTEGER NOT NULL,
    PRIMARY KEY ("id")
);
-- #[toasty::breakpoint]
CREATE VIRTUAL TABLE search_fts USING fts5(thread_id UNINDEXED, sort UNINDEXED, messages UNINDEXED, subject, content, "from", "to", cc, bcc, replyto, deliveredto, attachments, labels, list, rfc822msgid, meta, tokenize = "porter ascii tokenchars '_'");
