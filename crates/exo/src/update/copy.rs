//! What the updater says: the app menu's item, the alerts of ⌘K's `Check for Updates`, the
//! dialogs of the app menu's with no mail on screen, and the notification of an update ready.

use super::Stage;

/// The alerts' title.
pub const ALERT_TITLE: &str = "Update Exo";
/// The ready update's button that installs it.
pub const INSTALL_AND_RESTART: &str = "Install and Restart";
/// The ready update's button that leaves it for later.
pub const NOT_NOW: &str = "Not now";

impl Stage {
    /// The app menu's item: its title, and whether it can be chosen.
    pub fn menu_item(self) -> (&'static str, bool) {
        match self {
            Self::Idle => ("Check for Updates…", true),
            Self::Checking => ("Checking for Updates…", false),
            Self::Available => ("Update Available. Starting Download", false),
            Self::Downloading => ("Downloading Update…", false),
            Self::Preparing => ("Preparing to Install…", false),
            Self::Downloaded => ("Install Update and Restart", true),
            Self::Restarting => ("Restarting…", false),
        }
    }
}

/// An alert of ⌘K's `Check for Updates`, each with one button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alert {
    /// Checking, with `Cancel`.
    Checking,
    /// Nothing newer.
    Latest,
    /// The update server couldn't be reached.
    Offline,
    /// Downloading, this far in percent; `OK` leaves it to finish.
    Installing(u8),
    /// Something else went wrong.
    Failed,
}

impl Alert {
    /// Its text.
    pub fn text(self) -> String {
        match self {
            Self::Checking => "Checking for updates…".to_owned(),
            Self::Latest => "You are on the latest version of Exo!".to_owned(),
            Self::Offline => "Exo needs to connect to the internet to update. Please check your connection and try again.".to_owned(),
            Self::Installing(percent) => format!("Installing the latest update: {percent}%"),
            Self::Failed => "Exo could not complete the update. Please check your connection and try again.".to_owned(),
        }
    }

    /// Its button, which closes it.
    pub fn action(self) -> &'static str {
        match self {
            Self::Checking => "Cancel",
            Self::Latest | Self::Offline | Self::Installing(_) | Self::Failed => "OK",
        }
    }
}

/// A dialog of the app menu's `Check for Updates…` with no mail on screen, which waits for its
/// answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialog {
    /// A newer release, `Download` or `Not now`.
    Available(String),
    /// Nothing newer.
    UpToDate,
    /// Downloaded, `Install and Restart` or `Not now`: what the notification of an update ready
    /// says too.
    Ready(String),
    /// The update failed, saying why when it was the connection.
    Failed(Option<Connection>),
}

impl Dialog {
    /// Its message.
    pub fn title(&self) -> &'static str {
        match self {
            Self::Available(_) => "Update available!",
            Self::UpToDate => "Exo is up to date",
            Self::Ready(_) => "Update ready!",
            Self::Failed(_) => "Failed to update",
        }
    }

    /// The text under the message.
    pub fn body(&self) -> String {
        match self {
            Self::Available(version) => format!("Ready to download Exo {version}."),
            Self::UpToDate => "You're using the latest and greatest!".to_owned(),
            Self::Ready(version) => format!("Restart to start using Exo {version}."),
            Self::Failed(Some(Connection::Unreachable)) => {
                "Could not connect to the update server.".to_owned()
            }
            Self::Failed(Some(Connection::Lost)) => {
                "Connection lost while updating. Please try again.".to_owned()
            }
            Self::Failed(None) => "An error occurred while updating.".to_owned(),
        }
    }

    /// Its buttons, the default first.
    pub fn buttons(&self) -> &'static [&'static str] {
        match self {
            Self::Available(_) => &["Download", NOT_NOW],
            Self::Ready(_) => &[INSTALL_AND_RESTART, NOT_NOW],
            Self::UpToDate | Self::Failed(_) => &["OK"],
        }
    }
}

/// What went wrong with the connection, when that's why a step failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    /// The update server couldn't be reached.
    Unreachable,
    /// The connection dropped or stalled on the way.
    Lost,
}

impl Connection {
    /// The connection's part in `error`, if it had one.
    pub(super) fn of(error: &anyhow::Error) -> Option<Self> {
        use std::io::ErrorKind;
        error.chain().find_map(|cause| {
            if let Some(http) = http_error(cause) {
                if http.is_connect() {
                    return Some(Self::Unreachable);
                }
                if http.is_timeout() || http.is_body() || http.is_request() {
                    return Some(Self::Lost);
                }
            }
            let io = cause.downcast_ref::<std::io::Error>()?;
            matches!(
                io.kind(),
                ErrorKind::TimedOut
                    | ErrorKind::ConnectionReset
                    | ErrorKind::ConnectionAborted
                    | ErrorKind::BrokenPipe
            )
            .then_some(Self::Lost)
        })
    }
}

/// `cause` as the HTTP client's error, also where reading a body wrapped it in an I/O error.
fn http_error<'a>(cause: &'a (dyn std::error::Error + 'static)) -> Option<&'a reqwest::Error> {
    cause.downcast_ref::<reqwest::Error>().or_else(|| {
        cause
            .downcast_ref::<std::io::Error>()?
            .get_ref()?
            .downcast_ref::<reqwest::Error>()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_item_can_be_chosen_only_when_idle_or_ready() {
        let chosen: Vec<_> = [
            Stage::Idle,
            Stage::Checking,
            Stage::Available,
            Stage::Downloading,
            Stage::Preparing,
            Stage::Downloaded,
            Stage::Restarting,
        ]
        .into_iter()
        .filter(|stage| stage.menu_item().1)
        .map(|stage| stage.menu_item().0)
        .collect();
        assert_eq!(chosen, ["Check for Updates…", "Install Update and Restart"]);
    }

    #[test]
    fn a_dialog_names_the_version_and_why_it_failed() {
        let ready = Dialog::Ready("0.2.0".into());
        assert_eq!(
            (ready.title(), ready.body().as_str(), ready.buttons()),
            (
                "Update ready!",
                "Restart to start using Exo 0.2.0.",
                &["Install and Restart", "Not now"][..]
            )
        );
        assert_eq!(
            Dialog::Available("0.2.0".into()).body(),
            "Ready to download Exo 0.2.0."
        );
        assert_eq!(
            Dialog::Failed(Some(Connection::Lost)).body(),
            "Connection lost while updating. Please try again."
        );
        assert_eq!(
            Dialog::Failed(None).body(),
            "An error occurred while updating."
        );
    }

    #[test]
    fn a_stalled_or_dropped_connection_is_lost() {
        let stalled = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::TimedOut))
            .context("downloading the update");
        assert_eq!(Connection::of(&stalled), Some(Connection::Lost));
        let reset = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
        assert_eq!(Connection::of(&reset), Some(Connection::Lost));
        let other = anyhow::anyhow!("The download couldn't be verified. Try downloading it again.");
        assert_eq!(Connection::of(&other), None);
    }

    #[test]
    fn only_checking_can_be_cancelled() {
        assert_eq!(Alert::Checking.action(), "Cancel");
        assert_eq!(Alert::Installing(42).action(), "OK");
        assert_eq!(
            Alert::Installing(42).text(),
            "Installing the latest update: 42%"
        );
    }
}
