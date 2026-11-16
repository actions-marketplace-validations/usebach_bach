//! Windows' own folders, asked of Windows and not of the environment.
//!
//! Environment variables belong to the user, who can change them for any
//! program they start. Where setup runs a program with administrator rights it
//! must not trust them to say where `System32` or Program Files is.

use std::path::{Path, PathBuf};

use windows::core::GUID;
use windows::Win32::UI::Shell::{
    FOLDERID_ProgramData, FOLDERID_ProgramFiles, FOLDERID_ProgramFilesCommon, FOLDERID_ProgramFilesX86, FOLDERID_System, FOLDERID_Windows,
};

fn folder(id: &GUID) -> Option<PathBuf> {
    super::known_folder(id).ok()
}

/// `C:\Windows\System32`.
pub fn system_dir() -> Option<PathBuf> {
    folder(&FOLDERID_System)
}

/// The value of a variable Windows itself defines (`%SystemRoot%` and the
/// like), from the system and never from the environment. None for any other
/// name.
pub fn variable(name: &str) -> Option<String> {
    let id = match name.to_ascii_lowercase().as_str() {
        "systemroot" | "windir" => &FOLDERID_Windows,
        "programfiles" | "programw6432" => &FOLDERID_ProgramFiles,
        "programfiles(x86)" => &FOLDERID_ProgramFilesX86,
        "commonprogramfiles" => &FOLDERID_ProgramFilesCommon,
        "programdata" => &FOLDERID_ProgramData,
        _ => return None,
    };
    folder(id).map(|path| path.to_string_lossy().into_owned())
}

/// Whether `path` is inside Program Files or the Windows folder, where only
/// administrators can write: a program there can't have been swapped by the
/// user's own programs, so it may run with administrator rights.
pub fn is_admin_only(path: &Path) -> bool {
    [&FOLDERID_ProgramFiles, &FOLDERID_ProgramFilesX86, &FOLDERID_Windows]
        .into_iter()
        .filter_map(folder)
        .any(|root| starts_with_folder(path, &root))
}

/// Whether `path` is `root` or below it, compared by components so
/// `C:\Program Files Evil` isn't taken for `C:\Program Files`, and `..` can't
/// lead out.
fn starts_with_folder(path: &Path, root: &Path) -> bool {
    use std::path::Component;
    if path.components().any(|part| matches!(part, Component::ParentDir | Component::CurDir)) {
        return false;
    }
    let lower =
        |path: &Path| -> Vec<String> { path.components().map(|part| part.as_os_str().to_string_lossy().to_ascii_lowercase()).collect() };
    let (path, root) = (lower(path), lower(root));
    path.len() > root.len() && path[..root.len()] == root[..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_folders_come_from_windows() {
        let system = system_dir().expect("System32");
        assert!(system.ends_with("System32") || system.ends_with("system32"), "{}", system.display());
        // Setting the variable doesn't move the answer.
        let real = variable("SystemRoot").unwrap();
        assert!(Path::new(&real).is_dir());
        assert_eq!(variable("windir"), Some(real));
        assert_eq!(variable("LOCALAPPDATA"), None, "only Windows' own variables");
    }

    #[test]
    fn only_program_files_and_windows_count_as_admin_only() {
        let files = variable("ProgramFiles").unwrap();
        let program = Path::new(&files).join("Hello").join("unins000.exe");
        assert!(is_admin_only(&program));
        assert!(is_admin_only(&system_dir().unwrap().join("msiexec.exe")));
        assert!(!is_admin_only(Path::new(&files)), "the folder itself holds no program");
        assert!(!is_admin_only(Path::new(&format!("{files} Evil\\x.exe"))));
        assert!(!is_admin_only(Path::new(r"C:\Users\someone\AppData\Local\Programs\Hello\x.exe")));
        assert!(!is_admin_only(&Path::new(&files).join("..").join("Users").join("x.exe")), "no way out through ..");
    }
}
