//! The OS's word on whether the Mac has a network. It only says the state flipped; the account
//! then checks whether Gmail answers ([`Account::network_changed`](crate::account::Account::network_changed)).

/// Watches the Mac's route to the internet while alive.
pub struct Reachability {
    /// Watching until dropped.
    #[cfg(target_os = "macos")]
    _watch: macos::Watch,
}

impl Reachability {
    /// Calls `changed` whenever the Mac gains or loses its route to the internet, and once at
    /// the start when it has none (a launch with the network off). `None` where the OS can't be
    /// asked.
    pub fn watch(changed: impl Fn() + Send + Sync + 'static) -> Option<Self> {
        #[cfg(target_os = "macos")]
        return macos::Watch::start(Box::new(changed)).map(|watch| Self { _watch: watch });
        #[cfg(not(target_os = "macos"))]
        {
            let _ = changed;
            None
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    // SCNetworkReachability is deprecated in favour of Network.framework's path monitor, which
    // has no bindings in the objc2 family Exo uses; it still works.
    #![allow(deprecated)]

    use std::ffi::c_void;
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicBool, Ordering};

    use dispatch2::{DispatchQueue, DispatchRetained};
    use objc2_core_foundation::CFRetained;
    use objc2_system_configuration::{
        SCNetworkReachability, SCNetworkReachabilityContext, SCNetworkReachabilityFlags,
    };

    /// What the callback reaches through its `info` pointer.
    struct State {
        changed: Box<dyn Fn() + Send + Sync>,
        online: AtomicBool,
    }

    pub(super) struct Watch {
        target: CFRetained<SCNetworkReachability>,
        queue: DispatchRetained<DispatchQueue>,
        /// Owned; freed once the callback can no longer run.
        state: NonNull<State>,
    }

    /// Online: reachable without first having to bring a connection up.
    fn online(flags: SCNetworkReachabilityFlags) -> bool {
        flags.contains(SCNetworkReachabilityFlags::Reachable)
            && !flags.contains(SCNetworkReachabilityFlags::ConnectionRequired)
    }

    impl Watch {
        pub(super) fn start(changed: Box<dyn Fn() + Send + Sync>) -> Option<Self> {
            // The default route, 0.0.0.0.
            // SAFETY: `sockaddr_in` is plain data; all zeros is the any address.
            let mut address: libc::sockaddr_in = unsafe { std::mem::zeroed() };
            address.sin_len = std::mem::size_of::<libc::sockaddr_in>() as u8;
            address.sin_family = libc::AF_INET as u8;
            // SAFETY: `address` is a valid `sockaddr_in`, which the call copies.
            let target = unsafe {
                SCNetworkReachability::with_address(None, NonNull::from(&address).cast())
            }?;
            let mut flags = SCNetworkReachabilityFlags(0);
            // SAFETY: `flags` is valid for writes.
            let known = unsafe { target.flags(NonNull::from(&mut flags)) };
            let now_online = !known || online(flags);
            let state = NonNull::from(Box::leak(Box::new(State {
                changed,
                online: AtomicBool::new(now_online),
            })));
            let watch = Self {
                target,
                queue: DispatchQueue::new("exo.reachability", None),
                state,
            };
            let mut context = SCNetworkReachabilityContext {
                version: 0,
                info: state.as_ptr().cast(),
                retain: None,
                release: None,
                copyDescription: None,
            };
            // SAFETY: `callback` only reads `State` through `info`, which outlives the callback's
            // registration (`Drop` unregisters before freeing it); the context is copied.
            let watching = unsafe {
                watch.target.set_callback(Some(callback), &mut context)
                    && watch.target.set_dispatch_queue(Some(&watch.queue))
            };
            if !watching {
                return None;
            }
            if !now_online {
                // SAFETY: `state` is alive until `watch` drops.
                (unsafe { watch.state.as_ref() }.changed)();
            }
            Some(watch)
        }
    }

    unsafe extern "C-unwind" fn callback(
        _: NonNull<SCNetworkReachability>,
        flags: SCNetworkReachabilityFlags,
        info: *mut c_void,
    ) {
        // SAFETY: `info` is the `State` `start` registered, alive while the callback is.
        let state = unsafe { &*info.cast::<State>() };
        let now = online(flags);
        if state.online.swap(now, Ordering::SeqCst) != now {
            (state.changed)();
        }
    }

    impl Drop for Watch {
        fn drop(&mut self) {
            // SAFETY: unregistering takes no pointers.
            unsafe {
                self.target.set_dispatch_queue(None);
                self.target.set_callback(None, std::ptr::null_mut());
            }
            // A callback already on the queue runs before this does.
            self.queue.exec_sync(|| {});
            // SAFETY: `state` came from `Box::leak`, and nothing can reach it any more.
            drop(unsafe { Box::from_raw(self.state.as_ptr()) });
        }
    }
}
