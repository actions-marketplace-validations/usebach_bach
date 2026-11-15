#![no_main]
use libfuzzer_sys::fuzz_target;

// An update feed from a host the app doesn't control.
fuzz_target!(|data: &[u8]| {
    if let Ok(entry) = serde_json::from_slice::<bach_pack::manifest::FeedEntry>(data) {
        let _ = bach_pack::feedsig::verify(&"ab".repeat(32), "App", &entry);
        let _ = bach_pack::version::compare(&entry.version, "1.0.0");
    }
});
