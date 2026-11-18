//! Removing an install.
//!
//! The program goes first: if the app is still open that fails before
//! anything else has changed. Then every file the manifest lists, the
//! folders they leave empty, the shortcuts, start with Windows, PATH, link
//! schemes and file types, the Apps entry, the user's data only when asked,
//! and the install folder if nothing else is left in it. Files the app
//! created itself in the install folder stay, so a user's own files there
//! are never touched.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bach_pack::i18n::Key;
use bach_pack::manifest::{Choice, Manifest, UNINSTALLER};
use bach_pack::paths;

use super::{delete_after_exit, describe, failed, plain, remove_empty_parents, remove_file_if_present, remove_waiting, same_file, Report};
use crate::package::Package;
use crate::system::{self, changes, env_path, host, integration, task};
use crate::texts::Texts;

pub struct RemovePlan {
    pub package: Arc<Package>,
    pub texts: Arc<Texts>,
    pub dir: PathBuf,
    pub delete_data: bool,
}

pub fn remove(plan: &RemovePlan, report: Report) -> Result<(), String> {
    let manifest = &plan.package.manifest;
    let texts = &plan.texts;
    let dir = &plan.dir;
    check_install_dir(dir, manifest, texts)?;
    super::install::check_elevated_folder(manifest, dir, texts)?;
    let _lock = host::SetupLock::acquire(&manifest.app.id).ok_or_else(|| texts.get(Key::error_busy))?;
    let uninstaller = dir.join(UNINSTALLER);
    let current = std::env::current_exe().ok();
    let running_in_place = current.as_deref().is_some_and(|current| same_file(current, &uninstaller));

    // An update cut short earlier is undone first, so what's on disk is
    // what the manifest lists.
    if let Some(stuck) = super::place::recover(dir).stuck {
        return Err(super::recovery_failed(texts, &stuck));
    }
    // The app's own "before removal" steps run while its files are still there.
    super::steps::run_removal_steps(manifest, dir, texts, report, 0.05);
    report(0.1, &texts.get(Key::step_removing));
    let program = manifest.executable_in(dir);
    wait_until_closed(&program, texts)?;
    // All or nothing: every file steps aside first (a rename, which is quick
    // and can be undone), and only when all of them have are they deleted. A
    // file that can't move leaves the app as it was, still working.
    let mut wanted = Vec::new();
    for file in &manifest.files {
        let path = paths::under(dir, &file.path);
        if super::conflicts::changed_since_install(&path, file) {
            // Not the file setup wrote any more: it holds the user's changes.
            crate::log::line(&format!("Kept {}: it changed after it was installed.", file.path));
            continue;
        }
        wanted.push(path);
    }
    let aside = set_aside(&wanted).map_err(|(path, err)| failed(texts, Key::error_delete, &path, &err))?;
    // Everything below can fail, and then the app has to be as it was: its files come back, and
    // so do the shortcuts, start with Windows, PATH, link schemes and file types that were taken
    // away on the way (put back the way an update that failed puts them back).
    let put_back = |problem: String| {
        restore(&aside);
        let before = manifest.chosen.clone().unwrap_or_default();
        let _ = super::install::shortcuts(manifest, None, &before, dir, &program, texts);
        let _ = super::install::integrate(manifest, None, &before, dir, &program, texts);
        problem
    };

    // What can fail comes first, and what can't comes last: the registration entry is what Windows
    // uses to find this uninstaller, so it goes before the shortcuts, which are only convenience.
    undo_integration(manifest, dir, &program, texts).map_err(put_back)?;

    report(0.6, &texts.get(Key::step_unregistering));
    system::unregister(manifest).map_err(|err| put_back(format!("{} {}", texts.get(Key::error_unregister), plain(&err))))?;

    report(0.65, &texts.get(Key::step_removing_shortcuts));
    let extra = system::extra_shortcuts(manifest).into_iter().map(|(link, _)| link);
    for shortcut in [system::start_menu_shortcut(manifest), system::desktop_shortcut(manifest)].into_iter().flatten().chain(extra) {
        let _ = remove_file_if_present(&shortcut);
    }
    if manifest.install.start_menu_folder.is_some() {
        // Only an empty folder goes: another app may share it.
        if let Ok(folder) = system::start_menu_folder(manifest) {
            let _ = fs::remove_dir(folder);
        }
    }
    // From here on the app is gone from Windows' point of view: finish the job.
    for (_, moved) in &aside {
        let _ = remove_file_if_present(moved);
    }
    remove_empty_parents(dir, manifest.files.iter().map(|file| file.path.as_str()));
    // The access key kept for updates goes with the app.
    system::secret::forget(&manifest.app.id);
    system::channel::forget(&manifest.app.id);

    // The user's data goes only when asked. A folder that won't go (a program or a virus scanner
    // holds a file in it) is no reason to leave the rest of the removal undone: the app is
    // unregistered by now, so stopping here would leave its folder and uninstaller behind with
    // no entry in Settings to find them by. It is logged, and said at the end.
    let mut data_left = Vec::new();
    if plan.delete_data {
        report(0.75, &texts.get(Key::step_deleting_data));
        for template in &manifest.uninstall.data {
            if let Some(folder) = paths::expand_data_folder(template, |name| std::env::var(name).ok()) {
                crate::log::line(&format!("Deleting the data folder {}", folder.display()));
            }
        }
        data_left = delete_data(manifest, dir, &|name| std::env::var(name).ok());
        for (folder, why) in &data_left {
            crate::log::line(&format!("Couldn't delete {}: {why}", folder.display()));
        }
    }

    report(0.9, &texts.get(Key::step_cleaning));
    if !running_in_place {
        // The installed uninstaller handed over to this temp copy and is
        // exiting; give it a moment to let go of its file.
        remove_waiting(&uninstaller, texts, Key::error_busy)?;
    }
    // What an update's rename of the running uninstaller left behind.
    super::place::clear_leftovers(&uninstaller);
    // Only an empty folder goes: the user may have kept their own files there.
    let _ = fs::remove_dir(dir);
    system::notify_shell();

    // This process's own file can't go while it runs: a hidden helper waits
    // for it to exit and deletes it then.
    if let Some(current) = current {
        if running_in_place || is_temporary_copy(&current) {
            let folder = running_in_place.then_some(dir.as_path());
            let _ = delete_after_exit(&current, folder);
        }
    }
    match data_left.first() {
        None => report(1.0, &texts.get(Key::step_done)),
        Some((folder, why)) => {
            report(1.0, &format!("{} {why}", texts.with(Key::error_delete_data, &[("path", &folder.display().to_string())])))
        }
    }
    Ok(())
}

/// Takes back start with Windows, the PATH entry, link schemes and file
/// types; each only if it's this install's own.
fn undo_integration(manifest: &Manifest, dir: &Path, program: &Path, texts: &Texts) -> Result<(), String> {
    let fail = |err: anyhow::Error| format!("{} {}", texts.get(Key::error_system), plain(&err));
    integration::clear_autostart(manifest, program).map_err(fail)?;
    if manifest.updates.is_some() {
        // Best effort: a task that can't be deleted does nothing once the
        // app is gone (it removes itself on its next round).
        let _ = task::unregister(manifest);
        crate::updates::forget_staged(&manifest.app.id);
    }
    let added_to_path = match &manifest.chosen {
        Some(chosen) => chosen.add_to_path,
        None => manifest.install.add_to_path == Choice::Always,
    };
    if added_to_path {
        env_path::remove(manifest.install.scope, &manifest.path_folder_in(dir)).map_err(fail)?;
    }
    for protocol in &manifest.protocols {
        integration::unregister_protocol(manifest, protocol).map_err(fail)?;
    }
    for file_type in &manifest.file_types {
        integration::unregister_file_type(manifest, file_type).map_err(fail)?;
    }
    changes::undo(&changes::at(manifest), manifest, dir);
    Ok(())
}

/// Waits a moment for the app to close; an app that still runs from its
/// program file can't be removed, and Windows would let the file be renamed
/// anyway, so it is looked for explicitly.
fn wait_until_closed(program: &Path, texts: &Texts) -> Result<(), String> {
    for _ in 0..20 {
        if !super::is_running(program) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    Err(texts.get(Key::error_still_open))
}

/// Renames each file to `<name>.bach-old`. If one can't be renamed, those
/// already moved go back and the file at fault is reported.
fn set_aside(files: &[PathBuf]) -> Result<Vec<(PathBuf, PathBuf)>, (PathBuf, std::io::Error)> {
    let mut done: Vec<(PathBuf, PathBuf)> = Vec::with_capacity(files.len());
    for path in files {
        if fs::symlink_metadata(path).is_err() {
            continue; // already gone
        }
        let moved = super::place::with_suffix(path, ".bach-old");
        let _ = fs::remove_file(&moved);
        match fs::rename(path, &moved) {
            Ok(()) => done.push((path.clone(), moved)),
            Err(err) => {
                restore(&done);
                return Err((path.clone(), err));
            }
        }
    }
    Ok(done)
}

/// Undoes [`set_aside`], last file first.
fn restore(done: &[(PathBuf, PathBuf)]) {
    for (path, moved) in done.iter().rev() {
        let _ = fs::rename(moved, path);
    }
}

/// Refuses a folder that isn't plainly this app's install, so a wrong
/// `--install-dir` can never send the uninstaller anywhere else.
fn check_install_dir(dir: &Path, manifest: &Manifest, texts: &Texts) -> Result<(), String> {
    let registered = system::installed(manifest).is_some_and(|installed| same_file(&installed.dir, dir));
    let looks_installed = dir.join(UNINSTALLER).is_file() || manifest.executable_in(dir).is_file();
    if dir.is_absolute() && dir.is_dir() && (registered || looks_installed) {
        Ok(())
    } else {
        Err(texts.with(Key::error_not_found, &[("path", &dir.display().to_string())]))
    }
}

/// The folders the app keeps its data in (checked when the setup file was
/// built and again when it was read, so they're inside the user's profile),
/// deleted; the ones that couldn't be, with why. `lookup` reads the profile
/// folders (the environment in setup).
///
/// A setup file isn't trusted, so a folder that is, or holds, the install
/// folder is skipped here even though the manifest check let it through.
fn delete_data(manifest: &Manifest, install_dir: &Path, lookup: &dyn Fn(&str) -> Option<String>) -> Vec<(PathBuf, String)> {
    let mut left = Vec::new();
    for template in &manifest.uninstall.data {
        let Some(folder) = paths::expand_data_folder(template, lookup) else {
            continue;
        };
        if paths::is_same_or_inside(install_dir, &folder) {
            continue;
        }
        match fs::remove_dir_all(&folder) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => left.push((folder, describe(&err).to_string())),
        }
    }
    left
}

/// The uninstaller's own copy in the temp folder (see launch.rs).
fn is_temporary_copy(path: &Path) -> bool {
    let temp = std::env::temp_dir();
    path.parent().is_some_and(|parent| same_file(parent, &temp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;

    #[test]
    fn a_folder_that_will_not_go_is_reported_and_the_others_still_go() {
        let root = std::env::temp_dir().join(format!("bach-remove-data-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for folder in ["Locked", "Free"] {
            fs::create_dir_all(root.join(folder)).unwrap();
            fs::write(root.join(folder).join("data.bin"), b"x").unwrap();
        }
        // A program that holds a file open without sharing, like a virus scanner mid-scan.
        let holder = fs::OpenOptions::new().read(true).share_mode(0).open(root.join("Locked").join("data.bin")).unwrap();
        let mut manifest = crate::package::Package::sample().manifest;
        manifest.uninstall.data = vec![r"%APPDATA%\Locked".into(), r"%APPDATA%\Free".into(), r"%APPDATA%\Never".into()];
        let appdata = root.display().to_string();
        let lookup = |name: &str| (name == "APPDATA").then(|| appdata.clone());
        let left = delete_data(&manifest, Path::new(r"C:\Somewhere\Else"), &lookup);
        assert_eq!(left.len(), 1, "{left:?}");
        assert!(left[0].0.ends_with("Locked"));
        assert!(!root.join("Free").exists(), "the others went");
        assert!(root.join("Locked").exists());
        drop(holder);
        fs::remove_dir_all(&root).unwrap();
    }
}
