//! How a body gets the resources it references, without this crate doing any I/O.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use base64::Engine as _;
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};

use crate::{animation, view::BASE_URL};

/// Images that may be animated, by URL, waiting for the view to find the elements that show
/// them.
pub(crate) type MayAnimate = Arc<Mutex<Vec<(String, Arc<[u8]>)>>>;

/// Loads what a message body references, which is only ever an `http`/`https` URL (remote
/// images, and stylesheets or fonts the sender's CSS imports, loaded directly with no proxy)
/// or a `cid:<content-id>` (inline attachments). Recognized tracking pixels never get here
/// unless [`load_read_receipts`](crate::load_read_receipts) sends them, and with
/// [`ViewOptions::block_remote_content`](crate::ViewOptions::block_remote_content) only `cid:`
/// URLs do.
///
/// Implementations start the load and return; call [`Done::finish`] with the bytes from any
/// thread when they arrive (drop it on failure), then [`BodyView::poll`](crate::BodyView::poll)
/// on the UI thread to lay out with them.
pub trait Fetch: Send + Sync + 'static {
    /// Starts loading `url`.
    fn fetch(&self, url: &str, done: Done);
}

/// Completes one [`Fetch::fetch`].
pub struct Done {
    url: String,
    handler: Box<dyn NetHandler>,
    arrived: Arc<AtomicUsize>,
    may_animate: MayAnimate,
}

impl Done {
    /// A completion nobody waits for: what arrives is dropped.
    pub(crate) fn ignored(url: &str) -> Self {
        Self {
            url: url.to_owned(),
            handler: Box::new(Ignore),
            arrived: Arc::default(),
            may_animate: MayAnimate::default(),
        }
    }

    /// Hands the loaded bytes to the body that asked for them; images are decoded here, on the
    /// calling thread.
    pub fn finish(self, bytes: Vec<u8>) {
        note_animation(&self.may_animate, &self.url, &bytes);
        self.handler.bytes(self.url, Bytes::from(bytes));
        self.arrived.fetch_add(1, Ordering::Release);
    }
}

/// Drops a response (a read receipt's).
struct Ignore;

impl NetHandler for Ignore {
    fn bytes(self: Box<Self>, _url: String, _bytes: Bytes) {}
}

/// Keeps `bytes` aside for the view when they may be an animated image (Blitz shows only an
/// image's first frame).
fn note_animation(may_animate: &MayAnimate, url: &str, bytes: &[u8]) {
    if animation::may_animate(bytes)
        && let Ok(mut pending) = may_animate.lock()
    {
        pending.push((url.to_owned(), Arc::from(bytes)));
    }
}

/// Adapts a [`Fetch`] to Blitz, answering `data:` URLs itself.
pub(crate) struct FetchProvider {
    fetch: Arc<dyn Fetch>,
    /// Remote URLs go to `fetch`; without, only inline attachments do (remote content is
    /// blocked).
    remote: bool,
    /// How many responses have been handed over, for [`BodyView::poll`](crate::BodyView::poll).
    pub(crate) arrived: Arc<AtomicUsize>,
    pub(crate) may_animate: MayAnimate,
}

impl FetchProvider {
    pub(crate) fn new(fetch: Arc<dyn Fetch>, remote: bool) -> Self {
        Self {
            fetch,
            remote,
            arrived: Arc::default(),
            may_animate: MayAnimate::default(),
        }
    }
}

impl NetProvider for FetchProvider {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url.as_str();
        match request.url.scheme() {
            "data" => {
                if let Some(bytes) = decode_data_url(url) {
                    note_animation(&self.may_animate, url, &bytes);
                    handler.bytes(url.to_owned(), Bytes::from(bytes));
                }
            }
            "http" | "https" if !self.remote => {}
            "http" | "https" | "cid" if !url.starts_with(BASE_URL) => self.fetch.fetch(
                url,
                Done {
                    url: url.to_owned(),
                    handler,
                    arrived: self.arrived.clone(),
                    may_animate: self.may_animate.clone(),
                },
            ),
            // Unresolvable relative URLs, and local files or other schemes a sender names.
            _ => {}
        }
    }
}

/// The payload of a `data:` URL (RFC 2397), its base64 decoded as browsers decode it.
pub fn decode_data_url(url: &str) -> Option<Vec<u8>> {
    let (header, payload) = url.strip_prefix("data:")?.split_once(',')?;
    if header.to_ascii_lowercase().ends_with(";base64") {
        let compact: String = payload
            .chars()
            .filter(|c| !c.is_ascii_whitespace())
            .collect();
        // Forgiving base64, as browsers decode `data:` URLs: padding may be left off.
        let engine = base64::engine::GeneralPurpose::new(
            &base64::alphabet::STANDARD,
            base64::engine::GeneralPurposeConfig::new()
                .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
        );
        engine.decode(compact).ok()
    } else {
        Some(percent_decode(payload))
    }
}

/// Decodes `%XX` escapes, leaving malformed ones as they are (the URL spec's percent-decode).
pub(crate) fn percent_decode(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(Some(hi)), Some(Some(lo))) = (
                bytes.get(i + 1).map(|&b| hex(b)),
                bytes.get(i + 2).map(|&b| hex(b)),
            )
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_data_urls() {
        assert_eq!(
            decode_data_url("data:image/gif;base64,R0lG").unwrap(),
            b"GIF"
        );
        assert_eq!(decode_data_url("data:text/plain,a%20b").unwrap(), b"a b");
    }
}
