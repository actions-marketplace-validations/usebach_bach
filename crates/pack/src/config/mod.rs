//! `bach.toml`: what to install and how the installer looks and talks.
//!
//! [`load`] reads the file, checks every value and gathers the files to
//! package, so `bach check` and `bach build` fail early, with a sentence
//! naming the setting at fault, instead of producing a broken installer.
//! Paths in the file are relative to the file itself.

mod extras;
mod gather;
mod icon;
mod layout;
pub mod macos;
mod options;
mod profile;
mod secrets;
mod services;
mod style;
mod system;
mod text;
mod unsigned;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::archive::{Compression, Source};
use crate::manifest::{self, FileEntry, LogoKind, Manifest, Platform};
use crate::paths;

pub use macos::{Container as MacContainer, MacBuild, MacSigning};
pub use options::component_pack_name;
pub use profile::{list as list_profiles, valid_name as valid_profile_name, Kind as ProfileKind, Profile};
pub use system::pe_machine;

/// The default output, relative to `bach.toml`.
pub const DEFAULT_OUTPUT: &str = "dist/{name}-Setup-{version}.exe";

/// The default output of the online installer: no version in its name, so
/// one download link keeps pointing at the newest.
pub const DEFAULT_ONLINE_OUTPUT: &str = "dist/{name}-Web-Setup.exe";

/// The version of the `bach.toml` format this Bach reads. A file may say
/// `config_version = 1`; a format change that would break old files gets a new
/// number, and the error for an unknown one names it.
pub const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    /// The version of the config format this file is written for. Left out
    /// means the current one.
    config_version: Option<u32>,
    app: AppSection,
    /// The Windows files. Left out when the app only has a `[macos]` build.
    files: Option<FilesSection>,
    /// The macOS build (`[macos]`), packed into its own setup file.
    macos: Option<macos::MacosSection>,
    #[serde(default)]
    installer: InstallerSection,
    #[serde(default)]
    uninstall: UninstallSection,
    #[serde(default)]
    components: Vec<options::ComponentSection>,
    #[serde(default)]
    protocols: Vec<options::ProtocolSection>,
    #[serde(default)]
    file_types: Vec<options::FileTypeSection>,
    #[serde(default)]
    text: BTreeMap<String, toml::Value>,
    #[serde(default)]
    requirements: Vec<extras::RequirementSection>,
    #[serde(default)]
    shortcuts: Vec<extras::ShortcutSection>,
    #[serde(default)]
    run: Vec<extras::RunSection>,
    #[serde(default)]
    registry: Vec<system::RegistrySection>,
    #[serde(default)]
    env: Vec<system::EnvSection>,
    #[serde(default)]
    context_menu: Vec<system::ContextMenuSection>,
    #[serde(default)]
    migrate: Vec<system::MigrateSection>,
    #[serde(default)]
    services: Vec<services::ServiceSection>,
    #[serde(default)]
    firewall: Vec<services::FirewallSection>,
    updates: Option<extras::UpdatesSection>,
    access: Option<extras::AccessSection>,
    signing: Option<SigningSection>,
    style: Option<style::StyleSection>,
    layout: Option<layout::LayoutSection>,
    /// Other builds of the app (see `profile.rs`); checked there.
    #[serde(default)]
    profiles: BTreeMap<String, toml::Value>,
    /// The app's files for another processor architecture (`[arch.arm64]`).
    #[serde(default)]
    arch: BTreeMap<String, ArchSection>,
}

/// `[arch.arm64]`: the ARM64 build of the app, packed into its own setup
/// file next to the x64 one. One update feed (and one online installer) serves
/// both: a computer takes the file for its architecture.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchSection {
    /// The folder with the ARM64 files, instead of `files.source`.
    source: String,
    /// `files.exclude` for this folder; the main `files.exclude` when left out.
    exclude: Option<Vec<String>>,
    /// The ARM64 `bach-setup.exe` to build the ARM64 setup file on. Left out,
    /// `--runtime-arm64` or `BACH_RUNTIME_ARM64` names it.
    runtime: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AppSection {
    name: String,
    version: String,
    publisher: String,
    /// The Windows program; the macOS build names its own in `[macos]`.
    executable: Option<String>,
    id: Option<String>,
    description: Option<String>,
    website: Option<String>,
    icon: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FilesSection {
    source: String,
    #[serde(default)]
    exclude: Vec<String>,
    /// Patterns for files that look like secrets but are meant to ship.
    #[serde(default)]
    allow_secrets: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SigningSection {
    program: Option<String>,
    arguments: Option<String>,
}

/// How to sign the setup files: the program to run and its arguments. Signing
/// is optional; a project without `[signing]` builds unsigned files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signing {
    /// The signing program, `signtool` unless the project names another.
    pub program: String,
    /// Its arguments. `{file}` is the setup file (appended when not named) and
    /// `{env:NAME}` an environment variable, so secrets stay out of the file.
    pub arguments: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstallerSection {
    output: Option<String>,
    online: Option<bool>,
    online_output: Option<String>,
    folder: Option<String>,
    theme: Option<String>,
    style: Option<String>,
    accent: Option<String>,
    logo: Option<String>,
    compression: Option<String>,
    compression_threads: Option<u32>,
    compression_memory: Option<u32>,
    languages: Option<Vec<String>>,
    license: Option<toml::Value>,
    desktop_shortcut: Option<toml::Value>,
    start_menu_shortcut: Option<toml::Value>,
    start_with_windows: Option<toml::Value>,
    add_to_path: Option<toml::Value>,
    path_folder: Option<String>,
    change_folder: Option<bool>,
    open_button: Option<bool>,
    start_menu_folder: Option<String>,
    website_button: Option<bool>,
    logo_height: Option<u16>,
    scope: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct UninstallSection {
    #[serde(default)]
    data: Vec<String>,
    data_description: Option<toml::Value>,
    feedback_url: Option<String>,
}

/// A checked `bach.toml`, ready to build.
#[derive(Clone, Debug)]
pub struct Project {
    /// The folder `bach.toml` is in.
    pub root: PathBuf,
    pub manifest: Manifest,
    /// The files to package, in the manifest's order.
    pub sources: Vec<Source>,
    /// A `.ico` for the setup file's own icon.
    pub icon: Option<PathBuf>,
    /// The logo's bytes, read now so the build can't race an edit.
    pub logo: Option<Vec<u8>>,
    /// The fonts and picture `[style]` ships, in the order `look.files` lists them.
    pub look_files: Vec<Vec<u8>>,
    pub output: PathBuf,
    /// Whether `bach build` also writes the online installer (see
    /// `installer.online`).
    pub online: bool,
    /// Where the online installer goes; only written when [`Self::online`].
    pub online_output: PathBuf,
    pub compression: Compression,
    /// `installer.compression_threads`: how many threads to compress with
    /// (None: as many as the memory budget allows).
    pub compression_threads: Option<u32>,
    /// `installer.compression_memory`: the memory budget for compressing, in MiB.
    pub compression_memory_mib: u32,
    /// Whether the files are encrypted (`[access] encrypt`): the build then
    /// needs the key, from the environment variable [`Self::key_env`].
    pub encrypt: bool,
    /// The environment variable the build reads the access key from.
    pub key_env: String,
    /// Things that build fine but probably aren't meant, one sentence each.
    pub warnings: Vec<String>,
    /// `updates.download_url`: where the setup file will live, for the
    /// feed JSON `bach build` writes next to it. `{version}` is already the
    /// app's version; `{file}` is left for the builder to fill in.
    pub update_download_url: Option<String>,
    /// `updates.github`: the repository whose releases hold the updates.
    pub update_github: Option<String>,
    /// `[signing]`: how `bach build` signs the setup files, if it should.
    pub signing: Option<Signing>,
    /// The profile this project was read with (`[profiles.NAME]`), if any.
    pub profile: Option<String>,
    /// The architecture this project is the setup file for (`arm64`); None is the main (x64) one.
    pub arch: Option<String>,
    /// The architectures `[arch.NAME]` adds besides the main one, with the ARM64 runtime the config names.
    pub arches: Vec<ArchBuild>,
    /// What the build writes: a setup file, or (a `zip` profile) a ZIP of the files.
    pub kind: ProfileKind,
    /// The platform this project is the setup file for.
    pub platform: Platform,
    /// For a macOS project: the setup app's icon, identifier and signing.
    pub macos: Option<MacBuild>,
    /// Which setup files the config describes.
    pub declared: Declared,
}

/// An architecture `[arch.NAME]` adds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchBuild {
    pub name: String,
    /// `[arch.NAME] runtime`, relative to the config's folder.
    pub runtime: Option<PathBuf>,
}

impl Project {
    /// What goes in the setup file's look section: the logo, then the
    /// fonts and picture of `[style]`.
    pub fn look_section(&self) -> Vec<u8> {
        let mut section = self.logo.clone().unwrap_or_default();
        for file in &self.look_files {
            section.extend_from_slice(file);
        }
        section
    }
}

/// Settings that win over `bach.toml`, for builds in CI.
#[derive(Debug, Default, Clone)]
pub struct Overrides {
    /// Replaces `app.version`, so a release tag is the one place a version
    /// is written down. A leading `v` is dropped.
    pub version: Option<String>,
    /// Builds a profile of the config instead of the main file (see
    /// `[profiles.NAME]`).
    pub profile: Option<String>,
    /// Builds the setup file for this architecture (`arm64`) from `[arch.arm64]`
    /// instead of the main one.
    pub arch: Option<String>,
    /// Builds the setup file for this platform (`windows` or `macos`); Windows when None.
    pub platform: Option<String>,
}

/// Which setup files the config describes: `bach build` writes each one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Declared {
    pub windows: bool,
    pub macos: bool,
}

/// What `bach.toml` declares, read without checking the rest of it (so
/// `bach build` knows which setup files to make before it loads each one).
pub fn declared(text: &str) -> Result<Declared> {
    #[derive(Deserialize)]
    struct Probe {
        files: Option<toml::Value>,
        macos: Option<toml::Value>,
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let probe: Probe = toml::from_str(text).map_err(|err| anyhow::anyhow!("{}", err.to_string().trim()))?;
    Ok(Declared { windows: probe.files.is_some(), macos: probe.macos.is_some() })
}

/// Reads, checks and gathers a `bach.toml`.
pub fn load(path: &Path) -> Result<Project> {
    load_with(path, &Overrides::default())
}

/// [`load`] with some settings replaced (see [`Overrides`]).
pub fn load_with(path: &Path, overrides: &Overrides) -> Result<Project> {
    let text = crate::textfile::read(path).with_context(|| format!("couldn't read {}", path.display()))?;
    let root = path
        .parent()
        .map(|parent| if parent.as_os_str().is_empty() { Path::new(".") } else { parent })
        .unwrap_or(Path::new("."))
        .to_path_buf();
    parse_with(&text, &root, overrides).with_context(|| format!("{} isn't valid", path.display()))
}

/// [`load`] for text already in hand, with paths relative to `root`.
pub fn parse(text: &str, root: &Path) -> Result<Project> {
    parse_with(text, root, &Overrides::default())
}

/// [`parse`] with some settings replaced (see [`Overrides`]).
pub fn parse_with(text: &str, root: &Path, overrides: &Overrides) -> Result<Project> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let platform = macos::platform(overrides.platform.as_deref())?;
    if platform.is_macos() && (overrides.profile.is_some() || overrides.arch.is_some()) {
        bail!("the macOS setup file has no profiles or [arch.*] builds; leave --profile and --arch out when building for macOS");
    }
    let (mut file, profile) = match &overrides.profile {
        None => (toml::from_str::<File>(text).map_err(|err| anyhow::anyhow!("{}", err.to_string().trim()))?, None),
        Some(name) => {
            let applied = profile::apply(text, name)?;
            let file: File =
                applied.table.try_into().map_err(|err| anyhow::anyhow!("with the profile \"{name}\": {}", err.to_string().trim()))?;
            (file, Some((name.clone(), applied.kind, applied.output)))
        }
    };
    profile::check(&file.profiles)?;
    let kind = profile.as_ref().map_or(ProfileKind::Setup, |(_, kind, _)| *kind);
    for name in file.arch.keys() {
        if !manifest::extras::OTHER_ARCHES.contains(&name.as_str()) {
            bail!(
                "[arch.{name}] isn't an architecture Bach adds; the main build is x64, and {} can be added",
                manifest::extras::OTHER_ARCHES.join(", ")
            );
        }
    }
    let declared = Declared { windows: file.files.is_some(), macos: file.macos.is_some() };
    if !declared.windows && !declared.macos {
        bail!("bach.toml has neither [files] (the Windows files) nor [macos] (the macOS files), so there is nothing to build");
    }
    let mut files_cfg = file.files.take();
    let mut mac_cfg = file.macos.take();
    let mut mac_build = None;
    let mut mac_executable = None;
    let mut mac_modes: Vec<String> = Vec::new();
    let mut mac_agents: Vec<macos::AgentSection> = Vec::new();
    match platform {
        Platform::Windows => {
            if files_cfg.is_none() {
                bail!(
                    "bach.toml has no [files], so there is no Windows setup file to build; add [files], or build the macOS one with --platform macos"
                );
            }
            if file.app.executable.is_none() {
                bail!("app.executable is missing; name the program to start, like executable = \"hello.exe\"");
            }
        }
        Platform::Macos => {
            let Some(section) = mac_cfg.take() else {
                bail!("there is no [macos] in bach.toml, so there is no macOS setup file to build");
            };
            let app_id = file.app.id.as_deref().unwrap_or(&file.app.name).trim().to_string();
            mac_build = Some(macos::check(&section, root, &app_id)?);
            // The macOS build replaces what differs; the rest of the config is shared.
            let base = files_cfg.take();
            files_cfg = Some(FilesSection {
                source: section.source.clone(),
                exclude: section.exclude.clone().or_else(|| base.as_ref().map(|files| files.exclude.clone())).unwrap_or_default(),
                allow_secrets: base.map(|files| files.allow_secrets).unwrap_or_default(),
            });
            mac_executable = section.executable.clone().or_else(|| file.app.executable.clone());
            file.uninstall.data = section.data.clone();
            file.installer.output = section.output.clone();
            file.run = section.run;
            file.registry.clear();
            file.env.clear();
            file.context_menu.clear();
            file.migrate.clear();
            file.services.clear();
            file.firewall.clear();
            file.requirements.clear();
            file.shortcuts.clear();
            file.protocols.clear();
            file.file_types.clear();
            file.signing = None;
            file.installer.online = None;
            file.installer.online_output = None;
            mac_modes = section.executables;
            mac_agents = section.agents;
        }
    }
    let arch = overrides.arch.clone();
    if let Some(name) = &arch {
        let Some(section) = file.arch.get(name) else {
            bail!("there is no [arch.{name}] in bach.toml, so there is no {name} setup file to build");
        };
        if let Some(files) = &mut files_cfg {
            files.source = section.source.clone();
            if let Some(exclude) = &section.exclude {
                files.exclude = exclude.clone();
            }
        }
    }
    if let Some(version) = file.config_version.filter(|version| *version != CONFIG_VERSION) {
        bail!(
            "config_version is {version}, but this Bach reads version {CONFIG_VERSION}; {}",
            if version > CONFIG_VERSION { "update Bach to build this file" } else { "see the changelog for what changed in the format" }
        );
    }
    if let Some(version) = &overrides.version {
        let version = version.trim();
        let version = match version.strip_prefix(['v', 'V']) {
            Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest,
            _ => version,
        };
        file.app.version = version.to_string();
    }
    let mut warnings = Vec::new();
    let executable = match platform {
        Platform::Windows => file.app.executable.clone().unwrap_or_default(),
        Platform::Macos => mac_executable.clone().unwrap_or_default(),
    };
    if executable.trim().is_empty() {
        bail!(
            "{} is missing; name the program to start",
            if platform.is_macos() { "macos.executable (or app.executable)" } else { "app.executable" }
        );
    }
    let app = check_app(&file.app, &executable, platform)?;
    if crate::version::compare(&app.version, &app.version).is_none() {
        warnings.push(format!(
            "app.version is \"{}\", which isn't a version like 1.2.3 (dotted numbers, each up to 18446744073709551615, \
             an optional leading v, then -pre-release and +build). Installed copies can't tell whether a setup file is newer, \
             so they treat every one as an update and the downgrade protection does nothing",
            app.version
        ));
    }
    let bare_bundle =
        platform.is_macos() && files_cfg.as_ref().is_some_and(|files| macos::is_one_bundle(&root.join(&files.source), &app.executable));
    let install = options::install(&file.installer, &app, bare_bundle)?;
    let languages = text::languages(file.installer.languages.as_deref())?;
    let mut locale = text::locale(&file.text, &languages, &mut warnings)?;
    locale.license = text::license(file.installer.license.as_ref(), root, &languages)?;
    if let Some(value) = &file.uninstall.data_description {
        let description = text::localized(value, "uninstall.data_description", &languages)?;
        locale.text.insert(crate::i18n::Key::delete_data_description.name().into(), description);
    }
    for folder in &file.uninstall.data {
        let (setting, own) = match platform {
            Platform::Windows => {
                paths::check_data_folder(folder).context("uninstall.data")?;
                ("uninstall.data", paths::looks_like_apps_own(folder, &[&app.name, &app.id, &app.publisher]))
            }
            Platform::Macos => {
                paths::mac::check_data_folder(folder).context("macos.data")?;
                ("macos.data", paths::mac::looks_like_apps_own(folder, &[&app.name, &app.id, &app.publisher]))
            }
        };
        if !own {
            warnings.push(format!(
                "{setting} \"{folder}\" has no folder that looks like the app's name, id or publisher. The uninstaller \
                 deletes it when the user ticks \"delete my data\", so make sure it holds only your app's data"
            ));
        }
    }

    let mut styled = style::style(file.installer.style.as_deref(), file.style.as_ref(), file.installer.accent.as_deref(), root)?;
    let arranged = match &file.layout {
        None => None,
        Some(section) => {
            let (arranged, picture) = layout::layout(section, root)?;
            if let Some((kind, bytes)) = picture {
                styled.files.push(manifest::LookFile { kind, len: bytes.len() as u64 });
                styled.bytes.push(bytes);
            }
            Some(arranged).filter(|arranged| !arranged.is_empty())
        }
    };
    let theme = match &file.installer.theme {
        None => styled.theme.unwrap_or_default(),
        Some(value) => manifest::Theme::parse(value)
            .with_context(|| format!("installer.theme is \"{value}\"; use \"system\", \"light\" or \"dark\""))?,
    };
    let accent = match &file.installer.accent {
        None => None,
        Some(value) => Some(
            parse_hex(value)
                .map(|_| value.trim().to_ascii_lowercase())
                .with_context(|| format!("installer.accent is \"{value}\"; use a colour like \"#2563eb\""))?,
        ),
    };
    let compression = match &file.installer.compression {
        None => Compression::default(),
        Some(value) => Compression::parse(value)
            .with_context(|| format!("installer.compression is \"{value}\"; use \"best\", \"normal\" or \"fast\""))?,
    };
    if file.installer.compression_threads.is_some_and(|threads| !(1..=256).contains(&threads)) {
        bail!("installer.compression_threads is {}; use a number from 1 to 256", file.installer.compression_threads.unwrap_or(0));
    }
    let compression_memory_mib = file.installer.compression_memory.unwrap_or(crate::archive::DEFAULT_MEMORY_MIB);
    if !(64..=65_536).contains(&compression_memory_mib) {
        bail!("installer.compression_memory is {compression_memory_mib}; give the memory in MiB, from 64 to 65536");
    }

    let icon = match &file.app.icon {
        None => None,
        // The Windows icon isn't the macOS app's; `macos.icon` is the .icns.
        Some(_) if platform.is_macos() => None,
        Some(icon) => {
            let path = existing_file(root, icon, "app.icon")?;
            if !has_extension(&path, "ico") {
                bail!("app.icon must be a .ico file, so Windows can show it; {icon} isn't");
            }
            Some(path)
        }
    };
    let (logo_kind, logo, hide_logo) = logo(file.installer.logo.as_deref(), icon.as_deref(), root, &mut warnings)?;
    let look_size = styled.bytes.iter().chain(&logo).map(|bytes| bytes.len() as u64).sum::<u64>();
    if look_size > crate::container::MAX_LOOK {
        bail!(
            "the logo and the fonts and pictures in [style] and [layout] come to {} MB; keep them under {} MB together",
            look_size.div_ceil(1 << 20),
            crate::container::MAX_LOOK >> 20
        );
    }
    let logo_aspect = match (logo_kind, &logo) {
        (Some(LogoKind::Png), Some(png)) => extras::png_aspect(png).filter(|aspect| (aspect - 1.0).abs() > 0.02),
        _ => None,
    };
    if let Some(height) = file.installer.logo_height {
        if !(32..=96).contains(&height) {
            bail!("installer.logo_height is {height}; use 32 to 96 (points)");
        }
    }
    if file.app.website.is_none() && file.installer.website_button == Some(true) {
        bail!("installer.website_button is true, but app.website isn't set");
    }
    let update_settings = extras::updates(file.updates.as_ref(), install.scope.is_machine())?;
    if update_settings.is_some() && crate::version::compare(&app.version, &app.version).is_none() {
        bail!(
            "app.version is \"{}\"; an app that updates itself compares versions, so use numbers like 1.2.3 \
             (a suffix like -beta.1 is fine)",
            app.version
        );
    }
    let (updates, update_download_url, update_github) = match update_settings {
        Some(settings) => {
            (Some(settings.updates), settings.download_url.map(|url| url.replace("{version}", &app.version)), settings.github)
        }
        None => (None, None, None),
    };
    let feedback_url = extras::https(&file.uninstall.feedback_url, "uninstall.feedback_url")?;
    let access = extras::access(file.access.as_ref(), updates.is_some())?;

    let profile_name = profile.as_ref().map(|(name, _, _)| name.as_str());
    let (extension, default_output, default_online) = match (profile_name, kind) {
        (None, _) if platform.is_macos() => (
            mac_build.as_ref().map_or("dmg", |mac| mac.container.extension()),
            macos::DEFAULT_OUTPUT.to_string(),
            DEFAULT_ONLINE_OUTPUT.to_string(),
        ),
        (None, _) => ("exe", DEFAULT_OUTPUT.to_string(), DEFAULT_ONLINE_OUTPUT.to_string()),
        (Some(name), ProfileKind::Setup) => {
            ("exe", format!("dist/{{name}}-{name}-Setup-{{version}}.exe"), format!("dist/{{name}}-{name}-Web-Setup.exe"))
        }
        (Some(name), ProfileKind::Zip) => ("zip", format!("dist/{{name}}-{name}-{{version}}.zip"), DEFAULT_ONLINE_OUTPUT.to_string()),
    };
    let own_output = profile.as_ref().and_then(|(_, _, output)| output.as_deref());
    let output_template = own_output.or(file.installer.output.as_deref()).unwrap_or(&default_output);
    let mut output = root.join(fill_template(output_template, &app));
    if !has_extension(&output, extension) {
        let setting =
            if own_output.is_some() { format!("profiles.{}.output", profile_name.unwrap_or_default()) } else { "installer.output".into() };
        bail!("{setting} must end in .{extension}; it's \"{output_template}\"");
    }
    // GitHub renames an uploaded file that has spaces or odd characters in its
    // name, which would break the download link in the update feed. With
    // GitHub as the home of the updates, the setup file is named the way it
    // will be downloaded.
    if update_github.is_some() {
        if let Some(name) = output.file_name().map(|name| name.to_string_lossy().into_owned()) {
            output.set_file_name(github_asset_name(&name));
        }
    }

    let online_template = file.installer.online_output.as_deref().unwrap_or(&default_online);
    let mut online_output = root.join(fill_template(online_template, &app));
    if !platform.is_macos() && !has_extension(&online_output, "exe") {
        bail!("installer.online_output must end in .exe; it's \"{online_template}\"");
    }
    if update_github.is_some() {
        if let Some(name) = online_output.file_name().map(|name| name.to_string_lossy().into_owned()) {
            online_output.set_file_name(github_asset_name(&name));
        }
    }
    if online_output == output && !platform.is_macos() {
        bail!("installer.output and installer.online_output are the same file; give the online installer its own name");
    }
    // The ARM64 setup file is named like the x64 one, with the architecture before the extension:
    // Hello-Setup-1.0.0.exe, Hello-Setup-1.0.0-arm64.exe.
    if let Some(name) = &arch {
        if let (Some(stem), Some(extension)) = (
            output.file_stem().map(|stem| stem.to_string_lossy().into_owned()),
            output.extension().map(|ext| ext.to_string_lossy().into_owned()),
        ) {
            output.set_file_name(format!("{stem}-{name}.{extension}"));
        }
    }
    let online = file.installer.online.unwrap_or(false) && kind == ProfileKind::Setup && arch.is_none();
    if online && updates.is_none() {
        bail!(
            "installer.online is true, but there's no [updates]; the online installer downloads the newest version through the update feed"
        );
    }

    let Some(files_cfg) = files_cfg else {
        bail!("bach.toml has no [files]");
    };
    let source_dir = root.join(&files_cfg.source);
    if !source_dir.is_dir() {
        let setting = match (platform, &arch) {
            (Platform::Macos, _) => "macos.source".to_string(),
            (_, Some(name)) => format!("arch.{name}.source"),
            _ => "files.source".to_string(),
        };
        bail!("{setting} is \"{}\", but {} isn't a folder", files_cfg.source, source_dir.display());
    }
    // Every declared architecture is checked, so `bach check` finds a wrong folder before a build does.
    for (name, section) in &file.arch {
        let folder = root.join(&section.source);
        if !folder.is_dir() {
            bail!("arch.{name}.source is \"{}\", but {} isn't a folder", section.source, folder.display());
        }
        if section.runtime.as_deref().is_some_and(|runtime| runtime.trim().is_empty()) {
            bail!("arch.{name}.runtime is empty; name the ARM64 bach-setup.exe, or leave it out");
        }
    }
    for pattern in &files_cfg.exclude {
        if pattern.trim().is_empty() {
            bail!("files.exclude has an empty pattern");
        }
    }
    for pattern in &files_cfg.allow_secrets {
        if pattern.trim().is_empty() {
            bail!("files.allow_secrets has an empty pattern");
        }
    }
    let gathered = gather::gather_for(&source_dir, &files_cfg.exclude, &output, platform)?;
    let mut modes = gathered.modes;
    for pattern in &mac_modes {
        if pattern.trim().is_empty() {
            bail!("macos.executables has an empty pattern");
        }
        let before = modes.len();
        for source in gathered.sources.iter().filter(|source| paths::matches(pattern, &source.packaged)) {
            modes.entry(source.packaged.clone()).or_insert(0o755);
        }
        if modes.len() == before && !gathered.sources.iter().any(|source| paths::matches(pattern, &source.packaged)) {
            bail!("macos.executables has the pattern \"{pattern}\", which matches no file");
        }
    }
    let links = gathered.links;
    let sources = gathered.sources;
    secrets::check(&sources, &files_cfg.allow_secrets, &mut warnings)?;
    if platform.is_windows() {
        long_paths(&sources, &install, &mut warnings);
        if file.signing.is_some() {
            warnings.extend(unsigned::warning(&sources));
        }
    }
    let mut requirements = extras::requirements(&file.requirements)?;
    if let Some(version) = mac_build.as_ref().and_then(|mac| mac.minimum_version.clone()) {
        // The Setup app refuses to open on an older macOS (LSMinimumSystemVersion); this makes
        // the window say so in words, with a way forward, and a silent run exit with a reason.
        requirements.push(manifest::Requirement {
            check: manifest::Check::Macos { version: version.clone() },
            name: format!("macOS {version} or newer"),
            url: "https://www.apple.com/macos/".to_string(),
        });
    }
    for requirement in &requirements {
        if let manifest::extras::Check::Windows { build } = requirement.check {
            if build < manifest::extras::MIN_WINDOWS_BUILD {
                warnings.push(format!(
                    "requirements: Windows build {build} is older than the oldest Bach supports (Windows 10 version 1809, build {});                      the setup window may not run there, so the requirement is no help to those users",
                    manifest::extras::MIN_WINDOWS_BUILD
                ));
            }
        }
    }
    if sources.is_empty() {
        bail!("{} has no files to install", source_dir.display());
    }
    let uninstaller_name = match platform {
        Platform::Windows => manifest::UNINSTALLER.to_string(),
        Platform::Macos => format!("Uninstall {}.app", paths::file_name_part(&app.name, &app.id)),
    };
    if let Some(clash) = sources.iter().find(|source| {
        source.packaged.eq_ignore_ascii_case(&uninstaller_name)
            || source.packaged.to_lowercase().starts_with(&format!("{}/", uninstaller_name.to_lowercase()))
    }) {
        bail!("{} would replace the uninstaller Bach adds; rename it or add it to files.exclude", clash.path.display());
    }
    let setting = if platform.is_macos() { "macos.executable" } else { "app.executable" };
    let present = |path: &str| sources.iter().any(|source| source.packaged.eq_ignore_ascii_case(path));
    let in_bundle = |prefix: &str| {
        let prefix = format!("{}/", prefix.to_lowercase());
        sources.iter().any(|source| source.packaged.to_lowercase().starts_with(&prefix))
            || links.iter().any(|(path, _)| path.to_lowercase().starts_with(&prefix))
    };
    let executable_ok = match platform {
        Platform::Windows => present(&app.executable),
        // A bundle (`Hello.app`) is a folder of files; a plain program is a file.
        Platform::Macos => {
            if app.executable.to_lowercase().ends_with(".app") {
                in_bundle(&app.executable)
            } else {
                present(&app.executable)
            }
        }
    };
    if !executable_ok {
        bail!("{setting} is \"{}\", but there's no such file or app in {} (or it's excluded)", app.executable, source_dir.display());
    }

    let mut files: Vec<FileEntry> = sources
        .iter()
        .map(|source| FileEntry { mode: modes.get(&source.packaged).copied(), ..FileEntry::new(&source.packaged, source.size) })
        .collect();
    files.extend(links.iter().map(|(path, target)| FileEntry { link: Some(target.clone()), ..FileEntry::new(path, 0) }));
    let components = options::components(
        &file.components,
        &mut files,
        &app,
        &languages,
        &options::Downloads {
            update_link: update_download_url.as_deref(),
            flavour: &profile_name.into_iter().chain(arch.as_deref()).chain(platform.is_macos().then_some("macos")).collect::<Vec<&str>>(),
            encrypted: access.as_ref().is_some_and(|access| access.encrypt),
        },
    )?;
    let mut system_changes = system::changes(
        &file.registry,
        &file.env,
        &file.context_menu,
        &file.migrate,
        &system::Owner { id: &app.id, name: &app.name, publisher: &app.publisher },
        &mut warnings,
    )?;
    services::add(&mut system_changes, &file.services, &file.firewall, &files, &app.executable, install.scope.is_machine())?;
    // A 32-bit program reads HKLM\Software\Vendor from WOW6432Node.
    if install.scope.is_machine() && system::registry_leaves_view_open(&file.registry) {
        let program = sources.iter().find(|source| source.packaged.eq_ignore_ascii_case(&app.executable));
        if program.and_then(|source| system::pe_machine(&source.path)) == Some(0x014C) {
            warnings.push(
                "app.executable is a 32-bit program and installer.scope is \"machine\": it reads HKLM\\Software\\... from the 32-bit \
                 view (WOW6432Node), so give the [[registry]] values it reads view = \"32\" (or \"both\")"
                    .to_string(),
            );
        }
    }
    let agent_list: Vec<manifest::MacAgent> = mac_agents
        .iter()
        .map(|agent| manifest::MacAgent {
            name: agent.name.trim().to_string(),
            file: agent.file.clone().map(|file| file.trim().to_string()).unwrap_or_else(|| app.executable.clone()),
            arguments: optional(&agent.arguments),
            run_at_load: agent.run_at_load.unwrap_or(true),
            keep_alive: agent.keep_alive.unwrap_or(false),
            interval: agent.interval,
        })
        .collect();
    let manifest = Manifest {
        format: crate::FORMAT_VERSION,
        builder: crate::VERSION.into(),
        app,
        install,
        uninstall: manifest::Uninstall { data: file.uninstall.data.clone(), data_description: None, feedback_url },
        look: manifest::Look {
            theme,
            accent,
            logo: logo_kind,
            hide_logo,
            logo_height: file.installer.logo_height,
            logo_aspect,
            style: styled.name,
            custom: styled.custom,
            layout: arranged,
            files: styled.files,
        },
        files,
        components,
        protocols: options::protocols(&file.protocols)?,
        file_types: options::file_types(&file.file_types)?,
        locale,
        platform,
        agents: agent_list,
        mac: mac_build
            .as_ref()
            .map(|mac| manifest::MacInfo { bundle_id: mac.bundle_id.clone(), minimum_version: mac.minimum_version.clone() }),
        chosen: None,
        requirements,
        shortcuts: Vec::new(),
        run: Vec::new(),
        system: system_changes,
        updates,
        access: access.as_ref().map(|access| access.manifest.clone()),
        online: false,
        archive_sha256: None,
        patch_from: None,
    };
    if manifest.install.add_to_path != manifest::Choice::Never {
        if let Some(folder) = &manifest.install.path_folder {
            let inside = manifest.files.iter().any(|entry| entry.path.to_lowercase().starts_with(&format!("{}/", folder.to_lowercase())));
            if !inside {
                warnings.push(format!("installer.path_folder is \"{folder}\", but no installed file is in it"));
            }
        }
    }
    let mut manifest = manifest;
    manifest.shortcuts = extras::shortcuts(&file.shortcuts, &manifest.files)?;
    manifest.run = extras::run_steps(&file.run, &manifest.files, &manifest.components, platform)?;
    warnings.extend(file_list_warning(&manifest.files));
    if manifest.install.scope == manifest::Scope::User {
        for step in manifest.run.iter().filter(|step| step.elevated) {
            warnings.push(format!(
                "run step \"{}\" is elevated, but this is a per-user install: its program starts with administrator rights from \
                 a folder the user's own programs can write to. Setup checks the file against its SHA-256 right before it asks \
                 for rights, but keep elevated steps to well-known installers (and sign them), or use installer.scope = \"machine\"",
                step.name
            ));
        }
    }
    manifest.validate()?;
    Ok(Project {
        root: root.to_path_buf(),
        manifest,
        sources,
        icon,
        logo,
        look_files: styled.bytes,
        output,
        online,
        online_output,
        compression,
        compression_threads: file.installer.compression_threads,
        compression_memory_mib,
        encrypt: access.as_ref().is_some_and(|access| access.encrypt),
        key_env: access.as_ref().map_or_else(|| extras::DEFAULT_KEY_ENV.to_string(), |access| access.key_env.clone()),
        warnings,
        update_download_url,
        update_github,
        signing: signing(file.signing.as_ref())?,
        profile: profile.map(|(name, _, _)| name),
        arch,
        arches: file
            .arch
            .iter()
            .map(|(name, section)| ArchBuild {
                name: name.clone(),
                runtime: section.runtime.as_deref().map(|runtime| root.join(runtime.trim())),
            })
            .collect(),
        kind,
        platform,
        macos: mac_build,
        declared,
    })
}

fn signing(section: Option<&SigningSection>) -> Result<Option<Signing>> {
    let Some(section) = section else {
        return Ok(None);
    };
    let program = match section.program.as_deref().map(str::trim) {
        None | Some("") => "signtool".to_string(),
        Some(program) if program.chars().any(char::is_control) => bail!("signing.program has control characters in it"),
        Some(program) => program.to_string(),
    };
    let arguments = section.arguments.as_deref().map(str::trim).unwrap_or_default();
    if arguments.is_empty() {
        bail!(
            "[signing] needs arguments: how the signing program picks your certificate, like              arguments = '/fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /a'"
        );
    }
    if arguments.contains('\0') {
        bail!("signing.arguments has a NUL in it");
    }
    // signtool's first word is the command to run; without `sign` it answers
    // "Invalid command", which says nothing about where the mistake is.
    let is_signtool = program.eq_ignore_ascii_case("signtool") || program.eq_ignore_ascii_case("signtool.exe");
    if is_signtool && !arguments.split_whitespace().next().is_some_and(|first| first.eq_ignore_ascii_case("sign")) {
        bail!(
            "signing.arguments for signtool must start with the word sign, like \
             arguments = 'sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /a'"
        );
    }
    Ok(Some(Signing { program, arguments: arguments.to_string() }))
}

/// A file name as GitHub keeps it for a release download: letters, digits,
/// `.`, `-` and `_`; anything else becomes `-`.
/// How much the list of files may weigh before `bach check` says so. The list (path, size and
/// SHA-256 of every file) travels in the setup file and again in `uninstall.exe` on every
/// computer, and is read every time setup starts.
const FILE_LIST_WARN_BYTES: usize = 2 << 20;

/// A warning when the file list is big, with what to do about it. Each entry takes about 110
/// bytes besides its path (the keys, the size and the 64 hex digits of its SHA-256).
fn file_list_warning(files: &[FileEntry]) -> Option<String> {
    let bytes: usize = files.iter().map(|file| file.path.len() + 110).sum();
    (bytes > FILE_LIST_WARN_BYTES).then(|| {
        format!(
            "files.source holds {} files, which makes a file list of about {:.1} MB. It is stored in the setup file and \
             again in uninstall.exe on every computer. Fewer, larger files keep both smaller (for an Electron app, pack the \
             app into an asar archive; leave out test, docs and source map files with files.exclude)",
            files.len(),
            bytes as f64 / (1 << 20) as f64
        )
    })
}

/// What Windows' classic limit allows for a whole path: 260 characters with
/// the drive and the ending NUL.
const MAX_PATH_CHARS: usize = 259;

/// Warns when the longest packaged file would pass [`MAX_PATH_CHARS`] below the
/// usual install folder. Setup writes long paths fine, but shortcuts, Explorer and
/// many programs stop at 260 characters unless Windows' long path support is on.
fn long_paths(sources: &[Source], install: &manifest::Install, warnings: &mut Vec<String>) {
    // `C:\Users\` + a 20 character user name + `\AppData\Local\Programs\`, or `C:\Program Files\`.
    let base = match install.scope {
        manifest::Scope::User => 9 + 20 + 24,
        manifest::Scope::Machine => 19,
    };
    let folder = install.folder.encode_utf16().count();
    let longest = sources
        .iter()
        .map(|source| (source.packaged.replace('/', "\\").encode_utf16().count(), source.packaged.as_str()))
        .max_by_key(|(length, _)| *length);
    if let Some((length, name)) = longest {
        let total = base + folder + 1 + length;
        if total > MAX_PATH_CHARS {
            warnings.push(format!(
                "\"{name}\" would be {total} characters long once installed in the usual folder, \
                 more than the {MAX_PATH_CHARS} that shortcuts, Explorer and many programs handle unless Windows' long path \
                 support is on. Shorten the path (for example by flattening node_modules or shortening folder names)"
            ));
        }
    }
}

pub fn github_asset_name(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '-' }).collect()
}

fn check_app(section: &AppSection, executable: &str, platform: Platform) -> Result<manifest::App> {
    let name = required(&section.name, "app.name")?;
    let version = required(&section.version, "app.version")?;
    let publisher = required(&section.publisher, "app.publisher")?;
    let id = match &section.id {
        Some(id) => id.trim().to_string(),
        None => name.clone(),
    };
    if let Some(problem) = paths::id_problem(&id) {
        let hint = if section.id.is_some() { "app.id" } else { "app.name (or set app.id)" };
        // A long value would fill the screen; the length is in the reason.
        let shown: String = if id.chars().count() > 40 { id.chars().take(37).chain("...".chars()).collect() } else { id.clone() };
        bail!("{hint} is \"{shown}\", which {problem}. It names a folder and a registry key");
    }
    let setting = if platform.is_macos() { "macos.executable" } else { "app.executable" };
    let executable = paths::normalize(executable.trim()).with_context(|| format!("{setting} isn't a usable path"))?;
    if platform.is_windows() && !executable.to_ascii_lowercase().ends_with(".exe") {
        bail!("app.executable is \"{executable}\"; it must be an .exe");
    }
    if let Some(website) = &section.website {
        if !(website.starts_with("https://") || website.starts_with("http://")) {
            bail!("app.website is \"{website}\"; it must start with https://");
        }
    }
    Ok(manifest::App {
        id,
        name,
        version,
        publisher,
        description: optional(&section.description),
        website: optional(&section.website),
        executable,
    })
}

/// The logo: a file, `"none"`, or the app's icon when it has a PNG image.
fn logo(
    logo: Option<&str>,
    icon: Option<&Path>,
    root: &Path,
    warnings: &mut Vec<String>,
) -> Result<(Option<LogoKind>, Option<Vec<u8>>, bool)> {
    match logo.map(str::trim) {
        Some(none) if none.eq_ignore_ascii_case("none") => Ok((None, None, true)),
        Some(logo) => {
            let path = existing_file(root, logo, "installer.logo")?;
            let kind = if has_extension(&path, "svg") {
                LogoKind::Svg
            } else if has_extension(&path, "png") {
                LogoKind::Png
            } else {
                bail!("installer.logo must be an .svg or a .png file (or \"none\"); {logo} is neither");
            };
            let bytes = std::fs::read(&path).with_context(|| format!("couldn't read {}", path.display()))?;
            if kind == LogoKind::Png && !bytes.starts_with(icon::PNG_SIGNATURE) {
                bail!("installer.logo is \"{logo}\", but it isn't a PNG image inside");
            }
            Ok((Some(kind), Some(bytes), false))
        }
        None => {
            let Some(icon) = icon else {
                return Ok((None, None, false));
            };
            let bytes = std::fs::read(icon).with_context(|| format!("couldn't read {}", icon.display()))?;
            match icon::largest_png(&bytes) {
                Some(png) => Ok((Some(LogoKind::Png), Some(png), false)),
                None => {
                    warnings.push(
                        "app.icon has no PNG-compressed image, so the window shows the Bach mark; \
                         set installer.logo to show your own"
                            .into(),
                    );
                    Ok((None, None, false))
                }
            }
        }
    }
}

fn required(value: &str, key: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{key} is empty");
    }
    Ok(value.to_string())
}

fn optional(value: &Option<String>) -> Option<String> {
    value.as_ref().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn existing_file(root: &Path, relative: &str, key: &str) -> Result<PathBuf> {
    let path = root.join(relative);
    if !path.is_file() {
        bail!("{key} is \"{relative}\", but {} isn't a file", path.display());
    }
    Ok(path)
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
}

/// `{name}`, `{version}` and `{id}` in the output path.
pub fn fill_template(template: &str, app: &manifest::App) -> String {
    let clean = |value: &str| value.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) { '-' } else { c }).collect::<String>();
    template.replace("{name}", &clean(&app.name)).replace("{version}", &clean(&app.version)).replace("{id}", &clean(&app.id))
}

/// `#rrggbb` (or `rrggbb`) as a number.
pub fn parse_hex(value: &str) -> Option<u32> {
    let hex = value.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

#[cfg(test)]
mod tests;
