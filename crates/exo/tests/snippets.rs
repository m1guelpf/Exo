//! Personal snippets end to end: the folder (`G` `;`), creating and saving one, opening it
//! read-only, leaving it with unsaved changes, using it inline with `;` and with ⌘; (variables
//! replaced), and counting its sends and replies, on gmail-mock.

mod support;

use egui::{Key, Modifiers};
use gmail::{DraftQuery, Format};
use support::{Fixture, start, start_with_settings};

const COMMAND: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);

/// `G` `;`, `C`, Enter (`Private Snippet`): a snippet named `name` with `body`, saved with ⌘Enter.
fn create_snippet(app: &mut Fixture, name: &str, body: &str) {
    app.key(Key::G);
    app.key(Key::Semicolon);
    assert!(
        app.mail().snippets_folder_open(),
        "G ; goes to the Snippets folder"
    );
    app.key(Key::C);
    app.key(Key::Enter);
    assert!(
        app.mail().compose_pane_open(),
        "Private Snippet opens the editor"
    );
    app.type_text(name);
    app.key(Key::Tab);
    app.type_text(body);
    app.press(Key::Enter, COMMAND);
    assert!(!app.mail().compose_pane_open(), "Done closes it");
    assert!(app.mail().snippets_folder_open(), "back in the folder");
}

/// The raw message of Gmail's draft whose subject is `subject`.
fn draft_raw(app: &Fixture, subject: &str) -> String {
    let gmail = app.gmail();
    app.block_on(async {
        for draft in gmail
            .gmail()
            .drafts(&DraftQuery::default())
            .await
            .expect("drafts.list")
            .drafts
        {
            let full = gmail
                .gmail()
                .draft(&draft.id.expect("an id"), Format::Raw)
                .await
                .expect("drafts.get");
            let raw = String::from_utf8_lossy(
                &full
                    .message
                    .and_then(|message| message.raw)
                    .unwrap_or_default(),
            )
            .into_owned();
            if raw.contains(&format!("Subject: {subject}")) {
                return raw;
            }
        }
        String::new()
    })
}

#[test]
fn a_snippet_is_saved_in_the_settings_and_never_becomes_a_gmail_draft() {
    let mut app = start("snippets-create");
    let drafts = |app: &Fixture| {
        let gmail = app.gmail();
        app.block_on(async {
            gmail
                .gmail()
                .drafts(&DraftQuery::default())
                .await
                .expect("drafts.list")
                .drafts
                .len()
        })
    };
    let before = drafts(&app);
    create_snippet(&mut app, "intro", "Hi {first_name}, meet Dan.");
    let snippets = &app.mail().account().settings().snippets;
    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0].name, "intro");
    assert!(
        snippets[0].body.contains("Hi {first_name}, meet Dan."),
        "{}",
        snippets[0].body
    );
    app.wait(1.5);
    assert_eq!(drafts(&app), before, "no Gmail draft for the snippet");
}

#[test]
fn leaving_a_snippet_with_changes_asks_to_save_it() {
    let mut app = start("snippets-leave");
    app.key(Key::G);
    app.key(Key::Semicolon);
    app.key(Key::C);
    app.key(Key::Enter);
    app.type_text("draft name");
    // Escape lets go of the form; Escape again leaves, which asks first.
    app.key(Key::Escape);
    app.key(Key::Escape);
    assert!(app.mail().compose_pane_open(), "Save Snippet? holds it");
    // Enter takes `Save`.
    app.key(Key::Enter);
    assert!(!app.mail().compose_pane_open());
    assert_eq!(
        app.mail()
            .account()
            .settings()
            .snippets
            .iter()
            .map(|snippet| snippet.name.as_str())
            .collect::<Vec<_>>(),
        ["draft name"]
    );
}

#[test]
fn semicolon_in_a_reply_inserts_a_snippet_with_its_variables() {
    let mut app = start("snippets-inline");
    create_snippet(&mut app, "intro", "Hi {first_name}, meet Dan.");
    app.key(Key::G);
    app.key(Key::I);
    app.key(Key::Enter);
    app.key(Key::R);
    app.type_text(";intro");
    app.key(Key::Enter);
    app.wait(1.5);
    let raw = draft_raw(&app, "Re: Design crit moved to 3pm");
    assert!(
        raw.contains("Hi Tom, meet Dan."),
        "the name is Tom's and `;intro` is gone: {raw}"
    );
    assert!(!raw.contains(";intro"));
    assert!(
        app.mail()
            .account()
            .settings()
            .snippets_last_used
            .contains_key(&app.mail().account().settings().snippets[0].id),
        "used now"
    );
}

#[test]
fn command_semicolon_picks_a_snippet_for_the_draft() {
    let mut app = start("snippets-picker");
    create_snippet(&mut app, "thanks", "Thanks {first_name}!");
    app.key(Key::G);
    app.key(Key::I);
    app.key(Key::Enter);
    app.key(Key::R);
    app.press(Key::Semicolon, COMMAND);
    app.key(Key::Enter);
    app.wait(1.5);
    assert!(draft_raw(&app, "Re: Design crit moved to 3pm").contains("Thanks Tom!"));
}

#[test]
fn enter_opens_a_snippet_read_only_and_edit_snippet_opens_the_editor() {
    let mut app = start("snippets-view");
    create_snippet(&mut app, "intro", "Hi {first_name}, meet Dan.");
    let id = app.mail().account().settings().snippets[0].id.clone();
    app.key(Key::Enter);
    assert_eq!(
        app.mail().snippet_viewed(),
        Some(id.as_str()),
        "read-only first"
    );
    assert!(!app.mail().compose_pane_open());
    app.key(Key::Escape);
    assert_eq!(
        app.mail().snippet_viewed(),
        None,
        "Escape goes back to the folder"
    );
    assert!(app.mail().snippets_folder_open());
    app.key(Key::Enter);
    app.press(Key::K, COMMAND);
    app.type_text("Edit Snippet");
    app.key(Key::Enter);
    assert!(
        app.mail().compose_pane_open(),
        "Edit Snippet opens the editor"
    );
    assert_eq!(app.mail().snippet_viewed(), None);
}

#[test]
fn going_to_another_list_with_unsaved_changes_asks_to_save_the_snippet() {
    let mut app = start("snippets-leave-list");
    app.key(Key::G);
    app.key(Key::Semicolon);
    app.key(Key::C);
    app.key(Key::Enter);
    app.type_text("draft name");
    // The form lets go of the keyboard, then G I asks before going.
    app.key(Key::Escape);
    app.key(Key::G);
    app.key(Key::I);
    assert!(app.mail().compose_pane_open(), "Save Snippet? holds it");
    assert!(app.mail().snippets_folder_open());
    // Enter takes `Save`, and the going goes on.
    app.key(Key::Enter);
    assert!(!app.mail().compose_pane_open());
    assert!(!app.mail().snippets_folder_open(), "on to the inbox");
    assert_eq!(
        app.mail()
            .account()
            .settings()
            .snippets
            .iter()
            .map(|snippet| snippet.name.as_str())
            .collect::<Vec<_>>(),
        ["draft name"]
    );
}

#[test]
fn a_reply_to_a_message_sent_with_a_snippet_counts_in_the_folder() {
    let mut app = start("snippets-replies");
    create_snippet(&mut app, "thanks", "Thanks {first_name}!");
    app.key(Key::G);
    app.key(Key::I);
    app.key(Key::Enter);
    let thread = app.mail().thread().expect("open").id().to_owned();
    app.key(Key::R);
    app.press(Key::Semicolon, COMMAND);
    app.key(Key::Enter);
    app.press(Key::Enter, COMMAND);
    // Instant Send.
    app.press(Key::Z, COMMAND.plus(Modifiers::SHIFT));
    app.wait(1.0);
    let analytics = |app: &Fixture| {
        app.mail().account().settings().snippets[0]
            .analytics
            .clone()
    };
    assert_eq!(analytics(&app).sends, 1, "the send counts");
    assert!(!analytics(&app).last_sent.is_empty(), "and dates Last Sent");
    assert_eq!(analytics(&app).unreplied.len(), 1, "waiting for a reply");
    let deliver = app.mock_url().join("__mock/deliver").expect("a URL");
    let reply = serde_json::json!({ "threadId": thread, "from": "Tom Becker <tom@brightmoor.co>", "text": "Thanks!" }).to_string();
    app.block_on(async {
        reqwest::Client::new()
            .post(deliver)
            .header("content-type", "application/json")
            .body(reply)
            .send()
            .await?
            .error_for_status()
    })
    .expect("delivered");
    app.sync();
    // Opening the folder looks for replies.
    app.key(Key::G);
    app.key(Key::Semicolon);
    app.wait(1.0);
    assert_eq!(analytics(&app).replies, 1, "Tom's reply counts");
    assert!(
        analytics(&app).unreplied.is_empty(),
        "and isn't looked for again"
    );
}

/// `G` `;`, `C`, Enter, a name typed: a new snippet with unsaved changes, the form letting go of
/// the keyboard.
fn start_unsaved_snippet(app: &mut Fixture) {
    app.key(Key::G);
    app.key(Key::Semicolon);
    app.key(Key::C);
    app.key(Key::Enter);
    app.type_text("draft name");
    app.key(Key::Escape);
}

#[test]
fn popping_a_draft_in_over_an_unsaved_snippet_asks_to_save_it() {
    let mut app = start("snippets-pop-in");
    start_unsaved_snippet(&mut app);
    app.press(Key::C, Modifiers::SHIFT);
    assert!(
        app.mail().compose_popped_out(),
        "Pop Out Compose over the snippet"
    );
    app.press(Key::P, COMMAND.plus(Modifiers::SHIFT));
    assert_eq!(
        app.mail().compose_alert(),
        Some("Would you like to save changes to this Snippet? \n You cannot undo this action."),
        "Save Snippet? first"
    );
    assert!(
        app.mail().compose_popped_out(),
        "the draft waits, popped out"
    );
    // Enter takes `Save`, then the draft pops in.
    app.key(Key::Enter);
    assert!(!app.mail().compose_popped_out());
    assert!(app.mail().compose_pane_open(), "the draft is in the pane");
    assert_eq!(
        app.mail()
            .account()
            .settings()
            .snippets
            .iter()
            .map(|snippet| snippet.name.as_str())
            .collect::<Vec<_>>(),
        ["draft name"]
    );
}

#[test]
fn undo_send_into_the_pane_over_an_unsaved_snippet_asks_to_save_it() {
    let mut app = start("snippets-undo-send");
    app.key(Key::C);
    app.type_text("priya@brightmoor.co");
    app.key(Key::Enter);
    app.key(Key::Tab);
    app.type_text("Lunch");
    app.key(Key::Tab);
    app.type_text("Thursday?");
    app.press(Key::Enter, COMMAND);
    assert!(!app.mail().compose_pane_open(), "sent, held for Undo");
    start_unsaved_snippet(&mut app);
    app.key(Key::Z);
    app.settle();
    assert_eq!(
        app.mail().compose_alert(),
        Some("Would you like to save changes to this Snippet? \n You cannot undo this action."),
        "Save Snippet? before the draft comes back"
    );
    // `Don't save` (Delete, for one never saved), then the draft is back in the pane.
    app.key(Key::Tab);
    app.key(Key::Tab);
    app.key(Key::Enter);
    assert!(
        app.mail().compose_pane_open(),
        "the unsent draft is in the pane"
    );
    assert!(
        app.mail().account().settings().snippets.is_empty(),
        "the snippet was dropped"
    );
}

/// Moves the pointer down onto `at`, as a hand going down the list does (not heading for the
/// right pane, so the row takes the cursor at once).
fn hover_down(app: &mut Fixture, at: egui::Pos2) {
    for to in [at - egui::vec2(0.0, 6.0), at] {
        app.harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(to));
        app.harness.run_steps(1);
    }
    app.settle();
}

#[test]
fn hovering_a_row_moves_the_folders_cursor() {
    let mut app = start("snippets-hover");
    create_snippet(&mut app, "first", "One.");
    create_snippet(&mut app, "second", "Two.");
    app.key(Key::G);
    app.key(Key::Semicolon);
    // The rows are 36px from 156px down, at 1280×800.
    hover_down(&mut app, egui::pos2(300.0, 210.0));
    let second = app.mail().snippet_cursor().map(str::to_owned);
    hover_down(&mut app, egui::pos2(300.0, 174.0));
    let first = app.mail().snippet_cursor().map(str::to_owned);
    assert!(
        first.is_some() && second.is_some() && first != second,
        "each row takes the cursor: {first:?}, {second:?}"
    );
    app.key(Key::J);
    assert_eq!(
        app.mail().snippet_cursor().map(str::to_owned),
        second,
        "J goes on from the hovered row"
    );
}

#[test]
fn the_read_only_view_lists_the_snippets_files_and_zips_them() {
    let file = |uuid: &str, name: &str| exo_store::DraftAttachment {
        uuid: uuid.into(),
        name: name.into(),
        mime_type: "text/plain".into(),
        size: 5,
        content_id: None,
        source: exo_store::AttachmentSource::Local,
    };
    let mut app = start_with_settings("snippets-files", &|settings| {
        settings.snippets.push(exo_store::Snippet {
            id: "draft00snippetfiles".into(),
            name: "Pricing".into(),
            subject: "Pricing".into(),
            body: "<div>Our plans.</div>".into(),
            attachments: vec![file("u-1", "plans.txt"), file("u-2", "terms.txt")],
            date: "2026-11-12T15:30:00Z".into(),
            ..exo_store::Snippet::default()
        });
    });
    let kept = app.mail().account().draft_files().to_owned();
    std::fs::create_dir_all(&kept).expect("the draft files");
    for (uuid, text) in [("u-1", "plans"), ("u-2", "terms")] {
        std::fs::write(kept.join(uuid), text).expect("kept");
    }
    let downloads = app.folder("snippet-downloads");
    exo::downloads::choose(&app.harness.ctx, downloads.clone()).expect("chosen");
    app.key(Key::G);
    app.key(Key::Semicolon);
    app.key(Key::Enter);
    assert_eq!(app.mail().snippet_viewed(), Some("draft00snippetfiles"));
    // `Download all` is the last thing under the cards before the footer: just above where the
    // pointer stops being a hand.
    let mut on_files = false;
    let mut at = None;
    for step in 0..250 {
        let here = egui::pos2(280.0, 250.0 + 2.0 * step as f32);
        let hand = app.hover(here) == egui::CursorIcon::PointingHand;
        if on_files && !hand {
            at = Some(here - egui::vec2(0.0, 8.0));
            break;
        }
        on_files |= hand;
    }
    app.click(at.expect("Download all"));
    for _ in 0..20 {
        if downloads.join("pricing_1.zip").is_file() {
            break;
        }
        app.wait(0.25);
    }
    let listing = std::process::Command::new("unzip")
        .arg("-Z1")
        .arg(downloads.join("pricing_1.zip"))
        .output()
        .expect("unzip runs");
    assert_eq!(
        String::from_utf8_lossy(&listing.stdout)
            .lines()
            .collect::<Vec<_>>(),
        ["plans.txt", "terms.txt"]
    );
}
