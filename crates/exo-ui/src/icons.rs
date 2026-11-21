//! Exo's icons, compiled in from `assets/icons` (Material Symbols and open brand marks, each fitted
//! into its icon's box): monochrome icons white (so a tint colors them), and themed icons as one
//! file per theme (`<name>.light.svg`, `<name>.dark.svg`).
//!
//! The Classic style draws the legacy set (`legacy/…`), which CSS colors through placeholder
//! fills: `red` (written white, so a tint colors it) and sometimes `blue`
//! ([`paint_icon_two_tone`]). A legacy icon keeps its intrinsic size ([`icon_size`]).

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use egui::{Color32, Context, Id, Image, Rect, Ui, Vec2};

use crate::Theme;

const URI_PREFIX: &str = "bytes://exo-icon/";
const THEME_SUFFIXES: [(&str, Theme); 2] = [(".light", Theme::Snow), (".dark", Theme::Carbon)];

/// Every icon's SVG, by its path under `assets/icons` without the extension: `shell/tab-add`,
/// `legacy/close`, … (`build.rs`).
const FILES: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/icons.rs"));

/// What the registered icons are: which come in a variant per theme, each one's intrinsic size
/// (its root `width` and `height` in px, where it has them) and its `viewBox`'s size.
#[derive(Default)]
struct Registry {
    themed: HashSet<String>,
    sizes: HashMap<String, Vec2>,
    view_boxes: HashMap<String, Vec2>,
}

fn registry_id() -> Id {
    Id::new("exo_ui::icons")
}

fn registry(ctx: &Context) -> Option<Arc<Registry>> {
    ctx.data(|data| data.get_temp::<Arc<Registry>>(registry_id()))
}

/// Registers every icon with egui's byte loader under its name.
pub(crate) fn register(ctx: &Context) {
    let mut registry = Registry::default();
    for &(name, bytes) in FILES {
        let base = THEME_SUFFIXES
            .iter()
            .find_map(|(suffix, _)| name.strip_suffix(suffix));
        if let Some(base) = base {
            registry.themed.insert(base.to_owned());
        }
        let (size, view_box) = root_size(bytes);
        let key = base.unwrap_or(name).to_owned();
        if let Some(size) = size {
            registry.sizes.insert(key.clone(), size);
        }
        if let Some(view_box) = view_box {
            registry.view_boxes.insert(key, view_box);
        }
        ctx.include_bytes(format!("{URI_PREFIX}{name}.svg"), bytes);
    }
    ctx.data_mut(|data| data.insert_temp(registry_id(), Arc::new(registry)));
}

/// Icon `name`'s SVG, for drawing it outside egui (a printed page).
pub fn icon_svg(name: &str) -> Option<&'static str> {
    let (_, bytes) = FILES.iter().find(|(file, _)| *file == name)?;
    std::str::from_utf8(bytes).ok()
}

/// Paints icon `name` scaled to fill `rect`, tinted `color` (use white for icons that carry
/// their own colors). Themed icons use the current theme's variant.
pub fn paint_icon(ui: &Ui, name: &str, rect: Rect, color: Color32) {
    Image::new(icon_uri(ui.ctx(), name))
        .tint(color)
        .paint_at(ui, fit(ui.ctx(), name, rect));
}

/// [`paint_icon`] turned `angle` radians clockwise about its centre (`transform: rotate(…)`).
pub fn paint_icon_rotated(ui: &Ui, name: &str, rect: Rect, color: Color32, angle: f32) {
    Image::new(icon_uri(ui.ctx(), name))
        .tint(color)
        .rotate(angle, egui::Vec2::splat(0.5))
        .paint_at(ui, fit(ui.ctx(), name, rect));
}

/// Where in `rect` icon `name` draws: its `viewBox` scaled to fit and centered, as an `<svg>`
/// sized to `rect` draws (`preserveAspectRatio`'s default, `xMidYMid meet`).
pub(crate) fn fit(ctx: &Context, name: &str, rect: Rect) -> Rect {
    let Some(view_box) = registry(ctx).and_then(|registry| registry.view_boxes.get(name).copied())
    else {
        return rect;
    };
    let scale = (rect.width() / view_box.x).min(rect.height() / view_box.y);
    Rect::from_center_size(rect.center(), view_box * scale)
}

/// [`paint_icon`] through `painter`, inside its clip (an icon in a scrolling pane).
pub fn paint_icon_on(painter: &egui::Painter, name: &str, rect: Rect, color: Color32) {
    let ctx = painter.ctx();
    let pixels_per_point = painter.pixels_per_point();
    let rect = egui::emath::GuiRounding::round_to_pixels(fit(ctx, name, rect), pixels_per_point);
    let size = (rect.size() * pixels_per_point).round();
    let image = Image::new(icon_uri(ctx, name)).tint(color);
    let hint = egui::SizeHint::Size {
        width: size.x as _,
        height: size.y as _,
        maintain_aspect_ratio: false,
    };
    if let Ok(egui::load::TexturePoll::Ready { texture }) =
        image
            .source(ctx)
            .load(ctx, egui::TextureOptions::default(), hint)
    {
        egui::widgets::paint_texture_at(painter, rect, image.image_options(), &texture);
    }
}

/// The URI icon `name` is registered under, in the current theme's variant when it has them.
pub(crate) fn icon_uri(ctx: &egui::Context, name: &str) -> String {
    let is_themed = registry(ctx).is_some_and(|registry| registry.themed.contains(name));
    if is_themed {
        let theme = crate::theme(ctx);
        let (suffix, _) = THEME_SUFFIXES
            .iter()
            .find(|(_, variant)| *variant == theme)
            .expect("every theme has a suffix");
        format!("{URI_PREFIX}{name}{suffix}.svg")
    } else {
        format!("{URI_PREFIX}{name}.svg")
    }
}

/// [`paint_icon`] for a legacy icon whose placeholder strokes CSS widens (`stroke-width`, in the
/// icon's own units), as a done row's check widens to `stroke-width: 2`: the icon's heavier cut,
/// `<name>-stroke<stroke>`, which the build fits to that width.
pub fn paint_icon_stroked(ui: &Ui, name: &str, rect: Rect, color: Color32, stroke: f32) {
    paint_icon(ui, &format!("{name}-stroke{stroke}"), rect, color);
}

/// The URI of icon `name` with `edit` applied to its SVG, registered the first time it's asked
/// for under `key`; `None` until the icon's bytes are loaded.
pub(crate) fn edited_icon(
    ctx: &Context,
    name: &str,
    key: &str,
    edit: impl FnOnce(String) -> String,
) -> Option<String> {
    let uri = format!("{URI_PREFIX}{name}+{key}.svg");
    let known_id = Id::new("exo_ui::edited_icons");
    let known = ctx
        .data(|data| data.get_temp::<Arc<HashSet<String>>>(known_id))
        .is_some_and(|known| known.contains(&uri));
    if !known {
        let Ok(egui::load::BytesPoll::Ready { bytes, .. }) =
            ctx.try_load_bytes(&format!("{URI_PREFIX}{name}.svg"))
        else {
            return None;
        };
        ctx.include_bytes(
            uri.clone(),
            edit(String::from_utf8_lossy(&bytes).into_owned()).into_bytes(),
        );
        ctx.data_mut(|data| {
            let mut known: HashSet<String> = data
                .get_temp::<Arc<HashSet<String>>>(known_id)
                .map(|known| (*known).clone())
                .unwrap_or_default();
            known.insert(uri.clone());
            data.insert_temp(known_id, Arc::new(known));
        });
    }
    Some(uri)
}

/// Icon `name`'s intrinsic size: its SVG's `width` and `height` in px. `None` for icons sized by
/// their container (`width="100%"`) and unknown names.
pub fn icon_size(ctx: &Context, name: &str) -> Option<Vec2> {
    registry(ctx).and_then(|registry| registry.sizes.get(name).copied())
}

/// Where icon `name` sits in a box `within` that centers it: at its intrinsic size on the whole
/// pixel Chromium paints SVGs at, or filling the box for an icon without one.
pub fn icon_rect(ctx: &Context, name: &str, within: Rect) -> Rect {
    icon_size(ctx, name).map_or(within, |size| {
        crate::pixel_snap(Rect::from_center_size(within.center(), size))
    })
}

/// Paints legacy icon `name` filling `rect` with its `red` placeholder (written white by the
/// importer) in `red` and its `blue` one in `blue`, as Classic's CSS colors icons like the
/// contact pane's (`[fill*=red]`, `[fill*=blue]`).
pub fn paint_icon_two_tone(ui: &Ui, name: &str, rect: Rect, red: Color32, blue: Color32) {
    let hex = |color: Color32| {
        let [r, g, b, a] = color.to_srgba_unmultiplied();
        (format!("#{r:02x}{g:02x}{b:02x}"), f32::from(a) / 255.0)
    };
    let ((red_hex, red_alpha), (blue_hex, blue_alpha)) = (hex(red), hex(blue));
    let edit = |mut svg: String| {
        for attribute in ["fill", "stroke"] {
            svg = svg.replace(
                &format!(r##"{attribute}="#fff""##),
                &format!(r#"{attribute}="{red_hex}" {attribute}-opacity="{red_alpha}""#),
            );
            svg = svg.replace(
                &format!(r#"{attribute}="blue""#),
                &format!(r#"{attribute}="{blue_hex}" {attribute}-opacity="{blue_alpha}""#),
            );
        }
        svg
    };
    // No `#` in the key: a URI's fragment isn't part of what loaders look up.
    let key = format!("{red_hex}{red_alpha}{blue_hex}{blue_alpha}").replace('#', "");
    if let Some(uri) = edited_icon(ui.ctx(), name, &key, edit) {
        Image::new(uri).paint_at(ui, fit(ui.ctx(), name, rect));
    }
}

/// The root `<svg>`'s `width` and `height` in px (`"16"` or `"16px"`), when both are given that way.
fn root_size(svg: &[u8]) -> (Option<Vec2>, Option<Vec2>) {
    let root = std::str::from_utf8(svg).ok().and_then(|svg| {
        let root = &svg[svg.find("<svg")?..];
        Some(&root[..root.find('>')?])
    });
    let Some(root) = root else {
        return (None, None);
    };
    let attribute = |name: &str| -> Option<&str> {
        let start = root.find(&format!(" {name}=\""))? + name.len() + 3;
        Some(&root[start..start + root[start..].find('"')?])
    };
    let px = |name: &str| -> Option<f32> {
        let value = attribute(name)?;
        value.strip_suffix("px").unwrap_or(value).parse().ok()
    };
    let size = px("width")
        .zip(px("height"))
        .map(|(width, height)| Vec2::new(width, height));
    let numbers: Option<Vec<f32>> = attribute("viewBox").and_then(|value| {
        value
            .split([' ', ','])
            .filter(|part| !part.is_empty())
            .map(|part| part.parse().ok())
            .collect()
    });
    let view_box = match numbers.as_deref() {
        Some(&[_, _, width, height]) if width > 0.0 && height > 0.0 => {
            Some(Vec2::new(width, height))
        }
        _ => None,
    };
    (size, view_box)
}

/// Classic's spinner in `rect`, in `color`, at `now`: a 7.5-unit circle of a 17-unit box stroked
/// 1.25 wide with a 40-unit dash, so a sixth of it is open, turning clockwise once a second
/// (`1000ms infinite linear`). It asks for the next frame.
pub fn paint_spinner(painter: &egui::Painter, rect: Rect, now: f64, color: Color32) {
    const STEPS: usize = 48;
    let scale = rect.width() / 17.0;
    let radius = 7.5 * scale;
    let sweep = 40.0 / 7.5;
    let turn = (now % 1.0) as f32 * std::f32::consts::TAU;
    let points: Vec<egui::Pos2> = (0..=STEPS)
        .map(|step| {
            rect.center() + radius * Vec2::angled(turn + sweep * step as f32 / STEPS as f32)
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.25 * scale, color),
    ));
    painter.ctx().request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_compiled_in_with_its_geometry() {
        assert!(FILES.len() > 300, "assets/icons is built");
        for (name, bytes) in FILES {
            let (size, view_box) = root_size(bytes);
            assert!(
                size.is_some() || view_box.is_some(),
                "{name} has neither a size nor a viewBox"
            );
        }
        // `paint_icon_stroked`'s heavier cut sits beside the icon it widens.
        assert!(icon_svg("legacy/DEPRECATED_threadActionsDone-stroke2").is_some());
    }

    #[test]
    fn sizes_come_from_the_root_element() {
        let (v, none) = (Vec2::new, None);
        assert_eq!(
            root_size(
                br#"<svg width="14" height="12" viewBox="0 0 14 12"><rect width="12.9"/></svg>"#
            ),
            (Some(v(14.0, 12.0)), Some(v(14.0, 12.0)))
        );
        assert_eq!(
            root_size(br#"<svg width="19px" height="8px" viewBox="0 0 18 8"/>"#),
            (Some(v(19.0, 8.0)), Some(v(18.0, 8.0)))
        );
        assert_eq!(
            root_size(br#"<svg width="100%" height="100%" viewBox="-298 390.6 14 12.4"/>"#),
            (none, Some(v(14.0, 12.4)))
        );
        assert_eq!(
            root_size(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0,0,15,15"/>"#),
            (none, Some(v(15.0, 15.0)))
        );
        assert_eq!(
            root_size(br#"<svg width="100%" height="100%"/>"#),
            (none, none)
        );
    }
}
