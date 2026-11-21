//! The grammar: rules are runs of reading kinds, each optional (`?`), completed when the input
//! stops right before it (`>`), or never completed (`-`). Every way a rule can take the words'
//! readings is a parse; where the input stops, the next element is completed with each of its
//! readings, which is how `in 2` offers minutes, hours and days.

use std::sync::OnceLock;

use super::words::{Kind, Tag, Token, Vocabulary};

/// How a rule's parses become dates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Interpreter {
    /// From now by an amount: `in 3 days`, `2h`.
    Relative,
    /// The next such day or period: `tomorrow`, `fri`, `next month`, a time alone.
    Subsequent,
    /// A date as written: `aug 7`, `8/7`, `the 25th`.
    Absolute,
}

/// A rule element.
#[derive(Clone, Copy, Debug)]
struct Element {
    kind: Kind,
    optional: bool,
    /// `>`: when the input stops right after this element, complete what follows it.
    force: bool,
    /// `-`: never completed.
    never: bool,
    tag: Option<Tag>,
}

pub(super) struct Rule {
    elements: Vec<Element>,
    pub interpreter: Interpreter,
    pub confidence: f64,
    /// The rule has an `am?` right before an `in?`.
    pub am_then_in: bool,
}

/// A base rule: the date, which [`rules`] arranges the time and zone around (in the calendar,
/// durations, spans and all day too).
struct Base {
    rule: &'static str,
    interpreter: Interpreter,
    confidence: f64,
    /// Reminders: no time may come with it.
    no_time: bool,
    /// Calendar: the time and duration may also come before it.
    expanded: bool,
}

const fn base(rule: &'static str, interpreter: Interpreter, confidence: f64) -> Base {
    Base {
        rule,
        interpreter,
        confidence,
        no_time: false,
        expanded: false,
    }
}

/// The rules for reminders and Jump to Date (`month_first`: the locale writes the month before
/// the day) or for the calendar.
pub(super) fn rules(calendar: bool, month_first: bool) -> &'static [Rule] {
    static RULES: [OnceLock<Vec<Rule>>; 4] = [
        OnceLock::new(),
        OnceLock::new(),
        OnceLock::new(),
        OnceLock::new(),
    ];
    RULES[usize::from(calendar) * 2 + usize::from(month_first)].get_or_init(|| {
        if calendar {
            calendar_rules(month_first)
        } else {
            reminder_rules(month_first)
        }
    })
}

fn reminder_rules(month_first: bool) -> Vec<Rule> {
    use Interpreter::*;
    let no_time = |base: Base| Base {
        no_time: true,
        ..base
    };
    let bases = [
        base(
            "in? quantity unit and? quantity?> unit? time?",
            Relative,
            0.9,
        ),
        base("in? quantity unit and?> a> unit", Relative, 0.9),
        base("in? a? unit and? quantity unit", Relative, 0.9),
        base("in? a? unit and?> a?> unit?", Relative, 0.9),
        base("on? next? dayname daylight?", Subsequent, 0.95),
        base("tomorrow daylight?", Subsequent, 1.2),
        base("this?> daylight?", Subsequent, 0.3),
        base("today>", Subsequent, 1.0),
        base("tonight", Subsequent, 0.8),
        no_time(base("at?> hour> am?> today", Subsequent, 1.0)),
        no_time(base("next monthname", Subsequent, 0.4)),
        no_time(base("next quartername", Subsequent, 0.4)),
        base("someday", Subsequent, 0.5),
        no_time(base("next timespan", Subsequent, 0.9)),
        no_time(base("in? monthname? year", Subsequent, 0.1)),
        no_time(base("in? quartername> year?", Subsequent, 0.1)),
        base(
            "on? dayname? the? day th? slash? of? month slash?> year?",
            Absolute,
            if month_first { 0.05 } else { 0.5 },
        ),
        base(
            "on? dayname? month slash? day th? slash?> year?",
            Absolute,
            if month_first { 0.5 } else { 0.05 },
        ),
        base("on? year slash? month slash? day th?", Absolute, 0.01),
    ];
    let mut rules = Vec::new();
    for base in bases {
        if base.no_time {
            rules.push(rule(base.rule.to_owned(), &base));
        } else {
            rules.push(rule(
                format!("at?> hour colon?> minute? am? {}", base.rule),
                &base,
            ));
            rules.push(rule(
                format!("{} at?> hour? colon?> minute? am? in? timezone?", base.rule),
                &base,
            ));
        }
    }
    rules
}

fn calendar_rules(month_first: bool) -> Vec<Rule> {
    use Interpreter::*;
    let expanded = |base: Base| Base {
        expanded: true,
        ..base
    };
    let bases = [
        base(
            "in quantity unit and? quantity?> unit? time?",
            Relative,
            0.9,
        ),
        base("in quantity unit and?> a> unit", Relative, 0.9),
        base("in a? unit and? quantity unit", Relative, 0.9),
        base("in a? unit and?> a?> unit?", Relative, 0.9),
        base("on? next? dayname daylight?", Subsequent, 0.95),
        base("this?> daylight", Subsequent, 0.3),
        expanded(base("today>", Subsequent, 1.0)),
        expanded(base("tomorrow daylight?", Subsequent, 1.2)),
        base("tonight", Subsequent, 0.8),
        base(
            "on? dayname? the? day th? slash? of? month slash?> year?",
            Absolute,
            if month_first { 0.05 } else { 0.25 },
        ),
        base(
            "on? dayname? month slash? day th? slash? year?",
            Absolute,
            if month_first { 0.25 } else { 0.05 },
        ),
        base("on? year slash? month slash? day th?", Absolute, 0.01),
    ];
    let time = "at? hour colon? minute? am?";
    let maybe_time = "at? hour? colon? minute? am?";
    let duration =
        "for? quantity~duration duration_unit and?> quantity?>~duration_and duration_unit?~and";
    let maybe_duration =
        "for? quantity?>~duration duration_unit? and?> quantity?>~duration_and duration_unit?~and";
    let span = "span- hour- colon?> minute? am?";
    let zone = "in? timezone?";
    let all_day = "all- dayword?";
    let mut rules = Vec::new();
    for base in bases {
        let date = base.rule;
        let mut texts = vec![
            format!("{date} {maybe_duration} {maybe_time} {zone}"),
            format!("{date} {maybe_time} {maybe_duration} {zone}"),
            format!("{date} {time} {span} {zone}"),
            format!("{date} {time} {zone} {span}"),
            format!("{date} {maybe_time} {zone} {maybe_duration}"),
            format!("{time} {maybe_duration} {date} {zone}"),
            format!("{time} {span} {date} {zone}"),
            format!("{time} {date} {duration} {zone}"),
            format!("{time} {date} {zone} {duration}"),
            format!("{duration} {maybe_time} {date} {zone}"),
            format!("{duration} {date} {time} {zone}"),
        ];
        let days = [
            format!("{date} {maybe_duration}"),
            format!("{date} {span}"),
            format!("{maybe_duration} {date}"),
            format!("{span} {date}"),
            format!("{date} {duration}"),
            format!("{time} {date} {duration}"),
            format!("{duration} {date}"),
        ];
        texts.extend(days.iter().map(|text| format!("{text} {all_day}")));
        texts.extend(days.iter().map(|text| format!("{all_day} {text}")));
        if base.expanded {
            texts.push(format!("{time} {maybe_duration} {zone} {date}"));
            texts.push(format!("{time} {zone} {maybe_duration} {date}"));
        }
        rules.extend(texts.into_iter().map(|text| rule(text, &base)));
    }
    rules
}

fn rule(text: String, base: &Base) -> Rule {
    let elements = text.split(' ').map(element).collect();
    Rule {
        elements,
        interpreter: base.interpreter,
        confidence: base.confidence,
        am_then_in: text.contains("am? in?"),
    }
}

/// `type?->~tag`.
fn element(text: &str) -> Element {
    let (spec, tag) = text
        .split_once('~')
        .map_or((text, None), |(spec, tag)| (spec, Some(tag)));
    let name = spec.trim_end_matches(['?', '-', '>']);
    let flags = &spec[name.len()..];
    Element {
        kind: Kind::from_name(name).unwrap_or_else(|| panic!("the grammar names `{name}`")),
        optional: flags.contains('?'),
        force: flags.contains('>'),
        never: flags.contains('-'),
        tag: tag.map(|tag| match tag {
            "duration" => Tag::Duration,
            "duration_and" => Tag::DurationAnd,
            _ => Tag::And,
        }),
    }
}

/// Every way `rule` reads `words`, in this order: an optional element skipped before it's matched,
/// then each of the word's readings in turn.
pub(super) fn parses(
    rule: &Rule,
    words: &[Vec<Token>],
    vocabulary: &Vocabulary,
) -> Vec<Vec<Token>> {
    let mut out = Vec::new();
    walk(
        &rule.elements,
        words,
        false,
        vocabulary,
        &mut Vec::new(),
        &mut out,
    );
    out
}

fn walk(
    elements: &[Element],
    words: &[Vec<Token>],
    force: bool,
    vocabulary: &Vocabulary,
    path: &mut Vec<Token>,
    out: &mut Vec<Vec<Token>>,
) {
    let Some(word) = words.first() else {
        for guess in complete(elements, force, vocabulary) {
            out.push(path.iter().cloned().chain(guess).collect());
        }
        return;
    };
    let Some(element) = elements.first() else {
        return;
    };
    if element.optional {
        walk(&elements[1..], words, element.force, vocabulary, path, out);
    }
    for reading in word.iter().filter(|reading| reading.kind == element.kind) {
        path.push(Token {
            optional: element.optional,
            tag: element.tag,
            ..reading.clone()
        });
        walk(
            &elements[1..],
            &words[1..],
            element.force,
            vocabulary,
            path,
            out,
        );
        path.pop();
    }
}

/// What the input could go on with where it stops: nothing, or each reading of the one element it
/// completes. `force` is the last matched element's `>`, which skips the `>` elements after it and
/// completes the next even when it's optional.
fn complete(mut elements: &[Element], force: bool, vocabulary: &Vocabulary) -> Vec<Option<Token>> {
    if force {
        while elements.first().is_some_and(|element| element.force) {
            elements = &elements[1..];
        }
    }
    let Some(element) = elements.first() else {
        return vec![None];
    };
    if element.never {
        return vec![None];
    }
    if element.optional && !force {
        return complete(&elements[1..], false, vocabulary);
    }
    // The calendar doesn't guess when more than one required element would be left.
    if vocabulary.calendar && elements.iter().filter(|element| !element.optional).count() > 2 {
        return Vec::new();
    }
    vocabulary
        .readings(element.kind, "", 0.0)
        .into_iter()
        .map(|reading| {
            Some(Token {
                kind: element.kind,
                tag: element.tag,
                guessed: !force,
                suggested: true,
                confidence: None,
                optional: false,
                ..reading
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bases_expand_into_every_arrangement() {
        // Reminders: 13 bases with a time before or after them, 6 without; the calendar's 12
        // bases with 25 arrangements each, `today` and `tomorrow` 2 more.
        assert_eq!(rules(false, true).len(), 32);
        assert_eq!(rules(true, true).len(), 304);
        let first = &rules(true, true)[0];
        assert_eq!(
            (first.elements.len(), first.interpreter, first.am_then_in),
            (20, Interpreter::Relative, true)
        );
    }
}
