//! The system clipboard for copies egui's plain-text clipboard can't make (HTML, a subject with
//! the search that finds it), and an in-memory stand-in that screenshots and tests install so they leave the user's alone.

use std::sync::{Arc, Mutex};

use exo_editor::{Clipboard, MemoryClipboard, SystemClipboard};

type Captured = Arc<Mutex<MemoryClipboard>>;

fn id() -> egui::Id {
    egui::Id::new("exo::clipboard")
}

/// Puts `html`, with `text` as its plain-text alternative, on the clipboard: the system's, or
/// the stand-in [`capture`] installed in `ctx`.
pub(crate) fn write(ctx: &egui::Context, html: &str, text: &str) {
    match ctx.data(|data| data.get_temp::<Captured>(id())) {
        Some(captured) => {
            captured
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .write(html, text);
            ctx.data_mut(|data| data.remove_temp::<String>(search_id()));
            // The copy replaces what the clipboard held, copied attachments too.
            if let Some(attachments) =
                ctx.data(|data| data.get_temp::<CapturedAttachments>(attachments_id()))
            {
                *attachments
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
            }
        }
        None => SystemClipboard::default().write(html, text),
    }
}

/// The pasteboard type of a copy's search.
#[cfg(target_os = "macos")]
const SEARCH_TYPE: &str = "build.miguel.exo.search-query";

/// Puts `text` on the clipboard with `search`, the query that finds what was copied, beside it
/// for the search bar to paste instead (`Copy Subject`): the system's, or the stand-in
/// [`capture`] installed in `ctx`.
pub(crate) fn write_with_search(ctx: &egui::Context, text: &str, search: &str) {
    match ctx.data(|data| data.get_temp::<Captured>(id())) {
        Some(captured) => {
            let mut clipboard = captured
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (clipboard.html, clipboard.text) = (None, Some(text.to_owned()));
            ctx.data_mut(|data| data.insert_temp(search_id(), search.to_owned()));
        }
        None => system_write_with_search(text, search),
    }
}

/// The search copied with the clipboard's text ([`write_with_search`]), while that copy is still
/// what the clipboard holds.
pub(crate) fn read_search(ctx: &egui::Context) -> Option<String> {
    if ctx.data(|data| data.get_temp::<Captured>(id())).is_some() {
        return ctx.data(|data| data.get_temp::<String>(search_id()));
    }
    system_search()
}

fn search_id() -> egui::Id {
    id().with("search")
}

#[cfg(target_os = "macos")]
fn system_write_with_search(text: &str, search: &str) {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    use objc2_foundation::NSString;

    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    // SAFETY: AppKit's pasteboard type constant, read only.
    let string = unsafe { NSPasteboardTypeString };
    let written = pasteboard.setString_forType(&NSString::from_str(text), string)
        && pasteboard.setString_forType(
            &NSString::from_str(search),
            &NSString::from_str(SEARCH_TYPE),
        );
    if !written {
        log::warn!("couldn't copy to the clipboard");
    }
}

#[cfg(not(target_os = "macos"))]
fn system_write_with_search(text: &str, _search: &str) {
    if let Err(error) = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text))
    {
        log::warn!("couldn't copy to the clipboard: {error}");
    }
}

#[cfg(target_os = "macos")]
fn system_search() -> Option<String> {
    use objc2_foundation::NSString;

    let search = objc2_app_kit::NSPasteboard::generalPasteboard()
        .stringForType(&NSString::from_str(SEARCH_TYPE))?;
    Some(search.to_string())
}

#[cfg(not(target_os = "macos"))]
fn system_search() -> Option<String> {
    None
}

/// The clipboard's HTML, if any: the system's, or the stand-in's.
pub(crate) fn read_html(ctx: &egui::Context) -> Option<String> {
    match ctx.data(|data| data.get_temp::<Captured>(id())) {
        Some(captured) => captured
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .read_html(),
        None => SystemClipboard::default().read_html(),
    }
}

/// Puts `image` (straight RGBA) on the clipboard as a picture: the system's, or as a PNG the
/// stand-in's, which [`read_image`] then reads.
pub(crate) fn write_image(ctx: &egui::Context, image: &exo_mail::Image) {
    if ctx.data(|data| data.get_temp::<Captured>(id())).is_some() {
        let Some(buffer) =
            image::RgbaImage::from_raw(image.width, image.height, image.rgba.clone())
        else {
            return;
        };
        let mut png = Vec::new();
        if image::DynamicImage::ImageRgba8(buffer)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .is_ok()
        {
            ctx.data_mut(|data| data.insert_temp(image_id(), Arc::new(png)));
        }
        return;
    }
    let data = arboard::ImageData {
        width: image.width as usize,
        height: image.height as usize,
        bytes: std::borrow::Cow::Borrowed(&image.rgba),
    };
    if let Err(error) =
        arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_image(data))
    {
        log::warn!("couldn't copy an image to the clipboard: {error}");
    }
}

/// The clipboard's plain text, if any: the system's, or the stand-in's.
pub(crate) fn read_text(ctx: &egui::Context) -> Option<String> {
    let text = match ctx.data(|data| data.get_temp::<Captured>(id())) {
        Some(captured) => captured
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .text
            .clone(),
        None => arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.get_text())
            .ok(),
    };
    text.filter(|text| !text.is_empty())
}

/// The files on the clipboard (copied in Finder), if that's what it holds; never the stand-in's.
pub(crate) fn read_files(ctx: &egui::Context) -> Vec<std::path::PathBuf> {
    if ctx.data(|data| data.get_temp::<Captured>(id())).is_some() {
        return Vec::new();
    }
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.get().file_list())
        .unwrap_or_default()
}

/// The image on the clipboard, as a PNG: a screenshot taken to the clipboard, or an image copied
/// from a page or Preview. The stand-in's, once [`capture`] installed one, is what
/// [`capture_image`] put there.
pub(crate) fn read_image(ctx: &egui::Context) -> Option<Vec<u8>> {
    if ctx.data(|data| data.get_temp::<Captured>(id())).is_some() {
        return ctx
            .data(|data| data.get_temp::<Arc<Vec<u8>>>(image_id()))
            .map(|png| png.to_vec());
    }
    system_image()
}

/// Puts the PNG `png` on the stand-in clipboard [`capture`] installs (and installs it), for
/// screenshots and tests that paste an image.
pub fn capture_image(ctx: &egui::Context, png: Vec<u8>) {
    if ctx.data(|data| data.get_temp::<Captured>(id())).is_none() {
        capture(ctx);
    }
    ctx.data_mut(|data| data.insert_temp(image_id(), Arc::new(png)));
}

/// Puts `json` on the stand-in clipboard [`capture`] installs as Copy Attachments' files, for
/// tests that paste another account's.
pub fn capture_attachments(ctx: &egui::Context, json: &str) {
    if ctx
        .data(|data| data.get_temp::<CapturedAttachments>(attachments_id()))
        .is_none()
    {
        capture(ctx);
    }
    write_attachments(ctx, json);
}

fn image_id() -> egui::Id {
    id().with("image")
}

/// The general pasteboard's PNG, or its TIFF made a PNG.
#[cfg(target_os = "macos")]
fn system_image() -> Option<Vec<u8>> {
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard, NSPasteboardTypePNG,
        NSPasteboardTypeTIFF,
    };
    use objc2_foundation::NSDictionary;

    let pasteboard = NSPasteboard::generalPasteboard();
    // SAFETY: AppKit's pasteboard type constants, read only.
    let (png, tiff) = unsafe { (NSPasteboardTypePNG, NSPasteboardTypeTIFF) };
    if let Some(data) = pasteboard.dataForType(png) {
        return Some(data.to_vec());
    }
    let tiff = pasteboard.dataForType(tiff)?;
    let image = NSBitmapImageRep::imageRepWithData(&tiff)?;
    // SAFETY: no properties is a valid set of them.
    let data = unsafe {
        image.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }?;
    Some(data.to_vec())
}

#[cfg(not(target_os = "macos"))]
fn system_image() -> Option<Vec<u8>> {
    None
}

/// The pasteboard type Copy Attachments writes: JSON naming the files and their account.
#[cfg(target_os = "macos")]
const ATTACHMENTS: &str = "build.miguel.exo.attachments";

fn attachments_id() -> egui::Id {
    id().with("attachments")
}

type CapturedAttachments = Arc<Mutex<Option<String>>>;

/// Puts `json` (Copy Attachments' files) on the clipboard as all it holds: the system's, or
/// the stand-in [`capture`] installed.
pub(crate) fn write_attachments(ctx: &egui::Context, json: &str) {
    if let Some(captured) = ctx.data(|data| data.get_temp::<CapturedAttachments>(attachments_id()))
    {
        *captured
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(json.to_owned());
        if let Some(clipboard) = ctx.data(|data| data.get_temp::<Captured>(id())) {
            *clipboard
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = MemoryClipboard::default();
        }
        return;
    }
    system_write_attachments(json);
}

/// The files Copy Attachments put on the clipboard, if it holds them: the system's, or the
/// stand-in's.
pub(crate) fn read_attachments(ctx: &egui::Context) -> Option<String> {
    match ctx.data(|data| data.get_temp::<CapturedAttachments>(attachments_id())) {
        Some(captured) => captured
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone(),
        None => system_read_attachments(),
    }
}

/// The system clipboard holds Copy Attachments' files (and so no text: ⌘V still pastes them).
pub(crate) fn holds_attachments() -> bool {
    system_read_attachments().is_some()
}

#[cfg(target_os = "macos")]
fn system_write_attachments(json: &str) {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::NSString;
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    if !pasteboard.setString_forType(&NSString::from_str(json), &NSString::from_str(ATTACHMENTS)) {
        log::warn!("couldn't put the attachments on the clipboard");
    }
}

#[cfg(target_os = "macos")]
fn system_read_attachments() -> Option<String> {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::NSString;
    NSPasteboard::generalPasteboard()
        .stringForType(&NSString::from_str(ATTACHMENTS))
        .map(|json| json.to_string())
}

#[cfg(not(target_os = "macos"))]
fn system_write_attachments(_json: &str) {}

#[cfg(not(target_os = "macos"))]
fn system_read_attachments() -> Option<String> {
    None
}

/// Sends `ctx`'s HTML copies to memory from now on, readable with [`captured`]. For screenshots
/// and tests.
pub fn capture(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        data.insert_temp(id(), Captured::default());
        data.insert_temp(attachments_id(), CapturedAttachments::default());
    });
}

/// The text and search of the last copy with a search in `ctx` since [`capture`]
/// ([`write_with_search`]).
pub fn captured_search(ctx: &egui::Context) -> Option<(String, String)> {
    let captured = ctx.data(|data| data.get_temp::<Captured>(id()))?;
    let text = captured
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .text
        .clone()?;
    Some((text, ctx.data(|data| data.get_temp::<String>(search_id()))?))
}

/// The HTML and text of the last HTML copy in `ctx` since [`capture`].
pub fn captured(ctx: &egui::Context) -> Option<(String, String)> {
    let captured = ctx.data(|data| data.get_temp::<Captured>(id()))?;
    let clipboard = captured
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Some((clipboard.html.clone()?, clipboard.text.clone()?))
}
