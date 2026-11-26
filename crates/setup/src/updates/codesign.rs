//! Who signed a downloaded update, as macOS sees it.
//!
//! `codesign --verify` alone says that a signature fits the files, not who made
//! it: it takes a self-signed certificate as its own anchor, and such a
//! certificate can carry any name. So the verify call also demands (`-R`) a
//! Developer ID Application certificate that Apple issued. The publisher's name
//! is then read from that certificate itself, never from the text `codesign -d`
//! prints: that repeats the file's own path and identifier, which can be made to
//! hold lines that look like a signer.

use super::authenticode::Signature;

/// Apple's requirement for Developer ID programs (TN3127), for any team and
/// identifier: the chain ends at Apple, passes the Developer ID authority and
/// starts at a Developer ID Application certificate. It is a constant, so
/// nothing from a download or a `bach.toml` is ever part of it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const DEVELOPER_ID: &str =
    "anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists";

/// Reads the code signature of the Setup app inside a downloaded `.dmg` or
/// `.zip`. It is `Valid` only with a Developer ID, whose name is the signer. An
/// unsigned or ad hoc signature is `NotSigned`, because it names nobody.
#[cfg(target_os = "macos")]
pub fn read(file: &std::path::Path) -> Result<Signature, String> {
    let scratch = std::env::temp_dir().join(format!(
        "bach-verify-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
    ));
    let result = (|| {
        let program = super::macsetup::unpack(file, &scratch.join("download"))?;
        let app =
            program.ancestors().find(|part| part.extension().is_some_and(|ext| ext == "app")).ok_or("the download holds no setup app")?;
        of_app(app, &scratch.join("certificates"))
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

/// The signature of an unpacked app. `certificates` must not exist yet: nothing
/// the download brought along may pass for the certificate codesign writes there.
#[cfg(target_os = "macos")]
fn of_app(app: &std::path::Path, certificates: &std::path::Path) -> Result<Signature, String> {
    use std::process::Command;

    let ask = |err: std::io::Error| format!("couldn't ask macOS who signed the update: {err}");
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
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn found(accepted: bool, leaf: Option<&[u8]>) -> Signature {
    let Some(leaf) = leaf else { return Signature { status: "NotSigned".into(), subject: String::new() } };
    match subject(leaf).filter(|_| accepted) {
        Some(subject) => Signature { status: "Valid".into(), subject },
        None => Signature { status: "NotTrusted".into(), subject: String::new() },
    }
}

/// The signer written the way Windows writes a subject: `CN=Example Inc., OU=ABCDE12345`,
/// the name without "Developer ID Application: " and without the team id in brackets.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn subject(certificate: &[u8]) -> Option<String> {
    let names = subject_of(certificate)?;
    let value = |kind: u8| names.iter().find(|(found, _)| *found == kind).map(|(_, value)| value.as_str());
    let common = value(COMMON_NAME)?;
    let name = common.strip_prefix("Developer ID Application: ").unwrap_or(common);
    let name = name.rsplit_once(" (").filter(|(_, tail)| tail.ends_with(')')).map_or(name, |(head, _)| head);
    let team = value(UNIT).map(|team| format!(", OU={}", quoted(team))).unwrap_or_default();
    Some(format!("CN={}{team}", quoted(name)))
}

/// A value with a comma (`Example, Inc.`) is quoted, so it stays one value and
/// can't pass for further pairs of the subject.
fn quoted(value: &str) -> String {
    if value.contains([',', '"']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// The last byte of the attribute ids 2.5.4.3 (common name) and 2.5.4.11 (unit).
const COMMON_NAME: u8 = 3;
const UNIT: u8 = 11;

/// The X.520 attributes of an X.509 certificate's subject, as `(last byte of
/// 2.5.4.n, text)`. `None` when this isn't a certificate.
fn subject_of(certificate: &[u8]) -> Option<Vec<(u8, String)>> {
    let (_, certificate, _) = element(certificate)?;
    let (_, mut fields, _) = element(certificate)?;
    // The version is optional; serial, algorithm, issuer and validity come before the subject.
    if fields.first() == Some(&0xa0) {
        fields = element(fields)?.2;
    }
    for _ in 0..4 {
        fields = element(fields)?.2;
    }
    let (_, mut sets, _) = element(fields)?;
    let mut names = Vec::new();
    while !sets.is_empty() {
        let (_, mut set, rest) = element(sets)?;
        sets = rest;
        while !set.is_empty() {
            let (_, pair, rest) = element(set)?;
            set = rest;
            let (_, id, value) = element(pair)?;
            let (tag, value, _) = element(value)?;
            if let ([0x55, 0x04, kind], Some(text)) = (id, text(tag, value)) {
                names.push((*kind, text));
            }
        }
    }
    Some(names)
}

/// One DER element: its tag, its contents and what follows it.
fn element(der: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = der.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (length, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let count = usize::from(first & 0x7f);
        if count == 0 || count > 4 {
            return None;
        }
        let length = rest.get(..count)?.iter().fold(0usize, |length, byte| (length << 8) | usize::from(*byte));
        (length, rest.get(count..)?)
    };
    Some((tag, rest.get(..length)?, rest.get(length..)?))
}

/// The string types a certificate's names come in.
fn text(tag: u8, value: &[u8]) -> Option<String> {
    match tag {
        // UTF8String, PrintableString, TeletexString (ASCII in practice), IA5String
        0x0c | 0x13 | 0x14 | 0x16 => std::str::from_utf8(value).ok().map(str::to_string),
        // BMPString: UTF-16, big end first
        0x1e => {
            let (pairs, rest) = value.as_chunks::<2>();
            let units: Vec<u16> = pairs.iter().map(|pair| u16::from_be_bytes(*pair)).collect();
            String::from_utf16(&units).ok().filter(|_| rest.is_empty())
        }
        _ => None,
    }
}
