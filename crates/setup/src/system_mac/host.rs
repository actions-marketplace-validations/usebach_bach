//! Facts about this computer and this session that setup checks before it
//! changes anything: the user's languages, free space, and whether another
//! setup for the same app is already at work.

use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

/// The processor architecture of the update feed that serves this program. A macOS
/// setup file is one universal download, so every Mac takes the same `macos` entry.
pub fn own_arch() -> &'static str {
    "macos"
}

/// The architecture of the computer, as a feed names it: the same single `macos` entry.
pub fn native_arch() -> &'static str {
    own_arch()
}

/// The user's languages, most preferred first, like `["de-DE", "en-US"]`.
/// Empty if macOS won't say.
pub fn ui_languages() -> Vec<String> {
    let listed = Command::new("/usr/bin/defaults")
        .args(["read", "-g", "AppleLanguages"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| parse_languages(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default();
    if !listed.is_empty() {
        return listed;
    }
    // A shell session may only set LANG.
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .map(|value| value.split(['.', '@']).next().unwrap_or_default().replace('_', "-"))
        .find(|value| value.len() >= 2 && value != "C" && value != "POSIX")
        .into_iter()
        .collect()
}

/// `defaults read -g AppleLanguages` prints a property list: `(\n    "de-DE",\n    "en-DE"\n)`.
fn parse_languages(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.trim().trim_end_matches(',').trim_matches('"').to_string())
        .filter(|line| {
            !line.is_empty() && line != "(" && line != ")" && line.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        .map(|line| line.replace('_', "-"))
        .collect()
}

/// Bytes free for this user on the volume that holds `path` (or would: the
/// nearest folder above it that exists). None if macOS won't say.
pub fn free_space(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let existing = path.ancestors().find(|folder| folder.is_dir())?;
    let name = std::ffi::CString::new(existing.as_os_str().as_bytes()).ok()?;
    // SAFETY: `name` is a valid NUL-terminated path and `stats` is a plain output struct we own.
    let stats = unsafe {
        let mut stats: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(name.as_ptr(), &mut stats) != 0 {
            return None;
        }
        stats
    };
    // The fields are narrower than u64 on some Macs, so the conversion is needed there.
    #[allow(clippy::useless_conversion)]
    Some(u64::from(stats.f_bavail).saturating_mul(u64::from(stats.f_frsize)))
}

/// Held while a setup for one app installs or removes it, so two setup
/// windows (or a window and a silent run) never work on the same install at
/// once. Released when dropped, and by the system if setup is killed.
pub struct SetupLock(#[allow(dead_code)] File);

impl SetupLock {
    /// None when another setup for `app_id` holds the lock.
    pub fn acquire(app_id: &str) -> Option<Self> {
        Self::named("setup", app_id)
    }

    /// The lock for one app's updater (`--apply-update`, `--auto-update`),
    /// so two of them never download or apply at the same time. It is not
    /// the setup lock: the setup file an updater starts takes that one.
    pub fn acquire_updater(app_id: &str) -> Option<Self> {
        Self::named("update", app_id)
    }

    fn named(kind: &str, app_id: &str) -> Option<Self> {
        // The user's own temporary folder: private to them, and gone with a restart.
        let dir = std::env::temp_dir().join("bach-locks");
        std::fs::create_dir_all(&dir).ok()?;
        let file = File::options()
            .create(true)
            .append(true)
            .open(dir.join(format!("{kind}-{}.lock", bach_pack::paths::safe_dir_name(app_id))))
            .ok()?;
        // SAFETY: a plain advisory lock on a descriptor we own for as long as this value lives.
        let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
        locked.then_some(Self(file))
    }
}

/// Waits until the process `pid` has ended, up to `timeout`. True when it
/// has (or when there is no such process, which comes to the same).
pub fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return true;
    };
    let started = Instant::now();
    loop {
        // SAFETY: signal 0 only checks that the process exists.
        let alive = unsafe { libc::kill(pid, 0) } == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
        if !alive {
            return true;
        }
        if started.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A silent run started from a terminal prints there already.
pub fn attach_parent_console() {}

/// Tells the person who started setup what is wrong with its command line: on
/// stderr for a terminal, and in a dialog for someone who opened it from Finder.
pub fn show_command_line_mistake(text: &str) {
    eprintln!("{text}");
    // SAFETY: isatty only inspects descriptor 2.
    if unsafe { libc::isatty(2) } == 0 {
        // The sentence travels in the environment: as an argument it could be read as an option.
        let script = "display alert \"Setup\" message (system attribute \"BACH_MESSAGE\") as critical";
        let _ = Command::new("/usr/bin/osascript").arg("-e").arg(script).env("BACH_MESSAGE", text).status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_space_looks_above_missing_folders() {
        let missing = std::env::temp_dir().join("bach-free-space").join("not").join("there");
        assert!(free_space(&missing).is_some_and(|free| free > 0));
    }

    #[test]
    fn one_lock_per_app() {
        let id = format!("test-{}", std::process::id());
        let first = SetupLock::acquire(&id).expect("the first lock");
        assert!(SetupLock::acquire(&id).is_none(), "a second setup must wait");
        assert!(SetupLock::acquire_updater(&id).is_some(), "the updater's lock is its own");
        drop(first);
        assert!(SetupLock::acquire(&id).is_some(), "free again once released");
    }

    #[test]
    fn languages_are_read_from_the_defaults_listing() {
        let text = "(\n    \"de-DE\",\n    \"en-DE\",\n    \"zh-Hans-DE\"\n)\n";
        assert_eq!(parse_languages(text), ["de-DE", "en-DE", "zh-Hans-DE"]);
        assert!(parse_languages("").is_empty());
        for language in ui_languages() {
            assert!(language.len() >= 2 && !language.contains('\0'), "{language:?}");
        }
    }

    #[test]
    fn waiting_for_a_gone_process_returns_at_once() {
        assert!(wait_for_exit(std::process::id() + 4_000_000, Duration::from_secs(1)));
        assert!(!wait_for_exit(std::process::id(), Duration::from_millis(150)), "this process is still running");
    }
}
