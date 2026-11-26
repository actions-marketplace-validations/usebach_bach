//! The signer's name out of a certificate: just enough DER to read the subject.
//! The bytes come from a download, so every length is checked and nothing panics.

/// The signer written the way Windows writes a subject: `CN=Example Inc., OU=ABCDE12345`,
/// the name without "Developer ID Application: " and without the team id in brackets.
pub(super) fn subject(certificate: &[u8]) -> Option<String> {
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
