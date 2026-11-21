//! Finding what to hide behind the `…`: the quoted earlier message and signatures.

use std::sync::LazyLock;

use regex::Regex;

use crate::{
    content::{Content, Direction, REPLACED, Range},
    dom::{self, NodeRef},
    links::UNSUBSCRIBE_CLASS,
};

const QUOTED_CLASS: &str = "sh-quoted-content";
const SIGNATURE_CLASS: &str = "sh-signature";

/// Elements mail clients put signatures in.
fn is_signature_element(node: &NodeRef) -> bool {
    const IDS: &[&str] = &[
        "psignature",
        "Signature",
        "MAC_OUTLOOK_SIGNATURE",
        "cm_footer",
        "ms-outlook-mobile-signature",
        "AppleMailSignature",
    ];
    const CLASSES: &[&str] = &[
        "acompli_signature",
        "signature",
        "bloop_sign",
        "mailbox_signature",
        "messageSignatureSection",
        "gmail_signature",
        "front-signature",
        "moz-signature",
        "mixmax_signature",
        "superhuman-signature",
        "superhuman-signature-desktop",
    ];
    let id = dom::attr(node, "id").unwrap_or_default();
    IDS.contains(&id.as_str())
        || id.starts_with("WISESTAMP_SIG")
        || CLASSES.iter().any(|class| dom::has_class(node, class))
        || dom::attr(node, "name").as_deref() == Some("messageSignatureSection")
}

/// "On … wrote:" and its translations.
static REPLY_INTROS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r" *\-\-\-(\-)? Reply \-\-\-(\-)? *$",
        r"\s*On .+, .+ *wrote *: *$",
        r"\s*Le .+, .+ a écrit *: *$",
        r"\s*El .+, .+, .+ escribió *: *$",
        r"\s*Am (Do\.,)?.+ um .+ schrieb .+ *: *$",
        r"\s*W dniu .+, .+ napisał(\(a\))? *: *$",
        r"\s*Op .+ om .+ schreef .+ <.+> *$",
        r"^\s*.+ escreveu em .+, .+ às .+ *: *$",
        r"^\s*.+ 于.+写道： *$",
        r"^\s*.+ skrev .+ *: *$",
    ]
    .iter()
    .map(|pattern| Regex::new(pattern).expect("valid regex"))
    .collect()
});
/// A "Forwarded Message" separator line.
pub(crate) static FORWARDED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?im)^[\s-]*(Forwarded Message)[\s-]*$").expect("valid regex"));
static FROM_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*(?:From|Von|De|Da|Van|Od|Från|发件人)\s*:").expect("valid regex")
});
static OTHER_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*(?:Sent|Date|To|Cc|Bcc|Gesendet|Enviado(?:\s+el)?|Envoyé|Verzonden|Wysłano|Skickat|Datum|Data|An|Para|À|Aan|Do|Till|发送时间|收件人)\s*:")
        .expect("valid regex")
});
static SUBJECT_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^\s*(?:Subject|Betreff|Asunto|Objet|Oggetto|Assunto|Onderwerp|Temat|Ämne|主题)\s*:",
    )
    .expect("valid regex")
});
static NO_CONTENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[_\p{Cc}\p{Pc}\p{Pd}\p{Sm}*\p{Z}]*$").expect("valid regex"));
static VALEDICTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("(?im)^[\u{2327} \t]*(?:all my best|all the best|best|best regards|cheers|kind regards|kindly|many thanks|onwards|regards|sincerely|thanks|thanks and regards|thanks so much|thanks & regards|warmest aloha|warmly|bien cordialement|obrigado|saludo|saludos),?$")
        .expect("valid regex")
});
static SENT_WITH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?im)^Sent (using|with|from|via) ").expect("valid regex"));
static SENT_FROM_PHONE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^(sent from my|enviado desde mi|von meinem|enviado do meu) (iphone|ipad)")
        .expect("valid regex")
});
static DISCLAIMER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)You are receiving this|please see our Privacy Notice|Do not click links or open attachments|not for everyone|intended recipient|named addressee|recipient\(s\)|environment.*print.*email|confidentiality\s+notice|/mailman/|rights reserved|Copyright ©|intended only for person\(s\)|informational purposes only")
        .expect("valid regex")
});
static SIGNATURE_DASHES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?im)^--$").expect("valid regex"));
static SEPARATOR_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^[.,/#!$%^&*;:{}=\-_`~()]+$").expect("valid regex"));
static DASH_DASH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?im)^--\s?(?:sent from|$)").expect("valid regex"));
static OTHER_SIGNATURES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"(?i)^Sent via Superhuman( Mail)?$").expect("valid regex"),
        Regex::new(r"(?i)^via Newton Mail$").expect("valid regex"),
    ]
});

/// Who sent a message, as the quote detector compares names.
#[derive(Clone, Debug, Default)]
pub(crate) struct Sender {
    pub(crate) email: String,
    /// The display name, as the `From` header has it.
    pub(crate) name: String,
}

/// The message this one replies to.
pub(crate) struct Previous<'a> {
    /// Its whole cleaned render (`Render::html`), parsed.
    pub(crate) document: &'a NodeRef,
    pub(crate) from: &'a Sender,
    pub(crate) attachment_names: &'a [String],
}

/// The quoted part found in a message.
pub(crate) struct Quoted {
    /// Top-level nodes to move behind the `…` (already marked quoted or signature).
    pub(crate) nodes: Vec<NodeRef>,
    /// The hidden span of the text, for the visible text.
    pub(crate) hidden: Option<Range>,
}

#[derive(Clone, Copy)]
struct SignatureRange {
    range: Range,
    typical: Option<Range>,
}

/// Quote and signature detection over one message body.
pub(crate) struct MessageBody<'a> {
    body: NodeRef,
    pub(crate) content: Content,
    from: &'a Sender,
    rfc822_id: &'a str,
    previous: Option<Previous<'a>>,
}

impl<'a> MessageBody<'a> {
    pub(crate) fn new(document: &NodeRef, from: &'a Sender, rfc822_id: &'a str) -> Self {
        let body = dom::body(document).unwrap_or_else(|| document.clone());
        Self {
            content: Content::new(&body),
            body,
            from,
            rfc822_id,
            previous: None,
        }
    }

    /// Splits off what goes behind the `…`: the quote of `previous` when there is one, otherwise
    /// a trailing signature.
    pub(crate) fn split(mut self, previous: Option<Previous<'a>>) -> (Quoted, Content) {
        let quoted = match previous {
            Some(previous) => {
                self.previous = Some(previous);
                self.find_quoted_portion()
            }
            None => self.find_signatures(),
        };
        (quoted, self.content)
    }

    fn sender_pattern(&self) -> Option<Regex> {
        let from = self.previous.as_ref()?.from;
        let mut alternatives = vec![regex::escape(&from.email)];
        if !from.name.is_empty() {
            alternatives.push(regex::escape(&from.name));
        }
        Regex::new(&format!("(?i)({})", alternatives.join("|"))).ok()
    }

    fn find_quoted_portion(&mut self) -> Quoted {
        let Some(previous) = &self.previous else {
            return self.find_signatures();
        };
        let previous_body =
            dom::body(previous.document).unwrap_or_else(|| previous.document.clone());
        let mut quoted_content = Content::new(&previous_body);
        if self.content.equivalent_to(&quoted_content) {
            return self.find_signatures();
        }
        let equivalences: Vec<(String, String)> = previous
            .attachment_names
            .iter()
            .map(|name| (format!("<{name}>"), REPLACED.to_string()))
            .collect();
        let footer = footer_range(&quoted_content);
        let mut search = match footer {
            Some(footer) => quoted_content.search_string(Some(Range {
                start: 0,
                end: footer.start,
            })),
            None => quoted_content.search_string(None),
        };
        let length = self.content.len();
        let mut found = self
            .content
            .last_range_containing(&search, None, &equivalences);
        let mut keep_attribution = true;
        if self.rfc822_id.contains("helpscout")
            && found.is_none_or(|f| f.end < length)
            && let Some(quote) = self
                .content
                .ranges_matching(|n| dom::is(n, "blockquote") && dom::has_class(n, "gmail_quote"))
                .first()
            && found.is_none_or(|f| quote.end > f.end)
        {
            found = Some(*quote);
        }
        if self.rfc822_id.contains("zendesk") && found.is_none_or(|f| f.end < length) {
            if let Some(quote) = self
                .content
                .ranges_matching(|n| dom::has_class(n, "zendesk_quote"))
                .first()
                && found.is_none_or(|f| quote.end > f.end)
            {
                found = Some(*quote);
                keep_attribution = false;
            }
            let comment_table = self.content.ranges_matching(|n| {
                dom::is(n, "table")
                    && dom::parent(n).is_some_and(|p| dom::has_class(&p, "zd-comment"))
            });
            if let Some(table) = comment_table.first()
                && found.is_none_or(|f| table.end > f.end)
            {
                found = Some(*table);
                keep_attribution = false;
            }
        }
        let sender = self.sender_pattern();
        let mentions_sender = |text: &str| sender.as_ref().is_some_and(|s| s.is_match(text));
        if found.is_none() {
            let last = self.content.paragraph_before(length);
            if search.ends_with(&last) {
                search.truncate(search.len() - last.len());
                let tail = self.content.extend_range_backwards(
                    Range {
                        start: length,
                        end: length,
                    },
                    &last,
                );
                found = self
                    .content
                    .last_range_containing(&search, Some(tail), &equivalences);
                let names_sender = mentions_sender(&last);
                let nodes = self.content.nodes_for_range(tail);
                let whole_rows = !nodes.iter().any(|node| {
                    dom::is(node, "tr")
                        && dom::previous_element_sibling(node).is_some_and(|previous| {
                            !nodes.iter().any(|n| std::rc::Rc::ptr_eq(n, &previous))
                        })
                });
                if found.is_none() {
                    if names_sender {
                        found = Some(Range {
                            start: length,
                            end: length,
                        });
                    } else if whole_rows {
                        found = Some(tail);
                    }
                }
            }
        }
        if let Some(blockquote) = self.find_on_wrote_blockquote_range() {
            found = Some(union(found, blockquote));
        }
        let mut separator = None;
        if let Some(header) = self.find_quote_header_range() {
            let has_found = found.is_some_and(|f| f.end > f.start);
            if has_found || mentions_sender(&header.from_line) {
                found = Some(if has_found {
                    union(found, header.range)
                } else {
                    header.range
                });
                separator = Some(header.separator_start);
            }
        }
        let Some(mut range) = found else {
            return self.find_signatures();
        };
        range = self.extend_backward_through_banners(range);
        let before = self.content.paragraph_before(range.start);
        let unextended = range;
        if keep_attribution && mentions_sender(&before) {
            range = self.content.extend_range_backwards(range, &before);
        }
        if range.start == 0 && range.end == length {
            range = unextended;
        }
        let mut quote_only = range;
        // The fallback below keeps what the loop does to `range`.
        let mut moved_before_quote = false;
        for signature in self.potential_signature_ranges() {
            if signature.range.start < range.start {
                moved_before_quote = true;
                range.start = signature.range.start;
            } else if signature.range.start >= range.end {
                let between = self
                    .content
                    .formatted_slice(range.end, signature.range.start);
                if has_no_content(&between) {
                    range.end = signature.range.end;
                }
            }
        }
        let fallback = range;
        range = if moved_before_quote {
            self.extend_backwards(range, |this, text| {
                has_no_content(text) || this.is_senders_name(text)
            })
        } else {
            self.extend_for_signature(range, Direction::Backward)
        };
        range = self.extend_for_signature(range, Direction::Forward);
        if range.start == 0 && range.end == length {
            range = fallback;
        }
        if let Some(separator) = separator
            && separator < range.start
        {
            if quote_only.start == range.start {
                quote_only.start = separator;
            }
            range.start = separator;
        }
        self.non_empty_quoted_nodes(range, quote_only)
    }

    fn find_signatures(&mut self) -> Quoted {
        let candidates = self.potential_signature_ranges();
        let length = self.content.len();
        let range = if let Some(first) = candidates.first() {
            let mut range = first.range;
            let mut typical = first.typical.is_some();
            for candidate in &candidates[1..] {
                if range.end <= candidate.range.start {
                    let between = self
                        .content
                        .formatted_slice(range.end, candidate.range.start);
                    if has_no_content(&between) {
                        range.end = candidate.range.end;
                        typical = typical || candidate.typical.is_some();
                    }
                }
            }
            range = self.extend_for_signature(range, Direction::Forward);
            if !typical {
                range = self.extend_for_signature(range, Direction::Backward);
            }
            range
        } else {
            self.extend_for_signature(
                Range {
                    start: length,
                    end: length,
                },
                Direction::Backward,
            )
        };
        if range.start != range.end && range.start != 0 && range.end == length {
            let nodes = Content::remove_empty_nodes(self.content.nodes_for_range(range));
            let classes = vec![(SIGNATURE_CLASS, QUOTED_CLASS); nodes.len()];
            return Quoted {
                nodes: mark(nodes, &classes),
                hidden: Some(range),
            };
        }
        Quoted {
            nodes: Vec::new(),
            hidden: None,
        }
    }

    fn potential_signature_ranges(&mut self) -> Vec<SignatureRange> {
        let top: Vec<NodeRef> = dom::element_children(&self.body);
        if top.len() == 3
            && top
                .iter()
                .all(|n| dom::is(n, "div") && dom::attr(n, "dir").as_deref() == Some("ltr"))
            && dom::text_content(&top[1]) == "-- "
        {
            let first = top[0].clone();
            let body = self.body.clone();
            return self
                .content
                .ranges_matching(|n| {
                    dom::is(n, "div")
                        && !std::rc::Rc::ptr_eq(n, &first)
                        && dom::parent(n).is_some_and(|p| std::rc::Rc::ptr_eq(&p, &body))
                })
                .into_iter()
                .map(|range| SignatureRange {
                    range,
                    typical: None,
                })
                .collect();
        }
        let mut ranges = self.content.ranges_matching(is_signature_element);
        ranges.sort_by_key(|range| range.start);
        let mut merged: Vec<Range> = Vec::new();
        for range in ranges {
            match merged.last_mut() {
                Some(last)
                    if last.end <= range.start
                        && has_no_content(&self.content.formatted_slice(last.end, range.start)) =>
                {
                    last.end = range.end;
                }
                _ => merged.push(range),
            }
        }
        let mut results = self.filter_signature_ranges(merged);
        if let Some(mut footer) = footer_range(&self.content) {
            footer.end = self.content.len();
            results.push(SignatureRange {
                range: footer,
                typical: None,
            });
        }
        results
    }

    fn filter_signature_ranges(&mut self, ranges: Vec<Range>) -> Vec<SignatureRange> {
        let tables = self.content.ranges_matching(|n| dom::is(n, "table"));
        let length = self.content.len();
        let mut results = Vec::new();
        for range in ranges {
            if range.start == 0 && range.end == length {
                continue;
            }
            let Some(typical) = self.check_for_typical_signature(range) else {
                continue;
            };
            if typical.start <= range.start + 3 {
                results.push(SignatureRange {
                    range: Range {
                        start: range.start.min(typical.start),
                        end: range.end,
                    },
                    typical: Some(typical),
                });
                continue;
            }
            let parent_table = tables
                .iter()
                .filter(|t| typical.start >= t.start && typical.end <= t.end)
                .min_by_key(|t| t.end - t.start)
                .copied();
            let typical = parent_table.unwrap_or(typical);
            results.push(SignatureRange {
                range: Range {
                    start: typical.start,
                    end: range.end,
                },
                typical: Some(typical),
            });
        }
        results
    }

    fn check_for_typical_signature(&mut self, range: Range) -> Option<Range> {
        let unsubscribe = self
            .content
            .ranges_matching(|n| dom::has_class(n, UNSUBSCRIBE_CLASS));
        if unsubscribe
            .iter()
            .any(|u| u.start >= range.start && u.end <= range.end)
        {
            return Some(range);
        }
        let text = self.content.search_string(Some(range));
        if let Some(found) = VALEDICTION.find(&text) {
            return Some(match_to_range(&text, found, range.start));
        }
        let Some(name) = self.find_senders_name_range(range, &text) else {
            return OTHER_SIGNATURES
                .iter()
                .find_map(|pattern| pattern.find(&text))
                .map(|found| match_to_range(&text, found, range.start));
        };
        let extended = self.extend_backwards(range, |_, text| SIGNATURE_DASHES.is_match(text));
        let before_name = Range {
            start: extended.start,
            end: name.start,
        };
        self.find_pre_name_signature_content(before_name)
            .or(Some(name))
    }

    fn find_senders_name_range(&self, range: Range, text: &str) -> Option<Range> {
        let name = &self.from.name;
        if name.is_empty() {
            return None;
        }
        let mut names = vec![name.as_str()];
        names.extend(name.split_whitespace());
        names.dedup();
        let alternatives: Vec<String> = names.iter().map(|n| regex::escape(n)).collect();
        let pattern = Regex::new(&format!(
            "(?mi)^.{{0,50}}(?:{}).{{0,100}}$",
            alternatives.join("|")
        ))
        .ok()?;
        pattern
            .find_iter(text)
            .find(|found| valid_signature_name_line(found.as_str()))
            .map(|found| match_to_range(text, found, range.start))
    }

    fn find_pre_name_signature_content(&mut self, range: Range) -> Option<Range> {
        let text = self.content.formatted_slice(range.start, range.end);
        [&*SEPARATOR_LINE, &*DASH_DASH]
            .iter()
            .filter_map(|pattern| pattern.find(&text))
            .min_by_key(|found| found.start())
            .map(|found| match_to_range(&text, found, range.start))
    }

    fn find_on_wrote_blockquote_range(&mut self) -> Option<Range> {
        for blockquote in self.content.ranges_matching(|n| dom::is(n, "blockquote")) {
            let before = self.content.paragraph_before(blockquote.start);
            if !before.is_empty() {
                for intro in REPLY_INTROS.iter() {
                    if let Some(found) = intro.find(&before)
                        && !found.as_str().is_empty()
                    {
                        return Some(
                            self.content
                                .extend_range_backwards(blockquote, found.as_str()),
                        );
                    }
                }
            }
            let text = self
                .content
                .formatted_slice(blockquote.start, blockquote.end);
            if text
                .split('\n')
                .find(|line| !line.trim().is_empty())
                .is_some_and(|line| REPLY_INTROS.iter().any(|intro| intro.is_match(line)))
            {
                return Some(blockquote);
            }
        }
        None
    }

    fn find_quote_header_range(&mut self) -> Option<QuoteHeader> {
        let search = self.content.search_string(None);
        let lines: Vec<&str> = search.split('\n').collect();
        for (from, line) in lines.iter().enumerate() {
            if !FROM_HEADER.is_match(line) {
                continue;
            }
            let mut saw_header = false;
            let mut seen = 0;
            for next in from + 1..lines.len() {
                if seen >= 12 {
                    break;
                }
                if lines[next].trim().is_empty() {
                    continue;
                }
                seen += 1;
                if SUBJECT_HEADER.is_match(lines[next]) {
                    if !saw_header {
                        break;
                    }
                    let Some(header) = self
                        .content
                        .first_range_containing(&lines[from..=next].join("\n"))
                    else {
                        break;
                    };
                    if !self.has_content_above(header.start) {
                        return None;
                    }
                    let start_line = forwarded_header_start_line(&lines, from);
                    let separator = if start_line == from {
                        Some(header)
                    } else {
                        self.content
                            .first_range_containing(&lines[start_line..=next].join("\n"))
                    };
                    return Some(QuoteHeader {
                        range: Range {
                            start: header.start,
                            end: self.content.len(),
                        },
                        separator_start: separator.map_or(header.start, |s| s.start),
                        from_line: (*line).to_owned(),
                    });
                }
                if !OTHER_HEADER.is_match(lines[next]) {
                    break;
                }
                saw_header = true;
            }
        }
        None
    }

    fn has_content_above(&mut self, at: usize) -> bool {
        self.content.formatted_slice(0, at).split('\n').any(|line| {
            let line: String = line.chars().filter(|c| !is_format_char(*c)).collect();
            !line.trim().is_empty() && !has_no_content(&line) && !FORWARDED.is_match(&line)
        })
    }

    fn extend_backward_through_banners(&mut self, range: Range) -> Range {
        let banners = self
            .content
            .ranges_matching(|n| dom::has_class(n, "exo-external-warning-banner"));
        let mut range = range;
        let mut changed = true;
        while changed {
            changed = false;
            for banner in &banners {
                if banner.end > range.start || banner.start == range.start {
                    continue;
                }
                let filler = self.content.formatted_slice(banner.end, range.start);
                if has_no_content(&filler)
                    || (filler.chars().count() < 50 && !filler.chars().any(char::is_alphanumeric))
                {
                    range = Range {
                        start: banner.start,
                        end: range.end,
                    };
                    changed = true;
                }
            }
        }
        range
    }

    fn extend_for_signature(&mut self, range: Range, direction: Direction) -> Range {
        if direction == Direction::Backward {
            let before = self.content.paragraph_before(range.start);
            if let Some(found) = SENT_FROM_PHONE.find(&before) {
                return self
                    .content
                    .extend_range_backwards(range, &before[found.start()..]);
            }
            self.extend_backwards(range, Self::is_footer)
        } else {
            self.extend_forwards(range, Self::is_footer)
        }
    }

    fn extend_backwards(
        &mut self,
        mut range: Range,
        accept: impl Fn(&Self, &str) -> bool,
    ) -> Range {
        loop {
            let paragraph = self.content.paragraph_before(range.start);
            if paragraph.is_empty() || !accept(self, &paragraph) {
                return range;
            }
            let extended = self.content.extend_range_backwards(range, &paragraph);
            if extended == range {
                return range;
            }
            range = extended;
        }
    }

    fn extend_forwards(&mut self, mut range: Range, accept: impl Fn(&Self, &str) -> bool) -> Range {
        loop {
            if range.end >= self.content.len() {
                return range;
            }
            let paragraph = self.content.paragraph_after(range.end);
            if paragraph.is_empty() || !accept(self, &paragraph) {
                return range;
            }
            let extended = self.content.extend_range_forwards(range, &paragraph);
            if extended == range {
                return range;
            }
            range = Range {
                start: extended.start,
                end: extended.end.min(self.content.len()),
            };
        }
    }

    fn is_footer(&self, text: &str) -> bool {
        self.is_exchange_attachments(text)
            || is_disclaimer(text)
            || has_no_content(text)
            || (SENT_WITH.is_match(text) && text.encode_utf16().count() < 100)
            || self.is_senders_name(text)
    }

    fn is_exchange_attachments(&self, text: &str) -> bool {
        let Some(previous) = &self.previous else {
            return false;
        };
        let mut rest = text.to_owned();
        for name in previous.attachment_names {
            rest = rest.replacen(&format!("<{name}>"), "", 1);
        }
        rest.trim().is_empty()
    }

    fn is_senders_name(&self, text: &str) -> bool {
        let name = &self.from.name;
        let text = text.trim().to_lowercase();
        name.split_whitespace()
            .chain(std::iter::once(name.as_str()))
            .any(|name| {
                !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphabetic() || c.is_whitespace())
                    && text == name.to_lowercase()
            })
    }

    /// Marks the range's nodes quoted (or, before the quote, signature) and returns them.
    fn non_empty_quoted_nodes(&mut self, range: Range, quote_only: Range) -> Quoted {
        let nodes = self.content.nodes_for_range(range);
        let signature = if quote_only.start != range.start {
            self.content.nodes_for_range(Range {
                start: range.start,
                end: quote_only.start,
            })
        } else {
            Vec::new()
        };
        let classes: Vec<_> = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                let is_signature = signature
                    .get(index)
                    .is_some_and(|s| std::rc::Rc::ptr_eq(s, node));
                if is_signature {
                    (SIGNATURE_CLASS, QUOTED_CLASS)
                } else {
                    (QUOTED_CLASS, SIGNATURE_CLASS)
                }
            })
            .collect();
        Quoted {
            nodes: Content::remove_empty_nodes(mark(nodes, &classes)),
            hidden: Some(range),
        }
    }
}

struct QuoteHeader {
    range: Range,
    separator_start: usize,
    from_line: String,
}

/// Wraps text nodes in spans, then gives each node its `(class, other)`: sets `class`, clears
/// `other`.
fn mark(nodes: Vec<NodeRef>, classes: &[(&str, &str)]) -> Vec<NodeRef> {
    let mut wraps: Vec<(NodeRef, NodeRef)> = Vec::new();
    let marked: Vec<NodeRef> = nodes
        .into_iter()
        .map(|node| {
            if !dom::is_text(&node) {
                return node;
            }
            let span = dom::element("span", &[]);
            wraps.push((node, span.clone()));
            span
        })
        .collect();
    dom::replace_all(
        wraps
            .iter()
            .map(|(text, span)| (text.clone(), vec![span.clone()]))
            .collect(),
    );
    for (text, span) in &wraps {
        dom::append(span, text);
    }
    for (element, (class, other)) in marked.iter().zip(classes) {
        if dom::is_element(element) {
            dom::add_class(element, &[class]);
            dom::remove_class(element, &[other]);
        }
    }
    marked
}

/// The Google Groups footer.
fn footer_range(content: &Content) -> Option<Range> {
    let subscribed = content.first_range_containing(
        "--\nYou received this message because you are subscribed to the Google Groups ",
    );
    let topic = content.first_range_containing(
        "--\nYou received this message because you are subscribed to a topic in the Google Groups ",
    );
    match (subscribed, topic) {
        (Some(a), Some(b)) => Some(if a.end >= b.end { a } else { b }),
        (a, b) => a.or(b),
    }
}

fn union(range: Option<Range>, other: Range) -> Range {
    match range {
        Some(range) => Range {
            start: range.start.min(other.start),
            end: range.end.max(other.end),
        },
        None => other,
    }
}

/// A match in a search string as a clean range.
fn match_to_range(text: &str, found: regex::Match, offset: usize) -> Range {
    let before = crate::content::clean(&text[..found.start()]).len();
    let length = crate::content::clean(found.as_str()).len();
    Range {
        start: offset + before,
        end: offset + before + length,
    }
}

/// At least 60% of the words in a candidate name line are capitalized.
fn valid_signature_name_line(line: &str) -> bool {
    let stripped: String = line
        .chars()
        .filter(|c| !matches!(c, '|' | '.' | ',' | '-') && *c != REPLACED)
        .collect();
    let words: Vec<&str> = stripped.split_whitespace().collect();
    let words = if words.is_empty() { vec![""] } else { words };
    let capitalized = words
        .iter()
        .filter(|w| w.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
        .count();
    capitalized as f64 / words.len() as f64 >= 0.6
}

fn forwarded_header_start_line(lines: &[&str], from: usize) -> usize {
    for index in (0..from).rev() {
        if !lines[index].trim().is_empty() {
            return if FORWARDED.is_match(lines[index]) {
                index
            } else {
                from
            };
        }
    }
    from
}

fn has_no_content(text: &str) -> bool {
    NO_CONTENT.is_match(text)
}

fn is_disclaimer(text: &str) -> bool {
    DISCLAIMER.is_match(text) || (text.contains("liability") && text.contains("virus"))
}

fn is_format_char(c: char) -> bool {
    matches!(c, '\u{00AD}' | '\u{0600}'..='\u{0605}' | '\u{061C}' | '\u{06DD}' | '\u{070F}' | '\u{180E}'
        | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}'
        | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}')
}
