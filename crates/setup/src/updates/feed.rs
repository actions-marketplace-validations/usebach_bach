//! Reading the update feed: the JSON `bach build` writes next to a setup
//! file, `{ "version", "url", "sha256", "notes", "size" }`.

use std::cmp::Ordering;
use std::path::Path;

use bach_pack::manifest::FeedEntry;

use super::curl::{self, Fetch};

/// Fetches and checks the feed at `url`. `work` is a folder to keep the
/// download in while it is read.
pub fn fetch(url: &str, work: &Path, bearer: Option<&str>) -> Result<FeedEntry, String> {
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
pub fn fetch_offers(urls: &[String], work: &Path, bearer: Option<&str>) -> Result<Vec<FeedEntry>, String> {
    let mut entries = Vec::new();
    let mut first_error = None;
    for url in urls {
        match fetch(url, work, bearer) {
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
        let with = |version: &str, url: &str, sha: &str| {
            format!(r#"{{"version":"{version}","url":"{url}","sha256":"{sha}"}}"#)
        };
        assert!(parse("not json").is_err());
        assert!(parse(&with("nightly", "https://x/y.exe", SHA)).is_err(), "no version to compare");
        assert!(parse(&with("1.0", "http://x/y.exe", SHA)).is_err(), "not https");
        assert!(parse(&with("1.0", "https://x/y.exe", "abc")).is_err(), "short hash");
    }

    fn entry(version: &str, url: &str, sha: &str) -> FeedEntry {
        FeedEntry { version: version.into(), url: url.into(), sha256: sha.into(), notes: None, size: None, rollout: None }
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
