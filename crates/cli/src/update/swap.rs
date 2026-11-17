//! Putting a newer `bach` in place of the running one.
//!
//! Everything happens in a staging folder beside the program, so the last
//! step is a rename on the same disk. The download is checked against the
//! SHA-256 published next to it and the new program must report the version
//! that was asked for before anything is touched. The swap itself keeps the
//! old files until the new ones are in place and puts them back if a step
//! fails, so an update never leaves a half-replaced install.
//!
//! Windows won't let a running program be overwritten or deleted, but it can
//! be renamed. The old `bach.exe` is moved to `bach.exe.old`, and removed the
//! next time `bach` starts (see [`super::tidy`]).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use super::release::{Archive, Release};
use crate::version::Version;
use crate::{output, runtime};

/// Replaces the running `exe` (and the `bach-setup.exe` beside it, when the
/// release has one) with the release's files. `from` is a folder holding the
/// release's files instead of GitHub (`BACH_UPDATE_FROM`, for tests).
pub fn apply(release: &Release, exe: &Path, from: Option<&Path>) -> Result<()> {
    let dir = exe.parent().context("couldn't tell which folder bach is in")?;
    let stage = dir.join(format!(".bach-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir(&stage).with_context(|| {
        format!("couldn't write to {}; if Bach is installed for every user, run this from an administrator terminal", dir.display())
    })?;
    let result = stage_and_swap(release, exe, dir, &stage, from);
    let _ = std::fs::remove_dir_all(&stage);
    result
}

fn stage_and_swap(release: &Release, exe: &Path, dir: &Path, stage: &Path, from: Option<&Path>) -> Result<()> {
    let archive = release.archive()?;
    let file = stage.join(&archive.name);
    fetch(&archive, &file, from)?;

    let unpacked = stage.join("unpacked");
    extract(&file, &unpacked)?;

    let program = format!("bach{}", std::env::consts::EXE_SUFFIX);
    let new_bach = find(&unpacked, &program).with_context(|| format!("the download has no {program}; nothing was changed"))?;
    check_reports(&new_bach, &release.version)?;
    let mut pairs = vec![(new_bach, exe.to_path_buf())];
    if let Some(setup) = find(&unpacked, runtime::FILE_NAME) {
        runtime::read(&setup).context("the downloaded setup runtime isn't usable; nothing was changed")?;
        pairs.push((setup, dir.join(runtime::FILE_NAME)));
    }
    swap(&pairs)
}

/// Downloads the archive and its checksum (or copies them from `from`), and
/// stops unless they agree.
fn fetch(archive: &Archive, file: &Path, from: Option<&Path>) -> Result<()> {
    let sum = PathBuf::from(format!("{}.sha256", file.display()));
    match from {
        Some(folder) => {
            for name in [&archive.name, &format!("{}.sha256", archive.name)] {
                std::fs::copy(folder.join(name), file.with_file_name(name))
                    .with_context(|| format!("{} has no {name}", folder.display()))?;
            }
        }
        None => {
            say!("  Downloading {}…", archive.name);
            runtime::fetch(&archive.url, file)?;
            runtime::fetch(&format!("{}.sha256", archive.url), &sum)?;
        }
    }
    let published = std::fs::read_to_string(&sum).context("couldn't read the published checksum")?;
    let expected = published.split_whitespace().next().unwrap_or_default().to_ascii_lowercase();
    let actual = runtime::sha256_of(&std::fs::read(file).context("couldn't read the download")?);
    if expected.len() != 64 || expected != actual {
        bail!("the download doesn't match its published SHA-256, so nothing was changed");
    }
    Ok(())
}

/// Unpacks a `.zip` or `.tar.gz` with the system's own `tar` (Windows 10 and
/// later ship one that reads zip files), or PowerShell on a Windows without it.
fn extract(archive: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir(to)?;
    let tar = if cfg!(windows) {
        // The system's, by full path: a `tar` earlier on the PATH may not read zips.
        let system = std::env::var_os("SystemRoot").map(|root| PathBuf::from(root).join("System32").join("tar.exe"));
        system.filter(|tar| tar.is_file())
    } else {
        Some(PathBuf::from("tar"))
    };
    let status = match tar {
        Some(tar) => Command::new(tar).arg("-xf").arg(archive).arg("-C").arg(to).status(),
        None => Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Expand-Archive -LiteralPath $env:BACH_ARCHIVE -DestinationPath $env:BACH_DEST",
            ])
            .env("BACH_ARCHIVE", archive)
            .env("BACH_DEST", to)
            .status(),
    };
    match status {
        Ok(status) if status.success() => Ok(()),
        _ => bail!("the download isn't a readable archive; nothing was changed"),
    }
}

/// The file called `name` in the unpacked folder or one folder below it
/// (releases put their files in a folder named after the release). Links
/// don't count.
fn find(root: &Path, name: &str) -> Option<PathBuf> {
    let is_file = |path: &Path| std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file());
    let direct = root.join(name);
    if is_file(&direct) {
        return Some(direct);
    }
    std::fs::read_dir(root).ok()?.filter_map(Result::ok).map(|entry| entry.path().join(name)).find(|path| is_file(path))
}

/// Runs the downloaded program once, to see it is the version that was
/// asked for and that it starts on this computer.
fn check_reports(program: &Path, expected: &Version) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(program, std::fs::Permissions::from_mode(0o755))?;
    }
    let out = Command::new(program)
        .arg("--version")
        .output()
        .context("the downloaded bach doesn't start on this computer; nothing was changed")?;
    let text = String::from_utf8_lossy(&out.stdout);
    let reported = text.split_whitespace().last().and_then(Version::parse);
    if !out.status.success() || reported.as_ref() != Some(expected) {
        bail!("the downloaded bach says it is {:?}, not {expected}, so nothing was changed", text.trim());
    }
    Ok(())
}

/// Puts each new file in place of its target, keeping the old one beside it
/// until every file is in place, and putting the old ones back if one fails.
fn swap(pairs: &[(PathBuf, PathBuf)]) -> Result<()> {
    let mut done: Vec<PathBuf> = Vec::new();
    for (new, target) in pairs {
        if let Err(err) = replace(new, target) {
            for target in done.iter().rev() {
                let _ = std::fs::rename(backup_of(target), target);
            }
            return Err(err);
        }
        done.push(target.clone());
    }
    for target in &done {
        // Still running (Windows): it goes the next time bach starts.
        let _ = std::fs::remove_file(backup_of(target));
    }
    Ok(())
}

fn replace(new: &Path, target: &Path) -> Result<()> {
    let backup = backup_of(target);
    let _ = std::fs::remove_file(&backup);
    let had_old = target.exists();
    if had_old {
        std::fs::rename(target, &backup)
            .with_context(|| format!("couldn't move {} out of the way; nothing was changed", target.display()))?;
    }
    if let Err(err) = std::fs::rename(new, target) {
        if had_old {
            let _ = std::fs::rename(&backup, target);
        }
        return Err(err).with_context(|| format!("couldn't put the new {} in place; the old one is back", target.display()));
    }
    output::field("Updated", target.display());
    Ok(())
}

/// `bach.exe` → `bach.exe.old`.
pub fn backup_of(target: &Path) -> PathBuf {
    let mut name = target.as_os_str().to_owned();
    name.push(".old");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bach-swap-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn files_are_found_in_the_release_folder() {
        let dir = folder("find");
        std::fs::create_dir_all(dir.join("bach-0.8.0-linux-x64")).unwrap();
        std::fs::write(dir.join("bach-0.8.0-linux-x64").join("bach"), b"x").unwrap();
        assert!(find(&dir, "bach").is_some());
        assert!(find(&dir, "bach-setup.exe").is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_swap_replaces_and_cleans_up() {
        let dir = folder("ok");
        std::fs::write(dir.join("a"), b"old a").unwrap();
        std::fs::write(dir.join("a.new"), b"new a").unwrap();
        std::fs::write(dir.join("b.new"), b"new b").unwrap();
        swap(&[(dir.join("a.new"), dir.join("a")), (dir.join("b.new"), dir.join("b"))]).unwrap();
        assert_eq!(std::fs::read(dir.join("a")).unwrap(), b"new a");
        assert_eq!(std::fs::read(dir.join("b")).unwrap(), b"new b");
        assert!(!backup_of(&dir.join("a")).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_swap_puts_the_old_files_back() {
        let dir = folder("rollback");
        std::fs::write(dir.join("a"), b"old a").unwrap();
        std::fs::write(dir.join("a.new"), b"new a").unwrap();
        // The second new file doesn't exist, so the second step fails.
        let result = swap(&[(dir.join("a.new"), dir.join("a")), (dir.join("missing"), dir.join("b"))]);
        assert!(result.is_err());
        assert_eq!(std::fs::read(dir.join("a")).unwrap(), b"old a", "the first file is back as it was");
        assert!(!dir.join("b").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
