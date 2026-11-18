//! What kind of project `bach init` runs in, so the starter config already
//! names the app, its version and where its build ends up.
//!
//! Reads Cargo.toml (Rust), src-tauri/tauri.conf.json (Tauri), wails.json
//! (Go with Wails), package.json (Electron), *.csproj (.NET), pubspec.yaml
//! (Flutter), a PyInstaller *.spec, CMakeLists.txt or a *.pro file (Qt),
//! and the game engines in `engines.rs`: project.godot (Godot) and
//! ProjectSettings (Unity). Anything it can't read is simply left for the
//! developer to fill in.

use std::path::Path;

mod engines;

/// What the project says about itself; None where it doesn't say.
#[derive(Debug, Default, PartialEq)]
pub struct Detected {
    /// "Rust", "Tauri", "Wails", "Electron", ".NET", "Flutter", "PyInstaller", "Qt", "Godot" or
    /// "Unity", for the message and the style `bach init` suggests.
    pub stack: &'static str,
    pub name: Option<String>,
    pub version: Option<String>,
    pub publisher: Option<String>,
    pub executable: Option<String>,
    pub source: Option<String>,
    pub exclude: Vec<String>,
    /// A `[[requirements]]` block the app needs, ready to paste.
    pub requirements: Option<String>,
}

pub fn detect(dir: &Path) -> Option<Detected> {
    let mut found = engines::godot(dir)
        .or_else(|| engines::unity(dir))
        .or_else(|| tauri(dir))
        .or_else(|| wails(dir))
        .or_else(|| electron(dir))
        .or_else(|| dotnet(dir))
        .or_else(|| flutter(dir))
        .or_else(|| pyinstaller(dir))
        .or_else(|| qt(dir))
        .or_else(|| rust(dir))?;
    // A name from a manifest (`@scope/my-app`, `Notes: Pro`) goes into
    // `app.name`, which becomes a folder and a registry key. Whatever it can't
    // be, `bach init` drops, so the folder's own name is used and the config it
    // writes passes `bach check`.
    found.name = found.name.as_deref().and_then(app_name);
    // The other values come from files anyone who made the project could write. Each
    // must be what its setting expects, or it is dropped and `bach init` leaves the default.
    found.executable = found.executable.filter(|exe| is_plain_file_name(exe));
    found.source = found.source.filter(|source| is_plain_relative_path(source));
    found.version = found.version.filter(|version| is_plain_version(version));
    found.publisher = found
        .publisher
        .map(|publisher| publisher.chars().filter(|c| !c.is_control()).take(100).collect::<String>().trim().to_string())
        .filter(|publisher| !publisher.is_empty());
    Some(found)
}

/// A file name: no folders, no `..`, no characters Windows forbids.
fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && bach_pack::paths::normalize(name).is_ok_and(|normal| !normal.contains('/'))
        && !name.contains(['\\', '/', '"', '\''])
}

/// A folder inside the project, written with `/`: never absolute, never climbing out.
fn is_plain_relative_path(path: &str) -> bool {
    !path.is_empty() && !path.contains(['\\', '"', '\'']) && bach_pack::paths::normalize(path).is_ok()
}

/// A version: digits, letters and `. - + _` only.
fn is_plain_version(version: &str) -> bool {
    !version.is_empty() && version.len() <= 64 && version.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'))
}

/// A manifest's name as an `app.name` that is also a valid id: no npm scope,
/// letters, digits, spaces, dots, dashes and underscores only, tidied. None
/// when nothing usable is left.
fn app_name(raw: &str) -> Option<String> {
    let unscoped = match raw.trim().strip_prefix('@') {
        Some(scoped) => scoped.split_once('/').map_or(scoped, |(_, name)| name),
        None => raw.trim(),
    };
    let spaced: String =
        unscoped.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '.' | '-' | '_') { c } else { ' ' }).collect();
    let words: Vec<&str> = spaced.split_whitespace().collect();
    let name = words.join(" ");
    let name = name.trim_matches(|c| c == '.' || c == ' ').to_string();
    let name: String = name.chars().take(64).collect();
    let name = name.trim_matches(|c| c == '.' || c == ' ').to_string();
    bach_pack::paths::is_valid_id(&name).then_some(name)
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn string(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key).and_then(|value| value.as_str()).map(str::trim).filter(|text| !text.is_empty()).map(String::from)
}

/// "Jane Doe <jane@example.com>" → "Jane Doe".
fn person(text: &str) -> String {
    text.split(['<', '(']).next().unwrap_or(text).trim().to_string()
}

fn tauri(dir: &Path) -> Option<Detected> {
    let config = read_json(&dir.join("src-tauri").join("tauri.conf.json"))?;
    let name = string(&config, "productName").or_else(|| config.get("package").and_then(|package| string(package, "productName")));
    let version = string(&config, "version").or_else(|| config.get("package").and_then(|package| string(package, "version")));
    // `target/release` holds the program Cargo builds, named after the crate's
    // binary (or `mainBinaryName` in Tauri 2); the product name is only what
    // Tauri's own installers call it. Without a Cargo.toml the product name is
    // the best guess left.
    let binary = string(&config, "mainBinaryName")
        .or_else(|| cargo_binary(&dir.join("src-tauri").join("Cargo.toml")))
        .or_else(|| name.clone())
        .map(|name| format!("{name}.exe"));
    Some(Detected {
        stack: "Tauri",
        publisher: string(&config, "identifier").and_then(|id| id.split('.').nth(1).map(String::from)),
        name,
        version,
        executable: binary,
        source: Some("src-tauri/target/release".into()),
        exclude: rust_leftovers(),
        requirements: Some("[[requirements]]\nkind = \"webview2\"   # Tauri apps run on WebView2\n".into()),
    })
}

/// The program name a Cargo.toml builds: its first `[[bin]]`, else the package.
fn cargo_binary(path: &Path) -> Option<String> {
    let manifest: toml::Table = std::fs::read_to_string(path).ok()?.parse().ok()?;
    let named = |table: &toml::Table| {
        table.get("name").and_then(|name| name.as_str()).map(str::trim).filter(|name| !name.is_empty()).map(String::from)
    };
    manifest
        .get("bin")
        .and_then(|bins| bins.as_array())
        .and_then(|bins| bins.iter().filter_map(|bin| bin.as_table()).find_map(named))
        .or_else(|| manifest.get("package").and_then(|package| package.as_table()).and_then(named))
}

fn electron(dir: &Path) -> Option<Detected> {
    let package = read_json(&dir.join("package.json"))?;
    let depends = |name: &str| {
        ["dependencies", "devDependencies"].iter().any(|section| package.get(section).and_then(|deps| deps.get(name)).is_some())
    };
    if !depends("electron") {
        return None;
    }
    // `@scope/my-app` is a package name, not a product name: the scope goes,
    // and `my-app` is shown as "My App".
    let product = package.get("build").and_then(|build| string(build, "productName")).or_else(|| string(&package, "productName"));
    let unscoped = string(&package, "name").map(|name| name.rsplit('/').next().unwrap_or(&name).to_string());
    // The program is named like the product, or like the package without its scope.
    let stem = product.clone().or_else(|| unscoped.clone());
    let name = product.or_else(|| unscoped.as_deref().map(title_case));
    let author = match package.get("author") {
        Some(serde_json::Value::String(author)) => Some(person(author)),
        Some(author) => string(author, "name"),
        None => None,
    };
    let forge = depends("@electron-forge/cli");
    Some(Detected {
        stack: "Electron",
        executable: stem.as_ref().map(|stem| format!("{stem}.exe")),
        source: Some(match (&stem, forge) {
            (Some(stem), true) => format!("out/{stem}-win32-x64"),
            _ => "dist/win-unpacked".into(),
        }),
        name,
        version: string(&package, "version"),
        publisher: author,
        exclude: clutter(),
        requirements: None,
    })
}

const VCREDIST: &str = "[[requirements]]\nkind = \"vcredist\"   # the Visual C++ runtime this app is built with\n";

/// The value of a top-level `key: value` line of a YAML file, unquoted.
fn yaml_value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))
        .map(|value| value.split('#').next().unwrap_or(value).trim().trim_matches(['"', '\'']).to_string())
        .filter(|value| !value.is_empty())
}

/// A Flutter app: pubspec.yaml with a `flutter:` section. `flutter build windows`
/// leaves the program and its DLLs in build/windows/x64/runner/Release.
fn flutter(dir: &Path) -> Option<Detected> {
    let text = std::fs::read_to_string(dir.join("pubspec.yaml")).ok()?;
    if !text.lines().any(|line| line.trim_end() == "flutter:") {
        return None;
    }
    let name = yaml_value(&text, "name");
    // 1.2.3+4 is version 1.2.3, build 4.
    let version = yaml_value(&text, "version").map(|version| version.split('+').next().unwrap_or(&version).to_string());
    Some(Detected {
        stack: "Flutter",
        executable: name.as_ref().map(|name| format!("{name}.exe")),
        name: name.as_deref().map(title_case),
        version,
        source: Some("build/windows/x64/runner/Release".into()),
        exclude: clutter(),
        requirements: Some(VCREDIST.into()),
        ..Detected::default()
    })
}

/// A Go app with Wails: wails.json. `wails build` writes build/bin.
fn wails(dir: &Path) -> Option<Detected> {
    let config = read_json(&dir.join("wails.json"))?;
    let name = string(&config, "name");
    let binary = string(&config, "outputfilename").or_else(|| name.clone());
    let info = config.get("info");
    Some(Detected {
        stack: "Wails",
        executable: binary.map(|binary| format!("{binary}.exe")),
        name: info.and_then(|info| string(info, "productName")).or(name),
        version: info.and_then(|info| string(info, "productVersion")),
        publisher: info
            .and_then(|info| string(info, "companyName"))
            .or_else(|| config.get("author").and_then(|author| string(author, "name"))),
        source: Some("build/bin".into()),
        exclude: clutter(),
        requirements: Some("[[requirements]]\nkind = \"webview2\"   # Wails apps run on WebView2\n".into()),
    })
}

/// A Python app packaged with PyInstaller: a `*.spec` file with an `EXE(` in it.
/// An `--onedir` build is dist/<name>/<name>.exe.
fn pyinstaller(dir: &Path) -> Option<Detected> {
    let spec = std::fs::read_dir(dir).ok()?.flatten().map(|entry| entry.path()).find(|path| {
        path.extension().is_some_and(|ext| ext == "spec") && std::fs::read_to_string(path).is_ok_and(|text| text.contains("EXE("))
    })?;
    let text = std::fs::read_to_string(&spec).ok()?;
    // name='MyApp' or name="MyApp", in EXE( or COLLECT(.
    let name = text
        .match_indices("name=")
        // Not `filename=` or `pathex_name=`: the word has to start there.
        .filter(|(at, _)| !text[..*at].ends_with(|c: char| c.is_alphanumeric() || c == '_'))
        .find_map(|(at, word)| {
            let rest = &text[at + word.len()..];
            let quote = rest.chars().next().filter(|c| *c == '\'' || *c == '"')?;
            Some(rest[1..].split(quote).next()?.to_string()).filter(|name| !name.is_empty())
        })
        .or_else(|| spec.file_stem().map(|stem| stem.to_string_lossy().to_string()))?;
    let pyproject = std::fs::read_to_string(dir.join("pyproject.toml")).unwrap_or_default();
    let version = pyproject
        .lines()
        .find_map(|line| line.trim().strip_prefix("version")?.trim().strip_prefix('='))
        .map(|value| value.trim().trim_matches(['"', '\'']).to_string());
    Some(Detected {
        stack: "PyInstaller",
        executable: Some(format!("{name}.exe")),
        source: Some(format!("dist/{name}")),
        name: Some(name),
        version,
        exclude: vec!["*.pdb".into(), "*.log".into(), "__pycache__/**".into(), "Thumbs.db".into(), ".DS_Store".into()],
        requirements: None,
        ..Detected::default()
    })
}

/// A Qt app: CMakeLists.txt that finds Qt, or a qmake *.pro file. The program
/// and the DLLs `windeployqt` copied beside it are expected in build/Release.
fn qt(dir: &Path) -> Option<Detected> {
    let cmake = std::fs::read_to_string(dir.join("CMakeLists.txt"))
        .ok()
        .filter(|text| text.contains("find_package(Qt") || text.contains("find_package(QT"));
    let pro =
        std::fs::read_dir(dir).ok()?.flatten().map(|entry| entry.path()).find(|path| path.extension().is_some_and(|ext| ext == "pro"));
    let (name, version) = match (&cmake, &pro) {
        (Some(text), _) => {
            let call = text.lines().find_map(|line| line.trim().strip_prefix("project("))?;
            let mut words = call.trim_end_matches(')').split_whitespace();
            let name = words.next()?.to_string();
            let version = words.by_ref().skip_while(|word| !word.eq_ignore_ascii_case("VERSION")).nth(1).map(String::from);
            (name, version)
        }
        (None, Some(path)) => (path.file_stem()?.to_string_lossy().to_string(), None),
        _ => return None,
    };
    Some(Detected {
        stack: "Qt",
        executable: Some(format!("{name}.exe")),
        name: Some(title_case(&name)),
        version,
        source: Some("build/Release".into()),
        exclude: clutter(),
        requirements: Some(VCREDIST.into()),
        ..Detected::default()
    })
}

fn dotnet(dir: &Path) -> Option<Detected> {
    let project = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("csproj")))?;
    let text = std::fs::read_to_string(&project).ok()?;
    let tag = |name: &str| {
        let open = format!("<{name}>");
        let start = text.find(&open)? + open.len();
        let end = text[start..].find("</")? + start;
        Some(text[start..end].trim().to_string()).filter(|value| !value.is_empty())
    };
    let stem = project.file_stem()?.to_string_lossy().to_string();
    let assembly = tag("AssemblyName").unwrap_or(stem);
    let framework = tag("TargetFramework").unwrap_or_else(|| "net8.0".into());
    // net8.0-windows → .NET 8.0, and the Windows Desktop runtime.
    let version: String = framework.trim_start_matches("net").chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let desktop = framework.contains("-windows");
    let requirements = (version.split('.').count() == 2).then(|| {
        format!(
            "[[requirements]]\nkind = \"dotnet\"     # or publish --self-contained and drop this\nversion = \"{version}\"\ndesktop = {desktop}\n"
        )
    });
    Some(Detected {
        stack: ".NET",
        name: tag("Product").or_else(|| Some(assembly.clone())),
        version: tag("Version"),
        publisher: tag("Company").or_else(|| tag("Authors")),
        executable: Some(format!("{assembly}.exe")),
        source: Some(format!("bin/Release/{framework}/win-x64/publish")),
        exclude: clutter(),
        requirements,
    })
}

fn rust(dir: &Path) -> Option<Detected> {
    let manifest: toml::Value = toml::from_str(&std::fs::read_to_string(dir.join("Cargo.toml")).ok()?).ok()?;
    let package = manifest.get("package")?;
    let text = |key: &str| package.get(key).and_then(|value| value.as_str()).map(String::from);
    let binary = manifest
        .get("bin")
        .and_then(|bins| bins.as_array())
        .and_then(|bins| bins.first())
        .and_then(|bin| bin.get("name"))
        .and_then(|name| name.as_str())
        .map(String::from)
        .or_else(|| text("name"));
    let author = package
        .get("authors")
        .and_then(|authors| authors.as_array())
        .and_then(|authors| authors.first())
        .and_then(|author| author.as_str())
        .map(person);
    Some(Detected {
        stack: "Rust",
        name: text("name").map(|name| title_case(&name)),
        version: text("version"),
        publisher: author,
        executable: binary.map(|binary| format!("{binary}.exe")),
        source: Some("target/release".into()),
        exclude: rust_leftovers(),
        requirements: None,
    })
}

/// What `cargo build --release` leaves next to the program.
/// Files every build folder collects and no installer needs.
fn clutter() -> Vec<String> {
    ["*.pdb", "*.log", "Thumbs.db", ".DS_Store"].iter().map(|pattern| pattern.to_string()).collect()
}

fn rust_leftovers() -> Vec<String> {
    let mut patterns = clutter();
    patterns.extend(
        ["*.d", "*.rlib", "build/**", "deps/**", "incremental/**", ".fingerprint/**", "examples/**", "bundle/**"]
            .iter()
            .map(|pattern| pattern.to_string()),
    );
    patterns
}

/// "my-cool_app" → "My Cool App".
pub fn title_case(name: &str) -> String {
    name.split(['-', '_', ' '])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map(|first| first.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("bach-detect-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, text) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    /// A project file is not trusted: whatever it says can't add settings to the config
    /// `bach init` writes, or point it outside the project.
    #[test]
    fn hostile_project_files_cannot_inject_settings() {
        let hostile = project(
            "hostile",
            &[(
                "wails.json",
                r#"{"name":"ok","outputfilename":"..\\..\\evil","author":{"name":"Jane\"\n[signing]\nprogram = \"calc\""},"info":{"productVersion":"1.0\"\n[[run]]\nfile = \"x\"","companyName":"A\"\nB"}}"#,
            )],
        );
        let found = detect(&hostile).unwrap();
        assert_eq!(found.executable, None, "a path isn't a program name");
        assert_eq!(found.version, None, "a version with a quote and a newline is dropped");
        let text = crate::template::fill(Some(&found), "folder", &hostile);
        for line in text.lines() {
            assert!(!line.starts_with("[signing]") && !line.starts_with("[[run]]"), "injected: {line}");
        }
        // Even a value that slipped past detection is escaped when written.
        let sneaky = Detected {
            name: Some("a\"\n[signing]".into()),
            version: Some("1\"\n[[run]]".into()),
            source: Some("x\\\"y".into()),
            ..Detected::default()
        };
        let text = crate::template::fill(Some(&sneaky), "folder", &hostile);
        assert!(text.lines().all(|line| !line.starts_with("[signing]") && !line.starts_with("[[run]]")), "{text}");
        assert!(!is_plain_relative_path("../up") && !is_plain_relative_path("C:/abs") && !is_plain_relative_path("/abs"));
        assert!(is_plain_file_name("My App.exe") && !is_plain_file_name("a/b.exe"));
    }

    #[test]
    fn flutter_wails_pyinstaller_and_qt_projects_are_recognised() {
        let flutter = project("flutter", &[("pubspec.yaml", "name: my_notes\nversion: 1.4.2+7\nflutter:\n  uses-material-design: true\n")]);
        let found = detect(&flutter).unwrap();
        assert_eq!((found.stack, found.name.as_deref(), found.version.as_deref()), ("Flutter", Some("My Notes"), Some("1.4.2")));
        assert_eq!(found.executable.as_deref(), Some("my_notes.exe"));
        assert_eq!(found.source.as_deref(), Some("build/windows/x64/runner/Release"));
        assert!(found.requirements.as_deref().unwrap().contains("vcredist"));
        // A Dart package that isn't a Flutter app is not one.
        assert!(detect(&project("dart", &[("pubspec.yaml", "name: lib\nversion: 1.0.0\n")])).is_none());

        let wails = project(
            "wails",
            &[(
                "wails.json",
                r#"{"name":"clip","outputfilename":"clipper","author":{"name":"Jane"},"info":{"companyName":"Acme","productVersion":"2.0.1","productName":"Clipper"}}"#,
            )],
        );
        let found = detect(&wails).unwrap();
        assert_eq!(
            (found.stack, found.name.as_deref(), found.version.as_deref(), found.publisher.as_deref()),
            ("Wails", Some("Clipper"), Some("2.0.1"), Some("Acme"))
        );
        assert_eq!((found.executable.as_deref(), found.source.as_deref()), (Some("clipper.exe"), Some("build/bin")));
        assert!(found.requirements.as_deref().unwrap().contains("webview2"));

        let python = project(
            "pyinstaller",
            &[
                ("tool.spec", "a = Analysis(['main.py'])\nexe = EXE(pyz, a.scripts, name='PyTool')\n"),
                ("pyproject.toml", "[project]\nname = \"pytool\"\nversion = \"0.9.0\"\n"),
            ],
        );
        let found = detect(&python).unwrap();
        assert_eq!((found.stack, found.name.as_deref(), found.version.as_deref()), ("PyInstaller", Some("PyTool"), Some("0.9.0")));
        assert_eq!((found.executable.as_deref(), found.source.as_deref()), (Some("PyTool.exe"), Some("dist/PyTool")));
        assert!(detect(&project("spec-without-exe", &[("x.spec", "nothing here")])).is_none());

        let qt = project("qt", &[("CMakeLists.txt", "cmake_minimum_required(VERSION 3.20)\nproject(Notepad VERSION 3.1.0 LANGUAGES CXX)\nfind_package(Qt6 REQUIRED COMPONENTS Widgets)\n")]);
        let found = detect(&qt).unwrap();
        assert_eq!((found.stack, found.name.as_deref(), found.version.as_deref()), ("Qt", Some("Notepad"), Some("3.1.0")));
        assert_eq!(found.executable.as_deref(), Some("Notepad.exe"));
        assert!(detect(&project("cmake", &[("CMakeLists.txt", "project(Plain)\n")])).is_none(), "CMake without Qt isn't Qt");
    }

    #[test]
    fn manifest_names_become_valid_app_names() {
        for (raw, expected) in [
            ("@scope/my-app", Some("my-app")),
            ("@scope/Notes", Some("Notes")),
            ("name.with.dots", Some("name.with.dots")),
            ("Notes: Pro", Some("Notes Pro")),
            ("12345", Some("12345")),
            ("\u{6253}\u{5f00}", Some("\u{6253}\u{5f00}")),
            ("  Spaces  ", Some("Spaces")),
            ("nul", None),
            ("@/", None),
            ("???", None),
            ("", None),
        ] {
            assert_eq!(app_name(raw).as_deref(), expected, "{raw:?}");
        }
        assert!(app_name(&"a".repeat(100)).is_some_and(|name| name.len() <= 64));
    }

    #[test]
    fn a_scoped_npm_package_gets_a_valid_config() {
        let dir = project(
            "scoped",
            &[("package.json", r#"{"name":"@scope/my-app","version":"1.2.3","main":"main.js","devDependencies":{"electron":"^30.0.0"}}"#)],
        );
        let found = detect(&dir).unwrap();
        assert_eq!(found.name.as_deref(), Some("My App"));
        let text = crate::template::fill(Some(&found), "folder", &dir);
        assert!(text.contains("name = \"My App\""), "{text}");
        assert!(!text.contains('@'), "{text}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stacks_are_recognised() {
        let rust = project(
            "rust",
            &[("Cargo.toml", "[package]\nname = \"note-pad\"\nversion = \"0.3.1\"\nauthors = [\"Jane Doe <jane@x.dev>\"]\n")],
        );
        let found = detect(&rust).unwrap();
        assert_eq!((found.stack, found.name.as_deref(), found.version.as_deref()), ("Rust", Some("Note Pad"), Some("0.3.1")));
        assert_eq!(found.executable.as_deref(), Some("note-pad.exe"));
        assert_eq!(found.publisher.as_deref(), Some("Jane Doe"));

        let electron = project(
            "electron",
            &[(
                "package.json",
                r#"{"name":"notes","productName":"Notes","version":"2.0.0","author":{"name":"Acme"},"devDependencies":{"electron":"30"}}"#,
            )],
        );
        let found = detect(&electron).unwrap();
        assert_eq!(
            (found.stack, found.executable.as_deref(), found.source.as_deref()),
            ("Electron", Some("Notes.exe"), Some("dist/win-unpacked"))
        );

        let tauri =
            project("tauri", &[("src-tauri/tauri.conf.json", r#"{"productName":"Clip","version":"1.1.0","identifier":"com.acme.clip"}"#)]);
        let found = detect(&tauri).unwrap();
        assert_eq!((found.stack, found.publisher.as_deref()), ("Tauri", Some("acme")));
        assert_eq!(found.executable.as_deref(), Some("Clip.exe"), "without a Cargo.toml the product name is the guess");
        assert!(found.requirements.unwrap().contains("webview2"));

        // Cargo names the program after the crate, not after the product.
        let named = project(
            "tauri-named",
            &[
                ("src-tauri/tauri.conf.json", r#"{"productName":"My Clip","version":"1.1.0"}"#),
                ("src-tauri/Cargo.toml", "[package]\nname = \"clip-app\"\nversion = \"1.1.0\"\n"),
            ],
        );
        assert_eq!(detect(&named).unwrap().executable.as_deref(), Some("clip-app.exe"));
        let bins = project(
            "tauri-bins",
            &[
                ("src-tauri/tauri.conf.json", r#"{"productName":"My Clip"}"#),
                ("src-tauri/Cargo.toml", "[package]\nname = \"clip-app\"\n[[bin]]\nname = \"clip\"\npath = \"src/main.rs\"\n"),
            ],
        );
        assert_eq!(detect(&bins).unwrap().executable.as_deref(), Some("clip.exe"));
        let main = project(
            "tauri-main",
            &[
                ("src-tauri/tauri.conf.json", r#"{"productName":"My Clip","mainBinaryName":"Clip Pro"}"#),
                ("src-tauri/Cargo.toml", "[package]\nname = \"clip-app\"\n"),
            ],
        );
        assert_eq!(detect(&main).unwrap().executable.as_deref(), Some("Clip Pro.exe"));
        for dir in [named, bins, main] {
            std::fs::remove_dir_all(dir).unwrap();
        }

        let dotnet = project("dotnet", &[("Paint.csproj", "<Project><PropertyGroup><TargetFramework>net8.0-windows</TargetFramework><Version>4.2.0</Version></PropertyGroup></Project>")]);
        let found = detect(&dotnet).unwrap();
        assert_eq!((found.stack, found.executable.as_deref(), found.version.as_deref()), (".NET", Some("Paint.exe"), Some("4.2.0")));
        assert!(found.requirements.unwrap().contains("desktop = true"));

        let plain = project("plain", &[("readme.txt", "hi")]);
        assert!(detect(&plain).is_none());
        for dir in [rust, electron, tauri, dotnet, plain] {
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}
