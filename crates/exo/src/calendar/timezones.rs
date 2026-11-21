//! The Timezone input and its search: every place, abbreviation and zone of the time parser's
//! tables under a search key, the common ones first, matched by prefix.

use std::{collections::HashMap, sync::OnceLock};

use egui::{Color32, Id, Rect, Ui, pos2, vec2};
use exo_ui::{Family, Font, TextStyle};
use jiff::{Timestamp, tz::TimeZone};

use crate::time_suggest::zone_tables;

/// A zone the search found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub iana: String,
    /// `New York`.
    pub name: String,
    /// `GMT+00:00`: the zone's own offset, not the one Exo is in.
    pub offset: String,
    /// `EST`, or a bare offset written as one (`GMT+05:30`).
    pub abbreviation: String,
}

impl Found {
    /// `iana`, as the input shows it at `at`.
    pub fn new(iana: &str, name: String, at: Timestamp) -> Option<Self> {
        let tz = TimeZone::get(iana).ok()?;
        Some(Self {
            iana: iana.to_owned(),
            name,
            offset: offset(&tz, at),
            abbreviation: super::availability::abbreviation(&tz, at),
        })
    }

    /// The input's text for it: the offset padded to 16 characters, then the name.
    pub fn text(&self) -> String {
        format!("{:<16}{}", self.offset, self.name)
    }
}

/// The zone `iana` as the input shows it at `at`, named by its last segment.
pub fn current(iana: &str, at: Timestamp) -> Option<Found> {
    Found::new(iana, last_segment(iana), at)
}

/// The first `limit` zones whose search key starts with `text`'s, the common ones first.
pub fn search(text: &str, at: Timestamp, limit: usize) -> Vec<Found> {
    let key = search_key(text);
    let (keys, entries) = index();
    keys.iter()
        .filter(|candidate| candidate.starts_with(&key))
        .filter_map(|candidate| entries.get(candidate))
        .filter_map(|(iana, name)| Found::new(iana, name.clone(), at))
        .take(limit)
        .collect()
}

/// What the input lists when it takes the keyboard: `current` over the common zones, `limit` in
/// all.
pub fn suggested(current: Option<&Found>, at: Timestamp, limit: usize) -> Vec<Found> {
    let common = search("", at, limit);
    current
        .into_iter()
        .cloned()
        .chain(
            common
                .into_iter()
                .filter(|found| Some(&found.name) != current.map(|current| &current.name)),
        )
        .take(limit)
        .collect()
}

/// The input's text, 12px/16px over `padding-bottom: 2px` and a 2px rule.
const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
const RULE: f32 = 2.0;
/// Its list hangs 16px left of its box and 4px under it.
const LIST_LEFT: f32 = 16.0;
const LIST_BELOW: f32 = 4.0;
/// `#aeb1dd`, a Classic literal both themes keep.
const RULE_INK: Color32 = Color32::from_rgb(0xae, 0xb1, 0xdd);

/// A Timezone input's state: what's typed, and its list.
#[derive(Default)]
pub struct ZoneInput {
    text: String,
    typed: bool,
    zones: Vec<Found>,
    pick: usize,
}

/// How a Timezone input draws where it's used.
pub struct ZoneField<'a> {
    pub id: Id,
    /// The text's line box.
    pub line: Rect,
    /// The zone it shows, by IANA id.
    pub zone: Option<&'a str>,
    pub now: Timestamp,
    /// How many zones its list shows.
    pub limit: usize,
    /// The text's and the placeholder's colours.
    pub ink: Color32,
    pub placeholder: Color32,
    /// Its rule always shows (in Calendar Settings), or only while it's focused.
    pub always_ruled: bool,
    /// A pick gives the keyboard up.
    pub blur_on_pick: bool,
    pub list_width: f32,
    pub rows: super::share::ZoneRows,
}

/// What a Timezone input did this frame.
pub struct ZoneResponse {
    /// The zone picked, by IANA id.
    pub picked: Option<String>,
    pub focused: bool,
}

impl ZoneInput {
    /// Paints the input: the zone (`GMT+00:00       London`), or typed text with the zones it
    /// finds, picked with the pointer or ↑/↓ and Enter or Tab.
    pub fn show(&mut self, ui: &mut Ui, field: &ZoneField<'_>) -> ZoneResponse {
        let palette = exo_ui::palette(ui.ctx());
        let id = field.id;
        let current = field.zone.and_then(|iana| current(iana, field.now));
        if !self.typed {
            self.text = current.as_ref().map(Found::text).unwrap_or_default();
        }
        let had_focus = ui.memory(|memory| memory.has_focus(id));
        let keys = ui.input(|input| {
            (
                input.key_pressed(egui::Key::ArrowUp),
                input.key_pressed(egui::Key::ArrowDown),
                input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Tab),
            )
        });
        let length = self.text.chars().count();
        let response = exo_ui::text_field(
            ui,
            id,
            &mut self.text,
            field.line,
            &TEXT,
            field.ink,
            palette.text_selection_default,
        );
        if self.text.is_empty() && !response.has_focus() {
            TEXT.paint_line(
                ui.painter(),
                field.line.min,
                "Enter timezone or location",
                field.placeholder,
                Some(field.line.width()),
            );
        }
        let field_box = Rect::from_min_size(
            field.line.min,
            vec2(field.line.width(), TEXT.line_height + 2.0 + RULE),
        );
        if field.always_ruled || response.has_focus() {
            ui.painter().rect_filled(
                Rect::from_min_max(
                    pos2(field_box.left(), field_box.bottom() - RULE),
                    field_box.max,
                ),
                0.0,
                RULE_INK,
            );
        }
        if response.gained_focus() {
            // Its text selected, and the list: what was typed, else this zone over the common ones.
            exo_ui::focus_field(
                ui.ctx(),
                id,
                egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(length),
                ),
            );
            self.zones = if self.typed {
                search(&self.text, field.now, field.limit)
            } else {
                suggested(current.as_ref(), field.now, field.limit)
            };
            self.pick = 0;
        }
        if response.changed() {
            self.typed = true;
            self.zones = search(&self.text, field.now, field.limit);
            self.pick = 0;
        }
        let mut picked = None;
        if had_focus && !self.zones.is_empty() {
            match keys {
                (true, _, _) => self.pick = self.pick.saturating_sub(1),
                (_, true, _) => self.pick = (self.pick + 1).min(self.zones.len() - 1),
                (_, _, true) => picked = self.zones.get(self.pick).cloned(),
                _ => {}
            }
        }
        if response.has_focus() && !self.zones.is_empty() {
            let rows = &field.rows;
            let list = Rect::from_min_size(
                pos2(
                    field_box.left() - LIST_LEFT,
                    field_box.bottom() + LIST_BELOW,
                ),
                vec2(
                    field.list_width,
                    self.zones.len() as f32 * rows.row + rows.pad + rows.bottom,
                ),
            );
            let (hovered, clicked) =
                super::share::zone_list(ui, id.with("list"), list, &self.zones, self.pick, rows);
            self.pick = hovered.unwrap_or(self.pick);
            picked = picked.or_else(|| clicked.and_then(|index| self.zones.get(index).cloned()));
        }
        if response.lost_focus() && picked.is_none() {
            self.zones.clear();
            // Losing the keyboard, an emptied field takes the zone back.
            if self.text.trim().is_empty() {
                self.typed = false;
            }
        }
        let kept = picked.is_some() && !field.blur_on_pick;
        let picked = picked.map(|picked| {
            self.typed = false;
            self.zones.clear();
            if field.blur_on_pick {
                response.surrender_focus();
            } else {
                // Tab takes the pick and stays, as Enter does.
                ui.memory_mut(|memory| memory.request_focus(id));
            }
            picked.iana
        });
        ZoneResponse {
            picked,
            focused: response.has_focus() || kept,
        }
    }
}

/// The zone's offset at `at`: `GMT−05:00`, with a real minus sign.
fn offset(tz: &TimeZone, at: Timestamp) -> String {
    let seconds = tz.to_offset(at).seconds();
    let sign = if seconds < 0 { "−" } else { "+" };
    let minutes = seconds.abs() / 60;
    format!("GMT{sign}{:02}:{:02}", minutes / 60, minutes % 60)
}

/// A zone's name from its IANA id: `America/New_York` is `New York`.
fn last_segment(iana: &str) -> String {
    iana.rsplit('/').next().unwrap_or(iana).replace('_', " ")
}

/// A search key: lower-case letters only.
fn search_key(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .filter(char::is_ascii_lowercase)
        .collect()
}

type Entry = (&'static str, String);

/// The search keys in their order and what each finds: places, then abbreviations (named by
/// their zone), places' short names (as the place), then every zone by its last segment; a later
/// key replaces an earlier.
fn index() -> &'static (Vec<String>, HashMap<String, Entry>) {
    static INDEX: OnceLock<(Vec<String>, HashMap<String, Entry>)> = OnceLock::new();
    INDEX.get_or_init(|| {
        let (places, abbreviations, place_abbreviations, zones) = zone_tables();
        let mut entries: HashMap<String, Entry> = HashMap::new();
        for (name, iana) in places {
            entries.insert(search_key(name), (iana, (*name).to_owned()));
        }
        for (abbreviation, iana) in abbreviations
            .iter()
            .filter(|(_, iana)| !iana.starts_with("Etc/GMT"))
        {
            entries.insert(search_key(abbreviation), (iana, last_segment(iana)));
        }
        for (short, place) in place_abbreviations {
            if let Some(entry) = entries.get(&search_key(place)).cloned() {
                entries.insert(search_key(short), entry);
            }
        }
        for iana in zones.iter().filter(|iana| !iana.starts_with("Etc/GMT")) {
            let name = last_segment(iana);
            entries.insert(search_key(&name), (iana, name));
        }
        const FIRST: [&str; 10] = [
            "newyork",
            "losangeles",
            "chicago",
            "london",
            "denver",
            "sanfrancisco",
            "easterntime",
            "centraltime",
            "mountaintime",
            "pacifictime",
        ];
        let mut keys: Vec<String> = entries.keys().cloned().collect();
        keys.sort_by(|a, b| {
            let rank = |key: &String| {
                FIRST
                    .iter()
                    .position(|first| first == key)
                    .unwrap_or(FIRST.len())
            };
            rank(a).cmp(&rank(b)).then_with(|| a.cmp(b))
        });
        (keys, entries)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> Timestamp {
        "2026-11-17T14:00:00Z".parse().expect("timestamp")
    }

    #[test]
    fn the_common_zones_come_first() {
        let names: Vec<String> = search("", at(), 4)
            .into_iter()
            .map(|found| found.name)
            .collect();
        assert_eq!(names, ["New York", "Los Angeles", "Chicago", "London"]);
    }

    #[test]
    fn a_prefix_finds_places_abbreviations_and_zones() {
        let found = search("lon", at(), 4);
        // Each with its own offset, not the one Exo is in.
        assert_eq!(
            found.first().map(|found| (
                found.iana.as_str(),
                found.text(),
                found.abbreviation.as_str()
            )),
            Some(("Europe/London", "GMT+00:00       London".to_owned(), "GMT"))
        );
        let pst = search("pst", at(), 4);
        assert_eq!(
            pst.first().map(|found| (
                found.iana.as_str(),
                found.name.as_str(),
                found.abbreviation.as_str()
            )),
            Some(("America/Los_Angeles", "Los Angeles", "PST"))
        );
        // Places' short names stand for the place.
        assert_eq!(
            search("nyc", at(), 1)
                .first()
                .map(|found| found.name.as_str()),
            Some("New York")
        );
    }

    #[test]
    fn the_input_writes_the_offset_padded() {
        let new_york = current("America/New_York", at()).expect("zone");
        assert_eq!(new_york.text(), "GMT−05:00       New York");
        assert_eq!(new_york.abbreviation, "EST");
    }
}
