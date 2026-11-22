//! Running something with administrator rights. macOS asks for the password
//! in its own dialog (`osascript … with administrator privileges`); setup itself
//! never runs as root, because a window can't open from there.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How an elevated program ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It ran and ended with this exit code.
    Exited(u32),
    /// The user said no to the password prompt.
    Declined,
    /// It took longer than allowed, so setup stopped waiting.
    TimedOut,
    /// macOS couldn't start it, for this reason.
    Failed(String),
}

/// Whether this process already runs as root.
pub fn is_elevated() -> bool {
    // SAFETY: geteuid has no arguments and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

/// One argument as a shell reads it back unchanged: in single quotes.
pub fn quote(argument: &str) -> String {
    format!("'{}'", argument.replace('\'', r"'\''"))
}

/// Splits a command line the way a shell would, without running one: words are
/// separated by spaces, and single quotes, double quotes and backslashes keep
/// them together. Nothing is expanded.
pub fn split(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote = None::<char>;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => match chars.next() {
                Some(next @ ('"' | '\\' | '$' | '`')) => word.push(next),
                Some(next) => {
                    word.push('\\');
                    word.push(next);
                }
                None => word.push('\\'),
            },
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                started = true;
            }
            (None, '\\') => {
                if let Some(next) = chars.next() {
                    word.push(next);
                    started = true;
                }
            }
            (None, c) if c.is_whitespace() => {
                if started || !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (None, c) => {
                word.push(c);
                started = true;
            }
        }
    }
    if started || !word.is_empty() {
        words.push(word);
    }
    words
}

/// Starts `program` with administrator rights and waits for it, up to `limit`.
/// `parameters` is the command line after the program; `hidden` has no meaning here.
pub fn run(program: &Path, parameters: &str, directory: Option<&Path>, limit: Duration, _hidden: bool) -> Outcome {
    let mut shell = String::new();
    if let Some(dir) = directory {
        shell.push_str(&format!("cd {} && ", quote(&dir.to_string_lossy())));
    }
    shell.push_str(&quote(&program.to_string_lossy()));
    for argument in split(parameters) {
        shell.push(' ');
        shell.push_str(&quote(&argument));
    }
    // The command travels in the environment, never inside the script's text.
    let script = "do shell script (system attribute \"BACH_COMMAND\") with administrator privileges";
    let mut child = match Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .env("BACH_COMMAND", &shell)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => return Outcome::Failed(err.to_string()),
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut error = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut pipe, &mut error);
                }
                // -128 is "User canceled".
                if error.contains("(-128)") {
                    return Outcome::Declined;
                }
                // osascript reports the program's own exit code as "execution error: ... (N)".
                if !status.success() {
                    if let Some(code) = error.rsplit('(').next().and_then(|tail| tail.trim().trim_end_matches(')').parse::<i32>().ok()) {
                        return Outcome::Exited(code as u32);
                    }
                    return Outcome::Failed(error.trim().to_string());
                }
                return Outcome::Exited(0);
            }
            Ok(None) if started.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Outcome::TimedOut;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(150)),
            Err(err) => return Outcome::Failed(err.to_string()),
        }
    }
}

/// A machine-wide install on macOS needs no elevated setup: an administrator can
/// write to `/Applications` as themselves. So this is never asked for.
pub fn relaunch(_arguments: &[String]) -> Outcome {
    Outcome::Failed("setup doesn't run as an administrator on macOS".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_survive_a_shell() {
        assert_eq!(quote("--silent"), "'--silent'");
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(quote(""), "''");
    }

    #[test]
    fn command_lines_split_like_a_shell_without_running_one() {
        assert_eq!(split("--silent --dir /Applications/Hello"), ["--silent", "--dir", "/Applications/Hello"]);
        assert_eq!(split(r#"--name "Hello World" 'a b' c\ d"#), ["--name", "Hello World", "a b", "c d"]);
        assert_eq!(split(r#"a "" b"#), ["a", "", "b"]);
        assert_eq!(split("  "), Vec::<String>::new());
        assert_eq!(split("$HOME `id` $(id)"), ["$HOME", "`id`", "$(id)"], "nothing is expanded");
        assert_eq!(split(r#""say \"hi\"""#), [r#"say "hi""#]);
    }

    #[test]
    fn the_test_process_knows_whether_it_is_elevated() {
        let _ = is_elevated();
    }
}
