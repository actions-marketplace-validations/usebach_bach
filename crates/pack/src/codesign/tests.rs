use super::*;

/// A throwaway certificate shaped like a Developer ID one: `UID`, `CN=Developer ID
/// Application: Müller, Inc. (ABCDE12345)`, `OU=ABCDE12345`, `O`, `C`, all UTF-8.
const CERTIFICATE: &[u8] = include_bytes!("testdata/developer-id.der");

fn der(tag: u8, contents: &[u8]) -> Vec<u8> {
    let length = u8::try_from(contents.len()).expect("a short test element");
    let mut element = if length < 0x80 { vec![tag, length] } else { vec![tag, 0x81, length] };
    element.extend_from_slice(contents);
    element
}

/// The smallest thing shaped like a certificate with this common name.
fn certificate_of(common_name: &str) -> Vec<u8> {
    let pair = der(0x30, &[der(0x06, &[0x55, 0x04, 0x03]), der(0x0c, common_name.as_bytes())].concat());
    let name = der(0x30, &der(0x31, &pair));
    let empty = der(0x30, &[]);
    let fields = [der(0x02, &[1]), empty.clone(), empty.clone(), empty, name].concat();
    der(0x30, &der(0x30, &fields))
}

#[test]
fn the_signer_is_read_from_the_certificate() {
    let signature = found(true, Some(CERTIFICATE));
    assert_eq!(signature.status, "Valid");
    assert_eq!(signature.subject, "CN=\"Müller, Inc.\", OU=ABCDE12345");
    assert!(signature.is_by("Müller, Inc."));
    assert!(signature.is_by("CN=\"Müller, Inc.\", OU=ABCDE12345"), "the whole subject pins the team");
    assert!(!signature.is_by("CN=\"Müller, Inc.\", OU=ZZZZZ99999"), "another team");
    assert!(!signature.is_by("Müller"), "a part of a name with a comma isn't the name");
    assert!(!signature.is_by("Somebody Else"));
}

#[test]
fn a_name_cant_add_pairs_to_the_subject() {
    for name in ["Mallory, O=Example Inc.", "Example Inc., Mallory GmbH", "Mallory\", O=Example Inc., X=\""] {
        let signature = found(true, Some(&certificate_of(&format!("Developer ID Application: {name} (ZZZZZ99999)"))));
        assert_eq!(signature.status, "Valid", "{name}");
        assert!(!signature.is_by("Example Inc."), "{name}: {}", signature.subject);
    }
}

#[test]
fn without_apples_requirement_a_name_counts_for_nothing() {
    let imitation = certificate_of("Developer ID Application: Example Inc. (ABCDE12345)");
    let refused = found(false, Some(&imitation));
    assert_eq!(refused, Signature { status: "NotTrusted".into(), subject: String::new() });
    assert!(!refused.is_by("Example Inc."));
    assert_eq!(found(false, None).status, "NotSigned", "unsigned or ad hoc");
    assert_eq!(found(true, None).status, "NotSigned", "no certificate, nobody to name");
}

#[test]
fn a_damaged_certificate_names_nobody() {
    for length in 0..CERTIFICATE.len() {
        // Cut anywhere, it is refused and nothing panics.
        assert_eq!(found(true, Some(&CERTIFICATE[..length])).status, "NotTrusted", "{length}");
    }
    assert_eq!(found(true, Some(&[0x30, 0x84, 0xff, 0xff, 0xff, 0xff])).status, "NotTrusted", "a length past the end");
}

#[test]
fn the_requirement_asks_for_apples_developer_id() {
    for part in ["anchor apple generic", "1.2.840.113635.100.6.2.6", "1.2.840.113635.100.6.1.13"] {
        assert!(DEVELOPER_ID.contains(part), "{part}");
    }
}

/// These need the programs macOS signs with.
#[cfg(target_os = "macos")]
mod on_a_mac {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::*;
    use crate::macapp;

    fn run(command: &mut Command) {
        let output = command.output().unwrap();
        assert!(output.status.success(), "{command:?}: {}", String::from_utf8_lossy(&output.stderr));
    }

    fn zipped_app(base: &Path) -> (PathBuf, PathBuf) {
        let app = base.join("Install Hello.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::create_dir_all(app.join("Contents/Resources")).unwrap();
        std::fs::copy("/usr/bin/true", app.join(macapp::RUNTIME)).unwrap();
        std::fs::write(app.join(macapp::PAYLOAD), b"payload").unwrap();
        // codesign takes a folder for an app only when it names its program.
        let plist = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict>\
            <key>CFBundleExecutable</key><string>bach-setup</string>\
            <key>CFBundleIdentifier</key><string>app.bach.signer-test</string>\
            <key>CFBundlePackageType</key><string>APPL</string></dict></plist>";
        std::fs::write(app.join("Contents/Info.plist"), plist).unwrap();
        (app, base.join("Hello.zip"))
    }

    fn zip(app: &Path, into: &Path) {
        run(Command::new("/usr/bin/ditto").args(["-c", "-k", "--keepParent"]).arg(app).arg(into));
    }

    #[test]
    fn codesign_can_read_the_requirement() {
        run(Command::new("/usr/bin/csreq").arg(format!("-r={DEVELOPER_ID}")).arg("-t"));
    }

    #[test]
    fn an_ad_hoc_setup_app_names_nobody() {
        let base = std::env::temp_dir().join(format!("bach-adhoc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (app, file) = zipped_app(&base);
        run(Command::new("/usr/bin/codesign").args(["--force", "--sign", "-"]).arg(&app));
        zip(&app, &file);
        assert_eq!(read(&file).unwrap().status, "NotSigned");
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// Run by `scripts/test-macos-signer.sh`, which makes the certificate in a keychain of its own.
    #[test]
    #[ignore = "needs scripts/test-macos-signer.sh"]
    fn a_self_signed_publisher_name_is_refused() {
        let file = PathBuf::from(std::env::var_os("BACH_SELF_SIGNED_APP").expect("BACH_SELF_SIGNED_APP"));
        let signature = read(&file).unwrap();
        assert_eq!(signature.status, "NotTrusted", "{signature:?}");
        assert!(!signature.is_by("Example Inc."));
    }

    /// The other side: a real Developer ID app passes. The script names one that is installed.
    #[test]
    #[ignore = "needs scripts/test-macos-signer.sh and an app with a Developer ID"]
    fn a_real_developer_id_is_accepted() {
        let app = PathBuf::from(std::env::var_os("BACH_DEVELOPER_ID_APP").expect("BACH_DEVELOPER_ID_APP"));
        let work = std::env::temp_dir().join(format!("bach-developer-id-{}", std::process::id()));
        std::fs::create_dir_all(&work).unwrap();
        let signature = of_app(&app, &work.join("certificates")).unwrap();
        std::fs::remove_dir_all(&work).unwrap();
        assert_eq!(signature.status, "Valid", "{signature:?}");
        let publisher = std::env::var("BACH_DEVELOPER_ID_NAME").expect("BACH_DEVELOPER_ID_NAME");
        assert!(signature.is_by(&publisher), "{signature:?}");
    }
}
