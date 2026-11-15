//! What kind of project `bach init` runs in, so the starter config already
//! names the app, its version and where its build ends up.
//!
//! Reads Cargo.toml (Rust), src-tauri/tauri.conf.json (Tauri), package.json
//! (Electron) and *.csproj (.NET). Anything it can't read is simply left
//! for the developer to fill in.

use std::path::Path;

/// What the project says about itself; None where it doesn't say.
#[derive(Debug, Default, PartialEq)]
pub struct Detected {
    /// "Rust", "Tauri", "Electron" or ".NET", for the message.
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
    let mut found = tauri(dir).or_else(|| electron(dir)).or_else(|| dotnet(dir)).or_else(|| rust(dir))?;
    // A name from a manifest (`@scope/my-app`, `Notes: Pro`) goes into
    // `app.name`, which becomes a folder and a registry key. Whatever it can't
    // be, `bach init` drops, so the folder's own name is used and the config it
    // writes passes `bach check`.
    found.name = found.name.as_deref().and_then(app_name);
    Some(found)
}

/// A manifest's name as an `app.name` that is also a valid id: no npm scope,
/// letters, digits, spaces, dots, dashes and underscores only, tidied. None
/// when nothing usable is left.
fn app_name(raw: &str) -> Option<String> {
    let unscoped = match raw.trim().strip_prefix('@') {
        Some(scoped) => scoped.split_once('/').map_or(scoped, |(_, name)| name),
        None => raw.trim(),
    };
    let spaced: String = unscoped
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '.' | '-' | '_') { c } else { ' ' })
        .collect();
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
    let binary = name.as_ref().map(|name| format!("{name}.exe"));
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
        let rust = project("rust", &[("Cargo.toml", "[package]\nname = \"note-pad\"\nversion = \"0.3.1\"\nauthors = [\"Jane Doe <jane@x.dev>\"]\n")]);
        let found = detect(&rust).unwrap();
        assert_eq!((found.stack, found.name.as_deref(), found.version.as_deref()), ("Rust", Some("Note Pad"), Some("0.3.1")));
        assert_eq!(found.executable.as_deref(), Some("note-pad.exe"));
        assert_eq!(found.publisher.as_deref(), Some("Jane Doe"));

        let electron = project("electron", &[("package.json", r#"{"name":"notes","productName":"Notes","version":"2.0.0","author":{"name":"Acme"},"devDependencies":{"electron":"30"}}"#)]);
        let found = detect(&electron).unwrap();
        assert_eq!((found.stack, found.executable.as_deref(), found.source.as_deref()), ("Electron", Some("Notes.exe"), Some("dist/win-unpacked")));

        let tauri = project("tauri", &[("src-tauri/tauri.conf.json", r#"{"productName":"Clip","version":"1.1.0","identifier":"com.acme.clip"}"#)]);
        let found = detect(&tauri).unwrap();
        assert_eq!((found.stack, found.publisher.as_deref()), ("Tauri", Some("acme")));
        assert!(found.requirements.unwrap().contains("webview2"));

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
