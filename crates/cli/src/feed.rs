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
use bach_pack::manifest::{ArchEntry, DeltaEntry, FeedEntry};

use crate::output::field;

pub const FILE_NAME: &str = "latest.json";
/// A release note longer than this is cut, so a feed stays small.
const MAX_NOTES: usize = 4000;

/// A setup file for another architecture that the feed lists.
pub struct ArchFile {
    /// `arm64`.
    pub name: String,
    /// The https link it is downloaded from.
    pub url: String,
    pub path: PathBuf,
}

/// After a build: the feed, when the config says where the file will live.
/// `patch` is the update patch built this time and the version it updates
/// from; the feed offers it to computers that have that version.
pub fn after_build(project: &Project, setup: &Path, patch: Option<(&Path, &str)>, arch: &[(String, PathBuf)]) -> Result<()> {
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
            // A build without the private key (a developer's own machine) still writes the
            // feed, unsigned, with a warning; `bach feed` is where signing is required.
            let listed: Vec<ArchFile> = arch
                .iter()
                .map(|(name, file)| ArchFile { name: name.clone(), url: download_url(template, file), path: file.clone() })
                .collect();
            let written = write(Feed {
                setup,
                url: &download_url(template, setup),
                notes: None,
                rollout: None,
                delta,
                arch: &listed,
                out: None,
                key_required: false,
            })?;
            field("Update feed", written.display());
            crate::output::result("feed", written.display().to_string());
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
    /// The ARM64 setup file of the same version, to list for ARM64 computers.
    pub arm64: Option<&'a Path>,
    /// The link it is downloaded from; the config's download link when left out.
    pub arm64_url: Option<&'a str>,
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
    let mut listed = Vec::new();
    if let Some(file) = options.arm64 {
        let url = match options.arm64_url {
            Some(url) => url.to_string(),
            None => url_from_config(options.setup, file, options.config)?,
        };
        listed.push(ArchFile { name: "arm64".into(), url, path: file.to_path_buf() });
    }
    let notes = match (options.notes, options.notes_file) {
        (Some(notes), _) => Some(notes.to_string()),
        (None, Some(file)) => Some(bach_pack::textfile::read(file).with_context(|| format!("couldn't read {}", file.display()))?),
        (None, None) => None,
    };
    let written = write(Feed {
        setup: options.setup,
        url: &url,
        notes: notes.as_deref(),
        rollout: options.rollout,
        delta: delta.clone(),
        arch: &listed,
        out: options.out,
        key_required: true,
    })?;
    crate::output::result("feed", written.display().to_string());
    say!("Wrote {}", written.display());
    field("Download", &url);
    if let Some(delta) = &delta {
        field("Patch", format!("from {}, {}", delta.from, delta.url));
    }
    for file in &listed {
        field("Also", format!("{} setup file, {}", file.name, file.url));
    }
    if let Some(percent) = options.rollout {
        field("Rollout", format!("{percent}% of computers take it by themselves"));
    }
    Ok(())
}

/// The publisher's private feed key: `BACH_FEED_KEY`, or the first line of the
/// file `BACH_FEED_KEY_FILE` names.
fn private_key() -> Result<String> {
    use bach_pack::feedsig::{KEY_ENV, KEY_FILE_ENV};
    if let Ok(key) = std::env::var(KEY_ENV) {
        return Ok(key.trim().to_string());
    }
    if let Some(file) = std::env::var_os(KEY_FILE_ENV) {
        let text =
            std::fs::read_to_string(&file).with_context(|| format!("couldn't read the feed key file {}", Path::new(&file).display()))?;
        return Ok(text.lines().next().unwrap_or_default().trim().to_string());
    }
    bail!(
        "[updates] has a public_key, so the feed must be signed: set {KEY_ENV} (or {KEY_FILE_ENV}) to the private key `bach feed-key` made"
    )
}

/// `bach feed-key`: a new publisher key pair for signed update feeds.
pub fn key_command(out: Option<&Path>) -> Result<()> {
    let (private, public) = bach_pack::feedsig::generate()?;
    let file = out.map_or_else(|| PathBuf::from("feed-key.private"), Path::to_path_buf);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    // Owner only from the moment it exists (not chmod-ed afterwards). On Windows the file gets the
    // rights of its folder: keep it somewhere only you can read.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut handle = options.open(&file).with_context(|| format!("couldn't create {} (it must not exist yet)", file.display()))?;
    std::io::Write::write_all(&mut handle, format!("{private}\n").as_bytes())?;
    say!("Wrote the private key to {}. Keep it secret and out of Git; it signs your update feeds.", file.display());
    say!("Put this in bach.toml:\n\n[updates]\npublic_key = \"{public}\"\n");
    say!(
        "Sign with it by setting {} to the file's content (or {} to its path) when you run `bach build` or `bach feed`.",
        bach_pack::feedsig::KEY_ENV,
        bach_pack::feedsig::KEY_FILE_ENV
    );
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
    let project = config::load_with(&path, &Overrides { version: Some(version), ..Overrides::default() })?;
    match project.update_download_url {
        Some(template) => Ok(download_url(&template, file)),
        None => {
            bail!("{} has no [updates] download link; give github = \"owner/repo\" or download_url there, or pass --url", path.display())
        }
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
    let (sha256, size) = digest_of(patch)?;
    Ok(DeltaEntry { from, url: url.to_string(), sha256, size: Some(size) })
}

/// A file's SHA-256 and size, read in pieces: a setup file can be gigabytes,
/// and must not be held in memory to be hashed.
pub fn digest_of(path: &Path) -> Result<(String, u64)> {
    let file = std::fs::File::open(path).with_context(|| format!("couldn't read {}", path.display()))?;
    let size = file.metadata().with_context(|| format!("couldn't read {}", path.display()))?.len();
    let sha256 = bach_pack::protect::sha256_hex(std::io::BufReader::with_capacity(1 << 20, file))
        .with_context(|| format!("couldn't read {}", path.display()))?;
    Ok((sha256, size))
}

/// What the feed for one setup file is made of.
struct Feed<'a> {
    setup: &'a Path,
    url: &'a str,
    notes: Option<&'a str>,
    rollout: Option<u8>,
    delta: Option<DeltaEntry>,
    arch: &'a [ArchFile],
    out: Option<&'a Path>,
    /// A missing signing key is an error (`bach feed`), not a warning (`bach build`).
    key_required: bool,
}

fn write(feed: Feed) -> Result<PathBuf> {
    let Feed { setup, url, notes, rollout, delta, arch, out, key_required } = feed;
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
    let (sha256, size) = digest_of(setup)?;
    let mut listed = std::collections::BTreeMap::new();
    for file in arch {
        listed.insert(file.name.clone(), arch_entry(file, &manifest)?);
    }
    let notes = notes.map(str::trim).filter(|notes| !notes.is_empty()).map(|notes| notes.chars().take(MAX_NOTES).collect());
    let mut entry = FeedEntry {
        version: manifest.app.version.clone(),
        url: url.to_string(),
        sha256,
        notes,
        size: Some(size),
        rollout,
        delta,
        signature: None,
        published: None,
        arch: listed,
    };
    // An app that has a publisher key accepts only signed feeds, so an unsigned
    // one would silently stop every installed copy from updating.
    if let Some(updates) = manifest.updates.as_ref().filter(|updates| updates.public_key.is_some()) {
        match private_key() {
            Ok(private) => {
                let mine = bach_pack::feedsig::public_of(&private)?;
                let trusted = [updates.public_key.as_deref(), updates.next_public_key.as_deref()];
                if !trusted.iter().flatten().any(|key| key.eq_ignore_ascii_case(&mine)) {
                    bail!("the feed signing key doesn't belong to [updates] public_key (or next_public_key) in this setup file; use the private key that `bach feed-key` made with it");
                }
                // Signed with the time, so an app that limits a feed's age can tell it is fresh.
                entry.published = Some(bach_pack::feedsig::format_utc(signing_time()));
                entry.signature = Some(bach_pack::feedsig::sign(&private, &manifest.app.id, &entry)?);
                let version = entry.version.clone();
                for (name, listed) in entry.arch.iter_mut() {
                    listed.signature = Some(bach_pack::feedsig::sign_arch(&private, &manifest.app.id, &version, name, listed)?);
                }
            }
            Err(err) if key_required => return Err(err),
            Err(err) => crate::output::warnings(&[format!(
                "{FILE_NAME} is not signed: {err:#}. Run `bach feed` with the key before you publish it"
            )]),
        }
    }
    crate::output::result("version", entry.version.clone());
    crate::output::result("url", entry.url.clone());
    crate::output::result("sha256", entry.sha256.clone());
    crate::output::result("size", size);
    crate::output::result("signed", entry.signature.is_some());
    let target = match out {
        Some(out) => out.to_path_buf(),
        None => setup.parent().unwrap_or(Path::new(".")).join(FILE_NAME),
    };
    let json = serde_json::to_string_pretty(&entry)?;
    std::fs::write(&target, format!("{json}\n")).with_context(|| format!("couldn't write {}", target.display()))?;
    Ok(target)
}

/// What the feed lists for a setup file of another architecture: its link, SHA-256 and
/// size, read from the (signed) file. It has to be a full setup file of the same app and
/// version, for that architecture, because an installed app swaps one for the other.
fn arch_entry(file: &ArchFile, main: &bach_pack::manifest::Manifest) -> Result<ArchEntry> {
    let shown = file.path.display();
    if !bach_pack::manifest::extras::OTHER_ARCHES.contains(&file.name.as_str()) {
        bail!("the feed can't list a {} setup file", file.name);
    }
    if !bach_pack::manifest::extras::is_https(&file.url) {
        bail!("the {} setup file's download link \"{}\" must be an https link", file.name, file.url);
    }
    let mut handle = std::fs::File::open(&file.path).with_context(|| format!("couldn't open {shown}"))?;
    let Some(layout) = container::locate(&mut handle)? else {
        bail!("{shown} isn't a Bach setup file");
    };
    let manifest = container::read_manifest(&mut handle, &layout)?;
    if manifest.app.id != main.app.id || manifest.app.version != main.app.version {
        bail!(
            "{shown} is {} {}, but the setup file the feed is for is {} {}: build both from one bach.toml and version",
            manifest.app.id,
            manifest.app.version,
            main.app.id,
            main.app.version
        );
    }
    if manifest.patch_from.is_some() || !layout.has_archive() {
        bail!("{shown} isn't a full setup file (an update patch, an online installer or an uninstaller can't be listed here)");
    }
    if file.name == "arm64" && config::pe_machine(&file.path) != Some(0xAA64) {
        bail!("{shown} isn't an ARM64 setup file: it was built on the x64 runtime, so list it for x64 or build it with --runtime-arm64");
    }
    let (sha256, size) = digest_of(&file.path)?;
    Ok(ArchEntry { url: file.url.clone(), sha256, size: Some(size), signature: None })
}

/// When the feed is signed: now, or `SOURCE_DATE_EPOCH` when it is set (the
/// reproducible-builds convention), so a release that is rebuilt on purpose
/// gives the same feed.
fn signing_time() -> u64 {
    std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|seconds| seconds.trim().parse().ok())
        .unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_file_is_hashed_in_pieces_with_its_size() {
        let path = std::env::temp_dir().join(format!("bach-digest-{}", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        let (sha256, size) = super::digest_of(&path).unwrap();
        assert_eq!(sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(size, 3);
        // Bigger than the 1 MiB read buffer, to cross a piece boundary.
        std::fs::write(&path, vec![7u8; 3 << 20]).unwrap();
        let (_, size) = super::digest_of(&path).unwrap();
        assert_eq!(size, 3 << 20);
        std::fs::remove_file(&path).unwrap();
        assert!(super::digest_of(&path).is_err());
    }

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
