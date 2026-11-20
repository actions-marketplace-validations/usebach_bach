//! `bach check`, `bach build` and `bach preview`.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use bach_pack::archive::{self, Compression, DeltaBase};
use bach_pack::config::{self, Overrides, Project};
use bach_pack::container;
use bach_pack::manifest::Encryption;
use bach_pack::protect::Unlocked;
use sha2::{Digest, Sha256};

use crate::output::{self, field, Progress};
use crate::runtime;

/// Loads the config, with the version of a release tag instead of
/// `app.version` when one is given, and laid over with a profile when one is named.
pub(crate) fn load(config_path: &Path, version: Option<&str>, profile: Option<&str>) -> Result<Project> {
    load_arch(config_path, version, profile, None)
}

/// [`load`] for the setup file of another architecture (`arm64`), from `[arch.arm64]`.
pub(crate) fn load_arch(config_path: &Path, version: Option<&str>, profile: Option<&str>, arch: Option<&str>) -> Result<Project> {
    config::load_with(
        config_path,
        &Overrides { version: version.map(String::from), profile: profile.map(String::from), arch: arch.map(String::from) },
    )
}

/// With `--strict`, a warning is an error: the build is held back until the
/// config has none, so a pipeline can't ship what `bach check` complained about.
fn strict_gate(warnings: &[String], strict: bool) -> Result<()> {
    if strict && !warnings.is_empty() {
        let count = warnings.len();
        bail!(
            "bach.toml has {count} {}, and --strict treats {} as {}: fix {}, or run without --strict",
            if count == 1 { "warning" } else { "warnings" },
            if count == 1 { "it" } else { "them" },
            if count == 1 { "an error" } else { "errors" },
            if count == 1 { "it" } else { "them" },
        );
    }
    Ok(())
}

pub fn check(config_path: &Path, version: Option<&str>, profile: Option<&str>, strict: bool) -> Result<()> {
    let project = load(config_path, version, profile)?;
    let manifest = &project.manifest;
    output::result("name", manifest.app.name.clone());
    output::result("version", manifest.app.version.clone());
    output::result("files", manifest.files.len());
    output::result("size", manifest.total_size());
    match &project.profile {
        Some(name) => {
            output::result("profile", name.clone());
            say!("{} {} is ready to build, as the {name} profile ({}).", manifest.app.name, manifest.app.version, project.kind.name());
        }
        None => say!("{} {} is ready to build.", manifest.app.name, manifest.app.version),
    }
    summary(&project);
    say!();
    for file in &manifest.files {
        let component = file.component.as_deref().map(|id| format!("  ({id})")).unwrap_or_default();
        say!("  {:>10}  {}{component}", output::size(file.size), file.path);
    }
    output::warnings(&project.warnings);
    strict_gate(&project.warnings, strict)
}

/// What `bach build` was asked for.
pub struct BuildOptions<'a> {
    pub config: &'a Path,
    /// The ARM64 setup runtime for `[arch.arm64]` (`--runtime-arm64`), when bach.toml doesn't name one.
    pub runtime_arm64: Option<&'a Path>,
    /// Build only the main (x64) setup file, even when bach.toml has `[arch.arm64]`.
    pub x64_only: bool,
    /// Also write a Windows Installer package of the app.
    pub msi: bool,
    /// Build the setup file of this architecture (set when the main build asks for it).
    pub arch: Option<&'a str>,
    /// Build this profile of the config (`[profiles.NAME]`) instead of the main file.
    pub profile: Option<&'a str>,
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
    /// Stop when `bach check` would warn (see [`strict_gate`]).
    pub strict: bool,
}

/// The key the files are locked with, and what it unlocks while building.
pub(crate) struct Sealing {
    encryption: Encryption,
    unlocked: Unlocked,
}

/// Locks the project's files when `[access]` asks for it: reads the key,
/// makes fresh protection data and records it in the manifest. `key` is the
/// key to use when it isn't read from the environment (a preview's).
pub(crate) fn seal(project: &mut Project, key_file: Option<&Path>, key: Option<&str>) -> Result<Option<Sealing>> {
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

/// What one build wrote.
pub struct Built {
    pub path: PathBuf,
    pub profile: Option<String>,
    pub kind: config::ProfileKind,
    /// The file's size on disk.
    pub size: u64,
}

/// The main build, then each profile asked for; `all` means the main build
/// and every profile of the config. Stops at the first one that fails.
pub fn build_all(options: BuildOptions, profiles: &[String], all: bool) -> Result<()> {
    let mut wanted: Vec<Option<String>> = Vec::new();
    if all {
        wanted.push(None);
        let text = bach_pack::textfile::read(options.config).with_context(|| format!("couldn't read {}", options.config.display()))?;
        wanted.extend(config::list_profiles(&text)?.into_iter().map(|profile| Some(profile.name)));
    } else if profiles.is_empty() {
        wanted.push(options.profile.map(String::from));
    } else {
        wanted.extend(profiles.iter().map(|name| Some(name.clone())));
    }
    if wanted.len() > 1 && options.out.is_some() {
        bail!(
            "--out names one file, but this builds {} (the main build and its profiles); leave --out out and set each output in bach.toml",
            wanted.len()
        );
    }
    let mut built = Vec::new();
    for (index, profile) in wanted.iter().enumerate() {
        if index > 0 {
            say!();
        }
        let one = BuildOptions { profile: profile.as_deref(), ..options };
        built.push(build(one)?);
    }
    if built.len() > 1 || built.first().is_some_and(|first| first.profile.is_some()) {
        let list: Vec<serde_json::Value> = built
            .iter()
            .map(|one| {
                serde_json::json!({
                    "profile": one.profile,
                    "kind": one.kind.name(),
                    "file": one.path.display().to_string(),
                    "size": one.size,
                })
            })
            .collect();
        output::result("builds", list);
    }
    Ok(())
}

pub fn build(options: BuildOptions) -> Result<Built> {
    let BuildOptions {
        config: config_path,
        runtime_arm64,
        x64_only,
        msi,
        arch,
        profile,
        out,
        runtime,
        fast,
        version,
        online,
        key_file,
        sign,
        patch_from,
        strict,
    } = options;
    let started = Instant::now();
    let mut project = load_arch(config_path, version, profile, arch)?;
    if project.kind == config::ProfileKind::Zip {
        return build_zip(&project, out, strict);
    }
    if fast {
        project.compression = Compression::Fast;
    }
    // Only the main build writes the online installer, the patch and the feed.
    let is_arch = project.arch.is_some();
    if online && !is_arch {
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
    let what = match (&project.profile, &project.arch) {
        (Some(name), Some(arch)) => format!(" as the {name} profile, for {arch}"),
        (Some(name), None) => format!(" as the {name} profile"),
        (None, Some(arch)) => format!(" for {arch}"),
        (None, None) => String::new(),
    };
    say!("Building {} {}{what}", app.name, app.version);
    summary(&project);

    output::warnings(&project.warnings);
    strict_gate(&project.warnings, strict)?;

    let sealing = seal(&mut project, key_file, None)?;
    if sealing.is_some() {
        say!("The files are encrypted with the access key.");
    }
    let output = tidy(&output);
    // Components that are downloaded when ticked go into archives of their own, first: the
    // setup file records each one's SHA-256 and size, and doesn't carry the files.
    // An MSI holds every file, so it is made from the project as it was before the downloads were split off.
    let for_msi = (msi && !is_arch).then(|| project.clone());
    let packs = write_packs(&mut project, &output)?;
    // The checksums come out of the packing pass itself: each file is read once.
    let written = write_setup(&project, &runtime, &output, sealing.as_ref())?;
    let packed = written.packed;
    record_hashes(&mut project.manifest, &project.sources, &written.hashes)?;

    let size = fs::metadata(&output).map(|meta| meta.len()).unwrap_or(0);
    let installed = project.manifest.total_size().max(1);
    say!();
    say!("Wrote {} ({})", output.display(), output::size(size));
    for pack in &packs {
        say!("Wrote {} ({}), the download for the component {}", pack.path.display(), output::size(pack.size), pack.component);
    }
    if !packs.is_empty() {
        let listed: Vec<serde_json::Value> = packs
            .iter()
            .map(|pack| {
                serde_json::json!({
                    "component": pack.component,
                    "file": pack.path.display().to_string(),
                    "sha256": pack.sha256,
                    "size": pack.size,
                })
            })
            .collect();
        output::result(if is_arch { "packs_arm64" } else { "packs" }, listed);
    }
    match (&project.arch, &project.profile) {
        (Some(arch), None) => output::result(&format!("setup_{arch}"), output.display().to_string()),
        (None, None) => output::result("setup", output.display().to_string()),
        _ => {}
    }
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
    let patch = match patch_from.filter(|_| !is_arch) {
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
    if let Some(whole) = &for_msi {
        let msi_output = output.with_extension("msi");
        let plan = crate::msi::write(whole, &msi_output)?;
        let size = fs::metadata(&msi_output).map(|meta| meta.len()).unwrap_or(0);
        say!("Wrote {} ({}), the Windows Installer package", msi_output.display(), output::size(size));
        for what in &plan.skipped {
            say!("  not in the MSI: {what}");
        }
        output::result("msi", msi_output.display().to_string());
        if !plan.skipped.is_empty() {
            output::result("msi_skipped", plan.skipped.clone());
        }
    }
    // The setup files for the other architectures, before the feed that lists them.
    let mut arch_files: Vec<(String, PathBuf)> = Vec::new();
    if !is_arch && !x64_only {
        for other in &project.arches {
            let arm_runtime = arch_runtime(runtime_arm64, other)?;
            say!();
            let built = build(BuildOptions {
                arch: Some(&other.name),
                runtime: Some(&arm_runtime),
                out: None,
                online: false,
                patch_from: None,
                ..options
            })?;
            arch_files.push((other.name.clone(), built.path));
        }
    }
    // The update feed describes the main setup file; a profile's file isn't what installed apps update to.
    if project.manifest.updates.is_some() && project.profile.is_none() && !is_arch {
        crate::feed::after_build(&project, &output, patch.as_ref().map(|(file, from)| (file.as_path(), from.as_str())), &arch_files)?;
    }
    if output::json() {
        // Of the file as it is now: signed, if it was signed.
        let (sha256, final_size) = crate::feed::digest_of(&output)?;
        output::result("sha256", sha256);
        output::result("size", final_size);
        output::result("packed", packed);
    }
    let final_size = fs::metadata(&output).map(|meta| meta.len()).unwrap_or(0);
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
    Ok(Built { path: output, profile: project.profile.clone(), kind: project.kind, size: final_size })
}

/// The setup runtime of another architecture: `--runtime-arm64` (or
/// `BACH_RUNTIME_ARM64`), else the one `[arch.arm64]` names. It must be a
/// program for that architecture, because it ends up inside the setup file.
fn arch_runtime(given: Option<&Path>, arch: &config::ArchBuild) -> Result<PathBuf> {
    let Some(path) = given.map(Path::to_path_buf).or_else(|| arch.runtime.clone()) else {
        bail!(
            "bach.toml has [arch.{name}], so the build also writes the {name} setup file, and that needs the {name} setup runtime. \
             Pass --runtime-arm64 FILE (or set BACH_RUNTIME_ARM64), or set runtime in [arch.{name}]. \
             Every Bach release has one in bach-<version>-windows-arm64.zip; to build only the x64 file, add --x64-only",
            name = arch.name
        );
    };
    if !path.is_file() {
        bail!("the {} setup runtime {} doesn't exist", arch.name, path.display());
    }
    // 0xAA64 is ARM64; 0xA641 is ARM64EC, which a runtime never is.
    match config::pe_machine(&path) {
        Some(0xAA64) => Ok(path),
        Some(other) => bail!(
            "{} isn't an ARM64 program (its machine type is 0x{other:04X}), so a setup file built on it would not run on Windows on ARM; \
             use the bach-setup.exe from bach-<version>-windows-arm64.zip",
            path.display()
        ),
        None => bail!("{} isn't a Windows program", path.display()),
    }
}

/// A `kind = "zip"` profile: the files, zipped. No setup runtime is needed, and
/// nothing is signed, feed-listed or encrypted: it is a folder in a file.
fn build_zip(project: &Project, out: Option<&Path>, strict: bool) -> Result<Built> {
    let started = Instant::now();
    let app = &project.manifest.app;
    say!("Building {} {} as the {} profile (a ZIP)", app.name, app.version, project.profile.as_deref().unwrap_or_default());
    output::warnings(&project.warnings);
    strict_gate(&project.warnings, strict)?;
    if project.encrypt {
        bail!("a ZIP can't be locked with [access]: the files in it would be readable. Use a setup profile for a protected app, or turn [access] off in this profile");
    }
    let output = tidy(&out.map(Path::to_path_buf).unwrap_or_else(|| project.output.clone()));
    let written = crate::zipfile::write(project, &output)?;
    let size = fs::metadata(&output).map(|meta| meta.len()).unwrap_or(0);
    say!();
    say!("Wrote {} ({}), {} files", output.display(), output::size(size), written.files);
    field(
        "Zipped",
        format!(
            "{} of {} ({:.0}%), in {:.1}s",
            output::size(size),
            output::size(written.bytes),
            size as f64 / written.bytes.max(1) as f64 * 100.0,
            started.elapsed().as_secs_f32()
        ),
    );
    Ok(Built { path: output, profile: project.profile.clone(), kind: project.kind, size })
}

/// What writing a setup file gives back: the archive's packed size and the
/// SHA-256 of each packaged file (in the order of `project.sources`).
pub(crate) struct Written {
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
    // A changed file the earlier setup file also has goes in as what differs from it.
    let wanted: HashMap<String, String> = patch
        .sources
        .iter()
        .filter(|source| earlier.files.iter().any(|entry| entry.path.eq_ignore_ascii_case(&source.packaged) && entry.size >= MIN_DELTA))
        .map(|source| (source.packaged.to_ascii_lowercase(), source.packaged.clone()))
        .collect();
    let bases = read_bases(old, &earlier, &wanted).unwrap_or_else(|err| {
        output::warnings(&[format!(
            "the earlier setup file's files couldn't be read for binary deltas ({err:#}), so the patch holds whole files"
        )]);
        HashMap::new()
    });
    write_file(&patch, runtime, &target, sealing.as_ref(), |project, runtime, path, sealing| {
        write_into_with(project, runtime, path, sealing, &bases)
    })?;
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

/// A changed file smaller than this isn't worth a delta.
const MIN_DELTA: u64 = 64 * 1024;

/// The most earlier files a patch build holds in memory at once.
const MAX_BASES: u64 = 2 << 30;

/// The earlier version of the `wanted` files, read out of the earlier setup file's
/// archive: what a binary delta is made from. A file that is too big, and anything
/// once the memory budget is used up, is left out and goes in whole.
fn read_bases(old: &Path, earlier: &bach_pack::manifest::Manifest, wanted: &HashMap<String, String>) -> Result<HashMap<String, DeltaBase>> {
    use std::io::{Read, Seek, SeekFrom};
    // An encrypted setup file's files can't be read without its key; deltas aren't worth asking for it.
    if wanted.is_empty() || earlier.access.as_ref().is_some_and(|access| access.encryption.is_some()) {
        return Ok(HashMap::new());
    }
    let mut file = File::open(old).with_context(|| format!("couldn't open {}", old.display()))?;
    let Some(layout) = container::locate(&mut file)? else {
        bail!("{} isn't a Bach setup file", old.display());
    };
    if !layout.has_archive() {
        return Ok(HashMap::new());
    }
    file.seek(SeekFrom::Start(layout.archive.offset))?;
    let reader = std::io::BufReader::new(file.take(layout.archive.len));
    let mut bases = HashMap::new();
    let mut held = 0u64;
    let mut visit = |packaged: &str, size: u64, body: &mut dyn Read| -> Result<()> {
        // The patch packs the file under the new version's name for it.
        let Some(name) = wanted.get(&packaged.to_ascii_lowercase()) else {
            return Ok(());
        };
        if size > bach_pack::delta::MAX_BASE || held.saturating_add(size) > MAX_BASES {
            return Ok(());
        }
        let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
        body.read_to_end(&mut bytes)?;
        held = held.saturating_add(size);
        let sha256 = bach_pack::protect::sha256_hex(&bytes[..])?;
        bases.insert(name.clone(), DeltaBase { bytes, sha256 });
        Ok(())
    };
    // An earlier patch has deltas of its own, which can't be expanded without its earlier version.
    archive::read_with(reader, archive::Bases::Skip, &mut visit)?;
    Ok(bases)
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
pub(crate) fn write_setup(project: &Project, runtime_path: &Path, output: &Path, sealing: Option<&Sealing>) -> Result<Written> {
    write_file(project, runtime_path, output, sealing, write_into)
}

/// A downloaded component's archive, as written.
pub(crate) struct Pack {
    pub component: String,
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
}

/// Writes the archive of each component that has `download`, next to the setup
/// file, records its SHA-256 and size in the manifest and the files' checksums
/// in the file list, and takes those files out of what the setup file packs.
pub(crate) fn write_packs(project: &mut Project, setup: &Path) -> Result<Vec<Pack>> {
    let wanted: Vec<String> =
        project.manifest.components.iter().filter(|component| component.download.is_some()).map(|c| c.id.clone()).collect();
    let mut packs = Vec::new();
    let flavour: Vec<&str> = project.profile.as_deref().into_iter().chain(project.arch.as_deref()).collect();
    for id in wanted {
        let members: std::collections::HashSet<String> = project
            .manifest
            .files
            .iter()
            .filter(|file| file.component.as_deref() == Some(id.as_str()))
            .map(|file| file.path.clone())
            .collect();
        let sources: Vec<archive::Source> = project.sources.iter().filter(|source| members.contains(&source.packaged)).cloned().collect();
        if sources.is_empty() {
            bail!("the component \"{id}\" has no files to put in its download");
        }
        let name = config::component_pack_name(&project.manifest.app, &id, &flavour);
        let path = setup.with_file_name(name);
        let (sha256, size, hashes) = write_pack(project, &sources, &path)?;
        record_hashes(&mut project.manifest, &sources, &hashes)?;
        if let Some(download) =
            project.manifest.components.iter_mut().find(|component| component.id == id).and_then(|c| c.download.as_mut())
        {
            download.sha256 = sha256.clone();
            download.size = size;
        }
        project.sources.retain(|source| !members.contains(&source.packaged));
        packs.push(Pack { component: id, path, sha256, size });
    }
    Ok(packs)
}

/// One archive: through a `.partial` file, so a failed build never leaves half of it where
/// the real one belongs. Gives its SHA-256, its size and each packed file's SHA-256.
fn write_pack(project: &Project, sources: &[archive::Source], path: &Path) -> Result<(String, u64, Vec<String>)> {
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        fs::create_dir_all(dir).with_context(|| format!("couldn't create {}", dir.display()))?;
    }
    let mut partial = path.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    let written = (|| -> Result<(String, u64, Vec<String>)> {
        let file = File::create(&partial).with_context(|| format!("couldn't write {}", partial.display()))?;
        let total = sources.iter().map(|source| source.size).sum();
        let mut progress = Progress::new("Packing", total);
        let plan = project.compression.plan(project.compression_threads, project.compression_memory_mib);
        let hashing = Hashing { inner: BufWriter::new(file), hasher: Sha256::new() };
        let (hashing, hashes) =
            archive::write_hashed(hashing, sources, project.compression, plan.threads, &mut |done| progress.update(done))?;
        progress.finish();
        let Hashing { inner, hasher } = hashing;
        let file = inner.into_inner().map_err(|err| err.into_error())?;
        file.sync_all()?;
        let size = file.metadata()?.len();
        Ok((bach_pack::protect::hex(&hasher.finalize()), size, hashes))
    })();
    match written {
        Ok(done) => {
            if path.exists() {
                fs::remove_file(path).with_context(|| format!("couldn't replace {} (is it open?)", path.display()))?;
            }
            fs::rename(&partial, path).with_context(|| format!("couldn't move the download to {}", path.display()))?;
            Ok(done)
        }
        Err(err) => {
            let _ = fs::remove_file(&partial);
            Err(err)
        }
    }
}

/// The online installer: the same window and manifest as the setup file,
/// and no files. It downloads the newest setup file when it runs.
fn write_online_setup(project: &Project, runtime_path: &Path, output: &Path, sealing: Option<&Sealing>) -> Result<Written> {
    write_file(project, runtime_path, output, sealing, write_online_into)
}

fn write_file(
    project: &Project,
    runtime_path: &Path,
    output: &Path,
    sealing: Option<&Sealing>,
    write: impl Fn(&Project, &[u8], &Path, Option<&Sealing>) -> Result<Written>,
) -> Result<Written> {
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
    write_into_with(project, runtime, path, sealing, &HashMap::new())
}

/// [`write_into`] that stores a changed file as a delta from its earlier version when `bases` has one.
fn write_into_with(
    project: &Project,
    runtime: &[u8],
    path: &Path,
    sealing: Option<&Sealing>,
    bases: &HashMap<String, DeltaBase>,
) -> Result<Written> {
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
            let (writer, hashes) =
                archive::write_hashed_with(writer, &project.sources, bases, project.compression, plan.threads, &mut step)?;
            (writer.finish()?, hashes)
        }
        None => archive::write_hashed_with(hashing, &project.sources, bases, project.compression, plan.threads, &mut step)?,
    };
    progress.finish();
    let Hashing { inner, hasher } = hashing;
    let mut file = inner.into_inner().map_err(|err| err.into_error())?;
    let look = project.look_section();
    // The runtime checks this before unpacking anything.
    let mut manifest = project.manifest.clone();
    manifest.archive_sha256 = Some(bach_pack::protect::hex(&hasher.finalize()));
    // The file checksums were taken while packing; the manifest goes in after the archive.
    record_hashes(&mut manifest, &project.sources, &hashes)?;
    let layout = container::finish(&mut file, archive_offset, &look, &manifest)?;
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
    container::finish(&mut file, offset, &project.look_section(), &manifest)?;
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
