//! Profiles read through the whole config loader.

use super::*;

const WITH_PROFILES: &str = r#"
    [app]
    name = "Hello"
    version = "1.2.3"
    publisher = "Example"
    executable = "hello.exe"

    [files]
    source = "build"
    exclude = ["*.pdb"]

    [[components]]
    id = "samples"
    title = "Samples"
    files = ["samples/**"]

    [profiles.enterprise.installer]
    scope = "machine"
    desktop_shortcut = false

    [profiles.portable]
    kind = "zip"

    [[profiles.portable.components]]
    id = "samples"
    title = "Samples"
    files = ["samples/**"]
    default = true

    [profiles.slim.files]
    exclude = ["*.pdb", "sub/**"]
"#;

fn as_profile(text: &str, dir: &Path, name: &str) -> Result<Project> {
    parse_with(text, dir, &Overrides { profile: Some(name.into()), ..Overrides::default() })
}

#[test]
fn the_main_build_ignores_the_profiles() {
    let dir = project_dir("profiles-main");
    let project = parse(WITH_PROFILES, &dir).unwrap();
    assert_eq!((project.profile, project.kind), (None, ProfileKind::Setup));
    assert_eq!(project.manifest.install.scope, crate::manifest::Scope::User);
    assert_eq!(project.output, dir.join("dist/Hello-Setup-1.2.3.exe"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_setup_profile_overrides_settings_and_names_its_file_after_itself() {
    let dir = project_dir("profiles-enterprise");
    let project = as_profile(WITH_PROFILES, &dir, "enterprise").unwrap();
    assert_eq!(project.profile.as_deref(), Some("enterprise"));
    assert!(project.manifest.install.scope.is_machine());
    assert_eq!(project.manifest.install.desktop_shortcut, Choice::Off);
    assert_eq!(project.output, dir.join("dist/Hello-enterprise-Setup-1.2.3.exe"));
    assert_eq!(project.online_output, dir.join("dist/Hello-enterprise-Web-Setup.exe"));
    // What it didn't name is as in the main file.
    assert_eq!(project.manifest.files.len(), 3);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_zip_profile_picks_its_files_by_its_own_components() {
    let dir = project_dir("profiles-portable");
    let project = as_profile(WITH_PROFILES, &dir, "portable").unwrap();
    assert_eq!(project.kind, ProfileKind::Zip);
    assert_eq!(project.output, dir.join("dist/Hello-portable-1.2.3.zip"));
    assert!(project.manifest.components[0].default, "the profile ticks the component");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_profile_can_change_which_files_are_packed() {
    let dir = project_dir("profiles-slim");
    let slim = as_profile(WITH_PROFILES, &dir, "slim").unwrap();
    let paths: Vec<_> = slim.manifest.files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["hello.exe", "samples/a.txt"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_profile_names_its_output_or_gets_the_wrong_extension_named() {
    let dir = project_dir("profiles-output");
    let named = WITH_PROFILES.replace("kind = \"zip\"", "kind = \"zip\"\n    output = \"dist/{name}-Portable.zip\"");
    assert_eq!(as_profile(&named, &dir, "portable").unwrap().output, dir.join("dist/Hello-Portable.zip"));
    let wrong = WITH_PROFILES.replace("kind = \"zip\"", "kind = \"zip\"\n    output = \"dist/x.exe\"");
    let error = as_profile(&wrong, &dir, "portable").err().unwrap().to_string();
    assert!(error.contains("profiles.portable.output must end in .zip"), "{error}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_profile_that_doesnt_exist_or_is_broken_is_one_sentence() {
    let dir = project_dir("profiles-broken");
    let error = as_profile(WITH_PROFILES, &dir, "nope").err().unwrap().to_string();
    assert!(error.contains("its profiles are enterprise, portable, slim"), "{error}");
    // A broken profile is found by the main build too, so `bach check` catches it.
    let broken = format!("{WITH_PROFILES}\n[profiles.odd]\nkind = \"msi\"\n");
    let error = parse(&broken, &dir).err().unwrap().to_string();
    assert!(error.contains("profiles.odd.kind"), "{error}");
    // A setting that doesn't exist, inside a profile, is named.
    let typo = WITH_PROFILES.replace("scope = \"machine\"", "scoop = \"machine\"");
    let error = as_profile(&typo, &dir, "enterprise").err().unwrap().to_string();
    assert!(error.contains("scoop"), "{error}");
    assert!(error.contains("enterprise"), "{error}");
    std::fs::remove_dir_all(&dir).unwrap();
}
