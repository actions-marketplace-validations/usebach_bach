//! Keeping a setup file's contents from anyone without the access key.
//!
//! The key is turned into an encryption key with Argon2id (slow on purpose,
//! so guessing keys is expensive), and the archive is encrypted in 64 KiB
//! chunks with ChaCha20-Poly1305 in the STREAM construction: every chunk is
//! authenticated, and the last one is marked, so a flipped bit, a chunk cut
//! out of the middle and a file cut short all fail loudly instead of
//! unpacking something wrong.
//!
//! What this protects: the packaged files, at rest in the setup file. What
//! it does not: the file names and sizes, which the manifest lists in the
//! clear, and anything after install, because an installed app's files are
//! ordinary files on the user's disk.
//!
//! The scheme is fixed ([`SCHEME`]); a manifest that names another one is
//! refused rather than guessed at.

use std::io::{self, Read, Write};

use anyhow::{bail, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::generic_array::GenericArray;
use chacha20poly1305::aead::stream::{DecryptorBE32, EncryptorBE32};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
use sha2::{Digest, Sha256};

use crate::manifest::{Encryption, Slot};

/// The scheme with one key for the whole setup file (older setup files).
pub const SCHEME: &str = "argon2id-chacha20poly1305-v1";
/// The scheme with a slot per access key: each key opens the files on its
/// own, and a key is taken away by building without it. What new setup files use.
pub const SCHEME_SLOTS: &str = "argon2id-slots-chacha20poly1305-v2";
/// The most access keys one setup file can hold.
pub const MAX_SLOTS: usize = 1000;
const SLOT_NONCE_LEN: usize = 12;
const FILE_KEY_LEN: usize = 32;
/// Plain bytes per encrypted chunk.
pub const CHUNK: usize = 64 * 1024;
const TAG: usize = 16;
/// Bytes of the STREAM nonce prefix (a 12 byte nonce minus a counter and a flag).
pub const NONCE_LEN: usize = 7;
const SALT_LEN: usize = 16;
/// What new setup files use: 64 MiB and 3 passes, about a third of a second.
pub const MEMORY_KIB: u32 = 64 * 1024;
pub const PASSES: u32 = 3;
/// A setup file asking for more than this is not one Bach wrote: refusing
/// it keeps a hostile file from exhausting the computer.
pub const MAX_MEMORY_KIB: u32 = 256 * 1024;
pub const MAX_PASSES: u32 = 10;
/// The shortest and longest key a build accepts.
pub const MIN_KEY: usize = 8;
pub const MAX_KEY: usize = 128;

/// What a key unlocks. Never printed; wiped when dropped.
pub struct Unlocked {
    key: [u8; 32],
}

impl std::fmt::Debug for Unlocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Unlocked(..)")
    }
}

impl Drop for Unlocked {
    fn drop(&mut self) {
        // Volatile, so the compiler can't drop the write as dead.
        for byte in &mut self.key {
            // Safety: `byte` is a valid, aligned reference into `self.key`.
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
    }
}

/// Why a key didn't open a setup file.
#[derive(Debug, PartialEq, Eq)]
pub enum KeyError {
    /// The key is not the one the files were locked with.
    Wrong,
    /// The setup file's own protection data is unusable.
    Damaged(String),
}

impl std::fmt::Display for KeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Wrong => f.write_str("that key doesn't open this setup file"),
            Self::Damaged(why) => write!(f, "the setup file's protection is damaged ({why})"),
        }
    }
}

impl std::error::Error for KeyError {}

/// A key a build can use: `Err` says why not, in a sentence that fits after
/// "the access key".
pub fn check_key(key: &str) -> Result<(), String> {
    if key.len() < MIN_KEY {
        return Err(format!("is too short; use at least {MIN_KEY} characters"));
    }
    if key.len() > MAX_KEY {
        return Err(format!("is too long; use at most {MAX_KEY} characters"));
    }
    if let Some(bad) = key.chars().find(|c| !c.is_ascii_alphanumeric() && !"-_.~+/=:".contains(*c)) {
        return Err(format!("has {bad:?} in it; use letters, digits and - _ . ~ + / = : (it travels in an HTTP header too)"));
    }
    Ok(())
}

/// A new random key, like `K7QF-3MWD-9XPA-H2TY-6NBC`: 100 bits, no letters
/// that look alike (no 0 O 1 I).
pub fn generate_key() -> Result<String> {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut random = [0u8; 20];
    fill_random(&mut random)?;
    let mut key = String::with_capacity(24);
    for (index, byte) in random.iter().enumerate() {
        if index > 0 && index % 4 == 0 {
            key.push('-');
        }
        // 32 symbols and 256 byte values: every symbol is equally likely.
        key.push(ALPHABET[usize::from(byte & 31)] as char);
    }
    Ok(key)
}

pub(crate) fn fill_random(bytes: &mut [u8]) -> Result<()> {
    getrandom::fill(bytes).map_err(|err| anyhow::anyhow!("couldn't get random bytes from the system: {err}"))
}

impl Encryption {
    /// Protection for a new setup file, with a fresh salt and nonce, and
    /// what `key` unlocks.
    pub fn create(key: &str) -> Result<(Self, Unlocked)> {
        if let Err(why) = check_key(key) {
            bail!("the access key {why}");
        }
        let mut salt = [0u8; SALT_LEN];
        let mut nonce = [0u8; NONCE_LEN];
        fill_random(&mut salt)?;
        fill_random(&mut nonce)?;
        let unlocked = derive(key, &salt, MEMORY_KIB, PASSES).map_err(|why| anyhow::anyhow!("{why}"))?;
        let encryption = Self {
            scheme: SCHEME.into(),
            salt: hex(&salt),
            nonce: hex(&nonce),
            memory_kib: MEMORY_KIB,
            passes: PASSES,
            check: check_value(&unlocked),
            slots: Vec::new(),
        };
        Ok((encryption, unlocked))
    }

    /// Protection for a new setup file that any one of `keys` opens: a random
    /// key for the files, locked once per access key. Taking a key away is a
    /// new build without it.
    pub fn create_slots(keys: &[String]) -> Result<(Self, Unlocked)> {
        if keys.is_empty() {
            bail!("there is no access key to lock the files with");
        }
        if keys.len() > MAX_SLOTS {
            bail!("{} access keys are too many; a setup file holds at most {MAX_SLOTS}", keys.len());
        }
        for (index, key) in keys.iter().enumerate() {
            if let Err(why) = check_key(key) {
                bail!("access key {} {why}", index + 1);
            }
            if keys[..index].contains(key) {
                bail!("access key {} is the same as an earlier one", index + 1);
            }
        }
        let mut salt = [0u8; SALT_LEN];
        let mut nonce = [0u8; NONCE_LEN];
        let mut file_key = [0u8; FILE_KEY_LEN];
        fill_random(&mut salt)?;
        fill_random(&mut nonce)?;
        fill_random(&mut file_key)?;
        let unlocked = Unlocked { key: file_key };
        let mut slots = Vec::with_capacity(keys.len());
        for key in keys {
            let mut slot_salt = [0u8; SALT_LEN];
            let mut slot_nonce = [0u8; SLOT_NONCE_LEN];
            fill_random(&mut slot_salt)?;
            fill_random(&mut slot_nonce)?;
            let lock = derive(key, &slot_salt, MEMORY_KIB, PASSES).map_err(|why| anyhow::anyhow!("{why}"))?;
            let sealed = cipher(&lock)
                .encrypt(GenericArray::from_slice(&slot_nonce), Payload { msg: &file_key, aad: &slot_aad(&salt, &nonce) })
                .map_err(|_| anyhow::anyhow!("couldn't lock the files with an access key"))?;
            let mut wrapped = slot_nonce.to_vec();
            wrapped.extend_from_slice(&sealed);
            slots.push(Slot { id: hex(&[slot_id(key, &salt)]), salt: hex(&slot_salt), wrapped: hex(&wrapped) });
        }
        let encryption = Self {
            scheme: SCHEME_SLOTS.into(),
            salt: hex(&salt),
            nonce: hex(&nonce),
            memory_kib: MEMORY_KIB,
            passes: PASSES,
            check: check_value(&unlocked),
            slots,
        };
        Ok((encryption, unlocked))
    }

    /// Whether these settings are ones this version can use, without
    /// deriving anything. A setup file is untrusted input.
    pub fn validate(&self) -> Result<()> {
        if self.scheme == SCHEME_SLOTS {
            if self.slots.is_empty() || self.slots.len() > MAX_SLOTS {
                bail!("the setup file's protection data is damaged");
            }
            let damaged = self.slots.iter().any(|slot| {
                unhex::<1>(&slot.id).is_none()
                    || unhex::<SALT_LEN>(&slot.salt).is_none()
                    || unhex_vec(&slot.wrapped).is_none_or(|bytes| bytes.len() != SLOT_NONCE_LEN + FILE_KEY_LEN + TAG)
            });
            if damaged {
                bail!("the setup file's protection data is damaged");
            }
        } else if self.scheme != SCHEME {
            bail!("this setup file is protected in a way (\"{}\") this version of setup doesn't know", self.scheme);
        }
        if unhex::<SALT_LEN>(&self.salt).is_none() || unhex::<NONCE_LEN>(&self.nonce).is_none() {
            bail!("the setup file's protection data is damaged");
        }
        if self.check.len() != 64 || !self.check.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("the setup file's protection data is damaged");
        }
        if !(8..=MAX_MEMORY_KIB).contains(&self.memory_kib) || !(1..=MAX_PASSES).contains(&self.passes) {
            bail!("the setup file asks for more memory or time than setup allows to check a key");
        }
        Ok(())
    }

    /// Opens with `key`: slow (that is the point), so call it off the UI
    /// thread.
    pub fn unlock(&self, key: &str) -> Result<Unlocked, KeyError> {
        self.validate().map_err(|err| KeyError::Damaged(err.to_string()))?;
        let salt = unhex::<SALT_LEN>(&self.salt).ok_or_else(|| KeyError::Damaged("salt".into()))?;
        // A key the build would never have accepted can't be the right one.
        if check_key(key).is_err() {
            return Err(KeyError::Wrong);
        }
        if self.scheme == SCHEME_SLOTS {
            return self.unlock_slots(key, &salt);
        }
        let unlocked = derive(key, &salt, self.memory_kib, self.passes).map_err(KeyError::Damaged)?;
        if same(check_value(&unlocked).as_bytes(), self.check.to_ascii_lowercase().as_bytes()) {
            Ok(unlocked)
        } else {
            Err(KeyError::Wrong)
        }
    }

    /// Tries the slots that could belong to `key` (the one-byte id rules most
    /// out without the slow part), until one opens.
    fn unlock_slots(&self, key: &str, file_salt: &[u8; SALT_LEN]) -> Result<Unlocked, KeyError> {
        let id = hex(&[slot_id(key, file_salt)]);
        let nonce = self.nonce().map_err(|err| KeyError::Damaged(err.to_string()))?;
        for slot in self.slots.iter().filter(|slot| slot.id.eq_ignore_ascii_case(&id)) {
            let slot_salt = unhex::<SALT_LEN>(&slot.salt).ok_or_else(|| KeyError::Damaged("slot".into()))?;
            let wrapped = unhex_vec(&slot.wrapped).ok_or_else(|| KeyError::Damaged("slot".into()))?;
            let lock = derive(key, &slot_salt, self.memory_kib, self.passes).map_err(KeyError::Damaged)?;
            let (slot_nonce, sealed) = wrapped.split_at(SLOT_NONCE_LEN);
            let opened =
                cipher(&lock).decrypt(GenericArray::from_slice(slot_nonce), Payload { msg: sealed, aad: &slot_aad(file_salt, &nonce) });
            let Ok(opened) = opened else { continue };
            let Ok(file_key) = <[u8; FILE_KEY_LEN]>::try_from(opened.as_slice()) else { continue };
            let unlocked = Unlocked { key: file_key };
            // The slot opened, so it is genuine; the check ties it to these files.
            if same(check_value(&unlocked).as_bytes(), self.check.to_ascii_lowercase().as_bytes()) {
                return Ok(unlocked);
            }
        }
        Err(KeyError::Wrong)
    }

    /// The STREAM nonce prefix the chunks were sealed with.
    pub fn nonce(&self) -> Result<[u8; NONCE_LEN]> {
        unhex::<NONCE_LEN>(&self.nonce).ok_or_else(|| anyhow::anyhow!("the setup file's protection data is damaged"))
    }

    /// Encrypts everything written to `out`; call [`EncryptWriter::finish`].
    pub fn writer<W: Write>(&self, out: W, unlocked: &Unlocked) -> Result<EncryptWriter<W>> {
        Ok(EncryptWriter::new(out, unlocked, &self.nonce()?))
    }

    /// Decrypts what is read from `input`.
    pub fn reader<R: Read>(&self, input: R, unlocked: &Unlocked) -> Result<DecryptReader<R>> {
        Ok(DecryptReader::new(input, unlocked, &self.nonce()?))
    }
}

/// What a slot is bound to, so it can't be moved to another setup file.
fn slot_aad(file_salt: &[u8; SALT_LEN], nonce: &[u8; NONCE_LEN]) -> Vec<u8> {
    let mut aad = b"bach access slot v2".to_vec();
    aad.extend_from_slice(file_salt);
    aad.extend_from_slice(nonce);
    aad
}

/// One byte of a hash of the key, to pick the slots worth trying. It is a
/// fast hash: the byte tells somebody who guesses keys offline 1 in 256, which
/// only matters for keys that are easy to guess (`bach key` makes 100-bit ones).
fn slot_id(key: &str, file_salt: &[u8; SALT_LEN]) -> u8 {
    let mut hasher = Sha256::new();
    hasher.update(b"bach access slot id v2");
    hasher.update(file_salt);
    hasher.update(key.as_bytes());
    hasher.finalize()[0]
}

fn unhex_vec(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() || text.len() > 4096 {
        return None;
    }
    (0..text.len() / 2).map(|index| u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()).collect()
}

fn derive(key: &str, salt: &[u8], memory_kib: u32, passes: u32) -> Result<Unlocked, String> {
    let params = Params::new(memory_kib, passes, 1, Some(32)).map_err(|err| err.to_string())?;
    let mut out = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(key.as_bytes(), salt, &mut out)
        .map_err(|err| err.to_string())?;
    Ok(Unlocked { key: out })
}

/// What the manifest keeps to tell a right key from a wrong one: a hash of
/// the derived key under a label, so it reveals nothing the chunks don't.
fn check_value(unlocked: &Unlocked) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"bach access check v1");
    hasher.update(unlocked.key);
    hex(&hasher.finalize())
}

/// Equal in time that doesn't depend on where they differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// SHA-256 of everything `reader` yields, as lowercase hex. sha2 0.11
/// dropped its `io::Write` impl, so `io::copy` into a hasher no longer works.
pub fn sha256_hex(mut reader: impl Read) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buffer[..n]),
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
    Ok(hex(&hasher.finalize()))
}

/// Lowercase hex text for `bytes` (what a SHA-256 digest is shown as; the
/// digest type itself no longer formats as hex since sha2 0.11).
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N * 2 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

type Sealer = EncryptorBE32<ChaCha20Poly1305>;
type Opener = DecryptorBE32<ChaCha20Poly1305>;

fn cipher(unlocked: &Unlocked) -> ChaCha20Poly1305 {
    ChaCha20Poly1305::new(GenericArray::from_slice(&unlocked.key))
}

/// A writer that encrypts what it is given, chunk by chunk, into `inner`.
pub struct EncryptWriter<W: Write> {
    inner: W,
    sealer: Option<Sealer>,
    buffer: Vec<u8>,
}

impl<W: Write> EncryptWriter<W> {
    pub fn new(inner: W, unlocked: &Unlocked, nonce: &[u8; NONCE_LEN]) -> Self {
        let sealer = Sealer::from_aead(cipher(unlocked), GenericArray::from_slice(nonce));
        Self { inner, sealer: Some(sealer), buffer: Vec::with_capacity(CHUNK * 2) }
    }

    /// Seals what is left as the last chunk and gives `inner` back.
    pub fn finish(mut self) -> io::Result<W> {
        let sealer = self.sealer.take().ok_or_else(|| io::Error::other("already finished"))?;
        let last = sealer.encrypt_last(self.buffer.as_slice()).map_err(|_| io::Error::other("couldn't seal the last chunk"))?;
        self.inner.write_all(&last)?;
        self.inner.flush()?;
        Ok(self.inner)
    }
}

impl<W: Write> Write for EncryptWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(bytes);
        // Strictly more than a chunk: the last chunk is never empty.
        while self.buffer.len() > CHUNK {
            let sealer = self.sealer.as_mut().ok_or_else(|| io::Error::other("already finished"))?;
            let sealed = sealer.encrypt_next(&self.buffer[..CHUNK]).map_err(|_| io::Error::other("couldn't seal a chunk"))?;
            self.buffer.drain(..CHUNK);
            self.inner.write_all(&sealed)?;
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        // Buffered bytes stay buffered: a chunk is sealed whole.
        self.inner.flush()
    }
}

/// A reader that decrypts what `inner` holds. It looks one chunk ahead, to
/// know which chunk is the last; a file that stops before its last chunk, or
/// any chunk that doesn't check out, is an `InvalidData` error.
pub struct DecryptReader<R: Read> {
    inner: R,
    opener: Option<Opener>,
    /// The next sealed chunk, already read.
    ahead: Option<Vec<u8>>,
    started: bool,
    plain: Vec<u8>,
    taken: usize,
    finished: bool,
}

impl<R: Read> DecryptReader<R> {
    pub fn new(inner: R, unlocked: &Unlocked, nonce: &[u8; NONCE_LEN]) -> Self {
        let opener = Opener::from_aead(cipher(unlocked), GenericArray::from_slice(nonce));
        Self { inner, opener: Some(opener), ahead: None, started: false, plain: Vec::new(), taken: 0, finished: false }
    }

    /// Up to one sealed chunk; shorter only at the end of the input.
    fn read_chunk(&mut self) -> io::Result<Vec<u8>> {
        let mut chunk = vec![0u8; CHUNK + TAG];
        let mut filled = 0;
        while filled < chunk.len() {
            match self.inner.read(&mut chunk[filled..]) {
                Ok(0) => break,
                Ok(read) => filled += read,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                Err(err) => return Err(err),
            }
        }
        chunk.truncate(filled);
        Ok(chunk)
    }

    fn open_next(&mut self) -> io::Result<()> {
        let damaged = || io::Error::new(io::ErrorKind::InvalidData, "the protected data is damaged or cut short");
        let chunk = match self.ahead.take() {
            Some(chunk) => chunk,
            None if !self.started => self.read_chunk()?,
            None => return Err(damaged()),
        };
        self.started = true;
        // A full chunk may or may not be the last: read on to find out.
        let following = if chunk.len() == CHUNK + TAG { self.read_chunk()? } else { Vec::new() };
        let last = following.is_empty();
        let opener = self.opener.as_mut();
        self.plain = if last {
            self.opener.take().ok_or_else(damaged)?.decrypt_last(chunk.as_slice()).map_err(|_| damaged())?
        } else {
            opener.ok_or_else(damaged)?.decrypt_next(chunk.as_slice()).map_err(|_| damaged())?
        };
        self.taken = 0;
        if last {
            self.finished = true;
        } else {
            self.ahead = Some(following);
        }
        Ok(())
    }
}

impl<R: Read> Read for DecryptReader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.taken == self.plain.len() {
            if self.finished {
                return Ok(0);
            }
            self.open_next()?;
        }
        let count = out.len().min(self.plain.len() - self.taken);
        out[..count].copy_from_slice(&self.plain[self.taken..self.taken + count]);
        self.taken += count;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protection() -> (Encryption, Unlocked) {
        // Cheap settings keep the tests quick; the format is the same.
        let salt = [7u8; SALT_LEN];
        let nonce = [9u8; NONCE_LEN];
        let unlocked = derive("correct-horse-battery", &salt, 8, 1).unwrap();
        let encryption = Encryption {
            scheme: SCHEME.into(),
            salt: hex(&salt),
            nonce: hex(&nonce),
            memory_kib: 8,
            passes: 1,
            check: check_value(&unlocked),
            slots: Vec::new(),
        };
        (encryption, unlocked)
    }

    fn seal(plain: &[u8]) -> Vec<u8> {
        let (encryption, unlocked) = protection();
        let mut writer = encryption.writer(Vec::new(), &unlocked).unwrap();
        // Uneven writes, so chunk edges fall in the middle of calls.
        for piece in plain.chunks(10_007) {
            writer.write_all(piece).unwrap();
        }
        writer.finish().unwrap()
    }

    fn open(sealed: &[u8]) -> io::Result<Vec<u8>> {
        let (encryption, unlocked) = protection();
        let mut out = Vec::new();
        encryption.reader(sealed, &unlocked).unwrap().read_to_end(&mut out).map(|_| out)
    }

    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i.wrapping_mul(31) ^ (i >> 7)) as u8).collect()
    }

    #[test]
    fn data_round_trips_at_every_awkward_length() {
        for len in [0, 1, 15, CHUNK - 1, CHUNK, CHUNK + 1, 2 * CHUNK, 2 * CHUNK + 1, 3 * CHUNK - 1, 300_000] {
            let plain = pattern(len);
            let sealed = seal(&plain);
            assert!(sealed.len() >= len + TAG, "{len}");
            assert_eq!(open(&sealed).unwrap(), plain, "length {len}");
        }
    }

    #[test]
    fn sealed_data_doesnt_contain_the_plain_data() {
        let plain = b"a very recognisable sentence that must not show".repeat(100);
        let sealed = seal(&plain);
        assert!(!sealed.windows(24).any(|window| window == &plain[..24]));
    }

    #[test]
    fn a_flipped_bit_is_caught() {
        let plain = pattern(3 * CHUNK + 5);
        let sealed = seal(&plain);
        for at in [0, 100, CHUNK + TAG + 3, sealed.len() - 1] {
            let mut damaged = sealed.clone();
            damaged[at] ^= 1;
            let err = open(&damaged).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData, "bit at {at}");
        }
    }

    #[test]
    fn a_file_cut_short_is_caught_even_at_a_chunk_edge() {
        let plain = pattern(3 * CHUNK + 5);
        let sealed = seal(&plain);
        let chunk = CHUNK + TAG;
        for cut in [0, 10, chunk, 2 * chunk, 3 * chunk, sealed.len() - 1] {
            let err = open(&sealed[..cut]).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData, "cut at {cut}");
        }
    }

    #[test]
    fn chunks_cant_be_reordered_or_dropped() {
        let plain = pattern(3 * CHUNK);
        let sealed = seal(&plain);
        let chunk = CHUNK + TAG;
        let mut swapped = Vec::new();
        swapped.extend_from_slice(&sealed[chunk..2 * chunk]);
        swapped.extend_from_slice(&sealed[..chunk]);
        swapped.extend_from_slice(&sealed[2 * chunk..]);
        assert!(open(&swapped).is_err());
        let mut dropped = sealed[..chunk].to_vec();
        dropped.extend_from_slice(&sealed[2 * chunk..]);
        assert!(open(&dropped).is_err());
    }

    #[test]
    fn extra_bytes_after_the_end_are_caught() {
        let mut sealed = seal(&pattern(1000));
        sealed.extend_from_slice(&[0u8; 40]);
        assert!(open(&sealed).is_err());
    }

    #[test]
    fn a_wrong_key_is_told_apart_before_anything_is_decrypted() {
        let (encryption, _) = protection();
        assert!(encryption.unlock("correct-horse-battery").is_ok());
        assert_eq!(encryption.unlock("correct-horse-batterz").unwrap_err(), KeyError::Wrong);
        assert_eq!(encryption.unlock("").unwrap_err(), KeyError::Wrong);
        assert_eq!(encryption.unlock("has spaces in it").unwrap_err(), KeyError::Wrong);
    }

    #[test]
    fn the_wrong_key_cant_open_the_data_either() {
        let sealed = seal(&pattern(5000));
        let (encryption, _) = protection();
        let other = derive("another-key-entirely", &unhex::<SALT_LEN>(&encryption.salt).unwrap(), 8, 1).unwrap();
        let mut reader = encryption.reader(sealed.as_slice(), &other).unwrap();
        assert!(reader.read_to_end(&mut Vec::new()).is_err());
    }

    #[test]
    fn created_protection_unlocks_with_its_key_only() {
        let (encryption, unlocked) = Encryption::create("K7QF-3MWD-9XPA-H2TY-6NBC").unwrap();
        encryption.validate().unwrap();
        let again = encryption.unlock("K7QF-3MWD-9XPA-H2TY-6NBC").unwrap();
        assert_eq!(again.key, unlocked.key);
        assert_eq!(encryption.unlock("K7QF-3MWD-9XPA-H2TY-6NBD").unwrap_err(), KeyError::Wrong);
        // Fresh salt and nonce every time: two builds never share a key stream.
        let (other, _) = Encryption::create("K7QF-3MWD-9XPA-H2TY-6NBC").unwrap();
        assert_ne!(encryption.salt, other.salt);
        assert_ne!(encryption.nonce, other.nonce);
        assert_ne!(encryption.check, other.check);
    }

    #[test]
    fn hostile_protection_data_is_refused_not_obeyed() {
        let (good, _) = protection();
        let with = |change: &dyn Fn(&mut Encryption)| {
            let mut bad = good.clone();
            change(&mut bad);
            bad
        };
        for bad in [
            with(&|e| e.scheme = "rot13".into()),
            with(&|e| e.salt = "zz".into()),
            with(&|e| e.nonce = "00".repeat(8)),
            with(&|e| e.check = "abc".into()),
            with(&|e| e.memory_kib = u32::MAX),
            with(&|e| e.memory_kib = 0),
            with(&|e| e.passes = 0),
            with(&|e| e.passes = 1000),
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
            assert!(matches!(bad.unlock("correct-horse-battery"), Err(KeyError::Damaged(_))), "{bad:?}");
        }
    }

    #[test]
    fn keys_are_checked_before_a_build_uses_them() {
        assert!(check_key("K7QF-3MWD-9XPA-H2TY-6NBC").is_ok());
        assert!(check_key("abcdefgh").is_ok());
        assert!(check_key("short").unwrap_err().contains("too short"));
        assert!(check_key(&"a".repeat(129)).unwrap_err().contains("too long"));
        for bad in ["has space", "quote\"d1234", "back\\slash1", "ümlaut-key1", "new\nline123"] {
            assert!(check_key(bad).is_err(), "{bad:?}");
        }
        assert!(Encryption::create("short").is_err());
    }

    #[test]
    fn generated_keys_are_readable_and_valid() {
        let key = generate_key().unwrap();
        assert_eq!(key.len(), 24, "{key}");
        assert_eq!(key.matches('-').count(), 4);
        assert!(check_key(&key).is_ok());
        assert!(key.chars().all(|c| c == '-' || ("ABCDEFGHJKLMNPQRSTUVWXYZ23456789".contains(c))), "{key}");
        assert_ne!(key, generate_key().unwrap());
    }

    #[test]
    fn hex_round_trips_and_rejects_junk() {
        assert_eq!(hex(&[0, 15, 255]), "000fff");
        assert_eq!(unhex::<3>("000fff"), Some([0, 15, 255]));
        assert_eq!(unhex::<3>("000ff"), None);
        assert_eq!(unhex::<3>("00zfff"), None);
        assert_eq!(unhex::<2>("é1é1"), None);
    }

    #[test]
    fn unlocked_keys_are_never_printed() {
        let (_, unlocked) = protection();
        assert_eq!(format!("{unlocked:?}"), "Unlocked(..)");
    }

    /// Setup files already in the wild were locked with an older Argon2
    /// build; this value was computed with argon2 0.5 and must never change.
    #[test]
    fn key_derivation_matches_setups_already_released() {
        let unlocked = derive("correct horse", b"0123456789abcdef", 64, 2).unwrap();
        assert_eq!(check_value(&unlocked), "8cf3737dbfe980985526c929c7c8298f3e4ab2e08b4df5d926923a71e54db2dc");
    }

    const KEY_A: &str = "K7QF-3MWD-9XPA-H2TY-6NBC";
    const KEY_B: &str = "B3RT-9KWM-2XQD-7FHN-4PLC";
    const KEY_C: &str = "C8VH-5NJA-6TRZ-3WQK-2DMB";

    fn keys(list: &[&str]) -> Vec<String> {
        list.iter().map(|key| key.to_string()).collect()
    }

    #[test]
    fn each_access_key_opens_the_files_on_its_own() {
        let (encryption, unlocked) = Encryption::create_slots(&keys(&[KEY_A, KEY_B])).unwrap();
        encryption.validate().unwrap();
        assert_eq!((encryption.scheme.as_str(), encryption.slots.len()), (SCHEME_SLOTS, 2));
        // The same file key from either slot, and the data it seals opens with it.
        let mut writer = encryption.writer(Vec::new(), &unlocked).unwrap();
        writer.write_all(b"the files").unwrap();
        let sealed = writer.finish().unwrap();
        for key in [KEY_A, KEY_B] {
            let opened = encryption.unlock(key).unwrap_or_else(|err| panic!("{key}: {err}"));
            let mut text = String::new();
            encryption.reader(sealed.as_slice(), &opened).unwrap().read_to_string(&mut text).unwrap();
            assert_eq!(text, "the files");
        }
        // A key that isn't one of them, and one that couldn't be.
        assert_eq!(encryption.unlock(KEY_C).unwrap_err(), KeyError::Wrong);
        assert_eq!(encryption.unlock("short").unwrap_err(), KeyError::Wrong);
        // The slots don't reveal which key is which beyond a byte, and the
        // wrapped file keys differ though they hold the same key.
        assert_ne!(encryption.slots[0].wrapped, encryption.slots[1].wrapped);
        assert!(!encryption.salt.is_empty() && encryption.slots.iter().all(|slot| slot.id.len() == 2));
    }

    #[test]
    fn taking_a_key_away_is_a_build_without_it() {
        let (before, _) = Encryption::create_slots(&keys(&[KEY_A, KEY_B])).unwrap();
        let (after, _) = Encryption::create_slots(&keys(&[KEY_A])).unwrap();
        assert!(before.unlock(KEY_B).is_ok());
        assert_eq!(after.unlock(KEY_B).unwrap_err(), KeyError::Wrong, "the new build doesn't know B");
        assert!(after.unlock(KEY_A).is_ok());
    }

    #[test]
    fn a_tampered_or_moved_slot_is_not_trusted() {
        let (encryption, _) = Encryption::create_slots(&keys(&[KEY_A, KEY_B])).unwrap();
        // One flipped bit in A's slot: A no longer opens, B still does.
        let mut broken = encryption.clone();
        let wrapped = &broken.slots[0].wrapped;
        let flipped = format!("{}{}", if wrapped.starts_with('0') { "1" } else { "0" }, &wrapped[1..]);
        broken.slots[0].wrapped = flipped;
        assert_eq!(broken.unlock(KEY_A).unwrap_err(), KeyError::Wrong);
        assert!(broken.unlock(KEY_B).is_ok());
        // A's slot copied into another setup file doesn't open it (it is bound to its file).
        let (other, _) = Encryption::create_slots(&keys(&[KEY_C])).unwrap();
        let mut moved = other.clone();
        moved.slots = vec![encryption.slots[0].clone()];
        assert_eq!(moved.unlock(KEY_A).unwrap_err(), KeyError::Wrong);
        // And so is a changed check value.
        let mut forged = encryption.clone();
        forged.check = "0".repeat(64);
        assert_eq!(forged.unlock(KEY_A).unwrap_err(), KeyError::Wrong);
    }

    #[test]
    fn slot_data_that_is_damaged_is_refused_not_trusted() {
        let (encryption, _) = Encryption::create_slots(&keys(&[KEY_A])).unwrap();
        let mut none = encryption.clone();
        none.slots.clear();
        assert!(none.validate().is_err());
        let mut short = encryption.clone();
        short.slots[0].wrapped.truncate(20);
        assert!(short.validate().is_err());
        let mut odd = encryption.clone();
        odd.slots[0].salt = "zz".repeat(16);
        assert!(odd.validate().is_err());
        let mut many = encryption.clone();
        many.slots = vec![encryption.slots[0].clone(); MAX_SLOTS + 1];
        assert!(many.validate().is_err());
        assert!(matches!(none.unlock(KEY_A), Err(KeyError::Damaged(_))));
    }

    #[test]
    fn keys_for_slots_are_checked() {
        assert!(Encryption::create_slots(&[]).is_err());
        assert!(format!("{:#}", Encryption::create_slots(&keys(&[KEY_A, KEY_A])).unwrap_err()).contains("same as an earlier"));
        assert!(format!("{:#}", Encryption::create_slots(&keys(&[KEY_A, "short"])).unwrap_err()).contains("access key 2"));
        let many: Vec<String> = (0..=MAX_SLOTS).map(|n| format!("key-number-{n:05}")).collect();
        assert!(format!("{:#}", Encryption::create_slots(&many).unwrap_err()).contains("too many"));
    }

    #[test]
    fn a_setup_file_from_before_the_slots_still_opens() {
        // The single-key scheme stays readable: setup files in the wild must keep working.
        let (old, _) = Encryption::create(KEY_A).unwrap();
        assert_eq!(old.scheme, SCHEME);
        assert!(old.slots.is_empty() && old.unlock(KEY_A).is_ok());
        assert_eq!(old.unlock(KEY_B).unwrap_err(), KeyError::Wrong);
        // Its manifest JSON has no "slots" field at all.
        let json = serde_json::to_string(&old).unwrap();
        assert!(!json.contains("slots"));
        assert_eq!(serde_json::from_str::<Encryption>(&json).unwrap(), old);
    }
}
