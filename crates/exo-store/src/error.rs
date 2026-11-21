//! The store's errors.

use std::path::PathBuf;

use crate::ids::ThreadId;

/// Everything that can go wrong in the store.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Gmail couldn't be reached (no network, DNS, TLS, timeouts). Reads and queued mutations
    /// keep working; sync again once the network is back.
    #[error("Gmail is unreachable: {0}")]
    Offline(#[source] gmail::Error),
    /// Gmail is rate limiting the account, and Exo has paused calls until `until`.
    #[error("Gmail is rate limiting this account until {until}")]
    Throttled {
        /// When calls resume.
        until: jiff::Timestamp,
    },
    /// Gmail rejected the access token even after asking the token provider for a new one: the
    /// account needs to sign in again.
    #[error("Gmail rejected the credentials: {0}")]
    Unauthorized(#[source] gmail::Error),
    /// Gmail answered with an error that retrying won't fix, or retries ran out.
    #[error(transparent)]
    Gmail(gmail::Error),
    /// Calendar has the event an answer is for, cancelled (`Event has been deleted`).
    #[error("the event has been cancelled")]
    EventCancelled,
    /// The local database failed.
    #[error("local database: {0}")]
    Database(#[from] toasty::Error),
    /// A migration of the database file failed and was rolled back. Those before it stay
    /// applied, and the next open tries it again.
    #[error("applying {migration} to {} failed and was rolled back, to try again on the next open: {source}", path.display())]
    Upgrade {
        /// The database file.
        path: PathBuf,
        /// The migration's file (`0002_reminders.sql`).
        migration: String,
        /// What the database said.
        #[source]
        source: toasty::Error,
    },
    /// The database file has a schema this build can't open: a migration a newer Exo applied.
    /// It was left as it was.
    #[error("{} has a schema this Exo can't open: {found}", path.display())]
    UnknownSchema {
        /// The database file.
        path: PathBuf,
        /// What it has.
        found: String,
    },
    /// The database file couldn't be created or replaced.
    #[error("can't open {}: {source}", path.display())]
    Io {
        /// The file or directory involved.
        path: PathBuf,
        /// What the OS said.
        #[source]
        source: std::io::Error,
    },
    /// An account's file couldn't be deleted.
    #[error("can't delete {}: {source}", path.display())]
    Delete {
        /// The file or directory involved.
        path: PathBuf,
        /// What the OS said.
        #[source]
        source: std::io::Error,
    },
    /// The account name can't be used as a file name.
    #[error("{0:?} isn't a usable account name")]
    InvalidAccount(String),
    /// A message no longer has the MIME part asked for (or has no data for it).
    #[error("message {message} has no part {part}")]
    MissingPart {
        /// The Gmail message id.
        message: String,
        /// The MIME part id (`1`, `1.2`, …).
        part: String,
    },
    /// The thread isn't in the local store.
    #[error("thread {0} isn't in the local store")]
    UnknownThread(ThreadId),
    /// Something the call needs only exists after the first [`sync`](crate::Store::sync), such
    /// as the hidden mute label.
    #[error("{0} isn't available until the first sync")]
    NotSynced(&'static str),
    /// A bulk action can't reach the conversations asked for, because Gmail can't search for
    /// them: those the Important or No reply filter passes.
    #[error("Gmail can't search for the {0} filter")]
    Unsearchable(&'static str),
}

/// Why a store can't be used, as the error screens tell them apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unusable {
    /// The disk is full.
    DiskFull,
    /// Something else holds the database.
    Locked,
    /// Anything else stopping it: a file SQLite can't read, one from a schema this build has no
    /// upgrade from, an upgrade that failed. Every such database counts as corrupt, and clearing
    /// the cache ([`crate::Store::clear_cache`]) fixes them.
    Corrupt,
}

impl Error {
    /// Why the store can't be used, when this is its database's error (opening it, say); `None`
    /// for Gmail's errors and the like.
    #[must_use]
    pub fn unusable(&self) -> Option<Unusable> {
        let database = match self {
            Self::Database(source) | Self::Upgrade { source, .. } => source,
            Self::UnknownSchema { .. } => return Some(Unusable::Corrupt),
            Self::Io { source, .. } => {
                return Some(if source.kind() == std::io::ErrorKind::StorageFull {
                    Unusable::DiskFull
                } else {
                    Unusable::Corrupt
                });
            }
            _ => return None,
        };
        let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(database);
        while let Some(error) = cause {
            if let Some(code) = error
                .downcast_ref::<rusqlite::Error>()
                .and_then(rusqlite::Error::sqlite_error_code)
            {
                return Some(match code {
                    rusqlite::ErrorCode::DiskFull => Unusable::DiskFull,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => {
                        Unusable::Locked
                    }
                    _ => Unusable::Corrupt,
                });
            }
            cause = error.source();
        }
        Some(Unusable::Corrupt)
    }
}

/// What a database error says about a store that opened fine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failing {
    /// It can't be used: a corrupt file (`SQLITE_CORRUPT`, reported at once) or a full disk.
    Unusable(Unusable),
    /// A failure that may pass: I/O, or a file SQLite can't open (`SQLITE_IOERR`,
    /// `SQLITE_CANTOPEN`), which the app takes for a corrupt file after five in a row.
    Passing,
}

impl Error {
    /// What this error says about a store that opened fine; `None` when it says nothing (Gmail's
    /// errors, a busy database, a query of the store's own that failed).
    #[must_use]
    pub fn failing(&self) -> Option<Failing> {
        let Self::Database(database) = self else {
            return None;
        };
        let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(database);
        while let Some(error) = cause {
            if let Some(code) = error
                .downcast_ref::<rusqlite::Error>()
                .and_then(rusqlite::Error::sqlite_error_code)
            {
                return match code {
                    rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase => {
                        Some(Failing::Unusable(Unusable::Corrupt))
                    }
                    rusqlite::ErrorCode::DiskFull => Some(Failing::Unusable(Unusable::DiskFull)),
                    rusqlite::ErrorCode::SystemIoFailure | rusqlite::ErrorCode::CannotOpen => {
                        Some(Failing::Passing)
                    }
                    _ => None,
                };
            }
            cause = error.source();
        }
        None
    }
}

/// `Result` with the store's [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    fn sqlite(code: std::ffi::c_int) -> Error {
        let failure = rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None);
        Error::Database(toasty::Error::driver_operation_failed(failure))
    }

    #[test]
    fn database_errors_are_told_apart() {
        assert_eq!(
            sqlite(rusqlite::ffi::SQLITE_FULL).unusable(),
            Some(Unusable::DiskFull)
        );
        assert_eq!(
            sqlite(rusqlite::ffi::SQLITE_BUSY).unusable(),
            Some(Unusable::Locked)
        );
        assert_eq!(
            sqlite(rusqlite::ffi::SQLITE_LOCKED).unusable(),
            Some(Unusable::Locked)
        );
        assert_eq!(
            sqlite(rusqlite::ffi::SQLITE_NOTADB).unusable(),
            Some(Unusable::Corrupt)
        );
        assert_eq!(
            sqlite(rusqlite::ffi::SQLITE_CORRUPT).unusable(),
            Some(Unusable::Corrupt)
        );
        let full = std::io::Error::from(std::io::ErrorKind::StorageFull);
        assert_eq!(
            Error::Io {
                path: "x".into(),
                source: full
            }
            .unusable(),
            Some(Unusable::DiskFull)
        );
        let refused = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(
            Error::Io {
                path: "x".into(),
                source: refused
            }
            .unusable(),
            Some(Unusable::Corrupt)
        );
        assert_eq!(
            Error::UnknownSchema {
                path: "x".into(),
                found: "0003_x.sql, which this Exo doesn't have".into()
            }
            .unusable(),
            Some(Unusable::Corrupt)
        );
        let upgrade = Error::Upgrade {
            path: "x".into(),
            migration: "0002_x.sql".into(),
            source: toasty::Error::driver_operation_failed(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL),
                None,
            )),
        };
        assert_eq!(
            upgrade.unusable(),
            Some(Unusable::DiskFull),
            "an upgrade that ran out of disk"
        );
        assert_eq!(
            Error::NotSynced("labels").unusable(),
            None,
            "not the database's"
        );
    }

    #[test]
    fn a_store_that_opened_fails_on_corruption_and_a_full_disk_and_may_pass_io() {
        assert_eq!(
            sqlite(rusqlite::ffi::SQLITE_CORRUPT).failing(),
            Some(Failing::Unusable(Unusable::Corrupt))
        );
        assert_eq!(
            sqlite(rusqlite::ffi::SQLITE_NOTADB).failing(),
            Some(Failing::Unusable(Unusable::Corrupt))
        );
        assert_eq!(
            sqlite(rusqlite::ffi::SQLITE_FULL).failing(),
            Some(Failing::Unusable(Unusable::DiskFull))
        );
        assert_eq!(
            sqlite(rusqlite::ffi::SQLITE_IOERR).failing(),
            Some(Failing::Passing)
        );
        assert_eq!(
            sqlite(rusqlite::ffi::SQLITE_CANTOPEN).failing(),
            Some(Failing::Passing)
        );
        assert_eq!(
            sqlite(rusqlite::ffi::SQLITE_BUSY).failing(),
            None,
            "busy waits"
        );
        assert_eq!(
            sqlite(rusqlite::ffi::SQLITE_CONSTRAINT).failing(),
            None,
            "the store's own query"
        );
        assert_eq!(Error::NotSynced("labels").failing(), None);
    }

    #[tokio::test]
    async fn a_file_that_isnt_a_database_is_corrupt() {
        let dir =
            std::env::temp_dir().join(format!("exo-store-not-a-database-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("makes a directory");
        let path = dir.join("account.sqlite3");
        std::fs::write(&path, vec![0x5a; 8192]).expect("writes");
        let error = crate::db::open(&path).await.expect_err("isn't a database");
        assert_eq!(error.unusable(), Some(Unusable::Corrupt), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
