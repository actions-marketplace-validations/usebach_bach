//! What the Bach builder and the setup runtime agree on.
//!
//! - [`config`]: `bach.toml`, read and checked, with the files it names.
//! - [`manifest`]: the description of an app that travels inside a setup
//!   file (name, version, files, options, look, texts).
//! - [`i18n`]: every text the setup window shows, in every language.
//! - [`archive`]: the compressed stream of files.
//! - [`codesign`]: who signed a macOS Setup app, as the builder and the
//!   installed app's update check both ask it.
//! - [`container`]: how the archive, the logo and the manifest are appended
//!   to the setup runtime's executable, and found again.
//! - [`paths`]: the rules every packaged path and data folder must follow.
//! - [`pe`]: whether a Windows program carries a signature.
//! - [`textfile`]: reading text files in whatever encoding an editor or
//!   shell wrote them.
//! - [`protect`]: access keys, and the encryption that keeps a setup file's
//!   contents from anyone without one.
//! - [`style`]: the built-in styles of the setup window, and the
//!   developer's changes to them.
//!
//! No UI and no Windows calls live here, so the builder runs on any OS and
//! the format is tested in isolation.

pub mod archive;
pub mod codesign;
pub mod config;
pub mod container;
pub mod delta;
pub mod feedsig;
pub mod i18n;
pub mod macapp;
pub mod manifest;
pub mod paths;
pub mod pe;
pub mod protect;
pub mod style;
pub mod textfile;
pub mod version;

/// The payload format this build writes and reads. A setup runtime refuses
/// a payload with another number instead of misreading it.
pub const FORMAT_VERSION: u32 = 1;

/// This crate's version, recorded in every manifest as the builder version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
