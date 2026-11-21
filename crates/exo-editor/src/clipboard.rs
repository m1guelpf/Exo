//! The rich clipboard: copies carry `text/html` and `text/plain`; pastes prefer the HTML.
//! egui's own clipboard is plain text only, so the editor talks to the system's directly.

/// Where copies go and pastes come from.
pub trait Clipboard {
    /// Puts `html`, with `text` as its plain-text alternative, on the clipboard.
    fn write(&mut self, html: &str, text: &str);
    /// The clipboard's HTML, if it has any.
    fn read_html(&mut self) -> Option<String>;
}

/// The system clipboard.
#[derive(Default)]
pub struct SystemClipboard {
    inner: Option<arboard::Clipboard>,
}

impl SystemClipboard {
    fn get(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.inner.is_none() {
            self.inner = arboard::Clipboard::new().ok();
        }
        self.inner.as_mut()
    }
}

impl Clipboard for SystemClipboard {
    fn write(&mut self, html: &str, text: &str) {
        if let Some(clipboard) = self.get()
            && let Err(error) = clipboard.set_html(html, Some(text))
        {
            log::warn!("couldn't copy to the clipboard: {error}");
        }
    }

    fn read_html(&mut self) -> Option<String> {
        self.get()?
            .get()
            .html()
            .ok()
            .filter(|html| !html.is_empty())
    }
}

/// An in-memory clipboard, for tests and offscreen rendering.
#[derive(Clone, Debug, Default)]
pub struct MemoryClipboard {
    /// The HTML last written (or to paste).
    pub html: Option<String>,
    /// The plain text last written.
    pub text: Option<String>,
}

impl Clipboard for MemoryClipboard {
    fn write(&mut self, html: &str, text: &str) {
        self.html = Some(html.to_owned());
        self.text = Some(text.to_owned());
    }

    fn read_html(&mut self) -> Option<String> {
        self.html.clone()
    }
}
