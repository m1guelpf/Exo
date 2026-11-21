//! Small but valid attachment files: PDF, PNG, CSV and iCalendar.

use jiff::Timestamp;

/// A one-page PDF with a title and lines of text (Helvetica, Letter size).
pub fn pdf(title: &str, lines: &[&str]) -> Vec<u8> {
    let escape = |text: &str| {
        text.replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)")
    };
    let mut content = format!("BT /F1 20 Tf 72 712 Td ({}) Tj ET\n", escape(title));
    for (index, line) in lines.iter().enumerate() {
        let y = 676 - 18 * index;
        content.push_str(&format!(
            "BT /F1 11 Tf 72 {y} Td ({}) Tj ET\n",
            escape(line)
        ));
    }
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
            .to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!(
            "<< /Length {} >>\nstream\n{content}endstream",
            content.len()
        ),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// A palette PNG of a bar chart: `bars` heights in 0..=100, drawn on white.
pub fn chart_png(bars: &[u8]) -> Vec<u8> {
    const WIDTH: usize = 240;
    const HEIGHT: usize = 140;
    // White, grid gray, and Brightmoor's chart blue.
    let palette = [0xff, 0xff, 0xff, 0xe5, 0xe7, 0xeb, 0x3b, 0x82, 0xf6];
    let slot = WIDTH / bars.len().max(1);
    let mut raw = Vec::with_capacity((WIDTH + 1) * HEIGHT);
    for y in 0..HEIGHT {
        raw.push(0); // filter: none
        for x in 0..WIDTH {
            let bar = bars.get(x / slot).copied().unwrap_or(0) as usize;
            let top = HEIGHT - 10 - (HEIGHT - 20) * bar.min(100) / 100;
            let in_bar =
                x % slot > slot / 5 && x % slot < slot * 4 / 5 && y >= top && y < HEIGHT - 10;
            raw.push(if in_bar {
                2
            } else if y % 30 == 0 {
                1
            } else {
                0
            });
        }
    }
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&(WIDTH as u32).to_be_bytes());
    header.extend_from_slice(&(HEIGHT as u32).to_be_bytes());
    header.extend_from_slice(&[8, 3, 0, 0, 0]); // 8-bit palette, no interlace
    chunk(&mut png, b"IHDR", &header);
    chunk(&mut png, b"PLTE", &palette);
    chunk(&mut png, b"IDAT", &zlib_stored(&raw));
    chunk(&mut png, b"IEND", &[]);
    png
}

/// Maya's profile photo, `size` pixels square (Google's `=s{size}`): an invented portrait in
/// soft shapes on a sky gradient, each pixel the average of 4×4 samples, so every size shows
/// the same picture.
pub fn portrait_png(size: u32) -> Vec<u8> {
    const SAMPLES: u32 = 4;
    let side = size as usize;
    let mut raw = Vec::with_capacity((side * 3 + 1) * side);
    for y in 0..size {
        raw.push(0); // filter: none
        for x in 0..size {
            let mut sum = [0.0f32; 3];
            for (sx, sy) in (0..SAMPLES * SAMPLES).map(|i| (i % SAMPLES, i / SAMPLES)) {
                let at = |pixel: u32, sample: u32| {
                    (pixel as f32 + (sample as f32 + 0.5) / SAMPLES as f32) / size as f32
                };
                let color = portrait(at(x, sx), at(y, sy));
                for (total, channel) in sum.iter_mut().zip(color) {
                    *total += f32::from(channel);
                }
            }
            raw.extend(sum.map(|total| (total / (SAMPLES * SAMPLES) as f32).round() as u8));
        }
    }
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&size.to_be_bytes());
    header.extend_from_slice(&size.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit RGB, no interlace
    chunk(&mut png, b"IHDR", &header);
    chunk(&mut png, b"IDAT", &zlib_stored(&raw));
    chunk(&mut png, b"IEND", &[]);
    png
}

/// The portrait's color at `u` across and `v` down, both 0 to 1: hair, face, neck and sweater
/// over the sky.
fn portrait(u: f32, v: f32) -> [u8; 3] {
    let inside = |cx: f32, cy: f32, rx: f32, ry: f32| {
        ((u - cx) / rx).powi(2) + ((v - cy) / ry).powi(2) <= 1.0
    };
    if inside(0.5, 0.47, 0.155, 0.19) && v > 0.36 {
        [0xd9, 0xa0, 0x7a]
    } else if inside(0.5, 0.42, 0.2, 0.24) {
        [0x3a, 0x25, 0x1c]
    } else if (0.44..0.56).contains(&u) && (0.55..0.8).contains(&v) {
        [0xc4, 0x8a, 0x66]
    } else if inside(0.5, 1.02, 0.38, 0.3) {
        [0x2f, 0x6b, 0x5e]
    } else {
        let mix = |top: u8, bottom: u8| {
            (f32::from(top) + (f32::from(bottom) - f32::from(top)) * v).round() as u8
        };
        [mix(0xa7, 0x6f), mix(0xc7, 0x9f), mix(0xe7, 0xd0)]
    }
}

fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = png.len();
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let crc = crc32(&png[start..]);
    png.extend_from_slice(&crc.to_be_bytes());
}

/// A zlib stream of uncompressed deflate blocks: valid, if not small.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = data.chunks(65_535).collect();
    for (index, block) in blocks.iter().enumerate() {
        out.push(u8::from(index + 1 == blocks.len()));
        let len = block.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    out.extend_from_slice(&((b << 16) | a).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// An iTIP `REQUEST` for a meeting.
pub struct Invite<'a> {
    pub uid: &'a str,
    pub summary: &'a str,
    pub description: &'a str,
    pub location: &'a str,
    pub start: Timestamp,
    pub end: Timestamp,
    pub organizer: (&'a str, &'a str),
    pub attendees: &'a [(&'a str, &'a str)],
    pub stamp: Timestamp,
}

impl Invite<'_> {
    pub fn ics(&self) -> String {
        let utc = |at: Timestamp| at.strftime("%Y%m%dT%H%M%SZ").to_string();
        let mut lines = vec![
            "BEGIN:VCALENDAR".to_owned(),
            "PRODID:-//Brightmoor//Fixture Calendar//EN".to_owned(),
            "VERSION:2.0".to_owned(),
            "CALSCALE:GREGORIAN".to_owned(),
            "METHOD:REQUEST".to_owned(),
            "BEGIN:VEVENT".to_owned(),
            format!("DTSTART:{}", utc(self.start)),
            format!("DTEND:{}", utc(self.end)),
            format!("DTSTAMP:{}", utc(self.stamp)),
            format!(
                "ORGANIZER;CN={}:mailto:{}",
                self.organizer.0, self.organizer.1
            ),
            format!("UID:{}", self.uid),
        ];
        for (name, email) in self.attendees {
            lines.push(format!(
                "ATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE;CN={name}:mailto:{email}"
            ));
        }
        lines.extend([
            format!("DESCRIPTION:{}", self.description.replace('\n', "\\n")),
            format!("LOCATION:{}", self.location),
            "SEQUENCE:0".to_owned(),
            "STATUS:CONFIRMED".to_owned(),
            format!("SUMMARY:{}", self.summary),
            "TRANSP:OPAQUE".to_owned(),
            "END:VEVENT".to_owned(),
            "END:VCALENDAR".to_owned(),
        ]);
        lines.join("\r\n") + "\r\n"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_reference() {
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    }

    #[test]
    fn files_have_magic_numbers() {
        assert!(pdf("T", &["a"]).starts_with(b"%PDF-1.4"));
        assert!(pdf("T", &["a"]).ends_with(b"%%EOF\n"));
        let png = chart_png(&[40, 80, 60]);
        assert!(png.starts_with(b"\x89PNG"));
        assert!(png.ends_with(&[0xAE, 0x42, 0x60, 0x82]));
    }
}
