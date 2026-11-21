//! The menu bar: the app menu, File, Edit, View, Window and Help. Exo builds it as it finishes
//! launching, in place of winit's app menu, which is when AppKit adds its own items to an app's
//! Edit menu (Writing Tools, AutoFill, `Start Dictation…`, `Emoji & Symbols`).
//!
//! Most items stand for a keystroke ([`Keystroke`]): the tab ones, Print, the editing ones and the
//! zoom ones. Their keys reach Exo as any other keys do, so whatever has the keyboard handles them
//! (the editor, a field, the palette, the list) before the menu does. AppKit takes the keys of a
//! shortcut an item shows, even a disabled one, so these items show theirs only while their menu is
//! open. Clicked, an item hands its keystroke to Exo as egui-winit would have made it, for whatever
//! has the keyboard; it's enabled only while Exo's window has it. The other items are AppKit's (the
//! app menu's, Minimize, Bring All to Front) or the shell's (Reload, Toggle Full Screen, Clear
//! Cache and Restart), shortcuts and all.
//!
//! Edit's `Enable Autocorrect` is a checkbox, checked while autocorrect is on for the account on
//! screen; choosing it turns every account's the other way.
//!
//! Window's `Close Window` is AppKit's `performClose:`, showing its ⌘W only while its menu is
//! open, as the keystroke items do: File's `Close Tab` has ⌘W.
//!
//! The app menu's updater item, after `About Exo`, says where the updater is (`Check for
//! Updates…`, `Downloading Update…`, `Install Update and Restart`…) and does what it says when
//! it can be chosen. It's there only when Exo updates itself.
//!
//! There's no `New Window`: Exo is one window.

use egui::{Event, Key, Modifiers};

/// ⌘, as egui-winit reports it on the Mac.
const COMMAND: Modifiers = Modifiers {
    mac_cmd: cfg!(target_os = "macos"),
    command: true,
    ..Modifiers::NONE
};
const SHIFT_COMMAND: Modifiers = Modifiers {
    shift: true,
    ..COMMAND
};

/// A menu item that stands for a keystroke whatever has the keyboard handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keystroke {
    /// File ▸ `New Tab` (⌘T).
    NewTab,
    /// File ▸ `Close Window` (⇧⌘W).
    CloseWindow,
    /// File ▸ `Close Tab` (⌘W).
    CloseTab,
    /// File ▸ `Print` (⌘P), whatever prints the conversation on screen.
    Print,
    /// Edit ▸ `Undo` (⌘Z).
    Undo,
    /// Edit ▸ `Redo` (⇧⌘Z).
    Redo,
    /// Edit ▸ `Cut` (⌘X).
    Cut,
    /// Edit ▸ `Copy` (⌘C).
    Copy,
    /// Edit ▸ `Paste` (⌘V).
    Paste,
    /// Edit ▸ `Paste and Match Style` (⇧⌘V): a paste without the clipboard's formatting.
    PasteAndMatchStyle,
    /// Edit ▸ `Select All` (⌘A).
    SelectAll,
    /// View ▸ `Actual Size` (⌘0).
    ActualSize,
    /// View ▸ `Zoom In` (⌘+, which zooms as ⌘= does).
    ZoomIn,
    /// View ▸ `Zoom Out` (⌘-).
    ZoomOut,
    /// Window ▸ `Next Tab` (⇧⌘]).
    NextTab,
    /// Window ▸ `Previous Tab` (⇧⌘[).
    PreviousTab,
}

impl Keystroke {
    /// Every one, in the order of their menu items' tags.
    #[cfg(target_os = "macos")]
    const ALL: [Self; 16] = [
        Self::Print,
        Self::Undo,
        Self::Redo,
        Self::Cut,
        Self::Copy,
        Self::Paste,
        Self::PasteAndMatchStyle,
        Self::SelectAll,
        Self::ActualSize,
        Self::ZoomIn,
        Self::ZoomOut,
        Self::NewTab,
        Self::CloseWindow,
        Self::CloseTab,
        Self::NextTab,
        Self::PreviousTab,
    ];

    /// The keys it stands for, as egui has them.
    fn keys(self) -> (Modifiers, Key) {
        match self {
            Self::NewTab => (COMMAND, Key::T),
            Self::CloseWindow => (SHIFT_COMMAND, Key::W),
            Self::CloseTab => (COMMAND, Key::W),
            Self::NextTab => (SHIFT_COMMAND, Key::CloseBracket),
            Self::PreviousTab => (SHIFT_COMMAND, Key::OpenBracket),
            Self::Print => (COMMAND, Key::P),
            Self::Undo => (COMMAND, Key::Z),
            Self::Redo => (SHIFT_COMMAND, Key::Z),
            Self::Cut => (COMMAND, Key::X),
            Self::Copy => (COMMAND, Key::C),
            Self::Paste => (COMMAND, Key::V),
            Self::PasteAndMatchStyle => (SHIFT_COMMAND, Key::V),
            Self::SelectAll => (COMMAND, Key::A),
            Self::ActualSize => (COMMAND, Key::Num0),
            // ⌘+ needs ⇧ on most keyboards, so ⌘= zooms in without it.
            Self::ZoomIn => (COMMAND, Key::Equals),
            Self::ZoomOut => (COMMAND, Key::Minus),
        }
    }

    /// The modifiers held and the events egui-winit makes of the keys: ⌘X and ⌘C are a cut and a
    /// copy; ⌘V, with ⇧ or not, a paste of `clipboard`'s text; any other the key pressed and let
    /// go. So is ⌘V when the clipboard has no text (an image, files), where egui-winit would make
    /// nothing at all: compose pastes those itself.
    pub fn input(self, clipboard: impl FnOnce() -> Option<String>) -> (Modifiers, Vec<Event>) {
        let (modifiers, key) = self.keys();
        let pasted = || {
            clipboard()
                .map(|text| text.replace("\r\n", "\n"))
                .filter(|text| !text.is_empty())
                .map(Event::Paste)
        };
        let events = match key {
            Key::X => vec![Event::Cut],
            Key::C => vec![Event::Copy],
            Key::V if let Some(paste) = pasted() => vec![paste],
            _ => [true, false]
                .map(|pressed| Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed,
                    repeat: false,
                    modifiers,
                })
                .into(),
        };
        (modifiers, events)
    }
}

/// Builds the menu bar as Exo finishes launching, its items telling `launcher`. Call on the main
/// thread before the event loop starts, with winit's own menu bar turned off
/// ([`without_winit_menu`]).
pub fn install(launcher: super::Launcher) {
    #[cfg(target_os = "macos")]
    macos::install(launcher);
    #[cfg(not(target_os = "macos"))]
    drop(launcher);
}

/// The window has the keyboard from now on: the keystroke items send theirs to it. Call once the
/// window is made.
pub(super) fn window_ready() {
    #[cfg(target_os = "macos")]
    macos::window_ready();
}

/// The app menu's updater item's tag, which `Set as Default Email Client` goes after.
#[cfg(target_os = "macos")]
pub(crate) use macos::UPDATER_TAG;

/// Tells winit not to make its menu bar, so [`install`]'s stays.
pub fn without_winit_menu(builder: &mut eframe::EventLoopBuilder<eframe::UserEvent>) {
    #[cfg(target_os = "macos")]
    winit::platform::macos::EventLoopBuilderExtMacOS::with_default_menu(builder, false);
    #[cfg(not(target_os = "macos"))]
    let _ = builder;
}

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::OnceCell;

    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_app_kit::{
        NSAlert, NSAlertFirstButtonReturn, NSApplication,
        NSApplicationWillFinishLaunchingNotification, NSControlStateValueOff,
        NSControlStateValueOn, NSEventModifierFlags, NSMenu, NSMenuDelegate, NSMenuItem,
    };
    use objc2_foundation::{
        NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol, NSProcessInfo, NSString,
        ns_string,
    };

    use super::Keystroke;
    use crate::desktop::Launcher;
    use crate::desktop::macos::WINIT_VIEW as VIEW_CLASS;

    thread_local! {
        /// The menu bar's handler, alive for good: the notification center, the menus and their
        /// items hold it without retaining it.
        static HANDLER: OnceCell<Retained<Handler>> = const { OnceCell::new() };
    }

    /// The keystroke items' tags start here, clear of the ones AppKit's own items may carry.
    const TAG_BASE: isize = 0x4578_6F00;
    /// Edit ▸ `Enable Autocorrect`'s tag, past the keystrokes'.
    const AUTOCORRECT_TAG: isize = TAG_BASE + 0x80;
    /// Window ▸ `Close Window`'s tag, whose ⌘W shows only while the menu is open.
    const CLOSE_WINDOW_TAG: isize = TAG_BASE + 0x81;
    /// The app menu's updater item's tag.
    pub(crate) const UPDATER_TAG: isize = TAG_BASE + 0x82;

    impl Keystroke {
        /// Its item's title: the standard one for the editing and zoom items.
        fn title(self) -> &'static str {
            match self {
                Self::NewTab => "New Tab",
                Self::CloseWindow => "Close Window",
                Self::CloseTab => "Close Tab",
                Self::NextTab => "Next Tab",
                Self::PreviousTab => "Previous Tab",
                Self::Print => "Print",
                Self::Undo => "Undo",
                Self::Redo => "Redo",
                Self::Cut => "Cut",
                Self::Copy => "Copy",
                Self::Paste => "Paste",
                Self::PasteAndMatchStyle => "Paste and Match Style",
                Self::SelectAll => "Select All",
                Self::ActualSize => "Actual Size",
                Self::ZoomIn => "Zoom In",
                Self::ZoomOut => "Zoom Out",
            }
        }

        /// The shortcut its item shows while its menu is open.
        fn shortcut(self) -> (&'static str, NSEventModifierFlags) {
            let shift = NSEventModifierFlags::Shift | NSEventModifierFlags::Command;
            match self {
                Self::NewTab => ("t", NSEventModifierFlags::Command),
                Self::CloseWindow => ("w", shift),
                Self::CloseTab => ("w", NSEventModifierFlags::Command),
                Self::NextTab => ("]", shift),
                Self::PreviousTab => ("[", shift),
                Self::Print => ("p", NSEventModifierFlags::Command),
                Self::Undo => ("z", NSEventModifierFlags::Command),
                Self::Redo => ("z", shift),
                Self::Cut => ("x", NSEventModifierFlags::Command),
                Self::Copy => ("c", NSEventModifierFlags::Command),
                Self::Paste => ("v", NSEventModifierFlags::Command),
                Self::PasteAndMatchStyle => ("v", shift),
                Self::SelectAll => ("a", NSEventModifierFlags::Command),
                Self::ActualSize => ("0", NSEventModifierFlags::Command),
                // ⌘+.
                Self::ZoomIn => ("+", NSEventModifierFlags::Command),
                Self::ZoomOut => ("-", NSEventModifierFlags::Command),
            }
        }

        /// What its item sends whatever has the keyboard: AppKit's own actions for the edit ones,
        /// by which AppKit finds the Edit menu to add its items to.
        fn action(self) -> Sel {
            match self {
                Self::Undo => sel!(undo:),
                Self::Redo => sel!(redo:),
                Self::Cut => sel!(cut:),
                Self::Copy => sel!(copy:),
                Self::Paste => sel!(paste:),
                Self::PasteAndMatchStyle => sel!(pasteAndMatchStyle:),
                Self::SelectAll => sel!(selectAll:),
                Self::NewTab
                | Self::CloseWindow
                | Self::CloseTab
                | Self::NextTab
                | Self::PreviousTab
                | Self::Print
                | Self::ActualSize
                | Self::ZoomIn
                | Self::ZoomOut => sel!(exoKeystroke:),
            }
        }

        /// The tag its item carries: its place in [`Keystroke::ALL`], from [`TAG_BASE`].
        fn tag(self) -> isize {
            TAG_BASE
                + Self::ALL
                    .iter()
                    .position(|keystroke| *keystroke == self)
                    .map_or(0, |index| index as isize)
        }

        fn from_tag(tag: isize) -> Option<Self> {
            Self::ALL
                .get(usize::try_from(tag.checked_sub(TAG_BASE)?).ok()?)
                .copied()
        }
    }

    define_class!(
        // SAFETY: `NSObject` has no subclassing requirements, and `Handler` doesn't implement
        // `Drop`.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "ExoMenuBar"]
        #[ivars = Launcher]
        struct Handler;

        unsafe impl NSObjectProtocol for Handler {}

        impl Handler {
            #[unsafe(method(applicationWillFinishLaunching:))]
            fn will_finish_launching(&self, _notification: &NSNotification) {
                build(self);
            }

            /// Edit ▸ `Enable Autocorrect`.
            #[unsafe(method(toggleAutocorrect:))]
            fn toggle_autocorrect(&self, _sender: Option<&AnyObject>) {
                self.ivars().toggle_autocorrect();
            }

            /// View ▸ `Reload` (⌘R): the account on screen starts over.
            #[unsafe(method(reload:))]
            fn reload(&self, _sender: Option<&AnyObject>) {
                self.ivars().reload();
            }

            /// View ▸ `Toggle Full Screen` (⌃⌘F): the focused window goes into full screen, or out
            /// of it. Not `toggleFullScreen:`, whose item AppKit retitles `Enter Full Screen`.
            #[unsafe(method(toggleWindowFullScreen:))]
            fn toggle_full_screen(&self, _sender: Option<&AnyObject>) {
                if let Some(window) = NSApplication::sharedApplication(self.mtm()).keyWindow() {
                    window.toggleFullScreen(None);
                }
            }

            /// The app menu's updater item: installs the update ready, or checks.
            #[unsafe(method(updaterItem:))]
            fn updater_item(&self, _sender: Option<&AnyObject>) {
                if let Some(updates) = self.ivars().updates() {
                    updates.menu_chosen();
                }
            }

            /// The updater item says where the updater is as the menu opens, and can be chosen
            /// only when idle or ready; the handler's other items always can.
            #[unsafe(method(validateMenuItem:))]
            fn validate_menu_item(&self, item: &NSMenuItem) -> Bool {
                if item.tag() != UPDATER_TAG {
                    return Bool::YES;
                }
                let Some(updates) = self.ivars().updates() else {
                    return Bool::NO;
                };
                let (title, enabled) = updates.stage().menu_item();
                item.setTitle(&NSString::from_str(title));
                Bool::new(enabled)
            }

            /// Help ▸ `Clear Cache and Restart`: a modal alert, `Cancel` the default, before
            /// anything is cleared.
            #[unsafe(method(clearCacheAndRestart:))]
            fn clear_cache_and_restart(&self, _sender: Option<&AnyObject>) {
                let alert = NSAlert::new(self.mtm());
                alert.setMessageText(ns_string!("Are you sure?"));
                alert.setInformativeText(ns_string!("This will sign you out from Exo, delete all local data, and restart it."));
                alert.addButtonWithTitle(ns_string!("Cancel"));
                alert.addButtonWithTitle(ns_string!("Yes"));
                if alert.runModal() != NSAlertFirstButtonReturn {
                    self.ivars().clear_cache();
                }
            }
        }

        unsafe impl NSMenuDelegate for Handler {
            /// A keystroke item shows its shortcut while its menu is open, and only then; `Enable
            /// Autocorrect` is checked as it opens.
            #[unsafe(method(menuWillOpen:))]
            fn menu_will_open(&self, menu: &NSMenu) {
                show_shortcuts(menu, true);
                let on = self.ivars().autocorrect().unwrap_or(false);
                for item in menu.itemArray().iter().filter(|item| item.tag() == AUTOCORRECT_TAG) {
                    item.setState(if on { NSControlStateValueOn } else { NSControlStateValueOff });
                }
            }

            #[unsafe(method(menuDidClose:))]
            fn menu_did_close(&self, menu: &NSMenu) {
                show_shortcuts(menu, false);
            }
        }
    );

    pub(super) fn install(launcher: Launcher) {
        let Some(mtm) = MainThreadMarker::new() else {
            log::warn!("the menu bar needs the main thread");
            return;
        };
        HANDLER.with(|cell| {
            let handler = cell.get_or_init(|| {
                let this = Handler::alloc(mtm).set_ivars(launcher);
                // SAFETY: `NSObject`'s designated initializer.
                unsafe { msg_send![super(this), init] }
            });
            // SAFETY: the selector is the handler's, taking the notification; the handler lives
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

    /// An item sending `action` to `target` (nil: whatever has the keyboard), with `key` and
    /// `mask` as its shortcut.
    fn item(
        mtm: MainThreadMarker,
        title: &str,
        action: Sel,
        target: Option<&AnyObject>,
        key: &str,
        mask: NSEventModifierFlags,
    ) -> Retained<NSMenuItem> {
        // SAFETY: the actions take the sender, and a target outlives its items (the handler
        // lives for good).
        unsafe {
            let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                &NSString::from_str(title),
                Some(action),
                &NSString::from_str(key),
            );
            item.setKeyEquivalentModifierMask(mask);
            item.setTarget(target);
            item
        }
    }

    /// A keystroke's item: for whatever has the keyboard, its shortcut shown only while its menu
    /// is open.
    fn keystroke_item(mtm: MainThreadMarker, keystroke: Keystroke) -> Retained<NSMenuItem> {
        let item = item(
            mtm,
            keystroke.title(),
            keystroke.action(),
            None,
            "",
            NSEventModifierFlags::Command,
        );
        item.setTag(keystroke.tag());
        item
    }

    /// A menu titled `title` holding `items`, `None` a separator, added to `bar`.
    fn menu(
        mtm: MainThreadMarker,
        bar: &NSMenu,
        title: &str,
        items: impl IntoIterator<Item = Option<Retained<NSMenuItem>>>,
        delegate: Option<&Handler>,
    ) -> Retained<NSMenu> {
        let menu = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str(title));
        for item in items {
            menu.addItem(&item.unwrap_or_else(|| NSMenuItem::separatorItem(mtm)));
        }
        if let Some(delegate) = delegate {
            menu.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(delegate)));
        }
        let holder = NSMenuItem::new(mtm);
        holder.setTitle(&NSString::from_str(title));
        holder.setSubmenu(Some(&menu));
        bar.addItem(&holder);
        menu
    }

    /// The menu bar, with macOS's app menu.
    fn build(handler: &Handler) {
        let mtm = handler.mtm();
        let app = NSApplication::sharedApplication(mtm);
        let command = NSEventModifierFlags::Command;
        let ours: Option<&AnyObject> = Some(handler);
        let name = NSProcessInfo::processInfo().processName().to_string();
        let bar = NSMenu::new(mtm);

        // The updater's item and its separator after `About Exo`'s, when Exo updates itself.
        // `Set as Default Email Client` goes in after it, or else with a separator of its own
        // after `About Exo`'s (`mail_client`).
        let updater = handler.ivars().updates().map(|updates| {
            let item = item(
                mtm,
                updates.stage().menu_item().0,
                sel!(updaterItem:),
                ours,
                "",
                command,
            );
            item.setTag(UPDATER_TAG);
            item
        });
        let services = NSMenu::initWithTitle(mtm.alloc(), ns_string!("Services"));
        let services_item = NSMenuItem::new(mtm);
        services_item.setTitle(ns_string!("Services"));
        services_item.setSubmenu(Some(&services));
        menu(
            mtm,
            &bar,
            &name,
            [
                Some(item(
                    mtm,
                    &format!("About {name}"),
                    sel!(orderFrontStandardAboutPanel:),
                    None,
                    "",
                    command,
                )),
                None,
            ]
            .into_iter()
            .chain(
                updater
                    .into_iter()
                    .flat_map(|updater| [Some(updater), None]),
            )
            .chain([
                Some(services_item),
                None,
                Some(item(
                    mtm,
                    &format!("Hide {name}"),
                    sel!(hide:),
                    None,
                    "h",
                    command,
                )),
                Some(item(
                    mtm,
                    "Hide Others",
                    sel!(hideOtherApplications:),
                    None,
                    "h",
                    NSEventModifierFlags::Option | command,
                )),
                Some(item(
                    mtm,
                    "Show All",
                    sel!(unhideAllApplications:),
                    None,
                    "",
                    command,
                )),
                None,
                Some(item(
                    mtm,
                    &format!("Quit {name}"),
                    sel!(terminate:),
                    None,
                    "q",
                    command,
                )),
            ]),
            None,
        );
        app.setServicesMenu(Some(&services));

        menu(
            mtm,
            &bar,
            "File",
            [
                Some(keystroke_item(mtm, Keystroke::NewTab)),
                None,
                Some(keystroke_item(mtm, Keystroke::CloseWindow)),
                Some(keystroke_item(mtm, Keystroke::CloseTab)),
                None,
                Some(keystroke_item(mtm, Keystroke::Print)),
            ],
            Some(handler),
        );
        let edit = [
            Keystroke::Undo,
            Keystroke::Redo,
            Keystroke::Cut,
            Keystroke::Copy,
            Keystroke::Paste,
            Keystroke::PasteAndMatchStyle,
            Keystroke::SelectAll,
        ]
        .map(|keystroke| Some(keystroke_item(mtm, keystroke)));
        let [undo, redo, rest @ ..] = edit;
        let autocorrect = item(
            mtm,
            "Enable Autocorrect",
            sel!(toggleAutocorrect:),
            ours,
            "",
            command,
        );
        autocorrect.setTag(AUTOCORRECT_TAG);
        menu(
            mtm,
            &bar,
            "Edit",
            [undo, redo, None]
                .into_iter()
                .chain(rest)
                .chain([None, Some(autocorrect)]),
            Some(handler),
        );
        menu(
            mtm,
            &bar,
            "View",
            [
                Some(item(mtm, "Reload", sel!(reload:), ours, "r", command)),
                None,
                Some(item(
                    mtm,
                    "Toggle Full Screen",
                    sel!(toggleWindowFullScreen:),
                    ours,
                    "f",
                    NSEventModifierFlags::Control | command,
                )),
                None,
                Some(keystroke_item(mtm, Keystroke::ActualSize)),
                Some(keystroke_item(mtm, Keystroke::ZoomIn)),
                Some(keystroke_item(mtm, Keystroke::ZoomOut)),
            ],
            Some(handler),
        );
        // macOS adds its window items and the windows (`role: 'window'`).
        let close_window = item(mtm, "Close Window", sel!(performClose:), None, "", command);
        close_window.setTag(CLOSE_WINDOW_TAG);
        let window = menu(
            mtm,
            &bar,
            "Window",
            [
                Some(keystroke_item(mtm, Keystroke::NextTab)),
                Some(keystroke_item(mtm, Keystroke::PreviousTab)),
                Some(item(
                    mtm,
                    "Minimize",
                    sel!(performMiniaturize:),
                    None,
                    "m",
                    command,
                )),
                Some(close_window),
                None,
                Some(item(
                    mtm,
                    "Bring All to Front",
                    sel!(arrangeInFront:),
                    None,
                    "",
                    command,
                )),
            ],
            Some(handler),
        );
        app.setWindowsMenu(Some(&window));
        // macOS adds its search field (`role: 'help'`).
        let help = menu(
            mtm,
            &bar,
            "Help",
            [Some(item(
                mtm,
                "Clear Cache and Restart",
                sel!(clearCacheAndRestart:),
                ours,
                "",
                command,
            ))],
            None,
        );
        app.setHelpMenu(Some(&help));
        app.setMainMenu(Some(&bar));
    }

    /// Shows or hides the shortcuts of `menu`'s keystroke items, and Window ▸ `Close Window`'s.
    fn show_shortcuts(menu: &NSMenu, shown: bool) {
        for item in menu.itemArray().iter() {
            let shortcut = match Keystroke::from_tag(item.tag()) {
                Some(keystroke) => keystroke.shortcut(),
                None if item.tag() == CLOSE_WINDOW_TAG => ("w", NSEventModifierFlags::Command),
                None => continue,
            };
            let (key, mask) = shortcut;
            item.setKeyEquivalent(&NSString::from_str(if shown { key } else { "" }));
            item.setKeyEquivalentModifierMask(mask);
        }
    }

    /// Exo's view answers the keystroke items' actions, which AppKit sends whatever has the
    /// keyboard: Exo's window, while it's the key window. winit's view has none of them, so
    /// they're added to its class.
    pub(super) fn window_ready() {
        if MainThreadMarker::new().is_none() {
            return;
        }
        let Some(class) = AnyClass::get(VIEW_CLASS) else {
            log::warn!("the menu bar's keystrokes: no {VIEW_CLASS:?} class");
            return;
        };
        // SAFETY: `send` has the signature of an action (`self`, `_cmd`, the sender), which
        // `v@:@` describes; the class is winit's view, which doesn't define these actions.
        unsafe {
            let send: unsafe extern "C-unwind" fn(&AnyObject, Sel, Option<&AnyObject>) = send;
            let imp: Imp = std::mem::transmute(send);
            let mut added = Vec::new();
            for action in Keystroke::ALL.map(Keystroke::action) {
                if added.contains(&action) {
                    continue;
                }
                added.push(action);
                if !objc2::ffi::class_addMethod(
                    std::ptr::from_ref(class).cast_mut(),
                    action,
                    imp,
                    c"v@:@".as_ptr(),
                )
                .as_bool()
                {
                    log::warn!(
                        "the menu bar's keystrokes: {VIEW_CLASS:?} already answers {action}"
                    );
                }
            }
        }
    }

    /// A keystroke item, sent by AppKit to Exo's view: its keystroke goes to Exo. The same
    /// actions from anything else (a service, accessibility) do nothing.
    unsafe extern "C-unwind" fn send(_view: &AnyObject, _action: Sel, sender: Option<&AnyObject>) {
        let item = sender.and_then(AnyObject::downcast_ref::<NSMenuItem>);
        let Some(keystroke) = item.and_then(|item| Keystroke::from_tag(item.tag())) else {
            return;
        };
        HANDLER.with(|cell| {
            if let Some(handler) = cell.get() {
                handler.ivars().keystroke(keystroke);
            }
        });
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn each_keystroke_has_its_own_tag() {
            for keystroke in Keystroke::ALL {
                assert_eq!(Keystroke::from_tag(keystroke.tag()), Some(keystroke));
            }
            assert_eq!(Keystroke::from_tag(0), None, "the other items");
            assert_eq!(Keystroke::from_tag(TAG_BASE - 1), None);
            assert_eq!(
                Keystroke::from_tag(TAG_BASE + Keystroke::ALL.len() as isize),
                None
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_keystroke_is_what_egui_winit_makes_of_its_keys() {
        let clipboard = || Some("one\r\ntwo".to_owned());
        assert_eq!(
            Keystroke::Copy.input(clipboard),
            (COMMAND, vec![Event::Copy])
        );
        assert_eq!(Keystroke::Cut.input(clipboard), (COMMAND, vec![Event::Cut]));
        assert_eq!(
            Keystroke::Paste.input(clipboard),
            (COMMAND, vec![Event::Paste("one\ntwo".to_owned())])
        );
        // ⇧ held, which the editor reads to paste without the clipboard's formatting.
        assert_eq!(
            Keystroke::PasteAndMatchStyle.input(clipboard),
            (SHIFT_COMMAND, vec![Event::Paste("one\ntwo".to_owned())])
        );
        // No text (a screenshot): the key, which compose pastes an image for.
        let key = [true, false]
            .map(|pressed| Event::Key {
                key: Key::V,
                physical_key: Some(Key::V),
                pressed,
                repeat: false,
                modifiers: COMMAND,
            })
            .to_vec();
        assert_eq!(
            Keystroke::Paste.input(|| None),
            (COMMAND, key.clone()),
            "no text to paste"
        );
        assert_eq!(
            Keystroke::Paste.input(|| Some(String::new())),
            (COMMAND, key)
        );
        let (modifiers, events) = Keystroke::Redo.input(clipboard);
        assert_eq!(modifiers, SHIFT_COMMAND);
        assert_eq!(
            events,
            [true, false].map(|pressed| Event::Key {
                key: Key::Z,
                physical_key: Some(Key::Z),
                pressed,
                repeat: false,
                modifiers: SHIFT_COMMAND
            })
        );
    }
}
