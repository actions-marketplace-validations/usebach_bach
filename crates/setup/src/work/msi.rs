//! Installing, repairing and removing for the front of a `.msi` package: the
//! same jobs as a setup file's, done by Windows Installer (see `system/msi.rs`).
//! The window and the choices are Bach's; the files, shortcuts, registry
//! values, upgrade and removal are the package's own.

use std::path::PathBuf;
use std::time::Duration;

use bach_pack::i18n::Key;

use super::install::InstallPlan;
use super::remove::RemovePlan;
use super::Report;
use crate::system::{host, msi};

/// Windows Installer's own log, next to the setup log, for whoever has to find out why it stopped.
fn engine_log(app_id: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{}-msi.log", bach_pack::paths::safe_dir_name(app_id)))
}

/// Installs or updates the package; `repair` puts the installed version's files back instead.
pub fn install(plan: &InstallPlan, repair: bool, report: Report) -> Result<(), String> {
    super::cancel::reset();
    let Some(link) = msi::link() else {
        return Err(plan.texts.get(Key::error_damaged));
    };
    let manifest = &plan.package.manifest;
    let job = if repair { msi::Job::Repair } else { msi::Job::Install(msi::install_properties(manifest, &plan.choices, &plan.dir)) };
    crate::log::line(&match &job {
        msi::Job::Install(properties) => format!("Windows Installer installs {} with {properties}", link.package.display()),
        _ => format!("Windows Installer repairs {}", link.package.display()),
    });
    drive(&plan.texts, &manifest.app.id, &manifest.app.name, Key::step_copying, report, |shared, log| {
        msi::run(link, &job, Some(log), shared, super::cancel::requested)
    })
}

/// Removes the installed product.
pub fn remove(plan: &RemovePlan, report: Report) -> Result<(), String> {
    super::cancel::reset();
    let Some(link) = msi::link() else {
        return Err(plan.texts.get(Key::error_damaged));
    };
    let manifest = &plan.package.manifest;
    let product = msi::installed_product(link).unwrap_or_else(|| link.product.clone());
    crate::log::line(&format!("Windows Installer removes {product}"));
    let job = msi::Job::Remove(product);
    drive(&plan.texts, &manifest.app.id, &manifest.app.name, Key::step_removing, report, |shared, log| {
        msi::run(link, &job, Some(log), shared, super::cancel::requested)
    })
}

/// Runs `job` on a thread of its own, because it blocks, and tells the window what it says
/// about its progress every moment, until it ends.
fn drive(
    texts: &crate::texts::Texts,
    app_id: &str,
    app_name: &str,
    doing: Key,
    report: Report,
    job: impl FnOnce(&msi::Shared, &std::path::Path) -> msi::Outcome + Send,
) -> Result<(), String> {
    let _lock = host::SetupLock::acquire(app_id).ok_or_else(|| texts.get(Key::error_busy))?;
    let log = engine_log(app_id);
    let shared = msi::Shared::default();
    report(0.02, &texts.get(Key::step_preparing));
    let outcome = std::thread::scope(|scope| {
        let worker = scope.spawn(|| job(&shared, &log));
        while !worker.is_finished() {
            match shared.fraction() {
                Some(fraction) => report(0.04 + 0.94 * fraction, &texts.get(doing)),
                None => report(0.02, &texts.get(Key::step_preparing)),
            }
            std::thread::sleep(Duration::from_millis(80));
        }
        worker.join().unwrap_or(msi::Outcome::Failed(1))
    });
    crate::log::line(&format!("Windows Installer finished: {outcome:?} (its log: {})", log.display()));
    match outcome {
        msi::Outcome::Done => {
            report(1.0, &texts.get(Key::step_done));
            Ok(())
        }
        msi::Outcome::Cancelled => Err(super::cancel::CANCELLED.to_string()),
        msi::Outcome::Busy => Err(texts.with(Key::error_busy, &[("name", app_name)])),
        msi::Outcome::Failed(code) => Err(texts.with(Key::error_msi, &[("code", &code.to_string())])),
    }
}
