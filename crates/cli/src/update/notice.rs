//! The "a new version is available" line at the end of a command.
//!
//! A command must never wait on the network for this. So the check runs in a
//! separate, detached `bach update-refresh` that writes what it found to a
//! small file in the user's cache; a command only reads that file. The first
//! run after a release therefore says nothing, and the next one does. Every
//! command starts a check, and one that fails (no network, GitHub down) is
//! simply forgotten. It happens only when a person is looking: stderr is a
//! terminal, `--json` is off, and neither `CI` nor `BACH_NO_UPDATE_CHECK` is
//! set.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde_json::{json, Value};

use super::release::Release;
use super::{current, Method};
use crate::runtime;
use crate::version::Version;

/// Commands run within this many seconds of a started check don't start
/// another: a script that runs bach in a loop shouldn't fire a request each time.
const GAP: u64 = 10;

/// What the last check found.
struct Seen {
    /// Seconds since 1970 when a check last started.
    checked: u64,
    latest: Option<Version>,
}

fn file() -> Option<PathBuf> {
    runtime::cache_root().map(|root| root.join("update-check.json"))
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|since| since.as_secs()).unwrap_or(0)
}

fn read() -> Option<Seen> {
    parse(&std::fs::read_to_string(file()?).ok()?)
}

fn parse(text: &str) -> Option<Seen> {
    let json: Value = serde_json::from_str(text).ok()?;
    Some(Seen { checked: json["checked"].as_u64()?, latest: json["latest"].as_str().and_then(Version::parse) })
}

fn write(seen: &Seen) {
    let Some(file) = file() else { return };
    let text = json!({ "checked": seen.checked, "latest": seen.latest.as_ref().map(ToString::to_string) });
    if let Some(dir) = file.parent() {
        if runtime::private_dir(dir).is_err() {
            return;
        }
    }
    // A failed write only means asking again next time.
    let _ = std::fs::write(file, text.to_string());
}

/// Whether the last check started long enough ago (or never) for a new one. A
/// time in the future counts as old, so a wrong clock can't stop checks forever.
fn due(checked: Option<u64>, now: u64) -> bool {
    checked.is_none_or(|checked| checked > now || now - checked >= GAP)
}

/// What `bach update --check` and `bach update` learned, kept for the notice.
pub fn remember(latest: &Version) {
    write(&Seen { checked: now(), latest: Some(latest.clone()) });
}

/// The line to print when `latest` is newer than `current`.
fn message(current: &Version, latest: &Version, update: &str) -> Option<String> {
    (latest > current).then(|| format!("\nA new version of Bach is available: {current} to {latest}.\nUpdate with: {update}\n"))
}

fn wanted() -> bool {
    !crate::output::json()
        && std::io::stderr().is_terminal()
        && std::env::var_os("CI").is_none()
        && std::env::var_os("BACH_NO_UPDATE_CHECK").is_none()
}

/// Runs after a command: says so when a newer release is known, and starts a
/// background check (unless one started a moment ago).
pub fn show() {
    if !wanted() {
        return;
    }
    let method = std::env::current_exe().map(|exe| Method::detect(&exe)).unwrap_or(Method::Standalone);
    // A build from a checkout is for developers, who have git.
    if method == Method::Checkout {
        return;
    }
    let seen = read();
    if let Some(text) =
        seen.as_ref().and_then(|seen| seen.latest.as_ref()).and_then(|latest| message(&current(), latest, &method.command()))
    {
        eprint!("{text}");
    }
    let checked = seen.as_ref().map(|seen| seen.checked);
    if due(checked, now()) {
        // Claim the slot first, so bach runs started meanwhile don't all check.
        write(&Seen { checked: now(), latest: seen.and_then(|seen| seen.latest) });
        start_refresh();
    }
}

/// Starts `bach update-refresh` without waiting for it, away from this
/// terminal (Ctrl+C or closing it must not cut a check short).
fn start_refresh() {
    let Ok(exe) = std::env::current_exe() else { return };
    let mut command = Command::new(exe);
    command.arg("update-refresh").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
        command.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let _ = command.spawn();
}

/// `bach update-refresh`: asks GitHub, writes the answer, says nothing. A
/// failed check is not an error anyone should see.
pub fn refresh() -> Result<()> {
    if let Ok(release) = Release::latest(Some(10)) {
        remember(&release.version);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn a_check_is_due_for_every_command_but_not_every_instant() {
        assert!(due(None, 1_000_000));
        assert!(!due(Some(1_000_000), 1_000_000 + GAP - 1));
        assert!(due(Some(1_000_000), 1_000_000 + GAP));
        assert!(due(Some(2_000_000), 1_000_000), "a clock set back doesn't stop checks");
    }

    #[test]
    fn the_notice_names_both_versions_and_the_command() {
        let text = message(&v("0.7.1"), &v("0.8.0"), "bach update").unwrap();
        assert!(text.contains("0.7.1 to 0.8.0") && text.contains("Update with: bach update"), "{text}");
        assert!(message(&v("0.8.0"), &v("0.8.0"), "bach update").is_none());
        assert!(message(&v("0.9.0"), &v("0.8.0"), "bach update").is_none(), "never say a lower version is new");
        assert!(message(&v("0.8.0-rc.1"), &v("0.8.0"), "bach update").is_some(), "a release beats its own candidate");
    }

    #[test]
    fn the_cache_reads_back_and_bad_files_are_ignored() {
        let seen = parse(r#"{"checked": 1700000000, "latest": "0.8.0"}"#).unwrap();
        assert_eq!((seen.checked, seen.latest), (1_700_000_000, Some(v("0.8.0"))));
        let none = parse(r#"{"checked": 5, "latest": null}"#).unwrap();
        assert!(none.latest.is_none());
        let junk = parse(r#"{"checked": 5, "latest": "banana"}"#).unwrap();
        assert!(junk.latest.is_none(), "an unreadable version is no version");
        assert!(parse("{").is_none() && parse(r#"{"latest": "0.8.0"}"#).is_none());
    }
}
