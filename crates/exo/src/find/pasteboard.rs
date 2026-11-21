//! The text to find, shared with other apps.

use std::sync::{Arc, Mutex, PoisonError};

/// Where the text to find is shared.
#[derive(Clone, Debug)]
pub enum Pasteboard {
    /// The macOS find pasteboard (`NSPasteboardNameFind`), which ⌘E and the find fields of other
    /// apps share.
    System,
    /// A stand-in for it, for tests.
    Memory(Arc<Mutex<String>>),
}

impl Pasteboard {
    /// Puts `text` there.
    pub(super) fn write(&self, text: &str) {
        match self {
            Self::System => system::write(text),
            Self::Memory(memory) => {
                text.clone_into(&mut memory.lock().unwrap_or_else(PoisonError::into_inner))
            }
        }
    }

    /// What's there (empty for nothing).
    pub(super) fn read(&self) -> String {
        match self {
            Self::System => system::read(),
            Self::Memory(memory) => memory
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        }
    }
}

#[cfg(target_os = "macos")]
mod system {
    use objc2_app_kit::{NSPasteboard, NSPasteboardNameFind, NSPasteboardTypeString};
    use objc2_foundation::NSString;

    pub(super) fn write(text: &str) {
        // SAFETY: AppKit's pasteboard name and type constants, read only.
        let (find, string) = unsafe { (NSPasteboardNameFind, NSPasteboardTypeString) };
        let pasteboard = NSPasteboard::pasteboardWithName(find);
        pasteboard.clearContents();
        pasteboard.setString_forType(&NSString::from_str(text), string);
    }

    pub(super) fn read() -> String {
        // SAFETY: as above.
        let (find, string) = unsafe { (NSPasteboardNameFind, NSPasteboardTypeString) };
        NSPasteboard::pasteboardWithName(find)
            .stringForType(string)
            .map(|text| text.to_string())
            .unwrap_or_default()
    }
}

#[cfg(not(target_os = "macos"))]
mod system {
    pub(super) fn write(_text: &str) {}

    pub(super) fn read() -> String {
        String::new()
    }
}
