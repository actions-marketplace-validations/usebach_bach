//! The rules for every path that crosses from a config into a setup file,
//! and from a setup file onto someone's disk.
//!
//! Packaged paths are relative, use `/`, and can never climb out of the
//! install folder. Data folders (deleted on request when uninstalling) must
//! sit inside the user's own profile, so a typo in a config can never point
//! the uninstaller at `C:\` or a whole drive.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

pub mod mac;

pub use mac::check_link;

/// Characters Windows refuses in a file name, besides the path separators.
const FORBIDDEN: &[char] = &['<', '>', ':', '"', '|', '?', '*'];

/// Names Windows reserves for devices, with or without an extension.
const RESERVED: &[&str] = &[
    "con",
    "prn",
    "aux",
    "nul",
    "conin$",
    "conout$",
    "com0",
    "com1",
    "com2",
    "com3",
    "com4",
    "com5",
    "com6",
    "com7",
    "com8",
    "com9",
    "com\u{b9}",
    "com\u{b2}",
    "com\u{b3}",
    "lpt0",
    "lpt1",
    "lpt2",
    "lpt3",
    "lpt4",
    "lpt5",
    "lpt6",
    "lpt7",
    "lpt8",
    "lpt9",
    "lpt\u{b9}",
    "lpt\u{b2}",
    "lpt\u{b3}",
];

/// The longest one name (a file or a folder) may be: 255 UTF-16 units.
const MAX_NAME_UNITS: usize = 255;

/// Whether Windows takes `name` for a device (`nul`, `Com1.txt`, `lpt\u{b2}`, `con .log`),
/// with or without an extension and whatever the letter case.
pub fn is_reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end().to_lowercase();
    RESERVED.contains(&stem.as_str())
}

/// Checks a packaged path and returns it in its one canonical spelling:
/// relative, `/`-separated, no `.` or `..`, no empty parts.
pub fn normalize(path: &str) -> Result<String> {
    if path.starts_with(['/', '\\']) {
        bail!("\"{path}\" is absolute; packaged paths are relative to the install folder");
    }
    let mut parts = Vec::new();
    for part in path.split(['/', '\\']) {
        check_component(path, part)?;
        parts.push(part);
    }
    if parts.is_empty() {
        bail!("a packaged path is empty");
    }
    Ok(parts.join("/"))
}

fn check_component(path: &str, part: &str) -> Result<()> {
    if part.is_empty() || part == "." {
        bail!("\"{path}\" has an empty part");
    }
    if part == ".." {
        bail!("\"{path}\" climbs out of its folder with \"..\"");
    }
    if let Some(bad) = part.chars().find(|c| FORBIDDEN.contains(c) || c.is_control()) {
        bail!("\"{path}\" contains \"{}\", which Windows doesn't allow in names", bad.escape_default());
    }
    if part.ends_with(['.', ' ']) {
        bail!("\"{path}\" has a part ending in a dot or space, which Windows drops");
    }
    if is_reserved_name(part) {
        bail!("\"{path}\" uses \"{part}\", a name Windows reserves for devices");
    }
    if part.encode_utf16().count() > MAX_NAME_UNITS {
        bail!("\"{path}\" has a name part longer than {MAX_NAME_UNITS} characters, which Windows can't create");
    }
    Ok(())
}

/// Where a packaged path lands inside `dir`. Call [`normalize`] first; this
/// only joins.
pub fn under(dir: &Path, packaged: &str) -> PathBuf {
    packaged.split('/').fold(dir.to_path_buf(), |path, part| path.join(part))
}

/// `name` as one file name part: anything a file name can't hold becomes a
/// space, and `fallback` stands in when nothing is left. Used for names Bach
/// derives from the app's, like the macOS uninstaller bundle.
pub fn file_name_part(name: &str, fallback: &str) -> String {
    let name: String =
        name.chars().map(|c| if FORBIDDEN.contains(&c) || matches!(c, '/' | '\\') || c.is_control() { ' ' } else { c }).collect();
    let name = name.trim().trim_end_matches('.').trim();
    if name.is_empty() {
        fallback.to_string()
    } else {
        name.to_string()
    }
}

/// The longest an app id or install folder may be, in characters.
pub const MAX_ID_CHARS: usize = 64;

/// Why `id` can't name a folder and a registry key, in a sentence that fits
/// after "is \"<id>\"; it ...": None when it can. An id has letters, digits,
/// spaces, dots, dashes and underscores, at most [`MAX_ID_CHARS`] characters
/// (so ones in any script count the same), doesn't start or end with a space
/// or dot, and isn't a name Windows keeps for devices.
pub fn id_problem(id: &str) -> Option<String> {
    let chars = id.chars().count();
    if id.is_empty() {
        return Some("is empty".into());
    }
    if chars > MAX_ID_CHARS {
        return Some(format!("is {chars} characters long; at most {MAX_ID_CHARS} are allowed"));
    }
    if id.encode_utf16().count() > MAX_NAME_UNITS {
        return Some(format!("is too long for Windows ({MAX_NAME_UNITS} UTF-16 units at most)"));
    }
    if let Some(bad) = id.chars().find(|c| !(c.is_alphanumeric() || matches!(c, ' ' | '.' | '-' | '_'))) {
        return Some(format!("has \"{}\" in it; use letters, digits, spaces, dots, dashes and underscores", bad.escape_default()));
    }
    if id.starts_with([' ', '.']) || id.ends_with([' ', '.']) {
        return Some("starts or ends with a space or a dot, which Windows drops from folder names".into());
    }
    // `nul` and `Com1` make a folder Windows can't create or others can't open.
    if is_reserved_name(id) {
        return Some("is a name Windows keeps for devices (CON, PRN, AUX, NUL, COM0 to COM9, LPT0 to LPT9)".into());
    }
    None
}

/// Whether `id` can name a folder and a registry key (see [`id_problem`]).
pub fn is_valid_id(id: &str) -> bool {
    id_problem(id).is_none()
}

/// An app id as one file or folder name, for the places setup keeps scratch
/// files by id (logs, staged updates, the online installer's download). Ids
/// without a space come out unchanged. A space becomes `%20`: `%` can't be in
/// an id, so two different ids never share a name, which `My App` and
/// `My-App` did when both became `My-App`.
pub fn safe_dir_name(id: &str) -> String {
    id.replace(' ', "%20")
}

/// The folders a data folder may start with, and the environment variable
/// each one reads.
pub const DATA_ROOTS: &[&str] = &["APPDATA", "LOCALAPPDATA", "USERPROFILE"];

/// Folders that belong to the user or to Windows, not to any one app. A data
/// folder may not be one of them, though a folder *inside* one is the app's
/// own (`%USERPROFILE%\Documents\Hello`). Lower case; `OneDrive*` is matched
/// separately.
const SHARED_FOLDERS: &[&str] = &[
    "documents",
    "desktop",
    "downloads",
    "pictures",
    "music",
    "videos",
    "contacts",
    "favorites",
    "links",
    "saved games",
    "searches",
    "3d objects",
    "appdata",
    "roaming",
    "local",
    "locallow",
    "programs",
    "packages",
    "temp",
];

/// Folders that are off limits together with everything inside them: keys
/// and credentials, and Windows' own per-user data. Lower case.
const PROTECTED_TREES: &[&str] = &[".ssh", ".aws", ".azure", ".gnupg", ".kube", ".docker", ".terraform", "microsoft"];

/// Folders directly in the profile that belong to developer tools and the
/// user's own work, never to an app being uninstalled. Lower case.
const PROFILE_TOOL_FOLDERS: &[&str] = &[
    ".config",
    ".local",
    ".git",
    ".vscode",
    ".npm",
    ".cargo",
    ".rustup",
    ".nuget",
    ".m2",
    ".gradle",
    ".gitconfig",
    "source",
    "repos",
    "projects",
    "code",
    "workspace",
    "workspaces",
    "src",
];

/// Other programs' data below `%APPDATA%` and `%LOCALAPPDATA%`: browser and chat
/// profiles hold the user's logins and history. The folder, or anything below
/// it, is off limits. Lower case, `/`-separated.
const OTHER_APPS: &[&str] = &[
    "google/chrome",
    "google/chromeos",
    "mozilla/firefox",
    "mozilla/thunderbird",
    "opera software",
    "bravesoftware",
    "vivaldi",
    "microsoft edge",
    "code",
    "slack",
    "discord",
    "1password",
    "dropbox",
    "signal",
    "whatsapp",
    "telegram desktop",
];

/// Company folders that hold many programs' data: the folder itself may not be
/// a data folder (a program of that company may have one below it). Lower case.
const VENDOR_FOLDERS: &[&str] =
    &["google", "mozilla", "adobe", "jetbrains", "nvidia", "nvidia corporation", "intel", "amd", "oracle", "apple computer"];

/// Checks a data folder such as `%APPDATA%\Hello`: one of [`DATA_ROOTS`],
/// then at least one folder of the app's own, so never a profile folder like
/// `Documents` or `AppData\Roaming` and never anything in `.ssh` or
/// `Microsoft`.
pub fn check_data_folder(template: &str) -> Result<()> {
    let Some((root, rest)) = split_data_folder(template) else {
        bail!(
            "\"{template}\" must start with %APPDATA%, %LOCALAPPDATA% or %USERPROFILE%, \
             so the uninstaller only ever deletes inside your user's own folders"
        );
    };
    if !DATA_ROOTS.contains(&root.as_str()) {
        bail!("\"{template}\" starts with %{root}%; use %APPDATA%, %LOCALAPPDATA% or %USERPROFILE%");
    }
    if rest.is_empty() {
        bail!("\"{template}\" names the whole %{root}% folder; add your app's own folder after it");
    }
    let rest = normalize(&rest)?;
    let parts: Vec<String> = rest.split('/').map(str::to_lowercase).collect();
    if let Some(tree) = parts.iter().find(|part| PROTECTED_TREES.contains(&part.as_str())) {
        bail!("\"{template}\" is inside \"{tree}\", which holds data that isn't your app's to delete");
    }
    if root == "USERPROFILE" && PROFILE_TOOL_FOLDERS.contains(&parts[0].as_str()) {
        bail!("\"{template}\" is a folder of the user's tools or work (\"{}\"), not of your app; name your app's own folder", parts[0]);
    }
    let joined = parts.join("/");
    if let Some(other) = OTHER_APPS.iter().find(|other| joined == **other || joined.starts_with(&format!("{other}/"))) {
        bail!("\"{template}\" is the data of another program (\"{other}\"), which isn't your app's to delete");
    }
    if parts.len() == 1 && VENDOR_FOLDERS.contains(&parts[0].as_str()) {
        bail!("\"{template}\" is a whole company's folder; name your app's own folder inside it, such as {template}\\YourApp");
    }
    if parts.last().is_some_and(|last| last.starts_with("ntuser") || last.starts_with("usrclass")) {
        bail!("\"{template}\" is part of the user's Windows profile, not a folder of your app");
    }
    if parts.iter().all(|part| is_shared_folder(part)) {
        bail!(
            "\"{template}\" is a folder of the user or of Windows, not of your app; \
             name your app's own folder inside it, such as %{root}%\\YourApp"
        );
    }
    Ok(())
}

/// Whether some folder of the data folder `template` looks like it belongs to
/// the app: its name contains, or is part of, one of `names` (the app's name, id
/// or publisher), ignoring case and anything that isn't a letter or digit. A
/// data folder that doesn't is probably someone else's, and `bach check` warns.
pub fn looks_like_apps_own(template: &str, names: &[&str]) -> bool {
    let key = |text: &str| text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let Some((_, rest)) = split_data_folder(template) else {
        return false;
    };
    let wanted: Vec<String> = names.iter().map(|name| key(name)).filter(|name| name.chars().count() >= 3).collect();
    rest.split(['/', '\\'])
        .map(key)
        .filter(|part| !part.is_empty())
        .any(|part| wanted.iter().any(|name| part.contains(name.as_str()) || (part.chars().count() >= 3 && name.contains(part.as_str()))))
}

fn is_shared_folder(part: &str) -> bool {
    // OneDrive, "OneDrive - Contoso" and the like.
    SHARED_FOLDERS.contains(&part) || part.starts_with("onedrive")
}

/// The real folder a checked data folder points at, reading the root from
/// `lookup` (the environment, in the setup runtime). Refuses a folder that
/// turns out to be, or to hold, one of the profile roots, in case the
/// environment points them somewhere unexpected.
pub fn expand_data_folder(template: &str, lookup: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    check_data_folder(template).ok()?;
    let (root, rest) = split_data_folder(template)?;
    let base = lookup(&root).filter(|value| !value.trim().is_empty())?;
    let rest = normalize(&rest).ok()?;
    let folder = under(Path::new(&base), &rest);
    let holds_a_root = DATA_ROOTS.iter().any(|name| {
        lookup(name).filter(|value| !value.trim().is_empty()).is_some_and(|value| is_same_or_inside(Path::new(&value), &folder))
    });
    (!holds_a_root).then_some(folder)
}

/// [`expand_data_folder`] for either platform: a Windows template is read
/// through `lookup` (the environment); a macOS template (`~/Library/...`) is
/// placed below the `HOME` that `lookup` gives.
pub fn expand_data_folder_for(
    platform: crate::manifest::Platform,
    template: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> Option<PathBuf> {
    match platform {
        crate::manifest::Platform::Windows => expand_data_folder(template, lookup),
        crate::manifest::Platform::Macos => {
            let home = lookup("HOME").filter(|home| !home.trim().is_empty())?;
            mac::expand_data_folder(template, Path::new(&home))
        }
    }
}

/// A key under which two names are the same file on Windows: letters are
/// compared as Windows does, through their one-to-one upper case (so a
/// sigma and the final sigma are one letter, like `a` and `A`), not through
/// `to_lowercase`, which keeps them apart. Characters whose upper case is
/// several letters (the German sharp s) stay as they are, as on NTFS.
pub fn fold_case(text: &str) -> String {
    text.chars()
        .map(|c| {
            let mut upper = c.to_uppercase();
            match (upper.next(), upper.next()) {
                (Some(single), None) => single,
                _ => c,
            }
        })
        .flat_map(char::to_lowercase)
        .collect()
}

/// Whether `inner` is `outer` or lies inside it, ignoring case (as Windows
/// does, see [`fold_case`]), trailing separators and the difference between
/// `/` and `\`.
pub fn is_same_or_inside(inner: &Path, outer: &Path) -> bool {
    let parts = |path: &Path| -> Vec<String> {
        fold_case(&path.to_string_lossy().replace('\\', "/")).split('/').filter(|part| !part.is_empty()).map(str::to_string).collect()
    };
    let (inner, outer) = (parts(inner), parts(outer));
    inner.len() >= outer.len() && inner[..outer.len()] == outer[..]
}

/// `%APPDATA%\Hello\Cache` → ("APPDATA", "Hello\Cache").
fn split_data_folder(template: &str) -> Option<(String, String)> {
    let inner = template.strip_prefix('%')?;
    let end = inner.find('%')?;
    let root = inner[..end].to_ascii_uppercase();
    let rest = inner[end + 1..].trim_start_matches(['/', '\\']).to_string();
    Some((root, rest))
}

/// Matches a packaged path against an exclude pattern. `*` matches within
/// one name, `**` across folders, `?` one character. A pattern without `/`
/// matches the file name anywhere (`*.pdb`); one with `/` matches from the
/// top of the source folder (`docs/**`). Case-insensitive, like Windows.
pub fn matches(pattern: &str, path: &str) -> bool {
    let pattern = pattern.replace('\\', "/").to_lowercase();
    let path = path.to_lowercase();
    if pattern.contains('/') {
        glob(pattern.as_bytes(), path.as_bytes())
    } else {
        let name = path.rsplit('/').next().unwrap_or(&path);
        glob(pattern.as_bytes(), name.as_bytes())
    }
}

/// One piece of a pattern.
enum Piece {
    Literal(u8),
    /// `?`: one character that isn't `/`.
    One,
    /// `*`: any run of characters that aren't `/`.
    Star,
    /// `**` (and the `/` after it): any run at all.
    Deep,
}

/// Matches `text` against `pattern` by dynamic programming over (piece, position):
/// time and memory are the product of their lengths, whatever the pattern, so a
/// pattern like `*a*a*a*a*b` can't take exponential time on a long name.
fn glob(pattern: &[u8], text: &[u8]) -> bool {
    let mut pieces = Vec::new();
    let mut at = 0;
    while at < pattern.len() {
        match pattern[at] {
            b'*' if pattern.get(at + 1) == Some(&b'*') => {
                at += 2;
                if pattern.get(at) == Some(&b'/') {
                    at += 1;
                }
                pieces.push(Piece::Deep);
            }
            b'*' => {
                at += 1;
                pieces.push(Piece::Star);
            }
            b'?' => {
                at += 1;
                pieces.push(Piece::One);
            }
            c => {
                at += 1;
                pieces.push(Piece::Literal(c));
            }
        }
    }
    let (rows, cols) = (pieces.len() + 1, text.len() + 1);
    // `from[i * cols + j]`: the pieces from `i` on match the text from `j` on.
    let mut from = vec![false; rows * cols];
    from[(rows - 1) * cols + (cols - 1)] = true;
    for i in (0..pieces.len()).rev() {
        for j in (0..cols).rev() {
            let next_here = from[(i + 1) * cols + j];
            let next_after = j + 1 < cols && from[(i + 1) * cols + j + 1];
            let more = j + 1 < cols && from[i * cols + j + 1];
            let ch = text.get(j).copied();
            from[i * cols + j] = match pieces[i] {
                Piece::Literal(c) => ch == Some(c) && next_after,
                Piece::One => ch.is_some_and(|c| c != b'/') && next_after,
                Piece::Star => next_here || (ch.is_some_and(|c| c != b'/') && more),
                Piece::Deep => next_here || (ch.is_some() && more),
            };
        }
    }
    from[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_separators() {
        assert_eq!(normalize(r"bin\app.exe").unwrap(), "bin/app.exe");
        assert_eq!(normalize("a/b/c.txt").unwrap(), "a/b/c.txt");
    }

    #[test]
    fn rejects_paths_that_escape_or_break_windows() {
        for bad in ["", "/etc/passwd", r"\x", "a/../b", "./a", "a//b", "C:/x", "a/con.txt", "a/b.", "q?"] {
            assert!(normalize(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn ids_that_differ_never_share_a_folder_name() {
        assert_eq!(safe_dir_name("Hello"), "Hello");
        assert_eq!(safe_dir_name("My App"), "My%20App");
        assert_ne!(safe_dir_name("My App"), safe_dir_name("My-App"));
        assert_ne!(safe_dir_name("My App"), safe_dir_name("My_App"));
        assert!(!is_valid_id("My%20App"), "so no id can spell the encoded name");
        for id in ["My App", "A  B", "x y z"] {
            assert!(normalize(&safe_dir_name(id)).is_ok(), "{id} stays a valid name");
        }
    }

    #[test]
    fn device_names_are_not_ids_folders_or_paths() {
        for name in [
            "nul",
            "NUL",
            "con",
            "Aux",
            "prn",
            "COM1",
            "com9",
            "LPT9",
            "lpt1",
            "Com0",
            "lpt0",
            "nul.txt",
            "Com1.log",
            "con .txt",
            "com\u{b2}",
            "LPT\u{b9}",
            "CONIN$",
        ] {
            assert!(!is_valid_id(name), "{name:?} must not be an id or folder");
            assert!(normalize(&format!("sub/{name}")).is_err(), "{name:?} must not be a packaged name");
        }
        for name in ["Console", "Auxiliary", "Communicator", "Nullable", "com10", "lpt", "con-trol", "Hello"] {
            assert!(is_valid_id(name), "{name:?} is fine");
        }
    }

    #[test]
    fn a_name_part_has_at_most_255_characters() {
        let long = "a".repeat(255);
        assert!(normalize(&format!("{long}/x.txt")).is_ok());
        assert!(normalize(&format!("{}/x.txt", "a".repeat(256))).is_err());
        // Counted in UTF-16 units, like Windows: each of these is two.
        assert!(normalize(&"\u{1d11e}".repeat(127)).is_ok());
        assert!(normalize(&"\u{1d11e}".repeat(128)).is_err());
        assert!(!is_valid_id(&"a".repeat(65)), "ids stay at 64");
        assert!(is_valid_id(&"é".repeat(64)), "64 characters, though 128 bytes");
    }

    #[test]
    fn ids_name_folders() {
        assert!(is_valid_id("Hello World"));
        assert!(is_valid_id("my-app_2.0"));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("a/b"));
        assert!(!is_valid_id(" lead"));
        assert!(!is_valid_id("trail."));
    }

    #[test]
    fn data_folders_stay_in_the_profile() {
        assert!(check_data_folder(r"%APPDATA%\Hello").is_ok());
        assert!(check_data_folder("%localappdata%/Hello/Cache").is_ok());
        assert!(check_data_folder(r"%APPDATA%").is_err());
        assert!(check_data_folder(r"%APPDATA%\").is_err());
        assert!(check_data_folder(r"%APPDATA%\..\x").is_err());
        assert!(check_data_folder(r"%WINDIR%\x").is_err());
        assert!(check_data_folder(r"C:\Hello").is_err());
    }

    #[test]
    fn data_folders_never_name_personal_or_system_folders() {
        for template in [
            r"%USERPROFILE%\Documents",
            "%USERPROFILE%/Documents/",
            r"%userprofile%\DESKTOP",
            r"%USERPROFILE%\Downloads\",
            r"%USERPROFILE%\Pictures",
            r"%USERPROFILE%\Music",
            r"%USERPROFILE%\Videos",
            r"%USERPROFILE%\OneDrive",
            r"%USERPROFILE%\OneDrive - Contoso",
            r"%USERPROFILE%\.ssh",
            r"%USERPROFILE%\.ssh\keys",
            r"%USERPROFILE%\.aws\Hello",
            r"%USERPROFILE%\AppData",
            r"%USERPROFILE%\AppData\Roaming",
            r"%USERPROFILE%\AppData\Local",
            r"%USERPROFILE%\AppData\LocalLow",
            r"%APPDATA%\Microsoft",
            r"%APPDATA%\Microsoft\Windows",
            r"%APPDATA%\microsoft\Hello",
            r"%LOCALAPPDATA%\Programs",
            r"%LOCALAPPDATA%\Packages",
            r"%LOCALAPPDATA%\Temp",
            r"%LOCALAPPDATA%\Microsoft\WindowsApps",
        ] {
            assert!(check_data_folder(template).is_err(), "{template:?} should be rejected");
        }
    }

    #[test]
    fn data_folders_inside_shared_folders_are_the_apps_own() {
        for template in [
            r"%USERPROFILE%\Documents\My Games\Hello",
            r"%USERPROFILE%\Documents\Hello",
            r"%USERPROFILE%\AppData\Roaming\Hello",
            r"%LOCALAPPDATA%\Programs\Hello",
            r"%USERPROFILE%\.hello",
        ] {
            assert!(check_data_folder(template).is_ok(), "{template:?} should be accepted");
        }
    }

    #[test]
    fn data_folders_never_expand_over_a_profile_root() {
        let lookup = |name: &str| match name {
            "APPDATA" => Some(r"C:\Users\a\AppData\Roaming".to_string()),
            "LOCALAPPDATA" => Some(r"C:\Users\a\AppData\Local".to_string()),
            "USERPROFILE" => Some(r"C:\Users\a".to_string()),
            _ => None,
        };
        assert!(expand_data_folder(r"%USERPROFILE%\AppData\Local\Hello", lookup).is_some());
        // A hostile environment: APPDATA is the profile itself.
        let hostile = |name: &str| match name {
            "APPDATA" => Some(r"C:\Users\a".to_string()),
            "USERPROFILE" => Some(r"C:\Users\a\Hello".to_string()),
            _ => None,
        };
        assert!(expand_data_folder(r"%APPDATA%\Hello", hostile).is_none());
    }

    #[test]
    fn same_or_inside_ignores_case_and_separators() {
        assert!(is_same_or_inside(Path::new(r"C:\A\b"), Path::new("c:/a")));
        assert!(is_same_or_inside(Path::new(r"C:\A"), Path::new(r"c:\a\")));
        assert!(!is_same_or_inside(Path::new(r"C:\Ab"), Path::new(r"C:\A")));
        assert!(!is_same_or_inside(Path::new(r"C:\A"), Path::new(r"C:\A\b")));
    }

    #[test]
    fn data_folders_expand() {
        let lookup = |name: &str| (name == "APPDATA").then(|| r"C:\Users\a\AppData\Roaming".to_string());
        let path = expand_data_folder(r"%APPDATA%\Hello\Cache", lookup).unwrap();
        assert_eq!(path, Path::new(r"C:\Users\a\AppData\Roaming").join("Hello").join("Cache"));
        assert!(expand_data_folder(r"%LOCALAPPDATA%\Hello", lookup).is_none());
    }

    #[test]
    fn a_nasty_pattern_is_quick() {
        let started = std::time::Instant::now();
        let name = "a".repeat(400);
        assert!(!matches("*a*a*a*a*a*a*a*a*a*a*a*a*a*a*b", &name));
        assert!(!matches("**/*a*a*a*a*a*a*a*a*a*a*a*a*b", &format!("{name}/{name}")));
        assert!(matches("*a*a*a*a*a*a*a*a*a*a*a*a*a*a", &name));
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "took {:?}", started.elapsed());
    }

    #[test]
    fn globs() {
        assert!(matches("*.pdb", "bin/app.pdb"));
        assert!(matches("*.PDB", "app.pdb"));
        assert!(!matches("*.pdb", "app.exe"));
        assert!(matches("docs/**", "docs/a/b.md"));
        assert!(!matches("docs/*", "docs/a/b.md"));
        assert!(matches("docs/*", "docs/b.md"));
        assert!(matches("**/cache/*", "a/b/cache/x"));
        assert!(matches("app?.log", "app1.log"));
    }

    #[test]
    fn names_are_compared_as_windows_compares_them() {
        for (a, b) in [("a.txt", "A.TXT"), ("σ.txt", "ς.txt"), ("Σ.txt", "ς.txt"), ("ǅ", "ǆ"), ("k", "\u{212a}")] {
            assert_eq!(fold_case(a), fold_case(b), "{a} and {b} are one name");
        }
        for (a, b) in [("straße", "STRASSE"), ("i", "İ"), ("e\u{301}", "\u{e9}")] {
            assert_ne!(fold_case(a), fold_case(b), "{a} and {b} are two names");
        }
    }

    #[test]
    fn an_id_says_what_is_wrong_with_it() {
        assert_eq!(id_problem("Hello World"), None);
        assert!(id_problem(&"A".repeat(70)).unwrap().contains("70 characters"));
        assert_eq!(id_problem(&"日".repeat(64)), None, "characters are counted, not bytes");
        assert!(id_problem(&"日".repeat(65)).unwrap().contains("65 characters"));
        assert!(id_problem("a/b").unwrap().contains("\"/\""));
        assert!(id_problem("nul").unwrap().contains("devices"));
        assert!(id_problem(" x").unwrap().contains("space or a dot"));
        assert!(id_problem("").is_some());
    }

    #[test]
    fn data_folders_never_name_tools_browsers_or_profile_files() {
        for template in [
            r"%USERPROFILE%\.config",
            r"%USERPROFILE%\.docker\config",
            r"%USERPROFILE%\.npm",
            r"%USERPROFILE%\.git",
            r"%USERPROFILE%\source\repos",
            r"%USERPROFILE%\Repos\Hello",
            r"%USERPROFILE%\NTUSER.DAT",
            r"%USERPROFILE%\ntuser.dat.LOG1",
            r"%LOCALAPPDATA%\Google",
            r"%LOCALAPPDATA%\Google\Chrome",
            r"%LOCALAPPDATA%\Google\Chrome\User Data",
            r"%APPDATA%\Mozilla\Firefox\Profiles",
            r"%APPDATA%\Code\User",
            r"%APPDATA%\Discord",
            r"%LOCALAPPDATA%\Adobe",
            r"%USERPROFILE%\.terraform\x",
        ] {
            assert!(check_data_folder(template).is_err(), "{template:?} should be rejected");
        }
        for template in [
            r"%USERPROFILE%\.hello",
            r"%APPDATA%\Hello\.config",
            r"%LOCALAPPDATA%\Google\HelloApp",
            r"%APPDATA%\Acme\Hello",
            r"%USERPROFILE%\Hello\source",
            r"%APPDATA%\.config\Hello",
        ] {
            assert!(check_data_folder(template).is_ok(), "{template:?} should be accepted");
        }
    }

    #[test]
    fn a_data_folder_should_look_like_the_apps_own() {
        let names = ["Hello World", "HelloWorld", "Acme Inc."];
        for own in [
            r"%APPDATA%\Hello World",
            r"%APPDATA%\HelloWorld\Cache",
            r"%LOCALAPPDATA%\Acme\Settings",
            r"%USERPROFILE%\.hello-world",
            r"%APPDATA%\acme",
        ] {
            assert!(looks_like_apps_own(own, &names), "{own}");
        }
        for other in [r"%APPDATA%\Notes", r"%USERPROFILE%\Stuff\Cache", r"%LOCALAPPDATA%\ab"] {
            assert!(!looks_like_apps_own(other, &names), "{other}");
        }
    }
}
