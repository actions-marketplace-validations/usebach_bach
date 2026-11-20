//! What the end-to-end tests of the newer commands share: a project folder
//! with an app to package, and a way to run `bach` in it on the real runtime.
//! (`build.rs` predates this and keeps its own copies.)
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub fn bach() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bach"))
}

/// The setup runtime a build can use, if this checkout has built one.
pub fn runtime() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_BIN_EXE_bach")).parent()?.to_path_buf();
    ["bach-setup.exe", "bach-setup"].iter().map(|name| dir.join(name)).find(|path| path.is_file())
}

/// Skips a test when `bach-setup.exe` isn't built next to `bach`.
#[macro_export]
macro_rules! need_runtime {
    () => {
        if common::runtime().is_none() {
            eprintln!("skipped: bach-setup.exe isn't built next to bach (run `cargo build` first)");
            return;
        }
    };
}

/// A project folder with a small program and 1 MB of incompressible data in `app/`.
pub fn project(name: &str, config: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bach-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("app")).unwrap();
    std::fs::write(dir.join("app/hello.exe"), b"MZ").unwrap();
    let mut state = 0x9E37_79B9u32;
    let noise: Vec<u8> = (0..1_000_000)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect();
    std::fs::write(dir.join("app/data.bin"), noise).unwrap();
    std::fs::write(dir.join("bach.toml"), config).unwrap();
    dir
}

pub fn config(extra: &str) -> String {
    format!(
        r#"[app]
name = "Hello"
id = "BachIntegrationTest-Hello"
version = "1.0.0"
publisher = "Example"
executable = "hello.exe"

[files]
source = "app"

{extra}"#
    )
}

pub fn run(dir: &Path, args: &[&str]) -> Output {
    let runtime = runtime().expect("a runtime");
    bach().current_dir(dir).env("BACH_RUNTIME", runtime).args(args).output().expect("bach starts")
}

pub fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
