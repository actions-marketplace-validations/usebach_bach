//! Whether a folder is safe to run from with administrator rights: nothing in it
//! may be changed by anyone but an administrator or the user who owns it.

use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// None when `folder` is safe; a reason when it isn't. The folder is a macOS
/// install in `/Applications`, so it and everything below must belong to root or
/// the installing user, and nobody else (group "everyone", "other") may write.
pub fn weakness(folder: &Path) -> Result<Option<String>, String> {
    let meta = std::fs::symlink_metadata(folder).map_err(|err| format!("{} couldn't be read: {err}", folder.display()))?;
    // SAFETY: geteuid has no arguments and cannot fail.
    let me = unsafe { libc::geteuid() };
    if meta.uid() != 0 && meta.uid() != me {
        return Ok(Some(format!("{} belongs to another user", folder.display())));
    }
    if meta.mode() & 0o002 != 0 && !meta.file_type().is_symlink() {
        return Ok(Some(format!("anyone can change {}", folder.display())));
    }
    Ok(None)
}

/// [`weakness`] for the folder and everything below it.
pub fn tree_weakness(folder: &Path) -> Result<Option<String>, String> {
    if let Some(problem) = weakness(folder)? {
        return Ok(Some(problem));
    }
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(_) => return Ok(None),
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            if let Some(problem) = tree_weakness(&entry.path())? {
                return Ok(Some(problem));
            }
        } else if let Some(problem) = weakness(&entry.path())? {
            return Ok(Some(problem));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_world_writable_file_makes_the_tree_unsafe() {
        let dir = std::env::temp_dir().join(format!("bach-mac-acl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.join("sub/app"), b"x").unwrap();
        std::fs::set_permissions(dir.join("sub/app"), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(tree_weakness(&dir), Ok(None));
        std::fs::set_permissions(dir.join("sub/app"), std::fs::Permissions::from_mode(0o757)).unwrap();
        assert!(tree_weakness(&dir).unwrap().is_some_and(|why| why.contains("anyone can change")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
