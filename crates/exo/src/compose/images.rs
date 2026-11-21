//! The pixels of the images in the drafts' bodies: a draft's own inline attachments, from the
//! draft files or the Gmail message they're part of, `data:` URLs, and remote images as the
//! Images setting allows. Each editor asks for what it shows; the loads run off the UI thread and
//! come back here.

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::mpsc::{Receiver, Sender, channel},
};

use exo_editor::{ImageLoad, ImageSource, Picture};
use exo_store::{AttachmentSource, DraftAttachment, RemoteContentBlocking};

use super::draft::Open;
use crate::account::Loader;

/// The longest side an image is drawn from: a bigger file is scaled down for its texture, which
/// the editor never draws wider than the window anyway.
const MAX_SIDE: u32 = 2048;
/// Pixels per inch in a metre (`INCHES_PER_METER`), for a PNG's resolution.
const INCHES_PER_METER: f32 = 39.370_08;

/// An image loaded for a draft.
struct Loaded {
    draft: String,
    key: String,
    load: ImageLoad,
}

/// The loads in flight, the Images setting they were made under, and the drafts whose remote
/// images Show Images shows whatever it says, for the session.
pub(crate) struct Loads {
    sender: Sender<Loaded>,
    receiver: Receiver<Loaded>,
    setting: Option<RemoteContentBlocking>,
    shown: HashSet<String>,
}

impl Default for Loads {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Self {
            sender,
            receiver,
            setting: None,
            shown: HashSet::new(),
        }
    }
}

/// What a load needs from the account.
pub(crate) struct Sources<'a> {
    pub loader: &'a Loader,
    pub draft_files: &'a std::path::Path,
    pub setting: RemoteContentBlocking,
    pub ctx: &'a egui::Context,
}

impl Loads {
    pub(crate) fn is_shown(&self, draft: &str) -> bool {
        self.shown.contains(draft)
    }

    pub(crate) fn show(&mut self, draft: &str) {
        self.shown.insert(draft.to_owned());
    }

    /// Hands each draft the images that came, and starts loading what each asks for. A change of
    /// the Images setting has every remote image asked for again.
    pub(crate) fn update(&mut self, drafts: &mut [Open], sources: &Sources<'_>) {
        if self
            .setting
            .replace(sources.setting)
            .is_some_and(|before| before != sources.setting)
        {
            // A new setting forgets the drafts Show Images was used on.
            self.shown.clear();
            for open in drafts.iter_mut() {
                open.editor.reload_remote_images();
            }
        }
        while let Ok(loaded) = self.receiver.try_recv() {
            if let Some(open) = drafts.iter_mut().find(|open| open.id == loaded.draft) {
                open.editor.set_image(&loaded.key, loaded.load);
            }
        }
        for index in 0..drafts.len() {
            for (source, element) in drafts[index].editor.image_requests() {
                if let ImageSource::Cid(cid) = &source
                    && attachment(&drafts[index].record.attachments, cid).is_none()
                {
                    adopt(drafts, index, cid, sources.draft_files);
                }
                self.load(&drafts[index], source, &element, sources);
            }
        }
    }

    fn load(
        &self,
        open: &Open,
        source: ImageSource,
        element: &exo_editor::Element,
        sources: &Sources<'_>,
    ) {
        let key = source.key();
        let (sender, ctx, draft) = (self.sender.clone(), sources.ctx.clone(), open.id.clone());
        let finish = move |load: ImageLoad| {
            let _ = sender.send(Loaded { draft, key, load });
            ctx.request_repaint();
        };
        match source {
            ImageSource::Cid(cid) => match attachment(&open.record.attachments, &cid) {
                Some(attachment) => load_attachment(attachment, sources, finish),
                None => finish(ImageLoad::Broken),
            },
            ImageSource::Data(url) => decode_off_thread(exo_mail::decode_data_url(&url), finish),
            // A read receipt in a quote (Exo quotes a message as it came, receipts and all) is
            // never loaded, nor drawn blocked: the quote shows as if it had none.
            ImageSource::Remote(_) if is_read_receipt(element) => finish(ImageLoad::Broken),
            ImageSource::Remote(_)
                if sources.setting == RemoteContentBlocking::All
                    && !self.shown.contains(&open.id) =>
            {
                finish(ImageLoad::Blocked)
            }
            ImageSource::Remote(url) => sources
                .loader
                .url(&url, move |bytes| decode_off_thread(bytes, finish)),
        }
    }
}

/// An image pasted in from another open draft, whose HTML names that draft's inline attachment:
/// draft `index` takes a copy of it as its own inline attachment.
fn adopt(drafts: &mut [Open], index: usize, cid: &str, folder: &std::path::Path) {
    let Some(found) = drafts
        .iter()
        .enumerate()
        .filter(|(other, _)| *other != index)
        .find_map(|(_, open)| attachment(&open.record.attachments, cid))
        .cloned()
    else {
        return;
    };
    let copy = DraftAttachment {
        uuid: super::factory::uuid_v4(),
        ..found.clone()
    };
    // A quoted message's part stays in its message; a file the user added is copied.
    if copy.source == AttachmentSource::Local
        && let Err(error) = std::fs::copy(folder.join(&found.uuid), folder.join(&copy.uuid))
    {
        log::warn!("copying a pasted inline image: {error}");
        return;
    }
    drafts[index].record.attachments.push(copy);
}

/// The draft's inline attachment `cid` names (Content-IDs compare without case).
fn attachment<'a>(attachments: &'a [DraftAttachment], cid: &str) -> Option<&'a DraftAttachment> {
    attachments.iter().find(|attachment| {
        attachment
            .content_id
            .as_deref()
            .is_some_and(|id| id.eq_ignore_ascii_case(cid))
    })
}

/// An attachment's bytes: from the draft files, or from its Gmail message into them, as sending
/// takes them.
fn load_attachment(
    attachment: &DraftAttachment,
    sources: &Sources<'_>,
    finish: impl FnOnce(ImageLoad) + Send + 'static,
) {
    let path: PathBuf = sources.draft_files.join(&attachment.uuid);
    match &attachment.source {
        AttachmentSource::Gmail {
            message_id,
            attachment_id,
            part_id,
        } if !path.is_file() => {
            let part = exo_store::Attachment {
                message_id: message_id.clone(),
                attachment_id: attachment_id.clone(),
                part_id: part_id.clone(),
                filename: attachment.name.clone(),
                mime_type: attachment.mime_type.clone(),
                size: u32::try_from(attachment.size).unwrap_or(u32::MAX),
                content_id: attachment.content_id.clone(),
                inline: true,
            };
            sources.loader.attachment(&part, move |bytes| {
                if let Ok(bytes) = &bytes {
                    let kept = path
                        .parent()
                        .map_or(Ok(()), std::fs::create_dir_all)
                        .and_then(|()| std::fs::write(&path, bytes));
                    if let Err(error) = kept {
                        log::warn!("keeping a draft's inline image: {error}");
                    }
                }
                decode_off_thread(bytes.ok(), finish);
            });
        }
        _ => {
            std::thread::spawn(move || {
                finish(
                    std::fs::read(&path)
                        .ok()
                        .map_or(ImageLoad::Broken, |bytes| decode(&bytes)),
                )
            });
        }
    }
}

fn decode_off_thread(bytes: Option<Vec<u8>>, finish: impl FnOnce(ImageLoad) + Send + 'static) {
    std::thread::spawn(move || finish(bytes.map_or(ImageLoad::Broken, |bytes| decode(&bytes))));
}

/// An image file's pixels (the first frame of an animation), scaled down past [`MAX_SIDE`], and
/// its resolution; an animation's file, for the editor to play.
pub(crate) fn decode(bytes: &[u8]) -> ImageLoad {
    let Ok(image) = image::load_from_memory(bytes) else {
        return ImageLoad::Broken;
    };
    let natural = [image.width() as usize, image.height() as usize];
    let image = if image.width().max(image.height()) > MAX_SIDE {
        image.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let rgba = image.into_rgba8();
    let pixels = egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    );
    ImageLoad::Loaded(Picture {
        pixels,
        natural,
        retina: retina(bytes),
        file: std::sync::Arc::from(bytes),
    })
}

/// The image's retina factor: its resolution in multiples of 72 dpi, rounded: a PNG's `pHYs`
/// (pixels a metre), a JPEG's EXIF `XResolution` when its `ResolutionUnit` is inches; 1 for
/// anything else.
fn retina(bytes: &[u8]) -> u32 {
    let dpi = png_dpi(bytes).or_else(|| jpeg_dpi(bytes)).unwrap_or(72.0);
    (dpi / 72.0).round() as u32
}

fn png_dpi(bytes: &[u8]) -> Option<f32> {
    let mut rest = bytes.strip_prefix(b"\x89PNG\r\n\x1a\n")?;
    while rest.len() >= 12 {
        let length = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
        let (kind, data) = (&rest[4..8], &rest[8..]);
        if kind == b"IDAT" || data.len() < length {
            break;
        }
        if kind == b"pHYs" && length == 9 && data[8] == 1 {
            return Some(
                u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as f32 / INCHES_PER_METER,
            );
        }
        rest = &data[(length + 4).min(data.len())..];
    }
    None
}

/// A JPEG's dpi: from the `Exif` APP1 segment's first IFD, `XResolution` (0x011a) when
/// `ResolutionUnit` (0x0128) is 2, inches; 72 for another unit; none without the tag.
fn jpeg_dpi(bytes: &[u8]) -> Option<f32> {
    let mut rest = bytes.strip_prefix(&[0xff, 0xd8])?;
    let tiff = loop {
        let [0xff, marker, high, low, ..] = *rest else {
            return None;
        };
        let length = usize::from(u16::from_be_bytes([high, low]));
        // The image data starts: no more metadata.
        if marker == 0xda || length < 2 || rest.len() < 2 + length {
            return None;
        }
        let segment = &rest[4..2 + length];
        if marker == 0xe1
            && let Some(tiff) = segment.strip_prefix(b"Exif\0\0")
        {
            break tiff;
        }
        rest = &rest[2 + length..];
    };
    let little = tiff.starts_with(b"II");
    let u16_at = |at: usize| {
        tiff.get(at..at + 2).map(|b| {
            if little {
                u16::from_le_bytes([b[0], b[1]])
            } else {
                u16::from_be_bytes([b[0], b[1]])
            }
        })
    };
    let u32_at = |at: usize| {
        tiff.get(at..at + 4).map(|b| {
            if little {
                u32::from_le_bytes([b[0], b[1], b[2], b[3]])
            } else {
                u32::from_be_bytes([b[0], b[1], b[2], b[3]])
            }
        })
    };
    let ifd = u32_at(4)? as usize;
    let (mut unit, mut resolution) = (None, None);
    for entry in 0..usize::from(u16_at(ifd)?) {
        let at = ifd + 2 + entry * 12;
        match u16_at(at)? {
            0x0128 => unit = u16_at(at + 8),
            0x011a => {
                let value = u32_at(at + 8)? as usize;
                let (numerator, denominator) = (u32_at(value)?, u32_at(value + 4)?);
                resolution = (denominator != 0).then_some(numerator as f32 / denominator as f32);
            }
            _ => {}
        }
    }
    Some(if unit? == 2 {
        resolution.unwrap_or(72.0)
    } else {
        72.0
    })
}

/// Whether an `<img>` in a draft is a read receipt.
fn is_read_receipt(element: &exo_editor::Element) -> bool {
    let attributes: Vec<(&str, &str)> = element
        .attrs
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    exo_mail::is_read_receipt(&attributes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(pixels_per_metre: Option<u32>) -> Vec<u8> {
        let mut out = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut out);
        image::ImageEncoder::write_image(
            encoder,
            &[255, 0, 0, 255].repeat(6),
            3,
            2,
            image::ExtendedColorType::Rgba8,
        )
        .expect("encoded");
        let Some(ppm) = pixels_per_metre else {
            return out;
        };
        // `pHYs` goes before the first `IDAT`, after `IHDR` (8 + 25 bytes in).
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&9u32.to_be_bytes());
        chunk.extend_from_slice(b"pHYs");
        chunk.extend_from_slice(&ppm.to_be_bytes());
        chunk.extend_from_slice(&ppm.to_be_bytes());
        chunk.push(1);
        chunk.extend_from_slice(&[0; 4]);
        out.splice(33..33, chunk);
        out
    }

    fn draft(id: &str, attachments: Vec<DraftAttachment>) -> Open {
        let record = exo_store::DraftRecord {
            action: exo_store::DraftAction::Compose,
            from: None,
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: String::new(),
            body: String::new(),
            quoted_content: None,
            quoted_inlined: false,
            in_reply_to: None,
            in_reply_to_rfc822: None,
            references: Vec::new(),
            rfc822_id: "x@y".into(),
            fingerprint: String::new(),
            attachments,
            send_later: None,
            reminder: None,
        };
        Open::new(id.into(), None, record)
    }

    #[test]
    fn an_image_pasted_from_another_draft_comes_with_its_file() {
        let folder = std::env::temp_dir().join(format!("exo-adopt-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a folder");
        std::fs::write(folder.join("u1"), b"png").expect("written");
        let image = DraftAttachment {
            uuid: "u1".into(),
            name: "image.png".into(),
            mime_type: "image/png".into(),
            size: 3,
            content_id: Some("c1".into()),
            source: AttachmentSource::Local,
        };
        let mut drafts = [draft("a", vec![image]), draft("b", Vec::new())];
        adopt(&mut drafts, 1, "C1", &folder);
        let [copy] = drafts[1].record.attachments.as_slice() else {
            panic!("one copy: {:?}", drafts[1].record.attachments)
        };
        assert_eq!(
            (copy.content_id.as_deref(), copy.name.as_str()),
            (Some("c1"), "image.png")
        );
        assert_ne!(
            copy.uuid, "u1",
            "a file of its own, which discarding the other draft leaves"
        );
        assert_eq!(
            std::fs::read(folder.join(&copy.uuid)).expect("copied"),
            b"png"
        );
        adopt(&mut drafts, 1, "nowhere", &folder);
        assert_eq!(drafts[1].record.attachments.len(), 1);
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A JPEG with an EXIF segment giving `XResolution` and `ResolutionUnit` (big-endian).
    fn jpeg(dpi: u32, unit: u16) -> Vec<u8> {
        let mut tiff = b"MM\0\x2a\0\0\0\x08".to_vec();
        tiff.extend_from_slice(&2u16.to_be_bytes());
        // XResolution: RATIONAL, one, at offset 38 (8 + 2 + 2·12 + 4).
        tiff.extend_from_slice(&[0x01, 0x1a, 0, 5, 0, 0, 0, 1, 0, 0, 0, 38]);
        tiff.extend_from_slice(&[0x01, 0x28, 0, 3, 0, 0, 0, 1]);
        tiff.extend_from_slice(&unit.to_be_bytes());
        tiff.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        tiff.extend_from_slice(&dpi.to_be_bytes());
        tiff.extend_from_slice(&1u32.to_be_bytes());
        let mut out = vec![0xff, 0xd8, 0xff, 0xe1];
        out.extend_from_slice(&u16::try_from(tiff.len() + 8).expect("short").to_be_bytes());
        out.extend_from_slice(b"Exif\0\0");
        out.extend_from_slice(&tiff);
        out.extend_from_slice(&[0xff, 0xda, 0, 2]);
        out
    }

    #[test]
    fn a_jpeg_says_its_resolution_in_exif() {
        assert_eq!(retina(&jpeg(144, 2)), 2);
        assert_eq!(retina(&jpeg(216, 2)), 3);
        assert_eq!(
            retina(&jpeg(144, 3)),
            1,
            "a unit other than inches counts as 72 dpi"
        );
        assert_eq!(retina(&[0xff, 0xd8, 0xff, 0xda, 0, 2]), 1, "no EXIF");
    }

    #[test]
    fn a_retina_screenshot_is_two_pixels_a_point() {
        assert_eq!(
            retina(&png(Some(5669))),
            2,
            "144 dpi, as macOS saves a Retina screenshot"
        );
        assert_eq!(retina(&png(Some(2835))), 1, "72 dpi");
        assert_eq!(retina(&png(None)), 1);
        let ImageLoad::Loaded(picture) = decode(&png(Some(5669))) else {
            panic!("decodes")
        };
        assert_eq!((picture.natural, picture.retina), ([3, 2], 2));
        assert!(matches!(decode(b"not an image"), ImageLoad::Broken));
    }
}
