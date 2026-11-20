//! Facts about this computer and this session that setup checks before it
//! changes anything: the user's languages, free space, and whether another
//! setup for the same app is already at work.

use std::path::Path;
use std::time::Duration;

use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_TIMEOUT};
use windows::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
use windows::Win32::System::Threading::{CreateMutexW, OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};

/// The user's Windows display languages, most preferred first, like
/// `["de-DE", "en-US"]`. Empty if Windows won't say.
/// The processor architecture of the program that is running, as a feed
/// names it: `arm64` or `x64`. An installed app updates with its own.
pub fn own_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    }
}

/// The architecture of the computer, as a feed names it. A setup file built
/// for x64 runs on Windows on ARM too (emulated), so it asks the computer, not
/// itself: an online installer then takes the ARM64 download there.
pub fn native_arch() -> &'static str {
    use windows::Win32::System::SystemInformation::IMAGE_FILE_MACHINE;
    use windows::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};
    const ARM64: u16 = 0xAA64;
    let mut process = IMAGE_FILE_MACHINE(0);
    let mut native = IMAGE_FILE_MACHINE(0);
    // SAFETY: both pointers are valid for the call, and the pseudo handle needs no closing.
    let asked = unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, Some(&mut native)) };
    if asked.is_ok() && native.0 == ARM64 {
        "arm64"
    } else {
        own_arch()
    }
}

pub fn ui_languages() -> Vec<String> {
    let mut count = 0u32;
    let mut len = 0u32;
    // Safety: the first call only reports the buffer length it needs; the
    // second fills a buffer of exactly that many UTF-16 units.
    unsafe {
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, None, &mut len).is_err() || len == 0 {
            return Vec::new();
        }
        let mut buffer = vec![0u16; len as usize];
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, Some(PWSTR(buffer.as_mut_ptr())), &mut len).is_err() {
            return Vec::new();
        }
        // A list of NUL-terminated names, ending in an empty one.
        buffer.split(|unit| *unit == 0).filter(|name| !name.is_empty()).map(String::from_utf16_lossy).collect()
    }
}

/// Bytes free for this user on the drive that holds `path` (or would: the
/// nearest folder above it that exists). None if Windows won't say.
pub fn free_space(path: &Path) -> Option<u64> {
    let existing = path.ancestors().find(|folder| folder.is_dir())?;
    let mut free = 0u64;
    // Safety: `free` outlives the call; the other outputs are optional.
    unsafe { GetDiskFreeSpaceExW(&HSTRING::from(existing.as_os_str()), Some(&mut free), None, None).ok()? };
    Some(free)
}

/// Held while a setup for one app installs or removes it, so two setup
/// windows (or a window and a silent run) never work on the same install
/// at once. Released when dropped.
pub struct SetupLock(HANDLE);

// Safety: a mutex handle may be closed from any thread.
unsafe impl Send for SetupLock {}

impl SetupLock {
    /// None when another setup for `app_id` holds the lock.
    pub fn acquire(app_id: &str) -> Option<Self> {
        Self::named(&format!("Local\\BachSetup-{app_id}"))
    }

    /// The lock for one app's updater (`--apply-update`, `--auto-update`),
    /// so two of them never download or apply at the same time. It is not
    /// the setup lock: the setup file an updater starts takes that one.
    pub fn acquire_updater(app_id: &str) -> Option<Self> {
        Self::named(&format!("Local\\BachUpdate-{app_id}"))
    }

    fn named(name: &str) -> Option<Self> {
        let name = HSTRING::from(name);
        // Safety: a named mutex owned by this handle until Drop closes it.
        unsafe {
            let handle = CreateMutexW(None, true, &name).ok()?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                return None;
            }
            Some(Self(handle))
        }
    }
}

impl Drop for SetupLock {
    fn drop(&mut self) {
        // Safety: our own handle, closed once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Waits until the process `pid` has ended, up to `timeout`. True when it
/// has (or when there is no such process, which comes to the same).
pub fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
    // Safety: a synchronize-only handle to the process, closed below.
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) else {
            return true;
        };
        let millis = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
        let waited = WaitForSingleObject(process, millis);
        let _ = CloseHandle(process);
        waited != WAIT_TIMEOUT
    }
}

/// Lets a silent run started from a terminal print there, although setup
/// is a windowed program. Does nothing when there's no terminal.
pub fn attach_parent_console() {
    // Safety: no pointers; failing just means there's no console to use.
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// Tells the person who started setup what is wrong with its command line.
/// A terminal gets the sentence on stderr. Someone who double-clicked it, in a
/// session with a screen, gets a message box; a service or deployment tool
/// (session 0) gets only the exit code, because nobody could ever click it.
pub fn show_command_line_mistake(text: &str) {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE};
    use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    attach_parent_console();
    eprintln!("{text}");
    // Safety: plain handle and session queries; the text outlives the call.
    unsafe {
        let has_console = GetStdHandle(STD_ERROR_HANDLE).is_ok_and(|handle: HANDLE| !handle.is_invalid() && !handle.0.is_null());
        let mut session = 0u32;
        let _ = ProcessIdToSessionId(GetCurrentProcessId(), &mut session);
        if !has_console && session != 0 {
            let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
            let title: Vec<u16> = "Setup".encode_utf16().chain(std::iter::once(0)).collect();
            MessageBoxW(None, PCWSTR(wide.as_ptr()), PCWSTR(title.as_ptr()), MB_OK | MB_ICONERROR);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_space_looks_above_missing_folders() {
        let missing = std::env::temp_dir().join("bach-free-space").join("not").join("there");
        assert!(free_space(&missing).is_some_and(|free| free > 0));
    }

    #[test]
    fn one_lock_per_app() {
        let id = format!("test-{}", std::process::id());
        let first = SetupLock::acquire(&id).expect("the first lock");
        assert!(SetupLock::acquire(&id).is_none(), "a second setup must wait");
        drop(first);
        assert!(SetupLock::acquire(&id).is_some(), "free again once released");
    }

    #[test]
    fn languages_look_like_tags() {
        for language in ui_languages() {
            assert!(language.len() >= 2 && !language.contains('\0'), "{language:?}");
        }
    }
}
