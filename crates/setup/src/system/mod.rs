//! What setup asks of Windows: where per-user programs and shortcuts live,
//! `.lnk` shortcuts, and the entry under Settings › Apps that lists the
//! app and starts its uninstaller. Everything here is per user (HKCU, the
//! user's own folders), so setup never needs administrator rights, unless the
//! app asked for a machine-wide install (`installer.scope = "machine"`): then
//! the same things happen in HKLM and the shared folders, from an elevated
//! setup (see [`elevate`]).
//!
//! These calls block; setup runs them on its worker thread, apart from the
//! quick registry read at startup.
//!
//! - [`host`]: the user's languages, free space, the one-setup-at-a-time lock.
//! - [`env_path`]: adding the app to the user's `PATH`, and taking it off.
//! - [`integration`]: starting with Windows, link schemes and file types.
//! - [`changes`]: registry values, environment variables and right-click entries.
//! - [`secret`]: an app's access key, sealed for this Windows user.

pub mod acl;
pub mod apps;
pub mod changes;
pub mod channel;
pub mod elevate;
pub mod env_path;
pub mod folders;
pub mod host;
pub mod integration;
pub mod secret;
pub mod task;

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use bach_pack::manifest::{Manifest, Scope, UNINSTALLER};
use windows::core::{Interface, GUID, HSTRING};
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    FOLDERID_CommonPrograms, FOLDERID_Desktop, FOLDERID_ProgramFiles, FOLDERID_Programs, FOLDERID_PublicDesktop, FOLDERID_UserProgramFiles,
    IShellLinkW, SHChangeNotify, SHGetKnownFolderPath, ShellLink, KF_FLAG_DEFAULT, SHCNE_ASSOCCHANGED, SHCNF_IDLIST,
};
use windows_registry::{Key, CURRENT_USER, LOCAL_MACHINE};

/// An install found in the registry.
#[derive(Clone, Debug)]
pub struct Installed {
    pub dir: PathBuf,
    pub version: String,
}

/// The part of the registry this install lives in: the user's, or the
/// computer's for a machine-wide install.
pub fn hive(manifest: &Manifest) -> &'static Key {
    match manifest.install.scope {
        Scope::User => CURRENT_USER,
        Scope::Machine => LOCAL_MACHINE,
    }
}

fn uninstall_key(manifest: &Manifest) -> String {
    format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{}", manifest.app.id)
}

/// The install Settings › Apps lists for this app, if any, and only while
/// its program is still there.
pub fn installed(manifest: &Manifest) -> Option<Installed> {
    let key = hive(manifest).open(uninstall_key(manifest)).ok()?;
    let dir = PathBuf::from(key.get_string("InstallLocation").ok()?);
    let version = key.get_string("DisplayVersion").unwrap_or_default();
    manifest.executable_in(&dir).is_file().then_some(Installed { dir, version })
}

/// `%LOCALAPPDATA%\Programs\<folder>`, where per-user apps go, or
/// `%ProgramFiles%\<folder>` for a machine-wide install.
pub fn default_install_dir(manifest: &Manifest) -> PathBuf {
    let folder = &manifest.install.folder;
    let (id, variable, below) = match manifest.install.scope {
        Scope::User => (&FOLDERID_UserProgramFiles, "LOCALAPPDATA", "Programs"),
        Scope::Machine => (&FOLDERID_ProgramFiles, "ProgramFiles", ""),
    };
    known_folder(id)
        .or_else(|_| std::env::var_os(variable).map(|base| PathBuf::from(base).join(below)).ok_or_else(|| anyhow!("{variable} isn't set")))
        .map(|programs| programs.join(folder))
        .unwrap_or_else(|_| PathBuf::from(r"C:\").join(folder))
}

/// A shortcut's file name: `name`, minus anything Windows doesn't allow in
/// a file name (or the app id, if nothing is left).
fn link_name(name: &str, fallback: &str) -> String {
    let name: String = name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { ' ' } else { c }).collect();
    let name = name.trim().trim_end_matches('.');
    let name = if name.is_empty() { fallback } else { name };
    format!("{name}.lnk")
}

fn shortcut_name(manifest: &Manifest) -> String {
    link_name(&manifest.app.name, &manifest.app.id)
}

/// The app's folder in the Start menu: `installer.start_menu_folder`, or
/// the Start menu itself.
pub fn start_menu_folder(manifest: &Manifest) -> Result<PathBuf> {
    let programs = known_folder(match manifest.install.scope {
        Scope::User => &FOLDERID_Programs,
        Scope::Machine => &FOLDERID_CommonPrograms,
    })?;
    Ok(match &manifest.install.start_menu_folder {
        Some(folder) => programs.join(folder),
        None => programs,
    })
}

/// Where the Start menu shortcut goes.
pub fn start_menu_shortcut(manifest: &Manifest) -> Result<PathBuf> {
    Ok(start_menu_folder(manifest)?.join(shortcut_name(manifest)))
}

/// Where each of `[[shortcuts]]` goes, with its manifest entry.
pub fn extra_shortcuts(manifest: &Manifest) -> Vec<(PathBuf, &bach_pack::manifest::Shortcut)> {
    let Ok(folder) = start_menu_folder(manifest) else {
        return Vec::new();
    };
    manifest.shortcuts.iter().map(|shortcut| (folder.join(link_name(&shortcut.name, &manifest.app.id)), shortcut)).collect()
}

/// Where the desktop shortcut goes (the desktop may live in OneDrive).
pub fn desktop_shortcut(manifest: &Manifest) -> Result<PathBuf> {
    let desktop = match manifest.install.scope {
        Scope::User => &FOLDERID_Desktop,
        Scope::Machine => &FOLDERID_PublicDesktop,
    };
    Ok(known_folder(desktop)?.join(shortcut_name(manifest)))
}

/// The current user's desktop folder (it may live in OneDrive).
pub fn user_desktop() -> Result<PathBuf> {
    known_folder(&FOLDERID_Desktop)
}

fn known_folder(id: &GUID) -> Result<PathBuf> {
    // Safety: the returned buffer is ours to free, which we do once it's
    // copied into a PathBuf.
    unsafe {
        let raw = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).context("Windows didn't say where that folder is")?;
        let path = raw.to_string().map(PathBuf::from);
        CoTaskMemFree(Some(raw.0 as *const _));
        path.context("Windows returned a folder name that isn't valid text")
    }
}

/// The app's AppUserModelID: its app id, in the characters Windows allows
/// (no spaces; at most 128). The shortcuts carry it, so the taskbar groups the
/// windows of the app under them and notifications are traced back to the app.
/// It never changes with the version, so an update keeps the grouping.
pub fn app_user_model_id(manifest: &Manifest) -> String {
    let id: String =
        manifest.app.id.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '.' }).collect();
    id.chars().take(128).collect()
}

/// Writes a `.lnk` at `link` that starts `target` in its own folder, carrying
/// the app's AppUserModelID.
pub fn create_shortcut(link: &Path, target: &Path, description: &str, app_id: &str) -> Result<()> {
    create_shortcut_with(link, target, None, description, app_id)
}

/// [`create_shortcut`] with command-line arguments.
pub fn create_shortcut_with(link: &Path, target: &Path, arguments: Option<&str>, description: &str, app_id: &str) -> Result<()> {
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _com = ComThread::enter();
    // Safety: plain COM calls on an interface we own, on a thread with COM
    // initialised for as long as `_com` lives.
    unsafe {
        let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        shell_link.SetPath(&HSTRING::from(target.as_os_str()))?;
        if let Some(dir) = target.parent() {
            shell_link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
        }
        shell_link.SetDescription(&HSTRING::from(description))?;
        if let Some(arguments) = arguments {
            shell_link.SetArguments(&HSTRING::from(arguments))?;
        }
        shell_link.SetIconLocation(&HSTRING::from(target.as_os_str()), 0)?;
        if !app_id.is_empty() {
            let store: IPropertyStore = shell_link.cast()?;
            store.SetValue(&PKEY_AppUserModel_ID, &PROPVARIANT::from(app_id))?;
            store.Commit()?;
        }
        let file: IPersistFile = shell_link.cast()?;
        file.Save(&HSTRING::from(link.as_os_str()), true)?;
    }
    Ok(())
}

/// Lists the app under Settings › Apps, with its uninstaller. `size` is
/// what this install put on disk.
pub fn register(manifest: &Manifest, dir: &Path, size: u64) -> Result<()> {
    let app = &manifest.app;
    let exe = manifest.executable_in(dir);
    let uninstaller = dir.join(UNINSTALLER);
    let key = hive(manifest).create(uninstall_key(manifest))?;
    key.set_string("DisplayName", &app.name)?;
    key.set_string("DisplayVersion", &app.version)?;
    key.set_string("Publisher", &app.publisher)?;
    key.set_string("DisplayIcon", format!("{},0", exe.display()))?;
    key.set_string("InstallLocation", dir.to_string_lossy())?;
    key.set_string("UninstallString", format!("\"{}\"", uninstaller.display()))?;
    // What IT tools run to remove it without a window.
    key.set_string("QuietUninstallString", format!("\"{}\" --silent", uninstaller.display()))?;
    if let Some(website) = &app.website {
        key.set_string("URLInfoAbout", website)?;
    }
    let size_kb = size / 1024;
    key.set_u32("EstimatedSize", u32::try_from(size_kb).unwrap_or(u32::MAX))?;
    key.set_u32("NoModify", 1)?;
    key.set_u32("NoRepair", 1)?;
    Ok(())
}

/// Takes the app off Settings › Apps.
pub fn unregister(manifest: &Manifest) -> Result<()> {
    let key = uninstall_key(manifest);
    match hive(manifest).open(&key) {
        Ok(_) => Ok(hive(manifest).remove_tree(&key)?),
        // Nothing registered: nothing to remove.
        Err(_) => Ok(()),
    }
}

/// Tells Explorer that shortcuts and icons changed, so the Start menu and
/// the desktop pick them up straight away.
pub fn notify_shell() {
    // Safety: no pointers passed; the flags are the documented pair.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

/// COM for the current thread, released when dropped.
pub(crate) struct ComThread {
    owned: bool,
}

impl ComThread {
    pub(crate) fn enter() -> Self {
        // Safety: paired with CoUninitialize in Drop when it succeeded.
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        Self { owned: result.is_ok() }
    }
}

impl Drop for ComThread {
    fn drop(&mut self) {
        if self.owned {
            // Safety: this thread's own successful CoInitializeEx.
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::BSTR;

    fn manifest(id: &str) -> Manifest {
        let json = format!(
            r#"{{"format":1,"builder":"t","app":{{"id":"{id}","name":"Hello","version":"1.0.0","publisher":"P","executable":"hello.exe"}},
            "install":{{"folder":"Hello","desktop_shortcut":true}},"uninstall":{{"data":[],"data_description":null}},
            "look":{{"theme":"system","accent":null,"logo":null}},"files":[{{"path":"hello.exe","size":1}}]}}"#
        );
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn the_id_is_made_of_what_windows_allows() {
        assert_eq!(app_user_model_id(&manifest("Hello")), "Hello");
        assert_eq!(app_user_model_id(&manifest("My App (beta)")), "My.App..beta.");
        let long = "a".repeat(200);
        assert_eq!(app_user_model_id(&manifest(&long)).len(), 128);
        assert!(!app_user_model_id(&manifest("a b")).contains(' '));
    }

    /// What a shortcut says its AppUserModelID is, read back the way Explorer does.
    fn read_back(link: &Path) -> Option<String> {
        let _com = ComThread::enter();
        // Safety: COM calls on interfaces we own, on a thread with COM initialised.
        unsafe {
            let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
            let file: IPersistFile = shell_link.cast().ok()?;
            file.Load(&HSTRING::from(link.as_os_str()), windows::Win32::System::Com::STGM_READ).ok()?;
            let store: IPropertyStore = shell_link.cast().ok()?;
            let value = store.GetValue(&PKEY_AppUserModel_ID).ok()?;
            BSTR::try_from(&value).ok().map(|text| text.to_string())
        }
    }

    #[test]
    fn a_shortcut_carries_the_app_user_model_id() {
        let dir = std::env::temp_dir().join(format!("bach-aumid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("hello.exe");
        std::fs::write(&target, b"MZ").unwrap();
        let link = dir.join("Hello.lnk");
        let id = app_user_model_id(&manifest("BachQA.Aumid Test"));
        create_shortcut(&link, &target, "Says hello", &id).unwrap();
        assert_eq!(read_back(&link).as_deref(), Some("BachQA.Aumid.Test"));
        // An update writes the shortcut again with the same id.
        create_shortcut(&link, &target, "Says hello", &id).unwrap();
        assert_eq!(read_back(&link).as_deref(), Some("BachQA.Aumid.Test"));
        // Without an id nothing is written.
        let plain = dir.join("Plain.lnk");
        create_shortcut(&plain, &target, "Says hello", "").unwrap();
        assert_eq!(read_back(&plain).filter(|id| !id.is_empty()), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
