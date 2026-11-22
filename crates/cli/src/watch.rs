//! `bach watch`: the preview window, reopened every time the config changes.
//!
//! Designing the window means changing a setting, looking, and changing it
//! again. This closes the loop: save `bach.toml` (or the logo, the license, a
//! text file it names) and the window comes back with the change, so there is
//! no command to run in between. A mistake in the config is printed and the
//! window stays as it was, until the next save fixes it.

use std::path::{Component, Path, PathBuf};
use std::process::Child;
use std::time::{Duration, SystemTime};

use anyhow::{bail, Result};

use crate::output;
use crate::preview::{self, PreviewOptions};

pub struct Options<'a> {
    pub config: &'a Path,
    pub preview: PreviewOptions<'a>,
}

/// How often the files are looked at, and how long they have to stay still
/// before a change counts (an editor may write a file in two steps).
const POLL: Duration = Duration::from_millis(250);
const SETTLE: Duration = Duration::from_millis(200);

/// The most files watched: a config names a handful, never hundreds.
const MAX_WATCHED: usize = 256;

/// When each watched file was last changed, and how big it is; `None` for a
/// file that isn't there (yet).
type Fingerprint = Vec<Option<(SystemTime, u64)>>;

pub fn command(options: Options) -> Result<()> {
    if output::json() {
        bail!("bach watch runs until you stop it, so it has no --json result; `bach check --json` gives one");
    }
    preview::check(&options.preview, "watch")?;
    let runtime = preview::find_runtime(options.preview.runtime)?;
    say!("Watching {} and the files it names. Save a change and the preview opens again; press Ctrl+C to stop.", options.config.display());

    let mut window: Option<Child> = None;
    let mut serial = 0;
    loop {
        serial += 1;
        let watched = watched_files(options.config);
        // Taken before the build, so a save during it is noticed too.
        let before = fingerprint(&watched);
        match reopen(options.config, &options.preview, &runtime, serial) {
            Ok(child) => {
                close(window.replace(child));
                say!(
                    "{} the {} preview. Nothing is installed or removed.",
                    if serial == 1 { "Opened" } else { "Reopened" },
                    kind(&options.preview)
                );
            }
            Err(err) => {
                output::error(&err);
                say!("Fix it and save again; the window stays as it was.");
            }
        }
        wait_for_change(&watched, &before);
    }
}

fn kind(options: &PreviewOptions) -> &'static str {
    if options.uninstall {
        "uninstaller"
    } else {
        "installer"
    }
}

/// Builds the preview from the config as it is now and opens it.
fn reopen(config: &Path, options: &PreviewOptions, runtime: &Path, serial: u32) -> Result<Child> {
    let (project, sealing) = preview::load(config, options.profile)?;
    output::warnings(&project.warnings);
    let setup = preview::write(&project, sealing.as_ref(), runtime, serial)?;
    preview::open(&setup, options)
}

/// Closes a window that is still open. One the person already closed is fine.
fn close(window: Option<Child>) {
    if let Some(mut child) = window {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn fingerprint(files: &[PathBuf]) -> Fingerprint {
    files.iter().map(|file| std::fs::metadata(file).ok().and_then(|meta| Some((meta.modified().ok()?, meta.len())))).collect()
}

/// Blocks until a watched file changed and then stayed still for a moment.
fn wait_for_change(files: &[PathBuf], before: &Fingerprint) {
    while fingerprint(files) == *before {
        std::thread::sleep(POLL);
    }
    let mut last = fingerprint(files);
    loop {
        std::thread::sleep(SETTLE);
        let now = fingerprint(files);
        if now == last {
            return;
        }
        last = now;
    }
}

/// The config and every file it names: any text in it that is a path to a file
/// next to it (the logo, the license, a text file for the window, a style's
/// fonts and pictures, a layout's panel picture). Folders and
/// the program's own files are not watched; the preview doesn't show them.
pub(crate) fn watched_files(config: &Path) -> Vec<PathBuf> {
    let mut files = vec![config.to_path_buf()];
    let root = match config.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let Some(table) = std::fs::read_to_string(config).ok().and_then(|text| text.parse::<toml::Table>().ok()) else {
        return files;
    };
    let mut texts = Vec::new();
    collect_strings(&toml::Value::Table(table), &mut texts);
    for text in texts {
        if files.len() >= MAX_WATCHED {
            break;
        }
        if !is_inside(&text) {
            continue;
        }
        let path = root.join(&text);
        if path.is_file() && !files.contains(&path) {
            files.push(path);
        }
    }
    files
}

fn collect_strings(value: &toml::Value, out: &mut Vec<String>) {
    match value {
        toml::Value::String(text) => out.push(text.clone()),
        toml::Value::Array(items) => items.iter().for_each(|item| collect_strings(item, out)),
        toml::Value::Table(table) => table.values().for_each(|item| collect_strings(item, out)),
        _ => {}
    }
}

/// A short relative path that stays below the folder it starts from. It reads
/// `\` as a separator on every system, since a config written on Windows may
/// use it, so `C:\Windows\win.ini` and `..\x` are refused on Linux and macOS too.
fn is_inside(text: &str) -> bool {
    let unified = text.replace('\\', "/");
    !text.is_empty()
        && text.len() <= 260
        && !text.contains(['\0', '\n', '\r'])
        && !unified.starts_with('/')
        && unified.split('/').all(|part| part != ".." && !part.contains(':'))
        && Path::new(&unified).components().all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bach-watch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        dir
    }

    #[test]
    fn the_files_a_config_names_are_watched() {
        let dir = folder("named");
        std::fs::write(dir.join("assets/logo.svg"), "<svg/>").unwrap();
        std::fs::write(dir.join("LICENSE.txt"), "MIT").unwrap();
        std::fs::write(dir.join("LIZENZ.txt"), "MIT").unwrap();
        std::fs::write(dir.join("assets/Outfit.ttf"), "font").unwrap();
        std::fs::write(dir.join("assets/hero.jpg"), "picture").unwrap();
        let config = dir.join("bach.toml");
        std::fs::write(
            &config,
            r#"[app]
name = "Hello"
executable = "hello.exe"

[files]
source = "assets"

[installer]
logo = "assets/logo.svg"
license = { en = "LICENSE.txt", de = "./LIZENZ.txt" }

[style]
font = ["assets/Outfit.ttf"]

[layout]
panel_image = "assets/hero.jpg"
"#,
        )
        .unwrap();
        let watched = watched_files(&config);
        let names: Vec<String> = watched.iter().map(|path| path.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(names[0], "bach.toml", "the config comes first");
        // `./LIZENZ.txt` is the same file as `LIZENZ.txt` once Rust has read the path.
        // A style's font files and a layout's picture reload the window too.
        for expected in ["assets/logo.svg", "LICENSE.txt", "LIZENZ.txt", "assets/Outfit.ttf", "assets/hero.jpg"] {
            assert!(names.iter().any(|name| name == expected), "{expected} in {names:?}");
        }
        assert!(!names.iter().any(|name| name == "assets"), "a folder isn't watched: {names:?}");
        assert!(!names.iter().any(|name| name == "hello.exe"), "a file that isn't there isn't watched: {names:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_text_can_not_make_it_watch_outside_the_project() {
        // The same answer on every system: a Windows path is refused on Linux and macOS too.
        for text in [
            "../secret.txt",
            "..",
            "/etc/passwd",
            r"C:\Windows\win.ini",
            "C:win.ini",
            r"..\secret.txt",
            r"a\..\..\b",
            r"\\server\share\x",
            "a/../../b",
            "",
            "line\nbreak",
            "nul\0",
        ] {
            assert!(!is_inside(text), "{text:?}");
        }
        for text in ["logo.svg", "assets/logo.svg", "./logo.svg", r"assets\logo.svg"] {
            assert!(is_inside(text), "{text:?}");
        }
    }

    #[test]
    fn a_broken_config_still_watches_itself_so_the_next_save_is_noticed() {
        let dir = folder("broken");
        let config = dir.join("bach.toml");
        std::fs::write(&config, "[app\nname = ").unwrap();
        assert_eq!(watched_files(&config), vec![config.clone()]);
        let missing = dir.join("none.toml");
        assert_eq!(watched_files(&missing), vec![missing.clone()], "a config that isn't there yet is watched too");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_change_to_a_file_changes_its_fingerprint() {
        let dir = folder("fingerprint");
        let file = dir.join("bach.toml");
        let absent = dir.join("absent.txt");
        std::fs::write(&file, "a").unwrap();
        let files = vec![file.clone(), absent.clone()];
        let before = fingerprint(&files);
        assert_eq!(before[1], None);
        assert_eq!(fingerprint(&files), before, "nothing changed");
        std::fs::write(&file, "ab").unwrap();
        assert_ne!(fingerprint(&files), before, "the size changed");
        let before = fingerprint(&files);
        std::fs::write(&absent, "x").unwrap();
        assert_ne!(fingerprint(&files), before, "a file that appears counts");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_config_in_the_current_folder_is_found_without_a_parent() {
        // `bach watch` with no argument names plain "bach.toml": its folder is ".".
        let watched = watched_files(Path::new("bach.toml"));
        assert_eq!(watched[0], PathBuf::from("bach.toml"));
    }
}
