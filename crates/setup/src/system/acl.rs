//! Who may change a folder, read from its access list.
//!
//! A machine-wide app runs its uninstaller, its `[[run]]` steps and its
//! updates with administrator rights from its install folder. If ordinary
//! users can write there, they can swap a program for their own and have it
//! run elevated. Program Files is safe by default, but a folder that already
//! exists in it (from an older installer, a restore or a script) can carry any
//! rights, so setup looks before it writes and before it runs anything there.

use std::path::Path;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{LocalFree, ERROR_SUCCESS, HLOCAL};
use windows::Win32::Security::Authorization::{ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT};
use windows::Win32::Security::{
    GetAce, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
};

/// The only accounts that may hold a right to change an elevated app's
/// folder: Administrators, SYSTEM, TrustedInstaller and CREATOR OWNER (an
/// inherit-only entry that stands for whoever made the item, which is one of
/// the first three here). Anyone else, a named user included, is a weakness.
const TRUSTED_WRITERS: &[&str] = &["S-1-5-32-544", "S-1-5-18", "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464", "S-1-3-0"];

/// Owners that can be trusted with a folder that runs elevated: Administrators,
/// SYSTEM and TrustedInstaller.
const TRUSTED_OWNERS: &[&str] = &["S-1-5-32-544", "S-1-5-18", "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464"];

/// Any right that changes what is in a folder or who may: write data, append,
/// extended attributes, delete a child, attributes, delete, change the access
/// list or the owner, and the generic write and all rights.
const CHANGE_RIGHTS: u32 = 0x0000_0002 // FILE_WRITE_DATA / FILE_ADD_FILE
    | 0x0000_0004 // FILE_APPEND_DATA / FILE_ADD_SUBDIRECTORY
    | 0x0000_0010 // FILE_WRITE_EA
    | 0x0000_0040 // FILE_DELETE_CHILD
    | 0x0000_0100 // FILE_WRITE_ATTRIBUTES
    | 0x0001_0000 // DELETE
    | 0x0004_0000 // WRITE_DAC
    | 0x0008_0000 // WRITE_OWNER
    | 0x4000_0000 // GENERIC_WRITE
    | 0x1000_0000; // GENERIC_ALL

/// What is wrong with a folder's rights, in a short phrase, or None when
/// only administrators, SYSTEM and TrustedInstaller can change it.
pub fn weakness(folder: &Path) -> Result<Option<String>, String> {
    let wide: Vec<u16> = folder.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    let mut owner = PSID::default();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: the out parameters point at live locals; the descriptor Windows
    // allocates is freed below, after the last use of `owner` and `dacl`,
    // which point into it.
    let result = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(wide.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut dacl),
            None,
            &mut descriptor,
        )
    };
    if result != ERROR_SUCCESS {
        return Err(format!("the rights of {} can't be read (error {})", folder.display(), result.0));
    }
    let answer = inspect(owner, dacl);
    // SAFETY: allocated by GetNamedSecurityInfoW, not used after this.
    unsafe {
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
    }
    Ok(answer)
}

fn inspect(owner: PSID, dacl: *const ACL) -> Option<String> {
    let Some(owner_text) = sid_text(owner) else {
        return Some("its owner can't be read".into());
    };
    if !TRUSTED_OWNERS.contains(&owner_text.as_str()) {
        return Some(format!("it belongs to {owner_text}, not to Administrators"));
    }
    if dacl.is_null() {
        return Some("it has no access list, so everyone may change it".into());
    }
    // SAFETY: `dacl` points at the ACL inside the security descriptor.
    let count = unsafe { (*dacl).AceCount };
    for index in 0..u32::from(count) {
        let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
        // SAFETY: index is below the ACL's own count.
        if unsafe { GetAce(dacl, index, &mut ace) }.is_err() || ace.is_null() {
            continue;
        }
        // SAFETY: GetAce returned a pointer to an ACE inside the ACL; the
        // header comes first and an allowed ACE starts with it.
        let header = unsafe { *(ace as *const ACE_HEADER) };
        match header.AceType {
            0 => {}
            // Denials and audit entries grant nothing.
            1 | 2 | 3 | 6 | 7 | 8 | 17 | 18 | 19 => continue,
            // Object, callback and other allow entries have a different layout
            // that isn't read here, so they can't be vouched for.
            _ => return Some("it has an access entry of a kind setup can't check".into()),
        }
        // SAFETY: AceType 0 is an ACCESS_ALLOWED_ACE, whose SID starts at SidStart.
        let allowed = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
        let sid = PSID(std::ptr::addr_of!(allowed.SidStart) as *mut _);
        let Some(who) = sid_text(sid) else {
            return Some("it has an access entry setup can't read".into());
        };
        if allowed.Mask & CHANGE_RIGHTS != 0 && !TRUSTED_WRITERS.contains(&who.as_str()) {
            return Some(format!("{} may change it", account_name(&who)));
        }
    }
    None
}

fn account_name(sid: &str) -> &'static str {
    match sid {
        "S-1-1-0" => "Everyone",
        "S-1-5-4" => "Interactive users",
        "S-1-5-2" => "Network users",
        "S-1-5-11" => "Authenticated Users",
        "S-1-5-32-545" => "Users",
        _ => "Another account",
    }
}

/// The most items looked at, so a huge install doesn't make setup crawl.
const LIMIT: usize = 100_000;

/// [`weakness`] for a folder and everything in it that matters: every
/// subfolder (someone who can write there can add a file), every file, and any
/// link that could lead somewhere else. Anything it can't read fails the check.
pub fn tree_weakness(folder: &Path) -> Result<Option<String>, String> {
    use std::os::windows::fs::MetadataExt;
    // The folder itself must be a real folder, not a link to somewhere else.
    if std::fs::symlink_metadata(folder).is_ok_and(|meta| meta.file_attributes() & 0x400 != 0) {
        return Ok(Some(format!("{} is a link", folder.display())));
    }
    // Whoever can rename or replace a folder above it controls it too, so the
    // folders between it and Program Files (which is safe by default) count.
    for above in folder.ancestors().skip(1).take_while(|above| super::folders::is_admin_only(above)) {
        if let Some(why) = weakness(above)? {
            return Ok(Some(format!("{}: {why}", above.display())));
        }
    }
    let mut seen = 0usize;
    walk(folder, &mut seen)
}

fn walk(folder: &Path, seen: &mut usize) -> Result<Option<String>, String> {
    if let Some(why) = weakness(folder)? {
        return Ok(Some(if *seen == 0 { why } else { format!("{}: {why}", folder.display()) }));
    }
    *seen += 1;
    // Anything setup can't list or look at counts against the folder: a
    // check that skips what it can't read would pass exactly the odd cases.
    let entries = std::fs::read_dir(folder).map_err(|err| format!("{} can't be listed ({err})", folder.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("{} can't be listed ({err})", folder.display()))?;
        *seen += 1;
        if *seen > LIMIT {
            return Ok(Some("it holds more files than setup can check".into()));
        }
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).map_err(|err| format!("{} can't be inspected ({err})", path.display()))?;
        use std::os::windows::fs::MetadataExt;
        // A symbolic link or junction (a reparse point) can lead out of the folder.
        if meta.file_attributes() & 0x400 != 0 {
            return Ok(Some(format!("{} is a link", path.display())));
        }
        if meta.is_dir() {
            if let Some(why) = walk(&path, seen)? {
                return Ok(Some(why));
            }
        } else if let Some(why) = weakness(&path)? {
            // Every file, not only the ones that look runnable: a data file the
            // app loads, or one with an odd extension, is just as good a way in.
            return Ok(Some(format!("{}: {why}", path.display())));
        }
    }
    Ok(None)
}

fn sid_text(sid: PSID) -> Option<String> {
    let mut text = PWSTR::null();
    // SAFETY: the SID comes from a security descriptor that is still alive.
    unsafe { ConvertSidToStringSidW(sid, &mut text) }.ok()?;
    // SAFETY: Windows returned a NUL-terminated string that we free here.
    let out = unsafe { text.to_string() }.ok();
    unsafe {
        let _ = LocalFree(Some(HLOCAL(text.0 as *mut _)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_own_folders_are_only_for_administrators() {
        let windows = std::env::var("SystemRoot").unwrap();
        assert_eq!(weakness(Path::new(&windows)).unwrap(), None);
        let files = std::env::var("ProgramFiles").unwrap();
        assert_eq!(weakness(Path::new(&files)).unwrap(), None);
    }

    #[test]
    fn a_folder_the_user_owns_is_not() {
        let dir = std::env::temp_dir().join(format!("bach-acl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let problem = weakness(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        // Created by an ordinary user it belongs to them. An elevated session's
        // folders belong to Administrators, and the temp folder gives Users no
        // write right of its own.
        if !super::super::elevate::is_elevated() {
            assert!(problem.is_some(), "a folder the user owns can be changed by them");
        }
    }

    #[test]
    fn windows_own_tree_is_checked_below_the_top_folder_too() {
        // Windows' own subfolders belong to administrators alone; a few are
        // too private for an ordinary user to read, which isn't a weakness.
        let windows = std::env::var("SystemRoot").unwrap();
        match tree_weakness(&Path::new(&windows).join("System32").join("en-US")) {
            Ok(found) => assert_eq!(found, None),
            Err(why) => assert!(why.contains("can't be read"), "{why}"),
        }
    }

    #[test]
    fn a_weak_file_or_link_deep_in_the_tree_is_found() {
        let dir = std::env::temp_dir().join(format!("bach-acl-tree-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("tool.exe"), b"MZ").unwrap();
        // The folder is the user's, so the very top already fails; what matters
        // is that the walk reports a reason instead of an error.
        if !super::super::elevate::is_elevated() {
            assert!(tree_weakness(&dir).unwrap().is_some());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_folder_is_an_error() {
        assert!(weakness(Path::new(r"C:\definitely\not\here")).is_err());
    }
}
