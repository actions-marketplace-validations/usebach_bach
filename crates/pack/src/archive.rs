//! The archive: every packaged file, one after another, in a single zstd
//! stream. Inside the stream each file is
//!
//! ```text
//! b'F' | path length (u16 LE) | path (UTF-8, '/'-separated) | size (u64 LE) | bytes
//! ```
//!
//! An update patch may store a changed file as what differs from the file it
//! replaces, a binary delta (see [`crate::delta`]):
//!
//! ```text
//! b'D' | path length | path | size (u64 LE) | base size (u64 LE) | base SHA-256 (32 bytes) | delta length (u64 LE) | delta
//! ```
//!
//! `size` is the size of the *new* file in both, and a reader hands the new file's
//! bytes to its visitor either way: the delta is expanded against the earlier
//! file, which has to be exactly the one the entry names. A single `b'E'` ends the
//! archive. One stream compresses better than separate
//! files and can be read front to back without seeking, so extraction
//! needs no temp copy of the archive.

use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::{delta, paths};

const FILE: u8 = b'F';
const DELTA: u8 = b'D';
const END: u8 = b'E';

/// The memory budget for compressing when `installer.compression_memory`
/// isn't set, in MiB.
pub const DEFAULT_MEMORY_MIB: u32 = 1024;

// What compressing costs, from measurements of a Release build on 300 MB of
// mixed files (peak working set, in MiB). A level's memory grows with each
// thread by about the per-thread figure and starts at about the base; the
// estimate is on the high side on purpose, because the budget is a ceiling.
//   best:   1 thread 291, 4 threads 714, 12 threads 961
//   normal: 1 thread 156, 4 threads 299, 12 threads 443
//   fast:   1 thread  83, 4 threads 140, 12 threads 268
const MIB_PER_THREAD_BEST: u32 = 140;
const MIB_BASE_BEST: u32 = 150;
const MIB_PER_THREAD_NORMAL: u32 = 45;
const MIB_BASE_NORMAL: u32 = 110;
const MIB_PER_THREAD_FAST: u32 = 20;
const MIB_BASE_FAST: u32 = 60;

/// How many threads a build compresses with, and about how much memory that
/// takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub threads: u32,
    /// An estimate, in MiB, of what compressing takes besides reading files.
    pub estimated_mib: u32,
    /// Fewer threads than processor cores because of the memory budget.
    pub limited_by_memory: bool,
}

/// How hard the builder compresses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Compression {
    /// Smallest setup file; slowest to build. Unpacking is fast either way.
    #[default]
    Best,
    Normal,
    /// Quickest to build, for trying things out.
    Fast,
}

impl Compression {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "best" => Some(Self::Best),
            "normal" => Some(Self::Normal),
            "fast" => Some(Self::Fast),
            _ => None,
        }
    }

    /// What compressing with this level costs, and so how many threads fit in
    /// `memory_mib` of memory: `requested` threads when asked for (never more
    /// than there are processor cores), otherwise as many as the memory budget
    /// allows, at least one. One thread is always allowed, whatever the budget.
    pub fn plan(self, requested: Option<u32>, memory_mib: u32) -> Plan {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as u32;
        self.plan_for(requested, memory_mib, cores)
    }

    fn plan_for(self, requested: Option<u32>, memory_mib: u32, cores: u32) -> Plan {
        let per_thread = self.mib_per_thread();
        let fits = (memory_mib.saturating_sub(self.mib_base()) / per_thread).max(1);
        let threads = match requested {
            Some(requested) => requested.clamp(1, cores.max(1)),
            None => fits.min(cores.max(1)),
        };
        Plan { threads, estimated_mib: self.mib_base() + per_thread * threads, limited_by_memory: requested.is_none() && fits < cores }
    }

    /// Memory one compression thread needs besides the shared part, in MiB.
    fn mib_per_thread(self) -> u32 {
        match self {
            Self::Best => MIB_PER_THREAD_BEST,
            Self::Normal => MIB_PER_THREAD_NORMAL,
            Self::Fast => MIB_PER_THREAD_FAST,
        }
    }

    fn mib_base(self) -> u32 {
        match self {
            Self::Best => MIB_BASE_BEST,
            Self::Normal => MIB_BASE_NORMAL,
            Self::Fast => MIB_BASE_FAST,
        }
    }

    fn level(self) -> i32 {
        match self {
            Self::Best => 19,
            Self::Normal => 9,
            Self::Fast => 3,
        }
    }
}

/// A file on disk and the path it's packaged under.
#[derive(Clone, Debug)]
pub struct Source {
    pub packaged: String,
    pub path: PathBuf,
    pub size: u64,
}

/// The earlier version of a file, to store the new one as a delta from.
#[derive(Clone, Debug)]
pub struct DeltaBase {
    pub bytes: Vec<u8>,
    /// `bytes`' SHA-256, lowercase hex.
    pub sha256: String,
}

/// A delta in the archive was made from another file than the one on this
/// computer. Not a damaged archive: an update patch is only good over the version
/// it was made for.
#[derive(Debug)]
pub struct WrongBase {
    pub path: String,
}

impl std::fmt::Display for WrongBase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} isn't the file this update patch was made from", self.path)
    }
}

impl std::error::Error for WrongBase {}

/// What a reader does with a binary delta entry.
pub enum Bases<'a> {
    /// There are none: an entry like that is an error (a full setup file has none).
    Refuse,
    /// Step over them without expanding (to check the rest of an update patch alone).
    Skip,
    /// Ask for the earlier file of this path, which has this many bytes.
    From(&'a mut dyn FnMut(&str, u64) -> Result<Vec<u8>>),
}

/// Compresses `sources` into `out`, calling `progress` with the bytes read
/// so far. Returns `out` once the stream is finished.
pub fn write<W: Write>(out: W, sources: &[Source], compression: Compression, progress: &mut dyn FnMut(u64)) -> Result<W> {
    let threads = compression.plan(None, DEFAULT_MEMORY_MIB).threads;
    write_hashed(out, sources, compression, threads, progress).map(|(out, _)| out)
}

/// Like [`write`], and also gives the SHA-256 (hex) of every source file's
/// bytes, in the order of `sources`. The hash is taken from the very bytes
/// that go into the archive, so the build reads each file once, not twice.
///
/// `threads` is how many compression threads to use (see [`Compression::plan`]).
/// It only changes how fast and how big in memory the build is: the bytes are
/// the same for every number, so a build gives the same setup file on any
/// computer.
pub fn write_hashed<W: Write>(
    out: W,
    sources: &[Source],
    compression: Compression,
    threads: u32,
    progress: &mut dyn FnMut(u64),
) -> Result<(W, Vec<String>)> {
    write_hashed_with(out, sources, &std::collections::HashMap::new(), compression, threads, progress)
}

/// Like [`write_hashed`], and stores a file as a delta from its earlier version when
/// `bases` has one for its path and the delta is clearly smaller (under four fifths of the file).
pub fn write_hashed_with<W: Write>(
    out: W,
    sources: &[Source],
    bases: &std::collections::HashMap<String, DeltaBase>,
    compression: Compression,
    threads: u32,
    progress: &mut dyn FnMut(u64),
) -> Result<(W, Vec<String>)> {
    let mut encoder = zstd::stream::Encoder::new(out, compression.level())?;
    // Multithreading only changes speed, never the format; ignore a build
    // of zstd without it.
    let _ = encoder.multithread(threads);
    encoder.include_checksum(true)?;

    let mut done = 0u64;
    let mut hashes = Vec::with_capacity(sources.len());
    for source in sources {
        let packaged = paths::normalize(&source.packaged)?;
        let name = packaged.as_bytes();
        let Ok(name_len) = u16::try_from(name.len()) else {
            bail!("\"{packaged}\" is too long a path to package");
        };
        let mut file = File::open(&source.path).with_context(|| format!("couldn't open {}", source.path.display()))?;
        let size = file.metadata()?.len();
        if size != source.size {
            bail!("{} changed size while it was being packaged", source.path.display());
        }
        if let Some(base) = bases.get(&packaged).filter(|_| size > 0) {
            let limit = usize::try_from(size / 5 * 4).unwrap_or(usize::MAX);
            let mut reading = Verifying::new(&mut file);
            let made = delta::encode(&base.bytes, &mut reading, size, limit)
                .with_context(|| format!("couldn't read {}", source.path.display()))?;
            let (read, sha256) = reading.digest();
            if let Some(delta) = made.filter(|_| read == size) {
                encoder.write_all(&[DELTA])?;
                encoder.write_all(&name_len.to_le_bytes())?;
                encoder.write_all(name)?;
                encoder.write_all(&size.to_le_bytes())?;
                encoder.write_all(&(base.bytes.len() as u64).to_le_bytes())?;
                encoder.write_all(&unhex_sha(&base.sha256)?)?;
                encoder.write_all(&(delta.len() as u64).to_le_bytes())?;
                encoder.write_all(&delta)?;
                done = done.saturating_add(size);
                progress(done);
                hashes.push(sha256);
                continue;
            }
            // Not worth a delta (or the encoder stopped early): the whole file, from its start.
            file = File::open(&source.path).with_context(|| format!("couldn't open {}", source.path.display()))?;
        }
        encoder.write_all(&[FILE])?;
        encoder.write_all(&name_len.to_le_bytes())?;
        encoder.write_all(name)?;
        encoder.write_all(&size.to_le_bytes())?;
        let mut hasher = Sha256::new();
        let copied = copy_with(&mut file, &mut encoder, Some(&mut hasher), &mut |n| {
            done = done.saturating_add(n);
            progress(done);
        })
        .with_context(|| format!("couldn't read {}", source.path.display()))?;
        if copied != size {
            bail!("{} changed size while it was being packaged", source.path.display());
        }
        hashes.push(crate::protect::hex(&hasher.finalize()));
    }
    encoder.write_all(&[END])?;
    Ok((encoder.finish()?, hashes))
}

/// A SHA-256 in hex as its 32 bytes.
fn unhex_sha(hex: &str) -> Result<[u8; 32]> {
    let hex = hex.trim();
    if hex.len() != 64 || !hex.is_ascii() {
        bail!("a checksum isn't 64 hex digits");
    }
    let mut out = [0u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).context("a checksum isn't hex")?;
    }
    Ok(out)
}

/// Reads an archive front to back, handing each file to `visit` with its
/// packaged path, its size and a reader over exactly its bytes. Whatever
/// `visit` leaves unread is skipped. An archive with binary deltas needs
/// [`read_with`].
pub fn read<R: Read>(input: R, visit: &mut dyn FnMut(&str, u64, &mut dyn Read) -> Result<()>) -> Result<()> {
    read_with(input, Bases::Refuse, visit)
}

/// [`read`] for an archive that may hold binary deltas: `bases` says where the
/// earlier files are.
pub fn read_with<R: Read>(input: R, mut bases: Bases, visit: &mut dyn FnMut(&str, u64, &mut dyn Read) -> Result<()>) -> Result<()> {
    let mut decoder = zstd::stream::Decoder::new(BufReader::new(input))?;
    // The builder writes every file once. A path twice (or two paths Windows takes for
    // one file) is a damaged or crafted archive, and two writes to one file would
    // undo each other halfway through an update.
    let mut seen = std::collections::HashSet::new();
    loop {
        let mut kind = [0u8; 1];
        decoder.read_exact(&mut kind).context("the archive ends early")?;
        match kind[0] {
            END => return Ok(()),
            FILE | DELTA => {}
            other => bail!("the archive is damaged (unexpected byte {other:#04x})"),
        }
        let mut len = [0u8; 2];
        decoder.read_exact(&mut len)?;
        let mut name = vec![0u8; usize::from(u16::from_le_bytes(len))];
        decoder.read_exact(&mut name)?;
        let name = String::from_utf8(name).context("the archive holds a path that isn't UTF-8")?;
        let packaged = paths::normalize(&name)?;
        if packaged != name {
            bail!("the archive holds \"{name}\", which isn't a canonical path");
        }
        if !seen.insert(paths::fold_case(&packaged)) {
            bail!("the archive holds \"{packaged}\" more than once");
        }
        let mut size = [0u8; 8];
        decoder.read_exact(&mut size)?;
        let size = u64::from_le_bytes(size);

        if kind[0] == DELTA {
            let mut head = [0u8; 8 + 32 + 8];
            decoder.read_exact(&mut head)?;
            let base_size = u64::from_le_bytes(head[..8].try_into().unwrap_or_default());
            let mut base_sha = [0u8; 32];
            base_sha.copy_from_slice(&head[8..40]);
            let delta_len = u64::from_le_bytes(head[40..].try_into().unwrap_or_default());
            // A delta is held in no more than the file it makes; a longer one is damaged.
            if delta_len > size.saturating_add(1 << 20) || base_size > delta::MAX_BASE || size > delta::MAX_NEW {
                bail!("the archive is damaged (\"{packaged}\" has an impossible delta)");
            }
            let mut stored = io::BufReader::new((&mut decoder).take(delta_len));
            match &mut bases {
                Bases::Refuse => bail!("\"{packaged}\" is stored as a binary delta, which only an update patch has"),
                Bases::Skip => {}
                Bases::From(earlier) => {
                    let base = earlier(&packaged, base_size)?;
                    let actual = crate::protect::sha256_hex(&base[..])?;
                    if base.len() as u64 != base_size || !actual.eq_ignore_ascii_case(&crate::protect::hex(&base_sha)) {
                        return Err(WrongBase { path: packaged }.into());
                    }
                    let mut new = delta::decode(&base, size, &mut stored)?.take(size);
                    visit(&packaged, size, &mut new)?;
                    io::copy(&mut new, &mut io::sink())?;
                    if new.limit() != 0 {
                        bail!("the delta for \"{packaged}\" ends early");
                    }
                }
            }
            io::copy(&mut stored, &mut io::sink())?;
            continue;
        }

        let mut body = (&mut decoder).take(size);
        visit(&packaged, size, &mut body)?;
        io::copy(&mut body, &mut io::sink())?;
        if body.limit() != 0 {
            bail!("the archive ends in the middle of \"{packaged}\"");
        }
    }
}

/// A reader that counts and hashes what passes through it, so a file can be
/// checked against the size and SHA-256 the manifest records while it is written,
/// not in a second pass over the disk.
pub struct Verifying<R: Read> {
    inner: R,
    hasher: Sha256,
    count: u64,
}

impl<R: Read> Verifying<R> {
    pub fn new(inner: R) -> Self {
        Self { inner, hasher: Sha256::new(), count: 0 }
    }

    /// How many bytes were read and their SHA-256 (lowercase hex), so far.
    pub fn digest(&self) -> (u64, String) {
        (self.count, crate::protect::hex(&self.hasher.clone().finalize()))
    }
}

impl<R: Read> Read for Verifying<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.hasher.update(&buf[..read]);
        self.count = self.count.saturating_add(read as u64);
        Ok(read)
    }
}

/// `io::copy` that reports each chunk as it goes.
pub fn copy_counting(from: &mut dyn Read, to: &mut dyn Write, progress: &mut dyn FnMut(u64)) -> io::Result<u64> {
    copy_with(from, to, None, progress)
}

/// [`copy_counting`] that can also feed a hasher with the very bytes it copies.
fn copy_with(from: &mut dyn Read, to: &mut dyn Write, mut hasher: Option<&mut Sha256>, progress: &mut dyn FnMut(u64)) -> io::Result<u64> {
    let mut buffer = vec![0u8; 256 * 1024];
    let mut total = 0u64;
    loop {
        let read = match from.read(&mut buffer) {
            Ok(0) => return Ok(total),
            Ok(read) => read,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        };
        to.write_all(&buffer[..read])?;
        if let Some(hasher) = hasher.as_mut() {
            hasher.update(&buffer[..read]);
        }
        total = total.saturating_add(read as u64);
        progress(read as u64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bach-pack-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn files_round_trip() {
        let dir = temp_dir("archive");
        let big: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(dir.join("a.bin"), &big).unwrap();
        std::fs::write(dir.join("b.txt"), b"hello").unwrap();
        std::fs::write(dir.join("empty"), b"").unwrap();
        let sources: Vec<Source> = [("bin/a.bin", "a.bin"), ("b.txt", "b.txt"), ("empty", "empty")]
            .iter()
            .map(|(packaged, file)| {
                let path = dir.join(file);
                let size = std::fs::metadata(&path).unwrap().len();
                Source { packaged: packaged.to_string(), path, size }
            })
            .collect();

        let mut last = 0;
        let (bytes, hashes) = write_hashed(Vec::new(), &sources, Compression::Fast, 2, &mut |done| last = done).unwrap();
        assert_eq!(last, big.len() as u64 + 5);
        // One checksum per file, of exactly the file's bytes (the SHA-256 of "hello" and of nothing).
        assert_eq!(hashes.len(), 3);
        assert_eq!(hashes[0], crate::protect::sha256_hex(big.as_slice()).unwrap());
        assert_eq!(hashes[1], "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
        assert_eq!(hashes[2], "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert!(bytes.len() < big.len() / 10, "the pattern should compress");

        let mut seen = Vec::new();
        read(bytes.as_slice(), &mut |path, size, body| {
            let mut data = Vec::new();
            body.read_to_end(&mut data)?;
            assert_eq!(data.len() as u64, size);
            seen.push((path.to_string(), data));
            Ok(())
        })
        .unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0].0, "bin/a.bin");
        assert_eq!(seen[0].1, big);
        assert_eq!(seen[1].1, b"hello");
        assert!(seen[2].1.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_memory_budget_decides_how_many_threads_compress() {
        // A 12-core computer with the default 1 GiB: "best" is held back, the others aren't.
        let best = Compression::Best.plan_for(None, DEFAULT_MEMORY_MIB, 12);
        assert_eq!(best.threads, 6);
        assert!(best.limited_by_memory && best.estimated_mib <= DEFAULT_MEMORY_MIB, "{best:?}");
        assert_eq!(Compression::Normal.plan_for(None, DEFAULT_MEMORY_MIB, 12).threads, 12);
        assert_eq!(Compression::Fast.plan_for(None, DEFAULT_MEMORY_MIB, 12).threads, 12);
        // A small container: one thread, never none, whatever the budget.
        assert_eq!(Compression::Best.plan_for(None, 64, 12).threads, 1);
        assert_eq!(Compression::Best.plan_for(None, 2048, 2).threads, 2);
        assert!(!Compression::Best.plan_for(None, 2048, 2).limited_by_memory);
        // A bigger budget lets all cores work.
        assert_eq!(Compression::Best.plan_for(None, 4096, 12).threads, 12);
        // Asking for a number wins over the budget, but not over the cores.
        assert_eq!(Compression::Best.plan_for(Some(3), 64, 12).threads, 3);
        assert_eq!(Compression::Best.plan_for(Some(99), 4096, 8).threads, 8);
        assert_eq!(Compression::Best.plan_for(Some(0), 4096, 8).threads, 1);
    }

    #[test]
    fn the_archive_is_the_same_whatever_the_number_of_threads() {
        // Larger than one compression job, so the threads really split the work.
        let dir = temp_dir("threads");
        let mut state = 12345u32;
        let data: Vec<u8> = (0..12_000_000u32)
            .map(|i| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                // Half noise, half pattern: something for the compressor to do.
                if i % 2 == 0 {
                    (state >> 24) as u8
                } else {
                    (i % 97) as u8
                }
            })
            .collect();
        std::fs::write(dir.join("big.bin"), &data).unwrap();
        std::fs::write(dir.join("small.txt"), b"small").unwrap();
        let sources = vec![
            Source { packaged: "big.bin".into(), path: dir.join("big.bin"), size: data.len() as u64 },
            Source { packaged: "small.txt".into(), path: dir.join("small.txt"), size: 5 },
        ];
        for level in [Compression::Fast, Compression::Normal] {
            let make = |threads| write_hashed(Vec::new(), &sources, level, threads, &mut |_| {}).unwrap().0;
            let one = make(1);
            assert_eq!(one, make(2), "{level:?}: 1 and 2 threads");
            assert_eq!(one, make(7), "{level:?}: 1 and 7 threads");
            assert_eq!(one, make(1), "{level:?}: and again");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unread_bodies_are_skipped() {
        let dir = temp_dir("skip");
        std::fs::write(dir.join("one"), b"first file").unwrap();
        std::fs::write(dir.join("two"), b"second").unwrap();
        let sources: Vec<Source> = ["one", "two"]
            .iter()
            .map(|name| Source { packaged: name.to_string(), path: dir.join(name), size: std::fs::metadata(dir.join(name)).unwrap().len() })
            .collect();
        let bytes = write(Vec::new(), &sources, Compression::Fast, &mut |_| {}).unwrap();
        let mut names = Vec::new();
        read(bytes.as_slice(), &mut |path, _, _| {
            names.push(path.to_string());
            Ok(())
        })
        .unwrap();
        assert_eq!(names, ["one", "two"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn encrypted_archives_round_trip_and_stay_closed_to_other_keys() {
        use crate::manifest::Encryption;
        let dir = temp_dir("encrypted");
        let big: Vec<u8> = (0..900_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        std::fs::write(dir.join("a.bin"), &big).unwrap();
        std::fs::write(dir.join("b.txt"), b"private words").unwrap();
        let sources: Vec<Source> = ["a.bin", "b.txt"]
            .iter()
            .map(|name| Source { packaged: name.to_string(), path: dir.join(name), size: std::fs::metadata(dir.join(name)).unwrap().len() })
            .collect();
        let (encryption, unlocked) = Encryption::create("K7QF-3MWD-9XPA-H2TY-6NBC").unwrap();

        let writer = encryption.writer(Vec::new(), &unlocked).unwrap();
        let sealed = write(writer, &sources, Compression::Fast, &mut |_| {}).unwrap().finish().unwrap();
        assert!(!sealed.windows(13).any(|window| window == b"private words"));

        // The right key gets everything back.
        let again = encryption.unlock("K7QF-3MWD-9XPA-H2TY-6NBC").unwrap();
        let mut seen = Vec::new();
        read(encryption.reader(sealed.as_slice(), &again).unwrap(), &mut |path, _, body| {
            let mut data = Vec::new();
            body.read_to_end(&mut data)?;
            seen.push((path.to_string(), data));
            Ok(())
        })
        .unwrap();
        assert_eq!(seen[0], ("a.bin".to_string(), big));
        assert_eq!(seen[1], ("b.txt".to_string(), b"private words".to_vec()));

        // Without decrypting, the bytes are not an archive at all.
        assert!(read(sealed.as_slice(), &mut |_, _, _| Ok(())).is_err());
        // Cut short anywhere, it fails instead of unpacking a part.
        for cut in [sealed.len() / 3, sealed.len() - 1] {
            let result = read(encryption.reader(&sealed[..cut], &again).unwrap(), &mut |_, _, body| {
                io::copy(body, &mut io::sink())?;
                Ok(())
            });
            assert!(result.is_err(), "cut at {cut}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn truncated_archives_fail() {
        let dir = temp_dir("truncated");
        std::fs::write(dir.join("one"), vec![1u8; 4096]).unwrap();
        let sources = vec![Source { packaged: "one".into(), path: dir.join("one"), size: 4096 }];
        let bytes = write(Vec::new(), &sources, Compression::Fast, &mut |_| {}).unwrap();
        let cut = &bytes[..bytes.len() / 2];
        assert!(read(cut, &mut |_, _, _| Ok(())).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_path_twice_in_an_archive_is_refused() {
        let dir = temp_dir("twice");
        std::fs::write(dir.join("a.bin"), b"one").unwrap();
        let source = |packaged: &str| Source { packaged: packaged.into(), path: dir.join("a.bin"), size: 3 };
        for pair in [["a.bin", "a.bin"], ["a.bin", "A.BIN"], ["σ.bin", "ς.bin"]] {
            let sources = [source(pair[0]), source(pair[1])];
            let (bytes, _) = write_hashed(Vec::new(), &sources, Compression::Fast, 1, &mut |_| {}).unwrap();
            let mut seen = 0;
            let err = read(bytes.as_slice(), &mut |_, _, _| {
                seen += 1;
                Ok(())
            })
            .unwrap_err();
            assert!(format!("{err:#}").contains("more than once"), "{pair:?}: {err:#}");
            assert_eq!(seen, 1, "the second one is never handed out");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_verifying_reader_counts_and_hashes() {
        let mut reader = Verifying::new(&b"hello"[..]);
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        assert_eq!(reader.digest(), (5, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".to_string()));
    }
}
