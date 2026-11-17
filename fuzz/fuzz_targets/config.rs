#![no_main]
use libfuzzer_sys::fuzz_target;

// A bach.toml.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let root = std::env::temp_dir();
    let _ = bach_pack::config::parse(&text, &root);
});
