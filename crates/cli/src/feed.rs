//! The update feed: the small JSON an installed app reads to learn about a
//! newer version (`uninstall.exe --check-update`).
//!
//! `bach build` writes `latest.json` next to the setup file when the config
//! has `[updates]` with a download link. Signing changes the file and so
//! its SHA-256: after signing, write the feed again with `bach feed`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use bach_pack::config::{self, Overrides, Project};
use bach_pack::container;
use bach_pack::manifest::{DeltaEntry, FeedEntry};
use sha2::{Digest, Sha256};

use crate::output::field;

pub const FILE_NAME: &str = "latest.json";
/// A release note longer than this is cut, so a feed stays small.
const MAX_NOTES: usize = 4000;

/// After a build: the feed, when the config says where the file will live.
/// `patch` is the update patch built this time and the version it updates
/// from; the feed offers it to computers that have that version.
pub fn after_build(project: &Project, setup: &Path, patch: Option<(&Path, &str)>) -> Result<()> {
    match &project.update_download_url {
        Some(template) => {
            let delta = match patch {
                Some((file, _)) if worth_offering(file, setup) => Some(delta_entry(file, &download_url(template, file))?),
                Some((file, _)) => {
                    crate::output::warnings(&[format!(
                        "{} is not much smaller than the full setup file (it carries the setup runtime, about 10 MB, and every file that changed), so latest.json doesn't offer it. Add it with `bach feed --patch` if you want it anyway",
                        file.display()
                    )]);
                    None
                }
                None => None,
            };
            let written = write(setup, &download_url(template, setup), None, None, delta, None)?;
            field("Update feed", written.display());
        }
        None => crate::output::warnings(&["[updates] has no download_url, so no latest.json was written; \
             after uploading the setup file, run `bach feed SETUP --url URL`"
            .to_string()]),
    }
    Ok(())
}

/// What `bach feed` was asked for.
pub struct Options<'a> {
    pub setup: &'a Path,
    /// The link the setup file is downloaded from; the config's download
    /// link when left out.
    pub url: Option<&'a str>,
    /// The `bach.toml` to take the link from (`bach.toml` here by default).
    pub config: Option<&'a Path>,
    pub notes: Option<&'a str>,
    pub notes_file: Option<&'a Path>,
    /// A staged rollout: the percentage of computers that take it by themselves.
    pub rollout: Option<u8>,
    pub out: Option<&'a Path>,
    /// An update patch (`bach build --patch-from`) to offer to computers that
    /// have the version it updates from.
    pub patch: Option<&'a Path>,
    /// The link the patch is downloaded from; the config's download link when
    /// left out.
    pub patch_url: Option<&'a str>,
}

/// `bach feed`: the feed for a (possibly signed) setup file.
pub fn command(options: Options) -> Result<()> {
    let url = match options.url {
        Some(url) => url.to_string(),
        None => url_from_config(options.setup, options.setup, options.config)?,
    };
    let delta = match options.patch {
        Some(patch) => {
            let patch_url = match options.patch_url {
                Some(url) => url.to_string(),
                None => url_from_config(options.setup, patch, options.config)?,
            };
            Some(delta_entry(patch, &patch_url)?)
        }
        None => None,
    };
    let notes = match (options.notes, options.notes_file) {
        (Some(notes), _) => Some(notes.to_string()),
        (None, Some(file)) => {
            Some(std::fs::read_to_string(file).with_context(|| format!("couldn't read {}", file.display()))?)
        }
        (None, None) => None,
    };
    let written = write(options.setup, &url, notes.as_deref(), options.rollout, delta.clone(), options.out)?;
    println!("Wrote {}", written.display());
    field("Download", &url);
    if let Some(delta) = &delta {
        field("Patch", format!("from {}, {}", delta.from, delta.url));
    }
    if let Some(percent) = options.rollout {
        field("Rollout", format!("{percent}% of computers take it by themselves"));
    }
    Ok(())
}

/// The download link the config describes for `file` (the setup file, or its
/// patch), for the version `setup` carries (so the link and the file can't
/// disagree).
fn url_from_config(setup: &Path, file: &Path, config: Option<&Path>) -> Result<String> {
    let path = match config {
        Some(path) => path.to_path_buf(),
        None if Path::new("bach.toml").is_file() => PathBuf::from("bach.toml"),
        None => bail!(
            "pass --url with the https link the setup file will be downloaded from, \
             or --config with a bach.toml that has [updates]"
        ),
    };
    let version = setup_version(setup)?;
    let project = config::load_with(&path, &Overrides { version: Some(version) })?;
    match project.update_download_url {
        Some(template) => Ok(download_url(&template, file)),
        None => bail!(
            "{} has no [updates] download link; give github = \"owner/repo\" or download_url there, or pass --url",
            path.display()
        ),
    }
}

/// `https://example.com/{file}` with the setup file's name in it, encoded
/// for a link.
fn download_url(template: &str, setup: &Path) -> String {
    let name = setup.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    template.replace("{file}", &encode(&name))
}

/// Percent-encodes everything but letters, digits and `- . _ ~`.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn setup_version(setup: &Path) -> Result<String> {
    let mut file = std::fs::File::open(setup).with_context(|| format!("couldn't open {}", setup.display()))?;
    let Some(layout) = container::locate(&mut file)? else {
        bail!("{} isn't a Bach setup file", setup.display());
    };
    Ok(container::read_manifest(&mut file, &layout)?.app.version)
}

/// Whether a patch saves enough to be offered: at least a fifth smaller than
/// the full setup file. A patch is a setup file itself, so it carries the
/// runtime and never beats a small app's full download.
fn worth_offering(patch: &Path, full: &Path) -> bool {
    let size = |path: &Path| std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(u64::MAX);
    saves_enough(size(patch), size(full))
}

fn saves_enough(patch: u64, full: u64) -> bool {
    patch.saturating_mul(10) < full.saturating_mul(8)
}

/// What the feed says about an update patch: the version it updates from,
/// where it is, and its SHA-256, read from the (signed) file.
fn delta_entry(patch: &Path, url: &str) -> Result<DeltaEntry> {
    if !bach_pack::manifest::extras::is_https(url) {
        bail!("the patch's download link \"{url}\" must be an https link");
    }
    let mut file = std::fs::File::open(patch).with_context(|| format!("couldn't open {}", patch.display()))?;
    let Some(layout) = container::locate(&mut file)? else {
        bail!("{} isn't a Bach setup file", patch.display());
    };
    let manifest = container::read_manifest(&mut file, &layout)?;
    let Some(from) = manifest.patch_from else {
        bail!("{} isn't an update patch; make one with bach build --patch-from OLD-SETUP.exe", patch.display());
    };
    let bytes = std::fs::read(patch).with_context(|| format!("couldn't read {}", patch.display()))?;
    Ok(DeltaEntry { from, url: url.to_string(), sha256: bach_pack::protect::hex(&Sha256::digest(&bytes)), size: Some(bytes.len() as u64) })
}

fn write(setup: &Path, url: &str, notes: Option<&str>, rollout: Option<u8>, delta: Option<DeltaEntry>, out: Option<&Path>) -> Result<PathBuf> {
    if rollout.is_some_and(|percent| percent > 100) {
        bail!("--rollout is a percentage, from 0 to 100");
    }
    if !bach_pack::manifest::extras::is_https(url) {
        bail!("the download link \"{url}\" must be an https link");
    }
    let mut file = std::fs::File::open(setup).with_context(|| format!("couldn't open {}", setup.display()))?;
    let Some(layout) = container::locate(&mut file)? else {
        bail!("{} isn't a Bach setup file", setup.display());
    };
    let manifest = container::read_manifest(&mut file, &layout)?;
    if manifest.patch_from.is_some() {
        bail!("{} is an update patch; point bach feed at the full setup file and pass the patch with --patch", setup.display());
    }
    if !layout.has_archive() {
        let what = if manifest.online { "an online installer, which carries no files" } else { "an uninstaller" };
        bail!("{} is {what}; point bach feed at the full setup file", setup.display());
    }
    let bytes = std::fs::read(setup).with_context(|| format!("couldn't read {}", setup.display()))?;
    let notes = notes.map(str::trim).filter(|notes| !notes.is_empty()).map(|notes| notes.chars().take(MAX_NOTES).collect());
    let entry = FeedEntry {
        version: manifest.app.version.clone(),
        url: url.to_string(),
        sha256: bach_pack::protect::hex(&Sha256::digest(&bytes)),
        notes,
        size: Some(bytes.len() as u64),
        rollout,
        delta,
    };
    let target = match out {
        Some(out) => out.to_path_buf(),
        None => setup.parent().unwrap_or(Path::new(".")).join(FILE_NAME),
    };
    let json = serde_json::to_string_pretty(&entry)?;
    std::fs::write(&target, format!("{json}\n")).with_context(|| format!("couldn't write {}", target.display()))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_patch_has_to_be_clearly_smaller() {
        assert!(saves_enough(10_000_000, 40_000_000));
        assert!(saves_enough(79, 100));
        assert!(!saves_enough(80, 100), "a fifth smaller, or it isn't worth a second download path");
        assert!(!saves_enough(12_000_000, 11_000_000));
        assert!(!saves_enough(u64::MAX, 100), "an unreadable file is never offered");
    }

    #[test]
    fn download_urls_name_the_file() {
        let url = download_url("https://example.com/downloads/{file}", Path::new("dist/My App-Setup-1.0.exe"));
        assert_eq!(url, "https://example.com/downloads/My%20App-Setup-1.0.exe");
        let url = download_url("https://example.com/{file}", Path::new("dist/Café #1+2.exe"));
        assert_eq!(url, "https://example.com/Caf%C3%A9%20%231%2B2.exe");
        let url = download_url("https://github.com/o/r/releases/download/v1/{file}", Path::new("dist/Hello-Setup-1.0.0.exe"));
        assert!(url.ends_with("/Hello-Setup-1.0.0.exe"), "safe names stay as they are: {url}");
    }
}
