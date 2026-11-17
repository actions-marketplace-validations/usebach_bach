//! `bach update`, end to end: a copy of the real binary replaces itself from
//! a folder that stands in for a GitHub release (`BACH_UPDATE_FROM`), so
//! nothing here touches the network.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use sha2::{Digest, Sha256};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const EXE: &str = if cfg!(windows) { "bach.exe" } else { "bach" };

fn real_bach() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_bach"))
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bach-it-update-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A folder holding what a release would: this platform's archive of the real
/// `bach` and its `.sha256`.
fn release_folder(dir: &Path) -> PathBuf {
    let pack = dir.join("pack");
    std::fs::create_dir_all(&pack).unwrap();
    std::fs::copy(real_bach(), pack.join(EXE)).unwrap();
    let release = dir.join("release");
    std::fs::create_dir_all(&release).unwrap();
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
    let (os, ext) = if cfg!(windows) {
        ("windows", "zip")
    } else if cfg!(target_os = "macos") {
        ("macos", "tar.gz")
    } else {
        ("linux", "tar.gz")
    };
    let name = format!("bach-{VERSION}-{os}-{arch}.{ext}");
    let archive = release.join(&name);
    let status = if cfg!(windows) {
        // The system's tar writes zip files; `-a` picks the format from the name.
        let tar = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32").join("tar.exe");
        Command::new(tar).arg("-a").arg("-cf").arg(&archive).arg("-C").arg(&pack).arg(EXE).status().unwrap()
    } else {
        Command::new("tar").arg("-czf").arg(&archive).arg("-C").arg(&pack).arg(EXE).status().unwrap()
    };
    assert!(status.success(), "the test archive couldn't be made");
    let hash = bach_pack::protect::hex(&Sha256::digest(std::fs::read(&archive).unwrap()));
    std::fs::write(release.join(format!("{name}.sha256")), format!("{hash}  {name}\n")).unwrap();
    release
}

/// A copy of `bach` in `folder`, made different from the release's by a few
/// trailing bytes (an executable ignores them), so a replacement is visible.
fn old_install(folder: &Path) -> PathBuf {
    std::fs::create_dir_all(folder).unwrap();
    let exe = folder.join(EXE);
    let mut bytes = std::fs::read(real_bach()).unwrap();
    bytes.extend_from_slice(b"old copy");
    std::fs::write(&exe, bytes).unwrap();
    #[cfg(unix)]
    {
        // A freshly written file isn't executable; an installed bach is.
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    exe
}

fn update(exe: &Path, release: &Path, args: &[&str]) -> Output {
    Command::new(exe).arg("update").args(args).env("BACH_UPDATE_FROM", release).output().unwrap()
}

fn size(path: &Path) -> u64 {
    std::fs::metadata(path).unwrap().len()
}

#[test]
fn a_standalone_bach_replaces_itself_from_a_release() {
    let dir = scratch("ok");
    let release = release_folder(&dir);
    let exe = old_install(&dir.join("install"));
    let original = size(&real_bach());
    assert_eq!(size(&exe), original + 8);

    // The same version is left alone unless it's asked for again.
    let same = update(&exe, &release, &[VERSION]);
    assert!(same.status.success() && text(&same.stdout).contains("already installed"), "{}", text(&same.stdout));
    assert_eq!(size(&exe), original + 8, "nothing changed");

    let out = update(&exe, &release, &[VERSION, "--force"]);
    assert!(out.status.success(), "{}{}", text(&out.stdout), text(&out.stderr));
    assert!(text(&out.stdout).contains(&format!("Bach {VERSION} is installed")), "{}", text(&out.stdout));
    assert_eq!(size(&exe), original, "the new copy is in place");

    // The next run (any run) clears the old copy and the staging folder is gone.
    let version = Command::new(&exe).arg("--version").output().unwrap();
    assert!(text(&version.stdout).contains(VERSION));
    assert!(!dir.join("install").join(format!("{EXE}.old")).exists(), "the old copy was removed");
    let leftovers: Vec<_> = std::fs::read_dir(dir.join("install")).unwrap().filter_map(Result::ok).map(|e| e.file_name()).collect();
    assert_eq!(leftovers.len(), 1, "only bach is left: {leftovers:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_download_that_doesnt_match_its_checksum_changes_nothing() {
    let dir = scratch("tampered");
    let release = release_folder(&dir);
    let sums: Vec<_> = std::fs::read_dir(&release)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".sha256"))
        .collect();
    std::fs::write(sums[0].path(), format!("{}  x\n", "0".repeat(64))).unwrap();
    let exe = old_install(&dir.join("install"));
    let before = std::fs::read(&exe).unwrap();

    let out = update(&exe, &release, &[VERSION, "--force"]);
    assert!(!out.status.success());
    assert!(text(&out.stderr).contains("doesn't match its published SHA-256"), "{}", text(&out.stderr));
    assert_eq!(std::fs::read(&exe).unwrap(), before, "the old bach is untouched");
    let names: Vec<_> = std::fs::read_dir(dir.join("install")).unwrap().filter_map(Result::ok).collect();
    assert_eq!(names.len(), 1, "the staging folder is gone");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_bach_that_a_package_manager_owns_is_left_to_it() {
    let dir = scratch("npm");
    let release = release_folder(&dir);
    let exe = old_install(&dir.join("node_modules").join("@usebach").join("cli-test").join("bin"));
    let before = std::fs::read(&exe).unwrap();

    let out = update(&exe, &release, &[VERSION, "--force"]);
    assert!(!out.status.success());
    assert!(text(&out.stderr).contains("npm update --global @usebach/cli"), "{}", text(&out.stderr));
    assert_eq!(std::fs::read(&exe).unwrap(), before);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn update_needs_a_real_version_and_says_what_is_wrong() {
    let dir = scratch("args");
    let release = release_folder(&dir);
    let exe = old_install(&dir.join("install"));
    let bad = update(&exe, &release, &["banana"]);
    assert!(!bad.status.success() && text(&bad.stderr).contains("isn't a version"), "{}", text(&bad.stderr));
    let missing = update(&exe, &release, &[]);
    assert!(!missing.status.success() && text(&missing.stderr).contains("BACH_UPDATE_FROM needs a version"), "{}", text(&missing.stderr));
    let both = update(&exe, &release, &["--check", "--force"]);
    assert!(!both.status.success(), "--check and --force don't go together");
    std::fs::remove_dir_all(&dir).unwrap();
}
