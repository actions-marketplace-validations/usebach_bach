//! The work behind the setup window: installing, updating and removing.
//! Each job runs on its own thread (file copies, COM and the registry
//! block) and reports progress as it goes; the view only draws what it
//! hears. A silent run calls the same jobs without a window.

mod conflicts;
mod install;
mod migrate;
mod online;
mod patch;
mod place;
mod preview;
mod remove;
mod section;
mod steps;

use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

use bach_pack::i18n::Key;

pub use conflicts::{describe as describe_files, foreign_files};
pub use install::{install, is_running, previous_install, recover_interrupted, InstallPlan};
pub use preview::pretend;
pub use remove::{remove, RemovePlan};

use crate::texts::Texts;

/// Cancelling an install while its files are still only unpacked, never
/// moved into place: nothing of the earlier version has been touched, so
/// stopping is as safe as never having started. One flag for the one job a
/// setup window runs.
pub mod cancel {
    use std::io::{self, Read};
    use std::sync::atomic::{AtomicBool, Ordering};

    static REQUESTED: AtomicBool = AtomicBool::new(false);

    /// What a cancelled job reports instead of a sentence for the user.
    pub const CANCELLED: &str = "\u{1}cancelled";

    pub fn request() {
        REQUESTED.store(true, Ordering::SeqCst);
    }

    pub fn reset() {
        REQUESTED.store(false, Ordering::SeqCst);
    }

    pub fn requested() -> bool {
        REQUESTED.load(Ordering::SeqCst)
    }

    /// A reader that fails once the job is cancelled, so even one huge file
    /// stops within a chunk.
    pub struct Cancellable<R>(pub R);

    impl<R: Read> Read for Cancellable<R> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if requested() {
                return Err(io::Error::other("cancelled"));
            }
            self.0.read(buf)
        }
    }
}

/// What a job tells the view.
pub enum Event {
    /// How far along (0..1) and what it's doing, as a short label.
    Progress(f32, String),
    /// The end: a readable sentence when it failed.
    Finished(Result<(), String>),
}

/// How a job reports progress.
pub type Report<'a> = &'a dyn Fn(f32, &str);

/// Sharing violation or lock violation: another process has the file open.
fn is_in_use(err: &io::Error) -> bool {
    matches!(err.raw_os_error(), Some(32) | Some(33))
}

/// Whether `err`, from changing `path`, means a running program holds it.
/// "Access is denied" (5) is ambiguous: Windows says it for a running copy
/// and also for a read-only file, a permission, ransomware protection or a
/// security product. Only when opening the file for writing then fails with a
/// sharing violation is a program really holding it; otherwise telling the
/// person to close the app would send them in circles.
fn is_in_use_at(err: &io::Error, path: &Path) -> bool {
    match err.raw_os_error() {
        Some(32) | Some(33) => true,
        Some(5) => matches!(fs::OpenOptions::new().write(true).open(path), Err(open) if is_in_use(&open)),
        _ => false,
    }
}

/// ERROR_DISK_FULL or ERROR_HANDLE_DISK_FULL.
fn is_disk_full(err: &io::Error) -> bool {
    matches!(err.raw_os_error(), Some(112) | Some(39)) || err.kind() == io::ErrorKind::StorageFull
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Removes a file; one that isn't there is already gone.
fn remove_file_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

/// Deletes a file, waiting a moment for a program that is just exiting
/// (the app, or the uninstaller that started a temp copy) to let go of it.
fn remove_waiting(path: &Path, texts: &Texts, still_open: Key) -> Result<(), String> {
    let mut last = None;
    for _ in 0..20 {
        match remove_file_if_present(path) {
            Ok(()) => return Ok(()),
            Err(err) if is_in_use_at(&err, path) => last = Some(err),
            Err(err) => return Err(failed(texts, Key::error_delete, path, &err)),
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    match last {
        Some(_) => Err(texts.get(still_open)),
        None => Ok(()),
    }
}

/// Starts a hidden `cmd` that retries deleting `file` (and then removing
/// `folder`, if empty) for up to two minutes, until this process has exited
/// and let go of it.
pub(crate) fn delete_after_exit(file: &Path, folder: Option<&Path>) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    // The paths travel in environment variables and are read with delayed
    // expansion (`!NAME!`), which happens after cmd has parsed the line, so a
    // `%`, `&`, `^` or quote in a folder name is just text and never code.
    // The wait is `waitfor`, which sleeps its timeout when no signal comes and
    // needs neither a console nor the network (`ping` needs both).
    let cleanup = if folder.is_some() { "rmdir \"!BACH_DELETE_DIR!\" >nul 2>&1 & " } else { "" };
    // cmd reads everything after `if (…)` on the same line as part of the
    // if, so the wait sits in the if and the way out in the else.
    let script = format!(
        "/v:on /d /q /c \"for /l %i in (1,1,120) do @(del /f /q \"!BACH_DELETE_FILE!\" >nul 2>&1 & \
         if exist \"!BACH_DELETE_FILE!\" (waitfor /t 1 BachDeleteWait >nul 2>&1) else ({cleanup}exit))\""
    );
    // Windows' own cmd.exe, by full path: this may run with administrator rights.
    let cmd = std::env::var_os("SystemRoot")
        .map(|root| std::path::PathBuf::from(root).join(r"System32\cmd.exe"))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| "cmd.exe".into());
    let mut command = std::process::Command::new(cmd);
    command.env("BACH_DELETE_FILE", file).raw_arg(script).creation_flags(CREATE_NO_WINDOW);
    if let Some(folder) = folder {
        command.env("BACH_DELETE_DIR", folder);
    }
    command.spawn().map(|_| ())
}

#[cfg(test)]
mod delete_tests {
    use super::*;

    /// The file goes after the helper starts, also with characters in its name
    /// that cmd would treat as code, and the empty folder goes with it.
    #[test]
    fn a_file_with_odd_characters_is_deleted_and_nothing_else() {
        let base = std::env::temp_dir().join(format!("bach-delete-{}", std::process::id()));
        let folder = base.join("100% & ^ safe (x86) !");
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("a & b %PATH% ! ^.tmp");
        std::fs::write(&file, b"x").unwrap();
        let bystander = base.join("stays.txt");
        std::fs::write(&bystander, b"x").unwrap();

        delete_after_exit(&file, Some(&folder)).unwrap();
        for _ in 0..40 {
            if !file.exists() && !folder.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        assert!(!file.exists(), "the file is deleted");
        assert!(!folder.exists(), "and its empty folder");
        assert!(bystander.exists(), "nothing else is touched");
        std::fs::remove_dir_all(&base).unwrap();
    }
}

/// Removes the now-empty folders above each removed file, deepest first,
/// stopping at `root` (which stays).
fn remove_empty_parents<'a>(root: &Path, files: impl Iterator<Item = &'a str>) {
    let unique: std::collections::BTreeSet<&str> = files.flat_map(|file| file.match_indices('/').map(move |(at, _)| &file[..at])).collect();
    let mut folders: Vec<&str> = unique.into_iter().collect();
    folders.sort_by_key(|folder| std::cmp::Reverse(folder.matches('/').count()));
    for folder in folders {
        // Fails while the folder still holds something; that's the point.
        let _ = fs::remove_dir(bach_pack::paths::under(root, folder));
    }
}

/// "Couldn't write C:\…\app.dll. Access is denied." in the window's
/// language (the second sentence comes from Windows, in the user's).
fn failed(texts: &Texts, key: Key, path: &Path, err: &io::Error) -> String {
    // A full disk reads "There is not enough space on the disk." here, from
    // Windows; the free-space check before unpacking says it with numbers.
    let path = path.display().to_string();
    // The raw code is for whoever reads the log; the window gets the sentence.
    crate::log::line(&format!("{path}: {err} (code {})", err.raw_os_error().map_or("none".to_string(), |code| code.to_string())));
    format!("{} {}", texts.with(key, &[("path", &path)]), describe(err))
}

/// An OS error as a sentence, without the "(os error 5)" tail.
fn describe(err: &io::Error) -> String {
    let text = err.to_string();
    let text = match text.rfind(" (os error") {
        Some(at) => text[..at].trim_end().to_string(),
        None => text,
    };
    sentence(text)
}

fn plain(err: &anyhow::Error) -> String {
    sentence(format!("{err:#}"))
}

fn sentence(text: String) -> String {
    let text = text.trim().to_string();
    if text.is_empty() || text.ends_with(['.', '!', '?', '。']) {
        text
    } else {
        format!("{text}.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_errors_read_as_sentences() {
        let text = describe(&io::Error::from_raw_os_error(5));
        assert!(!text.contains("os error"), "{text}");
        assert!(text.ends_with('.'));
    }

    #[test]
    fn in_use_and_full_disks_are_recognised() {
        assert!(is_in_use(&io::Error::from_raw_os_error(32)));
        assert!(is_in_use(&io::Error::from_raw_os_error(33)));
        assert!(!is_in_use(&io::Error::from_raw_os_error(2)));
        assert!(!is_in_use(&io::Error::from_raw_os_error(5)), "access denied alone isn't proof of a running program");
    }

    #[test]
    #[allow(clippy::permissions_set_readonly_false)] // Windows: only the read-only attribute
    fn access_denied_is_in_use_only_while_a_program_holds_the_file() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = std::env::temp_dir().join(format!("bach-denied-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let denied = io::Error::from_raw_os_error(5);

        // Read-only: Windows says "access denied", and nobody has the file open.
        let readonly = dir.join("readonly.txt");
        fs::write(&readonly, b"x").unwrap();
        let mut permissions = fs::metadata(&readonly).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&readonly, permissions.clone()).unwrap();
        assert!(!is_in_use_at(&denied, &readonly));
        permissions.set_readonly(false);
        fs::set_permissions(&readonly, permissions).unwrap();

        // Held open without sharing, like a running program: in use.
        let held = dir.join("held.exe");
        fs::write(&held, b"MZ").unwrap();
        let _handle = fs::OpenOptions::new().read(true).share_mode(1).open(&held).unwrap();
        assert!(is_in_use_at(&denied, &held));
        assert!(is_in_use_at(&io::Error::from_raw_os_error(32), &dir.join("missing")), "32 needs no second look");
        assert!(!is_in_use_at(&io::Error::from_raw_os_error(2), &held));
        drop(_handle);
        fs::remove_dir_all(&dir).unwrap();
        assert!(is_disk_full(&io::Error::from_raw_os_error(112)));
        assert!(!is_disk_full(&io::Error::from_raw_os_error(5)));
    }

    #[test]
    fn empty_parents_go_and_the_root_stays() {
        let root = std::env::temp_dir().join(format!("bach-setup-parents-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::create_dir_all(root.join("keep")).unwrap();
        fs::write(root.join("keep/mine.txt"), b"user file").unwrap();
        remove_empty_parents(&root, ["a/b/x.dll", "keep/y.dll"].into_iter());
        assert!(!root.join("a").exists());
        assert!(root.join("keep/mine.txt").exists());
        assert!(root.exists());
        fs::remove_dir_all(&root).unwrap();
    }
}
