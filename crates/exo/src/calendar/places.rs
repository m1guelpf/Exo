//! Where's suggestions: places for what's typed, from Apple Maps, MapKit's local search completer,
//! which needs no key and no billing. A place's name and address are the completer's title and
//! subtitle.

use std::sync::{Arc, Mutex, PoisonError};

/// How many places Where lists.
const LIMIT: usize = 5;

/// A place found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    pub address: String,
}

impl Place {
    /// What Where takes for it: `{name}, {address}`, the address alone when it starts with the
    /// name, the name alone without one.
    pub fn location(&self) -> String {
        if self.address.is_empty() {
            self.name.clone()
        } else if self.address.starts_with(&self.name) {
            self.address.clone()
        } else {
            format!("{}, {}", self.name, self.address)
        }
    }
}

/// Places every ask is answered with instead of Apple Maps', for tests and screenshots.
static ANSWERS: Mutex<Option<Vec<Place>>> = Mutex::new(None);

/// Answers every ask of two characters or more with `places`, instead of asking Apple Maps (the
/// shot example's and the tests' stand-in for the network).
pub fn answer_with(places: Vec<Place>) {
    *ANSWERS.lock().unwrap_or_else(PoisonError::into_inner) = Some(places);
}

/// Finds places as Where is typed in.
#[derive(Default)]
pub struct Places {
    /// What was asked last.
    asked: String,
    found: Arc<Mutex<Vec<Place>>>,
    #[cfg(target_os = "macos")]
    completer: Option<macos::Completer>,
}

impl Places {
    /// Asks for places matching `text`, unless it was asked last; under two characters, or
    /// offline, there are none. Answers come in later and repaint `ctx`.
    pub fn ask(&mut self, ctx: &egui::Context, text: &str, online: bool) {
        if text == self.asked {
            return;
        }
        self.asked = text.to_owned();
        self.set(Vec::new());
        if text.chars().count() <= 1 || !online {
            return;
        }
        if let Some(answers) = ANSWERS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            self.set(answers.into_iter().take(LIMIT).collect());
            return;
        }
        #[cfg(target_os = "macos")]
        {
            if self.completer.is_none() {
                self.completer = macos::Completer::new(self.found.clone(), ctx.clone());
            }
            if let Some(completer) = &self.completer {
                completer.ask(text);
            }
        }
        #[cfg(not(target_os = "macos"))]
        drop(ctx);
    }

    /// The places found for what was asked last.
    pub fn found(&self) -> Vec<Place> {
        self.found
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Forgets what was asked and found (a pick, the field left).
    pub fn reset(&mut self) {
        if self.asked.is_empty() {
            return;
        }
        self.asked.clear();
        self.set(Vec::new());
        #[cfg(target_os = "macos")]
        if let Some(completer) = &self.completer {
            completer.cancel();
        }
    }

    fn set(&self, places: Vec<Place>) {
        *self.found.lock().unwrap_or_else(PoisonError::into_inner) = places;
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::sync::{Arc, Mutex, PoisonError};

    use objc2::{
        DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
        runtime::ProtocolObject,
    };
    use objc2_foundation::{NSError, NSObject, NSObjectProtocol, NSString};
    use objc2_map_kit::{
        MKLocalSearchCompleter, MKLocalSearchCompleterDelegate, MKLocalSearchCompleterResultType,
    };

    use super::{LIMIT, Place};

    /// Where the delegate puts what it hears, and who to tell.
    pub struct Answers {
        found: Arc<Mutex<Vec<Place>>>,
        ctx: egui::Context,
    }

    define_class!(
        // SAFETY: `NSObject` has no subclassing requirements, and `Delegate` doesn't implement
        // `Drop`.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "ExoPlacesDelegate"]
        #[ivars = Answers]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl MKLocalSearchCompleterDelegate for Delegate {
            #[unsafe(method(completerDidUpdateResults:))]
            fn did_update(&self, completer: &MKLocalSearchCompleter) {
                // SAFETY: the completer's results, read on the main thread its delegate is
                // called on.
                let results = unsafe { completer.results() };
                let places = results.iter().take(LIMIT).map(|result| {
                    // SAFETY: a completion's title and subtitle are plain strings.
                    let (name, address) = unsafe { (result.title(), result.subtitle()) };
                    Place {
                        name: name.to_string(),
                        address: address.to_string(),
                    }
                });
                let ivars = self.ivars();
                *ivars.found.lock().unwrap_or_else(PoisonError::into_inner) = places.collect();
                ivars.ctx.request_repaint();
            }

            #[unsafe(method(completer:didFailWithError:))]
            fn did_fail(&self, _completer: &MKLocalSearchCompleter, _error: &NSError) {
                self.ivars()
                    .found
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clear();
            }
        }
    );

    /// A local search completer for places and addresses, and its delegate.
    pub struct Completer {
        completer: Retained<MKLocalSearchCompleter>,
        // The completer holds its delegate weakly.
        _delegate: Retained<Delegate>,
    }

    impl Completer {
        /// A completer telling `found` and `ctx` what it finds; only on the main thread, whose
        /// run loop calls its delegate.
        pub fn new(found: Arc<Mutex<Vec<Place>>>, ctx: egui::Context) -> Option<Self> {
            let mtm = MainThreadMarker::new()?;
            let delegate = Delegate::alloc(mtm).set_ivars(Answers { found, ctx });
            // SAFETY: `NSObject`'s designated initializer.
            let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
            // SAFETY: a new completer, configured before its first query, on the main thread.
            let completer = unsafe {
                let completer = MKLocalSearchCompleter::new();
                // Establishments and addresses, not queries.
                completer.setResultTypes(
                    MKLocalSearchCompleterResultType::Address
                        | MKLocalSearchCompleterResultType::PointOfInterest,
                );
                completer.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                completer
            };
            Some(Self {
                completer,
                _delegate: delegate,
            })
        }

        pub fn ask(&self, text: &str) {
            // SAFETY: a query on the main thread the completer was made on.
            unsafe { self.completer.setQueryFragment(&NSString::from_str(text)) };
        }

        pub fn cancel(&self) {
            // SAFETY: as `ask`.
            unsafe { self.completer.cancel() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_place_reads_as_its_name_and_address() {
        let place = |name: &str, address: &str| Place {
            name: name.into(),
            address: address.into(),
        };
        assert_eq!(
            place("Lilia", "567 Union Ave, Brooklyn, NY").location(),
            "Lilia, 567 Union Ave, Brooklyn, NY"
        );
        assert_eq!(
            place("10 Downing Street", "10 Downing Street, London").location(),
            "10 Downing Street, London"
        );
        assert_eq!(place("Berlin", "").location(), "Berlin");
    }
}
