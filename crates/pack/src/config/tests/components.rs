//! `installer.shared_files`: files that more than one component claims.

use super::*;
use crate::manifest::{Choices, FileEntry};

/// Two switches over every vendor, then one component per vendor (issue 36).
const VENDORS: &str = r#"
    [app]
    name = "Hello"
    version = "1.2.3"
    publisher = "Example"
    executable = "hello.exe"

    [files]
    source = "build"
    exclude = ["*.pdb", "sub/**", "samples/**"]

    [installer]
    shared_files = "all"

    [[components]]
    id = "app"
    title = "Applications"
    files = ["vendor*/app/**"]
    default = true

    [[components]]
    id = "tools"
    title = "Tools"
    files = ["vendor*/tools/**"]

    [[components]]
    id = "vendor1"
    title = "Vendor 1"
    files = ["vendor1/**"]
    default = true

    [[components]]
    id = "vendor2"
    title = "Vendor 2"
    files = ["vendor2/**"]
"#;

fn vendors(name: &str) -> PathBuf {
    let dir = project_dir(name);
    for vendor in ["vendor1", "vendor2"] {
        for (folder, file) in [("app", "app.txt"), ("tools", "tool.txt")] {
            std::fs::create_dir_all(dir.join("build").join(vendor).join(folder)).unwrap();
            std::fs::write(dir.join("build").join(vendor).join(folder).join(file), b"x").unwrap();
        }
        std::fs::write(dir.join("build").join(vendor).join("about.txt"), b"x").unwrap();
    }
    dir
}

fn entry<'a>(project: &'a Project, path: &str) -> &'a FileEntry {
    project.manifest.files.iter().find(|file| file.path == path).unwrap()
}

fn installed(project: &Project, ticked: &[&str]) -> Vec<String> {
    let choices = Choices { components: ticked.iter().map(|id| id.to_string()).collect(), ..Choices::default() };
    let mut paths: Vec<String> =
        project.manifest.files.iter().filter(|file| choices.installs(file)).map(|file| file.path.clone()).collect();
    paths.sort();
    paths
}

#[test]
fn a_shared_file_is_installed_only_when_every_component_claiming_it_is_ticked() {
    let dir = vendors("shared-all");
    let project = parse(VENDORS, &dir).unwrap();
    let claims = |path: &str| entry(&project, path).claimed_by().collect::<Vec<_>>();
    assert_eq!(claims("vendor1/tools/tool.txt"), ["tools", "vendor1"]);
    assert_eq!(claims("vendor2/app/app.txt"), ["app", "vendor2"]);
    assert_eq!(claims("vendor2/about.txt"), ["vendor2"]);
    assert!(claims("hello.exe").is_empty());

    assert_eq!(installed(&project, &[]), ["hello.exe"]);
    assert_eq!(installed(&project, &["app", "tools"]), ["hello.exe"], "no vendor, nothing of any vendor");
    assert_eq!(installed(&project, &["app", "vendor1"]), ["hello.exe", "vendor1/about.txt", "vendor1/app/app.txt"]);
    assert_eq!(installed(&project, &["tools", "vendor2"]), ["hello.exe", "vendor2/about.txt", "vendor2/tools/tool.txt"]);
    assert_eq!(installed(&project, &["app", "tools", "vendor1", "vendor2"]).len(), 7);
    // A first install ticks the apps and Vendor 1, and the window offers each choice once.
    let defaults = Choices::defaults(&project.manifest);
    assert_eq!(defaults.components, ["app", "vendor1"]);
    assert_eq!(project.manifest.components.len(), 4);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_order_of_the_components_does_not_matter_for_what_is_installed() {
    let dir = vendors("shared-order");
    // The switch after the vendors, which "first" refuses: it would match nothing.
    let late = r#"
        [app]
        name = "Hello"
        version = "1.2.3"
        publisher = "Example"
        executable = "hello.exe"
        [files]
        source = "build"
        [installer]
        shared_files = "all"
        [[components]]
        id = "vendor1"
        title = "Vendor 1"
        files = ["vendor1/**"]
        [[components]]
        id = "tools"
        title = "Tools"
        files = ["vendor*/tools/**"]
    "#;
    let project = parse(late, &dir).unwrap();
    assert!(!installed(&project, &["vendor1"]).contains(&"vendor1/tools/tool.txt".to_string()));
    assert!(installed(&project, &["vendor1", "tools"]).contains(&"vendor1/tools/tool.txt".to_string()));

    for first in [late.replace("shared_files = \"all\"", "shared_files = \"first\""), late.replace("shared_files = \"all\"", "")] {
        let error = parse(&first.replace("vendor*/tools/**", "vendor1/tools/**"), &dir).err().unwrap().to_string();
        assert!(error.contains("components[1].files matches no files that another component hasn't claimed first"), "{error}");
        assert!(error.contains("installer.shared_files = \"all\""), "{error}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn without_the_setting_the_first_component_still_wins() {
    let dir = vendors("shared-first");
    let project = parse(&VENDORS.replace("shared_files = \"all\"", ""), &dir);
    // vendor1 and vendor2 still have their about.txt, so the config reads.
    let project = project.unwrap();
    assert!(project.manifest.files.iter().all(|file| file.also_needs.is_empty()));
    assert_eq!(entry(&project, "vendor1/tools/tool.txt").component.as_deref(), Some("tools"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn mistakes_name_the_setting() {
    let dir = vendors("shared-mistakes");
    let error = |text: String| parse(&text, &dir).err().unwrap().to_string();
    assert!(error(VENDORS.replace("\"all\"", "\"any\"")).starts_with("installer.shared_files is \"any\""));
    assert!(error(VENDORS.replace("vendor2/**", "vendor9/**")).contains("components[3].files matches no files"));
    assert!(error(VENDORS.replace("vendor2/**", "*.exe")).contains("includes app.executable"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_shared_file_travels_in_the_download_of_the_component_that_has_one() {
    let dir = vendors("shared-download");
    // `tools` is downloaded; it is listed after nothing in particular, the vendors are packed.
    let text = VENDORS
        .replace("files = [\"vendor*/tools/**\"]", "files = [\"vendor*/tools/**\"]\n    download = \"https://cdn.example.com/{file}\"");
    let project = parse(&text, &dir).unwrap();
    for path in ["vendor1/tools/tool.txt", "vendor2/tools/tool.txt"] {
        assert_eq!(entry(&project, path).component.as_deref(), Some("tools"), "{path}");
        assert_eq!(entry(&project, path).also_needs.len(), 1, "{path}");
    }
    assert_eq!(project.manifest.components[1].download.as_ref().unwrap().size, 2);

    // A downloaded component listed later takes its files out of the setup file just the same.
    let later = VENDORS.replace("files = [\"vendor2/**\"]", "files = [\"vendor2/**\"]\n    download = \"https://cdn.example.com/{file}\"");
    let project = parse(&later, &dir).unwrap();
    for path in ["vendor2/app/app.txt", "vendor2/tools/tool.txt", "vendor2/about.txt"] {
        assert_eq!(entry(&project, path).component.as_deref(), Some("vendor2"), "{path}");
    }
    assert_eq!(installed(&project, &["app", "vendor2"]), ["hello.exe", "vendor2/about.txt", "vendor2/app/app.txt"]);

    // Two downloads over the very same files: the second would be empty.
    let both =
        text.replace("files = [\"vendor2/**\"]", "files = [\"vendor2/tools/**\"]\n    download = \"https://cdn.example.com/{file}\"");
    let error = parse(&both, &dir).err().unwrap().to_string();
    assert!(error.contains("components[3].download has nothing to download"), "{error}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_manifest_with_impossible_claims_is_refused() {
    let dir = vendors("shared-damaged");
    let manifest = parse(VENDORS, &dir).unwrap().manifest;
    let damaged = |change: &dyn Fn(&mut FileEntry)| {
        let mut manifest = manifest.clone();
        change(manifest.files.iter_mut().find(|file| file.path == "vendor1/tools/tool.txt").unwrap());
        manifest.validate().unwrap_err().to_string()
    };
    assert!(damaged(&|file| file.component = None).contains("belongs to none"));
    assert!(damaged(&|file| file.also_needs = vec!["ghost".into()]).contains("doesn't exist"));
    assert!(damaged(&|file| file.also_needs = vec!["tools".into()]).contains("twice"));

    // The claims survive the trip through the setup file, and what was installed reads back.
    let json = serde_json::to_string(&manifest).unwrap();
    assert_eq!(serde_json::from_str::<crate::manifest::Manifest>(&json).unwrap(), manifest);
    let installed = manifest.as_installed(&Choices { components: vec!["tools".into(), "vendor1".into()], ..Choices::default() });
    installed.validate().unwrap();
    assert!(installed.files.iter().any(|file| file.path == "vendor1/tools/tool.txt"));
    assert!(!installed.files.iter().any(|file| file.path == "vendor1/app/app.txt"));
    std::fs::remove_dir_all(&dir).unwrap();
}
