//! Whether installed Macs will take a Setup app as an update: the question their
//! update check asks of `[updates] signer` (`bach_pack::codesign`), asked by
//! `bach build` and `bach sign` before the release rather than found out after it.
//!
//! One `signer` serves Windows and macOS, so a publisher who signs only for
//! Windows has to hear that the Mac file would be refused.

use std::path::Path;

use anyhow::{bail, Result};
use bach_pack::codesign::{self, Signature};

/// What to do about a Setup app that installed copies would refuse.
const FIX: &str = "set [macos.signing] identity to that publisher's Developer ID Application certificate and build with --sign, \
                   or change [updates] signer";

/// A build that didn't sign the Setup app with anyone's certificate.
pub fn warn_unsigned(publisher: &str) {
    crate::output::warnings(&[format!(
        "[updates] signer is {publisher}, but this Setup app has no Developer ID signature, so installed Macs will refuse it as an \
         update; before you publish it, {FIX}"
    )]);
}

/// The Setup app `bach build` has just signed, at `app`. An error when installed
/// copies would refuse it.
pub fn check_app(app: &Path, publisher: &str) -> Result<()> {
    let certificates = codesign::scratch();
    let found = codesign::of_app(app, &certificates);
    let _ = std::fs::remove_dir_all(&certificates);
    let signature = found.map_err(anyhow::Error::msg)?;
    judge(&signature, publisher, "the Setup app")
}

/// Whether `file` is a macOS setup file: a `.dmg`, or a `.zip` with a Setup app in it.
pub fn is_setup_file(file: &Path) -> bool {
    let extension = file.extension().map(|extension| extension.to_string_lossy().to_ascii_lowercase());
    match extension.as_deref() {
        Some("dmg") => true,
        Some("zip") => crate::macfile::zip_holds_payload(file),
        _ => false,
    }
}

/// For `bach sign`: the Setup app inside the `.dmg` or `.zip`, which is what an
/// installed copy looks at. Where nobody can look (not a Mac, or the file can't
/// be opened), that is a warning, not a failed signing.
pub fn check_file(file: &Path, publisher: &str) -> Result<()> {
    let found = if cfg!(target_os = "macos") { codesign::read(file) } else { Err("only a Mac can read it".into()) };
    match found {
        Ok(signature) => judge(&signature, publisher, &format!("the Setup app in {}", file.display())),
        Err(why) => {
            crate::output::warnings(&[format!(
                "couldn't check the signature of the Setup app in {} ({why}); installed Macs take it as an update only with {publisher}'s \
                 Developer ID Application signature on the app itself",
                file.display()
            )]);
            Ok(())
        }
    }
}

/// `what` with this signature against `[updates] signer`.
fn judge(signature: &Signature, publisher: &str, what: &str) -> Result<()> {
    if signature.is_by(publisher) {
        say!("Signature checked: {what} has a Developer ID of {publisher}");
        return Ok(());
    }
    let why = match signature.status.as_str() {
        "Valid" => format!("is signed by \"{}\"", signature.subject),
        "NotSigned" => "has no certificate (it is unsigned or signed ad hoc)".to_string(),
        _ => "isn't signed with a Developer ID Application certificate that Apple issued".to_string(),
    };
    bail!("{what} {why}, so installed Macs would refuse it as an update from {publisher} ([updates] signer); {FIX}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(status: &str, subject: &str) -> String {
        let signature = Signature { status: status.into(), subject: subject.into() };
        judge(&signature, "Example Inc.", "the Setup app").unwrap_err().to_string()
    }

    #[test]
    fn only_a_developer_id_of_the_named_publisher_passes() {
        let good = Signature { status: "Valid".into(), subject: "CN=Example Inc., OU=ABCDE12345".into() };
        assert!(judge(&good, "Example Inc.", "the Setup app").is_ok());
        assert!(judge(&good, "CN=Example Inc., OU=ABCDE12345", "the Setup app").is_ok());
        // A whole subject written for the Windows certificate doesn't fit the Mac one.
        assert!(judge(&good, "CN=Example Inc., O=Example Inc., C=DE", "the Setup app").is_err());

        let other = refused("Valid", "CN=Somebody Else, OU=ZZZZZ99999");
        assert!(other.contains("is signed by \"CN=Somebody Else") && other.contains("Example Inc."), "{other}");
        assert!(refused("NotSigned", "").contains("ad hoc"));
        assert!(refused("NotTrusted", "").contains("that Apple issued"));
        for sentence in [other, refused("NotSigned", ""), refused("NotTrusted", "")] {
            assert!(
                sentence.contains("[updates] signer") && sentence.contains("[macos.signing]") && !sentence.contains('\n'),
                "{sentence}"
            );
            assert!(!sentence.contains("  "), "{sentence}");
        }
    }

    #[test]
    fn macos_setup_files_are_told_apart_from_other_files() {
        assert!(is_setup_file(Path::new("dist/Hello-1.0.0.DMG")));
        assert!(!is_setup_file(Path::new("dist/Hello-Setup-1.0.0.exe")));
        // A zip that isn't there, or holds no Setup app, is somebody's other zip.
        assert!(!is_setup_file(Path::new("no-such-file.zip")));
    }
}
