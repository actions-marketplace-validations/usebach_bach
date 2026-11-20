//! Components whose files are downloaded when their box is ticked
//! (`download` in `[[components]]`).
//!
//! The setup file lists such a component's files (so the uninstaller and the
//! disk check know them) but doesn't carry them. The files are in an archive of
//! their own, in the same format as a setup file's, and the manifest holds that
//! archive's link, SHA-256 and size. Setup downloads it before anything is
//! written, so a failed or cancelled download leaves the computer as it was, and
//! refuses it unless it is exactly the file the setup file names: the link can
//! be any host.
//!
//! The archive is read like the setup file's own archive, through the same
//! checks (every file's size and SHA-256 against the manifest) and into the same
//! all-or-nothing transaction.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use bach_pack::i18n::Key;
use bach_pack::manifest::{Choices, Component, ComponentDownload, Manifest};

use super::cancel;
use super::InstallPlan;
use crate::texts::Texts;
use crate::updates::curl::{self, Fetch};

/// A downloaded, verified archive.
pub struct Pack {
    pub file: PathBuf,
}

/// The components these choices install that have to be downloaded: some file
/// of theirs isn't in the install folder already as the manifest records it.
pub fn needed<'a>(manifest: &'a Manifest, choices: &Choices, matching: &HashSet<String>) -> Vec<&'a Component> {
    manifest
        .components
        .iter()
        .filter(|component| component.download.is_some() && choices.components.contains(&component.id))
        .filter(|component| {
            manifest.files.iter().any(|file| file.component.as_deref() == Some(component.id.as_str()) && !matching.contains(&file.path))
        })
        .collect()
}

/// The component's title in the window's language.
fn title(texts: &Texts, manifest: &Manifest, component: &Component) -> String {
    texts.component(component, manifest).0
}

/// What the window says while a component downloads.
pub fn label(texts: &Texts, manifest: &Manifest, component: &Component) -> String {
    texts.with(Key::step_fetching, &[("name", &title(texts, manifest, component))])
}

/// Whether `label` is what the window says while some component downloads.
pub fn is_downloading(texts: &Texts, manifest: &Manifest, label: &str) -> bool {
    manifest
        .components
        .iter()
        .filter(|component| component.download.is_some())
        .any(|component| self::label(texts, manifest, component) == label)
}

fn folder(app_id: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{}-components", bach_pack::paths::safe_dir_name(app_id)))
}

/// Where a component's archive is kept while it downloads: named by its
/// hash, so another version's archive is never taken for this one's.
fn target(app_id: &str, component: &Component, download: &ComponentDownload) -> PathBuf {
    let short: String = download.sha256.chars().take(16).collect();
    folder(app_id).join(format!("{}-{}.bachpack", bach_pack::paths::safe_dir_name(&component.id), short.to_ascii_lowercase()))
}

/// Whether the file is exactly the archive the manifest names.
fn is_the_one(file: &Path, download: &ComponentDownload) -> bool {
    let Ok(open) = fs::File::open(file) else {
        return false;
    };
    if open.metadata().map_or(true, |meta| meta.len() != download.size) {
        return false;
    }
    bach_pack::protect::sha256_hex(std::io::BufReader::new(open)).is_ok_and(|actual| actual.eq_ignore_ascii_case(&download.sha256))
}

/// Downloads each component's archive (an earlier, interrupted download picks up
/// where it stopped) and checks it. `progress` is told how many bytes of all of
/// them are in, and the label to show.
pub fn fetch(plan: &InstallPlan, components: &[&Component], progress: &mut dyn FnMut(u64, &str)) -> Result<Vec<Pack>, String> {
    let manifest = &plan.package.manifest;
    let texts = &plan.texts;
    let mut packs = Vec::new();
    let mut before = 0u64;
    for component in components {
        let Some(download) = &component.download else { continue };
        let name = title(texts, manifest, component);
        let label = label(texts, manifest, component);
        let failed = |why: String| texts.with(Key::error_fetch, &[("name", &name), ("error", &sentence(&why))]);
        let file = target(&manifest.app.id, component, download);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).map_err(|err| failed(err.to_string()))?;
        }
        progress(before, &label);
        if !is_the_one(&file, download) {
            let partial = file.with_extension("bachpack.partial");
            // A partial file bigger than the archive is somebody else's; start over.
            if fs::metadata(&partial).is_ok_and(|meta| meta.len() > download.size) {
                let _ = fs::remove_file(&partial);
            }
            crate::log::line(&format!("Downloading {} ({}) from {}", component.id, texts_size(download.size), download.url));
            let result = curl::download_with(&download.url, &partial, Fetch::SETUP, None, &mut |bytes| {
                progress(before.saturating_add(bytes), &label)
            });
            if let Err(why) = result {
                if cancel::requested() {
                    return Err(cancel::CANCELLED.to_string());
                }
                crate::log::line(&format!("The download of {} failed: {why}", component.id));
                return Err(failed(why));
            }
            if !is_the_one(&partial, download) {
                let _ = fs::remove_file(&partial);
                crate::log::line(&format!("The download of {} isn't the file the setup file names (size or SHA-256).", component.id));
                return Err(failed("what came back isn't the file this setup file expects".into()));
            }
            let _ = fs::remove_file(&file);
            fs::rename(&partial, &file).map_err(|err| failed(err.to_string()))?;
        }
        before = before.saturating_add(download.size);
        progress(before, &label);
        packs.push(Pack { file });
    }
    Ok(packs)
}

/// The downloaded archives aren't needed once the install is done.
pub fn forget(app_id: &str) {
    let _ = fs::remove_dir_all(folder(app_id));
}

fn texts_size(bytes: u64) -> String {
    crate::texts::size(bytes)
}

/// A reason as a sentence.
fn sentence(why: &str) -> String {
    let why = why.trim();
    match why.chars().last() {
        Some('.' | '!' | '?') | None => why.to_string(),
        _ => format!("{why}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bach_pack::manifest::FileEntry;

    fn manifest() -> Manifest {
        let mut manifest: Manifest = serde_json::from_str(
            r#"{"format":1,"builder":"0","app":{"id":"A","name":"A","version":"1.0.0","publisher":"P","description":null,"website":null,"executable":"a.exe"},
                "install":{"folder":"A","desktop_shortcut":false},"uninstall":{"data":[],"data_description":null},
                "look":{"theme":"system","accent":null,"logo":null},
                "files":[{"path":"a.exe","size":1},{"path":"hd/one.bin","size":5,"component":"hd"},{"path":"hd/two.bin","size":6,"component":"hd"},{"path":"s/x.txt","size":2,"component":"s"}],
                "components":[
                  {"id":"hd","title":{"*":"HD"},"default":false,"download":{"url":"https://x.example/hd.bachpack","sha256":"ab","size":9}},
                  {"id":"s","title":{"*":"Samples"},"default":false}]}"#,
        )
        .unwrap();
        manifest.components[0].download.as_mut().unwrap().sha256 = "ab".repeat(32);
        manifest
    }

    fn choices(ids: &[&str]) -> Choices {
        Choices { components: ids.iter().map(|id| id.to_string()).collect(), ..Choices::default() }
    }

    #[test]
    fn only_ticked_downloaded_components_with_something_to_fetch_are_needed() {
        let manifest = manifest();
        let none = HashSet::new();
        assert!(needed(&manifest, &choices(&[]), &none).is_empty(), "unticked: never downloaded");
        assert!(needed(&manifest, &choices(&["s"]), &none).is_empty(), "a component that is packed in has nothing to download");
        let ids: Vec<&str> = needed(&manifest, &choices(&["hd", "s"]), &none).iter().map(|component| component.id.as_str()).collect();
        assert_eq!(ids, ["hd"]);
        // An update that finds every file already installed as recorded downloads nothing.
        let here: HashSet<String> = ["hd/one.bin".to_string(), "hd/two.bin".to_string()].into();
        assert!(needed(&manifest, &choices(&["hd"]), &here).is_empty());
        let one_missing: HashSet<String> = ["hd/one.bin".to_string()].into();
        assert_eq!(needed(&manifest, &choices(&["hd"]), &one_missing).len(), 1, "one changed file is enough");
    }

    #[test]
    fn a_file_is_taken_only_if_it_is_exactly_the_archive_the_manifest_names() {
        let dir = std::env::temp_dir().join(format!("bach-component-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("pack.bachpack");
        fs::write(&file, b"abc").unwrap();
        let sha = bach_pack::protect::sha256_hex(&b"abc"[..]).unwrap();
        let good = ComponentDownload { url: "https://x.example/p".into(), sha256: sha.clone(), size: 3 };
        assert!(is_the_one(&file, &good));
        assert!(is_the_one(&file, &ComponentDownload { sha256: sha.to_uppercase(), ..good.clone() }));
        assert!(!is_the_one(&file, &ComponentDownload { size: 4, ..good.clone() }), "a different size");
        assert!(!is_the_one(&file, &ComponentDownload { sha256: "00".repeat(32), ..good.clone() }), "a different file");
        assert!(!is_the_one(&dir.join("missing"), &good));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_archive_is_named_after_its_hash_so_versions_never_share_one() {
        let manifest = manifest();
        let component = &manifest.components[0];
        let download = component.download.as_ref().unwrap();
        let first = target("A", component, download);
        let other = ComponentDownload { sha256: "cd".repeat(32), ..download.clone() };
        assert_ne!(first, target("A", component, &other));
        assert!(first.file_name().unwrap().to_string_lossy().starts_with("hd-abababababababab"));
        let _ = FileEntry::new("x", 1);
    }

    #[test]
    fn reasons_end_like_sentences() {
        assert_eq!(sentence("timed out"), "timed out.");
        assert_eq!(sentence("done."), "done.");
        assert_eq!(sentence(""), "");
    }
}
