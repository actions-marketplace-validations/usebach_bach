//! Applying an update patch (see `bach build --patch-from`).
//!
//! A patch's archive holds only the files that changed since the version it
//! was made for. Every other file is already in the install folder, and setup
//! relies on it only after checking its size and SHA-256 against the manifest.

use std::collections::HashSet;
use std::fs::File;
use std::path::Path;

use bach_pack::manifest::{Choices, FileEntry, Manifest};
use bach_pack::paths;

/// For an update patch: the chosen files already in the install folder with
/// exactly the content the patch expects, which the patch leaves out. Empty
/// for a full setup file.
pub(super) fn patch_base(manifest: &Manifest, choices: &Choices, dir: &Path) -> HashSet<String> {
    if manifest.patch_from.is_none() {
        return HashSet::new();
    }
    manifest
        .files
        .iter()
        .filter(|file| choices.installs(file))
        .filter(|file| matches_on_disk(&paths::under(dir, &file.path), file))
        .map(|file| file.path.clone())
        .collect()
}

/// Whether the file at `path` is exactly what `file` records: same size and
/// SHA-256. A file that is missing, unreadable or unrecorded doesn't match.
fn matches_on_disk(path: &Path, file: &FileEntry) -> bool {
    let Some(expected) = &file.sha256 else {
        return false;
    };
    let Ok(open) = File::open(path) else {
        return false;
    };
    if open.metadata().map_or(true, |meta| meta.len() != file.size) {
        return false;
    }
    bach_pack::protect::sha256_hex(open).is_ok_and(|actual| actual.eq_ignore_ascii_case(expected))
}
