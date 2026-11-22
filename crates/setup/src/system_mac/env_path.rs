//! The user's `PATH`, on a Mac a block in the shell's login profile.
//!
//! `PATH` belongs to the user and to every program they installed, so setup edits
//! it with care. It adds one marked block per folder to `~/.zprofile` (what Terminal's
//! default shell reads) and to `~/.bash_profile` when that exists, and takes away
//! exactly that block again. Everything else in those files stays byte for byte as it was,
//! and nothing is written when there is nothing to change.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use bach_pack::manifest::Scope;

/// The files a folder is added to: always `.zprofile`, and `.bash_profile` for bash users.
fn profiles() -> Result<Vec<(PathBuf, bool)>> {
    let home = super::home()?;
    Ok(vec![(home.join(".zprofile"), true), (home.join(".bash_profile"), false)])
}

fn start(folder: &str) -> String {
    format!("# >>> bach path {folder} >>>")
}

fn end(folder: &str) -> String {
    format!("# <<< bach path {folder} <<<")
}

/// A folder as it is written inside double quotes in a shell script.
fn shell_quote(folder: &str) -> String {
    let mut out = String::new();
    for c in folder.chars() {
        if matches!(c, '"' | '\\' | '$' | '`') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The text with the folder's block added; None when it is there already.
fn with_block(text: &str, folder: &str) -> Option<String> {
    if text.lines().any(|line| line == start(folder)) {
        return None;
    }
    let mut out = text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("{}\nexport PATH=\"{}:$PATH\"\n{}\n", start(folder), shell_quote(folder), end(folder)));
    Some(out)
}

/// The text without the folder's block; None when it isn't there.
fn without_block(text: &str, folder: &str) -> Option<String> {
    let (open, close) = (start(folder), end(folder));
    let mut out = String::new();
    let mut inside = false;
    let mut found = false;
    for line in text.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']);
        if !inside && bare == open {
            inside = true;
            found = true;
        } else if inside && bare == close {
            inside = false;
        } else if !inside {
            out.push_str(line);
        }
    }
    // An unfinished block (someone edited the file) is left exactly as it was.
    (found && !inside).then_some(out)
}

fn check(folder: &Path) -> Result<String> {
    let text = folder.to_string_lossy().into_owned();
    if text.contains(['\n', '\r', ':']) {
        bail!("{} contains a character PATH can't hold", folder.display());
    }
    Ok(text)
}

/// Adds `folder` to the `PATH` unless it's there already. The scope makes no
/// difference on a Mac: a machine-wide install is still on this user's `PATH`.
pub fn add(_scope: Scope, folder: &Path) -> Result<()> {
    let folder = check(folder)?;
    for (path, always) in profiles()? {
        let current = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound && always => String::new(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err).with_context(|| format!("couldn't read {}", path.display())),
        };
        if let Some(changed) = with_block(&current, &folder) {
            write_keeping_mode(&path, &changed)?;
        }
    }
    Ok(())
}

/// Takes `folder` off the `PATH`, if setup put it there.
pub fn remove(_scope: Scope, folder: &Path) -> Result<()> {
    let folder = check(folder)?;
    for (path, _) in profiles()? {
        let Ok(current) = std::fs::read_to_string(&path) else { continue };
        if let Some(changed) = without_block(&current, &folder) {
            write_keeping_mode(&path, &changed)?;
        }
    }
    Ok(())
}

fn write_keeping_mode(path: &Path, text: &str) -> Result<()> {
    // Written through a copy and renamed, so a crash never leaves half a profile.
    let partial = path.with_extension("bach-new");
    std::fs::write(&partial, text).with_context(|| format!("couldn't write {}", partial.display()))?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&partial, meta.permissions());
    }
    std::fs::rename(&partial, path).with_context(|| format!("couldn't update {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_is_added_once_and_taken_away_exactly() {
        let original = "export FOO=1\nalias ll='ls -l'";
        let added = with_block(original, "/Applications/Hello/bin").unwrap();
        assert!(added.contains("export PATH=\"/Applications/Hello/bin:$PATH\""));
        assert_eq!(with_block(&added, "/Applications/Hello/bin"), None, "no second block");
        let other = with_block(&added, "/Applications/Other").unwrap();
        let back = without_block(&other, "/Applications/Hello/bin").unwrap();
        assert!(back.contains("/Applications/Other") && !back.contains("Hello"));
        let gone = without_block(&without_block(&other, "/Applications/Hello/bin").unwrap(), "/Applications/Other").unwrap();
        assert_eq!(gone.trim_end(), original, "everything else stays as it was");
        assert_eq!(without_block(original, "/x"), None);
    }

    #[test]
    fn odd_folders_are_quoted_and_a_colon_is_refused() {
        let added = with_block("", "/Apps/$HOME \"x\"").unwrap();
        assert!(added.contains(r#"export PATH="/Apps/\$HOME \"x\":$PATH""#), "{added}");
        assert!(check(Path::new("/a:b")).is_err());
        assert!(check(Path::new("/a\nb")).is_err());
    }

    #[test]
    fn an_unfinished_block_is_left_alone() {
        let broken = "# >>> bach path /x >>>\nexport PATH=\"/x:$PATH\"\n";
        assert_eq!(without_block(broken, "/x"), None);
    }
}
