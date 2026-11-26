//! `[[components]]`: the optional parts of the app, which files each of them
//! claims, and where a downloaded one comes from.

use std::collections::HashSet;

use anyhow::{bail, Result};
use serde::Deserialize;

use super::text::localized;
use crate::manifest::features::is_valid_component_id;
use crate::manifest::{self, Component, FileEntry, OPTION_AUTOSTART, OPTION_AUTO_UPDATE, OPTION_DESKTOP, OPTION_PATH, OPTION_START_MENU};
use crate::paths;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentSection {
    id: String,
    title: toml::Value,
    description: Option<toml::Value>,
    files: Vec<String>,
    #[serde(default)]
    default: bool,
    /// `true` (the app's `[updates]` download link) or an https link: the files
    /// aren't packed into the setup file but downloaded when the component is ticked.
    download: Option<toml::Value>,
}

/// Where a downloaded component's archive goes and is fetched from.
pub(super) struct Downloads<'a> {
    /// `[updates]` download link, with `{file}` still in it.
    pub update_link: Option<&'a str>,
    /// The profile and architecture being built, so each build's archives have names of their own.
    pub flavour: &'a [&'a str],
    /// Whether the files are locked with `[access]`.
    pub encrypted: bool,
}

/// `installer.shared_files`: what a file needs when the patterns of more than
/// one component match it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SharedFiles {
    /// It belongs to the first of them; the later ones don't count.
    First,
    /// It belongs to all of them, and is installed only when all are ticked.
    All,
}

impl SharedFiles {
    pub(super) fn parse(value: Option<&str>) -> Result<Self> {
        match value {
            None | Some("first") => Ok(Self::First),
            Some("all") => Ok(Self::All),
            Some(other) => bail!(
                "installer.shared_files is \"{other}\"; use \"first\" (a file belongs to the first component that matches it) \
                 or \"all\" (it is installed only when every component that matches it is ticked)"
            ),
        }
    }
}

/// Checks the components and marks each file with the components whose
/// patterns match it: the first one, or all of them (`shared_files`).
pub(super) fn components(
    sections: &[ComponentSection],
    shared_files: SharedFiles,
    files: &mut [FileEntry],
    app: &manifest::App,
    languages: &[String],
    downloads: &Downloads,
) -> Result<Vec<Component>> {
    let builtin = [OPTION_DESKTOP, OPTION_START_MENU, OPTION_AUTOSTART, OPTION_PATH, OPTION_AUTO_UPDATE];
    let mut seen = HashSet::new();
    for (index, section) in sections.iter().enumerate() {
        let at = format!("components[{index}]");
        let id = section.id.trim();
        if !is_valid_component_id(id) {
            bail!("{at}.id is \"{id}\"; use lowercase letters, digits, - and _ (like \"samples\")");
        }
        if builtin.contains(&id) {
            bail!("{at}.id is \"{id}\", which Bach uses for its own option; pick another");
        }
        if !seen.insert(id.to_string()) {
            bail!("{at}.id \"{id}\" is used by another component too");
        }
        if section.files.is_empty() || section.files.iter().any(|pattern| pattern.trim().is_empty()) {
            bail!("{at}.files needs at least one pattern, like \"samples/**\"");
        }
        claim(section, &at, id, shared_files, files, app)?;
    }
    // A shared file travels in one place only. When a component that claims it
    // is downloaded, that is its download, so the setup file stays small.
    let downloaded: HashSet<&str> =
        sections.iter().filter(|section| is_downloaded(section.download.as_ref())).map(|section| section.id.trim()).collect();
    for file in files.iter_mut() {
        let Some(first) = file.component.as_mut().filter(|first| !downloaded.contains(first.as_str())) else { continue };
        if let Some(other) = file.also_needs.iter_mut().find(|other| downloaded.contains(other.as_str())) {
            std::mem::swap(first, other);
        }
    }
    let mut components = Vec::new();
    for (index, section) in sections.iter().enumerate() {
        let at = format!("components[{index}]");
        let id = section.id.trim();
        let download = component_download(section.download.as_ref(), &at, id, app, files, downloads)?;
        components.push(Component {
            id: id.to_string(),
            title: localized(&section.title, &format!("{at}.title"), languages)?,
            description: section.description.as_ref().map(|value| localized(value, &format!("{at}.description"), languages)).transpose()?,
            default: section.default,
            download,
        });
    }
    Ok(components)
}

/// Marks the files `section` matches as `id`'s.
fn claim(
    section: &ComponentSection,
    at: &str,
    id: &str,
    shared_files: SharedFiles,
    files: &mut [FileEntry],
    app: &manifest::App,
) -> Result<()> {
    let mut matched = 0usize;
    let mut taken = false;
    for file in files.iter_mut() {
        if !section.files.iter().any(|pattern| paths::matches(pattern, &file.path)) {
            continue;
        }
        if file.path.eq_ignore_ascii_case(&app.executable) {
            bail!("{at}.files includes app.executable ({}), which must always be installed", app.executable);
        }
        if file.component.is_none() {
            file.component = Some(id.to_string());
        } else if shared_files == SharedFiles::All {
            file.also_needs.push(id.to_string());
        } else {
            taken = true;
            continue;
        }
        matched += 1;
    }
    if matched == 0 {
        match shared_files {
            SharedFiles::All => bail!("{at}.files matches no files"),
            SharedFiles::First if taken => bail!(
                "{at}.files matches no files that another component hasn't claimed first; to install those only when \
                 both are ticked, set installer.shared_files = \"all\""
            ),
            SharedFiles::First => bail!("{at}.files matches no files (that another component hasn't claimed first)"),
        }
    }
    Ok(())
}

/// Whether `download` asks for one (`true` or a link).
fn is_downloaded(value: Option<&toml::Value>) -> bool {
    !matches!(value, None | Some(toml::Value::Boolean(false)))
}

/// The name of the archive that holds a downloaded component's files:
/// `Hello-hd-textures-1.2.0.bachpack`. A profile or an architecture adds its
/// name (`Hello-hd-textures-1.2.0-arm64.bachpack`), because its files may differ.
/// A file name GitHub keeps as it is.
pub fn component_pack_name(app: &manifest::App, component: &str, flavour: &[&str]) -> String {
    let flavour: String = flavour.iter().map(|part| format!("-{part}")).collect();
    super::github_asset_name(&super::fill_template(&format!("{{name}}-{component}-{{version}}{flavour}.bachpack"), app))
}

/// What `download` in a component says: nothing, the app's own download link, or a link.
/// The archive's SHA-256 isn't known until it is built; the builder fills it in.
fn component_download(
    value: Option<&toml::Value>,
    at: &str,
    id: &str,
    app: &manifest::App,
    files: &[FileEntry],
    downloads: &Downloads,
) -> Result<Option<manifest::ComponentDownload>> {
    let pack = component_pack_name(app, id, downloads.flavour);
    let template = match value {
        None | Some(toml::Value::Boolean(false)) => return Ok(None),
        Some(toml::Value::Boolean(true)) => match downloads.update_link {
            Some(link) => link.to_string(),
            None => bail!(
                "{at}.download is true, which uses the download link of [updates]; add github = \"owner/repo\" or download_url there, or give the link here"
            ),
        },
        Some(toml::Value::String(link)) => link.trim().to_string(),
        Some(_) => bail!("{at}.download is true or an https link"),
    };
    let url = template.replace("{file}", &pack).replace("{version}", &app.version).replace("{id}", &app.id).replace("{component}", id);
    if !manifest::extras::is_https(&url) || url.chars().any(char::is_control) {
        bail!("{at}.download is \"{url}\", but it must be an https link");
    }
    if downloads.encrypted {
        bail!("{at}.download can't be used with [access] encrypt: the downloaded files would not be locked. Pack the files into the setup file, or turn encrypt off");
    }
    let carried: Vec<&FileEntry> = files.iter().filter(|file| file.component.as_deref() == Some(id)).collect();
    if carried.is_empty() {
        bail!("{at}.download has nothing to download: other downloads already carry all of its files, so leave download out here");
    }
    // Until the archive is built its size is about the files'; `bach preview` and `bach check` show that.
    let size = carried.iter().fold(0u64, |sum, file| sum.saturating_add(file.size));
    Ok(Some(manifest::ComponentDownload { url, sha256: "0".repeat(64), size }))
}
