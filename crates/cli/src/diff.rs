//! `bach diff OLD NEW`: what changed between two setup files of an app. The
//! files that were added, removed or changed, what the new one writes to the
//! registry and the environment that the old one didn't, and which files made
//! the installer bigger.
//!
//! Both sides are read from the manifests inside the setup files; nothing is
//! unpacked. A file counts as changed when its SHA-256 differs (setup files
//! built before checksums were recorded compare by size instead, and the
//! output says so).

use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;

use anyhow::{bail, Context, Result};
use bach_pack::container;
use bach_pack::manifest::{Manifest, RegistryData};
use serde_json::{json, Value};

use crate::output::{self, field};

pub struct Options<'a> {
    pub old: &'a Path,
    pub new: &'a Path,
    /// How many of the biggest changes to list (the files that grew most).
    pub top: usize,
}

/// A setup file read back: its manifest and how big it is.
struct Side {
    manifest: Manifest,
    /// The setup file on disk.
    file_size: u64,
    /// The compressed archive inside it; 0 for an online installer.
    packed: u64,
}

fn read(path: &Path) -> Result<Side> {
    let mut file = File::open(path).with_context(|| format!("couldn't open {}", path.display()))?;
    let Some(layout) = container::locate(&mut file)? else {
        bail!("{} isn't a Bach setup file", path.display());
    };
    let manifest = container::read_manifest(&mut file, &layout)?;
    let file_size = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    Ok(Side { manifest, file_size, packed: if layout.has_archive() { layout.archive.len } else { 0 } })
}

/// How a file differs between the two versions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileChange {
    Added { size: u64 },
    Removed { size: u64 },
    Changed { old: u64, new: u64 },
}

impl FileChange {
    /// By how many bytes the installed app grows (negative when it shrinks).
    pub fn growth(&self) -> i128 {
        match self {
            FileChange::Added { size } => i128::from(*size),
            FileChange::Removed { size } => -i128::from(*size),
            FileChange::Changed { old, new } => i128::from(*new) - i128::from(*old),
        }
    }
}

/// What a comparison of two manifests found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Diff {
    /// Changed files by their path, in path order.
    pub files: BTreeMap<String, FileChange>,
    pub unchanged: usize,
    /// Some file couldn't be compared by checksum, so equal sizes counted as equal.
    pub by_size_only: bool,
    /// Lines like `registry HKCU\Software\X\Y`, added or removed or changed.
    pub system: Vec<(Change, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Added,
    Removed,
    Changed,
}

impl Change {
    fn mark(self) -> char {
        match self {
            Change::Added => '+',
            Change::Removed => '-',
            Change::Changed => '~',
        }
    }
    fn word(self) -> &'static str {
        match self {
            Change::Added => "added",
            Change::Removed => "removed",
            Change::Changed => "changed",
        }
    }
}

/// Compares two manifests. Paths are matched the way Windows matches them, so
/// `Data.bin` and `data.bin` are one file.
pub fn compare(old: &Manifest, new: &Manifest) -> Diff {
    let mut diff = Diff::default();
    let old_files: BTreeMap<String, &bach_pack::manifest::FileEntry> =
        old.files.iter().map(|file| (file.path.to_ascii_lowercase(), file)).collect();
    let mut seen = std::collections::BTreeSet::new();
    for file in &new.files {
        let key = file.path.to_ascii_lowercase();
        seen.insert(key.clone());
        match old_files.get(&key) {
            None => {
                diff.files.insert(file.path.clone(), FileChange::Added { size: file.size });
            }
            Some(before) => {
                let same = match (&before.sha256, &file.sha256) {
                    (Some(a), Some(b)) => a.eq_ignore_ascii_case(b) && before.size == file.size,
                    _ => {
                        diff.by_size_only = true;
                        before.size == file.size
                    }
                };
                if same {
                    diff.unchanged += 1;
                } else {
                    diff.files.insert(file.path.clone(), FileChange::Changed { old: before.size, new: file.size });
                }
            }
        }
    }
    for file in &old.files {
        if !seen.contains(&file.path.to_ascii_lowercase()) {
            diff.files.insert(file.path.clone(), FileChange::Removed { size: file.size });
        }
    }
    diff.system = system_changes(old, new);
    diff
}

/// One thing the install does outside its files, as `(what it is, how it reads)`.
/// The first part identifies the thing; the second is what a change is about.
type Item = (String, String);

fn registry_items(manifest: &Manifest) -> Vec<Item> {
    manifest
        .system
        .registry
        .iter()
        .map(|value| {
            let name = if value.name.is_empty() { "(default)" } else { value.name.as_str() };
            let data = match &value.data {
                RegistryData::String(text) | RegistryData::Expand(text) => format!("\"{text}\""),
                RegistryData::Dword(number) => number.to_string(),
            };
            (format!("registry {}\\{name}", value.key), data)
        })
        .collect()
}

fn items(manifest: &Manifest) -> Vec<Item> {
    let mut items = registry_items(manifest);
    items
        .extend(manifest.system.environment.iter().map(|var| (format!("environment variable {}", var.name), format!("\"{}\"", var.value))));
    items.extend(manifest.system.context_menu.iter().map(|entry| {
        let on: Vec<String> = entry.on.iter().map(|target| String::from(target.clone())).collect();
        (
            format!("right-click entry \"{}\"", entry.name),
            format!("on {} {}", on.join(", "), entry.arguments.as_deref().unwrap_or("")).trim().to_string(),
        )
    }));
    items.extend(manifest.system.migrate.iter().map(|migrate| (format!("migrates {}", migrate.uninstall_key), String::new())));
    items.extend(manifest.protocols.iter().map(|protocol| (format!("link scheme {}://", protocol.scheme), String::new())));
    items.extend(
        manifest.file_types.iter().map(|kind| (format!("file type {}", kind.extension), kind.description.clone().unwrap_or_default())),
    );
    items.extend(manifest.shortcuts.iter().map(|shortcut| {
        (
            format!("Start menu shortcut \"{}\"", shortcut.name),
            format!("{} {}", shortcut.target, shortcut.arguments.as_deref().unwrap_or("")).trim().to_string(),
        )
    }));
    items.extend(manifest.run.iter().map(|step| {
        (
            format!("program run at {:?} \"{}\"", step.when, step.name),
            format!("{} {}", step.file, step.arguments.as_deref().unwrap_or("")).trim().to_string(),
        )
    }));
    items
        .extend(manifest.requirements.iter().map(|requirement| (format!("requirement \"{}\"", requirement.name), requirement.url.clone())));
    items.extend(
        manifest
            .components
            .iter()
            .map(|component| (format!("component {}", component.id), format!("ticked by default: {}", component.default))),
    );
    items
}

fn system_changes(old: &Manifest, new: &Manifest) -> Vec<(Change, String)> {
    let before: BTreeMap<String, String> = items(old).into_iter().collect();
    let after: BTreeMap<String, String> = items(new).into_iter().collect();
    let mut changes = Vec::new();
    for (what, how) in &after {
        match before.get(what) {
            None => changes.push((Change::Added, line(what, how))),
            Some(was) if was != how => changes.push((Change::Changed, format!("{what}: {was} → {how}"))),
            Some(_) => {}
        }
    }
    for (what, how) in &before {
        if !after.contains_key(what) {
            changes.push((Change::Removed, line(what, how)));
        }
    }
    changes
}

fn line(what: &str, how: &str) -> String {
    if how.is_empty() {
        what.to_string()
    } else {
        format!("{what} = {how}")
    }
}

fn signed(bytes: i128) -> String {
    let magnitude = output::size(bytes.unsigned_abs().min(u128::from(u64::MAX)) as u64);
    if bytes < 0 {
        format!("-{magnitude}")
    } else {
        format!("+{magnitude}")
    }
}

pub fn command(options: Options) -> Result<()> {
    let old = read(options.old)?;
    let new = read(options.new)?;
    if old.manifest.app.id != new.manifest.app.id {
        bail!(
            "{} is a setup file for \"{}\" and {} for \"{}\"; bach diff compares two versions of one app",
            options.old.display(),
            old.manifest.app.id,
            options.new.display(),
            new.manifest.app.id
        );
    }
    let diff = compare(&old.manifest, &new.manifest);
    let (old_app, new_app) = (&old.manifest.app, &new.manifest.app);
    say!("{} {} → {}", new_app.name, old_app.version, new_app.version);
    field(
        "Files",
        format!(
            "{} added, {} removed, {} changed, {} unchanged",
            count(&diff, "added"),
            count(&diff, "removed"),
            count(&diff, "changed"),
            diff.unchanged
        ),
    );
    let installed_growth = i128::from(new.manifest.total_size()) - i128::from(old.manifest.total_size());
    field(
        "Installed",
        format!("{} → {} ({})", output::size(old.manifest.total_size()), output::size(new.manifest.total_size()), signed(installed_growth)),
    );
    let (before, after) = if old.packed > 0 && new.packed > 0 { (old.packed, new.packed) } else { (old.file_size, new.file_size) };
    let what = if old.packed > 0 && new.packed > 0 { "Packed" } else { "Setup file" };
    field(what, format!("{} → {} ({})", output::size(before), output::size(after), signed(i128::from(after) - i128::from(before))));
    if diff.by_size_only {
        say!(
            "  note: a setup file without recorded checksums was compared by size, so a file that changed but kept its size isn't listed."
        );
    }
    if !diff.files.is_empty() {
        say!();
        say!("Files:");
        for (path, change) in &diff.files {
            let text = match change {
                FileChange::Added { size } => format!("+ {path}  ({})", output::size(*size)),
                FileChange::Removed { size } => format!("- {path}  ({})", output::size(*size)),
                FileChange::Changed { old, new } => {
                    format!("~ {path}  ({} → {}, {})", output::size(*old), output::size(*new), signed(i128::from(*new) - i128::from(*old)))
                }
            };
            say!("  {text}");
        }
    }
    let grew = grew_most(&diff, options.top);
    if !grew.is_empty() {
        say!();
        say!("Made the app bigger:");
        for (path, growth) in &grew {
            say!("  {:>12}  {path}", signed(*growth));
        }
    }
    if !diff.system.is_empty() {
        say!();
        say!("Besides files:");
        for (change, text) in &diff.system {
            say!("  {} {text}", change.mark());
        }
    }
    if diff.files.is_empty() && diff.system.is_empty() {
        say!();
        say!("Nothing differs: both setup files install the same files and make the same changes.");
    }
    output::result("name", new_app.name.clone());
    output::result("old_version", old_app.version.clone());
    output::result("new_version", new_app.version.clone());
    output::result("unchanged", diff.unchanged);
    output::result("installed_growth", i64::try_from(installed_growth).unwrap_or(i64::MAX));
    output::result("setup_size_old", old.file_size);
    output::result("setup_size_new", new.file_size);
    let files: Vec<Value> = diff
        .files
        .iter()
        .map(|(path, change)| {
            let (kind, old_size, new_size) = match change {
                FileChange::Added { size } => ("added", None, Some(*size)),
                FileChange::Removed { size } => ("removed", Some(*size), None),
                FileChange::Changed { old, new } => ("changed", Some(*old), Some(*new)),
            };
            json!({"path": path, "change": kind, "old_size": old_size, "new_size": new_size})
        })
        .collect();
    output::result("changed_files", files);
    let system: Vec<Value> = diff.system.iter().map(|(change, text)| json!({"change": change.word(), "what": text})).collect();
    output::result("changes", system);
    Ok(())
}

fn count(diff: &Diff, kind: &str) -> usize {
    diff.files
        .values()
        .filter(|change| {
            matches!(
                (change, kind),
                (FileChange::Added { .. }, "added") | (FileChange::Removed { .. }, "removed") | (FileChange::Changed { .. }, "changed")
            )
        })
        .count()
}

/// The files that grew the installed app the most, biggest first.
pub fn grew_most(diff: &Diff, top: usize) -> Vec<(String, i128)> {
    let mut grown: Vec<(String, i128)> =
        diff.files.iter().map(|(path, change)| (path.clone(), change.growth())).filter(|(_, growth)| *growth > 0).collect();
    grown.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    grown.truncate(top);
    grown
}

#[cfg(test)]
mod tests {
    use super::*;
    use bach_pack::manifest::{EnvVar, FileEntry, RegistryValue, RegistryView};

    fn manifest(version: &str, files: &[(&str, u64, &str)]) -> Manifest {
        let mut manifest: Manifest = serde_json::from_str(
            r#"{"format":1,"builder":"0","app":{"id":"A","name":"A","version":"1","publisher":"P","description":null,"website":null,"executable":"a.exe"},
                "install":{"folder":"A","desktop_shortcut":false},"uninstall":{"data":[],"data_description":null},
                "look":{"theme":"system","accent":null,"logo":null},"files":[]}"#,
        )
        .unwrap();
        manifest.app.version = version.into();
        manifest.files =
            files.iter().map(|(path, size, sha)| FileEntry { sha256: Some((*sha).into()), ..FileEntry::new(*path, *size) }).collect();
        manifest
    }

    #[test]
    fn it_finds_added_removed_and_changed_files() {
        let old = manifest("1", &[("a.exe", 10, "aa"), ("old.dll", 5, "bb"), ("same.txt", 3, "cc"), ("Data.bin", 100, "dd")]);
        let new = manifest("2", &[("a.exe", 30, "a2"), ("new.dll", 7, "ee"), ("same.txt", 3, "cc"), ("data.bin", 100, "dd")]);
        let diff = compare(&old, &new);
        assert_eq!(diff.files["a.exe"], FileChange::Changed { old: 10, new: 30 });
        assert_eq!(diff.files["new.dll"], FileChange::Added { size: 7 });
        assert_eq!(diff.files["old.dll"], FileChange::Removed { size: 5 });
        assert_eq!(diff.unchanged, 2, "same.txt, and data.bin whose case differs but nothing else");
        assert_eq!(diff.files.len(), 3);
        assert!(!diff.by_size_only);
    }

    #[test]
    fn a_changed_file_of_the_same_size_is_found_by_its_checksum() {
        let old = manifest("1", &[("a.exe", 10, "aa")]);
        let new = manifest("2", &[("a.exe", 10, "bb")]);
        assert_eq!(compare(&old, &new).files["a.exe"], FileChange::Changed { old: 10, new: 10 });
    }

    #[test]
    fn without_checksums_it_compares_sizes_and_says_so() {
        let mut old = manifest("1", &[("a.exe", 10, "aa")]);
        old.files[0].sha256 = None;
        let new = manifest("2", &[("a.exe", 10, "bb")]);
        let diff = compare(&old, &new);
        assert!(diff.files.is_empty() && diff.by_size_only);
    }

    #[test]
    fn the_biggest_growth_comes_first_and_shrinking_files_are_left_out() {
        let old = manifest("1", &[("a.exe", 10, "aa"), ("b.dll", 100, "bb"), ("c.txt", 50, "cc")]);
        let new = manifest("2", &[("a.exe", 1010, "a2"), ("b.dll", 60, "b2"), ("c.txt", 50, "cc"), ("d.bin", 400, "dd")]);
        let grew = grew_most(&compare(&old, &new), 5);
        assert_eq!(grew, vec![("a.exe".to_string(), 1000), ("d.bin".to_string(), 400)]);
        assert_eq!(grew_most(&compare(&old, &new), 1).len(), 1);
    }

    #[test]
    fn new_registry_values_and_variables_are_listed() {
        let old = manifest("1", &[]);
        let mut new = manifest("2", &[]);
        new.system.registry.push(RegistryValue {
            key: r"Software\Vendor\App".into(),
            name: "Mode".into(),
            data: RegistryData::Dword(1),
            view: RegistryView::Native,
        });
        new.system.environment.push(EnvVar { name: "APP_HOME".into(), value: "{app}".into() });
        let diff = compare(&old, &new);
        assert_eq!(diff.system.len(), 2);
        assert!(diff.system.iter().all(|(change, _)| *change == Change::Added));
        assert!(diff.system.iter().any(|(_, text)| text == r"registry Software\Vendor\App\Mode = 1"));
        // Changing the value is a change, and dropping it is a removal.
        let mut newer = new.clone();
        newer.system.registry[0].data = RegistryData::Dword(2);
        newer.system.environment.clear();
        let diff = compare(&new, &newer);
        assert!(diff.system.contains(&(Change::Changed, r"registry Software\Vendor\App\Mode: 1 → 2".to_string())));
        assert!(diff.system.contains(&(Change::Removed, "environment variable APP_HOME = \"{app}\"".to_string())));
    }
}
