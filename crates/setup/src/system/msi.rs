//! The `.msi` package's front: the same Bach window, with Windows Installer
//! doing the work underneath.
//!
//! A package built with `bach build --msi` carries a small copy of this program
//! (`--msi-package`). When someone opens the `.msi` (or removes it under
//! Settings › Apps) with Windows' own interface, the package starts that copy
//! instead of showing Windows Installer's plain progress box. The window asks the
//! same questions as a setup file's and then calls Windows Installer's own API
//! (`msi.dll`) with the answers as properties, so the install is a real one:
//! the same files, registry values, upgrade and removal rules, and Settings ›
//! Apps entry that `msiexec /i package.msi /qn` gives, which stays the way IT
//! tools install it. The bar follows Windows Installer's own progress messages.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

use bach_pack::manifest::{Choices, Manifest};

use super::Installed;

/// The package this run belongs to.
#[derive(Clone, Debug)]
pub struct Link {
    /// The `.msi` file the window installs from (`--msi-package`).
    pub package: PathBuf,
    /// This version's product code and the app's upgrade code, in braces.
    pub product: String,
    pub upgrade: String,
}

static LINK: OnceLock<Link> = OnceLock::new();

/// Remembers the package for the rest of the run.
pub fn set_link(link: Link) {
    let _ = LINK.set(link);
}

/// The package, when this run is the front of a `.msi`.
pub fn link() -> Option<&'static Link> {
    LINK.get()
}

/// The Windows Installer feature of a component or an option: the names the
/// package gives them (`bach build --msi` writes these, and `--with` ids are the
/// same words, with `_` where a feature can't hold a `-`).
pub fn feature_id(id: &str) -> String {
    id.replace('-', "_")
}

/// The features the choices turn on, as `ADDLOCAL` wants them.
pub fn features(manifest: &Manifest, choices: &Choices) -> Vec<String> {
    let mut features = vec!["Main".to_string()];
    let install = &manifest.install;
    for (on, offered, id) in [
        (choices.desktop_shortcut, install.desktop_shortcut.offered(), "desktop_shortcut"),
        (choices.start_menu_shortcut, install.start_menu_shortcut.offered(), "start_menu"),
        (choices.start_with_windows, install.start_with_windows.offered(), "autostart"),
        (choices.add_to_path, install.add_to_path.offered(), "path"),
    ] {
        if on && offered {
            features.push(id.to_string());
        }
    }
    features.extend(choices.components.iter().map(|id| feature_id(id)));
    features
}

/// The property list for an install: where it goes and which features.
pub fn install_properties(manifest: &Manifest, choices: &Choices, dir: &Path) -> String {
    // The folder keeps its closing backslash out of the quotes' way.
    let dir = dir.display().to_string();
    let dir = dir.trim_end_matches('\\');
    format!("INSTALLFOLDER=\"{}\" ADDLOCAL={}", dir.replace('"', "\"\""), features(manifest, choices).join(","))
}

/// The install of this app Windows Installer knows about, when there is one:
/// any version of the product (they share an upgrade code), and where it is.
pub fn installed(link: &Link, manifest: &Manifest) -> Option<Installed> {
    let product = related_products(&link.upgrade).into_iter().next()?;
    let dir = product_info(&product, "InstallLocation").filter(|dir| !dir.is_empty())?;
    let version = product_info(&product, "VersionString").unwrap_or_default();
    // The package's version is three numbers; the app's may be written "v7" or "7": the same version.
    let version = if numbers(&version) == numbers(&manifest.app.version) { manifest.app.version.clone() } else { version };
    Some(Installed { dir: PathBuf::from(dir), version })
}

/// A version as Windows Installer reads it: up to three numbers, a `v` in front ignored, the rest 0.
fn numbers(version: &str) -> Vec<u32> {
    let mut parts: Vec<u32> = version.trim().trim_start_matches(['v', 'V']).split('.').map(|part| part.parse().unwrap_or(0)).take(3).collect();
    parts.resize(3, 0);
    parts
}

/// The product code of the installed version, if any.
pub fn installed_product(link: &Link) -> Option<String> {
    related_products(&link.upgrade).into_iter().next()
}

/// Whether the installed product has `feature` installed on this computer.
pub fn feature_is_on(product: &str, feature: &str) -> bool {
    imp::feature_state(product, feature) == 3
}

fn related_products(upgrade: &str) -> Vec<String> {
    imp::related_products(upgrade)
}

fn product_info(product: &str, property: &str) -> Option<String> {
    imp::product_info(product, property)
}

/// How an install, repair or removal ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// The user stopped it; Windows Installer undid what it had done.
    Cancelled,
    /// Another install is running.
    Busy,
    Failed(u32),
}

/// How far the job is, as the engine's thread reports it for the window's thread to read:
/// per mille, or -1 while it is only getting ready.
pub struct Shared {
    permille: AtomicI32,
}

impl Default for Shared {
    fn default() -> Self {
        Self { permille: AtomicI32::new(-1) }
    }
}

impl Shared {
    /// The fraction done (0..1), or None while Windows Installer is getting ready.
    pub fn fraction(&self) -> Option<f32> {
        let permille = self.permille.load(Ordering::Relaxed);
        (permille >= 0).then(|| permille as f32 / 1000.)
    }

    fn set(&self, fraction: f32) {
        self.permille.store((fraction.clamp(0., 1.) * 1000.).round() as i32, Ordering::Relaxed);
    }
}

/// What to ask Windows Installer for.
pub enum Job {
    /// Install or update from the package, with these properties.
    Install(String),
    /// Put the installed version's files and registry values back.
    Repair,
    /// Remove the product with this code.
    Remove(String),
}

/// Runs the job without Windows Installer's own windows (this blocks, so call it from the
/// worker thread, and read `shared` from another). `cancelled` is asked on every message: when it
/// says yes, Windows Installer stops and undoes what it did.
pub fn run(link: &Link, job: &Job, log: Option<&Path>, shared: &Shared, cancelled: fn() -> bool) -> Outcome {
    imp::run(link, job, log, shared, cancelled)
}

/// Turns the numbers Windows Installer reports into a fraction. It reports in
/// "ticks": a reset says how many there are in all, increments move the bar,
/// and more can be added on the way. The first reset belongs to getting ready;
/// the one that says a script is running is the install itself.
#[derive(Default)]
pub struct Ticks {
    total: i64,
    done: i64,
    backwards: bool,
    running: bool,
}

impl Ticks {
    /// Takes one progress record (its fields as numbers, field 1 first).
    pub fn take(&mut self, fields: [i32; 4]) -> Option<f32> {
        let [kind, a, b, c] = fields.map(i64::from);
        match kind {
            // Reset: a new bar of `a` ticks, forward or backward; `c` says it is the script's own.
            0 => {
                self.total = a.max(0);
                self.done = if b == 1 { self.total } else { 0 };
                self.backwards = b == 1;
                self.running = c == 1;
            }
            // How much one action's own message moves the bar; the actions report it themselves.
            1 => {}
            2 => self.done = if self.backwards { self.done - a } else { self.done + a },
            3 => self.total += a,
            _ => {}
        }
        if !self.running || self.total <= 0 || self.backwards {
            return None;
        }
        Some((self.done.clamp(0, self.total) as f32 / self.total as f32).clamp(0., 1.))
    }
}

/// The message the engine hands the callback: its kind is in the high byte.
const MESSAGE_PROGRESS: u32 = 0x0A;
const MESSAGE_KIND_SHIFT: u32 = 24;

/// Answers the engine gives a callback: handled, or please stop.
const IDOK: i32 = 1;
const IDCANCEL: i32 = 2;

/// What the engine's callback needs: where to report, how to ask for a stop.
struct Context<'a> {
    shared: &'a Shared,
    cancelled: fn() -> bool,
    ticks: Mutex<Ticks>,
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::ffi::OsStrExt;

    type Handler = unsafe extern "system" fn(*const c_void, u32, u32) -> i32;

    #[link(name = "msi", kind = "raw-dylib")]
    unsafe extern "system" {
        fn MsiInstallProductW(package: *const u16, command_line: *const u16) -> u32;
        fn MsiConfigureProductExW(product: *const u16, level: i32, state: i32, command_line: *const u16) -> u32;
        fn MsiSetInternalUI(level: u32, window: *mut c_void) -> u32;
        fn MsiSetExternalUIRecord(handler: Option<Handler>, filter: u32, context: *const c_void, previous: *mut Option<Handler>) -> u32;
        fn MsiRecordGetInteger(record: u32, field: u32) -> i32;
        fn MsiEnableLogW(mode: u32, file: *const u16, attributes: u32) -> u32;
        fn MsiEnumRelatedProductsW(upgrade: *const u16, reserved: u32, index: u32, product: *mut u16) -> u32;
        fn MsiGetProductInfoW(product: *const u16, property: *const u16, value: *mut u16, len: *mut u32) -> u32;
        fn MsiQueryFeatureStateW(product: *const u16, feature: *const u16) -> i32;
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn wide_path(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }

    pub fn related_products(upgrade: &str) -> Vec<String> {
        let code = wide(upgrade);
        let mut found = Vec::new();
        for index in 0..32 {
            let mut product = [0u16; 39];
            // SAFETY: `product` holds the 39 characters the API writes (a GUID in braces and its end).
            let status = unsafe { MsiEnumRelatedProductsW(code.as_ptr(), 0, index, product.as_mut_ptr()) };
            if status != 0 {
                break;
            }
            let end = product.iter().position(|c| *c == 0).unwrap_or(product.len());
            found.push(String::from_utf16_lossy(&product[..end]));
        }
        found
    }

    pub fn product_info(product: &str, property: &str) -> Option<String> {
        let (product, property) = (wide(product), wide(property));
        let mut buffer = vec![0u16; 1024];
        let mut len = buffer.len() as u32;
        // SAFETY: the buffer and its length are valid for the call; the API writes at most `len` characters.
        let status = unsafe { MsiGetProductInfoW(product.as_ptr(), property.as_ptr(), buffer.as_mut_ptr(), &mut len) };
        (status == 0).then(|| String::from_utf16_lossy(&buffer[..(len as usize).min(buffer.len())]))
    }

    pub fn feature_state(product: &str, feature: &str) -> i32 {
        let (product, feature) = (wide(product), wide(feature));
        // SAFETY: both strings are NUL-ended and outlive the call.
        unsafe { MsiQueryFeatureStateW(product.as_ptr(), feature.as_ptr()) }
    }

    /// The engine calls this for every message it would show. Only progress
    /// matters here; every other message is left to the (hidden) default.
    unsafe extern "system" fn handler(context: *const c_void, message: u32, record: u32) -> i32 {
        // SAFETY: `context` is the `Context` `run` passed and keeps alive until the job ends.
        let context = unsafe { &*(context as *const Context) };
        if (message >> MESSAGE_KIND_SHIFT) == MESSAGE_PROGRESS {
            // SAFETY: `record` is a record handle the engine gives for the length of this call.
            let fields = [1, 2, 3, 4].map(|field| unsafe { MsiRecordGetInteger(record, field) });
            if let Some(fraction) = context.ticks.lock().ok().and_then(|mut ticks| ticks.take(fields)) {
                // The bar only moves forward; a later, smaller report (another phase) leaves it.
                if context.shared.fraction().is_none_or(|seen| fraction > seen) {
                    context.shared.set(fraction);
                }
            }
        }
        if (context.cancelled)() {
            IDCANCEL
        } else {
            IDOK
        }
    }

    pub fn run(link: &Link, job: &Job, log: Option<&Path>, shared: &Shared, cancelled: fn() -> bool) -> Outcome {
        let context = Context { shared, cancelled, ticks: Mutex::new(Ticks::default()) };
        if let Some(log) = log {
            let file = wide_path(log);
            // 0x1007: fatal errors, errors, warnings and the action lines; no lines for the user's own text.
            // SAFETY: the path is NUL-ended and outlives the call.
            unsafe { MsiEnableLogW(0x1007 | 0x0080 | 0x0400, file.as_ptr(), 0) };
        }
        // SAFETY: no windows (2 = none, the quiet level); the handler and its context stay alive until the job returns.
        let status = unsafe {
            MsiSetInternalUI(2, std::ptr::null_mut());
            let mut previous: Option<Handler> = None;
            // Errors and progress only: 0x2 error, 0x400 progress, 0x1 fatal exit.
            MsiSetExternalUIRecord(Some(handler), 0x1 | 0x2 | 0x400, &context as *const Context as *const c_void, &mut previous);
            let status = match job {
                Job::Install(properties) => MsiInstallProductW(wide_path(&link.package).as_ptr(), wide(properties).as_ptr()),
                Job::Repair => {
                    MsiInstallProductW(wide_path(&link.package).as_ptr(), wide("REINSTALL=ALL REINSTALLMODE=vomus").as_ptr())
                }
                // INSTALLSTATE_ABSENT is 2; INSTALLLEVEL_DEFAULT is 0.
                Job::Remove(product) => MsiConfigureProductExW(wide(product).as_ptr(), 0, 2, wide("").as_ptr()),
            };
            MsiSetExternalUIRecord(previous, 0, std::ptr::null(), std::ptr::null_mut());
            status
        };
        match status {
            // ERROR_SUCCESS, and "done, restart for the rest" (3010, 1641): the files are in.
            0 | 3010 | 1641 => Outcome::Done,
            1602 => Outcome::Cancelled,
            1618 => Outcome::Busy,
            other => Outcome::Failed(other),
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn related_products(_: &str) -> Vec<String> {
        Vec::new()
    }
    pub fn product_info(_: &str, _: &str) -> Option<String> {
        None
    }
    pub fn feature_state(_: &str, _: &str) -> i32 {
        -1
    }
    pub fn run(_: &Link, _: &Job, _: Option<&Path>, _: &Shared, _: fn() -> bool) -> Outcome {
        Outcome::Failed(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_the_way_the_package_writes_them() {
        assert_eq!(numbers("v7"), numbers("7.0.0"));
        assert_eq!(numbers("1.4"), [1, 4, 0]);
        assert_ne!(numbers("1.4.1"), numbers("1.4.2"));
    }

    #[test]
    fn the_bar_follows_the_scripts_ticks_only() {
        let mut ticks = Ticks::default();
        // Getting ready: a reset that isn't the script's own shows nothing yet.
        assert_eq!(ticks.take([0, 100, 0, 0]), None);
        assert_eq!(ticks.take([2, 50, 0, 0]), None);
        // The script: 1000 ticks, moved a little at a time, with more added on the way.
        assert_eq!(ticks.take([0, 1000, 0, 1]), Some(0.0));
        assert_eq!(ticks.take([2, 250, 0, 0]), Some(0.25));
        assert_eq!(ticks.take([3, 1000, 0, 0]), Some(0.125));
        assert_eq!(ticks.take([2, 2000, 0, 0]), Some(1.0), "never past the end");
    }

    #[test]
    fn a_bar_that_runs_backwards_is_a_rollback_and_shows_nothing() {
        let mut ticks = Ticks::default();
        ticks.take([0, 1000, 0, 1]);
        ticks.take([2, 600, 0, 0]);
        assert_eq!(ticks.take([0, 1000, 1, 1]), None);
        assert_eq!(ticks.take([2, 100, 0, 0]), None);
    }

    #[test]
    fn features_follow_the_choices_and_the_packages_own_names() {
        use bach_pack::manifest::Choice;
        let mut manifest = crate::package::Package::sample().manifest;
        manifest.install.desktop_shortcut = Choice::On;
        manifest.install.start_menu_shortcut = Choice::Always;
        manifest.install.add_to_path = Choice::Off;
        let mut choices = Choices::defaults(&manifest);
        assert_eq!(features(&manifest, &choices), ["Main", "desktop_shortcut"]);
        choices.add_to_path = true;
        choices.components.push("extra-tools".into());
        assert_eq!(features(&manifest, &choices), ["Main", "desktop_shortcut", "path", "extra_tools"]);
        let line = install_properties(&manifest, &choices, Path::new(r"D:\My Apps\Sample\"));
        assert_eq!(line, r#"INSTALLFOLDER="D:\My Apps\Sample" ADDLOCAL=Main,desktop_shortcut,path,extra_tools"#);
    }
}
