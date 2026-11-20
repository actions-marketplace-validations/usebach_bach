//! Where an install starts from: a fresh one, or an update over an install
//! already on this computer, with the choices it made last time. Shared by
//! the window and a silent run, so both offer the same thing.

use std::path::PathBuf;

use bach_pack::manifest::{Choices, Manifest};

use crate::system::{self, Installed};
use crate::work;

pub struct Start {
    /// The install Settings › Apps knows about.
    pub existing: Option<Installed>,
    /// That install's manifest, read from its uninstaller.
    pub previous: Option<Manifest>,
    pub dir: PathBuf,
    pub choices: Choices,
    /// The installed version is newer than this setup file's.
    pub downgrade: bool,
}

/// Why `--repair` can't go on, or None when it can: repair writes back what is
/// missing or changed of the version that is installed, so that version has to
/// be this setup file's own.
pub fn repair_problem(manifest: &Manifest, start: &Start) -> Option<String> {
    match &start.existing {
        None => Some(format!("{} isn't installed, so there is nothing to repair. Run this setup file without --repair to install it.", manifest.app.name)),
        Some(installed) if installed.version != manifest.app.version => Some(format!(
            "{} {} is installed, but this setup file is version {}. --repair needs the setup file of the installed version; run this one without --repair to update.",
            manifest.app.name, installed.version, manifest.app.version
        )),
        Some(_) => None,
    }
}

/// The requirements this computer doesn't meet yet, by index.
pub fn missing_requirements(manifest: &Manifest) -> Vec<usize> {
    manifest
        .requirements
        .iter()
        .enumerate()
        .filter(|(_, requirement)| !crate::system::apps::is_present(&requirement.check))
        .map(|(index, _)| index)
        .collect()
}

/// The start of a `.msi` package's window: the install Windows Installer knows about
/// (it keeps its own record, not the setup file's), with the options it was installed with.
fn install_from_package(manifest: &Manifest, link: &system::msi::Link) -> Start {
    let existing = system::msi::installed(link, manifest);
    let dir = existing.as_ref().map(|installed| installed.dir.clone()).unwrap_or_else(|| system::default_install_dir(manifest));
    let mut choices = Choices::defaults(manifest);
    if let Some(product) = system::msi::installed_product(link).filter(|_| existing.is_some()) {
        let on = |id: &str| system::msi::feature_is_on(&product, id);
        let install = &manifest.install;
        if install.desktop_shortcut.offered() {
            choices.desktop_shortcut = on("desktop_shortcut");
        }
        if install.start_menu_shortcut.offered() {
            choices.start_menu_shortcut = on("start_menu");
        }
        if install.start_with_windows.offered() {
            choices.start_with_windows = on("autostart");
        }
        if install.add_to_path.offered() {
            choices.add_to_path = on("path");
        }
        choices.components =
            manifest.components.iter().filter(|component| on(&system::msi::feature_id(&component.id))).map(|c| c.id.clone()).collect();
    }
    let downgrade = existing.as_ref().is_some_and(|installed| {
        bach_pack::version::compare(&installed.version, &manifest.app.version) == Some(std::cmp::Ordering::Greater)
    });
    Start { existing, previous: None, dir, choices, downgrade }
}

pub fn install(manifest: &Manifest) -> Start {
    if let Some(link) = system::msi::link() {
        return install_from_package(manifest, link);
    }
    let existing = system::installed(manifest);
    let dir = existing.as_ref().map(|installed| installed.dir.clone()).unwrap_or_else(|| system::default_install_dir(manifest));
    let previous = existing.as_ref().and_then(|_| work::previous_install(&dir, manifest));
    let choices = match &previous {
        Some(previous) => {
            let mut choices = Choices::for_update(manifest, previous);
            if previous.chosen.is_none() {
                // Bach 0.1 didn't record choices; its desktop shortcut tells.
                choices.desktop_shortcut = system::desktop_shortcut(manifest).is_ok_and(|path| path.exists());
            }
            choices
        }
        None => Choices::defaults(manifest),
    };
    // An online installer doesn't know which version it will install until
    // it has asked the feed, so there is nothing to compare yet.
    let downgrade = !manifest.online
        && existing.as_ref().is_some_and(|installed| {
            bach_pack::version::compare(&installed.version, &manifest.app.version) == Some(std::cmp::Ordering::Greater)
        });
    Start { existing, previous, dir, choices, downgrade }
}
