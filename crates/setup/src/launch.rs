//! What this launch should be, from the command line and the payload.
//!
//! - A setup file (payload with files): the install window. Run again over
//!   an install, it updates it in place.
//! - `uninstall.exe` in an install folder (payload without files, or
//!   `--uninstall`): the remove window. A running program can't delete its
//!   own file, so the installed uninstaller first hands over to a copy of
//!   itself in the temp folder and exits; that copy (started with
//!   `--install-dir`) shows the window, removes everything and deletes
//!   itself once it closes.
//! - `--silent`: the same work without a window, for IT tools and scripts.
//!   `--dir`, `--with`, `--without` and `--delete-data` make the choices;
//!   the exit code says how it went.
//! - `--preview`: the windows, with the work only pretended.
//! - `--theme light|dark|system` and `--lang <code>` override the payload's
//!   theme and the user's language.
//! - A setup file with an access key asks for it. `--key-file FILE` or the
//!   `BACH_ACCESS_KEY` environment variable give it up front instead (never
//!   an argument, which other programs can read).

use std::path::{Path, PathBuf};

use bach_pack::manifest::{Theme, UNINSTALLER};

use crate::package::Package;
use crate::system;

/// The environment variable that carries an access key to a setup file.
pub const KEY_ENV: &str = "BACH_ACCESS_KEY";

#[derive(Clone, Debug)]
pub enum Mode {
    Install,
    /// Remove the install in `dir`.
    Remove { dir: PathBuf },
}

#[derive(Debug, Default)]
pub struct Args {
    pub uninstall: bool,
    pub preview: bool,
    pub silent: bool,
    pub install_dir: Option<PathBuf>,
    /// `--dir`: where a silent install goes.
    pub dir: Option<PathBuf>,
    pub with: Vec<String>,
    pub without: Vec<String>,
    pub delete_data: bool,
    /// Lets a silent run install an older version over a newer one.
    pub allow_downgrade: bool,
    /// `--log FILE`: where this run's log goes.
    pub log: Option<PathBuf>,
    /// `--key-file FILE`: a file holding the access key.
    pub key_file: Option<PathBuf>,
    /// `--update`: look for a newer version and install it.
    pub update: bool,
    /// `--check-update`: only say whether there is one (exit code 10).
    pub check_update: bool,
    /// `--download-update`: fetch a newer version in the background and keep
    /// it ready (exit code 10 when one is ready).
    pub download_update: bool,
    /// `--apply-update`: install the version that was kept ready.
    pub apply_update: bool,
    /// `--auto-update`: one background round (check, download, and install
    /// when the app isn't running), as the scheduled task runs it.
    pub auto_update: bool,
    /// `--json`: `--check-update` prints its answer as JSON.
    pub json: bool,
    /// `--wait-for PID`: let that process (the app) end first.
    pub wait_for: Option<u32>,
    /// `--launch`: start the app again after a silent update.
    pub launch: bool,
    pub theme: Option<Theme>,
    pub lang: Option<String>,
}

impl Args {
    /// Whether this run is about updating the installed app, as opposed to
    /// installing, removing or previewing.
    pub fn updating(&self) -> bool {
        self.update || self.check_update || self.download_update || self.apply_update || self.auto_update
    }

    pub fn parse() -> Self {
        Self::from(std::env::args().skip(1).collect())
    }

    /// The access key given up front: the `--key-file`'s first line, else
    /// the `BACH_ACCESS_KEY` variable. None when neither is given; an error
    /// sentence when the file can't be read.
    pub fn access_key(&self) -> Result<Option<String>, String> {
        if let Some(file) = &self.key_file {
            let text = std::fs::read_to_string(file)
                .map_err(|err| format!("couldn't read the key file {}: {err}", file.display()))?;
            return Ok(first_line(&text));
        }
        Ok(std::env::var(KEY_ENV).ok().and_then(|text| first_line(&text)))
    }

    fn from(args: Vec<String>) -> Self {
        let value_of = |flag: &str| {
            args.iter()
                .position(|arg| arg.eq_ignore_ascii_case(flag))
                .and_then(|index| args.get(index + 1))
                .cloned()
        };
        let has = |flags: &[&str]| args.iter().any(|arg| flags.iter().any(|flag| arg.eq_ignore_ascii_case(flag)));
        let list = |flag: &str| {
            value_of(flag)
                .map(|value| value.split(',').map(|id| id.trim().to_ascii_lowercase()).filter(|id| !id.is_empty()).collect())
                .unwrap_or_default()
        };
        Self {
            uninstall: has(&["--uninstall"]),
            preview: has(&["--preview"]),
            // `/S` and `/quiet` are what IT tools try first.
            silent: has(&["--silent", "/s", "/silent", "/quiet", "/q"]),
            install_dir: value_of("--install-dir").map(PathBuf::from),
            dir: value_of("--dir").map(PathBuf::from),
            with: list("--with"),
            without: list("--without"),
            delete_data: has(&["--delete-data"]),
            allow_downgrade: has(&["--allow-downgrade"]),
            log: value_of("--log").map(PathBuf::from),
            key_file: value_of("--key-file").map(PathBuf::from),
            update: has(&["--update"]),
            check_update: has(&["--check-update"]),
            download_update: has(&["--download-update"]),
            apply_update: has(&["--apply-update"]),
            auto_update: has(&["--auto-update"]),
            json: has(&["--json"]),
            wait_for: value_of("--wait-for").and_then(|pid| pid.trim().parse().ok()),
            launch: has(&["--launch"]),
            theme: value_of("--theme").and_then(|value| Theme::parse(&value)),
            lang: value_of("--lang"),
        }
    }
}

/// The mode for this package. For the installed uninstaller this hands
/// over to a temp copy and exits the process.
pub fn mode(args: &Args, package: &Package) -> Mode {
    let removing = args.uninstall || !package.is_installer();
    if !removing {
        return Mode::Install;
    }
    if args.preview {
        let dir = system::installed(&package.manifest)
            .map(|installed| installed.dir)
            .unwrap_or_else(|| system::default_install_dir(&package.manifest.install.folder));
        return Mode::Remove { dir };
    }
    if let Some(dir) = &args.install_dir {
        return Mode::Remove { dir: dir.clone() };
    }
    let dir = install_dir_of_this_uninstaller(package)
        .or_else(|| system::installed(&package.manifest).map(|installed| installed.dir))
        .unwrap_or_default();
    // A silent removal removes in place (its own file goes after it
    // exits), so whoever started it can wait for its exit code.
    if !args.silent && hand_over_to_temp_copy(&dir, package, args) {
        std::process::exit(0);
    }
    // The hand-over failed: remove from here, and this file goes once the
    // process has exited.
    Mode::Remove { dir }
}

/// The folder this `uninstall.exe` sits in, when it's the installed one.
fn install_dir_of_this_uninstaller(package: &Package) -> Option<PathBuf> {
    let name = package.exe.file_name()?.to_string_lossy().to_lowercase();
    (name == UNINSTALLER).then(|| package.exe.parent().map(Path::to_path_buf)).flatten()
}

/// Starts a copy of this executable from the temp folder in remove mode.
/// True once that copy is running.
fn hand_over_to_temp_copy(dir: &Path, package: &Package, args: &Args) -> bool {
    let copy = std::env::temp_dir().join(format!(
        "{}-uninstall-{}.exe",
        package.manifest.app.id.replace(' ', "-"),
        std::process::id()
    ));
    if std::fs::copy(&package.exe, &copy).is_err() {
        return false;
    }
    let mut command = std::process::Command::new(&copy);
    command.arg("--uninstall").arg("--install-dir").arg(dir);
    if let Some(lang) = &args.lang {
        command.args(["--lang", lang]);
    }
    if command.spawn().is_ok() {
        true
    } else {
        let _ = std::fs::remove_file(&copy);
        false
    }
}

/// The first non-empty line, trimmed: what a key file or variable holds,
/// whether or not an editor added a newline.
fn first_line(text: &str) -> Option<String> {
    text.trim_start_matches('\u{feff}').lines().map(str::trim).find(|line| !line.is_empty()).map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Args {
        Args::from(line.split_whitespace().map(String::from).collect())
    }

    #[test]
    fn flags_are_read() {
        let parsed = args("--silent --dir D:\\Apps\\Hello --with samples,PATH --without desktop-shortcut --lang de");
        assert!(parsed.silent && !parsed.preview);
        assert_eq!(parsed.dir, Some(PathBuf::from(r"D:\Apps\Hello")));
        assert_eq!(parsed.with, ["samples", "path"]);
        assert_eq!(parsed.without, ["desktop-shortcut"]);
        assert_eq!(parsed.lang.as_deref(), Some("de"));
        assert!(args("/S").silent);
        assert!(args("--preview --theme dark").theme == Some(Theme::Dark));
    }

    #[test]
    fn keys_are_read_from_files_and_never_from_arguments() {
        assert_eq!(first_line("K7QF-3MWD\r\n"), Some("K7QF-3MWD".into()));
        assert_eq!(first_line("\u{feff}  K7QF-3MWD  \n\nsecond"), Some("K7QF-3MWD".into()));
        assert_eq!(first_line("\n  \n"), None);
        assert_eq!(first_line(""), None);
        let parsed = args("--silent --key-file C:\\keys\\hello.txt");
        assert_eq!(parsed.key_file, Some(PathBuf::from(r"C:\keys\hello.txt")));
        // A key on the command line would show in every process list.
        assert!(parsed.access_key().is_err(), "a missing file is an error, not no key");
        assert!(!format!("{:?}", args("--key K7QF-3MWD")).contains("K7QF"), "--key isn't a flag");
    }

    #[test]
    fn update_flags_are_read() {
        let parsed = args("--apply-update --wait-for 4242 --launch --silent");
        assert!(parsed.apply_update && parsed.launch && parsed.updating());
        assert_eq!(parsed.wait_for, Some(4242));
        assert!(args("--check-update --json").json);
        assert!(args("--auto-update").updating());
        assert!(args("--download-update").download_update);
        assert!(!args("--silent").updating());
        assert_eq!(args("--wait-for soon").wait_for, None);
    }
}
