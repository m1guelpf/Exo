//! Instant Intro: replying to an introduction, ⌘⇧I moves the introducer to Bcc and starts the reply
//! with the `customInstantIntro` line (`Thanks {first_name}! (to BCC)`) naming them. Each press
//! moves the next of the original To and Cc instead, the line following; after the last, the
//! recipients are as they were and the line goes, to come back on the next press.

use exo_commands::{Command, Commands, Hint, Region};
use exo_store::{Address, DraftAction, MessageDetail};
use html5ever::{parse_document, serialize, tendril::TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom, SerializableHandle};

use super::{Action, Field, Open, auto_bcc, contacts};
use crate::commands;

/// The snippet's element and its name spans, as they go out in the sent mail.
const SNIPPET: &str = "Exo-Intro-Snippet";
const NAME: &str = "Exo-Intro-Name";
/// The same line's classes in a draft begun in another client, read as Exo's.
const FOREIGN_SNIPPET: &str = "Superhuman-Intro-Snippet";
const FOREIGN_NAME: &str = "Superhuman-Intro-Name";

fn is_name(node: &Handle) -> bool {
    attribute(node, "class").is_some_and(|class| {
        class
            .split_whitespace()
            .any(|class| class == NAME || class == FOREIGN_NAME)
    })
}

/// Where a draft's cycle is.
#[derive(Clone, Debug, Default)]
pub(crate) struct Cycle {
    /// Which of the original To and Cc is in Bcc now.
    index: Option<usize>,
    /// The recipients as they were before the first press: To, Cc, Bcc.
    start: Option<[Vec<Address>; 3]>,
    /// The snippet taken out when the cycle came back round, to put back on the next press.
    last_snippet: Option<String>,
    /// The recipients as the cycle last left them: any other change starts it over.
    left: Option<[Vec<Address>; 3]>,
}

impl Cycle {
    /// Starts over if the recipients changed since the cycle last touched them.
    fn reset_if_changed(&mut self, now: &[Vec<Address>; 3]) {
        if self.left.as_ref().is_some_and(|left| left != now) {
            self.index = None;
            self.start = None;
            self.left = None;
        }
    }
}

fn recipients(open: &Open) -> [Vec<Address>; 3] {
    [
        open.record.to.clone(),
        open.record.cc.clone(),
        open.record.bcc.clone(),
    ]
}

/// One press. Returns whether it did anything (a draft needs two recipients), when Exo hints
/// `Not quite right? Hit ⌘ shift I again!`.
pub(crate) fn cycle(
    open: &mut Open,
    intro: &str,
    book: Option<&contacts::Index>,
    now: f64,
) -> bool {
    let current = recipients(open);
    open.intro.reset_if_changed(&current);
    let everyone = |open: &Open| open.recipients().count();
    if everyone(open) < 2 {
        return false;
    }
    // The next of the original To and Cc; after the last, no one.
    let cycle = &mut open.intro;
    match cycle.index {
        None => {
            cycle.index = Some(0);
            cycle.start = Some(current.clone());
        }
        Some(index) => cycle.index = Some(index + 1),
    }
    let [to, cc, bcc] = cycle.start.clone().unwrap_or_else(|| current.clone());
    if cycle
        .index
        .is_some_and(|index| index >= to.len() + cc.len())
    {
        cycle.index = None;
    }
    // That person moves to Bcc, and a To left empty takes the Cc (or failing that, the Bcc).
    let (mut to, mut cc, mut bcc) = (to, cc, bcc);
    let moved = match cycle.index {
        None => None,
        Some(index) if index < to.len() => {
            let person = to.remove(index);
            if to.is_empty() {
                to = std::mem::take(&mut cc);
                if to.is_empty() {
                    to = std::mem::take(&mut bcc);
                }
            }
            bcc.push(person.clone());
            Some(person)
        }
        Some(index) => {
            let person = cc.remove(index - to.len());
            bcc.push(person.clone());
            Some(person)
        }
    };
    let first_name = moved
        .as_ref()
        .and_then(|person| contacts::best_known_first_name(person, book));
    let index = open.intro.index;
    insert_snippet(open, first_name.as_deref(), index, intro);
    // The recipients and body change together (the recipient undo step is `chips::History`'s).
    open.record.to = to;
    open.record.cc = cc;
    open.record.bcc = bcc;
    open.intro.left = Some(recipients(open));
    auto_bcc::settle(open);
    open.touch(now);
    true
}

/// Into an empty body goes the snippet naming `first_name`; one already there names them instead,
/// or goes when no one's moved (kept aside for the next press, which puts it back when it starts
/// the cycle again). A body with anything else in it is left alone.
fn insert_snippet(open: &mut Open, first_name: Option<&str>, index: Option<usize>, intro: &str) {
    let dom = parse_document(RcDom::default(), Default::default()).one(open.editor.html());
    let Some(body) = find(&dom.document, |name, _| name == "body") else {
        return;
    };
    let mut snippet = find_snippet(&body);
    if snippet.is_none()
        && index == Some(0)
        && let Some(kept) = open.intro.last_snippet.take()
    {
        let node = fragment(&kept);
        prepend(&body, &node);
        snippet = Some(node);
    }
    match (snippet, first_name) {
        (Some(snippet), Some(name)) => set_names(&snippet, name),
        (Some(snippet), None) => {
            open.intro.last_snippet = Some(outer_html(&snippet));
            detach(&snippet);
        }
        (None, _) if text(&body).trim().is_empty() => {
            let line = escape(intro)
                .split("{first_name}")
                .collect::<Vec<_>>()
                .join(&format!("<span class='{NAME}'></span>"));
            let node = fragment(&format!(
                "<div class=\"{SNIPPET}\">{line}<div><br></div></div>"
            ));
            if let Some(name) = first_name {
                set_names(&node, name);
            }
            prepend(&body, &node);
        }
        (None, _) => return,
    }
    open.editor.modify_html(&inner_html(&body));
}

/// The body's first `div`, if it's the snippet with a name in it.
fn find_snippet(body: &Handle) -> Option<Handle> {
    let first = find(body, |name, _| name == "div")?;
    let class = attribute(&first, "class").unwrap_or_default();
    ((class.contains(SNIPPET) || class.contains(FOREIGN_SNIPPET))
        && find(&first, |_, node| is_name(node)).is_some())
    .then_some(first)
}

/// Puts `name` in each of the snippet's name spans.
fn set_names(snippet: &Handle, name: &str) {
    let mut stack = vec![snippet.clone()];
    while let Some(node) = stack.pop() {
        if is_name(&node) {
            let text = markup5ever_rcdom::Node::new(NodeData::Text {
                contents: std::cell::RefCell::new(name.into()),
            });
            for child in node.children.borrow_mut().drain(..) {
                child.parent.set(None);
            }
            text.parent.set(Some(std::rc::Rc::downgrade(&node)));
            node.children.borrow_mut().push(text);
        } else {
            stack.extend(node.children.borrow().iter().cloned());
        }
    }
}

/// The first element under `root` (depth first) that `matches` its tag name.
fn find(root: &Handle, matches: impl Fn(&str, &Handle) -> bool) -> Option<Handle> {
    let mut stack = vec![root.clone()];
    while let Some(node) = stack.pop() {
        if let NodeData::Element { name, .. } = &node.data
            && !std::rc::Rc::ptr_eq(&node, root)
            && matches(&name.local, &node)
        {
            return Some(node);
        }
        stack.extend(node.children.borrow().iter().rev().cloned());
    }
    None
}

fn attribute(node: &Handle, wanted: &str) -> Option<String> {
    let NodeData::Element { attrs, .. } = &node.data else {
        return None;
    };
    attrs
        .borrow()
        .iter()
        .find(|attr| &*attr.name.local == wanted)
        .map(|attr| attr.value.to_string())
}

/// `textContent`.
fn text(node: &Handle) -> String {
    let mut out = String::new();
    let mut stack = vec![node.clone()];
    while let Some(node) = stack.pop() {
        if let NodeData::Text { contents } = &node.data {
            out.push_str(&contents.borrow());
        }
        stack.extend(node.children.borrow().iter().rev().cloned());
    }
    out
}

/// The element `html` parses to, taken out of its document (whose drop empties every node
/// still in it).
fn fragment(html: &str) -> Handle {
    let dom = parse_document(RcDom::default(), Default::default()).one(html);
    let body = find(&dom.document, |name, _| name == "body").expect("a parsed document has a body");
    let node = body.children.borrow_mut().remove(0);
    node.parent.set(None);
    node
}

fn prepend(parent: &Handle, node: &Handle) {
    node.parent.set(Some(std::rc::Rc::downgrade(parent)));
    parent.children.borrow_mut().insert(0, node.clone());
}

fn detach(node: &Handle) {
    let Some(parent) = node.parent.take().and_then(|parent| parent.upgrade()) else {
        return;
    };
    parent
        .children
        .borrow_mut()
        .retain(|child| !std::rc::Rc::ptr_eq(child, node));
}

fn serialize_node(node: &Handle, scope: serialize::TraversalScope) -> String {
    let mut out = Vec::new();
    let handle: SerializableHandle = node.clone().into();
    let options = serialize::SerializeOpts {
        traversal_scope: scope,
        ..Default::default()
    };
    if serialize::serialize(&mut out, &handle, options).is_err() {
        return String::new();
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn inner_html(node: &Handle) -> String {
    serialize_node(node, serialize::TraversalScope::ChildrenOnly(None))
}

fn outer_html(node: &Handle) -> String {
    serialize_node(node, serialize::TraversalScope::IncludeNode)
}

/// The intro line as HTML text, so it shows as typed.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Registers `Instant Intro` (⌘⇧I) for the draft with the keyboard.
pub(super) fn register(commands: &mut Commands<commands::Action>) {
    commands
        .add(command(Action::InstantIntro).regions(&[Region::ComposeForm, Region::RecipientList]));
}

/// Registers `Instant Intro` on a conversation: in a thread, the focused message's; in the list,
/// the highlighted conversation's.
pub(super) fn register_on_conversation(commands: &mut Commands<commands::Action>, in_thread: bool) {
    let regions: &[Region] = if in_thread {
        &[Region::ThreadMessages]
    } else {
        exo_commands::CONVERSATION
    };
    let command = command(Action::InstantIntroReply).regions(regions);
    commands.add(if in_thread {
        command
    } else {
        command.scale(1.01)
    });
}

fn command(action: Action) -> Command<commands::Action> {
    Command::new(
        "INSTANT_INTRO",
        "Instant Intro",
        commands::Action::Compose(action),
    )
    .key("mod+shift+i")
    .key_code(["⌘", "shift", "I"])
    .aliases(["To BCC", "Instant Introduction", "Move sender to BCC"])
    .icon("data/instant-intro")
}

/// The hint after a press: `Not quite right? Hit ⌘ shift I again!`.
fn try_again() -> Hint {
    Hint::new()
        .text("Not quite right? Hit")
        .keys(["⌘", "shift", "I"])
        .text("again!")
}

impl crate::Mail {
    /// ⌘⇧I in a draft: the next step of its cycle.
    pub(super) fn instant_intro(&mut self, now: f64) {
        let intro = self.account.settings().instant_intro().to_owned();
        if let Some((open, book)) = self.compose.focused_with_book()
            && cycle(open, &intro, book, now)
        {
            self.commands.hint(try_again());
        }
    }

    /// ⌘⇧I on a conversation: the focused message's draft cycles, or a Reply All starts with the
    /// first step. A message with fewer than two recipients hints that Instant Intro is for
    /// introductions.
    pub(super) fn instant_intro_reply(&mut self, now: f64) {
        let focused = self.thread.as_ref().and_then(|view| {
            let detail = view.detail()?;
            let id = view.focused()?;
            detail
                .messages
                .iter()
                .find(|message| message.id == id)
                .cloned()
        });
        if let Some(message) = &focused
            && intro_recipients(message) < 2
        {
            self.commands.hint(
                Hint::new()
                    .text("Next time, hit")
                    .keys(["⌘", "shift", "I"])
                    .text("on an intro to instantly start a response."),
            );
        }
        let existing = focused.as_ref().and_then(|message| {
            self.compose
                .drafts
                .iter()
                .find(|open| open.record.in_reply_to.as_deref() == Some(message.id.as_str()))
                .map(|open| open.id.clone())
        });
        if let Some(id) = existing {
            self.compose.focus = Some(id);
            self.instant_intro(now);
            return;
        }
        self.compose_run(Action::Reply(DraftAction::ReplyAll), now);
        match &self.compose.pending {
            Some(pending) => self.compose.intro_pending = Some(pending.thread.clone()),
            None => self.instant_intro(now),
        }
    }

    /// The Reply All of an Instant Intro asked for before its thread loaded takes its first
    /// step once it exists.
    pub(super) fn start_pending_intro(&mut self, now: f64) {
        if self.compose.pending.is_some() {
            return;
        }
        let Some(thread) = self.compose.intro_pending.take() else {
            return;
        };
        if let Some(open) = self
            .compose
            .focused_mut()
            .filter(|open| open.thread.as_deref() == Some(thread.as_str()))
        {
            open.state.field = Field::Body;
            self.instant_intro(now);
        }
    }
}

/// How many people the message is to: its valid To, Cc and Bcc, each address once.
fn intro_recipients(message: &MessageDetail) -> usize {
    let mut seen: Vec<String> = Vec::new();
    for contact in message.to.iter().chain(&message.cc).chain(&message.bcc) {
        let email = contact.email.to_lowercase();
        if super::recipients::is_valid(&email) && !seen.contains(&email) {
            seen.push(email);
        }
    }
    seen.len()
}

#[cfg(test)]
mod tests {
    use exo_store::DraftRecord;

    use super::*;

    fn person(name: &str, email: &str) -> Address {
        Address {
            name: Some(name.into()),
            email: email.into(),
        }
    }

    fn reply(to: &[Address], cc: &[Address]) -> Open {
        let record = DraftRecord {
            action: DraftAction::ReplyAll,
            from: None,
            to: to.to_vec(),
            cc: cc.to_vec(),
            bcc: Vec::new(),
            subject: "Intro".into(),
            body: String::new(),
            quoted_content: None,
            quoted_inlined: false,
            in_reply_to: Some("m".into()),
            in_reply_to_rfc822: None,
            references: Vec::new(),
            rfc822_id: "x@y".into(),
            fingerprint: String::new(),
            attachments: Vec::new(),
            send_later: None,
            reminder: None,
        };
        Open::new("d".into(), Some("t".into()), record)
    }

    fn emails(list: &[Address]) -> Vec<&str> {
        list.iter().map(|address| address.email.as_str()).collect()
    }

    const INTRO: &str = "Thanks {first_name}! (to BCC)";

    #[test]
    fn each_press_moves_the_next_person_to_bcc_then_restores() {
        let (sam, bea, cy) = (
            person("Sam Lee", "sam@x.com"),
            person("Bea Ruiz", "bea@y.com"),
            person("Cy", "cy@z.com"),
        );
        let mut open = reply(std::slice::from_ref(&sam), &[bea.clone(), cy.clone()]);
        assert!(cycle(&mut open, INTRO, None, 0.0));
        // Sam was alone in To: Cc moves up.
        assert_eq!(
            (
                emails(&open.record.to),
                emails(&open.record.cc),
                emails(&open.record.bcc)
            ),
            (vec!["bea@y.com", "cy@z.com"], vec![], vec!["sam@x.com"])
        );
        assert_eq!(
            open.editor.document().blocks[0].text(),
            "Thanks Sam! (to BCC)"
        );
        assert!(cycle(&mut open, INTRO, None, 0.0));
        assert_eq!(
            (
                emails(&open.record.to),
                emails(&open.record.cc),
                emails(&open.record.bcc)
            ),
            (vec!["sam@x.com"], vec!["cy@z.com"], vec!["bea@y.com"])
        );
        assert_eq!(
            open.editor.document().blocks[0].text(),
            "Thanks Bea! (to BCC)"
        );
        assert!(cycle(&mut open, INTRO, None, 0.0));
        assert_eq!(emails(&open.record.bcc), ["cy@z.com"]);
        assert!(
            cycle(&mut open, INTRO, None, 0.0),
            "after the last, back to the start"
        );
        assert_eq!(
            (
                emails(&open.record.to),
                emails(&open.record.cc),
                emails(&open.record.bcc)
            ),
            (vec!["sam@x.com"], vec!["bea@y.com", "cy@z.com"], vec![])
        );
        assert!(open.editor.is_empty(), "the line goes with them");
        assert!(cycle(&mut open, INTRO, None, 0.0));
        assert_eq!(
            open.editor.document().blocks[0].text(),
            "Thanks Sam! (to BCC)",
            "and comes back"
        );
        assert!(open.editor.html().starts_with("<div class=\"Exo-Intro-Snippet\">Thanks <span class=\"Exo-Intro-Name\">Sam</span>! (to BCC)<div><br></div></div>"), "{}", open.editor.html());
    }

    #[test]
    fn a_body_with_something_in_it_keeps_it_and_one_recipient_does_nothing() {
        let mut lonely = reply(&[person("Sam", "sam@x.com")], &[]);
        assert!(!cycle(&mut lonely, INTRO, None, 0.0));
        let mut written = reply(&[person("Sam", "sam@x.com")], &[person("Bea", "bea@y.com")]);
        written.editor.type_text("Hi both");
        let body = written.editor.html();
        assert!(cycle(&mut written, INTRO, None, 0.0));
        assert_eq!(emails(&written.record.bcc), ["sam@x.com"]);
        assert_eq!(written.editor.html(), body);
    }

    #[test]
    fn a_line_in_the_foreign_classes_is_renamed_too() {
        let mut open = reply(&[person("Sam", "sam@x.com")], &[person("Bea", "bea@y.com")]);
        open.editor.set_html("<div class=\"Superhuman-Intro-Snippet\">Thanks <span class='Superhuman-Intro-Name'>Dee</span>! (to BCC)<div><br></div></div><div><br></div>");
        assert!(cycle(&mut open, INTRO, None, 0.0));
        assert_eq!(
            open.editor.document().blocks[0].text(),
            "Thanks Sam! (to BCC)"
        );
    }

    #[test]
    fn a_custom_line_and_a_change_by_hand_start_over() {
        let mut open = reply(&[person("Sam", "sam@x.com")], &[person("Bea", "bea@y.com")]);
        assert!(cycle(&mut open, "Merci {first_name} & co", None, 0.0));
        assert_eq!(open.editor.document().blocks[0].text(), "Merci Sam & co");
        open.record.to.push(person("Dee", "dee@w.com"));
        assert!(cycle(&mut open, INTRO, None, 0.0));
        assert_eq!(
            emails(&open.record.bcc),
            ["sam@x.com", "bea@y.com"],
            "a new cycle from the recipients as they are"
        );
    }
}
