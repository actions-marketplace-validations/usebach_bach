//! Binary deltas in update patches: a changed big file costs a few kilobytes in
//! the patch, not the whole file.
//!
//! Building and checking a patch is tested everywhere. Applying one is a real
//! update, so that test is ignored by default (`cargo test -p bach-cli --test
//! deltas -- --ignored`): it installs a throwaway app into a temp folder,
//! updates it with the patch and removes it.

mod common;
use common::{config, project, run, text};
use std::path::{Path, PathBuf};

fn program(len: usize, seed: u32) -> Vec<u8> {
    let mut state = seed;
    (0..len)
        .map(|index| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            if index % 7 == 0 {
                (index % 251) as u8
            } else {
                (state >> 24) as u8
            }
        })
        .collect()
}

/// The size of the archive inside a setup file, read from the payload's trailer.
fn archive_size(setup: &Path) -> u64 {
    let bytes = std::fs::read(setup).unwrap();
    let end = bytes.len();
    u64::from_le_bytes(bytes[end - 24..end - 16].try_into().unwrap())
}

/// A project whose big program changes a little between 1.0.0 and 1.1.0; leaves both setup files in `dist`.
fn two_versions(name: &str, id: &str) -> (PathBuf, Vec<u8>, Vec<u8>) {
    let dir = project(name, &config("").replace("BachIntegrationTest-Hello", id));
    let old = program(2_000_000, 11);
    let mut new = old.clone();
    new[1234] ^= 0xFF;
    new.splice(900_000..900_000, program(700, 12));
    std::fs::write(dir.join("app/big.dll"), &old).unwrap();
    std::fs::remove_file(dir.join("app/data.bin")).unwrap();
    assert!(run(&dir, &["build", "--fast"]).status.success());
    std::fs::copy(dir.join("dist/Hello-Setup-1.0.0.exe"), dir.join("old.exe")).unwrap();
    std::fs::write(dir.join("app/big.dll"), &new).unwrap();
    std::fs::write(dir.join("app/notes.txt"), "second").unwrap();
    (dir, old, new)
}

#[test]
fn a_patch_holds_what_changed_in_a_big_file_not_the_file() {
    need_runtime!();
    let (dir, _, _) = two_versions("deltas-size", "BachDeltasSize");
    let out = run(&dir, &["build", "--fast", "--app-version", "1.1.0", "--patch-from", "old.exe"]);
    assert!(out.status.success(), "{}\n{}", text(&out.stdout), text(&out.stderr));
    let patch = dir.join("dist/Hello-Setup-1.1.0-patch-from-1.0.0.exe");
    let full = dir.join("dist/Hello-Setup-1.1.0.exe");
    let (patch_archive, full_archive) = (archive_size(&patch), archive_size(&full));
    // The 2 MB program that changed by 700 bytes and 1 byte is a few KB in the patch.
    assert!(patch_archive < 20_000, "the patch's archive is {patch_archive} bytes");
    assert!(full_archive > 1_900_000, "the full archive is {full_archive} bytes");

    // The patch is still a sound setup file: its archive checks out, and the rest of it is verified.
    let verified = run(&dir, &["verify", patch.to_str().unwrap()]);
    assert!(verified.status.success(), "{}", text(&verified.stderr));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_file_that_changed_completely_goes_in_whole() {
    need_runtime!();
    let (dir, _, _) = two_versions("deltas-whole", "BachDeltasWhole");
    std::fs::write(dir.join("app/big.dll"), program(2_000_000, 99)).unwrap();
    let out = run(&dir, &["build", "--fast", "--app-version", "1.1.0", "--patch-from", "old.exe"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let patch = dir.join("dist/Hello-Setup-1.1.0-patch-from-1.0.0.exe");
    assert!(archive_size(&patch) > 1_900_000, "nothing to take from the old file, so it is stored whole");
    assert!(run(&dir, &["verify", patch.to_str().unwrap()]).status.success());
    std::fs::remove_dir_all(&dir).unwrap();
}

struct Cleanup {
    uninstaller: PathBuf,
    folders: Vec<PathBuf>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        if self.uninstaller.is_file() {
            let _ = std::process::Command::new(&self.uninstaller).args(["--silent", "--delete-data"]).status();
            std::thread::sleep(std::time::Duration::from_secs(4));
        }
        for folder in &self.folders {
            let _ = std::fs::remove_dir_all(folder);
        }
    }
}

fn silent(setup: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(setup).args(["--silent", "--lang", "en"]).args(args).output().unwrap()
}

#[test]
#[ignore = "installs, updates and removes a throwaway app on this computer"]
#[cfg(windows)]
fn a_patch_with_deltas_updates_an_installed_app_and_refuses_a_changed_base() {
    need_runtime!();
    let id = "BachDeltasInstall";
    let (dir, old, new) = two_versions("deltas-install", id);
    assert!(run(&dir, &["build", "--fast", "--app-version", "1.1.0", "--patch-from", "old.exe"]).status.success());
    let patch = dir.join("dist/Hello-Setup-1.1.0-patch-from-1.0.0.exe");
    let install = dir.join("Installed").join(id);
    let _cleanup = Cleanup { uninstaller: install.join("uninstall.exe"), folders: vec![dir.clone()] };

    let first = silent(&dir.join("old.exe"), &["--dir", install.to_str().unwrap()]);
    assert!(first.status.success(), "{}", text(&first.stderr));
    assert_eq!(std::fs::read(install.join("big.dll")).unwrap(), old);

    // A patch over a program somebody changed: it isn't the file the delta was made from, so nothing changes.
    let mut changed = old.clone();
    changed[5] ^= 1;
    std::fs::write(install.join("big.dll"), &changed).unwrap();
    let refused = silent(&patch, &[]);
    assert_eq!(refused.status.code(), Some(1), "{}", text(&refused.stderr));
    assert_eq!(std::fs::read(install.join("big.dll")).unwrap(), changed, "a refused update leaves the file as it was");
    assert!(!install.join("notes.txt").exists(), "and adds nothing");

    // Over the real earlier file the patch gives the new program, byte for byte.
    std::fs::write(install.join("big.dll"), &old).unwrap();
    let updated = silent(&patch, &[]);
    assert!(updated.status.success(), "{}", text(&updated.stderr));
    assert_eq!(std::fs::read(install.join("big.dll")).unwrap(), new);
    assert_eq!(std::fs::read_to_string(install.join("notes.txt")).unwrap(), "second");
}
