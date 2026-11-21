//! The address book compose suggests recipients from: everyone in stored mail, scored so people
//! you write to outrank people who only write to you, and frequent, recent correspondents
//! outrank the rest.

use std::collections::HashMap;

use crate::db::MessageRow;
use crate::error::Result;
use crate::ids::labels::{DRAFT, SENT, SPAM};
use crate::store::Store;

/// A person compose can suggest, and how strongly.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredContact {
    /// The address, as last written.
    pub email: String,
    /// The latest name seen with it.
    pub name: Option<String>,
    /// The contact's score: above 1 for anyone you've sent to.
    pub score: f64,
}

/// How many timestamps per direction a score looks at.
const KEPT: usize = 10;

/// Timestamps of one direction (sent or received) for one address.
#[derive(Default)]
struct Direction {
    count: usize,
    /// The latest [`KEPT`], unordered.
    latest: Vec<i64>,
}

impl Direction {
    fn add(&mut self, at: i64) {
        self.count += 1;
        if self.latest.len() < KEPT {
            self.latest.push(at);
        } else if let Some(oldest) = self.latest.iter_mut().min()
            && *oldest < at
        {
            *oldest = at;
        }
    }

    /// The contact's score from the messages counted: their weights by recency, scaled by how
    /// many there were and how recent the latest is, plus 1 when you've `sent` to them.
    fn score(&self, sent: bool) -> f64 {
        let Some(&latest) = self.latest.iter().max() else {
            return 0.0;
        };
        let kept = self.count.min(KEPT) as f64;
        let weights: f64 = self.latest.iter().map(|&at| weight(at, latest)).sum();
        let mut score = self.count as f64 * weights / kept;
        // Recency: the cube of how far into 2010–2028 the latest message is.
        let (start, end) = (1_262_304_000_000_f64, 1_830_297_600_000_f64);
        score *= (((latest as f64 - start).powi(3)) / (end - start).powi(3)).max(1e-6);
        score = score.min(4_294_967_296.0) / 4_294_967_296.0;
        if sent && score > 0.0 {
            score + 1.0
        } else {
            score
        }
    }
}

/// How much a message `at` counts next to the latest one.
fn weight(at: i64, latest: i64) -> f64 {
    const HOUR: i64 = 3_600_000;
    const DAY: i64 = 24 * HOUR;
    match latest - at {
        0 => 1.0,
        gap if gap < 4 * HOUR => 100.0,
        gap if gap < DAY => 80.0,
        gap if gap < 3 * DAY => 60.0,
        gap if gap < 7 * DAY => 40.0,
        gap if gap < 30 * DAY => 20.0,
        gap if gap < 90 * DAY => 10.0,
        _ => 1.0,
    }
}

#[derive(Default)]
struct Person {
    email: String,
    name: Option<(i64, String)>,
    sent: Direction,
    received: Direction,
}

impl Store {
    /// Everyone in stored mail, scored: a message you sent credits its To, Cc, Bcc and From; one
    /// you received credits its From. Drafts and Spam don't count.
    ///
    /// # Errors
    /// On database failures.
    pub async fn address_book(&self) -> Result<Vec<ScoredContact>> {
        let mut db = self.inner.db.clone();
        let rows = MessageRow::all().exec(&mut db).await?;
        let mut people: HashMap<String, Person> = HashMap::new();
        for row in rows {
            let labels = &row.label_ids;
            if labels.iter().any(|label| label == DRAFT || label == SPAM) {
                continue;
            }
            let sent = labels.iter().any(|label| label == SENT);
            let credited: Vec<&crate::db::Mailbox> = if sent {
                row.to
                    .0
                    .iter()
                    .chain(&row.cc.0)
                    .chain(&row.bcc.0)
                    .chain(row.sender.0.as_ref())
                    .collect()
            } else {
                row.sender.0.iter().collect()
            };
            for mailbox in credited {
                let key = mailbox.email.to_lowercase();
                if key.is_empty() {
                    continue;
                }
                let person = people.entry(key).or_default();
                person.email.clone_from(&mailbox.email);
                if let Some(name) = mailbox.name.as_ref().filter(|name| !name.is_empty())
                    && person
                        .name
                        .as_ref()
                        .is_none_or(|(at, _)| *at <= row.internal_date)
                {
                    person.name = Some((row.internal_date, name.clone()));
                }
                if sent {
                    person.sent.add(row.internal_date);
                } else {
                    person.received.add(row.internal_date);
                }
            }
        }
        Ok(people
            .into_values()
            .map(|person| ScoredContact {
                score: person.sent.score(true) + person.received.score(false),
                email: person.email,
                name: person.name.map(|(_, name)| name),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400_000;
    const NOV_2026: i64 = 1_794_960_000_000;

    #[test]
    fn writing_to_someone_outranks_being_written_to() {
        let mut wrote = Direction::default();
        wrote.add(NOV_2026 - 30 * DAY);
        let mut heard = Direction::default();
        for day in 0..20 {
            heard.add(NOV_2026 - day * DAY);
        }
        assert!(wrote.score(true) > 1.0);
        assert!(heard.score(false) < 1.0);
        assert!(wrote.score(true) > heard.score(false));
    }

    #[test]
    fn frequent_recent_mail_scores_higher() {
        let mut busy = Direction::default();
        for hour in 0..5 {
            busy.add(NOV_2026 - hour * 3_600_000);
        }
        let mut quiet = Direction::default();
        quiet.add(NOV_2026 - 200 * DAY);
        assert!(busy.score(false) > quiet.score(false));
    }
}
