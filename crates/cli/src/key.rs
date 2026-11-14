//! `bach key`: random access keys for an app with `[access]`.

use anyhow::{bail, Result};

/// The most keys one call makes; a customer list can come in several calls.
const MOST: u32 = 10_000;

pub fn command(count: u32) -> Result<()> {
    if count == 0 || count > MOST {
        bail!("--count is {count}; make between 1 and {MOST} keys at a time");
    }
    for _ in 0..count {
        println!("{}", bach_pack::protect::generate_key()?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_are_limited() {
        assert!(command(0).is_err());
        assert!(command(MOST + 1).is_err());
    }
}
