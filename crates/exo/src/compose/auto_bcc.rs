//! Auto Bcc: the `autoBcc` addresses go into the Bcc of every new message, reply and forward,
//! come out again while every other recipient matches an `autoBccExclusions` entry, and go back
//! once one doesn't. An address the user put in or took out of Bcc themselves is theirs from then
//! on.

use std::collections::HashSet;

use exo_commands::{Command, Commands, SubPalette, Toast};
use exo_store::Address;

use super::{Action, Open, recipients};
use crate::{commands, pickers::Sender, settings};

/// A draft's Auto Bcc bookkeeping.
#[derive(Clone, Debug, Default)]
pub(crate) struct State {
    /// The addresses the user put in or took out of Bcc, lowercased.
    managed: HashSet<String>,
    /// Bcc as of the last update, to tell what the user changed since.
    seen: Option<Vec<Address>>,
    /// The Auto Bcc addresses as of the last update, lowercased: a draft with only those in Bcc
    /// is still unmodified.
    pub emails: Vec<String>,
}

/// The addresses of `entries`, lowercased.
pub(crate) fn emails(entries: &[String]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| recipients::parse(entry).email.to_lowercase())
        .filter(|email| !email.is_empty())
        .collect()
}

/// An entry's address, else the entry (a domain), lowercased.
pub(crate) fn normalize(entry: &str) -> String {
    let email = recipients::parse(entry).email;
    if email.is_empty() {
        entry.to_lowercase()
    } else {
        email.to_lowercase()
    }
}

/// A new draft's Bcc gets the Auto Bcc addresses after its own, each address once.
pub(crate) fn add(bcc: &mut Vec<Address>, auto_bcc: &[String]) {
    for entry in auto_bcc {
        let address = recipients::parse(entry.trim());
        if !bcc.iter().any(|kept| kept.email == address.email) {
            bcc.push(address);
        }
    }
}

/// `Exclude from Auto Bcc` on one conversation with a sender who isn't you
/// (`ADD_TO_AUTO_BCC_EXCLUSIONS`).
pub(crate) fn register_exclude(commands: &mut Commands<commands::Action>, sender: &Sender) {
    let action = commands::Action::Compose(Action::OpenAutoBccExclusion(sender.email.clone()));
    commands.add(
        Command::new(
            "ADD_TO_AUTO_BCC_EXCLUSIONS",
            "Exclude from Auto Bcc",
            action,
        )
        .operative_word("exclude")
        .scale(0.84)
        .default_score(0.94)
        .icon("data/subtract")
        .regions(exo_commands::CONVERSATION),
    );
}

/// The `Exclude from Auto Bcc…` picker: the sender's address, or their domain.
fn exclusion_palette(email: &str) -> SubPalette<commands::Action> {
    let domain = email.split_once('@').map_or("", |(_, domain)| domain);
    let row = |id: &'static str, title: String, entry: &str| {
        Command::new(
            id,
            title,
            commands::Action::Compose(Action::ExcludeFromAutoBcc(entry.to_owned())),
        )
    };
    let rows = vec![
        row("email", format!("Exclude sender email ({email})"), email),
        row(
            "domain",
            format!("Exclude sender domain ({domain})"),
            domain,
        ),
    ];
    SubPalette::new(
        "ADD_TO_AUTO_BCC_EXCLUSIONS",
        "Exclude from Auto Bcc…",
        "data/subtract",
        rows,
    )
}

impl crate::Mail {
    /// Opens `Exclude from Auto Bcc…` for `email`.
    pub(super) fn open_auto_bcc_exclusion(&mut self, email: &str) {
        self.commands.open_sub_palette(exclusion_palette(email));
    }

    /// `entry` joins the exclusions unless an entry for it is there, and a toast says which, with
    /// a way to the Auto Bcc dialog.
    pub(super) fn exclude_from_auto_bcc(&mut self, entry: String) {
        let wanted = normalize(&entry);
        let known = self
            .account
            .settings()
            .auto_bcc_exclusions
            .iter()
            .any(|kept| normalize(kept) == wanted);
        let text = if known {
            format!("Already in Auto Bcc Exclusions: {entry}")
        } else {
            self.account
                .update_settings(|settings| settings.auto_bcc_exclusions.push(entry.clone()));
            format!("Added to Auto Bcc Exclusions: {entry}")
        };
        self.commands.toast(Toast::new(text).action(
            "Auto BCC Settings",
            None,
            commands::Action::Settings(settings::Action::Open(settings::Dialog::AutoBcc)),
        ));
    }

    /// A draft just made, a new message, reply or forward, takes the Auto Bcc addresses (a
    /// snippet doesn't).
    pub(super) fn with_auto_bcc(&self, open: &mut Open) {
        if !open.is_snippet() {
            add(&mut open.record.bcc, &self.account.settings().auto_bcc);
        }
    }

    /// Keeps every draft open in compose in line with the Auto Bcc settings, with a toast for
    /// what changed in the one with the keyboard.
    pub(super) fn auto_bcc_tick(&mut self) {
        let settings = self.account.settings();
        let (auto_bcc, exclusions) = (
            settings.auto_bcc.clone(),
            settings.auto_bcc_exclusions.clone(),
        );
        let focus = self.compose.focus.clone();
        let mut toast = None;
        for open in &mut self.compose.drafts {
            open.auto_bcc.emails = emails(&auto_bcc);
            if let Some(text) = update(open, &auto_bcc, &exclusions) {
                open.touch(self.ctx.input(|input| input.time));
                if focus.as_deref() == Some(open.id.as_str()) {
                    toast = Some(text);
                }
            }
        }
        if let Some(text) = toast {
            self.commands.toast(exo_commands::Toast::new(text));
        }
    }
}

/// What Bcc gains and loses for the recipients to match the settings.
/// While every recipient but the Auto Bcc addresses matches an exclusion (its domain, or the
/// address), the Auto Bcc addresses go, but for those in `managed`; otherwise each that isn't a
/// recipient yet comes, unless it's in `managed`.
fn difference(
    open: &Open,
    auto_bcc: &[String],
    exclusions: &[String],
    managed: &HashSet<String>,
) -> (Vec<Address>, Vec<Address>) {
    let record = &open.record;
    let auto = emails(auto_bcc);
    let everyone: Vec<String> = open
        .recipients()
        .map(|address| address.email.clone())
        .filter(|email| !email.is_empty())
        .collect();
    let exclusions: Vec<String> = exclusions.iter().map(|entry| normalize(entry)).collect();
    let others: Vec<&String> = everyone
        .iter()
        .filter(|email| !auto.iter().any(|auto| auto.eq_ignore_ascii_case(email)))
        .collect();
    let excluded = |email: &str| {
        let email = email.to_lowercase();
        exclusions
            .iter()
            .any(|exclusion| email.ends_with(&format!("@{exclusion}")) || *exclusion == email)
    };
    let mut wanted: Vec<Address> = record.bcc.clone();
    if !others.is_empty() && others.iter().all(|email| excluded(email)) {
        wanted.retain(|address| {
            !auto
                .iter()
                .any(|auto| auto.eq_ignore_ascii_case(&address.email))
                || managed.contains(&address.email.to_lowercase())
        });
    } else {
        for entry in auto_bcc {
            let address = recipients::parse(entry);
            let email = address.email.to_lowercase();
            if !everyone.contains(&email) && !managed.contains(&email) {
                wanted.push(address);
            }
        }
    }
    let has = |list: &[Address], address: &Address| {
        list.iter()
            .any(|other| other.email.eq_ignore_ascii_case(&address.email))
    };
    let insertions = wanted
        .iter()
        .filter(|address| !has(&record.bcc, address))
        .cloned()
        .collect();
    let removals = record
        .bcc
        .iter()
        .filter(|address| !has(&wanted, address))
        .cloned()
        .collect();
    (insertions, removals)
}

/// Run whenever the draft shows: takes what the user changed in Bcc since the last run as theirs,
/// brings Bcc in line with the settings, and returns the toast for what changed. A snippet has no
/// Auto Bcc.
pub(crate) fn update(
    open: &mut Open,
    auto_bcc: &[String],
    exclusions: &[String],
) -> Option<&'static str> {
    if let Some(seen) = open.auto_bcc.seen.take() {
        let lower = |list: &[Address]| {
            list.iter()
                .map(|address| address.email.to_lowercase())
                .collect::<Vec<_>>()
        };
        let (before, after) = (lower(&seen), lower(&open.record.bcc));
        let changed = after
            .iter()
            .filter(|email| !before.contains(email))
            .chain(before.iter().filter(|email| !after.contains(email)));
        open.auto_bcc.managed.extend(changed.cloned());
    }
    let toast = changes(open, auto_bcc, exclusions);
    open.auto_bcc.seen = Some(open.record.bcc.clone());
    toast
}

/// Takes `open`'s Bcc as it is now as Auto Bcc's own doing, not the user's (Instant Intro's).
pub(crate) fn settle(open: &mut Open) {
    open.auto_bcc.seen = Some(open.record.bcc.clone());
}

fn changes(open: &mut Open, auto_bcc: &[String], exclusions: &[String]) -> Option<&'static str> {
    if open.is_snippet() || auto_bcc.is_empty() {
        return None;
    }
    let (insertions, removals) = difference(open, auto_bcc, exclusions, &open.auto_bcc.managed);
    if insertions.is_empty() && removals.is_empty() {
        return None;
    }
    let bcc = &mut open.record.bcc;
    bcc.retain(|address| {
        !removals
            .iter()
            .any(|removed| removed.email.eq_ignore_ascii_case(&address.email))
    });
    bcc.extend(insertions.iter().cloned());
    Some(match (insertions.len(), removals.len()) {
        (1, 0) => "Auto Bcc address added",
        (_, 0) => "Auto Bcc addresses added",
        (0, 1) => "Auto Bcc address removed",
        (0, _) => "Auto Bcc addresses removed",
        _ => "Auto Bcc addresses were automatically updated",
    })
}

#[cfg(test)]
mod tests {
    use exo_store::{DraftAction, DraftRecord};

    use super::*;

    fn address(email: &str) -> Address {
        Address {
            name: None,
            email: email.into(),
        }
    }

    fn open(to: &[&str], bcc: &[&str]) -> Open {
        let record = DraftRecord {
            action: DraftAction::Compose,
            from: None,
            to: to.iter().map(|email| address(email)).collect(),
            cc: Vec::new(),
            bcc: bcc.iter().map(|email| address(email)).collect(),
            subject: String::new(),
            body: String::new(),
            quoted_content: None,
            quoted_inlined: false,
            in_reply_to: None,
            in_reply_to_rfc822: None,
            references: Vec::new(),
            rfc822_id: "x@y".into(),
            fingerprint: String::new(),
            attachments: Vec::new(),
            send_later: None,
            reminder: None,
        };
        Open::new("d".into(), None, record)
    }

    fn bcc(open: &Open) -> Vec<&str> {
        open.record
            .bcc
            .iter()
            .map(|address| address.email.as_str())
            .collect()
    }

    const CRM: &[&str] = &["123@bcc.hubspot.com"];

    fn settings(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|entry| (*entry).to_owned()).collect()
    }

    #[test]
    fn new_drafts_get_each_address_once() {
        let mut list = vec![address("me@x.com")];
        add(
            &mut list,
            &settings(&["CRM <123@bcc.hubspot.com>", " me@x.com "]),
        );
        assert_eq!(
            list,
            [
                address("me@x.com"),
                Address {
                    name: Some("CRM".into()),
                    email: "123@bcc.hubspot.com".into()
                }
            ]
        );
    }

    #[test]
    fn excluded_recipients_take_it_out_and_others_put_it_back() {
        let exclusions = settings(&["team.com", "lawyer@firm.com"]);
        let mut draft = open(&["ann@team.com"], CRM);
        assert_eq!(
            update(&mut draft, &settings(CRM), &exclusions),
            Some("Auto Bcc address removed")
        );
        assert!(bcc(&draft).is_empty());
        draft.record.to.push(address("bob@client.com"));
        assert_eq!(
            update(&mut draft, &settings(CRM), &exclusions),
            Some("Auto Bcc address added")
        );
        assert_eq!(bcc(&draft), CRM);
        assert_eq!(
            update(&mut draft, &settings(CRM), &exclusions),
            None,
            "nothing more to do"
        );
        draft.record.to = vec![address("lawyer@firm.com")];
        assert_eq!(
            update(&mut draft, &settings(CRM), &exclusions),
            Some("Auto Bcc address removed")
        );
    }

    #[test]
    fn a_draft_with_no_one_else_keeps_it() {
        let mut draft = open(&[], &[]);
        assert_eq!(
            update(&mut draft, &settings(CRM), &settings(&["team.com"])),
            Some("Auto Bcc address added")
        );
        assert_eq!(bcc(&draft), CRM);
    }

    #[test]
    fn what_the_user_moves_is_theirs() {
        let mut draft = open(&["bob@client.com"], CRM);
        assert_eq!(update(&mut draft, &settings(CRM), &[]), None);
        draft.record.bcc.clear();
        assert_eq!(
            update(&mut draft, &settings(CRM), &[]),
            None,
            "taken out by hand, it stays out"
        );
        let mut draft = open(&["ann@team.com"], &[]);
        assert_eq!(update(&mut draft, &[], &[]), None);
        draft.record.bcc.push(address("123@bcc.hubspot.com"));
        assert_eq!(
            update(&mut draft, &settings(CRM), &settings(&["team.com"])),
            None,
            "put in by hand, it stays in"
        );
    }

    #[test]
    fn what_auto_bcc_settles_isnt_the_users() {
        let mut draft = open(&["bob@client.com"], &[]);
        assert_eq!(update(&mut draft, &[], &[]), None);
        draft.record.bcc.push(address("123@bcc.hubspot.com"));
        settle(&mut draft);
        draft.record.to = vec![address("ann@team.com")];
        assert_eq!(
            update(&mut draft, &settings(CRM), &settings(&["team.com"])),
            Some("Auto Bcc address removed")
        );
    }

    #[test]
    fn several_changes_say_so() {
        let mut draft = open(&["bob@client.com"], &[]);
        assert_eq!(
            update(&mut draft, &settings(&["a@crm.com", "b@crm.com"]), &[]),
            Some("Auto Bcc addresses added")
        );
        draft.record.bcc.push(address("a@crm.com"));
        assert_eq!(
            update(&mut draft, &[], &[]),
            None,
            "no Auto Bcc, no changes"
        );
    }
}
