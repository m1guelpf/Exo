//! The parts cache: Gmail's attachment and inline-image bytes kept on disk, one file each, so a
//! conversation's images and files show offline and open without a round trip. What's prefetched
//! ([`Store::prefetch_parts`]): the inline parts of every conversation whose bodies the store
//! keeps, and the files of inbox conversations up to 25 MB each, never spam's or trash's, three
//! at a time. Every part fetched for any other reason ([`Store::part_data`]) is kept too. Never
//! remote images: only Gmail's parts are fetched ahead. The cache has a budget of its own, and a
//! conversation's parts go when eviction lets its bodies go.

use std::path::{Path, PathBuf};

use futures::{StreamExt, stream};
use toasty::stmt::Value;

use crate::error::Result;
use crate::ids::labels::{INBOX, SPAM, TRASH};
use crate::store::Store;

/// The most a file prefetched for an inbox conversation may weigh.
const MAX_FILE: i64 = 25 << 20;
/// Parts fetched at once.
const CONCURRENCY: usize = 3;
/// The parts the cache should have, newest message first, with their sizes: inline parts of
/// messages with a body stored, and files of up to `?1` bytes in inbox conversations, none in
/// spam or trash nor of Exo's drafts (whose files are in the draft files), `?2` of them.
const WANTED: &str = "SELECT a.message_id, a.part_id, a.attachment_id, a.size FROM attachments a JOIN messages m ON m.id = a.message_id WHERE NOT EXISTS (SELECT 1 FROM drafts d WHERE d.message_id = a.message_id) AND NOT EXISTS (SELECT 1 FROM list_ids s WHERE s.thread_id = m.thread_id AND s.list_id IN (?3, ?4)) AND ((a.inline AND EXISTS (SELECT 1 FROM bodies b WHERE b.message_id = a.message_id)) OR (NOT a.inline AND a.size <= ?1 AND EXISTS (SELECT 1 FROM list_ids i WHERE i.thread_id = m.thread_id AND i.list_id = ?5))) ORDER BY m.internal_date DESC LIMIT ?2";

/// Where a store keeps its parts, and how much they may take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartsCache {
    /// The folder, `~/Library/Caches/Exo/<account>/parts` for a real account.
    pub dir: PathBuf,
    /// The most the files may take, in bytes; past it the oldest go, down to nine tenths.
    pub budget: u64,
}

impl PartsCache {
    /// `dir`, within 512 MB.
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            budget: 512 << 20,
        }
    }

    /// The file part `part` of message `message` is kept in. Gmail's ids are hex and part ids
    /// digits and dots, so they make a safe name.
    fn file(&self, message: &str, part: &str) -> Option<PathBuf> {
        let safe = |id: &str| {
            !id.is_empty()
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
        };
        (safe(message) && safe(part)).then(|| self.dir.join(format!("{message}_{part}")))
    }
}

impl Store {
    /// Keeps Gmail's parts in `cache` from now on: [`part_data`](Self::part_data) reads it first
    /// and fills it, [`prefetch_parts`](Self::prefetch_parts) fills it ahead. A store without one
    /// keeps nothing.
    pub fn cache_parts(&self, cache: PartsCache) {
        let _ = self.inner.parts.set(cache);
    }

    /// Part `part` of message `message` if it's kept, without asking Gmail: for drawing a
    /// message's or a draft's inline image at once, and offline.
    #[must_use]
    pub fn cached_part(&self, message: &str, part: &str) -> Option<Vec<u8>> {
        std::fs::read(self.inner.parts.get()?.file(message, part)?).ok()
    }

    /// Keeps `data` as part `part` of message `message`, best effort: a cache that can't be
    /// written is only slower.
    pub(crate) fn keep_part(&self, message: &str, part: &str, data: &[u8]) {
        let Some(path) = self
            .inner
            .parts
            .get()
            .and_then(|cache| cache.file(message, part))
        else {
            return;
        };
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| {
                // Whole or not at all: a reader never sees half a file.
                let partial = path.with_extension("partial");
                std::fs::write(&partial, data).and_then(|()| std::fs::rename(&partial, &path))
            });
        if let Err(error) = written {
            log::debug!("keeping a part: {error}");
        }
    }

    /// Lets go of the kept parts of `messages`, as eviction lets their bodies go.
    pub(crate) fn forget_parts(&self, messages: &[(String, String)]) {
        let Some(cache) = self.inner.parts.get() else {
            return;
        };
        for (message, part) in messages {
            if let Some(path) = cache.file(message, part) {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    /// Fetches up to `limit` parts the cache should have and doesn't, newest first, three at a
    /// time, in the Sync lane, then trims the cache to its budget (oldest files first, down to
    /// nine tenths). Returns how many it fetched.
    ///
    /// # Errors
    /// [`Error::Offline`](crate::Error::Offline) and the like when Gmail can't be reached, and on
    /// database failures. A part Gmail no longer has is skipped.
    pub async fn prefetch_parts(&self, limit: usize) -> Result<usize> {
        let Some(cache) = self.inner.parts.get().cloned() else {
            return Ok(0);
        };
        let mut db = self.inner.db.clone();
        let rows = toasty::sql::query(WANTED)
            .bind(MAX_FILE)
            .bind(i64::try_from(limit.saturating_mul(4)).unwrap_or(i64::MAX))
            .bind(SPAM)
            .bind(TRASH)
            .bind(INBOX)
            .exec(&mut db)
            .await?;
        // What fits in the budget, as Gmail sizes the parts.
        let mut room = cache.budget.saturating_sub(used(&cache.dir));
        let wanted: Vec<(String, String, Option<String>)> = rows
            .into_iter()
            .filter_map(|row| match row {
                Value::Record(row) => match &row.fields[..] {
                    [
                        Value::String(message),
                        Value::String(part),
                        attachment,
                        Value::I64(size),
                    ] => Some((
                        message.clone(),
                        part.clone(),
                        match attachment {
                            Value::String(id) => Some(id.clone()),
                            _ => None,
                        },
                        u64::try_from(*size).unwrap_or_default(),
                    )),
                    _ => None,
                },
                _ => None,
            })
            .filter(|(message, part, _, _)| {
                cache.file(message, part).is_some_and(|path| !path.exists())
            })
            .take_while(|(_, _, _, size)| {
                room = room.saturating_sub(*size);
                room > 0
            })
            .take(limit)
            .map(|(message, part, attachment, _)| (message, part, attachment))
            .collect();
        let fetched = stream::iter(wanted)
            .map(|(message, part, attachment)| async move {
                let data = self
                    .fetch_part(
                        crate::quota::Lane::Sync,
                        &message,
                        attachment.as_deref(),
                        &part,
                        None,
                    )
                    .await;
                (message, part, data)
            })
            .buffer_unordered(CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        let mut kept = 0;
        for (message, part, data) in fetched {
            match data {
                Ok(data) => {
                    self.keep_part(&message, &part, &data);
                    kept += 1;
                }
                Err(
                    error @ (crate::Error::Offline(_)
                    | crate::Error::Throttled { .. }
                    | crate::Error::Unauthorized(_)),
                ) => return Err(error),
                Err(error) => log::debug!("prefetching part {part} of {message}: {error}"),
            }
        }
        trim(&cache);
        Ok(kept)
    }
}

/// The bytes the cache's files take.
fn used(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.metadata().ok())
                .filter(std::fs::Metadata::is_file)
                .map(|metadata| metadata.len())
                .sum()
        })
        .unwrap_or_default()
}

/// Past its budget, deletes the cache's oldest files until it's down to nine tenths of it.
fn trim(cache: &PartsCache) {
    let Ok(entries) = std::fs::read_dir(&cache.dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok().filter(std::fs::Metadata::is_file)?;
            Some((metadata.modified().ok()?, metadata.len(), entry.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    if total <= cache.budget {
        return;
    }
    files.sort();
    let target = cache.budget / 10 * 9;
    for (_, len, path) in files {
        if total <= target {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn past_its_budget_the_oldest_files_go() {
        let dir = std::env::temp_dir().join(format!("exo-parts-trim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a folder");
        for name in ["a", "b", "c", "d"] {
            std::fs::write(dir.join(name), [0u8; 100]).expect("writes");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        trim(&PartsCache {
            dir: dir.clone(),
            budget: 400,
        });
        assert_eq!(used(&dir), 400, "within the budget, nothing goes");
        trim(&PartsCache {
            dir: dir.clone(),
            budget: 300,
        });
        let left: Vec<String> = std::fs::read_dir(&dir)
            .expect("reads")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left.len(), 2, "down to 270 of 300: {left:?}");
        assert!(
            !left.contains(&"a".to_owned()) && !left.contains(&"b".to_owned()),
            "the oldest first"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_gmails_ids_name_a_file() {
        let cache = PartsCache::new(PathBuf::from("/c"));
        assert_eq!(
            cache.file("19a2f0c4", "1.2"),
            Some(PathBuf::from("/c/19a2f0c4_1.2"))
        );
        assert_eq!(cache.file("../x", "1"), None);
        assert_eq!(cache.file("19a2", ""), None);
    }
}
