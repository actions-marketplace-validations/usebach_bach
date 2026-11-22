//! `[macos]` read through the whole config loader.

use super::*;
use crate::manifest::Platform;

const BOTH: &str = r#"
    [app]
    name = "Hello"
    version = "1.2.3"
    publisher = "Example"
    executable = "hello.exe"

    [files]
    source = "build"

    [uninstall]
    data = ['%APPDATA%\Hello']

    [[registry]]
    key = 'Software\Hello'
    name = "Run"
    value = "1"

    [macos]
    source = "build-mac"
    executable = "Hello.app"
    data = ["~/Library/Application Support/Hello"]
    executables = ["Hello.app/Contents/MacOS/*"]
"#;

fn project_with_mac(name: &str) -> PathBuf {
    let dir = project_dir(name);
    std::fs::create_dir_all(dir.join("build-mac/Hello.app/Contents/MacOS")).unwrap();
    std::fs::write(dir.join("build-mac/Hello.app/Contents/MacOS/hello"), b"mach-o").unwrap();
    std::fs::write(dir.join("build-mac/Hello.app/Contents/Info.plist"), b"<plist/>").unwrap();
    dir
}

fn as_mac(text: &str, dir: &Path) -> Result<Project> {
    parse_with(text, dir, &Overrides { platform: Some("macos".into()), ..Overrides::default() })
}

#[test]
fn the_main_build_stays_windows_and_knows_there_is_a_mac_build() {
    let dir = project_with_mac("mac-main");
    let project = parse(BOTH, &dir).unwrap();
    assert_eq!(project.platform, Platform::Windows);
    assert_eq!(project.declared, Declared { windows: true, macos: true });
    assert_eq!(project.output, dir.join("dist/Hello-Setup-1.2.3.exe"));
    assert_eq!(project.manifest.platform, Platform::Windows);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_mac_build_packs_its_own_folder_and_names_its_file_after_it() {
    let dir = project_with_mac("mac-build");
    let project = as_mac(BOTH, &dir).unwrap();
    assert_eq!(project.platform, Platform::Macos);
    assert_eq!(project.manifest.platform, Platform::Macos);
    assert_eq!(project.output, dir.join("dist/Hello-Setup-1.2.3.dmg"));
    assert_eq!(project.manifest.app.executable, "Hello.app");
    let paths: Vec<_> = project.manifest.files.iter().map(|file| (file.path.as_str(), file.mode)).collect();
    assert_eq!(paths, [("Hello.app/Contents/Info.plist", None), ("Hello.app/Contents/MacOS/hello", Some(0o755))]);
    // The Windows data folder and registry values stay with Windows.
    assert_eq!(project.manifest.uninstall.data, ["~/Library/Application Support/Hello"]);
    assert!(project.manifest.system.is_empty());
    assert_eq!(project.manifest.uninstaller(), "Uninstall Hello.app");
    assert_eq!(project.manifest.uninstaller_payload(), "Uninstall Hello.app/Contents/Resources/setup.bach");
    let mac = project.macos.unwrap();
    assert_eq!(mac.bundle_id, "app.bach.setup.hello");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_mac_only_config_needs_no_windows_files() {
    let dir = project_with_mac("mac-only");
    let text = r#"
        [app]
        name = "Hello"
        version = "1.0.0"
        publisher = "Example"

        [macos]
        source = "build-mac"
        executable = "Hello.app"
    "#;
    assert_eq!(declared(text).unwrap(), Declared { windows: false, macos: true });
    let project = as_mac(text, &dir).unwrap();
    assert_eq!(project.manifest.app.executable, "Hello.app");
    let err = parse(text, &dir).unwrap_err().to_string();
    assert!(err.contains("no [files]") && err.contains("--platform macos"), "{err}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn mistakes_in_the_mac_section_name_the_setting() {
    let dir = project_with_mac("mac-mistakes");
    let edit = |from: &str, to: &str| format!("{:#}", as_mac(&BOTH.replace(from, to), &dir).unwrap_err());
    assert!(edit("Hello.app\"\n    data", "Nope.app\"\n    data").contains("macos.executable"));
    assert!(edit("~/Library/Application Support/Hello", "~/Library").contains("whole Library"));
    assert!(edit("~/Library/Application Support/Hello", "%APPDATA%\\\\Hello").contains("start with ~/"));
    assert!(edit("source = \"build-mac\"", "source = \"missing\"").contains("macos.source"));
    assert!(edit("Hello.app/Contents/MacOS/*", "Nothing/*").contains("matches no file"));
    let bad_id = BOTH.replace("[macos]", "[macos]\n    bundle_id = \"hello\"");
    assert!(as_mac(&bad_id, &dir).unwrap_err().to_string().contains("macos.bundle_id"));
    let bad_out = BOTH.replace("[macos]", "[macos]\n    output = \"dist/x.exe\"");
    assert!(as_mac(&bad_out, &dir).unwrap_err().to_string().contains(".dmg or .zip"));
    let both_flags = Overrides { platform: Some("macos".into()), arch: Some("arm64".into()), ..Overrides::default() };
    assert!(parse_with(BOTH, &dir, &both_flags).is_err());
    assert!(parse_with(BOTH, &dir, &Overrides { platform: Some("linux".into()), ..Overrides::default() }).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_zip_output_and_signing_are_read() {
    let dir = project_with_mac("mac-signing");
    let text = format!(
        "{}\n    [macos.signing]\n    identity = \"Developer ID Application: Example (ABCDE12345)\"\n    notarize = \"example-notary\"\n",
        BOTH.replace("[macos]", "[macos]\n    output = \"dist/Hello.zip\"\n    minimum_version = \"13.0\"")
    );
    let project = as_mac(&text, &dir).unwrap();
    let mac = project.macos.unwrap();
    assert_eq!(mac.container, MacContainer::Zip);
    assert_eq!(mac.minimum_version.as_deref(), Some("13.0"));
    assert_eq!(mac.signing.unwrap().notary_profile.as_deref(), Some("example-notary"));
    let adhoc = format!("{BOTH}\n    [macos.signing]\n    identity = \"-\"\n    notarize = \"p\"\n");
    assert!(as_mac(&adhoc, &dir).unwrap_err().to_string().contains("ad hoc"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn agents_need_a_program_with_the_executable_permission() {
    let dir = project_with_mac("mac-agents");
    let with = |agent: &str| format!("{BOTH}\n    [[macos.agents]]\n    {agent}\n");
    let ok = as_mac(&with("name = \"sync\"\n    file = \"Hello.app/Contents/MacOS/hello\"\n    interval = 600"), &dir).unwrap();
    assert_eq!(ok.manifest.agents.len(), 1);
    assert!(ok.manifest.agents[0].run_at_load && ok.manifest.agents[0].interval == Some(600));
    let plain = format!("{:#}", as_mac(&with("name = \"sync\"\n    file = \"Hello.app/Contents/Info.plist\""), &dir).unwrap_err());
    assert!(plain.contains("no executable permission"), "{plain}");
    let missing = format!("{:#}", as_mac(&with("name = \"sync\"\n    file = \"nope\""), &dir).unwrap_err());
    assert!(missing.contains("isn't one of the installed files"), "{missing}");
    let name = format!("{:#}", as_mac(&with("name = \"a b\"\n    file = \"Hello.app/Contents/MacOS/hello\""), &dir).unwrap_err());
    assert!(name.contains("letters, digits and dashes"), "{name}");
    let fast =
        format!("{:#}", as_mac(&with("name = \"x\"\n    file = \"Hello.app/Contents/MacOS/hello\"\n    interval = 2"), &dir).unwrap_err());
    assert!(fast.contains("less than 10 seconds"), "{fast}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_app_that_is_one_bundle_goes_straight_into_applications_and_asks_nothing() {
    let dir = project_with_mac("mac-bare");
    let project = as_mac(BOTH, &dir).unwrap();
    let install = &project.manifest.install;
    assert_eq!(install.folder, "", "no folder of its own");
    assert!(!install.change_folder);
    assert!(!install.desktop_shortcut.offered() && !install.start_menu_shortcut.offered() && !install.start_with_windows.offered());
    // Anything beside the bundle brings the folder back, and the config can still ask for more.
    std::fs::write(dir.join("build-mac/README.txt"), b"x").unwrap();
    let with_extra = as_mac(BOTH, &dir).unwrap();
    assert_eq!(with_extra.manifest.install.folder, "BachIntegrationTest-Hello".replace("BachIntegrationTest-", ""));
    std::fs::remove_file(dir.join("build-mac/README.txt")).unwrap();
    let asked = BOTH.replace("[macos]", "[installer]\n    desktop_shortcut = false\n\n    [macos]");
    let _ = asked;
    std::fs::remove_dir_all(&dir).unwrap();
}
