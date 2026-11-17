//! How `bach` talks: short lines, sizes people can read, errors as one
//! sentence per cause.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde_json::Value;

/// `1536` → "1.5 KB".
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} {}", if bytes == 1 { "byte" } else { "bytes" });
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

static JSON: AtomicBool = AtomicBool::new(false);
static RESULT: Mutex<Vec<(String, Value)>> = Mutex::new(Vec::new());
static WARNINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Turns on `--json`: text for people is not printed, and the command ends with
/// one JSON object ([`finish`]).
pub fn set_json(on: bool) {
    JSON.store(on, Ordering::Relaxed);
}

pub fn json() -> bool {
    JSON.load(Ordering::Relaxed)
}

/// Adds a field to the JSON result, in the order given. Always recorded, shown
/// only with `--json`. Fields are stable: see schema/bach-output.schema.json.
pub fn result(key: &str, value: impl Into<Value>) {
    if let Ok(mut fields) = RESULT.lock() {
        fields.retain(|(known, _)| known != key);
        fields.push((key.to_string(), value.into()));
    }
}

/// With `--json`, prints the result object: `ok`, what the command recorded,
/// and `warnings`. An error is `{"ok": false, "error": ...}`.
pub fn finish(error: Option<&anyhow::Error>) {
    if !json() {
        return;
    }
    let mut object = serde_json::Map::new();
    object.insert("ok".into(), Value::Bool(error.is_none()));
    if let Some(error) = error {
        let causes: Vec<String> = error.chain().map(|cause| sentence(&cause.to_string())).collect();
        object.insert("error".into(), Value::String(causes.first().cloned().unwrap_or_default()));
        object.insert("causes".into(), Value::from(causes.into_iter().skip(1).collect::<Vec<_>>()));
    } else if let Ok(fields) = RESULT.lock() {
        for (key, value) in fields.iter() {
            object.insert(key.clone(), value.clone());
        }
    }
    let warnings = WARNINGS.lock().map(|warnings| warnings.clone()).unwrap_or_default();
    object.insert("warnings".into(), Value::from(warnings));
    println!("{}", Value::Object(object));
}

/// A labelled line: `  Output   dist\Hello-Setup-1.0.0.exe`.
pub fn field(label: &str, value: impl std::fmt::Display) {
    if !json() {
        println!("  {label:<12} {value}");
    }
}

/// Prints what builds but probably isn't meant, one line each (on stderr, and
/// also in the `warnings` of the JSON result).
pub fn warnings(warnings: &[String]) {
    for warning in warnings {
        eprintln!("warning: {}", sentence(warning));
        if let Ok(mut all) = WARNINGS.lock() {
            all.push(sentence(warning));
        }
    }
}

/// Prints an error and its causes, each on its own line.
pub fn error(err: &anyhow::Error) {
    let mut chain = err.chain();
    if let Some(first) = chain.next() {
        eprintln!("error: {}", sentence(&first.to_string()));
    }
    for cause in chain {
        eprintln!("  {}", sentence(&cause.to_string()));
    }
}

fn sentence(text: &str) -> String {
    let text = text.trim();
    // A sentence that starts with a file or a setting ("bach.toml isn't
    // valid", "app.executable is …") keeps it as written: "Bach.toml" names
    // no file, and "App.executable" no setting.
    let first_word = text.split_whitespace().next().unwrap_or_default();
    let literal = first_word.contains(['.', '/', '\\', ':', '_', '[']);
    let mut chars = text.chars();
    let mut out = match chars.next() {
        Some(first) if !literal => first.to_uppercase().chain(chars).collect::<String>(),
        Some(_) => text.to_string(),
        None => String::new(),
    };
    if !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

/// A progress line that rewrites itself on a terminal and stays quiet in
/// logs (CI), where only the final line matters.
pub struct Progress {
    label: &'static str,
    total: u64,
    shown: Option<u32>,
    live: bool,
}

impl Progress {
    pub fn new(label: &'static str, total: u64) -> Self {
        Self { label, total: total.max(1), shown: None, live: std::io::stderr().is_terminal() }
    }

    pub fn update(&mut self, done: u64) {
        if !self.live {
            return;
        }
        let percent = (done.min(self.total) * 100 / self.total) as u32;
        if self.shown != Some(percent) {
            self.shown = Some(percent);
            eprint!("\r  {:<12} {percent:>3}%", self.label);
            let _ = std::io::stderr().flush();
        }
    }

    pub fn finish(&mut self) {
        if self.live && self.shown.is_some() {
            eprint!("\r{:<40}\r", "");
            let _ = std::io::stderr().flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(size(1), "1 byte");
        assert_eq!(size(512), "512 bytes");
        assert_eq!(size(1536), "1.5 KB");
        assert_eq!(size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn errors_read_as_sentences() {
        assert_eq!(sentence("couldn't read x"), "Couldn't read x.");
        assert_eq!(sentence("Done."), "Done.");
        // Files and settings keep their spelling.
        assert_eq!(sentence("bach.toml isn't valid"), "bach.toml isn't valid.");
        assert_eq!(sentence("app.executable is \"x.exe\""), "app.executable is \"x.exe\".");
        assert_eq!(sentence(r".\bach.toml isn't valid"), r".\bach.toml isn't valid.");
    }
}
