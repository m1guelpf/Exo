//! Where the Mac draws its power from, which sets how often an account off screen syncs: it's
//! plugged in unless it runs on its own battery, and a Mac without a battery counts as plugged in.

use tokio::sync::watch;

/// The Mac's power source, as the sync policy tells them apart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Power {
    /// Mains or a UPS, or a Mac with no battery.
    #[default]
    Plugged,
    /// Its own battery.
    Battery,
}

impl Power {
    /// The power source `IOPSGetProvidingPowerSourceType` names: `Battery Power`
    /// (`kIOPMBatteryPowerKey`) is the battery; `AC Power`, `UPS Power` or nothing known isn't.
    fn providing(source: Option<&str>) -> Self {
        if source == Some("Battery Power") {
            Self::Battery
        } else {
            Self::Plugged
        }
    }
}

/// The Mac's power source, followed while any clone is alive; or a fixed one.
#[derive(Clone, Default)]
pub struct PowerSource {
    power: watch::Sender<Power>,
    /// Following the OS until the last clone drops.
    #[cfg(target_os = "macos")]
    _watch: Option<std::sync::Arc<macos::Watch>>,
}

impl PowerSource {
    /// The Mac's power source, followed as the Mac switches between mains, a UPS and its battery.
    /// Plugged in where the OS can't be asked.
    pub fn watch() -> Self {
        #[cfg(target_os = "macos")]
        {
            let power = watch::Sender::new(macos::read());
            let watch = macos::Watch::start(power.clone()).map(std::sync::Arc::new);
            if watch.is_none() {
                log::warn!("can't follow the power source; it's read once");
            }
            Self {
                power,
                _watch: watch,
            }
        }
        #[cfg(not(target_os = "macos"))]
        Self::default()
    }

    /// `power` until [`set`](Self::set) says otherwise: for tests and screenshots, which mustn't
    /// depend on the Mac's.
    pub fn fixed(power: Power) -> Self {
        let source = Self::default();
        source.set(power);
        source
    }

    /// Changes the power source, as the OS would.
    pub fn set(&self, power: Power) {
        self.power
            .send_if_modified(|current| std::mem::replace(current, power) != power);
    }

    /// The power source now, and as it changes.
    pub fn subscribe(&self) -> watch::Receiver<Power> {
        self.power.subscribe()
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{CStr, c_char, c_int};
    use std::ptr::NonNull;

    use block2::{Block, RcBlock};
    use dispatch2::{DispatchQueue, DispatchRetained};
    use objc2_core_foundation::{CFRetained, CFString, CFType};
    use tokio::sync::watch;

    use super::Power;

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        /// A snapshot of the power sources, owned by the caller; null when there's none to read.
        fn IOPSCopyPowerSourcesInfo() -> *mut CFType;
        /// The source the Mac draws from in `snapshot`, owned by the snapshot.
        fn IOPSGetProvidingPowerSourceType(snapshot: &CFType) -> *const CFString;
    }

    // libSystem's notify(3).
    unsafe extern "C" {
        fn notify_register_dispatch(
            name: *const c_char,
            out_token: *mut c_int,
            queue: &DispatchQueue,
            handler: &Block<dyn Fn(c_int)>,
        ) -> u32;
        fn notify_cancel(token: c_int) -> u32;
    }

    /// `kIOPSNotifyPowerSource`, posted when the Mac switches between mains, a UPS and its
    /// battery (not for the battery's level).
    const POWER_SOURCE_CHANGED: &CStr = c"com.apple.system.powersources.source";
    /// `NOTIFY_STATUS_OK`.
    const OK: u32 = 0;

    /// The power source now.
    pub(super) fn read() -> Power {
        // SAFETY: takes no arguments; the snapshot it returns is ours to release.
        let Some(snapshot) = NonNull::new(unsafe { IOPSCopyPowerSourcesInfo() }) else {
            return Power::Plugged;
        };
        // SAFETY: a new snapshot (the Create rule), released when this drops.
        let snapshot = unsafe { CFRetained::from_raw(snapshot) };
        // SAFETY: `snapshot` is a live snapshot from `IOPSCopyPowerSourcesInfo`.
        let providing = unsafe { IOPSGetProvidingPowerSourceType(&snapshot) };
        // SAFETY: null or a string the snapshot owns, which outlives this read.
        Power::providing(
            unsafe { providing.as_ref() }
                .map(ToString::to_string)
                .as_deref(),
        )
    }

    /// Reads the power source into a channel each time macOS says it changed, on a queue of its
    /// own, until dropped.
    pub(super) struct Watch {
        token: c_int,
        queue: DispatchRetained<DispatchQueue>,
    }

    impl Watch {
        pub(super) fn start(power: watch::Sender<Power>) -> Option<Self> {
            let queue = DispatchQueue::new("exo.power", None);
            let handler = RcBlock::new(move |_token: c_int| {
                let now = read();
                power.send_if_modified(|current| std::mem::replace(current, now) != now);
            });
            let mut token = 0;
            // SAFETY: the name is a C string, `token` is valid for writes, and notify copies the
            // block, which owns all it captures.
            let status = unsafe {
                notify_register_dispatch(
                    POWER_SOURCE_CHANGED.as_ptr(),
                    &raw mut token,
                    &queue,
                    &handler,
                )
            };
            (status == OK).then_some(Self { token, queue })
        }
    }

    impl Drop for Watch {
        fn drop(&mut self) {
            // SAFETY: `token` came from `notify_register_dispatch`.
            unsafe { notify_cancel(self.token) };
            // A handler already on the queue runs before this returns.
            self.queue.exec_sync(|| {});
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_battery_counts_as_battery() {
        assert_eq!(Power::providing(Some("Battery Power")), Power::Battery);
        assert_eq!(Power::providing(Some("AC Power")), Power::Plugged);
        assert_eq!(
            Power::providing(Some("UPS Power")),
            Power::Plugged,
            "a UPS counts as plugged in"
        );
        assert_eq!(Power::providing(None), Power::Plugged);
    }

    #[test]
    fn a_fixed_source_changes_only_when_set() {
        let source = PowerSource::fixed(Power::Battery);
        let mut power = source.subscribe();
        assert_eq!(*power.borrow_and_update(), Power::Battery);
        source.clone().set(Power::Battery);
        assert!(
            !power.has_changed().expect("the source is alive"),
            "the same power isn't a change"
        );
        source.set(Power::Plugged);
        assert!(power.has_changed().expect("the source is alive"));
        assert_eq!(*power.borrow_and_update(), Power::Plugged);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_mac_says_where_its_power_comes_from() {
        // Whatever it is, reading and following it works and agrees.
        let source = PowerSource::watch();
        assert_eq!(*source.subscribe().borrow(), macos::read());
    }
}
