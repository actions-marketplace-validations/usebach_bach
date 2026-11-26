//! Who signed a macOS Setup app, and whether that is the publisher
//! `[updates] signer` names. The installed app asks this of a downloaded update,
//! and `bach build` asks it of what it just signed, so both give one answer.
//!
//! `codesign --verify` alone says that a signature fits the files, not who made
//! it: it takes a self-signed certificate as its own anchor, and such a
//! certificate can carry any name. So the verify call also demands (`-R`) a
//! Developer ID Application certificate that Apple issued. The publisher's name
//! is then read from that certificate itself, never from the text `codesign -d`
//! prints: that repeats the file's own path and identifier, which can be made to
//! hold lines that look like a signer.

mod certificate;

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::manifest::extras::subject_names;

/// What the system says about a file's signature.
#[derive(Debug, PartialEq, Eq)]
pub struct Signature {
    /// `Valid`, `NotSigned`, `HashMismatch`, `NotTrusted`, ...
    pub status: String,
    /// The signing certificate's subject, like `CN=Example Inc., O=Example`.
    pub subject: String,
}

impl Signature {
    /// Whether this is a good signature by the publisher `[updates] signer` names.
    pub fn is_by(&self, publisher: &str) -> bool {
        self.status == "Valid" && subject_names(&self.subject, publisher)
    }
}

/// Apple's requirement for Developer ID programs (TN3127), for any team and
/// identifier: the chain ends at Apple, passes the Developer ID authority and
/// starts at a Developer ID Application certificate. It is a constant, so
/// nothing from a download or a `bach.toml` is ever part of it.
const DEVELOPER_ID: &str =
    "anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists";

/// Reads the code signature of the Setup app inside a `.dmg` or `.zip` (only a
/// Mac can). It is `Valid` only with a Developer ID, whose name is the signer. An
/// unsigned or ad hoc signature is `NotSigned`, because it names nobody.
pub fn read(file: &Path) -> Result<Signature, String> {
    let scratch = scratch();
    let result = (|| {
        let program = crate::macapp::unpack::unpack(file, &scratch.join("download"))?;
        let app =
            program.ancestors().find(|part| part.extension().is_some_and(|ext| ext == "app")).ok_or("the download holds no setup app")?;
        of_app(app, &scratch.join("certificates"))
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

/// A name nothing in the temporary folder has yet, for what a check unpacks.
pub fn scratch() -> PathBuf {
    std::env::temp_dir().join(format!(
        "bach-verify-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
    ))
}

/// The signature of an unpacked app. `certificates` must not exist yet: nothing
/// the download brought along may pass for the certificate codesign writes there.
pub fn of_app(app: &Path, certificates: &Path) -> Result<Signature, String> {
    let ask = |err: std::io::Error| format!("couldn't ask macOS who signed the Setup app: {err}");
    let accepted = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(format!("-R={DEVELOPER_ID}"))
        .arg(app)
        .output()
        .map_err(ask)?
        .status
        .success();
    std::fs::create_dir(certificates).map_err(ask)?;
    let mut extract = std::ffi::OsString::from("--extract-certificates=");
    extract.push(certificates.join("cert"));
    // An unsigned or ad hoc app has no certificate, so no file appears.
    Command::new("/usr/bin/codesign").arg("-d").arg(extract).arg(app).output().map_err(ask)?;
    // Number 0 is the certificate that signed.
    let leaf = std::fs::read(certificates.join("cert0")).ok();
    Ok(found(accepted, leaf.as_deref()))
}

/// What the two codesign calls add up to: `accepted` is the verify call with the
/// requirement, `leaf` the signing certificate in DER, if there is one.
fn found(accepted: bool, leaf: Option<&[u8]>) -> Signature {
    let Some(leaf) = leaf else { return Signature { status: "NotSigned".into(), subject: String::new() } };
    match certificate::subject(leaf).filter(|_| accepted) {
        Some(subject) => Signature { status: "Valid".into(), subject },
        None => Signature { status: "NotTrusted".into(), subject: String::new() },
    }
}

#[cfg(test)]
mod tests;
