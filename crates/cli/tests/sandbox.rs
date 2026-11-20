//! `bach test`, without a sandbox: the files it prepares, and (on request)
//! the guest script run on this computer against a throwaway app.

mod common;
use common::{config, project, run, text};

#[test]
fn a_dry_run_writes_the_sandbox_folder_and_starts_nothing() {
    need_runtime!();
    let dir = project("sandbox-dry", &config(""));
    assert!(run(&dir, &["build", "--fast"]).status.success());
    let out = run(&dir, &["test", "--dry-run", "--json"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let object: serde_json::Value = serde_json::from_str(&text(&out.stdout)).unwrap();
    let wsb = std::path::PathBuf::from(object["sandbox_file"].as_str().unwrap());
    let work = wsb.parent().unwrap();
    let wsb_text = std::fs::read_to_string(&wsb).unwrap();
    assert!(wsb_text.contains(r"C:\bach-test\in") && wsb_text.contains("<Networking>Disable</Networking>"), "{wsb_text}");
    let params: serde_json::Value = serde_json::from_slice(&std::fs::read(work.join("in/params.json")).unwrap()).unwrap();
    assert_eq!(params["id"], "BachIntegrationTest-Hello");
    assert_eq!(params["files"].as_array().unwrap().len(), 2, "hello.exe and data.bin");
    assert!(work.join("in/Setup.exe").is_file() && work.join("in/run.ps1").is_file() && work.join("out").is_dir());
    std::fs::remove_dir_all(work).unwrap();

    // The setup file is found from bach.toml; options that don't exist, and files that aren't setup files, are refused.
    let bad = run(&dir, &["test", "--dry-run", "--with", "nothing-like-this"]);
    assert!(text(&bad.stderr).contains("offers no such option"), "{}", text(&bad.stderr));
    std::fs::write(dir.join("plain.txt"), "x").unwrap();
    let bad = run(&dir, &["test", "plain.txt", "--dry-run"]);
    assert!(text(&bad.stderr).contains("isn't a Bach setup file"), "{}", text(&bad.stderr));
    let bad = run(&dir, &["test", "--config", "nope.toml"]);
    assert!(!bad.status.success());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_online_installer_needs_the_network_to_be_tested() {
    need_runtime!();
    let updates = "[updates]\nfeed = \"https://bach-test.invalid/latest.json\"\n";
    let dir = project("sandbox-online", &config(updates));
    assert!(run(&dir, &["build", "--fast", "--online"]).status.success());
    let bad = run(&dir, &["test", "dist/Hello-Web-Setup.exe", "--dry-run"]);
    assert!(text(&bad.stderr).contains("needs the network"), "{}", text(&bad.stderr));
    assert!(run(&dir, &["test", "dist/Hello-Web-Setup.exe", "--dry-run", "--network"]).status.success());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Runs the guest script on this computer: it really installs a throwaway app
/// (its own id, per user), starts it, removes it and reports. Not part of the
/// default run: `cargo test -p bach-cli --test sandbox -- --ignored`.
#[test]
#[ignore = "installs and removes a throwaway app on this computer"]
#[cfg(windows)]
fn the_guest_script_installs_starts_and_removes_a_throwaway_app() {
    need_runtime!();
    let hello = std::path::PathBuf::from(env!("CARGO_BIN_EXE_bach")).with_file_name("hello.exe");
    if !hello.is_file() {
        eprintln!("skipped: build the example first (cargo build -p hello)");
        return;
    }
    let toml = config("").replace("BachIntegrationTest-Hello", "BachGuestScriptTest").replace("\"Hello\"", "\"Bach Guest Script Test\"");
    let dir = project("sandbox-guest", &toml);
    std::fs::copy(&hello, dir.join("app/hello.exe")).unwrap();
    std::fs::remove_file(dir.join("app/data.bin")).unwrap();
    assert!(run(&dir, &["build", "--fast"]).status.success());
    let out = run(&dir, &["test", "--dry-run", "--json", "--launch", "2"]);
    let object: serde_json::Value = serde_json::from_str(&text(&out.stdout)).unwrap();
    let wsb = std::path::PathBuf::from(object["sandbox_file"].as_str().unwrap());
    let work = wsb.parent().unwrap().to_path_buf();

    let ran = std::process::Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(work.join("in/run.ps1"))
        .arg("-Work")
        .arg(&work)
        .arg("-Local")
        .output()
        .unwrap();
    assert!(ran.status.success(), "{}", text(&ran.stderr));
    let report = bach_pack::textfile::read(&work.join("out/report.json")).unwrap();
    let steps: serde_json::Value = serde_json::from_str(&report).unwrap();
    let names: Vec<&str> = steps["steps"].as_array().unwrap().iter().map(|step| step["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Installed", "Registered", "Files", "Uninstaller", "Started", "Removed", "Left behind"], "{report}");
    for step in steps["steps"].as_array().unwrap() {
        assert_eq!(step["ok"], true, "{step}\n{report}");
    }
    std::fs::remove_dir_all(&work).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}
