//! One Exo at a time: a later launch hands the running Exo what it was asked for, its `mailto:`
//! link or else the window, and exits. A launch of the bundle while Exo runs doesn't get this far:
//! macOS hands the running Exo a Dock click (the reopen event) or the link (`kAEGetURL`) instead.

use fastframe_instance::{Claim, Guard, Slot};

use super::Launcher;

/// The slot's name, which starts each request.
const APP: &str = "Exo";
/// Asks for the window.
const SHOW: &str = "show";
/// Asks to open the URL after it.
const OPEN: &str = "open ";

/// The link a launch from the command line asks to open: the first `mailto:` or deep link (`exo:`)
/// among its arguments.
pub fn launch_url(args: impl IntoIterator<Item = String>) -> Option<String> {
    args.into_iter()
        .find(|arg| arg.starts_with("mailto:") || arg.starts_with(crate::links::SCHEME))
}

/// Why this launch isn't the running Exo, and is done.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum Unclaimed {
    /// The running Exo took the link or the request for the window.
    #[error("Exo is already running, and took the request")]
    HandedOver,
    /// The running Exo didn't understand the request.
    #[error("the running Exo turned down the request")]
    Declined,
    /// An Exo holds the slot but didn't answer in time.
    #[error("Exo is already running, but didn't answer")]
    Unanswered,
}

/// Makes this process the running Exo, which opens `url` once its window is up; or hands `url`, or
/// else a request for the window, to the Exo already running. `scope`, when given, is a slot of its
/// own beside the real one's, for an Exo that should run alongside it (on gmail-mock). Keep the
/// guard until the process exits; an error means this launch is done.
///
/// # Errors
/// When another Exo is running: it took the request, turned it down or didn't answer.
pub fn claim(
    scope: Option<&str>,
    url: Option<String>,
    launcher: &Launcher,
) -> Result<Guard, Unclaimed> {
    let slot = match scope {
        Some(scope) => Slot::new(APP).scoped(scope),
        None => Slot::new(APP),
    };
    claim_at(&slot, url, launcher)
}

fn claim_at(slot: &Slot, url: Option<String>, launcher: &Launcher) -> Result<Guard, Unclaimed> {
    let request = url
        .as_ref()
        .map_or_else(|| SHOW.to_owned(), |url| format!("{OPEN}{url}"));
    let running = launcher.clone();
    let claim = slot.claim(&request, move |request| {
        match request.strip_prefix(OPEN) {
            Some(url) => running.open_url(url),
            None if request == SHOW => running.show(),
            None => return None,
        }
        Some(String::new())
    });
    match claim {
        Claim::First(guard) => {
            if let Some(url) = url {
                launcher.open_url(&url);
            }
            Ok(guard)
        }
        Claim::Running(_) => Err(Unclaimed::HandedOver),
        Claim::Declined => Err(Unclaimed::Declined),
        Claim::Unanswered => Err(Unclaimed::Unanswered),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::Request;

    #[test]
    fn the_launch_url_is_the_first_mailto_argument() {
        let args = |args: &[&str]| args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            launch_url(args(&[
                "exo",
                "-v",
                "mailto:ana@example.com",
                "mailto:bo@example.com"
            ]))
            .as_deref(),
            Some("mailto:ana@example.com")
        );
        assert_eq!(launch_url(args(&["exo", "https://example.com"])), None);
    }

    #[test]
    fn a_later_launch_hands_its_link_or_the_window_over_and_exits() {
        let dir = std::env::temp_dir().join(format!("exo-instance-{}", std::process::id()));
        let slot = Slot::at(&dir, "exo-test");
        let running = Launcher::default();
        let guard = claim_at(&slot, Some("mailto:ana@example.com".into()), &running)
            .expect("the first launch runs");
        assert_eq!(
            running.take(),
            [Request::Open("mailto:ana@example.com".into())],
            "its own link"
        );

        let later = Launcher::default();
        assert_eq!(
            claim_at(
                &slot,
                Some("mailto:bo@example.com?subject=Hi".into()),
                &later
            )
            .err(),
            Some(Unclaimed::HandedOver)
        );
        assert_eq!(
            claim_at(&slot, None, &later).err(),
            Some(Unclaimed::HandedOver)
        );
        assert_eq!(
            running.take(),
            [
                Request::Open("mailto:bo@example.com?subject=Hi".into()),
                Request::Show
            ]
        );
        assert!(
            later.take().is_empty(),
            "the later launches ran nothing themselves"
        );

        drop(guard);
        let _ = std::fs::remove_dir_all(dir);
    }
}
