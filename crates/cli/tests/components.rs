//! `installer.shared_files = "all"` through the real `bach` and runtime: a file
//! that several components claim is installed only when all of them are ticked.
//!
//! The install test needs a real install, so it is ignored by default
//! (`cargo test -p bach-cli --test components -- --ignored`): it installs a
//! throwaway app into a temp folder, updates it and removes it.

mod common;
use common::{config, project, run, text};
#[cfg(windows)]
use std::path::Path;
use std::path::PathBuf;

const SHARED: &str = r#"
[installer]
shared_files = "all"

[[components]]
id = "tools"
title = "Tools"
files = ["vendor*/tools/**"]

[[components]]
id = "vendor1"
title = "Vendor 1"
files = ["vendor1/**"]
default = true

[[components]]
id = "vendor2"
title = "Vendor 2"
files = ["vendor2/**"]
"#;

fn shared_project(name: &str, id: &str) -> PathBuf {
    let dir = project(name, &config(SHARED).replace("BachIntegrationTest-Hello", id));
    for vendor in ["vendor1", "vendor2"] {
        std::fs::create_dir_all(dir.join("app").join(vendor).join("tools")).unwrap();
        std::fs::write(dir.join("app").join(vendor).join("app.txt"), vendor).unwrap();
        std::fs::write(dir.join("app").join(vendor).join("tools/tool.txt"), vendor).unwrap();
    }
    dir
}

#[test]
fn check_build_inspect_and_verify_know_every_claim() {
    need_runtime!();
    let dir = shared_project("shared-build", "BachSharedBuild");
    let checked = text(&run(&dir, &["check"]).stdout);
    assert!(checked.contains("vendor1/tools/tool.txt  (tools + vendor1)"), "{checked}");
    let built = run(&dir, &["build", "--fast"]);
    assert!(built.status.success(), "{}", text(&built.stderr));
    let setup = dir.join("dist/Hello-Setup-1.0.0.exe");
    let inspected = text(&run(&dir, &["inspect", setup.to_str().unwrap()]).stdout);
    assert!(inspected.contains("tools \"Tools\" (2 files)"), "{inspected}");
    assert!(inspected.contains("vendor2 \"Vendor 2\" (2 files)"), "{inspected}");
    let verified = run(&dir, &["verify", setup.to_str().unwrap()]);
    assert!(verified.status.success(), "{}", text(&verified.stderr));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_zip_of_the_defaults_leaves_out_what_an_unticked_component_also_claims() {
    need_runtime!();
    let dir = shared_project("shared-zip", "BachSharedZip");
    let mut toml = std::fs::read_to_string(dir.join("bach.toml")).unwrap();
    toml.push_str("\n[profiles.portable]\nkind = \"zip\"\n");
    std::fs::write(dir.join("bach.toml"), toml).unwrap();
    let built = run(&dir, &["build", "--fast", "--profile", "portable", "--json"]);
    assert!(built.status.success(), "{}\n{}", text(&built.stdout), text(&built.stderr));
    let zip = std::fs::read_dir(dir.join("dist"))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|extension| extension == "zip"))
        .expect("a ZIP");
    let archive = zip::ZipArchive::new(std::fs::File::open(&zip).unwrap()).unwrap();
    let mut names: Vec<String> = archive.file_names().map(|name| name.unwrap().into_owned()).collect();
    names.sort_unstable();
    // Only vendor1 is ticked by default; its tools need `tools` too.
    assert_eq!(names, ["data.bin", "hello.exe", "vendor1/app.txt"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Removes the throwaway app whatever happens in the test.
#[cfg(windows)]
struct Cleanup(PathBuf);

#[cfg(windows)]
impl Drop for Cleanup {
    fn drop(&mut self) {
        if self.0.is_file() {
            let _ = std::process::Command::new(&self.0).args(["--silent", "--delete-data"]).status();
            std::thread::sleep(std::time::Duration::from_secs(4));
        }
    }
}

#[cfg(windows)]
fn silent(setup: &Path, args: &[&str]) {
    let done = std::process::Command::new(setup).args(["--silent", "--lang", "en"]).args(args).output().unwrap();
    assert!(done.status.success(), "{}", text(&done.stderr));
}

#[test]
#[ignore = "installs and removes a throwaway app on this computer"]
#[cfg(windows)]
fn a_silent_install_adds_and_removes_shared_files_with_the_boxes() {
    need_runtime!();
    let id = "BachSharedInstall";
    let dir = shared_project("shared-install", id);
    assert!(run(&dir, &["build", "--fast"]).status.success());
    let setup = dir.join("dist/Hello-Setup-1.0.0.exe");
    let install = dir.join("Installed").join(id);
    let target = install.to_str().unwrap();
    let cleanup = Cleanup(install.join("uninstall.exe"));
    let there = |path: &str| install.join(path).is_file();

    silent(&setup, &["--dir", target]);
    assert!(there("vendor1/app.txt") && !there("vendor1/tools/tool.txt") && !there("vendor2/app.txt"));
    // One switch adds the tools of every ticked vendor, and only of those.
    silent(&setup, &["--dir", target, "--with", "tools"]);
    assert!(there("vendor1/tools/tool.txt") && !there("vendor2/tools/tool.txt"));
    silent(&setup, &["--dir", target, "--with", "vendor2"]);
    assert!(there("vendor1/tools/tool.txt") && there("vendor2/tools/tool.txt"));
    // ...and takes them away everywhere at once.
    silent(&setup, &["--dir", target, "--without", "tools"]);
    assert!(there("vendor1/app.txt") && there("vendor2/app.txt"));
    assert!(!there("vendor1/tools/tool.txt") && !there("vendor2/tools/tool.txt"));
    drop(cleanup);
    assert!(!there("hello.exe") && !there("vendor1/app.txt"));
    std::fs::remove_dir_all(&dir).unwrap();
}
