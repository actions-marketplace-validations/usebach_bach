use super::*;
use crate::i18n::{Key, ANY_LANGUAGE};
use crate::manifest::Choice;

mod style;

fn project_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bach-config-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("build/sub")).unwrap();
    std::fs::create_dir_all(dir.join("build/samples")).unwrap();
    std::fs::write(dir.join("build/hello.exe"), b"MZ").unwrap();
    std::fs::write(dir.join("build/sub/data.txt"), b"data").unwrap();
    std::fs::write(dir.join("build/samples/a.txt"), b"sample").unwrap();
    std::fs::write(dir.join("build/hello.pdb"), b"debug").unwrap();
    std::fs::write(dir.join("LICENSE.txt"), "\u{feff}Terms.\r\n\r\nMore terms.").unwrap();
    dir
}

const MINIMAL: &str = r#"
    [app]
    name = "Hello"
    version = "1.2.3"
    publisher = "Example"
    executable = "hello.exe"

    [files]
    source = "build"
    exclude = ["*.pdb"]
"#;

#[test]
fn a_minimal_config_builds() {
    let dir = project_dir("minimal");
    let project = parse(MINIMAL, &dir).unwrap();
    let paths: Vec<_> = project.manifest.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["hello.exe", "samples/a.txt", "sub/data.txt"]);
    assert_eq!(project.manifest.app.id, "Hello");
    assert_eq!(project.manifest.install.folder, "Hello");
    assert_eq!(project.manifest.install.desktop_shortcut, Choice::On);
    assert_eq!(project.manifest.install.start_menu_shortcut, Choice::Always);
    assert_eq!(project.manifest.install.start_with_windows, Choice::Never);
    assert!(project.manifest.locale.languages.is_empty());
    assert_eq!(project.output, dir.join("dist/Hello-Setup-1.2.3.exe"));
    assert!(project.warnings.is_empty(), "{:?}", project.warnings);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn mistakes_name_their_setting() {
    let dir = project_dir("mistakes");
    let installer = |line: &str| format!("{MINIMAL}\n[installer]\n{line}");
    let cases = [
        (MINIMAL.replace("hello.exe\"", "missing.exe\""), "app.executable"),
        (MINIMAL.replace("\"build\"", "\"nowhere\""), "files.source"),
        (installer("theme = \"blue\""), "installer.theme"),
        (installer("accent = \"red\""), "installer.accent"),
        (installer("desktop_shortcut = \"maybe\""), "installer.desktop_shortcut"),
        (installer("languages = [\"xx\"]"), "installer.languages"),
        (installer("languages = []"), "installer.languages"),
        (installer("license = \"NOPE.txt\""), "installer.license"),
        (installer("path_folder = \"../bin\""), "installer.path_folder"),
        (installer("logo = \"LICENSE.txt\""), "installer.logo"),
        (format!("{MINIMAL}\n[uninstall]\ndata = ['C:\\Windows']"), "uninstall.data"),
        (format!("{MINIMAL}\n[uninstall]\ndata = ['%USERPROFILE%\\Documents']"), "uninstall.data"),
        (format!("{MINIMAL}\n[uninstall]\ndata = ['%USERPROFILE%\\Desktop\\']"), "uninstall.data"),
        (format!("{MINIMAL}\n[uninstall]\ndata = ['%APPDATA%\\Microsoft']"), "uninstall.data"),
        (format!("{MINIMAL}\n[uninstall]\ndata = ['%USERPROFILE%\\.ssh']"), "uninstall.data"),
        (MINIMAL.replace("name = \"Hello\"", "name = \"He/llo\""), "app.name"),
        (format!("{MINIMAL}\n[text]\nnope = \"x\""), "text.nope"),
        (format!("{MINIMAL}\n[text]\ninstall_title = {{ xx = \"x\" }}"), "text.install_title.xx"),
        (format!("{MINIMAL}\n[text]\ninstall_title = \"{{size}}\""), "text.install_title"),
        (format!("{MINIMAL}\n[[components]]\nid = \"Bad Id\"\ntitle = \"x\"\nfiles = [\"*\"]"), "components[0].id"),
        (format!("{MINIMAL}\n[[components]]\nid = \"path\"\ntitle = \"x\"\nfiles = [\"*\"]"), "components[0].id"),
        (format!("{MINIMAL}\n[[components]]\nid = \"x\"\ntitle = \"x\"\nfiles = [\"nothing/**\"]"), "components[0].files"),
        (format!("{MINIMAL}\n[[components]]\nid = \"x\"\ntitle = \"x\"\nfiles = [\"*.exe\"]"), "app.executable"),
        (format!("{MINIMAL}\n[[protocols]]\nscheme = \"https\""), "protocols[0].scheme"),
        (format!("{MINIMAL}\n[[file_types]]\nextension = \".exe\""), "file_types[0].extension"),
    ];
    for (text, key) in cases {
        let err = format!("{:#}", parse(&text, &dir).unwrap_err());
        assert!(err.contains(key), "expected {key} in: {err}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn unknown_settings_are_typos() {
    let dir = project_dir("typo");
    let text = MINIMAL.replace("[files]", "nmae = \"x\"\n[files]");
    assert!(parse(&text, &dir).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn everything_can_be_customised() {
    let dir = project_dir("full");
    let text = format!(
        r#"{MINIMAL}
        [installer]
        languages = ["de", "en"]
        license = "LICENSE.txt"
        desktop_shortcut = false
        start_menu_shortcut = true
        start_with_windows = "always"
        add_to_path = true
        path_folder = "sub"
        change_folder = false
        open_button = false
        logo = "none"

        [[components]]
        id = "samples"
        title = {{ en = "Samples", de = "Beispiele" }}
        files = ["samples/**"]
        default = true

        [[protocols]]
        scheme = "Hello://"

        [[file_types]]
        extension = "HELLO"
        description = "Hello note"

        [text]
        install_title = {{ en = "Get {{name}}", de = "Hol dir {{name}}" }}
        footer = {{ en = "Made with care", de = "Mit Liebe gemacht" }}

        [uninstall]
        data = ['%APPDATA%\Hello']
        data_description = {{ en = "Notes.", de = "Notizen." }}
        "#
    );
    let project = parse(&text, &dir).unwrap();
    let manifest = &project.manifest;
    assert_eq!(manifest.locale.languages, ["de", "en"]);
    assert_eq!(manifest.license("de"), Some("Terms.\n\nMore terms."));
    assert_eq!(manifest.install.desktop_shortcut, Choice::Off);
    assert_eq!(manifest.install.start_with_windows, Choice::Always);
    assert_eq!(manifest.install.path_folder.as_deref(), Some("sub"));
    assert!(!manifest.install.change_folder && !manifest.install.open_button);
    assert!(manifest.look.hide_logo);
    assert_eq!(manifest.components[0].id, "samples");
    let sample = manifest.files.iter().find(|file| file.path == "samples/a.txt").unwrap();
    assert_eq!(sample.component.as_deref(), Some("samples"));
    assert_eq!(manifest.protocols[0].scheme, "hello");
    assert_eq!(manifest.file_types[0].extension, ".hello");
    assert_eq!(manifest.custom_text(Key::install_title, "de"), Some("Hol dir {name}"));
    assert_eq!(manifest.custom_text(Key::delete_data_description, "en"), Some("Notes."));
    assert!(project.warnings.is_empty(), "{:?}", project.warnings);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn untranslated_texts_are_pointed_out() {
    let dir = project_dir("untranslated");
    let text = format!("{MINIMAL}\n[text]\ninstall_button = \"Go\"");
    let project = parse(&text, &dir).unwrap();
    assert_eq!(project.manifest.locale.text["install_button"][ANY_LANGUAGE], "Go");
    assert!(project.warnings[0].contains("text.install_button"), "{:?}", project.warnings);

    // One language: nothing to translate.
    let text = format!("{MINIMAL}\n[installer]\nlanguages = [\"de\"]\n[text]\ninstall_button = \"Los\"");
    assert!(parse(&text, &dir).unwrap().warnings.is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn output_names_are_filled_in() {
    let dir = project_dir("template");
    let app = parse(MINIMAL, &dir).unwrap().manifest.app;
    assert_eq!(fill_template("{name}_{version}_{id}.exe", &app), "Hello_1.2.3_Hello.exe");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_byte_order_mark_is_fine() {
    let dir = project_dir("bom");
    assert!(parse(&format!("\u{feff}{MINIMAL}"), &dir).is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn hex_colours() {
    assert_eq!(parse_hex("#2563eb"), Some(0x2563eb));
    assert_eq!(parse_hex("FFFFFF"), Some(0xffffff));
    assert_eq!(parse_hex("#fff"), None);
}

#[test]
fn requirements_shortcuts_and_updates() {
    let dir = project_dir("extras");
    let app = MINIMAL.replace("[files]", "website = \"https://hello.example\"\n[files]");
    let text = format!(
        r#"{app}
        [installer]
        start_menu_folder = "Example"
        website_button = true
        logo_height = 72

        [[requirements]]
        kind = "webview2"

        [[requirements]]
        kind = "dotnet"
        version = "8.0"
        desktop = true

        [[shortcuts]]
        name = "Hello Docs"
        target = "sub/data.txt"
        arguments = "--docs"

        [updates]
        feed = "https://hello.example/latest.json"
        download_url = "https://hello.example/{{file}}"

        [uninstall]
        feedback_url = "https://hello.example/why"
        "#
    );
    let project = parse(&text, &dir).unwrap();
    let manifest = &project.manifest;
    assert_eq!(manifest.requirements.len(), 2);
    assert_eq!(manifest.requirements[0].name, "Microsoft Edge WebView2");
    assert_eq!(manifest.shortcuts[0].target, "sub/data.txt");
    assert_eq!(manifest.install.start_menu_folder.as_deref(), Some("Example"));
    assert!(manifest.install.website_button);
    assert_eq!(manifest.look.logo_height, Some(72));
    assert_eq!(manifest.updates.as_ref().unwrap().feed, "https://hello.example/latest.json");
    assert_eq!(project.update_download_url.as_deref(), Some("https://hello.example/{file}"));
    assert_eq!(manifest.uninstall.feedback_url.as_deref(), Some("https://hello.example/why"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn extras_name_their_setting() {
    let dir = project_dir("extras-mistakes");
    let cases = [
        (
            r#"[[requirements]]
kind = "java""#,
            "requirements[0].kind",
        ),
        (
            r#"[[requirements]]
kind = "dotnet"
version = "8""#,
            "requirements[0].version",
        ),
        (
            r#"[[requirements]]
kind = "file"
path = 'C:\x.dll'"#,
            "requirements[0].name",
        ),
        (
            r#"[[requirements]]
kind = "registry"
key = 'HKCR\x'
name = "x"
url = "https://x""#,
            "requirements[0].key",
        ),
        (
            r#"[[shortcuts]]
name = "Docs"
target = "nope.exe""#,
            "shortcuts[0].target",
        ),
        (
            r#"[[shortcuts]]
name = "a/b"
target = "hello.exe""#,
            "shortcuts[0].name",
        ),
        (
            r#"[updates]
feed = "http://insecure""#,
            "updates.feed",
        ),
        ("[installer]\nlogo_height = 200", "installer.logo_height"),
        ("[installer]\nwebsite_button = true", "installer.website_button"),
        ("[uninstall]\nfeedback_url = \"ftp://x\"", "uninstall.feedback_url"),
    ];
    for (extra, key) in cases {
        let err = format!("{:#}", parse(&format!("{MINIMAL}\n{extra}"), &dir).unwrap_err());
        assert!(err.contains(key), "expected {key} in: {err}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn github_is_a_whole_update_setup() {
    let dir = project_dir("github");
    let project = parse(&format!("{MINIMAL}\n[updates]\ngithub = \"usebach/hello\""), &dir).unwrap();
    let updates = project.manifest.updates.as_ref().unwrap();
    assert_eq!(updates.feed, "https://github.com/usebach/hello/releases/latest/download/latest.json");
    assert_eq!(updates.auto, Choice::On, "automatic updates are offered, ticked");
    assert_eq!(
        project.update_download_url.as_deref(),
        Some("https://github.com/usebach/hello/releases/download/v1.2.3/{file}"),
        "each version's file comes from its own tag"
    );
    assert_eq!(project.update_github.as_deref(), Some("usebach/hello"));

    let own = format!("{MINIMAL}\n[updates]\ngithub = \"usebach/hello\"\nauto = \"never\"\nfeed = \"https://hello.example/latest.json\"");
    let project = parse(&own, &dir).unwrap();
    let updates = project.manifest.updates.as_ref().unwrap();
    assert_eq!(updates.feed, "https://hello.example/latest.json", "an explicit feed wins");
    assert_eq!(updates.auto, Choice::Never);

    for (extra, key) in [
        ("[updates]\ngithub = \"just-a-name\"", "updates.github"),
        ("[updates]\ngithub = \"a/b/c\"", "updates.github"),
        ("[updates]\nauto = true", "[updates] needs"),
        ("[updates]\nfeed = \"https://x/latest.json\"\nauto = \"sometimes\"", "updates.auto"),
    ] {
        let err = format!("{:#}", parse(&format!("{MINIMAL}\n{extra}"), &dir).unwrap_err());
        assert!(err.contains(key), "expected {key} in: {err}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn github_downloads_are_named_the_way_github_serves_them() {
    let dir = project_dir("asset");
    let text = MINIMAL.replace("name = \"Hello\"", "name = \"Hello World\"\nid = \"Hello\"");
    let project = parse(&format!("{text}\n[updates]\ngithub = \"me/hello\""), &dir).unwrap();
    let file = project.output.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(file, "Hello-World-Setup-1.2.3.exe");
    // Without GitHub the name is left as it is.
    let file = parse(&text, &dir).unwrap().output.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(file, "Hello World-Setup-1.2.3.exe");
    assert_eq!(github_asset_name("Ünï cøde (1).exe"), "-n--c-de--1-.exe");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_release_tag_can_set_the_version() {
    let dir = project_dir("override");
    let overrides = Overrides { version: Some("v2.5.0".into()) };
    let project = parse_with(MINIMAL, &dir, &overrides).unwrap();
    assert_eq!(project.manifest.app.version, "2.5.0");
    assert!(project.output.to_string_lossy().contains("2.5.0"), "the file name follows: {:?}", project.output);
    // A version that only looks like a tag keeps its letters.
    let odd = Overrides { version: Some("vNext".into()) };
    assert_eq!(parse_with(MINIMAL, &dir, &odd).unwrap().manifest.app.version, "vNext");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_updating_app_needs_a_comparable_version() {
    let dir = project_dir("comparable");
    let text = MINIMAL.replace("1.2.3", "nightly") + "\n[updates]\nfeed = \"https://x/latest.json\"";
    let err = format!("{:#}", parse(&text, &dir).unwrap_err());
    assert!(err.contains("app.version"), "{err}");
    // Without updates, any text is a fine version.
    assert!(parse(&MINIMAL.replace("1.2.3", "nightly"), &dir).is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_online_installer_needs_a_feed() {
    let dir = project_dir("online-feed");
    let err = format!(
        "{:#}",
        parse(
            &format!(
                "{MINIMAL}
[installer]
online = true"
            ),
            &dir
        )
        .unwrap_err()
    );
    assert!(err.contains("installer.online") && err.contains("[updates]"), "{err}");
    let ok = format!(
        "{MINIMAL}
[installer]
online = true
[updates]
github = \"me/hello\""
    );
    let project = parse(&ok, &dir).unwrap();
    assert!(project.online);
    assert!(!project.manifest.online, "the full setup file isn't the online one");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_online_installer_has_a_name_without_a_version() {
    let dir = project_dir("online-name");
    let text = MINIMAL.replace(
        "name = \"Hello\"",
        "name = \"Hello World\"
id = \"Hello\"",
    );
    let project = parse(&text, &dir).unwrap();
    assert!(!project.online, "off unless asked for");
    assert_eq!(project.online_output, dir.join("dist/Hello World-Web-Setup.exe"));
    // GitHub's file names, as for the setup file.
    let github = parse(
        &format!(
            "{text}
[updates]
github = \"me/hello\""
        ),
        &dir,
    )
    .unwrap();
    assert_eq!(github.online_output.file_name().unwrap().to_string_lossy(), "Hello-World-Web-Setup.exe");
    let custom = parse(
        &format!(
            "{text}
[installer]
online_output = \"out/{{id}}-Web.exe\""
        ),
        &dir,
    )
    .unwrap();
    assert_eq!(custom.online_output, dir.join("out/Hello-Web.exe"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_two_installers_never_share_a_file() {
    let dir = project_dir("online-clash");
    let text = format!(
        "{MINIMAL}
[installer]
output = \"dist/Hello.exe\"
online_output = \"dist/Hello.exe\""
    );
    let err = format!("{:#}", parse(&text, &dir).unwrap_err());
    assert!(err.contains("online_output"), "{err}");
    let text = format!(
        "{MINIMAL}
[installer]
online_output = \"dist/Hello.txt\""
    );
    assert!(format!("{:#}", parse(&text, &dir).unwrap_err()).contains("online_output"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn access_encrypts_by_default_and_sends_the_key_with_updates() {
    let dir = project_dir("access");
    let alone = parse(
        &format!(
            "{MINIMAL}
[access]
get_key_url = \"https://example.com/buy\""
        ),
        &dir,
    )
    .unwrap();
    assert!(alone.encrypt);
    assert_eq!(alone.key_env, "BACH_ACCESS_KEY");
    let access = alone.manifest.access.as_ref().unwrap();
    assert!(!access.send_key, "nothing to send it to without [updates]");
    assert_eq!(access.get_key_url.as_deref(), Some("https://example.com/buy"));
    assert!(access.encryption.is_none(), "the builder makes the protection data, with the key");

    let updating = parse(
        &format!(
            "{MINIMAL}
[access]
key_env = \"HELLO_KEY\"
[updates]
github = \"me/hello\""
        ),
        &dir,
    )
    .unwrap();
    assert_eq!(updating.key_env, "HELLO_KEY");
    assert!(updating.manifest.access.as_ref().unwrap().send_key, "private updates by default");

    // No [access], no key.
    let none = parse(MINIMAL, &dir).unwrap();
    assert!(!none.encrypt && none.manifest.access.is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn access_mistakes_name_their_setting() {
    let dir = project_dir("access-mistakes");
    let updates = "[updates]
feed = \"https://x/latest.json\"";
    let cases = [
        (
            format!(
                "{MINIMAL}
[access]
encrypt = false"
            ),
            "[access] does nothing",
        ),
        (
            format!(
                "{MINIMAL}
[access]
send_key = true"
            ),
            "access.send_key",
        ),
        (
            format!(
                "{MINIMAL}
[access]
key_env = \"not a name\""
            ),
            "access.key_env",
        ),
        (
            format!(
                "{MINIMAL}
[access]
key_env = \"1ABC\""
            ),
            "access.key_env",
        ),
        (
            format!(
                "{MINIMAL}
[access]
get_key_url = \"http://x/buy\""
            ),
            "access.get_key_url",
        ),
        (
            format!(
                "{MINIMAL}
[access]
key = \"secret\""
            ),
            "key",
        ),
        (
            format!(
                "{MINIMAL}
[access]
encrypt = false
send_key = false
{updates}"
            ),
            "[access] does nothing",
        ),
    ];
    for (text, expected) in cases {
        let err = format!("{:#}", parse(&text, &dir).unwrap_err());
        assert!(err.contains(expected), "{expected}: {err}");
    }
    // Sending the key without encrypting is fine: a private server can check it.
    let gate = format!(
        "{MINIMAL}
[access]
encrypt = false
{updates}"
    );
    let project = parse(&gate, &dir).unwrap();
    assert!(!project.encrypt && project.manifest.access.as_ref().unwrap().send_key);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_key_can_never_be_written_into_bach_toml() {
    let dir = project_dir("no-key-in-config");
    for key in ["key", "password", "secret", "access_key"] {
        let text = format!(
            "{MINIMAL}
[access]
{key} = \"K7QF-3MWD-9XPA\""
        );
        assert!(parse(&text, &dir).is_err(), "[access] {key} must not exist");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn mirrors_are_feeds_of_their_own() {
    let dir = project_dir("mirrors");
    let text = format!(
        "{MINIMAL}
[updates]
github = \"me/hello\"
mirrors = [\"https://mirror.example/hello/latest.json\", \"https://cdn.example/latest.json\"]"
    );
    let project = parse(&text, &dir).unwrap();
    let updates = project.manifest.updates.as_ref().unwrap();
    assert_eq!(updates.mirrors.len(), 2);
    assert_eq!(
        updates.feeds(),
        [
            "https://github.com/me/hello/releases/latest/download/latest.json",
            "https://mirror.example/hello/latest.json",
            "https://cdn.example/latest.json"
        ]
    );
    for (bad, expected) in [
        ("mirrors = [\"http://x/latest.json\"]", "https link"),
        ("mirrors = [\"https://a/l.json\", \"https://a/l.json\"]", "twice"),
        (
            "mirrors = [\"https://x/latest.json\"]
feed = \"https://x/latest.json\"",
            "twice",
        ),
    ] {
        let err = format!(
            "{:#}",
            parse(
                &format!(
                    "{MINIMAL}
[updates]
github = \"me/hello\"
{bad}"
                ),
                &dir
            )
            .unwrap_err()
        );
        assert!(err.contains(expected), "{bad}: {err}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The editor schema (schema/bach.schema.json) knows every text, language
/// and style, so autocompletion never misses one.
#[test]
fn the_schema_knows_every_text() {
    let schema: serde_json::Value = serde_json::from_str(include_str!("../../../../schema/bach.schema.json")).unwrap();
    let texts = &schema["properties"]["text"]["properties"];
    for key in Key::ALL {
        assert!(texts.get(key.name()).is_some(), "schema has no text.{}", key.name());
    }
    let languages = schema["properties"]["installer"]["properties"]["languages"]["items"]["enum"].as_array().unwrap();
    assert_eq!(languages.len(), crate::i18n::LANGUAGES.len());
    let styles: Vec<&str> = schema["properties"]["installer"]["properties"]["style"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|name| name.as_str())
        .collect();
    let presets: Vec<&str> = crate::style::PRESETS.iter().map(|preset| preset.name).collect();
    assert_eq!(styles, presets, "schema/bach.schema.json lists every built-in style");
}

#[test]
fn run_steps_are_checked_against_the_files() {
    let dir = project_dir("run");
    std::fs::create_dir_all(dir.join("build/redist")).unwrap();
    std::fs::write(dir.join("build/redist/vc.exe"), b"MZ").unwrap();
    let ok = format!(
        "{MINIMAL}\n[[run]]\nname = \"Visual C++\"\nfile = \"redist/vc.exe\"\narguments = \"/quiet\"\nsuccess_codes = [3010]\n\n[[run]]\nfile = \"redist/vc.exe\"\nwhen = \"uninstall\""
    );
    let project = parse(&ok, &dir).unwrap();
    let steps = &project.manifest.run;
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].name, "Visual C++");
    assert_eq!(steps[0].success_codes, [0, 3010], "0 always counts");
    assert_eq!(steps[1].name, "vc", "the file's name by default");
    assert_eq!(steps[1].when, crate::manifest::RunWhen::Uninstall);
    project.manifest.validate().unwrap();

    let cases = [
        ("file = \"redist/missing.exe\"", "no installed file"),
        ("file = \"../vc.exe\"", "run[0].file"),
        ("file = \"redist/vc.exe\"\nwhen = \"sometimes\"", "run[0].when"),
        ("file = \"sub/data.txt\"", "must be an .exe"),
        ("file = \"redist/vc.exe\"\nsuccess_codes = []", "success_codes"),
        ("file = \"redist/vc.exe\"\ncomponent = \"ghost\"", "run[0].component"),
        ("file = \"redist/vc.exe\"\nwhen = \"uninstall\"\nignore_errors = true", "ignore_errors"),
        ("file = \"redist/vc.exe\"\ncommand = \"cmd /c x\"", "command"),
    ];
    for (body, expected) in cases {
        let err = format!("{:#}", parse(&format!("{MINIMAL}\n[[run]]\n{body}"), &dir).unwrap_err());
        assert!(err.contains(expected), "{body}: {err}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_windows_requirement_needs_a_build_number() {
    let dir = project_dir("windows-req");
    let good = parse(&format!("{MINIMAL}\n[[requirements]]\nkind = \"windows\"\nbuild = 19041"), &dir).unwrap();
    let requirement = &good.manifest.requirements[0];
    assert_eq!(requirement.check, crate::manifest::Check::Windows { build: 19041 });
    assert!(requirement.name.contains("19041") && requirement.url.starts_with("https://"));
    for body in ["kind = \"windows\"", "kind = \"windows\"\nbuild = 10", "kind = \"windows\"\nbuild = 12345678"] {
        let err = format!("{:#}", parse(&format!("{MINIMAL}\n[[requirements]]\n{body}"), &dir).unwrap_err());
        assert!(err.contains("requirements[0].build"), "{body}: {err}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_machine_wide_install_is_chosen_with_scope() {
    let dir = project_dir("scope");
    assert!(parse(MINIMAL, &dir).unwrap().manifest.install.scope.is_user(), "per user unless asked");
    let machine = format!("{MINIMAL}\n[installer]\nscope = \"machine\"\n");
    let project = parse(&machine, &dir).unwrap();
    assert!(project.manifest.install.scope.is_machine());
    project.manifest.validate().unwrap();
    let err = format!("{:#}", parse(&format!("{MINIMAL}\n[installer]\nscope = \"everyone\"\n"), &dir).unwrap_err());
    assert!(err.contains("installer.scope"), "{err}");

    // Background updates can't write to Program Files, so they're off there.
    let updates = "[updates]\nfeed = \"https://example.com/latest.json\"\ndownload_url = \"https://example.com/{file}\"\n";
    let project = parse(&format!("{machine}\n{updates}"), &dir).unwrap();
    assert_eq!(project.manifest.updates.unwrap().auto, Choice::Never);
    for auto in ["true", "false", "\"always\""] {
        let err = format!("{:#}", parse(&format!("{machine}\n{updates}auto = {auto}\n"), &dir).unwrap_err());
        assert!(err.contains("updates.auto"), "{auto}: {err}");
    }
    assert!(parse(&format!("{machine}\n{updates}auto = \"never\"\n"), &dir).is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn signtool_arguments_start_with_sign() {
    let dir = project_dir("signtool");
    let signing = |program: &str, arguments: &str| format!("{MINIMAL}\n[signing]\nprogram = \"{program}\"\narguments = '{arguments}'\n");
    let project = parse(&signing("signtool", "sign /fd SHA256 /a"), &dir).unwrap();
    assert_eq!(project.signing.unwrap().arguments, "sign /fd SHA256 /a");
    assert!(parse(&format!("{MINIMAL}\n[signing]\narguments = 'SIGN /a'\n"), &dir).is_ok(), "signtool is the default program");
    for program in ["signtool", "SignTool.exe"] {
        let err = format!("{:#}", parse(&signing(program, "/fd SHA256 /a"), &dir).unwrap_err());
        assert!(err.contains("must start with the word sign"), "{program}: {err}");
    }
    let err = format!("{:#}", parse(&format!("{MINIMAL}\n[signing]\narguments = '/fd SHA256 /a'\n"), &dir).unwrap_err());
    assert!(err.contains("must start with the word sign"), "{err}");
    // Other signers have their own syntax.
    assert!(parse(&signing("osslsigncode", "-certs c.pem -in {file}"), &dir).is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn older_installs_are_named_by_their_uninstall_entry() {
    let dir = project_dir("migrate");
    let ok = format!(
        r#"{MINIMAL}
[[migrate]]
uninstall_key = "{{8A69D345-D564-463C-AFF1-A69D9E530F96}}_is1"
arguments = "/VERYSILENT /NORESTART"

[[migrate]]
uninstall_key = "Hello"
elevated = true
"#
    );
    let project = parse(&ok, &dir).unwrap();
    let migrate = &project.manifest.system.migrate;
    assert_eq!(migrate.len(), 2);
    assert_eq!(migrate[0].arguments.as_deref(), Some("/VERYSILENT /NORESTART"));
    assert!(migrate[1].elevated && migrate[1].arguments.is_none());
    project.manifest.validate().unwrap();

    let cases = [
        ("[[migrate]]\nuninstall_key = \"\"", "migrate[0]"),
        ("[[migrate]]\nuninstall_key = \"a/b\"", "Uninstall entry"),
        ("[[migrate]]\nuninstall_key = 'Software\\Hello'", "Uninstall entry"),
        ("[[migrate]]\nuninstall_key = \"A\"\n[[migrate]]\nuninstall_key = \"a\"", "listed twice"),
        ("[[migrate]]\nname = \"x\"", "name"),
    ];
    for (body, expected) in cases {
        let err = format!("{:#}", parse(&format!("{MINIMAL}\n{body}"), &dir).unwrap_err());
        assert!(err.contains(expected), "{body}: {err}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn run_steps_can_ask_for_administrator_rights() {
    let dir = project_dir("elevated");
    std::fs::create_dir_all(dir.join("build/redist")).unwrap();
    std::fs::write(dir.join("build/redist/vc.exe"), b"MZ").unwrap();
    let step = |extra: &str| format!("{MINIMAL}\n[[run]]\nfile = \"redist/vc.exe\"\n{extra}");
    assert!(!parse(&step(""), &dir).unwrap().manifest.run[0].elevated);
    let project = parse(&step("elevated = true"), &dir).unwrap();
    assert!(project.manifest.run[0].elevated);
    project.manifest.validate().unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn system_changes_are_checked() {
    let dir = project_dir("system");
    let ok = format!(
        r#"{MINIMAL}
[[registry]]
key = 'Software\Hello'
name = "Dir"
type = "expand"
value = '{{app}}'

[[registry]]
key = 'Software\Hello'
name = "Level"
type = "dword"
value = 3

[[env]]
name = "HELLO_HOME"
value = '{{app}}'

[[context_menu]]
name = "Open with Hello"
on = ["files", ".md"]

[[context_menu]]
name = "Hello here"
on = "background"
arguments = '--here "%V"'
"#
    );
    let project = parse(&ok, &dir).unwrap();
    let system = &project.manifest.system;
    assert_eq!(system.registry.len(), 2);
    assert_eq!(system.registry[1].data, crate::manifest::RegistryData::Dword(3));
    assert_eq!(system.environment[0].name, "HELLO_HOME");
    assert_eq!(system.context_menu[0].on.len(), 2);
    assert_eq!(system.context_menu[1].arguments.as_deref(), Some("--here \"%V\""));
    project.manifest.validate().unwrap();
    assert!(parse(MINIMAL, &dir).unwrap().manifest.system.is_empty());

    let cases = [
        (
            r#"[[registry]]
key = 'Software\Classes\.txt'
value = "x""#,
            "registry[0]",
        ),
        (
            r#"[[registry]]
key = 'HKCU\Software\X'
value = "x""#,
            "below Software",
        ),
        (
            r#"[[registry]]
key = 'Software\X'
type = "dword"
value = -1"#,
            "dword",
        ),
        (
            r#"[[registry]]
key = 'Software\X'
type = "binary"
value = "x""#,
            "registry[0].type",
        ),
        (
            r#"[[registry]]
key = 'Software\X'
value = 4"#,
            "must be text",
        ),
        (
            r#"[[registry]]
key = 'Software\X'
value = "a"
[[registry]]
key = 'software\x'
value = "b""#,
            "second time",
        ),
        (
            r#"[[env]]
name = "PATH"
value = "x""#,
            "belongs to Windows",
        ),
        (
            r#"[[env]]
name = "HELLO_A"
value = "1"
[[env]]
name = "hello_a"
value = "2""#,
            "set twice",
        ),
        (
            r#"[[env]]
name = "JAVA_HOME"
value = "x""#,
            "doesn't start with HELLO_",
        ),
        (
            r#"[[env]]
name = "NODE_OPTIONS"
value = "x"
allow = true"#,
            "loads code",
        ),
        (
            r#"[[registry]]
key = 'Software\Microsoft\Windows Defender\Exclusions\Paths'
value = "x""#,
            "registry[0]",
        ),
        (
            r#"[[registry]]
key = 'Software\Google\Chrome\NativeMessagingHosts\x'
value = "x""#,
            "registry[0]",
        ),
        (
            r#"[[context_menu]]
name = "Open""#,
            "context_menu[0].on",
        ),
        (
            r#"[[context_menu]]
name = "Open"
on = ".exe""#,
            "context_menu[0].on",
        ),
        (
            r#"[[context_menu]]
name = "Open"
on = "files"
icon = "x""#,
            "icon",
        ),
    ];
    for (body, expected) in cases {
        let err = format!("{:#}", parse(&format!("{MINIMAL}\n{body}"), &dir).unwrap_err());
        assert!(err.contains(expected), "{body}: {err}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_secret_in_the_source_folder_stops_the_build_until_allowed() {
    let dir = project_dir("secrets");
    std::fs::write(dir.join("build/.env"), b"TOKEN=1").unwrap();
    std::fs::create_dir_all(dir.join("build/.git")).unwrap();
    std::fs::write(dir.join("build/.git/config"), b"[core]").unwrap();

    let err = format!("{:#}", parse(MINIMAL, &dir).unwrap_err());
    assert!(err.contains("files.source") && err.contains(".env") && err.contains("files.allow_secrets"), "{err}");

    let excluded = MINIMAL.replace("exclude = [\"*.pdb\"]", "exclude = [\"*.pdb\", \".env\", \".git/**\"]");
    parse(&excluded, &dir).unwrap();

    let allowed = MINIMAL.replace("exclude = [\"*.pdb\"]", "allow_secrets = [\".env\", \".git/**\"]");
    let project = parse(&allowed, &dir).unwrap();
    assert!(project.manifest.files.iter().any(|file| file.path == ".env"), "allowed on purpose, so it ships");
    assert!(project.warnings.iter().any(|warning| warning.contains("hello.pdb")), "{:?}", project.warnings);

    let empty = MINIMAL.replace("exclude = [\"*.pdb\"]", "allow_secrets = [\"\"]");
    assert!(format!("{:#}", parse(&empty, &dir).unwrap_err()).contains("files.allow_secrets"));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Every setting the editor schema knows has to be explained in docs/config.md,
/// and every setting named in a docs table has to exist, so the two can't drift.
#[test]
fn the_docs_and_the_schema_name_the_same_settings() {
    let schema: serde_json::Value = serde_json::from_str(include_str!("../../../../schema/bach.schema.json")).unwrap();
    let docs = include_str!("../../../../docs/config.md");
    // Settings with a table row of their own in docs/config.md: `name` in backticks.
    fn walk(value: &serde_json::Value, path: &str, out: &mut Vec<(String, String)>) {
        let Some(properties) = value.get("properties").and_then(|p| p.as_object()) else { return };
        for (name, child) in properties {
            out.push((format!("{path}{name}"), name.clone()));
            walk(child.get("items").unwrap_or(child), &format!("{path}{name}."), out);
        }
    }
    let mut settings = Vec::new();
    walk(&schema, "", &mut settings);
    assert!(settings.len() > 100, "the schema walk found only {} settings", settings.len());
    // In a table row, in a TOML example, or as a section heading.
    let explained = |name: &str| {
        [format!("`{name}`"), format!("{name} ="), format!("[{name}]"), format!("[[{name}]]")].iter().any(|form| docs.contains(form))
    };
    let missing: Vec<&str> =
        settings.iter().filter(|(path, name)| !path.starts_with("text.") && !explained(name)).map(|(path, _)| path.as_str()).collect();
    assert!(missing.is_empty(), "in the schema but not in docs/config.md: {missing:?}");

    // The other way: the first column of the settings tables names real settings.
    let known: std::collections::HashSet<&str> = settings.iter().map(|(_, name)| name.as_str()).collect();
    let mut unknown = Vec::new();
    for line in docs.lines() {
        let Some(rest) = line.strip_prefix("| `") else { continue };
        let Some((name, _)) = rest.split_once('`') else { continue };
        let simple = name.chars().all(|c| c.is_ascii_lowercase() || c == '_');
        if simple && !name.is_empty() && !known.contains(name) && !matches!(name, "none" | "false" | "true") {
            unknown.push(name.to_string());
        }
    }
    unknown.sort();
    unknown.dedup();
    // Names that belong to the command line tables, not to bach.toml.
    unknown.retain(|name| {
        !["silent", "dir", "with", "without", "lang", "log", "theme", "preview", "json", "launch", "update", "uninstall"]
            .contains(&name.as_str())
    });
    // The table of built-in styles names values of installer.style.
    unknown.retain(|name| crate::style::preset(name).is_none());
    assert!(unknown.is_empty(), "named in docs/config.md tables but not in the schema: {unknown:?}");
}

#[test]
fn a_publisher_key_for_updates_is_checked_and_kept() {
    let dir = project_dir("publickey");
    let (_, public) = crate::feedsig::generate().unwrap();
    let updates = |key: &str| format!("{MINIMAL}\n[updates]\nfeed = \"https://x.example/latest.json\"\npublic_key = \"{key}\"\n");
    let project = parse(&updates(&public.to_uppercase()), &dir).unwrap();
    assert_eq!(project.manifest.updates.as_ref().unwrap().public_key.as_deref(), Some(public.as_str()), "kept in lower case");
    // A second key for the hand-over: valid, different, and only with a first key.
    let (_, next) = crate::feedsig::generate().unwrap();
    let with_next = format!(
        "{MINIMAL}
[updates]
feed = \"https://x.example/latest.json\"
public_key = \"{public}\"
next_public_key = \"{}\"
",
        next.to_uppercase()
    );
    let project = parse(&with_next, &dir).unwrap();
    assert_eq!(project.manifest.updates.as_ref().unwrap().next_public_key.as_deref(), Some(next.as_str()));
    let same = with_next.replace(&next.to_uppercase(), &public);
    assert!(format!("{:#}", parse(&same, &dir).unwrap_err()).contains("same key"));
    let alone = format!(
        "{MINIMAL}
[updates]
feed = \"https://x.example/latest.json\"
next_public_key = \"{next}\"
"
    );
    assert!(format!("{:#}", parse(&alone, &dir).unwrap_err()).contains("needs updates.public_key"));
    project.manifest.validate().unwrap();
    for bad in ["short", &"zz".repeat(32), &"ab".repeat(31)] {
        let err = format!("{:#}", parse(&updates(bad), &dir).unwrap_err());
        assert!(err.contains("updates.public_key"), "{bad}: {err}");
    }
    let mut manifest = project.manifest.clone();
    manifest.updates.as_mut().unwrap().public_key = Some("nope".into());
    assert!(manifest.validate().is_err(), "a setup file with a damaged key is refused");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_config_version_is_optional_and_checked() {
    let dir = project_dir("configversion");
    parse(MINIMAL, &dir).unwrap();
    parse(&format!("config_version = 1\n{MINIMAL}"), &dir).unwrap();
    let newer = format!("{:#}", parse(&format!("config_version = 2\n{MINIMAL}"), &dir).unwrap_err());
    assert!(newer.contains("config_version is 2") && newer.contains("update Bach"), "{newer}");
    let older = format!("{:#}", parse(&format!("config_version = 0\n{MINIMAL}"), &dir).unwrap_err());
    assert!(older.contains("config_version is 0") && older.contains("changelog"), "{older}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_windows_requirement_below_the_supported_minimum_warns() {
    let dir = project_dir("minwindows");
    let with = |build: u32| format!("{MINIMAL}\n[[requirements]]\nkind = \"windows\"\nbuild = {build}\n");
    // 1809 itself and Windows 11 are fine.
    for build in [17_763, 19_041, 22_000] {
        let project = parse(&with(build), &dir).unwrap();
        assert!(project.warnings.is_empty(), "{build}: {:?}", project.warnings);
    }
    // The first Windows 10 is older than what Bach supports: said at build time.
    let project = parse(&with(10_240), &dir).unwrap();
    assert!(
        project.warnings.iter().any(|warning| warning.contains("10240") && warning.contains("1809") && warning.contains("17763")),
        "{:?}",
        project.warnings
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn compression_threads_and_memory_are_checked() {
    let dir = project_dir("threads");
    let with = |extra: &str| parse(&format!("{MINIMAL}\n[installer]\n{extra}\n"), &dir);
    let project = with("compression_threads = 3\ncompression_memory = 2048").unwrap();
    assert_eq!((project.compression_threads, project.compression_memory_mib), (Some(3), 2048));
    let project = parse(MINIMAL, &dir).unwrap();
    assert_eq!((project.compression_threads, project.compression_memory_mib), (None, crate::archive::DEFAULT_MEMORY_MIB));
    for bad in ["compression_threads = 0", "compression_threads = 1000", "compression_memory = 8", "compression_memory = 999999"] {
        let message = format!("{:#}", with(bad).err().unwrap_or_else(|| panic!("{bad} should be refused")));
        assert!(message.contains(bad.split(' ').next().unwrap()), "{bad}: {message}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_registry_view_is_read_and_a_32_bit_program_in_a_machine_install_is_warned_about() {
    let dir = project_dir("regview");
    let with = |view: &str| {
        let line = if view.is_empty() { String::new() } else { format!("view = \"{view}\"\n") };
        format!(
            "{MINIMAL}\n[installer]\nscope = \"machine\"\nfolder = \"Hello\"\n\n[[registry]]\nkey = 'Software\\Example\\Hello'\nname = \"Dir\"\nvalue = \"{{app}}\"\n{line}"
        )
    };
    let views =
        |text: &str| parse(text, &dir).map(|project| project.manifest.system.registry.iter().map(|value| value.view).collect::<Vec<_>>());
    assert_eq!(views(&with("")).unwrap(), [crate::manifest::RegistryView::Native]);
    assert_eq!(views(&with("32")).unwrap(), [crate::manifest::RegistryView::Wow32]);
    assert_eq!(views(&with("both")).unwrap(), [crate::manifest::RegistryView::Both]);
    let message = format!("{:#}", views(&with("16")).unwrap_err());
    assert!(message.contains("registry[0].view") && message.contains("\"64\""), "{message}");

    // A 32-bit program (the PE header says x86) with a machine-wide install: warned, once it is said.
    let mut x86 = vec![0u8; 256];
    x86[0..2].copy_from_slice(b"MZ");
    x86[0x3c] = 0x80;
    x86[0x80..0x84].copy_from_slice(b"PE\0\0");
    x86[0x84..0x86].copy_from_slice(&0x014Cu16.to_le_bytes());
    std::fs::write(dir.join("build/hello.exe"), &x86).unwrap();
    let warned = |text: &str| parse(text, &dir).unwrap().warnings.iter().any(|warning| warning.contains("32-bit program"));
    assert!(warned(&with("")), "a value that leaves the view open");
    assert!(!warned(&with("32")), "the view is said, so no warning");
    // The same program installed per user has no WOW6432Node view of HKCU\Software to worry about.
    assert!(!warned(&with("").replace("scope = \"machine\"", "scope = \"user\"")));
    // A 64-bit program: nothing to say.
    x86[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
    std::fs::write(dir.join("build/hello.exe"), &x86).unwrap();
    assert!(!warned(&with("")));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_beta_feed_is_read_checked_and_picked_by_channel() {
    let dir = project_dir("beta");
    let feed = "feed = \"https://x.example/latest.json\"";
    let with = |line: &str| {
        parse(
            &format!(
                "{MINIMAL}
[updates]
{feed}
mirrors = [\"https://m.example/latest.json\"]
{line}
"
            ),
            &dir,
        )
    };
    let updates = with("beta_feed = \"https://x.example/latest-beta.json\"").unwrap().manifest.updates.unwrap();
    assert_eq!(updates.beta_feed.as_deref(), Some("https://x.example/latest-beta.json"));
    // Stable asks the feed and its mirror; beta asks only the beta feed.
    assert_eq!(updates.feeds_for(false), ["https://x.example/latest.json", "https://m.example/latest.json"]);
    assert_eq!(updates.feeds_for(true), ["https://x.example/latest-beta.json"]);
    // An app without beta versions follows stable whatever was chosen.
    let plain = with("").unwrap().manifest.updates.unwrap();
    assert_eq!(plain.feeds_for(true), plain.feeds());
    for (bad, why) in [
        ("beta_feed = \"http://x.example/beta.json\"", "https"),
        ("beta_feed = \"https://x.example/latest.json\"", "same as the stable feed"),
        ("beta_feed = \"https://m.example/latest.json\"", "same as the stable feed"),
    ] {
        let message = format!("{:#}", with(bad).unwrap_err());
        assert!(message.contains("updates.beta_feed") && message.contains(why), "{bad}: {message}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_publisher_of_updates_is_read_and_checked() {
    let dir = project_dir("signer");
    let with = |line: &str| format!("{MINIMAL}\n[updates]\nfeed = \"https://x.example/latest.json\"\n{line}\n");
    let signer = |line: &str| parse(&with(line), &dir).map(|project| project.manifest.updates.unwrap().signer);
    assert_eq!(signer("signer = \"Example Inc.\"").unwrap().as_deref(), Some("Example Inc."));
    assert_eq!(signer("").unwrap(), None);
    assert_eq!(signer("signer = \"  \"").unwrap(), None, "blank means none");
    assert!(signer("signer = \"CN=Example Inc., O=Example, C=DE\"").unwrap().is_some(), "a whole subject is fine");
    for bad in ["signer = \"x\\u0007y\"", &format!("signer = \"{}\"", "x".repeat(301))] {
        let message = format!("{:#}", signer(bad).unwrap_err());
        assert!(message.contains("updates.signer"), "{bad}: {message}");
    }
    use crate::manifest::extras::subject_names;
    let subject = "CN=\"Example, Inc.\", O=Example Holding, L=Berlin, C=DE";
    assert!(subject_names(subject, "Example, Inc.") && subject_names(subject, "example holding"));
    assert!(!subject_names(subject, "Example") && !subject_names(subject, "Berlin") && !subject_names(subject, ""));
    assert!(!subject_names("", "Example Inc."));
    // A whole subject must be the subject, pair for pair: another company of the same name differs in L or C.
    let full = "CN=Example Inc., O=Example, L=Berlin, C=DE";
    assert!(
        subject_names(full, "CN=Example Inc., O=Example, L=Berlin, C=DE") && subject_names(full, "cn=example inc.,o=EXAMPLE,L=berlin,c=de")
    );
    assert!(!subject_names(full, "CN=Example Inc., O=Example, L=Paris, C=FR"), "same name, other place");
    assert!(!subject_names(full, "CN=Example Inc., O=Example, C=DE"), "a part of the subject is not the subject");
    assert!(!subject_names(&format!("{full}, E=x@y.z"), full), "more than was asked for is not the same");
    assert!(!subject_names("CN=Example Inc.\u{7}", "Example Inc."), "control characters");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_version_that_cannot_be_compared_warns() {
    let dir = project_dir("badversion");
    let with = |version: &str| parse(&MINIMAL.replacen("version = \"1.2.3\"", &format!("version = \"{version}\""), 1), &dir).unwrap();
    for fine in ["1.0.0", "2024.10.3.1", "v2.1", "2.0.0-beta.2", "1.0.0+build.5"] {
        assert!(with(fine).warnings.is_empty(), "{fine}: {:?}", with(fine).warnings);
    }
    for odd in ["abc", "-", "99999999999999999999.0.0", "1.x"] {
        let project = with(odd);
        assert!(
            project.warnings.iter().any(|warning| warning.contains("app.version") && warning.contains("downgrade")),
            "{odd}: {:?}",
            project.warnings
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_path_that_would_pass_the_classic_windows_limit_warns() {
    let dir = project_dir("longpath");
    let project = parse(MINIMAL, &dir).unwrap();
    assert!(!project.warnings.iter().any(|warning| warning.contains("characters long")), "{:?}", project.warnings);
    let deep = (0..12).map(|n| format!("directory_number_{n}")).collect::<Vec<_>>().join("/");
    std::fs::create_dir_all(dir.join("build").join(&deep)).unwrap();
    std::fs::write(dir.join("build").join(&deep).join("file.txt"), b"x").unwrap();
    let project = parse(MINIMAL, &dir).unwrap();
    let warning = project.warnings.iter().find(|warning| warning.contains("characters long")).expect("a warning");
    assert!(warning.contains("directory_number_11/file.txt") && warning.contains("259"), "{warning}");
    // Below Program Files there is more room than below a user's profile.
    let machine = format!("{MINIMAL}\n[installer]\nscope = \"machine\"\n");
    let shallow = (0..11).map(|n| format!("directory_number_{n}")).collect::<Vec<_>>().join("/");
    std::fs::remove_dir_all(dir.join("build").join("directory_number_0")).unwrap();
    std::fs::create_dir_all(dir.join("build").join(&shallow)).unwrap();
    std::fs::write(dir.join("build").join(&shallow).join("file.txt"), b"x").unwrap();
    assert!(parse(&machine, &dir).unwrap().warnings.iter().all(|warning| !warning.contains("characters long")));
    assert!(parse(MINIMAL, &dir).unwrap().warnings.iter().any(|warning| warning.contains("characters long")));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_data_folder_that_does_not_look_like_the_apps_own_warns() {
    let dir = project_dir("datawarn");
    let with = |folder: &str| parse(&format!("{MINIMAL}\n[uninstall]\ndata = ['{folder}']\n"), &dir).unwrap();
    assert!(with(r"%APPDATA%\Hello").warnings.is_empty());
    assert!(with(r"%LOCALAPPDATA%\Example\Cache").warnings.is_empty(), "the publisher counts");
    let warning = with(r"%APPDATA%\Notes").warnings.join(" ");
    assert!(warning.contains("uninstall.data") && warning.contains("looks like the app's name"), "{warning}");
    assert!(parse(&format!("{MINIMAL}\n[uninstall]\ndata = ['%USERPROFILE%\\source\\repos']\n"), &dir).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_elevated_step_in_a_per_user_install_warns() {
    let dir = project_dir("elevatedwarn");
    std::fs::create_dir_all(dir.join("build/redist")).unwrap();
    std::fs::write(dir.join("build/redist/vc_redist.exe"), b"MZ").unwrap();
    let step = "\n[[run]]\nname = \"VC++\"\nfile = \"redist/vc_redist.exe\"\nelevated = true\n";
    let user = parse(&format!("{MINIMAL}{step}"), &dir).unwrap();
    assert!(user.warnings.iter().any(|warning| warning.contains("VC++") && warning.contains("per-user")), "{:?}", user.warnings);
    let machine = parse(&format!("{MINIMAL}\n[installer]\nscope = \"machine\"\n{step}"), &dir).unwrap();
    assert!(machine.warnings.iter().all(|warning| !warning.contains("per-user")), "{:?}", machine.warnings);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_very_long_file_list_warns() {
    let entry = |n: usize| FileEntry { path: format!("data/part-{n:06}.bin"), size: 1, component: None, sha256: None };
    let few: Vec<FileEntry> = (0..1_000).map(entry).collect();
    assert!(file_list_warning(&few).is_none());
    let many: Vec<FileEntry> = (0..30_000).map(entry).collect();
    let warning = file_list_warning(&many).expect("a warning");
    assert!(warning.contains("30000 files") && warning.contains("uninstall.exe") && warning.contains("MB"), "{warning}");
}
