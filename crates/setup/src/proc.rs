//! Starting helper programs without a console window on Windows, where
//! macOS has nothing to hide.

use std::process::Command;

/// Keeps a helper program from opening a console window.
pub trait NoWindow {
    fn no_window(&mut self) -> &mut Self;
}

impl NoWindow for Command {
    #[cfg(windows)]
    fn no_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW
        self.creation_flags(0x0800_0000)
    }

    #[cfg(not(windows))]
    fn no_window(&mut self) -> &mut Self {
        self
    }
}

/// The name of the null device for curl's `--output`.
pub const NULL_DEVICE: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };
