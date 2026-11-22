//! The background task that keeps an app up to date: a per-user launch agent,
//! `app.bach.update.<app id>`, that runs the uninstaller with `--auto-update` every
//! four hours (and as soon as possible after a time the Mac was asleep). It needs no
//! administrator rights, runs at low priority, and goes away when the user unticks
//! "Automatic updates" and when the app is removed.

use std::path::Path;

use anyhow::Result;
use bach_pack::manifest::Manifest;

use super::launchd::{self, Agent};

/// How often the task runs, in seconds.
const EVERY: u32 = 4 * 60 * 60;

fn label(manifest: &Manifest) -> String {
    let id: String = manifest.app.id.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') { c } else { '-' }).collect();
    format!("app.bach.update.{id}")
}

/// Creates (or replaces) the app's update task, running the `uninstaller` bundle's program.
pub fn register(manifest: &Manifest, uninstaller: &Path) -> Result<()> {
    // The task runs the program inside the uninstaller bundle.
    let program = uninstaller.join(bach_pack::macapp::RUNTIME);
    let label = label(manifest);
    launchd::unload(&label);
    let agent = Agent {
        label: &label,
        program: vec![program.to_string_lossy().into_owned(), "--auto-update".into()],
        run_at_load: false,
        interval: Some(EVERY),
        background: true,
        environment: Vec::new(),
        watch: Vec::new(),
    };
    let file = launchd::write(&agent)?;
    launchd::load(&file);
    Ok(())
}

/// Deletes the app's update task. Fine when there is none.
pub fn unregister(manifest: &Manifest) -> Result<()> {
    launchd::remove(&label(manifest))
}

/// Whether the app's update task is there.
#[allow(dead_code)]
pub fn exists(manifest: &Manifest) -> bool {
    launchd::path(&label(manifest)).is_ok_and(|path| path.is_file())
}
