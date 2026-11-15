//! `bach sign` and the optional signing step of `bach build`.
//!
//! Signing is never required. A project that has no `[signing]` builds
//! unsigned setup files, exactly as before. With `[signing]`, `bach build
//! --sign` (or `BACH_SIGN=1`) signs every setup file it writes (the offline
//! and the online one) before the update feed is written, so the feed's
//! SHA-256 is the signed file's. It's opt-in on purpose: a `bach.toml` from
//! somewhere else must not run a program just because it was built.
//!
//! Bach doesn't sign anything itself: it runs the program the project names
//! (`signtool`, Azure's `AzureSignTool`, `osslsigncode`, …) with the project's
//! arguments, without a shell. Secrets stay out of `bach.toml`: write
//! `{env:NAME}` in the arguments and the build reads it from the environment.

use std::path::{Path, PathBuf};
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
    // The command as it will run, with secrets shown as the variable's name,
    // so a log never holds them and nobody is surprised by what bach.toml runs.
    let shown = self::arguments(signing, file, |name| Some(format!("{{env:{name}}}")))?;
    let program = find_program(&signing.program);
    println!("Signing: {} {}", program.display(), shown.join(" "));
    let status = Command::new(&program).args(&arguments).status().with_context(|| {
        if is_signtool(&signing.program) {
            "couldn't start signtool. It is part of the Windows SDK (install its \"Windows SDK Signing Tools for Desktop Apps\"), \
             or give its full path in [signing] program"
                .to_string()
        } else {
            format!("couldn't start {}. Install it or give its full path in [signing] program", signing.program)
        }
    })?;
    if !status.success() {
        bail!(
            "{} failed to sign {} (exit code {}); the setup file is unsigned",
            signing.program,
            file.display(),
            status.code().map_or_else(|| "unknown".to_string(), |code| code.to_string())
        );
    }
    // signtool is the tool whose job is known: a file it didn't sign is a failure. Another
    // program (a cloud signer, a script) may sign differently or not at all on purpose.
    if is_signtool(&signing.program) {
        check_signature(file)?;
    } else {
        println!("Signature not checked: {} isn't signtool", signing.program);
    }
    Ok(())
}

/// What the Authenticode check of a freshly signed file means.
enum Signed {
    /// Valid, with a timestamp: good for as long as the file lasts.
    Fine,
    /// Valid, but no timestamp: it stops being valid when the certificate expires.
    NoTimestamp,
    /// Signed, but Windows doesn't trust it (yet): a self-signed or unknown root.
    NotTrusted(String),
    /// Not a good signature: nothing was signed, or the file changed after.
    Broken(String),
}

fn judge(status: &str, timestamped: bool) -> Signed {
    match status {
        "Valid" if timestamped => Signed::Fine,
        "Valid" => Signed::NoTimestamp,
        "NotSigned" => Signed::Broken("it has no signature at all".into()),
        "HashMismatch" => Signed::Broken("the file doesn't match its signature".into()),
        other => Signed::NotTrusted(other.to_string()),
    }
}

/// Looks at the signature `file` has now, with Windows' own check, and says
/// what it finds. A file with no (or a broken) signature is an error: the build
/// would otherwise publish an unsigned setup file believing it signed. No
/// timestamp, or a signature Windows doesn't trust yet, is a warning.
fn check_signature(file: &Path) -> Result<()> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$s = Get-AuthenticodeSignature -LiteralPath $env:BACH_VERIFY_FILE; \
             \"$($s.Status)\"; \"$($null -ne $s.TimeStamperCertificate)\"",
        ])
        .env("BACH_VERIFY_FILE", file)
        .output();
    let Ok(output) = output else {
        println!("warning: couldn't check the signature of {} (PowerShell isn't available)", file.display());
        return Ok(());
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.lines().map(str::trim);
    let (status, timestamped) = (lines.next().unwrap_or(""), lines.next() == Some("True"));
    match judge(status, timestamped) {
        Signed::Fine => println!("Signature checked: valid and timestamped"),
        Signed::NoTimestamp => println!(
            "warning: {} is signed but has no timestamp, so the signature stops being valid when the certificate expires; \
             add a timestamp server to [signing] arguments (signtool: /tr URL /td sha256)",
            file.display()
        ),
        Signed::NotTrusted(status) => println!(
            "warning: {} is signed, but Windows reports \"{status}\" for the signature (a self-signed or unknown root certificate looks like this)",
            file.display()
        ),
        Signed::Broken(why) => bail!("the sign command ran, but {} isn't signed: {why}", file.display()),
    }
    Ok(())
}

/// Whether `program` names Windows' signing tool without a folder.
fn is_signtool(program: &str) -> bool {
    program.eq_ignore_ascii_case("signtool") || program.eq_ignore_ascii_case("signtool.exe")
}

/// The program to start. `signtool` is rarely on `PATH`: the Windows SDK puts
/// it in a versioned folder only its own command prompts know about. When it
/// isn't on `PATH`, the newest one in the SDK's folder is used instead.
fn find_program(program: &str) -> PathBuf {
    if is_signtool(program) && !on_path(program) {
        let kits = std::env::var_os("ProgramFiles(x86)").map(|files| PathBuf::from(files).join("Windows Kits").join("10").join("bin"));
        if let Some(found) = kits.as_deref().and_then(newest_signtool) {
            println!("signtool isn't on PATH; using {}", found.display());
            return found;
        }
    }
    PathBuf::from(program)
}

/// Whether a program of this name is in a folder on `PATH`.
fn on_path(program: &str) -> bool {
    let names = [program.to_string(), format!("{program}.exe")];
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| names.iter().any(|name| dir.join(name).is_file())))
}

/// The newest `signtool.exe` below `bin`, the SDK's folder of versioned
/// folders (`10.0.26100.0\x64\signtool.exe`), preferring 64-bit.
fn newest_signtool(bin: &Path) -> Option<PathBuf> {
    let mut versions: Vec<(Vec<u32>, PathBuf)> = std::fs::read_dir(bin)
        .ok()?
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| {
            let number = entry.file_name().to_string_lossy().split('.').map(|part| part.parse().unwrap_or(0)).collect();
            (number, entry.path())
        })
        .collect();
    versions.sort_by(|a, b| b.0.cmp(&a.0));
    versions.into_iter().find_map(|(_, folder)| {
        ["x64", "arm64", "x86"].iter().map(|arch| folder.join(arch).join("signtool.exe")).find(|path| path.is_file())
    })
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
    fn a_signature_is_judged_by_status_and_timestamp() {
        assert!(matches!(judge("Valid", true), Signed::Fine));
        assert!(matches!(judge("Valid", false), Signed::NoTimestamp));
        assert!(matches!(judge("NotSigned", false), Signed::Broken(_)));
        assert!(matches!(judge("HashMismatch", true), Signed::Broken(_)));
        assert!(matches!(judge("UnknownError", true), Signed::NotTrusted(_)));
        assert!(matches!(judge("NotTrusted", false), Signed::NotTrusted(_)));
    }

    #[cfg(windows)]
    #[test]
    fn a_file_nobody_signed_is_an_error() {
        let file = std::env::temp_dir().join(format!("bach-unsigned-{}.exe", std::process::id()));
        // A real program that nobody signed: this test's own executable.
        std::fs::copy(std::env::current_exe().unwrap(), &file).unwrap();
        let err = check_signature(&file).unwrap_err().to_string();
        std::fs::remove_file(&file).unwrap();
        assert!(err.contains("isn't signed"), "{err}");
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
    fn the_newest_signtool_in_the_sdk_is_found() {
        let bin = std::env::temp_dir().join(format!("bach-kits-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&bin);
        for (version, arch) in
            [("10.0.19041.0", "x64"), ("10.0.26100.0", "x86"), ("10.0.26100.0", "x64"), ("10.0.9.0", "x64"), ("notes", "x64")]
        {
            let folder = bin.join(version).join(arch);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("signtool.exe"), b"").unwrap();
        }
        let found = newest_signtool(&bin).unwrap();
        assert!(found.ends_with(Path::new("10.0.26100.0").join("x64").join("signtool.exe")), "{}", found.display());
        assert!(newest_signtool(&bin.join("missing")).is_none());
        std::fs::remove_dir_all(&bin).unwrap();
        assert!(is_signtool("SignTool.exe") && !is_signtool(r"C:\tools\signtool.exe") && !is_signtool("osslsigncode"));
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
