//! What a font file calls itself. `[style] font` can name a `.ttf` or `.otf`
//! file; the builder reads its family name and weight here so the window
//! asks for exactly that family, and refuses a file that isn't a font
//! Windows can load from memory.
//!
//! Only the `name` and `OS/2` tables are read, with every offset checked.

use anyhow::{bail, Result};

/// A font file's family and weight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontInfo {
    pub family: String,
    /// 100 (thin) to 900 (black); 400 when the file doesn't say.
    pub weight: u16,
}

/// The family and weight inside a TrueType or OpenType font file.
pub fn read(bytes: &[u8]) -> Result<FontInfo> {
    match bytes.get(..4) {
        Some([0, 1, 0, 0]) | Some(b"OTTO") | Some(b"true") => {}
        Some(b"wOFF") | Some(b"wOF2") => bail!("it's a web font (WOFF); use the .ttf or .otf file instead"),
        Some(b"ttcf") => bail!("it's a font collection (.ttc); use a single .ttf or .otf font"),
        _ => bail!("it isn't a TrueType or OpenType font"),
    }
    let family = table(bytes, b"name").and_then(family_name);
    let Some(family) = family.filter(|family| !family.trim().is_empty()) else {
        bail!("it has no family name");
    };
    let weight = table(bytes, b"OS/2").and_then(|os2| u16_at(os2, 4)).filter(|weight| (1..=1000).contains(weight)).unwrap_or(400);
    Ok(FontInfo { family: family.trim().to_string(), weight })
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    let end = at.checked_add(2)?;
    bytes.get(at..end).map(|b| u16::from_be_bytes([b[0], b[1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let end = at.checked_add(4)?;
    bytes.get(at..end).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// The bytes of the table tagged `tag`.
fn table<'a>(bytes: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
    let count = usize::from(u16_at(bytes, 4)?);
    (0..count).find_map(|index| {
        let record = 12usize.checked_add(index.checked_mul(16)?)?;
        if bytes.get(record..record.checked_add(4)?)? != tag {
            return None;
        }
        let offset = u32_at(bytes, record + 8)? as usize;
        let length = u32_at(bytes, record + 12)? as usize;
        bytes.get(offset..offset.checked_add(length)?)
    })
}

/// The family name Windows groups the font under: the WWS family (21),
/// else the typographic family (16), else the plain family (1). English
/// names on the Windows platform win over others.
fn family_name(name: &[u8]) -> Option<String> {
    let count = usize::from(u16_at(name, 2)?);
    let strings = usize::from(u16_at(name, 4)?);
    let mut best: Option<(u32, String)> = None;
    for index in 0..count {
        let record = 6usize.checked_add(index.checked_mul(12)?)?;
        let (Some(platform), Some(encoding), Some(language), Some(id), Some(length), Some(offset)) = (
            u16_at(name, record),
            u16_at(name, record + 2),
            u16_at(name, record + 4),
            u16_at(name, record + 6),
            u16_at(name, record + 8),
            u16_at(name, record + 10),
        ) else {
            continue;
        };
        let id_rank = match id {
            21 => 0,
            16 => 1,
            1 => 2,
            _ => continue,
        };
        let start = strings.checked_add(usize::from(offset))?;
        let Some(raw) = name.get(start..start.checked_add(usize::from(length))?) else {
            continue;
        };
        let (text, platform_rank) = match (platform, encoding) {
            (3, 1) | (3, 10) | (0, _) => (utf16(raw), if platform == 3 && language == 0x0409 { 0 } else { 1 }),
            (1, 0) => (Some(raw.iter().map(|&byte| char::from(byte)).collect()), 2),
            _ => continue,
        };
        let Some(text) = text.filter(|text: &String| !text.trim().is_empty()) else {
            continue;
        };
        let rank = id_rank * 10 + platform_rank;
        if best.as_ref().is_none_or(|(kept, _)| rank < *kept) {
            best = Some((rank, text));
        }
    }
    best.map(|(_, text)| text.chars().filter(|c| !c.is_control()).collect())
}

fn utf16(raw: &[u8]) -> Option<String> {
    let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|pair| u16::from_be_bytes(*pair)).collect();
    String::from_utf16(&units).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal font: an sfnt header, a `name` table with the given
    /// records and an `OS/2` table with the weight.
    fn font(names: &[(u16, u16, u16, u16, &str)], weight: u16) -> Vec<u8> {
        let mut strings = Vec::new();
        let mut records = Vec::new();
        for &(platform, encoding, language, id, text) in names {
            let encoded: Vec<u8> =
                if platform == 1 { text.bytes().collect() } else { text.encode_utf16().flat_map(u16::to_be_bytes).collect() };
            for value in [platform, encoding, language, id, encoded.len() as u16, strings.len() as u16] {
                records.extend(value.to_be_bytes());
            }
            strings.extend(encoded);
        }
        let mut name = Vec::new();
        name.extend(0u16.to_be_bytes());
        name.extend((names.len() as u16).to_be_bytes());
        name.extend((6 + records.len() as u16).to_be_bytes());
        name.extend(records);
        name.extend(strings);
        let mut os2 = vec![0u8; 8];
        os2[4..6].copy_from_slice(&weight.to_be_bytes());

        let header = 12 + 2 * 16;
        let mut out = vec![0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0];
        for (tag, offset, length) in [(b"name", header, name.len()), (b"OS/2", header + name.len(), os2.len())] {
            out.extend(tag);
            out.extend(0u32.to_be_bytes());
            out.extend((offset as u32).to_be_bytes());
            out.extend((length as u32).to_be_bytes());
        }
        out.extend(name);
        out.extend(os2);
        out
    }

    #[test]
    fn reads_the_family_windows_uses() {
        let bytes = font(&[(3, 1, 0x0409, 1, "Nunito SemiBold"), (3, 1, 0x0409, 16, "Nunito")], 600);
        assert_eq!(read(&bytes).unwrap(), FontInfo { family: "Nunito".into(), weight: 600 });
        let plain = font(&[(1, 0, 0, 1, "Orbitron")], 0);
        assert_eq!(read(&plain).unwrap(), FontInfo { family: "Orbitron".into(), weight: 400 });
        let english = font(&[(3, 1, 0x0407, 1, "Schrift"), (3, 1, 0x0409, 1, "Script")], 400);
        assert_eq!(read(&english).unwrap().family, "Script");
    }

    #[test]
    fn refuses_what_windows_cannot_load() {
        assert!(read(b"wOF2....").unwrap_err().to_string().contains("WOFF"));
        assert!(read(b"ttcf....").unwrap_err().to_string().contains(".ttc"));
        assert!(read(b"PK\x03\x04").is_err());
        assert!(read(&font(&[], 400)).unwrap_err().to_string().contains("no family"));
    }

    #[test]
    fn survives_damaged_tables() {
        let mut bytes = font(&[(3, 1, 0x0409, 1, "Nunito")], 400);
        // Point the name table past the end of the file.
        bytes[12 + 8..12 + 12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(read(&bytes).is_err());
        for cut in 0..bytes.len() {
            let _ = read(&bytes[..cut]);
        }
    }
}
