//! Signing in and out, switching and adding accounts, what Exo opens on launch, and the
//! teardowns for a store that can't be opened, end to end:
//! the app on an in-process gmail-mock (its OAuth endpoints too; a `login_hint` signs in another
//! address on its mailbox), with refresh tokens in memory and a scripted browser.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use egui::{Event, Key, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use exo::{
    App, Screen, Start,
    desktop::{Desktop, Launcher, Opening, Placement},
    notifications::Notifier,
    sign_in::{Browser, SignIn},
    teardown::Reason,
};
use exo_auth::{Auth, ClientConfig, MemorySecrets, SecretStore};
use url::Url;

/// The mock's account.
const EMAIL: &str = "maya@brightmoor.co";
/// Another account on the mock's mailbox.
const BEA: &str = "bea@example.com";
const CTRL: Modifiers = Modifiers::CTRL;
const CLIENT_ID: &str = "exo-test.apps.googleusercontent.com";
const CLIENT_SECRET: &str = "GOCSPX-test";
/// "Sign in with Google" at 1280×800: the column (388px) centers in the 759px below the tab
/// strip, and the button sits 140px into its card.
const BUTTON: Pos2 = pos2(640.0, 440.0);
/// A 60 fps frame.
const FRAME: f64 = 1.0 / 60.0;

/// A Mac Exo runs on: a gmail-mock, a state folder and the Keychain (in memory).
struct Device {
    runtime: tokio::runtime::Runtime,
    server: gmail_mock::MockServer,
    dir: PathBuf,
    secrets: Arc<MemorySecrets>,
}

impl Drop for Device {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Device {
    /// A fresh device; `name` keeps concurrent tests' folders apart.
    fn new(name: &str) -> Self {
        Self::on(name, gmail_mock::Config::default())
    }

    /// A fresh device whose Google account has a profile photo of its own.
    fn with_photo(name: &str) -> Self {
        Self::on(
            name,
            gmail_mock::Config {
                photo: true,
                ..gmail_mock::Config::default()
            },
        )
    }

    /// A fresh device on a mock made from `config`.
    fn on(name: &str, config: gmail_mock::Config) -> Self {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let server = runtime
            .block_on(gmail_mock::MockServer::start(config))
            .expect("mock starts");
        let dir = std::env::temp_dir().join(format!("exo-sign-in-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Self {
            runtime,
            server,
            dir,
            secrets: Arc::default(),
        }
    }

    fn client(&self) -> ClientConfig {
        ClientConfig::new(CLIENT_ID, CLIENT_SECRET, self.server.endpoints())
    }

    fn auth(&self) -> Auth {
        Auth::open(self.client(), &self.dir, self.secrets.clone()).expect("account list")
    }

    /// Signs the mock's account in behind Exo's back, as an earlier run would have.
    fn sign_in(&self) {
        self.sign_in_as(None);
    }

    /// Signs in the account Google offers for `hint` (the mock's own without one; any other
    /// address shares its mailbox), behind Exo's back.
    fn sign_in_as(&self, hint: Option<&str>) {
        let auth = self.auth();
        self.runtime
            .block_on(auth.sign_in(hint, |url| approving()(url)))
            .expect("signs in");
    }

    /// Where Exo keeps the accounts' stores.
    fn stores(&self) -> PathBuf {
        self.dir.join("stores")
    }

    /// The files of `email`'s store.
    fn store_files(&self, email: &str) -> Vec<String> {
        let names = std::fs::read_dir(self.stores()).map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        });
        names
            .unwrap_or_else(|_| Vec::new())
            .into_iter()
            .filter(|name: &String| name.starts_with(&format!("{email}.")))
            .collect()
    }

    /// Fails Gmail's POSTs (triage, drafts) with 503s from now on.
    fn fail_writes(&self) {
        self.fault(serde_json::json!({ "method": "POST", "path": "/gmail/v1/users/me/", "status": 503, "times": 100_000 }));
    }

    /// Arms a gmail-mock fault (`/__mock/faults`).
    fn fault(&self, fault: serde_json::Value) {
        let url = self.server.url().join("/__mock/faults").unwrap();
        self.runtime
            .block_on(async { reqwest::Client::new().post(url).json(&fault).send().await })
            .expect("fault armed");
    }

    /// Where Exo keeps `email`'s profile photo.
    fn photo_file(&self, email: &str) -> PathBuf {
        self.dir.join("profile-pictures").join(email)
    }

    /// Whether Google still honors `grant`.
    fn honors(&self, grant: &str) -> bool {
        let oauth = gmail::oauth::OAuthClient::new(
            self.server.endpoints().oauth,
            CLIENT_ID,
            Some(CLIENT_SECRET.into()),
        );
        self.runtime.block_on(oauth.refresh(grant)).is_ok()
    }

    /// Revokes the stored grant at the mock, as the user would from their Google account.
    fn revoke_at_google(&self) {
        let grant = self.secrets.get(EMAIL).unwrap().expect("a grant");
        let oauth = gmail::oauth::OAuthClient::new(
            self.server.endpoints().oauth,
            CLIENT_ID,
            Some(CLIENT_SECRET.into()),
        );
        self.runtime
            .block_on(oauth.revoke(&grant))
            .expect("revoked");
    }

    /// Launches Exo, showing consent in `browser`.
    fn launch(&self, browser: Browser) -> Harness<'static, App> {
        let (client, dir, secrets) = (self.client(), self.dir.clone(), self.secrets.clone());
        let open = move || Auth::open(client.clone(), &dir, secrets.clone());
        self.launch_with(
            SignIn::new(open, &self.dir.join("stores"), browser).expect("sign-in runtime"),
        )
    }

    fn launch_with(&self, sign_in: SignIn) -> Harness<'static, App> {
        self.launch_at(
            sign_in,
            exo::Clock::fixed(gmail_mock::Config::default().now),
        )
    }

    /// Launches Exo, dated by `clock`.
    fn launch_at(&self, sign_in: SignIn, clock: exo::Clock) -> Harness<'static, App> {
        let assets = exo::assets_dir();
        let mut harness = Harness::builder()
            .with_size([1280.0, 800.0])
            .with_os(egui::os::OperatingSystem::Mac)
            .with_step_dt(FRAME as f32)
            .build_eframe(move |cc| {
                App::new(cc, &assets, Start::SignIn(sign_in), clock).expect("launches")
            });
        harness.run_steps(2);
        harness
    }
}

/// A browser that approves: it loads the consent URL, which the mock answers with a redirect to
/// Exo's loopback listener.
fn approving() -> Browser {
    browser(Clone::clone)
}

/// A browser that loads what `visit` makes of the consent URL.
fn browser(visit: impl Fn(&Url) -> Url + Send + Sync + 'static) -> Browser {
    Arc::new(move |url| {
        let target = visit(url);
        tokio::spawn(async move {
            let _ = reqwest::get(target).await;
        });
        Ok(())
    })
}

/// A browser where the user picks `email` at Google (any address the mock signs in as).
fn choosing(email: &'static str) -> Browser {
    browser(move |url| {
        let mut visit = url.clone();
        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .filter(|(key, _)| key != "login_hint")
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        visit
            .query_pairs_mut()
            .clear()
            .extend_pairs(pairs)
            .append_pair("login_hint", email);
        visit
    })
}

/// A browser the user never comes back from.
fn abandoned() -> Browser {
    Arc::new(|_| Ok(()))
}

fn param(url: &Url, name: &str) -> String {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default()
}

/// Runs frames until `done` holds.
fn run_until(harness: &mut Harness<'_, App>, what: &str, done: impl Fn(&App) -> bool) {
    let started = Instant::now();
    while !done(harness.state()) {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "timed out waiting for {what}"
        );
        harness.run_steps(1);
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn press(harness: &mut Harness<'_, App>, key: Key) {
    press_with(harness, key, Modifiers::NONE);
}

fn press_with(harness: &mut Harness<'_, App>, key: Key, modifiers: Modifiers) {
    let event = |pressed| Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    };
    harness.input_mut().events.push(event(true));
    harness.run_steps(1);
    harness.input_mut().events.push(event(false));
    harness.run_steps(1);
}

fn click(harness: &mut Harness<'_, App>, at: Pos2) {
    for to in [at - vec2(5.0, 0.0), at] {
        harness.input_mut().events.push(Event::PointerMoved(to));
        harness.run_steps(1);
    }
    for pressed in [true, false] {
        harness.input_mut().events.push(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        harness.run_steps(1);
    }
}

fn type_text(harness: &mut Harness<'_, App>, text: &str) {
    for ch in text.chars() {
        harness.input_mut().events.push(Event::Text(ch.to_string()));
        harness.run_steps(1);
    }
}

/// `Sign Out` from the palette.
fn sign_out(harness: &mut Harness<'_, App>) {
    press_with(harness, Key::K, Modifiers::MAC_CMD.plus(Modifiers::COMMAND));
    type_text(harness, "sign out");
    press(harness, Key::Enter);
}

fn log_in(app: &App) -> Option<&exo::LogIn> {
    match app.screen() {
        Screen::LogIn(log_in) => Some(log_in),
        Screen::SignedOut(_) | Screen::Unopened(_) | Screen::Mail(_) => None,
    }
}

fn inbox_shows(app: &App) -> bool {
    app.mail()
        .is_some_and(|mail| !mail.inbox().threads().is_empty())
}

/// The address whose mail is on screen.
fn on_screen(app: &App) -> Option<&str> {
    app.mail().map(|mail| mail.account().email())
}

/// The addresses of the accounts open off screen, in the list's order.
fn off_screen(app: &App) -> Vec<&str> {
    let mut open: Vec<&str> = app.off_screen().map(exo::account::Account::email).collect();
    open.sort_by_key(|email| [EMAIL, BEA].iter().position(|listed| listed == email));
    open
}

/// Waits, without frames, for `done`: an account off screen needs none.
fn wait_for(what: &str, seconds: u64, done: impl Fn() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(
            started.elapsed() < Duration::from_secs(seconds),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Runs frames until nothing is in flight.
fn settle(harness: &mut Harness<'_, App>) {
    run_until(harness, "quiet", App::is_idle);
    harness.run_steps(5);
}

#[test]
fn signing_in_opens_the_inbox() {
    let device = Device::new("opens");
    let mut app = device.launch(approving());
    let screen = log_in(app.state()).expect("no account yet: the sign-in screen");
    assert!(!screen.is_loading() && screen.footer().is_none());

    click(&mut app, BUTTON);
    run_until(&mut app, "the inbox", inbox_shows);
    assert_eq!(app.state().mail().unwrap().account().email(), EMAIL);
    assert!(
        device
            .secrets
            .get(EMAIL)
            .unwrap()
            .is_some_and(|grant| grant.starts_with("1//")),
        "the grant is kept"
    );
    assert_eq!(
        device.auth().accounts()[0].email,
        EMAIL,
        "and the account listed"
    );
}

#[test]
fn a_first_sign_in_on_this_mac_is_welcomed() {
    let device = Device::new("welcome");
    let mut app = device.launch(approving());
    click(&mut app, BUTTON);
    run_until(&mut app, "the inbox", inbox_shows);
    run_until(&mut app, "the splash's photo", App::is_idle);
    let mail = app.state().mail().unwrap();
    assert!(
        mail.welcome_covers() && mail.account().device_settings().splash_pending,
        "the welcome splash, till it's been seen"
    );
    // The keys are the splash's: 2 doesn't open the week.
    press(&mut app, Key::Num2);
    assert_eq!(app.state().mail().unwrap().calendar_week(), None);

    // Five seconds on, with the inbox in, it gives way to the first-run flow: the discovery
    // questions, in Snow, their keys their own.
    app.run_steps((5.1 / FRAME) as usize);
    let mail = app.state().mail().unwrap();
    assert!(!mail.welcome_covers() && !mail.account().device_settings().splash_pending);
    assert!(mail.discovery_open() && !mail.account().settings().do_show_tutorial);
    press(&mut app, Key::Num2);
    assert_eq!(app.state().mail().unwrap().calendar_week(), None);

    // The splash only the first time, and the questions too: quit in them, it's the mail.
    drop(app);
    let mut app = device.launch(abandoned());
    run_until(&mut app, "the inbox", inbox_shows);
    let mail = app.state().mail().unwrap();
    assert!(!mail.welcome_covers() && !mail.discovery_open() && !mail.account_setup_open());
}

#[test]
fn enter_signs_in() {
    let device = Device::new("enter");
    let mut app = device.launch(approving());
    press(&mut app, Key::Enter);
    run_until(&mut app, "the inbox", inbox_shows);
}

#[test]
fn launch_opens_the_first_account() {
    let device = Device::new("launch");
    device.sign_in();
    let mut app = device.launch(abandoned());
    assert!(app.state().mail().is_some(), "straight to the inbox");
    run_until(&mut app, "the inbox", inbox_shows);
}

#[test]
fn a_refused_grant_asks_to_sign_in_again() {
    let device = Device::new("refused");
    device.sign_in();
    device.revoke_at_google();
    let mut app = device.launch(approving());
    // The first sync's token refresh learns Google's answer.
    run_until(
        &mut app,
        "Almost there…",
        |app| matches!(app.screen(), Screen::SignedOut(alert) if alert.email() == EMAIL),
    );

    // Five seconds on, the sign-in screen.
    run_until(&mut app, "the alert's photo", App::is_idle);
    app.run_steps((4.9 / FRAME) as usize);
    assert!(matches!(app.state().screen(), Screen::SignedOut(_)));
    app.run_steps((0.2 / FRAME) as usize);
    assert!(
        log_in(app.state()).is_some(),
        "the sign-in screen after 5 s"
    );

    press(&mut app, Key::Enter);
    run_until(&mut app, "the inbox", inbox_shows);
    assert_eq!(app.state().mail().unwrap().account().email(), EMAIL);
}

#[test]
fn the_alerts_action_signs_in_again() {
    let device = Device::new("alert-key");
    device.sign_in();
    device.secrets.delete(EMAIL).unwrap();
    let mut app = device.launch(abandoned());
    run_until(&mut app, "Almost there…", |app| {
        matches!(app.screen(), Screen::SignedOut(_))
    });
    press(&mut app, Key::Escape);
    assert!(log_in(app.state()).is_some(), "Escape performs Sign In");
}

/// A Keychain whose item this build may not read (the user clicked Deny), though it may write.
struct Denied(Arc<MemorySecrets>);

impl SecretStore for Denied {
    fn get(&self, _account: &str) -> Result<Option<String>, exo_auth::Error> {
        Err(exo_auth::Error::Secrets(
            "The user name or passphrase you entered is not correct.".into(),
        ))
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), exo_auth::Error> {
        self.0.set(account, secret)
    }

    fn delete(&self, account: &str) -> Result<(), exo_auth::Error> {
        self.0.delete(account)
    }
}

#[test]
fn a_denied_keychain_asks_to_sign_in_again() {
    let device = Device::new("denied");
    device.sign_in();
    let before = device.secrets.get(EMAIL).unwrap();
    let (client, dir, secrets) = (device.client(), device.dir.clone(), device.secrets.clone());
    let open = move || Auth::open(client.clone(), &dir, Denied(secrets.clone()));
    let mut app =
        device.launch_with(SignIn::new(open, &device.dir.join("stores"), approving()).unwrap());
    // Straight to "Almost there…": without the grant there's no mail to show.
    assert!(matches!(app.state().screen(), Screen::SignedOut(alert) if alert.email() == EMAIL));

    press(&mut app, Key::Enter);
    assert!(
        log_in(app.state()).is_some(),
        "Sign In leads to the sign-in screen"
    );
    press(&mut app, Key::Enter);
    run_until(&mut app, "the inbox", inbox_shows);
    assert_ne!(
        device.secrets.get(EMAIL).unwrap(),
        before,
        "signing in again writes a fresh grant"
    );
}

#[test]
fn the_spinner_gives_up_after_30_seconds() {
    let device = Device::new("spinner");
    let mut app = device.launch(abandoned());
    press(&mut app, Key::Enter);
    assert!(log_in(app.state()).unwrap().is_loading());
    app.run_steps((29.0 / FRAME) as usize);
    assert!(log_in(app.state()).unwrap().is_loading());
    app.run_steps((1.5 / FRAME) as usize);
    let screen = log_in(app.state()).unwrap();
    assert!(
        !screen.is_loading() && screen.footer().is_none(),
        "back to the button, silently"
    );
}

#[test]
fn declining_consent_says_nothing() {
    let device = Device::new("declined");
    let mut app = device.launch(browser(|url| {
        Url::parse(&format!(
            "{}/?error=access_denied&state={}",
            param(url, "redirect_uri"),
            param(url, "state")
        ))
        .unwrap()
    }));
    press(&mut app, Key::Enter);
    run_until(&mut app, "the button", |app| {
        log_in(app).is_some_and(|screen| !screen.is_loading())
    });
    assert_eq!(log_in(app.state()).unwrap().footer(), None);
}

#[test]
fn unticked_permissions_are_explained() {
    let device = Device::new("scopes");
    // The user unticks Calendar on Google's consent screen.
    let mut app = device.launch(browser(|url| {
        let mut visit = url.clone();
        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(key, value)| {
                let value = if key == "scope" {
                    value.replace(gmail::oauth::scopes::CALENDAR, "")
                } else {
                    value.into_owned()
                };
                (key.into_owned(), value)
            })
            .collect();
        visit.query_pairs_mut().clear().extend_pairs(pairs);
        visit
    }));
    press(&mut app, Key::Enter);
    run_until(&mut app, "the error", |app| {
        log_in(app).is_some_and(|screen| screen.footer().is_some())
    });
    assert_eq!(
        log_in(app.state()).unwrap().footer(),
        Some(
            "Please select all permissions when signing in with Google.\nExo requires these to manage your email."
        )
    );
    assert!(app.state().mail().is_none());
}

#[test]
fn without_an_oauth_client_it_says_where_to_put_one() {
    let device = Device::new("no-client");
    let path =
        PathBuf::from("/Users/someone/Library/Application Support/Exo/google-oauth-client.json");
    let missing = path.clone();
    let open = move || {
        Err(exo_auth::Error::MissingClient {
            path: missing.clone(),
            from_env: false,
        })
    };
    let mut app = device.launch_with(SignIn::new(open, &device.dir, approving()).unwrap());
    let expected = format!(
        "Please save your Google OAuth client (Desktop app) as {}.",
        path.display()
    );
    assert_eq!(
        log_in(app.state()).unwrap().footer(),
        Some(expected.as_str())
    );

    // Signing in looks for the file again, and still finds none.
    press(&mut app, Key::Enter);
    let screen = log_in(app.state()).unwrap();
    assert!(!screen.is_loading());
    assert_eq!(screen.footer(), Some(expected.as_str()));
}

#[test]
fn signing_out_forgets_the_account_and_deletes_its_mail() {
    let device = Device::new("sign-out");
    device.sign_in();
    let grant = device.secrets.get(EMAIL).unwrap().expect("a grant");
    let mut app = device.launch(abandoned());
    run_until(&mut app, "the inbox", inbox_shows);
    assert!(!device.store_files(EMAIL).is_empty());

    sign_out(&mut app);
    run_until(&mut app, "the sign-in screen", |app| log_in(app).is_some());
    let screen = log_in(app.state()).unwrap();
    assert!(
        !screen.is_loading() && screen.footer().is_none(),
        "the plain sign-in screen"
    );
    assert_eq!(
        device.secrets.get(EMAIL).unwrap(),
        None,
        "the Keychain item is gone"
    );
    assert!(device.auth().accounts().is_empty(), "and so is the account");
    assert_eq!(
        device.store_files(EMAIL),
        Vec::<String>::new(),
        "and its mail"
    );
    assert!(device.honors(&grant), "Google's grant is left alone");

    // Signing in again starts afresh.
    let mut app = device.launch(approving());
    press(&mut app, Key::Enter);
    run_until(&mut app, "the inbox", inbox_shows);
}

#[test]
fn signing_out_with_unsent_changes_asks_first() {
    let device = Device::new("sign-out-unsent");
    device.sign_in();
    let mut app = device.launch(abandoned());
    run_until(&mut app, "the inbox", inbox_shows);
    device.fail_writes();
    let done = app
        .state()
        .mail()
        .unwrap()
        .inbox()
        .cursor()
        .expect("a cursor")
        .id
        .clone();
    press(&mut app, Key::E);

    let alert = |app: &App| {
        app.mail().and_then(|mail| {
            mail.alert()
                .map(|(title, text)| (title.to_owned(), text.to_owned()))
        })
    };
    sign_out(&mut app);
    run_until(&mut app, "the alert", |app| alert(app).is_some());
    assert_eq!(
        alert(app.state()).unwrap(),
        (
            "One last thing…".to_owned(),
            "Exo is still uploading 1 change. Ignore and sign out anyway?".to_owned()
        )
    );
    press(&mut app, Key::Escape);
    assert!(
        app.state()
            .mail()
            .is_some_and(|mail| mail.alert().is_none()),
        "Cancel stays signed in"
    );
    assert!(device.secrets.get(EMAIL).unwrap().is_some());

    sign_out(&mut app);
    run_until(&mut app, "the alert", |app| alert(app).is_some());
    press(&mut app, Key::Enter);
    run_until(&mut app, "the sign-in screen", |app| log_in(app).is_some());
    assert!(device.auth().accounts().is_empty());
    assert_eq!(device.store_files(EMAIL), Vec::<String>::new());
    // The change was dropped with the account, not sent on its way out.
    let gmail = gmail::Client::new(
        device.server.endpoints(),
        gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into()),
    );
    let thread = device
        .runtime
        .block_on(
            gmail
                .gmail()
                .thread(&done, gmail::Format::Minimal, &[])
                .into_future(),
        )
        .expect("the thread");
    assert!(
        thread
            .messages
            .iter()
            .any(|message| message.label_ids.iter().any(|label| label == "INBOX")),
        "still in Gmail's Inbox"
    );
}

#[test]
fn signing_out_opens_the_next_account() {
    let device = Device::new("sign-out-next");
    device.sign_in_as(Some(BEA));
    device.sign_in();
    let mut app = device.launch(abandoned());
    run_until(&mut app, "the inbox", inbox_shows);
    assert_eq!(on_screen(app.state()), Some(EMAIL), "the account used last");

    sign_out(&mut app);
    run_until(&mut app, "the other account", |app| {
        on_screen(app) == Some(BEA)
    });
    run_until(&mut app, "its inbox", inbox_shows);
    let accounts: Vec<String> = device
        .auth()
        .accounts()
        .into_iter()
        .map(|account| account.email)
        .collect();
    assert_eq!(accounts, [BEA]);
    assert_eq!(
        device
            .auth()
            .default_account()
            .map(|account| account.email)
            .as_deref(),
        Some(BEA),
        "now the one used last"
    );
    assert_eq!(device.secrets.get(EMAIL).unwrap(), None);
    assert!(
        device.secrets.get(BEA).unwrap().is_some(),
        "the other account keeps its grant"
    );
    assert_eq!(device.store_files(EMAIL), Vec::<String>::new());
    assert!(!device.store_files(BEA).is_empty(), "and its mail");
}

#[test]
fn control_and_a_number_switch_accounts_and_launch_opens_the_last_used() {
    let device = Device::new("switch-keys");
    device.sign_in();
    device.sign_in_as(Some(BEA));
    let mut app = device.launch(abandoned());
    run_until(&mut app, "the inbox", inbox_shows);
    assert_eq!(
        on_screen(app.state()),
        Some(BEA),
        "the account that signed in last"
    );
    let listed: Vec<String> = app
        .state()
        .mail()
        .unwrap()
        .accounts()
        .iter()
        .map(|account| account.email.clone())
        .collect();
    assert_eq!(
        listed,
        [EMAIL, BEA],
        "the mail knows the device's accounts, in order"
    );

    press_with(&mut app, Key::Num1, CTRL);
    run_until(&mut app, "the first account", |app| {
        on_screen(app) == Some(EMAIL)
    });
    run_until(&mut app, "its inbox", inbox_shows);
    press_with(&mut app, Key::Num3, CTRL);
    settle(&mut app);
    assert_eq!(
        on_screen(app.state()),
        Some(EMAIL),
        "there's no third account"
    );

    // Relaunching opens the one switched to.
    drop(app);
    let mut app = device.launch(abandoned());
    run_until(&mut app, "the inbox", inbox_shows);
    assert_eq!(on_screen(app.state()), Some(EMAIL));
    press_with(&mut app, Key::Num2, CTRL);
    run_until(&mut app, "the second account", |app| {
        on_screen(app) == Some(BEA)
    });
}

#[test]
fn the_accounts_palette_switches_and_adds() {
    let device = Device::new("switch-palette");
    device.sign_in();
    device.sign_in_as(Some(BEA));
    let mut app = device.launch(abandoned());
    run_until(&mut app, "the inbox", inbox_shows);
    press_with(
        &mut app,
        Key::K,
        Modifiers::MAC_CMD.plus(Modifiers::COMMAND),
    );
    type_text(&mut app, "switch accounts");
    press(&mut app, Key::Enter);
    // The first row, Maya's.
    press(&mut app, Key::Enter);
    run_until(&mut app, "the first account", |app| {
        on_screen(app) == Some(EMAIL)
    });

    press_with(
        &mut app,
        Key::K,
        Modifiers::MAC_CMD.plus(Modifiers::COMMAND),
    );
    type_text(&mut app, "add account");
    press(&mut app, Key::Enter);
    let screen = log_in(app.state()).expect("Add Account shows the sign-in screen");
    assert_eq!(
        screen.back(),
        Some(&exo::log_in::Back::Previous(EMAIL.into()))
    );
    assert_eq!(screen.hint(), None);
}

#[test]
fn adding_an_account_signs_it_in_or_goes_back() {
    let device = Device::new("add-account");
    device.sign_in();
    let mut app = device.launch(choosing(BEA));
    run_until(&mut app, "the inbox", inbox_shows);
    assert!(app.state().mail().unwrap().accounts().len() == 1);

    press_with(&mut app, Key::Num0, CTRL);
    assert!(log_in(app.state()).is_some_and(|screen| screen.back() == Some(&exo::log_in::Back::Previous(EMAIL.into()))));
    press(&mut app, Key::Escape);
    run_until(&mut app, "back to Maya", |app| {
        on_screen(app) == Some(EMAIL)
    });

    press_with(&mut app, Key::Num0, CTRL);
    press(&mut app, Key::Enter);
    run_until(&mut app, "Bea's mail", |app| on_screen(app) == Some(BEA));
    let accounts: Vec<String> = device
        .auth()
        .accounts()
        .into_iter()
        .map(|account| account.email)
        .collect();
    assert_eq!(accounts, [EMAIL, BEA], "added at the end");
    assert_eq!(app.state().mail().unwrap().accounts().len(), 2);
    assert!(!device.store_files(BEA).is_empty(), "its own store");
}

#[test]
fn switching_accounts_keeps_see_your_weeks_week() {
    let device = Device::new("switch-week");
    device.sign_in();
    device.sign_in_as(Some(BEA));
    let mut app = device.launch(abandoned());
    run_until(&mut app, "the inbox", inbox_shows);
    let (from, to, key) = if on_screen(app.state()) == Some(EMAIL) {
        (EMAIL, BEA, Key::Num2)
    } else {
        (BEA, EMAIL, Key::Num1)
    };
    press(&mut app, Key::Num2);
    settle(&mut app);
    press(&mut app, Key::Equals);
    settle(&mut app);
    let mail = app.state().mail().unwrap();
    let week = mail.calendar_week_days(mail.calendar_week().expect("See Your Week"));

    press_with(&mut app, key, CTRL);
    run_until(&mut app, "the other account", |app| {
        on_screen(app) == Some(to)
    });
    let mail = app.state().mail().unwrap();
    let kept = mail
        .calendar_week()
        .expect("its See Your Week, on the same week");
    assert_eq!(mail.calendar_week_days(kept), week);
    assert_eq!(
        kept, week[3],
        "the middle of the week, three days after its start"
    );
    assert_ne!(from, to);
}

#[test]
fn a_signed_out_account_offers_to_sign_in_as_it() {
    let device = Device::new("sign-in-as");
    device.sign_in();
    device.sign_in_as(Some(BEA));
    device.auth().mark_signed_out(BEA).unwrap();
    let mut app = device.launch(approving());
    run_until(&mut app, "the inbox", inbox_shows);
    assert_eq!(
        on_screen(app.state()),
        Some(EMAIL),
        "the account used last is signed out: the first one that isn't"
    );

    press_with(&mut app, Key::Num2, CTRL);
    let screen = log_in(app.state()).expect("the sign-in screen");
    assert_eq!(screen.hint(), Some(BEA));
    assert_eq!(
        screen.back(),
        Some(&exo::log_in::Back::Previous(EMAIL.into()))
    );
    // "Use another account→" brings back the plain button.
    click(&mut app, pos2(640.0, 549.0));
    assert_eq!(log_in(app.state()).unwrap().hint(), None);
    press(&mut app, Key::Escape);
    run_until(&mut app, "back to Maya", |app| {
        on_screen(app) == Some(EMAIL)
    });

    // Signing in as it (Google offers it first, `login_hint`).
    press_with(&mut app, Key::Num2, CTRL);
    press(&mut app, Key::Enter);
    run_until(&mut app, "Bea's mail", |app| on_screen(app) == Some(BEA));
    assert!(
        device
            .auth()
            .accounts()
            .iter()
            .all(|account| !account.signed_out),
        "signed in again"
    );
}

#[test]
fn after_almost_there_the_back_button_leads_to_a_signed_in_account() {
    let device = Device::new("refused-back");
    device.sign_in_as(Some(BEA));
    device.sign_in();
    device.revoke_at_google();
    let mut app = device.launch(abandoned());
    run_until(&mut app, "Almost there…", |app| {
        matches!(app.screen(), Screen::SignedOut(_))
    });
    assert!(
        device
            .auth()
            .accounts()
            .iter()
            .any(|account| account.email == EMAIL && account.signed_out),
        "listed as signed out"
    );
    press(&mut app, Key::Enter);
    let screen = log_in(app.state()).expect("the sign-in screen");
    assert_eq!(
        screen.back(),
        Some(&exo::log_in::Back::SignedIn(BEA.into()))
    );
    press(&mut app, Key::Escape);
    run_until(&mut app, "Bea's mail", |app| on_screen(app) == Some(BEA));
}

/// Writes a new message to Priya and sends it: held for Undo.
fn send_new_message(app: &mut Harness<'_, App>, subject: &str) {
    press(app, Key::C);
    settle(app);
    type_text(app, "priya@brightmoor.co");
    settle(app);
    for (key, text) in [
        (Key::Enter, None),
        (Key::Tab, Some(subject)),
        (Key::Tab, Some("Hi there")),
    ] {
        press(app, key);
        settle(app);
        if let Some(text) = text {
            type_text(app, text);
            settle(app);
        }
    }
    press_with(app, Key::Enter, Modifiers::MAC_CMD.plus(Modifiers::COMMAND));
    settle(app);
}

#[test]
fn switching_keeps_both_accounts_open_and_a_held_send_goes_on_time() {
    let device = Device::new("switch-held");
    device.sign_in_as(Some(BEA));
    device.sign_in();
    let mut app = device.launch(abandoned());
    run_until(&mut app, "the inbox", inbox_shows);
    assert_eq!(
        off_screen(app.state()),
        [BEA],
        "Bea opened at launch, off screen"
    );
    send_new_message(&mut app, "Held across a switch");
    press_with(&mut app, Key::Num1, CTRL);
    run_until(&mut app, "Bea's mail", |app| on_screen(app) == Some(BEA));
    assert_eq!(off_screen(app.state()), [EMAIL], "Maya's mail stays open");

    // Back within the hold, it's the same mail: Undo still takes the send back.
    press_with(&mut app, Key::Num2, CTRL);
    run_until(&mut app, "Maya's mail", |app| on_screen(app) == Some(EMAIL));
    assert_eq!(off_screen(app.state()), [BEA], "and Bea's in turn");
    press(&mut app, Key::Z);
    settle(&mut app);
    assert!(
        app.state().mail().unwrap().compose_pane_open(),
        "the draft is back"
    );

    // Away again, the send goes out on time, with no frame, and Maya's mail stays open.
    press_with(
        &mut app,
        Key::Enter,
        Modifiers::MAC_CMD.plus(Modifiers::COMMAND),
    );
    settle(&mut app);
    press_with(&mut app, Key::Num1, CTRL);
    run_until(&mut app, "Bea's mail", |app| on_screen(app) == Some(BEA));
    wait_for("the held send to go", 60, || {
        sent(&device, "Held across a switch") > 0
    });
    assert_eq!(sent(&device, "Held across a switch"), 1, "sent once");
    assert_eq!(off_screen(app.state()), [EMAIL]);
}

#[test]
fn the_settings_menus_sign_out_signs_out() {
    let device = Device::new("sign-out-menu");
    device.sign_in();
    let mut app = device.launch(abandoned());
    run_until(&mut app, "the inbox", inbox_shows);
    press_with(
        &mut app,
        Key::K,
        Modifiers::MAC_CMD.plus(Modifiers::COMMAND),
    );
    type_text(&mut app, "settings");
    press(&mut app, Key::Enter);
    run_until(&mut app, "the settings menu", |app| {
        app.mail().is_some_and(exo::Mail::settings_visible)
    });
    // `Sign Out` at the bottom of the menu, scrolled to its end at 1280×800.
    app.input_mut()
        .events
        .push(Event::PointerMoved(pos2(1000.0, 500.0)));
    app.input_mut().events.push(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -800.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    app.run_steps(30);
    click(&mut app, pos2(1000.0, 707.0));
    run_until(&mut app, "the sign-in screen", |app| log_in(app).is_some());
    assert!(device.auth().accounts().is_empty());
}

/// Presses at `from`, moves to `to` in steps, and releases there.
fn drag(harness: &mut Harness<'_, App>, from: Pos2, to: Pos2) {
    harness.input_mut().events.push(Event::PointerMoved(from));
    harness.run_steps(1);
    harness.input_mut().events.push(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(1);
    for step in 1..=10 {
        harness
            .input_mut()
            .events
            .push(Event::PointerMoved(from + (to - from) * step as f32 / 10.0));
        harness.run_steps(1);
    }
    harness.input_mut().events.push(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(2);
}

fn listed(device: &Device) -> Vec<(String, Option<String>)> {
    device
        .auth()
        .accounts()
        .into_iter()
        .map(|account| (account.email, account.color))
        .collect()
}

#[test]
fn the_accounts_dialog_reorders_colors_signs_out_and_adds() {
    let device = Device::new("accounts-dialog");
    device.sign_in();
    device.sign_in_as(Some(BEA));
    let mut app = device.launch(abandoned());
    run_until(&mut app, "the inbox", inbox_shows);
    press_with(
        &mut app,
        Key::K,
        Modifiers::MAC_CMD.plus(Modifiers::COMMAND),
    );
    type_text(&mut app, "account settings");
    press(&mut app, Key::Enter);
    settle(&mut app);
    assert!(app.state().mail().unwrap().settings_dialog_open());

    // Two accounts at 1280×800: the card's top is at 323, rows 44px apart from 384; the drag
    // handles at x 761, the swatches at 877, `Sign out` around 716.
    drag(&mut app, pos2(761.0, 443.0), pos2(761.0, 395.0));
    settle(&mut app);
    assert_eq!(
        listed(&device),
        [(BEA.to_owned(), None), (EMAIL.to_owned(), None)],
        "Bea dragged above Maya"
    );
    assert_eq!(
        app.state().mail().unwrap().accounts()[0].email,
        BEA,
        "and the mail knows"
    );

    click(&mut app, pos2(877.0, 399.0));
    // The picker's first color, thistle.
    click(&mut app, pos2(790.0, 426.0));
    settle(&mut app);
    assert_eq!(
        listed(&device)[0],
        (BEA.to_owned(), Some("thistle".to_owned()))
    );
    // The picker stays open after a pick. Its keys take at once, and Escape goes
    // back to the color it opened on, Bea's default, closing only the picker.
    let color = |app: &mut Harness<'_, App>, key| {
        press(app, key);
        settle(app);
        listed(&device)[0].1.clone()
    };
    assert_eq!(color(&mut app, Key::ArrowRight).as_deref(), Some("orchid"));
    assert_eq!(
        color(&mut app, Key::ArrowDown).as_deref(),
        Some("grass"),
        "the row under"
    );
    assert_eq!(color(&mut app, Key::Escape).as_deref(), Some("wine"));
    assert!(app.state().mail().unwrap().settings_dialog_open());
    click(&mut app, pos2(877.0, 399.0));
    settle(&mut app);
    assert_eq!(color(&mut app, Key::ArrowLeft).as_deref(), Some("leaf"));
    assert_eq!(
        color(&mut app, Key::Enter).as_deref(),
        Some("leaf"),
        "Enter keeps it"
    );
    assert_eq!(
        color(&mut app, Key::ArrowLeft).as_deref(),
        Some("leaf"),
        "and closes the picker"
    );

    // Maya, now second, isn't on screen: she signs out at once.
    click(&mut app, pos2(716.0, 444.0));
    run_until(&mut app, "Maya to go", |app| {
        app.mail().is_some_and(|mail| mail.accounts().len() == 1)
    });
    assert_eq!(on_screen(app.state()), Some(BEA), "Bea stays on screen");
    assert_eq!(device.secrets.get(EMAIL).unwrap(), None);
    assert_eq!(device.store_files(EMAIL), Vec::<String>::new());

    // The footer adds one: one row now, so it's 44px higher (and the card 22px lower).
    click(&mut app, pos2(450.0, 460.0));
    let screen = log_in(app.state()).expect("the sign-in screen");
    assert_eq!(
        screen.back(),
        Some(&exo::log_in::Back::Previous(BEA.into()))
    );
}

/// How many of the mock's sent messages have `subject`.
fn sent(device: &Device, subject: &str) -> usize {
    let gmail = gmail::Client::new(
        device.server.endpoints(),
        gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into()),
    );
    let query = gmail::ListQuery {
        q: Some(format!("subject:\"{subject}\" in:sent")),
        ..gmail::ListQuery::default()
    };
    device
        .runtime
        .block_on(gmail.gmail().messages(&query).into_future())
        .expect("messages.list")
        .messages
        .len()
}

/// The mock's messages matching `query`, as `(id, snippet)`.
fn gmail_messages(device: &Device, query: &str) -> Vec<(String, String)> {
    let gmail = gmail::Client::new(
        device.server.endpoints(),
        gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into()),
    );
    let list = gmail::ListQuery {
        q: Some(query.into()),
        ..gmail::ListQuery::default()
    };
    let messages = device
        .runtime
        .block_on(gmail.gmail().messages(&list).into_future())
        .expect("messages.list")
        .messages;
    messages
        .into_iter()
        .map(|message| {
            let id = message.id.expect("an id");
            let full = device
                .runtime
                .block_on(
                    gmail
                        .gmail()
                        .message(&id, gmail::Format::Full, &[])
                        .into_future(),
                )
                .expect("messages.get");
            (id, full.snippet.unwrap_or_default())
        })
        .collect()
}

#[test]
fn switching_away_saves_a_drafts_last_keystrokes() {
    let device = Device::new("switch-draft");
    device.sign_in_as(Some(BEA));
    device.sign_in();
    let mut app = device.launch(abandoned());
    run_until(&mut app, "the inbox", inbox_shows);
    press(&mut app, Key::C);
    settle(&mut app);
    type_text(&mut app, "priya@brightmoor.co");
    settle(&mut app);
    for key in [Key::Enter, Key::Tab] {
        press(&mut app, key);
        settle(&mut app);
    }
    type_text(&mut app, "Switched mid-word");
    settle(&mut app);
    press(&mut app, Key::Tab);
    settle(&mut app);
    // Gmail has the draft, so its next write would wait 5 s.
    run_until(&mut app, "the draft in Gmail", |_| {
        !gmail_messages(&device, "subject:\"Switched mid-word\" in:drafts").is_empty()
    });
    // Within the second compose waits between saves, then away.
    type_text(&mut app, "The last keystroke");
    let switched = Instant::now();
    press_with(&mut app, Key::Num1, CTRL);
    run_until(&mut app, "Bea's mail", |app| on_screen(app) == Some(BEA));
    let written = || {
        gmail_messages(&device, "subject:\"Switched mid-word\" in:drafts")
            .iter()
            .any(|(_, snippet)| snippet.contains("The last keystroke"))
    };
    wait_for("Maya's draft to reach Gmail", 10, written);
    assert!(
        switched.elapsed() < Duration::from_secs(3),
        "written at once, not when its pace allowed: {:?}",
        switched.elapsed()
    );
    let drafts = gmail_messages(&device, "subject:\"Switched mid-word\" in:drafts");
    assert_eq!(drafts.len(), 1, "one Gmail draft: {drafts:?}");
    assert!(
        drafts[0].1.contains("The last keystroke"),
        "with the last keystroke: {drafts:?}"
    );
}

#[test]
fn signing_out_right_after_typing_writes_the_draft_rather_than_ask() {
    let device = Device::new("sign-out-typing");
    device.sign_in();
    let mut app = device.launch(abandoned());
    run_until(&mut app, "the inbox", inbox_shows);
    press(&mut app, Key::C);
    settle(&mut app);
    type_text(&mut app, "priya@brightmoor.co");
    settle(&mut app);
    for key in [Key::Enter, Key::Tab] {
        press(&mut app, key);
        settle(&mut app);
    }
    type_text(&mut app, "Signed out mid-word");
    press(&mut app, Key::Tab);
    type_text(&mut app, "First");
    run_until(&mut app, "the draft in Gmail", |_| {
        !gmail_messages(&device, "subject:\"Signed out mid-word\" in:drafts").is_empty()
    });
    // An edit that waits out the pace, kept on the device as compose closes.
    type_text(&mut app, " and last");
    press(&mut app, Key::Escape);
    press(&mut app, Key::Escape);
    settle(&mut app);
    sign_out(&mut app);
    run_until(
        &mut app,
        "the sign-in screen, with nothing left to ask about",
        |app| log_in(app).is_some(),
    );
    let drafts = gmail_messages(&device, "subject:\"Signed out mid-word\" in:drafts");
    assert_eq!(drafts.len(), 1, "one Gmail draft: {drafts:?}");
    assert!(
        drafts[0].1.contains("First and last"),
        "with the edit: {drafts:?}"
    );
}

#[test]
fn signing_out_an_account_off_screen_asks_about_its_send() {
    let device = Device::new("sign-out-parked");
    device.sign_in_as(Some(BEA));
    device.sign_in();
    let mut app = device.launch(abandoned());
    run_until(&mut app, "the inbox", inbox_shows);
    send_new_message(&mut app, "Held while asked");
    press_with(&mut app, Key::Num1, CTRL);
    run_until(&mut app, "Bea's mail", |app| on_screen(app) == Some(BEA));
    assert_eq!(
        off_screen(app.state()),
        [EMAIL],
        "Maya's mail holds the send off screen"
    );

    press_with(
        &mut app,
        Key::K,
        Modifiers::MAC_CMD.plus(Modifiers::COMMAND),
    );
    type_text(&mut app, "account settings");
    press(&mut app, Key::Enter);
    settle(&mut app);
    // Maya's row is the second: `Sign out` around (716, 444) at 1280×800.
    click(&mut app, pos2(716.0, 444.0));
    let alert = |app: &App| {
        app.mail().and_then(|mail| {
            mail.alert()
                .map(|(title, text)| (title.to_owned(), text.to_owned()))
        })
    };
    run_until(&mut app, "the alert", |app| alert(app).is_some());
    assert_eq!(
        alert(app.state()).unwrap(),
        (
            "One last thing…".to_owned(),
            "1 email is still sending. Ignore and sign out anyway?".to_owned()
        )
    );

    // Cancel (a frame on, once the alert has the keyboard): Maya stays, and her send goes out
    // on time, once.
    app.run_steps(2);
    press(&mut app, Key::Escape);
    assert!(
        app.state()
            .mail()
            .is_some_and(|mail| mail.alert().is_none())
    );
    assert_eq!(device.auth().accounts().len(), 2, "still signed in");
    wait_for("the held send to go", 60, || {
        sent(&device, "Held while asked") > 0
    });
    assert_eq!(sent(&device, "Held while asked"), 1, "sent once");
}

#[test]
fn signing_out_an_account_off_screen_anyway_closes_it_without_sending() {
    let device = Device::new("sign-out-parked-anyway");
    device.sign_in_as(Some(BEA));
    device.sign_in();
    let mut app = device.launch(abandoned());
    run_until(&mut app, "the inbox", inbox_shows);
    send_new_message(&mut app, "Dropped on sign-out");
    press_with(&mut app, Key::Num1, CTRL);
    run_until(&mut app, "Bea's mail", |app| on_screen(app) == Some(BEA));
    press_with(
        &mut app,
        Key::K,
        Modifiers::MAC_CMD.plus(Modifiers::COMMAND),
    );
    type_text(&mut app, "account settings");
    press(&mut app, Key::Enter);
    settle(&mut app);
    click(&mut app, pos2(716.0, 444.0));
    run_until(&mut app, "the alert", |app| {
        app.mail().is_some_and(|mail| mail.alert().is_some())
    });
    app.run_steps(2);
    press(&mut app, Key::Enter);
    run_until(&mut app, "Maya to go", |app| {
        app.mail().is_some_and(|mail| mail.accounts().len() == 1)
    });
    assert_eq!(
        off_screen(app.state()),
        Vec::<&str>::new(),
        "her mail closed without sending"
    );
    assert_eq!(device.store_files(EMAIL), Vec::<String>::new());
    assert_eq!(device.secrets.get(EMAIL).unwrap(), None);
    assert!(gmail_messages(&device, "subject:\"Dropped on sign-out\" in:sent").is_empty());
}

/// A Keychain that asks before each read, as macOS asks an ad hoc build: it keeps the order of
/// the reads, and refuses `denied`'s (the user clicked Deny for it).
struct Asking {
    secrets: Arc<MemorySecrets>,
    denied: &'static str,
    reads: Arc<Mutex<Vec<String>>>,
}

impl SecretStore for Asking {
    fn get(&self, account: &str) -> Result<Option<String>, exo_auth::Error> {
        self.reads.lock().unwrap().push(account.to_owned());
        if account == self.denied {
            return Err(exo_auth::Error::Secrets(
                "The user name or passphrase you entered is not correct.".into(),
            ));
        }
        self.secrets.get(account)
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), exo_auth::Error> {
        self.secrets.set(account, secret)
    }

    fn delete(&self, account: &str) -> Result<(), exo_auth::Error> {
        self.secrets.delete(account)
    }
}

/// Launches Exo on a Keychain that refuses `denied`'s grant, and the Keychain's reads.
fn launch_denying(
    device: &Device,
    denied: &'static str,
) -> (Harness<'static, App>, Arc<Mutex<Vec<String>>>) {
    let reads = Arc::new(Mutex::new(Vec::new()));
    let (client, dir, secrets, keychain_reads) = (
        device.client(),
        device.dir.clone(),
        device.secrets.clone(),
        reads.clone(),
    );
    let open = move || {
        Auth::open(
            client.clone(),
            &dir,
            Asking {
                secrets: secrets.clone(),
                denied,
                reads: keychain_reads.clone(),
            },
        )
    };
    (
        device.launch_with(SignIn::new(open, &device.stores(), abandoned()).unwrap()),
        reads,
    )
}

#[test]
fn launch_reads_every_grant_first_and_a_denied_one_leaves_the_others_open() {
    let device = Device::new("denied-one");
    device.sign_in();
    device.sign_in_as(Some(BEA));
    let (mut app, reads) = launch_denying(&device, EMAIL);
    assert_eq!(
        *reads.lock().unwrap(),
        [BEA, EMAIL],
        "every grant, before any mail opens: the account on screen's first"
    );
    run_until(&mut app, "Bea's inbox", inbox_shows);
    assert_eq!(on_screen(app.state()), Some(BEA));
    assert_eq!(
        off_screen(app.state()),
        Vec::<&str>::new(),
        "Maya's stays closed"
    );
    // Switching to her asks again, and the Keychain refusing again says "Almost there…".
    press_with(&mut app, Key::Num1, CTRL);
    run_until(
        &mut app,
        "Almost there…",
        |app| matches!(app.screen(), Screen::SignedOut(alert) if alert.email() == EMAIL),
    );
    assert_eq!(*reads.lock().unwrap(), [BEA, EMAIL, EMAIL]);
    assert_eq!(off_screen(app.state()), [BEA], "Bea's mail open off screen");
    assert!(
        device
            .auth()
            .accounts()
            .iter()
            .all(|account| !account.signed_out),
        "not listed as signed out"
    );
    drop(app);

    // Refused the account on screen's: "Almost there…" for Bea, and Maya open off screen.
    let (mut app, _) = launch_denying(&device, BEA);
    assert!(matches!(app.state().screen(), Screen::SignedOut(alert) if alert.email() == BEA));
    run_until(&mut app, "Maya's first sync", App::is_idle);
    assert_eq!(off_screen(app.state()), [EMAIL]);
}

/// Saves a new message to Priya in `account`, scheduled to send at `at`, as Schedule does.
fn schedule(account: &exo::account::Account, subject: &str, at: jiff::Timestamp) {
    let to = exo_store::Address {
        name: None,
        email: "priya@brightmoor.co".into(),
    };
    let record = exo_store::DraftRecord {
        action: exo_store::DraftAction::Compose,
        from: Some(exo_store::Address {
            name: None,
            email: EMAIL.into(),
        }),
        to: vec![to],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: subject.into(),
        body: "<div>On time</div>".into(),
        quoted_content: None,
        quoted_inlined: false,
        in_reply_to: None,
        in_reply_to_rfc822: None,
        references: Vec::new(),
        rfc822_id: "off-screen.1@brightmoor.co".into(),
        fingerprint: String::new(),
        attachments: Vec::new(),
        send_later: Some(exo_store::SendLater {
            at,
            abort_on_reply: false,
            current_message_ids: Vec::new(),
        }),
        reminder: None,
    };
    let email = gmail::mime::Email {
        from: Some(gmail::mime::Mailbox::address(EMAIL)),
        to: vec![gmail::mime::Mailbox::address("priya@brightmoor.co")],
        subject: subject.into(),
        html: Some("<div>On time</div>".into()),
        message_id: Some("off-screen.1@brightmoor.co".into()),
        ..gmail::mime::Email::default()
    };
    let id = "draft00000000000077";
    account.save_draft(
        exo_store::Draft {
            id: id.into(),
            thread: None,
            adopted: None,
            record,
            raw: String::new(),
            complete: false,
            sending: true,
        },
        email,
    );
    account.schedule_draft(id);
}

#[test]
fn an_account_off_screen_sends_on_time_and_notifies() {
    let device = Device::new("off-screen-notifies");
    device.sign_in_as(Some(BEA));
    device.sign_in();
    let mut app = device.launch(abandoned());
    let launcher = Launcher::default();
    let desktop = Desktop::new(
        &app.ctx,
        launcher.clone(),
        Placement::load(&device.dir.join("window")),
        Opening::Shown,
    );
    app.state_mut().set_desktop(desktop);
    let notifier = Notifier::recording(launcher.clone());
    app.state_mut().set_notifier(notifier.clone());
    run_until(&mut app, "the inbox", inbox_shows);
    // Both first syncs, which catch up quietly.
    settle(&mut app);

    // Maya schedules a message for a few seconds from now, then switches to Bea.
    let at = jiff::Timestamp::now()
        .checked_add(jiff::SignedDuration::from_secs(10))
        .unwrap();
    schedule(
        app.state().mail().unwrap().account(),
        "Sent from off screen",
        at,
    );
    settle(&mut app);
    press_with(&mut app, Key::Num1, CTRL);
    run_until(&mut app, "Bea's mail", |app| on_screen(app) == Some(BEA));
    settle(&mut app);
    assert_eq!(
        sent(&device, "Sent from off screen"),
        0,
        "not before its time"
    );

    // New mail comes in (for both: the mock's accounts share a mailbox).
    let deliver = serde_json::json!({ "from": "Priya Raman <priya@brightmoor.co>", "subject": "Launch checklist", "text": "Before 3?" });
    let url = device.server.url().join("/__mock/deliver").unwrap();
    let delivered: serde_json::Value = device.runtime.block_on(async {
        reqwest::Client::new()
            .post(url)
            .json(&deliver)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    });
    let (message, thread) = (
        delivered["id"].as_str().unwrap().to_owned(),
        delivered["threadId"].as_str().unwrap().to_owned(),
    );

    // With no frame, Maya's account sends on time, and its next sync (within a minute, an
    // account off screen's pace) notifies.
    wait_for("the scheduled send", 30, || {
        sent(&device, "Sent from off screen") > 0
    });
    let late = jiff::Timestamp::now().duration_since(at);
    assert!(
        late < jiff::SignedDuration::from_secs(5),
        "on time: {late:?} late"
    );
    let maya = |notice: &exo::notifications::Notice| {
        notice.click.account == EMAIL && notice.key == format!("EMAIL-{message}")
    };
    wait_for("Maya's notification", 90, || {
        notifier.posted().iter().any(maya)
    });

    // Its click switches to Maya and opens the conversation, Bea's mail open off screen.
    let notice = notifier.posted().into_iter().find(maya).unwrap();
    launcher.clicked(notice.click);
    run_until(&mut app, "Maya's conversation", |app| {
        on_screen(app) == Some(EMAIL)
            && app
                .mail()
                .and_then(exo::Mail::thread)
                .is_some_and(|open| open.id() == thread)
    });
    assert_eq!(off_screen(app.state()), [BEA]);
}

#[test]
fn a_reminder_off_screen_comes_back_and_notifies() {
    let device = Device::new("off-screen-reminder");
    device.sign_in_as(Some(BEA));
    device.sign_in();
    let now = Arc::new(Mutex::new(gmail_mock::Config::default().now));
    let clock = {
        let now = now.clone();
        exo::Clock::new(move || now.lock().unwrap().clone())
    };
    let (client, dir, secrets) = (device.client(), device.dir.clone(), device.secrets.clone());
    let open = move || Auth::open(client.clone(), &dir, secrets.clone());
    let mut app = device.launch_at(
        SignIn::new(open, &device.stores(), abandoned()).unwrap(),
        clock,
    );
    let notifier = Notifier::recording(Launcher::default());
    app.state_mut().set_notifier(notifier.clone());
    run_until(&mut app, "the inbox", inbox_shows);
    settle(&mut app);

    // Maya snoozes the cursor's conversation until 10 am (the mock's morning), then switches.
    let thread = app
        .state()
        .mail()
        .unwrap()
        .inbox()
        .cursor()
        .map(|thread| thread.id.clone())
        .expect("a cursor");
    press(&mut app, Key::H);
    type_text(&mut app, "10am");
    press(&mut app, Key::Enter);
    settle(&mut app);
    press_with(&mut app, Key::Num1, CTRL);
    run_until(&mut app, "Bea's mail", |app| on_screen(app) == Some(BEA));
    settle(&mut app);

    // Its time comes: with no frame, Maya's account brings it back and notifies.
    *now.lock().unwrap() = "2026-11-17T10:01[America/New_York]".parse().unwrap();
    let back = |notice: &exo::notifications::Notice| {
        notice.click.account == EMAIL
            && matches!(&notice.click.open, exo::notifications::Open::Thread { thread: id, .. } if *id == thread)
    };
    wait_for("Maya's reminder", 90, || notifier.posted().iter().any(back));
}

#[test]
fn google_refusing_an_account_off_screen_closes_it_signed_out() {
    let device = Device::new("refused-off-screen");
    device.sign_in();
    device.sign_in_as(Some(BEA));
    device.revoke_at_google();
    let mut app = device.launch(abandoned());
    run_until(&mut app, "Bea's inbox", inbox_shows);
    assert_eq!(on_screen(app.state()), Some(BEA));
    // Maya's first sync off screen learns Google's answer.
    run_until(&mut app, "Maya to close", |app| {
        app.off_screen().count() == 0
    });
    assert!(
        device
            .auth()
            .accounts()
            .iter()
            .any(|account| account.email == EMAIL && account.signed_out),
        "listed as signed out"
    );
    press_with(&mut app, Key::Num1, CTRL);
    assert_eq!(
        log_in(app.state()).expect("the sign-in screen").hint(),
        Some(EMAIL),
        "switching to her offers to sign her in"
    );
}

#[test]
fn signing_out_an_account_off_screen_since_launch_closes_it() {
    let device = Device::new("sign-out-off-screen");
    device.sign_in_as(Some(BEA));
    device.sign_in();
    device.auth().use_account(BEA).unwrap();
    let mut app = device.launch(abandoned());
    run_until(&mut app, "Bea's inbox", inbox_shows);
    settle(&mut app);
    assert_eq!(
        off_screen(app.state()),
        [EMAIL],
        "Maya open off screen, never shown"
    );
    press_with(
        &mut app,
        Key::K,
        Modifiers::MAC_CMD.plus(Modifiers::COMMAND),
    );
    type_text(&mut app, "account settings");
    press(&mut app, Key::Enter);
    settle(&mut app);
    // Maya's row is the second; with nothing unsent she goes without asking.
    click(&mut app, pos2(716.0, 444.0));
    run_until(&mut app, "Maya to go", |app| {
        app.mail().is_some_and(|mail| mail.accounts().len() == 1)
    });
    assert_eq!(
        off_screen(app.state()),
        Vec::<&str>::new(),
        "her mail closed"
    );
    assert_eq!(
        device.store_files(EMAIL),
        Vec::<String>::new(),
        "her store with her"
    );
    assert_eq!(device.secrets.get(EMAIL).unwrap(), None);
    assert_eq!(on_screen(app.state()), Some(BEA));
}

/// `email`'s database in Exo's stores.
fn database(device: &Device, email: &str) -> PathBuf {
    device.stores().join(format!("{email}.sqlite3"))
}

/// Leaves `email`'s store unreadable, as a damaged disk would.
fn break_store(device: &Device, email: &str) {
    std::fs::create_dir_all(device.stores()).expect("the stores' folder");
    std::fs::write(database(device, email), vec![0x5a; 8192]).expect("written over");
}

/// The teardown on screen: whose, and why.
fn unopened(app: &App) -> Option<(&str, Reason)> {
    match app.screen() {
        Screen::Unopened(alert) => Some((alert.email(), alert.reason())),
        Screen::LogIn(_) | Screen::SignedOut(_) | Screen::Mail(_) => None,
    }
}

#[test]
fn each_teardown_has_its_alert() {
    // Each reason's text and buttons.
    assert_eq!(
        Reason::Corrupt.alert(),
        (
            "The Exo cache is corrupted.\nTo fix this please clear your cache and restart the app.",
            &["Clear Cache and Restart"][..]
        )
    );
    assert_eq!(
        Reason::DiskFull.alert(),
        (
            "Your disk is full. To fix this, please free up some disk space and restart Exo.",
            &[][..]
        )
    );
    assert_eq!(
        Reason::Locked.alert(),
        ("Please reload this page to continue.", &["Reload"][..])
    );
    assert_eq!(
        Reason::Failed.alert(),
        (
            "We've encountered an error and will look into it right away.",
            &["Reload"][..]
        )
    );
}

#[test]
fn a_broken_store_says_so_and_clearing_the_cache_brings_the_mail_back() {
    let device = Device::new("broken-store");
    device.sign_in();
    break_store(&device, EMAIL);
    let mut app = device.launch(abandoned());
    assert_eq!(
        unopened(app.state()),
        Some((EMAIL, Reason::Corrupt)),
        "the corrupted cache, not a failed launch"
    );
    settle(&mut app);

    // Enter performs Clear Cache and Restart.
    press(&mut app, Key::Enter);
    run_until(&mut app, "the inbox", inbox_shows);
    assert_eq!(on_screen(app.state()), Some(EMAIL));
}

#[test]
fn a_newer_exos_store_is_cleared_keeping_what_gmail_doesnt_have() {
    let device = Device::new("newer-store");
    device.sign_in();
    // What an earlier run left: a draft only on this device, and a theme.
    device.runtime.block_on(async {
        let store = exo_store::Store::open(
            device.stores(),
            EMAIL,
            device.server.endpoints(),
            gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into()),
        )
        .await
        .expect("opens");
        store
            .save_draft(local_draft("Only on this Mac"))
            .await
            .expect("saves");
        let mut settings: exo_store::Settings = store.settings().await.expect("reads");
        settings.theme = exo_store::Theme::Carbon;
        store.save_settings(&settings).await.expect("saves");
    });
    // Then a newer Exo ran a migration this one doesn't have.
    rusqlite::Connection::open(database(&device, EMAIL)).and_then(|db| db.execute_batch("INSERT INTO __toasty_migrations (id, name, applied_at) VALUES (99, '0002_from_a_newer_exo.sql', datetime('now'))")).expect("a newer migration");
    let mut app = device.launch(abandoned());
    assert_eq!(unopened(app.state()), Some((EMAIL, Reason::Corrupt)));

    // Escape performs Clear Cache and Restart too.
    press(&mut app, Key::Escape);
    run_until(&mut app, "the inbox", inbox_shows);
    let (drafts, settings) = device.runtime.block_on(async {
        let store = exo_store::Store::open(
            device.stores(),
            EMAIL,
            device.server.endpoints(),
            gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into()),
        )
        .await
        .expect("opens");
        (
            store.drafts().await.expect("reads"),
            store
                .settings::<exo_store::Settings>()
                .await
                .expect("reads"),
        )
    });
    assert_eq!(
        drafts
            .iter()
            .map(|draft| draft.record.subject.as_str())
            .collect::<Vec<_>>(),
        ["Only on this Mac"],
        "the draft is kept"
    );
    assert_eq!(
        settings.theme,
        exo_store::Theme::Carbon,
        "and so are the settings"
    );
}

fn local_draft(subject: &str) -> exo_store::Draft {
    let record = exo_store::DraftRecord {
        action: exo_store::DraftAction::Compose,
        from: None,
        to: vec![exo_store::Address {
            name: None,
            email: "ann@example.com".into(),
        }],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: subject.into(),
        body: format!("<div>{subject}</div>"),
        quoted_content: None,
        quoted_inlined: false,
        in_reply_to: None,
        in_reply_to_rfc822: None,
        references: Vec::new(),
        rfc822_id: "local.draft@exo".into(),
        fingerprint: String::new(),
        attachments: Vec::new(),
        send_later: None,
        reminder: None,
    };
    let email = gmail::mime::Email {
        to: vec![gmail::mime::Mailbox::address("ann@example.com")],
        subject: subject.into(),
        html: Some(format!("<div>{subject}</div>")),
        message_id: Some("local.draft@exo".into()),
        ..gmail::mime::Email::default()
    };
    exo_store::Draft {
        id: "draft00000000000009".into(),
        thread: None,
        adopted: None,
        record,
        raw: String::from_utf8(email.to_rfc822()).expect("MIME is ASCII"),
        complete: true,
        sending: false,
    }
}

#[test]
fn a_locked_store_opens_once_reloaded() {
    let device = Device::new("locked-store");
    device.sign_in();
    std::fs::create_dir_all(device.stores()).expect("the stores' folder");
    // Something else holds the file: opening waits out SQLite's busy timeout, then gives up.
    let holder = rusqlite::Connection::open(database(&device, EMAIL)).expect("opens");
    holder.execute_batch("BEGIN EXCLUSIVE").expect("locks");
    let mut app = device.launch(abandoned());
    assert_eq!(unopened(app.state()), Some((EMAIL, Reason::Locked)));

    drop(holder);
    press(&mut app, Key::Enter);
    run_until(&mut app, "the inbox", inbox_shows);
}

#[test]
fn one_accounts_broken_store_leaves_the_others_a_control_number_away() {
    let device = Device::new("broken-one");
    device.sign_in();
    device.sign_in_as(Some(BEA));
    break_store(&device, BEA);
    // Bea signed in last, so launch opens her: her teardown, not a failed launch.
    let mut app = device.launch(abandoned());
    assert_eq!(unopened(app.state()), Some((BEA, Reason::Corrupt)));

    press_with(&mut app, Key::Num1, CTRL);
    run_until(&mut app, "the first account's inbox", |app| {
        on_screen(app) == Some(EMAIL) && inbox_shows(app)
    });
    press_with(&mut app, Key::Num2, CTRL);
    run_until(&mut app, "Bea's teardown", |app| unopened(app).is_some());
    assert_eq!(
        unopened(app.state()),
        Some((BEA, Reason::Corrupt)),
        "switching to her says so again"
    );
    press_with(&mut app, Key::Num1, CTRL);
    run_until(&mut app, "back to the first account", |app| {
        on_screen(app) == Some(EMAIL)
    });
}

/// Puts a row in `email`'s store that only a cleared cache loses.
fn mark(device: &Device, email: &str) {
    let db = rusqlite::Connection::open(database(device, email)).expect("opens");
    db.execute(
        "INSERT INTO kv (key, value) VALUES ('test-marker', '1')",
        [],
    )
    .expect("marks");
}

fn marked(device: &Device, email: &str) -> bool {
    let db = rusqlite::Connection::open(database(device, email)).expect("opens");
    db.query_row(
        "SELECT count(*) FROM kv WHERE key = 'test-marker'",
        [],
        |row| row.get::<_, i64>(0),
    )
    .expect("reads")
        > 0
}

#[test]
fn the_help_menus_clear_cache_clears_every_account_and_starts_over() {
    let device = Device::new("help-clear");
    device.sign_in_as(Some(BEA));
    device.sign_in();
    let mut app = device.launch(abandoned());
    run_until(&mut app, "the inbox", inbox_shows);
    // Bea signed in first, so she's first in the list.
    press_with(&mut app, Key::Num1, CTRL);
    run_until(&mut app, "Bea's inbox", |app| {
        on_screen(app) == Some(BEA) && inbox_shows(app)
    });
    for email in [EMAIL, BEA] {
        mark(&device, email);
    }

    let ctx = app.ctx.clone();
    app.state_mut().clear_cache_and_restart(&ctx);
    run_until(&mut app, "the inbox again", inbox_shows);
    assert_eq!(
        on_screen(app.state()),
        Some(BEA),
        "launch's account: the one used last"
    );
    for email in [EMAIL, BEA] {
        assert!(!marked(&device, email), "{email}'s cache is cleared");
    }
}

#[test]
fn an_account_whose_store_is_broken_signs_out_from_another() {
    let device = Device::new("broken-sign-out");
    device.sign_in();
    device.sign_in_as(Some(BEA));
    break_store(&device, BEA);
    let mut app = device.launch(abandoned());
    press_with(&mut app, Key::Num1, CTRL);
    run_until(&mut app, "the inbox", inbox_shows);
    press_with(
        &mut app,
        Key::K,
        Modifiers::MAC_CMD.plus(Modifiers::COMMAND),
    );
    type_text(&mut app, "account settings");
    press(&mut app, Key::Enter);
    settle(&mut app);

    // Bea's row is the second (`Sign out` around 716, 444): what her store holds can't be
    // counted, and she goes without asking.
    click(&mut app, pos2(716.0, 444.0));
    run_until(&mut app, "Bea to go", |app| {
        app.mail().is_some_and(|mail| mail.accounts().len() == 1)
    });
    assert_eq!(
        device.store_files(BEA),
        Vec::<String>::new(),
        "her broken store with her"
    );
}

/// `email`'s profile photo, when the avatars have one to draw.
fn photo(app: &Harness<'_, App>, email: &str) -> Option<exo_ui::AvatarPhoto> {
    exo::profile_pictures::photo(&app.ctx, email)
}

/// Runs frames until `email`'s profile photo is there to draw.
fn wait_for_photo(app: &mut Harness<'_, App>, email: &str) {
    let ctx = app.ctx.clone();
    run_until(app, "the photo", move |_| {
        exo::profile_pictures::photo(&ctx, email).is_some()
    });
}

/// Whether the last frame painted `email`'s profile photo.
fn paints_photo(app: &Harness<'_, App>, email: &str) -> bool {
    fn paints(shape: &egui::Shape, textures: &[egui::TextureId]) -> bool {
        match shape {
            egui::Shape::Mesh(mesh) => textures.contains(&mesh.texture_id),
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape| paints(shape, textures)),
            _ => false,
        }
    }
    let Some(photo) = photo(app, email) else {
        return false;
    };
    let textures: Vec<egui::TextureId> = photo.textures().collect();
    app.output()
        .shapes
        .iter()
        .any(|clipped| paints(&clipped.shape, &textures))
}

/// Opens the palette's `query` command.
fn command(app: &mut Harness<'_, App>, query: &str) {
    press_with(app, Key::K, Modifiers::MAC_CMD.plus(Modifiers::COMMAND));
    type_text(app, query);
    press(app, Key::Enter);
    settle(app);
}

#[test]
fn the_accounts_photo_shows_in_the_drawer_the_dialog_and_the_palette() {
    let device = Device::with_photo("photo-sites");
    device.sign_in_as(Some(BEA));
    device.sign_in();
    let mut app = device.launch(abandoned());
    wait_for_photo(&mut app, EMAIL);
    settle(&mut app);
    assert!(
        device.photo_file(EMAIL).exists(),
        "saved beside the account list"
    );
    assert!(
        photo(&app, BEA).is_none() && !device.photo_file(BEA).exists(),
        "Bea has Google's default avatar: her initial"
    );

    press(&mut app, Key::ArrowLeft);
    settle(&mut app);
    assert!(paints_photo(&app, EMAIL), "the folder drawer's account row");
    press(&mut app, Key::Escape);
    settle(&mut app);
    assert!(!paints_photo(&app, EMAIL));

    command(&mut app, "account settings");
    assert!(app.state().mail().unwrap().settings_dialog_open());
    assert!(paints_photo(&app, EMAIL), "the Accounts dialog's row");
    press(&mut app, Key::Escape);
    settle(&mut app);

    command(&mut app, "switch accounts");
    assert!(paints_photo(&app, EMAIL), "the Accounts palette's row");
}

#[test]
fn the_initial_shows_until_the_photo_comes_and_when_google_has_none() {
    let device = Device::on(
        "photo-loading",
        gmail_mock::Config {
            photo: true,
            latency: Duration::from_millis(500),
            ..gmail_mock::Config::default()
        },
    );
    device.sign_in();
    let mut app = device.launch(abandoned());
    press(&mut app, Key::ArrowLeft);
    app.run_steps(15);
    assert!(photo(&app, EMAIL).is_none(), "Google hasn't answered yet");
    assert!(!paints_photo(&app, EMAIL));
    wait_for_photo(&mut app, EMAIL);
    app.run_steps(2);
    assert!(
        paints_photo(&app, EMAIL),
        "the photo replaces the initial when it comes"
    );
    drop(app);

    // Google says the account has no photo any more: the saved one goes, and the initial shows.
    let removed = Device::new("photo-removed");
    removed.sign_in();
    removed
        .auth()
        .set_profile_picture(
            EMAIL,
            Some(&std::fs::read(device.photo_file(EMAIL)).unwrap()),
        )
        .unwrap();
    let mut app = removed.launch(abandoned());
    settle(&mut app);
    assert!(photo(&app, EMAIL).is_none());
    assert!(!removed.photo_file(EMAIL).exists());
}

#[test]
fn a_photo_google_wont_give_leaves_the_initial_and_says_nothing() {
    let device = Device::with_photo("photo-failing");
    device.sign_in();
    device.fault(serde_json::json!({ "path": "/v1/people/me", "status": 503, "times": 100 }));
    let mut app = device.launch(abandoned());
    run_until(&mut app, "the inbox", inbox_shows);
    settle(&mut app);
    assert!(photo(&app, EMAIL).is_none() && !device.photo_file(EMAIL).exists());
    let mail = app.state().mail().expect("the mail");
    assert!(mail.alert().is_none(), "no alert");
}

#[test]
fn the_saved_photo_outlives_clear_cache_and_shows_at_the_next_launch_offline() {
    let mut device = Device::with_photo("photo-relaunch");
    device.sign_in();
    let mut app = device.launch(abandoned());
    wait_for_photo(&mut app, EMAIL);
    settle(&mut app);
    let ctx = app.ctx.clone();
    app.state_mut().clear_cache_and_restart(&ctx);
    settle(&mut app);
    assert!(
        device.photo_file(EMAIL).exists(),
        "the photo isn't in the store Clear Cache makes again"
    );
    assert!(photo(&app, EMAIL).is_some());
    drop(app);

    device.runtime.block_on(device.server.disconnect());
    let mut app = device.launch(abandoned());
    wait_for_photo(&mut app, EMAIL);
    // Google out of reach: People is asked again in a second, two, four…
    app.run_steps(120);
    assert!(photo(&app, EMAIL).is_some(), "the saved photo stays");
    assert!(device.photo_file(EMAIL).exists());
}

#[test]
fn signing_out_deletes_the_saved_photo() {
    let device = Device::with_photo("photo-sign-out");
    device.sign_in_as(Some(BEA));
    device.sign_in();
    let mut app = device.launch(abandoned());
    wait_for_photo(&mut app, EMAIL);
    settle(&mut app);

    sign_out(&mut app);
    run_until(&mut app, "the other account", |app| {
        on_screen(app) == Some(BEA)
    });
    settle(&mut app);
    assert!(
        !device.photo_file(EMAIL).exists(),
        "the file goes with the account"
    );
    assert!(photo(&app, EMAIL).is_none());
}

#[test]
fn each_tab_keeps_its_account_and_comes_back_after_a_relaunch() {
    const COMMAND: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);
    let device = Device::new("tabs-accounts");
    device.sign_in();
    device.sign_in_as(Some(BEA));
    let mut app = device.launch(abandoned());
    let ctx = app.ctx.clone();
    app.state_mut().keep_tabs(&ctx, &device.dir);
    run_until(&mut app, "the inbox", inbox_shows);
    assert_eq!(on_screen(app.state()), Some(BEA));

    // ⌘T: Bea's home again, which ⌃1 switches to the first account in that tab alone.
    press_with(&mut app, Key::T, COMMAND);
    assert_eq!(
        on_screen(app.state()),
        Some(BEA),
        "a new tab opens the active tab's account"
    );
    press_with(&mut app, Key::Num1, CTRL);
    run_until(&mut app, "the first account", |app| {
        on_screen(app) == Some(EMAIL)
    });
    assert_eq!((app.state().tab_count(), app.state().active_tab()), (2, 1));
    press_with(&mut app, Key::Num1, COMMAND);
    run_until(&mut app, "Bea's tab", |app| on_screen(app) == Some(BEA));
    assert_eq!(
        off_screen(app.state()),
        [EMAIL],
        "Maya's mail stays open off screen"
    );
    settle(&mut app);
    let titles = app.state().tab_titles();
    assert!(
        titles[0].ends_with(&format!(" - {BEA}")) && titles[1].ends_with(&format!(" - {EMAIL}")),
        "each names its account: {titles:?}"
    );

    // Quit and launch again: both tabs, Bea's on screen.
    drop(app);
    let mut app = device.launch(abandoned());
    let ctx = app.ctx.clone();
    app.state_mut().keep_tabs(&ctx, &device.dir);
    run_until(&mut app, "the inbox", inbox_shows);
    assert_eq!(
        (
            app.state().tab_count(),
            app.state().active_tab(),
            on_screen(app.state())
        ),
        (2, 0, Some(BEA))
    );
    press_with(&mut app, Key::Num2, COMMAND);
    run_until(&mut app, "Maya's tab", |app| on_screen(app) == Some(EMAIL));
}

/// Another Google account of the user's, which signs in for its calendars alone (the mock gives
/// it a primary calendar of its own, with a dentist's appointment on Wednesday the 18th).
const PERSONAL: &str = "maya.lin@example.com";

/// The week view's Calendars header at 1280×800, its `+` menu's `Sign in with Google`, and the
/// first account header below the mail account's three calendars (a signed-out one's), with its
/// menu's `Sign in again`.
const CALENDARS_HEADER: Pos2 = pos2(1100.0, 438.0);
const SIGN_IN_WITH_GOOGLE: Pos2 = pos2(1200.0, 481.0);
const SECOND_ACCOUNT: Pos2 = pos2(1100.0, 594.0);
const SIGN_IN_AGAIN: Pos2 = pos2(1200.0, 637.0);

/// The week sidebar's calendar accounts: address, calendars, signed out.
fn calendar_accounts(app: &App) -> Vec<(String, Vec<String>, bool)> {
    app.mail()
        .map(exo::Mail::calendar_account_list)
        .unwrap_or_default()
}

fn dentist_shows(app: &App) -> bool {
    app.mail().is_some_and(|mail| {
        mail.calendar_events_on(jiff::civil::date(2026, 11, 18))
            .iter()
            .any(|(title, _)| title == "Dentist")
    })
}

#[test]
fn a_calendar_account_signs_in_from_the_week_and_its_calendar_shows_until_it_is_removed() {
    let device = Device::new("calendar-account");
    device.sign_in();
    let mut app = device.launch(choosing(PERSONAL));
    run_until(&mut app, "the inbox", inbox_shows);
    press(&mut app, Key::Num2);
    settle(&mut app);
    assert_eq!(
        calendar_accounts(app.state()).len(),
        1,
        "the mail account's calendars alone"
    );

    click(&mut app, CALENDARS_HEADER);
    click(&mut app, SIGN_IN_WITH_GOOGLE);
    run_until(&mut app, "its calendar on the week", dentist_shows);
    let listed = calendar_accounts(app.state());
    assert_eq!(
        listed[1],
        (PERSONAL.to_owned(), vec![PERSONAL.to_owned()], false),
        "under its address, its primary switched on"
    );
    assert_eq!(
        device.auth().accounts()[0].calendars,
        [PERSONAL],
        "its grant is the mail account's"
    );
    let settings = app
        .state()
        .mail()
        .expect("mail")
        .account()
        .settings()
        .calendar_accounts[PERSONAL]
        .clone();
    assert!(settings.authed && settings.calendars[PERSONAL].show);
    // Its header folds it away, for good.
    click(&mut app, SECOND_ACCOUNT);
    assert!(
        app.state()
            .mail()
            .expect("mail")
            .account()
            .device_settings()
            .folded_calendar_accounts
            .contains(PERSONAL)
    );

    app.state_mut()
        .mail_mut()
        .expect("mail")
        .account_mut()
        .remove_calendar_account(PERSONAL);
    run_until(&mut app, "its calendar gone", |app| {
        !dentist_shows(app) && calendar_accounts(app).len() == 1
    });
    assert!(
        device.auth().accounts()[0].calendars.is_empty(),
        "and its grant"
    );
    assert!(
        !app.state()
            .mail()
            .expect("mail")
            .account()
            .settings()
            .calendar_accounts
            .contains_key(PERSONAL)
    );
}

/// An invitation to `BEA`, at the top of the inbox.
const INVITE: &[u8] = include_bytes!("support/invite-calendar-account.eml");
/// At 1280×800 with that invitation open: the `RSVP` alert's provider dropdown, and its list's
/// `Google Calendar`; the day view's `No thanks`.
const PROVIDER: Pos2 = pos2(536.0, 442.0);
const GOOGLE_CALENDAR: Pos2 = pos2(540.0, 520.0);
const NO_THANKS: Pos2 = pos2(1095.0, 247.0);

/// A device whose account lists `BEA` as a calendar account Google signed out, with an
/// invitation to it in the inbox.
fn invited(name: &str) -> Device {
    let device = Device::on(
        name,
        gmail_mock::Config {
            extra_messages: vec![INVITE.to_vec()],
            ..gmail_mock::Config::default()
        },
    );
    device.sign_in();
    device.runtime.block_on(async {
        let store = exo_store::Store::open(
            device.stores(),
            EMAIL,
            device.server.endpoints(),
            gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into()),
        )
        .await
        .expect("opens");
        let mut settings: exo_store::Settings = store.settings().await.expect("reads");
        settings.calendar_accounts.insert(
            BEA.into(),
            exo_store::CalendarAccount {
                email: BEA.into(),
                authed: false,
                ..exo_store::CalendarAccount::default()
            },
        );
        store.save_settings(&settings).await.expect("saves");
    });
    device
}

fn add_calendar(app: &App) -> Option<String> {
    app.mail().and_then(exo::Mail::calendar_add_calendar)
}

fn calendar_prompt(app: &App) -> Option<&'static str> {
    app.mail().and_then(exo::Mail::calendar_prompt)
}

/// Opens the invitation and answers Yes from its card, which asks to sign `BEA` in.
fn answer_yes(app: &mut Harness<'_, App>) {
    run_until(app, "the inbox", inbox_shows);
    press(app, Key::Enter);
    run_until(app, "Add Calendar", |app| {
        add_calendar(app).as_deref() == Some(BEA)
    });
    press(app, Key::Tab);
    press(app, Key::Enter);
    run_until(app, "the RSVP alert", |app| {
        calendar_prompt(app) == Some("RSVP")
    });
}

#[test]
fn an_invite_to_a_signed_out_calendar_account_signs_it_in_to_answer() {
    let device = invited("calendar-invite-sign-in");
    let mut app = device.launch(approving());
    answer_yes(&mut app);
    assert_eq!(
        app.state().mail().and_then(exo::Mail::invite_answer),
        Some("needsAction"),
        "not answered yet"
    );
    press(&mut app, Key::Enter);
    assert_eq!(
        calendar_prompt(app.state()),
        Some("RSVP"),
        "Sign in waits for a provider"
    );

    click(&mut app, PROVIDER);
    click(&mut app, GOOGLE_CALENDAR);
    press(&mut app, Key::Enter);
    run_until(&mut app, "the answer", |app| {
        calendar_prompt(app).is_none()
            && app.mail().and_then(exo::Mail::invite_answer) == Some("accepted")
    });
    assert!(
        app.state()
            .mail()
            .expect("mail")
            .account()
            .settings()
            .calendar_accounts[BEA]
            .authed,
        "signed in"
    );
    assert_eq!(add_calendar(app.state()), None);
    let theirs = gmail::Client::new(
        device.server.endpoints(),
        gmail::StaticToken(format!("{}:{BEA}", gmail_mock::DEV_ACCESS_TOKEN)),
    );
    let query = gmail::calendar::EventsQuery {
        q: Some("Portfolio".into()),
        ..gmail::calendar::EventsQuery::default()
    };
    let accepted = || {
        let events = device
            .runtime
            .block_on(theirs.calendar().events(BEA, &query).into_future())
            .expect("lists")
            .items;
        events
            .iter()
            .flat_map(|event| &event.attendees)
            .any(|attendee| {
                attendee.email.as_deref() == Some(BEA)
                    && attendee.response_status.as_deref() == Some("accepted")
            })
    };
    run_until(&mut app, "the answer on its calendar", |_| accepted());
}

#[test]
fn signing_another_account_in_for_an_invite_says_so_and_no_thanks_puts_the_day_back() {
    let device = invited("calendar-invite-wrong-account");
    let mut app = device.launch(choosing(PERSONAL));
    answer_yes(&mut app);
    click(&mut app, PROVIDER);
    click(&mut app, GOOGLE_CALENDAR);
    press(&mut app, Key::Enter);
    run_until(&mut app, "Wrong email", |app| {
        app.mail().and_then(exo::Mail::alert).is_some()
    });
    assert_eq!(
        app.state().mail().and_then(exo::Mail::alert),
        Some((
            "Wrong email",
            "Please ensure you sign in with bea@example.com"
        ))
    );
    assert_eq!(calendar_prompt(app.state()), None);
    assert!(
        !app.state()
            .mail()
            .expect("mail")
            .account()
            .settings()
            .calendar_accounts[BEA]
            .authed,
        "nothing saved"
    );

    press(&mut app, Key::Enter);
    click(&mut app, NO_THANKS);
    assert_eq!(add_calendar(app.state()), None);
    assert_eq!(
        app.state()
            .mail()
            .expect("mail")
            .account()
            .settings()
            .calendar_invite_hint
            .get(BEA),
        Some(&true)
    );
}

#[test]
fn a_calendar_account_without_its_grant_is_signed_out_and_signs_in_again() {
    let device = Device::new("calendar-account-signed-out");
    device.sign_in();
    // An earlier run listed it, and its grant has gone since.
    device.runtime.block_on(async {
        let store = exo_store::Store::open(
            device.stores(),
            EMAIL,
            device.server.endpoints(),
            gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into()),
        )
        .await
        .expect("opens");
        let mut settings: exo_store::Settings = store.settings().await.expect("reads");
        settings.calendar_accounts.insert(
            PERSONAL.into(),
            exo_store::CalendarAccount {
                email: PERSONAL.into(),
                ..exo_store::CalendarAccount::default()
            },
        );
        store.save_settings(&settings).await.expect("saves");
    });
    let mut app = device.launch(choosing(PERSONAL));
    run_until(&mut app, "the inbox", inbox_shows);
    press(&mut app, Key::Num2);
    settle(&mut app);
    assert_eq!(
        calendar_accounts(app.state()).last(),
        Some(&(PERSONAL.to_owned(), Vec::new(), true)),
        "signed out, last"
    );

    click(&mut app, SECOND_ACCOUNT);
    click(&mut app, SIGN_IN_AGAIN);
    run_until(&mut app, "its calendar on the week", dentist_shows);
    assert_eq!(
        calendar_accounts(app.state()).last(),
        Some(&(PERSONAL.to_owned(), vec![PERSONAL.to_owned()], false))
    );
}
