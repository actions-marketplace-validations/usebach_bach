//! Telling whether a Windows program carries a signature, without Windows.
//!
//! A signed `.exe` or `.dll` has an Authenticode block appended to it, and
//! the PE header's "certificate table" entry says where. That entry is all
//! this reads: it doesn't check the signature (Windows does that when the
//! program runs), it only says whether one is there. That is enough for the
//! builder to warn about the unsigned programs inside a signed setup file.
//!
//! The file is untrusted: every offset is checked against the bytes read and
//! the file's length, and anything that isn't a PE file is simply "not one".

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const DOS_MAGIC: &[u8; 2] = b"MZ";
const PE_MAGIC: &[u8; 4] = b"PE\0\0";
/// A PE header is never this far into a real program; refuse a claim of more.
const MAX_HEADER_OFFSET: u32 = 1 << 20;
/// `IMAGE_NT_HEADERS`: the signature, then the 20-byte file header.
const FILE_HEADER_END: usize = 4 + 20;
const PE32: u16 = 0x10b;
const PE32_PLUS: u16 = 0x20b;
/// The certificate table is the fifth data directory.
const CERTIFICATE_DIRECTORY: usize = 4;

/// What a PE file says about its signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signature {
    /// An Authenticode block is attached.
    Signed,
    /// A program or library with no signature of its own.
    Unsigned,
}

/// Reads the signature entry of the PE file at `path`. `None` when the file
/// isn't a Windows program or library (or can't be read).
pub fn signature(path: &Path) -> Option<Signature> {
    let mut file = std::fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let mut dos = [0u8; 0x40];
    file.read_exact(&mut dos).ok()?;
    if &dos[..2] != DOS_MAGIC {
        return None;
    }
    let header_at = u32::from_le_bytes(dos[0x3c..0x40].try_into().ok()?);
    if header_at > MAX_HEADER_OFFSET {
        return None;
    }
    // The NT headers, with room for the optional header and its directories.
    let mut headers = vec![0u8; FILE_HEADER_END + 240];
    file.seek(SeekFrom::Start(u64::from(header_at))).ok()?;
    let read = read_up_to(&mut file, &mut headers);
    headers.truncate(read);
    parse(&headers, length)
}

/// `headers` start at the "PE\0\0" signature; `length` is the whole file's.
fn parse(headers: &[u8], length: u64) -> Option<Signature> {
    if headers.get(..4)? != PE_MAGIC {
        return None;
    }
    let optional = headers.get(FILE_HEADER_END..)?;
    // Where the data directories start, and where their count is.
    let (count_at, directories_at) = match u16::from_le_bytes(optional.get(..2)?.try_into().ok()?) {
        PE32 => (92, 96),
        PE32_PLUS => (108, 112),
        _ => return None,
    };
    let count = u32::from_le_bytes(optional.get(count_at..count_at + 4)?.try_into().ok()?);
    if count as usize <= CERTIFICATE_DIRECTORY {
        return Some(Signature::Unsigned);
    }
    let entry = directories_at + CERTIFICATE_DIRECTORY * 8;
    let offset = u32::from_le_bytes(optional.get(entry..entry + 4)?.try_into().ok()?);
    let size = u32::from_le_bytes(optional.get(entry + 4..entry + 8)?.try_into().ok()?);
    // The block has to be inside the file, or the entry is noise.
    let inside = u64::from(offset) + u64::from(size) <= length;
    Some(if offset != 0 && size != 0 && inside { Signature::Signed } else { Signature::Unsigned })
}

/// Reads until `buffer` is full or the file ends; returns how many bytes.
fn read_up_to(file: &mut std::fs::File, buffer: &mut [u8]) -> usize {
    let mut total = 0;
    while total < buffer.len() {
        match file.read(&mut buffer[total..]) {
            Ok(0) | Err(_) => break,
            Ok(read) => total += read,
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal PE file: DOS header, NT headers, an optional header of the
    /// given kind and a certificate entry (`offset`, `size`). `directories`
    /// is how many data directories it declares.
    fn pe(plus: bool, directories: u32, offset: u32, size: u32, total: usize) -> Vec<u8> {
        let header_at = 0x80usize;
        let mut bytes = vec![0u8; total.max(header_at + FILE_HEADER_END + 240)];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&(header_at as u32).to_le_bytes());
        bytes[header_at..header_at + 4].copy_from_slice(b"PE\0\0");
        let optional = header_at + FILE_HEADER_END;
        let (magic, count_at, directories_at) = if plus { (PE32_PLUS, 108, 112) } else { (PE32, 92, 96) };
        bytes[optional..optional + 2].copy_from_slice(&magic.to_le_bytes());
        bytes[optional + count_at..optional + count_at + 4].copy_from_slice(&directories.to_le_bytes());
        let entry = optional + directories_at + CERTIFICATE_DIRECTORY * 8;
        bytes[entry..entry + 4].copy_from_slice(&offset.to_le_bytes());
        bytes[entry + 4..entry + 8].copy_from_slice(&size.to_le_bytes());
        bytes
    }

    fn check(name: &str, bytes: &[u8]) -> Option<Signature> {
        let path = std::env::temp_dir().join(format!("bach-pe-{name}-{}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let found = signature(&path);
        std::fs::remove_file(&path).unwrap();
        found
    }

    #[test]
    fn a_signed_and_an_unsigned_program_are_told_apart() {
        for plus in [false, true] {
            let name = if plus { "plus" } else { "pe32" };
            // The block is at the end of a 4 KB file, 1 KB long.
            assert_eq!(check(name, &pe(plus, 16, 3072, 1024, 4096)), Some(Signature::Signed), "{name}");
            assert_eq!(check(name, &pe(plus, 16, 0, 0, 4096)), Some(Signature::Unsigned), "{name}");
        }
    }

    #[test]
    fn an_entry_that_points_outside_the_file_is_not_a_signature() {
        assert_eq!(check("outside", &pe(true, 16, 3072, 4096, 4096)), Some(Signature::Unsigned), "runs past the end");
        assert_eq!(check("wrap", &pe(true, 16, u32::MAX, u32::MAX, 4096)), Some(Signature::Unsigned), "the sum can't wrap");
        assert_eq!(check("no-size", &pe(true, 16, 3072, 0, 4096)), Some(Signature::Unsigned));
    }

    #[test]
    fn a_program_with_too_few_directories_has_no_certificate_entry() {
        assert_eq!(check("few", &pe(true, 4, 3072, 1024, 4096)), Some(Signature::Unsigned));
        assert_eq!(check("none", &pe(true, 0, 3072, 1024, 4096)), Some(Signature::Unsigned));
    }

    #[test]
    fn anything_that_is_not_a_pe_file_is_not_one() {
        assert_eq!(check("empty", b""), None);
        assert_eq!(check("fake", b"MZ"), None, "the two bytes the tests and examples use");
        assert_eq!(check("text", b"hello, this is not a program at all, just text\n\n\n\n\n\n\n\n\n\n\n\n"), None);
        // A DOS header whose PE offset points nowhere, or past what is sane.
        let mut far = vec![0u8; 0x200];
        far[..2].copy_from_slice(b"MZ");
        far[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(check("far", &far), None);
        let mut missing = vec![0u8; 0x200];
        missing[..2].copy_from_slice(b"MZ");
        missing[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        assert_eq!(check("no-pe", &missing), None, "no PE signature at the offset");
        // An unknown optional header (a ROM image, say).
        let mut odd = pe(true, 16, 3072, 1024, 4096);
        odd[0x80 + FILE_HEADER_END..0x80 + FILE_HEADER_END + 2].copy_from_slice(&0x107u16.to_le_bytes());
        assert_eq!(check("rom", &odd), None);
        assert_eq!(signature(Path::new("no/such/file.exe")), None);
    }

    #[test]
    fn a_file_cut_short_is_never_a_panic() {
        let whole = pe(true, 16, 3072, 1024, 4096);
        for cut in (0..whole.len()).step_by(7) {
            // Whatever it says, it says it without reading past the end.
            let _ = check("cut", &whole[..cut]);
        }
    }
}
