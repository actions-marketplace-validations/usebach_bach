//! `bach choco`: the Chocolatey package that installs a setup file. A
//! `.nuspec`, `tools/chocolateyInstall.ps1` (which downloads the setup file
//! and checks its SHA-256 before running it with `--silent`) and
//! `tools/chocolateyUninstall.ps1` (which runs the app's own uninstaller from
//! its Settings › Apps entry).
//!
//! Chocolatey runs elevated: a per-user app (the default) is installed for the
//! user who ran `choco`, so Chocolatey suits apps with `installer.scope =
//! "machine"` best, and the command says so for the others.

use std::path::Path;

use anyhow::{bail, Context, Result};
use bach_pack::manifest::Manifest;

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
    let manifest = crate::winget::read_full_setup(options.setup, "choco")?;
    let (sha256, _) = crate::feed::digest_of(options.setup)?;
    let architecture = crate::winget::setup_architecture(options.setup)?;
    let id = package_id(&manifest);
    let version = &manifest.app.version;
    // The version comes from the setup file and becomes a folder name, and NuGet
    // refuses anything else anyway: stop before a path is made from it.
    if !valid_version(version) {
        bail!("the version \"{version}\" isn't one NuGet accepts (like 1.2.3 or 1.2.3-beta), so there is no Chocolatey package for it");
    }
    let dir = match options.out {
        Some(out) => out.to_path_buf(),
        None => options.setup.parent().unwrap_or(Path::new(".")).join("choco").join(&id).join(version),
    };
    let tools = dir.join("tools");
    std::fs::create_dir_all(&tools).with_context(|| format!("couldn't create {}", tools.display()))?;
    let write =
        |path: std::path::PathBuf, text: String| std::fs::write(&path, text).with_context(|| format!("couldn't write {}", path.display()));
    write(dir.join(format!("{id}.nuspec")), nuspec(&manifest, &id, options.license))?;
    write(tools.join("chocolateyInstall.ps1"), install_script(&id, options.url, &sha256, architecture))?;
    write(tools.join("chocolateyUninstall.ps1"), uninstall_script(&manifest.app.id))?;

    say!("Wrote the Chocolatey package for {id} {version} to {}", dir.display());
    let mut warnings = Vec::new();
    if !manifest.install.scope.is_machine() {
        warnings.push(
            "Chocolatey installs as an administrator, so this per-user app would be installed for that account only; \
             set installer.scope = \"machine\" to install for every user"
                .to_string(),
        );
    }
    if options.license.is_none() {
        warnings.push("the package has no license; pass --license with yours (a link, or a name like MIT)".to_string());
    }
    if architecture == "arm64" {
        warnings.push("Chocolatey has no ARM64 download field; the link is listed as the 64-bit one".to_string());
    }
    crate::output::warnings(&warnings);
    say!("Build it with `choco pack` in that folder, then test it with `choco install {id} --source .`.");
    Ok(())
}

/// A Chocolatey package id: lowercase letters, digits, `.` and `-`, made from
/// the app id (which names the Settings › Apps entry and doesn't change).
fn package_id(manifest: &Manifest) -> String {
    let mut id = String::new();
    for c in manifest.app.id.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() || c == '.' {
            id.push(c);
        } else if !id.ends_with('-') {
            id.push('-');
        }
    }
    let id = id.trim_matches(|c| c == '-' || c == '.').to_string();
    if id.is_empty() {
        "app".into()
    } else {
        id
    }
}

/// What NuGet takes as a version: one to four numbers, then an optional `-label`.
fn valid_version(version: &str) -> bool {
    let (numbers, label) = match version.split_once('-') {
        Some((numbers, label)) => (numbers, Some(label)),
        None => (version, None),
    };
    let parts: Vec<&str> = numbers.split('.').collect();
    (1..=4).contains(&parts.len())
        && parts.iter().all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
        && label.is_none_or(|label| !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'))
}

/// Text for an XML element.
fn xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// A PowerShell string in single quotes: nothing in it is ever run. PowerShell
/// reads the curly single quotes (and the low and reversed ones) as quotes too,
/// so each of them is doubled the same way, which makes it a plain `'`.
pub(crate) fn ps(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('\'');
    for c in text.chars() {
        match c {
            '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => quoted.push_str("''"),
            other => quoted.push(other),
        }
    }
    quoted.push('\'');
    quoted
}

fn nuspec(manifest: &Manifest, id: &str, license: Option<&str>) -> String {
    let app = &manifest.app;
    let description = app.description.clone().unwrap_or_else(|| app.name.clone());
    let mut optional = String::new();
    if let Some(site) = &app.website {
        optional.push_str(&format!("    <projectUrl>{}</projectUrl>\n", xml(site)));
    }
    if let Some(license) = license {
        let url = if bach_pack::manifest::extras::is_https(license) {
            license.to_string()
        } else {
            format!("https://spdx.org/licenses/{license}.html")
        };
        optional.push_str(&format!("    <licenseUrl>{}</licenseUrl>\n", xml(&url)));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!-- Created with Bach (bach choco). -->\n\
         <package xmlns=\"http://schemas.microsoft.com/packaging/2015/06/nuspec.xsd\">\n  <metadata>\n    <id>{id}</id>\n    <version>{}</version>\n    \
         <title>{}</title>\n    <authors>{}</authors>\n{optional}    <requireLicenseAcceptance>false</requireLicenseAcceptance>\n    \
         <summary>{}</summary>\n    <description>{}</description>\n  </metadata>\n  <files>\n    <file src=\"tools\\**\" target=\"tools\" />\n  </files>\n</package>\n",
        xml(&app.version),
        xml(&app.name),
        xml(&app.publisher),
        xml(&description),
        xml(&description)
    )
}

fn install_script(id: &str, url: &str, sha256: &str, architecture: &str) -> String {
    // Chocolatey has a 32-bit and a 64-bit link; everything else is listed as the 64-bit one.
    let (link, sum, sum_type) =
        if architecture == "x86" { ("url", "checksum", "checksumType") } else { ("url64bit", "checksum64", "checksumType64") };
    format!(
        "# Created with Bach (bach choco). Downloads the setup file, checks its SHA-256 and runs it silently.\n\
         $ErrorActionPreference = 'Stop'\n\n\
         $packageArgs = @{{\n  packageName    = {}\n  fileType       = 'exe'\n  {link:<14} = {}\n  {sum:<14} = {}\n  {sum_type:<14} = 'sha256'\n  silentArgs     = '--silent'\n  validExitCodes = @(0)\n}}\n\n\
         Install-ChocolateyPackage @packageArgs\n",
        ps(id),
        ps(url),
        ps(&sha256.to_ascii_lowercase()),
    )
}

fn uninstall_script(app_id: &str) -> String {
    format!(
        "# Created with Bach (bach choco). Runs the app's own uninstaller, named in Settings > Apps.\n\
         $ErrorActionPreference = 'Stop'\n$id = {}\n$found = $false\n\n\
         foreach ($root in 'HKLM:', 'HKCU:') {{\n  $key = \"$root\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\$id\"\n  if (-not (Test-Path -LiteralPath $key)) {{ continue }}\n  \
         $uninstall = (Get-ItemProperty -LiteralPath $key).UninstallString\n  if (-not $uninstall) {{ continue }}\n  \
         $process = Start-Process -FilePath $uninstall.Trim('\"') -ArgumentList '--silent' -Wait -PassThru\n  \
         if ($process.ExitCode -ne 0) {{ throw \"The uninstaller of $id exited with code $($process.ExitCode).\" }}\n  $found = $true\n}}\n\n\
         if (-not $found) {{ Write-Warning \"$id isn't listed under Settings > Apps, so there is nothing to remove.\" }}\n",
        ps(app_id)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        serde_json::from_str(
            r#"{"format":1,"builder":"t","app":{"id":"Hello App","name":"Hello & <Co>","version":"1.2.0","publisher":"Hello Inc.",
            "description":"Says \"hi\".","website":"https://hello.example/?a=1&b=2","executable":"hello.exe"},
            "install":{"folder":"Hello","desktop_shortcut":true},"uninstall":{"data":[],"data_description":null},
            "look":{"theme":"system","accent":null,"logo":null},"files":[{"path":"hello.exe","size":1}]}"#,
        )
        .unwrap()
    }

    #[test]
    fn ids_follow_chocolatey_and_versions_follow_nuget() {
        assert_eq!(package_id(&manifest()), "hello-app");
        let mut odd = manifest();
        odd.app.id = "  --My_App!! v2 ".into();
        assert_eq!(package_id(&odd), "my-app-v2");
        odd.app.id = "***".into();
        assert_eq!(package_id(&odd), "app");
        for good in ["1", "1.2", "1.2.3.4", "1.2.3-beta.1", "2.0.0-rc-1"] {
            assert!(valid_version(good), "{good}");
        }
        for bad in ["", "1.2.3.4.5", "v1.2", "1..2", "1.2.3-", "1.2 3", "1.2.3+build"] {
            assert!(!valid_version(bad), "{bad}");
        }
    }

    #[test]
    fn a_version_that_could_leave_the_folder_is_not_a_version() {
        for evil in [r"..\..\x", "../x", "1.2/../..", r"C:\x", r"1.0.0\.."] {
            assert!(!valid_version(evil), "{evil}");
        }
    }

    #[test]
    fn the_nuspec_is_escaped() {
        let text = nuspec(&manifest(), "hello-app", Some("MIT"));
        assert!(text.contains("<title>Hello &amp; &lt;Co&gt;</title>"), "{text}");
        assert!(text.contains("<projectUrl>https://hello.example/?a=1&amp;b=2</projectUrl>"));
        assert!(text.contains("<licenseUrl>https://spdx.org/licenses/MIT.html</licenseUrl>"));
        assert!(text.contains("<summary>Says &quot;hi&quot;.</summary>"));
        let linked = nuspec(&manifest(), "hello-app", Some("https://hello.example/license"));
        assert!(linked.contains("<licenseUrl>https://hello.example/license</licenseUrl>"));
    }

    /// PowerShell reads curly, low and reversed single quotes as quotes too, so
    /// one of them in a link can't end the string early.
    #[test]
    fn powershell_strings_hold_every_kind_of_single_quote() {
        assert_eq!(ps("plain"), "'plain'");
        assert_eq!(ps("it's"), "'it''s'");
        for quote in ['\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}'] {
            assert_eq!(ps(&format!("a{quote}; calc; {quote}")), "'a''; calc; '''", "{quote:?}");
        }
        assert_eq!(ps("$(calc) `x` \"y\""), "'$(calc) `x` \"y\"'", "nothing else is special inside single quotes");
    }

    #[test]
    fn the_install_script_checks_the_download() {
        let sha = "AB".repeat(32);
        // Alignment spaces don't matter.
        let squash = |text: String| text.split_whitespace().collect::<Vec<_>>().join(" ");
        let script = squash(install_script("hello-app", "https://hello.example/it's.exe", &sha, "x64"));
        assert!(script.contains("url64bit = 'https://hello.example/it''s.exe'"), "{script}");
        assert!(script.contains(&format!("checksum64 = '{}'", sha.to_lowercase())), "{script}");
        assert!(script.contains("checksumType64 = 'sha256'") && script.contains("silentArgs = '--silent'"), "{script}");
        let x86 = squash(install_script("hello-app", "https://hello.example/x.exe", &sha, "x86"));
        assert!(
            x86.contains("url = ") && x86.contains("checksum = ") && x86.contains("checksumType = 'sha256'") && !x86.contains("url64bit"),
            "{x86}"
        );
    }

    #[test]
    fn the_uninstall_script_names_the_app_by_its_id_and_never_runs_it_as_code() {
        let script = uninstall_script("it's $(evil)");
        assert!(script.contains("$id = 'it''s $(evil)'"), "{script}");
        assert!(script.contains("Uninstall\\$id") && script.contains("'--silent'"));
    }
}
