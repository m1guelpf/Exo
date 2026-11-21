//! The window's side of the Mac's drag (`NSDraggingSession`), which egui-winit doesn't give: text
//! dragged over the window from another app, and an editor's selection dragged out of it. The
//! window's frame starts with what's over it and what was let go on it, and ends with where its
//! editors and fields take text and what they drag out ([`begin`], [`end`], through
//! [`exo_ui::text_drag`]); a drag of files is winit's as before (compose's attachments).
//!
//! A selection's drag carries it as HTML (with a UTF-8 `<meta>`) and as text; within Exo it may
//! move (a drop elsewhere in the window takes it out of its editor, ⌥ copies), outside it copies.
//! AppKit's drag loop takes the button's release, so once a drag Exo started is over, the window
//! hears the button go up ([`release`]).

use std::cell::RefCell;
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_os = "macos")]
mod macos;

/// A drag Exo started ended inside AppKit's own loop, which took the button's release: egui still
/// thinks it's held until it's told.
#[cfg(target_os = "macos")]
static RELEASED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Text let go without the Mac's drag ([`simulate_drop`]).
    static SIMULATED: RefCell<Option<exo_ui::text_drag::Dropped>> = const { RefCell::new(None) };
}

/// Lets `dropped` go on the next frame drawn without the Mac's drag, as it would: for tests, which
/// have none.
pub fn simulate_drop(dropped: exo_ui::text_drag::Dropped) {
    SIMULATED.with(|simulated| *simulated.borrow_mut() = Some(dropped));
}

/// The Mac's drag's part in the window's frame, as it starts: what's over the window, what was let
/// go on it, the end of a drag out of one of its editors.
pub(crate) fn begin(ctx: &egui::Context) {
    #[cfg(target_os = "macos")]
    if macos::begin(ctx) {
        return;
    }
    let dropped = SIMULATED.with(|simulated| simulated.borrow_mut().take());
    exo_ui::text_drag::begin(ctx, false, None, dropped, None);
}

/// As the frame ends: where text can be let go in the window, and a selection dragged out of it.
pub(crate) fn end(ctx: &egui::Context) {
    let report = exo_ui::text_drag::end(ctx);
    #[cfg(target_os = "macos")]
    macos::end(ctx, report);
    #[cfg(not(target_os = "macos"))]
    drop(report);
}

/// A drag Exo started (a selection's, a card's) ended: the window's next frame hears the button go
/// up, which AppKit's drag loop took ([`take_release`]).
#[cfg(target_os = "macos")]
pub(crate) fn release() {
    RELEASED.store(true, Ordering::SeqCst);
}

/// Whether a drag Exo started ended since the last call, so egui should see the button let go.
#[cfg(target_os = "macos")]
pub(crate) fn take_release() -> bool {
    RELEASED.swap(false, Ordering::SeqCst)
}
