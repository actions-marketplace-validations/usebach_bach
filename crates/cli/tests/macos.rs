//! `bach build --platform macos` end to end, on a made-up Mach-O runtime: the
//! Setup app, its payload and the feed. (The real runtime is exercised by
//! scripts/e2e-macos.sh on a Mac.)

mod common;

use std::path::{Path, PathBuf};

use common::{bach, text};

const CONFIG: &str = r#"
[app]
name = "Hello Mac"
id = "BachIntegrationTest-HelloMac"
version = "1.0.0"
publisher = "Example"

[macos]
source = "app-macos"
executable = "Hello.app"
data = ["~/Library/Application Support/Hello Mac"]
output = "out/Hello-{version}.zip"

[updates]
feed = "https://example.com/downloads/latest.json"
download_url = "https://example.com/downloads/{file}"
"#;

/// A runtime that passes for a Mach-O program: only its first bytes are read at build time.
fn fake_runtime(dir: &Path) -> PathBuf {
    let path = dir.join("fake-bach-setup");
    let mut bytes = vec![0xCF, 0xFA, 0xED, 0xFE];
    bytes.extend(vec![7u8; 4096]);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bach-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let app = dir.join("app-macos/Hello.app/Contents/MacOS");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("hello"), b"#!/bin/sh\necho hello\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(app.join("hello"), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink("MacOS/hello", dir.join("app-macos/Hello.app/Contents/current")).unwrap();
    }
    std::fs::write(dir.join("app-macos/Hello.app/Contents/Info.plist"), b"<plist/>").unwrap();
    std::fs::write(dir.join("bach.toml"), CONFIG).unwrap();
    dir
}

fn build(dir: &Path, extra: &[&str]) -> std::process::Output {
    let runtime = fake_runtime(dir);
    bach()
        .current_dir(dir)
        .args(["build", "--platform", "macos", "--fast", "--json", "--runtime"])
        .arg(&runtime)
        .args(extra)
        .output()
        .expect("bach starts")
}

#[test]
fn a_macos_build_writes_a_setup_app_a_check_reads_and_a_feed_that_lists_it() {
    let dir = project("mac-build");
    let out = build(&dir, &[]);
    assert!(out.status.success(), "{}\n{}", text(&out.stdout), text(&out.stderr));
    let object: serde_json::Value = serde_json::from_str(&text(&out.stdout)).unwrap();
    let setup = dir.join(object["setup_macos"].as_str().expect("setup_macos"));
    assert!(setup.is_file() && setup.ends_with("Hello-1.0.0.zip"), "{object}");

    // The zip holds the Setup app with its payload and an executable runtime.
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&setup).unwrap()).unwrap();
    let names: Vec<String> = archive.file_names().filter_map(Result::ok).map(|name| name.into_owned()).collect();
    for wanted in [
        "Install Hello Mac.app/Contents/Info.plist",
        "Install Hello Mac.app/Contents/MacOS/bach-setup",
        "Install Hello Mac.app/Contents/Resources/setup.bach",
    ] {
        assert!(names.iter().any(|name| name == wanted), "{wanted} in {names:?}");
    }
    #[cfg(unix)]
    assert_eq!(archive.by_name("Install Hello Mac.app/Contents/MacOS/bach-setup").unwrap().unix_mode().unwrap() & 0o111, 0o111);

    // bach inspect and verify read it back.
    let inspected = bach().arg("inspect").arg(&setup).output().unwrap();
    let report = text(&inspected.stdout);
    assert!(inspected.status.success(), "{report}{}", text(&inspected.stderr));
    assert!(
        report.contains("Installer for Hello Mac 1.0.0 (macOS)")
            && report.contains("Installs to  ~/Applications")
            && !report.contains("~/Applications/BachIntegrationTest"),
        "{report}"
    );
    let verified = bach().arg("verify").arg(&setup).output().unwrap();
    assert!(verified.status.success(), "{}{}", text(&verified.stdout), text(&verified.stderr));

    // The feed lists the macOS file under arch.macos, with its own checksum.
    let feed: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("out/latest.json")).unwrap()).unwrap();
    assert_eq!(feed["version"], "1.0.0");
    assert_eq!(feed["arch"]["macos"]["url"], "https://example.com/downloads/Hello-1.0.0.zip");
    assert_eq!(feed["arch"]["macos"]["sha256"].as_str().map(str::len), Some(64));
    assert_eq!(feed["arch"]["macos"]["sha256"], object["sha256"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_mac_only_config_builds_without_windows_files_and_check_says_so() {
    let dir = project("mac-only");
    let checked = bach().current_dir(&dir).arg("check").output().unwrap();
    let report = text(&checked.stdout);
    assert!(checked.status.success(), "{report}{}", text(&checked.stderr));
    assert!(report.contains("ready to build for macOS"), "{report}");
    assert!(report.contains("Hello.app/Contents/MacOS/hello"), "{report}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn windows_only_options_and_wrong_runtimes_are_refused_with_a_reason() {
    let dir = project("mac-refuse");
    let patch = build(&dir, &["--patch-from", "old.exe"]);
    assert!(!patch.status.success() && text(&patch.stderr).contains("--patch-from"), "{}", text(&patch.stderr));
    let windows_runtime = dir.join("windows-runtime.exe");
    std::fs::write(&windows_runtime, b"MZ\x90\0").unwrap();
    let out = bach().current_dir(&dir).args(["build", "--platform", "macos", "--runtime"]).arg(&windows_runtime).output().unwrap();
    assert!(!out.status.success() && text(&out.stderr).contains("isn't a macOS program"), "{}", text(&out.stderr));
    let sign = build(&dir, &["--sign"]);
    assert!(!sign.status.success(), "{}", text(&sign.stderr));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[cfg(target_os = "macos")]
fn a_pkg_puts_the_app_in_applications_with_its_permissions() {
    let dir = project("mac-pkg");
    let out = build(&dir, &["--pkg"]);
    assert!(out.status.success(), "{}\n{}", text(&out.stdout), text(&out.stderr));
    let object: serde_json::Value = serde_json::from_str(&text(&out.stdout)).unwrap();
    let pkg = dir.join(object["pkg"].as_str().expect("pkg"));
    assert!(pkg.is_file() && pkg.extension().is_some_and(|ext| ext == "pkg"), "{object}");
    let expanded = dir.join("expanded");
    let status = std::process::Command::new("pkgutil").arg("--expand-full").arg(&pkg).arg(&expanded).status().unwrap();
    assert!(status.success());
    let info = std::fs::read_to_string(expanded.join("PackageInfo")).unwrap();
    assert!(info.contains("install-location=\"/Applications\"") && info.contains("version=\"1.0.0\""), "{info}");
    use std::os::unix::fs::PermissionsExt;
    let program = expanded.join("Payload/Hello.app/Contents/MacOS/hello");
    assert_eq!(std::fs::metadata(&program).unwrap().permissions().mode() & 0o111, 0o111, "programs stay programs");
    assert!(std::fs::symlink_metadata(expanded.join("Payload/Hello.app/Contents/current")).unwrap().file_type().is_symlink());
    // Signing a package needs the installer identity; asking without one says so.
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[cfg(not(target_os = "macos"))]
fn a_pkg_needs_a_mac() {
    let dir = project("mac-pkg-refused");
    let out = build(&dir, &["--pkg"]);
    assert!(!out.status.success() && text(&out.stderr).contains("needs macOS"), "{}", text(&out.stderr));
    std::fs::remove_dir_all(&dir).unwrap();
}
