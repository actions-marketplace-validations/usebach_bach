//! Collecting the files under `[files] source` (or `[macos] source`).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::archive::Source;
use crate::manifest::Platform;
use crate::paths;

/// What a macOS source folder holds besides plain files: permission bits and
/// symbolic links, which a Windows folder never has.
#[derive(Debug, Default)]
pub struct Gathered {
    pub sources: Vec<Source>,
    /// The permission bits of files that have any execute bit, by packaged path.
    pub modes: BTreeMap<String, u32>,
    /// Symbolic links: packaged path and the relative target.
    pub links: Vec<(String, String)>,
}

/// Every file under `dir`, sorted, minus the excluded ones and the output.
/// A Windows folder may hold files only. A macOS folder may hold symbolic links (kept as
/// links, never followed) and its files' permission bits are read; `.DS_Store`
/// files Finder leaves behind are left out.
pub fn gather_for(dir: &Path, exclude: &[String], output: &Path, platform: Platform) -> Result<Gathered> {
    let output = std::fs::canonicalize(output).ok();
    let mut found = BTreeMap::new();
    let mut gathered = Gathered::default();
    let mut names = BTreeMap::new();
    walk(dir, "", platform, &mut |entry| {
        let packaged = entry.packaged;
        if platform.is_macos() && packaged.rsplit('/').next() == Some(".DS_Store") {
            return Ok(());
        }
        if exclude.iter().any(|pattern| paths::matches(pattern, packaged)) {
            return Ok(());
        }
        let key = paths::fold_case(packaged);
        if names.insert(key.clone(), ()).is_some() {
            bail!("{} is in the source folder twice, in different letter case", packaged);
        }
        match entry.kind {
            Kind::File { size, mode } => {
                if output.is_some() && std::fs::canonicalize(entry.path).ok() == output {
                    return Ok(());
                }
                if let Some(mode) = mode.filter(|mode| mode & 0o111 != 0) {
                    gathered.modes.insert(packaged.to_string(), mode & 0o777);
                }
                found.insert(key, Source { packaged: packaged.to_string(), path: entry.path.to_path_buf(), size });
            }
            Kind::Link { target } => {
                paths::check_link(packaged, &target)
                    .with_context(|| format!("{} is a symbolic link Bach can't package", entry.path.display()))?;
                gathered.links.push((packaged.to_string(), target));
            }
        }
        Ok(())
    })?;
    gathered.sources = found.into_values().collect();
    gathered.links.sort();
    Ok(gathered)
}

enum Kind {
    File { size: u64, mode: Option<u32> },
    Link { target: String },
}

struct Entry<'a> {
    packaged: &'a str,
    path: &'a Path,
    kind: Kind,
}

fn walk(dir: &Path, prefix: &str, platform: Platform, visit: &mut dyn FnMut(Entry) -> Result<()>) -> Result<()> {
    let entries = std::fs::read_dir(dir).with_context(|| format!("couldn't list {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            bail!("{} has a name that isn't valid text", entry.path().display());
        };
        let packaged = if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") };
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            if platform.is_windows() {
                bail!("{} is a symbolic link; Bach packages real files only", path.display());
            }
            let target = std::fs::read_link(&path)?;
            let Some(target) = target.to_str() else {
                bail!("{} is a link to a name that isn't valid text", path.display());
            };
            paths::normalize(&packaged).with_context(|| format!("can't package {}", path.display()))?;
            visit(Entry { packaged: &packaged, path: &path, kind: Kind::Link { target: target.to_string() } })?;
        } else if meta.is_dir() {
            walk(&path, &packaged, platform, visit)?;
        } else if meta.is_file() {
            paths::normalize(&packaged).with_context(|| format!("can't package {}", path.display()))?;
            visit(Entry { packaged: &packaged, path: &path, kind: Kind::File { size: meta.len(), mode: mode_of(&meta) } })?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn mode_of(meta: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode())
}

#[cfg(not(unix))]
fn mode_of(_: &std::fs::Metadata) -> Option<u32> {
    None
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn tree(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("bach-gather-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Hello.app/Contents/MacOS")).unwrap();
        std::fs::write(dir.join("Hello.app/Contents/MacOS/hello"), b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(dir.join("Hello.app/Contents/MacOS/hello"), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.join("Hello.app/Contents/Info.plist"), b"<plist/>").unwrap();
        std::fs::write(dir.join(".DS_Store"), b"x").unwrap();
        dir
    }

    #[test]
    fn a_mac_folder_keeps_modes_and_links_and_drops_ds_store() {
        let dir = tree("mac");
        symlink("MacOS/hello", dir.join("Hello.app/Contents/current")).unwrap();
        let out = dir.join("out.dmg");
        let gathered = gather_for(&dir, &[], &out, Platform::Macos).unwrap();
        let names: Vec<&str> = gathered.sources.iter().map(|s| s.packaged.as_str()).collect();
        assert_eq!(names, ["Hello.app/Contents/Info.plist", "Hello.app/Contents/MacOS/hello"]);
        assert_eq!(gathered.modes.get("Hello.app/Contents/MacOS/hello"), Some(&0o755));
        assert!(!gathered.modes.contains_key("Hello.app/Contents/Info.plist"));
        assert_eq!(gathered.links, [("Hello.app/Contents/current".to_string(), "MacOS/hello".to_string())]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_link_out_of_the_folder_is_refused() {
        let dir = tree("escape");
        symlink("../../../../etc/passwd", dir.join("Hello.app/Contents/evil")).unwrap();
        let err = gather_for(&dir, &[], &dir.join("o.dmg"), Platform::Macos).unwrap_err();
        assert!(format!("{err:#}").contains("climbs out"), "{err:#}");
        symlink("/etc/passwd", dir.join("abs")).unwrap();
        std::fs::remove_file(dir.join("Hello.app/Contents/evil")).unwrap();
        assert!(gather_for(&dir, &[], &dir.join("o.dmg"), Platform::Macos).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn windows_folders_still_refuse_links() {
        let dir = tree("win");
        symlink("MacOS/hello", dir.join("Hello.app/Contents/current")).unwrap();
        let err = gather_for(&dir, &[], &dir.join("o.exe"), Platform::Windows).unwrap_err();
        assert!(err.to_string().contains("symbolic link"), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
