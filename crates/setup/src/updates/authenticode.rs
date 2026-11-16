//! Who signed a downloaded update, as Windows sees it.
//!
//! With `[updates] signer`, an update is only run when its Authenticode
//! signature is valid for Windows and was made by the named publisher. This
//! stands next to the feed's own checks (SHA-256, `public_key`): a feed host
//! that was taken over can name any file and hash, but cannot sign as you.
//!
//! The check asks Windows' own PowerShell for the signature, never a shell
//! with the path in its command line: the file's name travels in an
//! environment variable.

use std::path::Path;
use std::process::Command;

/// What Windows says about a file's signature.
#[derive(Debug, PartialEq, Eq)]
pub struct Signature {
    /// `Valid`, `NotSigned`, `HashMismatch`, `NotTrusted`, ...
    pub status: String,
    /// The signing certificate's subject, like `CN=Example Inc., O=Example`.
    pub subject: String,
}

/// Reads the signature of `file`.
pub fn read(file: &Path) -> Result<Signature, String> {
    // `system_dir` is System32 itself, as Windows reports it.
    let system = crate::system::folders::system_dir().ok_or("Windows' folder couldn't be found")?;
    let powershell = system.join("WindowsPowerShell").join("v1.0").join("powershell.exe");
    let output = Command::new(powershell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$s = Get-AuthenticodeSignature -LiteralPath $env:BACH_CHECK_FILE; \"$($s.Status)\"; \"$($s.SignerCertificate.Subject)\"",
        ])
        .env("BACH_CHECK_FILE", file)
        // A module path inherited from PowerShell 7 can hide the signature cmdlet.
        .env_remove("PSModulePath")
        .output()
        .map_err(|err| format!("couldn't ask Windows who signed the update: {err}"))?;
    parse(&String::from_utf8_lossy(&output.stdout))
}

/// The two lines the PowerShell command prints.
fn parse(text: &str) -> Result<Signature, String> {
    let mut lines = text.lines().map(str::trim);
    let status = lines.next().filter(|status| !status.is_empty()).ok_or("Windows didn't say who signed the update")?;
    Ok(Signature { status: status.to_string(), subject: lines.next().unwrap_or_default().to_string() })
}

/// Whether `signature` is a good one by `publisher`.
pub fn judge(signature: &Signature, publisher: &str) -> Result<(), String> {
    match signature.status.as_str() {
        "Valid" if bach_pack::manifest::extras::subject_names(&signature.subject, publisher) => Ok(()),
        "Valid" => Err(format!(
            "the update is signed by \"{}\", not by {publisher}, so it isn't installed",
            if signature.subject.is_empty() { "somebody unknown" } else { &signature.subject }
        )),
        "NotSigned" => Err(format!("the update isn't signed, and this app only takes updates signed by {publisher}")),
        other => Err(format!("Windows doesn't accept the update's signature ({other}), so it isn't installed")),
    }
}

/// Checks `file` against the app's `[updates] signer`, if it names one.
pub fn check(manifest: &bach_pack::manifest::Manifest, file: &Path) -> Result<(), String> {
    let Some(publisher) = manifest.updates.as_ref().and_then(|updates| updates.signer.as_deref()) else {
        return Ok(());
    };
    judge(&read(file)?, publisher)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signature(status: &str, subject: &str) -> Signature {
        Signature { status: status.into(), subject: subject.into() }
    }

    #[test]
    fn only_a_valid_signature_by_the_named_publisher_passes() {
        let good = signature("Valid", "CN=Example Inc., O=Example, C=DE");
        assert!(judge(&good, "Example Inc.").is_ok());
        assert!(judge(&good, "example inc.").is_ok(), "the case doesn't matter");
        assert!(judge(&good, "Example").is_ok(), "the organisation counts too");
        let other = judge(&good, "Somebody Else").unwrap_err();
        assert!(other.contains("Example Inc.") && other.contains("Somebody Else"), "{other}");
        // A name that is only part of the subject isn't the subject.
        assert!(judge(&good, "Example Inc").is_err());
        for status in ["NotSigned", "HashMismatch", "NotTrusted", "UnknownError"] {
            assert!(judge(&signature(status, "CN=Example Inc."), "Example Inc.").is_err(), "{status}");
        }
        assert!(judge(&signature("NotSigned", ""), "Example Inc.").unwrap_err().contains("isn't signed"));
    }

    #[test]
    fn what_powershell_prints_is_read_or_refused() {
        assert_eq!(parse("Valid\r\nCN=A\r\n").unwrap(), signature("Valid", "CN=A"));
        assert_eq!(parse("NotSigned\r\n\r\n").unwrap(), signature("NotSigned", ""));
        assert!(parse("").is_err());
    }

    #[test]
    fn windows_is_asked_for_real_signatures() {
        // A program Microsoft signs, a file that is nobody's, and the named publisher.
        let notepad = crate::system::folders::system_dir().unwrap().join("notepad.exe");
        // Windows' own files are catalog-signed on some editions; then Windows says NotSigned for the file itself.
        let found = read(&notepad).unwrap();
        if found.status == "Valid" {
            assert!(judge(&found, "Microsoft Windows").is_ok() || judge(&found, "Microsoft Corporation").is_ok(), "{found:?}");
            assert!(judge(&found, "Not Microsoft").is_err());
        }
        let plain = std::env::temp_dir().join(format!("bach-unsigned-{}.exe", std::process::id()));
        // This test program: a real, unsigned executable.
        std::fs::copy(std::env::current_exe().unwrap(), &plain).unwrap();
        let unsigned = read(&plain).unwrap();
        assert_eq!(unsigned.status, "NotSigned", "{unsigned:?}");
        assert!(judge(&unsigned, "Example Inc.").unwrap_err().contains("isn't signed"));
        // Something that isn't a program at all is no good either.
        std::fs::write(&plain, b"MZ not a program").unwrap();
        assert!(judge(&read(&plain).unwrap(), "Example Inc.").is_err());
        std::fs::remove_file(plain).unwrap();
    }

    #[test]
    fn no_signer_in_the_manifest_means_no_check() {
        let manifest = crate::package::Package::sample().manifest;
        assert!(check(&manifest, Path::new("does-not-exist.exe")).is_ok());
    }
}
