//! The app's side of attachments: what the fetches report each frame, the file notifications,
//! the open thread's cards (progress, thumbnails, `Download all`) and the viewer.

use std::path::{Path, PathBuf};

use egui::{Context, Rect};
use exo_store::{Attachment, AttachmentSource, DraftAttachment, MessageDetail};

use super::notifications::{Click, STEP};
use super::viewer::{Action as ViewerAction, Output as ViewerOutput, Viewer};
use super::{AttachmentFiles, Event, Failure, Target, Update};
use crate::account::Loader;
use crate::thread::ThreadView;

/// The offline alert's text for a file and for `Download all`.
pub(crate) const OFFLINE: &str =
    "This attachment is not available offline.\nPlease check your connection and try again.";
pub(crate) const OFFLINE_ALL: &str =
    "These attachments are not available offline.\nPlease check your connection and try again.";

impl AttachmentFiles {
    /// Brings the open thread's cards up to date: how far its clicked files have come, which
    /// messages are zipping, and the thumbnails that have arrived; and fetches the thumbnails it
    /// asks for (an image card's data loads as it's shown).
    pub(crate) fn update_thread(&mut self, loader: &Loader, thread: &mut ThreadView) {
        let progress = self
            .downloads
            .iter()
            .filter(|download| download.state == super::State::Running)
            .map(|download| (download.key.clone(), download.progress))
            .collect();
        let files = thread.files_mut();
        files.set_progress(progress);
        files.set_zipping(&self.zipping);
        while let Ok((key, image)) = self.thumbnails.1.try_recv() {
            files.set_thumbnail(&self.ctx, key, image);
        }
        for attachment in thread.thumbnail_requests() {
            let (sender, ctx) = (self.thumbnails.0.clone(), self.ctx.clone());
            let key = (attachment.message_id.clone(), attachment.part_id.clone());
            let pixels_per_point = ctx.pixels_per_point();
            loader.attachment(&attachment, move |bytes| {
                // Decoding takes a while for a photo; the account's tasks have better to do.
                std::thread::spawn(move || {
                    let image = bytes
                        .ok()
                        .and_then(|bytes| super::thumbnails::thumbnail(&bytes, pixels_per_point));
                    let _ = sender.send((key, image));
                    ctx.request_repaint();
                });
            });
        }
    }

    /// Brings a read-only snippet's cards up to date: whether its files are zipping.
    pub(crate) fn update_files(&self, files: &mut crate::thread::attachments::Files) {
        files.set_zipping(&self.zipping);
    }

    /// `Download all` under `message`'s cards, the `position`th in a conversation about
    /// `subject`: its files zipped into the download folder, the card list saying `Downloading…`
    /// until then. Nothing happens while it's zipping already.
    pub(crate) fn download_all(
        &mut self,
        loader: &Loader,
        message: &MessageDetail,
        subject: &str,
        position: usize,
        now: jiff::civil::DateTime,
    ) {
        let files = crate::thread::attachments::listed(message)
            .cloned()
            .map(Part::Gmail)
            .collect();
        self.zip(
            loader,
            &message.id,
            super::zip::name(subject, position),
            files,
            now,
        );
    }

    /// [`Self::download_all`] under a read-only snippet's cards (the message `id`), about
    /// `subject`: its files, kept in `folder` (the draft files) or still Gmail's.
    pub(crate) fn download_snippet_files(
        &mut self,
        loader: &Loader,
        id: &str,
        subject: &str,
        files: &[DraftAttachment],
        folder: &Path,
        now: jiff::civil::DateTime,
    ) {
        let parts = files
            .iter()
            .filter(|file| file.content_id.is_none())
            .filter_map(|file| {
                let kept = folder.join(&file.uuid);
                match &file.source {
                    _ if kept.is_file() => Some(Part::Kept(file.name.clone(), kept)),
                    AttachmentSource::Gmail {
                        message_id,
                        attachment_id,
                        part_id,
                    } => Some(Part::Gmail(Attachment {
                        message_id: message_id.clone(),
                        attachment_id: attachment_id.clone(),
                        part_id: part_id.clone(),
                        filename: file.name.clone(),
                        mime_type: file.mime_type.clone(),
                        size: u32::try_from(file.size).unwrap_or(u32::MAX),
                        content_id: None,
                        inline: false,
                    })),
                    AttachmentSource::Local => None,
                }
            });
        // The snippet is its thread's only message.
        self.zip(
            loader,
            id,
            super::zip::name(subject, 0),
            parts.collect(),
            now,
        );
    }

    /// `files` zipped as `name` into the download folder, cards `id` saying `Downloading…`
    /// meanwhile.
    fn zip(
        &mut self,
        loader: &Loader,
        id: &str,
        name: String,
        files: Vec<Part>,
        now: jiff::civil::DateTime,
    ) {
        if self.zipping.contains(id) {
            return;
        }
        let Some(folder) = crate::downloads::folder(&self.ctx) else {
            return;
        };
        self.zipping.insert(id.to_owned());
        let (sender, ctx, id) = (self.updates.0.clone(), self.ctx.clone(), id.to_owned());
        loader.run(
            move |store| async move {
                let mut fetched = Vec::with_capacity(files.len());
                for file in files {
                    fetched.push(match file {
                        Part::Gmail(file) => (
                            file.filename,
                            store
                                .part_data(
                                    &file.message_id,
                                    file.attachment_id.as_deref(),
                                    &file.part_id,
                                )
                                .await
                                .map_err(|error| {
                                    log::warn!("zipping files: {error}");
                                    Failure::of(&error)
                                })?,
                        ),
                        Part::Kept(name, path) => (
                            name,
                            std::fs::read(&path).map_err(|error| {
                                log::warn!("zipping files: {error}");
                                Failure::Fetch
                            })?,
                        ),
                    });
                }
                Ok::<_, Failure>(fetched)
            },
            move |fetched| {
                std::thread::spawn(move || {
                    let target = Target::Downloads(folder);
                    let saved = fetched.and_then(|files| {
                        target
                            .save(&name, &super::zip::archive(&files, &now))
                            .map_err(|error| target.failed(&error))
                    });
                    let _ = sender.send(Update::Zipped {
                        message: id,
                        name,
                        saved,
                    });
                    ctx.request_repaint();
                });
            },
        );
    }
}

/// A file going into a zip.
enum Part {
    /// A Gmail message's part.
    Gmail(Attachment),
    /// A file kept on disk, under the name it goes in as.
    Kept(String, PathBuf),
}

impl crate::Mail {
    /// Copy Attachments: the focused message's files on the clipboard, with the hint saying where
    /// they go.
    pub(crate) fn copy_attachments(&mut self) {
        let Some(thread) = &self.thread else { return };
        let Some(message) = thread.detail().and_then(|detail| {
            detail
                .messages
                .iter()
                .find(|message| Some(message.id.as_str()) == thread.focused())
        }) else {
            return;
        };
        if !super::has_copied(&message.attachments) {
            return;
        }
        let copied = super::copy::Copied::new(self.account.email(), &message.attachments);
        crate::clipboard::write_attachments(
            &self.ctx,
            &serde_json::to_string(&copied).expect("copied attachments serialize"),
        );
        self.commands.hint(
            exo_commands::Hint::new()
                .text("Attachments copied to clipboard. Hit")
                .keys(["⌘", "V"])
                .text("in a draft to paste them."),
        );
    }

    /// Takes in this frame's news of the clicked files: a fetch offline says so in the offline
    /// alert.
    pub(crate) fn attachment_files_poll(&mut self, now: f64) {
        for event in self.attachment_files.poll(now) {
            let text = match event {
                Event::Offline => OFFLINE,
                Event::OfflineAll => OFFLINE_ALL,
            };
            self.pickers.inform("Sorry!", text.to_owned());
        }
    }

    /// Draws the file notifications over the toast and does what a click on one asks.
    pub(crate) fn show_file_notifications(&mut self, ctx: &Context, page: Rect, now: f64) {
        let bottom = self.commands.toast_bottom()
            + if self.commands.toast_text().is_some() {
                STEP
            } else {
                0.0
            };
        match self
            .attachment_files
            .show_notifications(ctx, page, bottom, now)
        {
            Some(Click::Show(path)) => self.attachment_files.reveal(&path),
            Some(Click::Settings) => self.run(
                crate::Action::Settings(crate::settings::Action::Open(
                    crate::settings::Dialog::Downloads,
                )),
                now,
            ),
            Some(Click::Preview(path, name)) => self.attachment_files.show(path, name),
            None => {}
        }
    }

    /// Exo's viewer over `attachments`, from the `start`th.
    pub(crate) fn preview_attachments(&mut self, attachments: Vec<Attachment>, start: usize) {
        if attachments.is_empty() {
            return;
        }
        let now = self.ctx.input(|input| input.time);
        self.attachment_files.viewer = Some(Viewer::open(
            &self.ctx,
            &self.account.loader(),
            attachments,
            start,
            now,
        ));
    }

    /// A search result's file card: an image or a message in the viewer over the conversation's
    /// others, anything else in Quick Look, or downloaded.
    pub(crate) fn open_result_file(&mut self, file: Attachment, preview: bool) {
        if preview && super::is_previewable(&file) {
            let files: Vec<Attachment> = self
                .inbox
                .threads()
                .iter()
                .find(|thread| thread.files.contains(&file))
                .map(|thread| {
                    thread
                        .files
                        .iter()
                        .filter(|other| super::is_previewable(other))
                        .cloned()
                        .collect()
                })
                .unwrap_or_else(|| vec![file.clone()]);
            let start = files.iter().position(|other| *other == file).unwrap_or(0);
            self.preview_attachments(files, start);
        } else {
            self.attachment_files
                .open(&self.account.loader(), file, preview);
        }
    }

    /// The viewer has the keyboard.
    pub(crate) fn previewing(&self) -> bool {
        self.attachment_files
            .viewer
            .as_ref()
            .is_some_and(|viewer| !viewer.is_closing())
    }

    /// Does what the viewer's keys or clicks ask: a download goes as a click on the card's download
    /// icon does.
    pub(crate) fn viewer_run(&mut self, action: ViewerAction, now: f64) {
        let Some(viewer) = self.attachment_files.viewer.as_mut() else {
            return;
        };
        match action {
            ViewerAction::Download => {
                if let Some(attachment) = viewer.current().cloned() {
                    self.attachment_files
                        .open(&self.account.loader(), attachment, false);
                }
            }
            ViewerAction::SaveFile(n) => {
                if let Some((name, bytes)) = viewer.file(n) {
                    self.attachment_files.save_file(name, bytes);
                }
            }
            ViewerAction::Copy => viewer.copy(&self.ctx),
            action => viewer.run(action, now),
        }
    }

    /// Draws the viewer over `page`, if it's open.
    pub(crate) fn show_viewer(&mut self, ctx: &Context, page: Rect, now: f64) {
        let loader = self.account.loader();
        let Some(viewer) = self.attachment_files.viewer.as_mut() else {
            return;
        };
        let clicked = viewer.show(
            ctx,
            page,
            ctx.content_rect().height(),
            &loader,
            self.account.email(),
            now,
            &self.now,
        );
        if viewer.is_gone(now) {
            self.attachment_files.viewer = None;
        }
        match clicked {
            Some(ViewerOutput::Action(action)) => self.viewer_run(action, now),
            // A `mailto:` link's draft opens under the viewer, which goes to show it.
            Some(ViewerOutput::Link { url, .. })
                if url
                    .get(..7)
                    .is_some_and(|scheme| scheme.eq_ignore_ascii_case("mailto:")) =>
            {
                self.viewer_run(ViewerAction::Close, now);
                self.compose_mailto(&url, false);
            }
            Some(ViewerOutput::Link {
                url,
                background: true,
            }) => self.open_url_in_background(&url),
            Some(ViewerOutput::Link {
                url,
                background: false,
            }) => self.open_url(&url),
            Some(ViewerOutput::Drag(at)) => {
                if let Some(attachment) = self
                    .attachment_files
                    .viewer
                    .as_ref()
                    .and_then(Viewer::current)
                    .cloned()
                {
                    self.attachment_files
                        .drag(&loader, &attachment, at, ctx.zoom_factor());
                }
            }
            None => {}
        }
    }

    /// `Download all` under message `id` of the open conversation.
    pub(crate) fn download_all(&mut self, id: &str) {
        let Some(detail) = self.thread.as_ref().and_then(ThreadView::shared_detail) else {
            return;
        };
        let Some(position) = detail.messages.iter().position(|message| message.id == id) else {
            return;
        };
        let loader = self.account.loader();
        self.attachment_files.download_all(
            &loader,
            &detail.messages[position],
            &detail.subject,
            position,
            self.now.datetime(),
        );
    }

    /// Holds the `card`th card of the open conversation's last message downloading at
    /// `percent`, its bar and notification showing (for screenshots).
    pub fn hold_download(&mut self, card: usize, percent: f32) {
        let Some(detail) = self.thread.as_ref().and_then(ThreadView::shared_detail) else {
            return;
        };
        let Some(attachment) = detail
            .messages
            .last()
            .and_then(|message| crate::thread::attachments::listed(message).nth(card))
            .cloned()
        else {
            return;
        };
        self.attachment_files.hold(&attachment, percent);
    }

    /// The file the viewer shows, while it's open (for tests).
    pub fn previewed(&self) -> Option<&str> {
        self.attachment_files
            .viewer
            .as_ref()
            .filter(|viewer| !viewer.is_closing())
            .and_then(Viewer::current)
            .map(|file| file.filename.as_str())
    }

    /// The file notifications' texts, oldest first (for tests).
    pub fn file_notifications(&self) -> Vec<String> {
        self.attachment_files
            .downloads()
            .iter()
            .map(|download| match &download.state {
                super::State::Failed(failure) => failure.text(&download.name),
                _ => download.name.clone(),
            })
            .collect()
    }
}
