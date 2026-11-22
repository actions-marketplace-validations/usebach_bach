//! `[macos]`: the macOS setup file of the app, built next to the Windows one
//! from the same `bach.toml`. It names the macOS files and what differs;
//! everything else (name, version, texts, look, choices, updates) is shared.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::manifest::Platform;
use crate::paths;

/// The default place the macOS setup file goes.
pub const DEFAULT_OUTPUT: &str = "dist/{name}-Setup-{version}.dmg";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MacosSection {
    /// The folder with the macOS files: usually the app bundle (`Hello.app`) and anything beside it.
    pub source: String,
    /// The program to start: the `.app` bundle (or a program) inside `source`.
    pub executable: Option<String>,
    /// `files.exclude` for this folder; the main `files.exclude` when left out.
    pub exclude: Option<Vec<String>>,
    /// Where the setup file goes; must end in `.dmg` or `.zip`.
    pub output: Option<String>,
    /// Folders like `~/Library/Application Support/Hello` that the uninstaller deletes on request.
    #[serde(default)]
    pub data: Vec<String>,
    /// Patterns (like `Hello.app/Contents/MacOS/*`) of files that must be programs. Needed when
    /// the folder comes from a computer that doesn't keep the executable bit (a Windows
    /// checkout, a zip); on a Mac or Linux folder the permissions already say it.
    #[serde(default)]
    pub executables: Vec<String>,
    /// A `.icns` for the setup app and the uninstaller.
    pub icon: Option<String>,
    /// The setup app's bundle identifier, like `com.example.hello.setup`.
    pub bundle_id: Option<String>,
    /// The oldest macOS the app runs on, like `"12.0"`.
    pub minimum_version: Option<String>,
    pub signing: Option<SigningSection>,
    /// Background programs of the app, run by launchd (`[[macos.agents]]`): the Mac's counterpart of
    /// `[[services]]`.
    #[serde(default)]
    pub agents: Vec<AgentSection>,
    /// Programs setup runs for the macOS build (`[[macos.run]]`), the way `[[run]]`
    /// does for Windows: files of the app that have the executable bit set.
    #[serde(default)]
    pub run: Vec<super::extras::RunSection>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AgentSection {
    pub name: String,
    pub file: Option<String>,
    pub arguments: Option<String>,
    pub run_at_load: Option<bool>,
    pub keep_alive: Option<bool>,
    pub interval: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SigningSection {
    /// A code signing identity, like `Developer ID Application: Example (TEAMID)`; `-` signs ad hoc.
    pub identity: Option<String>,
    /// A `notarytool` keychain profile; the build submits and staples when it is set.
    pub notarize: Option<String>,
    /// A "Developer ID Installer" identity, which signs the `.pkg` (`bach build --pkg`).
    pub installer_identity: Option<String>,
}

/// How `bach build` signs the macOS setup file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MacSigning {
    pub identity: String,
    pub notary_profile: Option<String>,
    /// The identity that signs the `.pkg`; the `.pkg` is built unsigned without it.
    pub installer_identity: Option<String>,
}

/// What the macOS build needs besides the manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MacBuild {
    /// The `.icns` for the setup app.
    pub icon: Option<PathBuf>,
    /// The setup app's bundle identifier.
    pub bundle_id: String,
    pub minimum_version: Option<String>,
    pub signing: Option<MacSigning>,
    /// `dmg` or `zip`.
    pub container: Container,
}

/// What the setup app is delivered in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    Dmg,
    Zip,
}

impl Container {
    pub fn extension(self) -> &'static str {
        match self {
            Container::Dmg => "dmg",
            Container::Zip => "zip",
        }
    }
}

/// The platform name from `--platform`, checked.
pub(super) fn platform(name: Option<&str>) -> Result<Platform> {
    match name.map(str::trim) {
        None | Some("") | Some("windows") => Ok(Platform::Windows),
        Some("macos") => Ok(Platform::Macos),
        Some(other) => bail!("the platform is \"{other}\"; use \"windows\" or \"macos\""),
    }
}

/// A bundle identifier made from the app id when the config gives none: letters,
/// digits, dots and dashes only.
pub(super) fn default_bundle_id(app_id: &str) -> String {
    let part: String = app_id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    let part = if part.is_empty() { "app".to_string() } else { part.to_lowercase() };
    format!("app.bach.setup.{part}")
}

pub(super) fn is_valid_bundle_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 155
        && id.split('.').count() >= 2
        && id.split('.').all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

/// `"12"`, `"12.0"` and `"13.4.1"` are fine; the first two parts matter to macOS.
pub(super) fn is_valid_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    (1..=3).contains(&parts.len())
        && parts.iter().all(|part| !part.is_empty() && part.len() <= 4 && part.chars().all(|c| c.is_ascii_digit()))
}

/// Whether the macOS files are one app bundle and nothing else (`Hello.app` and, at most, Finder's
/// `.DS_Store`): such an app goes straight into Applications.
pub(super) fn is_one_bundle(source: &Path, executable: &str) -> bool {
    if !executable.to_lowercase().ends_with(".app") || executable.contains('/') {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(source) else {
        return false;
    };
    let names: Vec<String> =
        entries.flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).filter(|name| name != ".DS_Store").collect();
    names.len() == 1 && names[0] == executable
}

/// The checks of `[macos]` that need only the section itself.
pub(super) fn check(section: &MacosSection, root: &Path, app_id: &str) -> Result<MacBuild> {
    let folder = root.join(&section.source);
    if !folder.is_dir() {
        bail!("macos.source is \"{}\", but {} isn't a folder", section.source, folder.display());
    }
    for data in &section.data {
        paths::mac::check_data_folder(data).context("macos.data")?;
    }
    let icon = match section.icon.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(icon) => {
            let path = root.join(icon);
            if !path.is_file() {
                bail!("macos.icon is \"{icon}\", but {} isn't a file", path.display());
            }
            if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("icns")) {
                bail!("macos.icon must be a .icns file, so macOS can show it; {icon} isn't");
            }
            Some(path)
        }
    };
    let bundle_id = match section.bundle_id.as_deref().map(str::trim) {
        None | Some("") => default_bundle_id(app_id),
        Some(id) if is_valid_bundle_id(id) => id.to_string(),
        Some(id) => bail!("macos.bundle_id is \"{id}\"; use a reverse domain name like com.example.hello.setup"),
    };
    let minimum_version = match section.minimum_version.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(version) if is_valid_version(version) => Some(version.to_string()),
        Some(version) => bail!("macos.minimum_version is \"{version}\"; use a version like \"12.0\""),
    };
    let signing = match &section.signing {
        None => None,
        Some(signing) => {
            let identity = signing.identity.as_deref().map(str::trim).unwrap_or("-");
            if identity.is_empty() || identity.chars().any(char::is_control) {
                bail!("macos.signing.identity is empty or has control characters in it; use \"-\" to sign ad hoc");
            }
            let notary_profile = signing.notarize.as_deref().map(str::trim).filter(|profile| !profile.is_empty()).map(str::to_string);
            if notary_profile.is_some() && identity == "-" {
                bail!("macos.signing.notarize needs a Developer ID identity; an ad hoc signature (\"-\") can't be notarized");
            }
            let installer_identity =
                signing.installer_identity.as_deref().map(str::trim).filter(|name| !name.is_empty()).map(str::to_string);
            if installer_identity.as_deref().is_some_and(|name| name.chars().any(char::is_control)) {
                bail!("macos.signing.installer_identity has control characters in it");
            }
            Some(MacSigning { identity: identity.to_string(), notary_profile, installer_identity })
        }
    };
    let container = match section.output.as_deref().map(|output| Path::new(output.trim()).extension().map(|ext| ext.to_ascii_lowercase())) {
        None => Container::Dmg,
        Some(Some(ext)) if ext == "dmg" => Container::Dmg,
        Some(Some(ext)) if ext == "zip" => Container::Zip,
        Some(_) => bail!("macos.output must end in .dmg or .zip; it's \"{}\"", section.output.as_deref().unwrap_or_default()),
    };
    Ok(MacBuild { icon, bundle_id, minimum_version, signing, container })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_ids_and_versions_are_checked() {
        assert!(is_valid_bundle_id("com.example.hello-setup"));
        assert!(!is_valid_bundle_id("hello"));
        assert!(!is_valid_bundle_id("com..x"));
        assert!(!is_valid_bundle_id("com.exa mple.x"));
        assert!(is_valid_version("12") && is_valid_version("12.0") && is_valid_version("13.4.1"));
        assert!(!is_valid_version("12.") && !is_valid_version("v12") && !is_valid_version("1.2.3.4"));
    }

    #[test]
    fn the_default_bundle_id_is_made_of_the_app_id() {
        assert_eq!(default_bundle_id("My App (beta)"), "app.bach.setup.myappbeta");
        assert_eq!(default_bundle_id("***"), "app.bach.setup.app");
        assert!(is_valid_bundle_id(&default_bundle_id("Hello")));
    }

    #[test]
    fn the_platform_names_are_checked() {
        assert_eq!(platform(None).unwrap(), Platform::Windows);
        assert_eq!(platform(Some("macos")).unwrap(), Platform::Macos);
        assert!(platform(Some("linux")).is_err());
    }
}
