//! A macOS setup file (`.dmg` or `.zip`) opened up: the Setup app inside it is
//! copied out to a folder of its own, so it can run (and a window can stay open)
//! after the disk image is gone, and so its signature can be read.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::macapp;

/// Copies the Setup app out of `file` into `into` (which is created, and is not
/// cleaned here) and gives back the path of its program.
pub fn unpack(file: &Path, into: &Path) -> Result<PathBuf, String> {
    let extension = file.extension().map(|ext| ext.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    unpack_as(file, &extension, into)
}

/// [`unpack`] for a file whose name doesn't say what it is (a download in progress): `kind` is `dmg` or `zip`.
pub fn unpack_as(file: &Path, kind: &str, into: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(into).map_err(|err| format!("couldn't create {}: {err}", into.display()))?;
    match Some(kind) {
        Some("dmg") => from_dmg(file, into)?,
        Some("zip") => run(Command::new("/usr/bin/ditto").args(["-x", "-k"]).arg(file).arg(into), "unzip the update")?,
        _ => return Err("the download isn't a macOS setup file (.dmg or .zip)".into()),
    }
    let app = find_app(into).ok_or("the download holds no setup app")?;
    Ok(app.join(macapp::RUNTIME))
}

fn run(command: &mut Command, what: &str) -> Result<(), String> {
    let output = command.output().map_err(|err| format!("couldn't {what}: {err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("couldn't {what}: {}", String::from_utf8_lossy(&output.stderr).trim()))
    }
}

fn from_dmg(file: &Path, into: &Path) -> Result<(), String> {
    let mount = into.join(".mount");
    std::fs::create_dir_all(&mount).map_err(|err| err.to_string())?;
    run(
        Command::new("/usr/bin/hdiutil")
            .args(["attach", "-quiet", "-nobrowse", "-readonly", "-noverify", "-noautoopen", "-mountpoint"])
            .arg(&mount)
            .arg(file),
        "open the disk image",
    )?;
    let copied = match find_app(&mount) {
        Some(app) => {
            run(Command::new("/usr/bin/ditto").arg(&app).arg(into.join(app.file_name().unwrap_or_default())), "copy the setup app")
        }
        None => Err("the disk image holds no setup app".to_string()),
    };
    let _ = Command::new("/usr/bin/hdiutil").args(["detach", "-quiet", "-force"]).arg(&mount).status();
    let _ = std::fs::remove_dir(&mount);
    copied
}

/// The first Setup app (an `.app` with a payload) in `folder` or one level below it.
fn find_app(folder: &Path) -> Option<PathBuf> {
    let has_payload = |path: &Path| path.extension().is_some_and(|ext| ext == "app") && path.join(macapp::PAYLOAD).is_file();
    let entries: Vec<PathBuf> = std::fs::read_dir(folder).ok()?.flatten().map(|entry| entry.path()).collect();
    if let Some(app) = entries.iter().find(|path| has_payload(path)) {
        return Some(app.clone());
    }
    entries
        .iter()
        .filter(|path| path.is_dir())
        .find_map(|dir| std::fs::read_dir(dir).ok()?.flatten().map(|entry| entry.path()).find(|path| has_payload(path)))
}

/// The payload file of the Setup app a program path belongs to.
pub fn payload_of(program: &Path) -> Option<PathBuf> {
    let app = program.ancestors().find(|part| part.extension().is_some_and(|ext| ext == "app"))?;
    Some(app.join(macapp::PAYLOAD))
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn a_zip_of_a_setup_app_unpacks_with_its_program_executable() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(format!("bach-macsetup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let app = base.join("src/Install Hello.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::create_dir_all(app.join("Contents/Resources")).unwrap();
        std::fs::write(app.join(macapp::RUNTIME), b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(app.join(macapp::RUNTIME), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(app.join(macapp::PAYLOAD), b"payload").unwrap();
        let zip = base.join("Hello.zip");
        assert!(Command::new("/usr/bin/ditto").args(["-c", "-k", "--keepParent"]).arg(&app).arg(&zip).status().unwrap().success());
        let program = unpack(&zip, &base.join("out")).unwrap();
        assert!(program.ends_with("Install Hello.app/Contents/MacOS/bach-setup"), "{}", program.display());
        assert!(std::fs::metadata(&program).unwrap().permissions().mode() & 0o111 != 0);
        assert!(payload_of(&program).unwrap().is_file());
        assert!(unpack(&base.join("Hello.exe"), &base.join("out2")).is_err());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
