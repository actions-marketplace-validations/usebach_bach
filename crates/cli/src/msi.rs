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
use bach_pack::manifest::{Choice, Manifest, RegistryData};
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
            Some(id) => format!("F_{id}"),
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
            id: format!("F_{}", component.id),
            parent: Some("Main".into()),
            title,
            level: if component.default { 1 } else { 4 },
        });
    }

    // Shortcuts live in the component of the program they start.
    let mut shortcuts = Vec::new();
    let program = manifest.files.iter().position(|file| file.path.eq_ignore_ascii_case(&app.executable));
    if let Some(program) = program {
        let (key, component) = (files[program].key.clone(), files[program].component.clone());
        if manifest.install.start_menu_shortcut != Choice::Never {
            shortcuts.push(PlannedShortcut {
                id: "StartMenuShortcut".into(),
                folder: "ProgramMenuFolder".into(),
                name: app.name.clone(),
                file_key: key.clone(),
                component: component.clone(),
                arguments: None,
            });
        }
        if matches!(manifest.install.desktop_shortcut, Choice::On | Choice::Always) {
            shortcuts.push(PlannedShortcut {
                id: "DesktopShortcut".into(),
                folder: "DesktopFolder".into(),
                name: app.name.clone(),
                file_key: key,
                component,
                arguments: None,
            });
        }
    }
    for (index, extra) in manifest.shortcuts.iter().enumerate() {
        if let Some(target) = manifest.files.iter().position(|file| file.path.eq_ignore_ascii_case(&extra.target)) {
            shortcuts.push(PlannedShortcut {
                id: format!("ExtraShortcut{}", index + 1),
                folder: "ProgramMenuFolder".into(),
                name: extra.name.clone(),
                file_key: files[target].key.clone(),
                component: files[target].component.clone(),
                arguments: extra.arguments.clone(),
            });
        }
    }

    let registry = manifest
        .system
        .registry
        .iter()
        .enumerate()
        .map(|(index, value)| PlannedRegistry {
            id: format!("r{}", index + 1),
            key: value.key.replace("{app}", "[INSTALLFOLDER]"),
            name: Some(value.name.clone()).filter(|name| !name.is_empty()),
            value: match &value.data {
                RegistryData::String(text) => text.replace("{app}", "[INSTALLFOLDER]"),
                RegistryData::Expand(text) => format!("#%{}", text.replace("{app}", "[INSTALLFOLDER]")),
                RegistryData::Dword(number) => format!("#{number}"),
            },
        })
        .collect();

    let mut skipped = Vec::new();
    for (present, what) in [
        (!manifest.run.is_empty(), "[[run]] steps"),
        (!manifest.requirements.is_empty(), "[[requirements]]"),
        (!manifest.system.migrate.is_empty(), "[[migrate]]"),
        (!manifest.system.environment.is_empty(), "[[env]] variables"),
        (!manifest.system.context_menu.is_empty(), "[[context_menu]] entries"),
        (!manifest.protocols.is_empty(), "[[protocols]]"),
        (!manifest.file_types.is_empty(), "[[file_types]]"),
        (manifest.install.start_with_windows != Choice::Never, "start with Windows"),
        (manifest.install.add_to_path != Choice::Never, "adding to PATH"),
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
pub fn write(project: &Project, out: &Path) -> Result<Plan> {
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
    let result = write_package(project, &plan, &partial, &cabinet);
    let _ = std::fs::remove_file(&cabinet);
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

fn write_package(project: &Project, plan: &Plan, path: &Path, cabinet: &Path) -> Result<()> {
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
    for table in ["InstallExecuteSequence", "AdminExecuteSequence"] {
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
    if let Some(first) = plan.registry.first() {
        components.push(vec![
            text("cRegistry"),
            text(guid(&[&app.id, "component", "registry"])),
            text("INSTALLFOLDER"),
            // 256: a 64-bit component; 4: its key path is a registry value, not a file.
            Value::Int(attributes | 4),
            Value::Null,
            text(&first.id),
        ]);
        feature_components.push(vec![text("Main"), text("cRegistry")]);
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
                    Value::Null,
                    Value::Null,
                    Value::Int(1),
                    text("INSTALLFOLDER"),
                ]
            })
            .collect(),
    )?;

    // Registry values: -1 is HKLM for a per-machine package and HKCU for a per-user one.
    insert(
        &mut package,
        "Registry",
        plan.registry
            .iter()
            .map(|value| {
                vec![text(&value.id), Value::Int(-1), text(&value.key), optional(value.name.clone()), text(&value.value), text("cRegistry")]
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
    insert(
        &mut package,
        "LaunchCondition",
        vec![vec![text("NOT NEWERPRODUCTFOUND"), text("A newer version of [ProductName] is already installed.")]],
    )?;
    let sequence = |actions: &[(&str, i32)]| -> Vec<Vec<Value>> {
        actions.iter().map(|(action, number)| vec![text(*action), Value::Null, Value::Int(*number)]).collect()
    };
    insert(
        &mut package,
        "InstallExecuteSequence",
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
