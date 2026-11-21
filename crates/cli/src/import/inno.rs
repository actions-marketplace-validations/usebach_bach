//! Reads an Inno Setup script (`.iss`).
//!
//! The file is a set of `[Sections]`. `[Setup]` holds `Key=Value` lines; the
//! others (`[Files]`, `[Icons]`, `[Run]`...) hold one entry per line, written
//! `Key: "value"; Key: value; Flags: a b`. `#define Name "value"` lines make
//! `{#Name}` mean that value.

use std::collections::HashMap;

use super::{is_relative, slashes, source_folder, Draft, Run, Shortcut};

/// One entry of a section: its parameters, with the names in lower case.
type Entry = Vec<(String, String)>;

fn get<'a>(entry: &'a Entry, key: &str) -> Option<&'a str> {
    entry.iter().find(|(name, _)| name == key).map(|(_, value)| value.as_str())
}

/// `Key: "a;b"; Flags: x y` as pairs. A quoted value may hold `;` and `""`.
fn entry(line: &str) -> Entry {
    let mut parts = Vec::new();
    let (mut current, mut quoted) = (String::new(), false);
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ';' if !quoted => parts.push(std::mem::take(&mut current)),
            c => current.push(c),
        }
    }
    parts.push(current);
    parts
        .into_iter()
        .filter_map(|part| part.split_once(':').map(|(key, value)| (key.trim().to_ascii_lowercase(), value.trim().to_string())))
        .collect()
}

/// `#define Name "value"` lines, for `{#Name}`.
fn defines(text: &str) -> HashMap<String, String> {
    let mut found = HashMap::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("#define") else { continue };
        let rest = rest.trim_start();
        let Some((name, value)) = rest.split_once(char::is_whitespace) else { continue };
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
        found.insert(name.to_string(), value.to_string());
    }
    found
}

fn expand(line: &str, defines: &HashMap<String, String>) -> String {
    let mut out = line.to_string();
    for (name, value) in defines {
        out = out.replace(&format!("{{#{name}}}"), value);
    }
    out
}

/// A path under `{app}` as a relative path: `{app}\bin\x.exe` gives `bin/x.exe`.
fn under_app(path: &str) -> Option<String> {
    let rest = path.strip_prefix("{app}")?.trim_start_matches(['\\', '/']);
    Some(slashes(rest))
}

/// The last folder of an install path: `{autopf}\{#Name}` gives the name.
fn last_folder(path: &str) -> Option<String> {
    let name = path.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next()?;
    (!name.is_empty() && !name.starts_with('{')).then(|| name.to_string())
}

pub fn parse(text: &str) -> Draft {
    let defines = defines(text);
    let mut draft = Draft::default();
    let mut section = String::new();
    let (mut files, mut icons, mut runs, mut uninstall_runs): (Vec<Entry>, Vec<Entry>, Vec<Entry>, Vec<Entry>) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut setup: HashMap<String, String> = HashMap::new();
    let mut tasks: Vec<Entry> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(directive) = line.strip_prefix('#') {
            if directive.starts_with("include") {
                draft.skip("An #include file isn't read: import it by hand, or paste it into the script first");
            }
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_ascii_lowercase();
            continue;
        }
        let line = expand(line, &defines);
        match section.as_str() {
            "setup" => {
                if let Some((key, value)) = line.split_once('=') {
                    setup.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
                }
            }
            "files" => files.push(entry(&line)),
            "icons" => icons.push(entry(&line)),
            "run" => runs.push(entry(&line)),
            "uninstallrun" => uninstall_runs.push(entry(&line)),
            "tasks" => tasks.push(entry(&line)),
            _ => {}
        }
    }
    setup_section(&mut draft, &setup);
    files_section(&mut draft, &files);
    icons_section(&mut draft, &icons, &tasks);
    runs_section(&mut draft, &runs, "install");
    runs_section(&mut draft, &uninstall_runs, "uninstall");
    unsupported(&mut draft, text);
    draft
}

fn setup_section(draft: &mut Draft, setup: &HashMap<String, String>) {
    let value = |key: &str| setup.get(key).filter(|value| !value.is_empty()).cloned();
    draft.name = value("appname");
    draft.version = value("appversion");
    draft.publisher = value("apppublisher");
    draft.website = value("apppublisherurl").filter(|url| url.starts_with("https://"));
    draft.icon = value("setupiconfile").map(|path| slashes(&path)).filter(|path| is_relative(path));
    draft.license = value("licensefile").map(|path| slashes(&path)).filter(|path| is_relative(path));
    if let Some(base) = value("outputbasefilename") {
        let dir = value("outputdir").map(|dir| slashes(&dir)).filter(|dir| is_relative(dir)).unwrap_or_else(|| "Output".into());
        draft.output = Some(format!("{dir}/{base}.exe"));
    }
    // Inno asks for administrator rights unless the script says "lowest".
    let privileges = value("privilegesrequired").unwrap_or_else(|| "admin".into()).to_ascii_lowercase();
    if privileges == "lowest" {
        draft.machine = Some(false);
    } else {
        draft.machine = Some(true);
        draft.skip(
            "The script installs for all users (Inno's default is administrator rights), so scope = \"machine\" is set; \
             delete that line if the app can live in one user's folder, which needs no administrator prompt",
        );
    }
    draft.folder = value("defaultdirname").and_then(|dir| last_folder(&dir));
    if let Some(dir) = value("defaultdirname") {
        let own_folder = !dir.starts_with("{autopf}") && !dir.starts_with("{pf}") && !dir.starts_with("{commonpf}") && !dir.starts_with("{userpf}");
        let custom = own_folder && !dir.starts_with("{localappdata}") && !dir.starts_with("{app}");
        if custom {
            draft.skip(format!("The install folder \"{dir}\" isn't Program Files or the user's programs folder: Bach picks its own"));
        }
    }
    if setup.contains_key("signtool") {
        draft.skip("SignTool: Bach signs with [signing], see the signing guide");
    }
    if setup.contains_key("appid") {
        draft.skip("AppId: set app.id to the same name if installed copies of this app have to be updated by the Bach setup");
    }
}

fn files_section(draft: &mut Draft, files: &[Entry]) {
    // The folder most files are taken from is the one `[files] source` names.
    let mut counts: Vec<(String, usize)> = Vec::new();
    let mut elsewhere = Vec::new();
    for file in files {
        let (Some(source), Some(dest)) = (get(file, "source"), get(file, "destdir")) else { continue };
        let (folder, _) = source_folder(source);
        let dest = dest.trim_end_matches(['\\', '/']);
        if dest == "{app}" && super::is_relative(source) {
            match counts.iter_mut().find(|(known, _)| *known == folder) {
                Some((_, count)) => *count += 1,
                None => counts.push((folder.clone(), 1)),
            }
            if let Some(excludes) = get(file, "excludes") {
                for pattern in excludes.split(',').map(slashes).filter(|pattern| !pattern.is_empty()) {
                    if !draft.exclude.contains(&pattern) {
                        draft.exclude.push(pattern);
                    }
                }
            }
            if draft.executable.is_none() && source.to_ascii_lowercase().ends_with(".exe") {
                draft.executable = Some(slashes(source).rsplit('/').next().unwrap_or_default().to_string());
            }
        } else {
            elsewhere.push(format!("{source} (to {dest})"));
        }
    }
    if let Some((folder, _)) = counts.iter().max_by_key(|(_, count)| *count) {
        let others: Vec<&String> = counts.iter().map(|(other, _)| other).filter(|other| *other != folder).collect();
        draft.source = Some(if folder.is_empty() { ".".to_string() } else { folder.clone() });
        if !others.is_empty() {
            let list: Vec<&str> = others.iter().map(|other| other.as_str()).collect();
            draft.skip(format!("Files from {} come from other folders: copy them into the source folder, keeping their layout", list.join(", ")));
        }
    }
    if !elsewhere.is_empty() {
        draft.skip(format!(
            "Files that aren't installed into the app's own folder ({}): Bach installs only the app's folder; use [[run]] for what has to go elsewhere",
            elsewhere.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
        ));
    }
}

fn icons_section(draft: &mut Draft, icons: &[Entry], tasks: &[Entry]) {
    for icon in icons {
        let (Some(name), Some(target)) = (get(icon, "name"), get(icon, "filename")) else { continue };
        if target.starts_with("{uninstallexe}") {
            continue;
        }
        let relative = under_app(target);
        let lower_name = name.to_ascii_lowercase();
        if lower_name.contains("desktop") {
            // A desktop shortcut that belongs to a task is offered unticked when the task is.
            let task = get(icon, "tasks").and_then(|id| tasks.iter().find(|task| get(task, "name") == Some(id)));
            let unticked = task.and_then(|task| get(task, "flags")).is_some_and(|flags| flags.to_ascii_lowercase().contains("unchecked"));
            draft.desktop_shortcut = Some(!unticked);
            if draft.executable.is_none() {
                draft.executable = relative.filter(|path| path.to_ascii_lowercase().ends_with(".exe"));
            }
            continue;
        }
        let Some(relative) = relative else { continue };
        if !relative.to_ascii_lowercase().ends_with(".exe") {
            continue;
        }
        if draft.executable.is_none() {
            draft.executable = Some(relative);
            continue;
        }
        if draft.executable.as_deref() == Some(relative.as_str()) {
            continue;
        }
        let label = name.rsplit(['\\', '/']).next().unwrap_or(name).to_string();
        draft.shortcuts.push(Shortcut { name: label, target: relative, arguments: get(icon, "parameters").map(str::to_string) });
    }
}

fn runs_section(draft: &mut Draft, runs: &[Entry], when: &'static str) {
    for run in runs {
        let Some(file) = get(run, "filename") else { continue };
        let flags = get(run, "flags").unwrap_or_default().to_ascii_lowercase();
        let Some(relative) = under_app(file) else {
            draft.skip(format!("A step runs {file}: Bach runs only a program that is in the app's folder ([[run]])"));
            continue;
        };
        // "Open the app when setup ends" is the window's Open button, not a step.
        if flags.contains("postinstall") {
            continue;
        }
        draft.runs.push(Run { file: relative, arguments: get(run, "parameters").map(str::to_string), when });
    }
}

/// The sections Bach has no setting for, named once each.
fn unsupported(draft: &mut Draft, text: &str) {
    let sections: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('[') && line.ends_with(']'))
        .map(|line| line[1..line.len() - 1].to_ascii_lowercase())
        .collect();
    let has = |name: &str| sections.iter().any(|section| section == name);
    for (name, what) in [
        ("code", "[Code] (Pascal script): custom pages, checks and logic have no Bach equivalent"),
        ("registry", "[Registry]: recreate each entry with [[registry]], [[protocols]] or [[file_types]]"),
        ("components", "[Components] and [Types]: recreate the choices with [[components]]"),
        ("dirs", "[Dirs]: empty folders aren't created"),
        ("ini", "[INI]: ini files aren't written"),
        ("installdelete", "[InstallDelete]: files an update should remove: Bach removes files the new version no longer ships"),
        ("uninstalldelete", "[UninstallDelete]: extra files to delete on removal: use [uninstall] data"),
        ("messages", "[Messages] and [CustomMessages]: use [text] for the window's words"),
        ("languages", "[Languages]: Bach has 19 built-in languages and follows Windows' own"),
    ] {
        if has(name) {
            draft.skip(what);
        }
    }
    if has("custommessages") {
        draft.skip("[Messages] and [CustomMessages]: use [text] for the window's words");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What Inno's own wizard writes, with a few of the usual additions.
    const SCRIPT: &str = r#"
; Script generated by the Inno Setup Script Wizard.
#define MyAppName "Hello"
#define MyAppVersion "1.5"
#define MyAppPublisher "Hello Inc."
#define MyAppURL "https://hello.example"
#define MyAppExeName "Hello.exe"

[Setup]
AppId={{8E2A5C54-1111-4F3C-9B52-0A1B2C3D4E5F}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppName}
LicenseFile=LICENSE.txt
OutputDir=installer
OutputBaseFilename=HelloSetup
SetupIconFile=assets\hello.ico
PrivilegesRequired=lowest
SignTool=mysign

[Tasks]
Name: "desktopicon"; Description: "Create a desktop icon"; Flags: unchecked

[Files]
Source: "build\Release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "build\Release\*"; DestDir: "{app}"; Excludes: "*.pdb,*.ilk"; Flags: ignoreversion recursesubdirs
Source: "vc\vc_redist.x64.exe"; DestDir: "{tmp}"; Flags: deleteafterinstall

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autoprograms}\Hello Docs"; Filename: "{app}\docs\viewer.exe"; Parameters: "--welcome"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch"; Flags: nowait postinstall skipifsilent
Filename: "{app}\redist\helper.exe"; Parameters: "/quiet"
Filename: "{tmp}\vc_redist.x64.exe"; Parameters: "/quiet"

[Registry]
Root: HKCU; Subkey: "Software\Hello"; ValueType: string; ValueName: "x"; ValueData: "1"

[Code]
function InitializeSetup(): Boolean;
begin
  Result := True;
end;
"#;

    #[test]
    fn the_names_and_choices_come_across() {
        let draft = parse(SCRIPT);
        assert_eq!(draft.name.as_deref(), Some("Hello"));
        assert_eq!(draft.version.as_deref(), Some("1.5"));
        assert_eq!(draft.publisher.as_deref(), Some("Hello Inc."));
        assert_eq!(draft.website.as_deref(), Some("https://hello.example"));
        assert_eq!(draft.executable.as_deref(), Some("Hello.exe"));
        assert_eq!(draft.icon.as_deref(), Some("assets/hello.ico"));
        assert_eq!(draft.license.as_deref(), Some("LICENSE.txt"));
        assert_eq!(draft.output.as_deref(), Some("installer/HelloSetup.exe"));
        assert_eq!(draft.folder.as_deref(), Some("Hello"));
        assert_eq!(draft.machine, Some(false), "PrivilegesRequired=lowest is a per-user install");
        assert_eq!(draft.desktop_shortcut, Some(false), "the desktop task starts unticked");
    }

    #[test]
    fn files_shortcuts_and_steps_come_across() {
        let draft = parse(SCRIPT);
        assert_eq!(draft.source.as_deref(), Some("build/Release"));
        assert_eq!(draft.exclude, ["*.pdb", "*.ilk"]);
        assert_eq!(draft.shortcuts, [Shortcut { name: "Hello Docs".into(), target: "docs/viewer.exe".into(), arguments: Some("--welcome".into()) }]);
        assert_eq!(draft.runs, [Run { file: "redist/helper.exe".into(), arguments: Some("/quiet".into()), when: "install" }]);
        assert!(draft.missing().is_empty(), "{:?}", draft.missing());
    }

    #[test]
    fn what_has_no_setting_is_said_not_dropped() {
        let draft = parse(SCRIPT);
        let said = draft.not_converted.join("\n");
        for part in ["[Code]", "[Registry]", "SignTool", "AppId", "{tmp}\\vc_redist.x64.exe", "{tmp}"] {
            assert!(said.contains(part), "{part} missing from:\n{said}");
        }
        assert!(!said.contains("scope"), "a per-user script gets no scope note: {said}");
    }

    #[test]
    fn administrator_scripts_become_machine_installs_and_say_so() {
        let draft = parse("[Setup]\nAppName=X\nAppVersion=1\nAppPublisher=P\nDefaultDirName={autopf}\\X\n");
        assert_eq!(draft.machine, Some(true));
        assert!(draft.not_converted.iter().any(|note| note.contains("scope = \"machine\"")));
    }

    #[test]
    fn the_written_file_is_valid_toml_with_bachs_keys() {
        let text = parse(SCRIPT).to_toml("Hello.iss");
        let table: toml::Table = text.parse().unwrap_or_else(|err| panic!("{err}\n{text}"));
        assert_eq!(table["app"]["name"].as_str(), Some("Hello"));
        assert_eq!(table["files"]["source"].as_str(), Some("build/Release"));
        assert_eq!(table["installer"]["desktop_shortcut"].as_bool(), Some(false));
        assert_eq!(table["run"].as_array().map(Vec::len), Some(1));
        assert_eq!(table["shortcuts"].as_array().map(Vec::len), Some(1));
        assert!(table["installer"].get("scope").is_none());
    }

    #[test]
    fn a_quoted_value_may_hold_semicolons_and_quotes() {
        let parsed = entry(r#"Source: "a;b.exe"; DestDir: "{app}"; Parameters: "say ""hi""""#);
        assert_eq!(get(&parsed, "source"), Some("a;b.exe"));
        assert_eq!(get(&parsed, "parameters"), Some("say \"hi\""));
    }

    #[test]
    fn odd_scripts_never_panic() {
        for text in ["", "[", "[]", "[Files]\nSource", "[Setup]\n=\n", "#define", "[Icons]\nName: ; Filename:", "\u{feff}[Setup]\nAppName=Ä"] {
            let _ = parse(text);
        }
    }
}
