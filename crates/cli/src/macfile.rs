//! Reading the payload out of a macOS setup file: a `.dmg`, a `.zip`, a
//! Setup app (`.app` folder) or the `setup.bach` payload itself. `bach inspect`
//! and `bach diff` read manifests through here.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use bach_pack::macapp;

/// A macOS setup file's payload, readable until this is dropped (a mounted
/// disk image is detached and an unpacked copy deleted then).
pub struct Payload {
    path: PathBuf,
    cleanup: Cleanup,
}

enum Cleanup {
    None,
    Detach(PathBuf),
    Delete(PathBuf),
}

impl Payload {
    pub fn open(&self) -> Result<File> {
        File::open(&self.path).with_context(|| format!("couldn't open {}", self.path.display()))
    }
}

impl Drop for Payload {
    fn drop(&mut self) {
        match &self.cleanup {
            Cleanup::None => {}
            Cleanup::Detach(mount) => {
                let _ = Command::new("hdiutil").args(["detach", "-quiet", "-force"]).arg(mount).status();
                let _ = std::fs::remove_dir(mount);
            }
            Cleanup::Delete(dir) => {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}

/// The payload of a macOS setup file, or None when `path` isn't one.
pub fn payload(path: &Path) -> Result<Option<Payload>> {
    if path.is_dir() {
        let inside = path.join(macapp::PAYLOAD);
        if !inside.is_file() {
            bail!("{} has no {} in it, so it isn't a Bach setup app", path.display(), macapp::PAYLOAD);
        }
        return Ok(Some(Payload { path: inside, cleanup: Cleanup::None }));
    }
    let extension = path.extension().map(|ext| ext.to_string_lossy().to_ascii_lowercase());
    match extension.as_deref() {
        Some("bach") => Ok(Some(Payload { path: path.to_path_buf(), cleanup: Cleanup::None })),
        Some("dmg") => from_dmg(path).map(Some),
        Some("zip") if zip_holds_payload(path) => from_zip(path).map(Some),
        _ => Ok(None),
    }
}

fn from_dmg(path: &Path) -> Result<Payload> {
    if !cfg!(target_os = "macos") {
        bail!("reading a .dmg needs macOS (hdiutil); unzip the setup file on a Mac, or build a .zip with macos.output");
    }
    let mount = scratch("mount")?;
    let attached = Command::new("hdiutil")
        .args(["attach", "-quiet", "-nobrowse", "-readonly", "-noverify", "-noautoopen", "-mountpoint"])
        .arg(&mount)
        .arg(path)
        .output()
        .context("couldn't run hdiutil")?;
    if !attached.status.success() {
        let _ = std::fs::remove_dir(&mount);
        bail!("couldn't open {}: {}", path.display(), String::from_utf8_lossy(&attached.stderr).trim());
    }
    // The mount is detached when this is dropped, also on the way out with an error.
    let mut guard = Payload { path: PathBuf::new(), cleanup: Cleanup::Detach(mount.clone()) };
    let app = std::fs::read_dir(&mount)?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "app") && path.join(macapp::PAYLOAD).is_file());
    match app {
        Some(app) => {
            guard.path = app.join(macapp::PAYLOAD);
            Ok(guard)
        }
        None => bail!("{} holds no Bach setup app", path.display()),
    }
}

fn zip_holds_payload(path: &Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    zip::ZipArchive::new(file).is_ok_and(|archive| archive.file_names().filter_map(Result::ok).any(|name| name.ends_with(macapp::PAYLOAD)))
}

fn from_zip(path: &Path) -> Result<Payload> {
    let file = File::open(path).with_context(|| format!("couldn't open {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file).with_context(|| format!("{} isn't a zip file", path.display()))?;
    let name = archive
        .file_names()
        .filter_map(Result::ok)
        .find(|name| name.ends_with(macapp::PAYLOAD))
        .map(|name| name.into_owned())
        .context("the zip holds no Bach setup app")?;
    let dir = scratch("zip")?;
    let target = dir.join("setup.bach");
    let mut entry = archive.by_name(&name)?;
    let mut out = File::create(&target)?;
    std::io::copy(&mut entry, &mut out)?;
    Ok(Payload { path: target, cleanup: Cleanup::Delete(dir) })
}

/// A new private folder in the user's cache for something unpacked or mounted.
fn scratch(kind: &str) -> Result<PathBuf> {
    let root = crate::runtime::cache_root().context("there's no folder of yours to unpack into (set HOME)")?;
    let dir = root.join("scratch").join(format!(
        "{kind}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
    ));
    crate::runtime::private_dir(&root)?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zip_without_a_setup_app_is_not_one() {
        let path = std::env::temp_dir().join(format!("bach-macfile-{}.zip", std::process::id()));
        {
            use std::io::Write;
            let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
            zip.start_file("readme.txt", zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(b"hi").unwrap();
            zip.finish().unwrap();
        }
        assert!(payload(&path).unwrap().is_none());
        std::fs::remove_file(&path).unwrap();
    }
}
