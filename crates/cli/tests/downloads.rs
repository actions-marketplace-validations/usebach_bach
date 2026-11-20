//! Components that are downloaded when they are ticked.
//!
//! The build side is tested everywhere. The install side needs a real install,
//! so that test is ignored by default (`cargo test -p bach-cli --test downloads
//! -- --ignored`): it installs a throwaway app into a temp folder and removes
//! it. The download itself is replaced by putting the archive where setup keeps
//! a finished download, so no network is needed; setup checks it like a download.

mod common;
use common::{config, project, run, text};
use std::path::{Path, PathBuf};

const COMPONENT: &str = r#"
[[components]]
id = "hd"
title = "HD textures"
files = ["hd/**"]
download = "https://bach-test.invalid/{file}"
"#;

/// A project whose `hd` component holds 600 KB of data that doesn't compress.
fn downloads_project(name: &str, id: &str) -> PathBuf {
    let toml = config(COMPONENT).replace("BachIntegrationTest-Hello", id);
    let dir = project(name, &toml);
    std::fs::create_dir_all(dir.join("app/hd")).unwrap();
    let mut state = 0x1234_5678u32;
    let noise: Vec<u8> = (0..600_000)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect();
    std::fs::write(dir.join("app/hd/big.bin"), noise).unwrap();
    std::fs::write(dir.join("app/hd/small.txt"), "small").unwrap();
    dir
}

#[test]
fn a_downloaded_component_is_packed_on_its_own_and_recorded_in_the_setup_file() {
    need_runtime!();
    let dir = downloads_project("downloads-build", "BachDownloadsBuild");
    let out = run(&dir, &["build", "--fast", "--json"]);
    assert!(out.status.success(), "{}\n{}", text(&out.stdout), text(&out.stderr));
    let object: serde_json::Value = serde_json::from_str(&text(&out.stdout)).unwrap();
    let setup = dir.join("dist/Hello-Setup-1.0.0.exe");
    let pack = dir.join("dist/Hello-hd-1.0.0.bachpack");
    assert!(setup.is_file() && pack.is_file(), "{object}");
    assert_eq!(object["packs"][0]["component"], "hd");
    assert_eq!(object["packs"][0]["size"].as_u64(), Some(std::fs::metadata(&pack).unwrap().len()));

    // The setup file doesn't carry the component: the 600 KB of data is in the other file.
    let with_files = {
        std::fs::write(dir.join("plain.toml"), config("")).unwrap();
        let out = run(&dir, &["build", "plain.toml", "--fast", "--out", "plain/Setup.exe"]);
        assert!(out.status.success(), "{}", text(&out.stderr));
        std::fs::metadata(dir.join("plain/Setup.exe")).unwrap().len()
    };
    let without = std::fs::metadata(&setup).unwrap().len();
    assert!(with_files > without + 500_000, "setup file {without} bytes, with the files packed {with_files}");
    assert!(std::fs::metadata(&pack).unwrap().len() > 500_000);

    // The manifest names the archive by link, SHA-256 and size; its files are still listed.
    let inspected = text(&run(&dir, &["inspect", setup.to_str().unwrap()]).stdout);
    assert!(inspected.contains("hd \"HD textures\" (2 files, downloaded:"), "{inspected}");
    let bytes = std::fs::read(&pack).unwrap();
    assert_eq!(object["packs"][0]["sha256"].as_str().unwrap(), bach_pack::protect::sha256_hex(&bytes[..]).unwrap());
    let mut file = std::fs::File::open(&setup).unwrap();
    let layout = bach_pack::container::locate(&mut file).unwrap().unwrap();
    let manifest = bach_pack::container::read_manifest(&mut file, &layout).unwrap();
    let download = manifest.components[0].download.as_ref().unwrap();
    assert_eq!(download.url, "https://bach-test.invalid/Hello-hd-1.0.0.bachpack");
    assert_eq!(download.sha256, object["packs"][0]["sha256"].as_str().unwrap());
    assert_eq!(manifest.files.iter().filter(|file| file.component.as_deref() == Some("hd")).count(), 2);
    assert!(manifest.files.iter().all(|file| file.sha256.is_some()), "every file, downloaded or not, has its checksum");

    // The setup file is still sound, and the archive next to it is checked against what it records.
    let verified = run(&dir, &["verify", setup.to_str().unwrap()]);
    assert!(verified.status.success(), "{}", text(&verified.stderr));
    assert!(text(&verified.stdout).contains("hd: Hello-hd-1.0.0.bachpack matches the SHA-256"), "{}", text(&verified.stdout));
    // A changed archive is found.
    let mut damaged = std::fs::read(&pack).unwrap();
    let last = damaged.len() - 1;
    damaged[last] ^= 1;
    std::fs::write(&pack, &damaged).unwrap();
    let bad = run(&dir, &["verify", setup.to_str().unwrap()]);
    assert!(!bad.status.success() && text(&bad.stderr).contains("doesn't match the SHA-256 and size"), "{}", text(&bad.stderr));
    std::fs::write(&pack, &bytes).unwrap();
    let mut found = Vec::new();
    bach_pack::archive::read(std::io::BufReader::new(std::fs::File::open(&pack).unwrap()), &mut |path, size, _| {
        found.push((path.to_string(), size));
        Ok(())
    })
    .unwrap();
    found.sort();
    assert_eq!(found, [("hd/big.bin".to_string(), 600_000), ("hd/small.txt".to_string(), 5)]);

    // A profile and an architecture name their own archives.
    let more =
        format!("{}\n[profiles.enterprise.installer]\nscope = \"machine\"\n", std::fs::read_to_string(dir.join("bach.toml")).unwrap());
    std::fs::write(dir.join("bach.toml"), more).unwrap();
    assert!(run(&dir, &["build", "--fast", "--profile", "enterprise"]).status.success());
    assert!(dir.join("dist/Hello-hd-1.0.0-enterprise.bachpack").is_file());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_component_that_cant_be_downloaded_is_refused_when_it_cant_be_checked() {
    need_runtime!();
    let dir = downloads_project("downloads-refused", "BachDownloadsRefused");
    let broken = std::fs::read_to_string(dir.join("bach.toml"))
        .unwrap()
        .replace("https://bach-test.invalid/{file}", "http://bach-test.invalid/{file}");
    std::fs::write(dir.join("bach.toml"), broken).unwrap();
    let out = run(&dir, &["check"]);
    assert!(text(&out.stderr).contains("must be an https link"), "{}", text(&out.stderr));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Removes the throwaway app whatever happens in the test.
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
#[ignore = "installs and removes a throwaway app on this computer"]
#[cfg(windows)]
fn a_silent_install_takes_the_files_from_the_downloaded_archive() {
    need_runtime!();
    let id = "BachDownloadsInstall";
    let dir = downloads_project("downloads-install", id);
    assert!(run(&dir, &["build", "--fast", "--json"]).status.success());
    let setup = dir.join("dist/Hello-Setup-1.0.0.exe");
    let pack = dir.join("dist/Hello-hd-1.0.0.bachpack");
    let mut file = std::fs::File::open(&setup).unwrap();
    let layout = bach_pack::container::locate(&mut file).unwrap().unwrap();
    let manifest = bach_pack::container::read_manifest(&mut file, &layout).unwrap();
    let download = manifest.components[0].download.clone().unwrap();

    // Where setup keeps a finished download: named by the component and the start of the archive's hash.
    let cache = std::env::temp_dir().join(format!("{id}-components"));
    let cached = cache.join(format!("hd-{}.bachpack", &download.sha256[..16]));
    let install = dir.join("Installed").join(id);
    let mut cleanup = Cleanup { uninstaller: install.join("uninstall.exe"), folders: vec![cache.clone()] };

    // Without the component: nothing is downloaded (the link doesn't even exist) and nothing of it is installed.
    let plain = silent(&setup, &["--dir", install.to_str().unwrap()]);
    assert!(plain.status.success(), "{}", text(&plain.stderr));
    assert!(install.join("hello.exe").is_file() && install.join("data.bin").is_file());
    assert!(!install.join("hd").exists(), "an unticked component is never installed");
    let removed = std::process::Command::new(install.join("uninstall.exe")).args(["--silent"]).status().unwrap();
    assert!(removed.success());
    std::thread::sleep(std::time::Duration::from_secs(4));
    assert!(!install.join("hello.exe").exists());

    // With it, and a wrong archive in the cache: refused, because it isn't the file the setup file names.
    std::fs::create_dir_all(&cache).unwrap();
    let mut wrong = std::fs::read(&pack).unwrap();
    let last = wrong.len() - 1;
    wrong[last] ^= 0xFF;
    std::fs::write(&cached, &wrong).unwrap();
    let refused = silent(&setup, &["--dir", install.to_str().unwrap(), "--with", "hd"]);
    assert_eq!(refused.status.code(), Some(1), "{}", text(&refused.stderr));
    assert!(text(&refused.stderr).contains("Couldn't download HD textures"), "{}", text(&refused.stderr));
    assert!(!install.join("hello.exe").exists(), "a failed download changes nothing: the app isn't half installed");

    // The right archive: installed, byte for byte.
    std::fs::write(&cached, std::fs::read(&pack).unwrap()).unwrap();
    let done = silent(&setup, &["--dir", install.to_str().unwrap(), "--with", "hd"]);
    assert!(done.status.success(), "{}", text(&done.stderr));
    assert_eq!(std::fs::read(install.join("hd/big.bin")).unwrap(), std::fs::read(dir.join("app/hd/big.bin")).unwrap());
    assert_eq!(std::fs::read_to_string(install.join("hd/small.txt")).unwrap(), "small");
    assert!(install.join("hello.exe").is_file());
    assert!(!cached.exists(), "the downloaded archive is deleted once the install is done");

    // The uninstaller knows the component's files and removes them with the rest.
    let removed = std::process::Command::new(install.join("uninstall.exe")).args(["--silent"]).status().unwrap();
    assert!(removed.success());
    std::thread::sleep(std::time::Duration::from_secs(4));
    assert!(!install.join("hd/big.bin").exists() && !install.join("hello.exe").exists());
    cleanup.folders.push(dir.clone());
}
