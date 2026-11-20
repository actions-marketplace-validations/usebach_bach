//! Components whose files are downloaded when they are ticked.

use super::*;

const BASE: &str = r#"
    [app]
    name = "Hello"
    version = "1.2.3"
    publisher = "Example"
    executable = "hello.exe"

    [files]
    source = "build"

    [[components]]
    id = "samples"
    title = "Samples"
    files = ["samples/**"]
    download = DOWNLOAD
"#;

const GITHUB: &str = "\n[updates]\ngithub = \"example/hello\"\n";

fn with(download: &str, extra: &str) -> String {
    format!("{}{extra}", BASE.replace("DOWNLOAD", download))
}

#[test]
fn true_uses_the_update_link_and_the_files_get_a_name_of_their_own() {
    let dir = project_dir("downloads-true");
    let project = parse(&with("true", GITHUB), &dir).unwrap();
    let download = project.manifest.components[0].download.as_ref().unwrap();
    assert_eq!(download.url, "https://github.com/example/hello/releases/download/v1.2.3/Hello-samples-1.2.3.bachpack");
    assert_eq!(download.sha256, "0".repeat(64), "the builder fills it in once the archive exists");
    assert_eq!(download.size, 6, "about the files' size until then");
    assert_eq!(component_pack_name(&project.manifest.app, "samples", &[]), "Hello-samples-1.2.3.bachpack");
    // The files stay in the file list: the uninstaller and the disk check need them.
    assert!(project.manifest.files.iter().any(|file| file.path == "samples/a.txt" && file.component.as_deref() == Some("samples")));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_link_of_your_own_is_filled_in() {
    let dir = project_dir("downloads-link");
    let project = parse(&with("\"https://cdn.example.com/{id}/{version}/{component}/{file}\"", ""), &dir).unwrap();
    let download = project.manifest.components[0].download.as_ref().unwrap();
    assert_eq!(download.url, "https://cdn.example.com/Hello/1.2.3/samples/Hello-samples-1.2.3.bachpack");
    // false is the same as leaving it out.
    assert!(parse(&with("false", ""), &dir).unwrap().manifest.components[0].download.is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn mistakes_in_download_name_their_setting() {
    let dir = project_dir("downloads-mistakes");
    let error = |text: String| parse(&text, &dir).err().unwrap().to_string();
    assert!(error(with("true", "")).contains("components[0].download is true, which uses the download link of [updates]"));
    assert!(error(with("\"http://x.example/a\"", "")).contains("must be an https link"));
    assert!(error(with("3", "")).contains("true or an https link"));
    let encrypted = with("true", &format!("{GITHUB}\n[access]\nencrypt = true\n"));
    assert!(error(encrypted).contains("can't be used with [access] encrypt"));
    std::fs::remove_dir_all(&dir).unwrap();
}
