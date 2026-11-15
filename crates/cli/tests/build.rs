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
    let out =
        Command::new(&online).args(["--silent", "--lang", "en", "--dir"]).arg(&install_dir).output().expect("the online installer starts");
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

#[cfg(windows)]
#[test]
fn a_setup_file_explains_its_options() {
    need_runtime!();
    let dir = project("help", &config(""));
    assert!(run(&dir, &["build", "--fast"]).status.success());
    let setup = dir.join("dist/Hello-Setup-1.0.0.exe");
    for flag in ["--help", "/?"] {
        let out = Command::new(&setup).arg(flag).output().unwrap();
        assert!(out.status.success(), "{flag}");
        let help = text(&out.stdout);
        assert!(help.contains("--silent") && help.contains("--key-file") && help.contains("--check-update"), "{help}");
    }
    assert!(!dir.join("installed").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Signing is optional: with `[signing]` the setup files are signed after they
/// are written (here a stand-in that copies the file), without it nothing is.
#[cfg(windows)]
#[test]
fn signing_is_optional_and_runs_before_the_feed() {
    need_runtime!();
    // Not set up: a plain build writes no signature and says nothing about one.
    let plain = project("sign-off", &config(UPDATES));
    let out = run(&plain, &["build", "--fast"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(!text(&out.stdout).contains("Signing"), "{}", text(&out.stdout));
    std::fs::remove_dir_all(&plain).unwrap();

    let signing = r#"[signing]
program = "cmd"
arguments = "/c copy /y {file} {file}.signed"
"#;
    let dir = project("sign-on", &format!("{}\n{signing}", config(UPDATES)));
    // A bach.toml never runs a program by itself: without --sign, nothing runs.
    let out = run(&dir, &["build", "--fast"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(!dir.join("dist/Hello-Setup-1.0.0.exe.signed").exists());
    let said = text(&out.stdout);
    assert!(said.contains("Not signed") && said.contains("--sign"), "{said}");

    // With --sign the file is handed to the signer, and the command is shown.
    let out = run(&dir, &["build", "--fast", "--sign"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(dir.join("dist/Hello-Setup-1.0.0.exe.signed").is_file(), "the setup file was handed to the signer");
    assert!(text(&out.stdout).contains("Signing: cmd /c copy"), "{}", text(&out.stdout));
    assert!(dir.join("dist/latest.json").is_file());
    std::fs::remove_file(dir.join("dist/Hello-Setup-1.0.0.exe.signed")).unwrap();

    // `bach sign` is explicit too, and signs a file that was built unsigned.
    let out = run(&dir, &["sign", "dist/Hello-Setup-1.0.0.exe"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(dir.join("dist/Hello-Setup-1.0.0.exe.signed").is_file());
    std::fs::remove_dir_all(&dir).unwrap();

    // --sign without a [signing] section is a mistake, not a silent no-op.
    let none = project("sign-none", &config(""));
    let out = run(&none, &["build", "--fast", "--sign"]);
    assert!(!out.status.success());
    assert!(text(&out.stderr).contains("no [signing]"), "{}", text(&out.stderr));
    std::fs::remove_dir_all(&none).unwrap();
}

#[cfg(windows)]
#[test]
fn a_signer_that_fails_fails_the_build_with_one_sentence() {
    need_runtime!();
    let signing = r#"[signing]
program = "cmd"
arguments = "/c exit 3"
"#;
    let dir = project("sign-fails", &format!("{}\n{signing}", config("")));
    let out = run(&dir, &["build", "--fast", "--sign"]);
    assert!(!out.status.success());
    let error = text(&out.stderr);
    assert!(error.contains("failed to sign") && error.contains("exit code 3"), "{error}");

    let missing = project("sign-missing", &format!("{}\n[signing]\nprogram = \"no-such-signer-xyz\"\narguments = \"/a\"\n", config("")));
    let out = run(&missing, &["build", "--fast", "--sign"]);
    assert!(text(&out.stderr).contains("ouldn't start no-such-signer-xyz"), "{}", text(&out.stderr));

    let empty = project("sign-empty", &format!("{}\n[signing]\n", config("")));
    let out = run(&empty, &["check"]);
    assert!(text(&out.stderr).contains("[signing] needs arguments"), "{}", text(&out.stderr));
    for dir in [dir, missing, empty] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// An update patch holds only what changed, knows the version it is for, and
/// goes into the feed when it saves enough (or when asked for).
#[test]
fn a_patch_carries_only_the_files_that_changed() {
    need_runtime!();
    let dir = project("patch", &config(UPDATES));
    let setup = |version: &str| dir.join(format!("dist/Hello-Setup-{version}.exe"));
    assert!(run(&dir, &["build", "--fast"]).status.success());
    std::fs::copy(setup("1.0.0"), dir.join("old.exe")).unwrap();

    // Version 1.1.0: one small file is new, the 1 MB file stays as it was.
    std::fs::write(dir.join("app/notes.txt"), "second").unwrap();
    let out = run(&dir, &["build", "--fast", "--app-version", "1.1.0", "--patch-from", "old.exe"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let said = text(&out.stdout);
    assert!(said.contains("the update patch from 1.0.0"), "{said}");
    let patch = dir.join("dist/Hello-Setup-1.1.0-patch-from-1.0.0.exe");
    assert!(patch.is_file(), "{said}");

    let inspected = text(&run(&dir, &["inspect", patch.to_str().unwrap()]).stdout);
    assert!(inspected.contains("Update patch") && inspected.contains("1.0.0"), "{inspected}");
    // The patch holds notes.txt and the (tiny) program change nothing; the 1 MB file isn't in it.
    let patch_archive = patch_archive_size(&patch);
    let full_archive = patch_archive_size(&setup("1.1.0"));
    assert!(patch_archive < 100_000 && full_archive > 900_000, "patch {patch_archive}, full {full_archive}");

    // A patch is a whole setup file (it carries the runtime), so with this small app
    // it saves nothing, and the feed leaves it out and says so.
    assert!(said.contains("not much smaller") || text(&out.stderr).contains("not much smaller"), "{said}\n{}", text(&out.stderr));
    let feed = std::fs::read_to_string(dir.join("dist/latest.json")).unwrap();
    assert!(!feed.contains("delta"), "{feed}");

    // Asked for, it goes in.
    let out = run(&dir, &["feed", setup("1.1.0").to_str().unwrap(), "--patch", patch.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let feed: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("dist/latest.json")).unwrap()).unwrap();
    assert_eq!(feed["delta"]["from"], "1.0.0");
    assert!(feed["delta"]["url"].as_str().unwrap().ends_with("Hello-Setup-1.1.0-patch-from-1.0.0.exe"));
    assert_eq!(feed["delta"]["sha256"].as_str().unwrap().len(), 64);

    // Not the right inputs.
    let out =
        run(&dir, &["build", "--fast", "--patch-from", "dist/Hello-Setup-1.0.0.exe", "--app-version", "1.2.0", "--out", "dist/x.exe"]);
    assert!(out.status.success());
    let out = run(&dir, &["feed", patch.to_str().unwrap()]);
    assert!(text(&out.stderr).contains("update patch"), "{}", text(&out.stderr));
    let out = run(&dir, &["feed", setup("1.1.0").to_str().unwrap(), "--patch", setup("1.1.0").to_str().unwrap()]);
    assert!(text(&out.stderr).contains("isn't an update patch"), "{}", text(&out.stderr));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The size of the archive inside a setup file, read from the payload's trailer.
fn patch_archive_size(setup: &Path) -> u64 {
    let bytes = std::fs::read(setup).unwrap();
    let end = bytes.len();
    u64::from_le_bytes(bytes[end - 24..end - 16].try_into().unwrap())
}

/// What a shell's `>` writes (UTF-16 in Windows PowerShell, UTF-8 with a mark in
/// newer ones) is read as the key it holds, and `bach key --out` writes plain text.
#[test]
fn key_files_from_any_shell_work() {
    need_runtime!();
    let dir = project("keyfiles", &config(ACCESS));
    let out = run(&dir, &["key", "--count", "3", "--out", "keys.txt"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("Wrote 3 keys to keys.txt"), "{}", text(&out.stdout));
    let keys = std::fs::read_to_string(dir.join("keys.txt")).unwrap();
    assert_eq!(keys.lines().count(), 3);
    assert!(!keys.starts_with('\u{feff}'));
    assert!(!run(&dir, &["key", "--out", "keys.txt"]).status.success(), "never overwrites a file of keys");
    let first = keys.lines().next().unwrap().to_string();

    // A list of keys: the first one is the key.
    let out = run(&dir, &["build", "--fast", "--key-file", "keys.txt"]);
    assert!(out.status.success(), "{}", text(&out.stderr));

    let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain(format!("{first}\r\n").encode_utf16().flat_map(u16::to_le_bytes)).collect();
    std::fs::write(dir.join("utf16.txt"), utf16).unwrap();
    std::fs::write(dir.join("bom.txt"), format!("\u{feff}{first}\r\n")).unwrap();
    for file in ["utf16.txt", "bom.txt"] {
        let out = run(&dir, &["build", "--fast", "--key-file", file]);
        assert!(out.status.success(), "{file}: {}", text(&out.stderr));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_feed_with_a_publisher_key_is_signed_and_refuses_the_wrong_key() {
    need_runtime!();
    let dir = project("feedkey", &config(UPDATES));
    let key_file = dir.join("feed-key.private");
    let made = run(&dir, &["feed-key", "--out", key_file.to_str().unwrap()]);
    assert!(made.status.success(), "{}", text(&made.stderr));
    let shown = text(&made.stdout);
    let public =
        shown.lines().find_map(|line| line.strip_prefix("public_key = \"")).and_then(|rest| rest.strip_suffix('"')).unwrap().to_string();
    let private = std::fs::read_to_string(&key_file).unwrap().trim().to_string();
    assert!(!run(&dir, &["feed-key", "--out", key_file.to_str().unwrap()]).status.success(), "an existing key file is never overwritten");

    let with_key = format!("{UPDATES}public_key = \"{public}\"\n");
    let signed_dir = project("feedkey-signed", &config(&with_key));
    let setup = signed_dir.join("dist/Hello-Setup-1.0.0.exe");
    // A build without the key still builds, and says that latest.json is unsigned.
    let mut build = bach();
    build.current_dir(&signed_dir).args(["build", "--fast"]).env_remove("BACH_FEED_KEY").env_remove("BACH_FEED_KEY_FILE");
    let built = build.output().unwrap();
    assert!(built.status.success(), "{}", text(&built.stderr));
    assert!(text(&built.stderr).contains("is not signed"), "{}", text(&built.stderr));
    let feed = signed_dir.join("signed.json");
    let write = |key: Option<&str>| {
        let mut command = bach();
        command.current_dir(&signed_dir).args(["feed", setup.to_str().unwrap(), "--out", feed.to_str().unwrap()]);
        command.env_remove("BACH_FEED_KEY").env_remove("BACH_FEED_KEY_FILE");
        if let Some(key) = key {
            command.env("BACH_FEED_KEY", key);
        }
        command.output().unwrap()
    };

    let unsigned = write(None);
    assert!(!unsigned.status.success());
    assert!(text(&unsigned.stderr).contains("must be signed"), "{}", text(&unsigned.stderr));

    let (other_private, _) = bach_pack::feedsig::generate().unwrap();
    let wrong = write(Some(&other_private));
    assert!(!wrong.status.success());
    assert!(text(&wrong.stderr).contains("doesn't belong"), "{}", text(&wrong.stderr));

    let good = write(Some(&private));
    assert!(good.status.success(), "{}", text(&good.stderr));
    let entry: bach_pack::manifest::FeedEntry = serde_json::from_str(&std::fs::read_to_string(&feed).unwrap()).unwrap();
    assert!(bach_pack::feedsig::verify(&public, "BachIntegrationTest-Hello", &entry).is_ok());
    let mut tampered = entry.clone();
    tampered.sha256 = "00".repeat(32);
    assert!(bach_pack::feedsig::verify(&public, "BachIntegrationTest-Hello", &tampered).is_err());
    // The latest.json that `bach build` wrote beside the setup file was signed with the same key.
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(&signed_dir).unwrap();
}
