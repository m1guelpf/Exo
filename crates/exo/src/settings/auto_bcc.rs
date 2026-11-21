//! The Auto Bcc dialog: the addresses every draft gets in Bcc (`autoBcc`), and the addresses and
//! domains whose mail goes without them (`autoBccExclusions`), each list over its input. Enter
//! adds what's typed; each entry's trash can removes it. What compose does with them is
//! `crate::compose::auto_bcc`.
//!
//! Metrics at a 1280×800 window. In Carbon the card keeps Snow's colours but for the row's hover
//! fill.

use egui::{Color32, Context, CursorIcon, Rect, Sense, Ui, pos2, vec2};
use exo_store::Settings as AccountSettings;
use exo_ui::{Dialog, DialogInput, DialogResponse, Family, Font, TextStyle};

use crate::compose::recipients;

/// The subheaders: 12px/20px, `padding-top: 24px`, 3px down; the entries and inputs bold.
const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
const BOLD: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 20.0);
const NUDGE: f32 = 3.0;
const SUBHEADER_PADDING: f32 = 24.0;
/// The subheader's `margin-bottom: 20px`; the exclusions' subheader keeps a dialog subheader's
/// 16px, which the list's 20px outgrows when it's empty.
const LIST_GAP: f32 = 20.0;
const EXCLUSIONS_LIST_GAP: f32 = 16.0;
/// An entry: 20px lines 8px apart.
const ITEM: f32 = 20.0;
const ITEM_GAP: f32 = 8.0;
/// The input's line and its 1px rule (`margin-bottom: -1px`).
const INPUT: f32 = 20.0;
/// The dialog's `padding-bottom: 24px`: 4px more than the shell's.
const EXTRA_PADDING: f32 = 4.0;
/// The subheaders' `rgba(0, 0, 0, .85)`, and the entries'.
const INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 217);
/// A link: `#54acdc` in both themes.
const LINK: Color32 = Color32::from_rgb(0x54, 0xac, 0xdc);
/// The hovered or focused row (`::before`, from 2px above it): `#f2f4fa`, `#eaeaea` in Carbon.
const ROW_FILL: Color32 = Color32::from_rgb(0xf2, 0xf4, 0xfa);
const ROW_FILL_CARBON: Color32 = Color32::from_rgb(0xea, 0xea, 0xea);
/// The trash can: `rgba(0, 0, 0, .3)`, `.4` hovered or focused, over 64ms.
const TRASH: &str = "legacy/DEPRECATED_trash";
const TRASH_INK: f32 = 0.3;
const TRASH_HOVER_INK: f32 = 0.4;
/// The subheaders' copy and links.
const LINKS: [(&str, &str); 2] = [
    (
        "HubSpot",
        "https://knowledge.hubspot.com/settings/log-email-in-your-crm-with-the-bcc-or-forwarding-address",
    ),
    (
        "Salesforce",
        "https://support.outreach.io/hc/en-us/articles/205261638-How-To-BCC-Outgoing-Emails-to-Salesforce",
    ),
];
const EXCLUSIONS_SUBHEADER: [&str; 2] = [
    "Would you like to exclude emails to your team, lawyers, or investors?",
    "If all recipients match an exclusion below, Auto Bcc will be removed.",
];

/// One of the dialog's two lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum List {
    /// `autoBcc`.
    Addresses,
    /// `autoBccExclusions`.
    Exclusions,
}

impl List {
    fn placeholder(self) -> &'static str {
        match self {
            Self::Addresses => "Enter an Auto Bcc address, e.g. 123@bcc.hubspot.com",
            Self::Exclusions => {
                "Enter an email address or domain, e.g. joe@example.com or example.com."
            }
        }
    }
}

/// What has the keyboard, in tab order (every control is `tabIndex: 0`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    /// `HubSpot` or `Salesforce`.
    Link(usize),
    /// An entry's trash can.
    Delete(List, usize),
    /// A list's input.
    Input(List),
}

/// Something done in the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Enter in a list's input: what's typed is added.
    Submit(List),
    /// A trash can, clicked or Enter/Space: that entry goes.
    Remove(List, usize),
    /// Tab (1) or ⇧Tab (-1).
    Tab(isize),
    /// Enter or Space on a link or a trash can.
    Activate,
    /// A click on a control.
    Focus(Focus),
}

/// The dialog's inputs as typed, which is marked wrong, and what has the keyboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    inputs: [String; 2],
    invalid: [bool; 2],
    focus: Focus,
    /// The focused input still has to get egui's keyboard.
    focus_pending: bool,
    /// A link to open, chosen from the keyboard.
    open: Option<&'static str>,
}

impl Default for State {
    /// The addresses' input has the keyboard.
    fn default() -> Self {
        Self {
            inputs: Default::default(),
            invalid: [false; 2],
            focus: Focus::Input(List::Addresses),
            focus_pending: true,
            open: None,
        }
    }
}

/// What a submission changed: the lists to save, and the toast for what wasn't added.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Saved {
    pub auto_bcc: Option<Vec<String>>,
    pub exclusions: Option<Vec<String>>,
    pub toast: Option<String>,
}

impl State {
    /// The controls in tab order for `settings`'s lists.
    fn order(settings: &AccountSettings) -> Vec<Focus> {
        let mut order = vec![Focus::Link(0), Focus::Link(1)];
        order.extend((0..settings.auto_bcc.len()).map(|at| Focus::Delete(List::Addresses, at)));
        order.push(Focus::Input(List::Addresses));
        order.extend(
            (0..settings.auto_bcc_exclusions.len()).map(|at| Focus::Delete(List::Exclusions, at)),
        );
        order.push(Focus::Input(List::Exclusions));
        order
    }

    /// Whether a list's input has the keyboard (Space types there).
    pub fn typing(&self) -> bool {
        matches!(self.focus, Focus::Input(_))
    }

    /// Carries out `event` on `settings` (the account's as they are), returning what to save.
    /// `domain` is the account's address's domain.
    pub fn handle(&mut self, event: Event, settings: &AccountSettings, domain: &str) -> Saved {
        match event {
            Event::Submit(List::Addresses) => self.submit_addresses(settings, domain),
            Event::Submit(List::Exclusions) => self.submit_exclusions(settings),
            Event::Remove(list, at) => self.remove(list, at, settings),
            Event::Tab(delta) => {
                let order = Self::order(settings);
                let at = order
                    .iter()
                    .position(|focus| *focus == self.focus)
                    .unwrap_or(0);
                self.set_focus(
                    order[(at as isize + delta).rem_euclid(order.len() as isize) as usize],
                );
                Saved::default()
            }
            Event::Activate => match self.focus {
                Focus::Link(at) => {
                    self.open = Some(LINKS[at].1);
                    Saved::default()
                }
                Focus::Delete(list, at) => self.remove(list, at, settings),
                Focus::Input(list) => self.handle(Event::Submit(list), settings, domain),
            },
            Event::Focus(focus) => {
                self.set_focus(focus);
                Saved::default()
            }
        }
    }

    fn set_focus(&mut self, focus: Focus) {
        self.focus = focus;
        self.focus_pending = matches!(focus, Focus::Input(_));
    }

    /// Adds the entries typed, split at `,` and `;`, that aren't on the list
    /// yet; those with a valid address are added, the rest stay typed, marked wrong, and named in
    /// a toast. The first address added also excludes the account's own domain, unless it's a
    /// webmail one.
    fn submit_addresses(&mut self, settings: &AccountSettings, domain: &str) -> Saved {
        let mut saved = Saved::default();
        let (valid, invalid): (Vec<String>, Vec<String>) = entries(&self.inputs[0])
            .into_iter()
            .filter(|entry| !settings.auto_bcc.contains(entry))
            .partition(|entry| recipients::is_valid(&recipients::parse(entry).email));
        if !valid.is_empty() {
            self.invalid[0] = false;
            let mut list = settings.auto_bcc.clone();
            list.extend(valid);
            saved.auto_bcc = Some(list);
            let domain = domain.to_lowercase();
            if settings.auto_bcc.is_empty()
                && !crate::format::names::is_popular_domain(&domain)
                && !settings.auto_bcc_exclusions.contains(&domain)
            {
                let mut exclusions = settings.auto_bcc_exclusions.clone();
                exclusions.push(domain);
                saved.exclusions = Some(exclusions);
            }
        }
        if !invalid.is_empty() {
            self.invalid[0] = true;
            saved.toast = Some(format!(
                "The following invalid Auto Bcc addresses were not added: {}.",
                quoted(&invalid)
            ));
        }
        self.inputs[0] = invalid.join(", ");
        saved
    }

    /// Adds exclusions as for addresses, an entry being an address or a domain (a
    /// leading `*@` or `@` dropped), compared by address.
    fn submit_exclusions(&mut self, settings: &AccountSettings) -> Saved {
        let mut saved = Saved::default();
        let existing: Vec<String> = settings
            .auto_bcc_exclusions
            .iter()
            .map(|entry| crate::compose::auto_bcc::normalize(entry))
            .collect();
        let entries: Vec<String> = entries(&self.inputs[1])
            .into_iter()
            .map(|entry| {
                entry
                    .strip_prefix("*@")
                    .or_else(|| entry.strip_prefix('@'))
                    .unwrap_or(&entry)
                    .to_owned()
            })
            .collect();
        let (valid, invalid): (Vec<String>, Vec<String>) = entries
            .into_iter()
            .filter(|entry| !existing.contains(&crate::compose::auto_bcc::normalize(entry)))
            .partition(|entry| {
                recipients::is_valid(&recipients::parse(entry).email) || is_valid_domain(entry)
            });
        if !valid.is_empty() {
            let mut list = settings.auto_bcc_exclusions.clone();
            list.extend(valid);
            saved.exclusions = Some(list);
        }
        self.invalid[1] = !invalid.is_empty();
        if !invalid.is_empty() {
            saved.toast = Some(format!(
                "The following invalid Auto Bcc Exclusions were not added: {}.",
                quoted(&invalid)
            ));
        }
        self.inputs[1] = invalid.join(", ");
        saved
    }

    /// A trash can: the entry goes; its row had the keyboard, which stays on the list's next
    /// control.
    fn remove(&mut self, list: List, at: usize, settings: &AccountSettings) -> Saved {
        let mut entries = match list {
            List::Addresses => settings.auto_bcc.clone(),
            List::Exclusions => settings.auto_bcc_exclusions.clone(),
        };
        if at >= entries.len() {
            return Saved::default();
        }
        entries.remove(at);
        if let Focus::Delete(focused, row) = self.focus
            && focused == list
            && row >= entries.len()
        {
            self.set_focus(
                entries
                    .len()
                    .checked_sub(1)
                    .map_or(Focus::Input(list), |last| Focus::Delete(list, last)),
            );
        }
        match list {
            List::Addresses => Saved {
                auto_bcc: Some(entries),
                ..Saved::default()
            },
            List::Exclusions => Saved {
                exclusions: Some(entries),
                ..Saved::default()
            },
        }
    }
}

/// What's typed, split at `,` or `;` (and the spaces after), trimmed, the empty ones dropped.
fn entries(text: &str) -> Vec<String> {
    text.trim()
        .split([',', ';'])
        .map(|entry| entry.trim().to_owned())
        .filter(|entry| !entry.is_empty())
        .collect()
}

/// `'a', 'b'`.
fn quoted(entries: &[String]) -> String {
    entries
        .iter()
        .map(|entry| format!("'{entry}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether `text` reads as a domain.
fn is_valid_domain(text: &str) -> bool {
    static DOMAIN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"(?i)^(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$",
        )
        .expect("valid regex")
    });
    DOMAIN.is_match(text)
}

/// The body's height for `settings`'s lists: the two subheaders, lists and inputs.
fn body_height(settings: &AccountSettings) -> f32 {
    exclusions_top(settings)
        + EXCLUSIONS_SUBHEADER.len() as f32 * TEXT.line_height
        + list_and_input(settings.auto_bcc_exclusions.len(), EXCLUSIONS_LIST_GAP)
        + INPUT
        + EXTRA_PADDING
}

/// The exclusions' subheader block's top, under the addresses' input.
fn exclusions_top(settings: &AccountSettings) -> f32 {
    SUBHEADER_PADDING
        + TEXT.line_height
        + list_and_input(settings.auto_bcc.len(), LIST_GAP)
        + INPUT
        + SUBHEADER_PADDING
}

/// From the subheader's bottom to its input's top: the list of `count` rows, its margins
/// collapsing with the subheader's `gap` (an empty list's 20px outgrows it).
fn list_and_input(count: usize, gap: f32) -> f32 {
    if count == 0 {
        LIST_GAP.max(gap)
    } else {
        gap + count as f32 * (ITEM + ITEM_GAP) - ITEM_GAP + LIST_GAP
    }
}

/// Draws the dialog over `area`.
pub fn dialog(
    ctx: &Context,
    area: Rect,
    open: bool,
    state: &mut State,
    settings: &AccountSettings,
) -> DialogResponse<Option<Event>> {
    if let Some(url) = state.open.take() {
        ctx.open_url(egui::OpenUrl::new_tab(url));
    }
    Dialog::new("auto-bcc", "Auto Bcc", body_height(settings))
        .show(ctx, area, open, |ui, body| show(ui, body, state, settings))
}

fn show(ui: &mut Ui, body: Rect, state: &mut State, settings: &AccountSettings) -> Option<Event> {
    let ctx = ui.ctx().clone();
    let painter = ui.painter().clone();
    let left = body.left();
    let mut event = None;

    // `Do you use a CRM like HubSpot or Salesforce? Automatically Bcc your sent messages.`
    let line = body.top() + SUBHEADER_PADDING + NUDGE;
    let mut x = left;
    for (index, part) in [
        "Do you use a CRM like ",
        "HubSpot",
        " or ",
        "Salesforce",
        "? Automatically Bcc your sent messages.",
    ]
    .into_iter()
    .enumerate()
    {
        let link = (index % 2 == 1).then_some(index / 2);
        let width = TEXT.paint_line(
            &painter,
            pos2(x, line),
            part,
            if link.is_some() { LINK } else { INK },
            None,
        );
        if let Some(link) = link {
            let hit = Rect::from_min_size(pos2(x, line), vec2(width, TEXT.line_height));
            if ui
                .interact(
                    hit,
                    exo_ui::id(ui.ctx(), ("auto-bcc-link", link)),
                    Sense::click(),
                )
                .on_hover_cursor(CursorIcon::PointingHand)
                .clicked()
            {
                ctx.open_url(egui::OpenUrl::new_tab(LINKS[link].1));
                event = Some(Event::Focus(Focus::Link(link)));
            }
        }
        x += width;
    }

    let input_top = body.top()
        + SUBHEADER_PADDING
        + TEXT.line_height
        + list_and_input(settings.auto_bcc.len(), LIST_GAP);
    let list_top = |gap: f32, subheader_bottom: f32| subheader_bottom + gap;
    let rows_event = rows(
        ui,
        state,
        List::Addresses,
        &settings.auto_bcc,
        list_top(LIST_GAP, body.top() + SUBHEADER_PADDING + TEXT.line_height),
        left,
        body.width(),
    );
    event = event.or(rows_event);
    event = event.or(input(
        ui,
        state,
        List::Addresses,
        pos2(left, input_top),
        body.width(),
    ));

    let top = body.top() + exclusions_top(settings);
    for (index, text) in EXCLUSIONS_SUBHEADER.into_iter().enumerate() {
        TEXT.paint_line(
            &painter,
            pos2(left, top + NUDGE + index as f32 * TEXT.line_height),
            text,
            INK,
            None,
        );
    }
    let subheader_bottom = top + EXCLUSIONS_SUBHEADER.len() as f32 * TEXT.line_height;
    let rows_event = rows(
        ui,
        state,
        List::Exclusions,
        &settings.auto_bcc_exclusions,
        list_top(EXCLUSIONS_LIST_GAP, subheader_bottom),
        left,
        body.width(),
    );
    event = event.or(rows_event);
    let input_top =
        subheader_bottom + list_and_input(settings.auto_bcc_exclusions.len(), EXCLUSIONS_LIST_GAP);
    event.or(input(
        ui,
        state,
        List::Exclusions,
        pos2(left, input_top),
        body.width(),
    ))
}

/// A list's rows from `top`: each entry with its trash can at the right edge; the hovered or
/// focused row filled from 2px above it.
fn rows(
    ui: &Ui,
    state: &State,
    list: List,
    entries: &[String],
    top: f32,
    left: f32,
    width: f32,
) -> Option<Event> {
    let ctx = ui.ctx();
    let painter = ui.painter();
    let fill = match exo_ui::app_theme(ctx) {
        exo_ui::Theme::Snow => ROW_FILL,
        exo_ui::Theme::Carbon => ROW_FILL_CARBON,
    };
    let mut event = None;
    for (at, entry) in entries.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(left, top + at as f32 * (ITEM + ITEM_GAP) + NUDGE),
            vec2(width, ITEM),
        );
        let hovered = ui.rect_contains_pointer(row);
        let focused = state.focus == Focus::Delete(list, at);
        if hovered || focused {
            painter.rect_filled(
                Rect::from_min_max(row.min - vec2(0.0, 2.0), row.max),
                0.0,
                fill,
            );
        }
        // The entry, with its whole value as its title.
        BOLD.paint_line(painter, row.min, entry, INK, Some(width - 20.0));
        // The trash can, centred over `padding-bottom: 2px`.
        let size = exo_ui::icon_rect(
            ctx,
            TRASH,
            Rect::from_min_size(egui::Pos2::ZERO, vec2(14.0, 14.0)),
        )
        .size();
        let can = Rect::from_min_size(
            pos2(
                row.right() - size.x,
                row.top() + (ITEM - 2.0 - size.y) / 2.0,
            ),
            size,
        );
        let button = Rect::from_min_size(pos2(row.right() - size.x, row.top()), vec2(size.x, ITEM));
        let response = ui
            .interact(
                button,
                exo_ui::id(ui.ctx(), ("auto-bcc-delete", list as usize, at)),
                Sense::click(),
            )
            .on_hover_cursor(CursorIcon::PointingHand);
        let ink = exo_ui::motion::transition(
            ctx,
            exo_ui::id(ctx, ("auto-bcc-delete-ink", list as usize, at)),
            response.hovered() || focused,
            0.064,
            exo_ui::motion::CubicBezier::LINEAR,
        );
        exo_ui::paint_icon(
            ui,
            TRASH,
            can,
            Color32::BLACK.gamma_multiply(TRASH_INK + (TRASH_HOVER_INK - TRASH_INK) * ink),
        );
        if response.clicked() {
            event = Some(Event::Remove(list, at));
        }
    }
    event
}

/// A list's input, its line at `at`.
fn input(ui: &mut Ui, state: &mut State, list: List, at: egui::Pos2, width: f32) -> Option<Event> {
    let index = list as usize;
    let id = exo_ui::id(ui.ctx(), ("auto-bcc-input", index));
    let line = Rect::from_min_size(at + vec2(0.0, NUDGE), vec2(width, INPUT));
    let before = state.inputs[index].clone();
    let response = DialogInput::new(BOLD, list.placeholder())
        .error(state.invalid[index])
        .show(ui, id, &mut state.inputs[index], line);
    if state.inputs[index] != before {
        // Typing clears the mark.
        state.invalid[index] = false;
    }
    if state.focus == Focus::Input(list) && std::mem::take(&mut state.focus_pending) {
        let end = egui::text::CCursor::new(state.inputs[index].chars().count());
        exo_ui::focus_field(ui.ctx(), id, egui::text::CCursorRange::one(end));
    }
    if response.gained_focus() && state.focus != Focus::Input(list) {
        return Some(Event::Focus(Focus::Input(list)));
    }
    if state.focus != Focus::Input(list) && response.has_focus() {
        response.surrender_focus();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(auto_bcc: &[&str], exclusions: &[&str]) -> AccountSettings {
        let mut settings = AccountSettings::default();
        settings.auto_bcc = auto_bcc.iter().map(|entry| (*entry).to_owned()).collect();
        settings.auto_bcc_exclusions = exclusions.iter().map(|entry| (*entry).to_owned()).collect();
        settings
    }

    #[test]
    fn the_first_address_also_excludes_your_domain() {
        let mut state = State::default();
        state.inputs[0] = "123@bcc.hubspot.com; nope, ".into();
        let saved = state.handle(
            Event::Submit(List::Addresses),
            &settings(&[], &[]),
            "acme.com",
        );
        assert_eq!(saved.auto_bcc, Some(vec!["123@bcc.hubspot.com".to_owned()]));
        assert_eq!(saved.exclusions, Some(vec!["acme.com".to_owned()]));
        assert_eq!(
            saved.toast.as_deref(),
            Some("The following invalid Auto Bcc addresses were not added: 'nope'.")
        );
        assert_eq!((state.inputs[0].as_str(), state.invalid[0]), ("nope", true));
        let mut state = State::default();
        state.inputs[0] = "CRM <crm@bcc.example.org>".into();
        let saved = state.handle(
            Event::Submit(List::Addresses),
            &settings(&[], &[]),
            "gmail.com",
        );
        assert_eq!(
            (saved.auto_bcc.map(|list| list.len()), saved.exclusions),
            (Some(1), None),
            "a webmail domain isn't excluded"
        );
        let saved = state.handle(
            Event::Submit(List::Addresses),
            &settings(&["a@b.co"], &[]),
            "acme.com",
        );
        assert_eq!(saved, Saved::default(), "nothing typed, nothing done");
    }

    #[test]
    fn exclusions_take_addresses_and_domains() {
        let mut state = State::default();
        state.inputs[1] = "*@firm.com, @fund.vc, joe@example.com, Example.com, not a domain".into();
        let saved = state.handle(
            Event::Submit(List::Exclusions),
            &settings(&[], &["example.com"]),
            "acme.com",
        );
        assert_eq!(
            saved.exclusions,
            Some(vec![
                "example.com".to_owned(),
                "firm.com".into(),
                "fund.vc".into(),
                "joe@example.com".into()
            ])
        );
        assert_eq!(
            saved.toast.as_deref(),
            Some("The following invalid Auto Bcc Exclusions were not added: 'not a domain'.")
        );
    }

    #[test]
    fn tab_goes_through_the_links_trash_cans_and_inputs() {
        let mut state = State::default();
        let settings = settings(&["a@b.co"], &["c.co"]);
        let mut seen = Vec::new();
        for _ in 0..6 {
            state.handle(Event::Tab(1), &settings, "x.co");
            seen.push(state.focus);
        }
        assert_eq!(
            seen,
            [
                Focus::Delete(List::Exclusions, 0),
                Focus::Input(List::Exclusions),
                Focus::Link(0),
                Focus::Link(1),
                Focus::Delete(List::Addresses, 0),
                Focus::Input(List::Addresses)
            ]
        );
        state.handle(Event::Tab(-1), &settings, "x.co");
        let saved = state.handle(Event::Activate, &settings, "x.co");
        assert_eq!(
            saved.auto_bcc,
            Some(Vec::new()),
            "Enter on a trash can removes its entry"
        );
    }

    #[test]
    fn the_body_grows_with_the_lists() {
        assert_eq!(
            body_height(&settings(&[], &[])),
            192.0,
            "measured: 252px with the shell's 60"
        );
        assert_eq!(
            body_height(&settings(&["a@b.co", "c@d.co"], &["e.co"])),
            296.0,
            "measured: 356px"
        );
    }
}
