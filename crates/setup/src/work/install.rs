//! Installing and updating.
//!
//! 1. Check: no other setup for this app is running, the app isn't open,
//!    and the drive has room.
//! 2. Unpack the chosen files next to their targets, then move them into
//!    place all at once (see `place.rs`), so a failure leaves the earlier
//!    version as it was.
//! 3. On an update, remove the files the old install had and this one
//!    doesn't. The old file list comes from the old uninstaller.
//! 4. Write `uninstall.exe`: this runtime, with the installed manifest.
//! 5. Shortcuts, start with Windows, PATH, link schemes and file types,
//!    each added or taken away to match the choices, and the entry under
//!    Settings › Apps.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bach_pack::archive;
use bach_pack::container;
use bach_pack::i18n::Key;
use bach_pack::manifest::{Choices, Manifest, UNINSTALLER};
use bach_pack::paths;
use bach_pack::protect::{KeyError, Unlocked};

use super::patch::patch_base;
use super::place::{self, Transaction};
use super::{describe, failed, is_disk_full, is_in_use, is_in_use_at, plain, remove_empty_parents, remove_file_if_present, Report};
use crate::package::Package;
use crate::system::{self, changes, env_path, host, integration, task};
use crate::texts::{self, Texts};

/// Unpacking is most of the work; the steps after it share the rest.
const UNPACK_SHARE: f32 = 0.8;
/// Room left over after the files, for the uninstaller and the file system.
const SPARE_BYTES: u64 = 32 << 20;

pub struct InstallPlan {
    pub package: Arc<Package>,
    pub texts: Arc<Texts>,
    pub dir: PathBuf,
    pub choices: Choices,
    /// The access key, for a setup file that asks for one.
    pub key: Option<String>,
}

/// Installs or updates. An online installer first downloads the newest
/// setup file and installs from that; everything after is the same.
pub fn install(plan: &InstallPlan, report: Report) -> Result<(), String> {
    super::cancel::reset();
    if !plan.package.manifest.online {
        return install_files(plan, report);
    }
    let share = super::online::DOWNLOAD_SHARE;
    let downloaded = super::online::fetch(plan, &|progress, label| report(progress.min(1.0) * share, label))?;
    let from_download = super::online::plan_for(plan, downloaded);
    let result = install_files(&from_download, &|progress, label| report(share + progress * (1.0 - share), label));
    if result.is_ok() {
        super::online::forget_download(plan);
    }
    result
}

fn install_files(plan: &InstallPlan, report: Report) -> Result<(), String> {
    let package = &plan.package;
    let texts = &plan.texts;
    let manifest = &package.manifest;
    let choices = plan.choices.clone().settle(manifest);
    let dir = &plan.dir;
    let Some(layout) = package.layout.filter(|layout| layout.has_archive()) else {
        return Err(texts.get(Key::error_damaged));
    };
    // A wrong key is found before anything on this computer is touched.
    let unlocked = unlock(plan)?;
    let _lock = host::SetupLock::acquire(&manifest.app.id).ok_or_else(|| texts.get(Key::error_busy))?;

    report(0.02, &texts.get(Key::step_preparing));
    // A machine-wide app runs, updates and uninstalls with administrator rights,
    // so it may only live where the user's own programs can't write: Program Files.
    if manifest.install.scope.is_machine() && !crate::system::folders::is_admin_only(dir) {
        crate::log::line(&format!("{} is outside Program Files, so a machine-wide install refuses it.", dir.display()));
        return Err(texts.with(Key::error_no_access, &[("path", &dir.display().to_string())]));
    }
    create_install_dir(dir, texts)?;
    check_elevated_folder(manifest, dir, texts)?;
    // An update that was cut short earlier is undone first; if a file won't
    // let go, nothing is written over it.
    if let Some(stuck) = place::recover(dir).stuck {
        return Err(super::recovery_failed(texts, &stuck));
    }
    let program = manifest.executable_in(dir);
    ensure_closed(&program, texts)?;
    let previous = previous_install(dir, manifest);
    let needed = manifest.selected_size(&choices);
    log_long_paths(manifest, &choices, dir);
    // An update patch carries only what changed; the rest is in the folder.
    let matching = patch_base(manifest, &choices, dir);
    if previous.is_some() {
        // What an update or a repair is about to write again: missing, or not what the manifest records.
        let rewritten: Vec<&str> = manifest
            .files
            .iter()
            .filter(|file| choices.installs(file) && !matching.contains(&file.path))
            .map(|file| file.path.as_str())
            .collect();
        let shown: Vec<&str> = rewritten.iter().take(20).copied().collect();
        crate::log::line(&format!(
            "{} file(s) are written: {}{}",
            rewritten.len(),
            shown.join(", "),
            if rewritten.len() > 20 { ", ..." } else { "" }
        ));
    }
    let unchanged: u64 = manifest.files.iter().filter(|file| matching.contains(&file.path)).map(|file| file.size).sum();
    let to_copy = needed.saturating_sub(unchanged);
    check_free_space(dir, to_copy, texts)?;

    // An older install of this app by another installer goes first, so its
    // uninstaller can't take away what this one puts in place.
    if previous.is_none() && !manifest.system.migrate.is_empty() {
        report(0.03, &texts.get(Key::step_migrating));
        super::migrate::run(manifest);
    }

    // The files are in place but still undoable: everything below can fail,
    // and then the earlier version comes back whole.
    // The installed uninstaller is kept before anything changes: an update that can't keep
    // it must not start, since putting the earlier version back would then delete it.
    let uninstaller_before = keep_uninstaller(dir).map_err(|(path, err)| failed(texts, Key::error_write, &path, &err))?;
    let committed = match unpack(plan, &choices, layout.archive, to_copy, &matching, unlocked.as_ref(), report) {
        Ok(committed) => committed,
        Err(problem) => {
            if let Some(kept) = &uninstaller_before {
                let _ = fs::remove_file(kept);
            }
            return Err(problem);
        }
    };
    let installed = manifest.as_installed(&choices);
    let later = (|| -> Result<(), String> {
        super::steps::run_install_steps(manifest, dir, previous.is_none(), &choices, texts, (0.895, 0.915), report)?;

        report(0.92, &texts.get(Key::step_shortcuts));
        shortcuts(manifest, previous.as_ref(), &choices, dir, &program, texts)?;
        integrate(manifest, previous.as_ref(), &choices, dir, &program, texts)?;

        report(0.95, &texts.get(Key::step_uninstaller));
        write_uninstaller(package, &installed, dir, texts)?;

        report(0.97, &texts.get(Key::step_registering));
        system::register(manifest, dir, needed).map_err(|err| format!("{} {}", texts.get(Key::error_register), plain(&err)))
    })();
    if let Err(problem) = later {
        crate::log::line("A step after the files were placed failed, so the earlier version is being put back.");
        let rolled_back = committed.rollback();
        if let Some(stuck) = &rolled_back.stuck {
            crate::log::line(&format!(
                "{} is held and couldn't be put back; the next run of setup finishes the undo.",
                stuck.path.display()
            ));
        }
        remove_empty_parents(
            dir,
            rolled_back.staged.iter().filter_map(|path| path.strip_prefix(dir).ok()).filter_map(|path| path.to_str()),
        );
        restore_uninstaller(dir, uninstaller_before);
        if let Some(previous) = &previous {
            // What the failed update changed around the files goes back too.
            let before = previous.chosen.clone().unwrap_or_default();
            let old_program = previous.executable_in(dir);
            let _ = shortcuts(previous, Some(&installed), &before, dir, &old_program, texts);
            let _ = integrate(previous, Some(&installed), &before, dir, &old_program, texts);
        }
        return Err(problem);
    }
    report(0.86, &texts.get(Key::step_tidying));
    committed.finish();
    if let Some(uninstaller) = uninstaller_before {
        let _ = fs::remove_file(uninstaller);
    }
    if let Some(previous) = &previous {
        remove_stale(dir, previous, &installed);
    }
    keep_key(plan);
    system::notify_shell();
    report(1.0, &texts.get(Key::step_done));
    Ok(())
}

/// Notes in the log when a file will sit at a path longer than classic Windows
/// handles (260 characters): setup writes it fine, but shortcuts, Explorer and
/// many programs may not cope, which is a first thing to know when a support
/// log is read.
fn log_long_paths(manifest: &Manifest, choices: &Choices, dir: &Path) {
    const CLASSIC_LIMIT: usize = 259;
    let longest = manifest
        .files
        .iter()
        .filter(|file| choices.installs(file))
        .map(|file| (paths::under(dir, &file.path).as_os_str().encode_wide().count(), file.path.as_str()))
        .max_by_key(|(length, _)| *length);
    if let Some((length, path)) = longest.filter(|(length, _)| *length > CLASSIC_LIMIT) {
        crate::log::line(&format!(
            "{path} will be {length} characters long, more than the {CLASSIC_LIMIT} classic Windows handles; \
             shortcuts and some programs may not open it unless long path support is on."
        ));
    }
}

/// The installed uninstaller, copied aside while an update runs, so a failed
/// update can put it back. `Ok(None)` is a first install, where there is none;
/// a copy that can't be made is an error (with the path it failed on), never
/// `None`, because [`restore_uninstaller`] deletes the uninstaller when nothing was kept.
fn keep_uninstaller(dir: &Path) -> Result<Option<PathBuf>, (PathBuf, std::io::Error)> {
    let current = dir.join(UNINSTALLER);
    if !current.is_file() {
        return Ok(None);
    }
    let kept = place::with_suffix(&current, ".bach-prev");
    let _ = fs::remove_file(&kept);
    fs::copy(&current, &kept).map_err(|err| (kept.clone(), err))?;
    Ok(Some(kept))
}

/// Puts the uninstaller kept by [`keep_uninstaller`] back; on a first install
/// (nothing kept) the new one is removed.
fn restore_uninstaller(dir: &Path, kept: Option<PathBuf>) {
    let current = dir.join(UNINSTALLER);
    match kept {
        Some(kept) => {
            let _ = place::replace_file(&kept, &current);
        }
        None => {
            let _ = fs::remove_file(&current);
        }
    }
}

/// Opens the files with the access key, when they are locked. A missing or
/// wrong key is the same sentence: nothing is unpacked either way.
fn unlock(plan: &InstallPlan) -> Result<Option<Unlocked>, String> {
    let Some(encryption) = plan.package.manifest.access.as_ref().and_then(|access| access.encryption.as_ref()) else {
        return Ok(None);
    };
    let texts = &plan.texts;
    let Some(key) = plan.key.as_deref() else {
        return Err(texts.get(Key::access_wrong));
    };
    match encryption.unlock(key) {
        Ok(unlocked) => Ok(Some(unlocked)),
        Err(KeyError::Wrong) => {
            crate::log::line("The access key didn't fit.");
            Err(texts.get(Key::access_wrong))
        }
        Err(KeyError::Damaged(why)) => {
            crate::log::line(&format!("The protection data is unusable: {why}"));
            Err(texts.get(Key::error_damaged))
        }
    }
}

/// Keeps the access key for this user, so the updater can send it and open
/// protected updates; takes it back when this version doesn't ask for one.
/// A key that can't be kept never fails the install: the next update just
/// asks for it again.
fn keep_key(plan: &InstallPlan) {
    let manifest = &plan.package.manifest;
    match (&manifest.access, plan.key.as_deref()) {
        (Some(_), Some(key)) if manifest.updates.is_some() => {
            if let Err(err) = system::secret::store(&manifest.app.id, key) {
                crate::log::line(&format!("Couldn't keep the access key for updates: {err:#}"));
            }
        }
        (None, _) => system::secret::forget(&manifest.app.id),
        _ => {}
    }
}

fn create_install_dir(dir: &Path, texts: &Texts) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|err| {
        if err.raw_os_error() == Some(5) {
            texts.with(Key::error_no_access, &[("path", &dir.display().to_string())])
        } else {
            failed(texts, Key::error_create, dir, &err)
        }
    })
}

/// A machine-wide app runs elevated from its folder, so the folder must not
/// be one that ordinary users can change (an old installer or a script may
/// have left such a folder in Program Files). Checked before anything is
/// written, and again before an elevated update or removal.
pub(super) fn check_elevated_folder(manifest: &Manifest, dir: &Path, texts: &Texts) -> Result<(), String> {
    if !manifest.install.scope.is_machine() {
        return Ok(());
    }
    let unsafe_folder = |why: &str| {
        crate::log::line(&format!("{} isn't safe to run elevated: {why}.", dir.display()));
        texts.with(Key::error_unsafe_folder, &[("path", &dir.display().to_string())])
    };
    match crate::system::acl::tree_weakness(dir) {
        Ok(None) => Ok(()),
        Ok(Some(why)) => Err(unsafe_folder(&why)),
        Err(why) => Err(unsafe_folder(&why)),
    }
}

/// Fails with a readable sentence when the app is running from `program`.
fn ensure_closed(program: &Path, texts: &Texts) -> Result<(), String> {
    if is_running(program) {
        Err(texts.get(Key::error_still_open))
    } else {
        Ok(())
    }
}

/// Whether the app is running from `program`: a program in use can't be
/// opened for writing.
pub fn is_running(program: &Path) -> bool {
    program.exists() && matches!(fs::OpenOptions::new().write(true).open(program), Err(err) if is_in_use(&err))
}

/// Undoes an update that was cut short in `dir` (see `place.rs`); how many
/// files it put back or took away. Callers hold the setup lock.
pub fn recover_interrupted(dir: &Path) -> usize {
    let recovery = place::recover(dir);
    if let Some(stuck) = &recovery.stuck {
        crate::log::line(&format!("Couldn't undo an update that was cut short: {} ({}).", stuck.path.display(), stuck.error));
    }
    recovery.changed
}

fn check_free_space(dir: &Path, needed: u64, texts: &Texts) -> Result<(), String> {
    match host::free_space(dir) {
        Some(free) if free < needed.saturating_add(SPARE_BYTES) => {
            Err(texts
                .with(Key::error_disk_full, &[("size", &texts::size(needed.saturating_add(SPARE_BYTES))), ("free", &texts::size(free))]))
        }
        _ => Ok(()),
    }
}

/// Unpacks the chosen files and moves them into place, all or nothing.
/// `needed` is what gets copied; `matching` are the files an update patch
/// leaves out because they are in the folder already.
fn unpack(
    plan: &InstallPlan,
    choices: &Choices,
    archive_section: container::Section,
    needed: u64,
    matching: &HashSet<String>,
    unlocked: Option<&Unlocked>,
    report: Report,
) -> Result<place::Committed, String> {
    let texts = &plan.texts;
    let manifest = &plan.package.manifest;
    let dir = &plan.dir;
    let read_failed = |err: std::io::Error| format!("{} {}", texts.get(Key::error_read_setup), describe(&err));
    let wanted: HashSet<&str> = manifest.files.iter().filter(|file| choices.installs(file)).map(|file| file.path.as_str()).collect();
    // What the manifest says each file is, to check against what the archive hands out.
    let recorded: std::collections::HashMap<&str, &bach_pack::manifest::FileEntry> =
        manifest.files.iter().filter(|file| choices.installs(file)).map(|file| (file.path.as_str(), file)).collect();
    for file in &manifest.files {
        place::clear_leftovers(&paths::under(dir, &file.path));
    }
    place::clear_leftovers(&dir.join(UNINSTALLER));

    let copying = texts.get(Key::step_copying);
    let total = needed.max(1);
    let mut done = 0u64;
    let mut source = File::open(&plan.package.exe).map_err(read_failed)?;
    source.seek(SeekFrom::Start(archive_section.offset)).map_err(read_failed)?;
    // The archive is checked against the builder's SHA-256 as it is read (and
    // before the commit below), not in a pass of its own beforehand.
    let (section, verdict) = super::section::hashing(source.take(archive_section.len));
    let reader = BufReader::new(section);
    let mut transaction = Transaction::default();
    let mut write_error: Option<String> = None;
    let mut present: HashSet<String> = HashSet::new();
    let mut visit = |packaged: &str, size: u64, body: &mut dyn Read| {
        if super::cancel::requested() {
            write_error = Some(super::cancel::CANCELLED.to_string());
            return Err(anyhow::anyhow!("cancelled"));
        }
        if !wanted.contains(packaged) {
            return Ok(());
        }
        present.insert(packaged.to_string());
        if matching.contains(packaged) {
            // Already here exactly as packaged: nothing to write.
            return Ok(());
        }
        let target = paths::under(dir, packaged);
        // The archive has to agree with the manifest about every file: its size before it
        // is unpacked, its size and SHA-256 as it is. A mismatch is a damaged setup file.
        let recorded = recorded.get(packaged).copied();
        if recorded.is_some_and(|file| file.size != size) {
            crate::log::line(&format!("{packaged} is {size} bytes in the archive, but the manifest says otherwise."));
            write_error = Some(texts.get(Key::error_damaged));
            return Err(anyhow::anyhow!("{packaged} doesn't have the size the manifest records"));
        }
        let mut verifying = archive::Verifying::new(super::cancel::Cancellable(body));
        transaction
            .stage(&target, &mut verifying, &mut |chunk| {
                done = done.saturating_add(chunk);
                report(0.04 + UNPACK_SHARE * (done as f32 / total as f32).min(1.0), &copying);
            })
            .map_err(|err| {
                write_error = Some(if super::cancel::requested() {
                    super::cancel::CANCELLED.to_string()
                } else if is_disk_full(&err) {
                    texts.with(Key::error_disk_full, &[("size", &texts::size(needed)), ("free", "0 KB")])
                } else {
                    failed(texts, Key::error_write, &target, &err)
                });
                anyhow::anyhow!("{err}")
            })?;
        let (written, sha256) = verifying.digest();
        let wrong_hash = recorded.and_then(|file| file.sha256.as_deref()).is_some_and(|expected| !sha256.eq_ignore_ascii_case(expected));
        if recorded.is_some_and(|file| file.size != written) || wrong_hash {
            crate::log::line(&format!("{packaged} doesn't match the size or SHA-256 the manifest records."));
            write_error = Some(texts.get(Key::error_damaged));
            return Err(anyhow::anyhow!("{packaged} doesn't match the manifest"));
        }
        Ok(())
    };
    let encryption = manifest.access.as_ref().and_then(|access| access.encryption.as_ref());
    let unpacked = match (encryption, unlocked) {
        (Some(encryption), Some(unlocked)) => {
            encryption.reader(reader, unlocked).and_then(|decrypted| archive::read(decrypted, &mut visit))
        }
        _ => archive::read(reader, &mut visit),
    };
    if let Err(err) = unpacked {
        transaction.discard();
        remove_empty_parents(dir, wanted.iter().copied());
        return Err(write_error.unwrap_or_else(|| format!("{} {}", texts.get(Key::error_damaged), plain(&err))));
    }
    if let Some(expected) = &manifest.archive_sha256 {
        let intact = match verdict.matches(expected) {
            Ok(intact) => intact,
            Err(err) => {
                transaction.discard();
                remove_empty_parents(dir, wanted.iter().copied());
                return Err(read_failed(err));
            }
        };
        if !intact {
            crate::log::line("The archive doesn't match the SHA-256 recorded when the setup file was built.");
            transaction.discard();
            remove_empty_parents(dir, wanted.iter().copied());
            return Err(texts.get(Key::error_damaged));
        }
    }
    // A cancel that came as the last file was written: nothing is moved yet.
    if super::cancel::requested() {
        transaction.discard();
        remove_empty_parents(dir, wanted.iter().copied());
        return Err(super::cancel::CANCELLED.to_string());
    }
    // Every chosen file has to be in the archive, or already here exactly as packaged (what
    // an update patch relies on). A setup file that lacks one would otherwise install without it.
    {
        let missing = wanted.iter().find(|path| !present.contains(**path) && !matching.contains(**path));
        if missing.is_some() && manifest.patch_from.is_none() {
            crate::log::line(&format!("{} is in the manifest but not in the archive.", missing.copied().unwrap_or("?")));
            transaction.discard();
            remove_empty_parents(dir, wanted.iter().copied());
            return Err(texts.get(Key::error_damaged));
        }
        // A patch is only good on the version it was made for.
        if let Some(missing) = missing {
            crate::log::line(&format!(
                "The update patch needs {missing} as it was in version {}, and it isn't.",
                manifest.patch_from.as_deref().unwrap_or("?")
            ));
            transaction.discard();
            remove_empty_parents(dir, wanted.iter().copied());
            return Err(texts.get(Key::error_patch_base));
        }
    }
    transaction.commit_open(&place::journal_path(dir)).map_err(|failure| {
        if is_in_use_at(&failure.error, &failure.target) {
            texts.with(Key::error_in_use, &[("path", &failure.target.display().to_string())])
        } else {
            failed(texts, Key::error_write, &failure.target, &failure.error)
        }
    })
}

/// The install already in `dir`, read from its uninstaller: only when it's
/// this same app (another app's uninstaller in a shared folder is none of
/// our business), and None when it can't be read, in which case nothing is
/// cleaned up rather than guessed.
pub fn previous_install(dir: &Path, manifest: &Manifest) -> Option<Manifest> {
    let mut file = File::open(dir.join(UNINSTALLER)).ok()?;
    let layout = container::locate(&mut file).ok()??;
    let previous = container::read_manifest(&mut file, &layout).ok()?;
    (previous.app.id == manifest.app.id).then_some(previous)
}

/// Removes what the old install put there and the new one doesn't. A file
/// the user changed since it was installed stays, exactly as the uninstaller
/// treats it. One that won't go (a program holds it) stays too and is written
/// to the log: the update itself is already complete and can't be undone here.
fn remove_stale(dir: &Path, previous: &Manifest, installed: &Manifest) {
    let keep: HashSet<String> = installed.files.iter().map(|file| paths::fold_case(&file.path)).collect();
    let mut gone: Vec<&str> = Vec::new();
    for file in previous.files.iter().filter(|file| !keep.contains(&paths::fold_case(&file.path))) {
        let path = paths::under(dir, &file.path);
        if super::conflicts::changed_since_install(&path, file) {
            crate::log::line(&format!("Kept {}: the new version doesn't ship it, but it changed after it was installed.", file.path));
        } else if let Err(err) = remove_file_if_present(&path) {
            crate::log::line(&format!("Couldn't remove {}, which the new version doesn't ship: {err}", file.path));
        } else {
            gone.push(file.path.as_str());
        }
    }
    remove_empty_parents(dir, gone.into_iter());
}

/// `uninstall.exe`: the runtime's own bytes with the installed manifest and
/// the logo, and no files.
fn write_uninstaller(package: &Package, installed: &Manifest, dir: &Path, texts: &Texts) -> Result<(), String> {
    let fail = |detail: String| format!("{} {}", texts.get(Key::error_uninstaller), super::sentence(detail));
    let layout = package.layout.ok_or_else(|| fail("this setup file has no payload".into()))?;
    let mut source = File::open(&package.exe).map_err(|err| fail(describe(&err)))?;
    let runtime_len = usize::try_from(layout.runtime_len).map_err(|_| texts.get(Key::error_damaged))?;
    let mut runtime = vec![0u8; runtime_len];
    source.read_exact(&mut runtime).map_err(|err| fail(describe(&err)))?;
    let logo = container::read_logo(&mut source, &layout).map_err(|err| fail(plain(&err)))?;

    let target = dir.join(UNINSTALLER);
    let partial = place::with_suffix(&target, ".bach-new");
    let written = File::create(&partial)
        .map_err(anyhow::Error::from)
        .and_then(|mut out| container::write_uninstaller(&mut out, &runtime, &logo, installed).map(|_| out))
        .and_then(|out| out.sync_all().map_err(anyhow::Error::from));
    if let Err(err) = written {
        let _ = fs::remove_file(&partial);
        return Err(fail(plain(&err)));
    }
    // The uninstaller may be what is running this update, which Windows only
    // lets be renamed, not overwritten.
    place::replace_file(&partial, &target).map_err(|err| {
        let _ = fs::remove_file(&partial);
        if is_in_use_at(&err, &target) {
            texts.with(Key::error_in_use, &[("path", &target.display().to_string())])
        } else {
            fail(describe(&err))
        }
    })
}

/// Adds or removes the Start menu and desktop shortcuts to match the
/// choices (an update may have unticked one), and the extra Start menu
/// shortcuts, which follow the Start menu choice.
pub(super) fn shortcuts(
    manifest: &Manifest,
    previous: Option<&Manifest>,
    choices: &Choices,
    dir: &Path,
    program: &Path,
    texts: &Texts,
) -> Result<(), String> {
    let description = manifest.app.description.clone().unwrap_or_else(|| manifest.app.name.clone());
    let failed = |link: &Path, err: anyhow::Error| {
        format!("{} {}", texts.with(Key::error_shortcut, &[("path", &link.display().to_string())]), plain(&err))
    };
    // Shortcuts the last version made and this one doesn't (renamed, or a
    // folder that changed) go first.
    if let Some(previous) = previous {
        let keep: Vec<PathBuf> = system::extra_shortcuts(manifest).into_iter().map(|(link, _)| link).collect();
        let mut old: Vec<PathBuf> = system::extra_shortcuts(previous).into_iter().map(|(link, _)| link).collect();
        old.extend(system::start_menu_shortcut(previous).ok());
        for link in old.iter().filter(|link| !keep.contains(link)) {
            if system::start_menu_shortcut(manifest).ok().as_ref() != Some(link) {
                let _ = remove_file_if_present(link);
            }
        }
        if previous.install.start_menu_folder != manifest.install.start_menu_folder {
            if let Ok(folder) = system::start_menu_folder(previous) {
                let _ = fs::remove_dir(folder);
            }
        }
    }
    let app_id = system::app_user_model_id(manifest);
    let wanted = [
        (system::start_menu_shortcut(manifest), choices.start_menu_shortcut),
        (system::desktop_shortcut(manifest), choices.desktop_shortcut),
    ];
    for (link, wanted) in wanted {
        let Ok(link) = link else {
            continue;
        };
        if wanted {
            system::create_shortcut(&link, program, &description, &app_id).map_err(|err| failed(&link, err))?;
        } else {
            let _ = remove_file_if_present(&link);
        }
    }
    for (link, shortcut) in system::extra_shortcuts(manifest) {
        let target = paths::under(dir, &shortcut.target);
        let installed = manifest.files.iter().any(|file| file.path == shortcut.target && choices.installs(file));
        if choices.start_menu_shortcut && installed {
            let description = shortcut.description.clone().unwrap_or_else(|| shortcut.name.clone());
            system::create_shortcut_with(&link, &target, shortcut.arguments.as_deref(), &description, &app_id)
                .map_err(|err| failed(&link, err))?;
        } else {
            let _ = remove_file_if_present(&link);
        }
    }
    if !choices.start_menu_shortcut && manifest.install.start_menu_folder.is_some() {
        if let Ok(folder) = system::start_menu_folder(manifest) {
            let _ = fs::remove_dir(folder);
        }
    }
    Ok(())
}

/// Start with Windows, PATH, link schemes and file types: added when
/// chosen, and taken back when an update no longer wants them.
pub(super) fn integrate(
    manifest: &Manifest,
    previous: Option<&Manifest>,
    choices: &Choices,
    dir: &Path,
    program: &Path,
    texts: &Texts,
) -> Result<(), String> {
    let fail = |err: anyhow::Error| format!("{} {}", texts.get(Key::error_system), plain(&err));
    if choices.start_with_windows {
        integration::set_autostart(manifest, program).map_err(fail)?;
    } else {
        integration::clear_autostart(manifest, program).map_err(fail)?;
    }

    // Automatic updates are a per-user scheduled task. A PC that won't allow
    // one just doesn't update by itself; that never fails the install.
    if manifest.updates.is_some() {
        let result = if choices.auto_update { task::register(manifest, &dir.join(UNINSTALLER)) } else { task::unregister(manifest) };
        if let Err(err) = result {
            crate::log::line(&format!("Automatic updates: {err:#}"));
        }
    }

    // PATH is only touched when this app put something there or will now.
    let path_folder = manifest.path_folder_in(dir);
    if let Some(previous) = previous.filter(|previous| previous.chosen.as_ref().is_some_and(|before| before.add_to_path)) {
        let old_folder = previous.path_folder_in(dir);
        if old_folder != path_folder || !choices.add_to_path {
            env_path::remove(previous.install.scope, &old_folder).map_err(fail)?;
        }
    }
    if choices.add_to_path {
        env_path::add(manifest.install.scope, &path_folder).map_err(fail)?;
    }

    if let Some(previous) = previous {
        for old in previous.protocols.iter().filter(|old| !manifest.protocols.contains(old)) {
            integration::unregister_protocol(manifest, old).map_err(fail)?;
        }
        for old in previous.file_types.iter().filter(|old| !manifest.file_types.iter().any(|new| new.extension == old.extension)) {
            integration::unregister_file_type(manifest, old).map_err(fail)?;
        }
    }
    for protocol in &manifest.protocols {
        integration::register_protocol(manifest, protocol, program).map_err(fail)?;
    }
    for file_type in &manifest.file_types {
        integration::register_file_type(manifest, file_type, program).map_err(fail)?;
    }
    changes::apply(&changes::at(manifest), manifest, previous, dir, program).map_err(fail)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bach-install-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_uninstaller_is_kept_or_the_update_does_not_start() {
        let dir = temp("keep");
        // A first install has none to keep.
        assert!(matches!(keep_uninstaller(&dir), Ok(None)));
        fs::write(dir.join(UNINSTALLER), b"old").unwrap();
        let kept = keep_uninstaller(&dir).unwrap().expect("a copy");
        assert_eq!(fs::read(&kept).unwrap(), b"old");
        // A copy that can't be made is an error, not "nothing to keep": a folder stands where it goes.
        fs::remove_file(&kept).unwrap();
        fs::create_dir(&kept).unwrap();
        assert!(keep_uninstaller(&dir).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_update_removes_dropped_files_but_keeps_the_ones_the_user_changed() {
        use bach_pack::manifest::FileEntry;
        let dir = temp("stale");
        let entry = |path: &str, content: &[u8]| {
            let mut entry = FileEntry::new(path, content.len() as u64);
            entry.sha256 = Some(bach_pack::protect::sha256_hex(content).unwrap());
            entry
        };
        let base = crate::package::Package::sample().manifest;
        let previous =
            Manifest { files: vec![entry("kept.dll", b"same"), entry("old.dll", b"old"), entry("edited.cfg", b"default")], ..base.clone() };
        let installed = Manifest { files: vec![entry("kept.dll", b"same")], ..base };
        for (name, body) in [("kept.dll", "same"), ("old.dll", "old"), ("edited.cfg", "the user's settings")] {
            fs::write(dir.join(name), body).unwrap();
        }
        remove_stale(&dir, &previous, &installed);
        assert!(dir.join("kept.dll").exists(), "still shipped");
        assert!(!dir.join("old.dll").exists(), "dropped and untouched: gone");
        assert_eq!(fs::read_to_string(dir.join("edited.cfg")).unwrap(), "the user's settings", "changed: kept");
        fs::remove_dir_all(&dir).unwrap();
    }
}
