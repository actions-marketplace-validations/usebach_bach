//! The window's smaller actions: requirements, links, and closing the app
//! so an update can go on. The install itself lives in `view.rs`.

use bach_pack::i18n::Key;
use gpui::Context;

use crate::system::apps;
use crate::view::{SetupView, Stage};

impl SetupView {
    /// Looks again after the user installed what was missing, and moves on
    /// once nothing is.
    pub fn recheck_requirements(&mut self, cx: &mut Context<Self>) {
        let missing = crate::start::missing_requirements(&self.package.manifest).first().copied();
        self.stage = match missing {
            Some(index) => Stage::Requirement(index),
            None => self.first_page(),
        };
        self.focused = None;
        cx.notify();
    }

    /// "Continue anyway": the requirement may be there although setup didn't
    /// find it (an unusual install). Setup goes on to the first page; the app
    /// may not start without it, and the log says what was passed over.
    pub fn skip_requirements(&mut self, cx: &mut Context<Self>) {
        let missing: Vec<String> = crate::start::missing_requirements(&self.package.manifest)
            .iter()
            .filter_map(|index| self.package.manifest.requirements.get(*index))
            .map(|requirement| requirement.name.clone())
            .collect();
        crate::log::line(&format!("The user went on without: {}.", missing.join(", ")));
        self.stage = self.first_page();
        self.focused = None;
        cx.notify();
    }

    /// Opens the download page of the requirement on screen.
    pub fn get_requirement(&mut self) {
        if let Stage::Requirement(index) = self.stage {
            if let Some(requirement) = self.package.manifest.requirements.get(index) {
                apps::open_url(&requirement.url);
            }
        }
    }

    pub fn open_website(&mut self) {
        if let Some(url) = &self.package.manifest.app.website {
            apps::open_url(url);
        }
    }

    pub fn open_feedback(&mut self) {
        if let Some(url) = &self.package.manifest.uninstall.feedback_url {
            apps::open_url(url);
        }
    }

    /// Whether the last attempt failed because the app is open, so the
    /// failed page can offer to close it.
    pub fn app_is_open(&self) -> bool {
        !self.is_removing() && matches!(&self.stage, Stage::Failed(message) if message.as_ref() == self.texts.get(Key::error_still_open))
    }

    /// Writes the support info (see `support.rs`) to the desktop and shows
    /// it in Explorer. The user decides whether it goes anywhere.
    pub fn save_support(&mut self, cx: &mut Context<Self>) {
        let message = match &self.stage {
            Stage::Failed(message) => message.to_string(),
            _ => String::new(),
        };
        let log = crate::log::path().and_then(|path| std::fs::read_to_string(path).ok()).unwrap_or_default();
        let manifest = &self.package.manifest;
        let facts = crate::support::Facts {
            manifest,
            message: &message,
            windows_build: apps::windows_build(),
            elevated: crate::system::elevate::is_elevated(),
            install_dir: &self.dir,
            free_bytes: crate::system::host::free_space(&self.dir),
            log: &log,
        };
        let saved = if self.preview {
            // A preview changes nothing on this computer.
            Ok(None)
        } else {
            crate::system::user_desktop()
                .map_err(|err| std::io::Error::other(err.to_string()))
                .and_then(|folder| crate::support::save(&folder, &facts, &crate::support::Hide::from_environment()))
                .map(Some)
        };
        match saved {
            Ok(path) => {
                if let Some(path) = path {
                    crate::log::line(&format!("Saved the support info to {}", path.display()));
                    // Show the file; Explorer takes `/select,path` as one argument.
                    let _ = std::process::Command::new("explorer.exe").arg(format!("/select,{}", path.display())).spawn();
                }
                self.support = crate::view::SupportState::Saved;
            }
            Err(err) => {
                crate::log::line(&format!("Couldn't save the support info: {err}"));
                self.support = crate::view::SupportState::Failed;
            }
        }
        cx.notify();
    }

    /// Asks the app to close (it may ask the user to save first), then
    /// tries again. Nothing is ever killed.
    pub fn close_app_and_retry(&mut self, cx: &mut Context<Self>) {
        if self.closing_app || self.preview {
            return;
        }
        self.closing_app = true;
        cx.notify();
        let program = self.package.manifest.executable_in(&self.dir);
        let closing = cx.background_executor().spawn(async move { apps::close_programs_using(&[program]) });
        self.closer = Some(cx.spawn(async move |this, cx| {
            let closed = closing.await;
            this.update(cx, |this, cx| {
                this.closing_app = false;
                this.closer = None;
                if closed {
                    crate::log::line("The app closed; trying again.");
                    this.start(cx);
                } else {
                    crate::log::line("The app didn't close.");
                    this.stage = Stage::Failed(this.texts.get(Key::error_close_app).into());
                    cx.notify();
                }
            })
            .ok();
        }));
    }
}
