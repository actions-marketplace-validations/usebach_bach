//! Checking a manifest read from a setup file, which is untrusted input.

use std::collections::HashSet;

use anyhow::{bail, Context, Result};

use super::extras::{is_https, is_valid_shortcut_name};
use super::features::{is_valid_component_id, is_valid_extension, is_valid_scheme};
use super::{Manifest, OPTION_AUTOSTART, OPTION_DESKTOP, OPTION_PATH, OPTION_START_MENU};
use crate::{i18n, paths};

/// A license longer than this is damage, not a license.
pub const MAX_LICENSE: usize = 1 << 20;

pub fn manifest(manifest: &Manifest) -> Result<()> {
    if manifest.format != crate::FORMAT_VERSION {
        bail!("this setup file uses payload format {}, but this runtime reads format {}", manifest.format, crate::FORMAT_VERSION);
    }
    if let Some(problem) = paths::id_problem(&manifest.app.id) {
        bail!("the app id \"{}\" can't name a folder: it {problem}", manifest.app.id);
    }
    // An app bundle that goes straight into Applications has no folder of its own.
    let bare = manifest.platform.is_macos() && manifest.install.folder.is_empty();
    if let Some(problem) = paths::id_problem(&manifest.install.folder).filter(|_| !bare) {
        bail!("the install folder \"{}\" isn't a plain folder name: it {problem}", manifest.install.folder);
    }
    if manifest.app.name.trim().is_empty() || manifest.app.version.trim().is_empty() {
        bail!("the app has no name or version");
    }
    paths::normalize(&manifest.app.executable).context("the app's executable")?;
    if let Some(folder) = &manifest.install.path_folder {
        if &paths::normalize(folder)? != folder {
            bail!("the PATH folder \"{folder}\" isn't stored in its canonical form");
        }
    }
    files(manifest)?;
    components(manifest)?;
    for protocol in &manifest.protocols {
        if !is_valid_scheme(&protocol.scheme) {
            bail!("\"{}\" can't be registered as a link scheme", protocol.scheme);
        }
    }
    for file_type in &manifest.file_types {
        if !is_valid_extension(&file_type.extension) {
            bail!("\"{}\" can't be registered as a file type", file_type.extension);
        }
    }
    for folder in &manifest.uninstall.data {
        match manifest.platform {
            super::Platform::Windows => paths::check_data_folder(folder)?,
            super::Platform::Macos => paths::mac::check_data_folder(folder)?,
        }
    }
    extras(manifest)?;
    super::system::check(&manifest.system)?;
    locale(manifest)
}

fn extras(manifest: &Manifest) -> Result<()> {
    for requirement in &manifest.requirements {
        if !is_https(&requirement.url) || requirement.name.trim().is_empty() {
            bail!("the requirement \"{}\" has no name or no https link", requirement.name);
        }
    }
    for shortcut in &manifest.shortcuts {
        if !is_valid_shortcut_name(&shortcut.name) {
            bail!("\"{}\" can't name a shortcut", shortcut.name);
        }
        if paths::normalize(&shortcut.target)? != shortcut.target {
            bail!("the shortcut \"{}\" points at \"{}\", which isn't a canonical path", shortcut.name, shortcut.target);
        }
    }
    if let Some(folder) = &manifest.install.start_menu_folder {
        if !is_valid_shortcut_name(folder) {
            bail!("\"{folder}\" can't name a Start menu folder");
        }
    }
    if let Some(updates) = &manifest.updates {
        if !is_https(&updates.feed) {
            bail!("the update feed \"{}\" isn't an https link", updates.feed);
        }
        if let Some(key) = &updates.public_key {
            crate::feedsig::check_public(key).map_err(|err| anyhow::anyhow!("the update public key: {err:#}"))?;
        }
        if let Some(key) = &updates.next_public_key {
            crate::feedsig::check_public(key).map_err(|err| anyhow::anyhow!("the next update public key: {err:#}"))?;
        }
        if let Some(mirror) = updates.mirrors.iter().find(|mirror| !is_https(mirror)) {
            bail!("the update mirror \"{mirror}\" isn't an https link");
        }
        if manifest.install.scope.is_machine() && updates.auto != super::Choice::Never {
            bail!("a machine-wide install can't update itself in the background");
        }
    }
    for step in &manifest.run {
        if step.name.trim().is_empty() || step.name.len() > 100 || step.name.chars().any(char::is_control) {
            bail!("a step that runs a program has no usable name");
        }
        if paths::normalize(&step.file)? != step.file {
            bail!("the step \"{}\" runs \"{}\", which isn't a canonical path", step.name, step.file);
        }
        if !step.file.to_ascii_lowercase().ends_with(".exe") {
            bail!("the step \"{}\" runs \"{}\", which isn't an .exe", step.name, step.file);
        }
        if step.arguments.as_deref().is_some_and(|arguments| arguments.len() > 2000 || arguments.contains('\0')) {
            bail!("the arguments of the step \"{}\" are damaged", step.name);
        }
        if step.timeout_minutes.is_some_and(|minutes| minutes == 0 || minutes > super::extras::MAX_TIMEOUT_MINUTES) {
            bail!("the step \"{}\" has a time limit outside 1 to {} minutes", step.name, super::extras::MAX_TIMEOUT_MINUTES);
        }
        if step.success_codes.is_empty() || step.success_codes.len() > 32 {
            bail!("the step \"{}\" has no exit codes that mean success", step.name);
        }
        if let Some(component) = &step.component {
            if !manifest.components.iter().any(|known| &known.id == component) {
                bail!("the step \"{}\" belongs to a component, \"{component}\", that doesn't exist", step.name);
            }
        }
    }
    if let Some(access) = &manifest.access {
        if let Some(url) = &access.get_key_url {
            if !is_https(url) {
                bail!("the link for getting a key \"{url}\" isn't an https link");
            }
        }
        if access.send_key && manifest.updates.is_none() {
            bail!("the app sends its access key with updates, but has no update feed");
        }
        if let Some(encryption) = &access.encryption {
            encryption.validate()?;
        }
    }
    if manifest.online && manifest.updates.is_none() {
        bail!("this online installer has no update feed to download the app from");
    }
    if let Some(url) = &manifest.uninstall.feedback_url {
        if !is_https(url) {
            bail!("the feedback link \"{url}\" isn't an https link");
        }
    }
    if let Some(hash) = &manifest.archive_sha256 {
        if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("the setup file's checksum is damaged");
        }
    }
    if !manifest.agents.is_empty() && !manifest.platform.is_macos() {
        bail!("only a macOS setup file can have launch agents");
    }
    let mut names = HashSet::new();
    for agent in &manifest.agents {
        if agent.name.is_empty() || agent.name.len() > 40 || !agent.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            bail!("the agent \"{}\" has a name with something other than letters, digits and dashes", agent.name);
        }
        if !names.insert(agent.name.to_lowercase()) {
            bail!("the agent \"{}\" is listed twice", agent.name);
        }
        let Some(file) = manifest.files.iter().find(|file| file.path == agent.file && !file.is_link()) else {
            bail!("the agent \"{}\" runs \"{}\", which isn't one of the installed files", agent.name, agent.file);
        };
        if file.mode.unwrap_or(0) & 0o111 == 0 {
            bail!("the agent \"{}\" runs \"{}\", which has no executable permission", agent.name, agent.file);
        }
        if agent.interval.is_some_and(|seconds| seconds < 10) {
            bail!("the agent \"{}\" runs every less than 10 seconds", agent.name);
        }
    }
    for file in &manifest.files {
        if file.sha256.as_deref().is_some_and(|hash| hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit())) {
            bail!("the checksum of \"{}\" is damaged", file.path);
        }
    }
    if manifest.patch_from.is_some() {
        if manifest.online {
            bail!("an online installer can't be an update patch");
        }
        if manifest.files.iter().any(|file| file.sha256.is_none()) {
            bail!("an update patch has to know every file's checksum");
        }
    }
    Ok(())
}

fn files(manifest: &Manifest) -> Result<()> {
    let mut seen = HashSet::new();
    for file in &manifest.files {
        let normal = paths::normalize(&file.path)?;
        if normal != file.path {
            bail!("\"{}\" isn't stored in its canonical form", file.path);
        }
        let uninstaller = manifest.uninstaller();
        if normal.eq_ignore_ascii_case(&uninstaller) || normal.to_lowercase().starts_with(&format!("{}/", uninstaller.to_lowercase())) {
            bail!("the app ships its own {uninstaller}, which would replace Bach's uninstaller");
        }
        if manifest.platform.is_windows() && (file.mode.is_some() || file.link.is_some()) {
            bail!("\"{}\" has a unix mode or is a link, which only a macOS setup file can install", file.path);
        }
        if let Some(mode) = file.mode {
            if mode > 0o777 {
                bail!("\"{}\" has a mode ({mode:o}) with more than the read, write and execute bits", file.path);
            }
        }
        if let Some(target) = &file.link {
            if file.size != 0 {
                bail!("the link \"{}\" carries bytes", file.path);
            }
            paths::check_link(&file.path, target)?;
        }
        if !seen.insert(paths::fold_case(&normal)) {
            bail!("\"{}\" is listed twice", file.path);
        }
        if let Some(component) = &file.component {
            if !manifest.components.iter().any(|known| &known.id == component) {
                bail!("\"{}\" belongs to a component, \"{component}\", that doesn't exist", file.path);
            }
        }
    }
    Ok(())
}

fn components(manifest: &Manifest) -> Result<()> {
    let builtin = [OPTION_DESKTOP, OPTION_START_MENU, OPTION_AUTOSTART, OPTION_PATH];
    let mut seen = HashSet::new();
    for component in &manifest.components {
        if !is_valid_component_id(&component.id) || builtin.contains(&component.id.as_str()) {
            bail!("\"{}\" can't name a component", component.id);
        }
        if !seen.insert(component.id.as_str()) {
            bail!("the component \"{}\" is listed twice", component.id);
        }
        if component.title.is_empty() {
            bail!("the component \"{}\" has no title", component.id);
        }
        if let Some(download) = &component.download {
            let plain = !download.url.chars().any(char::is_control);
            if !is_https(&download.url) || !plain {
                bail!("the download of the component \"{}\" isn't an https link", component.id);
            }
            if download.sha256.len() != 64 || !download.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
                bail!("the download of the component \"{}\" has no valid SHA-256", component.id);
            }
            // The files are checked against the archive's own SHA-256 as they are read,
            // and a download isn't locked with the access key: it would be readable.
            if manifest.access.as_ref().is_some_and(|access| access.encryption.is_some()) {
                bail!("the component \"{}\" is downloaded, which a setup file locked with an access key can't do", component.id);
            }
        }
    }
    Ok(())
}

fn locale(manifest: &Manifest) -> Result<()> {
    for code in &manifest.locale.languages {
        if i18n::language(code).is_none() {
            bail!("the setup file asks for a language Bach doesn't speak, \"{code}\"");
        }
    }
    for key in manifest.locale.text.keys() {
        if i18n::Key::parse(key).is_none() {
            bail!("the setup file replaces a text that doesn't exist, \"{key}\"");
        }
    }
    if let Some(license) = &manifest.locale.license {
        if license.values().any(|text| text.len() > MAX_LICENSE) {
            bail!("the license is longer than {} KB", MAX_LICENSE / 1024);
        }
    }
    Ok(())
}
