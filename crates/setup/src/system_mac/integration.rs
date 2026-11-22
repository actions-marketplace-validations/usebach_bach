//! Open at login, link schemes and file types.
//!
//! Opening at login is a launch agent in `~/Library/LaunchAgents`. A macOS app
//! declares its link schemes and file types in its own `Info.plist` (inside the
//! signed bundle, where setup must not change anything); setup only asks Launch
//! Services to read the bundle (`lsregister`), which makes them take effect now.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use bach_pack::manifest::{FileType, Manifest, Protocol};

use super::launchd::{self, Agent};

const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

fn label(manifest: &Manifest) -> String {
    let id: String = manifest.app.id.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') { c } else { '-' }).collect();
    format!("app.bach.login.{id}")
}

/// What opens the app: `open <bundle>` for a bundle, the program itself otherwise.
fn command(exe: &Path) -> Vec<String> {
    let exe = exe.to_string_lossy().into_owned();
    if exe.to_lowercase().ends_with(".app") {
        vec!["/usr/bin/open".into(), exe]
    } else {
        vec![exe]
    }
}

/// Opens the app when the user signs in.
pub fn set_autostart(manifest: &Manifest, exe: &Path) -> Result<()> {
    let label = label(manifest);
    let agent = Agent {
        label: &label,
        program: command(exe),
        run_at_load: true,
        interval: None,
        background: false,
        environment: Vec::new(),
        watch: Vec::new(),
    };
    launchd::write(&agent)?;
    Ok(())
}

/// Stops opening the app at login. Fine when it wasn't.
pub fn clear_autostart(manifest: &Manifest, _exe: &Path) -> Result<()> {
    launchd::remove(&label(manifest))
}

fn lsregister(args: &[&str], app: &Path) {
    let _ = Command::new(LSREGISTER).args(args).arg(app).output();
}

fn bundle(exe: &Path) -> Option<PathBuf> {
    exe.ancestors().find(|part| part.extension().is_some_and(|ext| ext == "app")).map(Path::to_path_buf)
}

/// Has Launch Services read the app's `Info.plist`, which is where its link scheme is declared.
pub fn register_protocol(_manifest: &Manifest, _protocol: &Protocol, exe: &Path) -> Result<bool> {
    if let Some(app) = bundle(exe) {
        lsregister(&["-f"], &app);
    }
    Ok(true)
}

pub fn unregister_protocol(_manifest: &Manifest, _protocol: &Protocol) -> Result<()> {
    Ok(())
}

/// Has Launch Services read the app's `Info.plist`, which is where its file types are declared.
pub fn register_file_type(_manifest: &Manifest, _file_type: &FileType, exe: &Path) -> Result<()> {
    if let Some(app) = bundle(exe) {
        lsregister(&["-f"], &app);
    }
    Ok(())
}

pub fn unregister_file_type(_manifest: &Manifest, _file_type: &FileType) -> Result<()> {
    Ok(())
}

/// Forgets the app's bundle before it is deleted, so Launch Services doesn't offer it any more.
pub fn forget_bundle(exe: &Path) {
    if let Some(app) = bundle(exe) {
        lsregister(&["-u"], &app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundle_is_opened_and_a_program_is_run() {
        assert_eq!(command(Path::new("/Applications/Hello/Hello.app")), ["/usr/bin/open", "/Applications/Hello/Hello.app"]);
        assert_eq!(command(Path::new("/Applications/Hello/bin/hello")), ["/Applications/Hello/bin/hello"]);
        assert_eq!(bundle(Path::new("/A/Hello.app/Contents/MacOS/hello")), Some(PathBuf::from("/A/Hello.app")));
        assert_eq!(bundle(Path::new("/A/bin/hello")), None);
    }
}
