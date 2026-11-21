//! Remind me from compose: ⌘⇧H, the palette's `Remind Me` and the action pane's Remind button
//! open the picker in Draft mode, whose time is the draft's (`reminder`); the store gives the
//! thread that reminder at Send. After a send, once, the follow-up hint offers the same for the
//! message just sent. Automatic Reminders put the default reminder on a draft with an external
//! recipient, and the Remind label flourishes as one comes.

use std::hash::{DefaultHasher, Hash, Hasher};

use egui::{Color32, Painter, Pos2};
use exo_commands::{Hint, Toast};
use exo_store::{Address, DraftRecord, DraftReminder, TriggerCondition};
use jiff::SignedDuration;

use super::{Action, Alert, AlertThen, recipients, save_changes};
use crate::{
    Mail, commands,
    remind::{self, Target},
};

/// The follow-up hint waits for this many sends from this device (`localSendsCount > 5`).
const SENDS_BEFORE_HINT: u32 = 5;
/// While the hint is open its send is held this long past now, every
/// [`POSTPONE_EVERY`] seconds, at most [`POSTPONES`] times (a minute).
const POSTPONE_BY: SignedDuration = SignedDuration::from_secs(10);
pub(super) const POSTPONE_EVERY: f64 = 5.0;
const POSTPONES: u32 = 12;

/// The hint's text around its `H` keycap.
pub(super) const HINT_BEFORE: &str = "Hit";
pub(super) const HINT_KEY: &str = "H";
pub(super) const HINT_AFTER: &str = "on any email you've sent, and choose a time. If you don't hear back by then, we'll remind you to follow up.";

/// The Remind label's flourish: a 2 s linear sweep.
const FLOURISH_SECONDS: f64 = 2.0;
/// The colour that sweeps across: `#aeb1dd` from 45% to 80% of the gradient, the label's own
/// ink to 30% and at 100%.
const FLOURISH_BAND: Color32 = Color32::from_rgb(0xae, 0xb1, 0xdd);

/// What Automatic Reminders know of an open draft, which they look at as it opens and each time
/// its participants change.
#[derive(Debug, Default)]
pub(crate) struct Automatic {
    /// The participants last looked at.
    seen: Option<u64>,
    /// The picker's `Remove Reminder` took its reminder off, so none
    /// comes back while it's open.
    removed: bool,
    /// When they put its reminder on: the Remind label flourishes from then.
    pub flourish: Option<f64>,
}

/// To, Cc, Bcc and From, as one number that changes when any does.
fn participants(record: &DraftRecord) -> u64 {
    let mut hasher = DefaultHasher::new();
    for (list, addresses) in [&record.to, &record.cc, &record.bcc]
        .into_iter()
        .enumerate()
    {
        list.hash(&mut hasher);
        for address in addresses {
            (&address.name, &address.email).hash(&mut hasher);
        }
    }
    record
        .from
        .as_ref()
        .map(|from| &from.email)
        .hash(&mut hasher);
    hasher.finish()
}

/// Whether the draft has an external recipient: some valid address in To, Cc or Bcc whose domain
/// isn't one of `team`'s, and that isn't one of Auto Bcc's (`auto_bcc`, lowercased).
fn has_external(record: &DraftRecord, team: &[String], auto_bcc: &[String]) -> bool {
    let external = |address: &Address| {
        recipients::is_valid(&address.email)
            && !auto_bcc.contains(&address.email.to_lowercase())
            && address.email.rsplit_once('@').is_none_or(|(_, domain)| {
                !team.iter().any(|team| team.eq_ignore_ascii_case(domain))
            })
    };
    record
        .to
        .iter()
        .chain(&record.cc)
        .chain(&record.bcc)
        .any(external)
}

/// Whether the draft is a note to self: only to `Me` at the default alias, subject `Note to self`.
fn is_note_to_self(record: &DraftRecord, default_alias: &str) -> bool {
    matches!(record.to.as_slice(), [only] if only.email == default_alias && only.name.as_deref() == Some("Me"))
        && record.cc.is_empty()
        && record.bcc.is_empty()
        && record.subject == "Note to self"
}

/// The team's domains: the Team split's domains, else the first alias's domain, unless that's a
/// webmail one.
pub(crate) fn team_domains(
    settings: &exo_store::Settings,
    aliases: &[exo_store::Alias],
    email: &str,
) -> Vec<String> {
    let team = settings
        .splits()
        .find(|split| split.kind.as_deref() == Some("team"));
    let defined: Vec<String> = team
        .map(|split| crate::settings::split_template::query_items(Some("team"), split.query()))
        .unwrap_or_default()
        .into_iter()
        .map(|domain| domain.to_lowercase())
        .collect();
    if !defined.is_empty() {
        return defined;
    }
    let first = aliases.first().map_or(email, |alias| alias.email.as_str());
    crate::splits::team_domain(first).into_iter().collect()
}

/// The flourish at `progress` (0 to 1) over a label `width` wide: its colour at `x` from the
/// label's left. The gradient, three labels wide and repeating, slides from `150% 0` to `0 0`,
/// so the band crosses the label left to right and the last frame, which stays, is the label's
/// ink with a hint of the band at its end.
pub(super) fn flourish_at(x: f32, width: f32, progress: f32, ink: Color32) -> Color32 {
    let image = 3.0 * width;
    let offset = (width - image) * 1.5 * (1.0 - progress);
    let at = ((x - offset).rem_euclid(image)) / image;
    let mix = |from, to, from_at: f32, to_at: f32| {
        exo_ui::motion::mix(from, to, (at - from_at) / (to_at - from_at))
    };
    match at {
        at if at < 0.30 => ink,
        at if at < 0.45 => mix(ink, FLOURISH_BAND, 0.30, 0.45),
        at if at < 0.80 => FLOURISH_BAND,
        _ => mix(FLOURISH_BAND, ink, 0.80, 1.0),
    }
}

/// Paints `text` in `style` with its line box at `line_top_left`, as `TextStyle::paint_line`
/// places it, its glyphs coloured by [`flourish_at`] (`-webkit-background-clip: text`).
pub(super) fn paint_flourish(
    painter: &Painter,
    line_top_left: Pos2,
    text: &str,
    style: exo_ui::TextStyle,
    progress: f32,
    ink: Color32,
) {
    // Laid out from the origin's sub-pixel phase and painted at the whole pixel below it.
    let pixels_per_point = painter.pixels_per_point();
    let x = line_top_left.x * pixels_per_point;
    let phase = (x - x.floor()) / pixels_per_point;
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), style.format(ink));
    job.wrap = egui::text::TextWrapping::no_max_width();
    if let Some(first) = job.sections.first_mut() {
        first.leading_space = phase;
    }
    let mut galley = exo_ui::layout(painter.ctx(), job);
    let width = galley.size().x - phase;
    for placed in &mut std::sync::Arc::make_mut(&mut galley).rows {
        let left = placed.pos.x - phase;
        for vertex in &mut std::sync::Arc::make_mut(&mut placed.row)
            .visuals
            .mesh
            .vertices
        {
            vertex.color = flourish_at(left + vertex.pos.x, width, progress, ink);
        }
    }
    exo_ui::paint_text(
        painter,
        Pos2::new(x.floor() / pixels_per_point, line_top_left.y),
        galley,
        &style,
    );
}

/// How far the Remind label's flourish is at `now`, while it shows: from the time Automatic
/// Reminders put the draft's reminder on, while it's still theirs.
pub(super) fn flourish_progress(open: &super::draft::Open, now: f64) -> Option<f32> {
    let start = open.automatic.flourish.filter(|_| {
        open.record
            .reminder
            .is_some_and(|reminder| reminder.automatic)
    })?;
    Some(((now - start) / FLOURISH_SECONDS).clamp(0.0, 1.0) as f32)
}

/// The follow-up hint holding its send: how often it has, and when last.
pub(super) struct FollowUp {
    pub draft: String,
    postponed: u32,
    at: f64,
}

impl Mail {
    /// Opens the picker for the focused draft (or the new-message pane's), in
    /// Draft mode, with `Remove Reminder` once it has one.
    pub(super) fn open_draft_reminder(&mut self) {
        let Some(id) = self
            .compose
            .focus
            .clone()
            .or_else(|| self.compose.pane().map(str::to_owned))
        else {
            return;
        };
        let Some(open) = self.compose.get_mut(&id).filter(|open| !open.is_snippet()) else {
            return;
        };
        let footer = remind::draft_footer(&id, open.record.reminder.is_some());
        self.open_remind_me(Target::Draft(id), footer);
    }

    /// Automatic Reminders, for every open draft whose participants changed since last
    /// looked at, or that just opened. With `All messages with an external recipient`
    /// (`EXTERNAL`): a draft with an external recipient that isn't a note to self and has no
    /// reminder gets the default one (`in {days} days` or weekdays, at the time, `regardless` as
    /// the dropdown says); one with none any more loses the reminder they gave it.
    pub(super) fn automatic_reminders(&mut self, now: f64) {
        let settings = self.account.settings();
        let reminders = settings.reminders.clone();
        let team = team_domains(settings, &self.snapshot.aliases, self.account.email());
        let auto_bcc = super::auto_bcc::emails(&settings.auto_bcc);
        let me = self
            .snapshot
            .aliases
            .iter()
            .find(|alias| alias.default)
            .map_or_else(
                || self.account.email().to_owned(),
                |alias| alias.email.clone(),
            );
        let aliases = self.snapshot.aliases.clone();
        let local = self.now.clone();
        for open in self
            .compose
            .drafts
            .iter_mut()
            .filter(|open| !open.is_snippet())
        {
            let seen = participants(&open.record);
            if open.automatic.seen.replace(seen) == Some(seen)
                || reminders.default_reminders_trigger != "EXTERNAL"
                || open.automatic.removed
            {
                continue;
            }
            let external =
                has_external(&open.record, &team, &auto_bcc) && !is_note_to_self(&open.record, &me);
            match open.record.reminder {
                Some(reminder) if reminder.automatic && !external => {
                    open.record.reminder = None;
                    open.automatic.flourish = None;
                }
                None if external => {
                    let at = remind::in_days(
                        &local,
                        reminders.days,
                        reminders.weekdays_only(),
                        reminders.hour,
                        reminders.minute,
                    );
                    open.record.reminder = Some(DraftReminder {
                        at,
                        keep_on_reply: reminders.trigger_condition == TriggerCondition::Regardless,
                        automatic: true,
                    });
                    open.automatic.flourish = Some(now);
                }
                _ => continue,
            }
            save_changes(open, &self.account, &aliases, &local);
        }
    }

    /// The picker's `Remove Reminder` took draft `id`'s reminder off: Automatic Reminders leave
    /// it while it's open.
    pub(crate) fn reminder_removed(&mut self, id: &str) {
        if let Some(open) = self.compose.get_mut(id) {
            open.automatic.removed = true;
        }
    }

    /// Draft `id` takes `reminder` (`None`: `Remove Reminder`), saved at once.
    pub(crate) fn set_draft_reminder(&mut self, id: &str, reminder: Option<DraftReminder>) {
        let aliases = self.snapshot.aliases.clone();
        let Some(open) = self.compose.get_mut(id) else {
            return;
        };
        open.record.reminder = reminder;
        save_changes(open, &self.account, &aliases, &self.now);
    }

    /// The follow-up hint's picker picked `reminder` for the message sent as draft `id`: its
    /// thread gets it without being marked done, and the `Reminder set: …` toast's Undo takes it
    /// off again.
    pub(crate) fn remind_sent(&mut self, id: &str, reminder: DraftReminder) {
        let thread = self.sent_thread(id);
        self.account.remind_sent(id, thread, Some(reminder));
        let text = remind::toast(reminder.at, reminder.keep_on_reply, &self.now.clone());
        self.commands
            .toast(
                Toast::new(text).undo(commands::Action::Compose(Action::UnremindSent(
                    id.to_owned(),
                ))),
            );
        self.commands.hint(
            Hint::new()
                .text("Nice! Next time hit")
                .keys(["⌘", "shift", "H"])
                .text("before sending, or")
                .keys(["H"])
                .text("on any sent email."),
        );
    }

    /// The `Reminder set` toast's Undo, for a message just sent, which says nothing.
    pub(super) fn unremind_sent(&mut self, id: &str) {
        let thread = self.sent_thread(id);
        self.account.remind_sent(id, thread, None);
    }

    /// The thread draft `id` was sent into, or will be: its held send's, the store's, or Gmail's
    /// once sent.
    fn sent_thread(&self, id: &str) -> Option<String> {
        self.compose
            .sends
            .iter()
            .find(|sending| sending.open.id == id)
            .and_then(|sending| sending.open.thread.clone())
            .or_else(|| {
                self.snapshot
                    .drafts
                    .iter()
                    .find(|draft| draft.id == id)
                    .and_then(|draft| draft.thread.as_ref())
                    .map(|thread| thread.as_str().to_owned())
            })
            .or_else(|| {
                self.compose
                    .sent_threads
                    .iter()
                    .find(|(draft, _)| draft == id)
                    .map(|(_, thread)| thread.clone())
            })
    }

    /// After a send of draft `id`: counts it, and once, after more than five from this device,
    /// for a message sent without a reminder, opens the follow-up hint.
    pub(super) fn follow_up_hint(&mut self, id: &str, reminded: bool, now: f64) {
        self.count_send(now);
        let sends = self.account.device_settings().local_sends_count;
        self.account
            .update_device_settings(|device| device.local_sends_count = sends.saturating_add(1));
        if self.account.settings().has_shown_follow_up_reminder_hint
            || sends <= SENDS_BEFORE_HINT
            || reminded
        {
            return;
        }
        self.account
            .update_settings(|settings| settings.has_shown_follow_up_reminder_hint = true);
        let text = format!("{HINT_BEFORE} {HINT_KEY} {HINT_AFTER}");
        let mut alert = Alert::new(
            Some("Follow up on time, every time"),
            text,
            vec!["Set reminder", "Maybe later"],
            id,
            AlertThen::FollowUp,
        );
        alert.key = true;
        self.compose.alert = Some(alert);
        self.compose.follow_up = Some(FollowUp {
            draft: id.to_owned(),
            postponed: 0,
            at: now,
        });
    }

    /// While the hint is open, its send waits: ten seconds past now every five, for a minute at
    /// most. A scheduled send keeps its time.
    pub(super) fn postpone_for_follow_up(&mut self, now: f64) {
        let open = self
            .compose
            .alert
            .as_ref()
            .is_some_and(|alert| matches!(alert.then, AlertThen::FollowUp));
        let Some(follow) = self.compose.follow_up.as_mut().filter(|_| open) else {
            self.compose.follow_up = None;
            return;
        };
        if follow.postponed >= POSTPONES {
            return;
        }
        if now - follow.at >= POSTPONE_EVERY {
            follow.at = now;
            follow.postponed += 1;
            let wanted = jiff::Timestamp::now()
                .checked_add(POSTPONE_BY)
                .unwrap_or(jiff::Timestamp::MAX);
            if let Some(sending) = self.compose.sends.iter_mut().find(|sending| {
                sending.open.id == follow.draft && !sending.scheduled && wanted > sending.until
            }) {
                sending.until = wanted;
                self.account.send_draft(&sending.open.id, wanted);
            }
        }
        self.ctx.request_repaint_after_secs(POSTPONE_EVERY as f32);
    }

    /// `Set reminder` (or `H`): the toast goes, and the picker opens for the message just sent,
    /// with a reminder's rows.
    pub(super) fn follow_up_set_reminder(&mut self, id: String) {
        self.compose.follow_up = None;
        self.commands.hide_toast();
        self.open_remind_me(Target::Sent(id), None);
    }
}

#[cfg(test)]
mod tests {
    use exo_store::DraftAction;

    use super::*;

    fn record(to: &[(&str, &str)], subject: &str) -> DraftRecord {
        DraftRecord {
            action: DraftAction::Compose,
            from: None,
            to: to
                .iter()
                .map(|(name, email)| Address {
                    name: (!name.is_empty()).then(|| (*name).to_owned()),
                    email: (*email).into(),
                })
                .collect(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: subject.into(),
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
        }
    }

    #[test]
    fn external_is_outside_the_team_domains() {
        let team = ["brightmoor.co".to_owned()];
        assert!(!has_external(
            &record(
                &[("", "tom@brightmoor.co"), ("", "PRIYA@Brightmoor.co")],
                ""
            ),
            &team,
            &[]
        ));
        assert!(has_external(
            &record(
                &[("", "tom@brightmoor.co"), ("", "grace@juniperpartners.vc")],
                ""
            ),
            &team,
            &[]
        ));
        assert!(
            has_external(&record(&[("", "tom@eng.brightmoor.co")], ""), &team, &[]),
            "subdomains don't match"
        );
        assert!(
            !has_external(&record(&[("", "not an address")], ""), &team, &[]),
            "only valid addresses count"
        );
        let mut bcc = record(&[("", "tom@brightmoor.co")], "");
        bcc.bcc.push(Address {
            name: None,
            email: "grace@juniperpartners.vc".into(),
        });
        assert!(has_external(&bcc, &team, &[]), "Bcc counts");
        assert!(
            has_external(&record(&[("", "me@gmail.com")], ""), &[], &[]),
            "with no team domains, everyone is"
        );
        let mut crm = record(&[("", "tom@brightmoor.co")], "");
        crm.bcc.push(Address {
            name: None,
            email: "123@BCC.hubspot.com".into(),
        });
        assert!(
            !has_external(&crm, &team, &["123@bcc.hubspot.com".to_owned()]),
            "Auto Bcc's addresses aren't external"
        );
    }

    #[test]
    fn a_note_to_self_is_only_that() {
        assert!(is_note_to_self(
            &record(&[("Me", "maya@gmail.com")], "Note to self"),
            "maya@gmail.com"
        ));
        assert!(!is_note_to_self(
            &record(&[("Me", "maya@gmail.com")], "Notes"),
            "maya@gmail.com"
        ));
        assert!(!is_note_to_self(
            &record(&[("Maya", "maya@gmail.com")], "Note to self"),
            "maya@gmail.com"
        ));
    }

    #[test]
    fn the_team_is_the_team_splits_or_the_first_aliass_domain() {
        let settings = exo_store::Settings::default();
        assert_eq!(
            team_domains(&settings, &[], "maya@brightmoor.co"),
            ["brightmoor.co"]
        );
        assert!(
            team_domains(&settings, &[], "maya@gmail.com").is_empty(),
            "webmail has none"
        );
    }

    #[test]
    fn the_flourish_sweeps_the_band_across_and_ends_as_it_began() {
        let ink = Color32::from_black_alpha(153);
        let band_at = |progress: f32| {
            (0..100)
                .map(|x| x as f32)
                .find(|x| flourish_at(*x, 100.0, progress, ink) == FLOURISH_BAND)
        };
        assert_eq!(band_at(0.0), None, "no band at the start");
        let (early, late) = (band_at(0.4), band_at(0.6));
        assert!(
            early.is_some() && late.is_some() && early < late,
            "left to right: {early:?} {late:?}"
        );
        for x in [0.0, 50.0, 99.0] {
            assert_eq!(
                flourish_at(x, 100.0, 0.0, ink),
                flourish_at(x, 100.0, 1.0, ink),
                "{x}"
            );
        }
        assert_eq!(
            flourish_at(0.0, 100.0, 1.0, ink),
            ink,
            "the last frame is the ink, but at its end"
        );
        assert_ne!(flourish_at(99.0, 100.0, 1.0, ink), ink);
    }
}
