//! `[[requirements]]`, `[[shortcuts]]`, `[updates]` and `[access]`.

use anyhow::{bail, Result};
use serde::Deserialize;

use crate::manifest::extras::{is_https, is_valid_shortcut_name};
use crate::manifest::{Access, Check, Choice, Component, FileEntry, Requirement, RunStep, RunWhen, Shortcut, Updates};
use crate::paths;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementSection {
    kind: String,
    version: Option<String>,
    #[serde(default)]
    desktop: bool,
    path: Option<String>,
    key: Option<String>,
    build: Option<u32>,
    name: Option<String>,
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSection {
    /// What setup says it is doing ("Setting up {name}"); the file's name by default.
    name: Option<String>,
    /// A packaged .exe, relative to the install folder.
    file: String,
    arguments: Option<String>,
    /// `"install"` (default), `"first_install"` or `"uninstall"`.
    when: Option<String>,
    /// Exit codes that mean it worked; `[0]` by default.
    success_codes: Option<Vec<i32>>,
    #[serde(default)]
    ignore_errors: bool,
    /// Only when this optional component is installed.
    component: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShortcutSection {
    name: String,
    target: String,
    arguments: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdatesSection {
    /// The update feed's https link. Not needed with `github`.
    feed: Option<String>,
    /// `owner/repo`: updates come from that repository's GitHub releases.
    github: Option<String>,
    /// Where the setup file will be downloaded from; `{file}` is its name
    /// and `{version}` the app's version.
    download_url: Option<String>,
    /// Keep the app up to date in the background: `true` (offered, ticked,
    /// the default), `false` (offered, unticked), `"always"` or `"never"`.
    auto: Option<toml::Value>,
    /// More feeds to ask when the main one can't be reached.
    #[serde(default)]
    mirrors: Vec<String>,
}

/// The environment variable the access key is read from unless `key_env`
/// says another.
pub const DEFAULT_KEY_ENV: &str = "BACH_ACCESS_KEY";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessSection {
    /// Encrypt the packaged files with the key (default true).
    encrypt: Option<bool>,
    /// The environment variable that holds the key while building.
    key_env: Option<String>,
    /// Where people get a key; the key page links to it.
    get_key_url: Option<String>,
    /// Send the key with update requests (default true with `[updates]`).
    send_key: Option<bool>,
}

/// What `[access]` becomes: the manifest part, and the build-time part.
pub struct AccessSettings {
    pub manifest: Access,
    pub encrypt: bool,
    pub key_env: String,
}

/// What `[updates]` becomes: the manifest part, and the build-time part.
pub struct UpdateSettings {
    pub updates: Updates,
    /// The download link template for the feed `bach build` writes.
    pub download_url: Option<String>,
    /// The GitHub repository, when releases are the home of the updates.
    pub github: Option<String>,
}

pub fn requirements(sections: &[RequirementSection]) -> Result<Vec<Requirement>> {
    sections
        .iter()
        .enumerate()
        .map(|(index, section)| {
            let at = format!("requirements[{index}]");
            let check = match section.kind.trim().to_ascii_lowercase().as_str() {
                "webview2" => Check::Webview2,
                "vcredist" => Check::Vcredist,
                "dotnet" => {
                    let version = section.version.as_deref().map(str::trim).unwrap_or_default();
                    let valid = version.split('.').count() == 2 && version.split('.').all(|part| part.parse::<u32>().is_ok());
                    if !valid {
                        bail!("{at}.version is \"{version}\"; give the .NET version as major.minor, like \"8.0\"");
                    }
                    Check::Dotnet { version: version.to_string(), desktop: section.desktop }
                }
                "file" => {
                    let path = section.path.as_deref().map(str::trim).unwrap_or_default();
                    if path.is_empty() {
                        bail!("{at}.path is missing; give the file to look for, like '%SystemRoot%\\System32\\x.dll'");
                    }
                    Check::File { path: path.to_string() }
                }
                "registry" => {
                    let key = section.key.as_deref().map(str::trim).unwrap_or_default();
                    let upper = key.to_ascii_uppercase();
                    if !(upper.starts_with("HKLM\\") || upper.starts_with("HKCU\\")) {
                        bail!("{at}.key is \"{key}\"; it must start with HKLM\\ or HKCU\\");
                    }
                    Check::Registry { key: key.to_string() }
                }
                "windows" => {
                    let Some(build) = section.build.filter(|build| (10_000..=99_999).contains(build)) else {
                        bail!(
                            "{at}.build is missing or isn't a Windows build number; use 10240 (Windows 10), \
                             17763 (1809), 19041 (2004) or 22000 (Windows 11)"
                        );
                    };
                    Check::Windows { build }
                }
                other => bail!(
                    "{at}.kind is \"{other}\"; use \"webview2\", \"dotnet\", \"vcredist\", \"file\", \"registry\" or \"windows\""
                ),
            };
            let defaults = check.defaults();
            let name = section.name.clone().or_else(|| match &check {
                Check::Windows { build } => Some(format!("Windows (build {build} or newer)")),
                _ => defaults.map(|(name, _)| name.to_string()),
            });
            let Some(name) = name.filter(|name| !name.trim().is_empty()) else {
                bail!("{at}.name is missing; say what it is, like \"Hello Runtime\"");
            };
            let url = section.url.clone().or_else(|| match &check {
                Check::Windows { .. } => Some("https://www.microsoft.com/windows".to_string()),
                _ => defaults.map(|(_, url)| url.to_string()),
            });
            let Some(url) = url.filter(|url| is_https(url)) else {
                bail!("{at}.url must be an https link where people can get {name}");
            };
            Ok(Requirement { check, name: name.trim().to_string(), url })
        })
        .collect()
}

pub fn shortcuts(sections: &[ShortcutSection], files: &[FileEntry]) -> Result<Vec<Shortcut>> {
    sections
        .iter()
        .enumerate()
        .map(|(index, section)| {
            let at = format!("shortcuts[{index}]");
            if !is_valid_shortcut_name(&section.name) {
                bail!("{at}.name is \"{}\"; it names a file, so leave out < > : \" / \\ | ? *", section.name);
            }
            let target = paths::normalize(section.target.trim()).map_err(|err| anyhow::anyhow!("{at}.target: {err}"))?;
            if !files.iter().any(|file| file.path.eq_ignore_ascii_case(&target)) {
                bail!("{at}.target is \"{target}\", but no installed file has that path");
            }
            Ok(Shortcut {
                name: section.name.trim().to_string(),
                target,
                arguments: super::optional(&section.arguments),
                description: super::optional(&section.description),
            })
        })
        .collect()
}

pub fn run_steps(sections: &[RunSection], files: &[FileEntry], components: &[Component]) -> Result<Vec<RunStep>> {
    sections
        .iter()
        .enumerate()
        .map(|(index, section)| {
            let at = format!("run[{index}]");
            let file = paths::normalize(section.file.trim()).map_err(|err| anyhow::anyhow!("{at}.file: {err}"))?;
            if !file.to_ascii_lowercase().ends_with(".exe") {
                bail!("{at}.file is \"{file}\"; it must be an .exe");
            }
            let Some(entry) = files.iter().find(|entry| entry.path.eq_ignore_ascii_case(&file)) else {
                bail!("{at}.file is \"{file}\", but no installed file has that path (is it excluded?)");
            };
            let name = match section.name.as_deref().map(str::trim) {
                Some(name) if !name.is_empty() => name.to_string(),
                _ => file.rsplit('/').next().unwrap_or(&file).trim_end_matches(".exe").trim_end_matches(".EXE").to_string(),
            };
            if name.len() > 100 || name.chars().any(char::is_control) {
                bail!("{at}.name is too long or has control characters in it");
            }
            let when = match section.when.as_deref() {
                None => RunWhen::Install,
                Some(word) => RunWhen::parse(word)
                    .ok_or_else(|| anyhow::anyhow!("{at}.when is \"{word}\"; use \"install\", \"first_install\" or \"uninstall\""))?,
            };
            if section.ignore_errors && when == RunWhen::Uninstall {
                bail!("{at}.ignore_errors has no effect with when = \"uninstall\": removing never stops for a failed step");
            }
            let mut success_codes = section.success_codes.clone().unwrap_or_else(|| vec![0]);
            if success_codes.is_empty() {
                bail!("{at}.success_codes is empty; list the exit codes that mean it worked, like [0, 3010]");
            }
            if !success_codes.contains(&0) {
                success_codes.insert(0, 0);
            }
            let mut unique = Vec::new();
            for code in success_codes {
                if !unique.contains(&code) {
                    unique.push(code);
                }
            }
            let success_codes = unique;
            let arguments = super::optional(&section.arguments);
            if arguments.as_deref().is_some_and(|arguments| arguments.len() > 2000 || arguments.contains('\0')) {
                bail!("{at}.arguments is too long or has a NUL in it");
            }
            let component = match section.component.as_deref().map(str::trim) {
                None | Some("") => entry.component.clone(),
                Some(id) if components.iter().any(|known| known.id == id) => Some(id.to_string()),
                Some(id) => bail!("{at}.component is \"{id}\", but no [[components]] has that id"),
            };
            Ok(RunStep { name, file, arguments, when, success_codes, ignore_errors: section.ignore_errors, component })
        })
        .collect()
}

/// The feed and the download link template for the JSON `bach build` writes.
///
/// With `github = "owner/repo"` both have a default that fits GitHub
/// releases made by the Bach action: the feed is the latest release's
/// `latest.json`, and each version's setup file is downloaded from that
/// version's own tag (`v{version}`), so a link never changes what it serves.
pub fn updates(section: Option<&UpdatesSection>) -> Result<Option<UpdateSettings>> {
    let Some(section) = section else {
        return Ok(None);
    };
    let github = match section.github.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(repo) if is_github_repo(repo) => Some(repo.to_string()),
        Some(repo) => bail!("updates.github is \"{repo}\"; use the repository as owner/name, like \"usebach/bach\""),
    };
    let feed = match (section.feed.as_deref().map(str::trim), &github) {
        (Some(feed), _) if !feed.is_empty() => {
            if !is_https(feed) {
                bail!("updates.feed is \"{feed}\"; it must be an https link");
            }
            feed.to_string()
        }
        (_, Some(repo)) => format!("https://github.com/{repo}/releases/latest/download/latest.json"),
        _ => bail!("[updates] needs github = \"owner/name\" or feed = \"https://…/latest.json\""),
    };
    let download = match (section.download_url.as_deref().map(str::trim), &github) {
        (Some(download), _) if !download.is_empty() => {
            if !is_https(download) {
                bail!("updates.download_url is \"{download}\"; it must be an https link");
            }
            Some(download.to_string())
        }
        (_, Some(repo)) => Some(format!("https://github.com/{repo}/releases/download/v{{version}}/{{file}}")),
        _ => None,
    };
    let auto = super::options::choice(section.auto.as_ref(), "updates.auto", Choice::On)?;
    let mut mirrors = Vec::new();
    for mirror in &section.mirrors {
        let mirror = mirror.trim();
        if !is_https(mirror) {
            bail!("updates.mirrors has \"{mirror}\"; every mirror must be the https link of a feed");
        }
        if mirror == feed || mirrors.iter().any(|known| known == mirror) {
            bail!("updates.mirrors lists \"{mirror}\" twice (or the feed itself)");
        }
        mirrors.push(mirror.to_string());
    }
    Ok(Some(UpdateSettings { updates: Updates { feed, auto, mirrors }, download_url: download, github }))
}

/// `[access]`: an access key the installer asks for.
pub fn access(section: Option<&AccessSection>, has_updates: bool) -> Result<Option<AccessSettings>> {
    let Some(section) = section else {
        return Ok(None);
    };
    let encrypt = section.encrypt.unwrap_or(true);
    let send_key = section.send_key.unwrap_or(has_updates);
    if send_key && !has_updates {
        bail!("access.send_key is true, but there's no [updates] to send the key to");
    }
    if !encrypt && !send_key {
        bail!(
            "[access] does nothing: leave encrypt on, or turn send_key on (with [updates]) so a private \
             server can check the key"
        );
    }
    let key_env = match section.key_env.as_deref().map(str::trim) {
        None | Some("") => DEFAULT_KEY_ENV.to_string(),
        Some(name) if is_env_name(name) => name.to_string(),
        Some(name) => bail!("access.key_env is \"{name}\"; use an environment variable's name, like \"{DEFAULT_KEY_ENV}\""),
    };
    let get_key_url = https(&section.get_key_url, "access.get_key_url")?;
    Ok(Some(AccessSettings {
        manifest: Access { get_key_url, send_key, encryption: None },
        encrypt,
        key_env,
    }))
}

fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `owner/name`, as GitHub allows them.
fn is_github_repo(text: &str) -> bool {
    let valid = |part: &str| {
        !part.is_empty()
            && part.len() <= 100
            && part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            && part != "."
            && part != ".."
    };
    text.split_once('/').is_some_and(|(owner, name)| valid(owner) && valid(name))
}

pub fn https(value: &Option<String>, key: &str) -> Result<Option<String>> {
    match value.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(url) if is_https(url) => Ok(Some(url.to_string())),
        Some(url) => bail!("{key} is \"{url}\"; it must be an https link"),
    }
}

/// A PNG's width divided by its height, read from its header.
pub fn png_aspect(png: &[u8]) -> Option<f32> {
    let width = u32::from_be_bytes(png.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(png.get(20..24)?.try_into().ok()?);
    (width > 0 && height > 0).then(|| width as f32 / height as f32)
}
