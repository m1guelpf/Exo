//! The app driven end to end for the integration tests: gmail-mock, a fresh store, keys and
//! clicks.

#![allow(
    dead_code,
    reason = "each test binary uses a different part of the fixture"
)]

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use exo::{
    App, Clock, Mail, Start,
    account::{Account, Credentials},
};

/// The app on a fresh store synced from an in-process gmail-mock, at 1280×800.
pub struct Fixture {
    pub harness: Harness<'static, App>,
    /// Serves the mock, and runs what other devices do to it.
    runtime: tokio::runtime::Runtime,
    server: gmail_mock::MockServer,
    endpoints: gmail::Endpoints,
    /// What time the app thinks it is: the mock's clock until [`Fixture::set_now`].
    now: Arc<Mutex<jiff::Zoned>>,
    /// Takes People API connections and never answers, for [`start_with_silent_people`].
    _silent: Option<std::net::TcpListener>,
    /// Last, so the app is gone before its store is.
    dir: Dir,
}

/// The fixture's folder (the store, attachment previews), deleted when the fixture is.
struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Where in the fixture's folder the app keeps attachment previews.
const PREVIEWS: &str = "previews";

/// Starts the app on an account with one Inbox and no splits, which most tests are about;
/// `name` keeps concurrent tests' stores apart.
pub fn start(name: &str) -> Fixture {
    start_with(name, false, true, &|_| {})
}

/// [`start`], with the account's settings changed by `change` first.
pub fn start_with_settings(name: &str, change: &dyn Fn(&mut exo_store::Settings)) -> Fixture {
    start_with(name, false, true, change)
}

/// [`start`], with Google's People API (contacts) taking requests and never answering.
pub fn start_with_silent_people(name: &str) -> Fixture {
    start_with(name, true, true, &|_| {})
}

/// [`start`] on a new account as Exo sets it up: Important and Other, and the default splits.
pub fn start_new_account(name: &str) -> Fixture {
    start_with(name, false, false, &|_| {})
}

fn start_with(
    name: &str,
    silent_people: bool,
    one_inbox: bool,
    change: &dyn Fn(&mut exo_store::Settings),
) -> Fixture {
    let config = gmail_mock::Config::default();
    let now = Arc::new(Mutex::new(config.now.clone()));
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let server = runtime
        .block_on(gmail_mock::MockServer::start(config))
        .expect("mock starts");
    let mut endpoints = server.endpoints();
    let silent = silent_people.then(|| {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        let address = listener.local_addr().expect("an address");
        endpoints.people = format!("http://{address}/").parse().expect("a URL");
        listener
    });
    let dir = std::env::temp_dir().join(format!("exo-keyboard-{name}-{}", std::process::id()));
    if one_inbox {
        runtime.block_on(async {
            let store = exo_store::Store::open(
                &dir,
                ACCOUNT,
                endpoints.clone(),
                gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into()),
            )
            .await
            .expect("store opens");
            let mut settings = exo_store::Settings::default();
            settings.set_up_splits(Vec::new);
            settings.toggle_important_other();
            change(&mut settings);
            store.save_settings(&settings).await.expect("settings save");
        });
    }
    let mut harness = launch(&dir, &endpoints, &now);
    use_split_library(&mut harness, None);
    let mut fixture = Fixture {
        harness,
        runtime,
        server,
        endpoints,
        now,
        _silent: silent,
        dir: Dir(dir),
    };
    fixture.settle();
    fixture
}

/// Uses `library` as the split library, whatever the assets hold.
fn use_split_library(
    harness: &mut Harness<'static, App>,
    library: Option<exo_store::SplitLibrary>,
) {
    if let Some(mail) = harness.state_mut().mail_mut() {
        mail.set_split_library(library);
    }
}

/// The mock's account.
const ACCOUNT: &str = "maya@brightmoor.co";

/// The app on the mock's account, its store and attachment previews in `dir`, its clock `now`.
fn launch(
    dir: &Path,
    endpoints: &gmail::Endpoints,
    now: &Arc<Mutex<jiff::Zoned>>,
) -> Harness<'static, App> {
    let now = now.clone();
    let clock = Clock::new(move || now.lock().expect("the clock's lock").clone());
    let credentials = Credentials {
        email: ACCOUNT.into(),
        endpoints: endpoints.clone(),
        tokens: std::sync::Arc::new(gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into())),
    };
    let assets = exo::assets_dir();
    let store_dir = dir.to_owned();
    let mut harness = Harness::builder()
        .with_size([1280.0, 800.0])
        .with_os(egui::os::OperatingSystem::Mac)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |cc| {
            let account =
                Account::open(&store_dir, credentials, cc.egui_ctx.clone()).expect("store opens");
            App::new(cc, &assets, Start::Mail(Box::new(account)), clock).expect("launches")
        });
    // Tests run side by side, in this process and others: the temp directory's cache is shared.
    if let Some(mail) = harness.state_mut().mail_mut() {
        mail.set_preview_cache(dir.join(PREVIEWS));
    }
    harness
}

impl Fixture {
    /// Quits the app and opens it again on the same store, as relaunching Exo does.
    pub fn relaunch(&mut self) {
        self.harness = launch(&self.dir.0, &self.endpoints, &self.now);
        use_split_library(&mut self.harness, None);
        self.settle();
    }

    /// Gives the app the made-up split library in `support/split-library.json`.
    pub fn with_sample_split_library(&mut self) {
        let library = include_str!("split-library.json")
            .parse()
            .expect("the sample parses");
        use_split_library(&mut self.harness, Some(library));
        self.settle();
    }

    /// Sets the app's clock to `at`, as time passing does, and runs a frame.
    pub fn set_now(&mut self, at: jiff::Zoned) {
        *self.now.lock().expect("the clock's lock") = at;
        self.harness.run_steps(1);
    }

    /// The mail on screen.
    pub fn mail(&self) -> &Mail {
        self.harness.state().mail().expect("the fixture shows mail")
    }

    /// The mail on screen, to change.
    pub fn mail_mut(&mut self) -> &mut Mail {
        self.harness
            .state_mut()
            .mail_mut()
            .expect("the fixture shows mail")
    }

    /// Where the app keeps attachment previews.
    pub fn previews(&self) -> PathBuf {
        self.dir.0.join(PREVIEWS)
    }

    /// A folder `name` in the fixture's, deleted with it once the app is gone.
    pub fn folder(&self, name: &str) -> PathBuf {
        self.dir.0.join(name)
    }

    /// Runs frames until the app has nothing in flight and animations are over: until it has
    /// been idle for 40 frames in a row, past the 550ms slide-out. Idle only counts after a
    /// frame, since each one handles the input queued before it and may start work.
    pub fn settle(&mut self) {
        let started = Instant::now();
        let mut quiet = 0;
        while quiet < 40 && started.elapsed() < Duration::from_secs(30) {
            self.harness.run_steps(1);
            if self.harness.state().is_idle() {
                quiet += 1;
            } else {
                quiet = 0;
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    pub fn press(&mut self, key: Key, modifiers: Modifiers) {
        let event = |pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        self.harness.input_mut().events.push(event(true));
        self.harness.run_steps(1);
        self.harness.input_mut().events.push(event(false));
        self.settle();
    }

    pub fn key(&mut self, key: Key) {
        self.press(key, Modifiers::NONE);
    }

    /// Types `text` into whatever has the keyboard.
    pub fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.harness
                .input_mut()
                .events
                .push(Event::Text(ch.to_string()));
            self.harness.run_steps(1);
        }
        self.settle();
    }

    /// Lets `seconds` of app time pass (autosave, toasts), then settles.
    pub fn wait(&mut self, seconds: f32) {
        self.harness.run_steps((seconds * 60.0).ceil() as usize);
        self.settle();
    }

    /// Moves the pointer onto `at`, as a mouse arrives, and returns the cursor it shows there.
    pub fn hover(&mut self, at: Pos2) -> egui::CursorIcon {
        for to in [at - vec2(5.0, 0.0), at] {
            self.harness
                .input_mut()
                .events
                .push(Event::PointerMoved(to));
            self.harness.run_steps(1);
        }
        self.settle();
        self.harness.output().platform_output.cursor_icon
    }

    /// Moves the pointer onto `at`, as a mouse arrives, and clicks there.
    pub fn click(&mut self, at: Pos2) {
        for to in [at - vec2(5.0, 0.0), at] {
            self.harness
                .input_mut()
                .events
                .push(Event::PointerMoved(to));
            self.harness.run_steps(1);
        }
        for pressed in [true, false] {
            let button = PointerButton::Primary;
            self.harness.input_mut().events.push(Event::PointerButton {
                pos: at,
                button,
                pressed,
                modifiers: Modifiers::NONE,
            });
            self.harness.run_steps(1);
        }
        self.settle();
    }

    /// Moves the pointer onto `at` and right-clicks there.
    pub fn right_click(&mut self, at: Pos2) {
        for to in [at - vec2(5.0, 0.0), at] {
            self.harness
                .input_mut()
                .events
                .push(Event::PointerMoved(to));
            self.harness.run_steps(1);
        }
        for pressed in [true, false] {
            let button = PointerButton::Secondary;
            self.harness.input_mut().events.push(Event::PointerButton {
                pos: at,
                button,
                pressed,
                modifiers: Modifiers::NONE,
            });
            self.harness.run_steps(1);
        }
        self.settle();
    }

    /// Presses at `from`, moves to `to` in steps with the button down, and lets go there.
    pub fn drag(&mut self, from: Pos2, to: Pos2) {
        let button = PointerButton::Primary;
        self.harness
            .input_mut()
            .events
            .push(Event::PointerMoved(from));
        self.harness.run_steps(1);
        self.harness.input_mut().events.push(Event::PointerButton {
            pos: from,
            button,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        self.harness.run_steps(1);
        for step in 1..=10 {
            self.harness.input_mut().events.push(Event::PointerMoved(
                from + (to - from) * (step as f32 / 10.0),
            ));
            self.harness.run_steps(1);
        }
        self.harness.input_mut().events.push(Event::PointerButton {
            pos: to,
            button,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        self.harness.run_steps(1);
        self.settle();
    }

    /// The mock's Gmail, as another device sees it; run its calls with [`Fixture::block_on`].
    pub fn gmail(&self) -> gmail::Client {
        gmail::Client::new(
            self.server.endpoints(),
            gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into()),
        )
    }

    /// Arms a gmail-mock fault (`/__mock/faults`), or with `None` clears every one.
    pub fn fault(&self, fault: Option<serde_json::Value>) {
        let url = self.server.url().join("/__mock/faults").expect("a URL");
        let client = reqwest::Client::new();
        let request = match fault {
            Some(fault) => client.post(url).json(&fault),
            None => client.delete(url),
        };
        self.block_on(request.send()).expect("the mock takes it");
    }

    /// The Gmail quota units the app has spent so far (`/__mock/quota`), but for `history.list`:
    /// the routine sync's look for new mail, which a busy machine can land in any window.
    pub fn units(&self) -> u64 {
        let url = self.server.url().join("/__mock/quota").expect("a URL");
        let quota: serde_json::Value = self
            .block_on(async { reqwest::get(url).await?.json().await })
            .expect("the mock answers");
        let history = quota["methods"]["history.list"]["units"]
            .as_u64()
            .unwrap_or_default();
        quota["units"].as_u64().expect("units") - history
    }

    /// Takes the mock away, as the network going does: what the app asks gets no answer.
    pub fn disconnect(&mut self) {
        self.runtime.block_on(self.server.disconnect());
    }

    /// Brings the mock back after [`Fixture::disconnect`], on the same port.
    pub fn reconnect(&mut self) {
        self.runtime
            .block_on(self.server.reconnect())
            .expect("the mock's port is free again");
    }

    pub fn mock_url(&self) -> url::Url {
        self.server.url()
    }

    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.runtime.block_on(future)
    }

    /// Syncs now, as the window regaining focus does.
    pub fn sync(&mut self) {
        self.harness.input_mut().focused = false;
        self.harness.run_steps(2);
        self.harness.input_mut().focused = true;
        self.settle();
    }

    pub fn subjects(&self) -> Vec<String> {
        self.mail()
            .inbox()
            .threads()
            .iter()
            .map(|thread| thread.subject.clone())
            .collect()
    }

    pub fn cursor(&self) -> String {
        self.mail()
            .inbox()
            .cursor()
            .map(|thread| thread.subject.clone())
            .unwrap_or_default()
    }
}
