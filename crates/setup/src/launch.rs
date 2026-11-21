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
//!   theme and the user's language; `--style <name>` and `--layout <name>`
//!   try another built-in style or layout in a preview.
//! - A setup file with an access key asks for it. `--key-file FILE` or the
//!   `BACH_ACCESS_KEY` environment variable give it up front instead (never
//!   an argument, which other programs can read).

use std::path::{Path, PathBuf};

use bach_pack::manifest::{Theme, UNINSTALLER};

use crate::package::Package;
use crate::system::{self, elevate};

/// What `--help` prints: the flags of a setup file and of `uninstall.exe`.
pub const HELP: &str = "Setup file and uninstall.exe options

  --silent, /S            Install or remove without a window. Exit code 0 done, 1 failed, 2 wrong options.
                          /VERYSILENT, /qn and /qb mean the same; /NORESTART, /SP-, /SUPPRESSMSGBOXES and
                          other Inno Setup and NSIS switches that ask for what setup does anyway are accepted.
  --dir FOLDER            Where a silent install goes (a first install only). /DIR=FOLDER and NSIS's /D=FOLDER work too.
  --with IDS              Turn these options on (comma-separated ids: desktop-shortcut, start-menu,
                          autostart, path, auto-update, or a component id).
  --without IDS           Turn these options off.
  --allow-downgrade       Let a silent run install an older version over a newer one.
  --skip-requirements     Let a silent install go on although a requirement (WebView2, .NET, ...) isn't detected.
  --repair                Check the installed files against this setup file (same version) and write back what is missing or changed.
  --overwrite             Let a silent first install replace files that are already in the folder (refused otherwise, exit code 1).
  --delete-data           Silent removal: also delete the app's data folders.
  --key-file FILE         The access key, for a setup file that asks for one (or set BACH_ACCESS_KEY).
  --log FILE              Write this run's log here (default %TEMP%\\<app id>-setup.log).
  --lang CODE             Use this language (en, de, fr, ...) instead of Windows'.
  --theme light|dark|system
  --preview               Show the window; nothing is installed or removed.
  --style NAME            With --preview: try another built-in style (bach, aurora, arcade, ...).
  --layout NAME           With --preview: try another built-in layout (classic, showcase, banner, ...).

uninstall.exe only
  --check-update [--json] Say whether a newer version is out (exit code 10: yes, 0: up to date).
  --download-update       Download it and keep it ready (exit code 10: ready).
  --apply-update          Install what was kept ready. [--wait-for PID] [--launch]
  --update                Download the newest version and open its setup (--silent: install it).
  --auto-update           One background round, as the scheduled task runs it.
  --channel beta|stable|show  Follow the beta or the stable update channel (show: say which).

  --help, -h, /?          This text.
";

/// The environment variable that carries an access key to a setup file.
pub const KEY_ENV: &str = "BACH_ACCESS_KEY";

#[derive(Clone, Debug)]
pub enum Mode {
    Install,
    /// Remove the install in `dir`.
    Remove {
        dir: PathBuf,
    },
}

/// The exit codes of a setup file and its uninstaller, as documented for
/// scripts and deployment tools. A test pins them, so they can't change by
/// accident.
pub mod exit {
    /// The install, update or removal is done (or there was nothing to do).
    pub const OK: i32 = 0;
    /// It failed: a missing requirement, a refused downgrade or overwrite, an
    /// app that is open, a full disk, a file that couldn't be written, a failed
    /// step, a declined administrator prompt. The reason is printed and logged.
    pub const FAILED: i32 = 1;
    /// A mistake in the command line, or a missing access key.
    pub const USAGE: i32 = 2;
    /// `--check-update` and `--download-update`: a newer version is available.
    pub const UPDATE_AVAILABLE: i32 = 10;
}

#[derive(Debug, Default)]
pub struct Args {
    pub uninstall: bool,
    pub preview: bool,
    pub help: bool,
    pub silent: bool,
    pub install_dir: Option<PathBuf>,
    /// `--dir`: where a silent install goes.
    pub dir: Option<PathBuf>,
    pub with: Vec<String>,
    pub without: Vec<String>,
    pub delete_data: bool,
    /// Lets a silent run install an older version over a newer one.
    pub allow_downgrade: bool,
    /// Lets a silent first install replace files that are already in the folder.
    pub overwrite: bool,
    /// `--repair`: put back what is missing or changed of the installed version.
    pub repair: bool,
    /// Lets a silent install go on although a requirement isn't detected.
    pub skip_requirements: bool,
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
    /// `--channel beta|stable|show`: which update feed this installation follows.
    pub channel: Option<String>,
    /// `--launch`: start the app again after a silent update.
    pub launch: bool,
    pub theme: Option<Theme>,
    pub lang: Option<String>,
    /// `--style NAME`: a built-in style to preview instead of the payload's.
    pub style: Option<String>,
    /// `--layout NAME`: a built-in layout to preview instead of the payload's.
    pub layout: Option<String>,
    /// `--msi-package FILE`, with `--msi-product` and `--msi-upgrade`: this is the
    /// front of a `.msi` package, which a Windows Installer package starts to show
    /// this window; the install itself is Windows Installer's (see `system/msi.rs`).
    pub msi_package: Option<PathBuf>,
    pub msi_product: Option<String>,
    pub msi_upgrade: Option<String>,
}

impl Args {
    /// Whether this run is about updating the installed app, as opposed to
    /// installing, removing or previewing.
    /// Whether the user asked what the options are.
    pub fn wants_help(&self) -> bool {
        self.help
    }

    pub fn updating(&self) -> bool {
        self.update || self.check_update || self.download_update || self.apply_update || self.auto_update || self.channel.is_some()
    }

    pub fn parse() -> Result<Self, String> {
        Self::parse_from(std::env::args().skip(1).collect())
    }

    /// The access key given up front: the `--key-file`'s first line, else
    /// the `BACH_ACCESS_KEY` variable. None when neither is given; an error
    /// sentence when the file can't be read.
    pub fn access_key(&self) -> Result<Option<String>, String> {
        if let Some(file) = &self.key_file {
            let text = bach_pack::textfile::read(file).map_err(|err| format!("couldn't read the key file {}: {err}", file.display()))?;
            return Ok(first_line(&text));
        }
        Ok(std::env::var(KEY_ENV).ok().and_then(|text| first_line(&text)))
    }

    /// Reads the command line, or says in one sentence what is wrong with it.
    /// Setup files are started by IT tools that never look at a window, so a
    /// mistake must be an exit code and a message, not a silent default.
    pub fn parse_from(args: Vec<String>) -> Result<Self, String> {
        // Asking for help never fails on the rest of the line.
        if args.iter().any(|arg| matches!(arg.to_ascii_lowercase().as_str(), "--help" | "-h" | "/?" | "/help")) {
            return Ok(Self { help: true, ..Self::default() });
        }
        let mut parsed = Self::default();
        let mut index = 0;
        while index < args.len() {
            let token = &args[index];
            index += 1;
            let Some((name, attached)) = split_flag(token) else {
                return Err(format!("\"{token}\" isn't an option this setup file knows. Run it with --help to see them."));
            };
            let Some(flag) = FLAGS.iter().find(|flag| flag.names.contains(&name.as_str())) else {
                return Err(format!("{token} isn't an option this setup file knows. Run it with --help to see them."));
            };
            let value = if flag.takes_value {
                match attached {
                    // NSIS reads `/D=` as the rest of the line, quotes or not, so
                    // `/D=C:\Program Files\App` arrives as two words.
                    Some(value) if name == "d" => {
                        let rest = args[index..].join(" ");
                        index = args.len();
                        Some(if rest.is_empty() { value } else { format!("{value} {rest}") })
                    }
                    Some(value) => Some(value),
                    None => match args.get(index) {
                        Some(next) if !next.starts_with("--") => {
                            index += 1;
                            Some(next.clone())
                        }
                        _ => return Err(format!("--{0} needs a value, like --{0} VALUE.", flag.names[0])),
                    },
                }
            } else if attached.is_some() {
                return Err(format!("--{} doesn't take a value.", flag.names[0]));
            } else {
                None
            };
            parsed.set(flag.names[0], value)?;
        }
        if parsed.msi_package.is_some() && (parsed.msi_product.is_none() || parsed.msi_upgrade.is_none()) {
            return Err("--msi-package goes with --msi-product and --msi-upgrade.".into());
        }
        if parsed.style.is_some() && !parsed.preview {
            return Err("--style only works with --preview: a setup file always wears the style it was built with.".into());
        }
        if parsed.layout.is_some() && !parsed.preview {
            return Err("--layout only works with --preview: a setup file always keeps the layout it was built with.".into());
        }
        Ok(parsed)
    }

    fn set(&mut self, flag: &str, value: Option<String>) -> Result<(), String> {
        let text = value.clone().unwrap_or_default();
        let ids =
            |text: &str| -> Vec<String> { text.split(',').map(|id| id.trim().to_ascii_lowercase()).filter(|id| !id.is_empty()).collect() };
        match flag {
            "uninstall" => self.uninstall = true,
            "preview" => self.preview = true,
            "silent" => self.silent = true,
            "allow-downgrade" => self.allow_downgrade = true,
            "overwrite" => self.overwrite = true,
            "repair" => self.repair = true,
            "skip-requirements" => self.skip_requirements = true,
            "delete-data" => self.delete_data = true,
            "update" => self.update = true,
            "check-update" => self.check_update = true,
            "download-update" => self.download_update = true,
            "apply-update" => self.apply_update = true,
            "auto-update" => self.auto_update = true,
            "json" => self.json = true,
            "launch" => self.launch = true,
            "install-dir" => self.install_dir = Some(PathBuf::from(text)),
            "dir" => self.dir = Some(PathBuf::from(text)),
            "with" => self.with.extend(ids(&text)),
            "without" => self.without.extend(ids(&text)),
            "log" => self.log = Some(PathBuf::from(text)),
            "msi-package" => self.msi_package = Some(PathBuf::from(text)),
            "msi-product" => self.msi_product = Some(text),
            "msi-upgrade" => self.msi_upgrade = Some(text),
            "key-file" => self.key_file = Some(PathBuf::from(text)),
            "wait-for" => {
                self.wait_for = Some(text.trim().parse().map_err(|_| format!("--wait-for needs a process number, not \"{text}\"."))?)
            }
            "theme" => self.theme = Some(Theme::parse(&text).ok_or_else(|| format!("--theme is light, dark or system, not \"{text}\"."))?),
            "lang" => self.lang = Some(text),
            "style" => {
                let Some(style) = bach_pack::style::preset(&text) else {
                    let names: Vec<&str> = bach_pack::style::PRESETS.iter().map(|preset| preset.name).collect();
                    return Err(format!("--style is one of {}, not \"{text}\".", names.join(", ")));
                };
                self.style = Some(style.name.to_string());
            }
            "layout" => {
                let Some((layout, _)) = bach_pack::style::layout::preset(&text) else {
                    let names: Vec<&str> = bach_pack::style::layout::LAYOUT_PRESETS.iter().map(|preset| preset.name).collect();
                    return Err(format!("--layout is one of {}, not \"{text}\".", names.join(", ")));
                };
                self.layout = Some(layout.name.to_string());
            }
            "channel" => {
                let channel = text.trim().to_ascii_lowercase();
                if !["beta", "stable", "show"].contains(&channel.as_str()) {
                    return Err(format!("--channel is beta, stable or show, not \"{text}\"."));
                }
                self.channel = Some(channel);
            }
            _ => {}
        }
        Ok(())
    }
}

/// One option this setup file understands: its names (the first is the
/// canonical one, spelled `--name` or `/name`) and whether a value follows.
struct Flag {
    names: &'static [&'static str],
    takes_value: bool,
}

const fn flag(names: &'static [&'static str]) -> Flag {
    Flag { names, takes_value: false }
}

const fn valued(names: &'static [&'static str]) -> Flag {
    Flag { names, takes_value: true }
}

/// Every option, matched without regard to case. `/S`, `/quiet` and friends
/// are what IT tools try first.
const FLAGS: &[Flag] = &[
    // `/VERYSILENT` and `/SILENT` are Inno Setup's, `/qn` and `/qb` Windows Installer's.
    flag(&["silent", "s", "quiet", "q", "verysilent", "qn", "qb"]),
    // Inno Setup and NSIS switches that ask for what a Bach setup does anyway
    // (no restart, no message boxes, no checksum prompt): accepted so an
    // existing deployment script keeps working, and ignored.
    flag(&[
        "norestart",
        "sp-",
        "suppressmsgboxes",
        "nocancel",
        "ncrc",
        "closeapplications",
        "nocloseapplications",
        "restartapplications",
        "norestartapplications",
    ]),
    flag(&["uninstall"]),
    flag(&["preview"]),
    flag(&["allow-downgrade"]),
    flag(&["overwrite"]),
    flag(&["repair"]),
    flag(&["skip-requirements"]),
    flag(&["delete-data"]),
    flag(&["update"]),
    flag(&["check-update"]),
    flag(&["download-update"]),
    flag(&["apply-update"]),
    flag(&["auto-update"]),
    flag(&["json"]),
    flag(&["launch"]),
    valued(&["install-dir"]),
    // `/D=` is NSIS's, and (see `parse_from`) takes the rest of the line.
    valued(&["dir", "d"]),
    valued(&["with"]),
    valued(&["without"]),
    valued(&["log"]),
    valued(&["key-file"]),
    valued(&["wait-for"]),
    valued(&["theme"]),
    valued(&["lang"]),
    valued(&["style"]),
    valued(&["layout"]),
    valued(&["channel"]),
    valued(&["msi-package"]),
    valued(&["msi-product"]),
    valued(&["msi-upgrade"]),
];

/// `--dir=C:\Apps` or `/DIR=C:\Apps` → ("dir", Some("C:\Apps")); `--silent` →
/// ("silent", None). None when the token isn't an option at all.
fn split_flag(token: &str) -> Option<(String, Option<String>)> {
    let body = token.strip_prefix("--").or_else(|| token.strip_prefix('/'))?;
    if body.is_empty() {
        return None;
    }
    Some(match body.split_once('=') {
        Some((name, value)) => (name.to_ascii_lowercase(), Some(value.to_string())),
        None => (body.to_ascii_lowercase(), None),
    })
}

/// Whether this run must start again with administrator rights before it does
/// anything: a machine-wide app installs, updates and removes elevated. A
/// preview, `--help` and the update checks change nothing system-wide (an
/// update starts the new setup file, which asks for itself).
pub fn needs_elevation(args: &Args, package: &Package) -> bool {
    package.manifest.install.scope.is_machine() && !args.preview && !args.help && !args.updating() && !elevate::is_elevated()
}

/// Starts this setup again elevated with the same command line and waits for
/// it; the exit code to end with. Windows asks the user once.
pub fn elevate_and_wait(args: &Args) -> i32 {
    if args.silent {
        system::host::attach_parent_console();
    }
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match elevate::relaunch(&arguments) {
        elevate::Outcome::Exited(code) => code as i32,
        elevate::Outcome::Declined => {
            if args.silent {
                eprintln!("This setup installs for every user, so it needs administrator rights, and Windows' prompt was declined.");
            }
            1
        }
        elevate::Outcome::TimedOut | elevate::Outcome::Failed(_) => {
            if args.silent {
                eprintln!("Couldn't start setup with administrator rights.");
            }
            1
        }
    }
}

/// The mode for this package. For the installed uninstaller this hands
/// over to a temp copy and exits the process.
pub fn mode(args: &Args, package: &Package) -> Mode {
    // The front of a .msi installs or removes the package's product, not a setup file's.
    if let Some(link) = system::msi::link() {
        if !args.uninstall {
            return Mode::Install;
        }
        let dir = system::msi::installed(link, &package.manifest)
            .map(|installed| installed.dir)
            .unwrap_or_else(|| system::default_install_dir(&package.manifest));
        return Mode::Remove { dir };
    }
    let removing = args.uninstall || !package.is_installer();
    if !removing {
        return Mode::Install;
    }
    if args.preview {
        let dir = system::installed(&package.manifest)
            .map(|installed| installed.dir)
            .unwrap_or_else(|| system::default_install_dir(&package.manifest));
        return Mode::Remove { dir };
    }
    if let Some(dir) = &args.install_dir {
        return Mode::Remove { dir: dir.clone() };
    }
    let dir = install_dir_of_this_uninstaller(package)
        .or_else(|| system::installed(&package.manifest).map(|installed| installed.dir))
        .unwrap_or_default();
    // A silent removal removes in place (its own file goes after it
    // exits), so whoever started it can wait for its exit code. So does a
    // machine-wide one, which runs elevated: a copy in the user's temp folder
    // is a file the user's own programs can swap before it starts, and
    // Program Files is where only administrators can write.
    let in_place = args.silent || package.manifest.install.scope.is_machine();
    if !in_place && hand_over_to_temp_copy(&dir, package, args) {
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
        bach_pack::paths::safe_dir_name(&package.manifest.app.id),
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
        Args::parse_from(line.split_whitespace().map(String::from).collect()).unwrap()
    }

    fn mistake(line: &str) -> String {
        Args::parse_from(line.split_whitespace().map(String::from).collect()).unwrap_err()
    }

    #[test]
    fn inno_setup_and_windows_installer_switches_mean_silent() {
        for line in ["/VERYSILENT", "/verysilent /NORESTART /SP- /SUPPRESSMSGBOXES", "/qn", "/qb", "/S /NCRC"] {
            assert!(args(line).silent, "{line}");
        }
        assert!(!args("/NORESTART /CLOSEAPPLICATIONS").silent, "the ignored switches do nothing on their own");
    }

    #[test]
    fn nsis_d_takes_the_rest_of_the_line() {
        assert_eq!(args(r"/S /D=C:\Apps\Hello").dir, Some(PathBuf::from(r"C:\Apps\Hello")));
        assert_eq!(
            args(r"/S /D=C:\Program Files\My App").dir,
            Some(PathBuf::from(r"C:\Program Files\My App")),
            "an unquoted folder with spaces arrives as several words"
        );
        assert_eq!(args(r#"/VERYSILENT /DIR=C:\Apps\Hello"#).dir, Some(PathBuf::from(r"C:\Apps\Hello")));
    }

    /// Whatever is on the command line, reading it ends in a value or a sentence.
    #[test]
    fn odd_command_lines_never_panic() {
        let pieces = [
            "--silent",
            "/S",
            "--dir",
            "--dir=",
            "=",
            "/",
            "--",
            "-",
            "",
            " ",
            "--with",
            "a,b,,",
            "--wait-for",
            "99999999999999999999",
            "--theme",
            "--lang",
            "\u{202e}",
            "C:\\x y",
            "--update=1",
            "/=",
            "--=--",
            "é",
        ];
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..5000 {
            let mut line = Vec::new();
            for _ in 0..(state % 6) {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                line.push(pieces[(state % pieces.len() as u64) as usize].to_string());
            }
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let shown = line.join(" ");
            std::panic::catch_unwind(|| Args::parse_from(line.clone())).unwrap_or_else(|_| panic!("panicked on {shown:?}")).ok();
        }
    }

    #[test]
    fn the_documented_exit_codes_do_not_change() {
        assert_eq!((exit::OK, exit::FAILED, exit::USAGE, exit::UPDATE_AVAILABLE), (0, 1, 2, 10));
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
        assert_eq!(args("--preview --style Arcade").style.as_deref(), Some("arcade"));
        assert_eq!(args("--preview --layout Showcase").layout.as_deref(), Some("showcase"));
    }

    #[test]
    fn a_value_flag_never_swallows_the_next_flag() {
        assert!(mistake("--dir --silent").contains("--dir needs a value"));
        assert!(mistake("--silent --with").contains("--with needs a value"));
        assert!(mistake("--lang").contains("needs a value"));
    }

    #[test]
    fn unknown_options_and_stray_words_are_mistakes() {
        assert!(mistake("--silnet").contains("--silnet"));
        assert!(mistake("--silent --no-such-flag").contains("--no-such-flag"));
        assert!(mistake("--silent stray").contains("stray"));
        assert!(mistake("--silent=yes").contains("doesn't take a value"));
        assert!(mistake("--theme purple").contains("light, dark or system"));
        assert!(mistake("--preview --style vaporwave").contains("bach, aurora"));
        assert!(mistake("--style arcade").contains("only works with --preview"));
        assert!(mistake("--preview --layout carousel").contains("classic, showcase"));
        assert!(mistake("--layout banner").contains("only works with --preview"));
        assert!(mistake("/x").contains("/x"));
    }

    #[test]
    fn values_can_be_attached_with_an_equals_sign() {
        let parsed = args(r"--silent --dir=C:\Apps\Hello /LANG=de /Log=C:\x\a=b.log");
        assert_eq!(parsed.dir, Some(PathBuf::from(r"C:\Apps\Hello")));
        assert_eq!(parsed.lang.as_deref(), Some("de"));
        assert_eq!(parsed.log, Some(PathBuf::from(r"C:\x\a=b.log")));
        assert!(args("/DIR=D:\\Hello /silent").dir.is_some());
    }

    #[test]
    fn lists_and_repeats_combine() {
        let parsed = args("--with a,b --with C --without x --without y --dir D:\\A --dir D:\\B");
        assert_eq!(parsed.with, ["a", "b", "c"]);
        assert_eq!(parsed.without, ["x", "y"]);
        assert_eq!(parsed.dir, Some(PathBuf::from(r"D:\B")), "the last of a single-value flag wins");
    }

    #[test]
    fn help_wins_over_a_mistake() {
        assert!(args("--silnet --help").wants_help());
        assert!(args("/?").wants_help());
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
        assert!(!mistake("--key K7QF-3MWD").contains("K7QF"), "--key isn't a flag, and its value isn't echoed");
    }

    #[test]
    fn help_is_asked_for_the_usual_ways() {
        for flag in ["--help", "-h", "/?", "/help", "--HELP"] {
            assert!(args(flag).wants_help(), "{flag}");
        }
        assert!(!args("--silent").wants_help());
        // Every flag the help text names is one the parser knows.
        for flag in [
            "--silent",
            "--dir",
            "--with",
            "--without",
            "--allow-downgrade",
            "--overwrite",
            "--repair",
            "--skip-requirements",
            "--delete-data",
            "--key-file",
            "--log",
            "--lang",
            "--theme",
            "--preview",
            "--style",
            "--layout",
            "--check-update",
            "--download-update",
            "--apply-update",
            "--update",
            "--auto-update",
        ] {
            assert!(HELP.contains(flag), "{flag} is missing from the help text");
        }
        // And the other way round: every option the parser knows is in the help.
        for flag in FLAGS {
            let name = format!("--{}", flag.names[0]);
            // "norestart" heads the accepted-and-ignored Inno and NSIS switches, which the help names together.
            let internal = ["json", "launch", "install-dir", "uninstall", "msi-package", "msi-product", "msi-upgrade", "norestart"]
                .contains(&flag.names[0]);
            assert!(HELP.contains(&name) || internal, "{name} is parsed but not in the help text");
        }
    }

    #[test]
    fn the_channel_flag_takes_beta_stable_or_show() {
        for (line, channel) in [("--channel beta", "beta"), ("--channel=Stable", "stable"), ("/CHANNEL show", "show")] {
            let parsed = args(line);
            assert_eq!(parsed.channel.as_deref(), Some(channel), "{line}");
            assert!(parsed.updating(), "it is a job of the uninstaller, like the update flags");
        }
        assert!(mistake("--channel nightly").contains("beta, stable or show"));
        assert!(mistake("--channel").contains("--channel"));
        assert!(args("--silent").channel.is_none());
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
        assert!(mistake("--wait-for soon").contains("process number"));
    }
}
