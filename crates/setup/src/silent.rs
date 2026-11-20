//! `--silent`: install, update or remove without a window, for IT tools
//! (`Setup.exe /S`), scripts and Settings › Apps' quiet uninstall.
//!
//! Exit codes: 0 done, 1 failed (the reason is printed when started from a
//! terminal), 2 a command-line mistake.

use std::cell::RefCell;
use std::sync::Arc;

use bach_pack::i18n::Key;
use bach_pack::manifest::{choices_auto_update, OPTION_AUTOSTART, OPTION_AUTO_UPDATE, OPTION_DESKTOP, OPTION_PATH, OPTION_START_MENU};

use crate::launch::{self, exit, Args, Mode};
use crate::package::Package;
use crate::system::host;
use crate::texts::Texts;
use crate::{start, work};

pub fn run(args: &Args, package: Package) -> i32 {
    host::attach_parent_console();
    let package = Arc::new(package);
    let manifest = &package.manifest;
    let texts = Arc::new(Texts::new(manifest, args.lang.as_deref()));
    let last = RefCell::new(String::new());
    let report = |_: f32, label: &str| {
        if *last.borrow() != label {
            eprintln!("{label}");
            crate::log::line(label);
            *last.borrow_mut() = label.to_string();
        }
    };

    let mut failed = Key::remove_failed;
    let result = match launch::mode(args, &package) {
        Mode::Install => {
            let start = start::install(manifest);
            if args.repair {
                if let Some(problem) = start::repair_problem(manifest, &start) {
                    eprintln!("{problem}");
                    crate::log::line(&format!("Repair refused: {problem}"));
                    return exit::FAILED;
                }
                crate::log::line("Repairing the installed version.");
            }
            let missing = start::missing_requirements(manifest);
            if args.skip_requirements && !missing.is_empty() {
                let names: Vec<&str> = missing.iter().map(|index| manifest.requirements[*index].name.as_str()).collect();
                eprintln!(
                    "Not detected, but --skip-requirements was given, so setup goes on: {}. {} may not start without it.",
                    names.join(", "),
                    manifest.app.name
                );
                crate::log::line(&format!("Requirements skipped on request: {}", names.join(", ")));
            }
            if let Some(index) = missing.first().filter(|_| !args.skip_requirements) {
                let requirement = &manifest.requirements[*index];
                let values = [("requirement", requirement.name.as_str())];
                eprintln!("{} {}", texts.with(Key::requirement_subtitle, &values), requirement.url);
                crate::log::line(&format!("Missing requirement: {}", requirement.name));
                return exit::FAILED;
            }
            if start.downgrade && !args.allow_downgrade {
                let installed = start.existing.as_ref().map(|installed| installed.version.as_str()).unwrap_or("?");
                eprintln!(
                    "{} Add --allow-downgrade to install it anyway.",
                    texts.with(Key::downgrade_subtitle, &[("old_version", installed)])
                );
                return exit::FAILED;
            }
            failed = if start.existing.is_some() { Key::update_failed } else { Key::install_failed };
            let mut choices = start.choices;
            for (ids, on) in [(&args.with, true), (&args.without, false)] {
                for id in ids {
                    if !choices.set(manifest, id, on) {
                        eprintln!("{} isn't an option this installer offers. It offers: {}.", id, offered(&package).join(", "));
                        return exit::USAGE;
                    }
                }
            }
            let dir = match (&args.dir, start.existing.is_some()) {
                (Some(dir), false) if manifest.install.change_folder => dir.clone(),
                (Some(_), false) => {
                    eprintln!("{} always installs to its own folder; --dir isn't allowed.", manifest.app.name);
                    return exit::USAGE;
                }
                // An update stays where it is.
                _ => start.dir,
            };
            if !dir.is_absolute() {
                eprintln!("--dir must be a full path, like C:\\Apps\\{}.", manifest.install.folder);
                return exit::USAGE;
            }
            let key = match args.access_key() {
                Ok(key) => key,
                Err(problem) => {
                    eprintln!("{problem}");
                    return exit::USAGE;
                }
            };
            if manifest.access.is_some() && key.is_none() {
                eprintln!(
                    "{} asks for an access key. Give it in a file with --key-file FILE, or in the {} environment variable.",
                    manifest.app.name,
                    launch::KEY_ENV
                );
                return exit::USAGE;
            }
            if start.existing.is_none() && !args.overwrite {
                let foreign = work::foreign_files(manifest, &choices, &dir);
                if !foreign.is_empty() {
                    let files = work::describe_files(&foreign);
                    eprintln!(
                        "{} {} Add --overwrite to install anyway.",
                        texts.get(Key::overwrite_title),
                        texts.with(Key::overwrite_subtitle, &[("files", files.as_str())])
                    );
                    crate::log::line(&format!("Refused: {} existing file(s) would be replaced.", foreign.len()));
                    return exit::FAILED;
                }
            }
            let plan = work::InstallPlan { package: package.clone(), texts: texts.clone(), dir, choices, key };
            if crate::system::msi::link().is_some() {
                work::msi::install(&plan, args.repair, &report)
            } else {
                work::install(&plan, &report)
            }
        }
        Mode::Remove { dir } => {
            let plan = work::RemovePlan { package: package.clone(), texts: texts.clone(), dir, delete_data: args.delete_data };
            if crate::system::msi::link().is_some() {
                work::msi::remove(&plan, &report)
            } else {
                work::remove(&plan, &report)
            }
        }
    };
    match result {
        Ok(()) => exit::OK,
        Err(message) => {
            let title = texts.get(failed);
            eprintln!("{title}: {message}");
            crate::log::line(&format!("Failed: {message}"));
            exit::FAILED
        }
    }
}

/// The option ids `--with` and `--without` accept for this installer.
fn offered(package: &Package) -> Vec<String> {
    let install = &package.manifest.install;
    let builtin = [
        (OPTION_DESKTOP, install.desktop_shortcut),
        (OPTION_START_MENU, install.start_menu_shortcut),
        (OPTION_AUTOSTART, install.start_with_windows),
        (OPTION_PATH, install.add_to_path),
        (OPTION_AUTO_UPDATE, choices_auto_update(&package.manifest)),
    ];
    let mut ids: Vec<String> = builtin.iter().filter(|(_, choice)| choice.offered()).map(|(id, _)| id.to_string()).collect();
    ids.extend(package.manifest.components.iter().map(|component| component.id.clone()));
    if ids.is_empty() {
        ids.push("none".into());
    }
    ids
}
