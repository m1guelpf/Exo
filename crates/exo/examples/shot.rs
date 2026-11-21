//! Renders Exo offscreen at 2× and saves a PNG, the same each run, for screenshot comparisons.
//!
//! usage: cargo run -p exo --example shot -- <out.png> [width height] [--keys KEYS] [--hover X,Y] [--theme snow|carbon|auto] [--settings FILE] [--device-settings FILE] [--pdf FILE]
//!        [--signed-out [--sign-in-error | --no-client] | --revoked] [--accounts LIST] [--calendar-accounts LIST] [--photo] [--eml FILE]...
//!        [--connection offline|auth|gmail|throttled|disabled] [--fault JSON]...
//!        [--teardown corrupt|full|locked|error] [--vacation JSON] [--calendar-event JSON]...
//!        [--places "NAME|ADDRESS;…"] [--joined TIMESTAMP] [--inbox-zero-photo FILE] [--first-run] [--discovery] [--account-setup] [--welcome-alert]
//!        [--updates]
//!
//! The account is an in-process gmail-mock with its defaults (seed 1, now 2026-11-17 09:00 New
//! York) synced into a fresh store in a temporary directory, so every run shows the same mail.
//! `--keys` presses keys: names separated by spaces (`e`, `Shift+M`, `Meta+k`, `!`, `Enter`,
//! `wait:800`, `mouse:X,Y`,
//! `click:X,Y` and `drag:X1,Y1,X2,Y2` for the pointer, `hold-drag:…` without letting go,
//! `double-drag:…`/`triple-drag:…` from the press that ends a double or triple click,
//! `scroll:X,Y,DY` for the wheel there,
//! `type:Hi%20there` for text, `reply` for a reply arriving in the open conversation,
//! `trash:N` for its message N going to Trash, `link:URL` for one of Exo's links macOS handed over, `paste-image:FILE` to paste that PNG as a
//! screenshot on the clipboard, `drop:FILE` to drop that file on the window, `download:N:P` for the open conversation's last message's card N
//! downloading with P% there), each followed by
//! whatever the account does about it. `--hover X,Y` puts the pointer there; `--theme` sets the
//! account's theme (`auto` renders Snow: an offscreen render has no system appearance). `--eml
//! FILE` delivers that message to the top of the inbox (gmail-mock's `--eml`). `--settings FILE`
//! starts the account with those settings (a JSON document of its settings: splits,
//! `unifiedInbox`); `--device-settings FILE` with this device's (`{"localSendsCount": 6}`).
//! `--split-library FILE` uses that split library instead of Exo's own, or none (`--split-library
//! none`). `--fault JSON` arms a gmail-mock fault (`/__mock/faults`) once the account has
//! synced, before the keys: a send Gmail refuses, say.
//! `--vacation JSON` sets the mock's vacation responder (Gmail's `VacationSettings`) and
//! `--calendar-event JSON` adds an event to its primary calendar, before the account opens: Out
//! of Office Reply's bar and nudge. `--places` answers Where's every ask with those places
//! instead of Apple Maps. `--joined` is when the account first signed in to Exo
//! (`2026-11-16T12:00:00Z`), which labels the reply buttons in its first week; without it the
//! account has no join date. `--inbox-zero-photo` shows that photo of `assets/inbox-zero` at
//! Inbox Zero whatever the day (the mock's day has `saad-chaudhry-G25LeMV7fAw.jpg`).
//! `--first-run` opens the account as its first time on this Mac, under the welcome splash.
//! `--discovery` opens the discovery questions at once, past the splash and its logo.
//! `--account-setup` opens account setup at once, and `--welcome-alert` the `Welcome to Exo`
//! alert it ends on with a small inbox.
//!
//! `--updates` gives Exo an updater whose checks never hear back, for ⌘K's `Check for Updates`:
//! `--keys "Meta+k type:check%20for%20updates Enter"` shows `Checking for updates…`.
//!
//! `--signed-out` starts on the sign-in screen, with no account on the device; its sign-in never
//! hears back from the browser, so `--keys Enter` shows the spinner, unless `--sign-in-error`
//! makes the browser fail to open (the generic error) or `--no-client` leaves Exo without an
//! OAuth client. `--revoked` starts with an account whose grant is gone, on "Almost there…".
//! `--calendar-accounts a@x.com` signs those in for the mock's account's calendars (the settings
//! say which are signed out). `--accounts a@x.com,!b@x.com` puts more accounts on the device after
//! the mock's (`!` marks one Google signed out); they sign in on the mock's mailbox, and the
//! mock's account opens. `--photo` gives the mock's account a profile photo of its own
//! (gmail-mock's `--photo`), and signs it in as `--accounts` does.
//!
//! `--teardown` shows the `Sorry!` alert for an account whose store can't be opened, for that
//! reason (a corrupt or locked database, a full disk, another error), with nothing else on screen.
//!
//! `--pdf FILE` also prints the open conversation into FILE as `Save as PDF` does, without its
//! panel.
//!
//! `--connection` brings the offline bar up the way it comes up for real, once the account has
//! synced: `offline` takes the mock away and reports a network change, `gmail` takes it away and
//! syncs, `auth` makes the token refresh go unanswered and syncs, `throttled` has Gmail rate limit
//! the next call for ten minutes, `disabled` has it say the account has no mail service.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::Context as _;
use egui::{Event, Key, Modifiers, Pos2};
use exo::{
    Start,
    account::{Account, Credentials},
    sign_in::SignIn,
};
use exo_auth::{Auth, ClientConfig, MemorySecrets};

struct Options {
    out: String,
    width: f32,
    height: f32,
    keys: Vec<String>,
    hover: Option<Pos2>,
    theme: Option<exo_store::Theme>,
    signed_out: bool,
    sign_in_error: bool,
    no_client: bool,
    revoked: bool,
    accounts: Option<String>,
    /// Calendar accounts signed in for the mock's account (`--calendar-accounts`).
    calendar_accounts: Vec<String>,
    photo: bool,
    eml: Vec<String>,
    settings: Option<String>,
    device_settings: Option<String>,
    split_library: Option<String>,
    connection: Option<Connection>,
    faults: Vec<String>,
    teardown: Option<exo::teardown::Reason>,
    vacation: Option<String>,
    calendar_events: Vec<String>,
    pdf: Option<String>,
    joined: Option<jiff::Timestamp>,
    inbox_zero_photo: Option<String>,
    first_run: bool,
    discovery: bool,
    account_setup: bool,
    welcome_alert: bool,
    updates: bool,
}

/// `--updates`' releases, whose checks never hear back, around an app that shows nothing of its
/// own.
struct Unanswered;

impl exo::update::Releases for Unanswered {
    fn check(&self) -> anyhow::Result<Option<fastframe_update::Release>> {
        loop {
            std::thread::park();
        }
    }

    fn download(
        &self,
        _: &fastframe_update::Release,
        _: &mut dyn FnMut(u64, u64),
    ) -> anyhow::Result<fastframe_update::Prepared> {
        anyhow::bail!("nothing to download")
    }

    fn install(&self, _: fastframe_update::Prepared, _: Vec<String>) -> anyhow::Result<()> {
        anyhow::bail!("nothing to install")
    }
}

impl exo::update::Shell for Unanswered {
    fn ready(&self, _: &str) {}

    fn ask(&self, _: &exo::update::Dialog) -> Option<usize> {
        None
    }

    fn quit(&self) {}

    fn changed(&self) {}
}

/// `--places`: `name|address` pairs separated by `;`, URL-escaped as `type:` text is.
fn places(text: &str) -> Vec<exo::places::Place> {
    text.split(';')
        .filter_map(|pair| pair.split_once('|'))
        .map(|(name, address)| exo::places::Place {
            name: name.to_owned(),
            address: address.to_owned(),
        })
        .collect()
}

/// What `--connection` takes down.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Connection {
    Offline,
    Auth,
    Gmail,
    Throttled,
    Disabled,
}

/// Tokens whose refresh stops getting an answer once `gone`, from a token endpoint nothing
/// listens on.
struct Refreshing {
    gone: Arc<AtomicBool>,
    token: url::Url,
}

impl gmail::TokenProvider for Refreshing {
    fn access_token(&self) -> gmail::TokenFuture<'_> {
        Box::pin(async move {
            if self.gone.load(Ordering::SeqCst) {
                return Err(gmail::Error::Http(
                    reqwest::get(self.token.clone())
                        .await
                        .expect_err("nothing listens"),
                ));
            }
            Ok(gmail_mock::DEV_ACCESS_TOKEN.to_owned())
        })
    }
}

fn options() -> anyhow::Result<Options> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: shot <out.png> [width height] [--keys \"j x e\"] [--hover X,Y] [--theme snow|carbon|auto] [--settings FILE] [--device-settings FILE] [--split-library FILE] [--signed-out [--sign-in-error | --no-client] | --revoked] [--accounts LIST] [--calendar-accounts LIST] [--photo] [--eml FILE]... [--connection offline|auth|gmail|throttled|disabled] [--fault JSON]... [--teardown corrupt|full|locked|error] [--vacation JSON] [--calendar-event JSON]... [--pdf FILE] [--places \"NAME|ADDRESS;…\"] [--joined TIMESTAMP] [--first-run] [--discovery] [--account-setup] [--welcome-alert]";
    let mut options = Options {
        out: args.next().context(usage)?,
        width: 1280.0,
        height: 800.0,
        keys: Vec::new(),
        hover: None,
        theme: None,
        signed_out: false,
        sign_in_error: false,
        no_client: false,
        revoked: false,
        accounts: None,
        calendar_accounts: Vec::new(),
        photo: false,
        eml: Vec::new(),
        settings: None,
        device_settings: None,
        split_library: None,
        connection: None,
        faults: Vec::new(),
        teardown: None,
        vacation: None,
        calendar_events: Vec::new(),
        pdf: None,
        joined: None,
        inbox_zero_photo: None,
        first_run: false,
        discovery: false,
        account_setup: false,
        welcome_alert: false,
        updates: false,
    };
    let mut positional = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--theme" => {
                options.theme = Some(match args.next().context(usage)?.as_str() {
                    "snow" => exo_store::Theme::Snow,
                    "carbon" => exo_store::Theme::Carbon,
                    "auto" => exo_store::Theme::Auto,
                    other => anyhow::bail!("unknown theme {other}: {usage}"),
                });
            }
            "--signed-out" => options.signed_out = true,
            "--sign-in-error" => options.sign_in_error = true,
            "--no-client" => options.no_client = true,
            "--revoked" => options.revoked = true,
            "--accounts" => options.accounts = Some(args.next().context(usage)?),
            "--calendar-accounts" => {
                options.calendar_accounts = args
                    .next()
                    .context(usage)?
                    .split(',')
                    .filter(|email| !email.is_empty())
                    .map(str::to_owned)
                    .collect()
            }
            "--photo" => options.photo = true,
            "--eml" => options.eml.push(args.next().context(usage)?),
            "--settings" => options.settings = Some(args.next().context(usage)?),
            "--device-settings" => options.device_settings = Some(args.next().context(usage)?),
            "--split-library" => options.split_library = Some(args.next().context(usage)?),
            "--connection" => {
                options.connection = Some(match args.next().context(usage)?.as_str() {
                    "offline" => Connection::Offline,
                    "auth" => Connection::Auth,
                    "gmail" => Connection::Gmail,
                    "throttled" => Connection::Throttled,
                    "disabled" => Connection::Disabled,
                    other => anyhow::bail!("unknown connection {other}: {usage}"),
                });
            }
            "--fault" => options.faults.push(args.next().context(usage)?),
            "--teardown" => {
                options.teardown = Some(match args.next().context(usage)?.as_str() {
                    "corrupt" => exo::teardown::Reason::Corrupt,
                    "full" => exo::teardown::Reason::DiskFull,
                    "locked" => exo::teardown::Reason::Locked,
                    "error" => exo::teardown::Reason::Failed,
                    other => anyhow::bail!("unknown teardown {other}: {usage}"),
                });
            }
            "--vacation" => options.vacation = Some(args.next().context(usage)?),
            "--calendar-event" => options.calendar_events.push(args.next().context(usage)?),
            "--pdf" => options.pdf = Some(args.next().context(usage)?),
            "--joined" => options.joined = Some(args.next().context(usage)?.parse()?),
            "--inbox-zero-photo" => options.inbox_zero_photo = Some(args.next().context(usage)?),
            "--first-run" => options.first_run = true,
            "--discovery" => options.discovery = true,
            "--account-setup" => options.account_setup = true,
            "--welcome-alert" => options.welcome_alert = true,
            "--updates" => options.updates = true,
            "--places" => {
                exo::places::answer_with(places(&percent_decode(&args.next().context(usage)?)))
            }
            "--keys" => {
                options.keys = args
                    .next()
                    .context(usage)?
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect()
            }
            "--hover" => {
                let value = args.next().context(usage)?;
                let (x, y) = value.split_once(',').context(usage)?;
                options.hover = Some(Pos2::new(x.parse()?, y.parse()?));
            }
            _ => positional.push(arg.parse::<f32>()?),
        }
    }
    if let [width, height] = positional[..] {
        (options.width, options.height) = (width, height);
    }
    // The photos are the device's: the account signs in.
    if options.photo {
        options.accounts.get_or_insert_with(String::new);
    }
    Ok(options)
}

/// A Playwright key name as egui events: the key with its modifiers, and the text it types.
fn key_events(name: &str) -> anyhow::Result<(Key, Modifiers, Option<String>)> {
    let mut parts: Vec<&str> = name.split('+').collect();
    // `Meta++` would be the plus key; nothing here presses it.
    let key = parts.pop().context("empty key")?;
    let mut modifiers = Modifiers::NONE;
    for modifier in parts {
        match modifier {
            "Shift" => modifiers.shift = true,
            "Control" => modifiers.ctrl = true,
            "Alt" => modifiers.alt = true,
            "Meta" => {
                modifiers.mac_cmd = true;
                modifiers.command = true;
            }
            other => anyhow::bail!("unknown modifier {other}"),
        }
    }
    let (key, shifted) = match key {
        // Shifted symbols are their US-layout key with Shift, as a browser reports them.
        "!" => (Key::Num1, true),
        "#" => (Key::Num3, true),
        "?" => (Key::Slash, true),
        single if single.chars().count() == 1 => (
            Key::from_name(&single.to_uppercase())
                .with_context(|| format!("unknown key {single}"))?,
            false,
        ),
        named => (
            Key::from_name(named).with_context(|| format!("unknown key {named}"))?,
            false,
        ),
    };
    modifiers.shift |= shifted;
    // Space types a space, as a browser's does.
    let typed = name
        .rsplit('+')
        .next()
        .map(|text| if text == "Space" { " " } else { text })
        .filter(|text| text.chars().count() == 1 && !modifiers.command && !modifiers.ctrl);
    let text = typed.map(|text| {
        if modifiers.shift {
            text.to_uppercase()
        } else {
            text.to_owned()
        }
    });
    Ok((key, modifiers, text))
}

/// `%XX` escapes decoded (a `type:` step's text can't hold spaces).
fn percent_decode(text: &str) -> String {
    let mut bytes = Vec::new();
    let mut rest = text.as_bytes();
    while let Some((&first, tail)) = rest.split_first() {
        if first == b'%'
            && let Some(byte) = tail
                .get(..2)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            bytes.push(byte);
            rest = &tail[2..];
            continue;
        }
        bytes.push(first);
        rest = tail;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Runs frames until the account has nothing in flight. Returns how many it ran.
fn settle(harness: &mut egui_kittest::Harness<'_, exo::App>) -> usize {
    let started = Instant::now();
    let mut frames = 0;
    loop {
        harness.run_steps(1);
        frames += 1;
        if harness.state().is_idle() || started.elapsed() > Duration::from_secs(60) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.run_steps(2);
    frames + 2
}

/// A client for the mock's OAuth endpoints, which take any.
fn mock_client(server: &gmail_mock::MockServer) -> ClientConfig {
    ClientConfig::new(
        "exo-shot.apps.googleusercontent.com",
        "GOCSPX-shot",
        server.endpoints(),
    )
}

/// Signs the mock's account in once, approving in a scripted browser, so the next launch lists
/// it. Its grant isn't kept: that launch finds it signed out.
fn sign_in_once(
    runtime: &tokio::runtime::Runtime,
    server: &gmail_mock::MockServer,
    state: &std::path::Path,
) -> anyhow::Result<()> {
    let auth = Auth::open(mock_client(server), state, MemorySecrets::default())?;
    sign_in_as(runtime, &auth, None)
}

/// Signs in the account the mock offers for `hint` (its own without one).
fn sign_in_as(
    runtime: &tokio::runtime::Runtime,
    auth: &Auth,
    hint: Option<&str>,
) -> anyhow::Result<()> {
    runtime.block_on(auth.sign_in(hint, |url| {
        // The mock's consent page redirects straight back to Exo's loopback listener.
        tokio::spawn(reqwest::get(url.clone()));
        Ok(())
    }))?;
    Ok(())
}

/// The `Sorry!` screen alone, as the app shows it.
/// A file `drop:` drops on the window.
#[derive(Debug)]
struct Dropped(std::path::PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|error| error.to_string())
    }
}

struct Teardown(exo::Unopened);

impl eframe::App for Teardown {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let _ = self.0.show(ui);
    }
}

/// `--teardown`: renders that screen and saves it.
fn teardown(options: &Options, reason: exo::teardown::Reason) -> anyhow::Result<()> {
    // The mock's day, whose Inbox Zero photo is the alert's backdrop.
    let now = gmail_mock::Config::default().now;
    let mut harness = egui_kittest::Harness::builder()
        .with_size([options.width, options.height])
        .with_pixels_per_point(2.0)
        .with_os(egui::os::OperatingSystem::Mac)
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| {
            exo_ui::install(&cc.egui_ctx).expect("the system fonts load");
            if options.theme == Some(exo_store::Theme::Carbon) {
                exo_ui::set_theme(&cc.egui_ctx, exo_ui::Theme::Carbon);
            }
            Teardown(exo::Unopened::new(
                &cc.egui_ctx,
                &now,
                "maya@brightmoor.co".into(),
                reason,
            ))
        });
    harness.remove_cursor();
    harness.run_steps(30);
    harness
        .render()
        .map_err(anyhow::Error::msg)?
        .save(&options.out)?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let options = options()?;
    if let Some(reason) = options.teardown {
        return teardown(&options, reason);
    }
    let config = gmail_mock::Config {
        extra_messages: options
            .eml
            .iter()
            .map(std::fs::read)
            .collect::<Result<_, _>>()?,
        photo: options.photo,
        ..gmail_mock::Config::default()
    };
    let now = config.now.clone();
    // The mock serves from this runtime; the account runs on its own.
    let runtime = tokio::runtime::Runtime::new()?;
    let mut server = runtime.block_on(gmail_mock::MockServer::start(config))?;
    let client = gmail::Client::new(
        server.endpoints(),
        gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into()),
    );
    let email = runtime
        .block_on(async { client.gmail().profile().await })?
        .email_address
        .context("the mock's profile has an address")?;
    if let Some(vacation) = &options.vacation {
        let vacation: gmail::VacationSettings = serde_json::from_str(vacation)?;
        runtime.block_on(async { client.gmail().update_vacation(&vacation).await })?;
    }
    for event in &options.calendar_events {
        let event: gmail::calendar::Event = serde_json::from_str(event)?;
        runtime.block_on(async {
            client
                .calendar()
                .insert_event("primary", &event, None)
                .await
        })?;
    }
    // Sign-in that can stop answering, at a token endpoint of its own (`--connection auth`).
    let mut endpoints = server.endpoints();
    let port = std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port();
    endpoints.oauth.token = format!("http://127.0.0.1:{port}/token").parse()?;
    let sign_in_gone = Arc::new(AtomicBool::new(false));
    let credentials = Credentials {
        email,
        tokens: Arc::new(Refreshing {
            gone: sign_in_gone.clone(),
            token: endpoints.oauth.token.clone(),
        }),
        endpoints,
    };
    let stores = std::env::temp_dir().join(format!("exo-shot-{}", std::process::id()));
    if (options.theme.is_some() || options.settings.is_some() || options.device_settings.is_some())
        && !options.signed_out
    {
        let mut settings: exo_store::Settings = match &options.settings {
            Some(file) => serde_json::from_str(&std::fs::read_to_string(file)?)?,
            None => exo_store::Settings::default(),
        };
        // The account's own setting themes its mail (and what follows it, like "Almost there…").
        if let Some(theme) = options.theme {
            settings.theme = theme;
        }
        let device: Option<exo_store::DeviceSettings> = match &options.device_settings {
            Some(file) => Some(serde_json::from_str(&std::fs::read_to_string(file)?)?),
            None => None,
        };
        runtime.block_on(async {
            let store = exo_store::Store::open(
                &stores,
                &credentials.email,
                server.endpoints(),
                gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into()),
            )
            .await?;
            if let Some(device) = &device {
                store.save_settings(device).await?;
            }
            store.save_settings(&settings).await
        })?;
    }
    let start = if options.signed_out
        || options.revoked
        || options.accounts.is_some()
        || !options.calendar_accounts.is_empty()
    {
        let state = stores.join("auth");
        if options.revoked {
            sign_in_once(&runtime, &server, &state)?;
        }
        let client = mock_client(&server);
        let secrets = Arc::new(MemorySecrets::default());
        if let Some(list) = &options.accounts {
            let auth = Auth::open(client.clone(), &state, secrets.clone())?;
            sign_in_as(&runtime, &auth, None)?;
            for entry in list.split(',').filter(|entry| !entry.is_empty()) {
                let email = entry.trim_start_matches('!');
                sign_in_as(&runtime, &auth, Some(email))?;
                if entry.starts_with('!') {
                    auth.mark_signed_out(email)?;
                }
            }
            auth.use_account(&credentials.email)?;
        }
        if !options.calendar_accounts.is_empty() {
            let auth = Auth::open(client.clone(), &state, secrets.clone())?;
            if options.accounts.is_none() {
                sign_in_as(&runtime, &auth, None)?;
            }
            for email in &options.calendar_accounts {
                runtime.block_on(auth.sign_in_calendar(
                    &credentials.email,
                    Some(email),
                    |url| {
                        tokio::spawn(reqwest::get(url.clone()));
                        Ok(())
                    },
                ))?;
            }
        }
        let no_client = options.no_client;
        let open = move || {
            if no_client {
                let path = std::env::var_os("HOME")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_default()
                    .join("Library/Application Support/Exo/google-oauth-client.json");
                return Err(exo_auth::Error::MissingClient {
                    path,
                    from_env: false,
                });
            }
            Auth::open(client.clone(), &state, secrets.clone())
        };
        // A browser that never comes back, so the spinner stays; or one that can't open.
        let browser: exo::sign_in::Browser = if options.sign_in_error {
            Arc::new(|_| Err(std::io::Error::other("no browser")))
        } else {
            Arc::new(|_| Ok(()))
        };
        Some(SignIn::new(open, &stores, browser)?)
    } else {
        None
    };

    let assets = exo::assets_dir();
    let mut harness = egui_kittest::Harness::builder()
        .with_size([options.width, options.height])
        .with_pixels_per_point(2.0)
        .with_os(egui::os::OperatingSystem::Mac)
        // 60 frames a second, so toasts (8s) are still up when we capture.
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| {
            // Copies stay in the shot, off the user's clipboard.
            exo::clipboard::capture(&cc.egui_ctx);
            // With no account, the sign-in screen is in the last account's theme, which
            // `--signed-out --theme carbon` stands for.
            if options.signed_out && options.theme == Some(exo_store::Theme::Carbon) {
                exo_ui::set_theme(&cc.egui_ctx, exo_ui::Theme::Carbon);
            }
            let start = match start {
                Some(sign_in) => Start::SignIn(sign_in),
                None => Start::Mail(Box::new(
                    Account::open(&stores, credentials, cc.egui_ctx.clone())
                        .expect("the store opens"),
                )),
            };
            let mut app =
                exo::App::new(cc, &assets, start, exo::Clock::fixed(now)).expect("assets load");
            // The settings menu's `Warn Before Quitting`, as in Exo itself, on.
            app.set_quit_warning(exo::desktop::QuitWarning::load(&stores));
            if options.updates {
                app.set_updates(exo::update::Updates::with(Unanswered, Unanswered));
            }
            // Shots have no caret: it blinks with no "on" phase.
            cc.egui_ctx.all_styles_mut(|style| {
                style.visuals.text_cursor.blink = true;
                style.visuals.text_cursor.on_duration = 0.0;
            });
            app
        });
    // As large textures as eframe gets from the GPU (egui assumes 2048 device px without one),
    // so a wide body paints whole, as it does in Exo.
    harness.input_mut().max_texture_side = Some(8192);
    // The device's account list, as signing in left it, with the join date asked for (signing in
    // here records the real time, which isn't the shot's).
    if let Some(mail) = harness.state_mut().mail_mut() {
        let accounts = mail
            .accounts()
            .iter()
            .map(|account| exo_auth::Account {
                joined: options.joined,
                ..account.clone()
            })
            .collect();
        mail.set_accounts(accounts);
        if options.first_run {
            mail.welcome_first_time();
        }
        if options.discovery {
            mail.open_discovery();
        }
        if options.account_setup {
            mail.open_account_setup();
        }
        if options.welcome_alert {
            mail.welcome_to_exo();
        }
    }
    if let Some(file) = &options.inbox_zero_photo {
        let mail = harness
            .state_mut()
            .mail_mut()
            .context("--inbox-zero-photo needs mail")?;
        anyhow::ensure!(
            mail.use_photo(file),
            "no Inbox Zero photo {file} in assets/inbox-zero"
        );
    }
    if let Some(file) = &options.split_library {
        let library = if file == "none" {
            None
        } else {
            Some(
                std::fs::read_to_string(file)?
                    .parse()
                    .map_err(anyhow::Error::msg)?,
            )
        };
        if let Some(mail) = harness.state_mut().mail_mut() {
            mail.set_split_library(library);
        }
    }
    settle(&mut harness);
    if let Some(connection) = options.connection {
        let url = server.url().join("/__mock/faults")?;
        let fault = |fault: serde_json::Value| {
            runtime
                .block_on(reqwest::Client::new().post(url.clone()).json(&fault).send())
                .map(drop)
        };
        match connection {
            Connection::Offline | Connection::Gmail => runtime.block_on(server.disconnect()),
            Connection::Auth => sign_in_gone.store(true, Ordering::SeqCst),
            Connection::Throttled => {
                let at = jiff::Timestamp::now() + jiff::SignedDuration::from_mins(10);
                fault(
                    serde_json::json!({ "path": "/gmail/v1/", "status": 429, "message": format!("User-rate limit exceeded.  Retry after {at}") }),
                )?;
            }
            Connection::Disabled => fault(
                serde_json::json!({ "path": "/gmail/v1/", "status": 400, "message": "Mail service not enabled", "reason": "failedPrecondition", "times": 1000 }),
            )?,
        }
        match connection {
            Connection::Offline => harness
                .state()
                .mail()
                .context("--connection needs mail")?
                .account()
                .network_changed(),
            // A sync finds out, as regaining focus starts one.
            _ => {
                harness.input_mut().focused = false;
                harness.run_steps(2);
                harness.input_mut().focused = true;
            }
        }
        settle(&mut harness);
    }
    for fault in &options.faults {
        let url = server.url().join("/__mock/faults")?;
        let fault: serde_json::Value = serde_json::from_str(fault)?;
        runtime
            .block_on(reqwest::Client::new().post(url).json(&fault).send())?
            .error_for_status()?;
    }
    // Frames since the last key press, to show infinite animations at their start.
    let mut since_key = None;
    // A `mouse:` or `click:` left the pointer somewhere, where the shot keeps it.
    let mut pointer = false;
    for name in &options.keys {
        if name == "reply" {
            // A reply from Ann into the open conversation, then the sync regaining focus starts:
            // the Show New Messages bar.
            let thread = harness
                .state()
                .mail()
                .and_then(|mail| mail.thread())
                .map(|thread| thread.id().to_owned())
                .context("`reply` needs an open conversation")?;
            let url = server.url().join("/__mock/deliver")?;
            let reply = serde_json::json!({ "threadId": thread, "from": "Ann Lee <ann@example.com>", "text": "Thanks for this." });
            runtime
                .block_on(reqwest::Client::new().post(url).json(&reply).send())?
                .error_for_status()?;
            harness.input_mut().focused = false;
            harness.run_steps(2);
            harness.input_mut().focused = true;
            since_key = Some(settle(&mut harness));
            continue;
        }
        if let Some(url) = name.strip_prefix("link:") {
            // One of Exo's links macOS handed over, in the account on screen.
            let link = exo::links::parse(&percent_decode(url))
                .context("`link:URL` needs one of Exo's links")?;
            harness
                .state_mut()
                .mail_mut()
                .context("`link:URL` needs an account")?
                .open_link(&link);
            since_key = Some(settle(&mut harness));
            continue;
        }
        if let Some(at) = name.strip_prefix("trash:") {
            // Message `at` (from 0) of the open conversation goes to Trash, as from another
            // device, then a sync: its marker.
            let message = harness
                .state()
                .mail()
                .and_then(|mail| mail.thread())
                .and_then(|thread| {
                    thread
                        .messages()
                        .get(at.parse::<usize>().ok()?)
                        .map(|message| message.id.clone())
                })
                .context("`trash:N` needs an open conversation with that message")?;
            runtime.block_on(async { client.gmail().trash_message(&message).await })?;
            harness.input_mut().focused = false;
            harness.run_steps(2);
            harness.input_mut().focused = true;
            since_key = Some(settle(&mut harness));
            continue;
        }
        if let Some(held) = name.strip_prefix("download:") {
            // The open conversation's last message's card N downloading, P% there: its bar and
            // its notification.
            let (card, percent) = held.split_once(':').context("download:N:P")?;
            harness
                .state_mut()
                .mail_mut()
                .context("`download:N:P` needs mail")?
                .hold_download(card.parse()?, percent.parse()?);
            since_key = Some(settle(&mut harness));
            continue;
        }
        if let Some(ms) = name.strip_prefix("wait:") {
            let frames = (ms.parse::<f32>()? / 1000.0 * 60.0).round() as usize;
            harness.run_steps(frames);
            since_key = since_key.map(|since: usize| since + frames);
            continue;
        }
        if let Some((kind, at)) = name
            .split_once(':')
            .filter(|(kind, _)| matches!(*kind, "click" | "mouse"))
        {
            let (x, y) = at.split_once(',').context("click:X,Y or mouse:X,Y")?;
            let pos = Pos2::new(x.parse()?, y.parse()?);
            pointer = true;
            // Arrive from 30px to the left, moving onto the spot as a mouse does.
            harness
                .input_mut()
                .events
                .push(Event::PointerMoved(pos - egui::vec2(30.0, 0.0)));
            harness.run_steps(1);
            harness.input_mut().events.push(Event::PointerMoved(pos));
            harness.run_steps(1);
            if kind == "click" {
                for pressed in [true, false] {
                    harness.input_mut().events.push(Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    });
                    harness.run_steps(1);
                }
            }
            since_key = Some(settle(&mut harness));
            continue;
        }
        if let Some(at) = name.strip_prefix("scroll:") {
            // The wheel at X,Y, DY px down.
            let numbers = at
                .split(',')
                .map(str::parse::<f32>)
                .collect::<Result<Vec<_>, _>>()?;
            let [x, y, dy] = numbers[..] else {
                anyhow::bail!("{name}: expected X,Y,DY")
            };
            pointer = true;
            harness
                .input_mut()
                .events
                .push(Event::PointerMoved(Pos2::new(x, y)));
            harness.run_steps(1);
            harness.input_mut().events.push(Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -dy),
                modifiers: Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            });
            harness.run_steps(1);
            since_key = Some(settle(&mut harness));
            continue;
        }
        let drag = name.split_once(':').and_then(|(kind, points)| {
            let (clicks, hold) = match kind {
                "drag" => (1, false),
                "hold-drag" => (1, true),
                "double-drag" => (2, false),
                "triple-drag" => (3, false),
                _ => return None,
            };
            Some((points, clicks, hold))
        });
        if let Some((points, clicks, hold)) = drag {
            // Presses at the first point, moves to the second in ten steps and releases;
            // `hold-drag:` keeps the button down, mid-drag, and
            // `double-drag:`/`triple-drag:` click there first, so the press ends a double or
            // triple click.
            let numbers = points
                .split(',')
                .map(str::parse::<f32>)
                .collect::<Result<Vec<_>, _>>()?;
            let [x1, y1, x2, y2] = numbers[..] else {
                anyhow::bail!("{name}: expected X1,Y1,X2,Y2")
            };
            let (from, to) = (Pos2::new(x1, y1), Pos2::new(x2, y2));
            let button = |pos, pressed| Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            harness.input_mut().events.push(Event::PointerMoved(from));
            harness.run_steps(1);
            for _ in 1..clicks {
                for pressed in [true, false] {
                    harness.input_mut().events.push(button(from, pressed));
                    harness.run_steps(1);
                }
            }
            harness.input_mut().events.push(button(from, true));
            harness.run_steps(1);
            for step in 1..=10 {
                harness
                    .input_mut()
                    .events
                    .push(Event::PointerMoved(from + (to - from) * step as f32 / 10.0));
                harness.run_steps(1);
            }
            if !hold {
                harness.input_mut().events.push(button(to, false));
                harness.run_steps(1);
            }
            pointer = true;
            since_key = Some(settle(&mut harness));
            continue;
        }
        if let Some(path) = name.strip_prefix("drop:") {
            // That file dropped on the window.
            harness
                .input_mut()
                .dropped_files
                .push(std::sync::Arc::new(Dropped(path.into())));
            harness.run_steps(2);
            since_key = Some(settle(&mut harness));
            continue;
        }
        if let Some(path) = name.strip_prefix("paste-image:") {
            // An image on the clipboard (a screenshot), pasted.
            exo::clipboard::capture_image(
                &harness.ctx,
                std::fs::read(path).with_context(|| format!("reading {path}"))?,
            );
            harness.input_mut().events.push(Event::Paste(String::new()));
            harness.run_steps(1);
            since_key = Some(settle(&mut harness));
            continue;
        }
        if let Some(text) = name.strip_prefix("type:") {
            // Each character's key (when it has one) and the character it types, as
            // Playwright's `keyboard.type` does.
            for ch in percent_decode(text).chars() {
                let typed = ch.to_string();
                match key_events(&typed) {
                    Ok((key, modifiers, _)) => {
                        let modifiers = Modifiers {
                            shift: modifiers.shift || ch.is_uppercase(),
                            ..modifiers
                        };
                        harness.input_mut().events.push(Event::Key {
                            key,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers,
                        });
                        harness.input_mut().events.push(Event::Text(typed));
                        harness.run_steps(1);
                        harness.input_mut().events.push(Event::Key {
                            key,
                            physical_key: None,
                            pressed: false,
                            repeat: false,
                            modifiers,
                        });
                    }
                    Err(_) => {
                        harness.input_mut().events.push(Event::Text(typed));
                        harness.run_steps(1);
                    }
                }
            }
            since_key = Some(settle(&mut harness));
            continue;
        }
        let (key, modifiers, text) = key_events(name)?;
        let event = |pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        harness.input_mut().events.push(event(true));
        if let Some(text) = text {
            harness.input_mut().events.push(Event::Text(text));
        }
        harness.run_steps(1);
        harness.input_mut().events.push(event(false));
        since_key = Some(settle(&mut harness));
    }
    if let Some(at) = options.hover {
        // Arrive from 30px to the left, moving onto the spot as a mouse does.
        harness
            .input_mut()
            .events
            .push(Event::PointerMoved(at - egui::vec2(30.0, 0.0)));
        harness.run_steps(1);
        harness.input_mut().events.push(Event::PointerMoved(at));
    } else if !pointer {
        harness.remove_cursor();
    }
    // Enough frames for fonts, SVG textures and the cursor's slide to settle.
    harness.run_steps(30);
    let frames = 30 + settle(&mut harness);
    // The sign-in spinner turns once a second: show it where it started.
    if options.signed_out
        && let Some(since) = since_key
    {
        harness.run_steps((60 - (since + frames) % 60) % 60);
    }
    harness
        .render()
        .map_err(anyhow::Error::msg)?
        .save(&options.out)?;
    if let Some(pdf) = &options.pdf {
        print_to_pdf(&mut harness, pdf)?;
    }
    drop(harness);
    let _ = std::fs::remove_dir_all(&stores);
    Ok(())
}

/// `--pdf`: prints the open conversation into `path`, running the main thread's run loop (which
/// WebKit answers on) until it's written.
#[cfg(target_os = "macos")]
fn print_to_pdf(
    harness: &mut egui_kittest::Harness<'_, exo::App>,
    path: &str,
) -> anyhow::Result<()> {
    let printed = std::rc::Rc::new(std::cell::Cell::new(None));
    let heard = printed.clone();
    harness
        .state_mut()
        .mail_mut()
        .context("--pdf needs mail")?
        .print_to_pdf(path.into(), Box::new(move |ok| heard.set(Some(ok))));
    let started = Instant::now();
    while printed.get().is_none() && started.elapsed() < Duration::from_secs(60) {
        let until = objc2_foundation::NSDate::dateWithTimeIntervalSinceNow(0.02);
        objc2_foundation::NSRunLoop::currentRunLoop().runUntilDate(&until);
    }
    anyhow::ensure!(printed.get() == Some(true), "printing to {path} failed");
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn print_to_pdf(
    _harness: &mut egui_kittest::Harness<'_, exo::App>,
    _path: &str,
) -> anyhow::Result<()> {
    anyhow::bail!("--pdf prints on macOS only")
}
