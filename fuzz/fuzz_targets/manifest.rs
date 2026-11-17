#![no_main]
use libfuzzer_sys::fuzz_target;

// The manifest in a setup file, read and validated.
fuzz_target!(|data: &[u8]| {
    if let Ok(manifest) = serde_json::from_slice::<bach_pack::manifest::Manifest>(data) {
        let _ = manifest.validate();
        let _ = manifest.as_installed(&bach_pack::manifest::Choices::defaults(&manifest));
    }
});
