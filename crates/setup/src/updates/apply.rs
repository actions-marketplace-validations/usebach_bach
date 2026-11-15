//! Installing a staged update: wait for the app to close, run the setup file
//! without a window, and start the app again if asked.
//!
//! The setup file does the real work (all or nothing, see `work/place.rs`),
//! so this only has to run it, and to cope with a world where the app was
//! closed a moment ago and a virus scanner still holds one of its files: a
//! failed run is tried again a couple of times before it is given up on, and
//! the staged update is kept for the next round.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use bach_pack::container;

use super::stage::{self, Staged};
use super::Context;
use crate::system::host;

/// How long `--wait-for` lets the app take to close.
const WAIT_FOR_APP: Duration = Duration::from_secs(10 * 60);
/// Tries at running the setup file, and the pause between them.
const ATTEMPTS: usize = 3;
const PAUSE: Duration = Duration::from_secs(5);

/// Waits for the process that asked for this update (the app) to end.
pub fn wait_for_app(ctx: &Context) -> Result<(), String> {
    match ctx.args.wait_for {
        Some(pid) if !host::wait_for_exit(pid, WAIT_FOR_APP) => {
            Err(format!("{} is still running, so it can't be updated yet", ctx.manifest().app.name))
        }
        _ => Ok(()),
    }
}

/// Checks the staged file once more, then installs it silently. Ok(0) when
/// it is installed, Ok(1) when it couldn't be (it stays staged to try
/// again), Err for a staged file that must not run (it is deleted).
pub fn install(ctx: &Context, staged: &Staged) -> Result<i32, String> {
    let app_id = &ctx.manifest().app.id;
    if let Err(problem) = stage::verify(staged, app_id) {
        stage::clear(app_id);
        return Err(problem);
    }
    let Some(_lock) = host::SetupLock::acquire_updater(app_id) else {
        crate::log::line("Another update of this app is running; leaving this one for later.");
        return Ok(0);
    };
    let mut done = run_attempts(ctx, staged)?;
    // A patch that can't be applied (the installed files aren't the version it
    // was made for, say) is no reason to leave the app out of date: the full
    // setup file does the same job, only bigger.
    if !done && staged.patch {
        if let Some(full) = &staged.full {
            crate::log::line("The update patch couldn't be applied; downloading the full setup file instead.");
            stage::clear(app_id);
            match stage::ensure_any(std::slice::from_ref(full), app_id, ctx.bearer().as_ref(), &mut |_| {}) {
                Ok(full_staged) => done = run_attempts(ctx, &full_staged)?,
                Err(error) => crate::log::line(&format!("The full setup file couldn't be downloaded either: {error}")),
            }
        }
    }
    if done {
        stage::clear(app_id);
        // This process may be the uninstaller the update just replaced;
        // the copy it left behind goes once it has exited.
        let old = ctx.dir.join(format!("{}.bach-old", bach_pack::manifest::UNINSTALLER));
        if old.exists() {
            let _ = crate::work::delete_after_exit(&old, None);
        }
        if ctx.args.launch {
            launch_app(ctx);
        }
        return Ok(0);
    }
    eprintln!(
        "{} {} couldn't be installed yet. It stays downloaded, and the next update run tries again.",
        ctx.manifest().app.name,
        staged.version
    );
    Ok(1)
}

/// Runs the staged setup file until it installs: true when it did. A patch is
/// tried once, because a patch that doesn't fit won't fit the second time; a
/// full setup file gets a few tries, for a virus scanner that still holds one
/// of the app's files.
fn run_attempts(ctx: &Context, staged: &Staged) -> Result<bool, String> {
    let tries = if staged.patch { 1 } else { ATTEMPTS };
    let kind = if staged.patch { "patch for" } else { "" };
    for attempt in 1..=tries {
        crate::log::line(&format!("Installing {kind} {} {} (try {attempt})", ctx.manifest().app.name, staged.version));
        let code = run_setup(ctx, &staged.file)?;
        if code == 0 {
            return Ok(true);
        }
        crate::log::line(&format!("The setup file ended with code {code}."));
        if attempt < tries {
            std::thread::sleep(PAUSE);
        }
    }
    Ok(false)
}

/// Runs a setup file with `--silent` and gives back its exit code.
fn run_setup(ctx: &Context, setup: &Path) -> Result<i32, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new(setup);
    command.arg("--silent");
    if let Some(lang) = &ctx.args.lang {
        command.args(["--lang", lang]);
    }
    if let Some(key) = &ctx.key {
        // A protected setup file opens with the key kept at install time.
        // An environment variable, not an argument: arguments can be read
        // by other programs.
        command.env(crate::launch::KEY_ENV, key);
    }
    let status = command.creation_flags(CREATE_NO_WINDOW).status().map_err(|err| format!("couldn't start the update: {err}"))?;
    Ok(status.code().unwrap_or(1))
}

/// Starts the app that was just updated, detached from this process. It
/// reads the program's name from the new uninstaller, since a new version
/// may have renamed it.
fn launch_app(ctx: &Context) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    let dir = &ctx.dir;
    let installed = std::fs::File::open(dir.join(bach_pack::manifest::UNINSTALLER))
        .ok()
        .and_then(|mut file| {
            let layout = container::locate(&mut file).ok()??;
            container::read_manifest(&mut file, &layout).ok()
        })
        .unwrap_or_else(|| ctx.manifest().clone());
    let program = installed.executable_in(dir);
    let started = Command::new(&program).current_dir(dir).creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP).spawn();
    match started {
        Ok(_) => crate::log::line(&format!("Started {}", program.display())),
        Err(err) => crate::log::line(&format!("Couldn't start {}: {err}", program.display())),
    }
}
