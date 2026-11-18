//! Games made with an engine: Godot (project.godot) and Unity
//! (ProjectSettings/ProjectSettings.asset). Each reads the game's name,
//! version and studio from the project, and where its Windows build goes.

use std::path::Path;

use super::Detected;

/// A Godot project. The Windows export preset in export_presets.cfg says
/// where the game goes; without one, `build/<name>.exe` is the guess.
pub(super) fn godot(dir: &Path) -> Option<Detected> {
    let text = std::fs::read_to_string(dir.join("project.godot")).ok()?;
    let name = setting(&text, "config/name");
    let version = setting(&text, "config/version");
    let export = std::fs::read_to_string(dir.join("export_presets.cfg")).ok().and_then(|presets| windows_export(&presets));
    let (source, executable) = match export.as_deref().and_then(|path| path.rsplit_once('/')) {
        Some((folder, file)) => (Some(folder.trim_start_matches("./").to_string()), Some(file.to_string())),
        None => (Some("build".into()), name.as_ref().map(|name| format!("{name}.exe"))),
    };
    Some(Detected {
        stack: "Godot",
        name,
        version,
        executable,
        source,
        // The game's .pck sits beside the program and must ship with it.
        exclude: vec!["*.pdb".into(), "*.log".into()],
        ..Detected::default()
    })
}

/// A `key="value"` line of a Godot config file.
fn setting(text: &str, key: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix(key)?.trim_start().strip_prefix('='))
        .map(|value| value.trim().trim_matches('"').trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The `export_path` of the "Windows Desktop" preset, written with `/`.
fn windows_export(presets: &str) -> Option<String> {
    let mut sections: Vec<Vec<&str>> = Vec::new();
    for line in presets.lines() {
        if line.trim_start().starts_with('[') {
            sections.push(Vec::new());
        } else if let Some(section) = sections.last_mut() {
            section.push(line);
        }
    }
    sections.iter().find_map(|lines| {
        let text = lines.join("\n");
        (setting(&text, "platform").as_deref() == Some("Windows Desktop"))
            .then(|| setting(&text, "export_path"))
            .flatten()
            // Godot writes a backslash in its config files as `\\`.
            .map(|path| path.replace("\\\\", "/").replace('\\', "/"))
    })
}

/// A Unity project. Builds go wherever the developer picks; `Build/` with
/// `<product>.exe` is the usual choice. The folders Unity marks as
/// "don't ship" stay out.
pub(super) fn unity(dir: &Path) -> Option<Detected> {
    let settings = dir.join("ProjectSettings");
    if !settings.join("ProjectVersion.txt").is_file() {
        return None;
    }
    let text = std::fs::read_to_string(settings.join("ProjectSettings.asset")).unwrap_or_default();
    let name = asset_value(&text, "productName");
    Some(Detected {
        stack: "Unity",
        executable: name.as_ref().map(|name| format!("{name}.exe")),
        publisher: asset_value(&text, "companyName").filter(|company| company != "DefaultCompany"),
        version: asset_value(&text, "bundleVersion"),
        name,
        source: Some("Build".into()),
        exclude: vec![
            "*_BurstDebugInformation_DoNotShip/**".into(),
            "*_BackUpThisFolder_ButDontShipItInYourBuild/**".into(),
            "*.pdb".into(),
        ],
        ..Detected::default()
    })
}

/// A `  key: value` line of a Unity settings file.
fn asset_value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.trim_start().strip_prefix(key)?.strip_prefix(':'))
        .map(|value| value.trim().trim_matches(['"', '\'']).to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("bach-detect-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_godot_game_and_its_windows_export() {
        let dir = folder("godot");
        std::fs::write(
            dir.join("project.godot"),
            "config_version=5\n\n[application]\n\nconfig/name=\"Moon Garden\"\nconfig/version=\"0.3.1\"\nrun/main_scene=\"res://main.tscn\"\n",
        )
        .unwrap();
        let guessed = godot(&dir).unwrap();
        assert_eq!((guessed.name.as_deref(), guessed.version.as_deref()), (Some("Moon Garden"), Some("0.3.1")));
        assert_eq!((guessed.source.as_deref(), guessed.executable.as_deref()), (Some("build"), Some("Moon Garden.exe")));

        std::fs::write(
            dir.join("export_presets.cfg"),
            "[preset.0]\n\nname=\"Web\"\nplatform=\"Web\"\nexport_path=\"web/index.html\"\n\n[preset.0.options]\n\n\
             [preset.1]\n\nname=\"Windows Desktop\"\nplatform=\"Windows Desktop\"\nexport_path=\"export\\\\win/MoonGarden.exe\"\n",
        )
        .unwrap();
        let exported = godot(&dir).unwrap();
        assert_eq!((exported.source.as_deref(), exported.executable.as_deref()), (Some("export/win"), Some("MoonGarden.exe")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_unity_game_from_its_player_settings() {
        let dir = folder("unity");
        assert!(unity(&dir).is_none());
        std::fs::create_dir_all(dir.join("ProjectSettings")).unwrap();
        std::fs::write(dir.join("ProjectSettings/ProjectVersion.txt"), "m_EditorVersion: 6000.0.23f1\n").unwrap();
        std::fs::write(
            dir.join("ProjectSettings/ProjectSettings.asset"),
            "%YAML 1.1\nPlayerSettings:\n  companyName: Tiny Moon\n  productName: Starfall\n  bundleVersion: 1.2.0\n",
        )
        .unwrap();
        let found = unity(&dir).unwrap();
        assert_eq!(found.name.as_deref(), Some("Starfall"));
        assert_eq!(found.publisher.as_deref(), Some("Tiny Moon"));
        assert_eq!(found.version.as_deref(), Some("1.2.0"));
        assert_eq!(found.executable.as_deref(), Some("Starfall.exe"));
        assert!(found.exclude.iter().any(|pattern| pattern.contains("DoNotShip")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
