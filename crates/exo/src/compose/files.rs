//! Files added to a draft: Gmail's blocked types and empty files are turned away, a batch that
//! would make the message too large for Gmail is refused, and the rest are copied into the
//! account's draft files.

use std::path::{Path, PathBuf};

use exo_store::{AttachmentSource, DraftAttachment};

/// Gmail's limit on a whole message.
const MAX_MESSAGE: u64 = 35_882_577;
/// What's left for headers and the like.
const HEADROOM: u64 = 256 * 1024;

/// Extensions Gmail won't send.
const BLOCKED: &[&str] = &[
    "ade", "adp", "apk", "bat", "chm", "cmd", "com", "cpl", "dll", "dmg", "exe", "hta", "ins",
    "isp", "jar", "js", "jse", "lib", "lnk", "mde", "msc", "msi", "msp", "mst", "nsh", "pif",
    "scr", "sct", "shb", "sys", "vb", "vbe", "vbs", "vxd", "wsc", "wsf", "wsh", "cab",
];

/// Why adding stopped or skipped files: an alert's title and text (no title is `Sorry!`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    pub title: Option<&'static str>,
    pub text: String,
}

/// The second paragraph of the empty and unreadable alerts.
const TRY_AGAIN: &str =
    "This can happen if you attach a file while it is being saved. Please try again.";

/// Adds the files at `paths` to a draft that has `kept` and would send `html`, copying them into
/// `folder`. Returns the new attachments and the alert to show, if any (later alerts replace
/// earlier ones).
pub(crate) fn add(
    paths: &[PathBuf],
    kept: &[DraftAttachment],
    html: &str,
    folder: &Path,
) -> (Vec<DraftAttachment>, Option<Notice>) {
    // A folder in a drop attaches nothing.
    if paths.iter().any(|path| path.is_dir()) {
        let text = "We can't yet attach folders. You can try attaching a compressed archive of the folder.".into();
        return (Vec::new(), Some(Notice { title: None, text }));
    }
    let named: Vec<(&PathBuf, String, Option<u64>)> = paths
        .iter()
        .map(|path| {
            (
                path,
                name(path),
                std::fs::metadata(path).ok().map(|meta| meta.len()),
            )
        })
        .collect();
    let (blocked, rest): (Vec<_>, Vec<_>) =
        named.into_iter().partition(|(_, name, _)| is_blocked(name));
    let (empty, rest): (Vec<_>, Vec<_>) =
        rest.into_iter().partition(|(_, _, size)| *size == Some(0));
    let names = |list: &[(&PathBuf, String, Option<u64>)]| {
        list.iter()
            .map(|(_, name, _)| name.clone())
            .collect::<Vec<_>>()
    };
    let mut notice = turned_away(&names(&blocked), &names(&empty));
    if rest.is_empty() {
        return (Vec::new(), notice);
    }
    let sizes = kept
        .iter()
        .map(|attachment| attachment.size)
        .chain(rest.iter().map(|(_, _, size)| size.unwrap_or(0)));
    if exceeds_gmail_limit(sizes, html) {
        let text =
            "This message is too large for Gmail. Shorten the message or remove attachments."
                .into();
        return (Vec::new(), Some(Notice { title: None, text }));
    }
    let mut added = Vec::new();
    let mut unreadable = Vec::new();
    for (path, name, _) in rest {
        let uuid = super::factory::uuid_v4();
        let copied =
            std::fs::create_dir_all(folder).and_then(|()| std::fs::copy(path, folder.join(&uuid)));
        match copied {
            Ok(size) => added.push(DraftAttachment {
                uuid,
                mime_type: mime_type(&name).into(),
                name,
                size,
                content_id: None,
                source: AttachmentSource::Local,
            }),
            Err(error) => {
                log::warn!("attaching a file: {error}");
                unreadable.push(name);
            }
        }
    }
    if !unreadable.is_empty() {
        notice = Some(match unreadable.as_slice() {
            [one] => self::unreadable(one),
            many => Notice {
                title: Some("Some files could not be opened…"),
                text: format!("{} files could not be opened.\n\n{TRY_AGAIN}", many.len()),
            },
        });
    }
    (added, notice)
}

/// An image put in the body: `bytes`, a `mime_type` file named `name`, copied into `folder` as the
/// inline attachment the body shows by Content-ID `cid`, unless the message would then be too
/// large for Gmail.
pub(crate) fn add_inline(
    name: &str,
    mime_type: &str,
    bytes: &[u8],
    cid: &str,
    kept: &[DraftAttachment],
    html: &str,
    folder: &Path,
) -> Result<DraftAttachment, Notice> {
    let size = bytes.len() as u64;
    if exceeds_gmail_limit(
        kept.iter().map(|attachment| attachment.size).chain([size]),
        html,
    ) {
        return Err(Notice {
            title: None,
            text: "This message is too large for Gmail. Shorten the message or remove attachments."
                .into(),
        });
    }
    let uuid = super::factory::uuid_v4();
    if let Err(error) =
        std::fs::create_dir_all(folder).and_then(|()| std::fs::write(folder.join(&uuid), bytes))
    {
        log::warn!("keeping an inline image: {error}");
        return Err(unreadable(name));
    }
    Ok(DraftAttachment {
        uuid,
        name: name.to_owned(),
        mime_type: mime_type.to_owned(),
        size,
        content_id: Some(cid.to_owned()),
        source: AttachmentSource::Local,
    })
}

/// Whether a file of this name goes in the body rather than on the draft when it's the only one
/// dropped, pasted or picked to insert (by its [`mime_type`]).
pub(crate) fn is_inline_image(name: &str) -> bool {
    matches!(
        mime_type(name),
        "image/jpeg" | "image/gif" | "image/png" | "image/bmp" | "image/webp"
    )
}

/// The alert for a file that couldn't be read.
pub(crate) fn unreadable(name: &str) -> Notice {
    Notice {
        title: Some("File could not be opened…"),
        text: format!("\"{name}\" could not be opened.\n\n{TRY_AGAIN}"),
    }
}

/// The alert for the files turned away as blocked or empty, if any were.
fn turned_away(blocked: &[String], empty: &[String]) -> Option<Notice> {
    let text = match (blocked, empty) {
        ([], []) => return None,
        ([_, ..], [_, ..]) => format!(
            "{} files have been blocked for security reasons or because they had no content.",
            blocked.len() + empty.len()
        ),
        ([one], []) => format!("\"{one}\" has been blocked for security reasons."),
        (many, []) => format!(
            "{} files have been blocked for security reasons.",
            many.len()
        ),
        ([], [one]) => {
            return Some(Notice {
                title: Some("File is empty…"),
                text: format!("\"{one}\" has no content.\n\n{TRY_AGAIN}"),
            });
        }
        ([], _) => {
            return Some(Notice {
                title: Some("Some files are empty…"),
                text: format!("Some of these files have no content.\n\n{TRY_AGAIN}"),
            });
        }
    };
    Some(Notice { title: None, text })
}

fn name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "untitled".into())
}

fn is_blocked(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, extension)| BLOCKED.contains(&extension.to_ascii_lowercase().as_str()))
}

/// Whether the message would be over Gmail's limit: each attachment as base64 with line breaks and
/// part headers, the HTML twice as quoted-printable (the text alternative is about as long), and
/// headroom.
pub(crate) fn exceeds_gmail_limit(sizes: impl IntoIterator<Item = u64>, html: &str) -> bool {
    let attachments: u64 = sizes
        .into_iter()
        .map(|size| {
            let base64 = 4 * size.div_ceil(3);
            base64 + 2 * base64.div_ceil(76) + 512
        })
        .sum();
    attachments + 2 * quoted_printable_size(html) + HEADROOM > MAX_MESSAGE
}

/// `text`'s size as quoted-printable.
fn quoted_printable_size(text: &str) -> u64 {
    let bytes: u64 = text
        .bytes()
        .map(|byte| match byte {
            b'\n' => 2,
            b'\t' => 1,
            b'=' => 3,
            32..=126 => 1,
            _ => 3,
        })
        .sum();
    let lines = bytes.div_ceil(75);
    bytes + 3 * lines.saturating_sub(1)
}

/// The type a browser gives a file by its extension (`File.type`), or
/// `application/octet-stream`.
pub(crate) fn mime_type(name: &str) -> &'static str {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" | "jfif" | "jif" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "heic" => "image/heic",
        "svg" => "image/svg+xml",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "htm" | "html" => "text/html",
        "ics" => "text/calendar",
        "json" => "application/json",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "key" => "application/x-iwork-keynote-sffkey",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "eml" => "message/rfc822",
        _ => "application/octet-stream",
    }
}

impl crate::Mail {
    /// ⌘V in a draft with Copy Attachments' files on the clipboard: they join it by reference to
    /// their Gmail parts, or, copied in another account, an alert says they can't. Returns whether
    /// the paste was theirs.
    pub(super) fn compose_paste_attachments(&mut self, ctx: &egui::Context, now: f64) -> bool {
        let Some(id) = self.compose.focus.clone() else {
            return false;
        };
        if !ctx.input(|input| {
            input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Paste(_)))
        }) {
            return false;
        }
        let Some(copied) = crate::attachment_files::pasted(ctx) else {
            return false;
        };
        ctx.input_mut(|input| {
            input
                .events
                .retain(|event| !matches!(event, egui::Event::Paste(_)))
        });
        if copied.account != self.account.email() {
            self.compose_alert_notice(
                &id,
                Notice {
                    title: None,
                    text: crate::attachment_files::OTHER_ACCOUNT.to_owned(),
                },
            );
            return true;
        }
        if let Some(open) = self.compose.get_mut(&id) {
            open.record
                .attachments
                .extend(copied.into_draft_attachments());
            open.touch(now);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("exo-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn blocked_and_empty_files_are_turned_away() {
        let dir = folder("blocked");
        let write = |name: &str, bytes: &[u8]| {
            let path = dir.join(name);
            std::fs::write(&path, bytes).expect("written");
            path
        };
        let plan = write("plan.csv", b"a,b");
        let tool = write("setup.EXE", b"MZ");
        let blank = write("blank.txt", b"");
        let files = dir.join("files");
        let (added, notice) = add(&[plan.clone(), tool.clone()], &[], "", &files);
        assert_eq!(added.len(), 1);
        assert_eq!(
            (
                added[0].name.as_str(),
                added[0].mime_type.as_str(),
                added[0].size
            ),
            ("plan.csv", "text/csv", 3)
        );
        assert_eq!(
            std::fs::read(files.join(&added[0].uuid)).expect("copied"),
            b"a,b"
        );
        assert_eq!(
            notice.expect("an alert").text,
            "\"setup.EXE\" has been blocked for security reasons."
        );
        let (added, notice) = add(std::slice::from_ref(&blank), &[], "", &files);
        assert!(added.is_empty());
        assert_eq!(notice.expect("an alert").title, Some("File is empty…"));
        let (_, notice) = add(&[tool, blank], &[], "", &files);
        assert_eq!(
            notice.expect("an alert").text,
            "2 files have been blocked for security reasons or because they had no content."
        );
        let (added, notice) = add(&[dir.clone(), plan], &[], "", &files);
        assert!(
            added.is_empty()
                && notice
                    .expect("an alert")
                    .text
                    .starts_with("We can't yet attach folders.")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_whole_message_must_fit_in_gmail() {
        assert!(!exceeds_gmail_limit([24 * 1024 * 1024], "<p>Hi</p>"));
        assert!(
            exceeds_gmail_limit([20 * 1024 * 1024, 6 * 1024 * 1024], "<p>Hi</p>"),
            "base64 grows files by a third"
        );
        assert_eq!(quoted_printable_size("a=b\n"), 1 + 3 + 1 + 2);
    }
}
