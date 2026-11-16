//! `bach winget`: the three manifests the Windows Package Manager needs to
//! list a setup file, ready for a pull request to microsoft/winget-pkgs.
//!
//! Bach's setup files are per-user `exe` installers with `--silent`, and
//! the Settings › Apps entry is named by the app id, which winget reads as
//! the product code to tell installed versions apart.

use std::path::Path;

use anyhow::{bail, Context, Result};
use bach_pack::container;
use bach_pack::manifest::Manifest;

const MANIFEST_VERSION: &str = "1.6.0";

pub struct Options<'a> {
    pub setup: &'a Path,
    pub url: &'a str,
    pub license: Option<&'a str>,
    pub out: Option<&'a Path>,
}

pub fn command(options: Options) -> Result<()> {
    if !bach_pack::manifest::extras::is_https(options.url) {
        bail!("--url must be the https link the setup file is downloaded from");
    }
    let manifest = read_full_setup(options.setup, "winget")?;
    // Hashed in pieces; only the start of the file is needed for the architecture.
    let (sha256, _) = crate::feed::digest_of(options.setup)?;
    let sha256 = sha256.to_ascii_uppercase();
    let architecture = setup_architecture(options.setup)?;
    let id = package_id(&manifest);
    let version = &manifest.app.version;
    let dir = match options.out {
        Some(out) => out.to_path_buf(),
        None => options.setup.parent().unwrap_or(Path::new(".")).join("winget").join(&id).join(folder_name(version)),
    };
    std::fs::create_dir_all(&dir).with_context(|| format!("couldn't create {}", dir.display()))?;
    let files = manifests(&manifest, &id, options.url, &sha256, options.license, architecture);
    for (name, text) in &files {
        std::fs::write(dir.join(name), text).with_context(|| format!("couldn't write {name}"))?;
    }
    say!("Wrote the winget manifests for {id} {version} to {}", dir.display());
    if options.license.is_none() {
        crate::output::warnings(&["the manifests say License: Proprietary; pass --license with yours (like MIT)".into()]);
    }
    say!("Check them with `winget validate {}`, then open a pull request to microsoft/winget-pkgs.", dir.display());
    Ok(())
}

/// A version as a folder name. It comes from the setup file, so it may hold
/// anything: only letters, digits, `.`, `-`, `+` and `_` stay, and a name made
/// of dots alone (`..`) is replaced.
fn folder_name(version: &str) -> String {
    let name: String =
        version.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_') { c } else { '_' }).collect();
    if name.chars().all(|c| c == '.') {
        "version".into()
    } else {
        name
    }
}

/// The manifest of a full setup file, for the commands that list a setup file
/// somewhere (`command` is theirs, for the message). An online installer or an
/// uninstaller carries no files, so there is nothing to list.
pub(crate) fn read_full_setup(setup: &Path, command: &str) -> Result<Manifest> {
    let mut file = std::fs::File::open(setup).with_context(|| format!("couldn't open {}", setup.display()))?;
    let Some(layout) = container::locate(&mut file)? else {
        bail!("{} isn't a Bach setup file", setup.display());
    };
    let manifest = container::read_manifest(&mut file, &layout)?;
    if !layout.has_archive() {
        let what = if manifest.online { "an online installer" } else { "an uninstaller" };
        bail!("{} is {what}; point bach {command} at the full setup file", setup.display());
    }
    Ok(manifest)
}

/// The architecture a setup file was built for, from the start of the file.
pub(crate) fn setup_architecture(setup: &Path) -> Result<&'static str> {
    let mut head = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(std::fs::File::open(setup)?, 64 * 1024), &mut head)?;
    Ok(architecture(&head))
}

/// `Publisher.AppId` with only the characters winget allows.
/// winget allows 32 characters in each of the two parts, and app ids can have 64.
fn package_id(manifest: &Manifest) -> String {
    let clean = |text: &str| text.chars().filter(|c| c.is_ascii_alphanumeric()).take(32).collect::<String>();
    let publisher = clean(&manifest.app.publisher);
    let app = clean(&manifest.app.id);
    format!("{}.{}", if publisher.is_empty() { "Publisher".into() } else { publisher }, if app.is_empty() { "App".into() } else { app })
}

/// A YAML text in double quotes, so a `#` or `: ` in a link is never read as
/// a comment or a mapping.
pub(crate) fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The architecture winget should list a setup file under, read from the
/// machine field of its PE header: the x64 runtime unless it was built with the
/// ARM64 (or an x86) one.
fn architecture(bytes: &[u8]) -> &'static str {
    let machine = bytes.get(0x3c..0x40).and_then(|at| at.try_into().ok()).map(u32::from_le_bytes).and_then(|pe| {
        bytes.get(usize::try_from(pe).ok()?.checked_add(4)?..)?.get(..2).map(|machine| u16::from_le_bytes([machine[0], machine[1]]))
    });
    match machine {
        Some(0xAA64) => "arm64",
        Some(0x014C) => "x86",
        _ => "x64",
    }
}

fn yaml(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

fn manifests(manifest: &Manifest, id: &str, url: &str, sha256: &str, license: Option<&str>, architecture: &str) -> Vec<(String, String)> {
    let app = &manifest.app;
    let version = &app.version;
    let header = |kind: &str| {
        format!(
            "# Created with Bach (bach winget).\n# yaml-language-server: $schema=https://aka.ms/winget-manifest.{kind}.{MANIFEST_VERSION}.schema.json\n\nPackageIdentifier: {id}\nPackageVersion: {}\n",
            yaml(version)
        )
    };
    let version_file = format!("{}DefaultLocale: en-US\nManifestType: version\nManifestVersion: {MANIFEST_VERSION}\n", header("version"));
    // A machine-wide setup file installs for every user (winget calls it "machine").
    let scope = if manifest.install.scope.is_machine() { "machine" } else { "user" };
    let installer = format!(
        "{}InstallerType: exe\nScope: {scope}\nInstallModes:\n  - interactive\n  - silent\n  - silentWithProgress\nInstallerSwitches:\n  Silent: --silent\n  SilentWithProgress: --silent\n  InstallLocation: --dir \"<INSTALLPATH>\"\n  Log: --log \"<LOGPATH>\"\nUpgradeBehavior: install\nProductCode: {}\nInstallers:\n  - Architecture: {architecture}\n    InstallerUrl: {}\n    InstallerSha256: {sha256}\nManifestType: installer\nManifestVersion: {MANIFEST_VERSION}\n",
        header("installer"),
        yaml(&app.id),
        quoted(url)
    );
    let description = app.description.clone().unwrap_or_else(|| app.name.clone());
    let homepage = app.website.as_ref().map(|site| format!("PackageUrl: {}\n", quoted(site))).unwrap_or_default();
    let locale = format!(
        "{}PackageLocale: en-US\nPublisher: {}\nPackageName: {}\n{homepage}License: {}\nShortDescription: {}\nManifestType: defaultLocale\nManifestVersion: {MANIFEST_VERSION}\n",
        header("defaultLocale"),
        yaml(&app.publisher),
        yaml(&app.name),
        yaml(license.unwrap_or("Proprietary")),
        yaml(&description)
    );
    vec![(format!("{id}.yaml"), version_file), (format!("{id}.installer.yaml"), installer), (format!("{id}.locale.en-US.yaml"), locale)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_ids_fit_winget_and_links_are_quoted() {
        let mut long = manifest();
        long.app.id = "A".repeat(50);
        long.app.publisher = "Publisher With A Rather Long Name Incorporated Worldwide".into();
        let id = package_id(&long);
        let (publisher, app) = id.split_once('.').unwrap();
        assert!(publisher.len() <= 32 && app.len() <= 32 && !publisher.is_empty() && !app.is_empty(), "{id}");
        assert_eq!(package_id(&manifest()), "HelloInc.HelloApp");
        // A `#` or `: ` in a link is text, not a comment or a mapping.
        assert_eq!(quoted("https://x.example/a#b: c"), "\"https://x.example/a#b: c\"");
        assert_eq!(quoted("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }

    #[test]
    fn a_version_never_leaves_the_output_folder() {
        assert_eq!(folder_name("1.2.3-beta+5"), "1.2.3-beta+5");
        assert_eq!(folder_name(r"..\..\x"), ".._.._x");
        for evil in ["..", ".", "", "../..", "a/b", r"a\b", "C:"] {
            let name = folder_name(evil);
            assert!(!name.is_empty() && name != ".." && name != "." && !name.contains(['/', '\\', ':']), "{evil:?} -> {name:?}");
        }
    }

    fn manifest() -> Manifest {
        serde_json::from_str(
            r#"{"format":1,"builder":"t","app":{"id":"Hello App","name":"Hello","version":"1.2.0","publisher":"Hello Inc.",
            "description":"Says \"hi\".","website":"https://hello.example","executable":"hello.exe"},
            "install":{"folder":"Hello","desktop_shortcut":true},"uninstall":{"data":[],"data_description":null},
            "look":{"theme":"system","accent":null,"logo":null},"files":[{"path":"hello.exe","size":1}]}"#,
        )
        .unwrap()
    }

    #[test]
    fn manifests_name_the_package_and_installer() {
        let manifest = manifest();
        let id = package_id(&manifest);
        assert_eq!(id, "HelloInc.HelloApp");
        let files = manifests(&manifest, &id, "https://hello.example/Setup.exe", "AB", Some("MIT"), "x64");
        assert_eq!(files.len(), 3);
        let installer = &files[1].1;
        assert!(installer.contains("Silent: --silent") && installer.contains("Scope: user"));
        let mut machine = manifest.clone();
        machine.install.scope = bach_pack::manifest::Scope::Machine;
        let files = manifests(&machine, &id, "https://hello.example/Setup.exe", "AB", None, "arm64");
        assert!(files[1].1.contains("Scope: machine"), "a machine-wide setup file says so");
        assert!(files[1].1.contains("Architecture: arm64"));
        // The machine field of a real PE header: this very test program.
        let exe = std::fs::read(std::env::current_exe().unwrap()).unwrap();
        assert_eq!(architecture(&exe), if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" });
        assert_eq!(architecture(b"not a program"), "x64");
        assert!(installer.contains("ProductCode: \"Hello App\""));
        assert!(files[2].1.contains("ShortDescription: \"Says \\\"hi\\\".\""));
    }
}
