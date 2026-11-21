//! Reads an NSIS script (`.nsi`).
//!
//! NSIS is a scripting language: commands with arguments, `!define NAME value`
//! constants used as `${NAME}`, and `Section`s and `Function`s of commands.
//! This reads the commands that state facts (`Name`, `OutFile`, `InstallDir`,
//! `File`, `CreateShortCut`...) and names what it passes over.

use std::collections::HashMap;

use super::{is_relative, slashes, source_folder, Draft, Run, Shortcut};

/// A line as words: whitespace separates, `"..."`, `'...'` and `` `...` `` group.
fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut current, mut quote, mut started) = (String::new(), None::<char>, false);
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => current.push(c),
            (None, '"' | '\'' | '`') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started || !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            // A comment ends the line (a `;` or `#` that opens a word).
            (None, ';' | '#') if current.is_empty() && !started => break,
            (None, c) => current.push(c),
        }
    }
    if started || !current.is_empty() {
        out.push(current);
    }
    out
}

fn expand(text: &str, defines: &HashMap<String, String>) -> String {
    let mut out = text.to_string();
    // A define may use another: two rounds settle the usual `${A}` inside `${B}`.
    for _ in 0..2 {
        for (name, value) in defines {
            out = out.replace(&format!("${{{name}}}"), value);
        }
    }
    out
}

/// A path under `$INSTDIR` as a relative path.
fn under_instdir(path: &str) -> Option<String> {
    let rest = path.strip_prefix("$INSTDIR")?.trim_start_matches(['\\', '/']);
    (!rest.is_empty()).then(|| slashes(rest))
}

fn last_folder(path: &str) -> Option<String> {
    let name = path.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next()?;
    (!name.is_empty() && !name.starts_with('$')).then(|| name.to_string())
}

/// The first define whose name contains `part` (upper case), as `PRODUCT_VERSION`.
fn define_like<'a>(defines: &'a HashMap<String, String>, parts: &[&str]) -> Option<&'a String> {
    let mut names: Vec<&String> = defines.keys().collect();
    names.sort();
    names
        .into_iter()
        .find(|name| {
            let upper = name.to_ascii_uppercase();
            parts.iter().any(|part| upper.ends_with(part)) && !defines[*name].is_empty()
        })
        .map(|name| &defines[name])
}

pub fn parse(text: &str) -> Draft {
    let mut defines: HashMap<String, String> = HashMap::new();
    // `!define` lines first, so a use may come before its definition in the file's order of reading.
    for line in text.lines() {
        let w = words(line.trim());
        if w.len() >= 3 && w[0].eq_ignore_ascii_case("!define") && !w[1].starts_with('/') {
            defines.insert(w[1].clone(), w[2].clone());
        }
    }
    let mut draft = Draft::default();
    let mut sources: Vec<(String, usize)> = Vec::new();
    let (mut registry, mut functions, mut custom_pages, mut other_commands) = (0usize, 0usize, 0usize, Vec::<String>::new());
    let mut shortcut_targets: Vec<(String, String, Option<String>)> = Vec::new();
    for raw in text.lines() {
        let line = expand(raw.trim(), &defines);
        let w = words(&line);
        let Some(command) = w.first().map(|word| word.to_ascii_lowercase()) else { continue };
        let arg = |index: usize| w.get(index).map(String::as_str);
        match command.as_str() {
            "name" => draft.name = draft.name.take().or_else(|| arg(1).map(str::to_string)),
            "outfile" => draft.output = arg(1).map(slashes).filter(|path| is_relative(path)),
            "installdir" => {
                if let Some(dir) = arg(1) {
                    draft.folder = last_folder(dir);
                    let upper = dir.to_ascii_uppercase();
                    if upper.starts_with("$PROGRAMFILES") {
                        draft.machine = draft.machine.or(Some(true));
                    }
                }
            }
            "requestexecutionlevel" => {
                let level = arg(1).unwrap_or_default().to_ascii_lowercase();
                draft.machine = Some(level == "admin" || level == "highest");
            }
            "licensedata" => draft.license = arg(1).map(slashes).filter(|path| is_relative(path)),
            "icon" => draft.icon = arg(1).map(slashes).filter(|path| is_relative(path)),
            "viaddversionkey" => match (arg(1).map(str::to_ascii_lowercase).as_deref(), arg(2)) {
                (Some("companyname"), Some(value)) => draft.publisher = draft.publisher.take().or_else(|| Some(value.to_string())),
                (Some("productversion"), Some(value)) => draft.version = draft.version.take().or_else(|| Some(value.to_string())),
                _ => {}
            },
            "file" => {
                // `File /r /x "*.pdb" "dist\*"`: switches first, the path last.
                let mut index = 1;
                while let Some(word) = arg(index).filter(|word| word.starts_with('/')) {
                    if word.eq_ignore_ascii_case("/x") {
                        if let Some(pattern) = arg(index + 1) {
                            draft.exclude.push(slashes(pattern));
                        }
                        index += 1;
                    }
                    index += 1;
                }
                if let Some(path) = arg(index).filter(|path| is_relative(path)) {
                    let (folder, _) = source_folder(path);
                    match sources.iter_mut().find(|(known, _)| *known == folder) {
                        Some((_, count)) => *count += 1,
                        None => sources.push((folder, 1)),
                    }
                    if draft.executable.is_none() && !path.contains(['*', '?']) && path.to_ascii_lowercase().ends_with(".exe") {
                        draft.executable = slashes(path).rsplit('/').next().map(str::to_string);
                    }
                }
            }
            "createshortcut" => {
                if let (Some(link), Some(target)) = (arg(1), arg(2)) {
                    shortcut_targets.push((link.to_string(), target.to_string(), arg(3).map(str::to_string)));
                }
            }
            "execwait" | "exec" => match arg(1).and_then(split_command) {
                Some((program, arguments)) => draft.runs.push(Run { file: program, arguments, when: "install" }),
                None => other_commands.push(w.join(" ")),
            },
            "writeregstr" | "writeregdword" | "writeregbin" | "writeregexpandstr" => registry += 1,
            "function" => functions += 1,
            "page" if arg(1).is_some_and(|kind| kind.eq_ignore_ascii_case("custom")) => custom_pages += 1,
            _ => {}
        }
        // The MUI macros that name a license file or an icon.
        if command == "!insertmacro" && arg(1) == Some("MUI_PAGE_LICENSE") {
            draft.license = draft.license.take().or_else(|| arg(2).map(slashes).filter(|path| is_relative(path)));
        }
        if command == "!define" && arg(1) == Some("MUI_ICON") {
            draft.icon = draft.icon.take().or_else(|| arg(2).map(slashes).filter(|path| is_relative(path)));
        }
    }
    draft.version = define_like(&defines, &["VERSION"]).cloned().or(draft.version.take());
    draft.publisher = define_like(&defines, &["PUBLISHER", "COMPANY"]).cloned().or(draft.publisher.take());
    draft.website = define_like(&defines, &["WEB_SITE", "WEBSITE", "URL"]).filter(|url| url.starts_with("https://")).cloned();
    if draft.name.is_none() {
        draft.name = define_like(&defines, &["PRODUCT_NAME", "APP_NAME"]).cloned();
    }
    if let Some((folder, _)) = sources.iter().max_by_key(|(_, count)| *count) {
        draft.source = Some(if folder.is_empty() { ".".to_string() } else { folder.clone() });
        let others: Vec<&str> = sources.iter().map(|(other, _)| other.as_str()).filter(|other| other != folder).collect();
        if !others.is_empty() {
            draft.skip(format!(
                "Files from {} come from other folders: copy them into the source folder, keeping their layout",
                others.join(", ")
            ));
        }
    }
    shortcuts(&mut draft, &shortcut_targets);
    for command in other_commands.iter().take(3) {
        draft.skip(format!("{command}: Bach runs only a program that is in the app's folder ([[run]])"));
    }
    if registry > 0 {
        draft.skip(format!("{registry} registry writes: recreate each with [[registry]], [[protocols]] or [[file_types]]"));
    }
    if functions > 0 || custom_pages > 0 {
        draft.skip("Functions and custom pages: their logic has no Bach equivalent");
    }
    draft
}

/// `"$INSTDIR\redist\vc.exe" /quiet` into the program (relative) and its arguments.
fn split_command(command: &str) -> Option<(String, Option<String>)> {
    let (program, arguments) = match command.split_once(".exe") {
        // The closing quote of `"$INSTDIR\x.exe" /quiet` belongs to the program, not the arguments.
        Some((head, tail)) => (format!("{head}.exe"), tail.trim_start_matches('"').trim().to_string()),
        None => return None,
    };
    let program = under_instdir(program.trim_matches('"'))?;
    Some((program, (!arguments.is_empty()).then_some(arguments)))
}

fn shortcuts(draft: &mut Draft, found: &[(String, String, Option<String>)]) {
    for (link, target, arguments) in found {
        let link_upper = link.to_ascii_uppercase();
        let relative = under_instdir(target);
        let label = link.rsplit(['\\', '/']).next().unwrap_or(link).trim_end_matches(".lnk").to_string();
        if link_upper.starts_with("$DESKTOP") || link_upper.starts_with("$COMMONDESKTOP") {
            draft.desktop_shortcut = Some(true);
            if draft.executable.is_none() {
                draft.executable = relative.filter(|path| path.to_ascii_lowercase().ends_with(".exe"));
            }
        } else if link_upper.starts_with("$SMPROGRAMS") || link_upper.starts_with("$STARTMENU") {
            let Some(relative) = relative.filter(|path| path.to_ascii_lowercase().ends_with(".exe")) else { continue };
            if draft.executable.is_none() {
                draft.executable = Some(relative);
            } else if draft.executable.as_deref() != Some(relative.as_str()) {
                draft.shortcuts.push(Shortcut {
                    name: label,
                    target: relative,
                    arguments: arguments.clone().filter(|text| !text.is_empty()),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = r#"
; The classic wizard layout
!define PRODUCT_NAME "Hello"
!define PRODUCT_VERSION "2.1.0"
!define PRODUCT_PUBLISHER "Hello Inc."
!define PRODUCT_WEB_SITE "https://hello.example"
!define MUI_ICON "assets\hello.ico"
!include "MUI2.nsh"
!insertmacro MUI_PAGE_LICENSE "LICENSE.txt"

Name "${PRODUCT_NAME}"
OutFile "dist\${PRODUCT_NAME}-Setup.exe"
InstallDir "$PROGRAMFILES64\${PRODUCT_NAME}"
RequestExecutionLevel admin

Section "Main"
  SetOutPath "$INSTDIR"
  File "build\Hello.exe"
  File /r /x "*.pdb" "build\*"
  CreateShortCut "$DESKTOP\${PRODUCT_NAME}.lnk" "$INSTDIR\Hello.exe"
  CreateShortCut "$SMPROGRAMS\Hello\Hello Docs.lnk" "$INSTDIR\docs\viewer.exe" "--welcome"
  ExecWait '"$INSTDIR\redist\helper.exe" /quiet'
  ExecWait '"$SYSDIR\regsvr32.exe" /s x.dll'
  WriteRegStr HKLM "Software\Hello" "Path" "$INSTDIR"
SectionEnd

Function .onInit
FunctionEnd
"#;

    #[test]
    fn the_names_and_choices_come_across() {
        let draft = parse(SCRIPT);
        assert_eq!(draft.name.as_deref(), Some("Hello"));
        assert_eq!(draft.version.as_deref(), Some("2.1.0"));
        assert_eq!(draft.publisher.as_deref(), Some("Hello Inc."));
        assert_eq!(draft.website.as_deref(), Some("https://hello.example"));
        assert_eq!(draft.icon.as_deref(), Some("assets/hello.ico"));
        assert_eq!(draft.license.as_deref(), Some("LICENSE.txt"));
        assert_eq!(draft.output.as_deref(), Some("dist/Hello-Setup.exe"));
        assert_eq!(draft.folder.as_deref(), Some("Hello"));
        assert_eq!(draft.machine, Some(true));
        assert_eq!(draft.desktop_shortcut, Some(true));
    }

    #[test]
    fn files_shortcuts_and_steps_come_across() {
        let draft = parse(SCRIPT);
        assert_eq!(draft.source.as_deref(), Some("build"));
        assert_eq!(draft.exclude, ["*.pdb"]);
        assert_eq!(draft.executable.as_deref(), Some("Hello.exe"));
        assert_eq!(
            draft.shortcuts,
            [Shortcut { name: "Hello Docs".into(), target: "docs/viewer.exe".into(), arguments: Some("--welcome".into()) }]
        );
        assert_eq!(draft.runs, [Run { file: "redist/helper.exe".into(), arguments: Some("/quiet".into()), when: "install" }]);
        assert!(draft.missing().is_empty(), "{:?}", draft.missing());
    }

    #[test]
    fn what_has_no_setting_is_said_not_dropped() {
        let said = parse(SCRIPT).not_converted.join("\n");
        for part in ["regsvr32", "1 registry writes", "Functions"] {
            assert!(said.contains(part), "{part} missing from:\n{said}");
        }
    }

    #[test]
    fn the_written_file_is_valid_toml() {
        let text = parse(SCRIPT).to_toml("hello.nsi");
        let table: toml::Table = text.parse().unwrap_or_else(|err| panic!("{err}\n{text}"));
        assert_eq!(table["app"]["version"].as_str(), Some("2.1.0"));
        assert_eq!(table["installer"]["scope"].as_str(), Some("machine"));
    }

    #[test]
    fn words_split_the_way_nsis_reads_a_line() {
        assert_eq!(words(r#"File /r "dist dir\*" ; comment"#), ["File", "/r", "dist dir\\*"]);
        assert_eq!(words("Name 'A B'"), ["Name", "A B"]);
        assert_eq!(words(r#"X "" y"#), ["X", "", "y"], "an empty quoted word is still a word");
        assert!(words("; only a comment").is_empty());
        assert!(words("   ").is_empty());
    }

    #[test]
    fn odd_scripts_never_panic() {
        for text in
            ["", "\"", "File", "File /r", "!define", "!define A", "CreateShortCut", "ExecWait", "Name", "InstallDir", "\u{feff}Name \"Ä\""]
        {
            let _ = parse(text);
        }
    }
}
