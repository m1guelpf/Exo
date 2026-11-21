//! The Images setting (`remoteContentBlocking`) end to end: what a message's body asks the
//! network for under each choice, counted by a server on this Mac (which Exo loads from only
//! because gmail-mock is here too), ⌘K Show Images, and the header's eye.

mod support;

use std::{
    io::{Read as _, Write as _},
    net::TcpListener,
    sync::{Arc, Mutex},
};

use egui::{Key, Modifiers};
use exo_store::RemoteContentBlocking;
use support::{Fixture, start_with_settings};

/// A 1×1 GIF.
const GIF: &[u8] = b"GIF89a\x01\x00\x01\x00\x80\x00\x00\xff\xff\xff\x00\x00\x00!\xf9\x04\x01\x00\x00\x00\x00,\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;";

/// Serves every request a GIF, and keeps the paths asked for.
struct ImageServer {
    port: u16,
    paths: Arc<Mutex<Vec<String>>>,
}

impl ImageServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let port = listener.local_addr().expect("an address").port();
        let paths = Arc::new(Mutex::new(Vec::new()));
        let seen = paths.clone();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().map_while(Result::ok) {
                let mut request = [0; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let request = String::from_utf8_lossy(&request[..read]).into_owned();
                if let Some(path) = request.split_whitespace().nth(1) {
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

    /// The paths asked for so far, sorted.
    fn paths(&self) -> Vec<String> {
        let mut paths = self.paths.lock().expect("the paths' lock").clone();
        paths.sort();
        paths.dedup();
        paths
    }
}

const SUBJECT: &str = "Photos from the hike";

/// Opens the app with Images set to `images`, delivers a message with a photo, a CSS
/// background and a read receipt from `server`, and opens it.
fn open_with(name: &str, images: RemoteContentBlocking, server: &ImageServer) -> Fixture {
    let mut app = start_with_settings(name, &|settings| {
        settings.remote_content_blocking = Some(images)
    });
    let base = format!("http://127.0.0.1:{}", server.port);
    let html = format!(
        "<p>Here they are:</p><p><img src=\"{base}/photo.gif\" width=\"40\" height=\"40\"></p>\
         <div style=\"background-image:url({base}/background.gif);height:20px\">The trail map is on the back.</div>\
         <img src=\"{base}/track/open.php?u=7\" width=\"1\" height=\"1\">"
    );
    let delivery = serde_json::json!({ "from": "Liam O'Connell <liam.oconnell@fastmail.com>", "subject": SUBJECT, "html": html });
    let deliver = app.mock_url().join("__mock/deliver").expect("a URL");
    app.block_on(async {
        reqwest::Client::new()
            .post(deliver)
            .json(&delivery)
            .send()
            .await?
            .error_for_status()
    })
    .expect("delivered");
    app.sync();
    while app.cursor() != SUBJECT {
        app.key(Key::K);
    }
    app.key(Key::Enter);
    app
}

const CONTENT: [&str; 2] = ["/background.gif", "/photo.gif"];
const RECEIPT: &str = "/track/open.php?u=7";

#[test]
fn show_all_images_loads_everything_and_the_read_receipt() {
    let server = ImageServer::start();
    let _app = open_with("images-none", RemoteContentBlocking::None, &server);
    assert_eq!(server.paths(), [CONTENT[0], CONTENT[1], RECEIPT]);
}

#[test]
fn blocking_tracking_pixels_loads_the_images_but_not_the_read_receipt() {
    let server = ImageServer::start();
    let mut app = open_with(
        "images-reduce-tracking",
        RemoteContentBlocking::ReduceTracking,
        &server,
    );
    assert_eq!(server.paths(), CONTENT);
    // Shift-H's header: the eye after From takes the pointer (`cursor: default`) where the
    // header's text would show the I-beam.
    app.press(Key::H, Modifiers::SHIFT);
    assert_eq!(
        app.hover(EYE),
        egui::CursorIcon::Default,
        "Email tracking blocked"
    );
}

#[test]
fn blocking_remote_images_loads_nothing_until_show_images() {
    let server = ImageServer::start();
    let mut app = open_with("images-all", RemoteContentBlocking::All, &server);
    assert_eq!(
        server.paths(),
        Vec::<String>::new(),
        "nothing remote, not even the CSS background"
    );
    app.press(Key::K, Modifiers::MAC_CMD.plus(Modifiers::COMMAND));
    app.type_text("show images");
    app.key(Key::Enter);
    assert_eq!(
        server.paths(),
        CONTENT,
        "the message's images, still without the read receipt"
    );
}

#[test]
fn the_eye_shows_only_when_read_receipts_are_blocked() {
    let server = ImageServer::start();
    let mut app = open_with("images-eye", RemoteContentBlocking::None, &server);
    app.press(Key::H, Modifiers::SHIFT);
    assert_eq!(
        app.hover(EYE),
        egui::CursorIcon::Text,
        "no eye: the header's text"
    );
}

/// The middle of the eye after `Liam O'Connell <liam.oconnell@fastmail.com>` in the full header
/// at 1280×800.
const EYE: egui::Pos2 = egui::pos2(617.0, 133.0);
