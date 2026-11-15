//! Signed update feeds.
//!
//! A feed names a download and its SHA-256, and both come from the same host,
//! so whoever controls that host (or a mirror) controls the update. The
//! publisher can take that away: `bach feed-key` makes an Ed25519 key pair, the
//! public half goes into `[updates] public_key` and with it into every setup
//! file, and `bach build` and `bach feed` sign each feed entry with the private
//! half. An installed app with a public key accepts only entries whose signature
//! checks out, wherever they were downloaded from.
//!
//! The signature covers what decides what gets installed (app id, version,
//! link, SHA-256, size, rollout and the patch offered), not the release notes.
//! Keys and signatures are lowercase hex.

use anyhow::{bail, Context, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use crate::manifest::{DeltaEntry, FeedEntry};

/// The environment variable that holds the private key (hex) for signing.
pub const KEY_ENV: &str = "BACH_FEED_KEY";
/// Like [`KEY_ENV`], but the path of a file that holds it.
pub const KEY_FILE_ENV: &str = "BACH_FEED_KEY_FILE";

/// A new key pair as (private key, public key), both hex.
pub fn generate() -> Result<(String, String)> {
    let mut seed = [0u8; 32];
    crate::protect::fill_random(&mut seed)?;
    let key = SigningKey::from_bytes(&seed);
    Ok((crate::protect::hex(&seed), crate::protect::hex(key.verifying_key().as_bytes())))
}

/// The public key (hex) that belongs to a private key (hex).
pub fn public_of(private: &str) -> Result<String> {
    Ok(crate::protect::hex(signing_key(private)?.verifying_key().as_bytes()))
}

/// Whether `public` is a well-formed public key.
pub fn check_public(public: &str) -> Result<()> {
    verifying_key(public).map(|_| ())
}

/// The bytes that are signed for `entry` of the app `app_id`.
fn message(app_id: &str, entry: &FeedEntry) -> Vec<u8> {
    let delta = |delta: &DeltaEntry| {
        format!(
            "{}|{}|{}|{}",
            delta.from,
            delta.url,
            delta.sha256.to_ascii_lowercase(),
            delta.size.map_or(String::new(), |size| size.to_string())
        )
    };
    format!(
        "bach-feed-v1\n{app_id}\n{}\n{}\n{}\n{}\n{}\n{}",
        entry.version,
        entry.url,
        entry.sha256.to_ascii_lowercase(),
        entry.size.map_or(String::new(), |size| size.to_string()),
        entry.rollout.map_or(String::new(), |rollout| rollout.to_string()),
        entry.delta.as_ref().map_or(String::new(), delta),
    )
    .into_bytes()
}

/// Signs `entry` with the private key and gives the signature (hex).
pub fn sign(private: &str, app_id: &str, entry: &FeedEntry) -> Result<String> {
    let key = signing_key(private)?;
    Ok(crate::protect::hex(&key.sign(&message(app_id, entry)).to_bytes()))
}

/// Checks `entry`'s signature against the publisher's public key, in a
/// sentence a person can read when it doesn't hold.
pub fn verify(public: &str, app_id: &str, entry: &FeedEntry) -> Result<(), String> {
    let key = verifying_key(public).map_err(|err| format!("{err:#}"))?;
    let Some(signature) = entry.signature.as_deref() else {
        return Err("the update feed isn't signed by the publisher".into());
    };
    let bytes = unhex::<64>(signature).ok_or("the update feed's signature isn't valid")?;
    key.verify(&message(app_id, entry), &Signature::from_bytes(&bytes))
        .map_err(|_| "the update feed's signature doesn't match the publisher's key".to_string())
}

fn signing_key(private: &str) -> Result<SigningKey> {
    let Some(seed) = unhex::<32>(private.trim()) else {
        bail!("the feed signing key must be 64 hex digits (make one with `bach feed-key`)");
    };
    Ok(SigningKey::from_bytes(&seed))
}

fn verifying_key(public: &str) -> Result<VerifyingKey> {
    let Some(bytes) = unhex::<32>(public.trim()) else {
        bail!("the public key must be 64 hex digits");
    };
    VerifyingKey::from_bytes(&bytes).context("the public key isn't a valid Ed25519 key")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> FeedEntry {
        FeedEntry {
            version: "2.0.0".into(),
            url: "https://example.com/App-Setup-2.0.0.exe".into(),
            sha256: "ab".repeat(32),
            notes: Some("what is new".into()),
            size: Some(1234),
            rollout: Some(50),
            delta: Some(DeltaEntry {
                from: "1.0.0".into(),
                url: "https://example.com/patch.exe".into(),
                sha256: "cd".repeat(32),
                size: Some(10),
            }),
            signature: None,
        }
    }

    fn signed(private: &str) -> FeedEntry {
        let mut entry = entry();
        entry.signature = Some(sign(private, "Hello", &entry).unwrap());
        entry
    }

    #[test]
    fn a_signed_entry_verifies_and_notes_may_change() {
        let (private, public) = generate().unwrap();
        assert_eq!(public_of(&private).unwrap(), public);
        let mut entry = signed(&private);
        assert!(verify(&public, "Hello", &entry).is_ok());
        entry.notes = Some("other notes".into());
        assert!(verify(&public, "Hello", &entry).is_ok(), "notes aren't part of the signature");
    }

    #[test]
    fn every_part_that_decides_the_install_is_covered() {
        let (private, public) = generate().unwrap();
        let good = signed(&private);
        type Change = Box<dyn Fn(&mut FeedEntry)>;
        let changes: Vec<(&str, Change)> = vec![
            ("the hash", Box::new(|e| e.sha256 = "ef".repeat(32))),
            ("the link", Box::new(|e| e.url = "https://evil.example/App.exe".into())),
            ("the version", Box::new(|e| e.version = "9.0.0".into())),
            ("the size", Box::new(|e| e.size = Some(1))),
            ("the rollout", Box::new(|e| e.rollout = None)),
            ("the patch link", Box::new(|e| e.delta.as_mut().unwrap().url = "https://evil.example/p.exe".into())),
            ("the patch hash", Box::new(|e| e.delta.as_mut().unwrap().sha256 = "00".repeat(32))),
            ("dropping the patch", Box::new(|e| e.delta = None)),
        ];
        for (what, change) in changes {
            let mut entry = good.clone();
            change(&mut entry);
            assert!(verify(&public, "Hello", &entry).is_err(), "changing {what} must break the signature");
        }
        assert!(verify(&public, "Other app", &good).is_err(), "another app id");
    }

    #[test]
    fn the_wrong_key_unsigned_and_damaged_feeds_are_refused() {
        let (private, public) = generate().unwrap();
        let (_, other_public) = generate().unwrap();
        let good = signed(&private);
        assert!(verify(&other_public, "Hello", &good).unwrap_err().contains("doesn't match"));
        assert!(verify(&public, "Hello", &entry()).unwrap_err().contains("isn't signed"));
        let mut damaged = good.clone();
        damaged.signature = Some("zz".into());
        assert!(verify(&public, "Hello", &damaged).unwrap_err().contains("isn't valid"));
    }

    #[test]
    fn keys_must_be_well_formed() {
        assert!(check_public("short").is_err());
        assert!(check_public(&"zz".repeat(32)).is_err());
        assert!(sign("short", "Hello", &entry()).is_err());
        assert!(public_of("not hex").is_err());
    }
}
