//! Text macOS inserts with no key behind it: Edit ▸ `Emoji & Symbols` (⌃⌘Space, or the globe
//! key's E), dictation, and the Character Viewer. They send `insertText:replacementRange:` to the
//! view that has the keyboard, which is winit's; winit keeps only the text that ends an input
//! method's composition (marked text), so the rest never reached Exo. Exo's view now hands it to
//! whatever has the keyboard, as typed text ([`super::Launcher::text`]), and leaves winit
//! everything a key press or a composition inserts, which reaches egui as it always did.
//!
//! The same `keyDown:` hands on ⌘V when the clipboard has no text (a screenshot, files copied in
//! Finder): egui-winit turns ⌘V into a paste of the clipboard's text and drops the key, so with no
//! text nothing reached egui, and compose never pasted the image ([`super::Launcher::keystroke`],
//! as Edit ▸ Paste does).

use std::cell::{Cell, OnceCell};
use std::sync::OnceLock;

use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{MainThreadMarker, msg_send, sel};
use objc2_app_kit::NSEvent;
use objc2_foundation::{NSAttributedString, NSNotFound, NSRange, NSString};

use super::Launcher;
use super::macos::WINIT_VIEW as VIEW_CLASS;

type InsertText = unsafe extern "C-unwind" fn(&AnyObject, Sel, &AnyObject, NSRange);
type KeyDown = unsafe extern "C-unwind" fn(&AnyObject, Sel, &NSEvent);

/// winit's own implementations, which Exo's call on.
static WINIT_INSERT_TEXT: OnceLock<InsertText> = OnceLock::new();
static WINIT_KEY_DOWN: OnceLock<KeyDown> = OnceLock::new();

thread_local! {
    /// Where the text goes.
    static LAUNCHER: OnceCell<Launcher> = const { OnceCell::new() };
    /// winit's view is handling a key press, whose text it reports itself.
    static IN_KEY_DOWN: Cell<bool> = const { Cell::new(false) };
}

/// Hands `launcher` the text macOS inserts with no key behind it, from now on. Call on the main
/// thread once the window is made, which is when winit's view class exists.
pub(super) fn listen(launcher: Launcher) {
    if MainThreadMarker::new().is_none() || LAUNCHER.with(|cell| cell.set(launcher).is_err()) {
        return;
    }
    let Some(class) = AnyClass::get(VIEW_CLASS) else {
        log::warn!("inserted text: no {VIEW_CLASS:?} class");
        return;
    };
    // SAFETY: each replacement has the signature of the method it replaces (`v@:@{_NSRange=QQ}`
    // and `v@:@`), and calls winit's implementation with the same arguments.
    unsafe {
        let insert_text: InsertText = insert_text;
        let key_down: KeyDown = key_down;
        if let Some(winit) = replace(
            class,
            sel!(insertText:replacementRange:),
            std::mem::transmute::<InsertText, Imp>(insert_text),
        ) {
            let _ = WINIT_INSERT_TEXT.set(std::mem::transmute::<Imp, InsertText>(winit));
        }
        if let Some(winit) = replace(
            class,
            sel!(keyDown:),
            std::mem::transmute::<KeyDown, Imp>(key_down),
        ) {
            let _ = WINIT_KEY_DOWN.set(std::mem::transmute::<Imp, KeyDown>(winit));
        }
    }
}

/// Points `class`'s own `sel` at `imp`, returning what it was. `None`, and nothing changed, when
/// the class doesn't define it itself (a superclass's is left alone).
///
/// # Safety
/// `imp` has the method's signature.
unsafe fn replace(class: &AnyClass, sel: Sel, imp: Imp) -> Option<Imp> {
    let methods = class.instance_methods();
    let Some(method) = methods.iter().find(|method| method.name() == sel) else {
        log::warn!("inserted text: {VIEW_CLASS:?} doesn't define {sel}");
        return None;
    };
    // SAFETY: the caller's.
    Some(unsafe { method.set_implementation(imp) })
}

/// winit's `keyDown:`, noting that a key press is being handled, then ⌘V's keystroke when egui
/// got nothing of it.
unsafe extern "C-unwind" fn key_down(view: &AnyObject, sel: Sel, event: &NSEvent) {
    let Some(winit) = WINIT_KEY_DOWN.get() else {
        return;
    };
    let outer = IN_KEY_DOWN.replace(true);
    // SAFETY: winit's implementation, with the arguments AppKit called this with.
    unsafe { winit(view, sel, event) };
    IN_KEY_DOWN.set(outer);
    if let Some(keystroke) = paste_without_text(event) {
        LAUNCHER.with(|cell| {
            if let Some(launcher) = cell.get() {
                launcher.keystroke(keystroke);
            }
        });
    }
}

/// ⌘V or ⇧⌘V, as egui-winit tells them (the key typing `v`, or the V key where the layout types
/// no Latin letter), while the clipboard holds no text, which egui-winit then hands on as nothing.
fn paste_without_text(event: &NSEvent) -> Option<super::menu::Keystroke> {
    use objc2_app_kit::NSEventModifierFlags;
    /// `kVK_ANSI_V`.
    const V: u16 = 9;
    let flags = event.modifierFlags();
    if !flags.contains(NSEventModifierFlags::Command)
        || flags.intersects(NSEventModifierFlags::Control | NSEventModifierFlags::Option)
    {
        return None;
    }
    let typed = event
        .charactersIgnoringModifiers()
        .map(|text| text.to_string())
        .unwrap_or_default();
    if !(typed.eq_ignore_ascii_case("v")
        || (!typed.chars().any(|ch| ch.is_ascii_alphabetic()) && event.keyCode() == V))
    {
        return None;
    }
    let text = arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.get_text())
        .unwrap_or_default();
    if !text.is_empty() {
        return None;
    }
    Some(if flags.contains(NSEventModifierFlags::Shift) {
        super::menu::Keystroke::PasteAndMatchStyle
    } else {
        super::menu::Keystroke::Paste
    })
}

/// winit's `insertText:replacementRange:`, except for text inserted with no key press behind
/// it, nothing marked and nothing to replace: that goes to whatever has the keyboard. A
/// replacement (the accent menu's) still goes to winit, which can't apply one either.
unsafe extern "C-unwind" fn insert_text(
    view: &AnyObject,
    sel: Sel,
    string: &AnyObject,
    replacement: NSRange,
) {
    // SAFETY: winit's view is an `NSTextInputClient`.
    let marked: bool = unsafe { msg_send![view, hasMarkedText] };
    if IN_KEY_DOWN.get() || marked || replacement.location != NSNotFound as usize {
        if let Some(winit) = WINIT_INSERT_TEXT.get() {
            // SAFETY: winit's implementation, with the arguments AppKit called this with.
            unsafe { winit(view, sel, string, replacement) };
        }
        return;
    }
    // `insertText:replacementRange:` takes an `NSString` or an `NSAttributedString`.
    let text = match string.downcast_ref::<NSAttributedString>() {
        Some(attributed) => attributed.string().to_string(),
        None => string
            .downcast_ref::<NSString>()
            .map(ToString::to_string)
            .unwrap_or_default(),
    };
    if !text.is_empty() {
        LAUNCHER.with(|cell| {
            if let Some(launcher) = cell.get() {
                launcher.text(text);
            }
        });
    }
}
