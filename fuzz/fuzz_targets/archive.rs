#![no_main]
use libfuzzer_sys::fuzz_target;
use std::io::Read;

// The payload archive of a setup file: any bytes must end in Ok or Err.
fuzz_target!(|data: &[u8]| {
    let _ = bach_pack::archive::read(data, &mut |_name, _size, body| {
        let mut sink = Vec::new();
        body.take(1 << 20).read_to_end(&mut sink)?;
        Ok(())
    });
});
