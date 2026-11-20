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
    let registry = rows(&mut package, "Registry");
    let install_path = registry.iter().find(|row| row[3] == "InstallPath").expect("the [[registry]] value");
    assert_eq!(install_path[4], "[INSTALLFOLDER]");
    // Every component that holds no file says it is installed with a registry value of its own.
    assert!(registry.iter().any(|row| row[5] == "cStart") && registry.iter().any(|row| row[5] == "cDesktop"));
    let upgrades = rows(&mut package, "Upgrade");
    assert_eq!(upgrades.len(), 2);
    assert!(upgrades.iter().any(|row| row[6] == "PREVIOUSVERSIONSINSTALLED") && upgrades.iter().any(|row| row[6] == "NEWERPRODUCTFOUND"));
    let sequence = rows(&mut package, "InstallExecuteSequence");
    let at = |name: &str| sequence.iter().find(|row| row[0] == name).map(|row| row[2].parse::<i32>().unwrap()).unwrap();
    assert!(at("FindRelatedProducts") < at("LaunchConditions"), "a newer version has to be found before the condition that refuses it");
    assert!(at("RemoveExistingProducts") < at("InstallInitialize") && at("InstallFiles") < at("CreateShortcuts"));

    // Opened with Windows' interface, the package shows Bach's window instead of the engine's own box.
    assert!(rows(&mut package, "Binary").iter().any(|row| row[0] == "BachFront"), "the window's program is in the package");
    let interface = rows(&mut package, "InstallUISequence");
    let ui_at = |name: &str| interface.iter().find(|row| row[0] == name).map(|row| (row[1].clone(), row[2].parse::<i32>().unwrap()));
    assert!(ui_at("BachWindow").is_some_and(|(condition, _)| condition.contains("UILevel >= 5")));
    assert!(ui_at("BachWindowRemove").is_some_and(|(condition, _)| condition.contains("REMOVE=\"ALL\"")));
    assert!(ui_at("ExecuteAction").is_some_and(|(condition, _)| condition == "UILevel < 5"), "only quieter levels run the engine's own install");
    assert!(ui_at("BachWindow").unwrap().1 > ui_at("CostFinalize").unwrap().1);
    let actions = rows(&mut package, "CustomAction");
    let window = actions.iter().find(|row| row[0] == "BachWindow").expect("the window's action");
    assert!(window[3].contains("--msi-package") && window[3].contains("[OriginalDatabase]") && !window[3].contains("--uninstall"));
    assert!(actions.iter().any(|row| row[0] == "BachWindowRemove" && row[3].ends_with("--uninstall")));

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
fn what_the_setup_file_does_beyond_files_is_in_the_package_too() {
    need_runtime!();
    let extra = "[installer]
scope = \"machine\"
desktop_shortcut = \"never\"
start_with_windows = true
add_to_path = \"always\"
start_menu_folder = \"Example Co\"

        [[protocols]]
scheme = \"hello\"

[[file_types]]
extension = \".hello\"

        [[context_menu]]
name = \"Open with Hello\"
on = [\"files\"]

        [[env]]
name = \"BACHMSIMACHINE_HOME\"
value = \"{app}\"

        [[run]]
name = \"Set up\"
file = \"hello.exe\"
arguments = \"--setup\"
when = \"first_install\"

        [[run]]
name = \"Clean up\"
file = \"hello.exe\"
when = \"uninstall\"

        [[requirements]]
kind = \"windows\"
build = 17763
";
    let dir = msi_project("msi-machine", "BachMsiMachine", extra);
    let out = run(&dir, &["build", "--fast", "--msi"]);
    let said = text(&out.stdout);
    assert!(
        out.status.success(),
        "{}
{}",
        said,
        text(&out.stderr)
    );
    assert!(!said.contains("not in the MSI"), "everything in this config has a place in the package: {said}");
    let mut package = msi::open(dir.join("dist/Hello-Setup-1.0.0.msi")).unwrap();
    let property = rows(&mut package, "Property");
    assert!(property.iter().any(|row| row[0] == "ALLUSERS" && row[1] == "1"));
    assert!(!property.iter().any(|row| row[0] == "MSIINSTALLPERUSER"));
    assert!(rows(&mut package, "Directory").iter().any(|row| row[0] == "ProgramFiles64Folder"));
    let shortcuts = rows(&mut package, "Shortcut");
    assert_eq!(shortcuts.len(), 1, "Start menu only: the desktop shortcut is never offered");
    assert_eq!(shortcuts[0][1], "StartMenuDir", "in the Start menu folder the config names");
    assert!(rows(&mut package, "RemoveFolder").iter().any(|row| row[2] == "StartMenuDir"));

    // Options of the install page are features named like the setup file's --with ids.
    let features: Vec<String> = rows(&mut package, "Feature").into_iter().map(|row| row[0].clone()).collect();
    assert!(features.contains(&"Main".to_string()));
    let registry = rows(&mut package, "Registry");
    let at = |key: &str, name: &str| registry.iter().any(|row| row[2] == key && row[3] == name);
    assert!(
        registry.iter().any(|row| row[2] == r"Software\Microsoft\Windows\CurrentVersion\Run" && row[3] == "BachMsiMachine"),
        "start with Windows"
    );
    assert!(at(r"Software\Classes\hello", "URL Protocol"), "the link scheme");
    assert!(registry.iter().any(|row| row[2].ends_with(r"\.hello\OpenWithProgids")), "the file type's Open with entry");
    assert!(
        registry.iter().any(|row| row[2].starts_with(r"Software\Classes\*\shell\Bach.BachMsiMachine.OpenwithHello")),
        "the right-click entry"
    );
    let environment = rows(&mut package, "Environment");
    assert!(environment.iter().any(|row| row[1] == "=-BACHMSIMACHINE_HOME" && row[2] == "[INSTALLFOLDER]"));
    assert!(environment.iter().any(|row| row[1] == "=-PATH" && row[2] == "[~];[INSTALLFOLDER]"), "{environment:?}");
    let actions = rows(&mut package, "CustomAction");
    assert!(actions.iter().any(|row| row[0] == "Run1" && row[1] == "1042"), "a deferred run of a packaged program: {actions:?}");
    assert!(actions.iter().any(|row| row[0] == "Run2" && row[1] == "1106"), "removal never stops for a failed step");
    assert!(actions.iter().any(|row| row[0] == "SetARPINSTALLLOCATION" && row[1] == "51"));
    let launch = rows(&mut package, "LaunchCondition");
    assert!(launch.iter().any(|row| row[0] == "WindowsBuild >= 17763"), "{launch:?}");
    let sequence = rows(&mut package, "InstallExecuteSequence");
    let seq = |name: &str| sequence.iter().find(|row| row[0] == name).map(|row| (row[1].clone(), row[2].parse::<i32>().unwrap())).unwrap();
    assert!(
        seq("Run1").1 > seq("InstallFiles").1 && seq("Run2").1 < seq("RemoveFiles").1,
        "install steps after the files, removal steps before them"
    );
    assert_eq!(seq("Run1").0, "NOT Installed AND NOT PREVIOUSVERSIONSINSTALLED");
    assert_eq!(seq("Run2").0, "REMOVE=\"ALL\" AND NOT UPGRADINGPRODUCTCODE");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn what_an_msi_still_cant_carry_is_said() {
    need_runtime!();
    let extra = "[[migrate]]
uninstall_key = \"OldApp_is1\"

[uninstall]
data = ['%APPDATA%\\BachMsiLeft']

        [[requirements]]
kind = \"webview2\"

[[run]]
name = \"Strict\"
file = \"hello.exe\"
when = \"install\"
success_codes = [0, 3010]
";
    let dir = msi_project("msi-skipped", "BachMsiLeft", extra);
    let out = run(&dir, &["build", "--fast", "--msi", "--json"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let object: serde_json::Value = serde_json::from_str(&text(&out.stdout)).unwrap();
    let skipped: Vec<&str> = object["msi_skipped"].as_array().unwrap().iter().map(|item| item.as_str().unwrap()).collect();
    for what in ["[[migrate]]", "[uninstall] data folders", "the requirement", "the exit codes"] {
        assert!(skipped.iter().any(|item| item.contains(what)), "{what} should be said: {skipped:?}");
    }
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

/// msiexec with Windows' full interface (what opening the package gives), with the package's window
/// told to work without showing itself, and a time limit so a stuck window can't hold the test.
#[cfg(windows)]
fn msiexec_with_window(args: &[&str]) -> i32 {
    let mut child = std::process::Command::new("msiexec").args(args).arg("/qf").env("BACH_MSI_FRONT_SILENT", "1").spawn().unwrap();
    let started = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code().unwrap_or(-1);
        }
        if started.elapsed() > std::time::Duration::from_secs(240) {
            let _ = child.kill();
            return -2;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

#[cfg(windows)]
fn log_text(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap_or_default();
    let wide = bytes.len() > 2 && bytes[0] == 0xFF && bytes[1] == 0xFE;
    if wide {
        String::from_utf16_lossy(&bytes.chunks(2).skip(1).filter(|pair| pair.len() == 2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect::<Vec<_>>())
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

#[test]
#[ignore = "installs, repairs and removes a throwaway app with Windows Installer on this computer"]
#[cfg(windows)]
fn opening_the_package_shows_bachs_window_which_installs_and_removes_through_windows_installer() {
    need_runtime!();
    let id = "BachMsiWindow";
    let extras = "[installer]
start_with_windows = false
";
    let dir = msi_project("msi-window", id, extras);
    assert!(run(&dir, &["build", "--fast", "--msi", "--out", "w/Setup.exe"]).status.success());
    let package = dir.join("w").join("Setup.msi").components().collect::<PathBuf>();
    let package = package.display().to_string();
    let _cleanup = Cleanup(PathBuf::from(&package));
    let installed = |name: &str| std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap().join("Programs").join(id).join(name);
    let log = |name: &str| dir.join(name).display().to_string();
    let engine_log = std::env::temp_dir().join(format!("{id}-msi.log"));
    let setup_log = std::env::temp_dir().join(format!("{id}-setup.log"));
    let _ = std::fs::remove_file(&engine_log);
    let _ = std::fs::remove_file(&setup_log);

    // The package opened with its interface: the window's copy of setup runs, and Windows Installer
    // (called by it) does the install; the outer run ends without installing a second time.
    let code = msiexec_with_window(&["/i", &package, "/l*v", &log("outer.log")]);
    let outer = log_text(&dir.join("outer.log"));
    assert_eq!(code, 0, "{outer}");
    assert!(installed("hello.exe").is_file() && installed("data/readme.txt").is_file(), "the files are in");
    assert!(outer.contains("BachWindow"), "the window's action ran: {outer}");
    assert!(!outer.contains("Doing action: InstallFiles"), "the outer run doesn't install by itself");
    let setup = log_text(&setup_log);
    assert!(setup.contains("Windows Installer installs") && setup.contains("Windows Installer finished: Done"), "{setup}");
    assert!(log_text(&engine_log).contains("InstallFiles"), "the engine did the work, and logged it");

    // Opened again, it is the same version: the window repairs instead.
    std::fs::write(installed("data/readme.txt"), "damaged").unwrap();
    let repaired = msiexec_with_window(&["/i", &package, "/l*v", &log("again.log")]);
    assert_eq!(repaired, 0, "{}", log_text(&dir.join("again.log")));

    // Removing it under Settings > Apps is the same interface, with the removal window.
    let removed = msiexec_with_window(&["/x", &package, "/l*v", &log("remove.log")]);
    assert_eq!(removed, 0, "{}", log_text(&dir.join("remove.log")));
    assert!(!installed("hello.exe").exists() && !installed("").exists(), "the files and the folder are gone");
    // And a quiet install never runs the interface sequence.
    assert_eq!(msiexec(&["/i", &package, "/l*v", &log("quiet.log")]), 0);
    assert!(installed("hello.exe").is_file());
    assert!(!log_text(&dir.join("quiet.log")).contains("BachWindow"));
    assert_eq!(msiexec(&["/x", &package]), 0);
    assert!(!installed("hello.exe").exists());
    std::fs::remove_dir_all(&dir).ok();
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

/// A registry value as `reg query` prints it, or None when it isn't there.
#[cfg(windows)]
fn reg_value(key: &str, name: &str) -> Option<String> {
    let out = std::process::Command::new("reg")
        .args(["query", key])
        .args(if name.is_empty() { vec!["/ve"] } else { vec!["/v", name] })
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let line = text.lines().find(|line| line.trim_start().starts_with(if name.is_empty() { "(" } else { name }))?;
    let mut parts = line.trim().splitn(3, "    ");
    parts.next()?;
    parts.next()?;
    Some(parts.next().unwrap_or("").trim().to_string())
}

#[test]
#[ignore = "installs and removes a throwaway app with Windows Installer on this computer"]
#[cfg(windows)]
fn windows_installer_does_everything_to_windows_the_setup_file_does() {
    need_runtime!();
    let id = "BachMsiEverything";
    let extra = "[installer]\nadd_to_path = true\nstart_with_windows = true\nstart_menu_folder = \"Bach Msi Test Co\"\n\n\
        [[protocols]]\nscheme = \"bachmsieverything\"\n\n\
        [[file_types]]\nextension = \".bmsi\"\n\n\
        [[context_menu]]\nname = \"Open with Everything\"\non = [\"files\"]\n\n\
        [[env]]\nname = \"BACHMSIEVERYTHING_HOME\"\nvalue = \"{app}\"\n\n\
        [[run]]\nname = \"Set up\"\nfile = \"tool.exe\"\narguments = '/c echo ran> \"{app}ran.txt\"'\nwhen = \"install\"\n\n\
        [[run]]\nname = \"Clean up\"\nfile = \"tool.exe\"\narguments = '/c echo bye> \"%TEMP%\\bachmsi-everything-bye.txt\"'\nwhen = \"uninstall\"\n";
    let dir = msi_project("msi-everything", id, extra);
    std::fs::copy(std::env::var_os("SystemRoot").map(PathBuf::from).unwrap().join("System32").join("cmd.exe"), dir.join("app/tool.exe"))
        .unwrap();
    let out = run(&dir, &["build", "--fast", "--msi", "--out", "pkg/Setup.exe"]);
    assert!(out.status.success(), "{}\n{}", text(&out.stdout), text(&out.stderr));
    let package = dir.join("pkg").join("Setup.msi");
    let path_of = |msi: &Path| msi.components().collect::<PathBuf>().display().to_string();
    let _cleanup = Cleanup(package.clone());
    let folder = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap().join("Programs").join(id);
    let folder_text = folder.display().to_string();
    let bye = std::env::temp_dir().join("bachmsi-everything-bye.txt");
    let _ = std::fs::remove_file(&bye);
    let log = dir.join("install.log").display().to_string();

    assert_eq!(msiexec(&["/i", &path_of(&package), "/l*v", &log]), 0, "{}", read_log(&dir.join("install.log")));
    assert!(folder.join("hello.exe").is_file());
    // The run step ran after the files were there, in the install folder's name.
    assert!(std::fs::read_to_string(folder.join("ran.txt")).unwrap().starts_with("ran"));
    // Environment: the variable, and the folder on PATH (the rest of PATH kept).
    let home = reg_value(r"HKCU\Environment", "BACHMSIEVERYTHING_HOME").expect("the variable");
    assert!(home.trim_end_matches('\\').eq_ignore_ascii_case(&folder_text), "{home}");
    let path = reg_value(r"HKCU\Environment", "Path").expect("PATH");
    assert!(path.to_ascii_lowercase().contains(&folder_text.to_ascii_lowercase()), "{path}");
    // Start with Windows, the link scheme, the file type and the right-click entry.
    let run_value = reg_value(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", id).expect("the Run entry");
    assert!(run_value.to_ascii_lowercase().contains("hello.exe"), "{run_value}");
    let command = reg_value(r"HKCU\Software\Classes\bachmsieverything\shell\open\command", "").expect("the link scheme");
    assert!(command.contains("hello.exe") && command.contains("%1"), "{command}");
    assert!(reg_value(r"HKCU\Software\Classes\.bmsi\OpenWithProgids", &format!("{id}.bmsi")).is_some());
    assert!(reg_value(&format!(r"HKCU\Software\Classes\*\shell\Bach.{id}.OpenwithEverything\command"), "").is_some());
    // Settings > Apps knows where the app is, and the Start menu has the app's own folder.
    let start = std::env::var_os("APPDATA").map(PathBuf::from).unwrap().join(r"Microsoft\Windows\Start Menu\Programs\Bach Msi Test Co");
    assert!(start.join("Hello.lnk").is_file(), "{}", start.display());

    assert_eq!(
        msiexec(&["/x", &path_of(&package), "/l*v", &dir.join("remove.log").display().to_string()]),
        0,
        "{}",
        read_log(&dir.join("remove.log"))
    );
    // What the install put there is gone; what a run step wrote (ran.txt) is the app's own business, as with the setup file.
    assert!(!folder.join("hello.exe").exists());
    let left: Vec<_> = std::fs::read_dir(&folder)
        .map(|entries| entries.flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    assert!(left.iter().all(|name| name == "ran.txt"), "{left:?}");
    let _ = std::fs::remove_dir_all(&folder);
    assert!(std::fs::read_to_string(&bye).unwrap().starts_with("bye"), "the removal step ran before the files went");
    let _ = std::fs::remove_file(&bye);
    assert!(reg_value(r"HKCU\Environment", "BACHMSIEVERYTHING_HOME").is_none(), "the variable is gone");
    let path = reg_value(r"HKCU\Environment", "Path").unwrap_or_default();
    assert!(!path.to_ascii_lowercase().contains(&folder_text.to_ascii_lowercase()), "PATH is as it was: {path}");
    assert!(reg_value(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", id).is_none());
    assert!(reg_value(r"HKCU\Software\Classes\bachmsieverything\shell\open\command", "").is_none());
    assert!(!start.exists(), "the Start menu folder is gone");
    std::fs::remove_dir_all(&dir).unwrap();
}
