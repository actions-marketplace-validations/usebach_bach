//! The look section of a setup file: the logo, then the fonts and the
//! background picture a `[style]` ships. The container keeps carrying one
//! blob after the archive (so the payload format is unchanged); the
//! manifest lists the files at its end, and whatever comes before them is
//! the logo, exactly as before styles existed.

use serde::{Deserialize, Serialize};

use super::Look;

/// A file carried in the look section after the logo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookFile {
    pub kind: LookFileKind,
    pub len: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LookFileKind {
    /// A TrueType or OpenType font the window registers before it draws.
    Font,
    /// The window's background picture.
    BackgroundPng,
    BackgroundJpeg,
}

/// The look section split into its parts.
#[derive(Debug, Default, PartialEq)]
pub struct LookBytes {
    pub logo: Vec<u8>,
    pub fonts: Vec<Vec<u8>>,
    /// The background picture, and whether it's a JPEG (else a PNG).
    pub background: Option<(Vec<u8>, bool)>,
}

impl Look {
    /// Splits the look section as the manifest describes it. A section too
    /// short for the files it lists is refused rather than misread.
    pub fn split_section(&self, mut section: Vec<u8>) -> Result<LookBytes, String> {
        let listed = self.files.iter().try_fold(0u64, |sum, file| sum.checked_add(file.len));
        let Some(logo_len) = listed.and_then(|listed| (section.len() as u64).checked_sub(listed)) else {
            return Err("the look section is shorter than the files it lists".into());
        };
        // Both fit in the section, which is in memory, so they fit in usize.
        let mut rest = section.split_off(logo_len as usize);
        let mut parts = LookBytes { logo: section, ..LookBytes::default() };
        for file in &self.files {
            let tail = rest.split_off(file.len as usize);
            let bytes = std::mem::replace(&mut rest, tail);
            match file.kind {
                LookFileKind::Font => parts.fonts.push(bytes),
                LookFileKind::BackgroundPng => parts.background = Some((bytes, false)),
                LookFileKind::BackgroundJpeg => parts.background = Some((bytes, true)),
            }
        }
        Ok(parts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_section_without_files_is_all_logo() {
        let parts = Look::default().split_section(b"<svg/>".to_vec()).unwrap();
        assert_eq!(parts, LookBytes { logo: b"<svg/>".to_vec(), ..LookBytes::default() });
    }

    #[test]
    fn files_are_cut_from_the_end() {
        let look = Look {
            files: vec![
                LookFile { kind: LookFileKind::Font, len: 3 },
                LookFile { kind: LookFileKind::Font, len: 2 },
                LookFile { kind: LookFileKind::BackgroundJpeg, len: 1 },
            ],
            ..Look::default()
        };
        let parts = look.split_section(b"LOGOaaabbc".to_vec()).unwrap();
        assert_eq!(parts.logo, b"LOGO");
        assert_eq!(parts.fonts, [b"aaa".to_vec(), b"bb".to_vec()]);
        assert_eq!(parts.background, Some((b"c".to_vec(), true)));
        // No logo at all is fine too.
        assert!(look.split_section(b"aaabbc".to_vec()).unwrap().logo.is_empty());
    }

    #[test]
    fn a_short_or_overflowing_list_is_refused() {
        let short = Look { files: vec![LookFile { kind: LookFileKind::Font, len: 9 }], ..Look::default() };
        assert!(short.split_section(b"abc".to_vec()).is_err());
        let huge = Look {
            files: vec![LookFile { kind: LookFileKind::Font, len: u64::MAX }, LookFile { kind: LookFileKind::Font, len: 2 }],
            ..Look::default()
        };
        assert!(huge.split_section(b"abc".to_vec()).is_err());
    }
}
