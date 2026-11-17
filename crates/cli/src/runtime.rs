//! Finding the setup runtime (`bach-setup.exe`) every installer is built on.
//!
//! In order: `--runtime` (or `BACH_RUNTIME`), the file next to `bach`, the
//! copy a `cargo build` left in this checkout, a copy downloaded earlier,
//! and finally the one published with this version of Bach on GitHub,
//! checked against its SHA-256 before it's used. So `bach build` works from
//! a release zip, from source, and on macOS or Linux CI with nothing but
//! the `bach` binary.
//!
//! The runtime ends up inside every setup file, so it is the one download
//! Bach must not take on trust:
//!
//! - A release build of `bach` has the runtime's SHA-256 built in
//!   (`BACH_RUNTIME_SHA256`, set by the release workflow). A download, and a
//!   copy found in the cache, must match it, so a changed release host or
//!   `BACH_RELEASES` mirror can't swap the file.
//! - A build without it (from source) falls back to the `.sha256` published
//!   next to the file, which only guards against a damaged transfer, and
//!   `bach doctor` says so.
//! - Downloads use https only, redirects included, and never go through a
//!   shell string, so a mirror address can't run commands.
//! - The cache lives in the user's own folder, never in the shared temp
//!   folder, and a cached copy is checked again before every use.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

/// The runtime's file name, next to `bach.exe` in a release and in
/// `target\<profile>` after `cargo build`.
pub const FILE_NAME: &str = "bach-setup.exe";

/// Where releases live. A fork can point `BACH_RELEASES` elsewhere.
const RELEASES: &str = "https://github.com/usebach/bach/releases/download";

/// The SHA-256 of this version's `bach-setup.exe`, when the release build
/// that made this `bach` knew it.
const PINNED_SHA256: Option<&str> = option_env!("BACH_RUNTIME_SHA256");

/// Where the runtime was found, for `bach doctor`.
pub enum Found {
    Here(PathBuf),
    /// Not on disk yet; this URL would be downloaded.
    Downloadable(String),
}

/// The runtime to use, downloading it when there's no other way.
pub fn find(given: Option<&Path>) -> Result<PathBuf> {
    match locate(given)? {
        Found::Here(path) => Ok(path),
        Found::Downloadable(url) => download(&url),
    }
}

/// Like [`find`], without downloading.
pub fn locate(given: Option<&Path>) -> Result<Found> {
    if let Some(path) = given {
        if !path.is_file() {
            bail!("the setup runtime {} doesn't exist", path.display());
        }
        return Ok(Found::Here(path.to_path_buf()));
    }
    let here = std::env::current_exe().context("couldn't find where bach is")?;
    let beside = here.parent().map(|dir| dir.join(FILE_NAME));
    for candidate in beside.into_iter().chain(checkout_builds(&here)) {
        if candidate.is_file() {
            return Ok(Found::Here(candidate));
        }
    }
    // A cached copy counts only while it still matches what was verified.
    if let Ok(cache) = cached() {
        if cache.is_file() && cache_is_trusted(&cache, PINNED_SHA256) {
            return Ok(Found::Here(cache));
        }
    }
    Ok(Found::Downloadable(url()?))
}

/// In a checkout, `bach` may run from `target\debug` while the runtime was
/// built in `target\release`, or the other way round.
fn checkout_builds(bach: &Path) -> Vec<PathBuf> {
    let Some(target) = bach.parent().and_then(Path::parent) else {
        return Vec::new();
    };
    if target.file_name().is_none_or(|name| name != "target") {
        return Vec::new();
    }
    ["release", "debug"].iter().map(|profile| target.join(profile).join(FILE_NAME)).collect()
}

/// The download cache: `%LOCALAPPDATA%\bach\runtime\<version>` on Windows,
/// `$XDG_CACHE_HOME/bach/runtime/<version>` (or `~/.cache/…`) elsewhere.
///
/// There is no fallback to the shared temp folder: on a build server another
/// user could have planted a file at that predictable path. Without a home
/// to cache in, pass `--runtime` or set `BACH_RUNTIME`.
fn cached() -> Result<PathBuf> {
    let Some(root) = cache_root() else {
        bail!(
            "there's no folder of yours to cache the setup runtime in ({} isn't set); \
             pass --runtime or set BACH_RUNTIME to a bach-setup.exe",
            if cfg!(windows) { "LOCALAPPDATA" } else { "XDG_CACHE_HOME or HOME" }
        );
    };
    Ok(root.join("runtime").join(env!("CARGO_PKG_VERSION")).join(FILE_NAME))
}

/// Bach's own folder in the user's cache (`%LOCALAPPDATA%\bach`,
/// `$XDG_CACHE_HOME/bach` or `~/.cache/bach`). The runtime and the update
/// check keep their files below it.
pub fn cache_root() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
    }
    .filter(|base| !base.as_os_str().is_empty())?;
    Some(base.join("bach"))
}

/// The file with the checked hash kept beside a cached runtime.
fn sidecar(runtime: &Path) -> PathBuf {
    let mut name = runtime.as_os_str().to_owned();
    name.push(".sha256");
    PathBuf::from(name)
}

/// Whether a cached runtime is still the file that was verified: its hash
/// equals the pinned one, or, in a build without one, the hash kept beside it.
fn cache_is_trusted(runtime: &Path, pinned: Option<&str>) -> bool {
    let Ok(bytes) = std::fs::read(runtime) else {
        return false;
    };
    let actual = sha256_of(&bytes);
    match pinned.map(|pin| pin.trim().to_ascii_lowercase()) {
        Some(pin) => actual == pin,
        None => std::fs::read_to_string(sidecar(runtime))
            .is_ok_and(|kept| kept.split_whitespace().next().unwrap_or_default().eq_ignore_ascii_case(&actual)),
    }
}

pub fn sha256_of(bytes: &[u8]) -> String {
    bach_pack::protect::hex(&Sha256::digest(bytes))
}

fn url() -> Result<String> {
    releases_url(std::env::var("BACH_RELEASES").ok().as_deref())
}

/// The runtime's address under `releases` (or the default). https only: the
/// runtime ends up in every installer.
fn releases_url(releases: Option<&str>) -> Result<String> {
    let releases = releases.unwrap_or(RELEASES).trim();
    if !releases.to_ascii_lowercase().starts_with("https://") {
        bail!("BACH_RELEASES is \"{releases}\", but it must start with https://, because the runtime is run inside every installer");
    }
    Ok(format!("{}/v{}/{FILE_NAME}", releases.trim_end_matches('/'), env!("CARGO_PKG_VERSION")))
}

/// What a download must hash to: the hash built into this `bach`, or else the
/// one published next to the file.
fn expected_hash(pinned: Option<&str>, published: &str) -> String {
    match pinned {
        Some(pin) => pin.trim().to_ascii_lowercase(),
        None => published.split_whitespace().next().unwrap_or_default().to_ascii_lowercase(),
    }
}

/// Downloads the runtime, checks it, and keeps it in the cache for next time.
fn download(url: &str) -> Result<PathBuf> {
    if std::env::var_os("BACH_NO_DOWNLOAD").is_some() {
        bail!("{FILE_NAME} isn't next to bach and BACH_NO_DOWNLOAD is set; pass --runtime or set BACH_RUNTIME");
    }
    let target = cached()?;
    let dir = target.parent().context("the cache has no folder")?;
    private_dir(dir).with_context(|| format!("couldn't prepare the cache folder {}", dir.display()))?;
    eprintln!("  Downloading the setup runtime for Bach {} (once)…", env!("CARGO_PKG_VERSION"));
    let partial = dir.join(format!("{FILE_NAME}.partial"));
    let checksum = dir.join(format!("{FILE_NAME}.download.sha256"));
    let result = (|| -> Result<()> {
        fetch(url, &partial)?;
        let published = if PINNED_SHA256.is_some() {
            String::new()
        } else {
            fetch(&format!("{url}.sha256"), &checksum)?;
            std::fs::read_to_string(&checksum)?
        };
        let expected = expected_hash(PINNED_SHA256, &published);
        let bytes = std::fs::read(&partial)?;
        let actual = sha256_of(&bytes);
        if expected.len() != 64 || expected != actual {
            bail!(
                "the downloaded runtime doesn't match {}, so it wasn't used",
                if PINNED_SHA256.is_some() { "the SHA-256 built into this bach" } else { "its published SHA-256" }
            );
        }
        check_bare(&bytes, &partial)?;
        std::fs::write(sidecar(&target), format!("{actual}\n"))?;
        std::fs::rename(&partial, &target)?;
        Ok(())
    })();
    let _ = std::fs::remove_file(&partial);
    let _ = std::fs::remove_file(&checksum);
    result.with_context(|| {
        format!(
            "couldn't get {FILE_NAME} from {url}. Download the Bach {} zip from \
             https://github.com/usebach/bach/releases and keep bach-setup.exe next to bach, \
             or pass --runtime",
            env!("CARGO_PKG_VERSION")
        )
    })?;
    Ok(target)
}

/// Creates the cache folder for its owner only and refuses one that someone
/// else owns or that others can write to.
#[cfg(unix)]
pub fn private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let meta = std::fs::metadata(dir)?;
    if meta.mode() & 0o022 != 0 {
        bail!("{} can be written by other users", dir.display());
    }
    let home = std::env::var_os("HOME").and_then(|home| std::fs::metadata(home).ok());
    if home.is_some_and(|home| home.uid() != meta.uid()) {
        bail!("{} belongs to someone else", dir.display());
    }
    Ok(())
}

/// On Windows the cache sits in `%LOCALAPPDATA%`, which only its user (and
/// administrators) can write to.
#[cfg(not(unix))]
pub fn private_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    Ok(())
}

/// The arguments curl gets: https only, redirects included, and giving up
/// after `seconds` when a limit is set.
fn curl_args(url: &str, to: &Path, seconds: Option<u32>) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["--fail", "--location", "--proto", "=https", "--proto-redir", "=https"]
        .iter()
        .chain(&["--silent", "--show-error", "--retry", "2"])
        .map(OsString::from)
        .collect();
    if let Some(seconds) = seconds {
        args.push("--max-time".into());
        args.push(seconds.to_string().into());
    }
    args.push("--output".into());
    args.push(to.into());
    args.push(url.into());
    args
}

/// The PowerShell fallback. The address and the file arrive in environment
/// variables, so nothing a mirror's address contains is ever parsed as code.
const POWERSHELL_FETCH: &str = "$ProgressPreference='SilentlyContinue'; \
     $r = Invoke-WebRequest -UseBasicParsing -PassThru -TimeoutSec ([int]$env:BACH_FETCH_TIMEOUT) -Uri $env:BACH_FETCH_URL -OutFile $env:BACH_FETCH_TO; \
     $final = if ($r.BaseResponse.ResponseUri) { $r.BaseResponse.ResponseUri } else { $r.BaseResponse.RequestMessage.RequestUri }; \
     if ($final.Scheme -ne 'https') { Remove-Item -Force $env:BACH_FETCH_TO; exit 3 }";

/// Fetches `url` into `to` with curl (built into Windows 10+, macOS and
/// most Linux), or PowerShell on a Windows without it.
pub fn fetch(url: &str, to: &Path) -> Result<()> {
    fetch_within(url, to, None)
}

/// [`fetch`] that gives up after `seconds` (none: no limit), for requests a
/// person shouldn't have to wait on, like the update check.
pub fn fetch_within(url: &str, to: &Path, seconds: Option<u32>) -> Result<()> {
    if !url.to_ascii_lowercase().starts_with("https://") {
        bail!("{url} isn't an https address");
    }
    match Command::new("curl").args(curl_args(url, to, seconds)).status() {
        Ok(status) if status.success() => return Ok(()),
        Ok(status) => bail!("curl couldn't download {url} ({status})"),
        Err(_) if cfg!(windows) => {}
        Err(_) => bail!("downloading needs curl, which isn't installed"),
    }
    let status = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", POWERSHELL_FETCH])
        .env("BACH_FETCH_URL", url)
        .env("BACH_FETCH_TO", to)
        .env("BACH_FETCH_TIMEOUT", seconds.unwrap_or(0).to_string())
        .status()?;
    if !status.success() {
        bail!("PowerShell couldn't download {url} over https");
    }
    Ok(())
}

/// What `bach doctor` says about how this build trusts the runtime.
pub fn trust_note() -> String {
    match PINNED_SHA256 {
        Some(pin) => format!("this bach expects SHA-256 {pin}"),
        None => "this bach has no SHA-256 built in (a build from source); downloads are checked against the .sha256 next to them".into(),
    }
}

/// The SHA-256 of the runtime at `path`, for `bach doctor`.
pub fn hash_of(path: &Path) -> Result<String> {
    Ok(sha256_of(&std::fs::read(path).with_context(|| format!("couldn't read {}", path.display()))?))
}

/// The runtime's bytes, checked to be a bare runtime and not a setup file
/// that already carries a payload.
pub fn read(path: &Path) -> Result<Vec<u8>> {
    let bytes = std::fs::read(path).with_context(|| format!("couldn't read {}", path.display()))?;
    check_bare(&bytes, path)?;
    Ok(bytes)
}

fn check_bare(bytes: &[u8], path: &Path) -> Result<()> {
    if !bytes.starts_with(b"MZ") {
        bail!("{} isn't a Windows executable", path.display());
    }
    let mut file = std::fs::File::open(path).with_context(|| format!("couldn't open {}", path.display()))?;
    if bach_pack::container::locate(&mut file)?.is_some() {
        bail!("{} is already an installer; point --runtime at a plain bach-setup.exe", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkouts_look_in_both_profiles() {
        let bach = Path::new("repo").join("target").join("debug").join("bach.exe");
        let found = checkout_builds(&bach);
        assert_eq!(found[0], Path::new("repo").join("target").join("release").join(FILE_NAME));
        assert!(checkout_builds(Path::new("tools/bach.exe")).is_empty());
    }

    #[test]
    fn downloads_name_this_version() {
        let url = releases_url(None).unwrap();
        assert!(url.ends_with(&format!("/v{}/bach-setup.exe", env!("CARGO_PKG_VERSION"))));
        let mirror = releases_url(Some("https://mirror.example/bach/")).unwrap();
        assert!(mirror.starts_with("https://mirror.example/bach/v"));
    }

    #[test]
    fn a_mirror_must_use_https() {
        for bad in ["http://mirror.example", "ftp://mirror.example", "file:///c:/x", "mirror.example", "  HTTP://x"] {
            assert!(releases_url(Some(bad)).is_err(), "{bad:?} should be refused");
        }
        assert!(releases_url(Some("HTTPS://mirror.example")).is_ok());
    }

    #[test]
    fn curl_follows_only_https() {
        let args: Vec<String> =
            curl_args("https://x.example/a", Path::new("out"), None).iter().map(|a| a.to_string_lossy().into_owned()).collect();
        let joined = args.join(" ");
        assert!(joined.contains("--proto =https") && joined.contains("--proto-redir =https"), "{joined}");
        assert!(!joined.contains("--max-time"), "{joined}");
        let limited = curl_args("https://x.example/a", Path::new("out"), Some(10))
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(limited.join(" ").contains("--max-time 10"));
        assert_eq!(args.last().map(String::as_str), Some("https://x.example/a"));
    }

    #[test]
    fn an_address_with_a_quote_never_reaches_the_script() {
        // The fallback script is a constant: the address travels in an
        // environment variable, so nothing in it can close a string.
        assert!(!POWERSHELL_FETCH.contains("{url}") && !POWERSHELL_FETCH.contains("http://"));
        assert!(POWERSHELL_FETCH.contains("$env:BACH_FETCH_URL"));
        assert!(fetch("http://x.example/a", Path::new("out")).is_err(), "plain http is refused before anything runs");
    }

    #[test]
    fn a_built_in_hash_beats_the_published_one() {
        let pin = "a".repeat(64);
        assert_eq!(expected_hash(Some(&pin), &"b".repeat(64)), pin);
        assert_eq!(expected_hash(None, &format!("{}  bach-setup.exe\n", "B".repeat(64))), "b".repeat(64));
    }

    #[test]
    fn a_cached_runtime_is_checked_before_use() {
        let dir = std::env::temp_dir().join(format!("bach-runtime-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(FILE_NAME);
        std::fs::write(&file, b"MZ original").unwrap();
        let good = sha256_of(b"MZ original");

        assert!(!cache_is_trusted(&file, None), "no hash kept beside it: not verified");
        std::fs::write(sidecar(&file), format!("{good}\n")).unwrap();
        assert!(cache_is_trusted(&file, None));
        assert!(cache_is_trusted(&file, Some(&good)));
        assert!(!cache_is_trusted(&file, Some(&"0".repeat(64))), "a different pinned hash");

        std::fs::write(&file, b"MZ planted").unwrap();
        assert!(!cache_is_trusted(&file, None), "the file no longer matches the hash kept beside it");
        assert!(!cache_is_trusted(&file, Some(&good)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
