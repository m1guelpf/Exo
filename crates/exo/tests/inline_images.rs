//! Inline images in compose end to end, on gmail-mock: a pasted screenshot and a dropped image go
//! in the body and into the Gmail draft as inline parts, sending carries them by Content-ID, a
//! draft written in Gmail shows its images and keeps them, and remote images follow the Images
//! setting and Show Images.

mod support;

use std::{
    io::{Read as _, Write as _},
    net::TcpListener,
    sync::{Arc, Mutex},
};

use egui::{Event, Key, Modifiers};
use exo_store::RemoteContentBlocking;
use gmail::{DraftQuery, Format, ListQuery};
use support::{Fixture, start, start_with_settings};

const COMMAND: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);
/// The first conversation, which Enter opens.
const REPLY: &str = "Re: Design crit moved to 3pm";

/// An 800×500 PNG at 144 dpi: a Retina screenshot, 400×250 at Best Fit.
fn screenshot() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(800, 500, image::Rgba([40, 90, 200, 255]));
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("encoded");
    // `pHYs` (5669 pixels a metre each way) after `IHDR`.
    let mut chunk = Vec::new();
    chunk.extend_from_slice(&9u32.to_be_bytes());
    chunk.extend_from_slice(b"pHYs");
    chunk.extend_from_slice(&5669u32.to_be_bytes());
    chunk.extend_from_slice(&5669u32.to_be_bytes());
    chunk.push(1);
    chunk.extend_from_slice(&[0; 4]);
    png.splice(33..33, chunk);
    png
}

/// Pastes `png` as an image on the clipboard, and lets it load.
fn paste_image(app: &mut Fixture, png: Vec<u8>) {
    exo::clipboard::capture_image(&app.harness.ctx, png);
    app.harness
        .input_mut()
        .events
        .push(Event::Paste(String::new()));
    app.harness.run_steps(1);
    app.wait(1.0);
}

/// The Content-ID the body's image is shown by.
fn body_cid(app: &Fixture) -> String {
    let body = app.mail().compose_body().expect("a draft has the keyboard");
    let start = body
        .find("src-cid=\"")
        .map(|at| at + 9)
        .unwrap_or_else(|| panic!("an inline image in {body}"));
    body[start..start + body[start..].find('"').expect("a closing quote")].to_owned()
}

/// `raw` with its quoted-printable line breaks and `=` escapes undone, enough to read HTML in.
fn unquoted(raw: &str) -> String {
    raw.replace("=\r\n", "").replace("=3D", "=")
}

/// The raw message of Gmail's draft with `subject`.
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

/// The raw message Gmail sent with `subject`.
fn sent_raw(app: &Fixture, subject: &str) -> String {
    let gmail = app.gmail();
    app.block_on(async {
        let query = ListQuery {
            label_ids: vec!["SENT".into()],
            max_results: Some(500),
            ..ListQuery::default()
        };
        for message in gmail
            .gmail()
            .messages(&query)
            .await
            .expect("messages.list")
            .messages
        {
            let full = gmail
                .gmail()
                .message(&message.id.expect("an id"), Format::Raw, &[])
                .await
                .expect("messages.get");
            let raw = String::from_utf8_lossy(&full.raw.unwrap_or_default()).into_owned();
            if raw.contains(&format!("Subject: {subject}")) {
                return raw;
            }
        }
        String::new()
    })
}

#[test]
fn a_pasted_screenshot_goes_in_the_body_and_out_by_content_id() {
    let mut app = start("images-paste");
    app.key(Key::Enter);
    app.key(Key::R);
    app.type_text("Here it is");
    paste_image(&mut app, screenshot());
    let cid = body_cid(&app);
    let body = app.mail().compose_body().expect("a draft");
    assert!(
        body.contains(&format!(r#"<img class="Exo-inline-image" src-cid="{cid}" style="max-width: 100%; height: auto; width: 400px;" src="" data-retina="2" width="400">"#)),
        "Best Fit at its resolution: {body}"
    );
    app.wait(1.5);
    let draft = unquoted(&draft_raw(&app, REPLY));
    assert!(
        draft.contains(&format!("Content-ID: <{cid}>")) && draft.contains("multipart/related"),
        "the Gmail draft carries it inline: {draft}"
    );
    assert!(draft.contains(&format!(r#"src="cid:{cid}""#)), "{draft}");

    app.press(Key::Enter, COMMAND);
    app.press(Key::Z, COMMAND.plus(Modifiers::SHIFT));
    app.wait(0.5);
    let sent = unquoted(&sent_raw(&app, REPLY));
    assert!(
        sent.contains(&format!("Content-ID: <{cid}>")),
        "sent inline: {sent}"
    );
    assert!(sent.contains(&format!(r#"<img class="" src-cid="{cid}" style="max-width: 100%; height: auto; width: 400px;" src="cid:{cid}" data-retina="2" width="400">"#)), "the image as it goes out: {sent}");
}

#[test]
fn command_v_alone_pastes_a_screenshot() {
    // A clipboard holding only an image gives egui-winit no text, so it sends no paste: the ⌘V
    // key is all that comes, from the keyboard or Edit ▸ Paste.
    let mut app = start("images-paste-key");
    app.key(Key::Enter);
    app.key(Key::R);
    app.type_text("Here");
    exo::clipboard::capture_image(&app.harness.ctx, screenshot());
    app.press(Key::V, COMMAND);
    let body = app.mail().compose_body().expect("a draft");
    assert!(
        body.contains(r#"<img class="Exo-inline-image""#),
        "⌘V puts the screenshot in: {body}"
    );
    app.press(Key::V, COMMAND.plus(Modifiers::SHIFT));
    assert_eq!(
        app.mail()
            .compose_body()
            .expect("a draft")
            .matches("<img")
            .count(),
        1,
        "Paste and Match Style pastes text only"
    );
}

#[test]
fn one_dropped_image_goes_in_the_body_and_several_files_go_on_the_draft() {
    let mut app = start("images-drop");
    let folder = app.folder("drops");
    std::fs::create_dir_all(&folder).expect("a folder");
    let (photo, notes) = (folder.join("photo.png"), folder.join("notes.txt"));
    std::fs::write(&photo, screenshot()).expect("written");
    std::fs::write(&notes, "agenda").expect("written");
    app.key(Key::Enter);
    app.key(Key::R);
    let drop = |app: &mut Fixture, paths: &[&std::path::Path]| {
        for path in paths {
            app.harness
                .input_mut()
                .dropped_files
                .push(Arc::new(Dropped(path.to_path_buf())));
        }
        app.harness.run_steps(2);
    };
    // Both before the draft's first save: after it, one with files waits out its pace.
    drop(&mut app, &[&photo]);
    let cid = body_cid(&app);
    drop(&mut app, &[&photo, &notes]);
    app.wait(1.5);
    let draft = unquoted(&draft_raw(&app, REPLY));
    assert!(
        draft.contains(&format!("Content-ID: <{cid}>")),
        "the dropped image is inline: {draft}"
    );
    assert!(
        draft.contains("filename=\"notes.txt\"") && draft.contains("filename=\"photo.png\""),
        "two files attach: {draft}"
    );
    assert_eq!(
        draft.matches("Content-ID:").count(),
        1,
        "only the first is inline"
    );
}

#[test]
fn undoing_an_image_leaves_it_out_of_what_goes_to_gmail() {
    let mut app = start("images-undo");
    app.key(Key::Enter);
    app.key(Key::R);
    app.type_text("Here");
    exo::clipboard::capture_image(&app.harness.ctx, screenshot());
    app.harness
        .input_mut()
        .events
        .push(Event::Paste(String::new()));
    app.harness.run_steps(2);
    let cid = body_cid(&app);
    // Before the draft's first save: after it, a draft with files waits out its pace.
    app.press(Key::Z, COMMAND);
    assert!(
        !app.mail().compose_body().expect("a draft").contains("<img"),
        "⌘Z takes the paste back"
    );
    app.wait(1.5);
    assert!(
        !draft_raw(&app, REPLY).contains(&format!("<{cid}>")),
        "an image the body no longer shows isn't sent"
    );
}

/// A file dropped on the window.
#[derive(Debug)]
struct Dropped(std::path::PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|error| error.to_string())
    }
}

/// In Gmail web, a reply to the first conversation with `html` and `parts` (inline images),
/// synced and the conversation opened.
fn open_gmail_web_draft(app: &mut Fixture, html: &str, parts: Vec<gmail::mime::Attachment>) {
    let thread = app.mail().inbox().threads()[0].id.clone();
    let gmail = app.gmail();
    app.block_on(async {
        let detail = gmail
            .gmail()
            .thread(
                &thread,
                Format::Metadata,
                &["Message-ID", "References", "Subject"],
            )
            .await
            .expect("threads.get");
        let last = detail.messages.last().expect("a message");
        let mut email = gmail::mime::Email {
            to: vec![gmail::mime::Mailbox::address("tom@brightmoor.co")],
            subject: gmail::mime::reply_subject(last.header("Subject").unwrap_or_default()),
            html: Some(html.to_owned()),
            attachments: parts,
            ..gmail::mime::Email::default()
        };
        email.reply_to_message(last);
        gmail
            .gmail()
            .create_draft(email.to_rfc822(), Some(&thread))
            .await
            .expect("drafts.create");
    });
    app.sync();
    app.key(Key::Enter);
}

#[test]
fn a_draft_written_in_gmail_shows_its_image_and_keeps_it() {
    let mut app = start("images-gmail-draft");
    let chart = gmail::mime::Attachment {
        filename: "chart.png".into(),
        content_type: "image/png".into(),
        data: screenshot(),
        content_id: Some("chart@brightmoor".into()),
    };
    open_gmail_web_draft(
        &mut app,
        "<div>Here's the chart:<br><img src=\"cid:chart@brightmoor\" width=\"200\" height=\"125\"></div>",
        vec![chart],
    );
    // R takes the draft up; its image loads from the Gmail message it's in.
    app.key(Key::R);
    app.wait(1.0);
    assert!(
        app.mail()
            .compose_body()
            .expect("the draft")
            .contains("src=\"cid:chart@brightmoor\"")
    );
    let files: Vec<u64> = std::fs::read_dir(app.mail().account().draft_files())
        .expect("the draft files")
        .filter_map(|entry| entry.ok()?.metadata().ok())
        .map(|meta| meta.len())
        .collect();
    assert!(
        files.contains(&(screenshot().len() as u64)),
        "its picture came from Gmail into the draft files: {files:?}"
    );
    app.type_text(" Thanks");
    app.wait(1.5);
    let draft = unquoted(&draft_raw(&app, REPLY));
    assert!(
        draft.contains("Content-ID: <chart@brightmoor>")
            && draft.contains("src=\"cid:chart@brightmoor\""),
        "Exo's save keeps it: {draft}"
    );
}

/// Serves a GIF for every request, and keeps the paths asked for.
struct ImageServer {
    port: u16,
    paths: Arc<Mutex<Vec<String>>>,
}

impl ImageServer {
    fn start() -> Self {
        const GIF: &[u8] = b"GIF89a\x01\x00\x01\x00\x80\x00\x00\xff\xff\xff\x00\x00\x00!\xf9\x04\x01\x00\x00\x00\x00,\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;";
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let port = listener.local_addr().expect("an address").port();
        let paths = Arc::new(Mutex::new(Vec::new()));
        let seen = paths.clone();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().map_while(Result::ok) {
                let mut request = [0; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                if let Some(path) = String::from_utf8_lossy(&request[..read])
                    .split_whitespace()
                    .nth(1)
                {
                    seen.lock().expect("the paths' lock").push(path.to_owned());
                }
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: image/gif\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    GIF.len()
                );
                let _ = stream
                    .write_all(head.as_bytes())
                    .and_then(|()| stream.write_all(GIF));
            }
        });
        Self { port, paths }
    }

    fn paths(&self) -> Vec<String> {
        self.paths.lock().expect("the paths' lock").clone()
    }
}

#[test]
fn a_drafts_remote_images_follow_the_images_setting() {
    for (images, loads) in [
        (RemoteContentBlocking::All, false),
        (RemoteContentBlocking::None, true),
    ] {
        let server = ImageServer::start();
        let mut app = start_with_settings(&format!("images-remote-{loads}"), &|settings| {
            settings.remote_content_blocking = Some(images)
        });
        let html = format!(
            "<div>Logo: <img src=\"http://127.0.0.1:{}/logo.gif\" width=\"40\" height=\"40\"></div>",
            server.port
        );
        open_gmail_web_draft(&mut app, &html, Vec::new());
        app.key(Key::R);
        app.wait(1.0);
        assert_eq!(
            server.paths().contains(&"/logo.gif".to_owned()),
            loads,
            "{images:?}: {:?}",
            server.paths()
        );
        if !loads {
            // ⌘K Show Images shows the focused draft's, for the session.
            app.press(Key::K, COMMAND);
            app.type_text("show images");
            app.key(Key::Enter);
            app.wait(1.0);
            assert!(
                server.paths().contains(&"/logo.gif".to_owned()),
                "shown: {:?}",
                server.paths()
            );
        }
    }
}
