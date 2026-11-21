#![deny(
    clippy::correctness,
    clippy::suspicious,
    clippy::complexity,
    clippy::perf
)]
//! `gmail-mock`: serves the mock Google APIs.

use anyhow::Context;
use clap::Parser;

/// Mock Google APIs (Gmail, Calendar, People, OAuth) with a deterministic fixture mailbox.
#[derive(Debug, Parser)]
struct Args {
    /// Port to listen on.
    #[arg(long, default_value_t = 8787)]
    port: u16,
    /// Address to bind.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    /// Fixture seed.
    #[arg(long, default_value_t = gmail_mock::Config::default().seed)]
    seed: u64,
    /// The mock's "now", from which its clock runs: RFC 3339, optionally with a zone that
    /// also becomes the account's (`2026-11-17T09:00:00-05:00[America/New_York]`).
    #[arg(long, value_parser = parse_now)]
    now: Option<jiff::Zoned>,
    /// An RFC 822 message (`.eml`) to deliver to the inbox, at startup and on every reset.
    /// Repeatable.
    #[arg(long)]
    eml: Vec<std::path::PathBuf>,
    /// Milliseconds every API request waits before it's answered, like a real network.
    #[arg(long, default_value_t = 0)]
    latency: u64,
    /// Enforce a per-user Gmail quota of this many units a minute, as Gmail does (6000 is
    /// Gmail's). Without it, no quota is enforced.
    #[arg(long)]
    quota: Option<u32>,
    /// Give the account a profile photo of its own (People's `people/me`, userinfo's
    /// `picture`), served at `/__mock/photo/…`. Without it, it has Google's generated avatar.
    #[arg(long)]
    photo: bool,
}

/// Instants without a zone keep the default zone (America/New_York).
fn parse_now(text: &str) -> Result<jiff::Zoned, jiff::Error> {
    gmail_mock::parse_time(text, gmail_mock::Config::default().now.time_zone())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();
    let mut config = gmail_mock::Config {
        seed: args.seed,
        extra_messages: args
            .eml
            .iter()
            .map(|path| std::fs::read(path).with_context(|| format!("reading {}", path.display())))
            .collect::<anyhow::Result<_>>()?,
        latency: std::time::Duration::from_millis(args.latency),
        quota: args.quota,
        photo: args.photo,
        ..gmail_mock::Config::default()
    };
    if let Some(now) = args.now {
        config.now = now;
    }
    let listener = tokio::net::TcpListener::bind((args.host.as_str(), args.port))
        .await
        .with_context(|| format!("binding {}:{}", args.host, args.port))?;
    log::info!(
        "gmail-mock on http://{} (seed {}, now {}, latency {:?}, quota {:?}); dev token: {}",
        listener.local_addr()?,
        config.seed,
        config.now,
        config.latency,
        config.quota,
        gmail_mock::DEV_ACCESS_TOKEN
    );
    gmail_mock::serve(listener, &config).await?;
    Ok(())
}
