//! Native context menus: Exo's right-click menus are AppKit's (`NSMenu`), so they look and behave
//! as a Mac app's do. [`show`] pops one up at the pointer and waits for the item chosen, as
//! AppKit's menu tracking does. The thread list's rows use it, and compose's body; [`look_up`] is
//! the shell's `Look Up`.

/// An entry of a context menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuItem {
    /// A command.
    Item {
        /// What it says.
        title: String,
        /// The key that runs it outside the menu, shown at its right.
        key: Option<MenuKey>,
        /// Its SF Symbol, shown at its left.
        symbol: Option<&'static str>,
        /// It can run now; otherwise it's dimmed.
        enabled: bool,
    },
    /// A line between groups of items.
    Separator,
}

/// A key a menu item shows: a character, with ⇧ when `shift` (`⇧E`) and ⌘ when `command`
/// (`⌘X`), the key that runs the item's command outside the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuKey {
    /// The character, lower-case for a letter.
    pub key: char,
    /// Shift is held with it.
    pub shift: bool,
    /// ⌘ is held with it.
    pub command: bool,
}

/// Shows macOS's definition of `text` (Dictionary's popover), its first line's baseline starting at
/// `at` (points from the window content's top left), in the system font at `size`. Does nothing off
/// the main thread and off macOS.
pub fn look_up(text: &str, at: egui::Pos2, size: f32) {
    #[cfg(target_os = "macos")]
    macos::look_up(text, at, size);
    #[cfg(not(target_os = "macos"))]
    let _ = (text, at, size);
}

/// What shows a context menu: [`show`], or a stand-in in tests.
pub type Show = dyn FnMut(&[MenuItem], egui::Pos2) -> Option<usize>;

/// Shows `items` at `at` (points from the window content's top left) and returns the index of
/// the one chosen, if one is. Blocks until the menu closes. `None` off the main thread and off
/// macOS, where there's no menu.
pub fn show(items: &[MenuItem], at: egui::Pos2) -> Option<usize> {
    #[cfg(target_os = "macos")]
    return macos::show(items, at);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (items, at);
        None
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::Cell;

    use objc2::{
        DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained, sel,
    };
    use objc2_app_kit::{
        NSApplication, NSEventModifierFlags, NSFont, NSFontAttributeName, NSImage, NSMenu,
        NSMenuItem, NSView,
    };
    use objc2_foundation::{
        NSAttributedString, NSDictionary, NSObject, NSObjectProtocol, NSPoint, NSString,
    };

    use super::MenuItem;

    define_class!(
        // SAFETY: `NSObject` has no subclassing requirements, and `Chooser` doesn't implement
        // `Drop`.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "ExoContextMenuChooser"]
        #[ivars = Cell<Option<isize>>]
        struct Chooser;

        unsafe impl NSObjectProtocol for Chooser {}

        impl Chooser {
            /// An item was chosen: remember which.
            #[unsafe(method(choose:))]
            fn choose(&self, sender: &NSMenuItem) {
                self.ivars().set(Some(sender.tag()));
            }
        }
    );

    pub(super) fn show(items: &[MenuItem], at: egui::Pos2) -> Option<usize> {
        let mtm = MainThreadMarker::new()?;
        let view = content_view(mtm)?;
        let this = Chooser::alloc(mtm).set_ivars(Cell::new(None));
        // SAFETY: `NSObject`'s designated initializer.
        let chooser: Retained<Chooser> = unsafe { msg_send![super(this), init] };
        let menu = NSMenu::new(mtm);
        // Items are on or off as `enabled` says, not as AppKit's validation would have them.
        menu.setAutoenablesItems(false);
        for (index, entry) in items.iter().enumerate() {
            let MenuItem::Item {
                title,
                key,
                symbol,
                enabled,
            } = entry
            else {
                menu.addItem(&NSMenuItem::separatorItem(mtm));
                continue;
            };
            let equivalent = key.map_or_else(String::new, |key| key.key.to_string());
            // SAFETY: the action is the chooser's, taking the sender; the chooser outlives the
            // menu, which lives until this returns.
            let item = unsafe {
                let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                    mtm.alloc(),
                    &NSString::from_str(title),
                    Some(sel!(choose:)),
                    &NSString::from_str(&equivalent),
                );
                item.setTarget(Some(&chooser));
                item
            };
            item.setTag(isize::try_from(index).ok()?);
            item.setEnabled(*enabled);
            // The key with what it takes held: ⇧, ⌘, or neither.
            let mut mask = NSEventModifierFlags::empty();
            if key.is_some_and(|key| key.shift) {
                mask |= NSEventModifierFlags::Shift;
            }
            if key.is_some_and(|key| key.command) {
                mask |= NSEventModifierFlags::Command;
            }
            item.setKeyEquivalentModifierMask(mask);
            if let Some(image) = symbol.and_then(|symbol| {
                NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &NSString::from_str(symbol),
                    None,
                )
            }) {
                item.setImage(Some(&image));
            }
            menu.addItem(&item);
        }
        // Tracks the menu until it closes, having sent the chosen item's action.
        menu.popUpMenuPositioningItem_atLocation_inView(None, point(&view, at), Some(&view));
        chooser
            .ivars()
            .get()
            .and_then(|tag| usize::try_from(tag).ok())
    }

    pub(super) fn look_up(text: &str, at: egui::Pos2, size: f32) {
        let Some(view) = MainThreadMarker::new().and_then(content_view) else {
            return;
        };
        let font = NSFont::systemFontOfSize(size.into());
        // SAFETY: a constant AppKit defines.
        let key = unsafe { NSFontAttributeName };
        let attributes = NSDictionary::from_slices(&[key], &[&*font as &objc2::runtime::AnyObject]);
        // SAFETY: `NSFontAttributeName` takes an `NSFont`.
        let string = unsafe {
            NSAttributedString::new_with_attributes(&NSString::from_str(text), &attributes)
        };
        view.showDefinitionForAttributedString_atPoint(Some(&string), point(&view, at));
    }

    /// The window's content view, which menus pop up in.
    fn content_view(mtm: MainThreadMarker) -> Option<Retained<NSView>> {
        let app = NSApplication::sharedApplication(mtm);
        app.keyWindow().or_else(|| app.mainWindow())?.contentView()
    }

    /// `at`, points from the content's top left, in `view`'s coordinates.
    fn point(view: &NSView, at: egui::Pos2) -> NSPoint {
        let height = view.frame().size.height;
        if view.isFlipped() {
            NSPoint::new(at.x.into(), at.y.into())
        } else {
            NSPoint::new(at.x.into(), height - f64::from(at.y))
        }
    }
}
