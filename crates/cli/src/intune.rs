//! `bach intune`: what to type into Microsoft Intune (or any tool that
//! deploys a Win32 app) for a setup file, and a detection script that says
//! whether it is installed.
//!
//! Intune needs three things for a setup file that isn't an MSI: a silent
//! install command, an uninstall command, and a rule that tells it the app
//! is there. Bach already has all three (`--silent`, `uninstall.exe --silent`
//! and the Settings › Apps entry named by the app id); this command only
//! writes them down, so nobody has to dig the values out of the docs.

use std::path::Path;

use anyhow::{Context, Result};
use bach_pack::manifest::Manifest;

use crate::choco::ps;
use crate::output::{self, field};

pub struct Options<'a> {
    pub setup: &'a Path,
    pub out: Option<&'a Path>,
}

/// Everything `bach intune` says, worked out from the setup file.
#[derive(Debug, PartialEq)]
struct Plan {
    name: String,
    id: String,
    version: String,
    setup_file: String,
    /// Installs for every user: Intune runs it as the system account.
    machine: bool,
    install: String,
    uninstall: String,
    /// `HKEY_LOCAL_MACHINE\...` for a machine-wide app; per-user apps are
    /// detected by the script, because the rule can't be sure to read `HKCU`.
    registry_key: Option<String>,
    /// Whether the version is plain numbers, which Intune's "Version
    /// comparison" can read. `2.0.0-beta.1` it can't.
    plain_version: bool,
    architecture: &'static str,
}

const UNINSTALL_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";

impl Plan {
    fn new(manifest: &Manifest, setup_file: &str, architecture: &'static str) -> Self {
        let app = &manifest.app;
        let machine = manifest.install.scope.is_machine();
        let folder = &manifest.install.folder;
        let uninstaller = if machine {
            format!(r"%ProgramFiles%\{folder}\{}", bach_pack::manifest::UNINSTALLER)
        } else {
            format!(r"%LOCALAPPDATA%\Programs\{folder}\{}", bach_pack::manifest::UNINSTALLER)
        };
        Self {
            name: app.name.clone(),
            id: app.id.clone(),
            version: app.version.clone(),
            setup_file: setup_file.to_string(),
            machine,
            install: format!("{} --silent", command_word(setup_file)),
            uninstall: format!("\"{uninstaller}\" --silent"),
            registry_key: machine.then(|| format!(r"HKEY_LOCAL_MACHINE\{UNINSTALL_KEY}\{}", app.id)),
            plain_version: !app.version.is_empty()
                && app.version.chars().all(|c| c.is_ascii_digit() || c == '.')
                && !app.version.starts_with('.')
                && !app.version.ends_with('.')
                && !app.version.contains(".."),
            architecture,
        }
    }

    fn run_as(&self) -> &'static str {
        if self.machine {
            "System"
        } else {
            "User"
        }
    }

    /// Intune's wording for the architecture requirement.
    fn architecture_label(&self) -> &'static str {
        match self.architecture {
            "arm64" => "ARM64",
            "x86" => "32-bit",
            _ => "64-bit (x64)",
        }
    }

    /// The settings sheet, as Markdown.
    fn sheet(&self) -> String {
        let mut text = format!("# {} {} in Microsoft Intune\n\n", self.name, self.version);
        text.push_str("Written by `bach intune`. These are the values to use when you add the setup file as a Windows app (Win32).\n\n");
        text.push_str("## Wrap the setup file\n\n");
        text.push_str(&format!(
            "Copy `{}` into an empty folder and run Microsoft's Win32 Content Prep Tool on it:\n\n```powershell\nIntuneWinAppUtil.exe -c <that folder> -s \"{}\" -o <output folder>\n```\n\nUpload the `.intunewin` file it writes.\n\n",
            self.setup_file, self.setup_file
        ));
        text.push_str("## Program\n\n| Setting | Value |\n| --- | --- |\n");
        text.push_str(&format!("| Install command | `{}` |\n", self.install));
        text.push_str(&format!("| Uninstall command | `{}` |\n", self.uninstall));
        text.push_str(&format!("| Install behavior | {} |\n", self.run_as()));
        text.push_str("| Device restart behavior | No specific action |\n");
        text.push_str("| Return codes | The defaults: `0` is success, `1` is a failure (the log in `%TEMP%` says why) |\n\n");
        text.push_str("## Requirements\n\n| Setting | Value |\n| --- | --- |\n");
        text.push_str(&format!("| Operating system architecture | {} |\n", self.architecture_label()));
        text.push_str("| Minimum operating system | Windows 10 1809 |\n\n");
        text.push_str("## Detection rules\n\n");
        match (&self.registry_key, self.plain_version) {
            (Some(key), true) => text.push_str(&format!(
                "Use **one** of these. The rule needs no script:\n\n| Setting | Value |\n| --- | --- |\n| Rule type | Registry |\n| Key path | `{key}` |\n| Value name | `DisplayVersion` |\n| Detection method | Version comparison |\n| Operator | Greater than or equal to |\n| Value | `{}` |\n| Associated with a 32-bit app on 64-bit clients | No |\n\nOr use `detect.ps1` (rule type: Use a custom detection script). Leave \"Run script as 32-bit process\" off.\n",
                self.version
            )),
            (Some(_), false) => text.push_str(&format!(
                "Use `detect.ps1` (rule type: Use a custom detection script, with \"Run script as 32-bit process\" off). Intune's registry rule can't compare `{}`, because it isn't made of plain numbers, and the script understands it.\n",
                self.version
            )),
            (None, _) => text.push_str(
                "Use `detect.ps1` (rule type: Use a custom detection script, with \"Run script as 32-bit process\" off). It reads the per-user entry under `HKEY_CURRENT_USER`, so run it as the signed-in user.\n",
            ),
        }
        text.push_str(
            "\nThe script says the app is installed when this version **or a newer one** is there. Bach updates an app in place, so a newer version must not make Intune install the old one again (setup refuses a downgrade anyway).\n",
        );
        text.push_str("\n## Notes\n\n");
        if self.machine {
            text.push_str(
                "- The app installs for every user, so Intune runs setup as the system account and nothing asks for permission.\n",
            );
        } else {
            text.push_str("- The app installs for one user, into that user's profile. Deploy it with install behavior **User**, to users rather than devices.\n");
        }
        text.push_str(&format!(
            "- If you install somewhere else with `--dir`, the uninstall command has to point there too: the entry named `{}` under Settings › Apps (`UninstallString` in the registry) says where.\n",
            self.id
        ));
        text.push_str("- Use the same app id and the same install scope in every release, or Windows counts it as another app.\n");
        text
    }

    /// The detection script: exit code 0 and some output mean "installed".
    fn script(&self) -> String {
        DETECT
            .replace("@NAME@", &one_line(&self.name))
            .replace("@VERSION_TEXT@", &one_line(&self.version))
            .replace("@ID@", &ps(&self.id))
            .replace("@WANTED@", &ps(&self.version))
            .replace("@HIVE@", if self.machine { "LocalMachine" } else { "CurrentUser" })
    }
}

/// Text for a one-line comment: PowerShell ends a comment at any line break,
/// and Unicode has more of them than `\n`.
fn one_line(text: &str) -> String {
    text.chars().map(|c| if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') { ' ' } else { c }).collect()
}

/// A file name as the first word of a command line: in quotes when it has a space.
fn command_word(file: &str) -> String {
    if file.contains(' ') {
        format!("\"{file}\"")
    } else {
        file.to_string()
    }
}

/// The detection script. `Split-Version` and `Compare-Version` follow
/// `bach_pack::version`, so the script and setup agree on what is newer.
const DETECT: &str = r#"# Detection script for @NAME@ @VERSION_TEXT@, written by `bach intune`.
# Intune counts the app as installed when this exits with 0 and prints something.
# "Installed" means this version or a newer one: Bach updates apps in place, and
# a newer version must not make Intune install the old one again.
$ErrorActionPreference = 'Stop'
$id = @ID@
$wanted = @WANTED@

# 1.2.10-beta.1+build -> numbers 1, 2, 10 and the pre-release "beta.1".
function Split-Version([string]$text) {
    $text = $text.Trim()
    if ($text -match '^[vV]') { $text = $text.Substring(1) }
    $text = $text.Split('+')[0]
    $pre = $null
    $dash = $text.IndexOf('-')
    if ($dash -ge 0) { $pre = $text.Substring($dash + 1); $text = $text.Substring(0, $dash) }
    $numbers = @()
    foreach ($part in $text.Split('.')) {
        $number = [int64]0
        if (-not [int64]::TryParse($part, [ref]$number)) { return $null }
        $numbers += $number
    }
    return @{ Numbers = $numbers; Pre = $pre }
}

# -1, 0 or 1 like a comparison; $null when either text isn't a version.
function Compare-Version([string]$a, [string]$b) {
    $left = Split-Version $a
    $right = Split-Version $b
    if ($null -eq $left -or $null -eq $right) { return $null }
    $length = [Math]::Max($left.Numbers.Count, $right.Numbers.Count)
    for ($i = 0; $i -lt $length; $i++) {
        $x = [int64]0; if ($i -lt $left.Numbers.Count) { $x = $left.Numbers[$i] }
        $y = [int64]0; if ($i -lt $right.Numbers.Count) { $y = $right.Numbers[$i] }
        if ($x -lt $y) { return -1 }
        if ($x -gt $y) { return 1 }
    }
    # A pre-release sorts before the release: 2.0.0-beta.2 < 2.0.0.
    if ($null -eq $left.Pre -and $null -eq $right.Pre) { return 0 }
    if ($null -eq $left.Pre) { return 1 }
    if ($null -eq $right.Pre) { return -1 }
    $l = $left.Pre.Split('.')
    $r = $right.Pre.Split('.')
    $count = [Math]::Max($l.Count, $r.Count)
    for ($i = 0; $i -lt $count; $i++) {
        if ($i -ge $l.Count) { return -1 }
        if ($i -ge $r.Count) { return 1 }
        $x = [int64]0; $y = [int64]0
        $xNumber = [int64]::TryParse($l[$i], [ref]$x)
        $yNumber = [int64]::TryParse($r[$i], [ref]$y)
        if ($xNumber -and $yNumber) {
            if ($x -lt $y) { return -1 }
            if ($x -gt $y) { return 1 }
        } elseif ($xNumber) { return -1 } elseif ($yNumber) { return 1 } else {
            $order = [string]::CompareOrdinal($l[$i], $r[$i])
            if ($order -lt 0) { return -1 }
            if ($order -gt 0) { return 1 }
        }
    }
    return 0
}

# The Settings > Apps entry Bach writes, in the 64-bit view whatever this process is.
try {
    $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::@HIVE@, [Microsoft.Win32.RegistryView]::Registry64)
    $key = $base.OpenSubKey('Software\Microsoft\Windows\CurrentVersion\Uninstall\' + $id)
    $installed = $null
    if ($key) { $installed = [string]$key.GetValue('DisplayVersion') }
} catch {
    exit 1
}
if (-not $installed) { exit 1 }

$order = Compare-Version $installed $wanted
if ($null -eq $order) { $found = ($installed -eq $wanted) } else { $found = ($order -ge 0) }
if ($found) {
    Write-Output "$id $installed is installed"
    exit 0
}
exit 1
"#;

pub fn command(options: Options) -> Result<()> {
    let manifest = crate::winget::read_full_setup(options.setup, "intune")?;
    let architecture = crate::winget::setup_architecture(options.setup)?;
    let file_name = options.setup.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_else(|| "Setup.exe".into());
    let plan = Plan::new(&manifest, &file_name, architecture);
    let dir = match options.out {
        Some(out) => out.to_path_buf(),
        None => options.setup.parent().unwrap_or(Path::new(".")).join("intune").join(plan.id.replace(' ', "-")),
    };
    std::fs::create_dir_all(&dir).with_context(|| format!("couldn't create {}", dir.display()))?;
    let script = dir.join("detect.ps1");
    let sheet = dir.join("intune.md");
    // A BOM keeps Windows PowerShell 5.1 from reading a name or version as ANSI.
    std::fs::write(&script, format!("\u{feff}{}", plan.script().replace('\n', "\r\n")))
        .with_context(|| format!("couldn't write {}", script.display()))?;
    std::fs::write(&sheet, plan.sheet()).with_context(|| format!("couldn't write {}", sheet.display()))?;

    output::result("app_id", plan.id.clone());
    output::result("name", plan.name.clone());
    output::result("version", plan.version.clone());
    output::result("install_command", plan.install.clone());
    output::result("uninstall_command", plan.uninstall.clone());
    output::result("run_as", plan.run_as().to_lowercase());
    if let Some(key) = &plan.registry_key {
        output::result("detection_key", key.clone());
    }
    output::result("detection_script", script.display().to_string());

    say!("Wrote the Intune settings for {} {} to {}", plan.name, plan.version, dir.display());
    field("Install", &plan.install);
    field("Uninstall", &plan.uninstall);
    field("Run as", format!("{} ({})", plan.run_as(), if plan.machine { "installs for every user" } else { "installs for one user" }));
    match &plan.registry_key {
        Some(key) if plan.plain_version => field("Detect", format!("registry {key}, DisplayVersion >= {}", plan.version)),
        _ => field("Detect", "the script detect.ps1"),
    }
    field("Requires", format!("{}, Windows 10 1809 or newer", plan.architecture_label()));
    say!("Open {} for the steps, from wrapping the file to the detection rule.", sheet.display());

    let mut warnings = Vec::new();
    if manifest.access.is_some() {
        warnings.push(
            "this setup file asks for an access key; add --key-file FILE to the install command (and put the key file in the package)"
                .to_string(),
        );
    }
    if !plan.machine {
        warnings.push(
            "this app installs per user; deploy it with install behavior User. For devices, set installer.scope = \"machine\"".to_string(),
        );
    }
    output::warnings(&warnings);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(scope: &str, id: &str, version: &str) -> Manifest {
        serde_json::from_str(&format!(
            r#"{{"format":1,"builder":"t","app":{{"id":"{id}","name":"Hello","version":"{version}","publisher":"Hello Inc.","executable":"hello.exe"}},
            "install":{{"folder":"Hello","scope":"{scope}","desktop_shortcut":"never"}},"uninstall":{{"data":[],"data_description":null}},
            "look":{{"theme":"system","accent":null,"logo":null}},"files":[{{"path":"hello.exe","size":1}}]}}"#
        ))
        .unwrap()
    }

    #[test]
    fn a_machine_wide_app_runs_as_system_and_is_found_in_hklm() {
        let plan = Plan::new(&manifest("machine", "Hello", "1.4.0"), "Hello-Setup-1.4.0.exe", "x64");
        assert_eq!(plan.install, "Hello-Setup-1.4.0.exe --silent");
        assert_eq!(plan.uninstall, r#""%ProgramFiles%\Hello\uninstall.exe" --silent"#);
        assert_eq!(plan.run_as(), "System");
        assert_eq!(plan.registry_key.as_deref(), Some(r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Hello"));
        assert!(plan.plain_version);
        let sheet = plan.sheet();
        assert!(sheet.contains("| Operating system architecture | 64-bit (x64) |"), "{sheet}");
        assert!(sheet.contains("| Value | `1.4.0` |"), "{sheet}");
        assert!(sheet.contains("IntuneWinAppUtil.exe"), "{sheet}");
        assert!(plan.script().contains("OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine"));
    }

    #[test]
    fn a_per_user_app_is_detected_by_the_script_only() {
        let plan = Plan::new(&manifest("user", "Hello", "1.4.0"), "Hello Setup.exe", "arm64");
        assert_eq!(plan.install, r#""Hello Setup.exe" --silent"#, "a name with a space is quoted");
        assert_eq!(plan.uninstall, r#""%LOCALAPPDATA%\Programs\Hello\uninstall.exe" --silent"#);
        assert_eq!(plan.run_as(), "User");
        assert_eq!(plan.registry_key, None);
        assert_eq!(plan.architecture_label(), "ARM64");
        let sheet = plan.sheet();
        assert!(sheet.contains("install behavior **User**"), "{sheet}");
        assert!(!sheet.contains("| Rule type | Registry |"), "{sheet}");
        assert!(plan.script().contains("RegistryHive]::CurrentUser"));
    }

    #[test]
    fn a_version_with_a_suffix_gets_no_registry_rule() {
        let plan = Plan::new(&manifest("machine", "Hello", "2.0.0-beta.1"), "S.exe", "x64");
        assert!(!plan.plain_version);
        let sheet = plan.sheet();
        assert!(sheet.contains("can't compare `2.0.0-beta.1`"), "{sheet}");
        assert!(!sheet.contains("| Rule type | Registry |"), "{sheet}");
    }

    /// The id and version go into a PowerShell script: they are single-quoted
    /// there and a quote in them is doubled, so nothing in them is ever run.
    #[test]
    fn the_script_never_runs_what_the_id_or_version_say() {
        let plan = Plan::new(&manifest("machine", "It's $(calc) `x`", "1.0'; calc; '"), "S.exe", "x64");
        let script = plan.script();
        assert!(script.contains("$id = 'It''s $(calc) `x`'"), "{script}");
        assert!(script.contains("$wanted = '1.0''; calc; '''"), "{script}");
        assert_no_placeholders(&script);
    }

    fn assert_no_placeholders(script: &str) {
        for placeholder in ["@NAME@", "@VERSION_TEXT@", "@ID@", "@WANTED@", "@HIVE@"] {
            assert!(!script.contains(placeholder), "{placeholder} is still in the script: {script}");
        }
    }

    /// A name can't start a new line in the script's comment, and PowerShell's
    /// curly quotes can't close a quoted string early.
    #[test]
    fn line_breaks_and_curly_quotes_stay_where_they_belong() {
        let mut odd = Plan::new(&manifest("machine", "Hello", "1.0.0"), "S.exe", "x64");
        odd.name = "Evil\u{2028}Remove-Item C:\\ -Recurse\nnext".into();
        let script = odd.script();
        assert!(!script.contains('\u{2028}'));
        assert!(script.lines().next().unwrap().contains("Evil Remove-Item"), "{script}");
        assert!(script.lines().nth(1).unwrap().starts_with("# Intune counts"), "{script}");
        let mut curly = Plan::new(&manifest("machine", "Hello", "1.0.0"), "S.exe", "x64");
        curly.version = "1.0\u{2019}; calc; \u{2018}".into();
        assert!(curly.script().contains("$wanted = '1.0''; calc; '''"), "{}", curly.script());
    }

    #[test]
    fn the_script_has_no_placeholders_left() {
        for scope in ["user", "machine"] {
            assert_no_placeholders(&Plan::new(&manifest(scope, "Hello", "1.0.0"), "S.exe", "x64").script());
        }
    }
}
