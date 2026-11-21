//! Inline images: where an `<img>` gets its pixels, the box Chromium lays it out in under the
//! editor's CSS, and the sizes the resizer writes on it.
//!
//! The editor never loads anything: [`crate::Editor::image_requests`] says which images it has
//! no pixels for, and the app answers each with [`crate::Editor::set_image`]. Until then an image
//! takes the room its attributes give it, as a loading image does in Chromium. An animated one
//! plays as Chromium plays it ([`exo_mail::Player`]) while it's on screen.

use std::{collections::HashMap, sync::Arc, time::Duration};

use egui::{ColorImage, TextureHandle, TextureOptions, Vec2, vec2};

use crate::model::Element;

/// The class on the images Exo adds to a draft. Sending takes it off.
pub(crate) const CLASS: &str = "Exo-inline-image";
/// An inserted image's style.
const INSERTED_STYLE: &str = "max-width: 100%; height: auto;";
/// A pasted `data:` image's style.
const PASTED_STYLE: &str = "max-width:100%; height: auto";
/// The narrowest a drag of the resizer makes an image.
pub(crate) const MIN_DRAG_WIDTH: f32 = 16.0;
/// A blocked image's least box (`img[src^="https://media-blocked"] { min-width: 16px;
/// min-height: 16px }`) and its side padding in the editor (`.Squire img[…] { padding: 0 3px }`).
const BLOCKED_MIN: f32 = 16.0;
const BLOCKED_PADDING: f32 = 3.0;

/// Where an image gets its pixels.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ImageSource {
    /// One of the draft's inline attachments, by Content-ID: `src-cid`, or a `cid:` URL as a
    /// draft written in Gmail has it.
    Cid(String),
    /// A `data:` URL (a signature's logo, say).
    Data(String),
    /// An `http` or `https` URL.
    Remote(String),
}

impl ImageSource {
    /// Where `image` (an `<img>`) gets its pixels; `None` when it names nothing loadable (an
    /// empty `src`, a `blob:` or `file:` URL).
    pub fn of(image: &Element) -> Option<Self> {
        if let Some(cid) = image.attr("src-cid").filter(|cid| !cid.is_empty()) {
            return Some(Self::Cid(cid.to_owned()));
        }
        let src = image.attr("src")?.trim();
        let scheme = src
            .split_once(':')
            .map(|(scheme, _)| scheme.to_ascii_lowercase());
        match scheme.as_deref() {
            Some("cid") => Some(Self::Cid(src[4..].to_owned()))
                .filter(|source| !matches!(source, Self::Cid(cid) if cid.is_empty())),
            Some("data") => Some(Self::Data(src.to_owned())),
            Some("http" | "https") => Some(Self::Remote(src.to_owned())),
            _ => None,
        }
    }

    /// What the editor knows its pixels by: its URL, `cid:` and the Content-ID for an
    /// attachment.
    pub fn key(&self) -> String {
        match self {
            Self::Cid(cid) => format!("cid:{cid}"),
            Self::Data(url) | Self::Remote(url) => url.clone(),
        }
    }
}

/// Decoded pixels for an image.
pub struct Picture {
    /// The pixels, perhaps scaled down from the file's for drawing.
    pub pixels: ColorImage,
    /// The file's own size in pixels, which lays it out.
    pub natural: [usize; 2],
    /// How many pixels make a CSS pixel by the file's resolution (2 for a 144 dpi Retina
    /// screenshot), which Best Fit divides by.
    pub retina: u32,
    /// The file: an animated one (a GIF, an APNG, an animated WebP) plays, and a kept table's
    /// Blitz document decodes its images from theirs.
    pub file: Arc<[u8]>,
}

/// What became of loading an image.
pub enum ImageLoad {
    /// It loaded.
    Loaded(Picture),
    /// The Images setting blocks it: an empty box shows instead.
    Blocked,
    /// It couldn't be had, or isn't an image.
    Broken,
}

/// An image's pixels as the editor holds them.
pub(crate) enum State {
    /// Asked for.
    Pending,
    Loaded {
        natural: Vec2,
        /// Its pixels until they're a texture.
        pixels: Option<ColorImage>,
        texture: Option<TextureHandle>,
        /// Its animation, which puts each frame in the texture as it's due.
        player: Option<Box<exo_mail::Player>>,
        /// Its file, for the kept tables that show it.
        file: Arc<[u8]>,
    },
    Blocked,
    Broken,
}

/// Every image's pixels, by [`ImageSource::key`].
#[derive(Default)]
pub(crate) struct Pictures {
    states: HashMap<String, State>,
    /// Bumped whenever one changes, so the layout knows to look again.
    pub(crate) revision: u64,
}

impl Pictures {
    pub(crate) fn get(&self, key: &str) -> Option<&State> {
        self.states.get(key)
    }

    /// Whether `key` has been asked for or answered; asks for it if not.
    pub(crate) fn ask(&mut self, key: String) -> bool {
        if self.states.contains_key(&key) {
            return false;
        }
        self.states.insert(key, State::Pending);
        true
    }

    pub(crate) fn set(&mut self, key: &str, load: ImageLoad) {
        let state = match load {
            ImageLoad::Loaded(picture) => State::Loaded {
                natural: vec2(picture.natural[0] as f32, picture.natural[1] as f32),
                pixels: Some(picture.pixels),
                texture: None,
                player: exo_mail::Player::new(picture.file.clone()).map(Box::new),
                file: picture.file,
            },
            ImageLoad::Blocked => State::Blocked,
            ImageLoad::Broken => State::Broken,
        };
        self.states.insert(key.to_owned(), state);
        self.revision += 1;
    }

    pub(crate) fn forget(&mut self, mut which: impl FnMut(&str) -> bool) {
        let before = self.states.len();
        self.states.retain(|key, _| !which(key));
        if self.states.len() != before {
            self.revision += 1;
        }
    }

    /// Makes textures of the pixels that arrived for the images `shown` (by key): an image only a
    /// kept table shows is drawn by its table, and needs none. Large images draw scaled down a
    /// lot, so they get mipmaps, as Chromium's image filtering has.
    pub(crate) fn upload(&mut self, ctx: &egui::Context, shown: impl Fn(&str) -> bool) {
        for (key, state) in &mut self.states {
            if let State::Loaded {
                pixels, texture, ..
            } = state
                && shown(key)
                && let Some(pixels) = pixels.take()
            {
                let options = TextureOptions {
                    mipmap_mode: Some(egui::TextureFilter::Linear),
                    ..TextureOptions::LINEAR
                };
                *texture = Some(ctx.load_texture(
                    format!("exo_editor::image {}", key.get(..64).unwrap_or(key)),
                    pixels,
                    options,
                ));
            }
        }
    }

    /// Moves the animated images `shown` (by key) on to the frames due at `now` (the input's clock,
    /// in seconds): each starts its clock the first time it shows. Returns how long until the
    /// next frame is due.
    pub(crate) fn animate(&mut self, now: f64, shown: impl Fn(&str) -> bool) -> Option<f64> {
        let now = Duration::from_secs_f64(now.max(0.0));
        let mut next: Option<Duration> = None;
        for (key, state) in &mut self.states {
            let State::Loaded {
                texture: Some(texture),
                player: Some(player),
                ..
            } = state
            else {
                continue;
            };
            if !shown(key) {
                continue;
            }
            if player.advance(now) {
                let frame = player.frame();
                let pixels = ColorImage::from_rgba_unmultiplied(
                    [frame.width() as usize, frame.height() as usize],
                    frame.as_raw(),
                );
                texture.set(
                    pixels,
                    TextureOptions {
                        mipmap_mode: Some(egui::TextureFilter::Linear),
                        ..TextureOptions::LINEAR
                    },
                );
            }
            if let Some(due) = player.next_due() {
                next = Some(next.map_or(due, |next| next.min(due)));
            }
        }
        next.map(|due| due.saturating_sub(now).as_secs_f64())
    }

    /// The natural size of `key`'s image, once it has loaded.
    pub(crate) fn natural(&self, key: &str) -> Option<Vec2> {
        match self.states.get(key)? {
            State::Loaded { natural, .. } => Some(*natural),
            _ => None,
        }
    }
}

/// How an image is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Look {
    /// Its pixels, once they come.
    Picture,
    /// The white box over a blocked image.
    Blocked,
    /// A kept table, drawn by its Blitz document (`view::table`), by its key.
    Table(u64),
}

/// An image's place on its line: its content box, its side padding, and how it sits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ImageBox {
    pub(crate) size: Vec2,
    pub(crate) padding: f32,
    pub(crate) look: Look,
    /// `display: block`: a block box, as tall as it is, with no line around it.
    pub(crate) block: bool,
}

impl ImageBox {
    /// The box Chromium gives `image` in a block `container` wide, with what's known of its
    /// pixels.
    pub(crate) fn of(image: &Element, state: Option<&State>, container: f32) -> Self {
        let block = is_block(image);
        match state {
            Some(State::Blocked) => Self {
                size: blocked_size(image, container),
                padding: BLOCKED_PADDING,
                look: Look::Blocked,
                block,
            },
            Some(State::Loaded { natural, .. }) => Self {
                size: css_size(image, Some(*natural), container),
                padding: 0.0,
                look: Look::Picture,
                block,
            },
            _ => Self {
                size: css_size(image, None, container),
                padding: 0.0,
                look: Look::Picture,
                block,
            },
        }
    }

    /// The room it takes on its line.
    pub(crate) fn advance(&self) -> f32 {
        self.size.x + 2.0 * self.padding
    }

    /// Its top and bottom from the line's baseline (down is positive): on the baseline, or, for
    /// a blocked one (`vertical-align: middle`), centred half the text's x-height above it.
    pub(crate) fn extent(&self, x_height: f32) -> (f32, f32) {
        match self.look {
            Look::Picture | Look::Table(_) => (-self.size.y, 0.0),
            Look::Blocked => {
                let middle = -x_height / 2.0;
                (middle - self.size.y / 2.0, middle + self.size.y / 2.0)
            }
        }
    }

    pub(crate) fn hash(&self, state: &mut impl std::hash::Hasher) {
        use std::hash::Hash as _;
        (
            self.size.x.to_bits(),
            self.size.y.to_bits(),
            self.padding.to_bits(),
            self.look,
            self.block,
        )
            .hash(state);
    }
}

/// Whether an `<img>` is `display: block`.
fn is_block(image: &Element) -> bool {
    declarations(image.attr("style").unwrap_or_default())
        .iter()
        .any(|(property, value)| property == "display" && value.starts_with("block"))
}

/// An `<img>`'s used size (CSS 2.1 sections 10.3.2, 10.6.2 and 10.4) under the editor's
/// `img { max-width: 100% !important }`: its `style` width and height (or, without them, its
/// attributes), the missing one from its aspect ratio (its pixels', or its attributes' while it
/// loads), clamped to the block.
fn css_size(image: &Element, natural: Option<Vec2>, container: f32) -> Vec2 {
    let style = declarations(image.attr("style").unwrap_or_default());
    let property = |name: &str| {
        style
            .iter()
            .rev()
            .find(|(property, _)| property == name)
            .map(|(_, value)| value.as_str())
    };
    let width = match property("width") {
        Some(value) => length(value, Some(container)),
        None => image
            .attr("width")
            .and_then(|value| dimension(value, Some(container))),
    };
    // A percentage height against a block of `auto` height is `auto`.
    let height = match property("height") {
        Some(value) => length(value, None),
        None => image
            .attr("height")
            .and_then(|value| dimension(value, None)),
    };
    let attribute_ratio = || {
        let width = image
            .attr("width")
            .and_then(|value| dimension(value, None))?;
        let height = image
            .attr("height")
            .and_then(|value| dimension(value, None))?;
        (width > 0.0 && height > 0.0).then_some(width / height)
    };
    let ratio = natural
        .filter(|size| size.x > 0.0 && size.y > 0.0)
        .map(|size| size.x / size.y)
        .or_else(attribute_ratio);
    let intrinsic = natural.unwrap_or(Vec2::ZERO);
    let (mut w, mut h) = match (width, height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, ratio.map_or(intrinsic.y, |ratio| w / ratio)),
        (None, Some(h)) => (ratio.map_or(intrinsic.x, |ratio| h * ratio), h),
        (None, None) => (intrinsic.x, intrinsic.y),
    };
    let max = property("max-width")
        .and_then(|value| length(value, Some(container)))
        .map_or(container, |max| max.min(container));
    if w > max {
        if height.is_none()
            && let Some(ratio) = ratio
        {
            h = max / ratio;
        }
        w = max;
    }
    vec2(w.max(0.0), h.max(0.0))
}

/// A blocked image's box: Chromium's for the broken image under it, its size and ratio with both
/// dimension attributes and either no alt text or `display: block`, else a 16px square; never
/// smaller than 16px.
fn blocked_size(image: &Element, container: f32) -> Vec2 {
    let attribute = |name: &str| {
        image
            .attr(name)
            .filter(|value| !value.trim_end().ends_with('%'))
            .and_then(|value| dimension(value, None))
    };
    let alt = image.attr("alt").is_some_and(|alt| !alt.is_empty());
    let block = is_block(image);
    let size = match (attribute("width"), attribute("height")) {
        (Some(width), Some(height)) if !alt || block => css_size(
            &Element::with(
                "img",
                &[
                    ("width", &css_number(width)),
                    ("height", &css_number(height)),
                ],
            ),
            None,
            container - 2.0 * BLOCKED_PADDING,
        ),
        _ => Vec2::ZERO,
    };
    size.max(Vec2::splat(BLOCKED_MIN))
}

/// Best Fit: the image at its resolution's size, no wider than 640px and no taller than 500px.
/// Both caps hold at once, so a wide and tall image (a whole-window screenshot) stays within
/// 640px rather than coming out 800px wide or more.
pub(crate) fn best_fit(natural: Vec2, retina: f32) -> f32 {
    let (width, height) = (natural.x / retina, natural.y / retina);
    let tall = if height > 500.0 {
        width / (height / 500.0)
    } else {
        width
    };
    tall.min(640.0)
}

/// The `data-retina` an image carries (`parseFloat(dataset.retina || "1") || 1`).
pub(crate) fn retina(image: &Element) -> f32 {
    image
        .attr("data-retina")
        .and_then(leading_number)
        .filter(|retina| *retina != 0.0)
        .unwrap_or(1.0)
}

/// The `<img>` Insert Image puts in: shown by Content-ID `cid`, its `src` empty until it loads.
pub(crate) fn inserted(cid: &str) -> Element {
    Element::with(
        "img",
        &[
            ("class", CLASS),
            ("src-cid", cid),
            ("style", INSERTED_STYLE),
            ("src", ""),
        ],
    )
}

/// What a pasted `data:` image becomes: its `src` emptied and the picture shown by Content-ID `cid`
/// (its own `src-cid` if it had one) instead.
pub(crate) fn pasted(image: &mut Element, cid: &str) {
    image.set_attr("src", "");
    if image.attr("src-cid").is_none_or(str::is_empty) {
        image.set_attr("src-cid", cid);
    }
    let mut classes: Vec<&str> = image
        .attr("class")
        .unwrap_or_default()
        .split_whitespace()
        .collect();
    if !classes.contains(&CLASS) {
        classes.push(CLASS);
    }
    let classes = classes.join(" ");
    image.set_attr("class", &classes);
    image.set_attr("style", PASTED_STYLE);
}

/// `_onImageLoaded`: an inserted image with no width yet gets its resolution and Best Fit's.
pub(crate) fn fit_on_load(image: &mut Element, natural: Vec2, picture_retina: u32) {
    image.set_attr("data-retina", &picture_retina.max(1).to_string());
    let style = declarations(image.attr("style").unwrap_or_default());
    if image.attr("width").is_some_and(|width| !width.is_empty())
        || style.iter().any(|(property, _)| property == "width")
    {
        return;
    }
    let width = best_fit(natural, retina(image));
    set_style(image, "width", &format!("{}px", css_number(width)));
    image.set_attr("width", &dom_unsigned(width));
}

/// Resizes `image` to `width`: the width, and the height that keeps the picture's ratio, in its
/// style and its attributes, marked resized.
pub(crate) fn resize(image: &mut Element, width: f32, natural: Vec2) {
    let height = width * (natural.y / natural.x);
    set_style(image, "width", &format!("{}px", css_number(width)));
    image.set_attr("width", &dom_unsigned(width));
    set_style(image, "height", &format!("{}px", css_number(height)));
    image.set_attr("height", &dom_unsigned(height));
    if image.attr("data-resized").is_none_or(str::is_empty) {
        image.set_attr("data-resized", "true");
    }
}

/// A `style` attribute's declarations, in order, names lowercased.
fn declarations(style: &str) -> Vec<(String, String)> {
    style
        .split(';')
        .filter_map(|declaration| {
            let (property, value) = declaration.split_once(':')?;
            Some((
                property.trim().to_ascii_lowercase(),
                value.trim().to_owned(),
            ))
        })
        .filter(|(property, value)| !property.is_empty() && !value.is_empty())
        .collect()
}

/// Sets one property through the CSSOM (`element.style.width = …`), which writes the whole
/// attribute back as Chromium serializes it: `name: value;`, space separated, the property kept
/// in its place or added last.
fn set_style(image: &mut Element, property: &str, value: &str) {
    let mut style = declarations(image.attr("style").unwrap_or_default());
    match style.iter_mut().find(|(name, _)| name == property) {
        Some((_, existing)) => value.clone_into(existing),
        None => style.push((property.to_owned(), value.to_owned())),
    }
    let written: Vec<String> = style
        .iter()
        .map(|(name, value)| format!("{name}: {value};"))
        .collect();
    image.set_attr("style", &written.join(" "));
}

/// A CSS `<length>` or `<percentage>` (of `percent_of`; `None` makes a percentage `auto`) in px.
fn length(value: &str, percent_of: Option<f32>) -> Option<f32> {
    let value = value
        .trim_end_matches("!important")
        .trim()
        .to_ascii_lowercase();
    if let Some(percent) = value.strip_suffix('%') {
        return Some(percent.trim().parse::<f32>().ok()? / 100.0 * percent_of?);
    }
    match value.strip_suffix("px") {
        Some(px) => px.trim().parse().ok(),
        None => value.parse::<f32>().ok().filter(|zero| *zero == 0.0),
    }
}

/// An HTML dimension attribute (the rules for parsing dimension values): leading digits, a
/// percentage of `percent_of` if a `%` follows.
fn dimension(value: &str, percent_of: Option<f32>) -> Option<f32> {
    let number = leading_number(value)?;
    let rest = value
        .trim_start()
        .trim_start_matches(|ch: char| ch.is_ascii_digit() || ch == '.');
    if rest.starts_with('%') {
        percent_of.map(|whole| number / 100.0 * whole)
    } else {
        Some(number)
    }
}

/// The number a value starts with (`parseFloat`, without exponents or signs).
fn leading_number(value: &str) -> Option<f32> {
    let value = value.trim_start();
    let end = value
        .find(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
        .unwrap_or(value.len());
    let digits = value[..end].trim_end_matches('.');
    let digits = match digits.match_indices('.').nth(1) {
        Some((second, _)) => &digits[..second],
        None => digits,
    };
    digits.parse().ok()
}

/// A number as Chromium's CSSOM writes it: at most six significant digits, no trailing zeros
/// (`String::Number`).
pub(crate) fn css_number(value: f32) -> String {
    let value = f64::from(value);
    if value == 0.0 {
        return "0".to_owned();
    }
    let integer_digits = value.abs().log10().floor() as i32 + 1;
    let decimals = (6 - integer_digits).max(0) as usize;
    let text = format!("{value:.decimals$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

/// A number as an `unsigned long` attribute takes it (`img.width = 333.3` writes `333`).
fn dom_unsigned(value: f32) -> String {
    (value.max(0.0).trunc() as u32).to_string()
}

/// A random (v4) UUID, for an inline image's Content-ID and for its uploads.
pub fn uuid_v4() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        bytes = jiff::Timestamp::now().as_nanosecond().to_le_bytes();
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// The selection colour over a selected image: Chromium blends an opaque `::selection` colour
/// with white into a translucent one that looks the same on white (`Color::BlendWithWhite`),
/// and leaves a translucent one as it is.
pub(crate) fn selection_tint(selection: egui::Color32) -> egui::Color32 {
    let [r, g, b, a] = selection.to_srgba_unmultiplied();
    if a != 255 {
        return selection;
    }
    // Alphas from 0.6 up, until no component goes negative.
    for alpha in (153..=204).step_by(17) {
        let blend = |component: u8| (255 * (i32::from(component) + alpha - 255)) / alpha;
        let (red, green, blue) = (blend(r), blend(g), blend(b));
        if red >= 0 && green >= 0 && blue >= 0 || alpha == 204 {
            let clamp = |component: i32| component.clamp(0, 255) as u8;
            return egui::Color32::from_rgba_unmultiplied(
                clamp(red),
                clamp(green),
                clamp(blue),
                alpha as u8,
            );
        }
    }
    selection
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(attrs: &[(&str, &str)]) -> Element {
        Element::with("img", attrs)
    }

    /// A GIF of a red frame then a blue one, `delay` milliseconds each, looping forever.
    fn gif(delay: u32) -> Arc<[u8]> {
        use image::{
            Delay, Frame, Rgba, RgbaImage,
            codecs::gif::{GifEncoder, Repeat},
        };
        let mut bytes = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut bytes);
            encoder.set_repeat(Repeat::Infinite).expect("loops");
            let frames = [Rgba([255, 0, 0, 255]), Rgba([0, 0, 255, 255])].map(|color| {
                Frame::from_parts(
                    RgbaImage::from_pixel(2, 2, color),
                    0,
                    0,
                    Delay::from_numer_denom_ms(delay, 1),
                )
            });
            encoder.encode_frames(frames).expect("encodes");
        }
        bytes.into()
    }

    #[test]
    fn an_animated_image_plays_while_it_shows() {
        let ctx = egui::Context::default();
        let mut pictures = Pictures::default();
        pictures.set(
            "cid:a",
            ImageLoad::Loaded(Picture {
                pixels: ColorImage::filled([2, 2], egui::Color32::RED),
                natural: [2, 2],
                retina: 1,
                file: gif(50),
            }),
        );
        pictures.upload(&ctx, |_| true);
        assert_eq!(
            pictures.animate(1.0, |_| false),
            None,
            "off screen it doesn't start"
        );
        let wait = pictures.animate(2.0, |_| true).expect("a frame due");
        assert!(
            (wait - 0.05).abs() < 1e-6,
            "its first frame shows 50ms from when it first shows: {wait}"
        );
        let wait = pictures.animate(2.06, |_| true).expect("a frame due");
        assert!(
            (wait - 0.04).abs() < 1e-6,
            "the second frame came at 2.05: {wait}"
        );
    }

    #[test]
    fn sources_come_from_src_cid_then_src() {
        assert_eq!(
            ImageSource::of(&img(&[("src-cid", "a@b"), ("src", "cid:a@b")])),
            Some(ImageSource::Cid("a@b".into()))
        );
        assert_eq!(
            ImageSource::of(&img(&[("src", "CID:ii_x")])),
            Some(ImageSource::Cid("ii_x".into()))
        );
        assert_eq!(
            ImageSource::of(&img(&[("src", "https://x.io/a.png")])).map(|source| source.key()),
            Some("https://x.io/a.png".into())
        );
        assert!(matches!(
            ImageSource::of(&img(&[("src", "data:image/png;base64,AA")])),
            Some(ImageSource::Data(_))
        ));
        assert_eq!(ImageSource::of(&img(&[("src", "")])), None);
        assert_eq!(ImageSource::of(&img(&[("src", "blob:https://x/1")])), None);
    }

    #[test]
    fn sizes_follow_chromium() {
        let natural = Some(vec2(800.0, 500.0));
        assert_eq!(
            css_size(&img(&[]), natural, 640.0),
            vec2(640.0, 400.0),
            "max-width: 100% keeps the ratio"
        );
        assert_eq!(
            css_size(
                &img(&[("style", "max-width: 100%; height: auto; width: 400px;")]),
                natural,
                640.0
            ),
            vec2(400.0, 250.0)
        );
        assert_eq!(
            css_size(&img(&[("width", "200")]), natural, 640.0),
            vec2(200.0, 125.0)
        );
        assert_eq!(
            css_size(&img(&[("width", "200"), ("height", "50")]), natural, 640.0),
            vec2(200.0, 50.0),
            "both given: the ratio goes"
        );
        assert_eq!(
            css_size(&img(&[("width", "900"), ("height", "300")]), natural, 640.0),
            vec2(640.0, 300.0),
            "a fixed height stays when the width is clamped"
        );
        assert_eq!(
            css_size(&img(&[("width", "50%")]), natural, 640.0),
            vec2(320.0, 200.0)
        );
        assert_eq!(
            css_size(&img(&[("width", "120"), ("height", "60")]), None, 640.0),
            vec2(120.0, 60.0),
            "loading, its attributes hold its room"
        );
        assert_eq!(css_size(&img(&[]), None, 640.0), Vec2::ZERO);
        let blocked = ImageBox::of(
            &img(&[("width", "40"), ("height", "8")]),
            Some(&State::Blocked),
            640.0,
        );
        assert_eq!((blocked.size, blocked.advance()), (vec2(40.0, 16.0), 46.0));
        assert_eq!(
            ImageBox::of(
                &img(&[("width", "40"), ("height", "30"), ("alt", "logo")]),
                Some(&State::Blocked),
                640.0
            )
            .size,
            vec2(16.0, 16.0)
        );
    }

    #[test]
    fn best_fit_keeps_images_within_640_by_500() {
        assert_eq!(best_fit(vec2(800.0, 500.0), 2.0), 400.0);
        assert_eq!(best_fit(vec2(1600.0, 400.0), 1.0), 640.0);
        assert_eq!(
            best_fit(vec2(1500.0, 1000.0), 1.0),
            640.0,
            "wide and tall: within both"
        );
        assert_eq!(
            best_fit(vec2(2880.0, 1800.0), 2.0),
            640.0,
            "a whole-window Retina screenshot"
        );
        assert_eq!(
            best_fit(vec2(400.0, 1000.0), 1.0),
            200.0,
            "tall: 500px high"
        );
        assert_eq!(retina(&img(&[("data-retina", "2")])), 2.0);
        assert_eq!(retina(&img(&[("data-retina", "0")])), 1.0);
    }

    #[test]
    fn resizing_writes_style_and_attributes_as_the_cssom_does() {
        let mut image = inserted("c1");
        fit_on_load(&mut image, vec2(800.0, 500.0), 2);
        assert_eq!(
            image
                .attrs
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>(),
            [
                "class=Exo-inline-image",
                "src-cid=c1",
                "style=max-width: 100%; height: auto; width: 400px;",
                "src=",
                "data-retina=2",
                "width=400"
            ]
        );
        resize(&mut image, 160.0, vec2(800.0, 300.0));
        assert_eq!(
            image.attr("style"),
            Some("max-width: 100%; height: 60px; width: 160px;")
        );
        resize(&mut image, 200.0, vec2(300.0, 200.0));
        assert_eq!(
            (
                image.attr("style"),
                image.attr("height"),
                image.attr("data-resized")
            ),
            (
                Some("max-width: 100%; height: 133.333px; width: 200px;"),
                Some("133"),
                Some("true")
            )
        );
        let mut pasted_image = img(&[("src", "data:image/png;base64,AA"), ("width", "30")]);
        pasted(&mut pasted_image, "c2");
        assert_eq!(
            pasted_image
                .attrs
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["src", "width", "src-cid", "class", "style"]
        );
        fit_on_load(&mut pasted_image, vec2(60.0, 60.0), 1);
        assert_eq!(
            pasted_image.attr("width"),
            Some("30"),
            "a width it has stays"
        );
    }

    #[test]
    fn numbers_print_as_chromium_prints_them() {
        assert_eq!(css_number(213.333_33), "213.333");
        assert_eq!(css_number(400.0), "400");
        assert_eq!(css_number(0.5), "0.5");
        assert_eq!(css_number(1_066.666_7), "1066.67");
        assert_eq!(dom_unsigned(133.9), "133");
    }

    #[test]
    fn a_selected_image_is_tinted_with_the_selection_blended_with_white() {
        // `#b4d5fe` over an image: rgba(130, 185, 253, .6).
        assert_eq!(
            selection_tint(egui::Color32::from_rgb(0xb4, 0xd5, 0xfe)).to_srgba_unmultiplied(),
            [130, 185, 253, 153]
        );
        let carbon = egui::Color32::from_rgba_unmultiplied(106, 134, 173, 254);
        assert_eq!(selection_tint(carbon), carbon);
    }
}
