//! `[arch.arm64]` read through the whole config loader.

use super::*;

const WITH_ARM: &str = r#"
    [app]
    name = "Hello"
    version = "1.2.3"
    publisher = "Example"
    executable = "hello.exe"

    [files]
    source = "build"
    exclude = ["*.pdb"]

    [arch.arm64]
    source = "build-arm"
    runtime = "tools/arm64/bach-setup.exe"
"#;

fn with_arm_folder(name: &str) -> PathBuf {
    let dir = project_dir(name);
    std::fs::create_dir_all(dir.join("build-arm")).unwrap();
    std::fs::write(dir.join("build-arm/hello.exe"), b"MZ-arm").unwrap();
    std::fs::write(dir.join("build-arm/arm-only.dll"), b"dll").unwrap();
    dir
}

fn as_arch(text: &str, dir: &Path, arch: &str) -> Result<Project> {
    parse_with(text, dir, &Overrides { arch: Some(arch.into()), ..Overrides::default() })
}

#[test]
fn the_main_build_knows_which_architectures_to_add() {
    let dir = with_arm_folder("arch-main");
    let project = parse(WITH_ARM, &dir).unwrap();
    assert_eq!(project.arch, None);
    assert_eq!(project.arches, vec![ArchBuild { name: "arm64".into(), runtime: Some(dir.join("tools/arm64/bach-setup.exe")) }]);
    assert_eq!(project.output, dir.join("dist/Hello-Setup-1.2.3.exe"));
    assert_eq!(project.manifest.files.len(), 3, "the main build packs the main folder");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_arm64_build_packs_its_own_folder_and_names_its_file_after_it() {
    let dir = with_arm_folder("arch-arm");
    let project = as_arch(WITH_ARM, &dir, "arm64").unwrap();
    assert_eq!(project.arch.as_deref(), Some("arm64"));
    let paths: Vec<_> = project.manifest.files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["arm-only.dll", "hello.exe"]);
    assert_eq!(project.output, dir.join("dist/Hello-Setup-1.2.3-arm64.exe"));
    // The app is the same app: one id, one version, so an update of either replaces the other.
    assert_eq!(project.manifest.app.id, "Hello");
    assert!(!project.online, "the online installer is built once, from the main build");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_arch_can_have_its_own_exclusions() {
    let dir = with_arm_folder("arch-exclude");
    std::fs::write(dir.join("build-arm/debug.pdb"), b"x").unwrap();
    let has = |project: &Project, path: &str| project.manifest.files.iter().any(|file| file.path == path);
    let inherited = as_arch(WITH_ARM, &dir, "arm64").unwrap();
    assert!(!has(&inherited, "debug.pdb"), "files.exclude carries over unless the arch says otherwise");
    let own = WITH_ARM.replace("runtime =", "exclude = [\"*.dll\"]
    runtime =");
    let project = as_arch(&own, &dir, "arm64").unwrap();
    assert!(has(&project, "debug.pdb") && !has(&project, "arm-only.dll"), "the arch's own list replaces it");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn mistakes_in_arch_name_their_setting() {
    let dir = with_arm_folder("arch-mistakes");
    let error = as_arch(MINIMAL, &dir, "arm64").err().unwrap().to_string();
    assert!(error.contains("no [arch.arm64]"), "{error}");
    let error = parse(&WITH_ARM.replace("arm64]", "ia64]"), &dir).err().unwrap().to_string();
    assert!(error.contains("[arch.ia64] isn't an architecture"), "{error}");
    // `bach check` finds a wrong folder without building it.
    let error = parse(&WITH_ARM.replace("build-arm", "nowhere"), &dir).err().unwrap().to_string();
    assert!(error.contains("arch.arm64.source is \"nowhere\""), "{error}");
    let error = parse(&WITH_ARM.replace("tools/arm64/bach-setup.exe", " "), &dir).err().unwrap().to_string();
    assert!(error.contains("arch.arm64.runtime is empty"), "{error}");
    let error = parse(&WITH_ARM.replace("source = \"build-arm\"", "src = \"build-arm\""), &dir).err().unwrap().to_string();
    assert!(error.contains("src"), "{error}");
    std::fs::remove_dir_all(&dir).unwrap();
}
