//! Other programs: closing the app (politely) so an update can continue,
//! opening a web page, and checking what the app needs before it installs.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use bach_pack::manifest::Check;

/// The ids of the running programs whose executable is `path` or inside it. `ps` lists
/// the full path of every program, which is what tells an app's own helpers apart.
pub fn processes_under(path: &Path) -> Vec<u32> {
    let Ok(output) = Command::new("/bin/ps").args(["-axo", "pid=,comm="]).output() else {
        return Vec::new();
    };
    let own = std::process::id();
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (pid, command) = line.split_once(' ')?;
            let pid: u32 = pid.parse().ok()?;
            (pid != own && Path::new(command.trim()).starts_with(path)).then_some(pid)
        })
        .collect()
}

/// Asks every program running from `files` (the app's bundle, or a program) to close, the
/// way Quit in the Dock does: each gets the chance to ask about unsaved work, and nothing is
/// killed. True once they've all closed.
pub fn close_programs_using(files: &[PathBuf]) -> bool {
    for file in files {
        if processes_under(file).is_empty() {
            continue;
        }
        // The bundle's own folder asks the app to quit; a plain program has no such way.
        if let Some(bundle) = file.ancestors().find(|part| part.extension().is_some_and(|ext| ext == "app")) {
            let script = "tell application (system attribute \"BACH_APP\") to quit";
            let _ = Command::new("/usr/bin/osascript").arg("-e").arg(script).env("BACH_APP", bundle).output();
        }
    }
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(20) {
        if files.iter().all(|file| processes_under(file).is_empty()) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    false
}

/// Starts the installed app the way Finder does: `open` for a bundle (running a bundle's folder
/// is "permission denied"), and the program itself otherwise. Fails with the reason when it
/// doesn't start.
pub fn launch(program: &Path, dir: &Path) -> std::io::Result<()> {
    if program.extension().is_some_and(|ext| ext == "app") {
        let output = Command::new("/usr/bin/open").arg(program).output()?;
        if output.status.success() {
            return Ok(());
        }
        let why = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(std::io::Error::other(if why.is_empty() { "macOS couldn't open it".to_string() } else { why }));
    }
    use std::os::unix::process::CommandExt;
    Command::new(program).current_dir(dir).process_group(0).spawn().map(|_| ())
}

/// Opens a web page in the user's browser (http and https only, so a
/// manifest can never start a program this way).
pub fn open_url(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    let _ = Command::new("/usr/bin/open").arg(url).status();
}

/// Whether what the app needs is on this computer. Only reads. The checks of a
/// macOS setup file are about the system (`macos.minimum_version` is read where setup
/// starts) and files; Windows' runtimes and registry keys are never present here.
pub fn is_present(check: &Check) -> bool {
    match check {
        Check::File { path } => expand_home(path).is_some_and(|path| path.exists()),
        // An unreadable version never keeps an app from installing.
        Check::Macos { version } => macos_version().is_none_or(|have| bach_pack::macapp::version_at_least(&have, version)),
        _ => false,
    }
}

fn expand_home(path: &str) -> Option<PathBuf> {
    match path.strip_prefix("~/") {
        Some(rest) => Some(super::home().ok()?.join(rest)),
        None if path.starts_with('/') => Some(PathBuf::from(path)),
        None => None,
    }
}

/// Windows' build number; a Mac has none.
pub fn windows_build() -> Option<u32> {
    None
}

/// The version of macOS that is running, like `14.5.1`.
pub fn macos_version() -> Option<String> {
    let output = Command::new("/usr/bin/sw_vers").arg("-productVersion").output().ok()?;
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!version.is_empty()).then_some(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_is_found_by_where_it_runs_from() {
        let exe = std::env::current_exe().unwrap();
        assert!(!processes_under(&exe).contains(&std::process::id()), "setup never counts itself");
        assert!(processes_under(Path::new("/no/such/app")).is_empty());
        // `ps` itself runs from /bin, so the folder holds at least one running program.
        assert!(!processes_under(Path::new("/sbin")).is_empty() || !processes_under(Path::new("/usr/sbin")).is_empty());
    }

    #[test]
    fn a_bundle_is_opened_and_a_missing_one_is_an_error_with_a_reason() {
        let dir = std::env::temp_dir();
        let err = launch(&dir.join("bach-no-such-app.app"), &dir).unwrap_err();
        assert!(!err.to_string().is_empty());
        assert!(launch(&dir.join("bach-no-such-program"), &dir).is_err());
    }

    #[test]
    fn only_web_links_are_opened() {
        open_url("file:///etc/passwd");
        open_url("ftp://example.com");
    }

    #[test]
    fn the_system_reports_its_version() {
        let version = macos_version().expect("sw_vers");
        assert!(version.split('.').next().unwrap().parse::<u32>().is_ok(), "{version}");
    }

    #[test]
    fn file_checks_read_home_and_absolute_paths() {
        assert!(is_present(&Check::File { path: "/usr/bin/true".into() }));
        assert!(!is_present(&Check::File { path: "relative/path".into() }));
        assert!(!is_present(&Check::Webview2));
    }
}
