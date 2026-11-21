//! `Download all`'s zip: a message's files in one archive, deflated, the second of two names
//! `name (1).ext` and so on, named for the conversation.

use std::collections::HashMap;
use std::io::Write as _;

/// The zip's name: the subject lowercased, each run of one or two spaces an underscore, anything
/// else that isn't a letter, digit or underscore dropped, then the message's place in the
/// conversation, counting from 1; `attachments.zip` without a subject.
pub(crate) fn name(subject: &str, position: usize) -> String {
    if subject.is_empty() {
        return "attachments.zip".into();
    }
    let mut slug = String::new();
    let mut chars = subject.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            // `/\s\s?/g`: a second space goes with the first.
            chars.next_if(|next| next.is_whitespace());
            slug.push('_');
        } else if c.is_ascii_alphanumeric() || c == '_' {
            slug.push(c.to_ascii_lowercase());
        }
    }
    format!("{slug}_{}.zip", position + 1)
}

/// The names the files go in under: each as it is, except that a name taken already gets ` (n)`
/// before its extension, counting how often it came up.
fn unique(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut unique_name = |name: String| -> String {
        let mut name = name;
        loop {
            let Some(count) = seen.get(&name).copied() else {
                seen.insert(name.clone(), 1);
                return name;
            };
            seen.insert(name.clone(), count + 1);
            name = match name.rfind('.').filter(|dot| *dot > 0) {
                Some(dot) => format!("{} ({count}){}", &name[..dot], &name[dot..]),
                None => format!("{name} ({count})"),
            };
        }
    };
    names.into_iter().map(&mut unique_name).collect()
}

/// `files` (name and bytes) as a zip, deflated, dated `now` (DOS time). Names are made safe for a
/// file system first, as downloads' are.
pub(crate) fn archive(files: &[(String, Vec<u8>)], now: &jiff::civil::DateTime) -> Vec<u8> {
    let (time, date) = dos_time(now);
    let names = unique(files.iter().map(|(name, _)| super::file_name(name)));
    let mut out = Vec::new();
    let mut central = Vec::new();
    for ((_, data), name) in files.iter().zip(&names) {
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder
            .write_all(data)
            .expect("deflating into memory doesn't fail");
        let compressed = encoder
            .finish()
            .expect("deflating into memory doesn't fail");
        let offset = out.len();
        let fields = Entry {
            crc: crc.sum(),
            compressed: compressed.len(),
            size: data.len(),
            time,
            date,
            name,
        };
        // Local file header.
        out.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        fields.write(&mut out);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&compressed);
        // Its central directory entry.
        central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        central.extend_from_slice(&20_u16.to_le_bytes());
        fields.write(&mut central);
        // No comment, disk 0, no internal or external attributes, then where the local header is.
        central.extend_from_slice(&[0; 10]);
        central.extend_from_slice(&u32::try_from(offset).unwrap_or(u32::MAX).to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let (start, length) = (out.len(), central.len());
    out.extend_from_slice(&central);
    // End of central directory.
    let count = u16::try_from(files.len()).unwrap_or(u16::MAX).to_le_bytes();
    out.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&count);
    out.extend_from_slice(&count);
    out.extend_from_slice(&u32::try_from(length).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(&u32::try_from(start).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(&[0; 2]);
    out
}

/// What a local header and a central directory entry share, from "version needed" to the
/// extra field's length.
struct Entry<'a> {
    crc: u32,
    compressed: usize,
    size: usize,
    time: u16,
    date: u16,
    name: &'a str,
}

impl Entry<'_> {
    fn write(&self, out: &mut Vec<u8>) {
        let size = |bytes: usize| u32::try_from(bytes).unwrap_or(u32::MAX).to_le_bytes();
        // Version 2.0, UTF-8 names (bit 11), deflated.
        out.extend_from_slice(&20_u16.to_le_bytes());
        out.extend_from_slice(&0x0800_u16.to_le_bytes());
        out.extend_from_slice(&8_u16.to_le_bytes());
        out.extend_from_slice(&self.time.to_le_bytes());
        out.extend_from_slice(&self.date.to_le_bytes());
        out.extend_from_slice(&self.crc.to_le_bytes());
        out.extend_from_slice(&size(self.compressed));
        out.extend_from_slice(&size(self.size));
        out.extend_from_slice(
            &u16::try_from(self.name.len())
                .unwrap_or(u16::MAX)
                .to_le_bytes(),
        );
        out.extend_from_slice(&0_u16.to_le_bytes());
    }
}

/// `at` as MS-DOS's time and date, to the even second.
fn dos_time(at: &jiff::civil::DateTime) -> (u16, u16) {
    let time = (u16::try_from(at.hour()).unwrap_or(0) << 11)
        | (u16::try_from(at.minute()).unwrap_or(0) << 5)
        | (u16::try_from(at.second()).unwrap_or(0) / 2);
    let date = (u16::try_from(at.year() - 1980).unwrap_or(0) << 9)
        | (u16::try_from(at.month()).unwrap_or(1) << 5)
        | u16::try_from(at.day()).unwrap_or(1);
    (time, date)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zip_is_named_for_the_conversation() {
        assert_eq!(name("Board deck — draft 2", 2), "board_deck__draft_2_3.zip");
        assert_eq!(
            name("Q3  numbers", 0),
            "q3_numbers_1.zip",
            "two spaces make one underscore"
        );
        assert_eq!(name("", 0), "attachments.zip");
    }

    #[test]
    fn a_name_taken_gets_a_number() {
        let names =
            unique(["deck.pdf", "deck.pdf", "deck.pdf", "deck (1).pdf", "notes"].map(String::from));
        assert_eq!(
            names,
            [
                "deck.pdf",
                "deck (1).pdf",
                "deck (2).pdf",
                "deck (1) (1).pdf",
                "notes"
            ]
        );
    }

    #[test]
    fn the_archive_opens() {
        let files = vec![
            ("a.txt".to_owned(), b"hello hello hello".to_vec()),
            ("a.txt".to_owned(), b"second".to_vec()),
        ];
        let zip = archive(&files, &jiff::civil::date(2026, 11, 17).at(9, 30, 4, 0));
        let dir = std::env::temp_dir().join(format!("exo-zip-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a folder");
        let path = dir.join("files.zip");
        std::fs::write(&path, &zip).expect("written");
        // `unzip` reads it back whole, checking each file's CRC.
        let status = std::process::Command::new("unzip")
            .arg("-q")
            .arg(&path)
            .arg("-d")
            .arg(&dir)
            .status()
            .expect("unzip runs");
        assert!(status.success());
        assert_eq!(
            std::fs::read(dir.join("a.txt")).expect("first"),
            b"hello hello hello"
        );
        assert_eq!(
            std::fs::read(dir.join("a (1).txt")).expect("second"),
            b"second"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
