//! Exo as the Mac's email app: the `mailto:` links macOS hands it, claiming them on Exo.app's
//! first run, and the app menu's `Set as Default Email Client`.
//!
//! macOS hands a link over as a "get URL" Apple Event, to the running app or to the one it
//! launches for it. The handler goes in as the app is about to finish launching, Apple's advice:
//! the event that launched the app comes right after, and installed any later it would go
//! unhandled. Links go to the [`Launcher`], as those another launch passes on do.

use std::path::Path;

use crate::desktop::Launcher;

/// The file in the state folder recording what Exo has done once, on its first run.
const INSTALL_FILE: &str = "install.json";

/// What the file holds.
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Install {
    /// Exo made itself the default email app.
    set_as_default_client: bool,
}

/// Starts listening for the links macOS hands Exo, which go to `launcher`. Call on the main
/// thread before the event loop starts.
pub fn listen(launcher: Launcher) {
    #[cfg(target_os = "macos")]
    macos::listen(launcher);
    #[cfg(not(target_os = "macos"))]
    drop(launcher);
}

/// The app is up: the app menu offers `Set as Default Email Client`. Call on the main thread
/// once the window is open.
pub fn ready() {
    #[cfg(target_os = "macos")]
    macos::add_menu_item();
}

/// Makes Exo the default email app on Exo.app's first run: whatever app had `mailto:` links
/// then, and never again, even once another app takes them back. macOS may ask to confirm.
/// `state` is the state folder, which keeps the record (`install.json`). A bare binary (`cargo
/// run`) never claims, since macOS hands links to bundles only; don't call it for a run on
/// gmail-mock.
pub fn claim_on_first_run(state: &Path) {
    #[cfg(target_os = "macos")]
    let bundled = macos::is_bundled();
    #[cfg(not(target_os = "macos"))]
    let bundled = false;
    if first_run(state, bundled) {
        #[cfg(target_os = "macos")]
        macos::set_as_default();
    }
}

/// Whether this run is a `bundled` Exo's first, recording in `state` that it was before saying
/// so, so no later run is, whatever macOS then answers. A record that can't be read or written
/// counts as one: Exo changes the system's default only when it knows it hasn't before. A bare
/// binary records nothing, so Exo.app still claims on its first run.
fn first_run(state: &Path, bundled: bool) -> bool {
    if !bundled {
        return false;
    }
    let path = state.join(INSTALL_FILE);
    let mut install = match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice::<Install>(&bytes) {
            Ok(install) => install,
            Err(error) => {
                log::warn!("ignoring {}: {error}", path.display());
                return false;
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Install::default(),
        Err(error) => {
            log::warn!("reading {}: {error}", path.display());
            return false;
        }
    };
    if install.set_as_default_client {
        return false;
    }
    install.set_as_default_client = true;
    let contents = serde_json::to_vec_pretty(&install).expect("a flag makes JSON");
    if let Err(error) =
        std::fs::create_dir_all(state).and_then(|()| std::fs::write(&path, contents))
    {
        log::warn!("writing {}: {error}", path.display());
        return false;
    }
    true
}

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::OnceCell;

    use block2::RcBlock;
    use objc2::{
        DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
        runtime::AnyObject, runtime::ProtocolObject, sel,
    };
    use objc2_app_kit::{
        NSApplication, NSApplicationWillFinishLaunchingNotification, NSMenu, NSMenuDelegate,
        NSMenuItem, NSWorkspace,
    };
    use objc2_foundation::{
        NSAppleEventDescriptor, NSAppleEventManager, NSBundle, NSError, NSInteger, NSNotification,
        NSNotificationCenter, NSObject, NSObjectProtocol, NSString, NSURL, ns_string,
    };

    use super::Launcher;

    /// `kInternetEventClass` and `kAEGetURL`: both `'GURL'`.
    const GET_URL: u32 = u32::from_be_bytes(*b"GURL");
    /// `keyDirectObject`: the event's URL.
    const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
    /// The app menu's item.
    const SET_AS_DEFAULT: &str = "Set as Default Email Client";
    /// Where it goes: after the updater's item, which has a separator after it, or else after
    /// `About Exo` and its separator, with one of its own.
    const MENU_INDEX: NSInteger = 2;

    thread_local! {
        /// The handler, alive for good: the notification center, the Apple Event manager, the
        /// menu item and the menu all hold it without retaining it.
        static HANDLER: OnceCell<Retained<Handler>> = const { OnceCell::new() };
    }

    /// Where links go, and the menu items once added.
    struct State {
        launcher: Launcher,
        items: OnceCell<Vec<Retained<NSMenuItem>>>,
    }

    define_class!(
        // SAFETY: `NSObject` has no subclassing requirements, and `Handler` doesn't implement
        // `Drop`.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "ExoMailClient"]
        #[ivars = State]
        struct Handler;

        impl Handler {
            #[unsafe(method(applicationWillFinishLaunching:))]
            fn will_finish_launching(&self, _notification: &NSNotification) {
                let manager = NSAppleEventManager::sharedAppleEventManager();
                // SAFETY: the selector is this class's, taking the event and the reply.
                // `setEventHandler:andSelector:forEventClass:andEventID:` through `msg_send!`:
                // objc2 types its four-character codes only with objc2-core-services.
                let () = unsafe { msg_send![&manager, setEventHandler: self, andSelector: sel!(handleGetURLEvent:withReplyEvent:), forEventClass: GET_URL, andEventID: GET_URL] };
            }

            #[unsafe(method(handleGetURLEvent:withReplyEvent:))]
            fn handle_get_url(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
                // SAFETY: `paramDescriptorForKeyword:` takes an `AEKeyword` and returns a
                // descriptor or nil.
                let url: Option<Retained<NSAppleEventDescriptor>> = unsafe { msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT] };
                if let Some(url) = url.and_then(|url| url.stringValue()) {
                    log::debug!("macOS handed Exo a link");
                    self.ivars().launcher.open_url(&url.to_string());
                }
            }

            #[unsafe(method(setAsDefaultEmailClient:))]
            fn set_as_default_email_client(&self, _sender: Option<&AnyObject>) {
                set_as_default();
            }
        }

        unsafe impl NSObjectProtocol for Handler {}

        unsafe impl NSMenuDelegate for Handler {
            /// The item shows only while another app is the `mailto:` handler, worked out each
            /// time the menu opens.
            #[unsafe(method(menuNeedsUpdate:))]
            fn menu_needs_update(&self, _menu: &NSMenu) {
                if let Some(items) = self.ivars().items.get() {
                    let default = is_default();
                    for item in items {
                        item.setHidden(default);
                    }
                }
            }
        }
    );

    impl Handler {
        fn new(mtm: MainThreadMarker, launcher: Launcher) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(State {
                launcher,
                items: OnceCell::new(),
            });
            // SAFETY: `NSObject`'s designated initializer.
            unsafe { msg_send![super(this), init] }
        }
    }

    pub(super) fn listen(launcher: Launcher) {
        let Some(mtm) = MainThreadMarker::new() else {
            log::warn!("mailto: links need the main thread");
            return;
        };
        HANDLER.with(|cell| {
            let handler = cell.get_or_init(|| Handler::new(mtm, launcher));
            // SAFETY: the selector is this class's, taking the notification; the handler lives
            // for good, so the center never calls a freed observer.
            unsafe {
                NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                    handler,
                    sel!(applicationWillFinishLaunching:),
                    Some(NSApplicationWillFinishLaunchingNotification),
                    None,
                );
            }
        });
    }

    /// Makes Exo the app macOS opens `mailto:` links with.
    pub(super) fn set_as_default() {
        let done = RcBlock::new(|error: *mut NSError| {
            // SAFETY: the error is nil or valid for the block's call.
            if let Some(error) = unsafe { error.as_ref() } {
                log::warn!(
                    "making Exo the default email app: {}",
                    error.localizedDescription()
                );
            }
        });
        let app = NSBundle::mainBundle().bundleURL();
        NSWorkspace::sharedWorkspace()
            .setDefaultApplicationAtURL_toOpenURLsWithScheme_completionHandler(
                &app,
                ns_string!("mailto"),
                Some(&done),
            );
    }

    /// Exo runs as Exo.app. macOS sends links to bundles only: a bare binary can't be the
    /// default.
    pub(super) fn is_bundled() -> bool {
        NSBundle::mainBundle().bundleIdentifier().is_some()
    }

    pub(super) fn add_menu_item() {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        if !is_bundled() {
            return;
        }
        let Some(menu) = NSApplication::sharedApplication(mtm)
            .mainMenu()
            .and_then(|bar| bar.itemAtIndex(0))
            .and_then(|app| app.submenu())
        else {
            return;
        };
        HANDLER.with(|cell| {
            let Some(handler) = cell.get() else { return };
            if handler.ivars().items.get().is_some() {
                return;
            }
            // SAFETY: the action is this class's, taking the sender; the target outlives the
            // item.
            let item = unsafe {
                let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                    mtm.alloc(),
                    &NSString::from_str(SET_AS_DEFAULT),
                    Some(sel!(setAsDefaultEmailClient:)),
                    ns_string!(""),
                );
                item.setTarget(Some(handler));
                item
            };
            let updater = menu.indexOfItemWithTag(crate::desktop::menu::UPDATER_TAG);
            let items = if updater >= 0 {
                menu.insertItem_atIndex(&item, updater + 1);
                vec![item]
            } else {
                let separator = NSMenuItem::separatorItem(mtm);
                let index = MENU_INDEX.min(menu.numberOfItems());
                menu.insertItem_atIndex(&item, index);
                menu.insertItem_atIndex(&separator, index + 1);
                vec![item, separator]
            };
            menu.setDelegate(Some(ProtocolObject::from_ref(&**handler)));
            let _ = handler.ivars().items.set(items);
        });
    }

    /// Exo opens `mailto:` links: the app macOS picks for them has Exo's bundle identifier.
    fn is_default() -> bool {
        let ours = NSBundle::mainBundle().bundleIdentifier();
        let handler = NSURL::URLWithString(ns_string!("mailto:"))
            .and_then(|url| NSWorkspace::sharedWorkspace().URLForApplicationToOpenURL(&url));
        ours.is_some()
            && handler
                .and_then(|app| NSBundle::bundleWithURL(&app))
                .and_then(|app| app.bundleIdentifier())
                == ours
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("exo-install-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn exo_app_claims_mailto_on_its_first_run_only() {
        // The state folder isn't there yet on a first run.
        let dir = dir("first");
        assert!(first_run(&dir, true));
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join(INSTALL_FILE)).expect("recorded"))
                .expect("JSON");
        assert_eq!(saved, serde_json::json!({ "set_as_default_client": true }));
        assert!(!first_run(&dir, true), "never again");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_bare_binary_never_claims_and_leaves_the_first_run_to_exo_app() {
        let dir = dir("bare");
        assert!(!first_run(&dir, false));
        assert!(!dir.join(INSTALL_FILE).exists());
        assert!(first_run(&dir, true));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_record_that_cant_be_read_counts_as_a_claim() {
        let dir = dir("corrupt");
        std::fs::create_dir_all(&dir).expect("a folder");
        let corrupt = "{\"set_as_default_client\": ";
        std::fs::write(dir.join(INSTALL_FILE), corrupt).expect("written");
        assert!(!first_run(&dir, true));
        assert_eq!(
            std::fs::read_to_string(dir.join(INSTALL_FILE)).expect("still there"),
            corrupt,
            "left as it was"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
