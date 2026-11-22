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
    /// How old a signed entry may be, in days (`[updates] max_age_days`).
    pub max_age_days: Option<u32>,
}

/// The signer an installed app asks for: its manifest's key, when it has one.
pub fn signer_of(manifest: &bach_pack::manifest::Manifest) -> Option<Signer<'_>> {
    let updates = manifest.updates.as_ref()?;
    let public_key = updates.public_key.as_deref()?;
    Some(Signer {
        public_key,
        next_public_key: updates.next_public_key.as_deref(),
        app_id: &manifest.app.id,
        max_age_days: updates.max_age_days,
    })
}

/// Fetches and checks the feed at `url`. `work` is a folder to keep the
/// download in while it is read.
pub fn fetch(url: &str, work: &Path, bearer: Option<&Bearer>) -> Result<FeedEntry, String> {
    std::fs::create_dir_all(work).map_err(|err| err.to_string())?;
    let file = work.join("feed.json");
    let _ = std::fs::remove_file(&file);
    let result =
        curl::download_with(url, &file, Fetch::FEED, bearer, &mut |_| {}).and_then(|()| read_limited(&file)).and_then(|text| parse(&text));
    let _ = std::fs::remove_file(&file);
    result
}

/// The most a feed may be: it is a few lines of JSON.
const MAX_FEED_BYTES: u64 = 1 << 20;

/// Reads the downloaded feed, refusing one bigger than [`MAX_FEED_BYTES`]. curl's own
/// limit only works when the server says the size up front (and older curl versions
/// don't stop an answer of unknown length), so the file is measured here, and
/// at most that much is read, before anything is made of it.
fn read_limited(file: &Path) -> Result<String, String> {
    use std::io::Read;
    let size = std::fs::metadata(file).map_err(|err| err.to_string())?.len();
    if size > MAX_FEED_BYTES {
        return Err(format!("the update feed is {size} bytes, more than the {MAX_FEED_BYTES} a feed may be"));
    }
    let mut text = String::new();
    std::fs::File::open(file).and_then(|open| open.take(MAX_FEED_BYTES + 1).read_to_string(&mut text)).map_err(|err| err.to_string())?;
    Ok(text)
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
pub fn fetch_offers(
    urls: &[String],
    work: &Path,
    bearer: Option<&Bearer>,
    signer: Option<Signer>,
    arch: &str,
) -> Result<Vec<FeedEntry>, String> {
    let mut entries = Vec::new();
    let mut first_error = None;
    for url in urls {
        let answer = fetch(url, work, bearer)
            .and_then(|entry| accepted(entry, signer))
            .map(|entry| pick_arch(entry, arch, signer))
            .and_then(mac_download_only);
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

/// On a Mac only a macOS setup file (`.dmg` or `.zip`) is ever downloaded: a feed that
/// lists none, because it was published for Windows only, offers this Mac nothing, and
/// the Windows setup file it does list is never taken for one.
fn mac_download_only(entry: FeedEntry) -> Result<FeedEntry, String> {
    if cfg!(target_os = "macos") {
        let path = entry.url.split(['?', '#']).next().unwrap_or_default().to_ascii_lowercase();
        if !(path.ends_with(".dmg") || path.ends_with(".zip")) {
            return Err("the update feed lists no macOS setup file".to_string());
        }
    }
    Ok(entry)
}

/// The entry for a computer of architecture `arch` (`x64` or `arm64`): the
/// entry itself, or, when it lists a setup file for that architecture, that
/// file's link, SHA-256 and size in its place. A listing the publisher's key
/// didn't sign, or that isn't usable, is left out and the entry stands: an x64
/// setup file runs on Windows on ARM too, so the worst a missing listing costs
/// is the emulated download. An update patch is for the x64 file, so it goes.
pub fn pick_arch(entry: FeedEntry, arch: &str, signer: Option<Signer>) -> FeedEntry {
    let Some(listed) = entry.arch.get(arch) else {
        return entry;
    };
    let plain = |text: &str| !text.chars().any(char::is_control);
    let usable = bach_pack::manifest::extras::is_https(&listed.url) && plain(&listed.url) && is_sha256(&listed.sha256);
    let trusted = match signer {
        None => true,
        Some(signer) => {
            let check = |key: &str| bach_pack::feedsig::verify_arch(key, signer.app_id, &entry.version, arch, listed);
            check(signer.public_key).or_else(|first| signer.next_public_key.map_or(Err(first), check)).is_ok()
        }
    };
    if !usable || !trusted {
        crate::log::line(&format!(
            "The update feed's {arch} download isn't usable or isn't signed by the publisher, so the default one is taken."
        ));
        return entry;
    }
    crate::log::line(&format!("Taking the {arch} download the feed lists."));
    FeedEntry { url: listed.url.clone(), sha256: listed.sha256.clone(), size: listed.size, delta: None, ..entry }
}

/// `entry`, if the publisher's signature allows it. The signature decides, not
/// the host that served it: a mirror or a changed host can't offer anything the
/// key didn't sign. An app without a key takes every entry, as before.
pub fn accepted(entry: FeedEntry, signer: Option<Signer>) -> Result<FeedEntry, String> {
    accepted_at(entry, signer, now())
}

/// [`accepted`] at a given time (seconds since 1970).
fn accepted_at(entry: FeedEntry, signer: Option<Signer>, now: u64) -> Result<FeedEntry, String> {
    match signer {
        Some(signer) => {
            // Either trusted key may have signed it; the first key's complaint is the one to show.
            let first = bach_pack::feedsig::verify(signer.public_key, signer.app_id, &entry);
            match (first, signer.next_public_key) {
                (Ok(()), _) => {}
                (Err(err), None) => return Err(err),
                (Err(err), Some(next)) => bach_pack::feedsig::verify(next, signer.app_id, &entry).map_err(|_| err)?,
            }
            // Only a signed time means anything, so freshness is judged after the signature.
            bach_pack::feedsig::check_fresh(&entry, signer.max_age_days, now)?;
            Ok(entry)
        }
        None => Ok(entry),
    }
}

/// Seconds since 1970 on this computer.
fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())
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
    // The signed text separates its fields with line breaks and bars, so a link or a version with
    // control characters in it could be read as another entry's fields.
    let plain = |text: &str| !text.chars().any(char::is_control);
    if !plain(&entry.url) || !plain(&entry.version) || entry.published.as_deref().is_some_and(|time| !plain(time)) {
        return Err("the update feed has control characters in its link or version".into());
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
            && plain(&delta.url)
            && plain(&delta.from)
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
    #[test]
    #[cfg(target_os = "macos")]
    fn a_mac_never_takes_a_windows_download() {
        let entry = |url: &str| FeedEntry {
            version: "2.0.0".into(),
            url: url.into(),
            sha256: "ab".repeat(32),
            notes: None,
            size: None,
            rollout: None,
            delta: None,
            signature: None,
            published: None,
            arch: Default::default(),
        };
        assert!(super::mac_download_only(entry("https://x.example/Setup-2.0.0.exe")).is_err());
        assert!(super::mac_download_only(entry("https://x.example/Setup-2.0.0.dmg?raw=1")).is_ok());
        assert!(super::mac_download_only(entry("https://x.example/Setup-2.0.0.ZIP")).is_ok());
    }

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
        let signer = Some(Signer { public_key: &public, next_public_key: None, app_id: "Hello", max_age_days: None });
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
            beta_feed: None,
            max_age_days: None,
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
        let both = Some(Signer { public_key: &old_public, next_public_key: Some(&new_public), app_id: "Hello", max_age_days: None });
        assert!(accepted(signed_by(&old_private), both).is_ok());
        assert!(accepted(signed_by(&new_private), both).is_ok());
        assert!(accepted(signed_by(&other_private), both).is_err(), "a third key is nobody's");
        // Copies that only know the old key (not updated yet) refuse the new one.
        let only_old = Some(Signer { public_key: &old_public, next_public_key: None, app_id: "Hello", max_age_days: None });
        assert!(accepted(signed_by(&new_private), only_old).is_err());
    }

    fn entry(version: &str, url: &str, sha: &str) -> FeedEntry {
        FeedEntry {
            published: None,
            version: version.into(),
            url: url.into(),
            sha256: sha.into(),
            notes: None,
            size: None,
            rollout: None,
            delta: None,
            signature: None,
            arch: Default::default(),
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

    #[test]
    fn a_feed_over_a_megabyte_is_refused_before_it_is_read() {
        let file = std::env::temp_dir().join(format!("bach-feed-big-{}.json", std::process::id()));
        std::fs::write(&file, vec![b' '; (MAX_FEED_BYTES + 1) as usize]).unwrap();
        assert!(read_limited(&file).unwrap_err().contains("more than"));
        std::fs::write(&file, b"{}").unwrap();
        assert_eq!(read_limited(&file).unwrap(), "{}");
        std::fs::remove_file(&file).unwrap();
    }

    #[test]
    fn control_characters_in_a_link_or_version_are_refused() {
        let with = |version: &str, url: &str| format!(r#"{{"version":"{version}","url":"{url}","sha256":"{SHA}"}}"#);
        assert!(parse(&with("1.0", "https://x/y.exe")).is_ok());
        assert!(parse(&with("1.0", "https://x/y.exe\\nabc")).is_err(), "a line break in the link");
        assert!(parse(&with("1.0\\u0007", "https://x/y.exe")).is_err());
    }

    #[test]
    fn an_old_signed_feed_is_refused_when_the_app_limits_the_age() {
        let (private, public) = bach_pack::feedsig::generate().unwrap();
        let limited = Signer { public_key: &public, next_public_key: None, app_id: "Hello", max_age_days: Some(30) };
        let signer = Some(limited);
        let now = bach_pack::feedsig::parse_utc("2026-11-17T06:36:00Z").unwrap();
        let signed = |published: Option<&str>| {
            let mut entry = entry("2.0.0", "https://example.com/Hello-2.0.0.exe", &"ab".repeat(32));
            entry.published = published.map(String::from);
            entry.signature = Some(bach_pack::feedsig::sign(&private, "Hello", &entry).unwrap());
            entry
        };
        assert!(accepted_at(signed(Some("2026-11-10T00:00:00Z")), signer, now).is_ok());
        assert!(accepted_at(signed(Some("2026-06-01T00:00:00Z")), signer, now).unwrap_err().contains("days ago"));
        assert!(accepted_at(signed(None), signer, now).unwrap_err().contains("no publishing time"));
        // Taking the time off an entry that had one is a bad signature, whatever the limit.
        let mut stripped = signed(Some("2026-11-10T00:00:00Z"));
        stripped.published = None;
        assert!(accepted_at(stripped, signer, now).unwrap_err().contains("doesn't match"));
        // Without a limit nothing changes.
        let open = Some(Signer { max_age_days: None, ..limited });
        assert!(accepted_at(signed(Some("2020-01-01T00:00:00Z")), open, now).is_ok());
    }

    fn listed_arm() -> bach_pack::manifest::ArchEntry {
        bach_pack::manifest::ArchEntry { url: "https://x/Setup-arm64.exe".into(), sha256: "cd".repeat(32), size: Some(90), signature: None }
    }

    #[test]
    fn a_computer_takes_the_download_listed_for_its_architecture() {
        let mut main = entry("2.0.0", "https://x/Setup.exe", SHA);
        main.size = Some(100);
        main.delta = Some(bach_pack::manifest::DeltaEntry {
            from: "1.0.0".into(),
            url: "https://x/patch.exe".into(),
            sha256: SHA.into(),
            size: Some(5),
        });
        main.arch.insert("arm64".into(), listed_arm());
        // x64 (and anything not listed) keeps the entry as it is.
        assert_eq!(pick_arch(main.clone(), "x64", None), main);
        assert_eq!(pick_arch(main.clone(), "x86", None), main);
        // ARM64 gets its own file, and not the x64 patch.
        let picked = pick_arch(main.clone(), "arm64", None);
        assert_eq!(
            (picked.url.as_str(), picked.sha256.as_str(), picked.size),
            ("https://x/Setup-arm64.exe", "cd".repeat(32).as_str(), Some(90))
        );
        assert!(picked.delta.is_none() && picked.version == "2.0.0");
    }

    #[test]
    fn an_unusable_listing_falls_back_to_the_default_download() {
        let mut main = entry("2.0.0", "https://x/Setup.exe", SHA);
        for broken in [
            bach_pack::manifest::ArchEntry { url: "http://x/a.exe".into(), ..listed_arm() },
            bach_pack::manifest::ArchEntry { sha256: "nothex".into(), ..listed_arm() },
            bach_pack::manifest::ArchEntry {
                url: "https://x/a
b.exe"
                    .into(),
                ..listed_arm()
            },
        ] {
            main.arch.insert("arm64".into(), broken);
            assert_eq!(pick_arch(main.clone(), "arm64", None), main);
        }
    }

    #[test]
    fn a_signed_app_takes_only_a_signed_listing() {
        let (private, public) = bach_pack::feedsig::generate().unwrap();
        let signer = Signer { public_key: &public, next_public_key: None, app_id: "Hello", max_age_days: None };
        let mut main = entry("2.0.0", "https://x/Setup.exe", SHA);
        main.arch.insert("arm64".into(), listed_arm());
        assert_eq!(pick_arch(main.clone(), "arm64", Some(signer)), main, "an unsigned listing is left out");

        let mut listed = listed_arm();
        listed.signature = Some(bach_pack::feedsig::sign_arch(&private, "Hello", "2.0.0", "arm64", &listed).unwrap());
        main.arch.insert("arm64".into(), listed.clone());
        assert_eq!(pick_arch(main.clone(), "arm64", Some(signer)).url, "https://x/Setup-arm64.exe");

        // A host that swaps the link keeps the signature, which no longer fits.
        let mut swapped = main.clone();
        swapped.arch.get_mut("arm64").unwrap().url = "https://evil.example/a.exe".into();
        assert_eq!(pick_arch(swapped.clone(), "arm64", Some(signer)), swapped);
        // The next key may have signed it, too.
        let (next_private, next_public) = bach_pack::feedsig::generate().unwrap();
        let mut by_next = listed_arm();
        by_next.signature = Some(bach_pack::feedsig::sign_arch(&next_private, "Hello", "2.0.0", "arm64", &by_next).unwrap());
        main.arch.insert("arm64".into(), by_next);
        let both = Signer { next_public_key: Some(&next_public), ..signer };
        assert_eq!(pick_arch(main.clone(), "arm64", Some(both)).url, "https://x/Setup-arm64.exe");
        assert_eq!(pick_arch(main.clone(), "arm64", Some(signer)), main, "the first key alone doesn't know that signature");
    }
}
