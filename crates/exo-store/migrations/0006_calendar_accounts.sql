-- Calendars are listed per Google account (calendar accounts besides the mail account's own):
-- keyed by account and id. The rows already there are the mail account's,
-- whose address is its primary calendar's id; without a primary they're left to the next list.
CREATE TABLE "calendars_new" ("account" TEXT NOT NULL, "id" TEXT NOT NULL, "position" INTEGER NOT NULL, "entry" TEXT NOT NULL, PRIMARY KEY ("account", "id"));
-- #[toasty::breakpoint]
INSERT INTO "calendars_new" ("account", "id", "position", "entry") SELECT (SELECT "id" FROM "calendars" WHERE json_extract("entry", '$.primary') = 1 LIMIT 1), "id", "position", "entry" FROM "calendars" WHERE EXISTS (SELECT 1 FROM "calendars" WHERE json_extract("entry", '$.primary') = 1);
-- #[toasty::breakpoint]
DROP TABLE "calendars";
-- #[toasty::breakpoint]
ALTER TABLE "calendars_new" RENAME TO "calendars";
