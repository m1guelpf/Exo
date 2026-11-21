//! The Quick Look panel: the shared `QLPreviewPanel` showing one item, the file, under the title it
//! was given.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained, runtime::ProtocolObject,
};
use objc2_app_kit::{NSApplication, NSResponder};
use objc2_foundation::{NSInteger, NSObject, NSObjectProtocol, NSString, NSURL};
use objc2_quick_look_ui::{QLPreviewItem, QLPreviewPanel, QLPreviewPanelDataSource};

/// Showing a different file waits this long after closing the panel.
const REOPEN_DELAY: Duration = Duration::from_millis(200);

/// What the panel shows.
struct Item {
    url: Retained<NSURL>,
    title: Retained<NSString>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `PreviewItem` doesn't implement
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "ExoPreviewItem"]
    #[ivars = Item]
    struct PreviewItem;

    unsafe impl NSObjectProtocol for PreviewItem {}

    unsafe impl QLPreviewItem for PreviewItem {
        #[unsafe(method_id(previewItemURL))]
        fn preview_item_url(&self) -> Option<Retained<NSURL>> {
            Some(self.ivars().url.clone())
        }

        #[unsafe(method_id(previewItemTitle))]
        fn preview_item_title(&self) -> Option<Retained<NSString>> {
            Some(self.ivars().title.clone())
        }
    }
);

impl PreviewItem {
    fn new(item: Item) -> Retained<Self> {
        let this = Self::alloc().set_ivars(item);
        // SAFETY: `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

define_class!(
    // SAFETY: `NSResponder` has no subclassing requirements, and `Controller` doesn't implement
    // `Drop`.
    #[unsafe(super(NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[name = "ExoQuickLookController"]
    #[ivars = RefCell<Option<Retained<PreviewItem>>>]
    struct Controller;

    // `QLPreviewPanelController`: the panel takes the first object in the responder chain that
    // accepts it, and hands itself over while it's open.
    impl Controller {
        #[unsafe(method(acceptsPreviewPanelControl:))]
        fn accepts_preview_panel_control(&self, _panel: Option<&QLPreviewPanel>) -> bool {
            true
        }

        #[unsafe(method(beginPreviewPanelControl:))]
        fn begin_preview_panel_control(&self, panel: Option<&QLPreviewPanel>) {
            if let Some(panel) = panel {
                // SAFETY: the panel doesn't retain its data source, and the controller is never
                // freed (see `QuickLook::controller`).
                unsafe { panel.setDataSource(Some(ProtocolObject::from_ref(self))) };
            }
        }

        #[unsafe(method(endPreviewPanelControl:))]
        fn end_preview_panel_control(&self, panel: Option<&QLPreviewPanel>) {
            if let Some(panel) = panel {
                // SAFETY: a panel without a data source shows nothing.
                unsafe { panel.setDataSource(None) };
            }
        }
    }

    unsafe impl NSObjectProtocol for Controller {}

    unsafe impl QLPreviewPanelDataSource for Controller {
        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        fn number_of_preview_items(&self, _panel: Option<&QLPreviewPanel>) -> NSInteger {
            NSInteger::from(self.ivars().borrow().is_some())
        }

        #[unsafe(method_id(previewPanel:previewItemAtIndex:))]
        fn preview_item(&self, _panel: Option<&QLPreviewPanel>, _index: NSInteger) -> Option<Retained<ProtocolObject<dyn QLPreviewItem>>> {
            self.ivars().borrow().clone().map(ProtocolObject::from_retained)
        }
    }
);

impl Controller {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RefCell::new(None));
        // SAFETY: `NSResponder`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// Quick Look for the app's window.
#[derive(Default)]
pub(super) struct QuickLook {
    /// The panel's controller, in the window's responder chain since the first preview.
    controller: Option<&'static Controller>,
    /// The file asked for last.
    last: Option<PathBuf>,
    /// A different file and its title, shown once the closed panel has had its pause.
    pending: Option<(PathBuf, String, Instant)>,
}

impl QuickLook {
    /// Previews `path` under `title`: at once when it's the file asked for last, else once the
    /// panel has been closed for [`REOPEN_DELAY`]. Returns how soon to [`QuickLook::poll`].
    pub(super) fn preview(
        &mut self,
        mtm: MainThreadMarker,
        path: PathBuf,
        title: String,
    ) -> Option<Duration> {
        if self.last.as_ref() == Some(&path) {
            self.pending = None;
            self.show(mtm, &path, &title);
        } else {
            self.close(mtm);
            self.pending = Some((path.clone(), title, Instant::now() + REOPEN_DELAY));
        }
        self.last = Some(path);
        self.poll(mtm)
    }

    /// Shows the pending file once its pause is over. Returns how long is left of it.
    pub(super) fn poll(&mut self, mtm: MainThreadMarker) -> Option<Duration> {
        let now = Instant::now();
        if let Some((path, title, _)) = self.pending.take_if(|(_, _, due)| *due <= now) {
            self.show(mtm, &path, &title);
        }
        self.pending.as_ref().map(|(_, _, due)| *due - now)
    }

    /// Opens the panel on `path` under `title`.
    fn show(&mut self, mtm: MainThreadMarker, path: &Path, title: &str) {
        let (Some(controller), Some(url)) = (self.controller(mtm), NSURL::from_file_path(path))
        else {
            return;
        };
        controller.ivars().replace(Some(PreviewItem::new(Item {
            url,
            title: NSString::from_str(title),
        })));
        // SAFETY: on the main thread, as `mtm` proves.
        if let Some(panel) = unsafe { QLPreviewPanel::sharedPreviewPanel(mtm) } {
            panel.makeKeyAndOrderFront(None);
        }
    }

    /// The panel's controller. winit owns the window and its delegate, so the first call puts a
    /// controller in the app window's responder chain right after the window, where AppKit puts
    /// an `NSWindowController`. While Exo isn't active there's no main window: its only window
    /// is the one, and the panel opens once Exo is back in front.
    fn controller(&mut self, mtm: MainThreadMarker) -> Option<&'static Controller> {
        if self.controller.is_none() {
            let app = NSApplication::sharedApplication(mtm);
            let Some(window) = app.mainWindow().or_else(|| app.windows().firstObject()) else {
                log::warn!("no window to preview attachments from");
                return None;
            };
            // Leaked: the responder chain doesn't retain it, and it must outlive the window.
            let controller: &'static Controller = Box::leak(Box::new(Controller::new(mtm)));
            // SAFETY: neither side retains its next responder. The controller is never freed, and
            // whatever followed the window is kept alive by its owner, as it was before.
            unsafe {
                controller.setNextResponder(window.nextResponder().as_deref());
                window.setNextResponder(Some(controller));
            }
            self.controller = Some(controller);
        }
        self.controller
    }

    /// Closes the panel and forgets its file, if anything was shown.
    fn close(&self, mtm: MainThreadMarker) {
        let Some(controller) = self.controller else {
            return;
        };
        // Asking for the shared panel when there's none would make one.
        // SAFETY: on the main thread, as `mtm` proves.
        let panel = unsafe {
            QLPreviewPanel::sharedPreviewPanelExists(mtm)
                .then(|| QLPreviewPanel::sharedPreviewPanel(mtm))
        };
        if let Some(panel) = panel.flatten() {
            panel.close();
        }
        controller.ivars().replace(None);
    }
}
