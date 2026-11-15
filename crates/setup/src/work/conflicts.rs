//! Files that belong to someone else, in the folder an install goes to.
//!
//! A first install into a folder that already holds files of the same names
//! would replace them, and the uninstaller would later delete them because
//! the manifest lists them. Both the window and a silent run look here first
//! so the user decides before anything is lost.

use std::path::Path;

use bach_pack::manifest::{Choices, FileEntry, Manifest};
use bach_pack::paths;

use super::install::previous_install;
use super::patch::matches_on_disk;

/// How many names are checked and shown at most; a folder with more is
/// plainly someone's, and the count says so.
const LIMIT: usize = 200;
/// How many names the sentence spells out.
const SHOWN: usize = 5;

/// The packaged paths this install would replace although they aren't from an
/// earlier install of this app: empty for an update, for a folder without a
/// clashing file, and for a file already identical to the packaged one.
pub fn foreign_files(manifest: &Manifest, choices: &Choices, dir: &Path) -> Vec<String> {
    if !dir.is_dir() || previous_install(dir, manifest).is_some() {
        return Vec::new();
    }
    let choices = choices.clone().settle(manifest);
    manifest
        .files
        .iter()
        .filter(|file| choices.installs(file))
        .filter(|file| {
            let path = paths::under(dir, &file.path);
            path.is_file() && !matches_on_disk(&path, file)
        })
        .map(|file| file.path.clone())
        .take(LIMIT)
        .collect()
}

/// Whether the user (or the app) changed `path` since setup wrote it: the file
/// is there and its SHA-256 no longer matches the manifest. Removing keeps
/// such a file, so the uninstaller never deletes work it didn't put there. A
/// file the manifest has no hash for counts as unchanged, and so does one that
/// can't be read right now (a locked file isn't an edited one: removing it
/// then fails loudly instead of quietly keeping it).
pub fn changed_since_install(path: &Path, file: &FileEntry) -> bool {
    file.sha256.is_some() && path.is_file() && std::fs::File::open(path).is_ok() && !matches_on_disk(path, file)
}

/// A short list for a sentence: the first few names, then how many more.
pub fn describe(files: &[String]) -> String {
    let shown = files.iter().take(SHOWN).cloned().collect::<Vec<_>>().join(", ");
    match files.len().saturating_sub(SHOWN) {
        0 => shown,
        more => format!("{shown} (+{more})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(content: &[u8]) -> FileEntry {
        let mut entry = FileEntry::new("a.txt", content.len() as u64);
        entry.sha256 = Some(bach_pack::protect::sha256_hex(content).unwrap());
        entry
    }

    #[test]
    fn a_file_the_user_changed_is_kept_on_removal() {
        let dir = std::env::temp_dir().join(format!("bach-conflicts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.txt");
        let file = entry(b"as installed");

        assert!(!changed_since_install(&path, &file), "a missing file has nothing to keep");
        std::fs::write(&path, b"as installed").unwrap();
        assert!(!changed_since_install(&path, &file));
        std::fs::write(&path, b"as installed, plus my notes").unwrap();
        assert!(changed_since_install(&path, &file));
        std::fs::write(&path, b"as Installed").unwrap();
        assert!(changed_since_install(&path, &file), "same size, other content");

        // Held open without sharing, it can't be read: not "changed".
        {
            use std::os::windows::fs::OpenOptionsExt;
            let _held = std::fs::OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
            assert!(!changed_since_install(&path, &file));
        }

        let mut unrecorded = file.clone();
        unrecorded.sha256 = None;
        assert!(!changed_since_install(&path, &unrecorded), "no hash recorded: nothing to compare");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn long_lists_are_cut_with_a_count() {
        let names: Vec<String> = (1..=8).map(|n| format!("f{n}.txt")).collect();
        assert_eq!(describe(&names[..2]), "f1.txt, f2.txt");
        assert_eq!(describe(&names), "f1.txt, f2.txt, f3.txt, f4.txt, f5.txt (+3)");
        assert_eq!(describe(&[]), "");
    }
}
