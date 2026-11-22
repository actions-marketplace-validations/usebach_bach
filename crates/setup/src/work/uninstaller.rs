//! The uninstaller of an install: `uninstall.exe` in the install folder on Windows, and on a Mac an
//! app bundle, `Uninstall <app>.app`, holding a copy of this runtime and the installed manifest (see
//! `bach_pack::macapp`). On a Mac it lives out of sight in `~/Library/Application Support/Bach/uninstallers`,
//! not beside the app: the app is the only thing in Applications, and moving it to the Trash is the uninstall.
//! The rest of setup asks for its place here instead of knowing either shape.

use std::path::{Path, PathBuf};

use bach_pack::manifest::Manifest;

/// The uninstaller: the file in the install folder on Windows, the bundle in the user's Library on a Mac.
pub fn path(manifest: &Manifest, dir: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        let _ = dir;
        crate::system::uninstaller_bundle(manifest)
    }
    #[cfg(windows)]
    dir.join(manifest.uninstaller())
}

/// The file that holds the manifest an install left: the uninstaller itself on
/// Windows, and the payload inside the bundle on a Mac. On a Mac an app that was
/// renamed between versions left a bundle under its old name, which is found too.
pub fn payload(manifest: &Manifest, dir: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    if let Some(found) = mac::find(manifest, dir) {
        return found.join(bach_pack::macapp::PAYLOAD);
    }
    #[cfg(target_os = "macos")]
    {
        path(manifest, dir).join(bach_pack::macapp::PAYLOAD)
    }
    #[cfg(windows)]
    dir.join(manifest.uninstaller_payload())
}

/// Whether the install has an uninstaller.
pub fn exists(manifest: &Manifest, dir: &Path) -> bool {
    payload(manifest, dir).is_file()
}

#[cfg(target_os = "macos")]
pub use mac::{keep, restore, write};

#[cfg(target_os = "macos")]
mod mac {
    use std::fs::{self, File};
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use bach_pack::container;
    use bach_pack::i18n::Key;
    use bach_pack::macapp;
    use bach_pack::manifest::Manifest;

    use crate::package::Package;
    use crate::texts::Texts;
    use crate::work::{describe, place, plain};

    /// The bundle of this install's uninstaller: under the manifest's name, or under
    /// another `Uninstall *.app` in the folder that carries this app's payload.
    pub fn find(manifest: &Manifest, dir: &Path) -> Option<PathBuf> {
        let named = super::path(manifest, dir);
        if named.join(macapp::PAYLOAD).is_file() {
            return Some(named);
        }
        let entries = fs::read_dir(crate::system::uninstaller_folder(manifest).ok()?).ok()?;
        entries.flatten().map(|entry| entry.path()).find(|path| {
            path.extension().is_some_and(|ext| ext == "app")
                && path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("Uninstall "))
                && carries(path, &manifest.app.id)
        })
    }

    /// Whether the bundle's payload is a manifest of `app_id`.
    fn carries(bundle: &Path, app_id: &str) -> bool {
        let Ok(mut file) = File::open(bundle.join(macapp::PAYLOAD)) else {
            return false;
        };
        let Ok(Some(layout)) = container::locate(&mut file) else {
            return false;
        };
        container::read_manifest(&mut file, &layout).is_ok_and(|manifest| manifest.app.id == app_id)
    }

    /// Copies `from` (a folder of files, no links) to `to`, keeping permissions.
    fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            let target = to.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_tree(&entry.path(), &target)?;
            } else {
                fs::copy(entry.path(), &target)?;
            }
        }
        Ok(())
    }

    /// The installed uninstaller, copied aside while an update runs, so a failed
    /// update can put it back. Ok(None) is a first install.
    pub fn keep(manifest: &Manifest, dir: &Path) -> Result<Option<PathBuf>, (PathBuf, std::io::Error)> {
        let Some(current) = find(manifest, dir) else {
            return Ok(None);
        };
        let kept = place::with_suffix(&current, ".bach-prev");
        let _ = fs::remove_dir_all(&kept);
        copy_tree(&current, &kept).map_err(|err| (kept.clone(), err))?;
        Ok(Some(kept))
    }

    /// Puts what [`keep`] kept back; on a first install (nothing kept) the new one is removed.
    pub fn restore(manifest: &Manifest, dir: &Path, kept: Option<PathBuf>) {
        let current = super::path(manifest, dir);
        let _ = fs::remove_dir_all(&current);
        if let Some(kept) = kept {
            // The kept copy has the name the old uninstaller had.
            let original = kept.with_extension("");
            let _ = fs::remove_dir_all(&original);
            let _ = fs::rename(&kept, &original);
        }
    }

    /// Writes `Uninstall <app>.app`: this runtime, the installed manifest and the
    /// logo, and no files.
    pub fn write(package: &Package, installed: &Manifest, dir: &Path, texts: &Texts) -> Result<(), String> {
        let fail = |detail: String| format!("{} {}", texts.get(Key::error_uninstaller), crate::work::sentence(detail));
        let layout = package.layout.ok_or_else(|| fail("this setup file has no payload".into()))?;
        let mut source = File::open(&package.exe).map_err(|err| fail(describe(&err)))?;
        let logo = container::read_logo(&mut source, &layout).map_err(|err| fail(plain(&err)))?;

        let bundle = super::path(installed, dir);
        if let Some(parent) = bundle.parent() {
            fs::create_dir_all(parent).map_err(|err| fail(describe(&err)))?;
        }
        let fresh = place::with_suffix(&bundle, ".bach-new");
        let _ = fs::remove_dir_all(&fresh);
        let built = build(package, installed, &fresh, &logo);
        if let Err(detail) = built {
            let _ = fs::remove_dir_all(&fresh);
            return Err(fail(detail));
        }
        // Seal the bundle (ad hoc), so macOS accepts a program that was copied into place.
        let _ = Command::new("/usr/bin/codesign").args(["--force", "--sign", "-"]).arg(&fresh).output();

        let old = place::with_suffix(&bundle, ".bach-old");
        let _ = fs::remove_dir_all(&old);
        if bundle.exists() {
            fs::rename(&bundle, &old).map_err(|err| {
                let _ = fs::remove_dir_all(&fresh);
                fail(describe(&err))
            })?;
        }
        if let Err(err) = fs::rename(&fresh, &bundle) {
            let _ = fs::rename(&old, &bundle);
            let _ = fs::remove_dir_all(&fresh);
            return Err(fail(describe(&err)));
        }
        let _ = fs::remove_dir_all(&old);
        // An uninstaller under an older name of the app goes.
        if let Ok(entries) = crate::system::uninstaller_folder(installed).and_then(|folder| Ok(fs::read_dir(folder)?)) {
            for path in entries.flatten().map(|entry| entry.path()) {
                let other = path != bundle
                    && path.extension().is_some_and(|ext| ext == "app")
                    && path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("Uninstall "))
                    && carries(&path, &installed.app.id);
                if other {
                    let _ = fs::remove_dir_all(&path);
                }
            }
        }
        Ok(())
    }

    fn build(package: &Package, installed: &Manifest, bundle: &Path, logo: &[u8]) -> Result<(), String> {
        let describe = |what: &str, err: &dyn std::fmt::Display| format!("{what}: {err}");
        for sub in ["Contents/MacOS", "Contents/Resources"] {
            fs::create_dir_all(bundle.join(sub)).map_err(|err| describe("couldn't create the folder", &err))?;
        }
        let program = bundle.join(macapp::RUNTIME);
        fs::copy(&package.runtime, &program).map_err(|err| describe("couldn't copy the setup program", &err))?;
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).map_err(|err| describe("couldn't make it a program", &err))?;
        // The icon: the Setup app's own, when it has one.
        let own_icon = package.runtime.parent().and_then(Path::parent).map(|contents| contents.join("Resources").join("AppIcon.icns"));
        let icon = own_icon.filter(|icon| icon.is_file()).is_some_and(|icon| fs::copy(icon, bundle.join(macapp::ICON)).is_ok());
        fs::write(bundle.join("Contents/Info.plist"), macapp::uninstaller_plist(installed, icon))
            .map_err(|err| describe("couldn't write Info.plist", &err))?;
        let mut out = File::create(bundle.join(macapp::PAYLOAD)).map_err(|err| describe("couldn't write the uninstaller's data", &err))?;
        container::write_uninstaller(&mut out, &[], logo, installed)
            .map_err(|err| describe("couldn't write the uninstaller's data", &err))?;
        out.sync_all().map_err(|err| describe("couldn't write the uninstaller's data", &err))?;
        // Reading the file back proves it is a payload.
        let mut check = File::open(bundle.join(macapp::PAYLOAD)).map_err(|err| err.to_string())?;
        let mut head = [0u8; 1];
        check.read(&mut head).map_err(|err| err.to_string())?;
        Ok(())
    }
}
