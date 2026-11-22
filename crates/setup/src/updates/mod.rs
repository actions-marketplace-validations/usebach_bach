//! Updates for an installed app, started by the app itself or by the
//! scheduled task. The installed `uninstall.exe` is this runtime with the
//! app's manifest, so it knows the update feed (`[updates]`):
//!
//! ```text
//! uninstall.exe --check-update [--json]   exit 10: a newer version is out, 0: up to date
//! uninstall.exe --download-update         fetch it in the background and keep it ready (exit 10: ready)
//! uninstall.exe --apply-update [--wait-for PID] [--launch]
//!                                         install what was kept ready, once the app has closed
//! uninstall.exe --update [--silent]       download if needed, then open its setup window (or install silently)
//! uninstall.exe --auto-update             one background round, as the scheduled task runs it
//! ```
//!
//! Everything is https only, checked against the feed's SHA-256, and a
//! setup file must carry this app's id and the feed's version before it
//! runs. See `stage.rs` for where a download waits, and `work/place.rs` for
//! why an update that is cut short can't leave a half-updated app.

mod apply;
pub(crate) mod authenticode;
mod auto;
pub(crate) mod curl;
pub(crate) mod feed;
#[cfg(target_os = "macos")]
pub(crate) mod macsetup;
mod rollout;
pub(crate) mod stage;

use std::path::PathBuf;
use std::process::Command;

use bach_pack::i18n::Key;
#[cfg(windows)]
use bach_pack::manifest::UNINSTALLER;
use bach_pack::manifest::{FeedEntry, Manifest};
use serde_json::json;

use crate::launch::Args;
use crate::package::Package;
use crate::system::{self, host};
use crate::texts::Texts;

/// Exit code of `--check-update` when a newer version is out, and of
/// `--download-update` and `--auto-update` when one is ready but not yet
/// installed.
pub use crate::launch::exit::UPDATE_AVAILABLE;

/// What one update run knows.
pub struct Context<'a> {
    pub args: &'a Args,
    pub package: &'a Package,
    pub texts: Texts,
    /// The feed's link, then its mirrors'.
    pub feeds: Vec<String>,
    /// The install folder.
    pub dir: PathBuf,
    /// The app's access key, when it has one and one is known.
    pub key: Option<String>,
}

/// What the feeds offer: the entry to download, and the same version as the
/// mirrors list it, to fall back on.
pub struct Offer {
    pub entry: FeedEntry,
    pub mirrors: Vec<FeedEntry>,
}

impl std::ops::Deref for Offer {
    type Target = FeedEntry;

    fn deref(&self) -> &FeedEntry {
        &self.entry
    }
}

impl Offer {
    /// Every entry that offers this version, the main feed's first.
    pub fn entries(&self) -> Vec<FeedEntry> {
        std::iter::once(self.entry.clone()).chain(self.mirrors.iter().cloned()).collect()
    }
}

impl Context<'_> {
    pub fn manifest(&self) -> &Manifest {
        &self.package.manifest
    }

    /// The version that is installed now.
    fn current_version(&self) -> String {
        system::installed(self.manifest())
            .map(|installed| installed.version)
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| self.manifest().app.version.clone())
    }

    /// The key to send with requests, when the app wants it sent, tied to
    /// the host of the app's main feed: nowhere else gets it.
    fn bearer(&self) -> Option<curl::Bearer> {
        let sends = self.manifest().access.as_ref().is_some_and(|access| access.send_key);
        let feed = self.feeds.first()?;
        self.key.as_deref().filter(|_| sends).and_then(|key| curl::Bearer::for_feed(key, feed))
    }

    /// What the feeds offer, in a sentence if none can be asked.
    fn latest(&self) -> Result<Offer, String> {
        let work = std::env::temp_dir().join(format!("{}-update", bach_pack::paths::safe_dir_name(&self.manifest().app.id)));
        let mut entries =
            feed::fetch_offers(&self.feeds, &work, self.bearer().as_ref(), feed::signer_of(self.manifest()), system::host::own_arch())
                .map_err(|error| self.texts.with(Key::error_download, &[("error", &error)]))?;
        let entry = entries.remove(0);
        Ok(Offer { entry, mirrors: entries })
    }

    /// Whether this computer takes `offer` by itself: a staged rollout may
    /// not have reached it yet. An update somebody asks for (`--update`)
    /// doesn't check this.
    fn in_rollout(&self, offer: &Offer) -> bool {
        let included = rollout::included(rollout::this_computer(&self.manifest().app.id, &offer.version), offer.rollout);
        if !included {
            crate::log::line(&format!(
                "{} is being rolled out to {}% of computers, and not this one yet.",
                offer.version,
                offer.rollout.unwrap_or(100)
            ));
        }
        included
    }

    /// A staged update that has been installed (or overtaken) is deleted.
    fn tidy(&self) {
        let app_id = &self.manifest().app.id;
        if let Some(staged) = stage::read(&stage::folder(app_id)) {
            if !feed::is_newer(&staged.version, &self.current_version()) {
                stage::clear(app_id);
            }
        }
    }
}

/// Deletes an update that was downloaded and never installed (the app is
/// being removed).
pub fn forget_staged(app_id: &str) {
    stage::clear(app_id);
}

pub fn run(args: &Args, package: &Package) -> i32 {
    host::attach_parent_console();
    let manifest = &package.manifest;
    let texts = Texts::new(manifest, args.lang.as_deref());
    let Some(updates) = &manifest.updates else {
        eprintln!("{} has no update feed ([updates] in its bach.toml).", manifest.app.name);
        return 2;
    };
    let dir = install_dir(package).unwrap_or_default();
    let key = match access_key(args, package) {
        Ok(key) => key,
        Err(problem) => {
            eprintln!("{problem}");
            return 1;
        }
    };
    if manifest.access.as_ref().is_some_and(|access| access.send_key) && key.is_none() {
        eprintln!(
            "{} sends an access key to look for updates, and none is saved on this computer. \
             Run its setup file again to enter it, or give one with --key-file.",
            manifest.app.name
        );
        return 1;
    }
    if let Some(channel) = &args.channel {
        return choose_channel(package, updates, channel);
    }
    let beta = system::channel::is_beta(&manifest.app.id);
    let ctx = Context { args, package, texts, feeds: updates.feeds_for(beta), dir, key };
    let result = if args.apply_update {
        apply_ready(&ctx)
    } else if args.auto_update {
        auto::run(&ctx)
    } else if args.download_update {
        download(&ctx)
    } else if args.check_update {
        check(&ctx)
    } else {
        update(&ctx)
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            crate::log::line(&format!("Failed: {error}"));
            1
        }
    }
}

/// `--channel beta|stable|show`: says or changes which feed this installation
/// follows. The next check, download or update uses it; nothing is downloaded now.
fn choose_channel(package: &Package, updates: &bach_pack::manifest::Updates, channel: &str) -> i32 {
    let app = &package.manifest.app;
    let now = if system::channel::is_beta(&app.id) { "beta" } else { "stable" };
    if channel == "show" {
        println!("{now}");
        return 0;
    }
    if channel == "beta" && updates.beta_feed.is_none() {
        eprintln!("{} has no beta versions: its bach.toml has no [updates] beta_feed.", app.name);
        return 1;
    }
    if let Err(err) = system::channel::set(&app.id, channel == "beta") {
        eprintln!("{err:#}");
        return 1;
    }
    if channel == now {
        println!("{} already follows the {channel} channel.", app.name);
    } else if channel == "beta" {
        println!("{} now follows the beta channel.", app.name);
    } else {
        println!("{} now follows the stable channel. A beta version that is installed stays until a stable one is newer.", app.name);
    }
    0
}

/// The access key an update run uses: one given on the command line (a key
/// file, or `BACH_ACCESS_KEY`), else the one kept for this app by its install.
/// None for an app without `[access]`.
fn access_key(args: &Args, package: &Package) -> Result<Option<String>, String> {
    if package.manifest.access.is_none() {
        return Ok(None);
    }
    Ok(args.access_key()?.or_else(|| system::secret::load(&package.manifest.app.id)))
}

/// The folder this `uninstall.exe` sits in, or else the one Settings › Apps
/// lists for the app.
fn install_dir(package: &Package) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let name = package.exe.file_name()?.to_string_lossy().to_lowercase();
        if name == UNINSTALLER {
            return package.exe.parent().map(PathBuf::from);
        }
    }
    // On a Mac the uninstaller is in the Library; the install record says where the app is.
    #[cfg(target_os = "macos")]
    if let Some(dir) = system::recorded_dir(&package.manifest) {
        return Some(dir);
    }
    system::installed(&package.manifest).map(|installed| installed.dir)
}

/// `--check-update`: says whether a newer version is out, and changes nothing.
fn check(ctx: &Context) -> Result<i32, String> {
    let entry = ctx.latest()?;
    let current = ctx.current_version();
    let newer = feed::is_newer(&entry.version, &current) && ctx.in_rollout(&entry);
    let ready = newer && stage::read(&stage::folder(&ctx.manifest().app.id)).is_some_and(|staged| staged.is_for(&entry));
    if ctx.args.json {
        let answer = json!({
            "app": ctx.manifest().app.name,
            "current": current,
            "latest": entry.version,
            "available": newer,
            "downloaded": ready,
            "notes": entry.notes,
            "size": entry.size,
        });
        println!("{answer}");
    } else if newer {
        println!("{} {} is available.", ctx.manifest().app.name, entry.version);
    } else {
        println!("{} {} is up to date.", ctx.manifest().app.name, current);
    }
    ctx.tidy();
    Ok(if newer { UPDATE_AVAILABLE } else { 0 })
}

/// `--download-update`: fetches a newer version and keeps it ready, so
/// installing it later takes seconds and needs no network.
fn download(ctx: &Context) -> Result<i32, String> {
    let entry = ctx.latest()?;
    ctx.tidy();
    if !feed::is_newer(&entry.version, &ctx.current_version()) || !ctx.in_rollout(&entry) {
        println!("{} {} is up to date.", ctx.manifest().app.name, ctx.current_version());
        return Ok(0);
    }
    stage_entry(ctx, &entry, true)?;
    println!("{} {} is ready to install.", ctx.manifest().app.name, entry.version);
    Ok(UPDATE_AVAILABLE)
}

/// Whether the installed files are the ones `delta` was made from, as far as
/// the update can tell before it downloads anything: the same version, and
/// every installed file's checksum on record (a setup file made before Bach
/// recorded them can't be checked, so no patch is tried).
fn delta_fits(ctx: &Context, delta: &bach_pack::manifest::DeltaEntry) -> bool {
    let files = &ctx.manifest().files;
    bach_pack::version::compare(&delta.from, &ctx.current_version()) == Some(std::cmp::Ordering::Equal)
        && !files.is_empty()
        && files.iter().all(|file| file.sha256.is_some())
}

/// Downloads what `offer` describes: the update patch when this computer has
/// the version it was made from and `allow_patch` (the window can't fall back
/// by itself, so it asks for the full setup file), the full setup file
/// otherwise or when the patch can't be downloaded.
fn stage_entry(ctx: &Context, offer: &Offer, allow_patch: bool) -> Result<stage::Staged, String> {
    if let Some(delta) = offer.entry.delta.as_ref().filter(|delta| allow_patch && delta_fits(ctx, delta)) {
        crate::log::line(&format!("Downloading the patch from {} to {} of {}", delta.from, offer.version, ctx.manifest().app.name));
        match stage::ensure_delta(&offer.entry, ctx.bearer().as_ref(), &ctx.manifest().app.id, &mut |_| {}) {
            Ok(staged) => return Ok(staged),
            Err(error) => crate::log::line(&format!("The patch couldn't be downloaded ({error}); downloading the full setup file.")),
        }
    }
    crate::log::line(&format!("Downloading {} {}", ctx.manifest().app.name, offer.version));
    stage::ensure_any(&offer.entries(), &ctx.manifest().app.id, ctx.bearer().as_ref(), &mut |_| {})
        .map_err(|error| ctx.texts.with(Key::error_download, &[("error", &error)]))
}

/// `--apply-update`: installs the update that was kept ready, once the app
/// that asked (`--wait-for`) has closed.
fn apply_ready(ctx: &Context) -> Result<i32, String> {
    ctx.tidy();
    let app_id = &ctx.manifest().app.id;
    let Some(staged) = stage::read(&stage::folder(app_id)) else {
        println!("No update is waiting.");
        return Ok(0);
    };
    apply::wait_for_app(ctx)?;
    apply::install(ctx, &staged)
}

/// `--update`: the newest version, now. Downloads it (or uses the one kept
/// ready) and opens its setup window, or with `--silent` installs it and
/// gives back the setup's own exit code.
fn update(ctx: &Context) -> Result<i32, String> {
    let entry = ctx.latest()?;
    ctx.tidy();
    if !feed::is_newer(&entry.version, &ctx.current_version()) {
        println!("{} {} is up to date.", ctx.manifest().app.name, ctx.current_version());
        return Ok(0);
    }
    eprintln!("{}", ctx.texts.get(Key::step_downloading));
    let staged = stage_entry(ctx, &entry, ctx.args.silent)?;
    // A publisher that signs its updates gets only updates it signed.
    if let Err(problem) = authenticode::check(ctx.manifest(), &staged.file) {
        stage::clear(&ctx.manifest().app.id);
        return Err(problem);
    }
    // The download is done before the app is asked to close, so waiting for
    // it never delays the user twice.
    apply::wait_for_app(ctx)?;
    if ctx.args.silent {
        return apply::install(ctx, &staged);
    }
    #[cfg(windows)]
    let program = staged.file.clone();
    // On a Mac the Setup app is copied out of the download, so the window can stay open
    // after the disk image is gone; the next update run clears the folder.
    #[cfg(target_os = "macos")]
    let program = macsetup::unpack(&staged.file, &stage::folder(&ctx.manifest().app.id).join(format!("run-{}", std::process::id())))?;
    let mut command = Command::new(&program);
    if let Some(lang) = &ctx.args.lang {
        command.args(["--lang", lang]);
    }
    if let Some(key) = &ctx.key {
        // So the window doesn't ask for a key this computer already holds.
        command.env(crate::launch::KEY_ENV, key);
    }
    command.spawn().map_err(|err| err.to_string())?;
    Ok(0)
}
