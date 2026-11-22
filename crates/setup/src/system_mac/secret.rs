//! An app's access key, kept for this user in the macOS Keychain so updates can
//! send it and open protected downloads without asking again. Driven through the
//! `security` tool every Mac has.

use std::process::Command;

use anyhow::{bail, Context, Result};

fn service(app_id: &str) -> String {
    format!("Bach access key: {app_id}")
}

/// Keeps the key (replacing an earlier one).
pub fn store(app_id: &str, key: &str) -> Result<()> {
    let output = Command::new("/usr/bin/security")
        .args(["add-generic-password", "-U", "-a", "bach", "-s"])
        .arg(service(app_id))
        .args(["-w", key])
        .output()
        .context("couldn't run the security tool")?;
    if !output.status.success() {
        bail!("the Keychain refused the key: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

/// The kept key, if there is one.
pub fn load(app_id: &str) -> Option<String> {
    let output =
        Command::new("/usr/bin/security").args(["find-generic-password", "-a", "bach", "-w", "-s"]).arg(service(app_id)).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_string()).filter(|key| !key.is_empty())
}

/// Forgets the key.
pub fn forget(app_id: &str) {
    let _ = Command::new("/usr/bin/security").args(["delete-generic-password", "-a", "bach", "-s"]).arg(service(app_id)).output();
}
