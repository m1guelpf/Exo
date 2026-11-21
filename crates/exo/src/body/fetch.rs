//! What message bodies load: remote images straight from the sender's server, with no proxy, on
//! the public internet only (`crate::remote_content`), and `cid:` images from the message's
//! inline attachments, both through the account's [`Loader`].

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::Sender,
    },
};

use exo_store::Attachment;

use super::Job;
use crate::account::Loader;

/// One thread's [`exo_mail::Fetch`]: it knows the thread's inline attachments by content id.
pub(super) struct Fetcher {
    loader: Loader,
    /// Inline attachments by lower-cased `Content-ID`.
    inline: HashMap<String, Attachment>,
    /// Wakes the body worker when bytes arrive, counted in its `pending`.
    wake: Sender<Job>,
    pending: Arc<AtomicUsize>,
}

impl Fetcher {
    pub(super) fn new(
        loader: Loader,
        attachments: Vec<Attachment>,
        wake: Sender<Job>,
        pending: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            loader,
            inline: by_content_id(attachments),
            wake,
            pending,
        }
    }
}

/// `attachments` that have a `Content-ID`, by it, lower-cased.
pub(crate) fn by_content_id(attachments: Vec<Attachment>) -> HashMap<String, Attachment> {
    attachments
        .into_iter()
        .filter_map(|attachment| Some((attachment.content_id.clone()?.to_lowercase(), attachment)))
        .collect()
}

/// The attachment in `inline` (from [`by_content_id`]) a `cid:` URL's `cid` names (`cid:` URLs
/// may be percent-encoded).
pub(crate) fn named<'a>(
    inline: &'a HashMap<String, Attachment>,
    cid: &str,
) -> Option<&'a Attachment> {
    let cid = cid.to_lowercase();
    inline
        .get(&cid)
        .or_else(|| inline.get(&percent_decode(&cid)))
}

impl exo_mail::Fetch for Fetcher {
    fn fetch(&self, url: &str, done: exo_mail::Done) {
        let (wake, pending) = (self.wake.clone(), self.pending.clone());
        let finish = move |bytes: Option<Vec<u8>>| {
            if let Some(bytes) = bytes {
                done.finish(bytes);
                pending.fetch_add(1, Ordering::SeqCst);
                if wake.send(Job::Wake).is_err() {
                    pending.fetch_sub(1, Ordering::SeqCst);
                }
            }
        };
        if let Some(cid) = url.strip_prefix("cid:") {
            match named(&self.inline, cid) {
                Some(attachment) => self
                    .loader
                    .attachment(attachment, |bytes| finish(bytes.ok())),
                None => log::debug!("no inline attachment for a `cid:` link"),
            }
        } else if url.starts_with("https://") || url.starts_with("http://") {
            self.loader.url(url, finish);
        }
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = |offset: usize| {
            bytes
                .get(at + offset)
                .and_then(|byte| (*byte as char).to_digit(16))
        };
        match (bytes[at], hex(1), hex(2)) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                at += 3;
            }
            (byte, _, _) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::percent_decode;

    #[test]
    fn decodes_percent_escapes_in_content_ids() {
        assert_eq!(percent_decode("image001.png%4001d9"), "image001.png@01d9");
        assert_eq!(percent_decode("plain@cid"), "plain@cid");
    }
}
