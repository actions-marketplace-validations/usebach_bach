//! The setup window's state: which page it's on, the choices on the first
//! page, and the running job. Drawing lives in `pages/`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bach_pack::i18n::Key;
use bach_pack::manifest::{Choices, Manifest, RunWhen};
use futures::channel::mpsc;
use futures::StreamExt;
use gpui::{Context, FocusHandle, ScrollHandle, SharedString, Task, Window};

use crate::launch::Mode;
use crate::license::LicenseView;
use crate::package::Package;
use crate::system::Installed;
use crate::texts::Texts;
use crate::work::{self, Event, InstallPlan, RemovePlan};

/// The bar gets this long to glide to full before the last page shows.
const FINISH_HOLD: Duration = Duration::from_millis(650);

#[derive(Clone, Debug)]
pub enum Stage {
    /// Something the app needs is missing (a requirement, by index).
    Requirement(usize),
    /// The access key to enter before anything else.
    Access,
    /// The license to accept before anything else.
    License,
    /// The first page: options and the go button.
    Ready,
    Working {
        progress: f32,
        label: SharedString,
    },
    Done,
    Failed(SharedString),
}

/// One box on the install page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    DesktopShortcut,
    StartMenu,
    StartWithWindows,
    AddToPath,
    AutoUpdate,
    /// A component, by its index in the manifest.
    Component(usize),
    DeleteData,
}

pub struct SetupView {
    pub package: Arc<Package>,
    pub texts: Arc<Texts>,
    pub mode: Mode,
    /// Pretend: nothing is installed or removed.
    pub preview: bool,
    /// An install already on this computer (install mode only).
    pub existing: Option<Installed>,
    /// Where the app goes (or is, when removing).
    pub dir: PathBuf,
    pub choices: Choices,
    pub delete_data: bool,
    /// The installed version is newer than this one.
    pub downgrade: bool,
    /// Files in the chosen folder that a first install would replace, and
    /// that aren't from an earlier install of this app.
    pub overwrites: Vec<String>,
    /// The access key comes after the requirements are met.
    pub access_pending: bool,
    /// What has been typed or pasted so far, or the key given up front.
    pub key: String,
    /// Why the last key didn't do, as a sentence.
    pub key_error: Option<SharedString>,
    /// A key is being checked on the background executor.
    pub key_checking: bool,
    /// The license comes after the requirements are met.
    pub license_pending: bool,
    /// Waiting for the app to close after "Close and continue".
    pub closing_app: bool,
    /// What happened to "Save support info" on the failed page.
    pub support: SupportState,
    /// "Cancel" was pressed while files were being unpacked; the job is stopping.
    pub cancelling: bool,
    /// The next run puts the installed version back (Windows Installer's repair).
    repairing: bool,
    pub stage: Stage,
    /// Counts attempts, so each one's page and bar start fresh.
    pub attempt: usize,
    /// Receives the keys; see `keyboard.rs`.
    pub focus_handle: FocusHandle,
    /// The control the keyboard is on, by its place in `targets()`. None
    /// until Tab is pressed, so mouse users never see a focus ring.
    pub focused: Option<usize>,
    /// The card of choices, for its scrollbar and for keeping the focused
    /// row in view.
    pub choices_scroll: ScrollHandle,
    /// The license page: its text, scroll position and selection.
    pub license: LicenseView,
    job: Option<Task<()>>,
    picker: Option<Task<()>>,
    pub(crate) closer: Option<Task<()>>,
    pub(crate) checker: Option<Task<()>>,
}

/// The failed page's "Save support info" button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupportState {
    Idle,
    Saved,
    Failed,
}

impl SetupView {
    pub fn new(
        package: Arc<Package>,
        texts: Arc<Texts>,
        mode: Mode,
        preview: bool,
        given_key: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let manifest = &package.manifest;
        let (existing, previous, dir, choices, downgrade) = match &mode {
            Mode::Install => {
                let start = crate::start::install(manifest);
                (start.existing, start.previous, start.dir, start.choices, start.downgrade)
            }
            Mode::Remove { dir } => (None, None, dir.clone(), Choices::default(), false),
        };
        // A preview can pretend this very version is installed, to show the repair page.
        let existing = match std::env::var("BACH_PREVIEW_INSTALLED") {
            Ok(version) if preview && matches!(mode, Mode::Install) && existing.is_none() => Some(Installed {
                dir: dir.clone(),
                version: if version.trim().is_empty() { manifest.app.version.clone() } else { version },
            }),
            _ => existing,
        };
        // Screenshots for docs shouldn't show whose computer took them.
        let dir = match std::env::var_os("BACH_PREVIEW_DIR") {
            Some(shown) if preview => PathBuf::from(shown),
            _ => dir,
        };
        let license_pending = needs_license(manifest, previous.as_ref(), &mode, &texts);
        // A key handed over up front (an update's own, or --key-file) skips
        // the page; it is still checked when the install starts.
        let key = given_key.unwrap_or_default();
        let access_pending = manifest.access.is_some() && matches!(mode, Mode::Install) && key.is_empty();
        let missing = match &mode {
            // Only reads, so a preview shows the page too.
            Mode::Install => crate::start::missing_requirements(manifest).first().copied(),
            _ => None,
        };
        let stage = match missing {
            Some(index) => Stage::Requirement(index),
            None if access_pending => Stage::Access,
            None if license_pending => Stage::License,
            None => Stage::Ready,
        };
        let license = LicenseView::new(manifest.license(texts.language.code).unwrap_or_default());
        let mut view = Self {
            package,
            texts,
            mode,
            preview,
            existing,
            dir,
            choices,
            delete_data: false,
            downgrade,
            overwrites: Vec::new(),
            access_pending,
            key,
            key_error: None,
            key_checking: false,
            license_pending,
            closing_app: false,
            support: SupportState::Idle,
            cancelling: false,
            repairing: false,
            stage,
            attempt: 0,
            focus_handle: cx.focus_handle(),
            focused: None,
            choices_scroll: ScrollHandle::new(),
            license,
            job: None,
            picker: None,
            closer: None,
            checker: None,
        };
        view.refresh_overwrites();
        view
    }

    /// Looks again at which foreign files the chosen folder and options
    /// would replace. Only a first install into an existing folder can.
    pub fn refresh_overwrites(&mut self) {
        self.overwrites = if self.is_removing() || self.existing.is_some() {
            Vec::new()
        } else {
            work::foreign_files(&self.package.manifest, &self.choices, &self.dir)
        };
    }

    /// The first page once nothing is missing: the key, then the license,
    /// then the choices.
    pub fn first_page(&self) -> Stage {
        if self.access_pending {
            Stage::Access
        } else if self.license_pending {
            Stage::License
        } else {
            Stage::Ready
        }
    }

    pub fn is_removing(&self) -> bool {
        matches!(self.mode, Mode::Remove { .. })
    }

    pub fn is_on(&self, toggle: Toggle) -> bool {
        let choices = &self.choices;
        match toggle {
            Toggle::DesktopShortcut => choices.desktop_shortcut,
            Toggle::StartMenu => choices.start_menu_shortcut,
            Toggle::StartWithWindows => choices.start_with_windows,
            Toggle::AddToPath => choices.add_to_path,
            Toggle::AutoUpdate => choices.auto_update,
            Toggle::Component(index) => {
                self.package.manifest.components.get(index).is_some_and(|component| choices.components.contains(&component.id))
            }
            Toggle::DeleteData => self.delete_data,
        }
    }

    pub fn toggle(&mut self, toggle: Toggle, cx: &mut Context<Self>) {
        let on = !self.is_on(toggle);
        let choices = &mut self.choices;
        match toggle {
            Toggle::DesktopShortcut => choices.desktop_shortcut = on,
            Toggle::StartMenu => choices.start_menu_shortcut = on,
            Toggle::StartWithWindows => choices.start_with_windows = on,
            Toggle::AddToPath => choices.add_to_path = on,
            Toggle::AutoUpdate => choices.auto_update = on,
            Toggle::Component(index) => {
                if let Some(component) = self.package.manifest.components.get(index) {
                    choices.set(&self.package.manifest, &component.id.clone(), on);
                }
            }
            Toggle::DeleteData => self.delete_data = on,
        }
        self.refresh_overwrites();
        cx.notify();
    }

    /// The license is accepted: on to the choices.
    pub fn accept_license(&mut self, cx: &mut Context<Self>) {
        self.license_pending = false;
        self.stage = Stage::Ready;
        self.focused = None;
        cx.notify();
    }

    /// Opens the folder picker; the pick becomes `<folder>\<app folder>`
    /// unless it already is one.
    pub fn choose_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker.is_some() || !self.can_change_folder() {
            return;
        }
        let title = self.texts.get(Key::choose_folder);
        let picked = crate::ui::folder_dialog::pick_folder(&title, window);
        let folder_name = self.package.manifest.install.folder.clone();
        self.picker = Some(cx.spawn(async move |this, cx| {
            let picked = picked.await;
            this.update(cx, |this, cx| {
                if let Some(folder) = picked {
                    this.dir = install_folder(folder, &folder_name);
                    this.refresh_overwrites();
                }
                this.picker = None;
                cx.notify();
            })
            .ok();
        }));
    }

    /// A first install may move; an update stays where it is.
    pub fn can_change_folder(&self) -> bool {
        !self.is_removing() && self.existing.is_none() && self.package.manifest.install.change_folder
    }

    /// Whether the first page offers "Repair": this very version is installed,
    /// so its files can be checked and put back without changing anything else.
    pub fn is_repairable(&self) -> bool {
        let manifest = &self.package.manifest;
        matches!(self.stage, Stage::Ready)
            && !self.is_removing()
            && !manifest.online
            && !self.downgrade
            && self.existing.as_ref().is_some_and(|installed| installed.version == manifest.app.version)
    }

    /// "Repair": writes back what is missing or changed of the installed
    /// version, with the options it was installed with (whatever was ticked
    /// or unticked on this page is left alone).
    pub fn repair(&mut self, cx: &mut Context<Self>) {
        if !self.is_repairable() {
            return;
        }
        self.choices = crate::start::install(&self.package.manifest).choices;
        crate::log::line("Repair was chosen: the installed version is checked and put right.");
        self.repairing = true;
        self.start(cx);
    }

    /// Starts installing or removing on a worker thread.
    pub fn start(&mut self, cx: &mut Context<Self>) {
        if matches!(self.stage, Stage::Working { .. }) {
            return;
        }
        let (sender, mut events) = mpsc::unbounded::<Event>();
        self.cancelling = false;
        let texts = self.texts.clone();
        let repairing = std::mem::take(&mut self.repairing);
        let job = match (&self.mode, self.preview) {
            (_, true) => Job::Pretend {
                removing: self.is_removing(),
                online: self.package.manifest.online,
                tasks: self
                    .package
                    .manifest
                    .run
                    .iter()
                    .filter(|step| (step.when == RunWhen::Uninstall) == self.is_removing())
                    .map(|step| step.name.clone())
                    .collect(),
            },
            (Mode::Install, false) => {
                let plan = InstallPlan {
                    package: self.package.clone(),
                    texts: texts.clone(),
                    dir: self.dir.clone(),
                    choices: self.choices.clone(),
                    key: (!self.key.is_empty()).then(|| self.key.clone()),
                };
                // The front of a .msi hands the work to Windows Installer.
                if crate::system::msi::link().is_some() {
                    Job::InstallWithMsi(plan, repairing)
                } else {
                    Job::Install(plan)
                }
            }
            (Mode::Remove { .. }, false) => Job::Remove(RemovePlan {
                package: self.package.clone(),
                texts: texts.clone(),
                dir: self.dir.clone(),
                delete_data: self.delete_data,
            }),
        };
        std::thread::spawn(move || {
            let last = std::cell::RefCell::new(String::new());
            let report = |progress: f32, label: &str| {
                if *last.borrow() != label {
                    crate::log::line(label);
                    *last.borrow_mut() = label.to_string();
                }
                sender.unbounded_send(Event::Progress(progress, label.to_string())).ok();
            };
            // A panic on the worker becomes a failed page, never a crash.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &job {
                Job::Install(plan) => work::install(plan, &report),
                Job::InstallWithMsi(plan, repair) => work::msi::install(plan, *repair, &report),
                Job::Remove(plan) if crate::system::msi::link().is_some() => work::msi::remove(plan, &report),
                Job::Remove(plan) => work::remove(plan, &report),
                Job::Pretend { removing, online, tasks } => work::pretend(&texts, *removing, *online, tasks, &report),
            }))
            .unwrap_or_else(|_| Err(texts.get(Key::error_damaged)));
            match &result {
                Ok(()) => crate::log::line("Finished."),
                Err(message) => crate::log::line(&format!("Failed: {message}")),
            }
            sender.unbounded_send(Event::Finished(result)).ok();
        });

        self.attempt = self.attempt.saturating_add(1);
        self.focused = None;
        self.stage = Stage::Working { progress: 0.0, label: self.texts.get(Key::step_preparing).into() };
        cx.notify();
        self.job = Some(cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                match event {
                    Event::Progress(progress, label) => {
                        let alive = this.update(cx, |this, cx| {
                            let shown = never_back(&this.stage, progress);
                            this.stage = Stage::Working { progress: shown, label: label.into() };
                            cx.notify();
                        });
                        if alive.is_err() {
                            return;
                        }
                    }
                    Event::Finished(result) => {
                        if result.is_ok() {
                            cx.background_executor().timer(FINISH_HOLD).await;
                        }
                        this.update(cx, |this, cx| {
                            // A cancelled install leaves nothing behind: the window just ends.
                            if matches!(&result, Err(message) if message == work::cancel::CANCELLED) {
                                cx.quit();
                                return;
                            }
                            this.stage = match result {
                                Ok(()) => Stage::Done,
                                // A key the files don't open (a kept one that a
                                // newer build changed, say) is asked for again.
                                Err(message) if message == this.texts.get(Key::access_wrong) => {
                                    this.key.clear();
                                    this.access_pending = true;
                                    this.key_error = Some(message.into());
                                    Stage::Access
                                }
                                Err(message) => Stage::Failed(message.into()),
                            };
                            this.focused = None;
                            this.job = None;
                            cx.notify();
                        })
                        .ok();
                        return;
                    }
                }
            }
        }));
    }

    /// Starts the installed app and closes setup.
    pub fn open_app(&mut self, cx: &mut Context<Self>) {
        if self.preview {
            cx.quit();
            return;
        }
        let exe = self.package.manifest.executable_in(&self.dir);
        match std::process::Command::new(&exe).current_dir(&self.dir).spawn() {
            Ok(_) => cx.quit(),
            Err(err) => {
                let message = self.texts.with(Key::error_open_failed, &[("error", &err.to_string())]);
                self.stage = Stage::Failed(message.into());
                cx.notify();
            }
        }
    }

    /// Closing is refused while work runs: stopping halfway is what the
    /// all-or-nothing install avoids.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if can_close(&self.stage) {
            cx.quit();
        } else {
            // While files are only being unpacked, closing means cancelling.
            self.cancel_install(cx);
        }
    }

    /// Whether the page offers "Cancel": while an install unpacks its files,
    /// which is the part that can be undone without a trace.
    pub fn can_cancel(&self) -> bool {
        match &self.stage {
            Stage::Working { label, .. } => {
                !self.preview
                    && !self.is_removing()
                    && (label.as_ref() == self.texts.get(Key::step_copying).as_str()
                        // A component's download is cancelled just as safely: nothing is written yet.
                        || work::is_downloading(&self.texts, &self.package.manifest, label.as_ref()))
            }
            _ => false,
        }
    }

    /// Stops the install. The worker undoes what it unpacked and the window ends.
    pub fn cancel_install(&mut self, cx: &mut Context<Self>) {
        if self.can_cancel() && !self.cancelling {
            self.cancelling = true;
            crate::log::line("Cancel was pressed; stopping before anything is moved into place.");
            work::cancel::request();
            cx.notify();
        }
    }
}

/// Whether the window may be closed on this page.
pub fn can_close(stage: &Stage) -> bool {
    !matches!(stage, Stage::Working { .. })
}

enum Job {
    Install(InstallPlan),
    /// The same, done by Windows Installer: a `.msi` package's window (the flag says repair).
    InstallWithMsi(InstallPlan, bool),
    Remove(RemovePlan),
    Pretend {
        removing: bool,
        online: bool,
        tasks: Vec<String>,
    },
}

/// A first install shows the license; an update only when it changed.
fn needs_license(manifest: &Manifest, previous: Option<&Manifest>, mode: &Mode, texts: &Texts) -> bool {
    let code = texts.language.code;
    let Some(license) = manifest.license(code) else {
        return false;
    };
    if matches!(mode, Mode::Remove { .. }) {
        return false;
    }
    previous.and_then(|previous| previous.license(code)) != Some(license)
}

/// The bar only moves forward: a step that reports less than an earlier one
/// changes the label, never takes the bar back.
fn never_back(stage: &Stage, progress: f32) -> f32 {
    match stage {
        Stage::Working { progress: before, .. } => progress.max(*before),
        _ => progress,
    }
}

/// Installing into "D:\Apps" means "D:\Apps\Hello"; "D:\Apps\Hello" stays.
fn install_folder(picked: PathBuf, folder_name: &str) -> PathBuf {
    let named_already = picked.file_name().is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(folder_name));
    if named_already {
        picked
    } else {
        picked.join(folder_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_closes_everywhere_except_while_files_move() {
        let pages = [Stage::Requirement(0), Stage::Access, Stage::License, Stage::Ready, Stage::Done, Stage::Failed("x".into())];
        assert!(pages.iter().all(can_close), "every page but one may be closed");
        assert!(
            !can_close(&Stage::Working { progress: 0.3, label: "Copying".into() }),
            "stopping halfway is what the all-or-nothing install avoids"
        );
    }

    #[test]
    fn the_progress_bar_never_runs_backwards() {
        let at = |progress: f32| Stage::Working { progress, label: "Registering".into() };
        assert_eq!(never_back(&at(0.97), 0.86), 0.97);
        assert_eq!(never_back(&at(0.5), 0.6), 0.6);
        assert_eq!(never_back(&Stage::Ready, 0.02), 0.02, "a new run starts from its own value");
    }

    #[test]
    fn picked_folders_get_the_app_folder() {
        assert_eq!(install_folder(PathBuf::from(r"D:\Apps"), "Hello"), PathBuf::from(r"D:\Apps\Hello"));
        assert_eq!(install_folder(PathBuf::from(r"D:\Apps\hello"), "Hello"), PathBuf::from(r"D:\Apps\hello"));
    }
}
