//! Windows Installer packages are a Windows format; the shared module compiles
//! here with stubs that find no package, so nothing on a Mac ever takes that path.

use super::Installed;

// The shared module is written for Windows Installer; here most of it is never used.
#[allow(dead_code, unused_imports)]
#[path = "../system/msi.rs"]
mod windows_msi;

pub use windows_msi::*;
