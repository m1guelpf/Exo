//! Animated images (GIF, APNG, animated WebP), played the way Chromium plays them: each on its own
//! clock from when it first shows, for as many loops as the file asks, decoded a frame at a time
//! so a long animation never sits in memory whole.

use std::{io::Cursor, sync::Arc, time::Duration};

use image::{
    AnimationDecoder, Frames, ImageDecoder, RgbaImage,
    codecs::{gif::GifDecoder, png::PngDecoder, webp::WebPDecoder},
    metadata::LoopCount,
};

/// Chromium shows a frame that asks for 10 ms or less for 100 ms, as other browsers do
/// (`ImageDecoder::FrameDurationAtIndex`).
const SHORTEST_DELAY: Duration = Duration::from_millis(11);
const SHORT_DELAY_SHOWN_FOR: Duration = Duration::from_millis(100);

/// A late animation (one that was off screen) picks up from its current frame, instead of
/// decoding its way through every frame it missed.
const MAX_CATCH_UP: Duration = Duration::from_secs(1);

/// The largest canvas played: 4096 × 4096, 64 MB a frame. Each frame is decoded whole at the
/// canvas's size, and a few bytes of GIF can claim 65535 × 65535 (17 GB), so a larger image stays
/// the still first frame Blitz shows.
const MAX_PIXELS: u64 = 4096 * 4096;

fn fits(decoder: &impl ImageDecoder) -> bool {
    let (width, height) = decoder.dimensions();
    u64::from(width) * u64::from(height) <= MAX_PIXELS
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Gif,
    Png,
    WebP,
}

/// Whether `bytes` may be an animated image, judged from their header alone (cheap enough for
/// every image a body loads).
pub fn may_animate(bytes: &[u8]) -> bool {
    format(bytes).is_some()
}

fn format(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(Format::Gif);
    }
    // An APNG has an `acTL` chunk before its first `IDAT`.
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let idat = find(bytes, b"IDAT").unwrap_or(bytes.len());
        return find(&bytes[..idat], b"acTL").map(|_| Format::Png);
    }
    // An animated WebP is an extended one (`VP8X`) with the animation flag.
    (bytes.len() > 20
        && &bytes[..4] == b"RIFF"
        && &bytes[8..16] == b"WEBPVP8X"
        && bytes[20] & 0x02 != 0)
        .then_some(Format::WebP)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// An animated image being played: a message body's, or one in compose's editor.
pub struct Player {
    bytes: Arc<[u8]>,
    format: Format,
    frames: Frames<'static>,
    /// How many more times it plays after this time through; `None` forever.
    plays_left: Option<u32>,
    /// The frame on show and how long it shows.
    shown: (RgbaImage, Duration),
    /// The frame after it, when it's been decoded already.
    upcoming: Option<(RgbaImage, Duration)>,
    /// When the frame on show started showing, once the animation has started.
    shown_at: Option<Duration>,
    /// It has played its last frame (or its file stopped decoding).
    finished: bool,
}

impl Player {
    /// A player for `bytes` when they're an image with more than one frame, showing its first.
    pub fn new(bytes: Arc<[u8]>) -> Option<Self> {
        let format = format(&bytes)?;
        let (mut frames, plays) = decode(&bytes, format)?;
        let first = next_frame(&mut frames)?;
        // A single frame is a still image, which Blitz already shows.
        let second = next_frame(&mut frames)?;
        Some(Self {
            bytes,
            format,
            frames,
            plays_left: plays.map(|plays| plays.saturating_sub(1)),
            shown: first,
            upcoming: Some(second),
            shown_at: None,
            finished: false,
        })
    }

    /// The frame on show.
    pub fn frame(&self) -> &RgbaImage {
        &self.shown.0
    }

    /// When the next frame is due, once [`Self::advance`] has started the clock; `None` when the
    /// animation is over.
    pub fn next_due(&self) -> Option<Duration> {
        (!self.finished)
            .then(|| self.shown_at.map(|at| at + self.shown.1))
            .flatten()
    }

    /// Moves to the frame that shows at `now`, starting the clock on the first call. Returns
    /// whether the frame on show changed.
    pub fn advance(&mut self, now: Duration) -> bool {
        let Some(shown_at) = self.shown_at else {
            self.shown_at = Some(now);
            return false;
        };
        if now > shown_at + self.shown.1 + MAX_CATCH_UP {
            self.shown_at = Some(now);
            return false;
        }
        let mut changed = false;
        while let Some(due) = self.next_due().filter(|due| *due <= now) {
            match self.next() {
                Some(frame) => {
                    self.shown = frame;
                    self.shown_at = Some(due);
                    changed = true;
                }
                None => self.finished = true,
            }
        }
        changed
    }

    /// The frame after the one on show, from the top again at the end of a play that isn't the
    /// last.
    fn next(&mut self) -> Option<(RgbaImage, Duration)> {
        if let Some(frame) = self
            .upcoming
            .take()
            .or_else(|| next_frame(&mut self.frames))
        {
            return Some(frame);
        }
        match &mut self.plays_left {
            Some(0) => return None,
            Some(plays) => *plays -= 1,
            None => {}
        }
        self.frames = decode(&self.bytes, self.format)?.0;
        next_frame(&mut self.frames)
    }
}

/// `bytes`' frames and how many times they play (`None`: forever).
fn decode(bytes: &Arc<[u8]>, format: Format) -> Option<(Frames<'static>, Option<u32>)> {
    let reader = || Cursor::new(ArcBytes(bytes.clone()));
    let plays = |count: LoopCount| match count {
        LoopCount::Infinite => None,
        LoopCount::Finite(plays) => Some(plays.get()),
    };
    match format {
        Format::Gif => {
            let decoder = GifDecoder::new(reader()).ok().filter(fits)?;
            Some((decoder.into_frames(), gif_plays(bytes)))
        }
        Format::Png => {
            let decoder = PngDecoder::new(reader()).ok().filter(fits)?.apng().ok()?;
            let count = plays(decoder.loop_count());
            Some((decoder.into_frames(), count))
        }
        Format::WebP => {
            let decoder = WebPDecoder::new(reader()).ok().filter(fits)?;
            let count = plays(decoder.loop_count());
            Some((decoder.into_frames(), count))
        }
    }
}

/// How many times a GIF plays: once without a `NETSCAPE2.0` loop count, forever for a count of
/// 0, else one more than the count (Chromium's `GIFImageDecoder::RepetitionCount`). The image
/// crate reads a missing count as forever.
fn gif_plays(bytes: &[u8]) -> Option<u32> {
    let mut decoder = gif::DecodeOptions::new().read_info(bytes).ok()?;
    // The loop count comes before the first frame.
    decoder.next_frame_info().ok()?;
    match decoder.repeat() {
        gif::Repeat::Infinite => None,
        gif::Repeat::Finite(count) => Some(u32::from(count) + 1),
    }
}

/// The next whole frame and how long it shows, or `None` at the end (or a decoding error, which
/// ends the animation where it is, as Chromium does).
fn next_frame(frames: &mut Frames<'static>) -> Option<(RgbaImage, Duration)> {
    let frame = frames.next()?.ok()?;
    let (numer, denom) = frame.delay().numer_denom_ms();
    let delay = Duration::from_secs_f64(f64::from(numer) / f64::from(denom.max(1)) / 1000.0);
    let delay = if delay < SHORTEST_DELAY {
        SHORT_DELAY_SHOWN_FOR
    } else {
        delay
    };
    Some((frame.into_buffer(), delay))
}

/// Shared bytes a decoder can own (`Cursor` needs `AsRef<[u8]>`).
struct ArcBytes(Arc<[u8]>);

impl AsRef<[u8]> for ArcBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gif(width: u16, height: u16) -> Arc<[u8]> {
        let mut bytes = Vec::new();
        let mut encoder = gif::Encoder::new(&mut bytes, width, height, &[0, 0, 0, 255, 255, 255])
            .expect("header");
        for index in [0, 1] {
            let frame = gif::Frame {
                width: 1,
                height: 1,
                buffer: vec![index].into(),
                ..gif::Frame::default()
            };
            encoder.write_frame(&frame).expect("frame");
        }
        drop(encoder);
        bytes.into()
    }

    #[test]
    fn a_huge_canvas_stays_still() {
        assert!(Player::new(gif(2, 2)).is_some(), "two frames play");
        assert!(
            Player::new(gif(8000, 8000)).is_none(),
            "8000 × 8000 would decode 256 MB a frame"
        );
    }
}
