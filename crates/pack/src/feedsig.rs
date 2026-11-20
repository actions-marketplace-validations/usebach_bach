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

use crate::manifest::{ArchEntry, DeltaEntry, FeedEntry};

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

/// The bytes that are signed for `entry` of the app `app_id`. An entry with a
/// `published` time is signed as `bach-feed-v2`, which adds it at the end; one
/// without keeps the first format, so feeds written before the time existed
/// still verify. Taking the time off a v2 entry breaks the signature, since the
/// two formats start differently.
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
    let (label, published) = match &entry.published {
        Some(time) => ("bach-feed-v2", format!("\n{time}")),
        None => ("bach-feed-v1", String::new()),
    };
    format!(
        "{label}\n{app_id}\n{}\n{}\n{}\n{}\n{}\n{}{published}",
        entry.version,
        entry.url,
        entry.sha256.to_ascii_lowercase(),
        entry.size.map_or(String::new(), |size| size.to_string()),
        entry.rollout.map_or(String::new(), |rollout| rollout.to_string()),
        entry.delta.as_ref().map_or(String::new(), delta),
    )
    .into_bytes()
}

/// The bytes that are signed for the setup file `arch` of version `version`
/// of the app `app_id`. It is its own message, not part of the entry's: an
/// older app that checks the entry's signature sees the same bytes it always
/// did, and ignores the `arch` list it doesn't know.
fn arch_message(app_id: &str, version: &str, arch: &str, entry: &ArchEntry) -> Vec<u8> {
    format!(
        "bach-feed-arch-v1
{app_id}
{version}
{arch}
{}
{}
{}",
        entry.url,
        entry.sha256.to_ascii_lowercase(),
        entry.size.map_or(String::new(), |size| size.to_string()),
    )
    .into_bytes()
}

/// Signs the `arch` file of the feed entry for `version` and gives the signature (hex).
pub fn sign_arch(private: &str, app_id: &str, version: &str, arch: &str, entry: &ArchEntry) -> Result<String> {
    let key = signing_key(private)?;
    Ok(crate::protect::hex(&key.sign(&arch_message(app_id, version, arch, entry)).to_bytes()))
}

/// Checks the signature of the `arch` file offered for `version`.
pub fn verify_arch(public: &str, app_id: &str, version: &str, arch: &str, entry: &ArchEntry) -> Result<(), String> {
    let key = verifying_key(public).map_err(|err| format!("{err:#}"))?;
    let Some(signature) = entry.signature.as_deref() else {
        return Err(format!("the {arch} download in the update feed isn't signed by the publisher"));
    };
    let bytes = unhex::<64>(signature).ok_or_else(|| format!("the {arch} download's signature isn't valid"))?;
    key.verify(&arch_message(app_id, version, arch, entry), &Signature::from_bytes(&bytes))
        .map_err(|_| format!("the {arch} download's signature doesn't match the publisher's key"))
}

/// Seconds since 1970-01-01 as `2026-11-17T06:30:00Z`.
pub fn format_utc(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = if month_part < 10 { month_part + 3 } else { month_part - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// The inverse of [`format_utc`]: only the exact `YYYY-MM-DDTHH:MM:SSZ` form.
pub fn parse_utc(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.len() != 20
        || !text.is_ascii()
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return None;
    }
    let number = |from: usize, to: usize| -> Option<i64> {
        let part = &text[from..to];
        part.chars().all(|c| c.is_ascii_digit()).then(|| part.parse().ok()).flatten()
    };
    let (year, month, day) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
    let (hour, minute, second) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
    if !(1970..=9999).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    // Civil date to days since 1970-01-01.
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let year_of_era = y.rem_euclid(400);
    let month_part = (month + 9) % 12;
    let day_of_year = (153 * month_part + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second;
    // Round trip: 2026-02-31 isn't a date.
    let seconds = u64::try_from(seconds).ok()?;
    (format_utc(seconds) == text).then_some(seconds)
}

/// How far in the future a `published` time may be before it is distrusted:
/// a computer's clock is never exact.
const CLOCK_SKEW_SECONDS: u64 = 24 * 3600;

/// Whether `entry` is recent enough for an app that sets `max_age_days`
/// (0 or none: always). `now` is seconds since 1970. Only call it for an
/// entry whose signature has been checked: the time means something only then.
pub fn check_fresh(entry: &FeedEntry, max_age_days: Option<u32>, now: u64) -> Result<(), String> {
    let Some(days) = max_age_days.filter(|days| *days > 0) else {
        return Ok(());
    };
    let published = entry.published.as_deref().and_then(parse_utc).ok_or_else(|| {
        format!("the update feed has no publishing time, and this app only takes feeds signed within the last {days} days")
    })?;
    if published > now.saturating_add(CLOCK_SKEW_SECONDS) {
        return Err("the update feed says it was published in the future, so this computer's clock or the feed is wrong".into());
    }
    let age_days = now.saturating_sub(published) / 86_400;
    if age_days > u64::from(days) {
        return Err(format!(
            "the update feed was signed {age_days} days ago, and this app only takes feeds signed within the last {days} days"
        ));
    }
    Ok(())
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
            published: None,
            arch: Default::default(),
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

    #[test]
    fn times_are_written_and_read_the_same_way() {
        for seconds in [0u64, 86_399, 951_782_400 /* 2000-02-29 */, 1_794_897_360, 4_102_444_800, 253_402_300_799] {
            let text = format_utc(seconds);
            assert_eq!(parse_utc(&text), Some(seconds), "{text}");
        }
        assert_eq!(format_utc(1_794_897_360), "2026-11-17T06:36:00Z");
        for bad in [
            "",
            "2026-11-17",
            "2026-11-17T06:36:00",
            "2026-11-17 06:36:00Z",
            "2026-02-31T00:00:00Z",
            "2026-13-01T00:00:00Z",
            "1969-12-31T23:59:59Z",
            "2026-11-17T24:00:00Z",
            "+026-11-17T06:36:00Z",
            "２０２６-11-17T06:36:00Z",
        ] {
            assert_eq!(parse_utc(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_publishing_time_is_signed_and_cannot_be_taken_off() {
        let (private, public) = generate().unwrap();
        let mut entry = entry();
        entry.published = Some("2026-11-17T06:36:00Z".into());
        entry.signature = Some(sign(&private, "Hello", &entry).unwrap());
        assert!(verify(&public, "Hello", &entry).is_ok());
        let mut later = entry.clone();
        later.published = Some("2027-01-01T00:00:00Z".into());
        assert!(verify(&public, "Hello", &later).is_err(), "a changed time breaks the signature");
        let mut stripped = entry.clone();
        stripped.published = None;
        assert!(verify(&public, "Hello", &stripped).is_err(), "so does taking it off");
        // An entry written before the time existed still verifies.
        let mut old = super::tests::entry();
        old.signature = Some(sign(&private, "Hello", &old).unwrap());
        assert!(verify(&public, "Hello", &old).is_ok());
    }

    #[test]
    fn an_old_entry_is_refused_when_the_app_sets_a_limit() {
        let now = parse_utc("2026-11-17T06:36:00Z").unwrap();
        let mut entry = entry();
        // No limit: anything goes, dated or not.
        assert!(check_fresh(&entry, None, now).is_ok());
        assert!(check_fresh(&entry, Some(0), now).is_ok());
        // A limit needs a time.
        assert!(check_fresh(&entry, Some(30), now).unwrap_err().contains("no publishing time"));
        entry.published = Some("2026-11-01T00:00:00Z".into());
        assert!(check_fresh(&entry, Some(30), now).is_ok());
        entry.published = Some("2026-09-01T00:00:00Z".into());
        assert!(check_fresh(&entry, Some(30), now).unwrap_err().contains("days ago"));
        entry.published = Some("2026-11-20T00:00:00Z".into());
        assert!(check_fresh(&entry, Some(30), now).unwrap_err().contains("future"));
        entry.published = Some("2026-11-17T20:00:00Z".into());
        assert!(check_fresh(&entry, Some(30), now).is_ok(), "a little clock skew is fine");
        entry.published = Some("garbage".into());
        assert!(check_fresh(&entry, Some(30), now).is_err());
    }

    fn arm() -> ArchEntry {
        ArchEntry { url: "https://example.com/Setup-arm64.exe".into(), sha256: "cd".repeat(32), size: Some(700), signature: None }
    }

    #[test]
    fn a_listing_for_another_architecture_is_signed_on_its_own() {
        let (private, public) = generate().unwrap();
        let mut listed = arm();
        listed.signature = Some(sign_arch(&private, "Hello", "2.0.0", "arm64", &listed).unwrap());
        assert!(verify_arch(&public, "Hello", "2.0.0", "arm64", &listed).is_ok());

        type Tamper = (&'static str, Box<dyn Fn(&mut ArchEntry)>);
        let tampered: [Tamper; 3] = [
            ("the link", Box::new(|e| e.url = "https://evil.example/a.exe".into())),
            ("the hash", Box::new(|e| e.sha256 = "00".repeat(32))),
            ("the size", Box::new(|e| e.size = Some(1))),
        ];
        for (what, change) in tampered {
            let mut copy = listed.clone();
            change(&mut copy);
            assert!(verify_arch(&public, "Hello", "2.0.0", "arm64", &copy).is_err(), "changing {what} must break it");
        }
        // It belongs to one app, one version and one architecture.
        assert!(verify_arch(&public, "Other", "2.0.0", "arm64", &listed).is_err());
        assert!(verify_arch(&public, "Hello", "2.0.1", "arm64", &listed).is_err(), "an old signature can't vouch for a new version");
        assert!(verify_arch(&public, "Hello", "2.0.0", "x64", &listed).is_err());
        // Without a signature, or with one from another key, it is refused.
        assert!(verify_arch(&public, "Hello", "2.0.0", "arm64", &arm()).unwrap_err().contains("isn't signed"));
        let (other, _) = generate().unwrap();
        let forged = ArchEntry { signature: Some(sign_arch(&other, "Hello", "2.0.0", "arm64", &arm()).unwrap()), ..arm() };
        assert!(verify_arch(&public, "Hello", "2.0.0", "arm64", &forged).is_err());
    }

    #[test]
    fn listing_other_architectures_does_not_change_what_the_entry_signs() {
        // An app built before `arch` existed checks the entry as it always did: the list is no part of it.
        let (private, public) = generate().unwrap();
        let mut plain = entry();
        plain.signature = Some(sign(&private, "Hello", &plain).unwrap());
        let mut with_arm = plain.clone();
        with_arm.arch.insert("arm64".into(), arm());
        assert!(verify(&public, "Hello", &with_arm).is_ok());
    }
}
