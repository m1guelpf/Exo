//! Renders one message of a Gmail thread to a PNG, the way Exo would show its body.
//!
//! ```sh
//! cargo run -p gmail-mock -- --port 8791 &
//! cargo run --release -p exo-mail --example render_message -- \
//!     --mock http://127.0.0.1:8791 --thread 1a1e9b426200015e --out body.png
//! ```
//!
//! Remote images load with reqwest and inline (`cid:`) images from the message's attachments;
//! the PNG is painted once everything has arrived.

use std::{
    collections::HashMap,
    future::IntoFuture as _,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use clap::Parser;
use exo_mail::{
    BodyView, Done, Fetch, Fonts, Message, Region, Render, Theme, ViewOptions, html_from_payload,
    render,
};
use gmail::{Client, Endpoints, Format, MessagePart, StaticToken};

#[derive(Parser)]
struct Args {
    /// gmail-mock (or any Gmail API) origin.
    #[arg(long, default_value = "http://127.0.0.1:8787")]
    mock: url::Url,
    /// Access token.
    #[arg(long, default_value = "ya29.mock-dev-token")]
    token: String,
    /// The thread to open.
    #[arg(long)]
    thread: String,
    /// Which message, 0-based (default: the last).
    #[arg(long)]
    message: Option<usize>,
    /// Message column width in CSS px.
    #[arg(long, default_value_t = 640.0)]
    width: f32,
    /// Device pixels per CSS px.
    #[arg(long, default_value_t = 2.0)]
    scale: f32,
    /// The fractional part of the body's on-screen x, to match a screenshot.
    #[arg(long, default_value_t = 0.0)]
    phase_x: f32,
    /// Dark theme.
    #[arg(long)]
    dark: bool,
    /// Expand the quoted part.
    #[arg(long)]
    show_quoted: bool,
    /// Also write the render's HTML parts next to the PNG.
    #[arg(long)]
    dump: bool,
    /// Output PNG.
    #[arg(long)]
    out: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let runtime = tokio::runtime::Runtime::new()?;
    let client = Arc::new(Client::new(
        Endpoints::mock(&args.mock),
        StaticToken(args.token.clone()),
    ));
    let thread = runtime.block_on(
        client
            .gmail()
            .thread(&args.thread, Format::Full, &[])
            .into_future(),
    )?;

    let messages: Vec<(Message, String, Option<String>)> = thread
        .messages
        .iter()
        .map(|raw| {
            let payload = raw.payload.clone().unwrap_or_default();
            let header = |name: &str| {
                payload
                    .headers
                    .iter()
                    .find(|h| h.name.eq_ignore_ascii_case(name))
                    .map(|h| h.value.clone())
            };
            let (from_name, from_email) = split_address(&header("From").unwrap_or_default());
            let message = Message {
                html: html_from_payload(&payload),
                from_email,
                from_name,
                rfc822_id: header("Message-ID").unwrap_or_default(),
                attachment_names: attachments(&payload)
                    .into_iter()
                    .map(|(name, _, _)| name)
                    .collect(),
            };
            (
                message,
                header("Message-ID").unwrap_or_default(),
                header("In-Reply-To"),
            )
        })
        .collect();

    // Each message is rendered against the one it replies to, oldest first.
    let started = Instant::now();
    let mut renders: Vec<Render> = Vec::new();
    for (index, (message, _, in_reply_to)) in messages.iter().enumerate() {
        let parent = in_reply_to
            .as_ref()
            .and_then(|id| {
                messages[..index]
                    .iter()
                    .position(|(_, message_id, _)| message_id == id)
            })
            .or(index.checked_sub(1));
        let replying_to = parent.map(|p| (&messages[p].0, &renders[p]));
        renders.push(render(message, replying_to, &[]));
    }
    let pipeline = started.elapsed();
    let index = args.message.unwrap_or(messages.len() - 1);
    let rendered = &renders[index];
    if args.dump {
        std::fs::write(
            args.out.with_extension("unquoted.html"),
            &rendered.unquoted_html,
        )?;
        std::fs::write(
            args.out.with_extension("quoted.html"),
            &rendered.quoted_html,
        )?;
    }

    let fonts = Fonts::load();
    let raw = &thread.messages[index];
    let fetcher = Arc::new(Fetcher {
        runtime: runtime.handle().clone(),
        client,
        message_id: raw.id.clone().unwrap_or_default(),
        attachments: attachments(raw.payload.as_ref().unwrap_or(&MessagePart::default()))
            .into_iter()
            .filter_map(|(_, cid, id)| Some((cid?, id?)))
            .collect(),
        pending: Arc::new(AtomicUsize::new(0)),
    });
    let options = ViewOptions {
        theme: if args.dark { Theme::Dark } else { Theme::Light },
        show_quoted: args.show_quoted,
        ..ViewOptions::default()
    };
    let started = Instant::now();
    let mut view = BodyView::new(rendered, options, &fonts, Some(fetcher.clone()));
    let mut height = view.layout(args.width, args.scale);
    let layout = started.elapsed();
    let deadline = Instant::now() + Duration::from_secs(10);
    while fetcher.pending.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    if view.poll() {
        height = view.layout(args.width, args.scale);
    }
    let started = Instant::now();
    let region = Region {
        top: 0.0,
        height,
        left: 0.0,
        width: view.content_width(),
        phase: [args.phase_x, 0.0],
    };
    // The message card: `--origin-color-background-elevated-default`.
    let image = view.paint(
        region,
        if args.dark {
            [0x38, 0x37, 0x35, 255]
        } else {
            [255; 4]
        },
    );
    let paint = started.elapsed();
    image::save_buffer(
        &args.out,
        &image.rgba,
        image.width,
        image.height,
        image::ColorType::Rgba8,
    )?;
    eprintln!(
        "{} messages through the pipeline in {pipeline:.1?}; message {index}: {height} px tall (content {} px wide), \
         marketing {}/{}, quoted part {}; view + layout {layout:.1?}, paint {paint:.1?}",
        messages.len(),
        view.content_width(),
        rendered.unquoted_is_marketing,
        rendered.quoted_is_marketing,
        rendered.has_quoted_content,
    );
    Ok(())
}

/// Loads remote images over HTTP and `cid:` images from the message's attachments.
struct Fetcher {
    runtime: tokio::runtime::Handle,
    client: Arc<Client>,
    message_id: String,
    /// Content-ID → attachment ID.
    attachments: HashMap<String, String>,
    pending: Arc<AtomicUsize>,
}

impl Fetch for Fetcher {
    fn fetch(&self, url: &str, done: Done) {
        let attachment = url
            .strip_prefix("cid:")
            .and_then(|cid| self.attachments.get(cid))
            .cloned();
        let (client, message_id, url, pending) = (
            self.client.clone(),
            self.message_id.clone(),
            url.to_owned(),
            self.pending.clone(),
        );
        pending.fetch_add(1, Ordering::SeqCst);
        self.runtime.spawn(async move {
            let bytes = match attachment {
                Some(id) => client
                    .gmail()
                    .attachment(&message_id, &id)
                    .await
                    .ok()
                    .and_then(|body| body.data),
                None if url.starts_with("http") => match reqwest::get(&url).await {
                    Ok(response) => response.bytes().await.ok().map(|b| b.to_vec()),
                    Err(_) => None,
                },
                None => None,
            };
            if let Some(bytes) = bytes {
                done.finish(bytes);
            }
            pending.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

/// `Name <address>` → (name, address).
fn split_address(header: &str) -> (String, String) {
    match (header.rfind('<'), header.rfind('>')) {
        (Some(open), Some(close)) if open < close => (
            header[..open].trim().trim_matches('"').to_owned(),
            header[open + 1..close].trim().to_owned(),
        ),
        _ => (String::new(), header.trim().to_owned()),
    }
}

/// Every attachment under a part: (file name, Content-ID, attachment ID).
fn attachments(part: &MessagePart) -> Vec<(String, Option<String>, Option<String>)> {
    let mut found = Vec::new();
    let name = part.filename.clone().unwrap_or_default();
    let id = part
        .body
        .as_ref()
        .and_then(|body| body.attachment_id.clone());
    if !name.is_empty() || id.is_some() {
        let cid = part
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("Content-ID"))
            .map(|h| h.value.trim_matches(['<', '>']).to_owned());
        found.push((name, cid, id));
    }
    for child in &part.parts {
        found.extend(attachments(child));
    }
    found
}
