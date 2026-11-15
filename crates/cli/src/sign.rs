//! `bach sign` and the optional signing step of `bach build`.
//!
//! Signing is never required. A project that has no `[signing]` builds
//! unsigned setup files, exactly as before. With `[signing]`, `bach build`
//! signs every setup file it writes (the offline and the online one) before
//! the update feed is written, so the feed's SHA-256 is the signed file's.
//!
//! Bach doesn't sign anything itself: it runs the program the project names
//! (`signtool`, Azure's `AzureSignTool`, `osslsigncode`, …) with the project's
//! arguments, without a shell. Secrets stay out of `bach.toml`: write
//! `{env:NAME}` in the arguments and the build reads it from the environment.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use bach_pack::config::Signing;

/// What `bach sign` was asked for.
pub struct Options<'a> {
    pub file: &'a Path,
    pub config: &'a Path,
}

/// `bach sign FILE`: signs a setup file the way `bach.toml` says.
pub fn command(options: Options) -> Result<()> {
    let project = bach_pack::config::load(options.config)?;
    let Some(signing) = &project.signing else {
        bail!("{} has no [signing] section, so Bach doesn't know how to sign; see docs/signing.md", options.config.display());
    };
    sign(signing, options.file)?;
    println!("Signed {}", options.file.display());
    Ok(())
}

/// Signs `file` with the program and arguments of `signing`.
pub fn sign(signing: &Signing, file: &Path) -> Result<()> {
    let arguments = arguments(signing, file, |name| std::env::var(name).ok())?;
    println!("Signing {} with {}", file.display(), signing.program);
    let status = Command::new(&signing.program).args(&arguments).status().with_context(|| {
        format!(
            "couldn't start {}. Install it or give its full path in [signing] program (Windows' signtool comes with the Windows SDK)",
            signing.program
        )
    })?;
    if !status.success() {
        bail!(
            "{} failed to sign {} (exit code {}); the setup file is unsigned",
            signing.program,
            file.display(),
            status.code().map_or_else(|| "unknown".to_string(), |code| code.to_string())
        );
    }
    Ok(())
}

/// The program's arguments: the text split like a command line, `{file}`
/// replaced by the setup file and `{env:NAME}` by the variable's value. The
/// file is appended when the arguments never mention it.
fn arguments(signing: &Signing, file: &Path, env: impl Fn(&str) -> Option<String>) -> Result<Vec<String>> {
    // `dist/x.exe` has to be `dist\x.exe` for Windows tools, which read `/` as a switch.
    let file_text = file.to_string_lossy().replace('/', std::path::MAIN_SEPARATOR_STR);
    let mut out = Vec::new();
    let mut named_file = false;
    for token in split(&signing.arguments) {
        named_file |= token.contains("{file}");
        out.push(fill(&token.replace("{file}", &file_text), &env)?);
    }
    if !named_file {
        out.push(file_text);
    }
    Ok(out)
}

/// Replaces `{env:NAME}` with the variable's value.
fn fill(text: &str, env: &impl Fn(&str) -> Option<String>) -> Result<String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("{env:") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 5..];
        let Some(end) = after.find('}') else {
            bail!("[signing] arguments has an {{env: without its closing }}");
        };
        let name = &after[..end];
        let Some(value) = env(name).filter(|value| !value.is_empty()) else {
            bail!("[signing] arguments needs the environment variable {name}, which isn't set");
        };
        out.push_str(&value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Splits a command line into words; double quotes group, and are dropped.
fn split(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    for c in text.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    words.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(current);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signing(arguments: &str) -> Signing {
        Signing { program: "signtool".into(), arguments: arguments.into() }
    }

    #[test]
    fn words_are_split_like_a_command_line() {
        assert_eq!(split(r#"sign /d "My App" /a"#), ["sign", "/d", "My App", "/a"]);
        assert_eq!(split("  a   b "), ["a", "b"]);
        assert_eq!(split(r#"x "" y"#), ["x", "", "y"]);
        assert!(split("   ").is_empty());
    }

    #[test]
    fn the_file_goes_where_it_is_named_or_last() {
        let file = Path::new("dist/Hello Setup.exe");
        let native = format!("dist{}Hello Setup.exe", std::path::MAIN_SEPARATOR);
        let none = |_: &str| None;
        assert_eq!(arguments(&signing("sign /fd SHA256"), file, none).unwrap(), ["sign", "/fd", "SHA256", native.as_str()]);
        assert_eq!(
            arguments(&signing("-in {file} -h sha256"), file, none).unwrap(),
            ["-in", native.as_str(), "-h", "sha256"],
            "named once: not appended again"
        );
    }

    #[test]
    fn secrets_come_from_the_environment() {
        let env = |name: &str| (name == "CERT_PASSWORD").then(|| "s3cret".to_string());
        let args = arguments(&signing("/f cert.pfx /p {env:CERT_PASSWORD}"), Path::new("a.exe"), env).unwrap();
        assert_eq!(args, ["/f", "cert.pfx", "/p", "s3cret", "a.exe"]);
        let err = arguments(&signing("/p {env:MISSING}"), Path::new("a.exe"), env).unwrap_err().to_string();
        assert!(err.contains("MISSING"), "{err}");
        assert!(arguments(&signing("/p {env:OPEN"), Path::new("a.exe"), env).is_err());
    }
}
