//! Recipient suggestions, ranked: each contact is found by its domain (at its
//! score), its address and every suffix of its name that starts a word (at its score + 3), all
//! by prefix, lower-cased and without diacritics. The open thread's participants come first, then
//! the address book; domain matches after both.

use std::collections::HashSet;

use exo_store::{Address, ScoredContact};
use unicode_normalization::UnicodeNormalization;

use super::recipients;

/// How many suggestions show.
pub(crate) const LIMIT: usize = 5;
/// What addresses and names score over domains.
const SCORE_RANGE: f64 = 3.0;
/// Thread participants are indexed at this score.
const PARTICIPANT_SCORE: f64 = 3.0;
/// Keys are cut to this many bytes.
const MAX_KEY: usize = 320;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Domain,
    Other,
}

struct Key {
    key: String,
    kind: Kind,
    contact: usize,
    score: f64,
}

/// A prefix index over contacts (the trie).
#[derive(Default)]
pub(crate) struct Index {
    contacts: Vec<Address>,
    /// Sorted by key.
    keys: Vec<Key>,
}

/// One match.
struct Match {
    contact: usize,
    score: f64,
}

impl Index {
    /// Indexes `contacts` at their scores; `no-reply` addresses aren't.
    pub fn new(contacts: impl IntoIterator<Item = (Address, f64)>) -> Self {
        let mut index = Self::default();
        for (contact, score) in contacts {
            index.add(contact, score);
        }
        index.keys.sort_by(|a, b| a.key.cmp(&b.key));
        index
    }

    /// The address book.
    pub fn book(contacts: &[ScoredContact]) -> Self {
        Self::book_without(contacts, &std::collections::BTreeMap::new())
    }

    /// The address book but the addresses `Remove Suggestion` took out: compose never suggests
    /// them.
    pub fn book_without(
        contacts: &[ScoredContact],
        excluded: &std::collections::BTreeMap<String, String>,
    ) -> Self {
        let kept = contacts.iter().filter(|contact| {
            !excluded
                .keys()
                .any(|email| email.eq_ignore_ascii_case(&contact.email))
        });
        Self::new(kept.map(|contact| {
            (
                Address {
                    name: contact.name.clone(),
                    email: contact.email.clone(),
                },
                contact.score,
            )
        }))
    }

    /// Only the addresses `Remove Suggestion` took out, which search suggests after the rest.
    pub fn excluded(
        contacts: &[ScoredContact],
        excluded: &std::collections::BTreeMap<String, String>,
    ) -> Self {
        let removed = contacts.iter().filter(|contact| {
            excluded
                .keys()
                .any(|email| email.eq_ignore_ascii_case(&contact.email))
        });
        Self::new(removed.map(|contact| {
            (
                Address {
                    name: contact.name.clone(),
                    email: contact.email.clone(),
                },
                contact.score,
            )
        }))
    }

    fn add(&mut self, contact: Address, score: f64) {
        let email = contact.email.to_lowercase();
        if email.is_empty() || no_reply(&email) {
            return;
        }
        let at = self.contacts.len();
        let domain = email
            .split_once('@')
            .map_or("", |(_, domain)| domain)
            .to_owned();
        self.keys.push(Key {
            key: truncate(domain),
            kind: Kind::Domain,
            contact: at,
            score,
        });
        self.keys.push(Key {
            key: truncate(email.clone()),
            kind: Kind::Other,
            contact: at,
            score: score + SCORE_RANGE,
        });
        let name = contact.name.as_deref().map(normalize).unwrap_or_default();
        if !name.is_empty() && name != email {
            // The name, and each suffix starting after a space.
            let mut suffixes = vec![name.as_str()];
            suffixes.extend(
                name.match_indices(' ')
                    .map(|(space, _)| &name[space + 1..])
                    .filter(|suffix| !suffix.is_empty()),
            );
            for suffix in suffixes {
                self.keys.push(Key {
                    key: truncate(suffix.to_owned()),
                    kind: Kind::Other,
                    contact: at,
                    score: score + SCORE_RANGE,
                });
            }
        }
        self.contacts.push(contact);
    }

    /// Every contact a key of which starts with `query`, best first (score, then address,
    /// then named before unnamed), once per contact: (address and name matches, domain
    /// matches).
    fn query(&self, query: &str) -> (Vec<Match>, Vec<Match>) {
        let query = truncate(normalize(query));
        let start = self
            .keys
            .partition_point(|key| key.key.as_str() < query.as_str());
        let mut best: Vec<Option<(f64, Kind)>> = vec![None; self.contacts.len()];
        for key in self.keys[start..]
            .iter()
            .take_while(|key| key.key.starts_with(&query))
        {
            let slot = &mut best[key.contact];
            if slot.is_none_or(|(score, _)| key.score > score) {
                *slot = Some((key.score, key.kind));
            }
        }
        let mut others = Vec::new();
        let mut domains = Vec::new();
        for (contact, slot) in best.into_iter().enumerate() {
            let Some((score, kind)) = slot else { continue };
            let found = Match { contact, score };
            if kind == Kind::Domain {
                domains.push(found)
            } else {
                others.push(found)
            }
        }
        let order = |a: &Match, b: &Match| {
            let (left, right) = (&self.contacts[a.contact], &self.contacts[b.contact]);
            b.score
                .total_cmp(&a.score)
                .then_with(|| left.email.to_lowercase().cmp(&right.email.to_lowercase()))
                .then_with(|| match (&left.name, &right.name) {
                    (Some(a), Some(b)) => a.cmp(b),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                })
        };
        others.sort_by(order);
        domains.sort_by(order);
        (others, domains)
    }

    /// The best `limit` contacts a key of which starts with `query`, by
    /// address and name first, then by domain (what the search bar suggests people from).
    pub fn top(&self, query: &str, limit: usize) -> Vec<&Address> {
        let (others, domains) = self.query(query);
        others
            .iter()
            .chain(&domains)
            .take(limit)
            .map(|found| self.contact(found))
            .collect()
    }

    /// The contact the book has for `email`.
    pub fn get(&self, email: &str) -> Option<&Address> {
        self.contacts
            .iter()
            .find(|contact| contact.email.eq_ignore_ascii_case(email))
    }

    /// The best-known name for `email`.
    pub fn name_for(&self, email: &str) -> Option<String> {
        let (others, _) = self.query(email);
        others
            .into_iter()
            .map(|found| &self.contacts[found.contact])
            .find(|contact| contact.email.eq_ignore_ascii_case(email))
            .and_then(|contact| contact.name.clone())
    }

    fn contact(&self, found: &Match) -> &Address {
        &self.contacts[found.contact]
    }
}

/// The open thread's participants, you included, as their own index (score 3): those the
/// address book doesn't know, or knows by a name no better than the thread's.
pub(crate) fn participants<'a>(
    people: impl IntoIterator<Item = &'a Address>,
    book: &Index,
) -> Index {
    let mut seen = HashSet::new();
    let fresh: Vec<(Address, f64)> = people
        .into_iter()
        .filter(|person| {
            recipients::is_valid(&person.email) && seen.insert(person.email.to_lowercase())
        })
        .filter(|person| {
            let Some(known) = book.get(&person.email) else {
                return true;
            };
            let name = person.name.as_deref().unwrap_or_default();
            better_name(name, known.name.as_deref().unwrap_or_default())
                .is_some_and(|better| !better.is_empty() && better == name)
        })
        .map(|person| {
            (
                Address {
                    name: person.name.clone(),
                    email: person.email.to_lowercase(),
                },
                PARTICIPANT_SCORE,
            )
        })
        .collect();
    Index::new(fresh)
}

/// Which of two names for one address says more, if they're the
/// same person's: the one the other is part of, the longer when they're alike, the one whose
/// words the other has all of; none when they share no first word.
fn better_name<'a>(a: &'a str, b: &'a str) -> Option<&'a str> {
    let squash = |name: &str| {
        name.to_lowercase()
            .chars()
            .filter(|ch| !"?,().\"".contains(*ch) && !ch.is_whitespace())
            .collect::<String>()
    };
    let (squashed_a, squashed_b) = (squash(a), squash(b));
    // Lengths are counted in UTF-16 units.
    let length = |name: &str| name.encode_utf16().count();
    if squashed_a == squashed_b {
        return Some(if length(a) > length(b) { a } else { b });
    }
    if squashed_a.contains(&squashed_b) {
        return Some(a);
    }
    if squashed_b.contains(&squashed_a) {
        return Some(b);
    }
    let words = |name: &str| {
        name.to_lowercase()
            .trim()
            .chars()
            .filter(|ch| !"?,().\"".contains(*ch))
            .collect::<String>()
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let (words_a, words_b) = (words(a), words(b));
    let (Some(first_a), Some(first_b)) = (words_a.first(), words_b.first()) else {
        return None;
    };
    let a_in_b = words_a.iter().all(|word| words_b.contains(word));
    let b_in_a = words_b.iter().all(|word| words_a.contains(word));
    match (a_in_b, b_in_a) {
        (true, false) => Some(a),
        (false, true) => Some(b),
        (true, true) => Some(if length(a) <= length(b) { a } else { b }),
        _ if first_a != first_b => None,
        _ => Some(if words_a.len() > words_b.len() { b } else { a }),
    }
}

/// The suggestions for `query`: thread participants, then
/// the address book, then each one's domain matches; one per address, at most five. Empty for a
/// blank query.
pub(crate) fn suggest(book: &Index, thread: &Index, query: &str) -> Vec<Address> {
    suggest_sourced(book, thread, query)
        .into_iter()
        .map(|(address, _)| address)
        .collect()
}

/// [`suggest`], each suggestion with whether it came from the thread's participants, which
/// can't be removed.
pub(crate) fn suggest_sourced(book: &Index, thread: &Index, query: &str) -> Vec<(Address, bool)> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let (thread_others, thread_domains) = thread.query(query);
    let (book_others, book_domains) = book.query(query);
    let mut seen = HashSet::new();
    let from_thread = |found: &Match| (thread.contact(found), true);
    let from_book = |found: &Match| (book.contact(found), false);
    let ordered = thread_others
        .iter()
        .map(from_thread)
        .chain(book_others.iter().take(LIMIT).map(from_book))
        .chain(thread_domains.iter().map(from_thread))
        .chain(book_domains.iter().map(from_book));
    ordered
        .filter(|(contact, _)| seen.insert(contact.email.to_lowercase()))
        .take(LIMIT)
        .map(|(contact, thread)| (contact.clone(), thread))
        .collect()
}

/// What to call `person`, from their name, the address book's, or their address's local part;
/// with no address, their name as typed.
pub(crate) fn best_known_first_name(person: &Address, book: Option<&Index>) -> Option<String> {
    if person.email.is_empty() {
        return person.name.clone().filter(|name| !name.is_empty());
    }
    let name = person
        .name
        .clone()
        .filter(|name| !name.is_empty())
        .or_else(|| book.and_then(|book| book.name_for(&person.email)))
        .unwrap_or_else(|| {
            person
                .email
                .split('@')
                .next()
                .unwrap_or_default()
                .to_owned()
        });
    Some(crate::format::names::first_name(&name, &person.email))
}

/// A suggestion where domains can be picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Suggestion {
    /// Someone.
    Contact(Address),
    /// Everyone at a domain.
    Domain(String),
}

/// The suggestions for `query` where a domain can be picked, with no thread: the address book's
/// matches, where the first contact found by its domain stands for that domain, and the domain
/// that best fits the query (the shortest that starts with it, else the first) goes first; at
/// most five.
pub(crate) fn suggest_with_domains(book: &Index, query: &str) -> Vec<Suggestion> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let (others, domains) = book.query(query);
    let mut found: Vec<Suggestion> = others
        .iter()
        .take(LIMIT)
        .map(|found| Suggestion::Contact(book.contact(found).clone()))
        .collect();
    for contact in domains.iter().map(|found| book.contact(found)) {
        let domain = contact
            .email
            .split_once('@')
            .map_or("", |(_, domain)| domain)
            .to_lowercase();
        let listed = found.iter().any(
            |suggestion| matches!(suggestion, Suggestion::Domain(listed) if *listed == domain),
        );
        found.push(if listed {
            Suggestion::Contact(contact.clone())
        } else {
            Suggestion::Domain(domain)
        });
    }
    let mut seen = HashSet::new();
    found.retain(|suggestion| {
        seen.insert(match suggestion {
            Suggestion::Contact(contact) => contact.email.to_lowercase(),
            Suggestion::Domain(domain) => domain.clone(),
        })
    });
    let lower = query.to_lowercase();
    let domains: Vec<(usize, &String)> = found
        .iter()
        .enumerate()
        .filter_map(|(at, suggestion)| match suggestion {
            Suggestion::Domain(domain) => Some((at, domain)),
            Suggestion::Contact(_) => None,
        })
        .collect();
    let best = domains
        .iter()
        .filter(|(_, domain)| domain.starts_with(&lower))
        .min_by_key(|(_, domain)| domain.len())
        .or(domains.first())
        .map(|(at, _)| *at);
    if let Some(best) = best {
        let domain = found.remove(best);
        found.insert(0, domain);
    }
    found.truncate(LIMIT);
    found
}

impl crate::Mail {
    /// `Remove Suggestion` on suggestion `index` of the focused field: its address joins the
    /// excluded contacts, the suggestions are asked again without it, and a toast offers Undo
    /// (⌘Z, also in the field until something's typed).
    pub(super) fn remove_suggestion(&mut self, index: usize) {
        let Some(open) = self.compose.focused_mut() else {
            return;
        };
        let Some(list) = open.state.field.list() else {
            return;
        };
        let input = open.input_mut(list);
        let Some(address) = input.suggestions.get(index).cloned() else {
            return;
        };
        input.just_excluded = Some(address.clone());
        let at = jiff::Timestamp::now().to_string();
        self.account.update_settings(|settings| {
            settings.excluded_contacts.insert(address.email.clone(), at);
        });
        self.suggest_again();
        let undo = crate::commands::Action::Compose(super::Action::UndoRemoveSuggestion(
            address.email.clone(),
        ));
        self.commands.toast(
            exo_commands::Toast::new(format!("Removed {}.", address.email)).action(
                "Undo",
                Some("mod+z"),
                undo,
            ),
        );
    }

    /// The toast's Undo, or ⌘Z in the field right after: `email` is suggested again.
    pub(super) fn undo_remove_suggestion(&mut self, email: &str) {
        self.account.update_settings(|settings| {
            settings.excluded_contacts.remove(email);
        });
        // From the field, the toast goes too.
        if self.commands.toast_text() == Some(format!("Removed {email}.").as_str()) {
            self.commands.hide_toast();
        }
        for open in &mut self.compose.drafts {
            for input in &mut open.inputs {
                input.just_excluded = None;
            }
        }
        self.suggest_again();
    }

    /// Asks the focused field's suggestions again for what's typed, against the address book as
    /// its exclusions stand now, keeping the highlight where it was (clamped).
    fn suggest_again(&mut self) {
        self.index_address_book();
        let super::Compose {
            drafts,
            focus,
            thread_people,
            book,
            ..
        } = &mut self.compose;
        let book = book.as_ref().map(|(_, index)| index);
        let Some(open) = drafts
            .iter_mut()
            .find(|open| focus.as_deref() == Some(open.id.as_str()))
        else {
            return;
        };
        let Some(list) = open.state.field.list() else {
            return;
        };
        let people = thread_people
            .as_ref()
            .filter(|(thread, _)| open.thread.as_deref() == Some(thread.as_str()))
            .map(|(_, index)| index);
        let empty = Index::default();
        let input = open.input_mut(list);
        let found = book
            .map(|book| suggest_sourced(book, people.unwrap_or(&empty), &input.text))
            .unwrap_or_default();
        (input.suggestions, input.from_thread) = found.into_iter().unzip();
        input.highlight = input
            .highlight
            .min(input.suggestions.len().saturating_sub(1));
    }
}

/// Lower-cased, without combining marks.
fn normalize(text: &str) -> String {
    text.to_lowercase()
        .nfd()
        .filter(|ch| !('\u{300}'..='\u{36f}').contains(ch))
        .collect()
}

fn truncate(mut key: String) -> String {
    if key.len() > MAX_KEY {
        let mut end = MAX_KEY;
        while !key.is_char_boundary(end) {
            end -= 1;
        }
        key.truncate(end);
    }
    key
}

fn no_reply(email: &str) -> bool {
    email.contains("noreply") || email.contains("no-reply")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(name: &str, email: &str, score: f64) -> ScoredContact {
        ScoredContact {
            email: email.into(),
            name: Some(name.into()).filter(|name: &String| !name.is_empty()),
            score,
        }
    }

    fn emails(list: &[Address]) -> Vec<&str> {
        list.iter().map(|address| address.email.as_str()).collect()
    }

    fn book() -> Index {
        Index::book(&[
            contact("Priya Raman", "priya@brightmoor.co", 1.5),
            contact("Tom Becker", "tom@brightmoor.co", 1.2),
            contact("Ana de la Cruz", "ana@calloway.com", 0.01),
            contact("", "noreply@calloway.com", 9.0),
            contact("Pri Sharma", "pri@x.com", 0.0001),
        ])
    }

    #[test]
    fn names_addresses_and_last_names_match_by_prefix() {
        let book = book();
        let empty = Index::default();
        assert_eq!(
            emails(&suggest(&book, &empty, "pri")),
            ["priya@brightmoor.co", "pri@x.com"],
            "by score"
        );
        assert_eq!(
            emails(&suggest(&book, &empty, "becker")),
            ["tom@brightmoor.co"]
        );
        assert_eq!(
            emails(&suggest(&book, &empty, "la cr")),
            ["ana@calloway.com"],
            "a suffix of the name"
        );
        assert_eq!(
            emails(&suggest(&book, &empty, "TOM@")),
            ["tom@brightmoor.co"]
        );
        assert!(suggest(&book, &empty, "  ").is_empty());
    }

    #[test]
    fn domains_list_their_people_last_and_no_reply_never() {
        let book = book();
        let found = suggest(&book, &Index::default(), "brightmoor");
        assert_eq!(emails(&found), ["priya@brightmoor.co", "tom@brightmoor.co"]);
        assert!(suggest(&book, &Index::default(), "noreply").is_empty());
    }

    #[test]
    fn a_domain_stands_for_the_first_contact_found_by_it_and_goes_first() {
        let book = book();
        let domain = |name: &str| Suggestion::Domain(name.to_owned());
        let contact = |email: &str| {
            Suggestion::Contact(Address {
                name: book.name_for(email),
                email: email.to_owned(),
            })
        };
        assert_eq!(
            suggest_with_domains(&book, "brightmoor"),
            [domain("brightmoor.co"), contact("tom@brightmoor.co")]
        );
        assert_eq!(
            suggest_with_domains(&book, "pri"),
            [contact("priya@brightmoor.co"), contact("pri@x.com")],
            "no domain matches"
        );
        assert_eq!(
            suggest_with_domains(&book, "calloway.c"),
            [domain("calloway.com")],
            "no-reply addresses aren't indexed"
        );
        assert!(suggest_with_domains(&book, " ").is_empty());
    }

    #[test]
    fn thread_participants_come_first_and_diacritics_dont_matter() {
        let book = book();
        let thread = participants(
            &[Address {
                name: Some("Zoë Prior".into()),
                email: "zoe@other.org".into(),
            }],
            &book,
        );
        assert_eq!(
            emails(&suggest(&book, &thread, "pri")),
            ["zoe@other.org", "priya@brightmoor.co", "pri@x.com"]
        );
        assert_eq!(emails(&suggest(&book, &thread, "zoe")), ["zoe@other.org"]);
        assert_eq!(
            book.name_for("PRIYA@brightmoor.co").as_deref(),
            Some("Priya Raman")
        );
    }

    /// The mock account's address book, each contact at its score.
    const MOCK_BOOK: &[(&str, &str, f64)] = &[
        ("maya@brightmoor.co", "Maya Okafor", 1.000000405960896),
        ("lena@brightmoor.co", "Lena Fischer", 1.000000115518068),
        ("daniel@brightmoor.co", "Daniel Reyes", 1.000000100195854),
        ("tom@brightmoor.co", "Tom Becker", 1.000000081004806),
        ("priya@brightmoor.co", "Priya Raman", 1.000000061802938),
        ("oliver@brightmoor.co", "Oliver Grant", 1.000000061743918),
        (
            "liam.oconnell@fastmail.com",
            "Liam O'Connell",
            1.000000061680446,
        ),
        ("hiro@brightmoor.co", "Hiro Tanaka", 1.000000057904684),
        ("chloe@brightmoor.co", "Chloé Martin", 1.00000005408233),
        ("jessmoreno.nyc@gmail.com", "Jess Moreno", 1.00000005026221),
        ("hweiss@gmail.com", "Hannah Weiss", 1.000000046354222),
        ("jordan@brightmoor.co", "Jordan Ellis", 1.000000042602309),
        ("felix@novakstudio.design", "Felix Novak", 1.000000031027362),
        (
            "rachel.kim@callowayfreight.com",
            "Rachel Kim",
            1.000000015737303,
        ),
        ("ngozi.okafor@gmail.com", "Ngozi Okafor", 1.000000015720487),
        (
            "nadia@ridgelinehealth.org",
            "Nadia Petrova",
            1.000000015707953,
        ),
        (
            "sofia@romanocatering.com",
            "Sofia Romano",
            1.000000015688709,
        ),
        ("emekao@gmail.com", "Emeka Okafor", 1.000000011883651),
        ("ines@brightmoor.co", "Inês Duarte", 1.000000011876716),
        (
            "pdurand@westbrook-realty.com",
            "Paul Durand",
            1.000000011845311,
        ),
        ("ruth@brightmoor.co", "Ruth Abernathy", 1.000000008058709),
        (
            "mei.watanabe@hollisconf.org",
            "Mei Watanabe",
            1.000000008032332,
        ),
        ("zara@brightmoor.co", "Zara Ahmed", 1.000000000383844),
        ("dev@callowayfreight.com", "Dev Malhotra", 1.000000000383809),
        (
            "theo@harborlightcoffee.com",
            "Theo Lindqvist",
            1.000000000383793,
        ),
        ("yusuf@tinkerform.io", "Yusuf Karaca", 1.000000000382511),
        ("amara@quillandoak.com", "Amara Nwosu", 1.000000000382477),
        (
            "carlos@pemberton-cpa.com",
            "Carlos Mendes",
            1.000000000381225,
        ),
        ("notifications@tracklight.dev", "Tracklight", 1.32748539e-07),
        ("digest@quorum.chat", "Quorum", 4.2426698e-08),
        ("daily@morningledger.news", "Morning Ledger", 3.8586578e-08),
        ("sam@brightmoor.co", "Sam Whitaker", 3.0841929e-08),
        ("updates@parcelpoint.com", "Parcelpoint", 1.93898e-08),
        ("grace@juniperpartners.vc", "Grace Liu", 1.5550341e-08),
        ("bortiz@marlowfinch.law", "Ben Ortiz", 1.91886e-10),
        ("alerts@harborbank.com", "Harbor Bank", 0.0),
        ("ana@letters.anaruiz.com", "Ana Ruiz", 0.0),
        ("marcus@brightmoor.co", "Marcus Hill", 0.0),
        ("newsletter@freightsignals.com", "Freight Signals", 0.0),
    ];
    /// What it suggests for each query, with no thread, five at most.
    const MOCK_SUGGESTIONS: &[(&str, &[&str])] = &[
        (
            "p",
            &[
                "priya@brightmoor.co",
                "nadia@ridgelinehealth.org",
                "pdurand@westbrook-realty.com",
                "updates@parcelpoint.com",
                "carlos@pemberton-cpa.com",
            ],
        ),
        ("pri", &["priya@brightmoor.co"]),
        ("da", &["daniel@brightmoor.co", "daily@morningledger.news"]),
        (
            "a",
            &[
                "ruth@brightmoor.co",
                "zara@brightmoor.co",
                "amara@quillandoak.com",
                "alerts@harborbank.com",
                "ana@letters.anaruiz.com",
            ],
        ),
        (
            "b",
            &[
                "tom@brightmoor.co",
                "bortiz@marlowfinch.law",
                "alerts@harborbank.com",
                "maya@brightmoor.co",
                "lena@brightmoor.co",
            ],
        ),
        (
            "c",
            &[
                "chloe@brightmoor.co",
                "carlos@pemberton-cpa.com",
                "rachel.kim@callowayfreight.com",
                "dev@callowayfreight.com",
            ],
        ),
        (
            "m",
            &[
                "maya@brightmoor.co",
                "chloe@brightmoor.co",
                "jessmoreno.nyc@gmail.com",
                "mei.watanabe@hollisconf.org",
                "dev@callowayfreight.com",
            ],
        ),
        (
            "j",
            &[
                "jessmoreno.nyc@gmail.com",
                "jordan@brightmoor.co",
                "grace@juniperpartners.vc",
            ],
        ),
        (
            "s",
            &[
                "sofia@romanocatering.com",
                "sam@brightmoor.co",
                "newsletter@freightsignals.com",
            ],
        ),
        (
            "t",
            &[
                "tom@brightmoor.co",
                "hiro@brightmoor.co",
                "theo@harborlightcoffee.com",
                "notifications@tracklight.dev",
                "yusuf@tinkerform.io",
            ],
        ),
        (
            "l",
            &[
                "lena@brightmoor.co",
                "liam.oconnell@fastmail.com",
                "theo@harborlightcoffee.com",
                "daily@morningledger.news",
                "grace@juniperpartners.vc",
            ],
        ),
        (
            "o",
            &[
                "maya@brightmoor.co",
                "oliver@brightmoor.co",
                "liam.oconnell@fastmail.com",
                "ngozi.okafor@gmail.com",
                "emekao@gmail.com",
            ],
        ),
        (
            "r",
            &[
                "daniel@brightmoor.co",
                "priya@brightmoor.co",
                "rachel.kim@callowayfreight.com",
                "sofia@romanocatering.com",
                "ruth@brightmoor.co",
            ],
        ),
        ("e", &["jordan@brightmoor.co", "emekao@gmail.com"]),
        (
            "n",
            &[
                "felix@novakstudio.design",
                "ngozi.okafor@gmail.com",
                "nadia@ridgelinehealth.org",
                "amara@quillandoak.com",
                "notifications@tracklight.dev",
            ],
        ),
        (
            "h",
            &[
                "hiro@brightmoor.co",
                "hweiss@gmail.com",
                "alerts@harborbank.com",
                "marcus@brightmoor.co",
                "mei.watanabe@hollisconf.org",
            ],
        ),
        (
            "g",
            &[
                "oliver@brightmoor.co",
                "grace@juniperpartners.vc",
                "jessmoreno.nyc@gmail.com",
                "hweiss@gmail.com",
                "ngozi.okafor@gmail.com",
            ],
        ),
        (
            "d",
            &[
                "daniel@brightmoor.co",
                "ines@brightmoor.co",
                "pdurand@westbrook-realty.com",
                "dev@callowayfreight.com",
                "digest@quorum.chat",
            ],
        ),
        (
            "k",
            &["rachel.kim@callowayfreight.com", "yusuf@tinkerform.io"],
        ),
        (
            "f",
            &[
                "lena@brightmoor.co",
                "felix@novakstudio.design",
                "newsletter@freightsignals.com",
                "liam.oconnell@fastmail.com",
            ],
        ),
        ("y", &["yusuf@tinkerform.io"]),
        ("z", &["zara@brightmoor.co"]),
        ("i", &["ines@brightmoor.co"]),
        (
            "brightmoor",
            &[
                "maya@brightmoor.co",
                "lena@brightmoor.co",
                "daniel@brightmoor.co",
                "tom@brightmoor.co",
                "priya@brightmoor.co",
            ],
        ),
        (
            "gmail.com",
            &[
                "jessmoreno.nyc@gmail.com",
                "hweiss@gmail.com",
                "ngozi.okafor@gmail.com",
                "emekao@gmail.com",
            ],
        ),
        (
            "callo",
            &["rachel.kim@callowayfreight.com", "dev@callowayfreight.com"],
        ),
        ("x", &[]),
        ("q", &["digest@quorum.chat", "amara@quillandoak.com"]),
        (
            "w",
            &[
                "hweiss@gmail.com",
                "mei.watanabe@hollisconf.org",
                "sam@brightmoor.co",
                "pdurand@westbrook-realty.com",
            ],
        ),
        ("u", &["updates@parcelpoint.com"]),
        ("v", &[]),
        (
            "ma",
            &[
                "maya@brightmoor.co",
                "chloe@brightmoor.co",
                "dev@callowayfreight.com",
                "marcus@brightmoor.co",
                "bortiz@marlowfinch.law",
            ],
        ),
        ("to", &["tom@brightmoor.co"]),
        (
            "pa",
            &["pdurand@westbrook-realty.com", "updates@parcelpoint.com"],
        ),
        ("de", &["dev@callowayfreight.com"]),
        (
            "ca",
            &[
                "carlos@pemberton-cpa.com",
                "rachel.kim@callowayfreight.com",
                "dev@callowayfreight.com",
            ],
        ),
        ("ro", &["sofia@romanocatering.com"]),
        (
            "li",
            &[
                "liam.oconnell@fastmail.com",
                "theo@harborlightcoffee.com",
                "grace@juniperpartners.vc",
            ],
        ),
        ("jo", &["jordan@brightmoor.co"]),
        (
            "ok",
            &[
                "maya@brightmoor.co",
                "ngozi.okafor@gmail.com",
                "emekao@gmail.com",
            ],
        ),
        ("sa", &["sam@brightmoor.co"]),
        ("he", &[]),
    ];
    /// And with the thread's participants, in To of a Reply All to `Design crit moved to 3pm`.
    const MOCK_THREAD_SUGGESTIONS: &[(&str, &[&str])] = &[
        (
            "b",
            &[
                "tom@brightmoor.co",
                "bortiz@marlowfinch.law",
                "alerts@harborbank.com",
                "maya@brightmoor.co",
                "priya@brightmoor.co",
            ],
        ),
        (
            "brightmoor",
            &[
                "maya@brightmoor.co",
                "priya@brightmoor.co",
                "tom@brightmoor.co",
                "lena@brightmoor.co",
                "daniel@brightmoor.co",
            ],
        ),
        (
            "o",
            &[
                "maya@brightmoor.co",
                "oliver@brightmoor.co",
                "liam.oconnell@fastmail.com",
                "ngozi.okafor@gmail.com",
                "emekao@gmail.com",
            ],
        ),
        (
            "m",
            &[
                "maya@brightmoor.co",
                "chloe@brightmoor.co",
                "jessmoreno.nyc@gmail.com",
                "mei.watanabe@hollisconf.org",
                "dev@callowayfreight.com",
            ],
        ),
        (
            "p",
            &[
                "priya@brightmoor.co",
                "nadia@ridgelinehealth.org",
                "pdurand@westbrook-realty.com",
                "updates@parcelpoint.com",
                "carlos@pemberton-cpa.com",
            ],
        ),
        (
            "t",
            &[
                "tom@brightmoor.co",
                "hiro@brightmoor.co",
                "theo@harborlightcoffee.com",
                "notifications@tracklight.dev",
                "yusuf@tinkerform.io",
            ],
        ),
    ];

    #[test]
    fn the_mock_book_suggests_each_query_in_rank_order() {
        let book = Index::book(
            &MOCK_BOOK
                .iter()
                .map(|(email, name, score)| contact(name, email, *score))
                .collect::<Vec<_>>(),
        );
        for (query, expected) in MOCK_SUGGESTIONS {
            assert_eq!(
                emails(&suggest(&book, &Index::default(), query)),
                *expected,
                "`{query}`"
            );
        }
        let named = |name: &str, email: &str| Address {
            name: Some(name.into()),
            email: email.into(),
        };
        // The thread's participants, you among them.
        let thread = participants(
            &[
                named("Tom Becker", "tom@brightmoor.co"),
                named("Maya Okafor", "maya@brightmoor.co"),
                named("Priya Raman", "priya@brightmoor.co"),
            ],
            &book,
        );
        for (query, expected) in MOCK_THREAD_SUGGESTIONS {
            assert_eq!(
                emails(&suggest(&book, &thread, query)),
                *expected,
                "`{query}` in the thread"
            );
        }
    }

    #[test]
    fn a_participant_is_indexed_under_a_name_as_good_as_the_books() {
        assert_eq!(better_name("Tom Becker", "Tom Becker"), Some("Tom Becker"));
        assert_eq!(
            better_name("Tom Becker", "Tom"),
            Some("Tom Becker"),
            "the other is part of it"
        );
        assert_eq!(
            better_name("Becker, Tom", "Tom Becker"),
            Some("Tom Becker"),
            "the same words: the shorter"
        );
        assert_eq!(
            better_name("Thomas B", "Tom Becker"),
            None,
            "no first word in common"
        );
        assert_eq!(better_name("", ""), Some(""));
        let book = Index::book(&[
            contact("Tom", "tom@x.co", 1.0),
            contact("Priya Raman", "priya@x.co", 1.0),
            contact("", "sam@x.co", 1.0),
        ]);
        let named = |name: &str, email: &str| Address {
            name: Some(name.into()).filter(|name: &String| !name.is_empty()),
            email: email.into(),
        };
        let thread = participants(
            &[
                named("Tom Becker", "tom@x.co"),
                named("Priya", "priya@x.co"),
                named("", "sam@x.co"),
                named("Zoe", "ZOE@y.org"),
            ],
            &book,
        );
        assert_eq!(
            thread
                .top("", 10)
                .iter()
                .map(|address| address.email.as_str())
                .collect::<Vec<_>>(),
            ["tom@x.co", "zoe@y.org"],
            "Priya's book name is better; Sam has none either way"
        );
    }
}
