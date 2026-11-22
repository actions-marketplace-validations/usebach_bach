//! Where only administrators can write, so a program there can't have been
//! swapped by the user's own programs.

use std::path::{Component, Path};

/// Whether `path` is inside `/Applications` or `/Library`, where a program can't be
/// changed by anything but an administrator. A per-user install in `~/Applications`
/// is not admin-only: the user's own programs can write there.
pub fn is_admin_only(path: &Path) -> bool {
    if path.components().any(|part| matches!(part, Component::ParentDir | Component::CurDir)) {
        return false;
    }
    ["/Applications", "/Library"].iter().any(|root| path.starts_with(root) && path != Path::new(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_shared_folders_count_as_admin_only() {
        assert!(is_admin_only(Path::new("/Applications/Hello/Hello.app")));
        assert!(!is_admin_only(Path::new("/Applications")), "the folder itself holds no program");
        assert!(!is_admin_only(Path::new("/ApplicationsEvil/x")));
        assert!(!is_admin_only(Path::new("/Users/me/Applications/Hello")));
        assert!(!is_admin_only(Path::new("/Applications/../Users/me/x")), "no way out through ..");
    }
}
