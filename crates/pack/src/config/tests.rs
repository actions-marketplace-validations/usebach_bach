use super::*;
use crate::i18n::{Key, ANY_LANGUAGE};
use crate::manifest::Choice;

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
        (r#"[[requirements]]
kind = "java""#, "requirements[0].kind"),
        (r#"[[requirements]]
kind = "dotnet"
version = "8""#, "requirements[0].version"),
        (r#"[[requirements]]
kind = "file"
path = 'C:\x.dll'"#, "requirements[0].name"),
        (r#"[[requirements]]
kind = "registry"
key = 'HKCR\x'
name = "x"
url = "https://x""#, "requirements[0].key"),
        (r#"[[shortcuts]]
name = "Docs"
target = "nope.exe""#, "shortcuts[0].target"),
        (r#"[[shortcuts]]
name = "a/b"
target = "hello.exe""#, "shortcuts[0].name"),
        (r#"[updates]
feed = "http://insecure""#, "updates.feed"),
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

    let own = format!(
        "{MINIMAL}\n[updates]\ngithub = \"usebach/hello\"\nauto = \"never\"\nfeed = \"https://hello.example/latest.json\""
    );
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
    let err = format!("{:#}", parse(&format!("{MINIMAL}
[installer]
online = true"), &dir).unwrap_err());
    assert!(err.contains("installer.online") && err.contains("[updates]"), "{err}");
    let ok = format!("{MINIMAL}
[installer]
online = true
[updates]
github = \"me/hello\"");
    let project = parse(&ok, &dir).unwrap();
    assert!(project.online);
    assert!(!project.manifest.online, "the full setup file isn't the online one");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_online_installer_has_a_name_without_a_version() {
    let dir = project_dir("online-name");
    let text = MINIMAL.replace("name = \"Hello\"", "name = \"Hello World\"
id = \"Hello\"");
    let project = parse(&text, &dir).unwrap();
    assert!(!project.online, "off unless asked for");
    assert_eq!(project.online_output, dir.join("dist/Hello World-Web-Setup.exe"));
    // GitHub's file names, as for the setup file.
    let github = parse(&format!("{text}
[updates]
github = \"me/hello\""), &dir).unwrap();
    assert_eq!(github.online_output.file_name().unwrap().to_string_lossy(), "Hello-World-Web-Setup.exe");
    let custom = parse(&format!("{text}
[installer]
online_output = \"out/{{id}}-Web.exe\""), &dir).unwrap();
    assert_eq!(custom.online_output, dir.join("out/Hello-Web.exe"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_two_installers_never_share_a_file() {
    let dir = project_dir("online-clash");
    let text = format!("{MINIMAL}
[installer]
output = \"dist/Hello.exe\"
online_output = \"dist/Hello.exe\"");
    let err = format!("{:#}", parse(&text, &dir).unwrap_err());
    assert!(err.contains("online_output"), "{err}");
    let text = format!("{MINIMAL}
[installer]
online_output = \"dist/Hello.txt\"");
    assert!(format!("{:#}", parse(&text, &dir).unwrap_err()).contains("online_output"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn access_encrypts_by_default_and_sends_the_key_with_updates() {
    let dir = project_dir("access");
    let alone = parse(&format!("{MINIMAL}
[access]
get_key_url = \"https://example.com/buy\""), &dir).unwrap();
    assert!(alone.encrypt);
    assert_eq!(alone.key_env, "BACH_ACCESS_KEY");
    let access = alone.manifest.access.as_ref().unwrap();
    assert!(!access.send_key, "nothing to send it to without [updates]");
    assert_eq!(access.get_key_url.as_deref(), Some("https://example.com/buy"));
    assert!(access.encryption.is_none(), "the builder makes the protection data, with the key");

    let updating = parse(&format!("{MINIMAL}
[access]
key_env = \"HELLO_KEY\"
[updates]
github = \"me/hello\""), &dir).unwrap();
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
        (format!("{MINIMAL}
[access]
encrypt = false"), "[access] does nothing"),
        (format!("{MINIMAL}
[access]
send_key = true"), "access.send_key"),
        (format!("{MINIMAL}
[access]
key_env = \"not a name\""), "access.key_env"),
        (format!("{MINIMAL}
[access]
key_env = \"1ABC\""), "access.key_env"),
        (format!("{MINIMAL}
[access]
get_key_url = \"http://x/buy\""), "access.get_key_url"),
        (format!("{MINIMAL}
[access]
key = \"secret\""), "key"),
        (format!("{MINIMAL}
[access]
encrypt = false
send_key = false
{updates}"), "[access] does nothing"),
    ];
    for (text, expected) in cases {
        let err = format!("{:#}", parse(&text, &dir).unwrap_err());
        assert!(err.contains(expected), "{expected}: {err}");
    }
    // Sending the key without encrypting is fine: a private server can check it.
    let gate = format!("{MINIMAL}
[access]
encrypt = false
{updates}");
    let project = parse(&gate, &dir).unwrap();
    assert!(!project.encrypt && project.manifest.access.as_ref().unwrap().send_key);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_key_can_never_be_written_into_bach_toml() {
    let dir = project_dir("no-key-in-config");
    for key in ["key", "password", "secret", "access_key"] {
        let text = format!("{MINIMAL}
[access]
{key} = \"K7QF-3MWD-9XPA\"");
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
        ("mirrors = [\"https://x/latest.json\"]
feed = \"https://x/latest.json\"", "twice"),
    ] {
        let err = format!("{:#}", parse(&format!("{MINIMAL}
[updates]
github = \"me/hello\"
{bad}"), &dir).unwrap_err());
        assert!(err.contains(expected), "{bad}: {err}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The editor schema (schema/bach.schema.json) knows every text and
/// language, so autocompletion never misses one.
#[test]
fn the_schema_knows_every_text() {
    let schema: serde_json::Value = serde_json::from_str(include_str!("../../../../schema/bach.schema.json")).unwrap();
    let texts = &schema["properties"]["text"]["properties"];
    for key in Key::ALL {
        assert!(texts.get(key.name()).is_some(), "schema has no text.{}", key.name());
    }
    let languages = schema["properties"]["installer"]["properties"]["languages"]["items"]["enum"].as_array().unwrap();
    assert_eq!(languages.len(), crate::i18n::LANGUAGES.len());
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
        (r#"[[registry]]
key = 'Software\Classes\.txt'
value = "x""#, "registry[0]"),
        (r#"[[registry]]
key = 'HKCU\Software\X'
value = "x""#, "below Software"),
        (r#"[[registry]]
key = 'Software\X'
type = "dword"
value = -1"#, "dword"),
        (r#"[[registry]]
key = 'Software\X'
type = "binary"
value = "x""#, "registry[0].type"),
        (r#"[[registry]]
key = 'Software\X'
value = 4"#, "must be text"),
        (r#"[[registry]]
key = 'Software\X'
value = "a"
[[registry]]
key = 'software\x'
value = "b""#, "second time"),
        (r#"[[env]]
name = "PATH"
value = "x""#, "belongs to Windows"),
        (r#"[[env]]
name = "A"
value = "1"
[[env]]
name = "a"
value = "2""#, "set twice"),
        (r#"[[context_menu]]
name = "Open""#, "context_menu[0].on"),
        (r#"[[context_menu]]
name = "Open"
on = ".exe""#, "context_menu[0].on"),
        (r#"[[context_menu]]
name = "Open"
on = "files"
icon = "x""#, "icon"),
    ];
    for (body, expected) in cases {
        let err = format!("{:#}", parse(&format!("{MINIMAL}\n{body}"), &dir).unwrap_err());
        assert!(err.contains(expected), "{body}: {err}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
