//! The ranking pipeline end to end: the palette's commands for a highlighted conversation in the
//! list, with their scores, scales, aliases and `follow`s, in the order the palette assembles them,
//! ranked for typical queries. `EXPECTED` keeps only the fixture's rows.

use crate::{command::Command, rank::rank_main};

fn fixture() -> Vec<Command<()>> {
    let command = |id: &'static str, title: &'static str, score: f64| {
        Command::new(id, title, ()).default_score(score)
    };
    let label = |id: &'static str, name: &'static str, keys: Option<&'static str>, system: bool| {
        let command = Command::new(id, format!("go to {name}"), ())
            .default_score(1.0)
            .scale(if system { 0.6 } else { 0.5 })
            .operative_word(if id == "sent" { "sent" } else { name })
            .query_only();
        match keys {
            Some(key) => command.key_code(["G", "then", key]),
            None => command,
        }
    };
    vec![
        // The region's own commands come first.
        command("CREATE_EVENT", "Create Empty Event", 1.3)
            .scale(0.99)
            .key_code(["B"])
            .operative_word("event")
            .aliases([
                "Empty Event",
                "Create Empty Calendar Event",
                "New Empty Event",
                "Add Empty Event",
                "Create Meeting",
                "Send Invitation",
            ]),
        // A conversation's commands.
        command("OPEN", "Open Link", 4.4)
            .key_code(["⌘", "O"])
            .aliases(["Open Attachment", "Open File"]),
        command("REPLY_ALL", "Reply All", 3.0).key_code(["enter"]),
        command("REPLY", "Reply", 2.0).key_code(["R"]),
        command("FORWARD", "Forward", 1.9).key_code(["F"]),
        // Gmail's actions on it.
        command("STAR", "Star", 9.0)
            .key_code(["S"])
            .aliases(["Flag"]),
        command("TRASH", "Trash", 6.0)
            .key_code(["#"])
            .aliases(["Delete"]),
        command("MARK_SPAM", "Mark Spam", 5.0)
            .key_code(["!"])
            .aliases(["Mark Junk"]),
        command("MARK_UNREAD", "Mark Read", 4.0)
            .key_code(["U"])
            .aliases(["Mark as Read", "Toggle Read"]),
        command("MUTE", "Mute", 4.0)
            .key_code(["shift", "M"])
            .aliases(["Ignore", "Unfollow"]),
        command("MOVE", "Move", 8.0)
            .key_code(["V"])
            .aliases(["Folder", "File"])
            .operative_word("move"),
        // Labels.
        command("REMOVE_ALL_LABELS", "Remove all Labels", 6.9)
            .key_code(["shift", "Y"])
            .aliases([
                "Remove all Labels and Mark Done",
                "Remove all Labels and Archive",
            ]),
        command("LABEL", "Label", 7.0)
            .key_code(["L"])
            .aliases([
                "Add Label",
                "Remove Label",
                "Create Label",
                "New Label",
                "Tag",
                "Create Tag",
                "Remove Tag",
            ])
            .follow("CREATE_EVENT"),
        command("REMOVE_FROM_LABEL", "Remove from Label", 7.0)
            .key_code(["Y"])
            .aliases(["Remove from List"]),
        // Archiving.
        command("MARK_DONE", "Mark Done", 11.0)
            .key_code(["E"])
            .aliases(["Archive"]),
        // The rows every screen has.
        command("COMPOSE", "Compose", 1.75)
            .scale(1.2)
            .key_code(["C"])
            .aliases(["New Email", "New Draft", "Write Email", "Send Email"]),
        command("SEARCH", "Search", 1.6)
            .key_code(["/"])
            .aliases(["Look Up"]),
        command("SHORTCUTS_HELP", "Shortcuts", 1.5)
            .aliases([
                "Keyboard Shortcuts",
                "Short Cuts",
                "Hotkeys",
                "Hot Keys",
                "Help",
            ])
            .follow("HELP_CENTER"),
        command("HELP_CENTER", "Help Center", 1.5).aliases([
            "Help",
            "Documentation",
            "FAQs",
            "Frequently Asked Questions",
            "Guide",
            "How do I…?",
            "Learn More",
            "Manual",
            "Resources",
            "Support",
            "Troubleshooting",
        ]),
        command("GUIDES", "Guides", 1.51)
            .aliases([
                "Get Started Guide",
                "Tutorial",
                "Getting Started",
                "How do I…?",
                "Guide",
                "Help",
            ])
            .follow("HELP_CENTER"),
        // The labels to go to, system labels first.
        label("all", "All Mail", Some("A"), true),
        label("sent", "Sent", Some("T"), true),
        label("inbox", "Inbox", Some("I"), true),
        label("trash", "Trash", Some("#"), true).follow("TRASH"),
        label("draft", "Drafts", Some("D"), true),
        label("spam", "Spam", Some("!"), true).follow("MARK_SPAM"),
        label("starred", "Starred", Some("S"), true),
        label("done", "Done", Some("E"), true).follow("MARK_DONE"),
        label("muted", "Muted", Some("M"), true),
        label("clients", "Clients", None, false),
        label("clients/calloway", "Clients/Calloway", None, false),
        label("family", "Family", None, false),
        // Reminders.
        command("SNOOZE", "Remind Me", 10.0)
            .scale(1.01)
            .key_code(["H"])
            .aliases(["Snooze", "Boomerang", "Add Reminder"]),
    ]
}

/// `(query, rows)`. Left out: queries that list more than 64 rows (`e`, `s`, `set`), which V8
/// merge-sorts, and `rem`, where `Label` follows `Create Empty Event` against the scores. Where
/// `follow` contradicts the scores the comparator isn't a total order, so the result depends on
/// every row in the list, not only the fixture's.
const EXPECTED: &[(&str, &[&str])] = &[
    (
        "",
        &[
            "Mark Done",
            "Remind Me",
            "Star",
            "Move",
            "Label",
            "Remove from Label",
            "Remove all Labels",
            "Trash",
            "Mark Spam",
            "Open Link",
            "Mark Read",
            "Mute",
            "Reply All",
            "Reply",
            "Forward",
            "Compose",
            "Search",
            "Help Center",
            "Guides",
            "Shortcuts",
            "Create Empty Event",
        ],
    ),
    (
        "sent",
        &["go to Sent", "Create Empty Event (Send Invitation)"],
    ),
    (
        "go to",
        &[
            "go to All Mail",
            "go to Sent",
            "go to Inbox",
            "go to Trash",
            "go to Drafts",
            "go to Spam",
            "go to Starred",
            "go to Done",
            "go to Muted",
            "go to Clients",
            "go to Clients/Calloway",
            "go to Family",
        ],
    ),
    ("snooze", &["Remind Me (Snooze)"]),
    (
        "mark",
        &[
            "Mark Done",
            "Mark Spam",
            "Mark Read",
            "Remove all Labels (Remove all Labels and Mark Done)",
        ],
    ),
    ("#", &["Trash"]),
    ("lab", &["Label", "Remove from Label", "Remove all Labels"]),
    ("trash", &["Trash", "go to Trash"]),
    (
        "star",
        &["Star", "Guides (Get Started Guide)", "go to Starred"],
    ),
    ("shortcuts", &["Shortcuts"]),
    (
        "help",
        &["Help Center", "Guides (Help)", "Shortcuts (Help)"],
    ),
    ("theme", &[]),
    (
        "un",
        &[
            "Mute (Unfollow)",
            "Mark Spam (Mark Junk)",
            "Help Center (Manual)",
        ],
    ),
    (
        "move",
        &[
            "Move",
            "Label (Remove Label)",
            "Remove from Label",
            "Remove all Labels",
        ],
    ),
    ("inbox", &["go to Inbox"]),
    ("settings", &[]),
    (
        "go",
        &[
            "go to All Mail",
            "go to Sent",
            "go to Inbox",
            "go to Trash",
            "go to Drafts",
            "go to Spam",
            "go to Starred",
            "go to Done",
            "go to Muted",
            "go to Clients",
            "go to Clients/Calloway",
            "go to Family",
            "Mark Read (Toggle Read)",
            "Mute (Ignore)",
        ],
    ),
    (
        "arch",
        &[
            "Mark Done (Archive)",
            "Remove all Labels (Remove all Labels and Archive)",
            "Search",
        ],
    ),
    (
        "dark",
        &["Remove all Labels (Remove all Labels and Mark Done)"],
    ),
    ("compose", &["Compose"]),
    ("reply", &["Reply", "Reply All"]),
    ("search", &["Search"]),
];

#[test]
fn ranks_a_conversations_commands_for_typical_queries() {
    let commands = fixture();
    let candidates: Vec<_> = commands.iter().collect();
    let mut failures = Vec::new();
    for (query, expected) in EXPECTED {
        let ranked: Vec<_> = rank_main(&candidates, query)
            .into_iter()
            .map(|row| row.title.into_owned())
            .collect();
        if ranked != *expected {
            failures.push(format!(
                "{query:?}\n  ranked:   {ranked:?}\n  expected: {expected:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
