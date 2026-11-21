//! `bach import`: a first `bach.toml` from an Inno Setup script (`.iss`) or an
//! NSIS script (`.nsi`).
//!
//! Both are programming languages, so this reads only what has a setting in
//! Bach: the app's name, version and publisher, the folder it installs from,
//! shortcuts, the license, the icon, the install scope and the programs the
//! script runs. Everything else (registry writes, custom pages, code) is
//! listed under "Not converted", so nobody believes the new file does what the
//! old script did when it doesn't. The result is a starting point to read, not
//! a finished config: `bach check` is the next command.

mod inno;
mod nsis;

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::output;

pub struct Options<'a> {
    pub script: &'a Path,
    /// Replace a `bach.toml` that is already there.
    pub force: bool,
    /// Print the file instead of writing it.
    pub dry_run: bool,
}

/// An extra Start menu shortcut (`[[shortcuts]]`).
#[derive(Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub name: String,
    pub target: String,
    pub arguments: Option<String>,
}

/// A program setup runs (`[[run]]`).
#[derive(Debug, PartialEq, Eq)]
pub struct Run {
    pub file: String,
    pub arguments: Option<String>,
    pub when: &'static str,
}

/// What a script said, in Bach's terms. `None` is "the script doesn't say".
#[derive(Debug, Default)]
pub struct Draft {
    pub name: Option<String>,
    pub version: Option<String>,
    pub publisher: Option<String>,
    pub website: Option<String>,
    pub executable: Option<String>,
    pub icon: Option<String>,
    /// Relative to the script, with `/`.
    pub source: Option<String>,
    pub exclude: Vec<String>,
    pub machine: Option<bool>,
    pub folder: Option<String>,
    pub license: Option<String>,
    pub output: Option<String>,
    pub desktop_shortcut: Option<bool>,
    pub shortcuts: Vec<Shortcut>,
    pub runs: Vec<Run>,
    /// What the script does that Bach has no setting for, one sentence each.
    pub not_converted: Vec<String>,
}

impl Draft {
    pub fn skip(&mut self, what: impl Into<String>) {
        let what = what.into();
        if !self.not_converted.contains(&what) {
            self.not_converted.push(what);
        }
    }

    /// The settings Bach needs that the script didn't give.
    pub fn missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        for (name, present) in [
            ("app.name", self.name.is_some()),
            ("app.version", self.version.is_some()),
            ("app.publisher", self.publisher.is_some()),
            ("app.executable", self.executable.is_some()),
            ("files.source", self.source.is_some()),
        ] {
            if !present {
                missing.push(name);
            }
        }
        missing
    }

    /// The `bach.toml` text.
    pub fn to_toml(&self, script_name: &str) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# Written by `bach import` from {script_name}. Read every line, then run `bach check`.");
        let _ = writeln!(out, "# What the script does that Bach has no setting for is listed by the command, not here.\n");
        let _ = writeln!(out, "[app]");
        required(&mut out, "name", &self.name, "the app's name");
        required(&mut out, "version", &self.version, "for example 1.0.0");
        required(&mut out, "publisher", &self.publisher, "you or your company");
        required(&mut out, "executable", &self.executable, "the program to start, relative to source");
        optional(&mut out, "website", &self.website);
        optional(&mut out, "icon", &self.icon);
        let _ = writeln!(out, "\n[files]");
        required(&mut out, "source", &self.source, "the folder your build puts the app in");
        if !self.exclude.is_empty() {
            let list: Vec<String> = self.exclude.iter().map(|pattern| quote(pattern)).collect();
            let _ = writeln!(out, "exclude = [{}]", list.join(", "));
        }
        let mut installer = String::new();
        optional(&mut installer, "output", &self.output);
        optional(&mut installer, "license", &self.license);
        optional(&mut installer, "folder", &self.folder);
        if self.machine == Some(true) {
            let _ = writeln!(installer, "scope = \"machine\"");
        }
        if let Some(on) = self.desktop_shortcut {
            let _ = writeln!(installer, "desktop_shortcut = {on}");
        }
        if !installer.is_empty() {
            let _ = writeln!(out, "\n[installer]\n{installer}");
        }
        for shortcut in &self.shortcuts {
            let _ = writeln!(out, "\n[[shortcuts]]\nname = {}\ntarget = {}", quote(&shortcut.name), quote(&shortcut.target));
            optional(&mut out, "arguments", &shortcut.arguments);
        }
        for run in &self.runs {
            let _ = writeln!(out, "\n[[run]]\nfile = {}", quote(&run.file));
            optional(&mut out, "arguments", &run.arguments);
            let _ = writeln!(out, "when = {}", quote(run.when));
        }
        out
    }
}

fn required(out: &mut String, key: &str, value: &Option<String>, hint: &str) {
    match value {
        Some(value) => {
            let _ = writeln!(out, "{key} = {}", quote(value));
        }
        None => {
            let _ = writeln!(out, "# {key} = \"\"   # the script doesn't say: {hint}");
        }
    }
}

fn optional(out: &mut String, key: &str, value: &Option<String>) {
    if let Some(value) = value {
        let _ = writeln!(out, "{key} = {}", quote(value));
    }
}

/// A TOML basic string.
pub fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A path as `bach.toml` spells it: forward slashes, no leading `.\`.
pub fn slashes(path: &str) -> String {
    let path = path.trim().replace('\\', "/");
    path.strip_prefix("./").unwrap_or(&path).to_string()
}

/// The folder a source path starts from, and whether it names many files
/// (`dist\*` gives `dist`; `bin\app.exe` gives `bin`; `*.dll` gives the
/// script's own folder, as an empty string).
pub fn source_folder(path: &str) -> (String, bool) {
    let path = slashes(path);
    let wildcard = path.contains(['*', '?']);
    // Up to the first wildcard (`dist/*/x.dll` starts at `dist`), else the whole path.
    let head = path.find(['*', '?']).map_or(path.as_str(), |first| &path[..first]);
    let folder = head.rfind('/').map_or(String::new(), |at| head[..at].to_string());
    (folder, wildcard)
}

/// Whether `path` is one Bach could use as a relative folder: no `..` going up
/// is fine (a script often sits in a `setup` folder), but not an absolute path.
pub fn is_relative(path: &str) -> bool {
    let path = path.trim();
    !(path.is_empty() || path.starts_with('/') || path.starts_with('\\') || path.chars().nth(1) == Some(':'))
}

/// The settings of a script, found by its extension.
pub fn read(script: &Path, text: &str) -> Result<Draft> {
    let extension = script.extension().and_then(|ext| ext.to_str()).unwrap_or_default().to_ascii_lowercase();
    match extension.as_str() {
        "iss" => Ok(inno::parse(text)),
        "nsi" | "nsh" => Ok(nsis::parse(text)),
        _ => bail!("{} isn't an Inno Setup script (.iss) or an NSIS script (.nsi)", script.display()),
    }
}

pub fn command(options: Options) -> Result<()> {
    let text = bach_pack::textfile::read(options.script).with_context(|| format!("couldn't read {}", options.script.display()))?;
    let draft = read(options.script, &text)?;
    let script_name = options.script.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let toml = draft.to_toml(&script_name);
    let folder = options.script.parent().filter(|dir| !dir.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let target = folder.join("bach.toml");
    if options.dry_run {
        say!("{toml}");
    } else {
        if target.exists() && !options.force {
            bail!("{} already exists: add --force to replace it, or --dry-run to look at the new one first", target.display());
        }
        fs::write(&target, &toml).with_context(|| format!("couldn't write {}", target.display()))?;
        say!("Wrote {} from {script_name}", target.display());
    }
    let missing = draft.missing();
    if !missing.is_empty() {
        say!("\nThe script doesn't say: {}. They are marked in the file; fill them in.", missing.join(", "));
    }
    if !draft.not_converted.is_empty() {
        say!("\nNot converted, because Bach has no setting for it:");
        for item in &draft.not_converted {
            say!("  - {item}");
        }
    }
    say!("\nPaths are relative to the script's folder. Next: bach check");
    output::result("file", target.display().to_string());
    output::result("missing", missing.iter().map(|name| name.to_string()).collect::<Vec<_>>());
    output::result("not_converted", draft.not_converted.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_path_gives_its_folder() {
        assert_eq!(source_folder(r"dist\*"), ("dist".to_string(), true));
        assert_eq!(source_folder(r"bin\Release\app.exe"), ("bin/Release".to_string(), false));
        assert_eq!(source_folder(r"..\build\*.*"), ("../build".to_string(), true));
        assert_eq!(source_folder("*"), (String::new(), true));
        assert_eq!(source_folder("app.exe"), (String::new(), false));
        assert_eq!(source_folder(r"out\*\bin\x.dll"), ("out".to_string(), true));
    }

    #[test]
    fn strings_are_written_so_the_file_parses_back_the_same() {
        for text in ["plain", r"back\slash", "say \"hi\"", "tab\there", "line\nbreak", "bell\u{7}"] {
            let parsed: toml::Table = format!("x = {}", quote(text)).parse().unwrap();
            assert_eq!(parsed["x"].as_str(), Some(text));
        }
    }

    #[test]
    fn an_empty_draft_marks_what_the_script_did_not_say() {
        let draft = Draft::default();
        let text = draft.to_toml("x.iss");
        assert!(text.contains("# name = \"\""), "{text}");
        assert_eq!(draft.missing(), ["app.name", "app.version", "app.publisher", "app.executable", "files.source"]);
        text.parse::<toml::Table>().expect("it is still TOML");
    }

    #[test]
    fn only_scripts_are_taken() {
        assert!(read(Path::new("setup.exe"), "").is_err());
        assert!(read(Path::new("SETUP.ISS"), "").is_ok());
    }
}
