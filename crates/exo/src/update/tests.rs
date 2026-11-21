use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender, channel};

use fastframe_update::{Installation, Kind};

use super::*;

/// Long enough that only the steps made to hang miss them.
const SLOW: Timing = Timing {
    between: Duration::from_secs(3600),
    step: Duration::from_secs(3),
    download: Duration::from_secs(3),
};

/// Releases served from memory: the newest release, how a download goes, and what was asked.
#[derive(Default)]
struct Fake {
    /// What a check finds.
    latest: Mutex<Option<&'static str>>,
    /// How long a check takes.
    check_takes: Duration,
    /// What a check fails with, if it does.
    check_fails: Option<&'static str>,
    /// Progress a download tells, waiting for a go from the test before each when given.
    progress: Vec<(u64, u64)>,
    gate: Mutex<Option<Receiver<()>>>,
    /// The pause between a download's progress reports.
    pace: Duration,
    /// What was asked: `check`, `download 0.2.0`, `install 0.2.0 --updated`.
    asked: Mutex<Vec<String>>,
}

impl Releases for Arc<Fake> {
    fn check(&self) -> anyhow::Result<Option<Release>> {
        self.asked.lock().unwrap().push("check".to_owned());
        std::thread::sleep(self.check_takes);
        if let Some(error) = self.check_fails {
            anyhow::bail!(error);
        }
        Ok(self.latest.lock().unwrap().map(|version| Release {
            version: version.to_owned(),
            url: String::new(),
            published_at: None,
        }))
    }

    fn download(
        &self,
        release: &Release,
        progress: &mut dyn FnMut(u64, u64),
    ) -> anyhow::Result<Prepared> {
        self.asked
            .lock()
            .unwrap()
            .push(format!("download {}", release.version));
        let gate = self.gate.lock().unwrap().take();
        for &(received, total) in &self.progress {
            if let Some(gate) = &gate {
                let _ = gate.recv();
            }
            std::thread::sleep(self.pace);
            progress(received, total);
        }
        // Nothing installs a sample: a handoff refuses it, and this one never gets there.
        let installation = Installation {
            executable: "/nonexistent/Exo.app/Contents/MacOS/Exo".into(),
            kind: Kind::MacBundle,
        };
        Ok(Prepared::sample(installation, &release.version))
    }

    fn install(&self, prepared: Prepared, arguments: Vec<String>) -> anyhow::Result<()> {
        self.asked.lock().unwrap().push(format!(
            "install {} {}",
            prepared.version(),
            arguments.join(" ")
        ));
        Ok(())
    }
}

impl Fake {
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }

    /// The download waits for a go before each progress report.
    fn gated(&self) -> Sender<()> {
        let (go, gate) = channel();
        *self.gate.lock().unwrap() = Some(gate);
        go
    }
}

/// The app around the updater, keeping what it was asked and answering dialogs in turn.
#[derive(Default)]
struct Recorder {
    said: Mutex<Vec<String>>,
    answers: Mutex<VecDeque<usize>>,
}

impl Shell for Arc<Recorder> {
    fn ready(&self, version: &str) {
        self.said.lock().unwrap().push(format!("ready {version}"));
    }

    fn ask(&self, dialog: &Dialog) -> Option<usize> {
        self.said
            .lock()
            .unwrap()
            .push(format!("{}: {}", dialog.title(), dialog.body()));
        self.answers.lock().unwrap().pop_front()
    }

    fn quit(&self) {
        self.said.lock().unwrap().push("quit".to_owned());
    }

    fn changed(&self) {}
}

impl Recorder {
    fn said(&self) -> Vec<String> {
        self.said.lock().unwrap().clone()
    }
}

fn start(fake: &Arc<Fake>, recorder: &Arc<Recorder>, timing: Timing) -> Updates {
    Updates::spawn(Arc::clone(fake), Arc::clone(recorder), timing)
}

/// Waits up to 10 seconds for `done`.
fn wait(what: &str, done: impl Fn() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn found(version: &'static str) -> Arc<Fake> {
    Arc::new(Fake {
        latest: Mutex::new(Some(version)),
        progress: vec![(50, 100), (100, 100)],
        ..Fake::default()
    })
}

#[test]
fn exo_knows_its_releases_and_bundle() {
    CONFIG.validate().expect("a valid configuration");
    assert_eq!(version(), format!("exo {}", env!("CARGO_PKG_VERSION")));
}

#[test]
fn only_a_copy_signed_by_a_team_updates_itself() {
    let signed = "Executable=/Applications/Exo.app/Contents/MacOS/Exo\nIdentifier=build.miguel.Exo\nTeamIdentifier=LR4RJT6586\nSealed Resources version=2\n";
    assert_eq!(team(signed), Some("LR4RJT6586"));
    let ad_hoc = "Identifier=build.miguel.Exo\nSignature=adhoc\nTeamIdentifier=not set\n";
    assert_eq!(team(ad_hoc), None);
    assert_eq!(team("code object is not signed at all\n"), None);
}

#[test]
fn nothing_newer_says_so_when_asked_and_nothing_otherwise() {
    let (fake, recorder) = (Arc::new(Fake::default()), Arc::new(Recorder::default()));
    let updates = start(&fake, &recorder, SLOW);
    wait("the launch's check", || {
        fake.asked().len() == 1 && updates.stage() == Stage::Idle && !updates.lock().busy
    });
    assert_eq!(updates.alert(), None, "nobody asked");
    updates.check(Mode::Alerts);
    assert_eq!(updates.alert(), Some(Alert::Checking), "at once");
    wait("the answer", || updates.alert() == Some(Alert::Latest));
    updates.dismiss();
    assert_eq!(updates.alert(), None);
    assert_eq!(recorder.said(), Vec::<String>::new());
}

#[test]
fn a_release_found_downloads_says_its_ready_and_installs_when_asked() {
    let (fake, recorder) = (found("9.0.0"), Arc::new(Recorder::default()));
    let updates = start(&fake, &recorder, SLOW);
    wait("the download", || updates.stage() == Stage::Downloaded);
    assert_eq!(recorder.said(), ["ready 9.0.0"]);
    assert_eq!(fake.asked(), ["check", "download 9.0.0"]);
    assert_eq!(
        Stage::Downloaded.menu_item().0,
        "Install Update and Restart"
    );

    // Checking again finds what's downloaded already: no second download or notification.
    updates.check(Mode::Background);
    wait("the second check", || fake.asked().len() == 3);
    wait("its end", || !updates.lock().busy);
    assert_eq!(updates.stage(), Stage::Downloaded);
    assert_eq!(recorder.said(), ["ready 9.0.0"]);

    updates.menu_chosen();
    wait("the helper", || recorder.said().len() == 2);
    assert_eq!(fake.asked()[3], "install 9.0.0 --updated");
    assert_eq!(recorder.said()[1], "quit");
    assert_eq!(updates.stage(), Stage::Restarting);
}

#[test]
fn asked_in_front_it_shows_the_download_then_restarts_into_it() {
    let (fake, recorder) = (found("9.0.0"), Arc::new(Recorder::default()));
    *fake.latest.lock().unwrap() = None;
    let go = fake.gated();
    let updates = start(&fake, &recorder, SLOW);
    wait("the launch's check", || {
        fake.asked().len() == 1 && !updates.lock().busy
    });
    *fake.latest.lock().unwrap() = Some("9.0.0");
    updates.check(Mode::Alerts);
    go.send(()).unwrap();
    wait("half the download", || {
        updates.alert() == Some(Alert::Installing(50))
    });
    assert_eq!(updates.stage(), Stage::Downloading);
    go.send(()).unwrap();
    wait("the restart", || recorder.said() == ["quit"]);
    assert_eq!(
        fake.asked(),
        [
            "check",
            "check",
            "download 9.0.0",
            "install 9.0.0 --updated"
        ]
    );
}

#[test]
fn closing_the_alert_leaves_the_download_to_say_its_ready() {
    let (fake, recorder) = (found("9.0.0"), Arc::new(Recorder::default()));
    *fake.latest.lock().unwrap() = None;
    let go = fake.gated();
    let updates = start(&fake, &recorder, SLOW);
    wait("the launch's check", || {
        fake.asked().len() == 1 && !updates.lock().busy
    });
    *fake.latest.lock().unwrap() = Some("9.0.0");
    updates.check(Mode::Alerts);
    go.send(()).unwrap();
    wait("half the download", || {
        updates.alert() == Some(Alert::Installing(50))
    });
    updates.dismiss();
    go.send(()).unwrap();
    wait("the download", || updates.stage() == Stage::Downloaded);
    wait("the notification", || recorder.said() == ["ready 9.0.0"]);
    assert_eq!(updates.alert(), None);
    assert!(
        !fake
            .asked()
            .iter()
            .any(|asked| asked.starts_with("install"))
    );
}

#[test]
fn the_app_menu_asks_before_downloading_and_installing() {
    let (fake, recorder) = (found("9.0.0"), Arc::new(Recorder::default()));
    *fake.latest.lock().unwrap() = None;
    let updates = start(&fake, &recorder, SLOW);
    wait("the launch's check", || {
        fake.asked().len() == 1 && !updates.lock().busy
    });
    updates.menu_chosen();
    wait("the answer", || {
        recorder.said() == ["Exo is up to date: You're using the latest and greatest!"]
    });

    *fake.latest.lock().unwrap() = Some("9.0.0");
    // `Download`, then `Not now`.
    recorder.answers.lock().unwrap().extend([0, 1]);
    updates.menu_chosen();
    wait("the second dialog", || recorder.said().len() == 3);
    assert_eq!(
        recorder.said()[1..],
        [
            "Update available!: Ready to download Exo 9.0.0.",
            "Update ready!: Restart to start using Exo 9.0.0."
        ]
    );
    wait("its end", || !updates.lock().busy);
    assert_eq!(updates.stage(), Stage::Downloaded);

    // The item now says `Install Update and Restart`, and does.
    updates.menu_chosen();
    wait("the helper", || recorder.said().len() == 4);
    assert_eq!(fake.asked().last().unwrap(), "install 9.0.0 --updated");

    // An alert shows instead where an account's mail is on screen.
    let (fake, recorder) = (Arc::new(Fake::default()), Arc::new(Recorder::default()));
    let updates = start(&fake, &recorder, SLOW);
    updates.set_alerts_shown(true);
    updates.menu_chosen();
    wait("the answer", || updates.alert() == Some(Alert::Latest));
    assert_eq!(recorder.said(), Vec::<String>::new());
}

#[test]
fn a_failure_says_so_where_it_was_asked() {
    let fake = Arc::new(Fake {
        check_fails: Some("The update server answered 500"),
        ..Fake::default()
    });
    let recorder = Arc::new(Recorder::default());
    let updates = start(&fake, &recorder, SLOW);
    wait("the launch's check", || {
        fake.asked().len() == 1 && !updates.lock().busy
    });
    assert_eq!(updates.alert(), None, "the launch's says nothing");
    updates.check(Mode::Alerts);
    wait("the alert", || updates.alert() == Some(Alert::Failed));
    updates.check(Mode::Dialogs);
    wait("the dialog", || {
        recorder.said() == ["Failed to update: An error occurred while updating."]
    });
    assert_eq!(updates.stage(), Stage::Idle);
}

#[test]
fn a_step_past_its_deadline_fails() {
    let fake = Arc::new(Fake {
        check_takes: Duration::from_secs(5),
        ..Fake::default()
    });
    let recorder = Arc::new(Recorder::default());
    let timing = Timing {
        step: Duration::from_millis(200),
        ..SLOW
    };
    let updates = start(&fake, &recorder, timing);
    updates.check(Mode::Alerts);
    let asked = Instant::now();
    wait("the deadline", || updates.alert() == Some(Alert::Failed));
    assert!(
        asked.elapsed() < Duration::from_secs(4),
        "not the late answer"
    );
    assert_eq!(updates.stage(), Stage::Idle);
}

#[test]
fn a_download_that_keeps_coming_has_no_deadline() {
    let fake = Arc::new(Fake {
        latest: Mutex::new(Some("9.0.0")),
        progress: (1..=12).map(|part| (part, 12)).collect(),
        pace: Duration::from_millis(50),
        ..Fake::default()
    });
    let recorder = Arc::new(Recorder::default());
    let timing = Timing {
        download: Duration::from_millis(400),
        ..SLOW
    };
    let updates = start(&fake, &recorder, timing);
    wait("the download", || updates.stage() == Stage::Downloaded);
    assert_eq!(recorder.said(), ["ready 9.0.0"]);
}

#[test]
fn it_checks_again_a_while_after_each_check_ends() {
    let (fake, recorder) = (Arc::new(Fake::default()), Arc::new(Recorder::default()));
    let timing = Timing {
        between: Duration::from_millis(50),
        ..SLOW
    };
    let _updates = start(&fake, &recorder, timing);
    wait("three checks", || fake.asked().len() >= 3);
}
