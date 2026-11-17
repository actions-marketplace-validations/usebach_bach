//! Support info: what a person can save from the failed page and send to
//! whoever makes the app, if they choose to.
//!
//! One text file: what the app and this computer are (names, versions, the
//! Windows build, free space), what went wrong, and setup's log. It never
//! holds the app's files or an access key, and the user's name and profile
//! folder are replaced, so it is safe to read before sending.

use std::fmt::Write;
use std::path::{Path, PathBuf};

use bach_pack::manifest::Manifest;

/// What goes into the file besides the log.
pub struct Facts<'a> {
    pub manifest: &'a Manifest,
    /// The sentence the failed page shows.
    pub message: &'a str,
    pub windows_build: Option<u32>,
    pub elevated: bool,
    pub install_dir: &'a Path,
    pub free_bytes: Option<u64>,
    /// The log's text, as written to disk.
    pub log: &'a str,
}

/// Who and what to hide: the profile folder, the user's name, and an access
/// key if one is in the environment.
pub struct Hide {
    pub profile: String,
    pub user: String,
    pub secrets: Vec<String>,
}

impl Hide {
    /// From this process's environment.
    pub fn from_environment() -> Hide {
        let get = |name: &str| std::env::var(name).unwrap_or_default();
        Hide {
            profile: get("USERPROFILE"),
            user: get("USERNAME"),
            secrets: ["BACH_ACCESS_KEY"].iter().map(|name| get(name)).filter(|value| value.chars().count() >= 4).collect(),
        }
    }
}

/// The text of the file.
pub fn report(facts: &Facts, hide: &Hide) -> String {
    let app = &facts.manifest.app;
    let mut text = String::new();
    let _ = writeln!(text, "Support info from Bach setup");
    let _ = writeln!(text, "(It holds no files of the app and no key. Your user name and profile folder are replaced.)");
    let _ = writeln!(text);
    let _ = writeln!(text, "App: {} {} (id {})", app.name, app.version, app.id);
    let _ = writeln!(text, "Publisher: {}", app.publisher);
    let _ = writeln!(text, "Setup built with Bach {}", bach_pack::VERSION);
    let _ = writeln!(text, "Windows build: {}", facts.windows_build.map_or("unknown".to_string(), |build| build.to_string()));
    let _ = writeln!(text, "Computer: {}", std::env::consts::ARCH);
    let _ = writeln!(text, "Setup runs as administrator: {}", if facts.elevated { "yes" } else { "no" });
    let _ = writeln!(text, "Install folder: {}", facts.install_dir.display());
    let _ = writeln!(
        text,
        "Free space on that drive: {}",
        facts.free_bytes.map_or("unknown".to_string(), |bytes| format!("{} MB", bytes / 1_000_000))
    );
    let _ = writeln!(text, "What went wrong: {}", facts.message);
    let _ = writeln!(text);
    let _ = writeln!(text, "---- log ----");
    let _ = writeln!(text, "{}", facts.log.trim_start_matches('\u{feff}').trim_end());
    redact(&text, hide)
}

/// `text` without the profile folder, the user's name or a secret.
pub fn redact(text: &str, hide: &Hide) -> String {
    let mut out = text.to_string();
    for secret in &hide.secrets {
        out = replace_ignoring_case(&out, secret, "<key>");
    }
    if !hide.profile.is_empty() {
        out = replace_ignoring_case(&out, &hide.profile, "%USERPROFILE%");
        // The same folder with forward slashes.
        out = replace_ignoring_case(&out, &hide.profile.replace('\\', "/"), "%USERPROFILE%");
    }
    let user = hide.user.trim();
    if user.chars().count() >= 3 {
        out = replace_ignoring_case(&out, user, "<user>");
    } else if !user.is_empty() {
        // A very short name would eat ordinary words: only where it is a folder.
        for separator in ['\\', '/'] {
            out = replace_ignoring_case(&out, &format!("{separator}{user}{separator}"), &format!("{separator}<user>{separator}"));
        }
    }
    out
}

/// Replaces every `needle` (compared without regard to case) in `text`.
fn replace_ignoring_case(text: &str, needle: &str, with: &str) -> String {
    if needle.is_empty() {
        return text.to_string();
    }
    let (lower_text, lower_needle) = (text.to_lowercase(), needle.to_lowercase());
    // Lower-casing may change a text's length (a few letters do); then only an exact match is safe.
    if lower_text.len() != text.len() || lower_needle.len() != needle.len() {
        return text.replace(needle, with);
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(found) = lower_text[at..].find(&lower_needle) {
        out.push_str(&text[at..at + found]);
        out.push_str(with);
        at += found + needle.len();
    }
    out.push_str(&text[at..]);
    out
}

/// The file's name: `<app id>-support.txt`.
pub fn file_name(manifest: &Manifest) -> String {
    format!("{}-support.txt", bach_pack::paths::safe_dir_name(&manifest.app.id))
}

/// Writes the report into `folder` and gives the file's path.
pub fn save(folder: &Path, facts: &Facts, hide: &Hide) -> std::io::Result<PathBuf> {
    let path = folder.join(file_name(facts.manifest));
    std::fs::write(&path, report(facts, hide))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        serde_json::from_str(
            r#"{"format":1,"builder":"t","app":{"id":"Hello App","name":"Hello","version":"1.2.0","publisher":"Hello Inc.","executable":"hello.exe"},
            "install":{"folder":"Hello","desktop_shortcut":true},"uninstall":{"data":[],"data_description":null},
            "look":{"theme":"system","accent":null,"logo":null},"files":[{"path":"hello.exe","size":1}]}"#,
        )
        .unwrap()
    }

    fn hide() -> Hide {
        Hide { profile: r"C:\Users\Maria Schmidt".into(), user: "Maria Schmidt".into(), secrets: vec!["s3cret-KEY-value".into()] }
    }

    #[test]
    fn the_report_says_what_and_where_without_the_person() {
        let manifest = manifest();
        let log = "\u{feff}[1] Installing Hello 1.2.0\n[2] Writing C:\\Users\\Maria Schmidt\\AppData\\Local\\Programs\\Hello\\hello.exe\n[3] key s3cret-KEY-value was wrong\n[4] c:/users/maria schmidt/x\n";
        let facts = Facts {
            manifest: &manifest,
            message: "Couldn't write C:\\Users\\Maria Schmidt\\AppData\\Local\\Programs\\Hello\\hello.exe.",
            windows_build: Some(26100),
            elevated: false,
            install_dir: Path::new(r"C:\Users\Maria Schmidt\AppData\Local\Programs\Hello"),
            free_bytes: Some(42_000_000_000),
            log,
        };
        let text = report(&facts, &hide());
        assert!(text.contains("App: Hello 1.2.0 (id Hello App)") && text.contains("Windows build: 26100"), "{text}");
        assert!(text.contains("Free space on that drive: 42000 MB") && text.contains("Installing Hello 1.2.0"), "{text}");
        assert!(text.contains("%USERPROFILE%\\AppData\\Local\\Programs\\Hello\\hello.exe"), "the install folder stays readable: {text}");
        for private in ["Maria", "Schmidt", "s3cret-KEY-value"] {
            assert!(!text.to_lowercase().contains(&private.to_lowercase()), "{private} is still in it:\n{text}");
        }
    }

    #[test]
    fn a_short_name_does_not_eat_ordinary_words() {
        let hide = Hide { profile: String::new(), user: "al".into(), secrets: Vec::new() };
        assert_eq!(redact(r"C:\Users\al\Desktop and a total of all", &hide), r"C:\Users\<user>\Desktop and a total of all");
    }

    #[test]
    fn it_is_saved_where_asked_under_the_apps_name() {
        let dir = std::env::temp_dir().join(format!("bach-support-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = manifest();
        let facts = Facts {
            manifest: &manifest,
            message: "x",
            windows_build: None,
            elevated: true,
            install_dir: Path::new("D:\\Apps\\Hello"),
            free_bytes: None,
            log: "",
        };
        let path = save(&dir, &facts, &hide()).unwrap();
        assert_eq!(path.file_name().unwrap().to_string_lossy(), "Hello%20App-support.txt");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("administrator: yes") && text.contains("Windows build: unknown") && text.contains("D:\\Apps\\Hello"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
