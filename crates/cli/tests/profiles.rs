//! Profiles: one config, several builds, on the real runtime.

mod common;
use common::{config, project, run, text};

const PROFILES: &str = r#"
[[components]]
id = "samples"
title = "Samples"
files = ["samples/**"]

[profiles.enterprise]
description = "For IT departments"

[profiles.enterprise.installer]
scope = "machine"
desktop_shortcut = false

[profiles.portable]
kind = "zip"

[[profiles.portable.components]]
id = "samples"
title = "Samples"
files = ["samples/**"]
default = true
"#;

fn setup(dir: &std::path::Path) -> std::path::PathBuf {
    let dir = dir.to_path_buf();
    std::fs::create_dir_all(dir.join("app/samples")).unwrap();
    std::fs::write(dir.join("app/samples/one.txt"), "sample").unwrap();
    dir
}

#[test]
fn profiles_are_listed_and_checked() {
    need_runtime!();
    let dir = setup(&project("profiles-list", &config(PROFILES)));
    let listed = text(&run(&dir, &["profiles"]).stdout);
    assert!(listed.contains("2 profiles") && listed.contains("enterprise  (setup)") && listed.contains("portable  (zip)"), "{listed}");
    assert!(listed.contains("writes dist/Hello-enterprise-Setup-1.0.0.exe"), "{listed}");
    assert!(listed.contains("writes dist/Hello-portable-1.0.0.zip") && listed.contains("For IT departments"), "{listed}");

    let json: serde_json::Value = serde_json::from_str(&text(&run(&dir, &["profiles", "--json"]).stdout)).unwrap();
    assert_eq!(json["profiles"][1]["kind"], "zip");

    let checked = text(&run(&dir, &["check", "--profile", "enterprise"]).stdout);
    assert!(checked.contains("ready to build, as the enterprise profile (setup)"), "{checked}");
    assert!(checked.contains("every user of the computer"), "the profile's scope shows: {checked}");

    let bad = run(&dir, &["check", "--profile", "nope"]);
    assert!(!bad.status.success() && text(&bad.stderr).contains("its profiles are enterprise, portable"), "{}", text(&bad.stderr));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_zip_profile_and_a_setup_profile_build_from_one_config() {
    need_runtime!();
    let dir = setup(&project("profiles-build", &config(PROFILES)));
    let out = run(&dir, &["build", "--fast", "--all-profiles"]);
    let said = text(&out.stdout);
    assert!(out.status.success(), "{}\n{}", said, text(&out.stderr));
    assert!(dir.join("dist/Hello-Setup-1.0.0.exe").is_file(), "{said}");
    assert!(dir.join("dist/Hello-enterprise-Setup-1.0.0.exe").is_file(), "{said}");
    let zip = dir.join("dist/Hello-portable-1.0.0.zip");
    assert!(zip.is_file(), "{said}");

    // The ZIP holds the files of the components its profile ticks, and unpacks to the same bytes.
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip).unwrap()).unwrap();
    let mut names: Vec<String> = archive.file_names().map(str::to_string).collect();
    names.sort();
    assert_eq!(names, ["data.bin", "hello.exe", "samples/one.txt"]);
    let mut data = Vec::new();
    std::io::Read::read_to_end(&mut archive.by_name("data.bin").unwrap(), &mut data).unwrap();
    assert_eq!(data, std::fs::read(dir.join("app/data.bin")).unwrap());

    // The main setup file leaves the samples out by default; the enterprise one is machine-wide.
    let main = text(&run(&dir, &["inspect", "dist/Hello-Setup-1.0.0.exe"]).stdout);
    assert!(!main.contains("every user of the computer"), "{main}");
    let company = text(&run(&dir, &["inspect", "dist/Hello-enterprise-Setup-1.0.0.exe"]).stdout);
    assert!(company.contains("every user of the computer"), "{company}");

    // The same files give the same ZIP, byte for byte.
    let first = std::fs::read(&zip).unwrap();
    assert!(run(&dir, &["build", "--profile", "portable"]).status.success());
    assert_eq!(std::fs::read(&zip).unwrap(), first, "a ZIP is reproducible");

    // JSON lists every build; one profile alone writes only its own file.
    let json: serde_json::Value = serde_json::from_str(&text(&run(&dir, &["build", "--fast", "--all-profiles", "--json"]).stdout)).unwrap();
    assert_eq!(json["builds"].as_array().unwrap().len(), 3);
    assert_eq!(json["builds"][2]["kind"], "zip");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_zip_cannot_be_locked_and_has_no_window() {
    need_runtime!();
    let dir = setup(&project("profiles-zip-rules", &config(&format!("{PROFILES}\n[access]\nencrypt = true\n"))));
    let key = dir.join("keys.txt");
    std::fs::write(&key, "correct-horse-battery-staple-1234567890\n").unwrap();
    let out = run(&dir, &["build", "--profile", "portable", "--key-file", "keys.txt"]);
    assert!(!out.status.success());
    assert!(text(&out.stderr).contains("A ZIP can't be locked"), "{}", text(&out.stderr));
    let preview = run(&dir, &["preview", "--profile", "portable"]);
    assert!(text(&preview.stderr).contains("makes a ZIP, which has no window"), "{}", text(&preview.stderr));
    // --out names one file, so it can't serve several builds.
    let out = run(&dir, &["build", "--all-profiles", "--out", "x.exe"]);
    assert!(text(&out.stderr).contains("--out names one file"), "{}", text(&out.stderr));
    std::fs::remove_dir_all(&dir).unwrap();
}
