//! Keeping an app's access key on this computer, so updates can use it
//! without asking again.
//!
//! The key is sealed with the Windows Data Protection API (DPAPI), which ties
//! it to this Windows user account: another user, or the same files copied to
//! another computer, can't read it. The app's id goes in as extra entropy, so
//! one app's sealed key can't be swapped in for another's. It lives in
//! `HKCU\Software\Bach\Access`, one value per app, and is deleted when the
//! app is removed.
//!
//! This is as safe as the account is: a program running as the same user can
//! ask Windows to unseal it, which is also true of every browser's saved
//! passwords. It keeps the key out of plain sight, not out of reach.

use anyhow::{anyhow, Context, Result};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};
use windows_registry::{Type, CURRENT_USER};

const KEY: &str = r"Software\Bach\Access";

/// Seals `key` and keeps it for `app_id`.
pub fn store(app_id: &str, key: &str) -> Result<()> {
    let sealed = seal(key.as_bytes(), app_id.as_bytes())?;
    let registry = CURRENT_USER.create(KEY).context("couldn't open the place the access key is kept")?;
    registry.set_bytes(app_id, Type::Bytes, &sealed).context("couldn't keep the access key")?;
    Ok(())
}

/// The key kept for `app_id`, if there is one this user can open.
pub fn load(app_id: &str) -> Option<String> {
    let registry = CURRENT_USER.open(KEY).ok()?;
    let sealed = registry.get_value(app_id).ok()?;
    let plain = unseal(&sealed, app_id.as_bytes()).ok()?;
    String::from_utf8(plain).ok().filter(|key| !key.is_empty())
}

/// Deletes the key kept for `app_id`; one that isn't there is already gone.
pub fn forget(app_id: &str) {
    // Opened for writing: a plain `open` is read-only and can't delete.
    if let Ok(registry) = CURRENT_USER.options().read().write().open(KEY) {
        let _ = registry.remove_value(app_id);
    }
}

fn blob(bytes: &[u8]) -> Result<CRYPT_INTEGER_BLOB> {
    Ok(CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(bytes.len()).map_err(|_| anyhow!("that is too much data to seal"))?,
        pbData: bytes.as_ptr() as *mut u8,
    })
}

/// Copies what Windows allocated and frees it.
///
/// # Safety
/// `out` must be a blob Windows filled in for us, not yet freed.
unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if out.pbData.is_null() {
        return Vec::new();
    }
    let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
    let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
    bytes
}

fn seal(data: &[u8], entropy: &[u8]) -> Result<Vec<u8>> {
    let input = blob(data)?;
    let extra = blob(entropy)?;
    let mut out = CRYPT_INTEGER_BLOB::default();
    // Safety: both inputs point at slices that outlive the call; `out` is
    // filled by Windows and handed to `take`, which frees it.
    unsafe {
        CryptProtectData(&input, PCWSTR::null(), Some(&extra), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)
            .context("Windows couldn't seal the access key")?;
        Ok(take(out))
    }
}

fn unseal(data: &[u8], entropy: &[u8]) -> Result<Vec<u8>> {
    let input = blob(data)?;
    let extra = blob(entropy)?;
    let mut out = CRYPT_INTEGER_BLOB::default();
    // Safety: as in `seal`.
    unsafe {
        CryptUnprotectData(&input, None, Some(&extra), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)
            .context("Windows couldn't open the kept access key")?;
        Ok(take(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(name: &str) -> String {
        format!("BachSecretTest-{name}-{}", std::process::id())
    }

    #[test]
    fn a_key_is_kept_and_forgotten() {
        let app = id("keep");
        assert_eq!(load(&app), None);
        store(&app, "K7QF-3MWD-9XPA-H2TY-6NBC").unwrap();
        assert_eq!(load(&app).as_deref(), Some("K7QF-3MWD-9XPA-H2TY-6NBC"));
        store(&app, "another-key-1234").unwrap();
        assert_eq!(load(&app).as_deref(), Some("another-key-1234"), "a new key replaces the old");
        forget(&app);
        assert_eq!(load(&app), None);
        forget(&app); // forgetting twice is fine
    }

    #[test]
    fn the_kept_bytes_are_not_the_key() {
        let sealed = seal(b"K7QF-3MWD-9XPA-H2TY-6NBC", b"app").unwrap();
        assert!(!sealed.windows(8).any(|window| window == b"K7QF-3MW"));
        assert_eq!(unseal(&sealed, b"app").unwrap(), b"K7QF-3MWD-9XPA-H2TY-6NBC");
    }

    #[test]
    fn one_apps_key_cannot_be_read_as_anothers() {
        let sealed = seal(b"secret-key-1234", b"app-one").unwrap();
        assert!(unseal(&sealed, b"app-two").is_err());
    }

    #[test]
    fn damaged_data_is_refused() {
        let mut sealed = seal(b"secret-key-1234", b"app").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0xFF;
        assert!(unseal(&sealed, b"app").is_err());
        assert!(unseal(b"nonsense", b"app").is_err());
    }
}
