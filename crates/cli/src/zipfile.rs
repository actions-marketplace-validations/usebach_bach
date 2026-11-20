//! The ZIP a `kind = "zip"` profile writes: the app's files, ready to unpack
//! and run without an installer.
//!
//! The ZIP is reproducible: files go in the order of the manifest and every
//! entry carries the same fixed date, so the same files give the same ZIP
//! whenever and wherever it is built.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use bach_pack::archive::Source;
use bach_pack::config::Project;
use bach_pack::manifest::Choices;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

use crate::output::Progress;

/// What was written.
pub struct Written {
    pub files: usize,
    /// The files' size before compressing.
    pub bytes: u64,
}

/// The files a ZIP profile holds: those of the components ticked by default,
/// like a first install, in the manifest's order.
pub fn sources_of(project: &Project) -> Vec<&Source> {
    let choices = Choices::defaults(&project.manifest);
    let chosen: std::collections::HashSet<&str> =
        project.manifest.files.iter().filter(|file| choices.installs(file)).map(|file| file.path.as_str()).collect();
    project.sources.iter().filter(|source| chosen.contains(source.packaged.as_str())).collect()
}

/// Writes the ZIP through a `.partial` file, so a failed build never leaves a
/// half-written ZIP where the real one belongs.
pub fn write(project: &Project, output: &Path) -> Result<Written> {
    if let Some(dir) = output.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("couldn't create {}", dir.display()))?;
    }
    let mut partial = output.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    let result = write_to(project, &partial);
    match result {
        Ok(written) => {
            if output.exists() {
                std::fs::remove_file(output).with_context(|| format!("couldn't replace {} (is it open?)", output.display()))?;
            }
            std::fs::rename(&partial, output).with_context(|| format!("couldn't move the ZIP to {}", output.display()))?;
            Ok(written)
        }
        Err(err) => {
            let _ = std::fs::remove_file(&partial);
            Err(err)
        }
    }
}

fn write_to(project: &Project, path: &Path) -> Result<Written> {
    let sources = sources_of(project);
    if sources.is_empty() {
        bail!("this profile has no files to put in a ZIP");
    }
    let total: u64 = sources.iter().map(|source| source.size).sum();
    let file = File::create(path).with_context(|| format!("couldn't write {}", path.display()))?;
    let mut zip = ZipWriter::new(BufWriter::new(file));
    let mut progress = Progress::new("Zipping", total);
    let mut done = 0u64;
    for source in &sources {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .last_modified_time(DateTime::default())
            .unix_permissions(0o644)
            // The ZIP format's classic sizes stop at 4 GiB.
            .large_file(source.size >= u64::from(u32::MAX));
        zip.start_file(source.packaged.as_str(), options).with_context(|| format!("couldn't add {} to the ZIP", source.packaged))?;
        let mut input = File::open(&source.path).with_context(|| format!("couldn't read {}", source.path.display()))?;
        let copied = std::io::copy(&mut input, &mut zip).with_context(|| format!("couldn't add {} to the ZIP", source.packaged))?;
        if copied != source.size {
            bail!("{} changed while it was being packed ({} bytes were expected, {copied} were read)", source.packaged, source.size);
        }
        done += copied;
        progress.update(done);
    }
    progress.finish();
    let mut writer = zip.finish().context("couldn't finish the ZIP")?;
    writer.flush()?;
    Ok(Written { files: sources.len(), bytes: total })
}
