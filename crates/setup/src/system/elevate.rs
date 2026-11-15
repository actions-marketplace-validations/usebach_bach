//! Administrator rights, asked for once and only when an app needs them.
//!
//! Setup normally runs as the user and never asks. Two things change that:
//! a machine-wide install (`installer.scope = "machine"`), where setup starts
//! again elevated before it does anything, and a `[[run]]` step with
//! `elevated = true`, where only that program is started elevated.
//!
//! Both go through `ShellExecuteEx` with the `runas` verb, which is what makes
//! Windows show its consent prompt (or ask for an administrator's password).
//! Nothing here sidesteps that prompt, and when setup is elevated already
//! Windows doesn't ask again.

use std::path::Path;
use std::time::{Duration, Instant};

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, OpenProcessToken, TerminateProcess, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL};

/// How an elevated program ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It ran and ended with this exit code.
    Exited(u32),
    /// The user said no to Windows' prompt.
    Declined,
    /// It took longer than allowed, so setup ended it.
    TimedOut,
    /// Windows couldn't start it, for this reason.
    Failed(String),
}

/// Whether this process already has administrator rights.
pub fn is_elevated() -> bool {
    // Safety: a query-only handle to our own token, closed below; the
    // output buffer is exactly one TOKEN_ELEVATION.
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0u32;
        let asked = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut TOKEN_ELEVATION as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        );
        let _ = CloseHandle(token);
        asked.is_ok() && elevation.TokenIsElevated != 0
    }
}

/// Starts `program` with administrator rights and waits for it, up to
/// `limit`. `hidden` keeps its window away (a helper program), which a setup
/// window that starts again must not do.
pub fn run(program: &Path, parameters: &str, directory: Option<&Path>, limit: Duration, hidden: bool) -> Outcome {
    let _com = super::ComThread::enter();
    let file = HSTRING::from(program.as_os_str());
    let parameters = HSTRING::from(parameters);
    let directory = directory.map(|dir| HSTRING::from(dir.as_os_str()));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: windows::core::w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        lpDirectory: directory.as_ref().map_or(PCWSTR::null(), |dir| PCWSTR(dir.as_ptr())),
        nShow: if hidden { SW_HIDE.0 } else { SW_SHOWNORMAL.0 },
        ..Default::default()
    };
    // Safety: `info` and the strings it points at outlive both calls; the
    // process handle Windows returns is ours to close.
    unsafe {
        if let Err(err) = ShellExecuteExW(&mut info) {
            return if err.code() == ERROR_CANCELLED.to_hresult() { Outcome::Declined } else { Outcome::Failed(err.message()) };
        }
        let process = info.hProcess;
        if process.is_invalid() {
            return Outcome::Failed("Windows didn't give back the program it started".into());
        }
        let started = Instant::now();
        let outcome = loop {
            if WaitForSingleObject(process, 150) == WAIT_OBJECT_0 {
                let mut code = 0u32;
                break match GetExitCodeProcess(process, &mut code) {
                    Ok(()) => Outcome::Exited(code),
                    Err(err) => Outcome::Failed(err.message()),
                };
            }
            if started.elapsed() > limit {
                let _ = TerminateProcess(process, 1);
                break Outcome::TimedOut;
            }
        };
        let _ = CloseHandle(process);
        outcome
    }
}

/// Starts this same program again with the same command line, elevated, and
/// waits for it: what a machine-wide setup does before it touches anything.
/// There's no time limit; a person is using the window it opens.
pub fn relaunch(arguments: &[String]) -> Outcome {
    let Ok(exe) = std::env::current_exe() else {
        return Outcome::Failed("Windows didn't say where setup is".into());
    };
    let line = arguments.iter().map(|argument| quote(argument)).collect::<Vec<_>>().join(" ");
    run(&exe, &line, exe.parent(), Duration::MAX, false)
}

/// One argument as it must be written on a Windows command line so the other
/// side reads it back unchanged.
pub fn quote(argument: &str) -> String {
    if !argument.is_empty() && !argument.contains([' ', '\t', '\n', '"']) {
        return argument.to_string();
    }
    let mut out = String::from('"');
    let mut backslashes = 0usize;
    for c in argument.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                // Backslashes before a quote are doubled, and the quote escaped.
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            c => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    // Trailing backslashes would escape the closing quote.
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_survive_the_command_line() {
        assert_eq!(quote("--silent"), "--silent");
        assert_eq!(quote("C:\\Program Files\\App"), "\"C:\\Program Files\\App\"");
        assert_eq!(quote(""), "\"\"");
        assert_eq!(quote("say \"hi\""), "\"say \\\"hi\\\"\"");
        // A trailing backslash must not swallow the closing quote.
        assert_eq!(quote("C:\\My Dir\\"), "\"C:\\My Dir\\\\\"");
    }

    #[test]
    fn the_test_process_knows_whether_it_is_elevated() {
        // Either answer is fine; it must just be an answer, not a crash.
        let _ = is_elevated();
    }
}
