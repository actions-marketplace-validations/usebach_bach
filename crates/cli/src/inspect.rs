//! `bach inspect`: what a setup file (or an uninstaller) carries, and the
//! summary `bach check` and `bach build` print for a config.

use std::path::Path;

use anyhow::{bail, Context, Result};
use bach_pack::container;
use bach_pack::i18n::{self, ANY_LANGUAGE};
use bach_pack::manifest::{Choice, Manifest};

use crate::output::{self, field};

pub fn inspect(path: &Path) -> Result<()> {
    // A macOS setup file keeps its payload inside the Setup app; keep it readable while we look.
    let mac = crate::macfile::payload(path)?;
    let mut file = match &mac {
        Some(payload) => payload.open()?,
        None => std::fs::File::open(path).with_context(|| format!("couldn't open {}", path.display()))?,
    };
    let Some(layout) = container::locate(&mut file)? else {
        bail!("{} isn't a Bach setup file", path.display());
    };
    let manifest = container::read_manifest(&mut file, &layout)?;
    let app = &manifest.app;
    let kind = if layout.has_archive() {
        "Installer"
    } else if manifest.online {
        "Online installer"
    } else {
        "Uninstaller"
    };
    output::result("kind", kind.to_lowercase());
    output::result("platform", manifest.platform.name());
    output::result("app_id", app.id.clone());
    output::result("name", app.name.clone());
    output::result("version", app.version.clone());
    output::result("publisher", app.publisher.clone());
    output::result("executable", app.executable.clone());
    output::result("files", manifest.files.len());
    output::result("size", manifest.total_size());
    say!("{kind} for {} {}{}", app.name, app.version, if manifest.platform.is_macos() { " (macOS)" } else { "" });
    field("App id", &app.id);
    details(&manifest, manifest.access.as_ref().is_some_and(|access| access.encryption.is_some()));
    if layout.has_archive() {
        field("Packed", output::size(layout.archive.len));
    }
    if let Some(chosen) = &manifest.chosen {
        let mut picked = Vec::new();
        let mac = manifest.platform.is_macos();
        for (on, name) in [
            (chosen.desktop_shortcut, "desktop shortcut"),
            (chosen.start_menu_shortcut, if mac { "Applications" } else { "Start menu" }),
            (chosen.start_with_windows, if mac { "open at login" } else { "start with Windows" }),
            (chosen.add_to_path, "PATH"),
            (chosen.auto_update, "automatic updates"),
        ] {
            if on {
                picked.push(name.to_string());
            }
        }
        picked.extend(chosen.components.iter().cloned());
        field("Chosen", if picked.is_empty() { "nothing optional".into() } else { picked.join(", ") });
    }
    field("Built by", format!("Bach {}", manifest.builder));
    Ok(())
}

/// Publisher, program, files, folder, options, languages and the rest.
/// `encrypted` says whether the files are (or will be) locked with a key.
pub fn details(manifest: &Manifest, encrypted: bool) {
    let install = &manifest.install;
    field("Publisher", &manifest.app.publisher);
    field("Program", &manifest.app.executable);
    field("Files", format!("{} ({})", manifest.files.len(), output::size(manifest.total_size())));
    let mac = manifest.platform.is_macos();
    let folder = match (mac, install.scope.is_machine()) {
        (true, true) => format!("/Applications/{}", install.folder).trim_end_matches('/').to_string(),
        (true, false) => format!("~/Applications/{}", install.folder).trim_end_matches('/').to_string(),
        (false, true) => format!(r"%ProgramFiles%\{}", install.folder),
        (false, false) => format!(r"%LOCALAPPDATA%\Programs\{}", install.folder),
    };
    field("Installs to", if install.change_folder { folder } else { format!("{folder} (fixed)") });
    if install.scope.is_machine() {
        field("For", "every user of the computer (asks for administrator rights once)");
    }

    let mut options = Vec::new();
    for (choice, name) in [
        (install.desktop_shortcut, "desktop shortcut"),
        (install.start_menu_shortcut, if mac { "Applications" } else { "Start menu" }),
        (install.start_with_windows, if mac { "open at login" } else { "start with Windows" }),
        (install.add_to_path, "add to PATH"),
    ] {
        match choice {
            Choice::Never => {}
            Choice::Off => options.push(format!("{name} (offered)")),
            Choice::On => options.push(format!("{name} (offered, ticked)")),
            Choice::Always => options.push(name.to_string()),
        }
    }
    match bach_pack::manifest::choices_auto_update(manifest) {
        Choice::Never => {}
        Choice::Off => options.push("automatic updates (offered)".to_string()),
        Choice::On => options.push("automatic updates (offered, ticked)".to_string()),
        Choice::Always => options.push("automatic updates".to_string()),
    }
    field("Options", options.join(", "));
    for component in &manifest.components {
        let title = i18n::resolve(&component.title, "en", &manifest.locale.languages).unwrap_or(&component.id);
        let count = manifest.files.iter().filter(|file| file.component.as_ref() == Some(&component.id)).count();
        let ticked = if component.default { ", ticked" } else { "" };
        let files = if count == 1 { "file" } else { "files" };
        let downloaded =
            component.download.as_ref().map(|download| format!(", downloaded: {}", output::size(download.size))).unwrap_or_default();
        field("Component", format!("{} \"{title}\" ({count} {files}{ticked}{downloaded})", component.id));
    }
    for protocol in &manifest.protocols {
        field("Opens links", format!("{}://", protocol.scheme));
    }
    if !manifest.file_types.is_empty() {
        let types: Vec<&str> = manifest.file_types.iter().map(|file_type| file_type.extension.as_str()).collect();
        field("Opens files", types.join(", "));
    }
    let languages = if manifest.locale.languages.is_empty() {
        format!("all {} (follows {})", i18n::LANGUAGES.len(), if mac { "macOS" } else { "Windows" })
    } else {
        manifest.locale.languages.join(", ")
    };
    field("Languages", languages);
    field("Look", look(manifest));
    if let Some(info) = &manifest.mac {
        field("Bundle id", &info.bundle_id);
        if let Some(minimum) = &info.minimum_version {
            field("Needs", format!("macOS {minimum} or newer"));
        }
    }
    if let Some(license) = &manifest.locale.license {
        let codes: Vec<&str> = license.keys().map(|code| if code == ANY_LANGUAGE { "all languages" } else { code }).collect();
        field("License", format!("shown first ({})", codes.join(", ")));
    }
    if !manifest.locale.text.is_empty() {
        let keys: Vec<&str> = manifest.locale.text.keys().map(String::as_str).collect();
        field("Own texts", keys.join(", "));
    }
    for requirement in &manifest.requirements {
        field("Requires", &requirement.name);
    }
    for step in &manifest.run {
        let when = match step.when {
            bach_pack::manifest::RunWhen::Install => "on install and update",
            bach_pack::manifest::RunWhen::FirstInstall => "on first install",
            bach_pack::manifest::RunWhen::Uninstall => "before removal",
        };
        let args = step.arguments.as_deref().map(|args| format!(" {args}")).unwrap_or_default();
        let elevated = if step.elevated { ", as administrator" } else { "" };
        field("Runs", format!("{} ({}{args}), {when}{elevated}", step.name, step.file));
    }
    for shortcut in &manifest.shortcuts {
        field("Shortcut", format!("{} → {}", shortcut.name, shortcut.target));
    }
    for value in &manifest.system.registry {
        let name = if value.name.is_empty() { "(default)" } else { value.name.as_str() };
        let hive = if manifest.install.scope.is_machine() { "HKLM" } else { "HKCU" };
        field("Registry", format!("{hive}\\{}\\{name}", value.key));
    }
    if let Some(from) = &manifest.patch_from {
        field("Update patch", format!("for computers on {from}: carries only the files that changed since"));
    }
    for old in &manifest.system.migrate {
        field("Replaces", format!("the older install \"{}\"", old.uninstall_key));
    }
    for var in &manifest.system.environment {
        field("Variable", &var.name);
    }
    for entry in &manifest.system.context_menu {
        let places: Vec<String> = entry.on.iter().cloned().map(String::from).collect();
        field("Right-click", format!("{} ({})", entry.name, places.join(", ")));
    }
    for service in &manifest.system.services {
        field("Service", format!("{} ({}, runs {})", service.name, service.display_name, service.file));
    }
    for rule in &manifest.system.firewall {
        let what = rule.file.clone().or_else(|| rule.ports.clone()).unwrap_or_default();
        field("Firewall", format!("{} ({what})", rule.name));
    }
    if let Some(updates) = &manifest.updates {
        field("Updates", &updates.feed);
    }
    if manifest.online {
        field("Online", "downloads the newest version through the update feed");
    }
    if let Some(access) = &manifest.access {
        let mut parts = vec!["asks for a key".to_string()];
        parts.push(if encrypted { "files encrypted".into() } else { "files not encrypted".into() });
        if access.send_key {
            parts.push("key sent with updates".into());
        }
        field("Access", parts.join(", "));
        if let Some(url) = &access.get_key_url {
            field("Get a key", url);
        }
    }
    if !manifest.uninstall.data.is_empty() {
        field("Data", manifest.uninstall.data.join(", "));
    }
}

/// The window's style and what the developer changed about it, in a line.
fn look(manifest: &Manifest) -> String {
    let look = &manifest.look;
    let theme = match look.theme {
        bach_pack::manifest::Theme::System => {
            if manifest.platform.is_macos() {
                "follows macOS' light or dark"
            } else {
                "follows Windows' light or dark"
            }
        }
        bach_pack::manifest::Theme::Light => "light",
        bach_pack::manifest::Theme::Dark => "dark",
    };
    let mut parts = vec![format!("{} style", look.style.as_deref().unwrap_or(bach_pack::style::DEFAULT)), theme.to_string()];
    if let Some(custom) = &look.custom {
        if let Some(font) = &custom.font {
            parts.push(format!("font {font}"));
        }
        if custom.heading_font.is_some() && custom.heading_font != custom.font {
            parts.push(format!("headings in {}", custom.heading_font.as_deref().unwrap_or_default()));
        }
        if !custom.light.is_empty() || !custom.dark.is_empty() || look.accent.is_some() {
            parts.push("own colours".into());
        }
        if custom.radius.is_some() || custom.button_radius.is_some() {
            parts.push("own corners".into());
        }
    } else if look.accent.is_some() {
        parts.push("own accent".into());
    }
    let shipped = |kind: bach_pack::manifest::LookFileKind| look.files.iter().filter(|file| file.kind == kind).count();
    let fonts = shipped(bach_pack::manifest::LookFileKind::Font);
    if fonts > 0 {
        parts.push(format!("{fonts} font file{}", if fonts == 1 { "" } else { "s" }));
    }
    if shipped(bach_pack::manifest::LookFileKind::BackgroundPng) + shipped(bach_pack::manifest::LookFileKind::BackgroundJpeg) > 0 {
        parts.push("background picture".into());
    }
    if let Some(layout) = &look.layout {
        use bach_pack::style::layout::Panel;
        let arranged = layout.resolve();
        let mut what = vec![match arranged.height {
            Some(height) => format!("{}×{height}", arranged.width),
            None => format!("{} wide", arranged.width),
        }];
        let side = match arranged.panel {
            Panel::None => None,
            Panel::Left => Some("panel at the left"),
            Panel::Right => Some("panel at the right"),
            Panel::Top => Some("banner on top"),
        };
        what.extend(side.map(String::from));
        if arranged.header_on_panel() {
            what.push("heading on it".into());
        }
        let name = layout.preset.as_deref().map(|preset| format!("{preset} layout")).unwrap_or_else(|| "own layout".into());
        parts.push(format!("{name} ({})", what.join(", ")));
    }
    if shipped(bach_pack::manifest::LookFileKind::PanelPng) + shipped(bach_pack::manifest::LookFileKind::PanelJpeg) > 0 {
        parts.push("panel picture".into());
    }
    parts.join(", ")
}
