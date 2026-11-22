//! `bach-setup`: the installer and uninstaller window every Bach setup file
//! runs. On its own it installs nothing; `bach build` appends an app's
//! files and manifest to a copy of it, and that copy is the setup file.
//! See `launch.rs` for how one executable plays every part.

// Release builds are a GUI app: no console window behind it.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(any(windows, target_os = "macos"))]
mod access;
#[cfg(any(windows, target_os = "macos"))]
mod actions;
#[cfg(any(windows, target_os = "macos"))]
mod app;
#[cfg(any(windows, target_os = "macos"))]
mod assets;
#[cfg(any(windows, target_os = "macos"))]
mod keyboard;
#[cfg(any(windows, target_os = "macos"))]
mod launch;
#[cfg(any(windows, target_os = "macos"))]
mod license;
#[cfg(any(windows, target_os = "macos"))]
mod log;
#[cfg(any(windows, target_os = "macos"))]
mod package;
#[cfg(any(windows, target_os = "macos"))]
mod pages;
#[cfg(any(windows, target_os = "macos"))]
mod picture;
#[cfg(any(windows, target_os = "macos"))]
mod proc;
#[cfg(any(windows, target_os = "macos"))]
mod silent;
#[cfg(any(windows, target_os = "macos"))]
mod start;
mod support;
#[cfg(windows)]
mod system;
#[cfg(target_os = "macos")]
#[path = "system_mac/mod.rs"]
mod system;
#[cfg(any(windows, target_os = "macos"))]
mod texts;
#[cfg(any(windows, target_os = "macos"))]
mod theme;
#[cfg(any(windows, target_os = "macos"))]
mod ui;
#[cfg(any(windows, target_os = "macos"))]
mod updates;
#[cfg(any(windows, target_os = "macos"))]
mod view;
#[cfg(any(windows, target_os = "macos"))]
mod work;

#[cfg(any(windows, target_os = "macos"))]
fn main() {
    app::main();
}

#[cfg(not(any(windows, target_os = "macos")))]
fn main() {
    eprintln!("bach-setup runs on Windows and macOS. Use `bach build` on any system to build installers for them.");
    std::process::exit(1);
}
