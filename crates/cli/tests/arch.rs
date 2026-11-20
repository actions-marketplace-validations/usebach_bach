//! One feed, two architectures: `[arch.arm64]` builds an ARM64 setup file next
//! to the x64 one and the update feed lists both.
//!
//! There is no ARM64 runtime on an x64 build machine, so these tests make one
//! by setting the machine type in a copy of the x64 runtime. It is never run.

mod common;
use common::{config, project, run, runtime, text};
use std::path::{Path, PathBuf};

const UPDATES: &str =
    "[updates]\nfeed = \"https://bach-test.invalid/hello/latest.json\"\ndownload_url = \"https://bach-test.invalid/hello/{file}\"\n";
const ARCH: &str = "[arch.arm64]\nsource = \"app-arm64\"\n";

/// A copy of the x64 runtime whose PE header says ARM64.
fn fake_arm_runtime(dir: &Path) -> PathBuf {
    let mut bytes = std::fs::read(runtime().unwrap()).unwrap();
    let pe = u32::from_le_bytes(bytes[0x3c..0x40].try_into().unwrap()) as usize;
    bytes[pe + 4..pe + 6].copy_from_slice(&0xAA64u16.to_le_bytes());
    let path = dir.join("arm64-bach-setup.exe");
    std::fs::write(&path, bytes).unwrap();
    path
}

fn arm_project(name: &str, extra: &str) -> PathBuf {
    let dir = project(name, &config(&format!("{extra}\n{ARCH}")));
    std::fs::create_dir_all(dir.join("app-arm64")).unwrap();
    std::fs::write(dir.join("app-arm64/hello.exe"), b"MZ-arm64").unwrap();
    std::fs::write(dir.join("app-arm64/data.bin"), vec![7u8; 4000]).unwrap();
    dir
}

fn machine_of(path: &Path) -> u16 {
    let bytes = std::fs::read(path).unwrap();
    let pe = u32::from_le_bytes(bytes[0x3c..0x40].try_into().unwrap()) as usize;
    u16::from_le_bytes(bytes[pe + 4..pe + 6].try_into().unwrap())
}

#[test]
fn a_build_writes_both_setup_files_and_one_feed_that_lists_both() {
    need_runtime!();
    let dir = arm_project("arch-both", UPDATES);
    let arm = fake_arm_runtime(&dir);
    let out = run(&dir, &["build", "--fast", "--runtime-arm64", arm.to_str().unwrap(), "--json"]);
    assert!(out.status.success(), "{}\n{}", text(&out.stdout), text(&out.stderr));
    let object: serde_json::Value = serde_json::from_str(&text(&out.stdout)).unwrap();
    let (x64, arm64) = (dir.join("dist/Hello-Setup-1.0.0.exe"), dir.join("dist/Hello-Setup-1.0.0-arm64.exe"));
    assert!(x64.is_file() && arm64.is_file(), "{object}");
    assert_eq!(object["setup_arm64"].as_str().map(|path| path.ends_with("Hello-Setup-1.0.0-arm64.exe")), Some(true), "{object}");
    assert_ne!(machine_of(&x64), 0xAA64);
    assert_eq!(machine_of(&arm64), 0xAA64, "the ARM64 file is built on the ARM64 runtime");

    // Same app and version, different files.
    let x64_info = text(&run(&dir, &["inspect", x64.to_str().unwrap(), "--json"]).stdout);
    let arm_info = text(&run(&dir, &["inspect", arm64.to_str().unwrap(), "--json"]).stdout);
    let (x64_info, arm_info): (serde_json::Value, serde_json::Value) =
        (serde_json::from_str(&x64_info).unwrap(), serde_json::from_str(&arm_info).unwrap());
    assert_eq!((&x64_info["app_id"], &x64_info["version"]), (&arm_info["app_id"], &arm_info["version"]));
    assert_ne!(x64_info["size"], arm_info["size"], "{x64_info} {arm_info}");

    // The feed lists the ARM64 file, with its real hash and size.
    let feed: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("dist/latest.json")).unwrap()).unwrap();
    assert!(feed["url"].as_str().unwrap().ends_with("/Hello-Setup-1.0.0.exe"), "{feed}");
    let listed = &feed["arch"]["arm64"];
    assert!(listed["url"].as_str().unwrap().ends_with("/Hello-Setup-1.0.0-arm64.exe"), "{feed}");
    let bytes = std::fs::read(&arm64).unwrap();
    assert_eq!(listed["size"].as_u64(), Some(bytes.len() as u64));
    assert_eq!(listed["sha256"].as_str().unwrap(), bach_pack::protect::sha256_hex(&bytes[..]).unwrap());
    assert!(feed.get("signature").is_none(), "no publisher key here, so nothing is signed");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_arm64_runtime_is_required_and_checked() {
    need_runtime!();
    let dir = arm_project("arch-runtime", UPDATES);
    let out = run(&dir, &["build", "--fast"]);
    assert!(!out.status.success());
    let said = text(&out.stderr);
    assert!(said.contains("--runtime-arm64") && said.contains("--x64-only") && said.contains("windows-arm64.zip"), "{said}");
    // The x64 runtime isn't an ARM64 program.
    let out = run(&dir, &["build", "--fast", "--runtime-arm64", runtime().unwrap().to_str().unwrap()]);
    assert!(text(&out.stderr).contains("isn't an ARM64 program"), "{}", text(&out.stderr));
    let out = run(&dir, &["build", "--fast", "--runtime-arm64", "nowhere.exe"]);
    assert!(text(&out.stderr).contains("doesn't exist"), "{}", text(&out.stderr));
    // --x64-only builds the x64 file and leaves the feed without a listing.
    assert!(run(&dir, &["build", "--fast", "--x64-only"]).status.success());
    assert!(!dir.join("dist/Hello-Setup-1.0.0-arm64.exe").exists());
    let feed = std::fs::read_to_string(dir.join("dist/latest.json")).unwrap();
    assert!(!feed.contains("arm64"), "{feed}");
    // The runtime can be named in bach.toml, relative to it.
    let named = std::fs::read_to_string(dir.join("bach.toml"))
        .unwrap()
        .replace("source = \"app-arm64\"", "source = \"app-arm64\"\nruntime = \"arm64-bach-setup.exe\"");
    fake_arm_runtime(&dir);
    std::fs::write(dir.join("bach.toml"), named).unwrap();
    assert!(run(&dir, &["build", "--fast"]).status.success());
    assert!(dir.join("dist/Hello-Setup-1.0.0-arm64.exe").is_file());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_online_installer_is_built_once_and_the_listing_is_signed() {
    need_runtime!();
    let (private, public) = bach_pack::feedsig::generate().unwrap();
    let dir = arm_project("arch-signed", &format!("{UPDATES}public_key = \"{public}\"\n"));
    let arm = fake_arm_runtime(&dir);
    let mut command = common::bach();
    let out = command
        .current_dir(&dir)
        .env("BACH_RUNTIME", runtime().unwrap())
        .env("BACH_FEED_KEY", &private)
        .args(["build", "--fast", "--online", "--runtime-arm64", arm.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}\n{}", text(&out.stdout), text(&out.stderr));
    assert!(dir.join("dist/Hello-Web-Setup.exe").is_file());
    assert!(!dir.join("dist/Hello-Web-Setup-arm64.exe").exists(), "the online installer picks the architecture itself");
    let feed: bach_pack::manifest::FeedEntry =
        serde_json::from_str(&std::fs::read_to_string(dir.join("dist/latest.json")).unwrap()).unwrap();
    bach_pack::feedsig::verify(&public, "BachIntegrationTest-Hello", &feed).unwrap();
    let listed = &feed.arch["arm64"];
    bach_pack::feedsig::verify_arch(&public, "BachIntegrationTest-Hello", "1.0.0", "arm64", listed).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn bach_feed_can_list_an_arm64_file_and_refuses_the_wrong_one() {
    need_runtime!();
    let dir = arm_project("arch-feed", UPDATES);
    let arm = fake_arm_runtime(&dir);
    assert!(run(&dir, &["build", "--fast", "--runtime-arm64", arm.to_str().unwrap()]).status.success());
    let x64 = "dist/Hello-Setup-1.0.0.exe";
    let arm64 = "dist/Hello-Setup-1.0.0-arm64.exe";
    let out = run(&dir, &["feed", x64, "--arm64", arm64, "--out", "feed.json"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("arm64 setup file"), "{}", text(&out.stdout));
    let feed: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("feed.json")).unwrap()).unwrap();
    assert!(feed["arch"]["arm64"]["url"].as_str().unwrap().ends_with("Hello-Setup-1.0.0-arm64.exe"));

    // An x64 file isn't an ARM64 one; another version isn't this version; a link must be https.
    let out = run(&dir, &["feed", x64, "--arm64", x64]);
    assert!(text(&out.stderr).contains("isn't an ARM64 setup file"), "{}", text(&out.stderr));
    std::fs::write(dir.join("app/notes.txt"), "x").unwrap();
    assert!(run(&dir, &["build", "--fast", "--x64-only", "--app-version", "2.0.0"]).status.success());
    let out = run(&dir, &["feed", "dist/Hello-Setup-2.0.0.exe", "--arm64", arm64]);
    assert!(text(&out.stderr).contains("build both from one bach.toml and version"), "{}", text(&out.stderr));
    let out = run(&dir, &["feed", x64, "--arm64", arm64, "--arm64-url", "http://x/a.exe"]);
    assert!(text(&out.stderr).contains("must be an https link"), "{}", text(&out.stderr));
    std::fs::remove_dir_all(&dir).unwrap();
}
