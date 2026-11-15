//! Changes setup makes to Windows besides copying files: registry values,
//! environment variables and Explorer's right-click menu.
//!
//! Everything here is per user (`HKCU`) unless the install is machine-wide
//! (`install.scope`), and everything is undone by the uninstaller: setup only
//! ever deletes what its own manifest lists, and only while it still holds
//! the value setup wrote.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// What an install changes in Windows. Empty for most apps.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemChanges {
    /// Registry values written under `Software\…`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registry: Vec<RegistryValue>,
    /// Environment variables of the user.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub environment: Vec<EnvVar>,
    /// Entries in Explorer's right-click menu.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_menu: Vec<ContextMenuEntry>,
    /// Older installs of the same app, made by another installer, that a
    /// first install removes before it puts the files in place.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub migrate: Vec<Migrate>,
}

impl SystemChanges {
    pub fn is_empty(&self) -> bool {
        self.registry.is_empty() && self.environment.is_empty() && self.context_menu.is_empty() && self.migrate.is_empty()
    }
}

/// An older install of the app that another installer made (Inno Setup, NSIS,
/// WiX or an MSI), listed under Settings › Apps. A first install with Bach
/// removes it first, with the uninstall command Windows already holds for it,
/// so the app isn't left twice on the computer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Migrate {
    /// The name of its entry under
    /// `Software\Microsoft\Windows\CurrentVersion\Uninstall`: `{GUID}_is1` for
    /// Inno Setup, the product code `{GUID}` for an MSI, the app's own name
    /// for NSIS.
    pub uninstall_key: String,
    /// Arguments that make the old uninstaller run without a window, for an
    /// entry that has no `QuietUninstallString` (`/VERYSILENT /NORESTART` for
    /// Inno Setup, `/S` for NSIS).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    /// Also remove an old install that is machine-wide (setup asks Windows
    /// for administrator rights then). Without it, only per-user installs go.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub elevated: bool,
}

pub fn check_migrate(entry: &Migrate) -> Result<()> {
    let key = entry.uninstall_key.trim();
    if key.is_empty() || key.len() > 128 || key.contains(['\\', '/']) || key.chars().any(char::is_control) {
        bail!("\"{}\" can't be the name of an Uninstall entry: use the entry's name only, like {{GUID}}_is1", entry.uninstall_key);
    }
    if entry.arguments.as_deref().is_some_and(|arguments| arguments.len() > 500 || arguments.contains('\0')) {
        bail!("the arguments for removing \"{key}\" are too long or damaged");
    }
    Ok(())
}

/// One registry value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryValue {
    /// The key, below the hive and starting with `Software\`, like
    /// `Software\Vendor\App`.
    pub key: String,
    /// The value's name; empty for the key's default value.
    #[serde(default)]
    pub name: String,
    pub data: RegistryData,
}

/// A registry value's content. `{app}` in text stands for the install folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum RegistryData {
    String(String),
    /// Text with `%VARIABLES%` Windows fills in when it's read.
    Expand(String),
    Dword(u32),
}

/// A user environment variable (`HKCU\Environment`). `{app}` in the value
/// stands for the install folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

/// An entry in Explorer's right-click menu that starts the app.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextMenuEntry {
    /// The menu text.
    pub name: String,
    /// What it shows up on.
    pub on: Vec<ContextTarget>,
    /// The program's command line after its path. `"%1"` (the clicked file or
    /// folder) when left out, `"%V"` (the open folder) for the background.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

/// Where a [`ContextMenuEntry`] appears.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum ContextTarget {
    /// Every file.
    Files,
    /// Every folder.
    Folders,
    /// The empty space inside an open folder.
    Background,
    /// Files with this extension: `.txt`.
    Extension(String),
}

impl ContextTarget {
    pub fn parse(text: &str) -> Result<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "files" => Ok(Self::Files),
            "folders" => Ok(Self::Folders),
            "background" => Ok(Self::Background),
            extension if super::features::is_valid_extension(extension) => Ok(Self::Extension(extension.to_string())),
            other => bail!("\"{other}\" isn't \"files\", \"folders\", \"background\" or a file extension like \".txt\""),
        }
    }

    /// The key below `Software\Classes` the menu entry's verb goes under.
    pub fn class(&self) -> String {
        match self {
            Self::Files => "*".into(),
            Self::Folders => "Directory".into(),
            Self::Background => r"Directory\Background".into(),
            Self::Extension(extension) => format!(r"SystemFileAssociations\{extension}"),
        }
    }

    /// The default command-line argument: the clicked item, or the open
    /// folder for the background.
    pub fn default_argument(&self) -> &'static str {
        match self {
            Self::Background => "\"%V\"",
            _ => "\"%1\"",
        }
    }
}

impl TryFrom<String> for ContextTarget {
    type Error = anyhow::Error;
    fn try_from(text: String) -> Result<Self> {
        Self::parse(&text)
    }
}

impl From<ContextTarget> for String {
    fn from(target: ContextTarget) -> String {
        match target {
            ContextTarget::Files => "files".into(),
            ContextTarget::Folders => "folders".into(),
            ContextTarget::Background => "background".into(),
            ContextTarget::Extension(extension) => extension,
        }
    }
}

impl ContextMenuEntry {
    /// The verb's key name: `Bach.<app>.<menu text>`, letters and digits only,
    /// so it's plainly this app's own key.
    ///
    /// A text in a script with no ASCII letters (Chinese, Japanese, Korean,
    /// Cyrillic, Arabic, ...) has nothing to show there, so those parts also
    /// carry the first 8 hex digits of the text's SHA-256: `Bach.Hello._ab12cd34`.
    /// Texts and ids in Latin script keep the key they have always had, so an
    /// installed app's entries are still found on update and removal.
    pub fn verb(&self, app_id: &str) -> String {
        format!("Bach.{}.{}", key_part(app_id), key_part(&self.name))
    }
}

/// One part of a key name: its ASCII letters and digits, plus a short hash of
/// the whole text when it is written in a non-Latin script.
fn key_part(text: &str) -> String {
    let mut part: String = text.chars().filter(char::is_ascii_alphanumeric).collect();
    // From U+0250 on, the letters are no longer Latin (U+0250 starts the IPA block).
    if text.chars().any(|c| c.is_alphabetic() && c as u32 >= 0x250) {
        let hash = crate::protect::sha256_hex(text.as_bytes()).unwrap_or_default();
        part.push('_');
        part.push_str(&hash[..hash.len().min(8)]);
    }
    part
}

/// `{app}` is the install folder in registry data and environment values.
pub fn expand_app(template: &str, dir: &std::path::Path) -> String {
    template.replace("{app}", &dir.to_string_lossy())
}

/// Longest registry text or environment value setup writes.
pub const MAX_VALUE: usize = 4096;

/// Keys below `Software` that Windows, other programs or Bach itself own.
const PROTECTED_KEYS: &[&str] = &[
    "classes",
    "policies",
    "wow6432node",
    "bach",
    // Whole vendors whose keys hold security settings, trust lists, browser
    // extension hooks and shell registrations: Office trusted locations,
    // Defender exclusions, certificates, native messaging hosts and the like.
    "microsoft",
    "google",
    "mozilla",
    "mozillaplugins",
    "chromium",
    "registeredapplications",
    "clients",
    "odbc",
];

/// Checks a registry key from `bach.toml` or a payload.
pub fn check_registry_key(key: &str) -> Result<()> {
    let parts: Vec<&str> = key.split('\\').collect();
    if parts.len() < 2 || !parts[0].eq_ignore_ascii_case("software") {
        bail!("the registry key \"{key}\" must be below Software\\, like Software\\Vendor\\App");
    }
    if parts.iter().any(|part| part.trim().is_empty() || part.len() > 255 || part.chars().any(char::is_control)) {
        bail!("the registry key \"{key}\" has an empty or unusable part");
    }
    let below = parts[1..].join("\\").to_ascii_lowercase();
    let protected = PROTECTED_KEYS.iter().any(|reserved| below == *reserved || below.starts_with(&format!("{reserved}\\")));
    if protected {
        bail!("the registry key \"{key}\" belongs to Windows, another program or Bach; use a key of your own, like Software\\Vendor\\App");
    }
    Ok(())
}

pub fn check_registry_value(value: &RegistryValue) -> Result<()> {
    check_registry_key(&value.key)?;
    if value.name.len() > 255 || value.name.chars().any(char::is_control) {
        bail!("the registry value name \"{}\" is unusable", value.name);
    }
    match &value.data {
        RegistryData::String(text) | RegistryData::Expand(text) if text.len() > MAX_VALUE || text.contains('\0') => {
            bail!("the registry value \"{}\" is too long or has a NUL in it", value.name)
        }
        _ => Ok(()),
    }
}

/// Variables that load code into other programs or redirect their network
/// traffic and trust. Never settable, whatever the config says.
const INJECTION_VARIABLES: &[&str] = &[
    "COR_ENABLE_PROFILING",
    "COR_PROFILER",
    "COR_PROFILER_PATH",
    "CORECLR_ENABLE_PROFILING",
    "CORECLR_PROFILER",
    "CORECLR_PROFILER_PATH",
    "DOTNET_STARTUP_HOOKS",
    "DOTNET_GCHEAPAFFINITIZEMASK",
    "JAVA_TOOL_OPTIONS",
    "_JAVA_OPTIONS",
    "JDK_JAVA_OPTIONS",
    "NODE_OPTIONS",
    "NODE_EXTRA_CA_CERTS",
    "NODE_TLS_REJECT_UNAUTHORIZED",
    "PYTHONSTARTUP",
    "PYTHONPATH",
    "PYTHONHOME",
    "PERL5OPT",
    "PERL5LIB",
    "RUBYOPT",
    "RUBYLIB",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
    "SSLKEYLOGFILE",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
];

/// Variables that make Windows or other programs misbehave when changed.
const PROTECTED_VARIABLES: &[&str] = &[
    "PATH",
    "PATHEXT",
    "TEMP",
    "TMP",
    "USERNAME",
    "USERPROFILE",
    "USERDOMAIN",
    "HOMEDRIVE",
    "HOMEPATH",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "COMSPEC",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    "PROGRAMDATA",
    "COMMONPROGRAMFILES",
    "APPDATA",
    "LOCALAPPDATA",
    "ALLUSERSPROFILE",
    "PUBLIC",
    "COMPUTERNAME",
    "LOGONSERVER",
    "OS",
    "PSMODULEPATH",
    "DRIVERDATA",
    "SESSIONNAME",
    "NUMBER_OF_PROCESSORS",
];

pub fn check_env_var(var: &EnvVar) -> Result<()> {
    let mut chars = var.name.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && var.name.len() <= 128
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        bail!("\"{}\" can't name an environment variable: use letters, digits and _, not starting with a digit", var.name);
    }
    let upper = var.name.to_ascii_uppercase();
    if PROTECTED_VARIABLES.contains(&upper.as_str()) || upper.starts_with("PROCESSOR_") || upper.starts_with("BACH_") {
        bail!("the environment variable {} belongs to Windows or Bach; choose a name of your own", var.name);
    }
    if INJECTION_VARIABLES.contains(&upper.as_str()) || upper.starts_with("DOTNET_") && upper.contains("PROFIL") {
        bail!("the environment variable {} loads code into other programs or redirects their traffic, so Bach never sets it", var.name);
    }
    if var.value.is_empty() || var.value.len() > MAX_VALUE || var.value.contains('\0') {
        bail!("the environment variable {} has an empty, too long or damaged value", var.name);
    }
    Ok(())
}

/// What an app's own environment variables start with: its id in capitals,
/// every other character an underscore (`My App` gives `MY_APP_`).
pub fn env_prefix(app_id: &str) -> String {
    let mut prefix: String = app_id.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect();
    prefix.push('_');
    prefix
}

/// Whether a registry key's vendor folder (`Software\<vendor>`) is named
/// after the publisher, the app or its id, ignoring case and punctuation.
pub fn key_is_named_after(key: &str, names: &[&str]) -> bool {
    let simple = |text: &str| text.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
    let Some(vendor) = key.split('\\').nth(1) else {
        return false;
    };
    let vendor = simple(vendor);
    !vendor.is_empty() && names.iter().any(|name| simple(name) == vendor)
}

pub fn check_context_entry(entry: &ContextMenuEntry) -> Result<()> {
    let name = entry.name.trim();
    if name.is_empty() || name.len() > 80 || name.chars().any(char::is_control) || entry.verb("x").ends_with('.') {
        bail!("\"{}\" can't be a menu text: use 1 to 80 characters with at least one letter or digit", entry.name);
    }
    if entry.on.is_empty() {
        bail!("the menu entry \"{name}\" shows up nowhere; say where with on = \"files\"");
    }
    if entry.arguments.as_deref().is_some_and(|arguments| arguments.len() > 500 || arguments.contains('\0')) {
        bail!("the arguments of the menu entry \"{name}\" are too long or damaged");
    }
    Ok(())
}

/// Checks everything in `changes`, including that no two menu entries would
/// share a key.
pub fn check(changes: &SystemChanges) -> Result<()> {
    for value in &changes.registry {
        check_registry_value(value)?;
    }
    for var in &changes.environment {
        check_env_var(var)?;
    }
    for entry in &changes.migrate {
        check_migrate(entry)?;
    }
    let mut verbs = std::collections::HashSet::new();
    for entry in &changes.context_menu {
        check_context_entry(entry)?;
        if !verbs.insert(entry.verb("").to_ascii_lowercase()) {
            bail!("the menu entries \"{}\" and another one have names that only differ in punctuation", entry.name);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_stay_below_software_and_out_of_windows() {
        assert!(check_registry_key(r"Software\Vendor\App").is_ok());
        for bad in [
            r"Vendor\App",
            "Software",
            r"Software\",
            r"Software\Classes\.txt",
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            r"Software\Microsoft\Windows NT\CurrentVersion",
            r"Software\Policies\Vendor",
            r"Software\Bach\Access",
            r"Software\\App",
        ] {
            assert!(check_registry_key(bad).is_err(), "{bad}");
        }
        assert!(check_registry_key(r"Software\Microsoftish\App").is_ok());
        assert!(check_registry_key(r"Software\Vendor\App").is_ok());
        assert!(check_registry_key(r"Software\My Vendor\My App\Settings").is_ok());
    }

    #[test]
    fn whole_vendors_with_security_settings_are_off_limits() {
        for bad in [
            r"Software\Microsoft\Office\16.0\Word\Security\Trusted Locations\Location9",
            r"Software\Microsoft\Office\16.0\Word\Security\VBAWarnings",
            r"Software\Microsoft\Windows Defender\Exclusions\Paths",
            r"Software\Microsoft\Windows Script Host\Settings",
            r"Software\Microsoft\SystemCertificates\Root\Certificates",
            r"Software\Microsoft\Cryptography\Defaults",
            r"Software\Google\Chrome\NativeMessagingHosts\x",
            r"Software\Mozilla\NativeMessagingHosts\x",
            r"Software\Microsoft\Edge\Extensions",
            r"Software\RegisteredApplications",
            r"software\MICROSOFT\x",
        ] {
            assert!(check_registry_key(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn keys_named_after_the_publisher_or_app() {
        assert!(key_is_named_after(r"Software\Example Corp\Hello", &["Example Corp", "Hello", "Hello"]));
        assert!(key_is_named_after(r"Software\HELLO\x", &["Example", "Hello"]));
        assert!(key_is_named_after(r"Software\Example-Corp", &["Example Corp"]));
        assert!(!key_is_named_after(r"Software\Other\Hello", &["Example", "Hello"]));
        assert!(!key_is_named_after("Software", &["Hello"]));
    }

    #[test]
    fn registry_data_round_trips() {
        let value = RegistryValue { key: r"Software\V\A".into(), name: "Dir".into(), data: RegistryData::Expand("{app}".into()) };
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(json, r#"{"key":"Software\\V\\A","name":"Dir","data":{"type":"expand","value":"{app}"}}"#);
        assert_eq!(serde_json::from_str::<RegistryValue>(&json).unwrap(), value);
        let dword: RegistryValue = serde_json::from_str(r#"{"key":"Software\\V","data":{"type":"dword","value":7}}"#).unwrap();
        assert_eq!(dword.data, RegistryData::Dword(7));
    }

    #[test]
    fn variables_leave_windows_alone() {
        let ok = EnvVar { name: "HELLO_HOME".into(), value: "{app}".into() };
        assert!(check_env_var(&ok).is_ok());
        for name in ["Path", "windir", "PROCESSOR_ARCHITECTURE", "BACH_ACCESS_KEY", "1X", "A B", ""] {
            assert!(check_env_var(&EnvVar { name: name.into(), value: "x".into() }).is_err(), "{name}");
        }
        assert!(check_env_var(&EnvVar { name: "X".into(), value: String::new() }).is_err());
    }

    #[test]
    fn variables_that_inject_code_or_redirect_traffic_are_never_set() {
        for name in [
            "COR_ENABLE_PROFILING",
            "COR_PROFILER",
            "CORECLR_PROFILER_PATH",
            "DOTNET_STARTUP_HOOKS",
            "JAVA_TOOL_OPTIONS",
            "_JAVA_OPTIONS",
            "NODE_OPTIONS",
            "PYTHONSTARTUP",
            "PYTHONPATH",
            "PERL5OPT",
            "HTTPS_PROXY",
            "https_proxy",
            "NODE_EXTRA_CA_CERTS",
            "SSL_CERT_FILE",
            "SSLKEYLOGFILE",
            "DOTNET_PROFILER_X",
            "LD_PRELOAD",
        ] {
            assert!(check_env_var(&EnvVar { name: name.into(), value: "x".into() }).is_err(), "{name}");
        }
        assert!(check_env_var(&EnvVar { name: "JAVA_HOME".into(), value: "x".into() }).is_ok(), "ordinary names pass the hard list");
    }

    #[test]
    fn an_apps_variables_start_with_its_id() {
        assert_eq!(env_prefix("Hello"), "HELLO_");
        assert_eq!(env_prefix("My App-2"), "MY_APP_2_");
    }

    #[test]
    fn menu_targets_and_verbs() {
        assert_eq!(ContextTarget::parse(" Files ").unwrap(), ContextTarget::Files);
        assert_eq!(ContextTarget::parse(".md").unwrap(), ContextTarget::Extension(".md".into()));
        assert!(ContextTarget::parse(".exe").is_err());
        assert!(ContextTarget::parse("desktop").is_err());
        assert_eq!(ContextTarget::Background.class(), r"Directory\Background");
        assert_eq!(ContextTarget::Background.default_argument(), "\"%V\"");
        let entry = ContextMenuEntry { name: "Open with Hello!".into(), on: vec![ContextTarget::Files], arguments: None };
        assert_eq!(entry.verb("My App"), "Bach.MyApp.OpenwithHello");
        let json = serde_json::to_string(&entry).unwrap();
        assert_eq!(json, r#"{"name":"Open with Hello!","on":["files"]}"#);
        assert_eq!(serde_json::from_str::<ContextMenuEntry>(&json).unwrap(), entry);
        assert!(serde_json::from_str::<ContextMenuEntry>(r#"{"name":"x","on":["nowhere"]}"#).is_err());
    }

    #[test]
    fn menu_texts_in_any_script_get_their_own_keys() {
        let entry = |name: &str| ContextMenuEntry { name: name.into(), on: vec![ContextTarget::Files], arguments: None };
        let texts = [
            "\u{6253}\u{5f00}",
            "\u{5f00}\u{59cb}",
            "\u{c5f4}\u{ae30}",
            "\u{41e}\u{442}\u{43a}\u{440}\u{44b}\u{442}\u{44c}",
            "\u{627}\u{641}\u{62a}\u{62d}",
        ];
        let mut verbs = std::collections::HashSet::new();
        for text in texts {
            check_context_entry(&entry(text)).unwrap_or_else(|err| panic!("{text}: {err}"));
            let verb = entry(text).verb("Hello");
            assert!(verb.starts_with("Bach.Hello._") && verb.is_ascii() && !verb.ends_with('.'), "{verb}");
            assert!(verbs.insert(verb), "{text} needs its own key");
        }
        // Two entries that differ only in CJK, next to a shared Latin word.
        assert_ne!(entry("\u{6253}\u{5f00}\u{ff08}Hello\u{ff09}").verb("x"), entry("\u{5173}\u{95ed}\u{ff08}Hello\u{ff09}").verb("x"));
        // An app id in a non-Latin script gets a key part too.
        assert!(entry("Open").verb("\u{6d4b}\u{8bd5}").starts_with("Bach._"));
        // Latin texts keep the key they always had, accents dropped as before.
        assert_eq!(entry("Mit Hello \u{f6}ffnen").verb("Hello"), "Bach.Hello.MitHelloffnen");
        assert_eq!(entry("Open with Hello!").verb("My App"), "Bach.MyApp.OpenwithHello");
        // Only punctuation is still refused, and said so.
        assert!(check_context_entry(&entry("!!!")).unwrap_err().to_string().contains("letter or digit"));
    }

    #[test]
    fn menu_entries_are_checked() {
        let entry = |name: &str, on: Vec<ContextTarget>| ContextMenuEntry { name: name.into(), on, arguments: None };
        assert!(check_context_entry(&entry("Open", vec![ContextTarget::Files])).is_ok());
        assert!(check_context_entry(&entry("!!!", vec![ContextTarget::Files])).is_err(), "nothing to name the key after");
        assert!(check_context_entry(&entry("Open", vec![])).is_err());
        let twin = SystemChanges {
            context_menu: vec![entry("Open it", vec![ContextTarget::Files]), entry("Open-it", vec![ContextTarget::Folders])],
            ..SystemChanges::default()
        };
        assert!(check(&twin).is_err());
    }

    #[test]
    fn old_installs_are_named_by_their_uninstall_entry() {
        let entry = |key: &str| Migrate { uninstall_key: key.into(), arguments: None, elevated: false };
        assert!(check_migrate(&entry("{8A69D345-D564-463C-AFF1-A69D9E530F96}_is1")).is_ok());
        assert!(check_migrate(&entry("Hello App")).is_ok());
        for bad in ["", "  ", r"Software\Hello", "a/b", "bad\u{7}key"] {
            assert!(check_migrate(&entry(bad)).is_err(), "{bad:?}");
        }
        let json = serde_json::to_string(&entry("Hello")).unwrap();
        assert_eq!(json, r#"{"uninstall_key":"Hello"}"#);
    }

    #[test]
    fn app_folder_is_filled_in() {
        assert_eq!(expand_app(r"{app}\bin", std::path::Path::new(r"C:\Apps\Hello")), r"C:\Apps\Hello\bin");
    }
}
