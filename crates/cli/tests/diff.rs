//! `bach diff` on two real setup files.

mod common;
use common::{config, project, run, text};

#[test]
fn diff_lists_what_changed_between_two_versions() {
    need_runtime!();
    let extra = r#"[[registry]]
key = 'Software\BachTest\Hello'
name = "Mode"
type = "dword"
value = 1
"#;
    let dir = project("diff", &config(extra));
    assert!(run(&dir, &["build", "--fast"]).status.success());
    std::fs::copy(dir.join("dist/Hello-Setup-1.0.0.exe"), dir.join("old.exe")).unwrap();

    // 1.1.0: a new file, a bigger program, a new registry value, and no 1 MB data file.
    std::fs::write(dir.join("app/notes.txt"), "second").unwrap();
    std::fs::write(dir.join("app/hello.exe"), vec![b'M'; 5000]).unwrap();
    std::fs::remove_file(dir.join("app/data.bin")).unwrap();
    let more = format!("{extra}\n[[registry]]\nkey = 'Software\\BachTest\\Hello'\nname = \"Channel\"\nvalue = \"stable\"\n");
    std::fs::write(dir.join("bach.toml"), config(&more)).unwrap();
    assert!(run(&dir, &["build", "--fast", "--app-version", "1.1.0"]).status.success());

    let out = run(&dir, &["diff", "old.exe", "dist/Hello-Setup-1.1.0.exe"]);
    let said = text(&out.stdout);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(said.starts_with("Hello 1.0.0 → 1.1.0"), "{said}");
    assert!(said.contains("1 added, 1 removed, 1 changed, 0 unchanged"), "{said}");
    assert!(said.contains("+ notes.txt") && said.contains("- data.bin") && said.contains("~ hello.exe"), "{said}");
    assert!(said.contains("Made the app bigger:") && said.contains("hello.exe"), "{said}");
    assert!(said.contains(r"+ registry Software\BachTest\Hello\Channel = "), "{said}");

    // The same data as JSON.
    let json = run(&dir, &["diff", "old.exe", "dist/Hello-Setup-1.1.0.exe", "--json"]);
    let object: serde_json::Value = serde_json::from_str(&text(&json.stdout)).unwrap();
    assert_eq!(object["ok"], true);
    assert_eq!(object["old_version"], "1.0.0");
    assert_eq!(object["changed_files"].as_array().unwrap().len(), 3);
    assert_eq!(object["changes"][0]["change"], "added");

    // The same file twice differs in nothing.
    let same = text(&run(&dir, &["diff", "old.exe", "old.exe"]).stdout);
    assert!(same.contains("Nothing differs"), "{same}");

    // Another app, or something that isn't a setup file, is one sentence.
    std::fs::write(dir.join("plain.txt"), "hi").unwrap();
    let bad = run(&dir, &["diff", "old.exe", "plain.txt"]);
    assert!(!bad.status.success() && text(&bad.stderr).contains("isn't a Bach setup file"), "{}", text(&bad.stderr));
    std::fs::remove_dir_all(&dir).unwrap();
}
