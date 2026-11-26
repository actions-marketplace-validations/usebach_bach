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

/// Checks the components and marks each file with the first component whose
/// patterns match it.
pub(super) fn components(
    sections: &[ComponentSection],
    files: &mut [FileEntry],
    app: &manifest::App,
    languages: &[String],
    downloads: &Downloads,
) -> Result<Vec<Component>> {
    let builtin = [OPTION_DESKTOP, OPTION_START_MENU, OPTION_AUTOSTART, OPTION_PATH, OPTION_AUTO_UPDATE];
    let mut seen = HashSet::new();
    let mut components = Vec::new();
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
        let mut matched = 0usize;
        for file in files.iter_mut().filter(|file| file.component.is_none()) {
            if section.files.iter().any(|pattern| paths::matches(pattern, &file.path)) {
                if file.path.eq_ignore_ascii_case(&app.executable) {
                    bail!("{at}.files includes app.executable ({}), which must always be installed", app.executable);
                }
                file.component = Some(id.to_string());
                matched += 1;
            }
        }
        if matched == 0 {
            bail!("{at}.files matches no files (that another component hasn't claimed first)");
        }
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
    // Until the archive is built its size is about the files'; `bach preview` and `bach check` show that.
    let size = files.iter().filter(|file| file.component.as_deref() == Some(id)).fold(0u64, |sum, file| sum.saturating_add(file.size));
    Ok(Some(manifest::ComponentDownload { url, sha256: "0".repeat(64), size }))
}
