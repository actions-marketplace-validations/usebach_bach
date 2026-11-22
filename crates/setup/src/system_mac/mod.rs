//! What setup asks of macOS: where per-user programs live, the links that
//! put the app in Applications and on the Desktop, and the record that says
//! where an install is (macOS has no "Settings › Apps"). Everything is per
//! user, in the user's own folders, so setup never needs administrator
//! rights, except for `installer.scope = "machine"`: that installs into
//! `/Applications`, which a macOS administrator can write to without any
//! prompt, so it needs no elevated setup either.
//!
//! This module answers the same questions as `system/` does on Windows, with
//! the same names, so the install and removal code is shared. Windows-only
//! ideas (registry values, services, the Windows Installer, ACLs) have no
//! macOS meaning; the matching functions here do nothing, and the config
//! never asks for them on a macOS build.
//!
//! These calls block; setup runs them on its worker thread, apart from the
//! quick record read at startup.

pub mod acl;
pub mod agents;
pub mod apps;
pub mod changes;
pub mod channel;
pub mod elevate;
pub mod env_path;
pub mod folders;
pub mod guard;
pub mod host;
pub mod integration;
pub mod launchd;
pub mod msi;
pub mod secret;
pub mod services;
pub mod task;

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use bach_pack::manifest::{Manifest, Scope};

/// An install found in the record.
#[derive(Clone, Debug)]
pub struct Installed {
    pub dir: PathBuf,
    pub version: String,
}

/// The user's home folder.
pub fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .ok_or_else(|| anyhow!("your home folder isn't known (HOME isn't set)"))
}

/// Bach's folder in the user's `Library/Application Support`: install records,
/// the update channel, logs of the updater.
pub fn support_dir() -> Result<PathBuf> {
    Ok(home()?.join("Library").join("Application Support").join("Bach"))
}

fn record_path(manifest: &Manifest) -> Result<PathBuf> {
    Ok(support_dir()?.join("installs").join(format!("{}.json", bach_pack::paths::safe_dir_name(&manifest.app.id))))
}

/// The install this app has, if any, and only while its program is still there.
/// It is found through the record `register` wrote, and, for an install made
/// into the usual folder, also without one.
pub fn installed(manifest: &Manifest) -> Option<Installed> {
    let from_record = record_path(manifest).ok().and_then(|path| std::fs::read(path).ok()).and_then(|bytes| {
        let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        let dir = PathBuf::from(value.get("dir")?.as_str()?);
        let version = value.get("version").and_then(|version| version.as_str()).unwrap_or_default().to_string();
        Some(Installed { dir, version })
    });
    if let Some(found) = from_record.filter(|found| manifest.program_exists(&found.dir)) {
        return Some(found);
    }
    // No record (another user installed it for everyone, or the record was lost): the usual folder.
    let dir = default_install_dir(manifest);
    let version = crate::work::previous_install(&dir, manifest).map(|previous| previous.app.version)?;
    manifest.program_exists(&dir).then_some(Installed { dir, version })
}

/// `~/Applications/<folder>`, where per-user apps go, or `/Applications/<folder>` for a machine-wide
/// install. An app that is one bundle has no folder (`installer.folder` is empty): the bundle goes
/// straight into Applications, so it is one icon, and moving it to the Trash is the whole uninstall.
pub fn default_install_dir(manifest: &Manifest) -> PathBuf {
    let folder = &manifest.install.folder;
    let base = match manifest.install.scope {
        Scope::User => home().map(|home| home.join("Applications")).unwrap_or_else(|_| PathBuf::from("/Applications")),
        Scope::Machine => PathBuf::from("/Applications"),
    };
    if folder.is_empty() {
        base
    } else {
        base.join(folder)
    }
}

/// Where the uninstaller of an app lives: out of sight in the user's Library, not beside the app. People
/// remove a Mac app by moving it to the Trash, and the watcher (`guard`) then uses this to clean up.
pub fn uninstaller_folder(manifest: &Manifest) -> Result<PathBuf> {
    Ok(support_dir()?.join("uninstallers").join(bach_pack::paths::safe_dir_name(&manifest.app.id)))
}

/// The uninstaller bundle, `Uninstall <app>.app`, in [`uninstaller_folder`].
pub fn uninstaller_bundle(manifest: &Manifest) -> PathBuf {
    uninstaller_folder(manifest).unwrap_or_default().join(manifest.uninstaller())
}

/// The install folder the record names, also when the app has been moved away (to the Trash).
pub fn recorded_dir(manifest: &Manifest) -> Option<PathBuf> {
    let bytes = std::fs::read(record_path(manifest).ok()?).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    Some(PathBuf::from(value.get("dir")?.as_str()?)).filter(|dir| dir.is_absolute())
}

/// The folder the app's link goes in: Applications, the user's or the shared one.
/// `installer.start_menu_folder` is a Windows idea; the link goes straight in Applications.
pub fn start_menu_folder(manifest: &Manifest) -> Result<PathBuf> {
    Ok(match manifest.install.scope {
        Scope::User => home()?.join("Applications"),
        Scope::Machine => PathBuf::from("/Applications"),
    })
}

/// The name of a link: the app's name, as the `.app` it points to is named when it is a bundle.
fn link_name(manifest: &Manifest) -> String {
    let name = bach_pack::paths::file_name_part(&manifest.app.name, &manifest.app.id);
    if manifest.app.executable.to_lowercase().ends_with(".app") {
        format!("{name}.app")
    } else {
        name
    }
}

/// Where the Applications link goes.
pub fn start_menu_shortcut(manifest: &Manifest) -> Result<PathBuf> {
    Ok(start_menu_folder(manifest)?.join(link_name(manifest)))
}

/// Extra shortcuts (`[[shortcuts]]`) are a Windows feature; there are none on macOS.
pub fn extra_shortcuts(_manifest: &Manifest) -> Vec<(PathBuf, &bach_pack::manifest::Shortcut)> {
    Vec::new()
}

/// Where the Desktop link goes.
pub fn desktop_shortcut(manifest: &Manifest) -> Result<PathBuf> {
    Ok(user_desktop()?.join(link_name(manifest)))
}

/// The current user's Desktop folder.
pub fn user_desktop() -> Result<PathBuf> {
    Ok(home()?.join("Desktop"))
}

/// Windows groups taskbar windows by this id; macOS has nothing like it.
pub fn app_user_model_id(manifest: &Manifest) -> String {
    manifest.app.id.clone()
}

/// A link at `link` that opens `target`: a symbolic link, which Finder, Launchpad
/// and `open` follow. An existing link is replaced; a real file or folder of that
/// name is never touched.
pub fn create_shortcut(link: &Path, target: &Path, _description: &str, _app_id: &str) -> Result<()> {
    if link.parent() == target.parent() {
        // The app already sits where the link would go.
        return Ok(());
    }
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("couldn't create {}", parent.display()))?;
    }
    match std::fs::symlink_metadata(link) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::remove_file(link)?,
        Ok(_) => bail!("{} is already there and isn't a link made by setup, so it was left alone", link.display()),
        Err(_) => {}
    }
    std::os::unix::fs::symlink(target, link).with_context(|| format!("couldn't link {}", link.display()))
}

/// [`create_shortcut`] with arguments: macOS links take none, so they are dropped.
pub fn create_shortcut_with(link: &Path, target: &Path, _arguments: Option<&str>, description: &str, app_id: &str) -> Result<()> {
    create_shortcut(link, target, description, app_id)
}

/// Writes the record of this install, so setup finds it again and the next run
/// knows it is an update. `size` is what this install put on disk.
pub fn register(manifest: &Manifest, dir: &Path, size: u64) -> Result<()> {
    let path = record_path(manifest)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("couldn't create {}", parent.display()))?;
    }
    let record = serde_json::json!({
        "name": manifest.app.name,
        "id": manifest.app.id,
        "version": manifest.app.version,
        "publisher": manifest.app.publisher,
        "dir": dir.to_string_lossy(),
        "uninstaller": uninstaller_bundle(manifest).to_string_lossy(),
        "size": size,
    });
    let text = serde_json::to_string_pretty(&record)?;
    let partial = path.with_extension("json.new");
    std::fs::write(&partial, format!("{text}\n")).with_context(|| format!("couldn't write {}", partial.display()))?;
    std::fs::rename(&partial, &path).with_context(|| format!("couldn't write {}", path.display()))
}

/// Takes the install record away.
pub fn unregister(manifest: &Manifest) -> Result<()> {
    match std::fs::remove_file(record_path(manifest)?) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err.into()),
        _ => Ok(()),
    }
}

/// Windows tells Explorer that icons changed; nothing is needed here.
pub fn notify_shell() {}

/// For tests: runs `f` with `HOME` pointing at `home` and launchd left alone, one test at a time (the
/// environment is shared by every test thread).
#[cfg(test)]
pub(crate) fn with_home<T>(home: &Path, f: impl FnOnce() -> T) -> T {
    use std::sync::Mutex;
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let (old_home, old_skip) = (std::env::var_os("HOME"), std::env::var_os("BACH_NO_LAUNCHCTL"));
    std::env::set_var("HOME", home);
    std::env::set_var("BACH_NO_LAUNCHCTL", "1");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    for (name, value) in [("HOME", old_home), ("BACH_NO_LAUNCHCTL", old_skip)] {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
    match result {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(executable: &str) -> Manifest {
        let json = format!(
            r#"{{"format":1,"builder":"t","platform":"macos","app":{{"id":"Hello","name":"Hello","version":"1.0.0","publisher":"P","executable":"{executable}"}},
            "install":{{"folder":"Hello","desktop_shortcut":true}},"uninstall":{{"data":[],"data_description":null}},
            "look":{{"theme":"system","accent":null,"logo":null}},"files":[{{"path":"{executable}/Contents/x","size":1}}]}}"#
        );
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn links_are_named_after_the_app() {
        assert_eq!(link_name(&manifest("Hello.app")), "Hello.app");
        assert_eq!(link_name(&manifest("bin/hello")), "Hello");
    }

    #[test]
    fn a_link_replaces_a_link_but_never_a_real_folder() {
        let dir = std::env::temp_dir().join(format!("bach-mac-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("apps/Hello.app")).unwrap();
        std::fs::create_dir_all(dir.join("apps2/Hello.app")).unwrap();
        let link = dir.join("links/Hello.app");
        create_shortcut(&link, &dir.join("apps/Hello.app"), "", "").unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), dir.join("apps/Hello.app"));
        create_shortcut(&link, &dir.join("apps2/Hello.app"), "", "").unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), dir.join("apps2/Hello.app"));
        // A real folder of that name stays.
        let real = dir.join("real/Hello.app");
        std::fs::create_dir_all(&real).unwrap();
        assert!(create_shortcut(&real, &dir.join("apps/Hello.app"), "", "").is_err());
        assert!(real.is_dir() && std::fs::symlink_metadata(&real).unwrap().is_dir());
        // The app already sits there: nothing to link.
        create_shortcut(&dir.join("apps/Other.app"), &dir.join("apps/Hello.app"), "", "").unwrap();
        assert!(!dir.join("apps/Other.app").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_usual_folders_are_in_applications() {
        let user = default_install_dir(&manifest("Hello.app"));
        assert!(user.ends_with("Applications/Hello"), "{}", user.display());
        // An app that is one bundle goes straight into Applications.
        let mut bare = manifest("Hello.app");
        bare.install.folder = String::new();
        assert!(default_install_dir(&bare).ends_with("Applications") && !default_install_dir(&bare).ends_with("Hello"));
        let mut machine = manifest("Hello.app");
        machine.install.scope = Scope::Machine;
        assert_eq!(default_install_dir(&machine), Path::new("/Applications/Hello"));
    }
}
