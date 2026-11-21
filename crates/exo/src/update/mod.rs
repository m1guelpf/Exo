//! Exo updates itself from its releases on GitHub.
//!
//! It checks as it starts and 15 minutes after each check ends. A newer release downloads in
//! the background, then a notification says it's ready (`Update ready!`): `Install and
//! Restart`, a click on the notification, or the app menu's `Install Update and Restart` hands
//! it to a helper (a copy of this Exo) and quits; the helper swaps the app for the new one and
//! opens it, its window back without taking the focus from the app in front. Should the new
//! one not open its window within a minute, the helper puts this one back. An update nobody
//! installs is installed as Exo next starts, before it opens anything ([`install_kept`]).
//!
//! ⌘K's `Check for Updates`, and the app menu's `Check for Updates…` while an account's mail is
//! on screen, check in front of the user: an alert (`Update Exo`) says how it goes, and Exo
//! restarts into the update once it's downloaded. Closing the alert leaves the check to finish
//! as one nobody asked for. With no mail on screen the app menu's asks with dialogs instead:
//! whether to download, then whether to install.
//!
//! Each step has a deadline: 30 seconds, or 90 from the last bytes while downloading and
//! checking the download. A step past it fails, and its answer, should it come, is ignored.
//!
//! A release carries `exo-v{version}-macos-arm64.dmg` and `checksums.txt`, whose SHA-256 the
//! image must have. The Exo in the image must have Exo's bundle identifier, the release's
//! version, this Exo's signing team and Gatekeeper's approval, and answer `--version` with `exo
//! {version}`. An Exo that isn't signed by a team, as a local build isn't, doesn't update itself.

mod copy;

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::{Duration, Instant};

use fastframe_update::{
    MacConfig, MacTarget, Prepared, Release, ReqwestTransport, UpdateConfig, Updater,
};

pub use copy::{ALERT_TITLE, Alert, Connection, Dialog, INSTALL_AND_RESTART, NOT_NOW};

use crate::desktop::Launcher;
use crate::notifications::Notifier;

/// Where Exo's releases are, on GitHub.
const REPOSITORY: &str = "m1guelpf/Exo";

/// Exo as its updates know it: its releases, its bundle, and the Apple silicon disk image a
/// release carries.
pub const CONFIG: UpdateConfig = UpdateConfig {
    macos: MacConfig {
        bundle_ids: &["build.miguel.Exo"],
        executable_names: &["Exo"],
        legacy_bundle_names: &[],
    },
    mac_target: MacTarget::Arm64Only,
    ..UpdateConfig::new(REPOSITORY, "Exo", "exo", env!("CARGO_PKG_VERSION"))
};

/// What Exo restarts with after installing an update it was asked to: its window comes back
/// without taking the focus from the app in front.
pub const UPDATED: &str = "--updated";

/// What `--version` prints: `exo {version}`, which the helper of an earlier Exo checks a
/// download with.
pub fn version() -> String {
    format!("{} {}", CONFIG.slug, CONFIG.current_version)
}

/// How long the updater waits.
#[derive(Clone, Copy, Debug)]
struct Timing {
    /// From the end of one check to the next.
    between: Duration,
    /// For a step to finish.
    step: Duration,
    /// For a download's next bytes, and once they're in, for its disk image to check out.
    download: Duration,
}

const TIMING: Timing = Timing {
    between: Duration::from_secs(15 * 60),
    step: Duration::from_secs(30),
    download: Duration::from_secs(90),
};

/// Where the updater is, as the app menu's item says it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    /// Not checking: it never has, found nothing newer, or failed.
    #[default]
    Idle,
    /// Asking for the newest release.
    Checking,
    /// A newer release was found, and its download is starting.
    Available,
    /// Downloading the release.
    Downloading,
    /// Downloaded; its disk image is being checked.
    Preparing,
    /// Downloaded and checked, ready to install.
    Downloaded,
    /// Handed to the helper; Exo is about to quit.
    Restarting,
}

/// How a check shows itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Nobody asked: a newer release downloads, then a notification says it's ready.
    #[default]
    Background,
    /// ⌘K's: alerts say how it goes, and Exo restarts into the update once downloaded.
    Alerts,
    /// The app menu's with no mail on screen: dialogs ask before downloading and installing.
    Dialogs,
}

/// Exo's releases, as the updater uses them. [`Updater`] is the real one.
pub trait Releases: Send + Sync + 'static {
    /// The newest release, when it's newer than this Exo.
    ///
    /// # Errors
    /// When it couldn't be found out.
    fn check(&self) -> anyhow::Result<Option<Release>>;

    /// Downloads `release` and checks it, telling `progress` the bytes received of the total.
    ///
    /// # Errors
    /// When it couldn't be downloaded, or didn't check out.
    fn download(
        &self,
        release: &Release,
        progress: &mut dyn FnMut(u64, u64),
    ) -> anyhow::Result<Prepared>;

    /// Hands `prepared` to the helper, which installs it once Exo quits and opens the new Exo
    /// with `arguments`.
    ///
    /// # Errors
    /// When the helper didn't start.
    fn install(&self, prepared: Prepared, arguments: Vec<String>) -> anyhow::Result<()>;
}

impl Releases for Updater {
    fn check(&self) -> anyhow::Result<Option<Release>> {
        Updater::check(self)
    }

    fn download(
        &self,
        release: &Release,
        progress: &mut dyn FnMut(u64, u64),
    ) -> anyhow::Result<Prepared> {
        Updater::download(self, release, progress)
    }

    fn install(&self, prepared: Prepared, arguments: Vec<String>) -> anyhow::Result<()> {
        self.handoff(prepared, arguments)
    }
}

/// What the updater asks of the app around it.
pub trait Shell: Send + Sync + 'static {
    /// Says the update to `version` is ready to install.
    fn ready(&self, version: &str);
    /// Shows `dialog` and waits for the button chosen, from 0; `None` when it couldn't show.
    fn ask(&self, dialog: &Dialog) -> Option<usize>;
    /// Quits Exo, for the helper to install the update.
    fn quit(&self);
    /// What the updater shows changed: its alert, or the app menu's item.
    fn changed(&self);
}

/// The updater for this copy of Exo, when it can replace itself: Exo.app signed by a team, not
/// run from its disk image or a copy macOS made of a download.
pub fn updater() -> Option<Updater> {
    let transport = ReqwestTransport::new(reqwest::blocking::Client::builder())
        .inspect_err(|error| log::warn!("the updater's HTTP client: {error}"))
        .ok()?;
    let updater = Updater::new(CONFIG, transport);
    match updater.installation() {
        Ok(installation) if signed_by_a_team(&installation.executable) => Some(updater),
        Ok(_) => {
            log::info!("Exo can't update itself: it isn't signed by a team");
            None
        }
        Err(reason) => {
            log::info!("Exo can't update itself: {reason}");
            None
        }
    }
}

/// Whether the Exo at `executable` is signed by a team. Only then must an update be signed by the
/// same team and pass Gatekeeper, so a copy signed ad hoc (a local build) doesn't update itself:
/// it would take any release whose checksum, identity and version match.
fn signed_by_a_team(executable: &std::path::Path) -> bool {
    std::process::Command::new("/usr/bin/codesign")
        .args(["--display", "--verbose=4"])
        .arg(executable)
        .output()
        .is_ok_and(|output| {
            output.status.success() && team(&String::from_utf8_lossy(&output.stderr)).is_some()
        })
}

/// The team in `codesign --display --verbose=4`'s output, which it writes to stderr.
fn team(display: &str) -> Option<&str> {
    display
        .lines()
        .find_map(|line| line.strip_prefix("TeamIdentifier="))
        .filter(|team| *team != "not set")
}

/// Installs the update an earlier run downloaded and nobody installed, if there's one: the
/// helper takes it, swaps it in once this process exits, and opens the new Exo with
/// `arguments`, this launch's own. Whether it did, for the launch to quit at once.
pub fn install_kept(updater: &Updater, arguments: Vec<String>) -> bool {
    let prepared = match updater.kept() {
        Ok(Some(prepared)) => prepared,
        Ok(None) => return false,
        Err(error) => {
            log::warn!("an update downloaded earlier: {error:#}");
            return false;
        }
    };
    let version = prepared.version().to_owned();
    match updater.handoff(prepared, arguments) {
        Ok(()) => {
            log::info!("installing Exo {version}, downloaded earlier");
            true
        }
        Err(error) => {
            log::warn!("installing Exo {version}, downloaded earlier: {error:#}");
            false
        }
    }
}

/// The updater: when to check, how it shows, and the update downloaded, run on a thread of its
/// own for as long as Exo runs. Cheap to clone, and usable from any thread.
#[derive(Clone)]
pub struct Updates(Arc<Shared>);

struct Shared {
    state: Mutex<State>,
    /// Wakes the worker for what's asked of it.
    wake: Condvar,
    releases: Arc<dyn Releases>,
    shell: Arc<dyn Shell>,
    timing: Timing,
}

#[derive(Default)]
struct State {
    stage: Stage,
    /// How the check under way shows itself.
    mode: Mode,
    /// A check is under way.
    busy: bool,
    /// A check asked for, for the worker to start.
    asked: Option<Mode>,
    /// Installing the update was asked for.
    install: bool,
    /// How far the download is, in percent.
    percent: u8,
    /// The alert on screen.
    alert: Option<Alert>,
    /// An account's mail is on screen, which can show the alerts.
    alerts_shown: bool,
    /// The update downloaded this run, ready to install.
    downloaded: Option<Prepared>,
}

impl State {
    /// Where a finished check leaves the updater: ready when there's an update to install.
    fn settled(&self) -> Stage {
        if self.downloaded.is_some() {
            Stage::Downloaded
        } else {
            Stage::Idle
        }
    }
}

/// What the worker does next.
enum Job {
    Check(Mode),
    Install,
}

/// What a step's thread sends.
enum Step<T> {
    Progress(u64, u64),
    Done(anyhow::Result<T>),
}

/// Why a step failed.
#[derive(Debug)]
enum Failure {
    Error(anyhow::Error),
    /// Past its deadline, in this stage.
    TimedOut(Stage),
    /// Its thread ended without an answer.
    Stopped,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error(error) => write!(formatter, "{error:#}"),
            Self::TimedOut(stage) => write!(formatter, "{stage:?} timed out"),
            Self::Stopped => formatter.write_str("a step stopped without an answer"),
        }
    }
}

impl Failure {
    fn connection(&self) -> Option<Connection> {
        match self {
            Self::Error(error) => Connection::of(error),
            Self::TimedOut(_) | Self::Stopped => None,
        }
    }
}

/// The real app around the updater: notifications through `notifier`, dialogs and quitting
/// through AppKit, and the window woken through `launcher`.
struct AppShell {
    launcher: Launcher,
    notifier: Option<Notifier>,
}

impl Shell for AppShell {
    fn ready(&self, version: &str) {
        if let Some(notifier) = &self.notifier {
            notifier.post_update(version);
        }
    }

    fn ask(&self, dialog: &Dialog) -> Option<usize> {
        crate::desktop::ask(dialog.title(), &dialog.body(), dialog.buttons())
    }

    fn quit(&self) {
        crate::desktop::quit();
    }

    fn changed(&self) {
        self.launcher.repaint();
    }
}

impl Updates {
    /// Updates this Exo with `updater`, checking now and every 15 minutes after: notifications
    /// through `notifier`, the window woken through `launcher`.
    pub fn start(updater: Updater, launcher: Launcher, notifier: Option<Notifier>) -> Self {
        Self::spawn(updater, AppShell { launcher, notifier }, TIMING)
    }

    /// An updater over `releases` and `shell`, checking now and every 15 minutes after.
    pub fn with(releases: impl Releases, shell: impl Shell) -> Self {
        Self::spawn(releases, shell, TIMING)
    }

    fn spawn(releases: impl Releases, shell: impl Shell, timing: Timing) -> Self {
        let updates = Self(Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            releases: Arc::new(releases),
            shell: Arc::new(shell),
            timing,
        }));
        let worker = updates.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("exo-updates".to_owned())
            .spawn(move || worker.run())
        {
            log::error!("starting the updater: {error}");
        }
        updates
    }

    /// Where the updater is.
    pub fn stage(&self) -> Stage {
        self.lock().stage
    }

    /// The alert to show, while a check in front of the user has one.
    pub fn alert(&self) -> Option<Alert> {
        self.lock().alert
    }

    /// Checks now, shown as `mode` says. With a check under way already, that check shows
    /// itself so from now on.
    pub fn check(&self, mode: Mode) {
        {
            let mut state = self.lock();
            if mode == Mode::Alerts {
                let downloading = matches!(
                    state.stage,
                    Stage::Available | Stage::Downloading | Stage::Preparing
                );
                state.alert = Some(if state.busy && downloading {
                    Alert::Installing(state.percent)
                } else {
                    Alert::Checking
                });
            }
            match (mode, state.busy) {
                (Mode::Alerts | Mode::Dialogs, true) => state.mode = mode,
                (Mode::Alerts | Mode::Dialogs, false) => state.asked = Some(mode),
                // One nobody asked for doesn't take over one asked for.
                (Mode::Background, false) if state.asked.is_none() => state.asked = Some(mode),
                (Mode::Background, _) => {}
            }
        }
        self.0.wake.notify_all();
        self.0.shell.changed();
    }

    /// The app menu's item chosen: installs the update ready, or else checks, with alerts while
    /// an account's mail is on screen to show them and dialogs otherwise.
    pub fn menu_chosen(&self) {
        let (stage, alerts_shown) = {
            let state = self.lock();
            (state.stage, state.alerts_shown)
        };
        match stage {
            Stage::Downloaded => self.install(),
            _ => self.check(if alerts_shown {
                Mode::Alerts
            } else {
                Mode::Dialogs
            }),
        }
    }

    /// Installs the update ready, once a check under way is over.
    pub fn install(&self) {
        self.lock().install = true;
        self.0.wake.notify_all();
    }

    /// Closes the alert. A check it showed goes on as one nobody asked for.
    pub fn dismiss(&self) {
        let mut state = self.lock();
        state.alert = None;
        if state.mode == Mode::Alerts {
            state.mode = Mode::Background;
        }
    }

    /// Whether an account's mail is on screen, which shows the alerts.
    pub fn set_alerts_shown(&self, shown: bool) {
        self.lock().alerts_shown = shown;
    }

    /// The helper put this Exo back, as the update it installed didn't open: `Failed to update`
    /// says so.
    pub fn restored(&self) {
        let shell = Arc::clone(&self.0.shell);
        if let Err(error) = std::thread::Builder::new()
            .name("exo-update-restored".to_owned())
            .spawn(move || {
                shell.ask(&Dialog::Failed(None));
            })
        {
            log::warn!("saying the update didn't install: {error}");
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The worker: a check now, then each one asked for and 15 minutes after the last ended,
    /// and installs asked for.
    fn run(self) {
        let mut due = Instant::now();
        loop {
            match self.next(due) {
                Job::Check(mode) => {
                    self.cycle(mode);
                    due = Instant::now() + self.0.timing.between;
                }
                Job::Install => self.install_now(),
            }
        }
    }

    /// Waits for what's asked, or for the check due.
    fn next(&self, due: Instant) -> Job {
        let mut state = self.lock();
        loop {
            if std::mem::take(&mut state.install) {
                return Job::Install;
            }
            if let Some(mode) = state.asked.take() {
                return Job::Check(mode);
            }
            let now = Instant::now();
            if now >= due {
                return Job::Check(Mode::Background);
            }
            state = self
                .0
                .wake
                .wait_timeout(state, due - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// A check, shown as `mode` says to start with, and what follows from it.
    fn cycle(&self, mode: Mode) {
        {
            let mut state = self.lock();
            state.busy = true;
            state.mode = mode;
            state.percent = 0;
        }
        if let Err(failure) = self.check_and_download() {
            self.failed(failure);
        }
        {
            let mut state = self.lock();
            state.busy = false;
            state.mode = Mode::Background;
        }
        self.0.shell.changed();
    }

    fn check_and_download(&self) -> Result<(), Failure> {
        let releases = Arc::clone(&self.0.releases);
        let Some(release) = self.step(Stage::Checking, self.0.timing.step, move |_| {
            releases.check()
        })?
        else {
            self.up_to_date();
            return Ok(());
        };
        let downloaded = self
            .lock()
            .downloaded
            .as_ref()
            .map(|prepared| prepared.version().to_owned());
        if downloaded.is_some_and(|version| !fastframe_update::is_newer(&release.version, &version))
        {
            self.ready(false);
            return Ok(());
        }
        let mode = self.lock().mode;
        if mode == Mode::Dialogs
            && self
                .0
                .shell
                .ask(&Dialog::Available(release.version.clone()))
                != Some(0)
        {
            let mut state = self.lock();
            state.stage = state.settled();
            return Ok(());
        }
        {
            let mut state = self.lock();
            state.stage = Stage::Available;
            // The download clears away the folders of earlier ones, this one's too.
            state.downloaded = None;
        }
        self.0.shell.changed();
        let releases = Arc::clone(&self.0.releases);
        let prepared = self.step(
            Stage::Downloading,
            self.0.timing.download,
            move |progress| releases.download(&release, progress),
        )?;
        self.lock().downloaded = Some(prepared);
        self.ready(true);
        Ok(())
    }

    /// Nothing newer: the alert or the dialog says so.
    fn up_to_date(&self) {
        let mode = {
            let mut state = self.lock();
            state.stage = state.settled();
            if state.mode == Mode::Alerts {
                state.alert = Some(Alert::Latest);
            }
            state.mode
        };
        self.0.shell.changed();
        if mode == Mode::Dialogs {
            self.0.shell.ask(&Dialog::UpToDate);
        }
    }

    /// The update is downloaded (`fresh`ly, or by an earlier check): a notification says so,
    /// Exo restarts into it, or a dialog asks.
    fn ready(&self, fresh: bool) {
        let (mode, version) = {
            let mut state = self.lock();
            state.stage = Stage::Downloaded;
            let version = state
                .downloaded
                .as_ref()
                .map(|prepared| prepared.version().to_owned());
            (state.mode, version.unwrap_or_default())
        };
        self.0.shell.changed();
        match mode {
            Mode::Background if fresh => self.0.shell.ready(&version),
            Mode::Background => {}
            Mode::Alerts => self.install_now(),
            Mode::Dialogs => {
                if self.0.shell.ask(&Dialog::Ready(version)) == Some(0) {
                    self.install_now();
                }
            }
        }
    }

    /// Hands the update ready to the helper and quits.
    fn install_now(&self) {
        let prepared = {
            let mut state = self.lock();
            if state.stage != Stage::Downloaded {
                return;
            }
            let Some(prepared) = state.downloaded.take() else {
                return;
            };
            prepared
        };
        let releases = Arc::clone(&self.0.releases);
        let installed = self.step(Stage::Restarting, self.0.timing.step, move |_| {
            releases.install(prepared, vec![UPDATED.to_owned()])
        });
        match installed {
            Ok(()) => {
                self.0.shell.quit();
                // Exo quits now; still running past the deadline, it didn't.
                std::thread::sleep(self.0.timing.step);
                self.failed(Failure::TimedOut(Stage::Restarting));
            }
            Err(failure) => self.failed(failure),
        }
    }

    /// A step failed: the alert or the dialog says so.
    fn failed(&self, failure: Failure) {
        log::warn!("updating Exo: {failure}");
        let connection = failure.connection();
        let mode = {
            let mut state = self.lock();
            state.stage = state.settled();
            if state.mode == Mode::Alerts {
                state.alert = Some(if connection == Some(Connection::Unreachable) {
                    Alert::Offline
                } else {
                    Alert::Failed
                });
            }
            state.mode
        };
        self.0.shell.changed();
        if mode == Mode::Dialogs {
            self.0.shell.ask(&Dialog::Failed(connection));
        }
    }

    /// Runs `work` in `stage` on a thread of its own, within `limit` of the start or of the last
    /// progress it told.
    fn step<T: Send + 'static>(
        &self,
        stage: Stage,
        limit: Duration,
        work: impl FnOnce(&mut dyn FnMut(u64, u64)) -> anyhow::Result<T> + Send + 'static,
    ) -> Result<T, Failure> {
        self.lock().stage = stage;
        self.0.shell.changed();
        let (sender, receiver) = mpsc::channel();
        let progress = sender.clone();
        std::thread::Builder::new()
            .name("exo-update-step".to_owned())
            .spawn(move || {
                let result = work(&mut |received, total| {
                    let _ = progress.send(Step::Progress(received, total));
                });
                let _ = sender.send(Step::Done(result));
            })
            .map_err(|error| Failure::Error(error.into()))?;
        let mut deadline = Instant::now() + limit;
        loop {
            match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Step::Progress(received, total)) => {
                    deadline = Instant::now() + limit;
                    self.progress(received, total);
                }
                Ok(Step::Done(result)) => return result.map_err(Failure::Error),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err(Failure::TimedOut(self.stage()));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(Failure::Stopped),
            }
        }
    }

    /// The download is `received` bytes of `total` in: once they're all in, its disk image is
    /// being checked.
    fn progress(&self, received: u64, total: u64) {
        {
            let mut state = self.lock();
            let percent = if total == 0 {
                0
            } else {
                (received.min(total) as f64 * 100.0 / total as f64).round() as u8
            };
            state.percent = percent;
            state.stage = if total > 0 && received >= total {
                Stage::Preparing
            } else {
                Stage::Downloading
            };
            if state.mode == Mode::Alerts {
                state.alert = Some(Alert::Installing(percent));
            }
        }
        self.0.shell.changed();
    }
}

#[cfg(test)]
mod tests;
