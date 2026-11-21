//! Image attachments' card thumbnails: the image cover-cropped to the 3:2 card
//! (`object-fit: cover`), at most as many pixels as the widest card takes on screen, with the
//! card's resting overlay baked in (a radial gradient from white to black, both at .15, blended
//! `overlay`), since it never changes.

use egui::ColorImage;

use super::super::thread::attachments::MAX_THUMBNAIL_SIDE;

/// A card's width at most, 3:2.
const CARD_WIDTH: f32 = 154.16;
const ASPECT: f32 = 1.0 / 0.6666;

/// `bytes`' thumbnail for a screen of `pixels_per_point`, or `None` when they aren't an image
/// Exo decodes or it's 10,000px or more either way.
pub(crate) fn thumbnail(bytes: &[u8], pixels_per_point: f32) -> Option<ColorImage> {
    let image = image::load_from_memory(bytes).ok()?;
    if image.width() >= MAX_THUMBNAIL_SIDE
        || image.height() >= MAX_THUMBNAIL_SIDE
        || image.width() == 0
        || image.height() == 0
    {
        return None;
    }
    // The middle of the image at the card's shape.
    let (width, height) = (image.width() as f32, image.height() as f32);
    let (crop_width, crop_height) = if width / height > ASPECT {
        (height * ASPECT, height)
    } else {
        (width, width / ASPECT)
    };
    let cropped = image.crop_imm(
        ((width - crop_width) / 2.0).round() as u32,
        ((height - crop_height) / 2.0).round() as u32,
        crop_width.round().max(1.0) as u32,
        crop_height.round().max(1.0) as u32,
    );
    let target = (CARD_WIDTH * pixels_per_point).ceil() as u32;
    let resized = if cropped.width() > target {
        cropped.resize_exact(
            target,
            (target as f32 / ASPECT).round() as u32,
            image::imageops::FilterType::Triangle,
        )
    } else {
        cropped
    };
    let mut rgba = resized.to_rgba8();
    overlay(&mut rgba);
    Some(ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ))
}

/// Blends `radial-gradient(circle, rgba(255,255,255,.15), rgba(0,0,0,.15))` (centered, out to
/// the farthest corner) over `image` with `mix-blend-mode: overlay`.
fn overlay(image: &mut image::RgbaImage) {
    let (width, height) = (image.width() as f32, image.height() as f32);
    let (cx, cy) = (width / 2.0, height / 2.0);
    let reach = (cx * cx + cy * cy).sqrt().max(1.0);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
        // White at the centre to black at the corners, at a constant .15.
        let source = 1.0 - ((dx * dx + dy * dy).sqrt() / reach).min(1.0);
        for channel in &mut pixel.0[..3] {
            let base = f32::from(*channel) / 255.0;
            let blended = if base <= 0.5 {
                2.0 * base * source
            } else {
                1.0 - 2.0 * (1.0 - base) * (1.0 - source)
            };
            *channel = ((base + 0.15 * (blended - base)) * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(width, height, image::Rgba([128, 128, 128, 255]))
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .expect("encoded");
        bytes
    }

    #[test]
    fn an_image_is_cropped_to_the_card_and_kept_small() {
        let thumbnail = thumbnail(&png(1200, 400), 2.0).expect("a thumbnail");
        // 3:2, at most the widest card's 308 device pixels.
        assert_eq!(thumbnail.size, [309, 206]);
        let small = super::thumbnail(&png(30, 30), 2.0).expect("a thumbnail");
        assert_eq!(small.size, [30, 20], "never enlarged");
    }

    #[test]
    fn the_overlay_lightens_the_middle_and_darkens_the_corners() {
        let thumbnail = thumbnail(&png(300, 200), 1.0).expect("a thumbnail");
        let [width, height] = thumbnail.size;
        let middle = thumbnail.pixels[height / 2 * width + width / 2].r();
        let corner = thumbnail.pixels[0].r();
        assert!(middle > 128 && corner < 128, "{middle} {corner}");
    }

    #[test]
    fn huge_and_broken_images_have_no_thumbnail() {
        assert!(thumbnail(b"not an image", 2.0).is_none());
        assert!(thumbnail(&png(10_000, 10), 2.0).is_none());
    }
}
