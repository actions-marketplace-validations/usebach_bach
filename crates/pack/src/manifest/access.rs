//! Access keys: who may install an app, and how its files are kept from
//! everyone else.

use serde::{Deserialize, Serialize};

/// An app that asks for an access key before it installs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Access {
    /// Where people get a key (https); the key page offers a button to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub get_key_url: Option<String>,
    /// Send the key with every request to the update feed and the download
    /// (`Authorization: Bearer <key>`), so a private server can refuse
    /// anyone without one. The installed uninstaller keeps this, so updates
    /// stay private after the install.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub send_key: bool,
    /// How the packaged files are locked. Only in a setup file; the
    /// installed uninstaller carries no files, so it drops this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption: Option<Encryption>,
}

/// The settings the files were encrypted with (see `protect.rs`). Salt,
/// nonce and check are lowercase hex.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Encryption {
    pub scheme: String,
    pub salt: String,
    pub nonce: String,
    /// Argon2id memory in KiB.
    pub memory_kib: u32,
    /// Argon2id passes.
    pub passes: u32,
    /// A hash that tells a right key from a wrong one.
    pub check: String,
}
