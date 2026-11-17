//! The online installer's first job: find the newest setup file through the
//! update feed, download it, and hand it over to the normal install.
//!
//! The online installer carries the manifest it was built with (so the
//! window can show the app's name, logo and choices) but no files. Nothing
//! it knows about the app's version matters: the feed decides. The download
//! is the same one an installed app's updater makes, so it resumes where an
//! earlier try stopped and is checked before anything is installed: its
//! SHA-256 against the feed's, that it is a setup file for this very app, that
//! it carries the version the feed names, and, when the app names a publisher
//! (`[updates] signer`), that it is signed by them.

use std::sync::Arc;

use bach_pack::i18n::Key;

use super::{sentence, InstallPlan, Report};
use crate::package::Package;
use crate::updates::curl::Bearer;
use crate::updates::{feed, stage};

/// Where the download's share of the progress bar ends.
pub const DOWNLOAD_SHARE: f32 = 0.5;

/// Finds, downloads and reads the newest full setup file for this app.
pub fn fetch(plan: &InstallPlan, report: Report) -> Result<Package, String> {
    let manifest = &plan.package.manifest;
    let texts = &plan.texts;
    let app_id = &manifest.app.id;
    let unavailable = |error: String| texts.with(Key::error_fetch, &[("error", &capitalised(&sentence(error)))]);
    let Some(updates) = &manifest.updates else {
        return Err(texts.get(Key::error_damaged));
    };

    // The key goes to the server only when the app asked for that.
    let sends = manifest.access.as_ref().is_some_and(|access| access.send_key);
    let bearer = plan.key.as_deref().filter(|_| sends).and_then(|key| Bearer::for_feed(key, &updates.feed));
    let refused = |error: String| {
        if error.contains("refused the access key") {
            texts.get(Key::access_wrong)
        } else {
            unavailable(error)
        }
    };

    report(0.02, &texts.get(Key::step_finding));
    let scratch = std::env::temp_dir().join(format!("{}-online", bach_pack::paths::safe_dir_name(app_id)));
    let mut offers = feed::fetch_offers(&updates.feeds(), &scratch, bearer.as_ref(), feed::signer_of(manifest)).map_err(refused)?;
    crate::log::line(&format!("The feed offers {} {}", manifest.app.name, offers[0].version));

    let label = texts.get(Key::step_fetching);
    let total = offers[0].size.filter(|size| *size > 0);
    let staged = stage::ensure_any(&offers, app_id, bearer.as_ref(), &mut |done| {
        let fraction = total.map_or(0.0, |total| (done as f32 / total as f32).min(1.0));
        report(0.04 + (DOWNLOAD_SHARE - 0.06) * fraction, &label);
    })
    .map_err(refused)?;
    offers.clear();
    // The file is installed from, not run, but it decides what lands on the computer, so it has
    // to be as the publisher signed it when the app names one (`[updates] signer`), like an update.
    if let Err(problem) = crate::updates::authenticode::check(manifest, &staged.file) {
        crate::log::line(&format!("The downloaded setup file was refused: {problem}"));
        stage::clear(app_id);
        return Err(unavailable(problem));
    }

    let package = Package::load_from(staged.file.clone())
        .map_err(|err| format!("{err:#}"))
        .and_then(|package| package.ok_or_else(|| "it isn't a setup file".to_string()))
        .map_err(|problem| format!("{} {}", texts.get(Key::error_damaged), capitalised(&sentence(problem))))?;
    if !package.has_files() || package.manifest.online || package.manifest.app.id != *app_id {
        stage::clear(app_id);
        return Err(texts.get(Key::error_damaged));
    }
    crate::log::line(&format!("Downloaded {} {}", package.manifest.app.name, package.manifest.app.version));
    Ok(package)
}

/// The downloaded setup file isn't needed once the app is installed.
pub fn forget_download(plan: &InstallPlan) {
    stage::clear(&plan.package.manifest.app.id);
}

/// The plan for installing from the downloaded setup file, with the
/// choices carried over to its manifest.
pub fn plan_for(plan: &InstallPlan, downloaded: Package) -> InstallPlan {
    let choices = plan.choices.clone().carry_over(&plan.package.manifest, &downloaded.manifest);
    InstallPlan { package: Arc::new(downloaded), texts: plan.texts.clone(), dir: plan.dir.clone(), choices, key: plan.key.clone() }
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_start_with_a_capital() {
        assert_eq!(capitalised("the update feed isn't valid."), "The update feed isn't valid.");
        assert_eq!(capitalised(""), "");
        assert_eq!(capitalised("éclair"), "Éclair");
    }
}
