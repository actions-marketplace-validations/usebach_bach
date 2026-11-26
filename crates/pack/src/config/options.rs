//! `[installer]` choices, `[[protocols]]` and `[[file_types]]`.

use std::collections::HashSet;

use anyhow::{bail, Result};
use serde::Deserialize;

use super::InstallerSection;
use crate::manifest::features::{is_valid_extension, is_valid_scheme};
use crate::manifest::{self, Choice, FileType, Protocol};
use crate::paths;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolSection {
    scheme: String,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileTypeSection {
    extension: String,
    description: Option<String>,
}

/// `bare_bundle`: a macOS app that is one app bundle and nothing else. It goes straight into
/// Applications (no folder of its own, so one icon and one thing to drag to the Trash), and the
/// window asks nothing but "Install" unless the config offers more.
pub(super) fn install(section: &InstallerSection, app: &manifest::App, bare_bundle: bool) -> Result<manifest::Install> {
    let folder = match &section.folder {
        Some(folder) => folder.trim().to_string(),
        None if bare_bundle => String::new(),
        None => app.id.clone(),
    };
    let bare = folder.is_empty();
    if bare && !bare_bundle {
        bail!("installer.folder is empty; it must be a plain folder name, like \"{}\"", app.id);
    }
    if let Some(problem) = paths::id_problem(&folder).filter(|_| !bare) {
        bail!("installer.folder is \"{folder}\", which {problem}. It must be a plain folder name, like \"{}\"", app.id);
    }
    let path_folder = match section.path_folder.as_deref().map(str::trim) {
        None | Some("") | Some(".") => None,
        Some(folder) => Some(paths::normalize(folder).map_err(|err| anyhow::anyhow!("installer.path_folder: {err}"))?),
    };
    Ok(manifest::Install {
        folder,
        desktop_shortcut: choice(
            section.desktop_shortcut.as_ref(),
            "installer.desktop_shortcut",
            if bare { Choice::Never } else { Choice::On },
        )?,
        start_menu_shortcut: choice(
            section.start_menu_shortcut.as_ref(),
            "installer.start_menu_shortcut",
            if bare { Choice::Never } else { Choice::Always },
        )?,
        start_with_windows: choice(section.start_with_windows.as_ref(), "installer.start_with_windows", Choice::Never)?,
        add_to_path: choice(section.add_to_path.as_ref(), "installer.add_to_path", Choice::Never)?,
        path_folder,
        change_folder: section.change_folder.unwrap_or(!bare),
        open_button: section.open_button.unwrap_or(true),
        start_menu_folder: match section.start_menu_folder.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(folder) if crate::manifest::extras::is_valid_shortcut_name(folder) => Some(folder.to_string()),
            Some(folder) => bail!("installer.start_menu_folder is \"{folder}\"; it names a folder, so leave out < > : \" / \\ | ? *"),
        },
        website_button: section.website_button.unwrap_or(false),
        scope: match section.scope.as_deref() {
            None => manifest::Scope::User,
            Some(word) => {
                manifest::Scope::parse(word).ok_or_else(|| anyhow::anyhow!("installer.scope is \"{word}\"; use \"user\" or \"machine\""))?
            }
        },
    })
}

/// `true` (offered, ticked), `false` (offered, unticked), `"always"` or
/// `"never"`.
pub(super) fn choice(value: Option<&toml::Value>, key: &str, default: Choice) -> Result<Choice> {
    let parsed = match value {
        None => return Ok(default),
        Some(toml::Value::Boolean(true)) => Some(Choice::On),
        Some(toml::Value::Boolean(false)) => Some(Choice::Off),
        Some(toml::Value::String(word)) => match word.trim().to_ascii_lowercase().as_str() {
            "always" => Some(Choice::Always),
            "never" => Some(Choice::Never),
            _ => None,
        },
        Some(_) => None,
    };
    parsed.ok_or_else(|| {
        anyhow::anyhow!(
            "{key} is {}; use true (offered, ticked), false (offered, unticked), \"always\" or \"never\"",
            value.map(|value| value.to_string()).unwrap_or_default()
        )
    })
}

pub(super) fn protocols(sections: &[ProtocolSection]) -> Result<Vec<Protocol>> {
    let mut seen = HashSet::new();
    sections
        .iter()
        .enumerate()
        .map(|(index, section)| {
            let scheme = section.scheme.trim().trim_end_matches("://").trim_end_matches(':').to_ascii_lowercase();
            if !is_valid_scheme(&scheme) {
                bail!(
                    "protocols[{index}].scheme is \"{}\"; use 2 to 32 lowercase letters, digits, +, - or ., \
                     starting with a letter, and not a scheme like http that belongs to Windows or browsers",
                    section.scheme
                );
            }
            if !seen.insert(scheme.clone()) {
                bail!("protocols[{index}].scheme \"{scheme}\" is listed twice");
            }
            Ok(Protocol { scheme, description: super::optional(&section.description) })
        })
        .collect()
}

pub(super) fn file_types(sections: &[FileTypeSection]) -> Result<Vec<FileType>> {
    let mut seen = HashSet::new();
    sections
        .iter()
        .enumerate()
        .map(|(index, section)| {
            let trimmed = section.extension.trim().to_ascii_lowercase();
            let extension = if trimmed.starts_with('.') { trimmed } else { format!(".{trimmed}") };
            if !is_valid_extension(&extension) {
                bail!(
                    "file_types[{index}].extension is \"{}\"; use one short extension like \".hello\", \
                     and not one that runs code (.exe, .bat, …)",
                    section.extension
                );
            }
            if !seen.insert(extension.clone()) {
                bail!("file_types[{index}].extension \"{extension}\" is listed twice");
            }
            Ok(FileType { extension, description: super::optional(&section.description) })
        })
        .collect()
}
