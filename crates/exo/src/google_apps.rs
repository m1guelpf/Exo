//! The palette's Google apps rows: Gmail, or the open message in Gmail, Google's other apps, and
//! Gmail's filter, alias and label settings, each opened in the default browser as the account.

use exo_commands::{Command, Commands};

use crate::{commands::Action, thread::ThreadView};

/// What a Google apps row opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum App {
    /// `open Gmail`.
    Gmail,
    /// `open in Gmail`: the message (or conversation) with this Gmail id.
    Message(String),
    Docs,
    Drive,
    Sheets,
    Calendar,
    Contacts,
    Slides,
    Hangouts,
    Meet,
    FilterSettings,
    AliasSettings,
    LabelSettings,
}

impl App {
    /// Its row's id and title.
    fn row(&self) -> (&'static str, &'static str) {
        match self {
            Self::Gmail => ("OPEN_GMAIL", "open Gmail"),
            Self::Message(_) => ("OPEN_IN_GMAIL", "open in Gmail"),
            Self::Docs => ("OPEN_GOOGLE_DOCS", "open Google Docs"),
            Self::Drive => ("OPEN_GOOGLE_DRIVE", "open Google Drive"),
            Self::Sheets => ("OPEN_GOOGLE_SHEETS", "open Google Sheets"),
            Self::Calendar => ("OPEN_GOOGLE_CALENDAR", "open Google Calendar"),
            Self::Contacts => ("OPEN_GOOGLE_CONTACTS", "open Google Contacts"),
            Self::Slides => ("OPEN_GOOGLE_SLIDES", "open Google Slides"),
            Self::Hangouts => ("OPEN_GOOGLE_HANGOUTS", "open Google Hangouts"),
            Self::Meet => ("OPEN_GOOGLE_MEET", "open Google Meet"),
            Self::FilterSettings => (
                "OPEN_GOOGLE_FILTER_SETTINGS",
                "open Filter Settings (Gmail)",
            ),
            Self::AliasSettings => ("OPEN_GOOGLE_ALIAS_SETTINGS", "open Alias Settings (Gmail)"),
            Self::LabelSettings => ("OPEN_GOOGLE_LABEL_SETTINGS", "open Label Settings (Gmail)"),
        }
    }

    /// Where it opens for `email`'s account. `nonce` makes a message's link new each time
    /// (`_shn`, a UUID), so Gmail goes to it even when it's already open there.
    pub fn url(&self, email: &str, nonce: &str) -> String {
        let base = match self {
            Self::Gmail => "https://mail.google.com/mail/?shxr=0".to_owned(),
            Self::Message(id) => {
                format!("https://mail.google.com/mail/?shxr=0&_shn={nonce}#all/{id}")
            }
            Self::Docs => "https://docs.google.com".to_owned(),
            Self::Drive => "https://drive.google.com/drive/".to_owned(),
            Self::Sheets => "https://docs.google.com/spreadsheets/".to_owned(),
            Self::Calendar => "https://calendar.google.com/calendar/".to_owned(),
            Self::Contacts => "https://contacts.google.com/".to_owned(),
            Self::Slides => "https://docs.google.com/presentation/".to_owned(),
            Self::Hangouts => "https://hangouts.google.com/".to_owned(),
            Self::Meet => "https://meet.google.com/".to_owned(),
            Self::FilterSettings => {
                "https://mail.google.com/mail/?shxr=0#settings/filters".to_owned()
            }
            Self::AliasSettings => {
                "https://mail.google.com/mail/?shxr=0#settings/accounts".to_owned()
            }
            Self::LabelSettings => {
                "https://mail.google.com/mail/?shxr=0#settings/labels".to_owned()
            }
        };
        as_account(&base, email)
    }
}

/// `url` with `authuser={email}` added to its query, ahead of any fragment.
fn as_account(url: &str, email: &str) -> String {
    let (url, fragment) = url
        .split_once('#')
        .map_or((url, None), |(url, fragment)| (url, Some(fragment)));
    let separator = if url.contains('?') { '&' } else { '?' };
    let mut out = format!(
        "{url}{separator}authuser={}",
        crate::format::urls::encode_component(email)
    );
    if let Some(fragment) = fragment {
        out.push('#');
        out.push_str(fragment);
    }
    out
}

/// What `open in Gmail` opens in `thread`: its focused message, else the conversation; nothing
/// for a draft, which Gmail can't open in All Mail.
pub fn target(thread: &ThreadView) -> Option<&str> {
    let id = thread.focused().unwrap_or(thread.id());
    let draft = thread
        .detail()
        .and_then(|detail| detail.messages.iter().find(|message| message.id == id))
        .is_some_and(|message| message.is_draft());
    (!draft).then_some(id)
}

/// Registers the rows. `open` is what an open thread opens in Gmail ([`target`]); without one
/// the first row opens Gmail.
pub fn register(commands: &mut Commands<Action>, open: Option<&str>) {
    let first = open.map_or(App::Gmail, |id| App::Message(id.to_owned()));
    let apps = [
        first,
        App::Docs,
        App::Drive,
        App::Sheets,
        App::Calendar,
        App::Contacts,
        App::Slides,
        App::Hangouts,
        App::Meet,
        App::FilterSettings,
        App::AliasSettings,
        App::LabelSettings,
    ];
    for app in apps {
        let (id, title) = app.row();
        commands.add(
            Command::new(id, title, Action::GoogleApp(app))
                .icon("data/g-suite")
                .scale(0.5)
                .operative_word("gmail"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "maya@brightmoor.co";

    #[test]
    fn each_app_opens_as_the_account() {
        let url = |app: App| app.url(ME, "nonce");
        assert_eq!(
            url(App::Gmail),
            "https://mail.google.com/mail/?shxr=0&authuser=maya%40brightmoor.co"
        );
        assert_eq!(
            url(App::Docs),
            "https://docs.google.com?authuser=maya%40brightmoor.co"
        );
        assert_eq!(
            url(App::Drive),
            "https://drive.google.com/drive/?authuser=maya%40brightmoor.co"
        );
        assert_eq!(
            url(App::Sheets),
            "https://docs.google.com/spreadsheets/?authuser=maya%40brightmoor.co"
        );
        assert_eq!(
            url(App::Calendar),
            "https://calendar.google.com/calendar/?authuser=maya%40brightmoor.co"
        );
        assert_eq!(
            url(App::Contacts),
            "https://contacts.google.com/?authuser=maya%40brightmoor.co"
        );
        assert_eq!(
            url(App::Slides),
            "https://docs.google.com/presentation/?authuser=maya%40brightmoor.co"
        );
        assert_eq!(
            url(App::Hangouts),
            "https://hangouts.google.com/?authuser=maya%40brightmoor.co"
        );
        assert_eq!(
            url(App::Meet),
            "https://meet.google.com/?authuser=maya%40brightmoor.co"
        );
    }

    #[test]
    fn gmail_settings_keep_their_fragment_after_the_account() {
        let url = |app: App| app.url(ME, "nonce");
        assert_eq!(
            url(App::FilterSettings),
            "https://mail.google.com/mail/?shxr=0&authuser=maya%40brightmoor.co#settings/filters"
        );
        assert_eq!(
            url(App::AliasSettings),
            "https://mail.google.com/mail/?shxr=0&authuser=maya%40brightmoor.co#settings/accounts"
        );
        assert_eq!(
            url(App::LabelSettings),
            "https://mail.google.com/mail/?shxr=0&authuser=maya%40brightmoor.co#settings/labels"
        );
    }

    #[test]
    fn a_message_opens_in_all_mail_with_a_fresh_nonce() {
        let url =
            App::Message("18c2f0a1b2c3d4e5".into()).url(ME, "6f1c2d3e-0000-4000-8000-000000000000");
        assert_eq!(
            url,
            "https://mail.google.com/mail/?shxr=0&_shn=6f1c2d3e-0000-4000-8000-000000000000&authuser=maya%40brightmoor.co#all/18c2f0a1b2c3d4e5"
        );
    }

    #[test]
    fn the_address_is_encoded_as_encode_uri_component_does() {
        assert_eq!(
            as_account("https://contacts.google.com/", "o'neil+b@x.co"),
            "https://contacts.google.com/?authuser=o'neil%2Bb%40x.co"
        );
    }
}
