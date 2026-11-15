//! `bach key`: random access keys for an app with `[access]`.

use anyhow::{bail, Result};

/// The most keys one call makes; a customer list can come in several calls.
const MOST: u32 = 10_000;

/// Prints `count` keys, or writes them to `out` as UTF-8 text. A file is what
/// `bach build --key-file` reads, and writing it here sidesteps what a shell's
/// `>` does to the text (Windows PowerShell makes UTF-16).
pub fn command(count: u32, out: Option<&std::path::Path>) -> Result<()> {
    if count == 0 || count > MOST {
        bail!("--count is {count}; make between 1 and {MOST} keys at a time");
    }
    let mut keys = Vec::new();
    for _ in 0..count {
        keys.push(bach_pack::protect::generate_key()?);
    }
    match out {
        Some(file) => {
            if file.exists() {
                bail!("{} exists already; Bach doesn't overwrite a file of keys", file.display());
            }
            std::fs::write(file, format!("{}\n", keys.join("\n")))
                .map_err(|err| anyhow::anyhow!("couldn't write {}: {err}", file.display()))?;
            say!("Wrote {} to {}", if count == 1 { "1 key".to_string() } else { format!("{count} keys") }, file.display());
        }
        None => {
            for key in keys {
                say!("{key}");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_are_limited() {
        assert!(command(0, None).is_err());
        assert!(command(MOST + 1, None).is_err());
    }
}
