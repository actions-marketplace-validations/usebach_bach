//! `bach build` for macOS: the Setup app (`Install Hello.app`) that carries the
//! payload, signed, and delivered in a `.dmg` (or a `.zip` where there is no
//! `hdiutil`). The payload is never appended to the runtime, because a Mach-O
//! program with trailing bytes fails code signing; it sits in the bundle's
//! `Contents/Resources/setup.bach` and the signature covers it.

use std::collections::HashMap;
use std::process::Command;

use bach_pack::config::{MacContainer, MacSigning};
use bach_pack::macapp;
use bach_pack::manifest::Platform;

use super::*;

/// The macOS build of the project `options` names.
pub(super) fn build(options: BuildOptions, mut project: Project, started: Instant) -> Result<Built> {
    let BuildOptions { out, runtime, fast, online, key_file, sign, strict, msi, patch_from, pkg, feed_from, .. } = options;
    if project.manifest.updates.is_some()
        && project.manifest.access.as_ref().is_some_and(|access| access.encryption.is_some() || project.encrypt)
    {
        // Fine: the access key is handled like on Windows.
    }
    if fast {
        project.compression = Compression::Fast;
    }
    if online {
        bail!("the online installer is a Windows setup file; leave --online out for macOS");
    }
    if pkg && !cfg!(target_os = "macos") {
        bail!("--pkg needs macOS (pkgbuild); build on a Mac, or leave it out");
    }
    if msi || patch_from.is_some() {
        bail!(
            "{} is a Windows feature; leave it out for macOS (or build the Windows file with --platform windows)",
            if msi { "--msi" } else { "--patch-from" }
        );
    }
    let mac = project.macos.clone().context("a macOS build needs [macos]")?;
    let mut output = out.map(Path::to_path_buf).unwrap_or_else(|| project.output.clone());
    // `--out` names the file, and its extension says what it is.
    let mut container = match output.extension().map(|ext| ext.to_string_lossy().to_ascii_lowercase()).as_deref() {
        Some("dmg") => MacContainer::Dmg,
        Some("zip") => MacContainer::Zip,
        _ => bail!("the macOS setup file must end in .dmg or .zip; {} doesn't", output.display()),
    };
    if container == MacContainer::Dmg && !cfg!(target_os = "macos") {
        // `hdiutil` exists only on a Mac. A zip of the same app works everywhere.
        say!("A .dmg needs macOS (hdiutil), so this build writes a .zip of the Setup app instead.");
        container = MacContainer::Zip;
        output.set_extension("zip");
    }
    if sign && !cfg!(target_os = "macos") {
        bail!("signing a macOS setup app needs macOS (codesign); build on a Mac or leave --sign out");
    }
    let runtime_path = runtime::find_for(Platform::Macos, runtime)?;
    let runtime_bytes = runtime::read_for(Platform::Macos, &runtime_path)?;
    let app = project.manifest.app.clone();
    say!("Building {} {} for macOS", app.name, app.version);
    summary(&project);
    output::warnings(&project.warnings);
    strict_gate(&project.warnings, strict)?;

    let sealing = seal(&mut project, key_file, None)?;
    if sealing.is_some() {
        say!("The files are encrypted with the access key.");
    }
    let output = tidy(&output);
    let packs = write_packs(&mut project, &output)?;

    let staging = staging_dir(&output);
    let _ = fs::remove_dir_all(&staging);
    let result = (|| -> Result<(Written, PathBuf)> {
        let app_dir = staging.join(macapp::setup_app_name(&project.manifest));
        let written = write_setup_app(&project, &mac, &runtime_bytes, &app_dir, sealing.as_ref())?;
        let signing = mac.signing.as_ref().filter(|_| sign);
        if let Some(signing) = signing {
            codesign(&app_dir, &signing.identity, true)?;
        } else if cfg!(target_os = "macos") {
            // Seal the bundle so its resources are part of its (ad hoc) signature.
            codesign(&app_dir, "-", false)?;
        }
        if let Some(dir) = output.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            fs::create_dir_all(dir).with_context(|| format!("couldn't create {}", dir.display()))?;
        }
        // hdiutil adds ".dmg" to a name that doesn't end in it, so the extension stays last.
        let partial = output
            .with_extension(format!("partial.{}", output.extension().map(|ext| ext.to_string_lossy().into_owned()).unwrap_or_default()));
        let _ = fs::remove_file(&partial);
        match container {
            MacContainer::Dmg => make_dmg(&staging, &app.name, &partial)?,
            MacContainer::Zip => make_zip(&staging, &partial)?,
        }
        if output.exists() {
            fs::remove_file(&output).with_context(|| format!("couldn't replace {} (is it open?)", output.display()))?;
        }
        fs::rename(&partial, &output).with_context(|| format!("couldn't move the setup file to {}", output.display()))?;
        if let Some(signing) = signing {
            if container == MacContainer::Dmg {
                codesign(&output, &signing.identity, false)?;
                notarize(signing, &output)?;
            }
        }
        Ok((written, app_dir))
    })();
    let _ = fs::remove_dir_all(&staging);
    let (written, _) = result?;
    record_hashes(&mut project.manifest, &project.sources, &written.hashes)?;

    let size = fs::metadata(&output).map(|meta| meta.len()).unwrap_or(0);
    let installed = project.manifest.total_size().max(1);
    say!();
    say!("Wrote {} ({})", output.display(), output::size(size));
    for pack in &packs {
        say!("Wrote {} ({}), the download for the component {}", pack.path.display(), output::size(pack.size), pack.component);
    }
    output::result("setup_macos", output.display().to_string());
    if pkg {
        let package = output.with_extension("pkg");
        write_pkg(&project, &mac, &package, mac.signing.as_ref().filter(|_| sign))?;
        let size = fs::metadata(&package).map(|meta| meta.len()).unwrap_or(0);
        say!("Wrote {} ({}), the installer package for MDM tools", package.display(), output::size(size));
        output::result("pkg", package.display().to_string());
    }
    output::result("version", project.manifest.app.version.clone());
    if mac.signing.is_some() && !sign {
        say!("Not signed with your identity: bach.toml has [macos.signing], but signing runs only with --sign (or BACH_SIGN=1).");
    } else if sign && mac.signing.is_none() {
        bail!("--sign was given, but bach.toml has no [macos.signing] section that says how to sign");
    }
    // The update feed lists this file for Macs.
    if project.manifest.updates.is_some() {
        crate::feed::after_mac_build(&project, &output, feed_from)?;
    }
    if output::json() {
        let (sha256, final_size) = crate::feed::digest_of(&output)?;
        output::result("sha256", sha256);
        output::result("size", final_size);
        output::result("packed", written.packed);
    }
    field(
        "Packed",
        format!(
            "{} of {} ({:.0}%), in {:.1}s",
            output::size(written.packed),
            output::size(installed),
            written.packed as f64 / installed as f64 * 100.0,
            started.elapsed().as_secs_f32()
        ),
    );
    Ok(Built { path: output, profile: None, kind: project.kind, size })
}

/// Where an installer package puts the app: straight into `/Applications` when the package is
/// only an app bundle, and into `/Applications/<folder>` when it is more (so loose files never
/// land among the user's apps).
fn pkg_location(project: &Project) -> String {
    let manifest = &project.manifest;
    let top: std::collections::BTreeSet<&str> = manifest.files.iter().map(|file| file.path.split('/').next().unwrap_or_default()).collect();
    let alone =
        manifest.app.executable.to_lowercase().ends_with(".app") && top.len() == 1 && top.contains(manifest.app.executable.as_str());
    if alone {
        "/Applications".to_string()
    } else {
        format!("/Applications/{}", manifest.install.folder)
    }
}

/// Copies the app's files, with their permissions and links, into `root`: what the package installs.
fn stage_pkg_root(project: &Project, root: &Path) -> Result<()> {
    for source in &project.sources {
        let target = bach_pack::paths::under(root, &source.packaged);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&source.path, &target).with_context(|| format!("couldn't copy {}", source.path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = project.manifest.files.iter().find(|file| file.path == source.packaged).and_then(|file| file.mode).unwrap_or(0o644);
            fs::set_permissions(&target, fs::Permissions::from_mode(mode & 0o777))?;
        }
    }
    #[cfg(unix)]
    for file in project.manifest.files.iter().filter(|file| file.is_link()) {
        let target = bach_pack::paths::under(root, &file.path);
        if let (Some(parent), Some(to)) = (target.parent(), file.link.as_deref()) {
            fs::create_dir_all(parent)?;
            std::os::unix::fs::symlink(to, &target)?;
        }
    }
    Ok(())
}

/// Writes the installer package (`pkgbuild`), signs it with the Developer ID Installer identity
/// when asked to, and has Apple notarize and staple it. A package puts the app in `/Applications`
/// for every user, and is what Jamf, Intune, Kandji and Munki deploy; it has no window, no
/// choices and no uninstaller of its own, which is how those tools want it.
fn write_pkg(project: &Project, mac: &config::MacBuild, output: &Path, signing: Option<&MacSigning>) -> Result<()> {
    let root = output.with_extension("pkg-root");
    let _ = fs::remove_dir_all(&root);
    let result = (|| -> Result<()> {
        stage_pkg_root(project, &root)?;
        if let Some(dir) = output.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            fs::create_dir_all(dir)?;
        }
        let unsigned = output.with_extension("unsigned.pkg");
        let _ = fs::remove_file(&unsigned);
        let identifier =
            mac.bundle_id.strip_suffix(".setup").map(|base| format!("{base}.pkg")).unwrap_or_else(|| format!("{}.pkg", mac.bundle_id));
        let built = Command::new("pkgbuild")
            .arg("--root")
            .arg(&root)
            .args(["--identifier", &identifier, "--version", &macapp::bundle_version(&project.manifest.app.version)])
            .args(["--install-location", &pkg_location(project), "--ownership", "recommended"])
            .arg(&unsigned)
            .output()
            .context("couldn't run pkgbuild (it comes with macOS)")?;
        if !built.status.success() {
            bail!("pkgbuild couldn't make the package: {}", String::from_utf8_lossy(&built.stderr).trim());
        }
        let _ = fs::remove_file(output);
        match signing {
            Some(signing) => {
                let Some(identity) = &signing.installer_identity else {
                    bail!("--pkg with --sign needs [macos.signing] installer_identity, a \"Developer ID Installer\" certificate (the app's own identity can't sign a package)");
                };
                let signed = Command::new("productsign")
                    .args(["--sign", identity])
                    .arg(&unsigned)
                    .arg(output)
                    .output()
                    .context("couldn't run productsign")?;
                let _ = fs::remove_file(&unsigned);
                if !signed.status.success() {
                    bail!("productsign couldn't sign the package: {}", String::from_utf8_lossy(&signed.stderr).trim());
                }
                notarize(signing, output)?;
            }
            None => fs::rename(&unsigned, output)?,
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&root);
    result
}

/// The folder the Setup app is made in before it goes into the `.dmg`.
fn staging_dir(output: &Path) -> PathBuf {
    let name = output.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "setup".into());
    output.with_file_name(format!(".{name}.bach-staging"))
}

/// Lays out the Setup app: the runtime, the payload, the `Info.plist` and the icon.
pub(crate) fn write_setup_app(
    project: &Project,
    mac: &config::MacBuild,
    runtime: &[u8],
    app_dir: &Path,
    sealing: Option<&Sealing>,
) -> Result<Written> {
    let at = |relative: &str| app_dir.join(relative);
    for dir in ["Contents/MacOS", "Contents/Resources"] {
        fs::create_dir_all(at(dir)).with_context(|| format!("couldn't create {}", at(dir).display()))?;
    }
    write_executable(&at(macapp::RUNTIME), runtime)?;
    let plist = macapp::info_plist(&macapp::Plist {
        bundle_id: &mac.bundle_id,
        display_name: &format!("Install {}", project.manifest.app.name),
        version: &project.manifest.app.version,
        icon: mac.icon.is_some(),
        minimum_system: mac.minimum_version.as_deref().unwrap_or(macapp::MIN_SYSTEM),
    });
    fs::write(at("Contents/Info.plist"), plist)?;
    if let Some(icon) = &mac.icon {
        fs::copy(icon, at(macapp::ICON)).with_context(|| format!("couldn't copy {}", icon.display()))?;
    }
    // The payload file is the container with no runtime in front of it.
    write_into_with(project, &[], &at(macapp::PAYLOAD), sealing, &HashMap::new())
}

/// Writes `bytes` and marks the file as a program.
fn write_executable(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).with_context(|| format!("couldn't write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Signs `target` with `identity`; `-` signs ad hoc. A Developer ID signature also
/// gets the hardened runtime and a secure timestamp, which notarization requires.
fn codesign(target: &Path, identity: &str, developer: bool) -> Result<()> {
    let mut command = Command::new("codesign");
    command.args(["--force", "--sign", identity]);
    if developer && identity != "-" {
        command.args(["--timestamp", "--options", "runtime"]);
    }
    let output = command.arg(target).output().context("couldn't run codesign (it comes with macOS)")?;
    if !output.status.success() {
        bail!("codesign couldn't sign {}: {}", target.display(), String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

/// Submits the `.dmg` to Apple's notary service, waits for the verdict and staples the ticket.
fn notarize(signing: &MacSigning, dmg: &Path) -> Result<()> {
    let Some(profile) = &signing.notary_profile else {
        return Ok(());
    };
    say!("Notarizing {} (this waits for Apple, usually a few minutes)", dmg.display());
    let submitted = Command::new("xcrun")
        .args(["notarytool", "submit"])
        .arg(dmg)
        .args(["--keychain-profile", profile, "--wait"])
        .output()
        .context("couldn't run xcrun notarytool (install Xcode's command line tools)")?;
    let text = String::from_utf8_lossy(&submitted.stdout);
    if !submitted.status.success() || !text.contains("status: Accepted") {
        bail!("Apple's notary service didn't accept the file: {}", text.trim().lines().last().unwrap_or("no answer").trim());
    }
    let stapled = Command::new("xcrun").args(["stapler", "staple"]).arg(dmg).output().context("couldn't run xcrun stapler")?;
    if !stapled.status.success() {
        bail!("the file was notarized, but stapling the ticket failed: {}", String::from_utf8_lossy(&stapled.stderr).trim());
    }
    Ok(())
}

/// A compressed disk image holding the folder's contents. Its window is laid out so opening it shows
/// one big icon, "Install <app>", and nothing else: no toolbar, no sidebar, no status bar.
fn make_dmg(folder: &Path, volume: &str, output: &Path) -> Result<()> {
    // The picture behind the icon, with the app's name as its title; without Swift the window is plain.
    let background = render_background(folder, &format!("Install {volume}"), "Double-click to install");
    let volume: String = volume.chars().filter(|c| !c.is_control() && *c != '/' && *c != ':').take(27).collect();
    let volume = if volume.trim().is_empty() { "Setup".to_string() } else { volume.trim().to_string() };
    let hdiutil = |args: &[&std::ffi::OsStr]| -> Result<std::process::Output> {
        Command::new("hdiutil").args(args).output().context("couldn't run hdiutil (it comes with macOS)")
    };
    let writable = output.with_extension("rw.dmg");
    let _ = fs::remove_file(&writable);
    let made = hdiutil(&[
        "create".as_ref(),
        "-quiet".as_ref(),
        "-ov".as_ref(),
        "-format".as_ref(),
        "UDRW".as_ref(),
        "-fs".as_ref(),
        "HFS+".as_ref(),
        "-volname".as_ref(),
        volume.as_ref(),
        "-srcfolder".as_ref(),
        folder.as_os_str(),
        writable.as_os_str(),
    ])?;
    if !made.status.success() {
        bail!("hdiutil couldn't make the disk image: {}", String::from_utf8_lossy(&made.stderr).trim());
    }
    let item =
        fs::read_dir(folder)?.flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).find(|name| name.ends_with(".app"));
    if let Some(item) = item {
        if !arrange_window(&writable, &volume, &item, background) {
            say!("  note: the disk image's window couldn't be arranged (Finder didn't answer), so it opens in Finder's default look");
        }
    }
    let converted = hdiutil(&[
        "convert".as_ref(),
        "-quiet".as_ref(),
        writable.as_os_str(),
        "-format".as_ref(),
        "UDZO".as_ref(),
        "-imagekey".as_ref(),
        "zlib-level=9".as_ref(),
        "-o".as_ref(),
        output.as_os_str(),
    ]);
    let _ = fs::remove_file(&writable);
    let converted = converted?;
    if !converted.status.success() {
        bail!("hdiutil couldn't compress the disk image: {}", String::from_utf8_lossy(&converted.stderr).trim());
    }
    Ok(())
}

/// Draws the disk image's background into `folder/.background` (a hidden folder, so Finder shows only
/// the app). Needs Swift, which comes with Xcode's command line tools; false when it isn't there.
fn render_background(folder: &Path, title: &str, subtitle: &str) -> bool {
    const SOURCE: &str = include_str!("dmg_background.swift");
    let dir = folder.join(".background");
    if fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let script = std::env::temp_dir().join(format!("bach-dmg-background-{}.swift", std::process::id()));
    if fs::write(&script, SOURCE).is_err() {
        return false;
    }
    let target = dir.join("background.png");
    let child = Command::new("swift")
        .arg(&script)
        .arg(&target)
        .arg(title)
        .arg(subtitle)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let mut drawn = false;
    if let Ok(mut child) = child {
        let started = Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(90) {
            match child.try_wait() {
                Ok(Some(status)) => {
                    drawn = status.success();
                    break;
                }
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(200)),
                Err(_) => break,
            }
        }
        if !drawn {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    let _ = fs::remove_file(&script);
    if !drawn || !target.is_file() {
        let _ = fs::remove_dir_all(&dir);
        return false;
    }
    true
}

/// Opens the writable image, has Finder lay its window out (icon view, one large icon, no toolbar) and
/// closes it again. Finder scripting needs a signed-in desktop session and may ask the user once to let
/// the terminal control Finder; when it can't, the image is simply left with Finder's default window.
fn arrange_window(image: &Path, volume: &str, item: &str, background: bool) -> bool {
    let mount = Path::new("/Volumes").join(volume);
    if mount.exists() {
        // A volume of that name is open already; Finder would lay out the wrong one.
        return false;
    }
    let attached = Command::new("hdiutil").args(["attach", "-quiet", "-readwrite", "-noverify", "-noautoopen"]).arg(image).output();
    if !attached.is_ok_and(|output| output.status.success()) {
        return false;
    }
    const SCRIPT: &str = "set vol to system attribute \"BACH_VOLUME\"\n\
        set itemName to system attribute \"BACH_ITEM\"\n\
        tell application \"Finder\"\n\
          tell disk vol\n\
            open\n\
            set current view of container window to icon view\n\
            set toolbar visible of container window to false\n\
            set statusbar visible of container window to false\n\
            set the bounds of container window to {200, 140, 760, 500}\n\
            set opts to the icon view options of container window\n\
            set arrangement of opts to not arranged\n\
            set icon size of opts to 128\n\
            if (system attribute \"BACH_BACKGROUND\") is not \"\" then set background picture of opts to (POSIX file (system attribute \"BACH_BACKGROUND\") as alias)\n\
            set text size of opts to 13\n\
            set position of item itemName of container window to {280, 215}\n\
            update without registering applications\n\
            delay 1\n\
            close\n\
          end tell\n\
        end tell";
    let child = Command::new("osascript")
        .arg("-e")
        .arg(SCRIPT)
        .env("BACH_VOLUME", volume)
        .env("BACH_ITEM", item)
        .env(
            "BACH_BACKGROUND",
            if background { mount.join(".background/background.png").to_string_lossy().into_owned() } else { String::new() },
        )
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let mut arranged = false;
    if let Ok(mut child) = child {
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    arranged = status.success();
                    if !arranged {
                        let mut why = String::new();
                        if let Some(mut pipe) = child.stderr.take() {
                            let _ = std::io::Read::read_to_string(&mut pipe, &mut why);
                        }
                        eprintln!("  Finder said: {}", why.trim());
                    }
                    break;
                }
                Ok(None) if started.elapsed() > std::time::Duration::from_secs(60) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(200)),
                Err(_) => break,
            }
        }
    }
    // Finder writes the window's look (`.DS_Store`) when the volume is ejected; a busy volume is detached with force.
    let _ = Command::new("sync").status();
    for attempt in 0..5 {
        let mut command = Command::new("hdiutil");
        command.args(["detach", "-quiet"]);
        if attempt >= 3 {
            command.arg("-force");
        }
        if command.arg(&mount).output().is_ok_and(|output| output.status.success()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    arranged
}

/// A zip of the folder's contents that keeps program permissions, for hosts without `hdiutil`.
fn make_zip(folder: &Path, output: &Path) -> Result<()> {
    use std::io::Write;
    let file = File::create(output).with_context(|| format!("couldn't write {}", output.display()))?;
    let mut zip = zip::ZipWriter::new(BufWriter::new(file));
    let mut stack = vec![(folder.to_path_buf(), String::new())];
    let mut entries: Vec<(PathBuf, String)> = Vec::new();
    while let Some((dir, prefix)) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            if entry.file_type()?.is_dir() {
                stack.push((entry.path(), relative));
            } else {
                entries.push((entry.path(), relative));
            }
        }
    }
    entries.sort_by(|a, b| a.1.cmp(&b.1));
    for (path, relative) in entries {
        let program = relative.ends_with(macapp::RUNTIME);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            // Unix attributes also when this runs on Windows, where the zip crate would write DOS ones.
            .system(zip::System::Unix)
            .unix_permissions(if program { 0o755 } else { 0o644 });
        zip.start_file(relative, options)?;
        zip.write_all(&fs::read(&path)?)?;
    }
    zip.finish()?.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zip_keeps_the_runtime_executable() {
        let dir = std::env::temp_dir().join(format!("bach-macmake-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Install Hello.app/Contents/MacOS")).unwrap();
        fs::write(dir.join("Install Hello.app/Contents/MacOS/bach-setup"), b"prog").unwrap();
        fs::write(dir.join("Install Hello.app/Contents/Info.plist"), b"<plist/>").unwrap();
        let out = std::env::temp_dir().join(format!("bach-macmake-{}.zip", std::process::id()));
        make_zip(&dir, &out).unwrap();
        let mut archive = zip::ZipArchive::new(File::open(&out).unwrap()).unwrap();
        let program = archive.by_name("Install Hello.app/Contents/MacOS/bach-setup").unwrap();
        assert_eq!(program.unix_mode().unwrap() & 0o777, 0o755);
        drop(program);
        let plist = archive.by_name("Install Hello.app/Contents/Info.plist").unwrap();
        assert_eq!(plist.unix_mode().unwrap() & 0o777, 0o644);
        fs::remove_dir_all(&dir).unwrap();
        fs::remove_file(&out).unwrap();
    }

    #[test]
    fn the_staging_folder_is_hidden_beside_the_output() {
        assert_eq!(staging_dir(Path::new("dist/Hello.dmg")), Path::new("dist/.Hello.dmg.bach-staging"));
    }
}
