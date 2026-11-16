//! The archive section of a setup file, read once and checked on the way.
//!
//! The builder records the SHA-256 of the archive's bytes. Reading them to
//! check that before unpacking would read a large setup file twice, and leave
//! the progress bar standing still meanwhile. Instead the bytes are hashed as
//! the unpacker consumes them, and compared before anything is moved into
//! place: until then every file is only staged beside its target.

use std::cell::RefCell;
use std::fs::File;
use std::io::{self, Read, Take};
use std::rc::Rc;

use sha2::{Digest, Sha256};

/// What the reader and its owner share: the bytes left in the section, and
/// the hash of those already read.
struct State {
    inner: Take<File>,
    hasher: Sha256,
}

/// Reads the section and hashes what it hands out.
pub struct HashingSection {
    state: Rc<RefCell<State>>,
}

/// Held by whoever wants the verdict once the unpacker is done with the reader.
pub struct Verdict {
    state: Rc<RefCell<State>>,
}

/// A reader over `section` and the way to check it afterwards.
pub fn hashing(section: Take<File>) -> (HashingSection, Verdict) {
    let state = Rc::new(RefCell::new(State { inner: section, hasher: Sha256::new() }));
    (HashingSection { state: state.clone() }, Verdict { state })
}

impl Read for HashingSection {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut state = self.state.borrow_mut();
        let read = state.inner.read(buf)?;
        state.hasher.update(&buf[..read]);
        Ok(read)
    }
}

impl Verdict {
    /// Whether the whole section hashes to `expected` (hex, any case). The
    /// part the unpacker didn't need (the end of a zstd frame, padding) is read
    /// here, so the hash covers exactly what the builder hashed. `Err` is a
    /// section that can't be read to its end.
    pub fn matches(self, expected: &str) -> io::Result<bool> {
        let mut state = self.state.borrow_mut();
        let mut buffer = vec![0u8; 256 * 1024];
        loop {
            let read = match state.inner.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            };
            state.hasher.update(&buffer[..read]);
        }
        if state.inner.limit() != 0 {
            // The file ended before the section did.
            return Ok(false);
        }
        let actual = bach_pack::protect::hex(&state.hasher.clone().finalize());
        Ok(actual == expected.to_ascii_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom, Write};

    /// The section, and the file to delete once it is closed (Windows won't delete it before).
    fn section_of(name: &str, bytes: &[u8], len: u64) -> (Take<File>, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("bach-section-{name}-{}", std::process::id()));
        let mut file = File::create(&path).unwrap();
        file.write_all(b"HEAD").unwrap();
        file.write_all(bytes).unwrap();
        drop(file);
        let mut file = File::open(&path).unwrap();
        file.seek(SeekFrom::Start(4)).unwrap();
        (file.take(len), path)
    }

    fn sha(bytes: &[u8]) -> String {
        bach_pack::protect::sha256_hex(bytes).unwrap()
    }

    #[test]
    fn a_partly_read_section_is_still_checked_whole() {
        let bytes: Vec<u8> = (0..300_000u32).map(|i| (i % 253) as u8).collect();
        let (section, path) = section_of("part", &bytes, bytes.len() as u64);
        let (mut reader, verdict) = hashing(section);
        let mut first = [0u8; 1000];
        reader.read_exact(&mut first).unwrap();
        assert_eq!(&first[..], &bytes[..1000]);
        assert!(verdict.matches(&sha(&bytes).to_uppercase()).unwrap());
        drop(reader);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_changed_or_cut_off_section_does_not_match() {
        let bytes = vec![7u8; 50_000];
        let (section, path) = section_of("changed", &bytes, bytes.len() as u64);
        let (reader, verdict) = hashing(section);
        let mut other = bytes.clone();
        other[100] ^= 1;
        assert!(!verdict.matches(&sha(&other)).unwrap());
        drop(reader);
        let _ = std::fs::remove_file(path);

        // The file is shorter than the section says.
        let (section, path) = section_of("cut", &bytes, bytes.len() as u64 + 10);
        let (reader, verdict) = hashing(section);
        assert!(!verdict.matches(&sha(&bytes)).unwrap());
        drop(reader);
        let _ = std::fs::remove_file(path);
    }
}
