//! `bach check`, `bach build` and `bach preview`.

use std::fs::{self, File};
use std::io::{BufWriter, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use bach_pack::archive::{self, Compression};
use bach_pack::config::{self, Overrides, Project};
use bach_pack::container;
use bach_pack::manifest::Encryption;
use bach_pack::protect::Unlocked;
use sha2::{Digest, Sha256};

use crate::output::{self, field, Progress};
use crate::runtime;

/// Loads the config, with the version of a release tag instead of
/// `app.version` when one is given.
fn load(config_path: &Path, version: Option<&str>) -> Result<Project> {
    config::load_with(config_path, &Overrides { version: version.map(String::from) })
}

pub fn check(config_path: &Path, version: Option<&str>) -> Result<()> {
    let project = load(config_path, version)?;
    let manifest = &project.manifest;
    output::result("name", manifest.app.name.clone());
    output::result("version", manifest.app.version.clone());
    output::result("files", manifest.files.len());
    output::result("size", manifest.total_size());
    say!("{} {} is ready to build.", manifest.app.name, manifest.app.version);
    summary(&project);
    say!();
    for file in &manifest.files {
        let component = file.component.as_deref().map(|id| format!("  ({id})")).unwrap_or_default();
        say!("  {:>10}  {}{component}", output::size(file.size), file.path);
    }
    output::warnings(&project.warnings);
    Ok(())
}

/// What `bach build` was asked for.
pub struct BuildOptions<'a> {
    pub config: &'a Path,
    pub out: Option<&'a Path>,
    pub runtime: Option<&'a Path>,
    pub fast: bool,
    pub version: Option<&'a str>,
    /// Also write the online installer.
    pub online: bool,
    /// A file holding the access key, instead of the environment variable.
    pub key_file: Option<&'a Path>,
    /// Sign the setup files with `[signing]`. Never implied by the config: a
    /// `bach.toml` must not run a program just because it was built.
    pub sign: bool,
    /// An older setup file of this app: also write an update patch from it,
    /// holding only the files that changed since.
    pub patch_from: Option<&'a Path>,
}

/// The key the files are locked with, and what it unlocks while building.
struct Sealing {
    encryption: Encryption,
    unlocked: Unlocked,
}

/// Locks the project's files when `[access]` asks for it: reads the key,
/// makes fresh protection data and records it in the manifest. `key` is the
/// key to use when it isn't read from the environment (a preview's).
fn seal(project: &mut Project, key_file: Option<&Path>, key: Option<&str>) -> Result<Option<Sealing>> {
    if !project.encrypt {
        return Ok(None);
    }
    let keys = match key {
        Some(key) => vec![key.to_string()],
        None => read_keys(&project.key_env, key_file)?,
    };
    if keys.len() > 1 {
        say!("{} access keys: each one opens the setup file on its own.", keys.len());
    }
    let (encryption, unlocked) = Encryption::create_slots(&keys)?;
    if let Some(access) = &mut project.manifest.access {
        access.encryption = Some(encryption.clone());
    }
    Ok(Some(Sealing { encryption, unlocked }))
}

/// The access keys: from a key file, one per line (`bach key --count 50` makes
/// such a file; blank lines and lines starting with # are skipped), or the one
/// key in the environment variable.
fn read_keys(env: &str, key_file: Option<&Path>) -> Result<Vec<String>> {
    let key = match key_file {
        Some(file) => bach_pack::textfile::read(file).with_context(|| format!("couldn't read the key file {}", file.display()))?,
        None => std::env::var(env).map_err(|_| {
            anyhow::anyhow!(
                "bach.toml has [access], so the files are encrypted and the build needs the access key. \
                 Set the environment variable {env} to it (make one with `bach key`), or pass --key-file FILE"
            )
        })?,
    };
    let keys: Vec<String> = match key_file {
        Some(_) => key.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')).map(str::to_string).collect(),
        // An environment variable holds one key.
        None => vec![bach_pack::textfile::first_line(&key).unwrap_or_default()],
    };
    if keys.is_empty() {
        bail!("the key file has no access key in it; put one key on each line");
    }
    Ok(keys)
}

pub fn build(options: BuildOptions) -> Result<PathBuf> {
    let BuildOptions { config: config_path, out, runtime, fast, version, online, key_file, sign, patch_from } = options;
    let started = Instant::now();
    let mut project = load(config_path, version)?;
    if fast {
        project.compression = Compression::Fast;
    }
    if online {
        project.online = true;
    }
    if project.online && project.manifest.updates.is_none() {
        bail!(
            "an online installer downloads the newest version through the update feed, so bach.toml needs an [updates] section (github = \"owner/repo\" or feed = \"https://…\")"
        );
    }
    let output = out.map(Path::to_path_buf).unwrap_or_else(|| project.output.clone());
    let runtime = runtime::find(runtime)?;
    let app = &project.manifest.app;
    say!("Building {} {}", app.name, app.version);
    summary(&project);

    output::warnings(&project.warnings);

    let sealing = seal(&mut project, key_file, None)?;
    if sealing.is_some() {
        say!("The files are encrypted with the access key.");
    }
    let output = tidy(&output);
    // The checksums come out of the packing pass itself: each file is read once.
    let written = write_setup(&project, &runtime, &output, sealing.as_ref())?;
    let packed = written.packed;
    record_hashes(&mut project.manifest, &project.sources, &written.hashes)?;

    let size = fs::metadata(&output).map(|meta| meta.len()).unwrap_or(0);
    let installed = project.manifest.total_size().max(1);
    say!();
    say!("Wrote {} ({})", output.display(), output::size(size));
    output::result("setup", output.display().to_string());
    output::result("version", project.manifest.app.version.clone());
    // Signing is optional: it needs [signing] and a --sign from whoever runs the build.
    let signing = project.signing.as_ref().filter(|_| sign);
    let publisher = project.manifest.updates.as_ref().and_then(|updates| updates.signer.as_deref());
    if let Some(signing) = signing {
        crate::sign::sign(signing, &output, publisher)?;
    }
    if project.online {
        let online_output = tidy(&project.online_output);
        write_online_setup(&project, &runtime, &online_output, sealing.as_ref())?;
        let size = fs::metadata(&online_output).map(|meta| meta.len()).unwrap_or(0);
        say!("Wrote {} ({}), the online installer", online_output.display(), output::size(size));
        output::result("online_setup", online_output.display().to_string());
        if let Some(signing) = signing {
            crate::sign::sign(signing, &online_output, publisher)?;
        }
    }
    let patch = match patch_from {
        Some(old) => {
            let (patch, from) = write_patch(&project, old, &runtime, &output, key_file)?;
            if let Some(signing) = signing {
                crate::sign::sign(signing, &patch, publisher)?;
            }
            output::result("patch", patch.display().to_string());
            Some((patch, from))
        }
        None => None,
    };
    if project.signing.is_some() && !sign {
        say!("Not signed: bach.toml has [signing], but signing runs only with --sign (or BACH_SIGN=1).");
    } else if sign && project.signing.is_none() {
        bail!("--sign was given, but bach.toml has no [signing] section that says how to sign");
    }
    if project.manifest.updates.is_some() {
        crate::feed::after_build(&project, &output, patch.as_ref().map(|(file, from)| (file.as_path(), from.as_str())))?;
    }
    if output::json() {
        // Of the file as it is now: signed, if it was signed.
        let (sha256, final_size) = crate::feed::digest_of(&output)?;
        output::result("sha256", sha256);
        output::result("size", final_size);
        output::result("packed", packed);
    }
    field(
        "Packed",
        format!(
            "{} of {} ({:.0}%), in {:.1}s",
            output::size(packed),
            output::size(installed),
            packed as f64 / installed as f64 * 100.0,
            started.elapsed().as_secs_f32()
        ),
    );
    Ok(output)
}

/// What writing a setup file gives back: the archive's packed size and the
/// SHA-256 of each packaged file (in the order of `project.sources`).
struct Written {
    packed: u64,
    hashes: Vec<String>,
}

/// Records every file's SHA-256 in the manifest, so an update patch can leave
/// unchanged files where they are and check them before relying on them.
/// `hashes` belong to `sources`, one each; a file that isn't packaged keeps
/// the checksum it has.
fn record_hashes(manifest: &mut bach_pack::manifest::Manifest, sources: &[archive::Source], hashes: &[String]) -> Result<()> {
    if sources.len() != hashes.len() {
        bail!("the packing pass gave {} checksums for {} files", hashes.len(), sources.len());
    }
    for (source, hash) in sources.iter().zip(hashes) {
        let Some(entry) = manifest.files.iter_mut().find(|entry| entry.path == source.packaged) else {
            bail!("the file list and the files to pack disagree about {}", source.packaged);
        };
        entry.sha256 = Some(hash.clone());
    }
    Ok(())
}

/// Writes the update patch from `old` (an earlier setup file of this app) to
/// this build: a setup file that carries only the files that changed. It sits
/// next to `output`. Gives its path and the version it updates from.
fn write_patch(project: &Project, old: &Path, runtime: &Path, output: &Path, key_file: Option<&Path>) -> Result<(PathBuf, String)> {
    let mut file = File::open(old).with_context(|| format!("couldn't open {}", old.display()))?;
    let Some(layout) = container::locate(&mut file)? else {
        bail!("--patch-from {} isn't a Bach setup file", old.display());
    };
    let earlier = container::read_manifest(&mut file, &layout)?;
    if earlier.app.id != project.manifest.app.id {
        bail!("--patch-from {} is a setup file for \"{}\", not for \"{}\"", old.display(), earlier.app.id, project.manifest.app.id);
    }
    if earlier.files.iter().any(|entry| entry.sha256.is_none()) {
        bail!(
            "--patch-from {} was built before Bach recorded checksums; build the patch from a setup file made with Bach {} or newer",
            old.display(),
            bach_pack::VERSION
        );
    }
    let same_as_before = |path: &str, sha: Option<&String>, size: u64| {
        earlier.files.iter().any(|entry| entry.path.eq_ignore_ascii_case(path) && entry.sha256.as_ref() == sha && entry.size == size)
    };
    let mut patch = project.clone();
    patch.sources.retain(|source| {
        let entry = project.manifest.files.iter().find(|entry| entry.path == source.packaged);
        !entry.is_some_and(|entry| same_as_before(&entry.path, entry.sha256.as_ref(), entry.size))
    });
    patch.manifest.patch_from = Some(earlier.app.version.clone());
    let kept = patch.sources.len();
    let name = output.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let from = bach_pack::config::github_asset_name(&earlier.app.version);
    let target = output.with_file_name(format!("{name}-patch-from-{from}.exe"));
    let target = tidy(&target);
    // Its own protection data: two files never share a key stream.
    let sealing = seal(&mut patch, key_file, None)?;
    write_setup(&patch, runtime, &target, sealing.as_ref())?;
    let size = fs::metadata(&target).map(|meta| meta.len()).unwrap_or(0);
    say!(
        "Wrote {} ({}), the update patch from {}: {kept} of {} files changed",
        target.display(),
        output::size(size),
        earlier.app.version,
        project.sources.len()
    );
    Ok((target, earlier.app.version))
}

pub struct PreviewOptions<'a> {
    pub uninstall: bool,
    pub theme: Option<&'a str>,
    pub lang: Option<&'a str>,
    pub runtime: Option<&'a Path>,
}

pub fn preview(config_path: &Path, options: PreviewOptions) -> Result<()> {
    let PreviewOptions { uninstall, theme, lang, runtime } = options;
    let mut project = config::load(config_path)?;
    project.compression = Compression::Fast;
    // A preview opens the key page too; any key passes there.
    let sealing = seal(&mut project, None, Some("preview-only-key"))?;
    if let Some(theme) = theme {
        if bach_pack::manifest::Theme::parse(theme).is_none() {
            bail!("--theme is \"{theme}\"; use system, light or dark");
        }
    }
    if let Some(lang) = lang {
        if bach_pack::i18n::language(lang).is_none() {
            let known: Vec<&str> = bach_pack::i18n::LANGUAGES.iter().map(|language| language.code).collect();
            bail!("--lang is \"{lang}\"; use one of {}", known.join(", "));
        }
    }
    if !cfg!(windows) {
        bail!("bach preview opens a Windows window, so it needs Windows; `bach check` works everywhere");
    }
    output::warnings(&project.warnings);
    let runtime = runtime::find(runtime)?;
    let dir = std::env::temp_dir().join("bach-preview");
    fs::create_dir_all(&dir)?;
    // Earlier previews go, except the ones still open; each preview gets a
    // file of its own, so an open window never blocks the next one.
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let _ = fs::remove_file(entry.path());
        }
    }
    let output =
        dir.join(format!("{}-Setup-preview-{}.exe", bach_pack::paths::safe_dir_name(&project.manifest.app.id), std::process::id()));
    write_setup(&project, &runtime, &output, sealing.as_ref())?;

    let mut command = std::process::Command::new(&output);
    command.arg("--preview");
    if uninstall {
        command.arg("--uninstall");
    }
    if let Some(theme) = theme {
        command.args(["--theme", theme]);
    }
    if let Some(lang) = lang {
        command.args(["--lang", lang]);
    }
    command.spawn().with_context(|| format!("couldn't start {}", output.display()))?;
    say!("Opened the {} preview. Nothing is installed or removed.", if uninstall { "uninstaller" } else { "installer" });
    Ok(())
}

fn summary(project: &Project) {
    crate::inspect::details(&project.manifest, project.encrypt);
}

/// `examples/hello/../../dist/x.exe` becomes `dist/x.exe`, without touching
/// the disk (the file may not exist yet).
fn tidy(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir if matches!(out.components().next_back(), Some(Component::Normal(_))) => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

/// Writes the setup file through a `.partial` file, so a failed build never
/// leaves a half-written installer where the real one belongs. Returns the
/// archive's packed size.
fn write_setup(project: &Project, runtime_path: &Path, output: &Path, sealing: Option<&Sealing>) -> Result<Written> {
    write_file(project, runtime_path, output, sealing, write_into)
}

/// The online installer: the same window and manifest as the setup file,
/// and no files. It downloads the newest setup file when it runs.
fn write_online_setup(project: &Project, runtime_path: &Path, output: &Path, sealing: Option<&Sealing>) -> Result<Written> {
    write_file(project, runtime_path, output, sealing, write_online_into)
}

type WriteSetup = fn(&Project, &[u8], &Path, Option<&Sealing>) -> Result<Written>;

fn write_file(project: &Project, runtime_path: &Path, output: &Path, sealing: Option<&Sealing>, write: WriteSetup) -> Result<Written> {
    let runtime = runtime::read(runtime_path)?;
    if let Some(dir) = output.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        fs::create_dir_all(dir).with_context(|| format!("couldn't create {}", dir.display()))?;
    }
    let mut partial = output.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);

    let written = match write(project, &runtime, &partial, sealing) {
        Ok(written) => written,
        Err(err) => {
            let _ = fs::remove_file(&partial);
            return Err(err);
        }
    };
    if output.exists() {
        fs::remove_file(output).with_context(|| format!("couldn't replace {} (is it open?)", output.display()))?;
    }
    fs::rename(&partial, output).with_context(|| format!("couldn't move the setup file to {}", output.display()))?;
    Ok(written)
}

fn write_into(project: &Project, runtime: &[u8], path: &Path, sealing: Option<&Sealing>) -> Result<Written> {
    fs::write(path, runtime).with_context(|| format!("couldn't write {}", path.display()))?;
    brand(project, path)?;

    let mut file = File::options().read(true).write(true).open(path)?;
    let archive_offset = file.seek(SeekFrom::End(0))?;
    let total = project.sources.iter().map(|source| source.size).sum();
    let mut progress = Progress::new("Packing", total);
    let hashing = Hashing { inner: BufWriter::new(file), hasher: Sha256::new() };
    let mut step = |done| progress.update(done);
    // How many threads compress: as many as the memory budget allows, never
    // more than there are cores, unless the config says how many. The setup
    // file is the same either way.
    let plan = project.compression.plan(project.compression_threads, project.compression_memory_mib);
    if plan.limited_by_memory {
        say!(
            "Compressing on {} threads (about {} MB of memory; installer.compression_memory is {} MB)",
            plan.threads,
            plan.estimated_mib,
            project.compression_memory_mib
        );
    }
    let (hashing, hashes) = match sealing {
        // The hash covers what is appended, which is the encrypted bytes.
        Some(sealing) => {
            let writer = sealing.encryption.writer(hashing, &sealing.unlocked)?;
            let (writer, hashes) = archive::write_hashed(writer, &project.sources, project.compression, plan.threads, &mut step)?;
            (writer.finish()?, hashes)
        }
        None => archive::write_hashed(hashing, &project.sources, project.compression, plan.threads, &mut step)?,
    };
    progress.finish();
    let Hashing { inner, hasher } = hashing;
    let mut file = inner.into_inner().map_err(|err| err.into_error())?;
    let logo = project.logo.as_deref().unwrap_or_default();
    // The runtime checks this before unpacking anything.
    let mut manifest = project.manifest.clone();
    manifest.archive_sha256 = Some(bach_pack::protect::hex(&hasher.finalize()));
    // The file checksums were taken while packing; the manifest goes in after the archive.
    record_hashes(&mut manifest, &project.sources, &hashes)?;
    let layout = container::finish(&mut file, archive_offset, logo, &manifest)?;
    file.sync_all()?;
    Ok(Written { packed: layout.archive.len, hashes })
}

fn write_online_into(project: &Project, runtime: &[u8], path: &Path, _sealing: Option<&Sealing>) -> Result<Written> {
    fs::write(path, runtime).with_context(|| format!("couldn't write {}", path.display()))?;
    brand(project, path)?;
    let mut file = File::options().read(true).write(true).open(path)?;
    let offset = file.seek(SeekFrom::End(0))?;
    let mut manifest = project.manifest.clone();
    manifest.online = true;
    manifest.archive_sha256 = None;
    let logo = project.logo.as_deref().unwrap_or_default();
    container::finish(&mut file, offset, logo, &manifest)?;
    file.sync_all()?;
    Ok(Written { packed: 0, hashes: Vec::new() })
}

/// A writer that hashes what passes through it.
struct Hashing<W> {
    inner: W,
    hasher: Sha256,
}

impl<W: std::io::Write> std::io::Write for Hashing<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(bytes)?;
        self.hasher.update(&bytes[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Gives the setup file the app's icon and version details, so Explorer,
/// the taskbar and the file's Properties name the app, not Bach.
#[cfg(windows)]
fn brand(project: &Project, path: &Path) -> Result<()> {
    let icon = match &project.icon {
        Some(icon) => Some(fs::read(icon).with_context(|| format!("couldn't read {}", icon.display()))?),
        None => None,
    };
    let file_name = path.file_name().map(|name| name.to_string_lossy().trim_end_matches(".partial").to_string()).unwrap_or_default();
    crate::resources::apply(path, icon.as_deref(), &project.manifest.app, &file_name)
}

#[cfg(not(windows))]
fn brand(project: &Project, _path: &Path) -> Result<()> {
    eprintln!(
        "  note: the setup file keeps Bach's icon and version details{}; \
         build on Windows to brand it",
        if project.icon.is_some() { " (app.icon is skipped)" } else { "" }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_paths_are_tidied() {
        let tidied = tidy(Path::new("examples/hello/../../dist/x.exe"));
        assert_eq!(tidied, Path::new("dist").join("x.exe"));
        assert_eq!(tidy(Path::new("../out/x.exe")), Path::new("..").join("out").join("x.exe"));
    }
}
