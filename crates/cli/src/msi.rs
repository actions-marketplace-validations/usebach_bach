//! `bach build --msi`: the same app as a Windows Installer package, for
//! companies that deploy with Group Policy, Configuration Manager or Intune.
//!
//! The MSI is made from `bach.toml` like the setup file, not from it: the files go
//! into a cabinet inside the package, and the package's tables say where they go.
//! What it covers, and keeps to Windows Installer's own rules:
//!
//! - **Files and folders**, every file its own component, with the component's
//!   id stable across versions (it comes from the app id and the file's path).
//! - **Shortcuts** in the Start menu and on the desktop, and `[[shortcuts]]`.
//! - **Registry values** from `[[registry]]`.
//! - **Optional components** as features: ticked ones installed by default, the others with
//!   `ADDLOCAL=id`.
//! - **Upgrades**: a package of a higher version replaces the older one (a major
//!   upgrade, with `UpgradeCode` taken from the app id), and an older one refuses to replace
//!   a newer one.
//! - **Scope**: `installer.scope = "machine"` gives a per-machine package (Program Files,
//!   all users), the default `"user"` a per-user one (`%LOCALAPPDATA%\Programs`, no
//!   administrator rights), exactly as the setup file installs.
//!
//! What it doesn't carry (and `bach build` says so for the ones the config uses):
//! run steps, requirements, `[[migrate]]`, environment variables, right-click entries, link
//! schemes, file types, start with Windows, PATH, the update settings and the user's data
//! folders. An MSI is updated by installing a newer MSI.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Seek, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use bach_pack::config::Project;
use bach_pack::i18n::Key;
use bach_pack::manifest::{Check, Choice, Manifest, RegistryData, RunWhen};
use cab::{CabinetBuilder, CompressionType};
use msi::{Category, Column, Insert, Package, PackageType, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// The largest cabinet Windows Installer reads (2 GiB, a 32-bit offset), with room for the headers.
const MAX_CABINET: u64 = (2 << 30) - (8 << 20);
/// `File.Sequence` is a 16-bit column.
const MAX_FILES: usize = 32_000;

/// What the package holds, worked out before anything is written.
#[derive(Debug)]
pub struct Plan {
    pub product_code: String,
    pub upgrade_code: String,
    pub package_code: String,
    pub version: String,
    pub per_machine: bool,
    pub files: Vec<PlannedFile>,
    /// Folders below the install folder: (id, parent id, name).
    pub folders: Vec<(String, String, String)>,
    pub features: Vec<PlannedFeature>,
    pub shortcuts: Vec<PlannedShortcut>,
    pub registry: Vec<PlannedRegistry>,
    /// Components that hold no file: shortcuts, registry values, variables, run steps' neighbours.
    pub extra_components: Vec<PlannedComponent>,
    pub environment: Vec<PlannedEnv>,
    pub actions: Vec<PlannedAction>,
    pub launch: Vec<(String, String)>,
    /// A Start menu folder of its own (`installer.start_menu_folder`), by its name.
    pub start_menu_folder: Option<String>,
    /// What the config asks for that an MSI doesn't carry.
    pub skipped: Vec<String>,
}

#[derive(Debug)]
pub struct PlannedFile {
    /// Its key in the File table, and its name inside the cabinet.
    pub key: String,
    pub component: String,
    pub component_id: String,
    pub feature: String,
    pub folder: String,
    pub name: String,
    pub short: String,
    pub size: u64,
    pub source: PathBuf,
}

#[derive(Debug)]
pub struct PlannedFeature {
    pub id: String,
    pub parent: Option<String>,
    pub title: String,
    pub level: i32,
}

#[derive(Debug)]
pub struct PlannedShortcut {
    pub id: String,
    /// `ProgramMenuFolder` or `DesktopFolder`.
    pub folder: String,
    pub name: String,
    pub file_key: String,
    pub component: String,
    pub arguments: Option<String>,
}

#[derive(Debug)]
pub struct PlannedRegistry {
    pub id: String,
    pub key: String,
    pub name: Option<String>,
    pub value: String,
    pub component: String,
}

/// A component with no file. Its key path is a registry value.
#[derive(Debug)]
pub struct PlannedComponent {
    pub id: String,
    pub feature: String,
    /// The registry row that says the component is installed.
    pub key_path: String,
}

#[derive(Debug)]
pub struct PlannedEnv {
    pub id: String,
    /// With the flags Windows Installer reads in front: `=-` sets it and takes it away on removal.
    pub name: String,
    pub value: String,
    pub component: String,
}

/// A custom action and when it runs.
#[derive(Debug)]
pub struct PlannedAction {
    pub action: String,
    pub kind: i32,
    pub source: String,
    pub target: Option<String>,
    pub sequence: i32,
    pub condition: Option<String>,
}

/// Text for a column Windows Installer formats: `[` and `]` are escaped, so the user's text can't
/// name a property, and `{app}` becomes the install folder (which ends in a backslash here).
fn formatted(text: &str) -> String {
    text.split("{app}").map(|part| part.replace('[', "[\\[]").replace(']', "[\\]]")).collect::<Vec<_>>().join("[INSTALLFOLDER]")
}

/// A registry value that is a string: one that starts with `#` is a number to Windows Installer, so it is doubled.
fn string_value(text: &str) -> String {
    let value = formatted(text);
    if value.starts_with('#') {
        format!("#{value}")
    } else {
        value
    }
}

/// A GUID made from names, so the same input gives the same id: a component keeps its id
/// from one version to the next, which is what lets an upgrade replace files in place.
fn guid(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // A version 5 style, variant 1 GUID.
    bytes[6] = (bytes[6] & 0x0F) | 0x50;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    format!("{{{}}}", Uuid::from_bytes(bytes).hyphenated().to_string().to_ascii_uppercase())
}

/// Windows Installer's `major.minor.build`: three numbers, 255, 255 and 65535 at most.
pub fn product_version(version: &str) -> Result<String> {
    let text = version.trim().trim_start_matches(['v', 'V']);
    let numbers: Vec<&str> = text.split('.').collect();
    let usable = !numbers.is_empty()
        && numbers.len() <= 3
        && numbers.iter().all(|part| !part.is_empty() && part.len() <= 5 && part.chars().all(|c| c.is_ascii_digit()));
    let parsed: Vec<u32> = if usable { numbers.iter().filter_map(|part| part.parse().ok()).collect() } else { Vec::new() };
    let (major, minor, build) =
        (parsed.first().copied().unwrap_or(u32::MAX), parsed.get(1).copied().unwrap_or(0), parsed.get(2).copied().unwrap_or(0));
    if parsed.is_empty() || major > 255 || minor > 255 || build > 65_535 {
        bail!(
            "app.version is \"{version}\", but a Windows Installer package's version is three numbers, like 1.4.2, \
             the first two up to 255 and the last up to 65535 (a suffix like -beta isn't allowed)"
        );
    }
    Ok(format!("{major}.{minor}.{build}"))
}

/// `F0000012.EXE|long name.exe`: Windows Installer's file name column holds a short name and the real one.
fn short_name(index: usize, name: &str) -> String {
    let extension: String = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.chars().filter(char::is_ascii_alphanumeric).take(3).collect::<String>().to_ascii_uppercase())
        .unwrap_or_default();
    if extension.is_empty() {
        format!("F{index:07}")
    } else {
        format!("F{index:07}.{extension}")
    }
}

/// The feature of a component is named like it, so `ADDLOCAL=samples` works. A feature's name
/// can't hold a dash, which a component's id may.
fn feature_id(component: &str) -> String {
    component.replace('-', "_")
}

/// One part of a registry key: the characters of a name that can't break the path.
fn registry_part(name: &str) -> String {
    let part: String = name.chars().map(|c| if matches!(c, '\\' | '/' | '[' | ']') || c.is_control() { ' ' } else { c }).collect();
    let part = part.trim();
    if part.is_empty() {
        "App".into()
    } else {
        part.to_string()
    }
}

fn is_user_scope(manifest: &Manifest) -> bool {
    !manifest.install.scope.is_machine()
}

/// Works out the package.
pub fn plan(project: &Project) -> Result<Plan> {
    let manifest = &project.manifest;
    let app = &manifest.app;
    if manifest.access.as_ref().is_some_and(|access| access.encryption.is_some()) || project.encrypt {
        bail!("an app locked with [access] can't be an MSI: the files in the package would be readable by anybody");
    }
    if !manifest.system.services.is_empty() || !manifest.system.firewall.is_empty() {
        bail!(
            "an MSI doesn't carry [[services]] or [[firewall]] yet: Bach's own setup file installs them, so ship that for an app with a service"
        );
    }
    let version = product_version(&app.version)?;
    if manifest.files.len() > MAX_FILES {
        bail!("an MSI holds at most {MAX_FILES} files here; this app has {}", manifest.files.len());
    }
    let total: u64 = manifest.files.iter().map(|file| file.size).sum();
    if total > MAX_CABINET {
        bail!("an MSI holds at most 2 GB of files (a cabinet's limit); this app has {}", crate::output::size(total));
    }
    let per_machine = !is_user_scope(manifest);
    let sources: BTreeMap<&str, &Path> = project.sources.iter().map(|source| (source.packaged.as_str(), source.path.as_path())).collect();

    // Folders below the install folder, one row for each distinct path.
    let mut folder_ids: BTreeMap<String, String> = BTreeMap::new();
    let mut folders: Vec<(String, String, String)> = Vec::new();
    let mut folder_of = |path: &str, folders: &mut Vec<(String, String, String)>| -> String {
        let Some((dir, _)) = path.rsplit_once('/') else {
            return "INSTALLFOLDER".to_string();
        };
        let mut parent = "INSTALLFOLDER".to_string();
        let mut so_far = String::new();
        for part in dir.split('/') {
            if !so_far.is_empty() {
                so_far.push('/');
            }
            so_far.push_str(part);
            let next_id = format!("D{}", folder_ids.len() + 1);
            let id = folder_ids.entry(so_far.clone()).or_insert_with(|| {
                folders.push((next_id.clone(), parent.clone(), part.to_string()));
                next_id.clone()
            });
            parent = id.clone();
        }
        parent
    };

    let mut files = Vec::new();
    for (index, entry) in manifest.files.iter().enumerate() {
        let Some(source) = sources.get(entry.path.as_str()) else {
            bail!("{} is in the file list but not among the files to package", entry.path);
        };
        let name = entry.path.rsplit('/').next().unwrap_or(&entry.path).to_string();
        let feature = match &entry.component {
            Some(id) => feature_id(id),
            None => "Main".to_string(),
        };
        files.push(PlannedFile {
            key: format!("f{}", index + 1),
            component: format!("c{}", index + 1),
            component_id: guid(&[&app.id, "component", &entry.path.to_ascii_lowercase()]),
            feature,
            folder: folder_of(&entry.path, &mut folders),
            short: short_name(index + 1, &name),
            name,
            size: entry.size,
            source: source.to_path_buf(),
        });
    }

    let mut features = vec![PlannedFeature { id: "Main".into(), parent: None, title: app.name.clone(), level: 1 }];
    for component in &manifest.components {
        let title = bach_pack::i18n::resolve(&component.title, "en", &manifest.locale.languages).unwrap_or(&component.id).to_string();
        // Level 1 is installed by default, 4 only when asked for (ADDLOCAL=id), like an unticked box.
        features.push(PlannedFeature {
            id: feature_id(&component.id),
            parent: Some("Main".into()),
            title,
            level: if component.default { 1 } else { 4 },
        });
    }

    let publisher_key = format!("Software\\{}\\{}", registry_part(&app.publisher), registry_part(&app.name));
    let mut registry: Vec<PlannedRegistry> = Vec::new();
    let mut extra_components: Vec<PlannedComponent> = Vec::new();
    let english = bach_pack::i18n::language("en").map(|language| language.strings);
    let text_of = |key: Key| english.map(|strings| strings.get(key).to_string()).unwrap_or_default();

    // The options of the install page are features, named like the setup file's own `--with` ids:
    // `ADDLOCAL=desktop_shortcut` is `--with desktop-shortcut`.
    let option = |choice: Choice, id: &str, title: Key, features: &mut Vec<PlannedFeature>| -> Option<String> {
        match choice {
            Choice::Never => None,
            Choice::Always => Some("Main".to_string()),
            Choice::On | Choice::Off => {
                features.push(PlannedFeature {
                    id: id.to_string(),
                    parent: Some("Main".into()),
                    title: text_of(title),
                    level: if choice == Choice::On { 1 } else { 4 },
                });
                Some(id.to_string())
            }
        }
    };
    let desktop_feature = option(manifest.install.desktop_shortcut, "desktop_shortcut", Key::desktop_shortcut, &mut features);
    let menu_feature = option(manifest.install.start_menu_shortcut, "start_menu", Key::start_menu, &mut features);
    let autostart_feature = option(manifest.install.start_with_windows, "autostart", Key::start_with_windows, &mut features);
    let path_feature = option(manifest.install.add_to_path, "path", Key::add_to_path, &mut features);

    // A component that holds no file says it is installed with a registry value, as Windows Installer wants
    // for anything it can't point at a file for.
    let mut add_component = |id: &str, feature: &str, registry: &mut Vec<PlannedRegistry>, key_path: Option<String>| {
        let key_path = key_path.unwrap_or_else(|| {
            let row = format!("k_{id}");
            registry.push(PlannedRegistry {
                id: row.clone(),
                key: publisher_key.clone(),
                name: Some(id.to_string()),
                value: "#1".into(),
                component: id.to_string(),
            });
            row
        });
        extra_components.push(PlannedComponent { id: id.to_string(), feature: feature.to_string(), key_path });
    };

    // Shortcuts, each set in a component of its own so the option that offers it can leave it out.
    let program = manifest.files.iter().position(|file| file.path.eq_ignore_ascii_case(&app.executable));
    let start_menu_folder = manifest.install.start_menu_folder.clone().filter(|name| !name.trim().is_empty());
    let menu_folder_id = if start_menu_folder.is_some() { "StartMenuDir" } else { "ProgramMenuFolder" };
    let mut shortcuts = Vec::new();
    if let Some(feature) = &menu_feature {
        add_component("cStart", feature, &mut registry, None);
        if let Some(program) = program {
            shortcuts.push(PlannedShortcut {
                id: "StartMenuShortcut".into(),
                folder: menu_folder_id.into(),
                name: app.name.clone(),
                file_key: files[program].key.clone(),
                component: "cStart".into(),
                arguments: None,
            });
        }
        for (index, extra) in manifest.shortcuts.iter().enumerate() {
            if let Some(target) = manifest.files.iter().position(|file| file.path.eq_ignore_ascii_case(&extra.target)) {
                shortcuts.push(PlannedShortcut {
                    id: format!("ExtraShortcut{}", index + 1),
                    folder: menu_folder_id.into(),
                    name: extra.name.clone(),
                    file_key: files[target].key.clone(),
                    component: "cStart".into(),
                    arguments: extra.arguments.clone(),
                });
            }
        }
    }
    if let (Some(feature), Some(program)) = (&desktop_feature, program) {
        add_component("cDesktop", feature, &mut registry, None);
        shortcuts.push(PlannedShortcut {
            id: "DesktopShortcut".into(),
            folder: "DesktopFolder".into(),
            name: app.name.clone(),
            file_key: files[program].key.clone(),
            component: "cDesktop".into(),
            arguments: None,
        });
    }

    // [[registry]] values.
    let first_user_row = registry.len();
    for (index, value) in manifest.system.registry.iter().enumerate() {
        registry.push(PlannedRegistry {
            id: format!("r{}", index + 1),
            key: formatted(&value.key),
            name: Some(formatted(&value.name)).filter(|name| !name.is_empty()),
            value: match &value.data {
                RegistryData::String(text) => string_value(text),
                RegistryData::Expand(text) => format!("#%{}", formatted(text)),
                RegistryData::Dword(number) => format!("#{number}"),
            },
            component: "cRegistry".into(),
        });
    }
    if let Some(first) = registry.get(first_user_row) {
        let key_path = first.id.clone();
        add_component("cRegistry", "Main", &mut registry, Some(key_path));
    }

    // Link schemes, file types and right-click entries are registry values too, under Classes.
    let program_key = program.map(|index| files[index].key.clone());
    if let Some(exe) = &program_key {
        let command = |arguments: &str| format!("\"[#{exe}]\" {arguments}");
        let first_integration = registry.len();
        let row = |registry: &mut Vec<PlannedRegistry>, key: String, name: Option<&str>, value: String| {
            let id = format!("i{}", registry.len() + 1);
            registry.push(PlannedRegistry { id, key, name: name.map(str::to_string), value, component: "cIntegration".into() });
        };
        for protocol in &manifest.protocols {
            let base = format!("Software\\Classes\\{}", protocol.scheme);
            let description = protocol.description.clone().unwrap_or_else(|| format!("URL:{}", app.name));
            row(&mut registry, base.clone(), None, formatted(&description));
            row(&mut registry, base.clone(), Some("URL Protocol"), String::new());
            row(&mut registry, format!("{base}\\DefaultIcon"), None, format!("[#{exe}],0"));
            row(&mut registry, format!("{base}\\shell\\open\\command"), None, command("\"%1\""));
        }
        for file_type in &manifest.file_types {
            let prog_id = file_type.prog_id(&app.id);
            let base = format!("Software\\Classes\\{prog_id}");
            let description = file_type.description.clone().unwrap_or_else(|| format!("{} file", app.name));
            row(&mut registry, base.clone(), None, formatted(&description));
            row(&mut registry, format!("{base}\\DefaultIcon"), None, format!("[#{exe}],0"));
            row(&mut registry, format!("{base}\\shell\\open\\command"), None, command("\"%1\""));
            row(&mut registry, format!("Software\\Classes\\{}\\OpenWithProgids", file_type.extension), Some(&prog_id), String::new());
        }
        for entry in &manifest.system.context_menu {
            for target in &entry.on {
                let base = format!("Software\\Classes\\{}\\shell\\{}", target.class(), entry.verb(&app.id));
                let arguments = entry.arguments.as_deref().map(formatted).unwrap_or_else(|| target.default_argument().to_string());
                row(&mut registry, base.clone(), None, formatted(&entry.name));
                row(&mut registry, base.clone(), Some("Icon"), format!("[#{exe}],0"));
                row(&mut registry, format!("{base}\\command"), None, command(&arguments));
            }
        }
        if let Some(first) = registry.get(first_integration) {
            let key_path = first.id.clone();
            add_component("cIntegration", "Main", &mut registry, Some(key_path));
        }
        if let Some(feature) = &autostart_feature {
            registry.push(PlannedRegistry {
                id: "autostart".into(),
                key: "Software\\Microsoft\\Windows\\CurrentVersion\\Run".into(),
                name: Some(formatted(&app.id)),
                value: format!("\"[#{exe}]\""),
                component: "cAutostart".into(),
            });
            add_component("cAutostart", feature, &mut registry, Some("autostart".into()));
        }
    }

    // Environment variables: `[[env]]`, and the install folder on PATH when that option is on.
    let mut environment = Vec::new();
    for (index, variable) in manifest.system.environment.iter().enumerate() {
        environment.push(PlannedEnv {
            id: format!("e{}", index + 1),
            name: format!("=-{}", variable.name),
            value: formatted(&variable.value),
            component: "cEnv".into(),
        });
    }
    if !environment.is_empty() {
        add_component("cEnv", "Main", &mut registry, None);
    }
    if let Some(feature) = &path_feature {
        let folder = match &manifest.install.path_folder {
            Some(path) => folder_of(&format!("{path}/x"), &mut folders),
            None => "INSTALLFOLDER".to_string(),
        };
        environment.push(PlannedEnv {
            id: "path".into(),
            name: "=-PATH".into(),
            value: format!("[~];[{folder}]"),
            component: "cPath".into(),
        });
        add_component("cPath", feature, &mut registry, None);
    }

    // Run steps are deferred custom actions that start a packaged program, before the shortcuts
    // (after the install) or before the files go (on removal).
    let mut actions = Vec::new();
    let mut skipped = Vec::new();
    for (index, step) in manifest.run.iter().enumerate() {
        let Some(file) = manifest.files.iter().position(|file| file.path.eq_ignore_ascii_case(&step.file)) else {
            bail!("the run step \"{}\" names {}, which isn't among the files", step.name, step.file);
        };
        if step.elevated && !per_machine {
            skipped.push(format!("the elevated run step \"{}\" (a per-user package never has administrator rights)", step.name));
            continue;
        }
        let removing = step.when == RunWhen::Uninstall;
        // 2: an executable, 16: from the File table, 1024: deferred, run in the install script.
        let mut kind = 2 | 16 | 1024;
        if step.ignore_errors || removing {
            kind |= 64;
        }
        if step.elevated {
            kind |= 2048;
        }
        let mut condition = match step.when {
            RunWhen::Install => "NOT REMOVE".to_string(),
            RunWhen::FirstInstall => "NOT Installed AND NOT PREVIOUSVERSIONSINSTALLED".to_string(),
            RunWhen::Uninstall => "REMOVE=\"ALL\" AND NOT UPGRADINGPRODUCTCODE".to_string(),
        };
        if let Some(component) = &step.component {
            let feature = feature_id(component);
            condition.push_str(&if removing { format!(" AND ?{feature}=3") } else { format!(" AND &{feature}=3") });
        }
        if step.success_codes != [0] {
            skipped.push(format!(
                "the exit codes {:?} the run step \"{}\" accepts (Windows Installer accepts 0 only)",
                step.success_codes, step.name
            ));
        }
        actions.push(PlannedAction {
            action: format!("Run{}", index + 1),
            kind,
            source: files[file].key.clone(),
            target: step.arguments.as_deref().map(formatted),
            sequence: if removing { 3400 + index as i32 } else { 4010 + index as i32 },
            condition: Some(condition),
        });
    }
    // Settings > Apps shows where the app is, as it does for the setup file.
    actions.push(PlannedAction {
        action: "SetARPINSTALLLOCATION".into(),
        kind: 51,
        source: "ARPINSTALLLOCATION".into(),
        target: Some("[INSTALLFOLDER]".into()),
        sequence: 1010,
        condition: None,
    });

    // Requirements: the Windows build is checked by Windows Installer itself.
    let mut launch = vec![("NOT NEWERPRODUCTFOUND".to_string(), "A newer version of [ProductName] is already installed.".to_string())];
    for requirement in &manifest.requirements {
        match &requirement.check {
            Check::Windows { build } => {
                launch.push((format!("WindowsBuild >= {build}"), format!("{} needs Windows build {build} or newer.", formatted(&app.name))))
            }
            _ => skipped.push(format!("the requirement \"{}\" (only a Windows build can be checked)", requirement.name)),
        }
    }

    for (present, what) in [
        (!manifest.system.migrate.is_empty(), "[[migrate]]"),
        (manifest.updates.is_some(), "[updates] (an MSI is updated by installing a newer MSI)"),
        (!manifest.uninstall.data.is_empty(), "[uninstall] data folders"),
        (
            manifest.components.iter().any(|component| component.download.is_some()),
            "downloaded components (their files are in the package)",
        ),
    ] {
        if present {
            skipped.push(what.to_string());
        }
    }

    // The package code names this exact build's content; the product code names this version of the app.
    let digest: String = manifest.files.iter().map(|file| file.sha256.as_deref().unwrap_or(&file.path)).collect::<Vec<_>>().join(",");
    Ok(Plan {
        product_code: guid(&[&app.id, "product", &version]),
        upgrade_code: guid(&[&app.id, "upgrade"]),
        package_code: guid(&[&app.id, "package", &version, &digest]),
        version,
        per_machine,
        files,
        folders,
        features,
        shortcuts,
        registry,
        extra_components,
        environment,
        actions,
        launch,
        start_menu_folder,
        skipped,
    })
}

fn id(name: &str, max: usize) -> Column {
    Column::build(name).primary_key().category(Category::Identifier).string(max)
}

fn create(package: &mut Package<File>, table: &str, columns: Vec<Column>) -> Result<()> {
    package.create_table(table, columns).with_context(|| format!("couldn't create the {table} table"))
}

/// Text as the package's code page can hold it. Windows Installer reads a package in a code page,
/// not in Unicode; Windows-1252 (Western European) is the one every Windows has. Characters it
/// can't hold show as `?`, so an app with another script in its name should say so to its users.
fn latin(value: &str) -> String {
    value.chars().map(|c| if c.is_ascii() || ('\u{a0}'..='\u{ff}').contains(&c) { c } else { '?' }).collect()
}

fn text(value: impl Into<String>) -> Value {
    Value::Str(latin(&value.into()))
}

fn optional(value: Option<String>) -> Value {
    value.map_or(Value::Null, |value| Value::Str(latin(&value)))
}

fn insert(package: &mut Package<File>, table: &str, rows: Vec<Vec<Value>>) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    package.insert_rows(Insert::into(table).rows(rows)).with_context(|| format!("couldn't fill the {table} table"))
}

/// Writes the package to `out` (through a `.partial` file).
pub fn write(project: &Project, out: &Path, runtime: &Path) -> Result<Plan> {
    let plan = plan(project)?;
    if let Some(dir) = out.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("couldn't create {}", dir.display()))?;
    }
    let mut partial = out.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    let mut cabinet = partial.clone().into_os_string();
    cabinet.push(".cab");
    let cabinet = PathBuf::from(cabinet);
    // The window the package shows when somebody opens it: this app's setup program without the files.
    let mut front = partial.clone().into_os_string();
    front.push(".front.exe");
    let front = PathBuf::from(front);
    let result = crate::builder::write_msi_front(project, runtime, &front)
        .context("couldn't make the window the package shows")
        .and_then(|_| write_package(project, &plan, &partial, &cabinet, &front));
    let _ = std::fs::remove_file(&cabinet);
    let _ = std::fs::remove_file(&front);
    match result {
        Ok(()) => {
            if out.exists() {
                std::fs::remove_file(out).with_context(|| format!("couldn't replace {} (is it open?)", out.display()))?;
            }
            std::fs::rename(&partial, out).with_context(|| format!("couldn't move the package to {}", out.display()))?;
            Ok(plan)
        }
        Err(err) => {
            let _ = std::fs::remove_file(&partial);
            Err(err)
        }
    }
}

/// The files, compressed into a cabinet, each named by its File table key.
fn write_cabinet(plan: &Plan, path: &Path) -> Result<u64> {
    let mut builder = CabinetBuilder::new();
    let folder = builder.add_folder(CompressionType::MsZip);
    for file in &plan.files {
        folder.add_file(file.key.clone());
    }
    let out = File::create(path).with_context(|| format!("couldn't write {}", path.display()))?;
    let mut writer = builder.build(BufWriter::new(out)).context("couldn't start the cabinet")?;
    let mut files = plan.files.iter();
    while let Some(mut target) = writer.next_file().context("couldn't add a file to the cabinet")? {
        let Some(file) = files.next() else { break };
        let mut source = BufReader::new(File::open(&file.source).with_context(|| format!("couldn't read {}", file.source.display()))?);
        let copied = std::io::copy(&mut source, &mut target).with_context(|| format!("couldn't add {} to the cabinet", file.name))?;
        if copied != file.size {
            bail!("{} changed while it was being packed ({} bytes were expected, {copied} were read)", file.name, file.size);
        }
    }
    let mut inner = writer.finish().context("couldn't finish the cabinet")?;
    inner.flush()?;
    drop(inner);
    let size = std::fs::metadata(path)?.len();
    if size > MAX_CABINET + (8 << 20) {
        bail!("the cabinet is {}, more than the 2 GB Windows Installer reads", crate::output::size(size));
    }
    Ok(size)
}

fn write_package(project: &Project, plan: &Plan, path: &Path, cabinet: &Path, front: &Path) -> Result<()> {
    let manifest = &project.manifest;
    let app = &manifest.app;
    let file = File::options().read(true).write(true).create(true).truncate(true).open(path)?;
    let mut package = Package::create(PackageType::Installer, file).context("couldn't start the package")?;

    // Summary information: what Explorer's Properties and msiexec read before they open the tables.
    package.set_database_codepage(msi::CodePage::Windows1252);
    let info = package.summary_info_mut();
    info.set_codepage(msi::CodePage::Windows1252);
    info.set_title("Installation Database");
    info.set_subject(latin(&app.name));
    info.set_author(latin(&app.publisher));
    info.set_comments(latin(&app.description.clone().unwrap_or_else(|| format!("{} {}", app.name, app.version))));
    info.set_creating_application(format!("Bach {}", env!("CARGO_PKG_VERSION")));
    info.set_arch("x64");
    info.set_languages(&[msi::LanguageId::from_tag("en-US")]);
    info.set_uuid(Uuid::parse_str(plan.package_code.trim_matches(['{', '}'])).context("the package code isn't a GUID")?);
    info.set_creation_time_to_now();
    info.set_page_count(500);
    // Bit 1: the files are compressed in a cabinet; bit 3: installing needs no administrator rights.
    info.set_word_count(if plan.per_machine { 2 } else { 2 | 8 });
    info.set_doc_security(2);

    create(&mut package, "Property", vec![id("Property", 72), Column::build("Value").localizable().text_string(0)])?;
    create(
        &mut package,
        "Directory",
        vec![
            id("Directory", 72),
            Column::build("Directory_Parent").nullable().foreign_key("Directory", 1).category(Category::Identifier).string(72),
            Column::build("DefaultDir").localizable().category(Category::DefaultDir).string(255),
        ],
    )?;
    create(
        &mut package,
        "Component",
        vec![
            id("Component", 72),
            Column::build("ComponentId").nullable().category(Category::Guid).string(38),
            Column::build("Directory_").category(Category::Identifier).string(72),
            Column::build("Attributes").int16(),
            Column::build("Condition").nullable().category(Category::Condition).string(255),
            Column::build("KeyPath").nullable().category(Category::Identifier).string(72),
        ],
    )?;
    create(
        &mut package,
        "Feature",
        vec![
            id("Feature", 38),
            Column::build("Feature_Parent").nullable().category(Category::Identifier).string(38),
            Column::build("Title").nullable().localizable().text_string(64),
            Column::build("Description").nullable().localizable().text_string(255),
            Column::build("Display").nullable().int16(),
            Column::build("Level").int16(),
            Column::build("Directory_").nullable().category(Category::Identifier).string(72),
            Column::build("Attributes").int16(),
        ],
    )?;
    create(&mut package, "FeatureComponents", vec![id("Feature_", 38), id("Component_", 72)])?;
    create(
        &mut package,
        "File",
        vec![
            id("File", 72),
            Column::build("Component_").category(Category::Identifier).string(72),
            Column::build("FileName").localizable().category(Category::Filename).string(255),
            Column::build("FileSize").int32(),
            Column::build("Version").nullable().category(Category::Version).string(72),
            Column::build("Language").nullable().category(Category::Language).string(20),
            Column::build("Attributes").nullable().int16(),
            Column::build("Sequence").int16(),
        ],
    )?;
    create(
        &mut package,
        "Media",
        vec![
            Column::build("DiskId").primary_key().int16(),
            Column::build("LastSequence").int16(),
            Column::build("DiskPrompt").nullable().localizable().text_string(64),
            Column::build("Cabinet").nullable().category(Category::Cabinet).string(255),
            Column::build("VolumeLabel").nullable().text_string(32),
            Column::build("Source").nullable().category(Category::Property).string(72),
        ],
    )?;
    for table in ["InstallExecuteSequence", "AdminExecuteSequence", "InstallUISequence"] {
        create(
            &mut package,
            table,
            vec![
                id("Action", 72),
                Column::build("Condition").nullable().category(Category::Condition).string(255),
                Column::build("Sequence").nullable().int16(),
            ],
        )?;
    }
    create(
        &mut package,
        "LaunchCondition",
        vec![
            Column::build("Condition").primary_key().category(Category::Condition).string(255),
            Column::build("Description").localizable().formatted_string(255),
        ],
    )?;
    create(
        &mut package,
        "Upgrade",
        vec![
            Column::build("UpgradeCode").primary_key().category(Category::Guid).string(38),
            Column::build("VersionMin").primary_key().nullable().category(Category::Text).string(20),
            Column::build("VersionMax").primary_key().nullable().category(Category::Text).string(20),
            Column::build("Language").primary_key().nullable().category(Category::Text).string(255),
            Column::build("Attributes").primary_key().int32(),
            Column::build("Remove").nullable().formatted_string(255),
            Column::build("ActionProperty").category(Category::Identifier).string(72),
        ],
    )?;
    create(
        &mut package,
        "Shortcut",
        vec![
            id("Shortcut", 72),
            Column::build("Directory_").category(Category::Identifier).string(72),
            Column::build("Name").localizable().category(Category::Filename).string(128),
            Column::build("Component_").category(Category::Identifier).string(72),
            Column::build("Target").category(Category::Shortcut).string(72),
            Column::build("Arguments").nullable().formatted_string(255),
            Column::build("Description").nullable().localizable().text_string(255),
            Column::build("Hotkey").nullable().int16(),
            Column::build("Icon_").nullable().category(Category::Identifier).string(72),
            Column::build("IconIndex").nullable().int16(),
            Column::build("ShowCmd").nullable().int16(),
            Column::build("WkDir").nullable().category(Category::Identifier).string(72),
        ],
    )?;
    create(
        &mut package,
        "Registry",
        vec![
            id("Registry", 72),
            Column::build("Root").int16(),
            Column::build("Key").localizable().category(Category::RegPath).string(255),
            Column::build("Name").nullable().localizable().formatted_string(255),
            Column::build("Value").nullable().localizable().formatted_string(0),
            Column::build("Component_").category(Category::Identifier).string(72),
        ],
    )?;
    create(&mut package, "CreateFolder", vec![id("Directory_", 72), id("Component_", 72)])?;
    create(
        &mut package,
        "RemoveFolder",
        vec![
            id("FileKey", 72),
            Column::build("Component_").category(Category::Identifier).string(72),
            Column::build("DirProperty").category(Category::Identifier).string(72),
            Column::build("InstallMode").int16(),
        ],
    )?;
    create(
        &mut package,
        "Environment",
        vec![
            id("Environment", 72),
            Column::build("Name").localizable().text_string(255),
            Column::build("Value").nullable().localizable().formatted_string(255),
            Column::build("Component_").category(Category::Identifier).string(72),
        ],
    )?;
    create(
        &mut package,
        "CustomAction",
        vec![
            id("Action", 72),
            Column::build("Type").int16(),
            Column::build("Source").nullable().category(Category::CustomSource).string(72),
            Column::build("Target").nullable().localizable().formatted_string(255),
            Column::build("ExtendedType").nullable().int32(),
        ],
    )?;
    create(&mut package, "Binary", vec![id("Name", 72), Column::build("Data").binary()])?;
    // The app's icon (app.icon), for Settings > Apps and the shortcuts, as the setup file gives them.
    let icon = project.icon.as_ref().and_then(|path| std::fs::read(path).ok().map(|bytes| (path, bytes)));
    if icon.is_some() {
        create(&mut package, "Icon", vec![id("Name", 72), Column::build("Data").binary()])?;
    }

    // Properties.
    let mut properties = vec![
        ("ProductCode", plan.product_code.clone()),
        ("UpgradeCode", plan.upgrade_code.clone()),
        ("ProductName", app.name.clone()),
        ("ProductVersion", plan.version.clone()),
        ("Manufacturer", app.publisher.clone()),
        ("ProductLanguage", "1033".into()),
        ("SecureCustomProperties", "PREVIOUSVERSIONSINSTALLED;NEWERPRODUCTFOUND".into()),
        ("ARPNOMODIFY", "1".into()),
    ];
    if icon.is_some() {
        properties.push(("ARPPRODUCTICON", "AppIcon.ico".into()));
    }
    if plan.per_machine {
        properties.push(("ALLUSERS", "1".into()));
    } else {
        properties.push(("MSIINSTALLPERUSER", "1".into()));
    }
    if let Some(website) = &app.website {
        properties.push(("ARPURLINFOABOUT", website.clone()));
    }
    if let Some(description) = &app.description {
        properties.push(("ARPCOMMENTS", description.clone()));
    }
    insert(&mut package, "Property", properties.into_iter().map(|(name, value)| vec![text(name), text(value)]).collect())?;

    // Directories: the install folder below Program Files, or below the user's Programs folder.
    let install_name = manifest.install.folder.clone();
    let mut directories = vec![vec![text("TARGETDIR"), Value::Null, text("SourceDir")]];
    let base = if plan.per_machine { "ProgramFiles64Folder" } else { "LocalAppDataFolder" };
    directories.push(vec![text(base), text("TARGETDIR"), text(".")]);
    directories.push(vec![text("ProgramMenuFolder"), text("TARGETDIR"), text(".")]);
    directories.push(vec![text("DesktopFolder"), text("TARGETDIR"), text(".")]);
    if let Some(name) = &plan.start_menu_folder {
        directories.push(vec![text("StartMenuDir"), text("ProgramMenuFolder"), text(format!("STARTM~1|{name}"))]);
    }
    let install_parent = if plan.per_machine {
        base.to_string()
    } else {
        directories.push(vec![text("UserProgramsFolder"), text(base), text("PROGRA~1|Programs")]);
        "UserProgramsFolder".to_string()
    };
    directories.push(vec![text("INSTALLFOLDER"), text(install_parent), text(format!("INSTALL~1|{install_name}"))]);
    for (index, (folder, parent, name)) in plan.folders.iter().enumerate() {
        directories.push(vec![text(folder), text(parent), text(format!("DIR{:05}|{name}", index + 1))]);
    }
    insert(&mut package, "Directory", directories)?;

    // Components: one per file, plus one for the registry values. 64-bit components (256) in the 64-bit folders.
    let attributes = 256;
    let mut components: Vec<Vec<Value>> = plan
        .files
        .iter()
        .map(|file| {
            vec![text(&file.component), text(&file.component_id), text(&file.folder), Value::Int(attributes), Value::Null, text(&file.key)]
        })
        .collect();
    let mut feature_components: Vec<Vec<Value>> = plan.files.iter().map(|file| vec![text(&file.feature), text(&file.component)]).collect();
    for extra in &plan.extra_components {
        components.push(vec![
            text(&extra.id),
            text(guid(&[&app.id, "component", &extra.id])),
            text("INSTALLFOLDER"),
            // 256: a 64-bit component; 4: its key path is a registry value, not a file.
            Value::Int(attributes | 4),
            Value::Null,
            text(&extra.key_path),
        ]);
        feature_components.push(vec![text(&extra.feature), text(&extra.id)]);
    }
    insert(&mut package, "Component", components)?;
    insert(&mut package, "FeatureComponents", feature_components)?;

    // The folders are created, and removed again when they are empty, with the component of the first file in each.
    let mut created: Vec<Vec<Value>> =
        vec![vec![text("INSTALLFOLDER"), text(plan.files.first().map(|file| file.component.clone()).unwrap_or_default())]];
    let mut seen = std::collections::HashSet::from(["INSTALLFOLDER".to_string()]);
    for file in &plan.files {
        if seen.insert(file.folder.clone()) {
            created.push(vec![text(&file.folder), text(&file.component)]);
        }
    }
    insert(&mut package, "CreateFolder", created)?;

    insert(
        &mut package,
        "Feature",
        plan.features
            .iter()
            .enumerate()
            .map(|(index, feature)| {
                vec![
                    text(&feature.id),
                    optional(feature.parent.clone()),
                    text(&feature.title),
                    Value::Null,
                    Value::Int(if feature.parent.is_none() { 1 } else { 2 + index as i32 }),
                    Value::Int(feature.level),
                    text("INSTALLFOLDER"),
                    Value::Int(0),
                ]
            })
            .collect(),
    )?;

    // Files and their cabinet.
    let size = write_cabinet(plan, cabinet)?;
    insert(
        &mut package,
        "File",
        plan.files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                vec![
                    text(&file.key),
                    text(&file.component),
                    text(format!("{}|{}", file.short, file.name)),
                    Value::Int(i32::try_from(file.size).unwrap_or(i32::MAX)),
                    Value::Null,
                    Value::Null,
                    Value::Int(0),
                    Value::Int(1 + index as i32),
                ]
            })
            .collect(),
    )?;
    insert(
        &mut package,
        "Media",
        vec![vec![Value::Int(1), Value::Int(plan.files.len() as i32), Value::Null, text("#files.cab"), Value::Null, Value::Null]],
    )?;
    {
        let mut stream = package.write_stream("files.cab").context("couldn't store the cabinet")?;
        let mut cab = BufReader::new(File::open(cabinet)?);
        std::io::copy(&mut cab, &mut stream).context("couldn't store the cabinet")?;
        stream.flush()?;
        let _ = size;
    }

    // Shortcuts.
    insert(
        &mut package,
        "Shortcut",
        plan.shortcuts
            .iter()
            .enumerate()
            .map(|(index, shortcut)| {
                vec![
                    text(&shortcut.id),
                    text(&shortcut.folder),
                    text(format!("SC{:05}|{}", index + 1, shortcut.name)),
                    text(&shortcut.component),
                    text(format!("[#{}]", shortcut.file_key)),
                    optional(shortcut.arguments.clone()),
                    optional(app.description.clone()),
                    Value::Null,
                    if icon.is_some() { text("AppIcon.ico") } else { Value::Null },
                    Value::Null,
                    Value::Int(1),
                    text("INSTALLFOLDER"),
                ]
            })
            .collect(),
    )?;

    if let Some((_, bytes)) = &icon {
        insert(&mut package, "Icon", vec![vec![text("AppIcon.ico"), Value::Binary]])?;
        let mut stream = package.write_stream("Icon.AppIcon.ico").context("couldn't store the icon")?;
        stream.write_all(bytes).context("couldn't store the icon")?;
        stream.flush()?;
    }

    // Registry values: -1 is HKLM for a per-machine package and HKCU for a per-user one.
    insert(
        &mut package,
        "Registry",
        plan.registry
            .iter()
            .map(|value| {
                vec![
                    text(&value.id),
                    Value::Int(-1),
                    text(&value.key),
                    optional(value.name.clone()),
                    text(&value.value),
                    text(&value.component),
                ]
            })
            .collect(),
    )?;

    // Upgrades: a higher version replaces older ones; an older one refuses to replace a newer one.
    insert(
        &mut package,
        "Upgrade",
        vec![
            vec![
                text(&plan.upgrade_code),
                text("0.0.0"),
                text(&plan.version),
                Value::Null,
                Value::Int(256),
                Value::Null,
                text("PREVIOUSVERSIONSINSTALLED"),
            ],
            vec![
                text(&plan.upgrade_code),
                text(&plan.version),
                Value::Null,
                Value::Null,
                Value::Int(2),
                Value::Null,
                text("NEWERPRODUCTFOUND"),
            ],
        ],
    )?;
    insert(&mut package, "LaunchCondition", plan.launch.iter().map(|(condition, message)| vec![text(condition), text(message)]).collect())?;
    insert(
        &mut package,
        "Environment",
        plan.environment.iter().map(|var| vec![text(&var.id), text(&var.name), text(&var.value), text(&var.component)]).collect(),
    )?;
    insert(
        &mut package,
        "CustomAction",
        plan.actions
            .iter()
            .map(|action| {
                vec![text(&action.action), Value::Int(action.kind), text(&action.source), optional(action.target.clone()), Value::Null]
            })
            .collect(),
    )?;
    // The window: opening the package with Windows' own interface (or removing it under Settings > Apps)
    // runs the package's copy of Bach's setup program, which shows the same pages as a setup file and
    // calls Windows Installer with the answers. Quiet installs (`/qn`) never run the interface sequence.
    insert(&mut package, "Binary", vec![vec![text("BachFront"), Value::Binary]])?;
    {
        let mut stream = package.write_stream("Binary.BachFront").context("couldn't store the window")?;
        let mut source = BufReader::new(File::open(front).context("couldn't read the window")?);
        std::io::copy(&mut source, &mut stream).context("couldn't store the window")?;
        stream.flush()?;
    }
    let window = |uninstall: bool| {
        // 2: an executable from the Binary table (waited for); 64: its exit code doesn't fail the install.
        let arguments = "--msi-package \"[OriginalDatabase]\" --msi-product \"[ProductCode]\" --msi-upgrade \"[UpgradeCode]\"";
        vec![
            text(if uninstall { "BachWindowRemove" } else { "BachWindow" }),
            Value::Int(2 | 64),
            text("BachFront"),
            text(if uninstall { format!("{arguments} --uninstall") } else { arguments.to_string() }),
            Value::Null,
        ]
    };
    insert(&mut package, "CustomAction", vec![window(false), window(true)])?;
    if plan.start_menu_folder.is_some() {
        insert(&mut package, "RemoveFolder", vec![vec![text("rf_start"), text("cStart"), text("StartMenuDir"), Value::Int(2)]])?;
    }
    let sequence = |actions: &[(&str, i32)]| -> Vec<Vec<Value>> {
        actions.iter().map(|(action, number)| vec![text(*action), Value::Null, Value::Int(*number)]).collect()
    };
    // The custom actions join the sequence with their conditions.
    let mut run_rows: Vec<Vec<Value>> = plan
        .actions
        .iter()
        .map(|action| vec![text(&action.action), optional(action.condition.clone()), Value::Int(action.sequence)])
        .collect();
    run_rows.extend(sequence(&[("RemoveEnvironmentStrings", 3300), ("WriteEnvironmentStrings", 5200)]));
    insert(
        &mut package,
        "InstallExecuteSequence",
        [
            run_rows,
            sequence(&[
                ("FindRelatedProducts", 200),
                ("LaunchConditions", 410),
                ("CostInitialize", 800),
                ("FileCost", 900),
                ("CostFinalize", 1000),
                ("MigrateFeatureStates", 1200),
                ("InstallValidate", 1400),
                ("RemoveExistingProducts", 1450),
                ("InstallInitialize", 1500),
                ("ProcessComponents", 1600),
                ("UnpublishFeatures", 1800),
                ("RemoveRegistryValues", 2600),
                ("RemoveShortcuts", 3200),
                ("RemoveFiles", 3500),
                ("RemoveFolders", 3600),
                ("CreateFolders", 3700),
                ("InstallFiles", 4000),
                ("CreateShortcuts", 4500),
                ("WriteRegistryValues", 5000),
                ("RegisterUser", 6000),
                ("RegisterProduct", 6100),
                ("PublishFeatures", 6300),
                ("PublishProduct", 6400),
                ("InstallFinalize", 6600),
            ]),
        ]
        .concat(),
    )?;
    // The interface sequence: with a window (UILevel 5, what opening the package gives), the window
    // does the work and the sequence ends before the engine's own install; at any other level the
    // engine installs as usual.
    insert(
        &mut package,
        "InstallUISequence",
        vec![
            vec![text("FindRelatedProducts"), Value::Null, Value::Int(200)],
            vec![text("LaunchConditions"), Value::Null, Value::Int(410)],
            vec![text("CostInitialize"), Value::Null, Value::Int(800)],
            vec![text("FileCost"), Value::Null, Value::Int(900)],
            vec![text("CostFinalize"), Value::Null, Value::Int(1000)],
            vec![text("BachWindow"), text("UILevel >= 5 AND NOT (REMOVE=\"ALL\")"), Value::Int(1100)],
            vec![text("BachWindowRemove"), text("UILevel >= 5 AND REMOVE=\"ALL\""), Value::Int(1110)],
            vec![text("ExecuteAction"), text("UILevel < 5"), Value::Int(1200)],
        ],
    )?;
    insert(
        &mut package,
        "AdminExecuteSequence",
        sequence(&[
            ("CostInitialize", 800),
            ("FileCost", 900),
            ("CostFinalize", 1000),
            ("InstallValidate", 1400),
            ("InstallInitialize", 1500),
            ("InstallAdminPackage", 3900),
            ("InstallFiles", 4000),
            ("InstallFinalize", 6600),
        ]),
    )?;
    package.flush().context("couldn't finish the package")?;
    let mut file = package.into_inner().context("couldn't close the package")?;
    file.flush()?;
    file.rewind()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_three_numbers_windows_installer_accepts() {
        assert_eq!(product_version("1.4.2").unwrap(), "1.4.2");
        assert_eq!(product_version("v2.0").unwrap(), "2.0.0");
        assert_eq!(product_version("7").unwrap(), "7.0.0");
        assert_eq!(product_version("255.255.65535").unwrap(), "255.255.65535");
        for bad in ["1.2.3.4", "256.0.0", "1.256.0", "1.0.65536", "1.0.0-beta", "x", "", "1..2", "2024.1.1"] {
            assert!(product_version(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn short_names_are_eight_dot_three() {
        assert_eq!(short_name(12, "hello.exe"), "F0000012.EXE");
        assert_eq!(short_name(1, "readme"), "F0000001");
        assert_eq!(short_name(3, "archive.tar.gz"), "F0000003.GZ");
        assert_eq!(short_name(4, "x.jpeg"), "F0000004.JPE");
        for (index, name) in [(1, "a b.txt"), (99, "ü.日本"), (123_456, "noext.")] {
            let short = short_name(index, name);
            let (stem, extension) = short.split_once('.').unwrap_or((&short, ""));
            assert!(stem.len() <= 8 && extension.len() <= 3 && short.is_ascii() && !short.contains(' '), "{short}");
        }
    }

    #[test]
    fn guids_are_stable_and_well_formed() {
        let one = guid(&["App", "component", "a/b.txt"]);
        assert_eq!(one, guid(&["App", "component", "a/b.txt"]));
        assert_ne!(one, guid(&["App", "component", "a/c.txt"]));
        assert_ne!(one, guid(&["Other", "component", "a/b.txt"]));
        // {XXXXXXXX-XXXX-5XXX-[89AB]XXX-XXXXXXXXXXXX}, upper case.
        assert_eq!(one.len(), 38);
        assert!(one.starts_with('{') && one.ends_with('}') && one == one.to_ascii_uppercase());
        assert_eq!(one.chars().nth(15), Some('5'));
        assert!(Uuid::parse_str(one.trim_matches(['{', '}'])).is_ok());
        // Without a separator, two splits of the same letters would give one GUID.
        assert_ne!(guid(&["ab", "c"]), guid(&["a", "bc"]));
    }
}
