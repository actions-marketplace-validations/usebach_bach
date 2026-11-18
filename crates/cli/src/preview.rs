//! `bach preview`: the real installer window, with nothing installed. It is
//! also what `bach watch` reopens each time the config changes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Child;

use anyhow::{bail, Context, Result};
use bach_pack::archive::Compression;
use bach_pack::config::{self, Project};

use crate::builder::{seal, write_setup, Sealing};
use crate::{output, runtime};

pub struct PreviewOptions<'a> {
    pub uninstall: bool,
    pub theme: Option<&'a str>,
    /// A built-in style to show instead of the config's.
    pub style: Option<&'a str>,
    pub lang: Option<&'a str>,
    pub runtime: Option<&'a Path>,
}

/// The config as the preview wants it: quick to compress, and sealed with a
/// made-up key, because a preview opens the key page too and any key passes there.
pub(crate) fn load(config_path: &Path) -> Result<(Project, Option<Sealing>)> {
    let mut project = config::load(config_path)?;
    project.compression = Compression::Fast;
    let sealing = seal(&mut project, None, Some("preview-only-key"))?;
    Ok((project, sealing))
}

/// Refuses what can't be previewed: a theme, style or language that doesn't
/// exist, and any computer that isn't running Windows.
pub(crate) fn check(options: &PreviewOptions, command: &str) -> Result<()> {
    if let Some(theme) = options.theme {
        if bach_pack::manifest::Theme::parse(theme).is_none() {
            bail!("--theme is \"{theme}\"; use system, light or dark");
        }
    }
    if let Some(style) = options.style {
        if bach_pack::style::preset(style).is_none() {
            let names: Vec<&str> = bach_pack::style::PRESETS.iter().map(|preset| preset.name).collect();
            match bach_pack::style::closest(style) {
                Some(near) => bail!("--style is \"{style}\"; did you mean \"{near}\"?"),
                None => bail!("--style is \"{style}\"; use one of {}", names.join(", ")),
            }
        }
    }
    if let Some(lang) = options.lang {
        if bach_pack::i18n::language(lang).is_none() {
            let known: Vec<&str> = bach_pack::i18n::LANGUAGES.iter().map(|language| language.code).collect();
            bail!("--lang is \"{lang}\"; use one of {}", known.join(", "));
        }
    }
    if !cfg!(windows) {
        bail!("bach {command} opens a Windows window, so it needs Windows; `bach check` works everywhere");
    }
    Ok(())
}

/// Writes the preview's setup file. Earlier ones go, except those still open;
/// each preview gets a file of its own (`serial` tells them apart), so an
/// open window never blocks the next one.
pub(crate) fn write(project: &Project, sealing: Option<&Sealing>, runtime: &Path, serial: u32) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("bach-preview");
    fs::create_dir_all(&dir)?;
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let _ = fs::remove_file(entry.path());
        }
    }
    let id = bach_pack::paths::safe_dir_name(&project.manifest.app.id);
    let output = match serial {
        0 => dir.join(format!("{id}-Setup-preview-{}.exe", std::process::id())),
        serial => dir.join(format!("{id}-Setup-preview-{}-{serial}.exe", std::process::id())),
    };
    write_setup(project, runtime, &output, sealing)?;
    Ok(output)
}

/// Opens the window and doesn't wait for it.
pub(crate) fn open(setup: &Path, options: &PreviewOptions) -> Result<Child> {
    let mut command = std::process::Command::new(setup);
    command.arg("--preview");
    if options.uninstall {
        command.arg("--uninstall");
    }
    if let Some(theme) = options.theme {
        command.args(["--theme", theme]);
    }
    if let Some(lang) = options.lang {
        command.args(["--lang", lang]);
    }
    if let Some(style) = options.style {
        command.args(["--style", style]);
    }
    command.spawn().with_context(|| format!("couldn't start {}", setup.display()))
}

pub fn preview(config_path: &Path, options: PreviewOptions) -> Result<()> {
    let (project, sealing) = load(config_path)?;
    check(&options, "preview")?;
    output::warnings(&project.warnings);
    let runtime = runtime::find(options.runtime)?;
    let setup = write(&project, sealing.as_ref(), &runtime, 0)?;
    open(&setup, &options)?;
    say!("Opened the {} preview. Nothing is installed or removed.", if options.uninstall { "uninstaller" } else { "installer" });
    Ok(())
}
