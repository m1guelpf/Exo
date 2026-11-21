//! Tagging text colors so the restyle can recolor them, and neutralizing CSS that fights the
//! message column.
//!
//! Exo reads the inline styles and presentational attributes, not computed styles, and inherits
//! them down the tree; colors set only by a `<style>` sheet aren't seen (they mostly matter in
//! marketing mail, which isn't restyled).

use std::sync::LazyLock;

use peniko::color::{AlphaColor, Srgb, parse_color};
use regex::Regex;

use crate::dom::{self, NodeRef, Style};

/// The dark-mode text colors.
const CARBON_COLORS: &[(&str, [u8; 3])] = &[
    ("red", [255, 122, 122]),
    ("green", [92, 186, 135]),
    ("blue", [74, 143, 225]),
    ("purple", [213, 174, 239]),
    ("yellow", [230, 230, 0]),
    ("orange", [233, 150, 56]),
    ("pink", [201, 118, 209]),
    ("grey", [176, 176, 176]),
];

/// The colors author text snaps to, by name (module 3472's palette).
const PALETTE: &[(&str, &[[u8; 3]])] = &[
    (
        "black",
        &[
            [0, 0, 0],
            [12, 52, 61],
            [34, 34, 34],
            [33, 47, 64],
            [51, 51, 51],
            [32, 28, 59],
            [31, 73, 125],
            [0, 32, 96],
            [0, 48, 135],
        ],
    ),
    ("white", &[[255, 255, 255]]),
    ("orange", &[[180, 95, 6], [233, 150, 56]]),
    (
        "red",
        &[
            [102, 0, 0],
            [153, 0, 0],
            [87, 6, 6],
            [255, 0, 0],
            [222, 84, 84],
        ],
    ),
    ("brown", &[[120, 63, 4], [147, 69, 17]]),
    (
        "yellow",
        &[[127, 96, 0], [191, 144, 0], [195, 151, 29], [255, 255, 0]],
    ),
    (
        "green",
        &[
            [39, 78, 19],
            [56, 118, 29],
            [6, 64, 12],
            [0, 255, 0],
            [92, 186, 135],
        ],
    ),
    ("blue", &[[11, 83, 148], [0, 0, 255], [74, 143, 225]]),
    ("purple", &[[32, 18, 77], [53, 28, 117], [59, 32, 77]]),
    ("pink", &[[201, 118, 209], [255, 192, 203]]),
    ("grey", &[[102, 102, 102], [176, 176, 176]]),
];

/// Outlook's default text blues, which count as black.
const OUTLOOK_BLUES: &[[u8; 3]] = &[[0, 48, 135], [47, 84, 150]];

/// The UA's link color (`-webkit-link`).
const LINK_BLUE: Rgba = Rgba([0, 0, 238, 255]);

static CJK_FONT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)pingfang|hiragino|meiryo|yu gothic|apple sd gothic neo|microsoft yahei|microsoft jhenghei")
        .expect("valid regex")
});
static MONOSPACE_FONT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(^|,)\s*monospace\s*$").expect("valid regex"));
static VIEWPORT_UNIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\d\s*(d|s|l)?(vh|vw|vmin|vmax)\b").expect("valid regex"));

/// A CSS color value (`#rgb`, `rgb()`, `hsl()`, a named color, …) as sRGB with alpha, the way
/// the pipeline reads colors; `None` if it isn't one.
pub fn css_color(value: &str) -> Option<[u8; 4]> {
    Rgba::parse(value).map(|color| color.0)
}

/// An sRGB color with alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rgba([u8; 4]);

impl Rgba {
    const TRANSPARENT: Self = Self([0, 0, 0, 0]);
    const WHITE: Self = Self([255, 255, 255, 255]);

    fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        // `color` knows only the "gray" spellings of CSS's grays (`grey`, `darkslategrey`, …).
        let color = parse_color(text)
            .or_else(|_| parse_color(&text.to_ascii_lowercase().replace("grey", "gray")))
            .ok()?
            .to_alpha_color::<Srgb>();
        Some(Self(color.to_rgba8().to_u8_array()))
    }

    /// The color as `getComputedStyle` prints it.
    fn css(self) -> String {
        let [r, g, b, a] = self.0;
        if a == 255 {
            format!("rgb({r}, {g}, {b})")
        } else {
            format!(
                "rgba({r}, {g}, {b}, {})",
                (f32::from(a) / 255.0 * 100.0).round() / 100.0
            )
        }
    }

    fn rgb(self) -> [u8; 3] {
        [self.0[0], self.0[1], self.0[2]]
    }

    fn is_transparent(self) -> bool {
        self == Self::TRANSPARENT
    }

    /// Not transparent and more than 20% opaque.
    fn is_visible(self) -> bool {
        !self.is_transparent() && f32::from(self.0[3]) / 255.0 > 0.2
    }

    /// Every channel within `threshold` of `other`'s.
    fn similar_to(self, other: Self, threshold: u8) -> bool {
        self.rgb()
            .iter()
            .zip(other.rgb())
            .all(|(a, b)| a.abs_diff(b) <= threshold)
    }

    fn luminance(self) -> f64 {
        let channel = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let [r, g, b] = self.rgb();
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }

    /// tinycolor's `readability` (WCAG contrast, alpha ignored) of at least 2.5.
    fn readable_on(self, background: Self) -> bool {
        let (a, b) = (self.luminance(), background.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05) >= 2.5
    }

    /// Hue between 180° and 230°.
    fn is_blue_like(self) -> bool {
        let rgb = AlphaColor::<Srgb>::from_rgba8(self.0[0], self.0[1], self.0[2], 255);
        let hue = rgb.convert::<peniko::color::Hsl>().components[0];
        (180.0..=230.0).contains(&hue)
    }
}

/// The rule for a pasted text color: one unreadable on white becomes the light theme's color of its
/// kind (grey, black); one that snaps to black or white then goes, so the text takes the theme's.
/// `None` keeps `value`, `Some(None)` drops it.
pub(crate) fn pasted_color(value: &str) -> Option<Option<String>> {
    let color = Rgba::parse(value)?;
    let readable = color.readable_on(Rgba::WHITE);
    let color = if readable {
        color
    } else {
        match snap(color) {
            "grey" => Rgba([0, 0, 0, 255]),
            name => themed(false, name),
        }
    };
    if snaps_to_black(color, true) {
        Some(None)
    } else {
        (!readable).then(|| Some(color.css()))
    }
}

/// Whether a `<font color>` snaps to black (not counting near white), which a paste drops.
pub(crate) fn font_color_snaps_to_black(value: &str) -> bool {
    Rgba::parse(value).is_some_and(|color| snaps_to_black(color, false))
}

/// Near black, one of the blues Outlook and Gmail quote in, or, with `white`, near white.
fn snaps_to_black(color: Rgba, white: bool) -> bool {
    color.similar_to(Rgba([0, 0, 0, 255]), 10)
        || [[0, 48, 135], [47, 84, 150], [28, 69, 135]].contains(&color.rgb())
        || white && color.similar_to(Rgba::WHITE, 10)
}

/// The palette name a color snaps to.
fn snap(color: Rgba) -> &'static str {
    if OUTLOOK_BLUES.contains(&color.rgb()) {
        return "black";
    }
    let lab = to_lab(color.rgb());
    let mut best = ("black", f64::MAX);
    for (name, swatches) in PALETTE {
        for swatch in *swatches {
            let distance = delta_e(lab, to_lab(*swatch));
            if distance < best.1 {
                best = (name, distance);
            }
        }
    }
    best.0
}

fn to_lab([r, g, b]: [u8; 3]) -> [f64; 3] {
    let linear = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c > 0.04045 {
            ((c + 0.055) / 1.055).powf(2.4)
        } else {
            c / 12.92
        }
    };
    let (r, g, b) = (linear(r), linear(g), linear(b));
    let f = |t: f64| {
        if t > 0.008856 {
            t.cbrt()
        } else {
            7.787 * t + 16.0 / 116.0
        }
    };
    let x = f((0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047);
    let y = f(0.2126 * r + 0.7152 * g + 0.0722 * b);
    let z = f((0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883);
    [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
}

/// CIE94 color difference.
fn delta_e(a: [f64; 3], b: [f64; 3]) -> f64 {
    let dl = a[0] - b[0];
    let (da, db) = (a[1] - b[1], a[2] - b[2]);
    let c1 = (a[1] * a[1] + a[2] * a[2]).sqrt();
    let dc = c1 - (b[1] * b[1] + b[2] * b[2]).sqrt();
    let dh = (da * da + db * db - dc * dc).max(0.0).sqrt();
    let (sc, sh) = (1.0 + 0.045 * c1, 1.0 + 0.015 * c1);
    (dl * dl + (dc / sc).powi(2) + (dh / sh).powi(2))
        .max(0.0)
        .sqrt()
}

/// The text color the themes would draw a palette name in.
fn themed(dark: bool, name: &str) -> Rgba {
    match (dark, name) {
        (false, "black") | (true, "white") => Rgba([0, 0, 0, 230]),
        (false, "white") | (true, "black") => Rgba([255, 255, 255, 230]),
        _ => CARBON_COLORS
            .iter()
            .find(|(n, _)| *n == name)
            .map_or(Rgba([0, 0, 0, 255]), |(_, [r, g, b])| {
                Rgba([*r, *g, *b, 255])
            }),
    }
}

/// Inherited state while walking the tree.
#[derive(Clone, Copy)]
struct Inherited {
    color: Rgba,
    background: Rgba,
    background_image: bool,
    colorize_children: bool,
    has_background: bool,
    cjk_font: bool,
    monospace_font: bool,
    /// Inside an external-sender banner, whose light background is dropped more eagerly.
    in_banner: bool,
}

/// Runs the carbonize pass over a cleaned message.
pub(crate) fn carbonize(root: &NodeRef, is_marketing: bool) {
    for style in dom::select(root, |n| dom::is(n, "style")) {
        let css = dom::text_content(&style);
        let kept = drop_color_scheme_rules(&css);
        if kept != css {
            dom::set_children(&style, vec![dom::text_node(&kept)]);
        }
    }
    let root_state = Inherited {
        color: Rgba([0, 0, 0, 255]),
        background: Rgba::TRANSPARENT,
        background_image: false,
        colorize_children: true,
        has_background: false,
        cjk_font: false,
        monospace_font: false,
        in_banner: false,
    };
    if dom::is_element(root) {
        visit(root, root_state, is_marketing);
    } else {
        for child in dom::element_children(root) {
            visit(&child, root_state, is_marketing);
        }
    }
}

fn visit(element: &NodeRef, parent: Inherited, is_marketing: bool) {
    // Quotes keep the tags of the client that quoted them: Exo's, or the `sh-` ones some
    // senders' mail carries.
    const STALE: [&str; 4] = ["exo-color", "exo-original", "sh-color", "sh-original"];
    dom::remove_class(element, &["exo-modified-inline", "sh-modified-inline"]);
    if let Some(classes) = dom::attr(element, "class") {
        let stale: Vec<&str> = classes
            .split_ascii_whitespace()
            .filter(|c| STALE.iter().any(|prefix| c.starts_with(prefix)))
            .collect();
        if !stale.is_empty() {
            dom::remove_class(element, &stale);
        }
    }
    let mut state = computed(element, parent);
    if !carbonize_element(element, &mut state, parent, is_marketing) {
        return;
    }
    for child in dom::element_children(element) {
        visit(&child, state, is_marketing);
    }
}

/// The element's styles as far as Exo can compute them without a style engine.
fn computed(element: &NodeRef, parent: Inherited) -> Inherited {
    let style = Style::of(element);
    let tag = dom::tag(element).unwrap_or_default().to_ascii_lowercase();
    let mut color = Rgba::parse(style.get("color")).or_else(|| {
        (tag == "font")
            .then(|| dom::attr(element, "color"))
            .flatten()
            .and_then(|c| Rgba::parse(&c))
    });
    if color.is_none() && tag == "a" && dom::has_attr(element, "href") {
        color = Some(LINK_BLUE);
    }
    let background = Rgba::parse(style.get("background-color"))
        .or_else(|| background_shorthand_color(style.get("background")))
        .filter(|c| !c.is_transparent())
        .unwrap_or(parent.background);
    let own_image = style.get("background-image").contains("url")
        || style.get("background").contains("url")
        || dom::attr(element, "background").is_some_and(|b| !b.is_empty());
    let font = style.get("font-family");
    let face = (tag == "font")
        .then(|| dom::attr(element, "face"))
        .flatten()
        .unwrap_or_default();
    Inherited {
        color: color.unwrap_or(parent.color),
        background,
        background_image: own_image || parent.background_image,
        colorize_children: parent.colorize_children,
        // Only colorizing sets it, so it doesn't pass through elements that skip it.
        has_background: false,
        cjk_font: parent.cjk_font || CJK_FONT.is_match(font) || CJK_FONT.is_match(&face),
        monospace_font: parent.monospace_font || MONOSPACE_FONT.is_match(font),
        in_banner: parent.in_banner || dom::has_class(element, "exo-external-warning-banner"),
    }
}

fn background_shorthand_color(value: &str) -> Option<Rgba> {
    if value.is_empty() {
        return None;
    }
    Rgba::parse(value).or_else(|| {
        value
            .split_whitespace()
            .filter(|token| !token.contains("url("))
            .find_map(Rgba::parse)
    })
}

/// Unpins positioned elements and drops viewport-sized dimensions, marks fonts and link colors
/// to keep, and colorizes. Returns whether to visit the children.
fn carbonize_element(
    element: &NodeRef,
    state: &mut Inherited,
    parent: Inherited,
    is_marketing: bool,
) -> bool {
    let tag = dom::tag(element).unwrap_or_default().to_ascii_lowercase();
    let mut style = Style::of(element);
    let mut modified = false;
    let position = style.get("position").to_ascii_lowercase();
    if position == "absolute" || position == "fixed" {
        let important = style.is_important("position");
        style.set("--exo-original-position", &position, false);
        if important {
            style.set("--exo-original-position-priority", "important", false);
        }
        style.set("position", "unset", false);
        modified = true;
    }
    for property in [
        "height",
        "min-height",
        "max-height",
        "width",
        "min-width",
        "max-width",
    ] {
        let value = style.get(property);
        if !value.starts_with("var(") && VIEWPORT_UNIT.is_match(value) {
            style.remove(property);
            modified = true;
        }
    }
    if modified {
        style.apply(element);
        dom::add_class(element, &["exo-modified-inline"]);
    }
    if state.cjk_font || state.monospace_font {
        dom::add_class(element, &["exo-preserve-font-family"]);
    }
    match tag.as_str() {
        "svg" | "meta" | "head" | "wbr" | "br" | "style" => return false,
        "a" => {
            let background = if state.background.is_visible() {
                state.background
            } else {
                parent.background
            };
            if (state.background.is_visible() || parent.background.is_visible())
                && state.color.readable_on(background)
                && !state.color.is_blue_like()
            {
                dom::add_class(element, &["exo-preserve-color"]);
            }
            if parent.colorize_children {
                colorize(element, state, parent, is_marketing);
            }
            return true;
        }
        _ => {}
    }
    let pasted_whitespace = [
        ("background-color", "transparent"),
        ("font-variant-numeric", "normal"),
        ("font-variant-east-asian", "normal"),
        ("font-variant-alternates", "normal"),
        ("vertical-align", "baseline"),
        ("white-space", "pre"),
    ];
    if pasted_whitespace
        .iter()
        .all(|(name, value)| style.get(name) == *value)
    {
        style.remove("white-space");
        style.apply(element);
    }
    if tag == "img" && maintains_aspect_ratio(element, &style) {
        dom::add_class(element, &["exo-im-maintain-aspect-ratio"]);
    }
    if parent.colorize_children {
        colorize(element, state, parent, is_marketing);
    }
    true
}

/// A sized image at least 15px tall keeps its proportions when `max-width` shrinks it.
///
/// Its size is the computed value, not the used one: the inline style, else the `width`/`height`
/// attribute, else `auto`. An image sized by its height alone (`height: 39px`, no width) keeps that
/// height.
fn maintains_aspect_ratio(image: &NodeRef, style: &Style) -> bool {
    let number = |value: Option<String>| {
        value.and_then(|v| v.trim().trim_end_matches("px").parse::<f64>().ok())
    };
    let width_attr = number(dom::attr(image, "width")).filter(|w| *w >= 0.0);
    let height_attr = number(dom::attr(image, "height")).filter(|h| *h >= 0.0);
    let (width_style, height_style) = (style.get("width"), style.get("height"));
    let mut height = None;
    if width_attr.is_some()
        && height_attr.is_some()
        && width_style != "auto"
        && height_style != "auto"
    {
        height = height_attr;
    }
    let styled = !width_style.is_empty()
        && width_style != "auto"
        && !height_style.is_empty()
        && height_style != "auto";
    if width_attr.is_some() == height_attr.is_some() || styled {
        let computed = |style: &str, attr: Option<f64>| match (style, attr) {
            ("", Some(attr)) => Some(format!("{attr}px")),
            ("", None) | ("auto", _) => None,
            (style, _) => Some(style.to_owned()),
        };
        if computed(width_style, width_attr).is_some()
            && let Some(px) = computed(height_style, height_attr)
                .and_then(|h| h.strip_suffix("px")?.trim().parse::<f64>().ok())
        {
            height = Some(px);
        }
    }
    height.is_some_and(|h| h >= 15.0)
}

/// Classes the restyle recolors by, and light backgrounds dropped from personal mail.
fn colorize(element: &NodeRef, state: &mut Inherited, parent: Inherited, is_marketing: bool) {
    let mut style = Style::of(element);
    let original_style = dom::attr(element, "style");
    if let Some(bgcolor) = dom::attr(element, "bgcolor")
        && style.get("background-color").is_empty()
    {
        style.set("background-color", &bgcolor, false);
        style.apply(element);
        if let Some(color) = Rgba::parse(&bgcolor) {
            state.background = color;
        }
    }
    if state.background_image {
        dom::add_class(element, &["exo-color", "exo-original-color"]);
        if !style.get("color").is_empty() {
            dom::add_class(element, &["exo-modified-inline"]);
            let color = style.get("color").to_owned();
            style.set("--exo-original-color", &color, false);
            style.apply(element);
        }
        if let Some(original) = &original_style {
            dom::set_attr(element, "exo-original-style", original);
        }
        state.colorize_children = false;
        state.has_background = true;
        return;
    }
    let threshold = if state.in_banner { 50 } else { 10 };
    if !state.background.is_transparent()
        && Rgba::WHITE.similar_to(state.background, threshold)
        && !parent.has_background
        && !is_marketing
    {
        let mut changed = false;
        for property in ["background-color", "background"] {
            if !style.get(property).is_empty() {
                style.remove(property);
                style.set(property, "transparent", true);
                changed = true;
            }
        }
        if changed {
            style.apply(element);
            dom::add_class(element, &["exo-modified-inline"]);
            if let Some(original) = &original_style {
                dom::set_attr(element, "exo-original-style", original);
            }
        }
        dom::add_class(element, &["exo-color"]);
        state.background = parent.background;
    }
    let visible_background = !state.background.is_transparent() && state.background.is_visible();
    state.has_background = parent.has_background || visible_background;
    let name = snap(state.color);
    if visible_background {
        let light = themed(false, name).readable_on(state.background);
        let dark = themed(true, name).readable_on(state.background);
        if !light {
            dom::add_class(element, &["exo-color", "exo-original-color-snow"]);
        }
        if !dark {
            dom::add_class(element, &["exo-color", "exo-original-color-carbon"]);
        }
        if !light || !dark {
            dom::add_class(element, &["exo-modified-inline"]);
            let mut style = Style::of(element);
            style.set("--exo-original-color", &state.color.css(), false);
            style.apply(element);
        }
        return;
    }
    if style.is_important("color") {
        let color = style.get("color").to_owned();
        style.set("color", &color, false);
        style.apply(element);
    }
    dom::add_class(element, &[&format!("exo-color-{name}"), "exo-color"]);
}

/// Drops the sender's dark-mode CSS: rules mentioning `color-scheme` and
/// `@media (prefers-color-scheme)` blocks.
fn drop_color_scheme_rules(css: &str) -> String {
    let mut kept = String::with_capacity(css.len());
    for rule in top_level_rules(css) {
        let prelude = rule
            .split('{')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let is_media = prelude.starts_with("@media");
        let dark_media =
            is_media && (prelude.contains("prefers-color-scheme") || prelude.ends_with("not all"));
        if dark_media || rule.contains("color-scheme") {
            continue;
        }
        kept.push_str(rule);
    }
    kept
}

/// Splits a stylesheet into its top-level rules (statements and blocks), comments dropped.
fn top_level_rules(css: &str) -> Vec<&str> {
    let bytes = css.as_bytes();
    let mut rules = Vec::new();
    let (mut start, mut depth, mut i) = (0, 0usize, 0);
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                let end = css[i + 2..].find("*/").map_or(css.len(), |e| i + 2 + e + 2);
                if depth == 0 && css[start..i].trim().is_empty() {
                    start = end;
                }
                i = end;
                continue;
            }
            quote @ (b'"' | b'\'') => {
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    rules.push(&css[start..=i]);
                    start = i + 1;
                }
            }
            b';' if depth == 0 => {
                rules.push(&css[start..=i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if !css[start..].trim().is_empty() {
        rules.push(&css[start..]);
    }
    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_grey_spellings() {
        assert_eq!(Rgba::parse("grey"), Some(Rgba([128, 128, 128, 255])));
        assert_eq!(Rgba::parse("DarkSlateGrey"), Rgba::parse("darkslategray"));
    }

    #[test]
    fn parses_css_colors() {
        assert_eq!(css_color("rgb(222, 84, 84)"), Some([222, 84, 84, 255]));
        assert_eq!(css_color(" #a0b "), Some([170, 0, 187, 255]));
        assert_eq!(css_color("red"), Some([255, 0, 0, 255]));
        assert_eq!(css_color("rgba(0, 0, 0, 0.5)"), Some([0, 0, 0, 128]));
        assert_eq!(css_color("#aééa"), None);
        assert_eq!(css_color(""), None);
    }

    #[test]
    fn snaps_to_the_palette() {
        assert_eq!(snap(Rgba([0, 0, 0, 255])), "black");
        assert_eq!(snap(Rgba([34, 34, 34, 255])), "black");
        assert_eq!(snap(Rgba([0, 0, 238, 255])), "blue");
        assert_eq!(snap(Rgba([204, 0, 0, 255])), "red");
        assert_eq!(snap(Rgba([136, 136, 136, 255])), "grey");
    }

    #[test]
    fn tags_text_and_drops_light_backgrounds() {
        let document = dom::parse(
            "<div>a<span style=\"color:#c00\">b</span><a href=\"x\">c</a><td bgcolor=\"#fefefe\">d</td></div>",
        );
        let body = dom::body(&document).unwrap();
        carbonize(&body, false);
        let html = dom::inner_html(&body);
        assert!(
            html.starts_with("<div class=\"exo-color-black exo-color\">"),
            "{html}"
        );
        assert!(
            html.contains("<span style=\"color:#c00\" class=\"exo-color-red exo-color\">"),
            "{html}"
        );
        assert!(
            html.contains("<a href=\"x\" class=\"exo-color-blue exo-color\">"),
            "{html}"
        );
    }

    #[test]
    fn keeps_the_aspect_ratio_of_images_sized_both_ways() {
        let classes = |img: &str| {
            let document = dom::parse(img);
            let body = dom::body(&document).unwrap();
            carbonize(&body, true);
            let image = dom::select_first(&body, |n| dom::is(n, "img")).unwrap();
            dom::attr(&image, "class").unwrap_or_default()
        };
        let keeps = |img: &str| classes(img).contains("exo-im-maintain-aspect-ratio");
        assert!(keeps("<img width=\"46\" height=\"29\">"));
        assert!(keeps("<img style=\"width: 24px; height: 24px\">"));
        assert!(keeps(
            "<img width=\"24\" height=\"30\" style=\"height: 24px\">"
        ));
        assert!(!keeps("<img width=\"14\" height=\"14\">"), "under 15px");
        assert!(
            !keeps("<img width=\"160\" height=\"90\" style=\"height: auto; width: 100%\">"),
            "height: auto"
        );
        // Kickstarter's step icons: their width is `auto`, so they stay 39px tall.
        assert!(!keeps("<img style=\"min-width: 39px; height: 39px\">"));
        assert!(!keeps(
            "<img height=\"1\" style=\"min-width: 540px; height: 1px\">"
        ));
    }

    #[test]
    fn removes_dark_mode_css() {
        assert_eq!(
            drop_color_scheme_rules(
                "p{color:red}@media (prefers-color-scheme: dark){p{color:white}}:root{color-scheme:light dark}a{b:c}"
            ),
            "p{color:red}a{b:c}"
        );
    }
}
