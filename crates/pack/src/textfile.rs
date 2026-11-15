//! Reading text files people write by hand or with their shell.
//!
//! `bach key > key.txt` in Windows PowerShell writes UTF-16 with a byte order
//! mark, newer PowerShell and Notepad write UTF-8 with one, and a license or a
//! `bach.toml` saved from an editor may be any of these. Every file Bach reads
//! as text goes through here, so none of them fails with "stream did not
//! contain valid UTF-8" or leaves a stray `U+FEFF` in an access key.

use std::io;
use std::path::Path;

/// `bytes` as text: UTF-8, or UTF-16 (either byte order) when it starts with
/// that encoding's byte order mark. A UTF-8 byte order mark is dropped.
pub fn decode(bytes: &[u8]) -> io::Result<String> {
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, format!("the file isn't valid {what} text"));
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, u16::from_le_bytes).ok_or_else(|| bad("UTF-16"));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, u16::from_be_bytes).ok_or_else(|| bad("UTF-16"));
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8(bytes.to_vec()).map_err(|_| bad("UTF-8"))
}

fn utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> Option<String> {
    let (pairs, rest) = bytes.as_chunks::<2>();
    if !rest.is_empty() {
        return None;
    }
    let units: Vec<u16> = pairs.iter().map(|pair| unit(*pair)).collect();
    String::from_utf16(&units).ok()
}

/// [`decode`] for a file.
pub fn read(path: &Path) -> io::Result<String> {
    decode(&std::fs::read(path)?)
}

/// The first line that isn't empty, trimmed: what a key file holds, whether
/// it has one key or a list from `bach key --count`.
pub fn first_line(text: &str) -> Option<String> {
    text.lines().map(str::trim).find(|line| !line.is_empty()).map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_encoding_an_editor_or_shell_writes_reads_back() {
        assert_eq!(decode(b"plain").unwrap(), "plain");
        assert_eq!(decode(b"\xEF\xBB\xBFwith mark").unwrap(), "with mark");
        let utf16le: Vec<u8> = [0xFF, 0xFE].into_iter().chain("K7QF\r\nÄ".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        assert_eq!(decode(&utf16le).unwrap(), "K7QF\r\nÄ");
        let utf16be: Vec<u8> = [0xFE, 0xFF].into_iter().chain("K7QF".encode_utf16().flat_map(u16::to_be_bytes)).collect();
        assert_eq!(decode(&utf16be).unwrap(), "K7QF");
        assert!(decode(&[0xFF, 0xFE, 0x41]).is_err(), "half a UTF-16 unit");
        assert!(decode(&[0x80, 0x81]).is_err(), "neither encoding");
        assert_eq!(decode(b"").unwrap(), "");
    }

    #[test]
    fn a_key_file_gives_its_first_line() {
        assert_eq!(first_line("\nK7QF-3MWD  \r\nsecond"), Some("K7QF-3MWD".into()));
        assert_eq!(first_line("  \n"), None);
    }
}
