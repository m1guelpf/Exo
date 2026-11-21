ALTER TABLE "bodies" ADD COLUMN "visible" TEXT;
-- #[toasty::breakpoint]
CREATE TABLE "text_due" ("thread_id" TEXT NOT NULL, PRIMARY KEY ("thread_id"));
-- #[toasty::breakpoint]
INSERT INTO "text_due" ("thread_id") SELECT DISTINCT m."thread_id" FROM "bodies" b JOIN "messages" m ON m."id" = b."message_id";
