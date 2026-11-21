//! Dragging a card out of the window: a native drag of a file promise, which Finder (or any app
//! that takes files) fulfils by asking for the file where it was dropped. Exo fetches the part (the
//! parts cache first) when the promise is called in.

use std::path::PathBuf;

use block2::RcBlock;
use exo_store::Attachment;
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained, runtime::ProtocolObject,
};
use objc2_app_kit::{
    NSApplication, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession,
    NSDraggingSource, NSEvent, NSEventModifierFlags, NSEventType, NSFilePromiseProvider,
    NSFilePromiseProviderDelegate, NSWorkspace,
};
use objc2_foundation::{
    NSArray, NSError, NSObject, NSObjectProtocol, NSPoint, NSProcessInfo, NSRect, NSSize, NSString,
    NSURL, ns_string,
};

use crate::account::Loader;

/// What a promise needs to keep: the file and how to fetch it.
pub(crate) struct Promise {
    attachment: Attachment,
    loader: Loader,
}

/// AppKit's completion handler, called from the fetch's thread.
struct Completion(RcBlock<dyn Fn(*mut NSError)>);

// SAFETY: a file promise's completion handler may be called from any thread once the file is
// written (`NSFilePromiseProviderDelegate`), and this one is called exactly once.
unsafe impl Send for Completion {}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `Source` doesn't implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ExoAttachmentDragSource"]
    #[ivars = Promise]
    pub(crate) struct Source;

    unsafe impl NSObjectProtocol for Source {}

    unsafe impl NSDraggingSource for Source {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn source_operation_mask(
            &self,
            _session: &NSDraggingSession,
            _context: NSDraggingContext,
        ) -> NSDragOperation {
            NSDragOperation::Copy
        }

        // AppKit's drag loop took the button's release: the window hears it now.
        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn ended(
            &self,
            _session: &NSDraggingSession,
            _point: NSPoint,
            _operation: NSDragOperation,
        ) {
            crate::desktop::drag::release();
        }
    }

    unsafe impl NSFilePromiseProviderDelegate for Source {
        #[unsafe(method_id(filePromiseProvider:fileNameForType:))]
        fn file_name(
            &self,
            _provider: &NSFilePromiseProvider,
            _file_type: &NSString,
        ) -> Retained<NSString> {
            NSString::from_str(&super::file_name(&self.ivars().attachment.filename))
        }

        // Called on the main queue (there's no `operationQueueForFilePromiseProvider:`): it
        // only starts the fetch, and the handler hears from its thread.
        #[unsafe(method(filePromiseProvider:writePromiseToURL:completionHandler:))]
        fn write(
            &self,
            _provider: &NSFilePromiseProvider,
            url: &NSURL,
            completion: &block2::DynBlock<dyn Fn(*mut NSError)>,
        ) {
            let completion = Completion(completion.copy());
            let Some(path) = url.to_file_path() else {
                fail(&completion);
                return;
            };
            let Promise { attachment, loader } = self.ivars();
            loader.attachment(attachment, move |bytes| {
                let written = bytes.ok().is_some_and(|bytes| write(&path, &bytes));
                if written {
                    completion.0.call((std::ptr::null_mut(),))
                } else {
                    fail(&completion)
                }
            });
        }
    }
);

/// Writes `bytes` to `path`, quarantined as a download. Logs why it couldn't.
fn write(path: &PathBuf, bytes: &[u8]) -> bool {
    match std::fs::write(path, bytes) {
        Ok(()) => {
            super::quarantine(path);
            true
        }
        Err(error) => {
            log::warn!("writing a dragged attachment: {error}");
            false
        }
    }
}

/// Tells AppKit the promise wasn't kept.
fn fail(completion: &Completion) {
    let error = NSError::new(-1, ns_string!("ExoAttachmentDrag"));
    completion.0.call((Retained::as_ptr(&error).cast_mut(),));
}

/// Starts dragging `attachment` from `at` (egui points in the window) and returns the drag's
/// source, which must live until the drop has been written, or `None` off the main thread or
/// without a window.
pub(crate) fn start(
    loader: &Loader,
    attachment: &Attachment,
    at: egui::Pos2,
    zoom: f32,
) -> Option<Retained<Source>> {
    let mtm = MainThreadMarker::new()?;
    let app = NSApplication::sharedApplication(mtm);
    let window = app.keyWindow().or_else(|| app.mainWindow())?;
    let view = window.contentView()?;
    let source = Source::alloc(mtm).set_ivars(Promise {
        attachment: attachment.clone(),
        loader: loader.clone(),
    });
    // SAFETY: `NSObject`'s designated initializer.
    let source: Retained<Source> = unsafe { msg_send![super(source), init] };
    let provider = NSFilePromiseProvider::initWithFileType_delegate(
        NSFilePromiseProvider::alloc(),
        ns_string!("public.data"),
        ProtocolObject::from_ref(&*source),
    );
    let item = NSDraggingItem::initWithPasteboardWriter(
        NSDraggingItem::alloc(),
        ProtocolObject::from_ref(&*provider),
    );
    // The content view's coordinates start at its bottom left, in points (egui's are zoomed).
    let height = view.frame().size.height;
    let point = NSPoint::new(f64::from(at.x * zoom), height - f64::from(at.y * zoom));
    let icon = NSWorkspace::sharedWorkspace()
        .iconForFile(&NSString::from_str(&super::file_name(&attachment.filename)));
    // SAFETY: an `NSImage` is what a dragging item's contents may be.
    unsafe {
        item.setDraggingFrame_contents(
            NSRect::new(
                NSPoint::new(point.x - 16.0, point.y - 16.0),
                NSSize::new(32.0, 32.0),
            ),
            Some(&icon),
        )
    };
    // AppKit wants the mouse event the drag comes from; egui has had it already, so a dragged
    // event at the pointer stands in for it.
    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        NSEventType::LeftMouseDragged,
        view.convertPoint_toView(point, None),
        NSEventModifierFlags::empty(),
        NSProcessInfo::processInfo().systemUptime(),
        window.windowNumber(),
        None,
        0,
        1,
        1.0,
    )?;
    view.beginDraggingSessionWithItems_event_source(
        &NSArray::from_retained_slice(&[item]),
        &event,
        ProtocolObject::from_ref(&*source),
    );
    Some(source)
}
