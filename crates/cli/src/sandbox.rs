//! `bach test`: tries a setup file on a clean Windows, in Windows Sandbox.
//!
//! The sandbox starts from a fresh copy of Windows every time, so the test
//! needs nothing set up and leaves nothing on this computer. Bach prepares a
//! folder with the setup file, a PowerShell script (`sandbox/guest.ps1`) and
//! the facts the script checks against (`params.json`), a second folder for the
//! answer, and a `.wsb` file that maps both into the sandbox and starts the
//! script when it logs on. The script installs the app silently, checks what the
//! install did, starts the app, removes it and writes `report.json`; Bach reads
//! that back and prints it. Nothing the setup file says is put into the script's
//! text: it reads `params.json` as data.
//!
//! `--dry-run` writes the folder and the `.wsb` and stops, which also works on
//! a computer without Windows Sandbox (Windows 10/11 Pro, Enterprise and
//! Education have it; Home doesn't).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use bach_pack::container;
use bach_pack::manifest::{Choices, Manifest};
use serde_json::{json, Value};

use crate::output::{self, field};

const GUEST: &str = include_str!("sandbox/guest.ps1");

/// Where the folders appear inside the sandbox.
const GUEST_ROOT: &str = r"C:\bach-test";

pub struct Options<'a> {
    pub file: Option<&'a Path>,
    pub config: &'a Path,
    pub with: &'a [String],
    pub without: &'a [String],
    /// Seconds to leave the app running; 0 doesn't start it.
    pub launch_seconds: u32,
    pub network: bool,
    pub timeout_minutes: u32,
    /// Leave the sandbox open after the test, to look around in it.
    pub keep: bool,
    /// Write the folder and the `.wsb` and stop.
    pub dry_run: bool,
    /// Run the test on this computer instead of in Windows Sandbox (`--here`):
    /// for a CI runner or a virtual machine that is thrown away anyway.
    pub here: bool,
}

/// Everything the guest script is told, as `params.json`.
pub fn params(manifest: &Manifest, choices: &Choices, options: &Options, setup_name: &str) -> Value {
    let files: Vec<Value> =
        manifest.files.iter().filter(|file| choices.installs(file)).map(|file| json!({"path": file.path, "size": file.size})).collect();
    json!({
        "setup": setup_name,
        "id": manifest.app.id,
        "name": manifest.app.name,
        "version": manifest.app.version,
        "executable": manifest.app.executable,
        "online": manifest.online,
        "files": if manifest.online { Vec::new() } else { files },
        "with": options.with,
        "without": options.without,
        "launch_seconds": options.launch_seconds,
        "keep": options.keep,
    })
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// The `.wsb` file: both folders mapped, the script started at logon, and the
/// things a test doesn't need (sound, camera, clipboard, printers) switched off.
pub fn wsb(input: &Path, output: &Path, network: bool) -> String {
    let command = format!(r"powershell.exe -NoProfile -ExecutionPolicy Bypass -File {GUEST_ROOT}\in\run.ps1");
    format!(
        "<Configuration>\n\
         \x20 <Networking>{}</Networking>\n\
         \x20 <AudioInput>Disable</AudioInput>\n\
         \x20 <VideoInput>Disable</VideoInput>\n\
         \x20 <PrinterRedirection>Disable</PrinterRedirection>\n\
         \x20 <ClipboardRedirection>Disable</ClipboardRedirection>\n\
         \x20 <MappedFolders>\n\
         \x20   <MappedFolder>\n\
         \x20     <HostFolder>{}</HostFolder>\n\
         \x20     <SandboxFolder>{GUEST_ROOT}\\in</SandboxFolder>\n\
         \x20     <ReadOnly>true</ReadOnly>\n\
         \x20   </MappedFolder>\n\
         \x20   <MappedFolder>\n\
         \x20     <HostFolder>{}</HostFolder>\n\
         \x20     <SandboxFolder>{GUEST_ROOT}\\out</SandboxFolder>\n\
         \x20     <ReadOnly>false</ReadOnly>\n\
         \x20   </MappedFolder>\n\
         \x20 </MappedFolders>\n\
         \x20 <LogonCommand>\n\
         \x20   <Command>{}</Command>\n\
         \x20 </LogonCommand>\n\
         </Configuration>\n",
        if network { "Default" } else { "Disable" },
        xml(&input.display().to_string()),
        xml(&output.display().to_string()),
        xml(&command),
    )
}

/// The setup file to test: the one named, or the one `bach.toml` builds.
fn setup_file(options: &Options) -> Result<PathBuf> {
    if let Some(file) = options.file {
        return Ok(file.to_path_buf());
    }
    let project = bach_pack::config::load(options.config)?;
    let file = project.output.clone();
    if !file.is_file() {
        bail!("{} doesn't exist yet: run `bach build` first, or name the setup file: bach test FILE", file.display());
    }
    Ok(file)
}

/// One line of the report.
#[derive(Debug, PartialEq, Eq)]
pub struct Line {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

/// What the guest script found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub lines: Vec<Line>,
    pub files: Vec<String>,
    pub registry: Vec<String>,
    pub other: Vec<String>,
}

impl Report {
    pub fn passed(&self) -> bool {
        !self.lines.is_empty() && self.lines.iter().all(|line| line.ok)
    }
}

/// A list the way PowerShell writes it to JSON: an array, or, when it has one
/// item, the item itself; a missing one is empty.
fn list(value: Option<&Value>) -> Vec<&Value> {
    match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.iter().collect(),
        Some(item) => vec![item],
    }
}

fn texts(value: Option<&Value>) -> Vec<String> {
    list(value).into_iter().filter_map(Value::as_str).map(str::to_string).collect()
}

/// Reads `report.json`.
pub fn parse_report(text: &str) -> Result<Report> {
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).context("the sandbox's report isn't readable")?;
    let lines = list(value.get("steps"))
        .into_iter()
        .map(|step| Line {
            name: step.get("name").and_then(Value::as_str).unwrap_or("?").to_string(),
            ok: step.get("ok").and_then(Value::as_bool).unwrap_or(false),
            detail: step.get("detail").and_then(Value::as_str).unwrap_or_default().to_string(),
        })
        .collect();
    let left = value.get("left_behind");
    Ok(Report {
        lines,
        files: texts(left.and_then(|left| left.get("files"))),
        registry: texts(left.and_then(|left| left.get("registry"))),
        other: texts(left.and_then(|left| left.get("other"))),
    })
}

/// The report as lines for a person. Everything in it came out of a sandbox
/// that ran the app, so it is cleaned before it is printed.
pub fn show(report: &Report) {
    for line in &report.lines {
        let mark = if line.ok { "ok  " } else { "FAIL" };
        say!("  {mark}  {:<14} {}", crate::diff::clean(&line.name), crate::diff::clean(&line.detail));
    }
    for (title, items) in [("Files and folders", &report.files), ("Registry keys", &report.registry), ("Other changes", &report.other)] {
        if items.is_empty() {
            continue;
        }
        say!();
        say!("{title} left behind:");
        for item in items.iter().take(40) {
            say!("  {}", crate::diff::clean(item));
        }
        if items.len() > 40 {
            say!("  ... and {} more (all of them are in the JSON report)", items.len() - 40);
        }
    }
}

/// Where Windows keeps the sandbox program.
fn sandbox_program() -> PathBuf {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join("System32").join("WindowsSandbox.exe")
}

pub fn command(options: Options) -> Result<()> {
    let setup = setup_file(&options)?;
    let mut file = fs::File::open(&setup).with_context(|| format!("couldn't open {}", setup.display()))?;
    let Some(layout) = container::locate(&mut file)? else {
        bail!("{} isn't a Bach setup file", setup.display());
    };
    let manifest = container::read_manifest(&mut file, &layout)?;
    if !layout.has_archive() && !manifest.online {
        bail!("{} is an uninstaller, not a setup file; test the setup file the app was installed from", setup.display());
    }
    if manifest.access.is_some() {
        bail!("{} asks for an access key, which a silent test can't type: build a copy without [access] to test", setup.display());
    }
    let mut choices = Choices::defaults(&manifest);
    for (ids, on) in [(options.with, true), (options.without, false)] {
        for id in ids {
            if !choices.set(&manifest, &id.to_ascii_lowercase(), on) {
                bail!("--{} {id}: this setup file offers no such option", if on { "with" } else { "without" });
            }
        }
    }
    if manifest.online && !options.network && !options.here {
        bail!("an online installer downloads the newest version, so the sandbox needs the network: add --network");
    }

    let work = std::env::temp_dir().join(format!("bach-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    let (input, output_dir) = (work.join("in"), work.join("out"));
    fs::create_dir_all(&input)?;
    fs::create_dir_all(&output_dir)?;
    let setup_name = "Setup.exe";
    fs::copy(&setup, input.join(setup_name)).with_context(|| format!("couldn't copy {} for the test", setup.display()))?;
    fs::write(input.join("params.json"), serde_json::to_vec_pretty(&params(&manifest, &choices, &options, setup_name))?)?;
    fs::write(input.join("run.ps1"), GUEST.replace("\r\n", "\n").replace('\n', "\r\n"))?;
    let wsb_file = work.join("bach-test.wsb");
    fs::write(&wsb_file, wsb(&input, &output_dir, options.network))?;

    let app = &manifest.app;
    let place = if options.here { "on this computer" } else { "in Windows Sandbox" };
    say!("Testing {} {} {place}", crate::diff::clean(&app.name), crate::diff::clean(&app.version));
    output::result("name", app.name.clone());
    output::result("version", app.version.clone());
    if options.dry_run {
        say!("Dry run: nothing was started. The sandbox is described in {}", wsb_file.display());
        output::result("sandbox_file", wsb_file.display().to_string());
        return Ok(());
    }
    let result = if options.here {
        run_here(&input, &work, &output_dir, options.timeout_minutes)
    } else {
        run_sandbox(&wsb_file, &output_dir, options.timeout_minutes)
    };
    let report = match result {
        Ok(report) => report,
        Err(err) => {
            let _ = fs::remove_dir_all(&work);
            return Err(err);
        }
    };
    let _ = fs::remove_dir_all(&work);
    show(&report);
    output::result("passed", report.passed());
    output::result(
        "steps",
        report.lines.iter().map(|line| json!({"name": line.name, "ok": line.ok, "detail": line.detail})).collect::<Vec<_>>(),
    );
    output::result("left_behind", json!({"files": report.files, "registry": report.registry, "other": report.other}));
    if report.passed() {
        say!();
        if options.here {
            say!("{} installs, starts and removes cleanly on this computer.", crate::diff::clean(&app.name));
        } else {
            say!("{} installs, starts and removes cleanly on a fresh Windows.", crate::diff::clean(&app.name));
        }
        Ok(())
    } else {
        let failed = report.lines.iter().filter(|line| !line.ok).count();
        bail!("{failed} of {} checks failed", report.lines.len())
    }
}

/// Runs the guest script on this computer (`--here`), where it installs the app
/// for real and removes it again, and waits for the script's report. It refuses
/// on a computer that already has the app (the script checks), so the person's
/// own copy is never the one that gets removed.
fn run_here(input: &Path, work: &Path, output_dir: &Path, timeout_minutes: u32) -> Result<Report> {
    if !cfg!(windows) {
        bail!("bach test --here needs Windows: it installs the setup file on this computer (`--dry-run` writes the files it would use on any computer)");
    }
    let mut child = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(input.join("run.ps1"))
        .arg("-Work")
        .arg(work)
        .arg("-Local")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("couldn't start powershell.exe to run the test")?;
    field("Test", "installing, starting and removing the app on this computer");
    let report = output_dir.join("report.json");
    let started = Instant::now();
    let limit = Duration::from_secs(u64::from(timeout_minutes.max(1)) * 60);
    loop {
        // The script writes the report as its last act, so a report means it is done.
        if report.is_file() {
            let _ = child.wait();
            let text = bach_pack::textfile::read(&report).context("couldn't read the test's report")?;
            return parse_report(&text);
        }
        if let Ok(Some(status)) = child.try_wait() {
            if !report.is_file() {
                bail!("the test script ended with {status} and left no report");
            }
        }
        if started.elapsed() > limit {
            let _ = child.kill();
            bail!("the test didn't finish within {timeout_minutes} minutes (a setup that asks for administrator rights waits for a prompt nobody can answer in a CI job)");
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Starts the sandbox and waits for the script's report.
fn run_sandbox(wsb_file: &Path, output_dir: &Path, timeout_minutes: u32) -> Result<Report> {
    if !cfg!(windows) {
        bail!("bach test needs Windows with Windows Sandbox (`--dry-run` writes the files it would use on any computer)");
    }
    let program = sandbox_program();
    if !program.is_file() {
        bail!(
            "Windows Sandbox isn't installed: {} doesn't exist. It comes with Windows 10 and 11 Pro, Enterprise and Education \
             (Home doesn't have it): turn it on under \"Turn Windows features on or off\", then restart",
            program.display()
        );
    }
    let mut child =
        std::process::Command::new(&program).arg(wsb_file).spawn().with_context(|| format!("couldn't start {}", program.display()))?;
    field("Sandbox", "starting; a fresh Windows takes about a minute");
    let report = output_dir.join("report.json");
    let started = Instant::now();
    let limit = Duration::from_secs(u64::from(timeout_minutes.max(1)) * 60);
    loop {
        if report.is_file() {
            let text = bach_pack::textfile::read(&report).context("couldn't read the sandbox's report")?;
            return parse_report(&text);
        }
        // Windows Sandbox's own program normally lets go of the console once the
        // window is up; one that stops at once with an error never started it.
        if let Ok(Some(status)) = child.try_wait() {
            if !status.success() && started.elapsed() < Duration::from_secs(20) {
                bail!("Windows Sandbox stopped with {status}. Only one can run at a time: close the other one and try again");
            }
        }
        if started.elapsed() > limit {
            bail!("no report came back from the sandbox within {timeout_minutes} minutes; its window is still open, so look there");
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options<'a>(with: &'a [String]) -> Options<'a> {
        Options {
            file: None,
            config: Path::new("bach.toml"),
            with,
            without: &[],
            launch_seconds: 5,
            network: false,
            timeout_minutes: 20,
            keep: false,
            dry_run: false,
            here: false,
        }
    }

    #[test]
    fn the_sandbox_file_maps_two_folders_and_starts_the_script() {
        let text = wsb(Path::new(r"C:\Temp\a&b\in"), Path::new(r"C:\Temp\a&b\out"), false);
        assert!(text.contains("<Networking>Disable</Networking>"));
        assert!(text.contains(r"<HostFolder>C:\Temp\a&amp;b\in</HostFolder>"), "paths are escaped: {text}");
        assert!(text.contains(r"<SandboxFolder>C:\bach-test\in</SandboxFolder>"));
        assert!(text.contains("<ReadOnly>true</ReadOnly>") && text.contains("<ReadOnly>false</ReadOnly>"), "in is read-only, out is not");
        assert!(text.contains(r"-File C:\bach-test\in\run.ps1"));
        assert!(wsb(Path::new("i"), Path::new("o"), true).contains("<Networking>Default</Networking>"));
        assert!(text.contains("<ClipboardRedirection>Disable</ClipboardRedirection>"));
    }

    #[test]
    fn xml_special_characters_cannot_break_out_of_the_file() {
        let text = wsb(Path::new(r#"C:\x"</HostFolder><Command>calc"#), Path::new("o"), false);
        assert!(!text.contains("<Command>calc"), "{text}");
        assert!(text.contains("&lt;Command&gt;calc"));
    }

    #[test]
    fn the_script_is_told_what_to_check_as_data() {
        let mut manifest: Manifest = serde_json::from_str(
            r#"{"format":1,"builder":"0","app":{"id":"A B","name":"A\"; calc; \"","version":"1.0.0","publisher":"P","description":null,"website":null,"executable":"a.exe"},
                "install":{"folder":"A","desktop_shortcut":false},"uninstall":{"data":[],"data_description":null},
                "look":{"theme":"system","accent":null,"logo":null},
                "files":[{"path":"a.exe","size":10},{"path":"extra/b.dat","size":5,"component":"extra"}],
                "components":[{"id":"extra","title":{"*":"Extra"},"default":false}]}"#,
        )
        .unwrap();
        let with = vec!["extra".to_string()];
        let mut choices = Choices::defaults(&manifest);
        let value = params(&manifest, &choices, &options(&with), "Setup.exe");
        assert_eq!(value["files"].as_array().unwrap().len(), 1, "the optional component isn't installed unless asked for");
        assert_eq!(value["name"], "A\"; calc; \"", "kept as data, and the script never builds a command from it");
        choices.set(&manifest, "extra", true);
        assert_eq!(params(&manifest, &choices, &options(&with), "Setup.exe")["files"].as_array().unwrap().len(), 2);
        manifest.online = true;
        assert!(params(&manifest, &choices, &options(&with), "Setup.exe")["files"].as_array().unwrap().is_empty());
    }

    #[test]
    fn the_report_is_read_the_way_powershell_writes_it() {
        // One item: PowerShell writes the item, not an array. A BOM leads the file.
        let text = "\u{feff}{\"version\":1,\"steps\":{\"name\":\"Installed\",\"ok\":true,\"detail\":\"exit code 0\"},\"left_behind\":{\"files\":\"C:\\\\x\",\"registry\":[],\"other\":null}}";
        let report = parse_report(text).unwrap();
        assert_eq!(report.lines, vec![Line { name: "Installed".into(), ok: true, detail: "exit code 0".into() }]);
        assert_eq!(report.files, [r"C:\x"]);
        assert!(report.registry.is_empty() && report.other.is_empty());
        assert!(report.passed());
        assert!(parse_report("not json").is_err());
        assert!(!parse_report("{}").unwrap().passed(), "a report with no checks proves nothing");
    }

    #[test]
    fn a_failed_check_fails_the_report() {
        let text = r#"{"steps":[{"name":"Installed","ok":true,"detail":""},{"name":"Left behind","ok":false,"detail":"1 file"}]}"#;
        assert!(!parse_report(text).unwrap().passed());
    }

    #[test]
    fn the_guest_script_is_plain_ascii_and_reads_its_facts_from_data() {
        assert!(GUEST.is_ascii(), "Windows PowerShell 5.1 reads a script without a byte order mark as ANSI");
        assert!(GUEST.contains("params.json") && !GUEST.contains("@NAME@"));
    }

    #[test]
    fn a_local_run_refuses_to_replace_the_persons_own_install() {
        // `bach test --here` installs for real and removes the app again, so the
        // script must stop before it touches anything when the app is already there.
        let guard = GUEST.find("if ($Local -and (Find-Install))").expect("the guard is in the script");
        let install = GUEST.find("# 1. Install.").expect("the install step is in the script");
        assert!(guard < install, "the guard comes before the first step");
        assert!(GUEST.contains("Write-Report
    exit 0") || GUEST.contains("Write-Report
    exit 0"), "and it ends the run");
    }
}
