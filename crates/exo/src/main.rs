#![deny(
    clippy::correctness,
    clippy::suspicious,
    clippy::complexity,
    clippy::perf
)]
//! The Exo desktop app.

use std::sync::Arc;

use anyhow::Context as _;
use exo::desktop::{self, Desktop, Launcher, Opening, Placement, Unclaimed};
use exo::notifications::{self, Notifier};
use exo::sign_in::{self, SignIn};
use exo_auth::{Auth, ClientConfig, MemorySecrets};

/// Where Exo keeps its files: `~/Library/Application Support/Exo` on macOS.
const DIRS: fastframe_dirs::App = fastframe_dirs::App::new("exo", "", "", "Exo");

/// gmail-mock takes any OAuth client.
const MOCK_CLIENT_ID: &str = "exo-dev.apps.googleusercontent.com";
const MOCK_CLIENT_SECRET: &str = "GOCSPX-gmail-mock";

fn main() -> anyhow::Result<()> {
    // First: an update's helper runs here and exits, and a relaunch's update flags come off.
    let launch = fastframe_update::intercept(&exo::update::CONFIG);
    let arguments: Vec<String> = launch
        .arguments
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    // The helper of an earlier Exo asks a download its version before installing it.
    if arguments
        .get(1)
        .is_some_and(|argument| argument == "--version")
    {
        println!("{}", exo::update::version());
        return Ok(());
    }
    fastframe_titlebar::init();
    let assets = exo::assets_dir();
    let dirs = DIRS
        .discover(|_, _| Ok(()))
        .dirs
        .context("finding Exo's folders")?;
    // `EXO_GMAIL_MOCK=http://127.0.0.1:8787/` signs in against a running gmail-mock instead of
    // Google. Its grants and mail die with it, so the account list and the accounts' stores go
    // in a folder that lasts this run only, away from the real ones.
    let mock = std::env::var("EXO_GMAIL_MOCK")
        .ok()
        .map(|origin| origin.parse::<url::Url>())
        .transpose()
        .context("EXO_GMAIL_MOCK isn't a URL")?;
    // One Exo at a time; one on gmail-mock runs beside the real one.
    let launcher = Launcher::default();
    let claimed = desktop::claim(
        mock.as_ref().map(url::Url::as_str),
        desktop::launch_url(arguments.iter().cloned()),
        &launcher,
    );
    // The log file starts afresh each run, so only the running Exo on real mail keeps one: a
    // later launch, or one on gmail-mock, would start the running Exo's over.
    exo::logging::init(
        exo::logging::folder()
            .filter(|_| claimed.is_ok() && mock.is_none())
            .as_deref(),
    );
    if let Some(failure) = &launch.failure {
        log::warn!("the update didn't install, so the previous Exo is back: {failure}");
    }
    let _instance = match claimed {
        Ok(guard) => guard,
        Err(Unclaimed::HandedOver) => return Ok(()),
        Err(error) => {
            log::error!("{error}");
            return Ok(());
        }
    };
    // Exo.app run from outside an Applications folder offers to move there, before its window.
    #[cfg(target_os = "macos")]
    if mock.is_none()
        && desktop::applications::offer(&dirs.state) != desktop::applications::Moved::Stay
    {
        return Ok(());
    }
    // Exo updates itself, but not on gmail-mock. An update an earlier run downloaded installs now,
    // before anything opens: the updated Exo opens in this launch's place, with its arguments.
    let updater = mock.is_none().then(exo::update::updater).flatten();
    if let Some(updater) = &updater
        && exo::update::install_kept(updater, arguments.iter().skip(1).cloned().collect())
    {
        return Ok(());
    }
    let mock_state = mock
        .is_some()
        .then(|| std::env::temp_dir().join(format!("exo-gmail-mock-{}", std::process::id())));
    if let Some(dir) = &mock_state {
        // A crashed run's, should its process ID come round again.
        let _ = std::fs::remove_dir_all(dir);
    }
    let state = mock_state.clone().unwrap_or_else(|| dirs.state.clone());
    exo::mail_client::listen(launcher.clone());
    desktop::menu::install(launcher.clone());
    // Only an app bundle's notifications show (Exo.app); a bare binary posts none.
    let notifier = notifications::bundle_id().and_then(|id| {
        Notifier::new(&id, launcher.clone())
            .inspect_err(|error| log::warn!("notifications: {error}"))
            .ok()
    });
    let updates = updater.map(|updater| {
        let updates = exo::update::Updates::start(updater, launcher.clone(), notifier.clone());
        launcher.set_updates(updates.clone());
        if launch.error.is_some() {
            updates.restored();
        }
        updates
    });
    // Restarted to install an update, Exo leaves the app in front there; the update waits for
    // word that its window is up.
    let updated = arguments
        .iter()
        .any(|argument| argument == exo::update::UPDATED);
    let receipt = std::rc::Rc::new(std::cell::Cell::new(launch.receipt));
    // The window opens where it was left, minimized when macOS opened Exo at login: known only
    // as the window is made, while macOS hands Exo the event it launched it with.
    let placement = Placement::load(&state);
    let options = || {
        let mut options = exo::native_options();
        options.viewport = placement.apply(options.viewport);
        options.window_builder = Some(Box::new(|builder| Opening::launch().apply(builder)));
        options.event_loop_builder = Some(Box::new(move |builder| {
            desktop::menu::without_winit_menu(builder);
            if updated {
                desktop::without_activating(builder);
            }
        }));
        options
    };
    let result = fastframe_graphics::Graphics::new("Exo")
        .run_native(options, || {
            let (assets, config, state, mock) = (
                assets.clone(),
                dirs.config.clone(),
                state.clone(),
                mock.clone(),
            );
            let (launcher, placement, notifier, updates, receipt) = (
                launcher.clone(),
                placement.clone(),
                notifier.clone(),
                updates.clone(),
                receipt.clone(),
            );
            Box::new(move |cc| {
                exo::mail_client::ready();
                // Exo.app's first run on real mail makes it the default email app.
                if mock.is_none() {
                    exo::mail_client::claim_on_first_run(&state);
                }
                // `Warn Before Quitting`, which the quit alert and the settings menu share.
                let quit_warning = desktop::QuitWarning::load(&state);
                let zoom = exo::zoom::Zoom::load(&state);
                // The tabs open where the last run left them, kept beside the window's place.
                let tabs = state.clone();
                desktop::confirm_quit(quit_warning.clone());
                // Where attachments download to, for every account.
                exo::downloads::install(&cc.egui_ctx, &state);
                // Find's text, shared with other apps through the macOS find pasteboard.
                exo::find::share_find_text(&cc.egui_ctx, exo::find::Pasteboard::System);
                let stores = state.join("accounts");
                let open: Box<dyn Fn() -> Result<Auth, exo_auth::Error>> = match mock {
                    Some(origin) => Box::new(move || {
                        let client = ClientConfig::new(
                            MOCK_CLIENT_ID,
                            MOCK_CLIENT_SECRET,
                            gmail::Endpoints::mock(&origin),
                        );
                        Auth::open(client, &state, MemorySecrets::default())
                    }),
                    None => Box::new(move || {
                        Auth::open(ClientConfig::load(&config)?, &state, exo_auth::Keychain)
                    }),
                };
                let mut sign_in = SignIn::new(open, &stores, Arc::new(sign_in::system_browser))?;
                sign_in.set_power(exo::power::PowerSource::watch());
                let mut app = exo::App::new(
                    cc,
                    &assets,
                    exo::Start::SignIn(sign_in),
                    exo::Clock::system(),
                )?;
                app.set_desktop(Desktop::new(
                    &cc.egui_ctx,
                    launcher,
                    placement,
                    Opening::launch(),
                ));
                app.set_quit_warning(quit_warning);
                app.set_zoom(zoom);
                app.keep_tabs(&cc.egui_ctx, &tabs);
                if let Some(notifier) = notifier {
                    app.set_notifier(notifier);
                }
                if let Some(updates) = updates {
                    app.set_updates(updates);
                }
                if let Some(receipt) = receipt.take() {
                    app.set_update_receipt(receipt);
                }
                Ok(Box::new(app))
            })
        })
        .map_err(|error| anyhow::anyhow!("{error}"));
    if let Some(dir) = &mock_state {
        let _ = std::fs::remove_dir_all(dir);
    }
    result
}
