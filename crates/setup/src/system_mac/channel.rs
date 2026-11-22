//! Which update feed an install follows: the stable one, or the beta one.
//! The choice is kept per app in the user's Application Support folder.

use std::path::PathBuf;

use anyhow::{Context, Result};

fn file(app_id: &str) -> Option<PathBuf> {
    Some(super::support_dir().ok()?.join("channels").join(bach_pack::paths::safe_dir_name(app_id)))
}

/// Whether the beta channel is chosen for this app.
pub fn is_beta(app_id: &str) -> bool {
    file(app_id).and_then(|path| std::fs::read_to_string(path).ok()).is_some_and(|text| text.trim() == "beta")
}

/// Follows the beta channel (or the stable one).
pub fn set(app_id: &str, beta: bool) -> Result<()> {
    let path = file(app_id).context("your home folder isn't known")?;
    if !beta {
        forget(app_id);
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("couldn't create {}", parent.display()))?;
    }
    std::fs::write(&path, "beta\n").with_context(|| format!("couldn't write {}", path.display()))
}

/// Forgets the choice (removing the app does).
pub fn forget(app_id: &str) {
    if let Some(path) = file(app_id) {
        let _ = std::fs::remove_file(path);
    }
}
