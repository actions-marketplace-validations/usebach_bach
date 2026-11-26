//! Requirements, extra shortcuts and the update feed.

use serde::{Deserialize, Serialize};

use super::Choice;

/// Something the app needs on the computer before it can run. Setup only
/// looks (read-only, no administrator rights needed) and, when it's
/// missing, sends the user to `url` to get it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requirement {
    #[serde(flatten)]
    pub check: Check,
    /// What people call it: "Microsoft Edge WebView2".
    pub name: String,
    /// Where to get it (https).
    pub url: String,
}

/// How setup tells whether a requirement is there.
/// The oldest Windows Bach supports: Windows 10 version 1809 (build 17763) and
/// Windows 11. Setup files are tested on Windows 11 (and Windows Server 2022,
/// which shares Windows 10 1809's kernel generation), and the update feature
/// needs the `curl.exe` that Windows 10 has had since 1803.
pub const MIN_WINDOWS_BUILD: u32 = 17_763;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Check {
    /// The Edge WebView2 runtime (Tauri, WebView2 apps).
    Webview2,
    /// A .NET runtime of this major.minor version, like "8.0". `desktop`
    /// asks for the Windows Desktop runtime (WinForms, WPF).
    Dotnet { version: String, desktop: bool },
    /// The Visual C++ 2015-2022 redistributable (x64).
    Vcredist,
    /// A file, like `%SystemRoot%\System32\foo.dll`.
    File { path: String },
    /// A registry key, like `HKLM\SOFTWARE\Vendor\Thing`.
    Registry { key: String },
    /// Windows itself, this build number or newer (Windows 10 is 10240,
    /// 1809 is 17763, 2004 is 19041, Windows 11 is 22000).
    Windows { build: u32 },
    /// macOS itself, this version or newer (`macos.minimum_version`): `12.0`, `13.4`.
    Macos { version: String },
}

impl Check {
    /// The name and download page Bach uses when the config gives none.
    pub fn defaults(&self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Webview2 => Some(("Microsoft Edge WebView2", "https://go.microsoft.com/fwlink/p/?LinkId=2124703")),
            Self::Dotnet { desktop: true, .. } => Some((".NET Desktop Runtime", "https://dotnet.microsoft.com/download/dotnet")),
            Self::Dotnet { desktop: false, .. } => Some((".NET Runtime", "https://dotnet.microsoft.com/download/dotnet")),
            Self::Vcredist => Some(("Microsoft Visual C++ Redistributable", "https://aka.ms/vs/17/release/vc_redist.x64.exe")),
            Self::File { .. } | Self::Registry { .. } | Self::Windows { .. } | Self::Macos { .. } => None,
        }
    }
}

/// A program from the app's own files that setup runs: something the app
/// needs set up (a runtime's installer, a driver) rather than copied.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStep {
    /// What setup says it is doing: "Setting up {name}".
    pub name: String,
    /// The program, a packaged file, relative to the install folder.
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    pub when: RunWhen,
    /// Exit codes that mean it worked. Always at least `0`.
    #[serde(default = "zero")]
    pub success_codes: Vec<i32>,
    /// Go on when it fails (installing and updating only: removing never
    /// stops for a failed step).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ignore_errors: bool,
    /// Only when this optional component is installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    /// Run it with administrator rights: Windows asks the user once (or not
    /// at all when setup is elevated already).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub elevated: bool,
    /// How many minutes the step may run before setup ends it and everything it
    /// started. None means [`DEFAULT_TIMEOUT_MINUTES`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_minutes: Option<u32>,
}

/// How long a run step may take unless it says otherwise.
pub const DEFAULT_TIMEOUT_MINUTES: u32 = 15;
/// The longest a run step may be given.
pub const MAX_TIMEOUT_MINUTES: u32 = 120;

fn zero() -> Vec<i32> {
    vec![0]
}

/// When a [`RunStep`] runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunWhen {
    /// After the files are in place, on a first install and on every update.
    Install,
    /// Like `Install`, but not on an update.
    FirstInstall,
    /// Before the files are removed.
    Uninstall,
}

impl RunWhen {
    pub fn parse(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "install" => Some(Self::Install),
            "first_install" => Some(Self::FirstInstall),
            "uninstall" => Some(Self::Uninstall),
            _ => None,
        }
    }
}

impl RunStep {
    /// Whether the exit code `code` means success.
    pub fn succeeded(&self, code: i32) -> bool {
        self.success_codes.contains(&code)
    }

    /// How long setup lets this step run.
    pub fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(u64::from(self.timeout_minutes.unwrap_or(DEFAULT_TIMEOUT_MINUTES)) * 60)
    }
}

/// A Start menu shortcut besides the app's own, to another program in the
/// install folder or to the app with arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shortcut {
    /// The shortcut's name, as the Start menu shows it.
    pub name: String,
    /// The program, relative to the install folder.
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Where the installed app looks for a newer setup file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Updates {
    /// An https URL serving the JSON `bach build` writes next to the setup
    /// file: `{ "version", "url", "sha256", "notes" }`.
    pub feed: String,
    /// Whether the installed app keeps itself up to date in the background
    /// (a per-user scheduled task). `Never` is also what a setup file built
    /// before this option existed reads as, so an update offers it fresh.
    #[serde(default)]
    pub auto: Choice,
    /// More feeds to ask when `feed` can't be reached, each a full feed with
    /// its own download link. The first feed that answers decides the newest
    /// version; the others only serve as other places to download the same
    /// file (same version and SHA-256) from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mirrors: Vec<String>,
    /// The publisher's Ed25519 public key (hex). With it, the installed app
    /// accepts only feed entries signed by the matching private key, whichever
    /// host or mirror they came from. Without it, feeds are trusted as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
    /// A second key that is trusted too, to hand over from one key to the next
    /// without stranding installed copies: release a version that lists the new
    /// key here, still signed with the old one, then switch (see docs/signing.md).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_public_key: Option<String>,
    /// The name of the publisher a downloaded update has to be signed by
    /// (Authenticode): the common name or organisation of the signing
    /// certificate, as Windows shows it. An update that is unsigned, not valid
    /// for Windows, or signed by anyone else is refused. On a Mac it is the name
    /// of an Apple-issued Developer ID Application certificate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signer: Option<String>,
    /// The feed of the beta channel (https): what an installation that chose
    /// beta versions asks instead of `feed` and `mirrors`. It is the same JSON,
    /// and the publisher's keys and signer apply to it as well.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beta_feed: Option<String>,
    /// How old, in days, a signed feed entry may be (counted from its
    /// `published` time) before the installed app ignores it. Needs `public_key`.
    /// Left out, an entry never expires. The publisher has to sign the feed
    /// again (`bach feed`) within that time, release or not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_days: Option<u32>,
}

/// A distinguished name like `CN=Example Inc., O=Example, C=DE` as its
/// `(key, value)` pairs, keys in upper case. A value may be quoted and then
/// holds commas of its own.
fn name_parts(subject: &str) -> Vec<(String, String)> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in subject.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => parts.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    parts.push(current);
    parts
        .iter()
        .filter_map(|part| part.trim().split_once('='))
        .map(|(key, value)| (key.trim().to_ascii_uppercase(), value.trim().to_string()))
        .collect()
}

/// Whether a certificate `subject` is the publisher `wanted`. A plain name
/// (`Example Inc.`) is the subject's common name or organisation, the whole
/// text, any case. A whole distinguished name (`CN=Example Inc., O=Example,
/// L=Berlin, C=DE`) has to be the subject exactly, pair for pair: that is how
/// to tell two companies of the same name apart. A subject with a control
/// character in it is nobody's.
pub fn subject_names(subject: &str, wanted: &str) -> bool {
    let wanted = wanted.trim();
    if wanted.is_empty() || subject.chars().any(char::is_control) {
        return false;
    }
    let found = name_parts(subject);
    if wanted.contains('=') {
        let asked = name_parts(wanted);
        return !asked.is_empty()
            && asked.len() == found.len()
            && asked.iter().zip(&found).all(|((ak, av), (fk, fv))| ak == fk && av.eq_ignore_ascii_case(fv));
    }
    found.iter().any(|(key, value)| matches!(key.as_str(), "CN" | "O") && value.eq_ignore_ascii_case(wanted))
}

impl Updates {
    /// Every feed to ask, the main one first.
    pub fn feeds(&self) -> Vec<String> {
        std::iter::once(self.feed.clone()).chain(self.mirrors.iter().cloned()).collect()
    }

    /// The feeds an installation asks: the beta channel's, when it chose beta
    /// and there is one, else [`Self::feeds`].
    pub fn feeds_for(&self, beta: bool) -> Vec<String> {
        match (&self.beta_feed, beta) {
            (Some(feed), true) => vec![feed.clone()],
            _ => self.feeds(),
        }
    }
}

/// The JSON a feed serves.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedEntry {
    pub version: String,
    /// The setup file to download (https).
    pub url: String,
    /// Its SHA-256, lowercase hex; a download that doesn't match is refused.
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// The setup file's size in bytes, so an update can check the disk has
    /// room before it downloads anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// A staged rollout: the percentage of computers (0 to 100) that should
    /// take this version by themselves. Left out, everyone does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollout: Option<u8>,
    /// A smaller download for computers that have exactly `from` installed:
    /// a patch with only the files that changed. Anything else downloads the
    /// full setup file above, as does a patch that can't be applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<DeltaEntry>,
    /// The publisher's signature over this entry (hex), see `feedsig`. Feeds
    /// of apps without `[updates] public_key` don't need one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// When the publisher signed this entry, `2026-11-17T06:30:00Z` (UTC). It is
    /// part of what the signature covers, and lets an app that sets
    /// `[updates] max_age_days` refuse an old entry a taken-over host keeps serving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
    /// Setup files for other processor architectures, by name (`arm64`). The
    /// entry's own link, SHA-256 and size are the x64 setup file; a computer
    /// that runs another architecture takes its own file from here, so one
    /// feed (and one online installer) serves them all. Each has its own
    /// signature, so an app that checks signatures still can't be handed a
    /// file the publisher didn't sign.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub arch: std::collections::BTreeMap<String, ArchEntry>,
}

/// The setup file a [`FeedEntry`] offers to computers of another architecture.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchEntry {
    /// The setup file (https).
    pub url: String,
    /// Its SHA-256, lowercase hex.
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// The publisher's signature over this file's entry (hex), see `feedsig`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

/// The architectures a feed can list besides x64.
pub const OTHER_ARCHES: &[&str] = &["arm64"];

/// The patch a [`FeedEntry`] offers to computers that have one version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeltaEntry {
    /// The version it updates from.
    pub from: String,
    /// The patch (a setup file with only the changed files), https.
    pub url: String,
    /// Its SHA-256, lowercase hex.
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// Whether `url` is an https URL (the only kind updates and requirement
/// links may use).
pub fn is_https(url: &str) -> bool {
    url.strip_prefix("https://").is_some_and(|rest| !rest.is_empty() && !rest.contains(char::is_whitespace))
}

/// Whether `name` can name a `.lnk` file.
pub fn is_valid_shortcut_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && name.len() <= 100 && !name.chars().any(|c| r#"<>:"/\|?*"#.contains(c) || c.is_control()) && !name.ends_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requirements_read_their_kind() {
        let json = r#"{"kind":"dotnet","version":"8.0","desktop":true,"name":".NET","url":"https://x"}"#;
        let requirement: Requirement = serde_json::from_str(json).unwrap();
        assert_eq!(requirement.check, Check::Dotnet { version: "8.0".into(), desktop: true });
        assert_eq!(serde_json::to_string(&requirement).unwrap(), json);
    }

    #[test]
    fn run_steps_read_their_defaults() {
        let step: RunStep = serde_json::from_str(r#"{"name":"VC++","file":"redist/vc.exe","when":"first_install"}"#).unwrap();
        assert_eq!(step.when, RunWhen::FirstInstall);
        assert_eq!(step.success_codes, [0]);
        assert!(!step.ignore_errors && step.arguments.is_none() && step.component.is_none() && !step.elevated);
        assert!(step.succeeded(0) && !step.succeeded(3010));
        assert_eq!(RunWhen::parse(" First-Install "), Some(RunWhen::FirstInstall));
        assert_eq!(RunWhen::parse("later"), None);
    }

    #[test]
    fn links_and_names() {
        assert!(is_https("https://example.com/feed.json"));
        assert!(!is_https("http://example.com"));
        assert!(!is_https("https://"));
        assert!(is_valid_shortcut_name("Hello Docs"));
        assert!(!is_valid_shortcut_name("a/b"));
        assert!(!is_valid_shortcut_name("  "));
    }
}
