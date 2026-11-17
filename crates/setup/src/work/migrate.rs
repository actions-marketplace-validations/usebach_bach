//! `[[migrate]]`: removing an older install that another installer made.
//!
//! An app that moves to Bach would otherwise sit on the computer twice, once
//! from the old Inno Setup, NSIS or MSI install and once from the new one. On
//! a first install, setup looks for the old install's entry under Settings ›
//! Apps (the one the developer named) and runs the uninstall command Windows
//! already holds for it, before any file is placed.
//!
//! This never stops an install. An old install that can't be found, can't be
//! removed silently or fails to go is written to the log and left alone, and
//! the new version is installed anyway. Only the command in that entry is ever
//! run, never through a shell.
//!
//! The command comes from the registry, so what may run with administrator
//! rights is limited:
//!
//! - A per-user entry (`HKCU`, which the user's own programs can write) only
//!   ever runs without administrator rights. An elevated setup (a machine-wide
//!   install, or one started from an administrator's terminal) ignores it.
//! - A machine-wide entry (`HKLM`) runs only when the developer allowed it
//!   (`elevated = true`, which makes Windows ask) or setup is elevated
//!   already, and only if its program sits in Program Files or the Windows
//!   folder, where the user's own programs can't have swapped it.
//! - Windows' own folders in the command (`%SystemRoot%`, `msiexec`) come from
//!   Windows, never from environment variables.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bach_pack::manifest::{Manifest, Migrate};
use windows_registry::{CURRENT_USER, LOCAL_MACHINE};

use super::steps::{self, Ended};
use crate::system::{elevate, folders};

const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
const UNINSTALL_32BIT: &str = r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";
/// Exit codes that mean it is gone: done, done but a restart is due, and an
/// MSI saying the product wasn't installed (anymore).
const GONE: [i32; 3] = [0, 3010, 1605];
/// An uninstaller may hand over to a copy of itself and exit at once; its
/// entry disappears when the real work is done.
const WAIT_FOR_ENTRY: Duration = Duration::from_secs(90);

/// An old install's entry under Settings › Apps.
#[derive(Debug, Default, PartialEq, Eq)]
struct Found {
    /// Whether it lives in `HKLM` (machine-wide) and not `HKCU`.
    machine: bool,
    quiet: Option<String>,
    uninstall: Option<String>,
}

/// Removes every old install the manifest lists that's still there.
pub fn run(manifest: &Manifest) {
    for entry in &manifest.system.migrate {
        remove(entry);
    }
}

fn remove(entry: &Migrate) {
    let key = entry.uninstall_key.as_str();
    let Some(found) = find(key) else {
        crate::log::line(&format!("No older install \"{key}\" to remove."));
        return;
    };
    let already_elevated = elevate::is_elevated();
    if !found.machine && already_elevated {
        crate::log::line(&format!(
            "The older install \"{key}\" is a per-user one, and setup has administrator rights, so setup leaves it."
        ));
        return;
    }
    if found.machine && !already_elevated && !entry.elevated {
        crate::log::line(&format!(
            "The older install \"{key}\" is machine-wide, so setup leaves it (elevated = true in [[migrate]] removes it)."
        ));
        return;
    }
    // Whether the old uninstaller will run with administrator rights.
    let with_rights = found.machine;
    let Some(system) = folders::system_dir() else {
        crate::log::line("Windows didn't say where its folder is, so setup leaves the older install.");
        return;
    };
    // The user's own variables are fine for a program that runs as the user.
    let env = |name: &str| folders::variable(name).or_else(|| if with_rights { None } else { std::env::var(name).ok() });
    let command = match plan(&found, entry, key, &system, env) {
        Ok(command) => command,
        Err(why) => {
            crate::log::line(&format!("The older install \"{key}\" stays: {why}."));
            return;
        }
    };
    if with_rights && !folders::is_admin_only(&command.program) {
        crate::log::line(&format!(
            "The uninstaller of \"{key}\" isn't in Program Files or the Windows folder, so setup won't run it with administrator rights."
        ));
        return;
    }
    let elevated = found.machine && !already_elevated;
    if std::env::current_exe().is_ok_and(|me| super::same_file(&me, &command.program)) {
        crate::log::line(&format!("The uninstall command of \"{key}\" is this setup itself, so setup leaves it."));
        return;
    }
    crate::log::line(&format!("Removing the older install \"{key}\"."));
    match steps::execute(&command.program, command.arguments.as_deref(), elevated, steps::LIMIT) {
        Ended::Code(code) if GONE.contains(&code) => {}
        other => crate::log::line(&format!("Removing \"{key}\" didn't finish cleanly ({other:?}); going on.")),
    }
    let waited = Instant::now();
    while find(key).is_some() && waited.elapsed() < WAIT_FOR_ENTRY {
        std::thread::sleep(Duration::from_millis(500));
    }
    if find(key).is_some() {
        crate::log::line(&format!("\"{key}\" is still listed; going on anyway."));
    }
}

/// The entry named `key`, in the user's registry, then the computer's (64-bit
/// and 32-bit apps).
fn find(key: &str) -> Option<Found> {
    let places = [(CURRENT_USER, false, UNINSTALL), (LOCAL_MACHINE, true, UNINSTALL), (LOCAL_MACHINE, true, UNINSTALL_32BIT)];
    for (hive, machine, path) in places {
        if let Ok(entry) = hive.open(format!(r"{path}\{key}")) {
            let text = |name: &str| entry.get_string(name).ok().map(|text| text.trim().to_string()).filter(|text| !text.is_empty());
            return Some(Found { machine, quiet: text("QuietUninstallString"), uninstall: text("UninstallString") });
        }
    }
    None
}

/// What to run.
#[derive(Debug, PartialEq, Eq)]
struct Command {
    program: PathBuf,
    arguments: Option<String>,
}

/// The command that removes the install quietly, or why there is none.
/// `system` is Windows' `System32` folder (for `msiexec`), and `env` says what
/// `%VARIABLES%` in the command stand for.
fn plan(found: &Found, entry: &Migrate, key: &str, system: &Path, env: impl Fn(&str) -> Option<String>) -> Result<Command, String> {
    // The developer's own arguments win; they go after the uninstall string.
    let line = match (&entry.arguments, &found.uninstall, &found.quiet) {
        (Some(arguments), Some(uninstall), _) | (Some(arguments), None, Some(uninstall)) => format!("{uninstall} {arguments}"),
        (None, _, Some(quiet)) => quiet.clone(),
        (None, Some(uninstall), None) if is_msi(uninstall) => uninstall.clone(),
        (None, Some(_), None) => {
            return Err("it has no silent uninstall command; say how to run it quietly with arguments in [[migrate]]".into());
        }
        (_, None, None) => return Err("it has no uninstall command".into()),
    };
    let (program, rest) = split_command(&line).ok_or_else(|| "its uninstall command isn't readable".to_string())?;
    let program = expand(&program, &env);
    let file_name = Path::new(&program).file_name().map(|name| name.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if file_name == "msiexec.exe" || file_name == "msiexec" {
        // Whatever the entry says (/I opens a window), remove that product quietly.
        let product = guid_in(&line).or_else(|| guid_in(key)).ok_or("it is an MSI, but its product code can't be found")?;
        return Ok(Command { program: system.join("msiexec.exe"), arguments: Some(format!("/x {product} /qn /norestart")) });
    }
    let path = PathBuf::from(&program);
    if !path.is_absolute() || !file_name.ends_with(".exe") || !path.is_file() {
        return Err(format!("its uninstall program \"{program}\" isn't an .exe that exists"));
    }
    Ok(Command { program: path, arguments: Some(rest).filter(|rest| !rest.is_empty()) })
}

fn is_msi(line: &str) -> bool {
    split_command(line).is_some_and(|(program, _)| {
        let name = program.rsplit(['\\', '/']).next().unwrap_or("").to_ascii_lowercase();
        name == "msiexec.exe" || name == "msiexec"
    })
}

/// The first `{…}` that looks like a GUID.
fn guid_in(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let end = start + text[start..].find('}')?;
    let guid = &text[start..=end];
    let inner = &guid[1..guid.len() - 1];
    let shaped = inner.len() == 36
        && inner.char_indices().all(|(at, c)| if matches!(at, 8 | 13 | 18 | 23) { c == '-' } else { c.is_ascii_hexdigit() });
    shaped.then(|| guid.to_string())
}

/// A command line as the program and what follows it. A quoted program ends
/// at the closing quote; an unquoted one ends at its `.exe` (paths with
/// spaces are often left unquoted) or at the first space.
fn split_command(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if let Some(quoted) = line.strip_prefix('"') {
        let end = quoted.find('"')?;
        return Some((quoted[..end].to_string(), quoted[end + 1..].trim().to_string()));
    }
    let lower = line.to_ascii_lowercase();
    let end = match lower.find(".exe") {
        Some(at) => at + 4,
        None => line.find(char::is_whitespace).unwrap_or(line.len()),
    };
    (!line[..end].is_empty()).then(|| (line[..end].to_string(), line[end..].trim().to_string()))
}

/// `%NAME%` replaced by the variable's value; unknown ones stay as written.
fn expand(text: &str, env: &impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => match env(&after[..end]) {
                Some(value) => {
                    out.push_str(&value);
                    rest = &after[end + 1..];
                }
                None => {
                    out.push('%');
                    rest = after;
                }
            },
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(arguments: Option<&str>) -> Migrate {
        Migrate { uninstall_key: "Hello".into(), arguments: arguments.map(String::from), elevated: false }
    }

    fn windows_env(name: &str) -> Option<String> {
        folders::variable(name).or_else(|| std::env::var(name).ok())
    }

    fn system() -> PathBuf {
        folders::system_dir().unwrap()
    }

    fn cmd() -> String {
        system().join("cmd.exe").to_string_lossy().into_owned()
    }

    #[test]
    fn command_lines_split_at_the_program() {
        assert_eq!(
            split_command(r#""C:\Program Files\Hello\unins000.exe" /SILENT"#),
            Some((r"C:\Program Files\Hello\unins000.exe".into(), "/SILENT".into()))
        );
        assert_eq!(
            split_command(r"C:\Program Files\Hello\uninst.exe /S"),
            Some((r"C:\Program Files\Hello\uninst.exe".into(), "/S".into()))
        );
        assert_eq!(
            split_command("MsiExec.exe /I{8A69D345-D564-463C-AFF1-A69D9E530F96}"),
            Some(("MsiExec.exe".into(), "/I{8A69D345-D564-463C-AFF1-A69D9E530F96}".into()))
        );
        assert_eq!(split_command(r#""C:\x.exe""#), Some((r"C:\x.exe".into(), String::new())));
        assert_eq!(split_command(""), None);
        assert_eq!(split_command(r#""unterminated"#), None);
    }

    #[test]
    fn guids_are_found_by_their_shape() {
        assert_eq!(guid_in("/I{8A69D345-D564-463C-AFF1-A69D9E530F96} /q").as_deref(), Some("{8A69D345-D564-463C-AFF1-A69D9E530F96}"));
        assert_eq!(guid_in("{not-a-guid}"), None);
        assert_eq!(guid_in("no braces"), None);
    }

    #[test]
    fn variables_are_filled_in() {
        let env = |name: &str| (name == "SystemRoot").then(|| r"C:\Windows".to_string());
        assert_eq!(expand(r"%SystemRoot%\x.exe", &env), r"C:\Windows\x.exe");
        assert_eq!(expand("100%", &env), "100%");
        assert_eq!(expand("%UNKNOWN%x", &env), "%UNKNOWN%x");
    }

    #[test]
    fn a_quiet_command_is_used_as_it_is() {
        let found =
            Found { quiet: Some(format!("\"{}\" /c exit 0", cmd())), uninstall: Some(format!("\"{}\"", cmd())), ..Found::default() };
        let command = plan(&found, &entry(None), "Hello", &system(), windows_env).unwrap();
        assert_eq!(command.program, PathBuf::from(cmd()));
        assert_eq!(command.arguments.as_deref(), Some("/c exit 0"));
        // The developer's arguments replace it and go after the uninstall string.
        let command = plan(&found, &entry(Some("/c exit 1")), "Hello", &system(), windows_env).unwrap();
        assert_eq!(command.arguments.as_deref(), Some("/c exit 1"));
    }

    #[test]
    fn without_a_quiet_way_the_old_install_stays() {
        let found = Found { uninstall: Some(format!("\"{}\"", cmd())), ..Found::default() };
        let why = plan(&found, &entry(None), "Hello", &system(), windows_env).unwrap_err();
        assert!(why.contains("arguments"), "{why}");
        assert!(plan(&found, &entry(Some("/S")), "Hello", &system(), windows_env).is_ok());
        assert!(plan(&Found::default(), &entry(None), "Hello", &system(), windows_env).is_err());
    }

    #[test]
    fn only_an_exe_that_exists_runs() {
        let missing = Found { quiet: Some(r#""C:\no\such\uninst.exe" /S"#.into()), ..Found::default() };
        assert!(plan(&missing, &entry(None), "Hello", &system(), windows_env).unwrap_err().contains("isn't an .exe that exists"));
        let relative = Found { quiet: Some("uninst.exe /S".into()), ..Found::default() };
        assert!(plan(&relative, &entry(None), "Hello", &system(), windows_env).is_err());
        let script = Found { quiet: Some(format!(r#""{}\drivers\etc\hosts" /S"#, system().display())), ..Found::default() };
        assert!(plan(&script, &entry(None), "Hello", &system(), windows_env).is_err(), "not an .exe");
    }

    #[test]
    fn an_msi_is_removed_by_its_product_code_and_quietly() {
        let found = Found { uninstall: Some("MsiExec.exe /I{8A69D345-D564-463C-AFF1-A69D9E530F96}".into()), ..Found::default() };
        let command = plan(&found, &entry(None), "{8A69D345-D564-463C-AFF1-A69D9E530F96}", &system(), windows_env).unwrap();
        assert_eq!(command.program, system().join("msiexec.exe"), "from Windows, not from the environment");
        assert_eq!(command.arguments.as_deref(), Some("/x {8A69D345-D564-463C-AFF1-A69D9E530F96} /qn /norestart"));
        let no_code = Found { uninstall: Some("MsiExec.exe /I".into()), ..Found::default() };
        assert!(plan(&no_code, &entry(None), "Hello", &system(), windows_env).is_err());
    }

    /// A stand-in for an old installer: an entry under Settings › Apps whose
    /// uninstall command deletes that entry, as a real uninstaller does.
    #[test]
    fn an_older_install_is_removed_through_its_own_entry() {
        let key = format!("BachTestMigrate-{}", std::process::id());
        let path = format!(r"{UNINSTALL}\{key}");
        let entry_key = CURRENT_USER.create(&path).unwrap();
        let quiet = format!(r#""{}" /c reg delete "HKCU\{path}" /f"#, cmd());
        entry_key.set_string("QuietUninstallString", &quiet).unwrap();
        entry_key.set_string("DisplayName", "Old Hello").unwrap();
        drop(entry_key);
        assert!(find(&key).is_some_and(|found| !found.machine));

        remove(&Migrate { uninstall_key: key.clone(), arguments: None, elevated: false });
        if elevate::is_elevated() {
            // An elevated setup never runs a command a user-level entry names.
            assert!(find(&key).is_some(), "left alone");
            let _ = CURRENT_USER.remove_tree(&path);
        } else {
            assert!(find(&key).is_none(), "the old entry is gone");
        }

        // Nothing there: nothing happens, and nothing fails.
        remove(&Migrate { uninstall_key: key, arguments: None, elevated: false });
    }
}
