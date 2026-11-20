//! The manifest: everything the setup window needs to know about the app it
//! installs. The builder writes it from `bach.toml`; the setup runtime reads
//! it back out of its own file. It's JSON, so a setup file can be inspected
//! with `bach inspect`.
//!
//! Fields added after Bach 0.1 all have defaults, so a newer runtime still
//! reads the uninstaller an older version installed (that's how an update
//! finds the files to clean up).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::i18n::{self, Key, Localized};
use crate::paths;

mod access;
mod choices;
pub mod extras;
pub(crate) mod features;
mod look_files;
mod scope;
pub mod system;
mod validate;

pub use access::{Access, Encryption, Slot};
pub use choices::{
    auto_update_setting as choices_auto_update, Choice, Choices, OPTION_AUTOSTART, OPTION_AUTO_UPDATE, OPTION_DESKTOP, OPTION_PATH,
    OPTION_START_MENU,
};
pub use extras::{ArchEntry, Check, DeltaEntry, FeedEntry, Requirement, RunStep, RunWhen, Shortcut, Updates};
pub use features::{Component, ComponentDownload, FileType, Protocol};
pub use look_files::{LookBytes, LookFile, LookFileKind};
pub use scope::Scope;
pub use system::{ContextMenuEntry, ContextTarget, EnvVar, Migrate, RegistryData, RegistryValue, RegistryView, SystemChanges};
pub use validate::MAX_LICENSE;

/// The uninstaller's file name in every install folder.
pub const UNINSTALLER: &str = "uninstall.exe";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// The payload format, [`crate::FORMAT_VERSION`].
    pub format: u32,
    /// The Bach version that built this setup file.
    pub builder: String,
    pub app: App,
    pub install: Install,
    pub uninstall: Uninstall,
    pub look: Look,
    /// Every packaged file, relative to the install folder. In an installed
    /// uninstaller, only the files that were installed: the uninstaller
    /// removes exactly these, and an update removes the ones a new version
    /// no longer installs.
    pub files: Vec<FileEntry>,
    /// Optional groups of files the user can leave out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<Component>,
    /// Link schemes like `hello://` that open the app.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protocols: Vec<Protocol>,
    /// File extensions the app offers to open.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_types: Vec<FileType>,
    #[serde(default)]
    pub locale: Locale,
    /// What the user chose. Only in an installed uninstaller, so an update
    /// offers the same choices and the uninstaller knows what to undo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chosen: Option<Choices>,
    /// What the app needs on the computer first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requirements: Vec<Requirement>,
    /// Start menu shortcuts besides the app's own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shortcuts: Vec<Shortcut>,
    /// Programs setup runs while installing or removing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub run: Vec<RunStep>,
    /// What the install changes in Windows besides files: registry values,
    /// environment variables and right-click menu entries.
    #[serde(default, skip_serializing_if = "SystemChanges::is_empty")]
    pub system: SystemChanges,
    /// Where the installed app checks for updates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updates: Option<Updates>,
    /// Asks for an access key before installing; see [`Access`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<Access>,
    /// An online installer carries no files. It downloads the newest setup
    /// file from the update feed when it runs and installs from that, so it
    /// never installs what was current when it was built.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub online: bool,
    /// SHA-256 of the archive as appended, checked before unpacking. Not
    /// carried by an installed uninstaller, which has no archive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_sha256: Option<String>,
    /// Set on an update patch: the version it updates from. A patch's archive
    /// holds only the files that changed since then; every other file of
    /// `files` must already be in the install folder with the recorded SHA-256.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_from: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct App {
    /// Names the registry entry; stays the same across versions.
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    /// One line: the shortcut's tooltip, and the install page's subtitle
    /// unless `[text] install_subtitle` replaces it.
    pub description: Option<String>,
    pub website: Option<String>,
    /// The program to start, relative to the install folder.
    pub executable: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Install {
    /// The folder's name under `%LOCALAPPDATA%\Programs`, and the name added
    /// to any folder the user picks instead.
    pub folder: String,
    /// Bach 0.1 wrote a bool here, which still reads as on or off.
    pub desktop_shortcut: Choice,
    #[serde(default = "Choice::always")]
    pub start_menu_shortcut: Choice,
    #[serde(default)]
    pub start_with_windows: Choice,
    #[serde(default)]
    pub add_to_path: Choice,
    /// The folder, inside the install folder, that goes on `PATH`; the
    /// install folder itself when None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_folder: Option<String>,
    /// Whether the user may pick another install folder.
    #[serde(default = "yes")]
    pub change_folder: bool,
    /// Whether the last page offers to open the app.
    #[serde(default = "yes")]
    pub open_button: bool,
    /// A folder in the Start menu for the shortcuts, like the publisher's
    /// name; None puts them at the top level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_menu_folder: Option<String>,
    /// Whether the last page links to `app.website`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub website_button: bool,
    /// Whether the app installs for the user or for every user of the computer.
    #[serde(default, skip_serializing_if = "Scope::is_user")]
    pub scope: Scope,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Uninstall {
    /// Folders such as `%APPDATA%\Hello`, deleted only when the user ticks
    /// "Also delete my data".
    pub data: Vec<String>,
    /// What lives in them, in a few words. Newer setup files keep this in
    /// `[text] delete_data_description` instead.
    pub data_description: Option<String>,
    /// A page (https) the last page of the uninstaller links to, to ask why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback_url: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Look {
    pub theme: Theme,
    /// `#rrggbb` for buttons, the progress bar and ticked boxes.
    pub accent: Option<String>,
    /// The logo stored next to the manifest, if any.
    pub logo: Option<LogoKind>,
    /// No logo at all, not even the Bach mark.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hide_logo: bool,
    /// The logo's height in points (32 to 96); 56 when None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo_height: Option<u16>,
    /// A PNG logo's width divided by its height, so a wide logo keeps its
    /// shape; the SVG and square logos leave it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo_aspect: Option<f32>,
    /// The built-in style (`installer.style`); Bach's own when None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// The developer's changes to it (`[style]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<crate::style::Custom>,
    /// How the window is arranged, sized and animated (`[layout]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<crate::style::layout::Layout>,
    /// Files stored after the logo in the look section, in order: fonts,
    /// a background picture and a side panel's picture. The logo is what comes before them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<LookFile>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Follow Windows' light or dark setting, live.
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogoKind {
    /// Drawn in one colour that follows the theme, like the Bach mark.
    Svg,
    /// Drawn as is, in full colour.
    Png,
}

/// The window's languages and the developer's own texts.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Locale {
    /// Language codes the window may use, the first being the fallback.
    /// Empty: every built-in language.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    /// Replacements for built-in texts, by [`Key`] name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub text: BTreeMap<String, Localized>,
    /// A license to accept before installing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<Localized>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Relative, `/`-separated, checked by [`paths::normalize`].
    pub path: String,
    pub size: u64,
    /// The optional component it belongs to; None means always installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    /// The file's SHA-256 (lowercase hex). Setup files built since patches
    /// exist record it: an update patch leaves unchanged files where they are
    /// and checks them against this before it relies on them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

impl FileEntry {
    pub fn new(path: impl Into<String>, size: u64) -> Self {
        Self { path: path.into(), size, component: None, sha256: None }
    }
}

impl Manifest {
    /// All packaged bytes.
    pub fn total_size(&self) -> u64 {
        self.files.iter().fold(0u64, |sum, file| sum.saturating_add(file.size))
    }

    /// The bytes the user's choices install, for the progress bar, the
    /// free-space check and Settings › Apps.
    pub fn selected_size(&self, choices: &Choices) -> u64 {
        self.files.iter().filter(|file| choices.installs(file)).fold(0u64, |sum, file| sum.saturating_add(file.size))
    }

    /// The program inside an install folder.
    pub fn executable_in(&self, dir: &Path) -> PathBuf {
        paths::under(dir, &self.app.executable)
    }

    /// The folder that goes on `PATH`.
    pub fn path_folder_in(&self, dir: &Path) -> PathBuf {
        match &self.install.path_folder {
            Some(folder) => paths::under(dir, folder),
            None => dir.to_path_buf(),
        }
    }

    /// The manifest the installed uninstaller carries: only the files these
    /// choices installed, and the choices themselves.
    pub fn as_installed(&self, choices: &Choices) -> Manifest {
        let mut installed = self.clone();
        installed.files.retain(|file| choices.installs(file));
        installed.chosen = Some(choices.clone());
        installed.online = false;
        installed.archive_sha256 = None;
        installed.patch_from = None;
        // The uninstaller has no files to unlock, but an update still
        // sends the key when `send_key` says so.
        if let Some(access) = &mut installed.access {
            access.encryption = None;
        }
        installed
    }

    /// The developer's text for `key` in `language`, if they replaced it.
    pub fn custom_text(&self, key: Key, language: &str) -> Option<&str> {
        let custom = self.locale.text.get(key.name()).and_then(|text| i18n::resolve(text, language, &self.locale.languages));
        match (custom, key) {
            (None, Key::delete_data_description) => self.uninstall.data_description.as_deref(),
            (custom, _) => custom,
        }
    }

    /// The license text in `language`, if there is one.
    pub fn license(&self, language: &str) -> Option<&str> {
        let license = self.locale.license.as_ref()?;
        i18n::resolve(license, language, &self.locale.languages)
    }

    /// Checks what a setup runtime relies on, so a damaged or hand-edited
    /// payload fails with a sentence instead of misbehaving.
    pub fn validate(&self) -> anyhow::Result<()> {
        validate::manifest(self)
    }
}

#[cfg(test)]
pub(crate) fn sample() -> Manifest {
    Manifest {
        format: crate::FORMAT_VERSION,
        builder: crate::VERSION.into(),
        app: App {
            id: "Hello".into(),
            name: "Hello".into(),
            version: "1.0.0".into(),
            publisher: "Example".into(),
            description: Some("Says hello.".into()),
            website: None,
            executable: "hello.exe".into(),
        },
        install: Install {
            folder: "Hello".into(),
            desktop_shortcut: Choice::On,
            start_menu_shortcut: Choice::Always,
            start_with_windows: Choice::Off,
            add_to_path: Choice::Never,
            path_folder: None,
            change_folder: true,
            open_button: true,
            start_menu_folder: None,
            website_button: false,
            scope: Scope::User,
        },
        uninstall: Uninstall { data: vec![r"%APPDATA%\Hello".into()], data_description: Some("Settings.".into()), feedback_url: None },
        look: Look::default(),
        files: vec![
            FileEntry::new("hello.exe", 10),
            FileEntry::new("data/readme.txt", 5),
            FileEntry { path: "samples/one.txt".into(), size: 3, component: Some("samples".into()), sha256: None },
        ],
        components: vec![Component {
            id: "samples".into(),
            title: [(i18n::ANY_LANGUAGE.to_string(), "Samples".to_string())].into(),
            description: None,
            default: false,
            download: None,
        }],
        protocols: vec![Protocol { scheme: "hello".into(), description: None }],
        file_types: vec![FileType { extension: ".hello".into(), description: Some("Hello note".into()) }],
        locale: Locale::default(),
        chosen: None,
        requirements: Vec::new(),
        shortcuts: vec![Shortcut {
            name: "Hello Docs".into(),
            target: "hello.exe".into(),
            arguments: Some("--docs".into()),
            description: None,
        }],
        run: Vec::new(),
        system: SystemChanges::default(),
        updates: None,
        access: None,
        online: false,
        archive_sha256: None,
        patch_from: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let manifest = sample();
        let json = serde_json::to_string(&manifest).unwrap();
        let back: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, manifest);
        assert_eq!(back.total_size(), 18);
        assert!(back.validate().is_ok());
    }

    /// A manifest exactly as Bach 0.1 wrote it, before options existed.
    #[test]
    fn reads_the_first_format() {
        let old = r#"{"format":1,"builder":"0.1.0",
            "app":{"id":"Hello","name":"Hello","version":"1.0.0","publisher":"Bach",
                   "description":null,"website":null,"executable":"hello.exe"},
            "install":{"folder":"Hello","desktop_shortcut":false},
            "uninstall":{"data":[],"data_description":"Hello's settings."},
            "look":{"theme":"system","accent":null,"logo":null},
            "files":[{"path":"hello.exe","size":10}]}"#;
        let manifest: Manifest = serde_json::from_str(old).unwrap();
        manifest.validate().unwrap();
        assert_eq!(manifest.install.desktop_shortcut, Choice::Off);
        assert_eq!(manifest.install.start_menu_shortcut, Choice::Always);
        assert!(manifest.install.change_folder && manifest.install.open_button);
        assert_eq!(manifest.custom_text(Key::delete_data_description, "de"), Some("Hello's settings."));
    }

    #[test]
    fn rejects_other_formats_and_bad_paths() {
        let mut manifest = sample();
        manifest.format = 99;
        assert!(manifest.validate().is_err());

        let mut manifest = sample();
        manifest.files.push(FileEntry::new("../evil.dll", 1));
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn a_setup_file_with_a_personal_data_folder_is_rejected() {
        for folder in [r"%USERPROFILE%\Documents", r"%APPDATA%\Microsoft", r"%USERPROFILE%\.ssh"] {
            let mut manifest = sample();
            manifest.uninstall.data.push(folder.to_string());
            assert!(manifest.validate().is_err(), "{folder} should be rejected");
        }
        let mut manifest = sample();
        manifest.uninstall.data.push(r"%APPDATA%\Hello".to_string());
        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn the_installed_manifest_lists_what_was_installed() {
        let manifest = sample();
        let mut choices = Choices::defaults(&manifest);
        assert_eq!(manifest.as_installed(&choices).files.len(), 2);
        choices.components.push("samples".into());
        let installed = manifest.as_installed(&choices);
        assert_eq!(installed.files.len(), 3);
        assert_eq!(installed.chosen, Some(choices));
        assert!(installed.validate().is_ok());
    }

    #[test]
    fn custom_texts_win() {
        let mut manifest = sample();
        let text: Localized = [("de".to_string(), "Los".to_string()), ("en".to_string(), "Go".to_string())].into();
        manifest.locale.text.insert("install_button".into(), text);
        assert_eq!(manifest.custom_text(Key::install_button, "de"), Some("Los"));
        assert_eq!(manifest.custom_text(Key::install_button, "fr"), Some("Go"));
        assert_eq!(manifest.custom_text(Key::cancel, "de"), None);
    }

    #[test]
    fn names_that_windows_takes_for_one_file_are_listed_twice() {
        let mut manifest = sample();
        let template = manifest.files[0].clone();
        for (a, b) in [("a.txt", "A.TXT"), ("σ.txt", "ς.txt")] {
            manifest.files = vec![FileEntry { path: a.into(), ..template.clone() }, FileEntry { path: b.into(), ..template.clone() }];
            let err = manifest.validate().unwrap_err().to_string();
            assert!(err.contains("listed twice"), "{a} and {b}: {err}");
        }
    }
}
