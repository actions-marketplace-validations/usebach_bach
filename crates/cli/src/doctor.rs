//! `bach doctor`: whether everything a build needs is in place, in a few
//! lines a person or an agent can act on.

use std::path::Path;

use anyhow::Result;

use bach_pack::manifest::Platform;

use crate::output::field;
use crate::runtime::{self, Found};

pub fn doctor(config: &Path) -> Result<()> {
    say!("Bach {} on {} ({})", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH);
    let mut problems = 0;
    let mut signing_program = None;
    // The runtimes this config builds on: Windows' when it has [files] (or there is no config), macOS' for [macos].
    let declared = std::fs::read_to_string(config).ok().and_then(|text| bach_pack::config::declared(&text).ok());
    let wanted: Vec<Platform> = match declared {
        Some(declared) => [(Platform::Windows, declared.windows), (Platform::Macos, declared.macos)]
            .into_iter()
            .filter(|(_, on)| *on)
            .map(|(platform, _)| platform)
            .collect(),
        None => vec![Platform::Windows],
    };
    for platform in &wanted {
        let label = if *platform == Platform::Macos { "macOS runtime" } else { "Runtime" };
        let trust = if *platform == Platform::Macos { "macOS runtime trust" } else { "Runtime trust" };
        match runtime::locate_for(*platform, None) {
            Ok(Found::Here(path)) => match runtime::read_for(*platform, &path) {
                Ok(_) => {
                    let hash = runtime::hash_of(&path).unwrap_or_default();
                    field(label, format!("{} (SHA-256 {hash})", path.display()));
                    field(trust, runtime::trust_note_for(*platform));
                }
                Err(err) => {
                    problems += 1;
                    field(label, format!("{} is unusable: {err:#}", path.display()));
                }
            },
            Ok(Found::Downloadable(url)) => {
                field(label, format!("not here yet; the first build downloads it from {url}"));
                field(trust, runtime::trust_note_for(*platform));
            }
            Err(err) => {
                problems += 1;
                field(label, format!("{err:#}"));
            }
        }
    }

    if config.is_file() {
        let platform = (wanted == [Platform::Macos]).then_some("macos");
        match crate::builder::load_platform(config, None, None, None, platform) {
            Ok(project) => {
                let manifest = &project.manifest;
                signing_program = project.signing.as_ref().map(|signing| signing.program.clone());
                let warnings = match project.warnings.len() {
                    0 => String::new(),
                    1 => " (1 warning: run bach check)".into(),
                    count => format!(" ({count} warnings: run bach check)"),
                };
                field("Config", format!("{} is valid: {} {}{warnings}", config.display(), manifest.app.name, manifest.app.version));
            }
            Err(err) => {
                problems += 1;
                field("Config", format!("{err:#}"));
            }
        }
    } else {
        field("Config", format!("no {} here; `bach init` writes one", config.display()));
    }

    field(
        "Preview",
        if cfg!(windows) {
            "available".to_string()
        } else if cfg!(target_os = "macos") {
            "available for the macOS setup window (needs [macos]); the Windows one needs Windows".to_string()
        } else {
            "needs Windows or macOS (bach check works here)".to_string()
        },
    );
    match signing_program {
        Some(program) => field("Signing", format!("[signing] runs {program} when you build with --sign (or BACH_SIGN=1)")),
        None => field(
            "Signing",
            match crate::sign::known_signer() {
                Some(tool) => format!("off, {tool} found (add [signing] and build with --sign)"),
                None => "off (optional: [signing] in bach.toml, see docs/signing.md)".to_string(),
            },
        ),
    }
    say!();
    crate::output::result("ready", problems == 0);
    crate::output::result("problems", problems);
    if problems == 0 {
        say!("Ready to build.");
    } else {
        say!("Fix the {} above, then run bach doctor again.", if problems == 1 { "problem" } else { "problems" });
    }
    Ok(())
}
