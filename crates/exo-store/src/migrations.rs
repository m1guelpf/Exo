//! The database's migrations: how a file gets this build's schema, made new or brought up from
//! an older Exo's in place, keeping everything in it.
//!
//! Each migration is a file of hand-written SQL in `crates/exo-store/migrations/`, listed in
//! [`FILES`] and run by toasty's `MigrationSet`. Toasty records each in its `__toasty_migrations`
//! table by id and applies those a file hasn't run, each in a transaction of its own. A new file
//! runs them all from `0001_baseline.sql`, so nothing but the migrations makes the schema. Exo
//! hands toasty one file at a time, so a failure names its file ([`Error::Upgrade`]). Those
//! before it stay applied, the failed one is rolled back, and the next open tries it again. A
//! file that has run a migration this build doesn't know (a newer Exo applied it) isn't touched
//! ([`Error::UnknownSchema`]).
//!
//! Toasty's generator (`toasty-cli`) isn't used. It writes `ADD COLUMN … NOT NULL` with no
//! default, which SQLite refuses on a table with rows, and it knows nothing of the search index.
//!
//! # Changing the schema
//!
//! Any change to a model (a field, its type, an `Option`, a key, an index, a `#[table]`) or to
//! the search index needs a migration, or files written before it stop matching the models:
//!
//! 1. Change the model.
//! 2. Add `crates/exo-store/migrations/NNNN_what.sql`, numbered after the last, and list it last
//!    in [`FILES`]. Give it a new random id (`python3 -c "import secrets;
//!    print(hex(secrets.randbits(63)))"`), and never change the id. Toasty knows a migration by
//!    its id, so a file renumbered because two branches each added one still counts as run where
//!    it ran.
//! 3. Write its statements as a new file has them (`sqlite3 <file> .schema` on a new store
//!    prints them; so does `an_upgraded_file_has_a_new_files_schema` for the tables that
//!    differ): quoted names, and toasty's types (`TEXT` for strings, JSON and times, `BIGINT`
//!    for `i64`, `INTEGER` for `u32` and `u64`, `BOOLEAN`). Toasty runs one statement at a
//!    time, so put a line reading `-- #[toasty::breakpoint]` between statements. For example:
//!    - a new model: its `CREATE TABLE`, then a `CREATE INDEX` for each `#[index]`;
//!    - a new `#[index]`: its `CREATE INDEX`;
//!    - a new `Option` field: `ALTER TABLE "drafts" ADD COLUMN "remind_at" TEXT;`
//!    - any other new field: SQLite adds a `NOT NULL` column only with a value for the rows
//!      already there, so give it theirs: `ALTER TABLE "drafts" ADD COLUMN "pinned" BOOLEAN NOT
//!      NULL DEFAULT 0;` (toasty writes every column, so the default fills only those rows);
//!    - a removed field or model: `DROP INDEX` for its indexes, then `ALTER TABLE … DROP
//!      COLUMN …` or `DROP TABLE …`;
//!    - a changed type, `Option`, key or `#[auto]`: SQLite can't change a column, so make the
//!      table again: `CREATE TABLE "t_new" (…)`, `INSERT INTO "t_new" (…) SELECT … FROM "t"`,
//!      `DROP TABLE "t"`, `ALTER TABLE "t_new" RENAME TO "t"`, then its indexes;
//!    - rows that need new values: `UPDATE`s in the same file.
//!
//!    A `Json<T>` column's `T` isn't part of the schema: keep what older Exos wrote readable
//!    (`#[serde(default)]`), or rewrite it in a migration. Migrations are SQL only: one that
//!    needs Rust (indexing every thread again for a new search column) needs this module
//!    extended first.
//! 4. Run exo-store's tests. The ones here make a new file with every migration and check it has
//!    the models' schema. They also bring `tests/fixtures/store-0001.sqlite3` (a file that has
//!    run the baseline, with synced mail, bodies, the search index, calendar events, drafts, a
//!    held send, a queued change and settings) through every migration, and check that the result
//!    has a new file's schema, has lost nothing, and syncs without downloading mail again. Add a test
//!    of what your migration does to rows already there (a new column's value on the fixture's
//!    drafts, say); `tests::upgraded_fixture` gives you the fixture with every migration run.
//!
//! Never change, reorder or remove a migration once it's on `build`: files out there have run
//! it. When two branches each add one, the branch that lands second renumbers its file after
//! the other's, keeps its id, and runs the tests again.

use std::path::Path;

use toasty::Db;
use toasty::migration::{MigrationFile, MigrationSet};
use toasty::stmt::Value;

use crate::db::query_i64;
use crate::error::{Error, Result};

/// The migrations, oldest first, each with an id of its own.
static FILES: &[MigrationFile] = &[
    MigrationFile::new(
        0x15a8_b503_535e_c4f4,
        "0001_baseline.sql",
        include_str!("../migrations/0001_baseline.sql"),
    ),
    MigrationFile::new(
        0x5b81_1617_5f38_b1a9,
        "0002_reminders.sql",
        include_str!("../migrations/0002_reminders.sql"),
    ),
    MigrationFile::new(
        0x65ac_86bd_e728_ec0c,
        "0003_calendar_changes.sql",
        include_str!("../migrations/0003_calendar_changes.sql"),
    ),
    MigrationFile::new(
        0x3900_3fe7_941f_fc5c,
        "0004_visible_text.sql",
        include_str!("../migrations/0004_visible_text.sql"),
    ),
    MigrationFile::new(
        0x6cf0_1f39_cd59_62b7,
        "0005_list_floors.sql",
        include_str!("../migrations/0005_list_floors.sql"),
    ),
    MigrationFile::new(
        0x40a1_cdc9_5f41_82dd,
        "0006_calendar_accounts.sql",
        include_str!("../migrations/0006_calendar_accounts.sql"),
    ),
];

/// Brings `db`, the file at `path`, to this build's schema.
pub(crate) async fn migrate(db: &mut Db, path: &Path) -> Result<()> {
    // A new file has no tables yet, toasty's included.
    let tracked = query_i64(
        db,
        "SELECT count(*) FROM sqlite_master WHERE name = '__toasty_migrations'",
    )
    .await?
        > 0;
    if tracked {
        refuse_unknown(db, path).await?;
    }
    apply(db, path, FILES).await
}

/// Refuses a file that has run a migration [`FILES`] doesn't have: a newer Exo's.
async fn refuse_unknown(db: &mut Db, path: &Path) -> Result<()> {
    let rows = toasty::sql::query("SELECT id, name FROM __toasty_migrations ORDER BY applied_at")
        .exec(db)
        .await?;
    let unknown: Vec<String> = rows
        .iter()
        .filter_map(|row| match row {
            Value::Record(row) => match &row.fields[..] {
                [Value::I64(run), Value::String(name)]
                    if !FILES.iter().any(|file| id(file) == *run) =>
                {
                    Some(name.clone())
                }
                _ => None,
            },
            _ => None,
        })
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(Error::UnknownSchema {
        path: path.to_owned(),
        found: format!("{}, which this Exo doesn't have", unknown.join(", ")),
    })
}

/// Applies those of `files` that `db`, the file at `path`, hasn't run, in order, one at a time.
async fn apply(db: &Db, path: &Path, files: &'static [MigrationFile]) -> Result<()> {
    for file in files {
        let report = MigrationSet::new(std::slice::from_ref(file))
            .apply(db)
            .await
            .map_err(|source| Error::Upgrade {
                path: path.to_owned(),
                migration: file.name().to_owned(),
                source,
            })?;
        if report.applied() > 0 {
            log::info!("applied {} to an account's store", file.name());
        }
    }
    Ok(())
}

/// `file`'s id as toasty stores it: an `INTEGER`. Every id has 63 bits.
fn id(file: &MigrationFile) -> i64 {
    i64::try_from(file.id()).expect("migration ids have 63 bits")
}

#[cfg(test)]
mod tests {
    //! `tests/fixtures/store-0001.sqlite3` is the file Exo wrote at 2a2832a syncing gmail-mock's
    //! default mailbox, every list backfilled and its first bodies fetched, the search index, a
    //! week of the calendar, three drafts (one in Gmail, one held to send in 2030, one on this
    //! device only), a star Gmail hasn't got and the Carbon theme; with the baseline recorded as
    //! run, as files from then have it. It stands for those files, so it's never written
    //! again.

    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;
    use std::time::Duration;

    use gmail_mock::{Config, DEV_ACCESS_TOKEN, MockServer};
    use jiff::Timestamp;
    use toasty::stmt::Value;

    use super::*;
    use crate::db::{self, CalendarChangeRow, ReminderRow, query_i64};
    use crate::{Options, Settings, Store, SyncReport, Theme};

    const ACCOUNT: &str = "maya@brightmoor.co";

    /// A directory for one test, removed when it ends.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("exo-store-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("makes a directory");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A copy of the fixture in `dir`, as `ACCOUNT`'s file.
    fn fixture(dir: &Path) -> PathBuf {
        let path = dir.join(format!("{ACCOUNT}.sqlite3"));
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/store-0001.sqlite3"
            ),
            &path,
        )
        .expect("copies the fixture");
        path
    }

    /// The fixture in `dir`, with every migration run.
    pub(super) async fn upgraded_fixture(dir: &Path) -> Db {
        db::open(&fixture(dir)).await.expect("upgrades the fixture")
    }

    /// A new file in `dir`.
    async fn new_file(dir: &Path) -> Db {
        db::open(&dir.join("new.sqlite3"))
            .await
            .expect("creates a file")
    }

    /// The names of the migrations `db` has run, by number.
    async fn run_so_far(db: &mut Db) -> Vec<String> {
        rows(db, "SELECT name FROM __toasty_migrations ORDER BY name")
            .await
            .into_iter()
            .flatten()
            .collect()
    }

    /// Runs `statements` on `db`.
    async fn set(db: &mut Db, statements: &[&str]) {
        for sql in statements {
            toasty::sql::statement(*sql)
                .exec(db)
                .await
                .unwrap_or_else(|error| panic!("{sql}: {error}"));
        }
    }

    /// `sql`'s rows, each value as text.
    async fn rows(db: &mut Db, sql: &str) -> Vec<Vec<String>> {
        let rows = toasty::sql::query(sql)
            .exec(db)
            .await
            .unwrap_or_else(|error| panic!("{sql}: {error}"));
        rows.iter()
            .map(|row| match row {
                Value::Record(row) => row
                    .fields
                    .iter()
                    .map(|value| match value {
                        Value::String(text) => text.clone(),
                        value => format!("{value:?}"),
                    })
                    .collect(),
                row => panic!("not a row: {row:?}"),
            })
            .collect()
    }

    /// What must match between an upgraded file and a new one, one sorted line per column,
    /// index, `AUTOINCREMENT` key and virtual table, each starting with its table; toasty's own
    /// table aside. Column order and defaults may differ: `ADD COLUMN` appends, and its default
    /// only fills rows already there.
    async fn schema(db: &mut Db) -> Vec<String> {
        const SCHEMA: &str = r#"
            SELECT m.name || ' column ' || c.name || ' ' || c.type || CASE WHEN c."notnull" THEN ' NOT NULL' ELSE '' END || CASE WHEN c.pk THEN ' PRIMARY KEY ' || c.pk ELSE '' END
            FROM sqlite_master m JOIN pragma_table_xinfo(m.name) c WHERE m.type = 'table' AND m.name NOT LIKE 'sqlite_%' AND m.name <> '__toasty_migrations'
            UNION ALL
            SELECT m.name || ' index ' || CASE WHEN i.origin = 'c' THEN i.name ELSE i.origin END || CASE WHEN i."unique" THEN ' UNIQUE' ELSE '' END
                || ' (' || (SELECT group_concat(name, ', ') FROM (SELECT name FROM pragma_index_xinfo(i.name) WHERE key ORDER BY seqno)) || ')'
            FROM sqlite_master m JOIN pragma_index_list(m.name) i WHERE m.type = 'table' AND m.name NOT LIKE 'sqlite_%' AND m.name <> '__toasty_migrations'
            UNION ALL
            SELECT name || ' AUTOINCREMENT' FROM sqlite_master WHERE type = 'table' AND sql LIKE '%AUTOINCREMENT%'
            UNION ALL
            SELECT name || ' ' || sql FROM sqlite_master WHERE sql LIKE 'CREATE VIRTUAL TABLE%'
            ORDER BY 1"#;
        rows(db, SCHEMA).await.into_iter().flatten().collect()
    }

    async fn assert_same_schema(new: &mut Db, upgraded: &mut Db) {
        let (want, got) = (schema(new).await, schema(upgraded).await);
        let missing: Vec<&String> = want.iter().filter(|line| !got.contains(line)).collect();
        let extra: Vec<&String> = got.iter().filter(|line| !want.contains(line)).collect();
        if missing.is_empty() && extra.is_empty() {
            return;
        }
        let tables: BTreeSet<&str> = missing
            .iter()
            .chain(&extra)
            .filter_map(|line| line.split(' ').next())
            .collect();
        let created: Vec<String> = rows(
            new,
            "SELECT tbl_name, sql FROM sqlite_master WHERE sql NOT NULL ORDER BY rowid",
        )
        .await
        .into_iter()
        .filter(|row| tables.contains(row[0].as_str()))
        .map(|row| format!("{};", row[1]))
        .collect();
        let lines = |lines: &[&String]| {
            lines
                .iter()
                .map(|line| format!("  {line}\n"))
                .collect::<String>()
        };
        panic!(
            "the schemas differ: a migration is missing or wrong (crate::migrations).\nOnly in the first:\n{}Only in the second:\n{}The first makes those tables with:\n{}",
            lines(&missing),
            lines(&extra),
            created.join("\n")
        );
    }

    /// Every table's columns and rows (sorted), SQLite's own and the search index's internals
    /// aside.
    type Contents = BTreeMap<String, (Vec<String>, Vec<Vec<String>>)>;

    async fn contents(db: &mut Db) -> Contents {
        let columns = rows(db, "SELECT m.name, c.name FROM sqlite_master m JOIN pragma_table_info(m.name) c WHERE m.type = 'table' AND m.name NOT LIKE 'sqlite_%' AND m.name NOT LIKE 'search_fts_%' AND m.name <> '__toasty_migrations' ORDER BY m.name, c.cid").await;
        let mut contents = Contents::new();
        for row in columns {
            contents
                .entry(row[0].clone())
                .or_default()
                .0
                .push(row[1].clone());
        }
        for (table, (columns, rows_)) in &mut contents {
            let list = columns
                .iter()
                .map(|column| format!(r#""{column}""#))
                .collect::<Vec<_>>()
                .join(", ");
            *rows_ = rows(db, &format!(r#"SELECT {list} FROM "{table}""#)).await;
            rows_.sort();
        }
        contents
    }

    /// Asserts that `after` has every row `before` had, in the columns both have.
    fn assert_kept(before: &Contents, after: &Contents) {
        for (table, (columns, rows)) in before {
            let (now_columns, now_rows) = after
                .get(table)
                .unwrap_or_else(|| panic!("{table} is gone"));
            let common: Vec<&String> = columns
                .iter()
                .filter(|column| now_columns.contains(column))
                .collect();
            fn project<'a>(
                common: &[&String],
                columns: &[String],
                rows: &'a [Vec<String>],
            ) -> Vec<Vec<&'a str>> {
                let at: Vec<usize> = common
                    .iter()
                    .map(|column| {
                        columns
                            .iter()
                            .position(|c| c == *column)
                            .expect("a common column")
                    })
                    .collect();
                let mut rows: Vec<Vec<&str>> = rows
                    .iter()
                    .map(|row| at.iter().map(|&i| row[i].as_str()).collect())
                    .collect();
                rows.sort();
                rows
            }
            let (was, is) = (
                project(&common, columns, rows),
                project(&common, now_columns, now_rows),
            );
            let lost: Vec<&Vec<&str>> =
                was.iter().filter(|row| !is.contains(row)).take(2).collect();
            assert!(
                was == is,
                "{table} had {} rows and has {}; lost: {lost:?}",
                was.len(),
                is.len()
            );
        }
    }

    /// Asserts SQLite and the search index find nothing wrong with `db`.
    async fn assert_sound(db: &mut Db) {
        assert_eq!(rows(db, "PRAGMA integrity_check").await, [["ok"]]);
        set(
            db,
            &["INSERT INTO search_fts (search_fts) VALUES ('integrity-check')"],
        )
        .await;
    }

    #[tokio::test]
    async fn migrations_make_the_models_schema() {
        let dir = TempDir::new("migrate-models");
        let mut new = new_file(&dir.0).await;
        assert_eq!(
            run_so_far(&mut new).await,
            FILES
                .iter()
                .map(|file| file.name().to_owned())
                .collect::<Vec<_>>()
        );
        // The models as toasty makes them, with nothing else: the search index is the
        // migrations' alone.
        let mut models = db::connect(&dir.0.join("models.sqlite3"))
            .await
            .expect("creates a file");
        models
            .push_schema()
            .await
            .expect("makes the models' tables");
        let ours = |lines: Vec<String>| {
            lines
                .into_iter()
                .filter(|line| !line.starts_with("search_fts"))
                .collect::<Vec<_>>()
        };
        let (want, got) = (
            ours(schema(&mut models).await),
            ours(schema(&mut new).await),
        );
        if want != got {
            assert_same_schema(&mut models, &mut new).await;
        }
        assert!(
            schema(&mut new)
                .await
                .iter()
                .any(|line| line.starts_with("search_fts CREATE VIRTUAL TABLE")),
            "and the search index"
        );
    }

    #[test]
    fn every_migration_has_an_id_and_a_number_of_its_own() {
        let mut ids: Vec<i64> = FILES.iter().map(id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), FILES.len(), "ids are unique");
        for (number, file) in (1..).zip(FILES) {
            assert!(
                file.name().starts_with(&format!("{number:04}_")) && file.name().ends_with(".sql"),
                "{} is migration {number}",
                file.name()
            );
        }
    }

    #[tokio::test]
    async fn an_upgraded_file_has_a_new_files_schema() {
        let dir = TempDir::new("migrate-schema");
        let mut new = new_file(&dir.0).await;
        let mut upgraded = upgraded_fixture(&dir.0).await;
        assert_same_schema(&mut new, &mut upgraded).await;
        assert_eq!(run_so_far(&mut upgraded).await, run_so_far(&mut new).await);
        drop(upgraded);
        // Opened again, nothing runs twice.
        let mut again = db::open(&dir.0.join(format!("{ACCOUNT}.sqlite3")))
            .await
            .expect("opens again");
        assert_eq!(run_so_far(&mut again).await, run_so_far(&mut new).await);
    }

    async fn store(dir: &Path, server: &MockServer) -> Store {
        let options = Options {
            backoff: Duration::from_millis(5),
            quota: None,
            ..Options::default()
        };
        Store::open_with(
            dir,
            ACCOUNT,
            server.endpoints(),
            gmail::StaticToken(DEV_ACCESS_TOKEN.into()),
            options,
        )
        .await
        .expect("opens the store")
    }

    /// One sync, and the Gmail quota units it spent.
    async fn sync(store: &Store, server: &MockServer) -> (SyncReport, u64) {
        let units = async || {
            let url = server.url().join("/__mock/quota").expect("a URL");
            let quota: serde_json::Value = reqwest::get(url)
                .await
                .expect("asks the mock")
                .json()
                .await
                .expect("reads");
            quota["units"].as_u64().expect("units")
        };
        let before = units().await;
        let report = store.sync().await.expect("syncs");
        (report, units().await - before)
    }

    #[tokio::test]
    async fn an_upgraded_file_keeps_everything_and_syncs_without_downloading_mail_again() {
        let server = MockServer::start(Config::default())
            .await
            .expect("the mock starts");
        // What an up-to-date store's first sync after a restart costs: labels, identity, history.
        let current = TempDir::new("migrate-current");
        let store_ = store(&current.0, &server).await;
        while store_.sync().await.expect("syncs").backfilling {}
        drop(store_);
        let (_, restart) = sync(&store(&current.0, &server).await, &server).await;

        let dir = TempDir::new("migrate-kept");
        let path = fixture(&dir.0);
        let mut db = db::connect(&path).await.expect("opens the file as it is");
        // Its mail came from another gmail-mock: take up history where this one is.
        let gmail = gmail::Client::new(
            server.endpoints(),
            gmail::StaticToken(DEV_ACCESS_TOKEN.into()),
        );
        let history = gmail
            .gmail()
            .profile()
            .await
            .expect("profile")
            .history_id
            .expect("a history id");
        db::kv_set(&mut db, db::HISTORY_ID, &history)
            .await
            .expect("writes");
        let before = contents(&mut db).await;
        drop(db);

        let store = store(&dir.0, &server).await;
        let mut db = store.inner.db.clone();
        assert_eq!(
            run_so_far(&mut db).await.len(),
            FILES.len(),
            "every migration counts as run"
        );
        assert_kept(&before, &contents(&mut db).await);
        assert_sound(&mut db).await;
        let mut drafts: Vec<(String, Option<Timestamp>, bool)> = store
            .drafts()
            .await
            .expect("reads")
            .into_iter()
            .map(|draft| (draft.record.subject, draft.send_at, draft.synced))
            .collect();
        drafts.sort();
        let held = "2030-01-01T00:00:00Z".parse().expect("a time");
        assert_eq!(
            drafts,
            [
                ("Held send".into(), Some(held), true),
                ("Local draft".into(), None, false),
                ("Pushed draft".into(), None, true)
            ]
        );
        let unsent = store.unsent().await.expect("reads");
        assert_eq!(
            (unsent.sends, unsent.changes),
            (1, 2),
            "the held send, the star and the local draft"
        );
        let settings: Settings = store.settings().await.expect("reads");
        assert_eq!(settings.theme, Theme::Carbon);

        let (report, spent) = sync(&store, &server).await;
        assert_eq!(
            report,
            SyncReport::default(),
            "a history check with nothing new, no backfill"
        );
        assert_eq!(
            spent, restart,
            "the first sync after the upgrade costs what a restart's does"
        );
    }

    /// Two migrations after the real ones: a table, then what toasty's generator writes for a
    /// new `i64` field, which SQLite refuses on `drafts`, with rows.
    static FAILING: &[MigrationFile] = &[
        MigrationFile::new(
            1,
            "9002_extra.sql",
            r#"CREATE TABLE "extra" ("id" TEXT NOT NULL, PRIMARY KEY ("id"))"#,
        ),
        MigrationFile::new(
            2,
            "9003_remind_at.sql",
            r#"ALTER TABLE "drafts" ADD COLUMN "remind_at" BIGINT NOT NULL"#,
        ),
    ];
    /// The same, the second fixed.
    static FIXED: &[MigrationFile] = &[
        MigrationFile::new(
            1,
            "9002_extra.sql",
            r#"CREATE TABLE "extra" ("id" TEXT NOT NULL, PRIMARY KEY ("id"))"#,
        ),
        MigrationFile::new(
            2,
            "9003_remind_at.sql",
            "ALTER TABLE \"drafts\" ADD COLUMN \"remind_at\" BIGINT NOT NULL DEFAULT 0;\n-- #[toasty::breakpoint]\nUPDATE \"drafts\" SET \"remind_at\" = 1;",
        ),
    ];

    #[tokio::test]
    async fn the_reminders_migration_starts_empty_and_keeps_what_is_set() {
        let dir = TempDir::new("migrate-reminders");
        let path = fixture(&dir.0);
        let mut db = db::open(&path).await.expect("upgrades the fixture");
        assert!(
            ReminderRow::all()
                .exec(&mut db)
                .await
                .expect("reads")
                .is_empty(),
            "nothing was reminded before"
        );
        let at: Timestamp = "2026-11-20T13:00:00Z".parse().expect("a time");
        ReminderRow::create()
            .thread_id("t1")
            .at(at)
            .keep_on_reply(false)
            .message_ids(vec!["m1".to_owned()])
            .created_at(at)
            .fired_at(None::<Timestamp>)
            .aborted(false)
            .exec(&mut db)
            .await
            .expect("writes a reminder");
        drop(db);
        // Opening it again runs no step and keeps it.
        let mut db = db::open(&path).await.expect("opens");
        let kept = ReminderRow::all().exec(&mut db).await.expect("reads");
        assert_eq!(
            kept.iter()
                .map(|row| (row.thread_id.as_str(), row.at, row.message_ids.as_slice()))
                .collect::<Vec<_>>(),
            [("t1", at, ["m1".to_owned()].as_slice())]
        );
    }

    #[tokio::test]
    async fn the_calendar_changes_migration_starts_empty_and_keeps_what_is_queued() {
        let dir = TempDir::new("migrate-calendar-changes");
        let path = fixture(&dir.0);
        let mut db = db::open(&path).await.expect("upgrades the fixture");
        assert!(
            CalendarChangeRow::all()
                .exec(&mut db)
                .await
                .expect("reads")
                .is_empty(),
            "nothing was queued before"
        );
        let change = crate::CalendarChange::Delete {
            calendar: ACCOUNT.into(),
            id: "e1".into(),
            notify: true,
        };
        CalendarChangeRow::create()
            .token(9)
            .queue("q")
            .change(toasty::Json(change.clone()))
            .failures(0)
            .not_before(None::<Timestamp>)
            .exec(&mut db)
            .await
            .expect("queues a change");
        drop(db);
        let mut db = db::open(&path).await.expect("opens");
        let kept = CalendarChangeRow::all().exec(&mut db).await.expect("reads");
        assert_eq!(
            kept.iter()
                .map(|row| (row.token, &row.change.0))
                .collect::<Vec<_>>(),
            [(9, &change)]
        );
    }

    #[tokio::test]
    async fn the_visible_text_migration_queues_every_conversation_with_a_body() {
        let dir = TempDir::new("migrate-visible-text");
        let mut db = upgraded_fixture(&dir.0).await;
        let bodied = rows(&mut db, "SELECT DISTINCT m.thread_id FROM bodies b JOIN messages m ON m.id = b.message_id ORDER BY 1").await;
        assert!(!bodied.is_empty(), "the fixture has bodies");
        assert_eq!(
            rows(&mut db, "SELECT thread_id FROM text_due ORDER BY 1").await,
            bodied,
            "each is worked out once the app can"
        );
        assert_eq!(
            query_i64(
                &mut db,
                "SELECT count(*) FROM bodies WHERE visible IS NOT NULL"
            )
            .await
            .expect("reads"),
            0,
            "and the index has the whole text till then"
        );
    }

    #[tokio::test]
    async fn the_list_floors_migration_leaves_every_pass_where_it_was() {
        let dir = TempDir::new("migrate-list-floors");
        let mut db = upgraded_fixture(&dir.0).await;
        assert!(
            query_i64(&mut db, "SELECT count(*) FROM lists")
                .await
                .expect("reads")
                > 0,
            "the fixture has passes"
        );
        assert_eq!(
            query_i64(
                &mut db,
                "SELECT count(*) FROM lists WHERE before IS NOT NULL"
            )
            .await
            .expect("reads"),
            0,
            "no eviction has moved one"
        );
    }

    #[tokio::test]
    async fn the_calendar_accounts_migration_gives_the_list_to_the_mail_account() {
        let dir = TempDir::new("migrate-calendar-accounts");
        let mut db = upgraded_fixture(&dir.0).await;
        assert_eq!(
            query_i64(&mut db, "SELECT count(*) FROM calendars")
                .await
                .expect("reads"),
            3,
            "the fixture's list is kept"
        );
        assert_eq!(
            query_i64(
                &mut db,
                "SELECT count(*) FROM calendars WHERE account = 'maya@brightmoor.co'"
            )
            .await
            .expect("reads"),
            3,
            "as its primary calendar's account's"
        );
    }

    #[tokio::test]
    async fn a_failing_migration_is_rolled_back_named_and_tried_again() {
        let dir = TempDir::new("migrate-failing");
        let path = fixture(&dir.0);
        let mut db = db::open(&path).await.expect("upgrades");
        let before = contents(&mut db).await;
        let error = apply(&db, &path, FAILING)
            .await
            .expect_err("the second fails");
        assert!(
            matches!(&error, Error::Upgrade { migration, .. } if migration == "9003_remind_at.sql"),
            "{error}"
        );
        assert!(
            error.to_string().contains("Cannot add a NOT NULL column"),
            "{error}"
        );
        // The first stays: each migration is its own transaction.
        assert_eq!(
            run_so_far(&mut db).await,
            [
                "0001_baseline.sql",
                "0002_reminders.sql",
                "0003_calendar_changes.sql",
                "0004_visible_text.sql",
                "0005_list_floors.sql",
                "0006_calendar_accounts.sql",
                "9002_extra.sql"
            ]
        );
        assert_eq!(
            query_i64(
                &mut db,
                "SELECT count(*) FROM sqlite_master WHERE name = 'extra'"
            )
            .await
            .expect("reads"),
            1
        );
        assert_kept(&before, &contents(&mut db).await);
        assert!(
            rows(
                &mut db,
                "SELECT name FROM pragma_table_info('drafts') WHERE name = 'remind_at'"
            )
            .await
            .is_empty(),
            "the second is rolled back"
        );

        // Fixed, the next open runs only what's left.
        apply(&db, &path, FIXED).await.expect("applies");
        assert_eq!(
            run_so_far(&mut db).await,
            [
                "0001_baseline.sql",
                "0002_reminders.sql",
                "0003_calendar_changes.sql",
                "0004_visible_text.sql",
                "0005_list_floors.sql",
                "0006_calendar_accounts.sql",
                "9002_extra.sql",
                "9003_remind_at.sql"
            ]
        );
        assert_eq!(
            query_i64(&mut db, "SELECT min(remind_at) FROM drafts")
                .await
                .expect("reads"),
            1,
            "its rows are there"
        );
    }

    #[tokio::test]
    async fn a_newer_exos_file_is_left_alone() {
        // It has run a migration this Exo doesn't have.
        let dir = TempDir::new("migrate-newer");
        let path = fixture(&dir.0);
        let mut db = db::open(&path).await.expect("upgrades");
        set(&mut db, &[r#"INSERT INTO __toasty_migrations (id, name, applied_at) VALUES (99, '0002_from_a_newer_exo.sql', datetime('now'))"#, r#"CREATE TABLE "newer" ("id" TEXT NOT NULL, PRIMARY KEY ("id"))"#]).await;
        let before = contents(&mut db).await;
        drop(db);
        let error = db::open(&path).await.expect_err("isn't opened");
        assert!(
            matches!(&error, Error::UnknownSchema { found, .. } if found == "0002_from_a_newer_exo.sql, which this Exo doesn't have"),
            "{error}"
        );
        let mut db = db::connect(&path).await.expect("opens the file as it is");
        assert!(
            contents(&mut db).await == before,
            "the newer file is as it was"
        );
    }
}
