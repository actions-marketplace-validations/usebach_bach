//! `bach build --msi`: the package is read back, and (ignored by default) installed,
//! upgraded and removed with Windows Installer itself.

mod common;
use common::{config, project, run, text};
use std::io::Read;
use std::path::{Path, PathBuf};

fn msi_project(name: &str, id: &str, extra: &str) -> PathBuf {
    let dir = project(name, &config(extra).replace("BachIntegrationTest-Hello", id));
    std::fs::create_dir_all(dir.join("app/data")).unwrap();
    std::fs::write(dir.join("app/data/readme.txt"), "hello from the msi").unwrap();
    std::fs::write(dir.join("app/data/long file name with spaces.txt"), "second").unwrap();
    dir
}

/// A table's rows as text, through the `msi` crate.
fn rows(package: &mut msi::Package<std::fs::File>, table: &str) -> Vec<Vec<String>> {
    package
        .select_rows(msi::Select::table(table))
        .unwrap()
        .map(|row| {
            (0..row.len())
                .map(|index| match &row[index] {
                    msi::Value::Str(text) => text.clone(),
                    msi::Value::Null => String::new(),
                    other => other.to_string(),
                })
                .collect()
        })
        .collect()
}

#[test]
fn an_msi_holds_the_files_the_shortcuts_and_the_upgrade_rules() {
    need_runtime!();
    let extra = "[[registry]]\nkey = 'Software\\BachMsiTest'\nname = \"InstallPath\"\nvalue = '{app}'\n";
    let dir = msi_project("msi-tables", "BachMsiTables", extra);
    let out = run(&dir, &["build", "--fast", "--msi", "--json"]);
    assert!(out.status.success(), "{}\n{}", text(&out.stdout), text(&out.stderr));
    let object: serde_json::Value = serde_json::from_str(&text(&out.stdout)).unwrap();
    let msi_path = dir.join(object["msi"].as_str().unwrap());
    assert!(msi_path.ends_with("Hello-Setup-1.0.0.msi"), "{object}");

    let mut package = msi::open(&msi_path).unwrap();
    let property = |package: &mut msi::Package<std::fs::File>, name: &str| -> String {
        rows(package, "Property").into_iter().find(|row| row[0] == name).map(|row| row[1].clone()).unwrap_or_default()
    };
    assert_eq!(property(&mut package, "ProductName"), "Hello");
    assert_eq!(property(&mut package, "ProductVersion"), "1.0.0");
    assert_eq!(property(&mut package, "Manufacturer"), "Example");
    assert_eq!(property(&mut package, "MSIINSTALLPERUSER"), "1", "installer.scope is user, so the package needs no administrator");
    assert_eq!(package.summary_info().arch(), Some("x64"));

    // Three files (the program, 1 MB of data, ...) are listed, each with a short and a long name, and the cabinet holds them all.
    let files = rows(&mut package, "File");
    assert_eq!(files.len(), 4, "{files:?}");
    assert!(
        files.iter().any(|row| row[2].ends_with("|hello.exe"))
            && files.iter().any(|row| row[2].ends_with("|long file name with spaces.txt"))
    );
    assert!(files.iter().all(|row| row[2].split('|').next().is_some_and(|short| short.len() <= 12)), "short names are 8.3");
    let mut cabinet = cab::Cabinet::new(package.read_stream("files.cab").unwrap()).unwrap();
    let names: Vec<String> =
        cabinet.folder_entries().flat_map(|folder| folder.file_entries().map(|file| file.name().to_string()).collect::<Vec<_>>()).collect();
    assert_eq!(names.len(), 4);
    let key = files.iter().find(|row| row[2].ends_with("|readme.txt")).unwrap()[0].clone();
    let mut content = String::new();
    cabinet.read_file(&key).unwrap().read_to_string(&mut content).unwrap();
    assert_eq!(content, "hello from the msi", "a file comes out of the cabinet as it went in");

    // Folders, shortcuts, registry values and the upgrade rows.
    let directories = rows(&mut package, "Directory");
    assert!(
        directories.iter().any(|row| row[0] == "INSTALLFOLDER") && directories.iter().any(|row| row[2].ends_with("|data")),
        "{directories:?}"
    );
    assert_eq!(rows(&mut package, "Shortcut").len(), 2, "the Start menu shortcut and the desktop one, which is on by default");
    assert_eq!(rows(&mut package, "Registry")[0][3], "InstallPath");
    assert_eq!(rows(&mut package, "Registry")[0][4], "[INSTALLFOLDER]");
    let upgrades = rows(&mut package, "Upgrade");
    assert_eq!(upgrades.len(), 2);
    assert!(upgrades.iter().any(|row| row[6] == "PREVIOUSVERSIONSINSTALLED") && upgrades.iter().any(|row| row[6] == "NEWERPRODUCTFOUND"));
    let sequence = rows(&mut package, "InstallExecuteSequence");
    let at = |name: &str| sequence.iter().find(|row| row[0] == name).map(|row| row[2].parse::<i32>().unwrap()).unwrap();
    assert!(at("FindRelatedProducts") < at("LaunchConditions"), "a newer version has to be found before the condition that refuses it");
    assert!(at("RemoveExistingProducts") < at("InstallInitialize") && at("InstallFiles") < at("CreateShortcuts"));

    // The package is the same every time it is built from the same files, except for its creation time; the codes are stable.
    let again = run(&dir, &["build", "--fast", "--msi", "--out", "second/Setup.exe"]);
    assert!(again.status.success());
    let mut second = msi::open(dir.join("second/Setup.msi")).unwrap();
    for name in ["ProductCode", "UpgradeCode"] {
        assert_eq!(property(&mut second, name), property(&mut package, name), "{name} comes from the app id and version");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn what_an_msi_cant_carry_is_said_and_a_machine_scope_is_per_machine() {
    need_runtime!();
    let extra = "[installer]\nscope = \"machine\"\ndesktop_shortcut = true\n\n[[protocols]]\nscheme = \"hello\"\n\n[[env]]\nname = \"BACHMSIMACHINE_HOME\"\nvalue = \"{app}\"\n";
    let dir = msi_project("msi-machine", "BachMsiMachine", extra);
    let out = run(&dir, &["build", "--fast", "--msi"]);
    let said = text(&out.stdout);
    assert!(out.status.success(), "{}\n{}", said, text(&out.stderr));
    assert!(said.contains("not in the MSI: [[env]] variables") && said.contains("not in the MSI: [[protocols]]"), "{said}");
    let mut package = msi::open(dir.join("dist/Hello-Setup-1.0.0.msi")).unwrap();
    let property = rows(&mut package, "Property");
    assert!(property.iter().any(|row| row[0] == "ALLUSERS" && row[1] == "1"));
    assert!(!property.iter().any(|row| row[0] == "MSIINSTALLPERUSER"));
    assert!(rows(&mut package, "Directory").iter().any(|row| row[0] == "ProgramFiles64Folder"));
    assert_eq!(rows(&mut package, "Shortcut").len(), 2, "Start menu and desktop");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_version_windows_installer_cant_hold_and_a_locked_app_are_refused() {
    need_runtime!();
    let dir = msi_project("msi-refused", "BachMsiRefused", "");
    let out = run(&dir, &["build", "--fast", "--msi", "--app-version", "1.0.0-beta.1"]);
    assert!(!out.status.success());
    assert!(text(&out.stderr).contains("three numbers"), "{}", text(&out.stderr));
    let locked = std::fs::read_to_string(dir.join("bach.toml")).unwrap() + "\n[access]\nencrypt = true\n";
    std::fs::write(dir.join("bach.toml"), locked).unwrap();
    std::fs::write(dir.join("keys.txt"), "correct-horse-battery-staple-1234567890\n").unwrap();
    let out = run(&dir, &["build", "--fast", "--msi", "--key-file", "keys.txt"]);
    assert!(text(&out.stderr).contains("locked with [access]"), "{}", text(&out.stderr));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(windows)]
fn msiexec(args: &[&str]) -> i32 {
    std::process::Command::new("msiexec").args(args).arg("/qn").status().unwrap().code().unwrap_or(-1)
}

/// Removes the throwaway app whatever happens in the test.
#[cfg(windows)]
struct Cleanup(PathBuf);

#[cfg(windows)]
impl Drop for Cleanup {
    fn drop(&mut self) {
        let package: PathBuf = self.0.components().collect();
        let _ = std::process::Command::new("msiexec").arg("/x").arg(package).arg("/qn").status();
    }
}

#[test]
#[ignore = "installs, upgrades and removes a throwaway app with Windows Installer on this computer"]
#[cfg(windows)]
fn windows_installer_installs_upgrades_refuses_a_downgrade_and_removes() {
    need_runtime!();
    let id = "BachMsiLifecycle";
    let extras = "[[components]]
id = \"extras\"
title = \"Extras\"
files = [\"extras/**\"]
";
    let dir = msi_project("msi-lifecycle", id, extras);
    std::fs::create_dir_all(dir.join("app/extras")).unwrap();
    std::fs::write(dir.join("app/extras/more.txt"), "more").unwrap();
    let build = |version: &str, text_in_file: &str| -> PathBuf {
        std::fs::write(dir.join("app/data/readme.txt"), text_in_file).unwrap();
        let out = format!("v{version}/Setup.exe");
        assert!(run(&dir, &["build", "--fast", "--msi", "--app-version", version, "--out", &out]).status.success());
        // Backslashes all the way: msiexec reads a path with a forward slash as an option.
        dir.join(format!("v{version}")).join("Setup.msi")
    };
    let (old, new) = (build("1.0.0", "old"), build("1.1.0", "new"));
    let _cleanup = Cleanup(new.clone());
    let installed = |name: &str| std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap().join("Programs").join(id).join(name);
    // The log's path is its own argument: attached to /l*v it is read as more flags, and msiexec shows its help and waits.
    let log = |name: &str| dir.join(name).display().to_string();
    let path = |msi: &Path| msi.components().collect::<PathBuf>().display().to_string();

    assert_eq!(msiexec(&["/i", &path(&old), "/l*v", &log("install.log")]), 0, "{}", read_log(&dir.join("install.log")));
    assert_eq!(std::fs::read_to_string(installed("data/readme.txt")).unwrap(), "old");
    assert!(installed("hello.exe").is_file() && installed("data/long file name with spaces.txt").is_file());

    // A newer package replaces it in place.
    assert_eq!(msiexec(&["/i", &path(&new), "/l*v", &log("upgrade.log")]), 0, "{}", read_log(&dir.join("upgrade.log")));
    assert_eq!(std::fs::read_to_string(installed("data/readme.txt")).unwrap(), "new");

    // An older one doesn't replace a newer one.
    let refused = msiexec(&["/i", &path(&old), "/l*v", &log("downgrade.log")]);
    assert!(refused == 1603 || refused == 1638, "the downgrade ended with {refused}");
    assert_eq!(std::fs::read_to_string(installed("data/readme.txt")).unwrap(), "new");

    // The optional component is a feature of the same name: left out by default, installed with ADDLOCAL.
    assert!(!installed("extras/more.txt").exists(), "an unticked component isn't installed");
    assert_eq!(msiexec(&["/i", &path(&new), "/l*v", &log("addlocal.log"), "ADDLOCAL=ALL"]), 0, "{}", read_log(&dir.join("addlocal.log")));
    assert_eq!(std::fs::read_to_string(installed("extras/more.txt")).unwrap(), "more");

    // Removing leaves nothing of it.
    assert_eq!(msiexec(&["/x", &path(&new), "/l*v", &log("remove.log")]), 0, "{}", read_log(&dir.join("remove.log")));
    assert!(!installed("hello.exe").exists() && !installed("").exists(), "the files and the folder are gone");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(windows)]
fn read_log(path: &Path) -> String {
    let log = std::fs::read(path)
        .map(|bytes| {
            String::from_utf16_lossy(
                &bytes.chunks(2).filter(|pair| pair.len() == 2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect::<Vec<_>>(),
            )
        })
        .unwrap_or_default();
    log.lines().filter(|line| line.contains("Return value 3") || line.contains("Error ")).take(3).collect::<Vec<_>>().join("\n")
}

#[test]
fn the_apps_icon_goes_into_the_package_for_settings_apps_and_the_shortcuts() {
    need_runtime!();
    let toml = config("")
        .replace("executable = \"hello.exe\"", "executable = \"hello.exe\"\nicon = \"bach.ico\"")
        .replace("BachIntegrationTest-Hello", "BachMsiIcon");
    let dir = project("msi-icon", &toml);
    let icon = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../setup/icon/bach.ico")).unwrap();
    std::fs::write(dir.join("bach.ico"), &icon).unwrap();
    assert!(run(&dir, &["build", "--fast", "--msi"]).status.success());
    let mut package = msi::open(dir.join("dist/Hello-Setup-1.0.0.msi")).unwrap();
    assert_eq!(rows(&mut package, "Icon").len(), 1);
    assert!(rows(&mut package, "Property").iter().any(|row| row[0] == "ARPPRODUCTICON" && row[1] == "AppIcon.ico"));
    assert!(rows(&mut package, "Shortcut").iter().all(|row| row[8] == "AppIcon.ico"));
    let mut stored = Vec::new();
    package.read_stream("Icon.AppIcon.ico").unwrap().read_to_end(&mut stored).unwrap();
    assert_eq!(stored, icon, "the icon comes out as it went in");
    std::fs::remove_dir_all(&dir).unwrap();
}
