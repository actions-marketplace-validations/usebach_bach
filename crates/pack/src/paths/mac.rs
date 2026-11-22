//! The macOS side of the path rules: data folders and links.
//!
//! A data folder is written like `~/Library/Application Support/Hello`. It
//! must sit inside the user's home folder, in one of the places apps keep
//! their data, and below a folder of the app's own, so a typo in a config can
//! never make the uninstaller delete `~` or `~/Library`. The same promise
//! the Windows rules make for `%APPDATA%`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use super::{normalize, under};

/// The folders of `~/Library` apps keep their own data in. Lower case.
const LIBRARY_FOLDERS: &[&str] = &[
    "application support",
    "caches",
    "logs",
    "preferences",
    "containers",
    "group containers",
    "saved application state",
    "httpstorages",
    "webkit",
    "application scripts",
    "launchagents",
];

/// Folders of the user's own files: a folder *inside* one is the app's, the
/// folder itself never is. Lower case.
const USER_FOLDERS: &[&str] = &["documents", "desktop", "downloads", "pictures", "music", "movies", "public", "library", "applications"];

/// Off limits together with everything inside, wherever they sit below `~`.
const PROTECTED_TREES: &[&str] = &[
    ".ssh",
    ".aws",
    ".azure",
    ".gnupg",
    ".kube",
    ".docker",
    ".terraform",
    "keychains",
    "mail",
    "messages",
    "safari",
    "cookies",
    "mobile documents",
    "photos library.photoslibrary",
    "trash",
];

/// Folders directly in the home folder that belong to tools and the user's own work.
const HOME_TOOL_FOLDERS: &[&str] = &[
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
    ".zshrc",
    ".zprofile",
    ".zshenv",
    ".bash_profile",
    ".bashrc",
    ".profile",
    ".gitconfig",
    ".oh-my-zsh",
    ".cache",
    ".bun",
    "source",
    "repos",
    "projects",
    "code",
    "workspace",
    "workspaces",
    "src",
    "developer",
];

/// Other programs' data below `Library/Application Support`, `Caches` and the like: browser
/// and chat profiles hold the user's logins. Lower case, `/`-separated.
const OTHER_APPS: &[&str] = &[
    "google/chrome",
    "firefox",
    "mozilla",
    "microsoft edge",
    "bravesoftware",
    "vivaldi",
    "code",
    "slack",
    "discord",
    "1password",
    "dropbox",
    "signal",
    "telegram desktop",
    "com.apple.safari",
    "addressbook",
    "calendars",
    "icloud",
];

/// Company folders that hold many programs' data; the folder itself isn't a data folder.
const VENDOR_FOLDERS: &[&str] = &["google", "mozilla", "adobe", "jetbrains", "microsoft", "apple", "com.apple"];

fn split(template: &str) -> Option<String> {
    let rest = template.strip_prefix('~')?;
    // `~user` and `~+` are other things; only the user's own home.
    let rest = rest.strip_prefix('/')?;
    Some(rest.trim_start_matches('/').to_string())
}

/// Checks a macOS data folder such as `~/Library/Application Support/Hello`.
pub fn check_data_folder(template: &str) -> Result<()> {
    let Some(rest) = split(template) else {
        bail!("\"{template}\" must start with ~/, so the uninstaller only ever deletes inside your user's own folder");
    };
    if rest.is_empty() {
        bail!("\"{template}\" names the whole home folder; add your app's own folder after it");
    }
    let rest = normalize(&rest)?;
    let parts: Vec<String> = rest.split('/').map(str::to_lowercase).collect();
    if let Some(tree) = parts.iter().find(|part| PROTECTED_TREES.contains(&part.as_str())) {
        bail!("\"{template}\" is inside \"{tree}\", which holds data that isn't your app's to delete");
    }
    if HOME_TOOL_FOLDERS.contains(&parts[0].as_str()) {
        bail!("\"{template}\" is a folder of the user's tools or work (\"{}\"), not of your app; name your app's own folder", parts[0]);
    }
    if parts[0] == "library" {
        match parts.get(1) {
            None => bail!("\"{template}\" is the whole Library folder; name your app's own folder inside it"),
            Some(folder) if !LIBRARY_FOLDERS.contains(&folder.as_str()) => bail!(
                "\"{template}\" isn't in a place apps keep their data; use a folder in ~/Library/Application Support, Caches, \
                 Logs, Preferences or Containers"
            ),
            Some(_) if parts.len() < 3 => bail!("\"{template}\" is a whole Library folder; name your app's own folder inside it"),
            Some(_) => {}
        }
    } else if parts.len() == 1 && USER_FOLDERS.contains(&parts[0].as_str()) {
        bail!("\"{template}\" is a folder of the user, not of your app; name your app's own folder inside it, such as {template}/YourApp");
    }
    let below_library = if parts[0] == "library" { parts[2..].join("/") } else { parts.join("/") };
    if let Some(other) = OTHER_APPS.iter().find(|other| below_library == **other || below_library.starts_with(&format!("{other}/"))) {
        bail!("\"{template}\" is the data of another program (\"{other}\"), which isn't your app's to delete");
    }
    let own = parts.iter().skip(if parts[0] == "library" { 2 } else { 0 }).collect::<Vec<_>>();
    if own.len() == 1 && VENDOR_FOLDERS.contains(&own[0].as_str()) {
        bail!("\"{template}\" is a whole company's folder; name your app's own folder inside it, such as {template}/YourApp");
    }
    Ok(())
}

/// The real folder a checked data folder points at, below `home`. Refuses a
/// folder that is, or holds, the home folder itself.
pub fn expand_data_folder(template: &str, home: &Path) -> Option<PathBuf> {
    check_data_folder(template).ok()?;
    if home.as_os_str().is_empty() || !home.is_absolute() {
        return None;
    }
    let rest = normalize(&split(template)?).ok()?;
    let folder = under(home, &rest);
    (!super::is_same_or_inside(home, &folder)).then_some(folder)
}

/// Whether some part of the data folder looks like it belongs to the app (see
/// [`super::looks_like_apps_own`]).
pub fn looks_like_apps_own(template: &str, names: &[&str]) -> bool {
    let key = |text: &str| text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let Some(rest) = split(template) else {
        return false;
    };
    let wanted: Vec<String> = names.iter().map(|name| key(name)).filter(|name| name.chars().count() >= 3).collect();
    rest.split('/')
        .map(key)
        .filter(|part| !part.is_empty())
        .any(|part| wanted.iter().any(|name| part.contains(name.as_str()) || (part.chars().count() >= 3 && name.contains(part.as_str()))))
}

/// Checks that a symbolic link at `path` pointing at `target` stays inside
/// the install folder: the target is relative, has no empty parts and,
/// resolved from the link's own folder, never climbs above the top.
pub fn check_link(path: &str, target: &str) -> Result<()> {
    if target.is_empty() || target.starts_with('/') || target.contains('\\') || target.contains('\0') {
        bail!("the link \"{path}\" points at \"{target}\"; a link must point at a relative path inside the app");
    }
    // Where the link's own folder is, as a depth below the install folder.
    let mut depth = path.matches('/').count() as i64;
    for part in target.split('/') {
        match part {
            "" => bail!("the link \"{path}\" points at \"{target}\", which has an empty part"),
            "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    bail!("the link \"{path}\" points at \"{target}\", which climbs out of the install folder");
                }
            }
            _ => depth += 1,
        }
    }
    Ok(())
}
