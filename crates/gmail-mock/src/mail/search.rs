//! Gmail's search language (`q`): parsing and evaluation against stored messages.
//!
//! Like Gmail, parsing never fails: unknown operators are searched as text and unresolvable
//! values (an unknown label, a bad date) match nothing.

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};

use super::StoredMessage;
use super::mime::tokenize;

/// What a query needs to know about the mailbox to resolve names and relative dates.
pub struct Context<'a> {
    /// Every label, for resolving `label:`/`in:` names.
    pub labels: &'a [gmail::Label],
    /// The account's address, which `me` stands for.
    pub account: &'a str,
    /// Current time, for `older_than:`/`newer_than:`.
    pub now: Timestamp,
    /// The account's time zone, in which `after:2026/11/01` is midnight.
    pub tz: &'a TimeZone,
}

/// A compiled query.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    root: Expr<Pred>,
    /// Whether the query names `SPAM`, `TRASH` or `in:anywhere`, which lifts the default
    /// exclusion of spam and trash.
    pub mentions_spam_trash: bool,
}

impl Query {
    /// Parses and resolves `q`.
    pub fn compile(q: &str, context: &Context<'_>) -> Self {
        let tokens = lex(q);
        let mut parser = Parser { tokens, at: 0 };
        let ast = parser.parse_or();
        let mut mentions_spam_trash = false;
        let root = ast.map(&mut |term| resolve(term, context, &mut mentions_spam_trash));
        Self {
            root,
            mentions_spam_trash,
        }
    }

    /// Whether `message` matches.
    pub fn matches(&self, message: &StoredMessage) -> bool {
        self.root.eval(&|pred| pred.matches(message))
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Expr<T> {
    And(Vec<Expr<T>>),
    Or(Vec<Expr<T>>),
    Not(Box<Expr<T>>),
    Leaf(T),
}

impl<T> Expr<T> {
    fn map<U>(self, f: &mut impl FnMut(T) -> Expr<U>) -> Expr<U> {
        match self {
            Self::And(items) => Expr::And(items.into_iter().map(|item| item.map(f)).collect()),
            Self::Or(items) => Expr::Or(items.into_iter().map(|item| item.map(f)).collect()),
            Self::Not(item) => Expr::Not(Box::new(item.map(f))),
            Self::Leaf(leaf) => f(leaf),
        }
    }

    fn eval(&self, leaf: &impl Fn(&T) -> bool) -> bool {
        match self {
            Self::And(items) => items.iter().all(|item| item.eval(leaf)),
            Self::Or(items) => items.iter().any(|item| item.eval(leaf)),
            Self::Not(item) => !item.eval(leaf),
            Self::Leaf(value) => leaf(value),
        }
    }
}

/// A search term before resolution: an optional operator and its value.
#[derive(Debug, Clone, PartialEq)]
struct Term {
    op: Option<Op>,
    value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    From,
    To,
    Cc,
    Bcc,
    Subject,
    Label,
    In,
    Is,
    Has,
    Category,
    Filename,
    List,
    DeliveredTo,
    Rfc822MsgId,
    After,
    Before,
    OlderThan,
    NewerThan,
    Larger,
    Smaller,
}

impl Op {
    fn parse(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "from" => Self::From,
            "to" => Self::To,
            "cc" => Self::Cc,
            "bcc" => Self::Bcc,
            "subject" => Self::Subject,
            "label" | "l" => Self::Label,
            "in" => Self::In,
            "is" => Self::Is,
            "has" => Self::Has,
            "category" => Self::Category,
            "filename" => Self::Filename,
            "list" => Self::List,
            "deliveredto" => Self::DeliveredTo,
            "rfc822msgid" => Self::Rfc822MsgId,
            "after" | "newer" => Self::After,
            "before" | "older" => Self::Before,
            "older_than" => Self::OlderThan,
            "newer_than" => Self::NewerThan,
            "larger" | "size" => Self::Larger,
            "smaller" => Self::Smaller,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Open,
    Close,
    OpenBrace,
    CloseBrace,
    Minus,
    Or,
    And,
    Word(String),
    Quoted(String),
}

fn lex(q: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = q.chars().peekable();
    // A `-` only negates at the start of a term.
    let mut at_term_start = true;
    while let Some(&ch) = chars.peek() {
        match ch {
            ch if ch.is_whitespace() || ch == ',' => {
                chars.next();
                at_term_start = true;
                continue;
            }
            '(' | ')' | '{' | '}' => {
                chars.next();
                tokens.push(match ch {
                    '(' => Token::Open,
                    ')' => Token::Close,
                    '{' => Token::OpenBrace,
                    _ => Token::CloseBrace,
                });
                at_term_start = true;
                continue;
            }
            '"' => {
                chars.next();
                let phrase: String = chars.by_ref().take_while(|&ch| ch != '"').collect();
                tokens.push(Token::Quoted(phrase));
            }
            '-' if at_term_start => {
                chars.next();
                tokens.push(Token::Minus);
                continue;
            }
            _ => {
                let mut word = String::new();
                while let Some(&ch) = chars.peek() {
                    if ch.is_whitespace() || matches!(ch, '(' | ')' | '{' | '}' | '"' | ',') {
                        break;
                    }
                    word.push(ch);
                    chars.next();
                    // `op:` ends the word so a quoted or grouped value can follow.
                    if ch == ':' && Op::parse(word.trim_end_matches(':')).is_some() {
                        break;
                    }
                }
                tokens.push(match word.as_str() {
                    "OR" => Token::Or,
                    "AND" => Token::And,
                    _ => Token::Word(word),
                });
            }
        }
        at_term_start = false;
    }
    tokens
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.at).cloned();
        self.at += 1;
        token
    }

    fn parse_or(&mut self) -> Expr<Term> {
        let mut items = vec![self.parse_and()];
        while self.peek() == Some(&Token::Or) {
            self.next();
            items.push(self.parse_and());
        }
        flatten(items, Expr::Or)
    }

    fn parse_and(&mut self) -> Expr<Term> {
        let mut items = Vec::new();
        loop {
            match self.peek() {
                None | Some(Token::Close | Token::CloseBrace | Token::Or) => break,
                Some(Token::And) => {
                    self.next();
                }
                Some(_) => items.extend(self.parse_unary()),
            }
        }
        flatten(items, Expr::And)
    }

    fn parse_unary(&mut self) -> Option<Expr<Term>> {
        if self.peek() == Some(&Token::Minus) {
            self.next();
            return self.parse_unary().map(|expr| Expr::Not(Box::new(expr)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Option<Expr<Term>> {
        match self.next()? {
            Token::Open => {
                let expr = self.parse_or();
                if self.peek() == Some(&Token::Close) {
                    self.next();
                }
                Some(expr)
            }
            // `{a b c}` is `a OR b OR c`.
            Token::OpenBrace => {
                let mut items = Vec::new();
                loop {
                    match self.peek() {
                        None => break,
                        Some(Token::CloseBrace) => {
                            self.next();
                            break;
                        }
                        Some(Token::Or | Token::And | Token::Close) => {
                            self.next();
                        }
                        Some(_) => items.extend(self.parse_unary()),
                    }
                }
                Some(flatten(items, Expr::Or))
            }
            Token::Quoted(value) => Some(Expr::Leaf(Term { op: None, value })),
            Token::Word(word) => {
                let word = word.strip_prefix('+').unwrap_or(&word).to_owned();
                let Some((name, value)) = word.split_once(':') else {
                    return Some(Expr::Leaf(Term {
                        op: None,
                        value: word,
                    }));
                };
                let Some(op) = Op::parse(name) else {
                    return Some(Expr::Leaf(Term {
                        op: None,
                        value: word,
                    }));
                };
                if !value.is_empty() {
                    return Some(Expr::Leaf(Term {
                        op: Some(op),
                        value: value.to_owned(),
                    }));
                }
                // `from:"a b"`, `from:(a OR b)`, `to:{a b}`: the operator applies to each term.
                let value = self.parse_primary()?;
                Some(value.map(&mut |term: Term| {
                    Expr::Leaf(Term {
                        op: term.op.or(Some(op)),
                        value: term.value,
                    })
                }))
            }
            // Stray operators and closers are ignored, as Gmail does.
            Token::Minus | Token::Or | Token::And | Token::Close | Token::CloseBrace => None,
        }
    }
}

fn flatten(mut items: Vec<Expr<Term>>, wrap: fn(Vec<Expr<Term>>) -> Expr<Term>) -> Expr<Term> {
    if items.len() == 1 {
        items.remove(0)
    } else {
        wrap(items)
    }
}

/// A resolved, directly checkable condition.
#[derive(Debug, Clone, PartialEq)]
enum Pred {
    Always,
    Never,
    /// Phrase anywhere searchable.
    Text(Vec<String>),
    /// Phrase in particular fields.
    Field(&'static [Field], Vec<String>),
    Label(String),
    HasAttachment,
    HasUserLabels,
    /// `internalDate >= ms`.
    After(i64),
    /// `internalDate < ms`.
    Before(i64),
    Larger(usize),
    Smaller(usize),
    MessageId(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    From,
    To,
    Cc,
    Bcc,
    Subject,
    Body,
    Filename,
    List,
    DeliveredTo,
}

const ANY_TEXT: &[Field] = &[
    Field::From,
    Field::To,
    Field::Cc,
    Field::Subject,
    Field::Body,
    Field::Filename,
];

impl Pred {
    fn matches(&self, message: &StoredMessage) -> bool {
        let fields = &message.mime.search;
        let field = |field: Field| -> &[String] {
            match field {
                Field::From => &fields.from,
                Field::To => &fields.to,
                Field::Cc => &fields.cc,
                Field::Bcc => &fields.bcc,
                Field::Subject => &fields.subject,
                Field::Body => &fields.body,
                Field::Filename => &fields.filenames,
                Field::List => &fields.list,
                Field::DeliveredTo => &fields.delivered_to,
            }
        };
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Text(phrase) => ANY_TEXT
                .iter()
                .any(|&name| contains_phrase(field(name), phrase)),
            Self::Field(names, phrase) => names
                .iter()
                .any(|&name| contains_phrase(field(name), phrase)),
            Self::Label(id) => message.labels.contains(id),
            Self::HasAttachment => fields.has_attachment,
            Self::HasUserLabels => message.labels.iter().any(|id| id.starts_with("Label_")),
            Self::After(ms) => message.internal_date >= *ms,
            Self::Before(ms) => message.internal_date < *ms,
            Self::Larger(bytes) => message.raw.len() > *bytes,
            Self::Smaller(bytes) => message.raw.len() < *bytes,
            Self::MessageId(id) => message.mime.rfc822_id.as_deref() == Some(id),
        }
    }
}

fn contains_phrase(haystack: &[String], needle: &[String]) -> bool {
    needle.is_empty()
        || haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn resolve(term: Term, context: &Context<'_>, mentions_spam_trash: &mut bool) -> Expr<Pred> {
    let value = term.value.trim();
    let lower = value.to_lowercase();
    let address = |fields: &'static [Field]| {
        let value = if lower == "me" {
            context.account
        } else {
            value
        };
        Pred::Field(fields, tokenize(value))
    };
    let mut label = |id: &str| {
        if id == "SPAM" || id == "TRASH" {
            *mentions_spam_trash = true;
        }
        Pred::Label(id.to_owned())
    };
    let pred = match term.op {
        None => Pred::Text(tokenize(value)),
        Some(Op::From) => address(&[Field::From]),
        // Gmail's `to:` covers every recipient field.
        Some(Op::To) => address(&[Field::To, Field::Cc, Field::Bcc]),
        Some(Op::Cc) => address(&[Field::Cc]),
        Some(Op::Bcc) => address(&[Field::Bcc]),
        Some(Op::Subject) => Pred::Field(&[Field::Subject], tokenize(value)),
        Some(Op::Filename) => Pred::Field(&[Field::Filename], tokenize(value)),
        Some(Op::List) => Pred::Field(&[Field::List], tokenize(value)),
        Some(Op::DeliveredTo) => Pred::Field(&[Field::DeliveredTo], tokenize(value)),
        Some(Op::Rfc822MsgId) => Pred::MessageId(value.trim_matches(['<', '>']).to_owned()),
        Some(Op::Is) => match lower.as_str() {
            "unread" => label("UNREAD"),
            "read" => return Expr::Not(Box::new(Expr::Leaf(label("UNREAD")))),
            "starred" => label("STARRED"),
            "important" => label("IMPORTANT"),
            "chat" => label("CHAT"),
            "sent" => label("SENT"),
            "draft" => label("DRAFT"),
            "spam" => label("SPAM"),
            "trash" => label("TRASH"),
            // Snoozing and muting aren't exposed through the API, so nothing is snoozed here.
            _ => Pred::Never,
        },
        Some(Op::In) => match lower.as_str() {
            "anywhere" | "all" => {
                *mentions_spam_trash = true;
                Pred::Always
            }
            "drafts" => label("DRAFT"),
            "bin" => label("TRASH"),
            "chats" => label("CHAT"),
            "snoozed" | "scheduled" => Pred::Never,
            _ => label_named(value, context).map_or(Pred::Never, |id| label(&id)),
        },
        Some(Op::Label) => label_named(value, context).map_or(Pred::Never, |id| label(&id)),
        Some(Op::Category) => match lower.as_str() {
            "primary" | "personal" => label("CATEGORY_PERSONAL"),
            "social" => label("CATEGORY_SOCIAL"),
            "promotions" => label("CATEGORY_PROMOTIONS"),
            "updates" => label("CATEGORY_UPDATES"),
            "forums" => label("CATEGORY_FORUMS"),
            _ => Pred::Never,
        },
        Some(Op::Has) => match lower.as_str() {
            "attachment" => Pred::HasAttachment,
            "userlabels" => Pred::HasUserLabels,
            "nouserlabels" => return Expr::Not(Box::new(Expr::Leaf(Pred::HasUserLabels))),
            star if star.ends_with("-star") => label("STARRED"),
            _ => Pred::Never,
        },
        Some(Op::After) => date(value, context).map_or(Pred::Never, Pred::After),
        Some(Op::Before) => date(value, context).map_or(Pred::Never, Pred::Before),
        Some(Op::OlderThan) => relative(value, context).map_or(Pred::Never, Pred::Before),
        Some(Op::NewerThan) => relative(value, context).map_or(Pred::Never, Pred::After),
        Some(Op::Larger) => size(value).map_or(Pred::Never, Pred::Larger),
        Some(Op::Smaller) => size(value).map_or(Pred::Never, Pred::Smaller),
    };
    Expr::Leaf(pred)
}

/// Finds a label by ID or by name. Gmail lets names be written lowercase with `-` for spaces
/// and slashes (`label:projects-exo`).
fn label_named(value: &str, context: &Context<'_>) -> Option<String> {
    let normalize = |name: &str| name.to_lowercase().replace([' ', '/', '_'], "-");
    let wanted = normalize(value);
    context.labels.iter().find_map(|label| {
        let id = label.id.as_deref()?;
        let name = label.name.as_deref().unwrap_or(id);
        (id == value || normalize(name) == wanted || normalize(id) == wanted).then(|| id.to_owned())
    })
}

/// `after:`/`before:` values: epoch seconds, `YYYY/MM/DD`, `YYYY-MM-DD` or `MM/DD/YYYY`.
fn date(value: &str, context: &Context<'_>) -> Option<i64> {
    if value.chars().all(|ch| ch.is_ascii_digit()) {
        return value.parse::<i64>().ok().map(|seconds| seconds * 1000);
    }
    let parts: Vec<i32> = value
        .split(['/', '-'])
        .map(|part| part.parse().ok())
        .collect::<Option<_>>()?;
    let (year, month, day) = match parts[..] {
        [year, month, day] if year > 999 => (year, month, day),
        [month, day, year] if year > 999 => (year, month, day),
        _ => return None,
    };
    let date = Date::new(
        i16::try_from(year).ok()?,
        i8::try_from(month).ok()?,
        i8::try_from(day).ok()?,
    )
    .ok()?;
    let midnight = date.to_zoned(context.tz.clone()).ok()?;
    Some(midnight.timestamp().as_millisecond())
}

/// `older_than:`/`newer_than:` values like `3d`, `2m`, `1y`, as a cutoff in epoch ms.
fn relative(value: &str, context: &Context<'_>) -> Option<i64> {
    let unit = value.chars().last()?;
    let amount: i64 = value[..value.len() - unit.len_utf8()].parse().ok()?;
    let span = match unit.to_ascii_lowercase() {
        'd' => amount.days(),
        'm' => amount.months(),
        'y' => amount.years(),
        _ => return None,
    };
    let now = context.now.to_zoned(context.tz.clone());
    Some(now.checked_sub(span).ok()?.timestamp().as_millisecond())
}

/// `larger:`/`smaller:` values: bytes, or with a `K`/`M` suffix.
fn size(value: &str) -> Option<usize> {
    let (digits, multiplier) = match value.chars().last()?.to_ascii_lowercase() {
        'k' => (&value[..value.len() - 1], 1024),
        'm' => (&value[..value.len() - 1], 1024 * 1024),
        _ => (value, 1),
    };
    digits
        .parse::<usize>()
        .ok()
        .map(|amount| amount * multiplier)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ast(q: &str) -> Expr<Term> {
        Parser {
            tokens: lex(q),
            at: 0,
        }
        .parse_or()
    }

    fn leaf(op: Option<Op>, value: &str) -> Expr<Term> {
        Expr::Leaf(Term {
            op,
            value: value.into(),
        })
    }

    #[test]
    fn parses_split_queries() {
        assert_eq!(
            ast(r#"-in:DRAFT -in:CHAT in:inbox from:"acme.com" to:{ava@kestrel.co}"#),
            Expr::And(vec![
                Expr::Not(Box::new(leaf(Some(Op::In), "DRAFT"))),
                Expr::Not(Box::new(leaf(Some(Op::In), "CHAT"))),
                leaf(Some(Op::In), "inbox"),
                leaf(Some(Op::From), "acme.com"),
                leaf(Some(Op::To), "ava@kestrel.co"),
            ])
        );
        assert_eq!(
            ast("(is:UNREAD OR in:[Superhuman]/ru)"),
            Expr::Or(vec![
                leaf(Some(Op::Is), "UNREAD"),
                leaf(Some(Op::In), "[Superhuman]/ru"),
            ])
        );
    }

    #[test]
    fn operator_applies_to_groups() {
        assert_eq!(
            ast("from:{a@x.com, b@y.com} -(subject:lunch OR dinner)"),
            Expr::And(vec![
                Expr::Or(vec![
                    leaf(Some(Op::From), "a@x.com"),
                    leaf(Some(Op::From), "b@y.com"),
                ]),
                Expr::Not(Box::new(Expr::Or(vec![
                    leaf(Some(Op::Subject), "lunch"),
                    leaf(None, "dinner"),
                ]))),
            ])
        );
        assert_eq!(
            ast("(from:a OR to:a) AND -in:DRAFT"),
            Expr::And(vec![
                Expr::Or(vec![leaf(Some(Op::From), "a"), leaf(Some(Op::To), "a")]),
                Expr::Not(Box::new(leaf(Some(Op::In), "DRAFT"))),
            ])
        );
    }

    #[test]
    fn unknown_operators_and_hyphenated_words_are_text() {
        assert_eq!(
            ast("foo:bar well-known 10:30"),
            Expr::And(vec![
                leaf(None, "foo:bar"),
                leaf(None, "well-known"),
                leaf(None, "10:30"),
            ])
        );
    }

    #[test]
    fn resolves_dates_in_the_account_time_zone() {
        let tz = TimeZone::get("America/New_York").unwrap();
        let context = Context {
            labels: &[],
            account: "ava@kestrel.co",
            now: "2026-11-17T14:00:00Z".parse().unwrap(),
            tz: &tz,
        };
        assert_eq!(date("2026/11/17", &context), Some(1_794_891_600_000));
        assert_eq!(date("11/17/2026", &context), Some(1_794_891_600_000));
        assert_eq!(date("1794891600", &context), Some(1_794_891_600_000));
        assert_eq!(date("yesterday", &context), None);
        assert_eq!(relative("1d", &context), Some(1_794_837_600_000));
        assert_eq!(size("2K"), Some(2048));
    }
}
