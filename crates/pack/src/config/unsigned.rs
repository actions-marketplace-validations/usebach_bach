//! A signed setup file that unpacks unsigned programs.
//!
//! Signing the setup file covers the setup file only. The programs and
//! libraries inside it keep whatever signature they had when they were
//! packed, and Windows 11's Smart App Control looks at every program that
//! starts, not just the one that was downloaded. So a developer who signs
//! the setup file (`[signing]`) but packs unsigned programs gets an installer
//! that looks trustworthy and an app that can still be blocked.

use crate::archive::Source;
use crate::pe::{self, Signature};

/// How many unsigned files the warning names.
const NAMED: usize = 3;

/// A warning when some of the programs and libraries being packed aren't
/// signed. Files that aren't Windows programs are ignored: a PNG is not
/// "unsigned".
pub(super) fn warning(sources: &[Source]) -> Option<String> {
    let mut total = 0;
    let mut unsigned = Vec::new();
    for source in sources {
        let name = source.packaged.to_ascii_lowercase();
        if !(name.ends_with(".exe") || name.ends_with(".dll")) {
            continue;
        }
        match pe::signature(&source.path) {
            Some(Signature::Signed) => total += 1,
            Some(Signature::Unsigned) => {
                total += 1;
                unsigned.push(source.packaged.as_str());
            }
            None => {}
        }
    }
    if unsigned.is_empty() {
        return None;
    }
    // Programs first: they are what starts, and what Windows looks at first.
    unsigned.sort_by_key(|name| (!name.to_ascii_lowercase().ends_with(".exe"), name.to_ascii_lowercase()));
    let mut names = unsigned.iter().take(NAMED).copied().collect::<Vec<_>>().join(", ");
    if unsigned.len() > NAMED {
        names.push_str(&format!(" and {} more", unsigned.len() - NAMED));
    }
    Some(format!(
        "{} of the {total} programs and libraries in the setup file aren't signed ({names}). Signing the setup file doesn't sign them, \
         and Windows 11's Smart App Control can still block an unsigned program once it is installed: sign them in your build, \
         before Bach packs them (see docs/signing.md)",
        unsigned.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A PE32+ file with or without a certificate entry (4 KB, block at the end).
    fn program(signed: bool) -> Vec<u8> {
        let mut bytes = vec![0u8; 4096];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        let optional = 0x80 + 24;
        bytes[optional..optional + 2].copy_from_slice(&0x20bu16.to_le_bytes());
        bytes[optional + 108..optional + 112].copy_from_slice(&16u32.to_le_bytes());
        if signed {
            let entry = optional + 112 + 4 * 8;
            bytes[entry..entry + 4].copy_from_slice(&3072u32.to_le_bytes());
            bytes[entry + 4..entry + 8].copy_from_slice(&1024u32.to_le_bytes());
        }
        bytes
    }

    fn sources(name: &str, files: &[(&str, Vec<u8>)]) -> (PathBuf, Vec<Source>) {
        let dir = std::env::temp_dir().join(format!("bach-unsigned-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sources = files
            .iter()
            .enumerate()
            .map(|(index, (packaged, bytes))| {
                let path = dir.join(format!("file{index}"));
                std::fs::write(&path, bytes).unwrap();
                Source { packaged: packaged.to_string(), path, size: bytes.len() as u64 }
            })
            .collect();
        (dir, sources)
    }

    #[test]
    fn unsigned_programs_are_named_programs_first() {
        let (dir, found) = sources(
            "named",
            &[
                ("lib/b.dll", program(false)),
                ("App.exe", program(false)),
                ("signed.dll", program(true)),
                ("a.dll", program(false)),
                ("data.png", vec![1, 2, 3]),
                ("fake.exe", b"MZ".to_vec()),
                ("z.dll", program(false)),
            ],
        );
        let text = warning(&found).expect("a warning");
        assert!(text.starts_with("4 of the 5 programs and libraries"), "{text}");
        assert!(text.contains("(App.exe, a.dll, lib/b.dll and 1 more)"), "{text}");
        assert!(text.contains("docs/signing.md"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nothing_to_say_when_everything_is_signed_or_not_a_program() {
        let (dir, found) = sources("fine", &[("App.exe", program(true)), ("lib.dll", program(true)), ("notes.txt", b"hi".to_vec())]);
        assert_eq!(warning(&found), None);
        std::fs::remove_dir_all(&dir).unwrap();
        // Only files that aren't PE files (the two-byte placeholders of tests and examples).
        let (dir, found) = sources("fake", &[("App.exe", b"MZ".to_vec())]);
        assert_eq!(warning(&found), None);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(warning(&[]), None);
    }
}
