//! `bach verify`: is this setup file whole? It unpacks nothing to disk and
//! installs nothing. It checks what setup checks while it installs, ahead of
//! time: the archive against the SHA-256 recorded at build, and every file
//! against the size and SHA-256 in the manifest. For a download that went
//! wrong, a copy that was changed, or a file a build server handed over.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{bail, Context, Result};
use bach_pack::archive::{self, Verifying};
use bach_pack::container;
use bach_pack::manifest::FileEntry;
use bach_pack::pe::{self, Signature};
use bach_pack::protect;

use crate::output::{self, field, Progress};

pub struct Options<'a> {
    pub file: &'a Path,
    /// The access key's file, for a setup file that is encrypted.
    pub key_file: Option<&'a Path>,
}

/// The most damaged files named in the error; the log of a broken download
/// can be long, and the first few say what is wrong.
const NAMED: usize = 5;

pub fn command(options: Options) -> Result<()> {
    let path = options.file;
    // A macOS setup file keeps its payload inside the Setup app.
    let mac = crate::macfile::payload(path)?;
    let mut file = match &mac {
        Some(payload) => payload.open()?,
        None => File::open(path).with_context(|| format!("couldn't open {}", path.display()))?,
    };
    let Some(layout) = container::locate(&mut file)? else {
        bail!("{} isn't a Bach setup file", path.display());
    };
    let manifest = container::read_manifest(&mut file, &layout)?;
    let app = &manifest.app;
    say!("Verifying {} {} ({})", app.name, app.version, path.display());
    output::result("name", app.name.clone());
    output::result("version", app.version.clone());

    let signature = if mac.is_some() { None } else { pe::signature(path) };
    let signature_text = match signature {
        Some(Signature::Signed) => {
            "signed (the file carries a signature; Windows decides whether it is valid, `signtool verify /pa` checks it)"
        }
        Some(Signature::Unsigned) => "not signed",
        None if mac.is_some() => {
            "not checked here: macOS checks the Setup app's signature when it opens (`codesign --verify --deep --strict` checks it)"
        }
        None => "unknown",
    };
    output::result("signature", if signature == Some(Signature::Signed) { "signed" } else { "unsigned" });
    field("Signature", signature_text);

    if !layout.has_archive() {
        let what = if manifest.online {
            "an online installer: it carries no files and downloads the newest version when it runs"
        } else {
            "an uninstaller: it carries no files"
        };
        output::result("verified", true);
        output::result("files", 0);
        say!("This is {what}, so there is nothing more to check.");
        return Ok(());
    }

    let mut problems: Vec<String> = Vec::new();

    // 1. The archive as a whole, against the SHA-256 recorded when it was built.
    match &manifest.archive_sha256 {
        Some(expected) => {
            file.seek(SeekFrom::Start(layout.archive.offset))?;
            let actual = protect::sha256_hex(BufReader::new((&mut file).take(layout.archive.len)))
                .with_context(|| format!("couldn't read {}", path.display()))?;
            if actual.eq_ignore_ascii_case(expected) {
                field("Archive", format!("SHA-256 matches ({} bytes)", layout.archive.len));
            } else {
                problems.push("the archive doesn't match the SHA-256 recorded when the setup file was built".into());
            }
        }
        None => field("Archive", "no checksum recorded (built by an older Bach); the files are checked below"),
    }

    // 2. Every file: size and SHA-256 against the manifest.
    let encryption = manifest.access.as_ref().and_then(|access| access.encryption.as_ref());
    let unlocked = match encryption {
        Some(encryption) => match read_key(options.key_file)? {
            Some(key) => Some(encryption.unlock(&key).map_err(|err| anyhow::anyhow!("{err}"))?),
            None => None,
        },
        None => None,
    };
    let checked = if encryption.is_some() && unlocked.is_none() {
        field("Files", "encrypted: pass --key-file FILE (or set BACH_ACCESS_KEY) to check them too");
        None
    } else {
        file.seek(SeekFrom::Start(layout.archive.offset))?;
        let reader = BufReader::new((&mut file).take(layout.archive.len));
        let recorded: HashMap<&str, &FileEntry> = manifest.files.iter().map(|entry| (entry.path.as_str(), entry)).collect();
        // The files of a component that is downloaded aren't in this archive; their own archive is checked below.
        let downloaded: HashSet<&str> = manifest
            .files
            .iter()
            .filter(|entry| {
                manifest
                    .components
                    .iter()
                    .any(|component| component.download.is_some() && entry.component.as_deref() == Some(component.id.as_str()))
            })
            .map(|entry| entry.path.as_str())
            .collect();
        let total = manifest.total_size();
        let mut progress = Progress::new("Checking", total);
        let mut done = 0u64;
        let mut seen: HashSet<String> = HashSet::new();
        let mut bad: Vec<String> = Vec::new();
        let mut visit = |packaged: &str, size: u64, body: &mut dyn Read| -> Result<()> {
            let mut verifying = Verifying::new(body);
            io::copy(&mut verifying, &mut io::sink())?;
            let (count, sha256) = verifying.digest();
            done = done.saturating_add(count);
            progress.update(done);
            seen.insert(packaged.to_string());
            match recorded.get(packaged) {
                None => bad.push(format!("{packaged} is in the archive but not in the manifest")),
                Some(entry) if entry.size != size || entry.size != count => {
                    bad.push(format!("{packaged} doesn't have the size the manifest records"));
                }
                Some(entry) if entry.sha256.as_deref().is_some_and(|expected| !sha256.eq_ignore_ascii_case(expected)) => {
                    bad.push(format!("{packaged} doesn't match its SHA-256"));
                }
                Some(_) => {}
            }
            Ok(())
        };
        let outcome = match (encryption, &unlocked) {
            (Some(encryption), Some(unlocked)) => {
                encryption.reader(reader, unlocked).and_then(|decrypted| archive::read_with(decrypted, archive::Bases::Skip, &mut visit))
            }
            _ => archive::read_with(reader, archive::Bases::Skip, &mut visit),
        };
        progress.finish();
        if let Err(err) = outcome {
            problems.push(format!("the archive is damaged: {err:#}"));
        }
        // An update patch carries only what changed; every other file is on the computer already.
        if manifest.patch_from.is_none() {
            for entry in &manifest.files {
                // Links have no bytes: they are in the manifest only.
                if !entry.is_link() && !seen.contains(&entry.path) && !downloaded.contains(entry.path.as_str()) {
                    bad.push(format!("{} is in the manifest but not in the archive", entry.path));
                }
            }
        }
        problems.extend(bad);
        Some(seen.len())
    };

    if !problems.is_empty() {
        let shown: Vec<&str> = problems.iter().take(NAMED).map(String::as_str).collect();
        let more = problems.len().saturating_sub(NAMED);
        output::result("verified", false);
        let mut message = format!("{} is damaged: {}", path.display(), shown.join("; "));
        if more > 0 {
            message.push_str(&format!("; and {more} more"));
        }
        bail!("{message}. Download or build it again");
    }

    // 3. The archives of components that are downloaded: the one next to the setup file, when it is there.
    for component in manifest.components.iter().filter(|component| component.download.is_some()) {
        let Some(download) = &component.download else { continue };
        let name = download.url.split(['?', '#']).next().and_then(|link| link.rsplit('/').next()).unwrap_or_default();
        let beside = path.parent().map(|folder| folder.join(name)).filter(|candidate| !name.is_empty() && candidate.is_file());
        match beside {
            None => field("Download", format!("{}: {name} isn't next to the setup file, so it wasn't checked", component.id)),
            Some(archive_path) => {
                let actual = protect::sha256_hex(BufReader::new(File::open(&archive_path)?))
                    .with_context(|| format!("couldn't read {}", archive_path.display()))?;
                let size = std::fs::metadata(&archive_path).map(|meta| meta.len()).unwrap_or(0);
                if actual.eq_ignore_ascii_case(&download.sha256) && size == download.size {
                    field("Download", format!("{}: {name} matches the SHA-256 in the setup file ({size} bytes)", component.id));
                } else {
                    output::result("verified", false);
                    bail!(
                        "{} doesn't match the SHA-256 and size the setup file records for the component {}. Download or build it again",
                        archive_path.display(),
                        component.id
                    );
                }
            }
        }
    }

    output::result("verified", true);
    output::result("files", checked.unwrap_or(0));
    match checked {
        Some(count) => {
            field("Files", format!("{count} checked against the manifest"));
            say!("{} is intact.", path.display());
        }
        None => say!("The archive is intact; the encrypted files were not opened."),
    }
    Ok(())
}

/// The access key: the first line of `--key-file`, else `BACH_ACCESS_KEY`.
fn read_key(key_file: Option<&Path>) -> Result<Option<String>> {
    if let Some(file) = key_file {
        let text = bach_pack::textfile::read(file).with_context(|| format!("couldn't read the key file {}", file.display()))?;
        return match bach_pack::textfile::first_line(&text) {
            Some(key) => Ok(Some(key)),
            None => bail!("the key file has no access key in it"),
        };
    }
    Ok(std::env::var("BACH_ACCESS_KEY").ok().map(|key| key.trim().to_string()).filter(|key| !key.is_empty()))
}
