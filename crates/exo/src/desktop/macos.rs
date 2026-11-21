//! The AppKit side: how Exo was launched, a click on the Dock icon, asking before quitting, the
//! Dock badge, the window's full screen and animations, and the updater's dialogs.

use std::sync::atomic::{AtomicBool, Ordering};

use objc2::runtime::AnyObject;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained, sel,
};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApplication, NSControlStateValueOn,
    NSWindowAnimationBehavior, NSWindowDidExitFullScreenNotification, NSWindowStyleMask,
};
use objc2_foundation::{
    NSAppleEventDescriptor, NSAppleEventManager, NSNotification, NSNotificationCenter, NSObject,
    NSObjectProtocol, NSString, NSUserDefaults, ns_string,
};

use super::Launcher;
use super::quit::{self, QuitWarning};

/// The class of the view egui draws in, winit's, which has the keyboard in Exo's window.
pub(super) const WINIT_VIEW: &std::ffi::CStr = c"WinitView";

/// `kCoreEventClass`.
const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
/// `kAEOpenApplication`: the event an app is launched with, when not to open something.
const OPEN_APPLICATION: u32 = u32::from_be_bytes(*b"oapp");
/// `kAEReopenApplication`: the Dock icon was clicked, or Exo opened again, while it runs.
const REOPEN_APPLICATION: u32 = u32::from_be_bytes(*b"rapp");
/// `keyAEPropData`: the launch event's property saying why the app was launched.
const PROPERTY_DATA: u32 = u32::from_be_bytes(*b"prdt");
/// `keyAELaunchedAsLogInItem`: that property for a login item.
const LAUNCHED_AS_LOGIN_ITEM: u32 = u32::from_be_bytes(*b"lgit");
/// `kAEQuitApplication`: the Dock's Quit, Activity Monitor's, AppleScript's, and macOS's as it
/// ends the session.
const QUIT_APPLICATION: u32 = u32::from_be_bytes(*b"quit");
/// `kAEQuitReason`: why macOS sent a quit event (`kAEReallyLogOut`, `kAERestart`, `kAEShutDown`,
/// `kAEQuitAll`), there only when macOS sent it.
const QUIT_REASON: u32 = u32::from_be_bytes(*b"why?");

/// Whether macOS launched Exo as a login item (Open at Login): the Apple event being handled is
/// the open-application one, saying so. Only while Exo finishes launching, when AppKit is
/// handling that event; `false` off the main thread.
pub(super) fn launched_at_login() -> bool {
    if MainThreadMarker::new().is_none() {
        return false;
    }
    let Some(event) = NSAppleEventManager::sharedAppleEventManager().currentAppleEvent() else {
        return false;
    };
    // SAFETY: `eventClass` and `eventID` return the event's `AEEventClass` and `AEEventID`
    // (`UInt32`), `paramDescriptorForKeyword:` takes an `AEKeyword` and returns a descriptor or
    // nil. Through `msg_send!`: objc2 types them only with objc2-core-services.
    let (class, id, property): (u32, u32, Option<Retained<NSAppleEventDescriptor>>) = unsafe {
        (
            msg_send![&event, eventClass],
            msg_send![&event, eventID],
            msg_send![&event, paramDescriptorForKeyword: PROPERTY_DATA],
        )
    };
    is_login_launch(class, id, property.map(|property| property.enumCodeValue()))
}

/// Whether an Apple event of `class` and `id`, with `property` as its `keyAEPropData`, launched
/// the app as a login item.
fn is_login_launch(class: u32, id: u32, property: Option<u32>) -> bool {
    (class, id, property)
        == (
            CORE_EVENT_CLASS,
            OPEN_APPLICATION,
            Some(LAUNCHED_AS_LOGIN_ITEM),
        )
}

/// Turns AppKit's animations for ordering Exo's window in and out off, or back to the ones AppKit
/// picks (`animationBehavior`); minimizing has its own, either way. Off the main thread, nothing.
pub(super) fn set_animations(on: bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let behavior = if on {
        NSWindowAnimationBehavior::Default
    } else {
        NSWindowAnimationBehavior::None
    };
    for window in NSApplication::sharedApplication(mtm).windows().iter() {
        window.setAnimationBehavior(behavior);
    }
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `Reopen` doesn't implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ExoReopenHandler"]
    #[ivars = Launcher]
    struct Reopen;

    unsafe impl NSObjectProtocol for Reopen {}

    impl Reopen {
        #[unsafe(method(handleReopen:withReplyEvent:))]
        fn handle_reopen(&self, _event: &NSObject, _reply: &NSObject) {
            self.ivars().show();
        }
    }
);

/// Brings the window back when the Dock icon is clicked or Exo is opened again while it runs.
/// winit owns the app's delegate, so this answers the reopen event itself, in place of AppKit's
/// handler, which only took a minimized window out of the Dock (bringing the window back does
/// that too).
pub(super) fn on_reopen(launcher: Launcher) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let this = Reopen::alloc(mtm).set_ivars(launcher);
    // SAFETY: `NSObject`'s designated initializer.
    let handler: Retained<Reopen> = unsafe { msg_send![super(this), init] };
    // SAFETY: `handleReopen:withReplyEvent:` takes the event and its reply, as an Apple event
    // handler's selector must; the class and ID are `AEEventClass` and `AEEventID` (`UInt32`).
    unsafe {
        let _: () = msg_send![
            &NSAppleEventManager::sharedAppleEventManager(),
            setEventHandler: &*handler,
            andSelector: sel!(handleReopen:withReplyEvent:),
            forEventClass: CORE_EVENT_CLASS,
            andEventID: REOPEN_APPLICATION,
        ];
    }
    // The manager doesn't retain its handler, which answers for as long as Exo runs.
    std::mem::forget(handler);
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `Quit` doesn't implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ExoQuit"]
    #[ivars = QuitWarning]
    struct Quit;

    unsafe impl NSObjectProtocol for Quit {}

    impl Quit {
        /// ⌘Q and the app menu's `Quit Exo`, which AppKit's `terminate:` answered.
        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            self.terminate_if_confirmed();
        }

        /// A quit Apple event. macOS ending the session says why, and quits at once.
        #[unsafe(method(handleQuit:withReplyEvent:))]
        fn handle_quit(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            if is_session_end(event) {
                NSApplication::sharedApplication(self.mtm()).terminate(None);
            } else {
                self.terminate_if_confirmed();
            }
        }
    }
);

impl Quit {
    /// Quits once the user confirms, or at once with `Warn Before Quitting` off.
    fn terminate_if_confirmed(&self) {
        let app = NSApplication::sharedApplication(self.mtm());
        if !self.ivars().is_on() || confirm_quit(&app, self.ivars()) {
            app.terminate(None);
        }
    }
}

/// Whether quit event `event` comes from macOS ending the session: it gives a reason only then.
/// `AERegistry.h` calls the reason a parameter, and apps read it as an attribute; either counts.
fn is_session_end(event: &NSAppleEventDescriptor) -> bool {
    // SAFETY: both take an `AEKeyword` and return a descriptor or nil. Through `msg_send!`: objc2
    // types them only with objc2-core-services.
    let (attribute, parameter): (
        Option<Retained<NSAppleEventDescriptor>>,
        Option<Retained<NSAppleEventDescriptor>>,
    ) = unsafe {
        (
            msg_send![event, attributeDescriptorForKeyword: QUIT_REASON],
            msg_send![event, paramDescriptorForKeyword: QUIT_REASON],
        )
    };
    attribute.is_some() || parameter.is_some()
}

/// Asks before quitting: a native alert, in front of whatever app is (the Dock's Quit leaves
/// another app active). Whether to quit; quitting with `Do not ask again` ticked turns `warning`
/// off. Cancel, Escape included, leaves everything as it was, whatever the checkbox says.
fn confirm_quit(app: &NSApplication, warning: &QuitWarning) -> bool {
    // `activate` is macOS 14's, and Exo runs on 13 (`LSMinimumSystemVersion`).
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    let alert = NSAlert::new(app.mtm());
    alert.setAlertStyle(NSAlertStyle::Warning);
    alert.setMessageText(&NSString::from_str(quit::MESSAGE));
    alert.setInformativeText(&NSString::from_str(quit::INFORMATIVE_TEXT));
    // NSAlert gives the first button Return, and one titled `Cancel` Escape.
    alert.addButtonWithTitle(&NSString::from_str(quit::QUIT));
    alert.addButtonWithTitle(&NSString::from_str(quit::CANCEL));
    // macOS titles it (`Do not ask again`), in the user's language.
    alert.setShowsSuppressionButton(true);
    let quit = alert.runModal() == NSAlertFirstButtonReturn;
    warning.answered(
        quit,
        alert
            .suppressionButton()
            .is_some_and(|checkbox| checkbox.state() == NSControlStateValueOn),
    )
}

/// Asks before quitting from now on, while `warning` is on (see [`quit`]): the app menu's `Quit
/// Exo` (⌘Q) and quit Apple events come here rather than to AppKit, which would quit at once.
/// `terminate:` stays AppKit's, for Exo to quit by itself without asking. Call once the menu bar
/// is up.
pub(super) fn confirm_quit_from(warning: QuitWarning) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let this = Quit::alloc(mtm).set_ivars(warning);
    // SAFETY: `NSObject`'s designated initializer.
    let handler: Retained<Quit> = unsafe { msg_send![super(this), init] };
    let app_menu = NSApplication::sharedApplication(mtm)
        .mainMenu()
        .and_then(|bar| bar.itemAtIndex(0))
        .and_then(|item| item.submenu());
    for item in app_menu.iter().flat_map(|menu| menu.itemArray()) {
        if item.action() == Some(sel!(terminate:)) {
            // SAFETY: the action is the handler's, taking the sender; the handler outlives the item.
            unsafe {
                item.setAction(Some(sel!(quit:)));
                item.setTarget(Some(&handler));
            }
        }
    }
    // SAFETY: as `on_reopen`'s: the selector takes the event and its reply, and the class and ID
    // are `UInt32`s. This replaces AppKit's handler, which calls `terminate:`.
    unsafe {
        let _: () = msg_send![
            &NSAppleEventManager::sharedAppleEventManager(),
            setEventHandler: &*handler,
            andSelector: sel!(handleQuit:withReplyEvent:),
            forEventClass: CORE_EVENT_CLASS,
            andEventID: QUIT_APPLICATION,
        ];
    }
    // Neither the menu item nor the Apple Event manager retains it; it answers for as long as Exo
    // runs.
    std::mem::forget(handler);
}

/// `The background process has crashed repeatedly.`, a modal critical alert with `gave_up`'s
/// attempts. Its second step names Clear Cache and Restart, which keeps what Gmail doesn't have.
/// Then Exo quits once the frame showing it is over, without asking.
pub(super) fn crashed_repeatedly(gave_up: crate::recovery::GaveUp) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // In front: the window may be closed, or another app active. `activate` is macOS 14's.
    #[allow(deprecated)]
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    let alert = NSAlert::new(mtm);
    // An error.
    alert.setAlertStyle(NSAlertStyle::Critical);
    alert
        .window()
        .setTitle(ns_string!("Exo Background Process Error"));
    alert.setMessageText(ns_string!("The background process has crashed repeatedly."));
    let detail = format!(
        "Exo needs to quit. Please try:\n\n1. Restart Exo\n2. If the problem persists, choose Help > Clear Cache and Restart\n\nError: {} ({} attempts)",
        crate::recovery::GaveUp::KIND,
        gave_up.attempts
    );
    alert.setInformativeText(&NSString::from_str(&detail));
    alert.addButtonWithTitle(ns_string!("Quit Exo"));
    alert.runModal();
    terminate_soon();
}

/// Quits through AppKit's `terminate:`, without asking, once the main thread is done with what
/// it's doing: not from inside a frame, which AppKit's `applicationWillTerminate:` would run
/// again.
pub(super) fn terminate_soon() {
    dispatch2::DispatchQueue::main().exec_async(|| {
        if let Some(mtm) = MainThreadMarker::new() {
            NSApplication::sharedApplication(mtm).terminate(None);
        }
    });
}

/// A modal alert, on the main thread, with Exo in front: see [`super::ask`].
pub(super) fn ask(message: &str, detail: &str, buttons: &[&str]) -> Option<usize> {
    let (message, detail) = (message.to_owned(), detail.to_owned());
    let buttons: Vec<String> = buttons.iter().map(|&button| button.to_owned()).collect();
    let (answer, answered) = std::sync::mpsc::channel();
    dispatch2::DispatchQueue::main().exec_async(move || {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        // In front: the window may be closed, or another app active. `activate` is macOS 14's.
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(&message));
        alert.setInformativeText(&NSString::from_str(&detail));
        for button in &buttons {
            alert.addButtonWithTitle(&NSString::from_str(button));
        }
        let chosen = alert.runModal() - NSAlertFirstButtonReturn;
        let _ = answer.send(usize::try_from(chosen).ok());
    });
    answered.recv().ok().flatten()
}

/// What the observer tells the window.
struct Exit {
    /// Woken when the window is out.
    ctx: egui::Context,
    exited: AtomicBool,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `ExitObserver` doesn't implement
    // `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ExoFullScreenExitObserver"]
    #[ivars = Exit]
    struct ExitObserver;

    unsafe impl NSObjectProtocol for ExitObserver {}

    impl ExitObserver {
        #[unsafe(method(windowDidExitFullScreen:))]
        fn window_did_exit_full_screen(&self, _notification: &NSNotification) {
            self.ivars().exited.store(true, Ordering::Relaxed);
            self.ivars().ctx.request_repaint();
        }
    }
);

/// Hears a window finish leaving full screen (`windowDidExitFullScreen:`), which winit doesn't pass
/// on: it says the window is out as soon as it's asked to leave, and hiding it mid-animation has
/// AppKit put it back on screen at the end.
pub(super) struct FullScreenExits(Retained<ExitObserver>);

impl FullScreenExits {
    /// Listens until dropped, waking `ctx` at each exit. `None` off the main thread, where
    /// AppKit can't be asked.
    pub(super) fn listen(ctx: &egui::Context) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let this = ExitObserver::alloc(mtm).set_ivars(Exit {
            ctx: ctx.clone(),
            exited: AtomicBool::new(false),
        });
        // SAFETY: `NSObject`'s designated initializer.
        let observer: Retained<ExitObserver> = unsafe { msg_send![super(this), init] };
        // SAFETY: `windowDidExitFullScreen:` takes the notification, as an observer's selector
        // must, and the observer is removed before it's freed (`Drop`).
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                &observer,
                sel!(windowDidExitFullScreen:),
                Some(NSWindowDidExitFullScreenNotification),
                None,
            );
        }
        Some(Self(observer))
    }

    /// Whether a window has finished leaving full screen since the last call.
    pub(super) fn take(&self) -> bool {
        self.0.ivars().exited.swap(false, Ordering::Relaxed)
    }
}

impl Drop for FullScreenExits {
    fn drop(&mut self) {
        // SAFETY: the observer was added in `listen`.
        unsafe { NSNotificationCenter::defaultCenter().removeObserver(&self.0) };
    }
}

/// macOS's accent colour is Graphite: the `AppleAccentColor` default is -1 (0 when unset).
pub(super) fn graphite_accent() -> bool {
    NSUserDefaults::standardUserDefaults().integerForKey(ns_string!("AppleAccentColor")) == -1
}

/// The Mac's model (`hw.model`, `MacBookPro15,2`), if the kernel says.
pub(super) fn model() -> Option<String> {
    let name = c"hw.model";
    let mut size = 0;
    // SAFETY: a null buffer asks for the value's size, which `size` receives.
    if unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    let mut value = vec![0u8; size];
    // SAFETY: `value` holds `size` bytes, which `size` says.
    if unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            value.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    std::ffi::CStr::from_bytes_until_nul(&value)
        .ok()?
        .to_str()
        .ok()
        .map(str::to_owned)
}

/// Puts `label` on the Dock icon; empty, none.
pub(super) fn set_badge(label: &str) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let label = (!label.is_empty()).then(|| NSString::from_str(label));
    NSApplication::sharedApplication(mtm)
        .dockTile()
        .setBadgeLabel(label.as_deref());
}

/// Opens `url` in its app, which stays behind Exo. `false` off the main thread, or for a URL
/// `NSURL` won't take.
pub(super) fn open_in_background(url: &str) -> bool {
    use objc2_app_kit::{NSWorkspace, NSWorkspaceOpenConfiguration};
    if MainThreadMarker::new().is_none() {
        return false;
    }
    let Some(url) = objc2_foundation::NSURL::URLWithString(&NSString::from_str(url)) else {
        return false;
    };
    let configuration = NSWorkspaceOpenConfiguration::configuration();
    configuration.setActivates(false);
    NSWorkspace::sharedWorkspace().openURL_configuration_completionHandler(
        &url,
        &configuration,
        None,
    );
    true
}

/// Whether Exo's window is on screen: shown, and not in the Dock. `None` off the main thread,
/// where AppKit can't be asked.
pub(super) fn on_screen() -> Option<bool> {
    let mtm = MainThreadMarker::new()?;
    let windows = NSApplication::sharedApplication(mtm).windows();
    Some(
        windows
            .iter()
            .any(|window| window.isVisible() && !window.isMiniaturized()),
    )
}

/// Whether Exo's window is in full screen, as AppKit has it. `None` off the main thread, where
/// AppKit can't be asked.
pub(super) fn in_full_screen() -> Option<bool> {
    let mtm = MainThreadMarker::new()?;
    let windows = NSApplication::sharedApplication(mtm).windows();
    Some(
        windows
            .iter()
            .any(|window| window.styleMask().contains(NSWindowStyleMask::FullScreen)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_login_items_open_event_is_a_login_launch() {
        assert!(is_login_launch(
            CORE_EVENT_CLASS,
            OPEN_APPLICATION,
            Some(LAUNCHED_AS_LOGIN_ITEM)
        ));
        // Opened from the Finder, the Dock or `open`.
        assert!(!is_login_launch(CORE_EVENT_CLASS, OPEN_APPLICATION, None));
        // `keyAELaunchedAsServiceItem`.
        assert!(!is_login_launch(
            CORE_EVENT_CLASS,
            OPEN_APPLICATION,
            Some(u32::from_be_bytes(*b"svit"))
        ));
        // Launched for a `mailto:` link (`kAEGetURL`), or a Dock click while running.
        let get_url = u32::from_be_bytes(*b"GURL");
        assert!(!is_login_launch(get_url, get_url, None));
        assert!(!is_login_launch(
            CORE_EVENT_CLASS,
            REOPEN_APPLICATION,
            Some(LAUNCHED_AS_LOGIN_ITEM)
        ));
    }

    /// A quit Apple event, with `reason` as an attribute or a parameter.
    fn quit_event(reason: Option<(&[u8; 4], bool)>) -> Retained<NSAppleEventDescriptor> {
        let class = <NSAppleEventDescriptor as objc2::ClassType>::class();
        // SAFETY: Foundation's constructors and setters, through `msg_send!` as above:
        // `kAutoGenerateReturnID` (an `AEReturnID`) and `kAnyTransactionID` (an
        // `AETransactionID`), no target, and descriptors for `AEKeyword`s.
        unsafe {
            let event: Retained<NSAppleEventDescriptor> = msg_send![class, appleEventWithEventClass: CORE_EVENT_CLASS, eventID: QUIT_APPLICATION, targetDescriptor: None::<&NSAppleEventDescriptor>, returnID: -1_i16, transactionID: 0_i32];
            if let Some((reason, attribute)) = reason {
                let value: Retained<NSAppleEventDescriptor> =
                    msg_send![class, descriptorWithTypeCode: u32::from_be_bytes(*reason)];
                if attribute {
                    let _: () =
                        msg_send![&event, setAttributeDescriptor: &*value, forKeyword: QUIT_REASON];
                } else {
                    let _: () =
                        msg_send![&event, setParamDescriptor: &*value, forKeyword: QUIT_REASON];
                }
            }
            event
        }
    }

    #[test]
    fn only_macos_ending_the_session_quits_without_asking() {
        for reason in [b"rlgo", b"rest", b"shut", b"quia"] {
            assert!(
                is_session_end(&quit_event(Some((reason, true)))),
                "{}",
                String::from_utf8_lossy(reason)
            );
        }
        assert!(
            is_session_end(&quit_event(Some((b"rlgo", false)))),
            "as a parameter"
        );
        // The Dock's Quit, Activity Monitor's, AppleScript's.
        assert!(!is_session_end(&quit_event(None)));
    }
}
