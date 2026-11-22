//! Listing the macOS setup file in the update feed.
//!
//! A macOS Setup app is offered under `arch.macos`, next to `arm64`: one feed
//! serves every platform, and each signs its own entry. A project that only
//! builds for macOS gets a feed whose own link is the macOS file too, so the
//! entry's version, link and checksum always mean something.

use std::path::Path;

use anyhow::{bail, Context, Result};
use bach_pack::config::Project;
use bach_pack::manifest::{ArchEntry, FeedEntry};

use super::{digest_of, download_url, private_key, signing_time, FILE_NAME};

/// The name the macOS file has in a feed's `arch` list.
pub const NAME: &str = "macos";

/// After `bach build` wrote a macOS setup file: puts it in the feed beside the
/// Windows one when that was built from the same version, or makes a feed of its own.
pub fn after_mac_build(project: &Project, mac_file: &Path, feed_from: Option<&Path>) -> Result<()> {
    let Some(template) = &project.update_download_url else {
        crate::output::warnings(&["[updates] has no download_url, so the macOS setup file isn't listed in latest.json; \
             after uploading it, run `bach feed SETUP --macos FILE --url URL`"
            .to_string()]);
        return Ok(());
    };
    let url = download_url(template, mac_file);
    let target = mac_file.parent().unwrap_or(Path::new(".")).join(FILE_NAME);
    let version = project.manifest.app.version.clone();
    // The feed another job wrote comes first (`--feed-from`), then one beside the output.
    let base = feed_from
        .and_then(read_feed)
        .filter(|feed| feed.version == version)
        .or_else(|| read_feed(&target).filter(|feed| feed.version == version));
    let mut entry = match base {
        Some(existing) => existing,
        _ => whole_feed(mac_file, &url, &version)?,
    };
    entry.arch.insert(NAME.to_string(), arch_entry(mac_file, &url)?);
    sign(project, &mut entry)?;
    let json = serde_json::to_string_pretty(&entry)?;
    std::fs::write(&target, format!("{json}\n")).with_context(|| format!("couldn't write {}", target.display()))?;
    crate::output::field("Update feed", target.display());
    crate::output::result("feed", target.display().to_string());
    Ok(())
}

fn read_feed(path: &Path) -> Option<FeedEntry> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// A feed whose own entry is the macOS file, for a project with no Windows setup file.
fn whole_feed(file: &Path, url: &str, version: &str) -> Result<FeedEntry> {
    check_url(url)?;
    let (sha256, size) = digest_of(file)?;
    Ok(FeedEntry {
        version: version.to_string(),
        url: url.to_string(),
        sha256,
        notes: None,
        size: Some(size),
        rollout: None,
        delta: None,
        signature: None,
        published: None,
        arch: Default::default(),
    })
}

fn check_url(url: &str) -> Result<()> {
    if !bach_pack::manifest::extras::is_https(url) {
        bail!("the macOS setup file's download link \"{url}\" must be an https link");
    }
    Ok(())
}

/// What the feed lists for a macOS setup file: link, SHA-256 and size. The file
/// has to be a disk image or a zip, which is what `bach build` writes.
pub fn arch_entry(file: &Path, url: &str) -> Result<ArchEntry> {
    check_url(url)?;
    let shown = file.display();
    let extension = file.extension().map(|ext| ext.to_string_lossy().to_ascii_lowercase());
    if !matches!(extension.as_deref(), Some("dmg") | Some("zip")) {
        bail!("{shown} isn't a macOS setup file (a .dmg or a .zip that `bach build` wrote)");
    }
    let (sha256, size) = digest_of(file)?;
    Ok(ArchEntry { url: url.to_string(), sha256, size: Some(size), signature: None })
}

/// Signs what is unsigned in `entry` when the app has a publisher key: the
/// macOS entry always, the entry itself when this feed is new.
fn sign(project: &Project, entry: &mut FeedEntry) -> Result<()> {
    let Some(updates) = project.manifest.updates.as_ref().filter(|updates| updates.public_key.is_some()) else {
        return Ok(());
    };
    match private_key() {
        Ok(private) => {
            let mine = bach_pack::feedsig::public_of(&private)?;
            let trusted = [updates.public_key.as_deref(), updates.next_public_key.as_deref()];
            if !trusted.iter().flatten().any(|key| key.eq_ignore_ascii_case(&mine)) {
                bail!("the feed signing key doesn't belong to [updates] public_key (or next_public_key); use the private key that `bach feed-key` made with it");
            }
            let id = &project.manifest.app.id;
            if entry.signature.is_none() {
                entry.published = Some(bach_pack::feedsig::format_utc(signing_time()));
                entry.signature = Some(bach_pack::feedsig::sign(&private, id, entry)?);
            }
            let version = entry.version.clone();
            if let Some(listed) = entry.arch.get_mut(NAME) {
                listed.signature = Some(bach_pack::feedsig::sign_arch(&private, id, &version, NAME, listed)?);
            }
        }
        Err(err) => {
            crate::output::warnings(&[format!("{FILE_NAME} is not signed: {err:#}. Run `bach feed` with the key before you publish it")])
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dmg(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("bach-macfeed-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn a_disk_image_is_listed_with_its_checksum() {
        let file = dmg("a.dmg", b"abc");
        let entry = arch_entry(&file, "https://example.com/a.dmg").unwrap();
        assert_eq!(entry.sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(entry.size, Some(3));
        assert!(entry.signature.is_none());
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn only_setup_downloads_over_https_are_listed() {
        let exe = dmg("a.exe", b"MZ");
        assert!(arch_entry(&exe, "https://example.com/a.exe").is_err());
        let file = dmg("b.dmg", b"x");
        assert!(arch_entry(&file, "http://example.com/b.dmg").is_err());
        std::fs::remove_file(exe).unwrap();
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn a_mac_only_feed_points_at_the_mac_file() {
        let file = dmg("c.dmg", b"hello");
        let feed = whole_feed(&file, "https://example.com/c.dmg", "1.2.3").unwrap();
        assert_eq!((feed.version.as_str(), feed.url.as_str(), feed.size), ("1.2.3", "https://example.com/c.dmg", Some(5)));
        std::fs::remove_file(file).unwrap();
    }
}
