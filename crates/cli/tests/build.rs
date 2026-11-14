//! End-to-end checks of the `bach` binary: real builds on the real setup
//! runtime, read back with `bach inspect`. They need `bach-setup.exe` next to
//! `bach` (a `cargo build` puts it there) and say so, instead of failing,
//! when it isn't.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bach() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bach"))
}

/// The setup runtime a build can use, if this checkout has built one.
fn runtime() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_BIN_EXE_bach")).parent()?.to_path_buf();
    ["bach-setup.exe", "bach-setup"].iter().map(|name| dir.join(name)).find(|path| path.is_file())
}

/// A project folder with an app to package and the given `bach.toml`.
fn project(name: &str, config: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bach-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("app")).unwrap();
    std::fs::write(dir.join("app/hello.exe"), b"MZ").unwrap();
    // Incompressible bytes, so the full setup file is clearly bigger than
    // the online one.
    let mut state = 0x9E37_79B9u32;
    let noise: Vec<u8> = (0..1_000_000)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect();
    std::fs::write(dir.join("app/data.bin"), noise).unwrap();
    std::fs::write(dir.join("bach.toml"), config).unwrap();
    dir
}

fn config(extra: &str) -> String {
    format!(
        r#"[app]
name = "Hello"
id = "BachIntegrationTest-Hello"
version = "1.0.0"
publisher = "Example"
executable = "hello.exe"

[files]
source = "app"

{extra}"#
    )
}

fn run(dir: &Path, args: &[&str]) -> Output {
    let runtime = runtime().expect("a runtime");
    bach().current_dir(dir).env("BACH_RUNTIME", runtime).args(args).output().expect("bach starts")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn size(path: &Path) -> u64 {
    std::fs::metadata(path).unwrap_or_else(|err| panic!("{}: {err}", path.display())).len()
}

macro_rules! need_runtime {
    () => {
        if runtime().is_none() {
            eprintln!("skipped: bach-setup.exe isn't built next to bach (run `cargo build` first)");
            return;
        }
    };
}

const UPDATES: &str = r#"[updates]
feed = "https://bach-test.invalid/hello/latest.json"
download_url = "https://bach-test.invalid/hello/{file}"
"#;

#[test]
fn a_build_writes_a_setup_file_that_inspect_reads() {
    need_runtime!();
    let dir = project("plain", &config(""));
    let built = run(&dir, &["build", "--fast"]);
    assert!(built.status.success(), "{}", text(&built.stderr));
    let setup = dir.join("dist/Hello-Setup-1.0.0.exe");
    assert!(setup.is_file());
    assert!(!dir.join("dist/Hello-Web-Setup.exe").exists(), "no online installer unless asked for");

    let inspected = run(&dir, &["inspect", setup.to_str().unwrap()]);
    let out = text(&inspected.stdout);
    assert!(inspected.status.success(), "{}", text(&inspected.stderr));
    assert!(out.starts_with("Installer for Hello 1.0.0"), "{out}");
    assert!(out.contains("Packed"), "{out}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_online_installer_is_the_same_window_without_the_files() {
    need_runtime!();
    let dir = project("online", &config(&format!("[installer]\nonline = true\n\n{UPDATES}")));
    let built = run(&dir, &["build", "--fast"]);
    assert!(built.status.success(), "{}", text(&built.stderr));
    let full = dir.join("dist/Hello-Setup-1.0.0.exe");
    let online = dir.join("dist/Hello-Web-Setup.exe");
    assert!(full.is_file() && online.is_file());
    assert!(
        size(&online) + 900_000 < size(&full),
        "the online installer ({}) should lack the app's files ({})",
        size(&online),
        size(&full)
    );
    let inspected = text(&run(&dir, &["inspect", online.to_str().unwrap()]).stdout);
    assert!(inspected.starts_with("Online installer for Hello 1.0.0"), "{inspected}");
    assert!(inspected.contains("bach-test.invalid"), "it names the feed it asks: {inspected}");
    assert!(!inspected.contains("Packed"), "{inspected}");
    // The feed describes the full setup file, never the online one.
    let feed = std::fs::read_to_string(dir.join("dist/latest.json")).unwrap();
    assert!(feed.contains("Hello-Setup-1.0.0.exe"), "{feed}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_online_flag_builds_it_without_the_setting() {
    need_runtime!();
    let dir = project("online-flag", &config(UPDATES));
    let built = run(&dir, &["build", "--fast", "--online"]);
    assert!(built.status.success(), "{}", text(&built.stderr));
    assert!(dir.join("dist/Hello-Web-Setup.exe").is_file());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_online_installer_needs_somewhere_to_download_from() {
    need_runtime!();
    let dir = project("online-nofeed", &config(""));
    let built = run(&dir, &["build", "--fast", "--online"]);
    assert!(!built.status.success());
    let message = text(&built.stderr);
    assert!(message.contains("[updates]"), "{message}");
    assert!(!dir.join("dist").exists() || !dir.join("dist/Hello-Web-Setup.exe").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_feed_and_winget_refuse_an_online_installer() {
    need_runtime!();
    let dir = project("online-feed", &config(&format!("[installer]\nonline = true\n\n{UPDATES}")));
    assert!(run(&dir, &["build", "--fast"]).status.success());
    let online = dir.join("dist/Hello-Web-Setup.exe");
    for args in [
        vec!["feed", online.to_str().unwrap(), "--url", "https://x.invalid/a.exe"],
        vec!["winget", online.to_str().unwrap(), "--url", "https://x.invalid/a.exe"],
    ] {
        let out = run(&dir, &args);
        assert!(!out.status.success(), "{args:?}");
        assert!(text(&out.stderr).contains("online installer"), "{args:?}: {}", text(&out.stderr));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The online installer run without a window, with a feed that can't be
/// reached: it must say so in one sentence, exit with 1, and change nothing.
#[cfg(windows)]
#[test]
fn an_unreachable_feed_fails_the_online_installer_cleanly() {
    need_runtime!();
    let dir = project("online-silent", &config(&format!("[installer]\nonline = true\n\n{UPDATES}")));
    assert!(run(&dir, &["build", "--fast"]).status.success());
    let online = dir.join("dist/Hello-Web-Setup.exe");
    let install_dir = dir.join("installed");
    let out = Command::new(&online)
        .args(["--silent", "--lang", "en", "--dir"])
        .arg(&install_dir)
        .output()
        .expect("the online installer starts");
    let message = text(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{message}");
    assert!(message.contains("Couldn't download Hello"), "{message}");
    assert!(message.contains("internet connection"), "{message}");
    assert!(!install_dir.join("hello.exe").exists(), "nothing was installed");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn mistakes_in_the_config_are_one_sentence_naming_the_setting() {
    need_runtime!();
    let dir = project("mistake", &config("[installer]\naccent = \"red\"\n"));
    let out = run(&dir, &["check"]);
    assert!(!out.status.success());
    let message = text(&out.stderr);
    assert!(message.contains("installer.accent"), "{message}");
    assert!(!message.contains("panicked"), "{message}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A phrase that shows up in a setup file's bytes only if the files in it
/// are readable.
const SECRET: &str = "Zq7-private-plan-ZX91-launch-codes-alpha-0042";

fn secret_project(name: &str, config: &str) -> PathBuf {
    let dir = project(name, config);
    std::fs::write(dir.join("app/notes.txt"), format!("{SECRET}\n")).unwrap();
    dir
}

fn contains(path: &Path, needle: &str) -> bool {
    let bytes = std::fs::read(path).unwrap();
    bytes.windows(needle.len()).any(|window| window == needle.as_bytes())
}

const ACCESS: &str = "[access]\n";
const KEY: &str = "K7QF-3MWD-9XPA-H2TY-6NBC";

#[test]
fn protected_setup_files_do_not_hold_readable_files() {
    need_runtime!();
    let open = secret_project("open", &config(""));
    assert!(run(&open, &["build", "--fast"]).status.success());
    assert!(
        contains(&open.join("dist/Hello-Setup-1.0.0.exe"), SECRET),
        "an unprotected setup file holds its files as they are (this test's premise)"
    );

    let locked = secret_project("locked", &config(ACCESS));
    let built = bach()
        .current_dir(&locked)
        .env("BACH_RUNTIME", runtime().unwrap())
        .env("BACH_ACCESS_KEY", KEY)
        .args(["build", "--fast"])
        .output()
        .unwrap();
    assert!(built.status.success(), "{}", text(&built.stderr));
    let setup = locked.join("dist/Hello-Setup-1.0.0.exe");
    assert!(!contains(&setup, SECRET), "the protected setup file leaks a packaged file");
    assert!(!contains(&setup, KEY), "the key must never be in the file");
    assert!(text(&built.stdout).contains("encrypted"), "{}", text(&built.stdout));

    let inspected = text(&run(&locked, &["inspect", setup.to_str().unwrap()]).stdout);
    assert!(inspected.contains("files encrypted"), "{inspected}");
    assert!(!inspected.contains(KEY), "{inspected}");
    for dir in [open, locked] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn a_protected_build_says_where_the_key_goes() {
    need_runtime!();
    let dir = project("nokey", &config(ACCESS));
    let out = bach()
        .current_dir(&dir)
        .env("BACH_RUNTIME", runtime().unwrap())
        .env_remove("BACH_ACCESS_KEY")
        .args(["build", "--fast"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let message = text(&out.stderr);
    assert!(message.contains("BACH_ACCESS_KEY") && message.contains("bach key"), "{message}");
    assert!(!dir.join("dist").join("Hello-Setup-1.0.0.exe").exists(), "no half-built file is left");

    // The variable can have another name, and a key file works too.
    let renamed = project("renamed", &config("[access]\nkey_env = \"HELLO_KEY\"\n"));
    let out = bach()
        .current_dir(&renamed)
        .env("BACH_RUNTIME", runtime().unwrap())
        .env_remove("HELLO_KEY")
        .args(["build", "--fast"])
        .output()
        .unwrap();
    assert!(text(&out.stderr).contains("HELLO_KEY"), "{}", text(&out.stderr));
    std::fs::write(renamed.join("key.txt"), format!("{KEY}\n")).unwrap();
    let out = run(&renamed, &["build", "--fast", "--key-file", "key.txt"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    for dir in [dir, renamed] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn weak_and_odd_keys_are_refused_before_building() {
    need_runtime!();
    let dir = project("weak", &config(ACCESS));
    for (key, expected) in [("short", "too short"), ("has space in it", "has ' '"), ("tab\there123", "has '\\t'")] {
        std::fs::write(dir.join("key.txt"), key).unwrap();
        let out = run(&dir, &["build", "--fast", "--key-file", "key.txt"]);
        assert!(!out.status.success(), "{key:?}");
        assert!(text(&out.stderr).contains(expected), "{key:?}: {}", text(&out.stderr));
    }
    assert!(!dir.join("dist/Hello-Setup-1.0.0.exe").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn bach_key_makes_distinct_valid_keys() {
    let out = bach().args(["key", "--count", "5"]).output().unwrap();
    assert!(out.status.success());
    let keys: Vec<String> = text(&out.stdout).lines().map(String::from).collect();
    assert_eq!(keys.len(), 5);
    let mut unique = keys.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 5, "{keys:?}");
    for key in &keys {
        assert_eq!(key.len(), 24, "{key}");
        assert!(bach_pack::protect::check_key(key).is_ok(), "{key}");
    }
    assert!(!bach().args(["key", "--count", "0"]).output().unwrap().status.success());
}

/// The protected setup file run without a window: no key is a command-line
/// mistake, a wrong key is a failure, and either way nothing is installed.
#[cfg(windows)]
#[test]
fn a_protected_setup_file_refuses_without_the_right_key() {
    need_runtime!();
    let dir = project("protected-silent", &config(ACCESS));
    let built = bach()
        .current_dir(&dir)
        .env("BACH_RUNTIME", runtime().unwrap())
        .env("BACH_ACCESS_KEY", KEY)
        .args(["build", "--fast"])
        .output()
        .unwrap();
    assert!(built.status.success(), "{}", text(&built.stderr));
    let setup = dir.join("dist/Hello-Setup-1.0.0.exe");
    let install_dir = dir.join("installed");
    let silent = |key_file: Option<&Path>| {
        let mut command = Command::new(&setup);
        command.args(["--silent", "--lang", "en", "--dir"]).arg(&install_dir).env_remove("BACH_ACCESS_KEY");
        if let Some(file) = key_file {
            command.arg("--key-file").arg(file);
        }
        command.output().unwrap()
    };

    let out = silent(None);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("asks for an access key"), "{}", text(&out.stderr));

    std::fs::write(dir.join("wrong.txt"), "K7QF-3MWD-9XPA-H2TY-6NBD\n").unwrap();
    let out = silent(Some(&dir.join("wrong.txt")));
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("That key doesn't fit"), "{}", text(&out.stderr));

    let out = silent(Some(&dir.join("missing.txt")));
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("couldn't read the key file"), "{}", text(&out.stderr));

    assert!(!install_dir.exists(), "a refused key touches nothing on this computer");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_feed_can_be_rolled_out_to_some_computers_first() {
    need_runtime!();
    let dir = project("rollout", &config(UPDATES));
    assert!(run(&dir, &["build", "--fast"]).status.success());
    let setup = dir.join("dist/Hello-Setup-1.0.0.exe");
    let feed = dir.join("staged.json");
    let write = |extra: &[&str]| {
        let mut args = vec!["feed", setup.to_str().unwrap(), "--out", feed.to_str().unwrap(), "--notes", "Faster"];
        args.extend_from_slice(extra);
        run(&dir, &args)
    };

    let out = write(&["--rollout", "25"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("25% of computers"), "{}", text(&out.stdout));
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&feed).unwrap()).unwrap();
    assert_eq!(json["rollout"], 25);
    assert_eq!(json["version"], "1.0.0");
    assert_eq!(json["notes"], "Faster");
    assert_eq!(json["sha256"].as_str().unwrap().len(), 64);
    assert!(json["url"].as_str().unwrap().starts_with("https://bach-test.invalid/hello/Hello-Setup-1.0.0"));

    // Everyone again: the field goes away rather than saying 100.
    assert!(write(&[]).status.success());
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&feed).unwrap()).unwrap();
    assert!(json.get("rollout").is_none(), "{json}");

    for bad in ["101", "-1", "many"] {
        assert!(!write(&["--rollout", bad]).status.success(), "--rollout {bad}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
