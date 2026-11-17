//! `bach.toml`: what to install and how the installer looks and talks.
//!
//! [`load`] reads the file, checks every value and gathers the files to
//! package, so `bach check` and `bach build` fail early, with a sentence
//! naming the setting at fault, instead of producing a broken installer.
//! Paths in the file are relative to the file itself.

mod extras;
mod gather;
mod icon;
mod options;
mod secrets;
mod system;
mod text;
mod unsigned;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::archive::{Compression, Source};
use crate::manifest::{self, FileEntry, LogoKind, Manifest};
use crate::paths;

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
    files: FilesSection,
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
    updates: Option<extras::UpdatesSection>,
    access: Option<extras::AccessSection>,
    signing: Option<SigningSection>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AppSection {
    name: String,
    version: String,
    publisher: String,
    executable: String,
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
}

/// Settings that win over `bach.toml`, for builds in CI.
#[derive(Debug, Default, Clone)]
pub struct Overrides {
    /// Replaces `app.version`, so a release tag is the one place a version
    /// is written down. A leading `v` is dropped.
    pub version: Option<String>,
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
    let mut file: File = toml::from_str(text).map_err(|err| anyhow::anyhow!("{}", err.to_string().trim()))?;
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
    let app = check_app(&file.app)?;
    if crate::version::compare(&app.version, &app.version).is_none() {
        warnings.push(format!(
            "app.version is \"{}\", which isn't a version like 1.2.3 (dotted numbers, each up to 18446744073709551615, \
             an optional leading v, then -pre-release and +build). Installed copies can't tell whether a setup file is newer, \
             so they treat every one as an update and the downgrade protection does nothing",
            app.version
        ));
    }
    let install = options::install(&file.installer, &app)?;
    let languages = text::languages(file.installer.languages.as_deref())?;
    let mut locale = text::locale(&file.text, &languages, &mut warnings)?;
    locale.license = text::license(file.installer.license.as_ref(), root, &languages)?;
    if let Some(value) = &file.uninstall.data_description {
        let description = text::localized(value, "uninstall.data_description", &languages)?;
        locale.text.insert(crate::i18n::Key::delete_data_description.name().into(), description);
    }
    for folder in &file.uninstall.data {
        paths::check_data_folder(folder).context("uninstall.data")?;
        if !paths::looks_like_apps_own(folder, &[&app.name, &app.id, &app.publisher]) {
            warnings.push(format!(
                "uninstall.data \"{folder}\" has no folder that looks like the app's name, id or publisher. The uninstaller \
                 deletes it when the user ticks \"delete my data\", so make sure it holds only your app's data"
            ));
        }
    }

    let theme = match &file.installer.theme {
        None => manifest::Theme::System,
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
        Some(icon) => {
            let path = existing_file(root, icon, "app.icon")?;
            if !has_extension(&path, "ico") {
                bail!("app.icon must be a .ico file, so Windows can show it; {icon} isn't");
            }
            Some(path)
        }
    };
    let (logo_kind, logo, hide_logo) = logo(file.installer.logo.as_deref(), icon.as_deref(), root, &mut warnings)?;
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

    let output_template = file.installer.output.as_deref().unwrap_or(DEFAULT_OUTPUT);
    let mut output = root.join(fill_template(output_template, &app));
    if !has_extension(&output, "exe") {
        bail!("installer.output must end in .exe; it's \"{output_template}\"");
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

    let online_template = file.installer.online_output.as_deref().unwrap_or(DEFAULT_ONLINE_OUTPUT);
    let mut online_output = root.join(fill_template(online_template, &app));
    if !has_extension(&online_output, "exe") {
        bail!("installer.online_output must end in .exe; it's \"{online_template}\"");
    }
    if update_github.is_some() {
        if let Some(name) = online_output.file_name().map(|name| name.to_string_lossy().into_owned()) {
            online_output.set_file_name(github_asset_name(&name));
        }
    }
    if online_output == output {
        bail!("installer.output and installer.online_output are the same file; give the online installer its own name");
    }
    let online = file.installer.online.unwrap_or(false);
    if online && updates.is_none() {
        bail!(
            "installer.online is true, but there's no [updates]; the online installer downloads the newest version through the update feed"
        );
    }

    let source_dir = root.join(&file.files.source);
    if !source_dir.is_dir() {
        bail!("files.source is \"{}\", but {} isn't a folder", file.files.source, source_dir.display());
    }
    for pattern in &file.files.exclude {
        if pattern.trim().is_empty() {
            bail!("files.exclude has an empty pattern");
        }
    }
    for pattern in &file.files.allow_secrets {
        if pattern.trim().is_empty() {
            bail!("files.allow_secrets has an empty pattern");
        }
    }
    let sources = gather::gather(&source_dir, &file.files.exclude, &output)?;
    secrets::check(&sources, &file.files.allow_secrets, &mut warnings)?;
    long_paths(&sources, &install, &mut warnings);
    if file.signing.is_some() {
        warnings.extend(unsigned::warning(&sources));
    }
    let requirements = extras::requirements(&file.requirements)?;
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
    if let Some(clash) = sources.iter().find(|source| source.packaged.eq_ignore_ascii_case(manifest::UNINSTALLER)) {
        bail!("{} would replace the uninstaller Bach adds; rename it or add it to files.exclude", clash.path.display());
    }
    if !sources.iter().any(|source| source.packaged.eq_ignore_ascii_case(&app.executable)) {
        bail!("app.executable is \"{}\", but there's no such file in {} (or it's excluded)", app.executable, source_dir.display());
    }

    let mut files: Vec<FileEntry> = sources.iter().map(|source| FileEntry::new(&source.packaged, source.size)).collect();
    let components = options::components(&file.components, &mut files, &app, &languages)?;
    let system_changes = system::changes(
        &file.registry,
        &file.env,
        &file.context_menu,
        &file.migrate,
        &system::Owner { id: &app.id, name: &app.name, publisher: &app.publisher },
        &mut warnings,
    )?;
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
    let manifest = Manifest {
        format: crate::FORMAT_VERSION,
        builder: crate::VERSION.into(),
        app,
        install,
        uninstall: manifest::Uninstall { data: file.uninstall.data.clone(), data_description: None, feedback_url },
        look: manifest::Look { theme, accent, logo: logo_kind, hide_logo, logo_height: file.installer.logo_height, logo_aspect },
        files,
        components,
        protocols: options::protocols(&file.protocols)?,
        file_types: options::file_types(&file.file_types)?,
        locale,
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
    manifest.run = extras::run_steps(&file.run, &manifest.files, &manifest.components)?;
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

fn check_app(section: &AppSection) -> Result<manifest::App> {
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
    let executable = paths::normalize(section.executable.trim()).context("app.executable isn't a usable path")?;
    if !executable.to_ascii_lowercase().ends_with(".exe") {
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
