//! Binary deltas: a changed file stored as what differs from its earlier
//! version, so an update patch for a big program carries a few kilobytes
//! instead of the whole program.
//!
//! This is zstd's patch mode: the earlier file is handed to the compressor as a
//! prefix it may refer back to, with a window as large as both files together and
//! long-distance matching on, and the decompressor is given the same file. A change
//! that moves a block of code a few kilobytes along still matches almost all of
//! it. The delta is only ever applied to the exact file it was made from: the archive
//! records that file's SHA-256, and the result is checked against the SHA-256 in the
//! manifest like any other file.

use std::io::{self, Read, Write};

use zstd::stream::raw::{Decoder as RawDecoder, Encoder as RawEncoder};
use zstd::zstd_safe::{CParameter, DParameter};

/// The biggest earlier file a delta is made from or applied to. It is held in
/// memory (compressing and expanding both need all of it at hand), so a bigger
/// one goes into the patch whole.
pub const MAX_BASE: u64 = 1 << 29;

/// The biggest new file a delta is made for: the window has to cover both files,
/// and zstd's windows end at 2 GiB.
pub const MAX_NEW: u64 = 1 << 30;

/// zstd's level for deltas: patches are built once and applied often.
const LEVEL: i32 = 19;

/// The window (as a power of two) that covers the earlier file and the new one.
fn window_log(base: u64, new: u64) -> u32 {
    let total = base.saturating_add(new).max(1);
    (u64::BITS - (total - 1).leading_zeros()).clamp(10, 31)
}

/// A writer that gives up once more than `limit` bytes were written to it.
struct Limited {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for Limited {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.bytes.len().saturating_add(buf.len()) > self.limit {
            return Err(io::Error::other("the delta is not smaller than the file"));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The delta that turns `base` into the `new_size` bytes `new` gives, or None
/// when it would be more than `limit` bytes (a file that changed so much that the
/// delta saves nothing), or a file is too big for a delta (see [`MAX_BASE`]).
pub fn encode(base: &[u8], new: &mut dyn Read, new_size: u64, limit: usize) -> io::Result<Option<Vec<u8>>> {
    if base.len() as u64 > MAX_BASE || new_size > MAX_NEW || base.is_empty() {
        return Ok(None);
    }
    let mut raw = RawEncoder::with_ref_prefix(LEVEL, base)?;
    raw.set_parameter(CParameter::WindowLog(window_log(base.len() as u64, new_size)))?;
    raw.set_parameter(CParameter::EnableLongDistanceMatching(true))?;
    raw.set_parameter(CParameter::ChecksumFlag(true))?;
    let mut encoder = zstd::stream::write::Encoder::with_encoder(Limited { bytes: Vec::new(), limit }, raw);
    match io::copy(new, &mut encoder).and_then(|_| encoder.finish()) {
        Ok(limited) => Ok(Some(limited.bytes)),
        Err(err) if err.to_string().contains("not smaller") => Ok(None),
        Err(err) => Err(err),
    }
}

/// A reader over the new file: `delta` expanded against `base`. `new_size` is the
/// size the file will have, which the window is sized by.
pub fn decode<'a, R: io::BufRead + 'a>(base: &'a [u8], new_size: u64, delta: R) -> io::Result<impl Read + 'a> {
    let mut raw = RawDecoder::with_ref_prefix(base)?;
    raw.set_parameter(DParameter::WindowLogMax(window_log(base.len() as u64, new_size)))?;
    let reader = zstd::stream::zio::Reader::new(delta, raw);
    Ok(reader)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for a program: structure that repeats, so it compresses a little,
    /// but mostly bytes that don't.
    fn program(len: usize, seed: u32) -> Vec<u8> {
        let mut state = seed;
        (0..len)
            .map(|index| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                if index % 7 == 0 {
                    (index % 251) as u8
                } else {
                    (state >> 24) as u8
                }
            })
            .collect()
    }

    fn apply(base: &[u8], new_len: u64, delta: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        decode(base, new_len, io::BufReader::new(delta)).unwrap().read_to_end(&mut out).unwrap();
        out
    }

    #[test]
    fn a_small_change_to_a_big_file_is_a_small_delta() {
        let old = program(2_000_000, 7);
        let mut new = old.clone();
        // A few edited bytes, a block inserted near the front (everything after it moves) and one near the end.
        new[1000] ^= 0xFF;
        new.splice(50_000..50_000, program(300, 9));
        new.splice(1_500_000..1_500_020, program(64, 10));
        let delta = encode(&old, &mut &new[..], new.len() as u64, new.len()).unwrap().expect("a delta");
        assert!(delta.len() < 5_000, "the delta is {} bytes for a 2 MB file", delta.len());
        assert_eq!(apply(&old, new.len() as u64, &delta), new);
    }

    #[test]
    fn a_file_that_changed_completely_has_no_delta() {
        let old = program(100_000, 1);
        let new = program(100_000, 2);
        assert!(encode(&old, &mut &new[..], new.len() as u64, new.len() * 8 / 10).unwrap().is_none());
    }

    #[test]
    fn the_wrong_base_gives_wrong_bytes_or_an_error_never_the_new_file() {
        let old = program(200_000, 3);
        let mut new = old.clone();
        new[10] ^= 1;
        let delta = encode(&old, &mut &new[..], new.len() as u64, new.len()).unwrap().unwrap();
        let other = program(200_000, 4);
        let mut out = Vec::new();
        let result = decode(&other, new.len() as u64, io::BufReader::new(&delta[..])).unwrap().read_to_end(&mut out);
        assert!(result.is_err() || out != new, "a delta made from one file must not turn another into the new one");
    }

    #[test]
    fn a_damaged_delta_is_refused() {
        let old = program(100_000, 5);
        let mut new = old.clone();
        new[500] ^= 1;
        let mut delta = encode(&old, &mut &new[..], new.len() as u64, new.len()).unwrap().unwrap();
        let middle = delta.len() / 2;
        delta[middle] ^= 0xFF;
        let mut out = Vec::new();
        assert!(decode(&old, new.len() as u64, io::BufReader::new(&delta[..])).unwrap().read_to_end(&mut out).is_err());
    }

    #[test]
    fn windows_cover_both_files_and_stay_in_range() {
        assert_eq!(window_log(0, 0), 10);
        assert_eq!(window_log(1 << 20, 1 << 20), 21);
        assert_eq!(window_log(MAX_BASE, MAX_NEW), 31);
        assert!(window_log(u64::MAX, u64::MAX) <= 31);
    }

    #[test]
    fn empty_and_oversized_inputs_are_left_to_the_full_file() {
        assert!(encode(&[], &mut &b"x"[..], 1, 10).unwrap().is_none());
        assert!(encode(b"x", &mut &b"x"[..], MAX_NEW + 1, 10).unwrap().is_none());
    }
}
