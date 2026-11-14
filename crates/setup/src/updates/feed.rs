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

/// Asks every feed (the main one and its mirrors) and gives the entries that
/// offer the newest version, the one asked first leading. One feed that
/// can't be reached is no problem while another answers; when none does, the
/// first one's complaint is the answer.
pub fn fetch_newest(urls: &[String], work: &Path, bearer: Option<&str>) -> Result<Vec<FeedEntry>, String> {
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
    Ok(newest(entries))
}

/// The entries with the highest version, in their order.
pub fn newest(entries: Vec<FeedEntry>) -> Vec<FeedEntry> {
    let best = entries
        .iter()
        .map(|entry| entry.version.clone())
        .reduce(|best, version| if is_newer(&version, &best) { version } else { best });
    match best {
        Some(best) => entries
            .into_iter()
            .filter(|entry| bach_pack::version::compare(&entry.version, &best) == Some(Ordering::Equal))
            .collect(),
        None => entries,
    }
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

    #[test]
    fn only_a_higher_version_is_an_update() {
        assert!(is_newer("1.10.0", "1.9.9"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("0.9", "1.0"));
        assert!(!is_newer("2.0.0-beta.1", "2.0.0"), "a pre-release isn't newer than its release");
        assert!(!is_newer("nightly", "1.0"));
    }
}
