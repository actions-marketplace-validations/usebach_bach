//! Reading the update feed: the JSON `bach build` writes next to a setup
//! file, `{ "version", "url", "sha256", "notes", "size" }`.

use std::cmp::Ordering;
use std::path::Path;

use bach_pack::manifest::FeedEntry;

use super::curl::{self, Bearer, Fetch};

/// The publisher's key that feed entries must be signed with.
#[derive(Clone, Copy)]
pub struct Signer<'a> {
    pub public_key: &'a str,
    /// A second key that is trusted too (see `[updates] next_public_key`).
    pub next_public_key: Option<&'a str>,
    pub app_id: &'a str,
}

/// The signer an installed app asks for: its manifest's key, when it has one.
pub fn signer_of(manifest: &bach_pack::manifest::Manifest) -> Option<Signer<'_>> {
    let updates = manifest.updates.as_ref()?;
    let public_key = updates.public_key.as_deref()?;
    Some(Signer { public_key, next_public_key: updates.next_public_key.as_deref(), app_id: &manifest.app.id })
}

/// Fetches and checks the feed at `url`. `work` is a folder to keep the
/// download in while it is read.
pub fn fetch(url: &str, work: &Path, bearer: Option<&Bearer>) -> Result<FeedEntry, String> {
    std::fs::create_dir_all(work).map_err(|err| err.to_string())?;
    let file = work.join("feed.json");
    let _ = std::fs::remove_file(&file);
    let result = curl::download_with(url, &file, Fetch::FEED, bearer, &mut |_| {})
        .and_then(|()| std::fs::read_to_string(&file).map_err(|err| err.to_string()))
        .and_then(|text| parse(&text));
    let _ = std::fs::remove_file(&file);
    result
}

/// Asks every feed (the main one, then its mirrors) and gives what they offer.
///
/// The first feed that answers decides which version is the newest; a mirror
/// never gets to say "there's a newer one", because a mirror is often the
/// weaker host and must not be able to widen what an installed app trusts. The
/// rest of the answers are kept only when they offer exactly the same file
/// (same version, same SHA-256): those are other places to download the same
/// bytes from if the first one is down. One feed that can't be reached is no
/// problem while another answers; when none does, the first one's complaint is
/// the answer.
pub fn fetch_offers(urls: &[String], work: &Path, bearer: Option<&Bearer>, signer: Option<Signer>) -> Result<Vec<FeedEntry>, String> {
    let mut entries = Vec::new();
    let mut first_error = None;
    for url in urls {
        let answer = fetch(url, work, bearer).and_then(|entry| accepted(entry, signer));
        match answer {
            Ok(entry) => entries.push(entry),
            Err(error) => {
                crate::log::line(&format!("The feed {url} didn't answer: {error}"));
                first_error.get_or_insert(error);
            }
        }
    }
    if entries.is_empty() {
        return Err(first_error.unwrap_or_else(|| "there is no update feed to ask".to_string()));
    }
    Ok(agreeing(entries))
}

/// `entry`, if the publisher's signature allows it. The signature decides, not
/// the host that served it: a mirror or a changed host can't offer anything the
/// key didn't sign. An app without a key takes every entry, as before.
pub fn accepted(entry: FeedEntry, signer: Option<Signer>) -> Result<FeedEntry, String> {
    match signer {
        Some(signer) => {
            // Either trusted key may have signed it; the first key's complaint is the one to show.
            let first = bach_pack::feedsig::verify(signer.public_key, signer.app_id, &entry);
            match (first, signer.next_public_key) {
                (Ok(()), _) => Ok(entry),
                (Err(err), None) => Err(err),
                (Err(err), Some(next)) => bach_pack::feedsig::verify(next, signer.app_id, &entry).map(|()| entry).map_err(|_| err),
            }
        }
        None => Ok(entry),
    }
}

/// The first entry, then the others that offer the same version with the
/// same SHA-256 (the first one leads, and no other can change what it says).
pub fn agreeing(entries: Vec<FeedEntry>) -> Vec<FeedEntry> {
    let mut entries = entries.into_iter();
    let Some(first) = entries.next() else {
        return Vec::new();
    };
    let same = |other: &FeedEntry| {
        bach_pack::version::compare(&other.version, &first.version) == Some(Ordering::Equal)
            && other.sha256.trim().eq_ignore_ascii_case(first.sha256.trim())
    };
    let mut offers = vec![first.clone()];
    for other in entries {
        if same(&other) {
            offers.push(other);
        } else {
            crate::log::line(&format!("A mirror offers {} instead of {}; ignoring it.", other.version, first.version));
        }
    }
    offers
}

/// Reads feed text, refusing anything an update couldn't safely use.
pub fn parse(text: &str) -> Result<FeedEntry, String> {
    let entry: FeedEntry =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|_| "the update feed isn't valid".to_string())?;
    if bach_pack::version::compare(&entry.version, &entry.version).is_none() {
        return Err(format!("the update feed's version \"{}\" isn't a version", entry.version));
    }
    if !bach_pack::manifest::extras::is_https(&entry.url) {
        return Err("the update feed points at a link that isn't https".into());
    }
    if !is_sha256(&entry.sha256) {
        return Err("the update feed has no valid SHA-256".into());
    }
    if entry.rollout.is_some_and(|percent| percent > 100) {
        return Err("the update feed's rollout isn't a percentage".into());
    }
    // A patch that isn't usable is no reason to refuse the update: the full
    // setup file is still there, so the patch is just left out.
    let mut entry = entry;
    if let Some(delta) = &entry.delta {
        let usable = bach_pack::version::compare(&delta.from, &delta.from).is_some()
            && bach_pack::manifest::extras::is_https(&delta.url)
            && is_sha256(&delta.sha256);
        if !usable {
            crate::log::line("The update feed's patch isn't usable, so only the full setup file is offered.");
            entry.delta = None;
        }
    }
    Ok(entry)
}

/// 64 hexadecimal digits.
pub fn is_sha256(text: &str) -> bool {
    let text = text.trim();
    text.len() == 64 && text.chars().all(|c| c.is_ascii_hexdigit())
}

/// Whether `version` is newer than `current`. A version that can't be
/// compared is never taken for an update.
pub fn is_newer(version: &str, current: &str) -> bool {
    bach_pack::version::compare(version, current) == Some(Ordering::Greater)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn a_good_feed_reads() {
        let json = format!(r#"{{"version":"1.2.0","url":"https://x/y.exe","sha256":"{SHA}","notes":"New","size":42}}"#);
        let entry = parse(&json).unwrap();
        assert_eq!((entry.version.as_str(), entry.size), ("1.2.0", Some(42)));
        assert!(parse(&format!("\u{feff}{json}")).is_ok(), "a byte order mark is fine");
    }

    #[test]
    fn a_bad_feed_is_refused() {
        let with = |version: &str, url: &str, sha: &str| format!(r#"{{"version":"{version}","url":"{url}","sha256":"{sha}"}}"#);
        assert!(parse("not json").is_err());
        assert!(parse(&with("nightly", "https://x/y.exe", SHA)).is_err(), "no version to compare");
        assert!(parse(&with("1.0", "http://x/y.exe", SHA)).is_err(), "not https");
        assert!(parse(&with("1.0", "https://x/y.exe", "abc")).is_err(), "short hash");
    }

    /// Feed text comes from a host the app doesn't control: damaged, it must be refused or
    /// read, never a panic.
    #[test]
    fn a_damaged_feed_is_refused_never_a_panic() {
        let good = br#"{"version":"2.0.0","url":"https://example.com/a.exe","sha256":"abababababababababababababababababababababababababababababababab","size":10,"rollout":20,"delta":{"from":"1.0.0","url":"https://example.com/p.exe","sha256":"cd"}}"#;
        assert!(parse(std::str::from_utf8(good).unwrap()).is_ok());
        let mut state = 0x1234_5678_9ABC_DEF1u64;
        let mut next = move || {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            state.wrapping_mul(0x2545_F491_4F6C_DD1D)
        };
        for _ in 0..3000 {
            let mut bytes = good.to_vec();
            for _ in 0..1 + next() % 5 {
                let at = (next() % bytes.len() as u64) as usize;
                match next() % 4 {
                    0 => bytes[at] = next() as u8,
                    1 => bytes.truncate(at),
                    2 => bytes.insert(at, next() as u8),
                    _ => {
                        bytes.remove(at);
                    }
                }
                if bytes.is_empty() {
                    break;
                }
            }
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let _ = std::panic::catch_unwind(|| parse(&text)).unwrap_or_else(|_| panic!("parse panicked on {text:?}"));
        }
    }

    #[test]
    fn a_publisher_key_makes_the_signature_the_only_way_in() {
        let (private, public) = bach_pack::feedsig::generate().unwrap();
        let signer = Some(Signer { public_key: &public, next_public_key: None, app_id: "Hello" });
        let mut good = entry("2.0.0", "https://example.com/Hello-2.0.0.exe", &"ab".repeat(32));
        good.signature = Some(bach_pack::feedsig::sign(&private, "Hello", &good).unwrap());
        assert!(accepted(good.clone(), signer).is_ok());

        // The same file offered by a mirror that changed the link or the hash.
        let moved = FeedEntry { url: "https://mirror.example/evil.exe".into(), ..good.clone() };
        assert!(accepted(moved, signer).unwrap_err().contains("doesn't match"));
        let swapped = FeedEntry { sha256: "cd".repeat(32), ..good.clone() };
        assert!(accepted(swapped, signer).is_err());
        // Unsigned, and signed by somebody else.
        let unsigned = FeedEntry { signature: None, ..good.clone() };
        assert!(accepted(unsigned.clone(), signer).unwrap_err().contains("isn't signed"));
        let (other, _) = bach_pack::feedsig::generate().unwrap();
        let forged = FeedEntry { signature: Some(bach_pack::feedsig::sign(&other, "Hello", &unsigned).unwrap()), ..unsigned.clone() };
        assert!(accepted(forged, signer).is_err());
        // Without a key in the setup file, nothing changes.
        assert!(accepted(unsigned, None).is_ok());
    }

    #[test]
    fn the_manifests_key_names_the_signer() {
        let mut manifest = crate::package::Package::sample().manifest;
        assert!(signer_of(&manifest).is_none());
        manifest.updates = Some(bach_pack::manifest::Updates {
            feed: "https://x.example/latest.json".into(),
            auto: bach_pack::manifest::Choice::Never,
            mirrors: Vec::new(),
            public_key: Some("ab".repeat(32)),
            next_public_key: Some("cd".repeat(32)),
            signer: None,
        });
        let signer = signer_of(&manifest).unwrap();
        assert_eq!((signer.public_key, signer.app_id), ("ab".repeat(32).as_str(), manifest.app.id.as_str()));
        assert_eq!(signer.next_public_key, Some("cd".repeat(32).as_str()));
    }

    #[test]
    fn a_second_key_is_trusted_for_the_hand_over() {
        let (old_private, old_public) = bach_pack::feedsig::generate().unwrap();
        let (new_private, new_public) = bach_pack::feedsig::generate().unwrap();
        let (other_private, _) = bach_pack::feedsig::generate().unwrap();
        let unsigned = entry("2.0.0", "https://example.com/Hello-2.0.0.exe", &"ab".repeat(32));
        let signed_by = |private: &str| FeedEntry {
            signature: Some(bach_pack::feedsig::sign(private, "Hello", &unsigned).unwrap()),
            ..unsigned.clone()
        };

        // Copies that know both keys take a feed signed with either: the old
        // key keeps working while the new one takes over.
        let both = Some(Signer { public_key: &old_public, next_public_key: Some(&new_public), app_id: "Hello" });
        assert!(accepted(signed_by(&old_private), both).is_ok());
        assert!(accepted(signed_by(&new_private), both).is_ok());
        assert!(accepted(signed_by(&other_private), both).is_err(), "a third key is nobody's");
        // Copies that only know the old key (not updated yet) refuse the new one.
        let only_old = Some(Signer { public_key: &old_public, next_public_key: None, app_id: "Hello" });
        assert!(accepted(signed_by(&new_private), only_old).is_err());
    }

    fn entry(version: &str, url: &str, sha: &str) -> FeedEntry {
        FeedEntry {
            version: version.into(),
            url: url.into(),
            sha256: sha.into(),
            notes: None,
            size: None,
            rollout: None,
            delta: None,
            signature: None,
        }
    }

    #[test]
    fn a_mirror_can_never_offer_a_newer_version_than_the_main_feed() {
        let main = entry("1.0.0", "https://main/a.exe", SHA);
        let evil = entry("9.9.9", "https://mirror/evil.exe", &"ab".repeat(32));
        let offers = agreeing(vec![main.clone(), evil]);
        assert_eq!(offers, [main], "a bigger number on a mirror changes nothing");
    }

    #[test]
    fn mirrors_may_only_serve_the_same_file() {
        let main = entry("1.0.0", "https://main/a.exe", SHA);
        let same = entry("1.0.0", "https://mirror/a.exe", &SHA.to_uppercase());
        let other_hash = entry("1.0.0", "https://mirror/b.exe", &"cd".repeat(32));
        let same_number = entry("1.0", "https://cdn/a.exe", SHA);
        let offers = agreeing(vec![main.clone(), other_hash, same.clone(), same_number.clone()]);
        assert_eq!(offers, [main, same, same_number]);
    }

    #[test]
    fn when_the_main_feed_is_down_the_first_mirror_leads() {
        // fetch_offers drops feeds that didn't answer, so the first answer is
        // what `agreeing` sees first.
        let first = entry("2.0.0", "https://mirror/a.exe", SHA);
        let later = entry("3.0.0", "https://other/a.exe", &"ab".repeat(32));
        assert_eq!(agreeing(vec![first.clone(), later]), [first]);
        assert!(agreeing(Vec::new()).is_empty());
    }

    #[test]
    fn only_a_higher_version_is_an_update() {
        assert!(is_newer("1.10.0", "1.9.9"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("0.9", "1.0"));
        assert!(!is_newer("2.0.0-beta.1", "2.0.0"), "a pre-release isn't newer than its release");
        assert!(!is_newer("nightly", "1.0"));
    }
}
