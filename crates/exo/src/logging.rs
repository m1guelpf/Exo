//! Exo's log: every line to stderr and, for the running Exo, to `~/Library/Logs/Exo/Exo.log`,
//! macOS's place for an app's logs (Console lists it). Each run starts the file afresh and keeps
//! the run before's as `Exo.previous.log`; a run whose file reaches [`MAX_LOG`] starts it afresh
//! again, its lines so far becoming `Exo.previous.log`. Panics go to `panic.log` there too
//! (fastframe-log's hook).
//!
//! It's fastframe-log's logger (its line, filter, file and permissions) with that cap, which
//! fastframe-log doesn't have.
//!
//! No line may carry mail, tokens or addresses: call sites don't format them, and every line
//! goes through [`redacted`] in case an error quotes one (a store's path is its account's
//! address, an HTTP error quotes its URL).

use std::io::Write;
use std::path::{Path, PathBuf};

use fastframe_log::redact;

/// The name in the start line and the panic log, and the start line's target, which [`FILTER`]
/// keeps.
const APP: &str = "Exo";

/// The filter without `RUST_LOG`: Exo's own warnings (why a download, a sync or a send failed),
/// every crate's errors, and the start line; other crates' warnings (wgpu's, winit's) are noise.
const FILTER: &str = "error,Exo=info,exo=warn,exo_auth=warn,exo_commands=warn,exo_editor=warn,exo_mail=warn,exo_store=warn,exo_ui=warn,gmail=warn";

/// The most `Exo.log` grows to before it starts afresh: the two files hold twice this at most.
const MAX_LOG: u64 = 10 << 20;

/// The folder Exo's logs go in: `~/Library/Logs/Exo`.
pub fn folder() -> Option<PathBuf> {
    std::env::home_dir().map(|home| home.join("Library/Logs").join(APP))
}

/// Starts logging to stderr and, given `folder`, to `Exo.log` and `panic.log` in it. Call once,
/// first thing; only the running Exo passes a folder, since the file starts afresh each run.
pub fn init(folder: Option<&Path>) {
    let mut builder = env_logger::Builder::new();
    builder.parse_filters(std::env::var("RUST_LOG").as_deref().unwrap_or(FILTER));
    let mut unkept = None;
    if let Some(folder) = folder {
        // The place a panic happened, and its message without addresses or links: Exo's are
        // `expect`s and the like, which quote no mail.
        fastframe_log::log_panics_with(
            folder.join("panic.log"),
            APP,
            env!("CARGO_PKG_VERSION"),
            fastframe_log::PanicMessage::Redacted(redacted),
        );
        let path = folder.join(format!("{APP}.log"));
        match Capped::create(path, MAX_LOG) {
            Ok(file) => {
                builder.target(env_logger::Target::Pipe(Box::new(file)));
            }
            Err(error) => unkept = Some(error),
        }
    }
    builder.format(|buffer, record| {
        let message = record.args().to_string();
        let message = if private(&message) {
            redacted(&message)
        } else {
            message
        };
        writeln!(
            buffer,
            "[{} {} {}] {message}",
            buffer.timestamp(),
            record.level(),
            record.target()
        )
    });
    let logger = builder.build();
    let level = logger.filter();
    if let Err(error) = log::set_boxed_logger(Box::new(logger)) {
        return eprintln!("logging: {error}");
    }
    log::set_max_level(level);
    if let Some(error) = unkept {
        log::warn!("not keeping a log file: {error}");
    }
    log::info!(target: APP, "Starting {APP} {} on {} ({})", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH);
}

/// The run's log file, which starts afresh past `max` bytes, keeping what it had as the
/// previous log. Every line goes to stderr too, best effort: a launch from the Finder has none.
struct Capped {
    path: PathBuf,
    file: std::fs::File,
    written: u64,
    max: u64,
}

impl Capped {
    /// Starts the log at `path` afresh, the last run's kept as [`fastframe_log::previous_log`].
    fn create(path: PathBuf, max: u64) -> std::io::Result<Self> {
        if let Some(folder) = path.parent() {
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true);
            std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
            builder.create(folder)?;
        }
        let file = Self::start(&path)?;
        Ok(Self {
            path,
            file,
            written: 0,
            max,
        })
    }

    /// Moves the log at `path`, if any, to the previous log, and makes a new one only the user
    /// can read.
    fn start(path: &Path) -> std::io::Result<std::fs::File> {
        // The earlier lines are worth more than a failed rename: the new file is made either way.
        let _ = std::fs::rename(path, fastframe_log::previous_log(path));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        options.open(path)
    }
}

impl Write for Capped {
    /// Writes `line`, one record: env_logger writes each whole.
    fn write(&mut self, line: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(line);
        if self.written > 0 && self.written + line.len() as u64 > self.max {
            self.file = Self::start(&self.path)?;
            self.written = 0;
        }
        self.file.write_all(line)?;
        self.written += line.len() as u64;
        Ok(line.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let _ = std::io::stderr().flush();
        self.file.flush()
    }
}

/// Whether `word` could be an address or a link (`a@b.c`, `mailto:…`, a store's path, `https://…`).
fn is_private(word: &str) -> bool {
    word.contains('@') || redact::is_link(word)
}

/// Whether `message` has a word [`is_private`] keeps out.
fn private(message: &str) -> bool {
    message.split_whitespace().any(is_private)
}

/// `message` with its addresses and links replaced by `<link>`.
fn redacted(message: &str) -> String {
    redact::words(message, is_private)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_paths_and_links_stay_out_of_the_log() {
        let error = "opening: applying 0003_x.sql to /Users/me/Library/Application Support/Exo/accounts/maya@brightmoor.co.sqlite3 failed";
        assert!(private(error));
        assert_eq!(
            redacted(error),
            "opening: applying 0003_x.sql to /Users/me/Library/Application <link> failed"
        );
        assert_eq!(
            redacted(
                "error sending request for url (https://gmail.googleapis.com/gmail/v1/users/me/threads?q=from%3Abo)"
            ),
            "error sending request for url <link>"
        );
        assert_eq!(
            redacted("turned down \"open mailto:bo@example.com\""),
            "turned down \"open <link>"
        );
        assert!(
            !private("sync: Gmail is unreachable: connection reset"),
            "kept as it is"
        );
    }

    #[test]
    fn past_its_cap_the_log_starts_afresh_and_keeps_what_it_had() {
        let folder = std::env::temp_dir().join(format!("exo-log-cap-{}", std::process::id()));
        let path = folder.join("Exo.log");
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("a folder");
        std::fs::write(&path, "the last run\n").expect("writes");
        let mut log = Capped::create(path.clone(), 20).expect("opens");
        let previous = fastframe_log::previous_log(&path);
        assert_eq!(
            std::fs::read_to_string(&previous).expect("kept"),
            "the last run\n"
        );
        for line in ["one 123456\n", "two 1234\n", "three 123\n"] {
            log.write_all(line.as_bytes()).expect("writes");
        }
        assert_eq!(
            std::fs::read_to_string(&previous).expect("kept"),
            "one 123456\ntwo 1234\n",
            "whole lines, up to the cap"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("reads"),
            "three 123\n"
        );
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path)
                .expect("there")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_start_line_and_exos_warnings_are_kept() {
        use log::Log as _;
        let logger = env_logger::Builder::new().parse_filters(FILTER).build();
        let enabled = |level, target| {
            logger.enabled(&log::Metadata::builder().level(level).target(target).build())
        };
        assert!(
            enabled(log::Level::Info, APP),
            "fastframe-log's `Starting Exo …` line"
        );
        assert!(enabled(log::Level::Warn, "exo::account"));
        assert!(!enabled(log::Level::Info, "exo::account"));
        assert!(!enabled(log::Level::Warn, "wgpu_core"));
    }
}
